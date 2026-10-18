//! Claude Messages API over the system `curl`, streaming Server-Sent Events.
//! The key and the request body go to curl on stdin (`--config -`), never on its command line.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

pub const API_VERSION: &str = "2023-06-01";
pub const DEFAULT_BASE: &str = "https://api.anthropic.com";

#[derive(Clone, Debug)]
pub struct Settings {
    pub model: String,
    pub max_tokens: u32,
    pub base_url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    Error(String),
}

/// One finished assistant turn: the content blocks to keep in the history and why it stopped.
#[derive(Clone, Debug, Default)]
pub struct Turn {
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
}

/// Anything that can produce an assistant turn; the real one is `Curl`, tests script it.
pub trait Model: Send {
    fn stream(
        &self,
        system: &str,
        tools: &Value,
        messages: &[Value],
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Turn, String>;
}

pub fn build_body(s: &Settings, system: &str, tools: &Value, messages: &[Value]) -> Value {
    let mut tools = tools.clone();
    if let Some(last) = tools.as_array_mut().and_then(|a| a.last_mut()) {
        last["cache_control"] = json!({"type": "ephemeral"});
    }
    let mut body = json!({
        "model": s.model,
        "max_tokens": s.max_tokens,
        "stream": true,
        "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
        "messages": messages,
    });
    if tools.as_array().is_some_and(|a| !a.is_empty()) {
        body["tools"] = tools;
    }
    body
}

/// Escape a string for a curl config-file quoted value.
fn quote(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The curl config fed on stdin: endpoint, headers (including the key) and body.
pub fn curl_config(base_url: &str, key: &str, body: &Value) -> String {
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let mut cfg = String::new();
    for (k, v) in [
        ("url", url),
        ("header", format!("x-api-key: {key}")),
        ("header", format!("anthropic-version: {API_VERSION}")),
        ("header", "content-type: application/json".to_string()),
        ("data-binary", body.to_string()),
        ("connect-timeout", "15".to_string()),
        ("max-time", "300".to_string()),
    ] {
        if matches!(k, "connect-timeout" | "max-time") {
            cfg.push_str(&format!("{k} = {v}\n"));
        } else {
            cfg.push_str(&format!("{k} = {}\n", quote(&v)));
        }
    }
    cfg.push_str("silent\nno-buffer\n");
    cfg
}

fn friendly(kind: &str, message: &str) -> String {
    match kind {
        "authentication_error" => "API key rejected: check ANTHROPIC_API_KEY".into(),
        "permission_error" => format!("API key not allowed to do this: {message}"),
        "rate_limit_error" => "Rate limited by the API; try again in a moment".into(),
        "overloaded_error" => "The API is overloaded; try again in a moment".into(),
        "billing_error" => format!("Billing problem: {message}"),
        _ => message.to_string(),
    }
}

/// Turns SSE lines into events and assembles the final content blocks.
#[derive(Default)]
pub struct Assembler {
    blocks: Vec<Block>,
    stop_reason: Option<String>,
    pub failed: bool,
}
struct Block {
    index: u64,
    kind: String,
    text: String,
    id: String,
    name: String,
    json: String,
    done: bool,
}
impl Assembler {
    pub fn line(&mut self, raw: &str) -> Vec<Event> {
        let line = raw.trim();
        let payload = line.strip_prefix("data:").map(str::trim).or_else(|| {
            // A plain HTTP error body (not SSE) is one JSON object.
            line.starts_with('{').then_some(line)
        });
        let Some(Ok(v)) = payload.map(serde_json::from_str::<Value>) else {
            return Vec::new();
        };
        let index = v["index"].as_u64().unwrap_or(0);
        match v["type"].as_str().unwrap_or("") {
            "error" => {
                self.failed = true;
                let e = &v["error"];
                vec![Event::Error(friendly(
                    e["type"].as_str().unwrap_or(""),
                    e["message"].as_str().unwrap_or("unknown API error"),
                ))]
            }
            "content_block_start" => {
                let b = &v["content_block"];
                self.blocks.push(Block {
                    index,
                    kind: b["type"].as_str().unwrap_or("").into(),
                    text: String::new(),
                    id: b["id"].as_str().unwrap_or("").into(),
                    name: b["name"].as_str().unwrap_or("").into(),
                    json: String::new(),
                    done: false,
                });
                Vec::new()
            }
            "content_block_delta" => {
                let d = &v["delta"];
                let Some(b) = self.blocks.iter_mut().rev().find(|b| b.index == index) else {
                    return Vec::new();
                };
                match d["type"].as_str() {
                    Some("text_delta") => {
                        let t = d["text"].as_str().unwrap_or("");
                        b.text.push_str(t);
                        vec![Event::Text(t.to_string())]
                    }
                    Some("input_json_delta") => {
                        b.json.push_str(d["partial_json"].as_str().unwrap_or(""));
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
            "content_block_stop" => {
                let Some(b) = self.blocks.iter_mut().rev().find(|b| b.index == index) else {
                    return Vec::new();
                };
                b.done = true;
                if b.kind == "tool_use" {
                    let input = if b.json.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&b.json).unwrap_or(json!({}))
                    };
                    vec![Event::ToolUse {
                        id: b.id.clone(),
                        name: b.name.clone(),
                        input,
                    }]
                } else {
                    Vec::new()
                }
            }
            "message_delta" => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.into());
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
    pub fn finish(self) -> Turn {
        let content = self
            .blocks
            .into_iter()
            .filter_map(|b| match b.kind.as_str() {
                "text" if !b.text.is_empty() => Some(json!({"type": "text", "text": b.text})),
                "tool_use" => Some(json!({
                    "type": "tool_use",
                    "id": b.id,
                    "name": b.name,
                    "input": if b.json.trim().is_empty() { json!({}) } else { serde_json::from_str(&b.json).unwrap_or(json!({})) },
                })),
                _ => None,
            })
            .collect();
        Turn {
            content,
            stop_reason: self.stop_reason,
        }
    }
}

pub struct Curl {
    pub settings: Settings,
    pub key: String,
}

impl Model for Curl {
    fn stream(
        &self,
        system: &str,
        tools: &Value,
        messages: &[Value],
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Turn, String> {
        let body = build_body(&self.settings, system, tools, messages);
        let config = curl_config(&self.settings.base_url, &self.key, &body);
        let mut child = Command::new("curl")
            .args(["--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not run curl: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("curl stdin unavailable")?;
        stdin
            .write_all(config.as_bytes())
            .map_err(|e| format!("could not send the request: {e}"))?;
        drop(stdin);
        let out = child.stdout.take().ok_or("curl stdout unavailable")?;
        let mut asm = Assembler::default();
        let mut seen = false;
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if !line.trim().is_empty() {
                seen = true;
            }
            for ev in asm.line(&line) {
                on_event(ev);
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        if asm.failed {
            return Err("request failed".into());
        }
        if !status.success() || !seen {
            return Err(match status.code() {
                Some(6) | Some(7) | Some(28) => "network error: cannot reach the API".into(),
                Some(c) => format!("network error (curl exit {c})"),
                None => "curl was interrupted".into(),
            });
        }
        Ok(asm.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str]) -> (Vec<Event>, Turn) {
        let mut asm = Assembler::default();
        let mut events = Vec::new();
        for l in lines {
            events.extend(asm.line(l));
        }
        (events, asm.finish())
    }

    #[test]
    fn text_and_split_tool_input_are_assembled() {
        let (events, turn) = run(&[
            "event: content_block_start",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"open_app","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"name\": \"Calc"}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"ulator\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        ]);
        assert_eq!(events[0], Event::Text("Hel".into()));
        assert_eq!(events[1], Event::Text("lo".into()));
        assert_eq!(
            events[2],
            Event::ToolUse {
                id: "tu_1".into(),
                name: "open_app".into(),
                input: json!({"name": "Calculator"})
            }
        );
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(turn.content[0]["text"], "Hello");
        assert_eq!(turn.content[1]["input"]["name"], "Calculator");
    }

    #[test]
    fn api_errors_become_readable_messages() {
        let (events, _) = run(&[
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
        ]);
        assert_eq!(
            events,
            vec![Event::Error(
                "API key rejected: check ANTHROPIC_API_KEY".into()
            )]
        );
        let (events, _) = run(&[
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ]);
        assert!(matches!(&events[0], Event::Error(m) if m.contains("overloaded")));
    }

    #[test]
    fn the_key_goes_only_into_the_stdin_config() {
        let s = Settings {
            model: "m".into(),
            max_tokens: 100,
            base_url: DEFAULT_BASE.into(),
        };
        let body = build_body(
            &s,
            "sys \"quoted\"\nline",
            &json!([{"name": "t"}]),
            &[json!({"role": "user", "content": "hi"})],
        );
        assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");
        assert!(build_body(&s, "x", &json!([]), &[]).get("tools").is_none());
        let cfg = curl_config(DEFAULT_BASE, "sk-ant-secret", &body);
        assert!(cfg.contains("x-api-key: sk-ant-secret"));
        assert!(cfg.contains("url = \"https://api.anthropic.com/v1/messages\""));
        // The body survives curl's config-file unquoting: \\ -> \, \" -> ", \n -> newline.
        let line = cfg.lines().find(|l| l.starts_with("data-binary")).unwrap();
        assert!(!line.contains('\n'));
        let quoted = line.trim_start_matches("data-binary = ");
        let unquoted: String = {
            let inner = &quoted[1..quoted.len() - 1];
            let mut out = String::new();
            let mut it = inner.chars();
            while let Some(c) = it.next() {
                if c == '\\' {
                    match it.next() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('r') => out.push('\r'),
                        Some(o) => out.push(o),
                        None => {}
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };
        assert_eq!(serde_json::from_str::<Value>(&unquoted).unwrap(), body);
    }
}

#[cfg(test)]
mod curl_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve one canned SSE reply on 127.0.0.1 and return what the client sent.
    fn serve(
        status: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = s.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).to_string();
                if let Some(at) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if req.len() >= at + 4 + len {
                        break;
                    }
                }
            }
            let reply = format!("HTTP/1.1 {status}\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&req).into_owned()
        });
        (url, h)
    }

    #[test]
    fn curl_streams_a_reply_and_sends_the_key_as_a_header() {
        let sse = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"pong\"}}\n\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n";
        let (url, server) = serve("200 OK", sse);
        let curl = Curl {
            settings: Settings {
                model: "m".into(),
                max_tokens: 64,
                base_url: url,
            },
            key: "sk-ant-test-key".into(),
        };
        let mut text = String::new();
        let turn = curl
            .stream(
                "sys",
                &json!([{"name": "t", "input_schema": {"type": "object"}}]),
                &[json!({"role": "user", "content": "ping"})],
                &mut |e| {
                    if let Event::Text(t) = e {
                        text.push_str(&t);
                    }
                },
            )
            .unwrap();
        assert_eq!(text, "pong");
        assert_eq!(turn.stop_reason.as_deref(), Some("end_turn"));
        let req = server.join().unwrap().to_lowercase();
        assert!(req.contains("x-api-key: sk-ant-test-key"));
        assert!(req.contains("post /v1/messages"));
        assert!(req.contains("\"stream\":true") && req.contains("ping"));
    }

    #[test]
    fn curl_reports_a_rejected_key() {
        let (url, server) = serve("401 Unauthorized", "{\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"invalid x-api-key\"}}");
        let curl = Curl {
            settings: Settings {
                model: "m".into(),
                max_tokens: 64,
                base_url: url,
            },
            key: "bad".into(),
        };
        let mut errors = Vec::new();
        let r = curl.stream(
            "s",
            &json!([]),
            &[json!({"role": "user", "content": "x"})],
            &mut |e| {
                if let Event::Error(m) = e {
                    errors.push(m);
                }
            },
        );
        server.join().unwrap();
        assert!(r.is_err());
        assert_eq!(
            errors,
            vec!["API key rejected: check ANTHROPIC_API_KEY".to_string()]
        );
    }
}

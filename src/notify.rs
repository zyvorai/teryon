//! Delivering a daemon notification to the configured sinks (macOS, ntfy, a webhook).
use crate::config::NotifyConfig;
use std::io::Write;
use std::process::{Command, Stdio};

/// One notification: `event` is a stable machine name such as `agent_needs_you`.
pub struct Note {
    pub event: &'static str,
    pub title: String,
    pub message: String,
    pub workspace: String,
    pub pane: u64,
}

/// Printable, single-line text of at most `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// Quote a value for a curl config file. A raw newline would end the line and let the rest be read as
/// another curl option, so line breaks and tabs use curl's escapes and other control characters are dropped.
fn quote(v: &str) -> String {
    let mut out = String::from("\"");
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A curl `--config -` body, so the URL (often a secret topic) and the message stay off the command line.
pub fn curl_config(url: &str, headers: &[(&str, String)], body: &str) -> String {
    let mut s = format!("url = {}\nrequest = \"POST\"\n", quote(url));
    for (k, v) in headers {
        s.push_str(&format!("header = {}\n", quote(&format!("{k}: {v}"))));
    }
    s.push_str(&format!("data = {}\n", quote(body)));
    s
}

fn post(config: String) {
    let child = Command::new("curl")
        .args(["-fsS", "-m", "10", "--config", "-", "-o", "/dev/null"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(config.as_bytes());
        }
        let _ = child.wait();
    }
}

/// The JSON a webhook receives.
pub fn webhook_body(note: &Note) -> String {
    serde_json::json!({
        "title": note.title,
        "message": note.message,
        "event": note.event,
        "workspace": note.workspace,
        "pane": note.pane,
    })
    .to_string()
}

/// Send `note` to every configured sink on a background thread; failures are ignored.
pub fn send(cfg: &NotifyConfig, note: Note) {
    let cfg = cfg.clone();
    std::thread::spawn(move || {
        if cfg.macos {
            crate::util::notify(&note.message);
        }
        if let Some(url) = &cfg.ntfy_url {
            let headers = [
                ("Title", note.title.clone()),
                ("Tags", "computer".to_string()),
            ];
            post(curl_config(url, &headers, &note.message));
        }
        if let Some(url) = &cfg.webhook_url {
            let headers = [("Content-Type", "application/json".to_string())];
            post(curl_config(url, &headers, &webhook_body(&note)));
        }
    });
}

/// Send `note` to the ntfy and webhook sinks only; the desktop app shows its own macOS notification.
pub fn send_remote(cfg: &NotifyConfig, note: Note) {
    if cfg.ntfy_url.is_none() && cfg.webhook_url.is_none() {
        return;
    }
    send(
        &NotifyConfig {
            macos: false,
            ..cfg.clone()
        },
        note,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_is_flattened_and_capped() {
        assert_eq!(clean("a\nb\t\x1b[31mc  d", 50), "a b [31mc d");
        assert_eq!(clean("abcdef", 3), "abc");
    }
    #[test]
    fn curl_config_escapes_quotes_and_keeps_secrets_out_of_argv() {
        let c = curl_config(
            "https://ntfy.sh/secret",
            &[("Title", "T".into())],
            r#"say "hi" \o/"#,
        );
        assert!(c.contains("url = \"https://ntfy.sh/secret\""));
        assert!(c.contains("header = \"Title: T\""));
        assert!(c.contains(r#"data = "say \"hi\" \\o/""#));
    }
    #[test]
    fn curl_config_cannot_be_injected_through_line_breaks() {
        let c = curl_config(
            "https://x/\noutput = \"/tmp/pwn\"",
            &[("Title", "a\r\nurl = \"http://evil\"".into())],
            "m\nm",
        );
        assert_eq!(c.lines().count(), 4, "{c}");
        assert!(c
            .lines()
            .all(|l| ["url = ", "request = ", "header = ", "data = "]
                .iter()
                .any(|p| l.starts_with(p))));
    }
    #[test]
    fn a_webhook_receives_the_note_over_http() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let cfg = NotifyConfig {
            webhook_url: Some(url),
            ..NotifyConfig::default()
        };
        send(
            &cfg,
            Note {
                event: "agent_done",
                title: "TERYON".into(),
                message: "Claude finished in w (pane 1)".into(),
                workspace: "w".into(),
                pane: 1,
            },
        );
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0u8; 4096];
        let mut got = String::new();
        while !got.contains("pane 1") {
            let n = s.read(&mut buf).unwrap();
            assert!(n > 0, "connection closed early: {got}");
            got.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        assert!(got.starts_with("POST /hook"), "{got}");
        assert!(got.contains("\"event\":\"agent_done\""), "{got}");
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    }
    #[test]
    fn webhook_gets_structured_json() {
        let n = Note {
            event: "agent_needs_you",
            title: "TERYON".into(),
            message: "Claude needs you".into(),
            workspace: "w".into(),
            pane: 2,
        };
        let v: serde_json::Value = serde_json::from_str(&webhook_body(&n)).unwrap();
        assert_eq!(v["event"], "agent_needs_you");
        assert_eq!(v["pane"], 2);
    }
}

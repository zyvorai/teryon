//! "Describe it, get a command": one-shot translation of a request into a single shell command.
use super::api::{Event, Model};
use super::tools::command_denied;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub command: String,
    pub explanation: String,
    /// "low", "medium" or "high", as rated by the model (never trusted on its own).
    pub risk: String,
    /// Set when the shared denylist refuses the command; such a command can be shown but never run.
    pub blocked: Option<&'static str>,
}
impl Suggestion {
    /// Whether the command may be run directly: rated low, one line, and not on the denylist.
    pub fn runnable(&self) -> bool {
        self.risk == "low" && self.blocked.is_none() && !self.command.contains('\n')
    }
}

pub const SYSTEM: &str = "You turn a request into ONE shell command for the user's terminal. \
Use the OS, shell and directory in the context; prefer portable, widely installed tools; never use sudo. \
Reply with ONLY a JSON object: {\"command\": \"...\", \"explanation\": \"one short sentence\", \"risk\": \"low|medium|high\"}. \
risk is low only for read-only or easily undone commands (listing, searching, printing); anything that deletes, \
overwrites, installs, kills processes or touches the network is medium or high. No markdown, no extra text.";

/// Pull the JSON object out of a reply that may be wrapped in fences or prose.
pub fn parse(text: &str) -> Result<Suggestion, String> {
    let start = text.find('{').ok_or("the model did not return a command")?;
    let end = text
        .rfind('}')
        .ok_or("the model did not return a command")?;
    let v: Value = serde_json::from_str(&text[start..=end])
        .map_err(|_| "the model's reply was not valid JSON".to_string())?;
    let command = v["command"].as_str().unwrap_or("").trim().to_string();
    if command.is_empty() {
        return Err("the model returned an empty command".into());
    }
    if command.len() > 2000 {
        return Err("the suggested command is too long".into());
    }
    let risk = match v["risk"].as_str().unwrap_or("").to_lowercase().as_str() {
        "low" => "low",
        "medium" => "medium",
        _ => "high",
    }
    .to_string();
    Ok(Suggestion {
        explanation: v["explanation"].as_str().unwrap_or("").trim().to_string(),
        blocked: command_denied(&command),
        command,
        risk,
    })
}

/// Ask `model` for a command. `context` describes host, OS, shell and directory (already redacted).
pub fn run(model: &dyn Model, context: &str, request: &str) -> Result<Suggestion, String> {
    let user = format!("Context:\n{context}\n\nRequest: {request}");
    let messages = [serde_json::json!({"role": "user", "content": user})];
    let mut text = String::new();
    let mut error = None;
    model
        .stream(
            SYSTEM,
            &serde_json::json!([]),
            &messages,
            &mut |e| match e {
                Event::Text(t) => text.push_str(&t),
                Event::Error(m) => error = Some(m),
                Event::ToolUse { .. } => {}
            },
        )
        .map_err(|e| error.take().unwrap_or(e))?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::api::Turn;

    #[test]
    fn replies_are_parsed_even_in_fences_and_unknown_risk_is_high() {
        let s = parse("```json\n{\"command\": \"ls -la\", \"explanation\": \"List files\", \"risk\": \"low\"}\n```").unwrap();
        assert_eq!((s.command.as_str(), s.risk.as_str()), ("ls -la", "low"));
        assert!(s.runnable());
        assert_eq!(
            parse("{\"command\": \"find . -size +100M\", \"risk\": \"???\"}")
                .unwrap()
                .risk,
            "high"
        );
        assert!(parse("sorry, I can't").is_err());
        assert!(parse("{\"command\": \"\"}").is_err());
        assert!(parse("{\"command\": ").is_err());
    }

    #[test]
    fn denylisted_or_multiline_commands_are_never_runnable() {
        let s = parse("{\"command\": \"sudo rm -rf /\", \"risk\": \"low\"}").unwrap();
        assert!(s.blocked.is_some() && !s.runnable());
        let s = parse("{\"command\": \"echo a\\necho b\", \"risk\": \"low\"}").unwrap();
        assert!(!s.runnable());
        let s = parse("{\"command\": \"rm -r build\", \"risk\": \"medium\"}").unwrap();
        assert!(!s.runnable(), "only low-risk commands run directly");
    }

    struct Fixed(&'static str);
    impl Model for Fixed {
        fn stream(
            &self,
            _: &str,
            tools: &Value,
            messages: &[Value],
            on: &mut dyn FnMut(Event),
        ) -> Result<Turn, String> {
            assert_eq!(tools, &serde_json::json!([]));
            assert!(messages[0]["content"]
                .as_str()
                .unwrap()
                .contains("Request: big files"));
            on(Event::Text(self.0.into()));
            Ok(Turn::default())
        }
    }
    #[test]
    fn run_sends_context_and_request_and_parses_the_reply() {
        let s = run(&Fixed("{\"command\": \"du -sh * | sort -h | tail -5\", \"explanation\": \"x\", \"risk\": \"low\"}"), "cwd: /tmp", "big files").unwrap();
        assert!(s.command.starts_with("du -sh"));
    }
}

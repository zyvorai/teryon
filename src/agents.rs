//! Recognizing AI coding agents (Claude Code, Codex, Gemini CLI, aider…) in a pane and reading their state.
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Claude,
    Codex,
    Gemini,
    Aider,
    Cursor,
    OpenCode,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini",
            Self::Aider => "aider",
            Self::Cursor => "Cursor",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Producing output or showing its "interrupt" hint.
    Working,
    /// Waiting for an approval or an answer.
    NeedsYou,
    /// Quiet at its own prompt.
    Idle,
    /// The agent exited and the shell is back.
    Done,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::NeedsYou => "needs you",
            Self::Idle => "idle",
            Self::Done => "done",
        }
    }
}

fn base(token: &str) -> String {
    let name = token.rsplit('/').next().unwrap_or(token).to_lowercase();
    name.trim_start_matches('-')
        .trim_end_matches(".js")
        .trim_end_matches(".mjs")
        .trim_end_matches(".py")
        .to_string()
}

fn kind_of(name: &str) -> Option<Kind> {
    Some(match name {
        "claude" | "claude-code" => Kind::Claude,
        "codex" => Kind::Codex,
        "gemini" | "gemini-cli" => Kind::Gemini,
        "aider" => Kind::Aider,
        "cursor-agent" => Kind::Cursor,
        "opencode" => Kind::OpenCode,
        _ => return None,
    })
}

/// The agent named by a foreground command line (`claude`, `node /…/codex.js`, `python -m aider`), if any.
/// Only the program and its first non-flag argument count, so `vim claude.md` is not an agent.
pub fn detect(command: &str) -> Option<Kind> {
    let mut tokens = command.split_whitespace();
    let first = tokens.next()?;
    if let Some(k) = kind_of(&base(first)) {
        return Some(k);
    }
    // Interpreters running an agent script or module.
    if matches!(
        base(first).as_str(),
        "node"
            | "bun"
            | "deno"
            | "python"
            | "python3"
            | "npx"
            | "uv"
            | "uvx"
            | "pipx"
            | "sh"
            | "bash"
            | "zsh"
    ) {
        for t in tokens.filter(|t| !t.starts_with('-')).take(2) {
            if let Some(k) = kind_of(&base(t)) {
                return Some(k);
            }
            if t.contains("@anthropic-ai/claude-code") {
                return Some(Kind::Claude);
            }
            if t.contains("@openai/codex") {
                return Some(Kind::Codex);
            }
            if t.contains("@google/gemini-cli") {
                return Some(Kind::Gemini);
            }
        }
    }
    None
}

/// Whether the foreground command is an interactive shell (so an agent has returned control).
pub fn is_shell(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or("");
    matches!(
        base(first).as_str(),
        "zsh" | "bash" | "fish" | "sh" | "dash" | "ksh" | "tcsh" | "nu" | "elvish"
    )
}

const ASKS: &[&str] = &[
    "do you want to",
    "(y/n)",
    "[y/n]",
    "[y/n/",
    "(yes/no)",
    "yes/no",
    "❯ 1. yes",
    "1. yes",
    "press enter to continue",
    "esc to cancel",
    "enter to confirm",
    "allow this",
    "would you like to proceed",
    "approve",
];
const BUSY: &[&str] = &[
    "esc to interrupt",
    "ctrl+c to interrupt",
    "ctrl-c to interrupt",
    "thinking…",
    "thinking...",
    "working (",
];

/// Read an agent's state from the visible rows and how long the screen has been quiet.
pub fn status(rows: &[String], quiet: Duration) -> Status {
    let tail: Vec<String> = rows
        .iter()
        .rev()
        .filter(|r| !r.trim().is_empty())
        .take(14)
        .map(|r| r.to_lowercase())
        .collect();
    let text = tail.join("\n");
    if ASKS.iter().any(|a| text.contains(a)) {
        return Status::NeedsYou;
    }
    if BUSY.iter().any(|b| text.contains(b)) || quiet < Duration::from_secs(2) {
        return Status::Working;
    }
    Status::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_are_recognized_by_their_command_line() {
        for (cmd, want) in [
            ("claude", Some(Kind::Claude)),
            ("/opt/homebrew/bin/claude --resume", Some(Kind::Claude)),
            (
                "node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js",
                Some(Kind::Claude),
            ),
            ("codex", Some(Kind::Codex)),
            (
                "node /x/node_modules/@openai/codex/bin/codex.js --full-auto",
                Some(Kind::Codex),
            ),
            ("gemini", Some(Kind::Gemini)),
            ("python3 -m aider --model x", Some(Kind::Aider)),
            ("aider", Some(Kind::Aider)),
            ("cursor-agent", Some(Kind::Cursor)),
            ("npx opencode", Some(Kind::OpenCode)),
            ("/bin/sh /tmp/x/claude", Some(Kind::Claude)),
            ("vim claude.md", None),
            ("cat codex.txt", None),
            ("-zsh", None),
            ("cargo test", None),
            ("", None),
        ] {
            assert_eq!(detect(cmd), want, "{cmd}");
        }
        assert!(is_shell("-zsh") && is_shell("/bin/bash -l") && !is_shell("claude"));
    }

    fn rows(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }
    const LONG: Duration = Duration::from_secs(30);
    const NOW: Duration = Duration::from_millis(100);

    #[test]
    fn approval_prompts_need_you_even_while_output_is_fresh() {
        let ask = rows(&[
            "Edit file src/main.rs",
            " Do you want to make this edit?",
            " ❯ 1. Yes",
            "   2. No",
            " Esc to cancel",
        ]);
        assert_eq!(status(&ask, NOW), Status::NeedsYou);
        assert_eq!(status(&rows(&["Overwrite? [y/N]"]), LONG), Status::NeedsYou);
        assert_eq!(status(&rows(&["Continue? (y/n)"]), LONG), Status::NeedsYou);
    }

    #[test]
    fn busy_hints_and_fresh_output_mean_working() {
        assert_eq!(
            status(&rows(&["✻ Thinking…", "(esc to interrupt)"]), LONG),
            Status::Working
        );
        assert_eq!(
            status(&rows(&["compiling crate 12/40"]), NOW),
            Status::Working
        );
    }

    #[test]
    fn a_quiet_prompt_is_idle() {
        assert_eq!(status(&rows(&["> ", "", ""]), LONG), Status::Idle);
        assert_eq!(status(&[], LONG), Status::Idle);
    }

    #[test]
    fn only_the_tail_of_the_screen_counts() {
        let mut old: Vec<&str> = vec!["Do you want to proceed? (y/n)"];
        old.extend(std::iter::repeat_n("some later output line", 20));
        assert_eq!(status(&rows(&old), LONG), Status::Idle);
    }
}

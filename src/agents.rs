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

impl Kind {
    pub const ALL: [Kind; 6] = [
        Self::Claude,
        Self::Codex,
        Self::Gemini,
        Self::Aider,
        Self::Cursor,
        Self::OpenCode,
    ];
    /// The command that starts this agent in a shell.
    pub fn launch(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Aider => "aider",
            Self::Cursor => "cursor-agent",
            Self::OpenCode => "opencode",
        }
    }
}

/// Interface text that is never the answer: hints, key legends and mode lines.
const CHROME: [&str; 8] = [
    "esc to",
    "? for shortcuts",
    "shift+tab",
    "ctrl+",
    "bypass permissions",
    "auto-accept",
    "tokens",
    "/help",
];

/// One line saying where the agent got to: the last line of real text on screen, without the
/// bullets and frames the agents draw around it. `None` when the screen holds nothing readable.
pub fn summary(rows: &[String]) -> Option<String> {
    const MAX: usize = 120;
    for row in rows.iter().rev() {
        let line = row
            .trim()
            .trim_matches(|c: char| "│┃|".contains(c))
            .trim_start_matches(|c: char| "⏺●•⎿✻✽·*-> ".contains(c) || c == '❯')
            .trim();
        let letters = line.chars().filter(|c| c.is_alphanumeric()).count();
        let lower = line.to_lowercase();
        if letters < 4 || CHROME.iter().any(|c| lower.contains(c)) {
            continue;
        }
        let mut out: String = line.chars().take(MAX).collect();
        if line.chars().count() > MAX {
            out.push('…');
        }
        return Some(out);
    }
    None
}

impl Kind {
    /// Keystrokes that answer an approval prompt with "yes": Enter takes the preselected choice in
    /// the menu-style agents, aider asks for a typed `y`.
    pub fn approve_keys(self) -> &'static [u8] {
        match self {
            Self::Aider => b"y\r",
            _ => b"\r",
        }
    }
    /// Keystrokes that decline: Esc cancels the menu-style prompts, aider takes `n`.
    pub fn deny_keys(self) -> &'static [u8] {
        match self {
            Self::Aider => b"n\r",
            _ => b"\x1b",
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
    fn launch_commands_are_detected_as_their_own_agent() {
        for k in Kind::ALL {
            assert_eq!(detect(k.launch()), Some(k));
        }
    }
    #[test]
    fn the_summary_is_the_last_real_line() {
        let rows: Vec<String> = [
            "⏺ Fixed the failing test in src/lib.rs",
            "",
            "╭──────────────╮",
            "│ > Try \"fix lint\" │",
            "╰──────────────╯",
            "  ? for shortcuts",
            "",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(summary(&rows).as_deref(), Some("Try \"fix lint\""));
        let rows = vec![
            "⏺ All done: 12 tests pass".to_string(),
            "  esc to interrupt".into(),
        ];
        assert_eq!(summary(&rows).as_deref(), Some("All done: 12 tests pass"));
        assert_eq!(summary(&["───".to_string(), "".into()]), None);
        let long = vec!["x".repeat(300)];
        assert_eq!(summary(&long).unwrap().chars().count(), 121);
    }
    #[test]
    fn approval_keys_fit_each_agents_prompt() {
        assert_eq!(Kind::Claude.approve_keys(), b"\r");
        assert_eq!(Kind::Claude.deny_keys(), b"\x1b");
        assert_eq!(Kind::Aider.approve_keys(), b"y\r");
        assert_eq!(Kind::Aider.deny_keys(), b"n\r");
    }
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

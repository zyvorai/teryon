//! Watching a pane for things worth telling someone about: an agent that needs you or finished,
//! and a long command that ended. Pure state, so the daemon can run it without a client attached.
use crate::agents::{self, Kind, Status};
use crate::protocol::Mark;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    AgentNeedsYou(Kind),
    AgentDone(Kind),
    CommandDone {
        program: String,
        exit: Option<i32>,
        secs: u64,
    },
}

pub struct Tracker {
    kind: Option<Kind>,
    seen: bool,
    rev: u64,
    changed: Instant,
    status: Option<Status>,
    /// `at` of the newest finished command already looked at; `None` until the first look.
    announced: Option<u64>,
}
impl Tracker {
    pub fn new(now: Instant) -> Self {
        Self {
            kind: None,
            seen: false,
            rev: 0,
            changed: now,
            status: None,
            announced: None,
        }
    }
    /// Feed one observation of a pane; returns what changed since the last one. `rows` is only
    /// called when an agent's screen has to be read.
    pub fn observe(
        &mut self,
        now: Instant,
        rev: u64,
        fg: &str,
        exited: bool,
        rows: impl FnOnce() -> Vec<String>,
        marks: &[Mark],
        long_ms: u64,
    ) -> Vec<Event> {
        let mut out = Vec::new();
        if rev != self.rev {
            self.rev = rev;
            self.changed = now;
        }
        // Commands: look at the newest finished one, but never announce what was already there.
        if let Some(d) = marks.iter().rev().find(|m| m.kind == 'D') {
            if self.announced.is_some_and(|a| d.at > a) && d.ms.is_some_and(|ms| ms >= long_ms) {
                let program = marks
                    .iter()
                    .rev()
                    .find(|m| m.kind == 'C' && m.line <= d.line)
                    .and_then(|c| c.cmd.as_deref())
                    .and_then(|c| c.split_whitespace().next())
                    .and_then(|p| p.rsplit('/').next())
                    .unwrap_or("command")
                    .to_string();
                out.push(Event::CommandDone {
                    program,
                    exit: d.exit,
                    secs: d.ms.unwrap_or(0) / 1000,
                });
            }
            self.announced = Some(self.announced.map_or(d.at, |a| a.max(d.at)));
        } else if self.announced.is_none() {
            self.announced = Some(0);
        }
        // Agents.
        let found = agents::detect(fg);
        if found.is_none() && !self.seen {
            self.status = None;
            return out;
        }
        if let Some(k) = found {
            self.kind = Some(k);
            self.seen = true;
        }
        let next = if exited || (found.is_none() && agents::is_shell(fg)) {
            Status::Done
        } else if found.is_none() {
            Status::Working
        } else {
            agents::status(&rows(), now.duration_since(self.changed))
        };
        let prev = self.status;
        if prev != Some(next) {
            self.status = Some(next);
            if let (Some(_), Some(kind)) = (prev, self.kind) {
                match next {
                    Status::NeedsYou => out.push(Event::AgentNeedsYou(kind)),
                    Status::Done => out.push(Event::AgentDone(kind)),
                    _ => {}
                }
            }
        }
        out
    }
}

/// How often the daemon looks at its panes.
pub const TICK: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::*;
    fn mark(kind: char, line: u64, at: u64, ms: Option<u64>, cmd: Option<&str>) -> Mark {
        Mark {
            kind,
            line,
            exit: Some(0),
            ms,
            at,
            cmd: cmd.map(str::to_string),
        }
    }
    fn screen(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn an_agent_that_starts_asking_is_reported_once() {
        let t0 = Instant::now();
        let mut tr = Tracker::new(t0);
        let work = || screen(&["esc to interrupt"]);
        let ask = || screen(&["Do you want to proceed?", "1. Yes"]);
        assert!(tr.observe(t0, 1, "claude", false, work, &[], 1000).is_empty());
        let later = t0 + Duration::from_secs(5);
        assert_eq!(
            tr.observe(later, 2, "claude", false, ask, &[], 1000),
            vec![Event::AgentNeedsYou(Kind::Claude)]
        );
        assert!(tr.observe(later, 2, "claude", false, ask, &[], 1000).is_empty());
        assert_eq!(
            tr.observe(later, 3, "-zsh", false, Vec::new, &[], 1000),
            vec![Event::AgentDone(Kind::Claude)]
        );
    }
    #[test]
    fn plain_shells_are_not_agents() {
        let t0 = Instant::now();
        let mut tr = Tracker::new(t0);
        assert!(tr.observe(t0, 1, "-zsh", false, Vec::new, &[], 1000).is_empty());
        assert!(tr.observe(t0, 2, "cargo test", false, Vec::new, &[], 1000).is_empty());
    }
    #[test]
    fn long_commands_are_reported_after_the_first_look_with_only_the_program_name() {
        let t0 = Instant::now();
        let mut tr = Tracker::new(t0);
        let old = vec![
            mark('C', 1, 10, None, Some("make")),
            mark('D', 3, 20, Some(99_000), None),
        ];
        assert!(
            tr.observe(t0, 1, "-zsh", false, Vec::new, &old, 30_000).is_empty(),
            "what finished before the first look is not news"
        );
        let mut now = old.clone();
        now.push(mark('C', 5, 30, None, Some("/usr/bin/cargo build --token=secret")));
        now.push(mark('D', 9, 95, Some(65_000), None));
        assert_eq!(
            tr.observe(t0, 2, "-zsh", false, Vec::new, &now, 30_000),
            vec![Event::CommandDone {
                program: "cargo".into(),
                exit: Some(0),
                secs: 65
            }]
        );
        assert!(tr.observe(t0, 3, "-zsh", false, Vec::new, &now, 30_000).is_empty());
        now.push(mark('D', 12, 99, Some(2_000), None));
        assert!(
            tr.observe(t0, 4, "-zsh", false, Vec::new, &now, 30_000).is_empty(),
            "a short command is not worth a notification"
        );
    }
}

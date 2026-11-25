// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Waiting on a pane (`teryon wait`, the MCP `wait_for` tool) and following its output
//! (`teryon capture --follow`). Pure state fed with captures, so it is the same over SSH and in tests.
use regex::Regex;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// One look at a pane.
#[derive(Clone, Copy)]
pub struct Look<'a> {
    /// The daemon's counter of screen changes.
    pub revision: u64,
    pub exited: Option<u32>,
    /// Retained text, one line per row (`Capture`).
    pub text: &'a str,
    /// Absolute number of the first line of `text`.
    pub first: u64,
}

fn numbered(text: &str, first: u64) -> impl DoubleEndedIterator<Item = (u64, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .into_iter()
        .enumerate()
        .map(move |(i, l)| (first.saturating_add(i as u64), l))
}

/// What to wait for; the first that happens ends the wait.
#[derive(Default)]
pub struct Until {
    /// A line matching this appears.
    pub pattern: Option<Regex>,
    /// The pane prints nothing for this long.
    pub idle: Option<Duration>,
    /// The pattern only counts in lines that are new or changed since the first look, so text that was
    /// already there (or the typed command's earlier runs) does not end the wait.
    pub only_new: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Match(String),
    Idle,
    /// The pane's program ended (after one more look, so its last output was checked).
    Exited(u32),
}

pub struct Waiter {
    until: Until,
    before: Option<BTreeMap<u64, String>>,
    last: Option<(u64, u64, String)>,
    quiet_since: Instant,
    exit_seen: bool,
}

impl Waiter {
    pub fn new(until: Until, now: Instant) -> Self {
        Self {
            until,
            before: None,
            last: None,
            quiet_since: now,
            exit_seen: false,
        }
    }

    /// An exit was seen and the next look will report it; do not give up before that.
    pub fn settling(&self) -> bool {
        self.exit_seen
    }

    pub fn look(&mut self, now: Instant, look: Look<'_>) -> Option<Outcome> {
        if self.until.only_new && self.before.is_none() {
            self.before = Some(
                numbered(look.text, look.first)
                    .map(|(n, l)| (n, l.to_string()))
                    .collect(),
            );
        }
        let changed = self.last.as_ref().is_none_or(|(rev, first, text)| {
            *rev != look.revision || *first != look.first || text != look.text
        });
        if changed {
            self.quiet_since = now;
            self.last = Some((look.revision, look.first, look.text.to_string()));
        }
        if let Some(re) = &self.until.pattern {
            let before = self.before.as_ref();
            // Lines that scrolled out of the history before the first look are not new.
            let floor = before.and_then(|b| b.keys().next().copied()).unwrap_or(0);
            let hit = numbered(look.text, look.first).rev().find(|(n, l)| {
                let fresh =
                    before.is_none_or(|b| *n >= floor && b.get(n).is_none_or(|old| old != l));
                fresh && re.is_match(l)
            });
            if let Some((_, line)) = hit {
                return Some(Outcome::Match(line.to_string()));
            }
        }
        if let Some(code) = look.exited {
            if self.exit_seen {
                return Some(Outcome::Exited(code));
            }
            self.exit_seen = true;
            return None;
        }
        match self.until.idle {
            Some(idle) if now.saturating_duration_since(self.quiet_since) >= idle => {
                Some(Outcome::Idle)
            }
            _ => None,
        }
    }
}

/// Turns repeated captures into a stream of new lines.
#[derive(Default)]
pub struct Follower {
    printed: BTreeMap<u64, String>,
}

impl Follower {
    /// Treat everything but the newest `keep` lines as already shown.
    pub fn skip_all_but(&mut self, text: &str, first: u64, keep: usize) {
        let total = text.lines().count();
        for (n, line) in numbered(text, first).take(total.saturating_sub(keep)) {
            self.printed.insert(n, line.to_string());
        }
    }

    /// The lines that are new or rewritten since the last call. The bottom line is usually still being
    /// written (a prompt, a progress bar), so it is only returned with `include_last`.
    pub fn take(&mut self, text: &str, first: u64, include_last: bool) -> Vec<String> {
        let total = text.lines().count();
        let end = first.saturating_add(total as u64);
        // Forget lines that left the history or were cleared away.
        self.printed.retain(|n, _| *n >= first && *n < end);
        let mut out = Vec::new();
        for (i, (n, line)) in numbered(text, first).enumerate() {
            if i + 1 == total && !include_last {
                break;
            }
            if self.printed.get(&n).is_some_and(|old| old == line) {
                continue;
            }
            let was_there = self.printed.insert(n, line.to_string()).is_some();
            // A line that was only erased is not output.
            if !(was_there && line.is_empty()) {
                out.push(line.to_string());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(revision: u64, text: &str) -> Look<'_> {
        Look {
            revision,
            exited: None,
            text,
            first: 10,
        }
    }
    fn until(pattern: Option<&str>, idle: Option<u64>, only_new: bool) -> Until {
        Until {
            pattern: pattern.map(|p| Regex::new(p).unwrap()),
            idle: idle.map(Duration::from_secs),
            only_new,
        }
    }

    #[test]
    fn a_pattern_matches_the_newest_line_including_what_is_already_there() {
        let t = Instant::now();
        let mut w = Waiter::new(until(Some("ready"), None, false), t);
        assert_eq!(
            w.look(t, look(1, "ready 1\nready 2\nx")),
            Some(Outcome::Match("ready 2".into()))
        );
    }

    #[test]
    fn only_new_ignores_old_text_but_sees_new_and_rewritten_lines() {
        let t = Instant::now();
        let mut w = Waiter::new(until(Some("done"), None, true), t);
        assert_eq!(w.look(t, look(1, "done\n$ ")), None);
        assert_eq!(w.look(t, look(2, "done\n$ make\nworking")), None);
        assert_eq!(
            w.look(t, look(3, "done\n$ make\nworking\nall done")),
            Some(Outcome::Match("all done".into()))
        );
        // A line rewritten in place (a progress bar, a cleared screen) counts as new.
        let mut w = Waiter::new(until(Some("done"), None, true), t);
        assert_eq!(w.look(t, look(1, "50%")), None);
        assert_eq!(
            w.look(t, look(2, "done")),
            Some(Outcome::Match("done".into()))
        );
        // Lines above the first retained one at the start never count.
        let mut w = Waiter::new(until(Some("done"), None, true), t);
        assert_eq!(w.look(t, look(1, "x")), None);
        let older = Look {
            first: 9,
            ..look(2, "done\nx")
        };
        assert_eq!(w.look(t, older), None);
    }

    #[test]
    fn idle_needs_a_quiet_stretch_that_any_output_restarts() {
        let t = Instant::now();
        let at = |s| t + Duration::from_secs(s);
        let mut w = Waiter::new(until(None, Some(5), false), t);
        assert_eq!(w.look(at(0), look(1, "a")), None);
        assert_eq!(w.look(at(4), look(1, "a")), None);
        // The same text printed again still counts as output: the revision moved.
        assert_eq!(w.look(at(5), look(2, "a")), None);
        assert_eq!(w.look(at(9), look(2, "a")), None);
        assert_eq!(w.look(at(10), look(2, "a")), Some(Outcome::Idle));
    }

    #[test]
    fn an_exit_is_reported_after_one_more_look_at_the_final_output() {
        let t = Instant::now();
        let gone = |text| Look {
            exited: Some(3),
            ..look(2, text)
        };
        let mut w = Waiter::new(until(Some("bye"), Some(1), false), t);
        assert_eq!(w.look(t, gone("")), None);
        assert!(w.settling());
        assert_eq!(w.look(t, gone("bye")), Some(Outcome::Match("bye".into())));
        let mut w = Waiter::new(until(Some("never"), None, false), t);
        assert_eq!(w.look(t, gone("x")), None);
        assert_eq!(w.look(t, gone("x")), Some(Outcome::Exited(3)));
    }

    #[test]
    fn following_prints_each_line_once_and_holds_the_unfinished_one() {
        let mut f = Follower::default();
        assert_eq!(f.take("a\nb\n$ ", 0, false), ["a", "b"]);
        assert_eq!(f.take("a\nb\n$ ", 0, false), Vec::<String>::new());
        assert_eq!(f.take("a\nb\n$ ", 0, true), ["$ "]);
        // The prompt line grew, then more output followed and the history scrolled.
        assert_eq!(f.take("b\n$ ls\nfile\n$ ", 1, false), ["$ ls", "file"]);
        // Cleared screen: new text at old positions is shown, erased rows are not.
        assert_eq!(f.take("b\n\nfresh\n$ ", 1, false), ["fresh"]);
        // Rows cleared off the end and written again are new output.
        assert_eq!(f.take("b", 1, true), Vec::<String>::new());
        assert_eq!(f.take("b\n\nfresh", 1, true), ["", "fresh"]);
    }

    #[test]
    fn tail_skips_all_but_the_newest_lines() {
        let mut f = Follower::default();
        f.skip_all_but("1\n2\n3\n4", 0, 2);
        assert_eq!(f.take("1\n2\n3\n4", 0, true), ["3", "4"]);
        let mut f = Follower::default();
        f.skip_all_but("1\n2", 0, 0);
        assert_eq!(f.take("1\n2\n3", 0, true), ["3"]);
    }
}

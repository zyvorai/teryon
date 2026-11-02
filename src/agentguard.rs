//! A time budget for AI agents: notice an agent that has been working for too long without a pause
//! (`teryon agent-guard`), so a run that went off the rails does not burn tokens unnoticed.
use crate::agents::{Kind, Status};
use crate::mcp::Backend;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Where an agent runs: host, workspace, pane.
pub type Key = (String, String, u64);

/// What a stretch of work looks like to the guard.
#[derive(Debug, PartialEq)]
pub struct Over {
    pub key: Key,
    pub worked: Duration,
}

struct Run {
    since: Instant,
    /// The budget was already reported for this stretch of work.
    reported: bool,
}

/// Follows how long each agent has been working without stopping.
pub struct Tracker {
    limit: Duration,
    runs: HashMap<Key, Run>,
}

impl Tracker {
    pub fn new(limit: Duration) -> Self {
        Self {
            limit,
            runs: HashMap::new(),
        }
    }

    /// Feed one look at every agent pane. Returns the agents that have just gone over the limit: each
    /// stretch of work is reported once, and a pause (any status but working, or the pane vanishing)
    /// starts a fresh stretch.
    pub fn observe(&mut self, now: Instant, seen: &[(Key, Status)]) -> Vec<Over> {
        self.runs
            .retain(|k, _| seen.iter().any(|(s, st)| s == k && *st == Status::Working));
        let mut over = Vec::new();
        for (key, status) in seen {
            if *status != Status::Working {
                continue;
            }
            let run = self.runs.entry(key.clone()).or_insert(Run {
                since: now,
                reported: false,
            });
            let worked = now.saturating_duration_since(run.since);
            if worked >= self.limit && !run.reported {
                run.reported = true;
                over.push(Over {
                    key: key.clone(),
                    worked,
                });
            }
        }
        over
    }
}

/// Look at every agent pane on `hosts`: where it runs, which agent it is and what it is doing now.
/// A host that cannot be reached is skipped (and named in the second part).
pub fn look<B: Backend>(
    backend: &mut B,
    hosts: &[String],
) -> (Vec<(Key, Kind, Status)>, Vec<String>) {
    let (mut rows, mut down) = (Vec::new(), Vec::new());
    for host in hosts {
        let workspaces = match backend.workspaces(host) {
            Ok(w) => w,
            Err(e) => {
                down.push(format!("{host}: unreachable ({e:#})"));
                continue;
            }
        };
        for w in workspaces {
            for (pane, kind, status) in
                crate::mcp::agent_rows(backend, host, &w.name).unwrap_or_default()
            {
                rows.push(((host.clone(), w.name.clone(), pane), kind, status));
            }
        }
    }
    (rows, down)
}

/// "90m" style spans: `30s`, `45m`, `2h`, or bare minutes.
pub fn parse_span(text: &str) -> Option<Duration> {
    let t = text.trim();
    let (digits, unit) = match t.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => t.split_at(i),
        None => (t, "m"),
    };
    let n: u64 = digits.parse().ok().filter(|n| *n > 0)?;
    let secs = match unit {
        "s" => n,
        "m" => n.checked_mul(60)?,
        "h" => n.checked_mul(3600)?,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

pub fn span(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=119 => format!("{s}s"),
        120..=7199 => format!("{} min", s / 60),
        _ => format!("{}h{:02}m", s / 3600, s / 60 % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::protocol::{PaneView, WorkspaceInfo};
    use anyhow::Result;

    struct Fake;
    impl Backend for Fake {
        fn workspaces(&mut self, host: &str) -> Result<Vec<WorkspaceInfo>> {
            if host == "gone" {
                anyhow::bail!("no route");
            }
            Ok(vec![WorkspaceInfo {
                name: "demo".into(),
                panes: 2,
                controlled: false,
            }])
        }
        fn panes(&mut self, _: &str, _: &str) -> Result<Vec<PaneView>> {
            Ok(vec![
                PaneView {
                    id: 1,
                    fg: Some("claude".into()),
                    ..Default::default()
                },
                PaneView {
                    id: 2,
                    fg: Some("vim".into()),
                    ..Default::default()
                },
            ])
        }
        fn capture(&mut self, _: &str, _: &str, _: u64) -> Result<(String, u64)> {
            Ok(("✻ Thinking…\n(esc to interrupt)".into(), 0))
        }
        fn new_pane(&mut self, _: &str, _: &str, _: Vec<String>, _: Option<String>) -> Result<u64> {
            Ok(0)
        }
        fn send(&mut self, _: &str, _: &str, _: u64, _: &str, _: bool) -> Result<()> {
            Ok(())
        }
        fn close(&mut self, _: &str, _: &str, _: u64) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn looking_finds_only_agent_panes_and_names_hosts_that_are_down() {
        let hosts = ["local".to_string(), "gone".to_string()];
        let (rows, down) = look(&mut Fake, &hosts);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, key(1));
        assert_eq!(rows[0].1, Kind::Claude);
        assert_eq!(rows[0].2, Status::Working);
        assert_eq!(down.len(), 1);
        assert!(down[0].starts_with("gone: unreachable"));
    }

    fn key(pane: u64) -> Key {
        ("local".into(), "demo".into(), pane)
    }

    #[test]
    fn a_long_stretch_of_work_is_reported_once() {
        let t0 = Instant::now();
        let mut t = Tracker::new(Duration::from_secs(600));
        let w = |p| vec![(key(p), Status::Working)];
        assert!(t.observe(t0, &w(1)).is_empty());
        assert!(t.observe(t0 + Duration::from_secs(599), &w(1)).is_empty());
        let over = t.observe(t0 + Duration::from_secs(600), &w(1));
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].key, key(1));
        assert!(t.observe(t0 + Duration::from_secs(900), &w(1)).is_empty());
    }

    #[test]
    fn a_pause_starts_a_new_stretch() {
        let t0 = Instant::now();
        let mut t = Tracker::new(Duration::from_secs(600));
        t.observe(t0, &[(key(1), Status::Working)]);
        t.observe(t0 + Duration::from_secs(500), &[(key(1), Status::NeedsYou)]);
        let back = t0 + Duration::from_secs(550);
        assert!(t.observe(back, &[(key(1), Status::Working)]).is_empty());
        assert!(t
            .observe(
                back + Duration::from_secs(599),
                &[(key(1), Status::Working)]
            )
            .is_empty());
        assert_eq!(
            t.observe(
                back + Duration::from_secs(600),
                &[(key(1), Status::Working)]
            )
            .len(),
            1
        );
    }

    #[test]
    fn agents_are_timed_separately_and_a_closed_pane_is_forgotten() {
        let t0 = Instant::now();
        let mut t = Tracker::new(Duration::from_secs(60));
        t.observe(t0, &[(key(1), Status::Working)]);
        let both = [(key(1), Status::Working), (key(2), Status::Working)];
        let over = t.observe(t0 + Duration::from_secs(60), &both);
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].key, key(1));
        t.observe(t0 + Duration::from_secs(70), &[]);
        let again = t.observe(t0 + Duration::from_secs(80), &[(key(1), Status::Working)]);
        assert!(again.is_empty());
    }

    #[test]
    fn spans_parse_and_print() {
        assert_eq!(parse_span("45m"), Some(Duration::from_secs(2700)));
        assert_eq!(parse_span("30"), Some(Duration::from_secs(1800)));
        assert_eq!(parse_span("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_span("90s"), Some(Duration::from_secs(90)));
        assert_eq!(parse_span("0m"), None);
        assert_eq!(parse_span("soon"), None);
        assert_eq!(span(Duration::from_secs(7320)), "2h02m");
        assert_eq!(span(Duration::from_secs(600)), "10 min");
    }
}

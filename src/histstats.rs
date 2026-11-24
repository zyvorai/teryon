// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `teryon hist stats` (an overview of what you run: most-used commands, success rate, busiest hours
//! and days) and `teryon hist prune` (drop old or repeated entries from this machine's history).
use crate::history::Entry;
use std::collections::HashMap;

/// Exit status of a command interrupted with Ctrl-C; not counted as a failure.
const INTERRUPTED: i32 = 130;
/// Tools whose first argument names the real command (`git commit`, `cargo test`).
const SUBCOMMAND_TOOLS: &[&str] = &[
    "git",
    "cargo",
    "npm",
    "pnpm",
    "yarn",
    "docker",
    "kubectl",
    "make",
    "go",
    "brew",
    "systemctl",
    "teryon",
    "ssh",
    "apt",
    "pip",
];

#[derive(Debug, Default, PartialEq)]
pub struct Stats {
    pub total: usize,
    pub failed: usize,
    pub unique: usize,
    pub avg_ms: u64,
    /// Most-used commands (`git status`, `ls`), most used first.
    pub top: Vec<(String, usize)>,
    /// Commands per hour of the day in the given time zone.
    pub by_hour: [usize; 24],
    /// Commands per weekday, Sunday first.
    pub by_weekday: [usize; 7],
}

/// The name a command is counted under: its first word, plus the next one for tools like `git`.
/// Leading `VAR=value` words and `sudo` are skipped.
pub fn key_of(cmd: &str) -> Option<String> {
    let mut words = cmd
        .split_whitespace()
        .skip_while(|w| w.contains('=') && !w.starts_with('=') || *w == "sudo");
    let first = words.next()?;
    let first = first
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(first);
    if SUBCOMMAND_TOOLS.contains(&first) {
        if let Some(second) = words.next().filter(|w| !w.starts_with('-')) {
            return Some(format!("{first} {second}"));
        }
    }
    Some(first.to_string())
}

/// Statistics of the entries since `since` (epoch seconds); `host` keeps one machine (`Some("")` is
/// this one). `tz` is the offset from UTC in seconds, for the hour and weekday.
pub fn stats(entries: &[Entry], since: u64, host: Option<&str>, top: usize, tz: i64) -> Stats {
    let mut s = Stats::default();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut total_ms = 0u64;
    let mut seen = std::collections::HashSet::new();
    for e in entries.iter().filter(|e| e.at >= since) {
        if host.is_some_and(|h| e.host.as_deref().unwrap_or("") != h) {
            continue;
        }
        let cmd = e.cmd.trim();
        let Some(key) = key_of(cmd) else { continue };
        s.total += 1;
        total_ms += e.ms;
        if e.exit != 0 && e.exit != INTERRUPTED {
            s.failed += 1;
        }
        seen.insert(cmd.to_string());
        *counts.entry(key).or_default() += 1;
        let local = e.at as i64 + tz;
        s.by_hour[(local.rem_euclid(86_400) / 3_600) as usize] += 1;
        s.by_weekday[((local.div_euclid(86_400) + 4).rem_euclid(7)) as usize] += 1;
    }
    s.unique = seen.len();
    s.avg_ms = total_ms.checked_div(s.total as u64).unwrap_or(0);
    let mut top_list: Vec<_> = counts.into_iter().collect();
    top_list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top_list.truncate(top);
    s.top = top_list;
    s
}

fn bar(n: usize, max: usize, width: usize) -> String {
    if max == 0 {
        return String::new();
    }
    "#".repeat((n * width).div_ceil(max).min(width) * usize::from(n > 0))
}

pub fn render(s: &Stats, days: u64) -> String {
    if s.total == 0 {
        return format!("No commands recorded in the last {days} days.\n");
    }
    let pct = 100 - s.failed * 100 / s.total;
    let mut out = format!(
        "Last {days} days: {} commands ({} different), {pct}% succeeded, {} failed, {} on average\n",
        s.total,
        s.unique,
        s.failed,
        crate::histtools::duration(s.avg_ms)
    );
    out.push_str("\nMost used\n");
    let max = s.top.first().map_or(0, |t| t.1);
    for (name, n) in &s.top {
        out.push_str(&format!("  {n:>5}  {:<20} {}\n", name, bar(*n, max, 20)));
    }
    out.push_str("\nBy hour\n");
    let max = s.by_hour.iter().copied().max().unwrap_or(0);
    for (h, n) in s.by_hour.iter().enumerate().filter(|(_, n)| **n > 0) {
        out.push_str(&format!("  {h:02}:00 {n:>5} {}\n", bar(*n, max, 30)));
    }
    out.push_str("\nBy weekday\n");
    let max = s.by_weekday.iter().copied().max().unwrap_or(0);
    for (d, n) in ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .iter()
        .zip(s.by_weekday)
    {
        out.push_str(&format!("  {d} {n:>5} {}\n", bar(n, max, 30)));
    }
    out
}

pub fn to_json(s: &Stats, days: u64) -> serde_json::Value {
    serde_json::json!({
        "days": days,
        "total": s.total,
        "failed": s.failed,
        "unique": s.unique,
        "avg_ms": s.avg_ms,
        "top": s.top.iter().map(|(c, n)| serde_json::json!({"command": c, "count": n})).collect::<Vec<_>>(),
        "by_hour": s.by_hour,
        "by_weekday": s.by_weekday,
    })
}

/// `entries` (oldest first) without those older than `before` (epoch seconds, 0 keeps all) and, with
/// `dedupe`, without repeats of the same command in the same directory (the newest is kept). Returns
/// the kept entries and how many were dropped.
pub fn prune(entries: Vec<Entry>, before: u64, dedupe: bool) -> (Vec<Entry>, usize) {
    let total = entries.len();
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<Entry> = entries
        .into_iter()
        .rev()
        .filter(|e| e.at >= before)
        .filter(|e| !dedupe || seen.insert((e.cmd.trim().to_string(), e.cwd.clone())))
        .collect();
    kept.reverse();
    let dropped = total - kept.len();
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(cmd: &str, exit: i32, ms: u64, at: u64) -> Entry {
        Entry {
            cmd: cmd.into(),
            cwd: "/w".into(),
            exit,
            ms,
            at,
            branch: None,
            host: None,
        }
    }

    #[test]
    fn keys_use_the_subcommand_for_known_tools_and_skip_env_and_sudo() {
        assert_eq!(key_of("git commit -m x").as_deref(), Some("git commit"));
        assert_eq!(key_of("git -C . status").as_deref(), Some("git"));
        assert_eq!(key_of("FOO=1 sudo /usr/bin/ls -l").as_deref(), Some("ls"));
        assert_eq!(key_of("cargo").as_deref(), Some("cargo"));
        assert_eq!(key_of("   "), None);
    }

    #[test]
    fn stats_count_failures_but_not_ctrl_c_and_bucket_by_local_time() {
        // 2024-01-01 is a Monday; 00:30 UTC is 02:30 at +2h.
        let t = 1_704_069_000;
        let list = vec![
            e("git status", 0, 100, t),
            e("git status", 1, 300, t + 10),
            e("ls", 130, 0, t + 20),
        ];
        let s = stats(&list, 0, None, 5, 7_200);
        assert_eq!((s.total, s.failed, s.unique, s.avg_ms), (3, 1, 2, 133));
        assert_eq!(s.top[0], ("git status".to_string(), 2));
        assert_eq!(s.by_hour[2], 3);
        assert_eq!(s.by_weekday[1], 3);
        assert_eq!(stats(&list, t + 15, None, 5, 0).total, 1);
        assert_eq!(stats(&list, 0, Some("mini"), 5, 0).total, 0);
        let text = render(&s, 7);
        assert!(
            text.contains("3 commands (2 different), 67% succeeded"),
            "{text}"
        );
        assert_eq!(to_json(&s, 7)["top"][0]["count"], 2);
    }

    #[test]
    fn prune_drops_old_entries_and_keeps_the_newest_repeat() {
        let list = vec![
            e("a", 0, 0, 10),
            e("b", 0, 0, 20),
            e("a", 0, 0, 30),
            e("a", 0, 0, 40),
        ];
        let (kept, n) = prune(list.clone(), 15, false);
        assert_eq!((kept.len(), n), (3, 1));
        let (kept, n) = prune(list, 0, true);
        assert_eq!(n, 2);
        assert_eq!(kept.iter().map(|e| e.at).collect::<Vec<_>>(), [20, 40]);
    }
}

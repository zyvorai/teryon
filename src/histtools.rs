//! Things worth doing with the recorded command history: a per-project report of what is slow and
//! what keeps failing (`teryon hist report`), and finding a past command to run again in the place it
//! originally ran (`teryon hist replay`).
use crate::history::Entry;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

/// What a Ctrl-C exits with; stopping a command yourself is not a failure worth reporting.
const INTERRUPTED: i32 = 130;

#[derive(Debug, PartialEq)]
pub struct Project {
    pub name: String,
    pub runs: usize,
    pub failures: usize,
    pub total_ms: u64,
    /// Commands by their slowest run, slowest first.
    pub slowest: Vec<(String, u64)>,
    /// Commands by how often they failed, most first.
    pub failing: Vec<(String, usize)>,
}

/// The directory a command belongs to: the nearest ancestor of `cwd` that holds a `.git`, else `cwd`.
/// Entries pulled from other machines name directories that do not exist here, so they stay as they are.
pub fn project_of(cwd: &str) -> String {
    let mut dir = Path::new(cwd);
    loop {
        if dir.join(".git").exists() {
            return dir.display().to_string();
        }
        match dir.parent() {
            Some(p) if p != Path::new("/") && !p.as_os_str().is_empty() => dir = p,
            _ => return cwd.to_string(),
        }
    }
}

fn tilde(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => path.to_string(),
    }
}

/// Group the entries since `since` (epoch seconds) by project, busiest project first. `host` keeps
/// only commands from that machine (`Some("")` means this one).
pub fn report(
    entries: &[Entry],
    since: u64,
    host: Option<&str>,
    top: usize,
    home: &str,
) -> Vec<Project> {
    struct Acc {
        runs: usize,
        failures: usize,
        total_ms: u64,
        slow: HashMap<String, u64>,
        fail: HashMap<String, usize>,
    }
    let mut by: HashMap<String, Acc> = HashMap::new();
    for e in entries.iter().filter(|e| e.at >= since) {
        if let Some(h) = host {
            if e.host.as_deref().unwrap_or("") != h {
                continue;
            }
        }
        let a = by.entry(project_of(&e.cwd)).or_insert_with(|| Acc {
            runs: 0,
            failures: 0,
            total_ms: 0,
            slow: HashMap::new(),
            fail: HashMap::new(),
        });
        let cmd = e.cmd.trim();
        if cmd.is_empty() || cmd.contains('\n') {
            continue;
        }
        a.runs += 1;
        a.total_ms += e.ms;
        if e.ms > 0 {
            let s = a.slow.entry(cmd.to_string()).or_default();
            *s = (*s).max(e.ms);
        }
        if e.exit != 0 && e.exit != INTERRUPTED {
            a.failures += 1;
            *a.fail.entry(cmd.to_string()).or_default() += 1;
        }
    }
    let mut out: Vec<Project> = by
        .into_iter()
        .filter(|(_, a)| a.runs > 0)
        .map(|(name, a)| {
            let mut slowest: Vec<_> = a.slow.into_iter().collect();
            slowest.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
            slowest.truncate(top);
            let mut failing: Vec<_> = a.fail.into_iter().collect();
            failing.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
            failing.truncate(top);
            Project {
                name: tilde(&name, home),
                runs: a.runs,
                failures: a.failures,
                total_ms: a.total_ms,
                slowest,
                failing,
            }
        })
        .collect();
    out.sort_by(|a, b| b.runs.cmp(&a.runs).then(a.name.cmp(&b.name)));
    out
}

pub fn duration(ms: u64) -> String {
    match ms {
        0..=999 => format!("{ms}ms"),
        1_000..=59_999 => format!("{:.1}s", ms as f64 / 1000.0),
        60_000..=3_599_999 => format!("{}m{:02}s", ms / 60_000, ms / 1000 % 60),
        _ => format!("{}h{:02}m", ms / 3_600_000, ms / 60_000 % 60),
    }
}

fn clip(cmd: &str) -> String {
    let mut s: String = cmd.chars().take(72).collect();
    if cmd.chars().count() > 72 {
        s.push('…');
    }
    s
}

pub fn render(projects: &[Project], days: u64) -> String {
    if projects.is_empty() {
        return format!("No commands recorded in the last {days} day(s).\n");
    }
    let mut out = String::new();
    for p in projects {
        let pct = p.failures * 100 / p.runs;
        let _ = writeln!(
            out,
            "{}  {} commands, {} failed ({pct}%), {} total",
            p.name,
            p.runs,
            p.failures,
            duration(p.total_ms)
        );
        if !p.slowest.is_empty() {
            let _ = writeln!(out, "  slowest");
            for (cmd, ms) in &p.slowest {
                let _ = writeln!(out, "    {:>8}  {}", duration(*ms), clip(cmd));
            }
        }
        if !p.failing.is_empty() {
            let _ = writeln!(out, "  failing most");
            for (cmd, n) in &p.failing {
                let _ = writeln!(out, "    {:>7}x  {}", n, clip(cmd));
            }
        }
        out.push('\n');
    }
    out
}

/// The newest command that ran on this machine and contains every word of `query` (any case); with
/// `failed`, only one that did not succeed. Commands with a newline are left out: they do not survive
/// being put back on a command line.
pub fn find<'a>(entries: &'a [Entry], query: &[String], failed: bool) -> Option<&'a Entry> {
    let words: Vec<String> = query.iter().map(|w| w.to_lowercase()).collect();
    entries
        .iter()
        .rev()
        .filter(|e| e.host.is_none() && !e.cmd.contains('\n'))
        .filter(|e| !failed || (e.exit != 0 && e.exit != INTERRUPTED))
        .find(|e| {
            let lower = e.cmd.to_lowercase();
            words.iter().all(|w| lower.contains(w))
        })
}

/// "3 hours ago", "yesterday", "2 days ago".
pub fn ago(then: u64, now: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..=59 => "just now".into(),
        60..=3_599 => format!("{} min ago", s / 60),
        3_600..=86_399 => format!("{} h ago", s / 3_600),
        86_400..=172_799 => "yesterday".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// What to tell the user about `e` before running it again.
pub fn context(e: &Entry, now: u64, branch_now: Option<&str>) -> String {
    let mut s = format!("ran {} in {}", ago(e.at, now), e.cwd);
    if let Some(b) = &e.branch {
        let _ = write!(s, " on {b}");
    }
    if e.exit != 0 {
        let _ = write!(s, ", exit {}", e.exit);
    }
    if let (Some(then), Some(now_b)) = (&e.branch, branch_now) {
        if then != now_b {
            let _ = write!(s, " (that directory is now on {now_b})");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(cmd: &str, cwd: &str, exit: i32, ms: u64, at: u64) -> Entry {
        Entry {
            cmd: cmd.into(),
            cwd: cwd.into(),
            exit,
            ms,
            at,
            branch: None,
            host: None,
        }
    }

    #[test]
    fn commands_group_under_the_repository_that_holds_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("app/.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("app/src/deep")).unwrap();
        let root = dir.path().join("app").display().to_string();
        assert_eq!(project_of(&format!("{root}/src/deep")), root);
        assert_eq!(project_of("/nowhere/at/all"), "/nowhere/at/all");
    }

    #[test]
    fn the_report_ranks_slow_and_failing_commands_and_ignores_ctrl_c() {
        let entries = vec![
            e("cargo test", "/p/a", 1, 90_000, 100),
            e("cargo test", "/p/a", 1, 60_000, 110),
            e("cargo test", "/p/a", 0, 30_000, 120),
            e("sleep 100", "/p/a", INTERRUPTED, 5_000, 130),
            e("ls", "/p/b", 0, 5, 140),
            e("old", "/p/a", 1, 1, 1),
        ];
        let r = report(&entries, 50, None, 3, "/p");
        assert_eq!(r.len(), 2);
        let a = &r[0];
        assert_eq!((a.name.as_str(), a.runs, a.failures), ("~/a", 4, 2));
        assert_eq!(a.slowest[0], ("cargo test".to_string(), 90_000));
        assert_eq!(a.failing, vec![("cargo test".to_string(), 2)]);
        assert_eq!(r[1].name, "~/b");
        assert!(render(&r, 7).contains("2 failed (50%)"));
    }

    #[test]
    fn the_report_can_be_limited_to_one_machine() {
        let mut remote = e("uptime", "/srv", 0, 10, 100);
        remote.host = Some("mini".into());
        let entries = vec![remote, e("ls", "/srv", 0, 10, 100)];
        assert_eq!(report(&entries, 0, Some("mini"), 3, "")[0].runs, 1);
        assert_eq!(report(&entries, 0, Some(""), 3, "")[0].runs, 1);
        assert_eq!(report(&entries, 0, None, 3, "")[0].runs, 2);
    }

    #[test]
    fn replay_finds_the_newest_local_match_and_only_failures_when_asked() {
        let mut other = e("make deploy", "/x", 0, 1, 300);
        other.host = Some("mini".into());
        let entries = vec![
            e("make deploy staging", "/a", 1, 1, 100),
            e("make deploy prod", "/b", 0, 1, 200),
            other,
        ];
        let q = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(find(&entries, &q("MAKE deploy"), false).unwrap().cwd, "/b");
        assert_eq!(find(&entries, &q("deploy"), true).unwrap().cwd, "/a");
        assert!(find(&entries, &q("nothing"), false).is_none());
    }

    #[test]
    fn the_replay_context_says_where_when_and_whether_the_branch_moved() {
        let mut old = e("make", "/a", 2, 1, 1_000);
        old.branch = Some("main".into());
        let text = context(&old, 1_000 + 7_300, Some("dev"));
        assert_eq!(
            text,
            "ran 2 h ago in /a on main, exit 2 (that directory is now on dev)"
        );
        assert_eq!(ago(0, 86_400 * 3), "3 days ago");
        assert_eq!(duration(61_000), "1m01s");
    }
}

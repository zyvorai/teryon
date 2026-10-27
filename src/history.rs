//! Command history for the shell integration (`~/.teryon/history.jsonl`): what was run, where, how
//! it ended. Feeds autosuggestions today and the history picker later.
use crate::util;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

/// Entries kept after a compaction.
const MAX_ENTRIES: usize = 20_000;
/// Compact once the file grows past this many bytes.
const COMPACT_BYTES: u64 = 4 << 20;
/// A suggestion lookup only reads this much of the end of the file.
const TAIL_BYTES: u64 = 512 << 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub cmd: String,
    pub cwd: String,
    pub exit: i32,
    pub ms: u64,
    pub at: u64,
    /// The Git branch the command ran on (or a short commit id when detached), if `cwd` is in a repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The machine the command ran on, for entries pulled from another host (`teryon hist pull`);
    /// `None` for commands run here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// The branch checked out at `cwd`, read from `.git/HEAD` without starting a process (this runs after
/// every command). Follows the `gitdir:` file that worktrees and submodules use.
pub fn git_branch(cwd: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    let git = loop {
        let candidate = dir.join(".git");
        if candidate.exists() {
            break candidate;
        }
        dir = dir.parent()?;
    };
    let git_dir = if git.is_file() {
        let text = std::fs::read_to_string(&git).ok()?;
        let target = text.trim().strip_prefix("gitdir:")?.trim();
        let target = std::path::Path::new(target);
        if target.is_absolute() {
            target.to_path_buf()
        } else {
            dir.join(target)
        }
    } else {
        git
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(r) => Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string()),
        None => (head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| head[..7].to_string()),
    }
}

pub fn path() -> Result<PathBuf> {
    Ok(util::home()?.join(".teryon/history.jsonl"))
}
/// Whether `cmd` belongs in the history: not blank, not hidden by a leading space, and not
/// carrying a recognizable secret (a token, a password flag, a URL with credentials). Such a command
/// is simply not kept, the way a shell's `HISTIGNORE` would, since a history that suggests it back
/// or lists it in a picker would show the secret again.
pub fn should_record(cmd: &str) -> bool {
    !cmd.trim().is_empty() && !cmd.starts_with(' ') && !has_secret(cmd)
}
/// Whether `cmd` contains something the redaction rules would hide.
pub fn has_secret(cmd: &str) -> bool {
    crate::ai::redact::redact(cmd).1 > 0
}
/// `entries` without the ones that contain a secret, and how many were dropped.
pub fn scrubbed(entries: Vec<Entry>) -> (Vec<Entry>, usize) {
    let before = entries.len();
    let kept: Vec<Entry> = entries
        .into_iter()
        .filter(|e| !has_secret(&e.cmd))
        .collect();
    let dropped = before - kept.len();
    (kept, dropped)
}
/// Remove entries with secrets from the history file itself; returns how many were removed.
pub fn scrub_file() -> Result<usize> {
    let path = path()?;
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(0);
    };
    let (kept, dropped) = scrubbed(parse(&text));
    if dropped > 0 {
        let out: String = kept
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .map(|l| l + "\n")
            .collect();
        util::write_private_atomic(&path, out.as_bytes())?;
    }
    Ok(dropped)
}
/// Where the history pulled from other machines is kept: one file per host.
fn hosts_dir() -> Result<PathBuf> {
    Ok(util::home()?.join(".teryon/history-hosts"))
}
/// The file name for a host: its characters made safe for a path.
fn host_file(host: &str) -> String {
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.jsonl")
}
/// Commands kept per host, and the longest command kept.
const MAX_HOST_ENTRIES: usize = 5_000;
const MAX_CMD_BYTES: usize = 4_096;
/// This machine's newest `limit` entries as JSON lines, what `teryon hist pull` reads over SSH.
/// Entries that contain a secret are left out again, whatever the file holds.
pub fn export(limit: usize) -> String {
    let (all, _) = scrubbed(load_all());
    let skip = all.len().saturating_sub(limit.min(MAX_HOST_ENTRIES));
    all[skip..]
        .iter()
        .filter(|e| e.host.is_none())
        .filter_map(|e| serde_json::to_string(e).ok())
        .map(|l| l + "\n")
        .collect()
}
/// The entries in `text` (an `export`), marked as run on `host`. Anything that does not belong in
/// a history here (secrets, blank or over-long commands) is dropped, whatever the other machine sent.
pub fn import(host: &str, text: &str) -> Vec<Entry> {
    let mut list: Vec<Entry> = parse(text)
        .into_iter()
        .filter(|e| should_record(&e.cmd) && e.cmd.len() <= MAX_CMD_BYTES)
        .map(|e| Entry {
            host: Some(host.to_string()),
            ..e
        })
        .collect();
    let excess = list.len().saturating_sub(MAX_HOST_ENTRIES);
    list.drain(..excess);
    list
}
/// Replace what is kept for `host` with `entries`.
pub fn save_host(host: &str, entries: &[Entry]) -> Result<()> {
    let text: String = entries
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .map(|l| l + "\n")
        .collect();
    util::write_private_atomic(&hosts_dir()?.join(host_file(host)), text.as_bytes())
}
/// Local history together with everything pulled from other hosts, oldest first.
pub fn load_with_hosts() -> Vec<Entry> {
    let mut all = load_all();
    if let Some(files) = hosts_dir().ok().and_then(|d| std::fs::read_dir(d).ok()) {
        for f in files.filter_map(|f| f.ok()) {
            if let Ok(text) = std::fs::read_to_string(f.path()) {
                all.extend(parse(&text).into_iter().filter(|e| e.host.is_some()));
            }
        }
    }
    all.sort_by_key(|e| e.at);
    all
}
/// Parse file text, oldest first; lines that are not valid entries are skipped.
pub fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
/// The newest history that fits in `TAIL_BYTES`, oldest first.
pub fn load_tail() -> Vec<Entry> {
    let read = || -> Result<String> {
        let mut file = std::fs::File::open(path()?)?;
        let len = file.metadata()?.len();
        let cut = len > TAIL_BYTES;
        if cut {
            file.seek(SeekFrom::Start(len - TAIL_BYTES))?;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        // A cut file starts mid-line; drop that fragment.
        Ok(match (cut, text.find('\n')) {
            (true, Some(i)) => text[i + 1..].to_string(),
            (true, None) => String::new(),
            _ => text,
        })
    };
    read().map(|t| parse(&t)).unwrap_or_default()
}
/// The whole history, oldest first.
pub fn load_all() -> Vec<Entry> {
    path()
        .and_then(|p| Ok(std::fs::read_to_string(p)?))
        .map(|t| parse(&t))
        .unwrap_or_default()
}
/// The command to ghost after `prefix`: the newest longer one that starts with it, preferring
/// commands that ran in `cwd`. Single-line commands only.
pub fn suggest(entries: &[Entry], prefix: &str, cwd: &str) -> Option<String> {
    if prefix.is_empty() || prefix.contains('\n') {
        return None;
    }
    let mut other = None;
    for e in entries.iter().rev() {
        if e.cmd.len() <= prefix.len() || !e.cmd.starts_with(prefix) || e.cmd.contains('\n') {
            continue;
        }
        if e.cwd == cwd {
            return Some(e.cmd.clone());
        }
        other.get_or_insert(&e.cmd);
    }
    other.cloned()
}
/// Keep the newest `MAX_ENTRIES`.
fn compact(mut entries: Vec<Entry>) -> Vec<Entry> {
    let excess = entries.len().saturating_sub(MAX_ENTRIES);
    entries.drain(..excess);
    entries
}
/// Append one entry (a single `write`, so concurrent shells do not interleave lines).
pub fn add(entry: &Entry) -> Result<()> {
    if !should_record(&entry.cmd) {
        return Ok(());
    }
    let path = path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    if file.metadata()?.len() > COMPACT_BYTES {
        let all = parse(&std::fs::read_to_string(&path)?);
        let text: String = compact(all)
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .map(|l| l + "\n")
            .collect();
        util::write_private_atomic(&path, text.as_bytes())?;
    }
    Ok(())
}

/// Commands in a zsh history file (`: 1700000000:0;cmd` with `EXTENDED_HISTORY`, or bare lines), oldest
/// first. A command spread over lines ends each but the last with a backslash; the lines are rejoined.
pub fn parse_zsh_history(text: &str) -> Vec<(u64, String)> {
    let mut out: Vec<(u64, String)> = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let (at, first) = match line
            .strip_prefix(": ")
            .and_then(|r| r.split_once(';'))
            .and_then(|(meta, cmd)| Some((meta.split_once(':')?.0.trim().parse().ok()?, cmd)))
        {
            Some((at, cmd)) => (at, cmd),
            None => (0, line),
        };
        let mut cmd = first.to_string();
        while cmd.ends_with('\\') {
            let Some(next) = lines.next() else { break };
            cmd.pop();
            cmd.push('\n');
            cmd.push_str(next);
        }
        if out.last().is_some_and(|(_, c)| *c == cmd) {
            continue;
        }
        out.push((at, cmd));
    }
    out
}
/// Put the commands from a zsh history file ahead of what is already recorded, so Ctrl-R and the
/// suggestions know your past on day one. Secrets and over-long commands are left out. Returns how many
/// were added.
pub fn import_shell_history(file: &std::path::Path) -> Result<usize> {
    let text = String::from_utf8_lossy(&std::fs::read(file)?).into_owned();
    let existing = match std::fs::read_to_string(path()?) {
        Ok(t) => parse(&t),
        Err(_) => Vec::new(),
    };
    let oldest = existing.iter().map(|e| e.at).filter(|a| *a > 0).min();
    let mut added: Vec<Entry> = parse_zsh_history(&text)
        .into_iter()
        .filter(|(_, cmd)| should_record(cmd) && cmd.len() <= MAX_CMD_BYTES)
        .map(|(at, cmd)| Entry {
            cmd,
            cwd: String::new(),
            exit: 0,
            ms: 0,
            // Entries without a timestamp sort before everything that has one.
            at: if at == 0 {
                oldest.unwrap_or(1).saturating_sub(1)
            } else {
                at
            },
            branch: None,
            host: None,
        })
        .collect();
    let n = added.len();
    added.extend(existing);
    let text: String = compact(added)
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .map(|l| l + "\n")
        .collect();
    util::write_private_atomic(&path()?, text.as_bytes())?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn e(cmd: &str, cwd: &str, at: u64) -> Entry {
        Entry {
            cmd: cmd.into(),
            cwd: cwd.into(),
            exit: 0,
            ms: 5,
            at,
            branch: None,
            host: None,
        }
    }
    #[test]
    fn parsing_skips_garbage_and_round_trips() {
        let good = serde_json::to_string(&e("ls -la", "/a", 1)).unwrap();
        let list = parse(&format!("not json\n{good}\n\n{{\"cmd\":1}}\n"));
        assert_eq!(list, vec![e("ls -la", "/a", 1)]);
    }
    #[test]
    fn zsh_history_parses_extended_plain_and_multiline() {
        let text = ": 1700000000:0;ls -la\n: 1700000005:2;echo a \\\nb\nplain cmd\nplain cmd\n";
        assert_eq!(
            parse_zsh_history(text),
            vec![
                (1700000000, "ls -la".to_string()),
                (1700000005, "echo a \nb".to_string()),
                (0, "plain cmd".to_string()),
            ]
        );
    }
    #[test]
    fn blank_and_space_prefixed_commands_are_not_recorded() {
        assert!(should_record("git status"));
        assert!(!should_record("   "));
        assert!(!should_record(" secret --token abc"));
    }
    #[test]
    fn commands_with_secrets_are_not_recorded() {
        for cmd in [
            "export GITHUB_TOKEN=ghp_aB3dEfGhIjKlMnOpQrStUvWxYz0123456789",
            "curl -u me --password hunter2xyz https://example.com",
            "git clone https://me:p4ssw0rd@example.com/r.git",
            "AWS_SECRET_ACCESS_KEY=abcdef123456 aws s3 ls",
        ] {
            assert!(!should_record(cmd), "{cmd}");
        }
        for cmd in [
            "git status",
            "cargo test --all-features",
            "echo the token count is 5",
        ] {
            assert!(should_record(cmd), "{cmd}");
        }
    }
    #[test]
    fn scrubbing_drops_only_the_entries_with_secrets() {
        let list = vec![
            e("ls", "/a", 1),
            e("deploy --token=abc123def456", "/a", 2),
            e("git status", "/a", 3),
        ];
        let (kept, dropped) = scrubbed(list);
        assert_eq!(dropped, 1);
        assert_eq!(
            kept.iter().map(|x| x.cmd.as_str()).collect::<Vec<_>>(),
            ["ls", "git status"]
        );
    }
    #[test]
    fn imported_history_is_marked_with_its_host_and_never_carries_secrets() {
        let line = |cmd: &str| {
            format!("{{\"cmd\":{cmd:?},\"cwd\":\"/s\",\"exit\":0,\"ms\":1,\"at\":5}}\n")
        };
        let text = [
            line("uptime"),
            line("deploy --token=abc123def456"),
            line("  hidden"),
            line(&"x".repeat(MAX_CMD_BYTES + 1)),
            "garbage\n".to_string(),
        ]
        .concat();
        let got = import("web1", &text);
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].cmd.as_str(), got[0].host.as_deref()),
            ("uptime", Some("web1"))
        );
        // The host name is made safe for a file name.
        assert_eq!(host_file("me@host:22/../x"), "me_host_22_.._x.jsonl");
        assert_eq!(host_file("../etc"), ".._etc.jsonl");
    }
    #[test]
    fn suggestion_is_the_newest_longer_match_preferring_this_directory() {
        let h = vec![
            e("git status", "/a", 1),
            e("git stash", "/b", 2),
            e("git show", "/a", 3),
        ];
        assert_eq!(suggest(&h, "git s", "/a").as_deref(), Some("git show"));
        assert_eq!(suggest(&h, "git sta", "/a").as_deref(), Some("git status"));
        assert_eq!(suggest(&h, "git sta", "/zzz").as_deref(), Some("git stash"));
        assert_eq!(suggest(&h, "git sta", "/b").as_deref(), Some("git stash"));
    }
    #[test]
    fn nothing_is_suggested_for_exact_unmatched_empty_or_multiline_input() {
        let h = vec![e("make", "/a", 1), e("echo a\necho b", "/a", 2)];
        assert_eq!(suggest(&h, "make", "/a"), None);
        assert_eq!(suggest(&h, "cargo", "/a"), None);
        assert_eq!(suggest(&h, "", "/a"), None);
        assert_eq!(suggest(&h, "echo a", "/a"), None);
        assert_eq!(suggest(&h, "echo\n", "/a"), None);
    }
    #[test]
    fn compaction_keeps_the_newest_entries() {
        let many: Vec<Entry> = (0..MAX_ENTRIES as u64 + 5)
            .map(|i| e(&format!("c{i}"), "/", i))
            .collect();
        let kept = compact(many);
        assert_eq!(kept.len(), MAX_ENTRIES);
        assert_eq!(kept[0].cmd, "c5");
        assert_eq!(kept.last().unwrap().cmd, format!("c{}", MAX_ENTRIES + 4));
    }

    #[test]
    fn the_branch_is_read_from_git_head_without_running_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(git_branch(sub.to_str().unwrap()), None, "not a repository");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(
            git_branch(sub.to_str().unwrap()).as_deref(),
            Some("feature/x")
        );
        // Detached: a short commit id.
        std::fs::write(
            root.join(".git/HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .unwrap();
        assert_eq!(
            git_branch(root.to_str().unwrap()).as_deref(),
            Some("0123456")
        );
        // A worktree has a `.git` file pointing at its own git directory.
        let wt = root.join("wt");
        let real = root.join(".git/worktrees/wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();
        std::fs::write(real.join("HEAD"), "ref: refs/heads/agent/claude-1\n").unwrap();
        assert_eq!(
            git_branch(wt.to_str().unwrap()).as_deref(),
            Some("agent/claude-1")
        );
        // Garbage in HEAD gives nothing rather than a wrong answer.
        std::fs::write(real.join("HEAD"), "nonsense").unwrap();
        assert_eq!(git_branch(wt.to_str().unwrap()), None);
    }
    #[test]
    fn old_entries_without_a_branch_still_load() {
        let old = r#"{"cmd":"ls","cwd":"/a","exit":0,"ms":3,"at":9}"#;
        let list = parse(old);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].branch, None);
        assert!(!serde_json::to_string(&list[0]).unwrap().contains("branch"));
    }
}

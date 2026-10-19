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
}

pub fn path() -> Result<PathBuf> {
    Ok(util::home()?.join(".teryon/history.jsonl"))
}
/// Whether `cmd` belongs in the history: not blank, and not hidden by a leading space.
pub fn should_record(cmd: &str) -> bool {
    !cmd.trim().is_empty() && !cmd.starts_with(' ')
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
        }
    }
    #[test]
    fn parsing_skips_garbage_and_round_trips() {
        let good = serde_json::to_string(&e("ls -la", "/a", 1)).unwrap();
        let list = parse(&format!("not json\n{good}\n\n{{\"cmd\":1}}\n"));
        assert_eq!(list, vec![e("ls -la", "/a", 1)]);
    }
    #[test]
    fn blank_and_space_prefixed_commands_are_not_recorded() {
        assert!(should_record("git status"));
        assert!(!should_record("   "));
        assert!(!should_record(" secret --token abc"));
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
}

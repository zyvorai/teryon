//! Frequently and recently used directories for `z` (`~/.teryon/jump.jsonl`). The file is an
//! append-only log of `{p: path, n: visits, at: last visit}` increments, summed per path on read,
//! so shells in different windows can append without locking.
use crate::{history, util};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

/// Compact once the log grows past this many bytes (a few thousand lines).
const COMPACT_BYTES: u64 = 256 << 10;
/// Directories kept after a compaction.
const MAX_DIRS: usize = 2000;
/// When the visit counts add up to more than this, they are scaled down by a tenth.
const MAX_TOTAL: u64 = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dir {
    #[serde(rename = "p")]
    pub path: String,
    #[serde(rename = "n")]
    pub visits: u64,
    pub at: u64,
}

pub fn path() -> Result<PathBuf> {
    Ok(util::home()?.join(".teryon/jump.jsonl"))
}
/// Sum the log: visits add up and the newest visit time wins. Bad lines are skipped.
pub fn parse(text: &str) -> Vec<Dir> {
    let mut by_path: HashMap<String, Dir> = HashMap::new();
    for line in text.lines() {
        let Ok(d) = serde_json::from_str::<Dir>(line) else {
            continue;
        };
        by_path
            .entry(d.path.clone())
            .and_modify(|e| {
                e.visits = e.visits.saturating_add(d.visits);
                e.at = e.at.max(d.at);
            })
            .or_insert(d);
    }
    by_path.into_values().collect()
}
/// Visits weighted by how recently the directory was used.
pub fn frecency(d: &Dir, now: u64) -> f64 {
    let age = now.saturating_sub(d.at);
    let weight = match age {
        0..=3599 => 4.0,
        3600..=86_399 => 2.0,
        86_400..=604_799 => 0.5,
        _ => 0.25,
    };
    d.visits as f64 * weight
}
/// Whether `path` matches every keyword in order (case-insensitive), the last one inside the final
/// path component: `z foo bar` finds `/src/foo/x/bar` but not `/src/bar/foo`.
pub fn matches(path: &str, keywords: &[String]) -> bool {
    let lower = path.to_lowercase();
    let mut at = 0;
    for kw in keywords {
        let kw = kw.to_lowercase();
        match lower[at..].find(&kw) {
            Some(i) => at += i + kw.len(),
            None => return false,
        }
    }
    match keywords.last() {
        Some(last) => {
            let name = lower.rsplit('/').next().unwrap_or("");
            name.contains(&last.to_lowercase())
        }
        None => true,
    }
}
/// Existing directories matching `keywords`, best first, never `cwd` itself.
pub fn rank(dirs: &[Dir], keywords: &[String], cwd: &str, now: u64) -> Vec<String> {
    let mut hits: Vec<(f64, &Dir)> = dirs
        .iter()
        .filter(|d| d.path != cwd && matches(&d.path, keywords))
        .filter(|d| std::path::Path::new(&d.path).is_dir())
        .map(|d| (frecency(d, now), d))
        .collect();
    hits.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.path.cmp(&b.1.path))
    });
    hits.into_iter().map(|(_, d)| d.path.clone()).collect()
}
/// Aggregate the log into at most `MAX_DIRS` live directories, scaling visits down when huge.
fn compact(mut dirs: Vec<Dir>, now: u64) -> Vec<Dir> {
    dirs.retain(|d| std::path::Path::new(&d.path).is_dir());
    let total: u64 = dirs.iter().map(|d| d.visits).sum();
    if total > MAX_TOTAL {
        for d in &mut dirs {
            d.visits = d.visits * 9 / 10;
        }
        dirs.retain(|d| d.visits > 0);
    }
    dirs.sort_by(|a, b| {
        frecency(b, now)
            .partial_cmp(&frecency(a, now))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    dirs.truncate(MAX_DIRS);
    dirs
}
fn serialize(dirs: &[Dir]) -> String {
    dirs.iter()
        .filter_map(|d| serde_json::to_string(d).ok())
        .map(|l| l + "\n")
        .collect()
}
pub fn load() -> Vec<Dir> {
    path()
        .and_then(|p| Ok(std::fs::read_to_string(p)?))
        .map(|t| parse(&t))
        .unwrap_or_default()
}
/// Record one visit to `dir` at `now`.
pub fn add(dir: &str, now: u64) -> Result<()> {
    if dir.is_empty() || dir == "/" {
        return Ok(());
    }
    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(&Dir {
        path: dir.into(),
        visits: 1,
        at: now,
    })?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    if file.metadata()?.len() > COMPACT_BYTES {
        let text = std::fs::read_to_string(&path)?;
        util::write_private_atomic(&path, serialize(&compact(parse(&text), now)).as_bytes())?;
    }
    Ok(())
}
/// Directories used in the command history, as a starting point when no jump log exists yet.
pub fn from_history(entries: &[history::Entry]) -> Vec<Dir> {
    let mut by_path: HashMap<&str, Dir> = HashMap::new();
    for e in entries.iter().filter(|e| e.cwd.starts_with('/')) {
        let d = by_path.entry(&e.cwd).or_insert_with(|| Dir {
            path: e.cwd.clone(),
            visits: 0,
            at: 0,
        });
        d.visits += 1;
        d.at = d.at.max(e.at);
    }
    by_path.into_values().collect()
}
/// Write the history-derived starting log, but only if there is no log yet.
pub fn seed_if_missing() {
    let Ok(path) = path() else { return };
    if path.exists() {
        return;
    }
    let dirs = from_history(&history::load_all());
    if !dirs.is_empty() {
        let _ = util::write_private_atomic(&path, serialize(&dirs).as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(path: &str, visits: u64, at: u64) -> Dir {
        Dir {
            path: path.into(),
            visits,
            at,
        }
    }
    fn kw(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }
    #[test]
    fn the_log_sums_visits_keeps_the_newest_time_and_skips_garbage() {
        let text = "junk\n{\"p\":\"/a\",\"n\":2,\"at\":10}\n{\"p\":\"/a\",\"n\":1,\"at\":30}\n{\"p\":\"/b\",\"n\":1,\"at\":5}\n";
        let mut dirs = parse(text);
        dirs.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(dirs, vec![d("/a", 3, 30), d("/b", 1, 5)]);
    }
    #[test]
    fn recent_use_outweighs_old_use() {
        let now = 100_000_000;
        let fresh = frecency(&d("/a", 1, now - 60), now);
        let day = frecency(&d("/a", 1, now - 7200), now);
        let week = frecency(&d("/a", 1, now - 2 * 86_400), now);
        let old = frecency(&d("/a", 1, now - 90 * 86_400), now);
        assert!(fresh > day && day > week && week > old);
        assert!(frecency(&d("/a", 20, now - 90 * 86_400), now) > fresh);
    }
    #[test]
    fn keywords_match_in_order_and_the_last_one_in_the_final_component() {
        assert!(matches("/src/foo/x/bar", &kw(&["foo", "bar"])));
        assert!(!matches("/src/bar/foo", &kw(&["foo", "bar"])));
        assert!(!matches("/src/foo/bar/x", &kw(&["foo", "bar"])));
        assert!(matches("/Users/me/Teryon", &kw(&["teryon"])));
        assert!(matches("/a/b", &[]));
        assert!(!matches("/a/b", &kw(&["zzz"])));
    }
    #[test]
    fn ranking_skips_the_current_and_missing_directories() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("proj-a"), dir.path().join("proj-b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let (a, b) = (
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
        );
        let dirs = vec![
            d(&a, 1, 100),
            d(&b, 5, 100),
            d("/definitely/not/here/proj-c", 99, 100),
        ];
        assert_eq!(
            rank(&dirs, &kw(&["proj"]), "", 100),
            vec![b.clone(), a.clone()]
        );
        assert_eq!(rank(&dirs, &kw(&["proj"]), &b, 100), vec![a]);
        assert!(rank(&dirs, &kw(&["nope"]), "", 100).is_empty());
    }
    #[test]
    fn compaction_drops_missing_dirs_and_scales_down_huge_counts() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().to_string_lossy().to_string();
        let kept = compact(vec![d(&here, 5, 10), d("/gone/for/sure", 5, 10)], 100);
        assert_eq!(kept, vec![d(&here, 5, 10)]);
        let scaled = compact(vec![d(&here, MAX_TOTAL + 10, 10)], 100);
        assert_eq!(scaled[0].visits, (MAX_TOTAL + 10) * 9 / 10);
    }
    #[test]
    fn history_directories_seed_the_log() {
        let e = |cwd: &str, at: u64| history::Entry {
            cmd: "ls".into(),
            cwd: cwd.into(),
            exit: 0,
            ms: 0,
            at,
            branch: None,
            host: None,
        };
        let mut dirs = from_history(&[e("/a", 1), e("/a", 9), e("/b", 3), e("relative", 4)]);
        dirs.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(dirs, vec![d("/a", 2, 9), d("/b", 1, 3)]);
    }
}

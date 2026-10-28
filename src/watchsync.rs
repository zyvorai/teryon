//! `teryon watch-sync SRC DEST`: copy a local directory to DEST (see `teryon cp`) now and again after
//! every change, backing off while the destination cannot be reached.
use anyhow::{bail, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

/// Most files a watched directory may hold; a bigger tree is a sign of the wrong directory.
const MAX_FILES: usize = 200_000;
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// A fingerprint of the files below `root` (path, size and modification time), ignoring `.git`.
/// Symbolic links are not followed.
pub fn fingerprint(root: &Path) -> Result<u64> {
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            // A directory deleted while walking is just a change.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && dir != root => continue,
            Err(e) => bail!("cannot read {}: {e}", dir.display()),
        };
        for entry in read.flatten() {
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let ft = entry.file_type().ok();
            if ft.is_some_and(|t| t.is_symlink()) {
                continue;
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                let rel = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(&entry.path())
                    .to_string_lossy()
                    .into_owned();
                entries.push((rel, meta.len(), mtime));
                if entries.len() > MAX_FILES {
                    bail!(
                        "{} holds more than {MAX_FILES} files; watch a smaller directory",
                        root.display()
                    );
                }
            }
        }
    }
    entries.sort();
    let mut h = DefaultHasher::new();
    entries.hash(&mut h);
    Ok(h.finish())
}

/// Decides when to copy: once at the start, then after the files have changed and stopped changing
/// for `debounce`; after a failed copy, not again until a growing delay has passed.
pub struct Machine {
    synced: Option<u64>,
    seen: u64,
    seen_at: Instant,
    debounce: Duration,
    retry_at: Option<Instant>,
    backoff: Duration,
}

impl Machine {
    pub fn new(first: u64, now: Instant, debounce: Duration) -> Self {
        Self {
            synced: None,
            seen: first,
            // The first copy does not wait for the files to settle.
            seen_at: now.checked_sub(debounce).unwrap_or(now),
            debounce,
            retry_at: None,
            backoff: Duration::from_secs(2),
        }
    }
    /// Feed the current fingerprint; true when a copy should start now.
    pub fn observe(&mut self, fp: u64, now: Instant) -> bool {
        if fp != self.seen {
            self.seen = fp;
            self.seen_at = now;
            return false;
        }
        if self.synced == Some(fp) || now.duration_since(self.seen_at) < self.debounce {
            return false;
        }
        self.retry_at.is_none_or(|t| now >= t)
    }
    /// Record the outcome of the copy of fingerprint `fp`.
    pub fn done(&mut self, fp: u64, ok: bool, now: Instant) {
        if ok {
            self.synced = Some(fp);
            self.retry_at = None;
            self.backoff = Duration::from_secs(2);
        } else {
            self.retry_at = Some(now + self.backoff);
            self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
        }
    }
    pub fn backoff_secs(&self) -> u64 {
        self.retry_at.map_or(0, |t| {
            t.saturating_duration_since(Instant::now())
                .as_millis()
                .div_ceil(1000) as u64
        })
    }
}

/// `src` as rsync wants a directory whose contents are copied: with a trailing slash.
pub fn contents_of(src: &str) -> String {
    if src.ends_with('/') {
        src.to_string()
    } else {
        format!("{src}/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn the_first_copy_starts_at_once_and_a_quiet_tree_is_not_copied_again() {
        let t0 = Instant::now();
        let mut m = Machine::new(7, t0, S);
        assert!(m.observe(7, t0));
        m.done(7, true, t0);
        assert!(!m.observe(7, t0 + S * 5));
    }

    #[test]
    fn a_change_is_copied_only_after_it_stops_changing() {
        let t0 = Instant::now();
        let mut m = Machine::new(1, t0, S);
        assert!(m.observe(1, t0));
        m.done(1, true, t0);
        assert!(!m.observe(2, t0 + S), "just changed");
        assert!(!m.observe(3, t0 + S * 2), "still changing");
        assert!(
            !m.observe(3, t0 + S * 2 + S / 2),
            "settled for less than the debounce"
        );
        assert!(m.observe(3, t0 + S * 4));
    }

    #[test]
    fn a_failed_copy_waits_with_a_growing_delay_then_tries_the_same_state_again() {
        let t0 = Instant::now();
        let mut m = Machine::new(1, t0, S);
        assert!(m.observe(1, t0));
        m.done(1, false, t0);
        assert!(!m.observe(1, t0 + S), "backing off for 2 s");
        assert!(m.observe(1, t0 + S * 3));
        m.done(1, false, t0 + S * 3);
        assert!(!m.observe(1, t0 + S * 6), "now 4 s");
        assert!(m.observe(1, t0 + S * 8));
        m.done(1, true, t0 + S * 8);
        assert!(!m.observe(1, t0 + S * 20));
    }

    #[test]
    fn the_fingerprint_changes_with_content_size_and_new_files_but_ignores_git() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::create_dir_all(p.join("sub")).unwrap();
        std::fs::create_dir_all(p.join(".git")).unwrap();
        std::fs::write(p.join("a.txt"), "one").unwrap();
        let first = fingerprint(p).unwrap();
        assert_eq!(first, fingerprint(p).unwrap());
        std::fs::write(p.join(".git/HEAD"), "x").unwrap();
        assert_eq!(first, fingerprint(p).unwrap(), ".git is ignored");
        std::fs::write(p.join("sub/b.txt"), "new").unwrap();
        let second = fingerprint(p).unwrap();
        assert_ne!(first, second);
        std::fs::write(p.join("a.txt"), "longer text").unwrap();
        assert_ne!(second, fingerprint(p).unwrap());
        assert!(fingerprint(&p.join("missing")).is_err());
    }

    #[test]
    fn a_directory_is_copied_by_its_contents() {
        assert_eq!(contents_of("src"), "src/");
        assert_eq!(contents_of("src/"), "src/");
    }
}

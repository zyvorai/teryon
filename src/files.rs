//! Read-only file browsing for the desktop sidebar: list a directory and preview a file, on this
//! machine or over SSH. Remote access uses plain `ls` and `head` through the existing OpenSSH
//! transport, so it works with every daemon version and needs nothing installed on the host.
use crate::transport::Target;
use crate::util;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::process::Stdio;

/// Most bytes a preview reads.
pub const PREVIEW_BYTES: usize = 64 * 1024;
/// Most entries one listing returns; the sidebar says when it was cut.
pub const MAX_ENTRIES: usize = 5000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

/// What a previewed file turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Text { text: String, truncated: bool },
    Binary { bytes: usize },
}

/// Join a directory and a child name with exactly one slash.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The parent directory, or `None` at the root.
pub fn parent(dir: &str) -> Option<String> {
    let trimmed = dir.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".into()),
        Some(i) => Some(trimmed[..i].to_string()),
        None => None,
    }
}

/// `(label, full path)` of every component, for a clickable breadcrumb.
pub fn crumbs(dir: &str) -> Vec<(String, String)> {
    let mut out = vec![("/".to_string(), "/".to_string())];
    let mut at = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        at.push('/');
        at.push_str(part);
        out.push((part.to_string(), at.clone()));
    }
    out
}

/// Folders first, then names case-insensitively; dotfiles only when asked for.
pub fn arrange(mut entries: Vec<Entry>, hidden: bool) -> Vec<Entry> {
    if !hidden {
        entries.retain(|e| !e.name.starts_with('.'));
    }
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

/// Parse `ls -1Ap` output: one name per line, folders end in a slash.
pub fn parse_ls(text: &str) -> Vec<Entry> {
    text.lines()
        .filter(|l| !l.is_empty())
        .take(MAX_ENTRIES)
        .map(|l| match l.strip_suffix('/') {
            Some(name) => Entry {
                name: name.to_string(),
                dir: true,
            },
            None => Entry {
                name: l.to_string(),
                dir: false,
            },
        })
        .collect()
}

/// Is this a path the sidebar may hand to a remote shell command?
fn safe_path(path: &str) -> Result<()> {
    if !path.starts_with('/') || path.contains(['\n', '\0']) {
        bail!("not an absolute path");
    }
    Ok(())
}

/// Entries of `dir` (unsorted).
pub fn list(target: &Target, dir: &str) -> Result<Vec<Entry>> {
    safe_path(dir)?;
    if target.local() {
        let mut out = Vec::new();
        for item in std::fs::read_dir(dir).with_context(|| format!("read {dir}"))? {
            let Ok(item) = item else { continue };
            let Ok(name) = item.file_name().into_string() else {
                continue;
            };
            // `metadata` follows links, so a link to a folder opens like a folder.
            let dir = item.metadata().map(|m| m.is_dir()).unwrap_or(false);
            out.push(Entry { name, dir });
            if out.len() >= MAX_ENTRIES {
                break;
            }
        }
        return Ok(out);
    }
    let script = format!("cd -- {} && LC_ALL=C ls -1Ap", util::shell_quote(dir));
    let bytes = run_remote(target, &script, 4 * 1024 * 1024)?;
    Ok(parse_ls(&String::from_utf8_lossy(&bytes)))
}

/// The first `PREVIEW_BYTES` (plus one, to tell truncation) of a file.
pub fn read_head(target: &Target, path: &str) -> Result<Vec<u8>> {
    safe_path(path)?;
    let want = PREVIEW_BYTES + 1;
    if target.local() {
        let file = std::fs::File::open(path).with_context(|| format!("open {path}"))?;
        let mut buf = Vec::new();
        file.take(want as u64).read_to_end(&mut buf)?;
        return Ok(buf);
    }
    let script = format!("head -c {want} -- {}", util::shell_quote(path));
    run_remote(target, &script, want)
}

fn run_remote(target: &Target, script: &str, cap: usize) -> Result<Vec<u8>> {
    let output = target
        .ssh()
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .context("run ssh")?;
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{}",
            why.lines().last().unwrap_or("remote command failed").trim()
        );
    }
    let mut bytes = output.stdout;
    bytes.truncate(cap);
    Ok(bytes)
}

/// Decide whether `bytes` (the head of a file) is text worth showing.
pub fn classify(bytes: &[u8]) -> Preview {
    let truncated = bytes.len() > PREVIEW_BYTES;
    let head = &bytes[..bytes.len().min(PREVIEW_BYTES)];
    let sniff = &head[..head.len().min(8192)];
    if sniff.contains(&0) {
        return Preview::Binary { bytes: bytes.len() };
    }
    // A cut multi-byte character at the end of the window is fine; garbage throughout is not.
    let text = String::from_utf8_lossy(head);
    let bad = text.chars().filter(|c| *c == '\u{fffd}').count();
    if bad > 4 && bad * 50 > text.chars().count() {
        return Preview::Binary { bytes: bytes.len() };
    }
    Preview::Text {
        text: text.into_owned(),
        truncated,
    }
}

/// Human size such as `3.4 KB`.
pub fn human(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024. && u + 1 < UNITS.len() {
        v /= 1024.;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            dir,
        }
    }

    #[test]
    fn paths_join_split_and_crumb() {
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("/", "b"), "/b");
        assert_eq!(parent("/a/b"), Some("/a".into()));
        assert_eq!(parent("/a"), Some("/".into()));
        assert_eq!(parent("/"), None);
        let c = crumbs("/Users/me/src");
        assert_eq!(c.len(), 4);
        assert_eq!(c[2], ("me".into(), "/Users/me".into()));
        assert_eq!(crumbs("/").len(), 1);
    }

    #[test]
    fn ls_output_becomes_entries_and_sorts_folders_first() {
        let list = parse_ls("zeta\nAlpha/\n.hidden\nbeta/\n\nnotes.txt\n");
        assert_eq!(list.len(), 5);
        let shown = arrange(list.clone(), false);
        let names: Vec<&str> = shown.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "beta", "notes.txt", "zeta"]);
        let all = arrange(list, true);
        assert_eq!(all[2].name, ".hidden");
    }

    #[test]
    fn previews_tell_text_from_binary_and_truncation() {
        assert_eq!(
            classify(b"hello\nworld\n"),
            Preview::Text {
                text: "hello\nworld\n".into(),
                truncated: false
            }
        );
        assert!(matches!(
            classify(b"\x7fELF\0\0\0\x01"),
            Preview::Binary { .. }
        ));
        let big = vec![b'a'; PREVIEW_BYTES + 1];
        let Preview::Text { text, truncated } = classify(&big) else {
            panic!("text expected");
        };
        assert!(truncated);
        assert_eq!(text.len(), PREVIEW_BYTES);
        let noise: Vec<u8> = (0..400).map(|i| 0x80 + (i % 7) as u8).collect();
        assert!(matches!(classify(&noise), Preview::Binary { .. }));
    }

    #[test]
    fn local_listing_and_reading_work_and_reject_odd_paths() {
        let dir = std::env::temp_dir().join(format!("teryon-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "hi").unwrap();
        let target = Target::new("local".into(), None).unwrap();
        let shown = arrange(list(&target, dir.to_str().unwrap()).unwrap(), true);
        assert_eq!(shown[0], entry("sub", true));
        assert_eq!(shown[1], entry("a.txt", false));
        let head = read_head(&target, dir.join("a.txt").to_str().unwrap()).unwrap();
        assert_eq!(head, b"hi");
        assert!(list(&target, "relative").is_err());
        assert!(read_head(&target, "/x\n/y").is_err());
        assert!(list(&target, "/definitely/not/here").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Opt-in: `TERYON_TEST_SSH_HOST=mini cargo test remote_listing` against a real SSH host.
    #[test]
    fn remote_listing_and_reading_work_over_ssh() {
        let Ok(host) = std::env::var("TERYON_TEST_SSH_HOST") else {
            return;
        };
        let target = Target::new(host, None).unwrap();
        let root = arrange(list(&target, "/").unwrap(), true);
        assert!(root
            .iter()
            .any(|e| e.dir && e.name == "Users" || e.name == "etc"));
        let head = read_head(&target, "/etc/hosts").unwrap();
        assert!(matches!(classify(&head), Preview::Text { .. }));
        assert!(list(&target, "/definitely/not/here").is_err());
        // A name with a quote and a space must survive the remote shell.
        assert!(read_head(&target, "/tmp/it's here").is_err());
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(human(12), "12 B");
        assert_eq!(human(2048), "2.0 KB");
        assert_eq!(human(5 * 1024 * 1024), "5.0 MB");
    }
}

//! Session notes and bookmarks.
//!
//! Notes are free-form text attached to a host/workspace.
//! Bookmarks are named pane or directory shortcuts.

use std::fs;
use std::path::PathBuf;

fn notes_dir() -> Option<PathBuf> {
    let base = crate::util::home().ok()?.join(".teryon").join("notes");
    let _ = fs::create_dir_all(&base);
    Some(base)
}

fn note_path(host: &str, workspace: &str) -> Option<PathBuf> {
    let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect::<String>();
    Some(notes_dir()?.join(format!("{}-{}.md", safe(host), safe(workspace))))
}

pub fn load_note(host: &str, workspace: &str) -> String {
    note_path(host, workspace).and_then(|p| fs::read_to_string(p).ok()).unwrap_or_default()
}

pub fn save_note(host: &str, workspace: &str, content: &str) -> std::io::Result<()> {
    let Some(path) = note_path(host, workspace) else { return Ok(()); };
    if content.trim().is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    fs::write(path, content)
}

pub fn list_notes() -> Vec<(String, String)> {
    let Some(dir) = notes_dir() else { return Vec::new(); };
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if name.ends_with(".md") {
                    if let Ok(body) = fs::read_to_string(e.path()) {
                        out.push((name.trim_end_matches(".md").to_string(), body));
                    }
                }
            }
        }
    }
    out
}

pub fn add_bookmark(name: &str, host: &str, workspace: &str, pane: Option<u64>, cwd: Option<&str>) -> std::io::Result<()> {
    let Some(dir) = notes_dir() else { return Ok(()); };
    let path = dir.join("bookmarks.jsonl");
    let line = format!(r#"{{"name":"{name}","host":"{host}","workspace":"{workspace}","pane":{},"cwd":"{}"}}"#,
        pane.map(|p| p.to_string()).unwrap_or_else(|| "null".into()),
        cwd.unwrap_or(""));
    use std::io::Write;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

pub fn list_bookmarks() -> Vec<String> {
    let Some(dir) = notes_dir() else { return Vec::new(); };
    let path = dir.join("bookmarks.jsonl");
    fs::read_to_string(path).map(|s| s.lines().map(|l| l.to_string()).collect()).unwrap_or_default()
}

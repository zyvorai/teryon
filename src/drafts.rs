//! Persistent offline drafts.
//!
//! Drafts are saved under `~/.teryon/drafts/<workspace>.txt` so they survive
//! client restarts. The TUI loads them on start and saves on change.

use std::fs;
use std::path::PathBuf;

fn drafts_dir() -> Option<PathBuf> {
    let base = crate::util::home().ok()?.join(".teryon").join("drafts");
    let _ = fs::create_dir_all(&base);
    Some(base)
}

fn draft_path(workspace: &str) -> Option<PathBuf> {
    let safe: String = workspace
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(drafts_dir()?.join(format!("{safe}.txt")))
}

pub fn load_draft(workspace: &str) -> String {
    draft_path(workspace)
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default()
}

pub fn save_draft(workspace: &str, content: &str) -> std::io::Result<()> {
    let Some(path) = draft_path(workspace) else {
        return Ok(());
    };
    if content.trim().is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    fs::write(path, content)
}

pub fn list_drafts() -> Vec<(String, String)> {
    let Some(dir) = drafts_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some(ws) = name.strip_suffix(".txt") {
                    if let Ok(body) = fs::read_to_string(e.path()) {
                        out.push((ws.to_string(), body));
                    }
                }
            }
        }
    }
    out
}

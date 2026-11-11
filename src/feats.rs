//! Additional features bundle.
//!
//! - Draft management helpers
//! - Preset apply scaffolding
//! - Persistent host metrics snapshot
//! - Port auto-forward helpers
//! - Session summary export

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn metrics_dir() -> Option<PathBuf> {
    let base = crate::util::home().ok()?.join(".teryon").join("metrics");
    let _ = fs::create_dir_all(&base);
    Some(base)
}

pub fn save_host_metrics(host: &str, load: f64, mem_used: u64, mem_total: u64) -> std::io::Result<()> {
    let Some(dir) = metrics_dir() else { return Ok(()); };
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("{host}.jsonl"));
    let line = format!(r#"{{"ts":{ts},"load":{load},"mem_used":{mem_used},"mem_total":{mem_total}}}"#);
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

pub fn recent_metrics(host: &str, n: usize) -> Vec<String> {
    let Some(dir) = metrics_dir() else { return Vec::new(); };
    let path = dir.join(format!("{host}.jsonl"));
    let Ok(text) = fs::read_to_string(path) else { return Vec::new(); };
    text.lines().rev().take(n).map(|s| s.to_string()).collect()
}

pub fn clear_drafts() -> usize {
    let drafts = crate::drafts::list_drafts();
    let mut n = 0;
    for (ws, _) in drafts {
        if crate::drafts::save_draft(&ws, "").is_ok() {
            n += 1;
        }
    }
    n
}

pub fn session_summary(host: &str, workspace: &str) -> String {
    let mut s = format!("# Teryon session summary\n\nHost: {host}\nWorkspace: {workspace}\n\n");
    s.push_str("## Drafts\n");
    for (ws, body) in crate::drafts::list_drafts() {
        s.push_str(&format!("- {ws}: {}\n", body.lines().next().unwrap_or("")));
    }
    s.push_str("\n## Presets\n");
    for name in crate::presets::list_presets() {
        s.push_str(&format!("- {name}\n"));
    }
    s.push_str("\n## Recent metrics\n");
    for line in recent_metrics(host, 5) {
        s.push_str(&format!("- {line}\n"));
    }
    s
}

pub fn apply_preset_hint(name: &str) -> String {
    match crate::presets::load_preset(name) {
        Some(p) => {
            let mut s = format!("Preset '{name}'\n");
            if let Some(d) = &p.description {
                s.push_str(&format!("  {d}\n"));
            }
            for pane in &p.panes {
                s.push_str(&format!("  - {} : {}\n", pane.title, pane.command));
            }
            s.push_str("\nUse `teryon up` with a matching .teryon.toml or start panes manually.\n");
            s
        }
        None => format!("preset '{name}' not found"),
    }
}

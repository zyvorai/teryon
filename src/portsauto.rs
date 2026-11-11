//! Automatic port-forward suggestions and simple rules.
//!
//! Stores preferred forwards under ~/.teryon/ports.jsonl so the Ports window
//! or CLI can offer "forward these again".

use std::fs;
use std::path::PathBuf;

fn rules_path() -> Option<PathBuf> {
    crate::util::home().ok().map(|h| h.join(".teryon").join("ports.jsonl"))
}

pub fn add_rule(host: &str, local: u16, remote: u16, name: &str) -> std::io::Result<()> {
    let Some(path) = rules_path() else { return Ok(()); };
    let line = format!(r#"{{"host":"{host}","local":{local},"remote":{remote},"name":"{name}"}}"#);
    use std::io::Write;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

pub fn list_rules() -> Vec<String> {
    let Some(path) = rules_path() else { return Vec::new(); };
    fs::read_to_string(path).map(|s| s.lines().map(|l| l.to_string()).collect()).unwrap_or_default()
}

pub fn clear_rules() -> std::io::Result<()> {
    if let Some(path) = rules_path() {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

//! Agent workspace presets.
//!
//! A preset is a small TOML file under `~/.teryon/presets/<name>.toml` that
//! describes a common agent workspace (panes + commands + optional recipe).
//!
//! Example `~/.teryon/presets/dev.toml`:
//! ```toml
//! name = "dev"
//! description = "Vite frontend + API + Claude"
//!
//! [[panes]]
//! title = "frontend"
//! command = "npm run dev"
//!
//! [[panes]]
//! title = "api"
//! command = "cargo run"
//!
//! [[panes]]
//! title = "agent"
//! command = "claude"
//! ```

use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Preset {
    pub name: String,
    pub description: Option<String>,
    pub panes: Vec<PresetPane>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct PresetPane {
    pub title: String,
    pub command: String,
    pub cwd: Option<String>,
}

fn presets_dir() -> Option<PathBuf> {
    let base = crate::util::home().ok()?.join(".teryon").join("presets");
    let _ = fs::create_dir_all(&base);
    Some(base)
}

pub fn list_presets() -> Vec<String> {
    let Some(dir) = presets_dir() else { return Vec::new(); };
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some(stem) = name.strip_suffix(".toml") {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

pub fn load_preset(name: &str) -> Option<Preset> {
    let path = presets_dir()?.join(format!("{name}.toml"));
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

pub fn save_preset(preset: &Preset) -> std::io::Result<()> {
    let Some(dir) = presets_dir() else { return Ok(()); };
    let path = dir.join(format!("{}.toml", preset.name));
    let text = toml::to_string_pretty(preset).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(path, text)
}

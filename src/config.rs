use crate::util;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Show the TERYON banner on `connect`; `--no-splash` and `TERYON_NO_SPLASH` also disable it.
    #[serde(default = "yes")]
    pub splash: bool,
    /// Capture the mouse in the TUI (click to focus; wheel opens history). Off by default because it
    /// replaces the terminal's own text selection.
    #[serde(default)]
    pub mouse: bool,
    #[serde(default)]
    pub hosts: BTreeMap<String, Host>,
    /// Desktop app options (`[desktop]`).
    #[serde(default)]
    pub desktop: DesktopConfig,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneTitles {
    /// Title bars only when more than one pane is visible.
    #[default]
    Auto,
    Always,
    Never,
}
/// Options for the native desktop app; every field is optional in `config.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DesktopConfig {
    /// Terminal font: a family or file name found in the macOS font folders, or an absolute path.
    pub font_family: Option<String>,
    pub font_size: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Space around the terminal text, in pixels.
    pub padding: f32,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    /// Window transparency, 0.3 (see-through) to 1.0 (opaque).
    pub window_opacity: f32,
    /// Brightness of unfocused panes, 0.2 (dim) to 1.0 (no dimming).
    pub unfocused_opacity: f32,
    pub copy_on_select: bool,
    /// Lines of history shown when scrolling back.
    pub scrollback_lines: usize,
    pub pane_titles: PaneTitles,
    /// Ask before closing the window (shells keep running either way).
    pub confirm_quit: bool,
    /// Key bindings: action name -> chord such as "cmd+shift+d"; "none" unbinds.
    pub keys: BTreeMap<String, String>,
}
impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size: 13.,
            line_height: 1.45,
            padding: 8.,
            cursor_style: CursorStyle::Block,
            cursor_blink: false,
            window_opacity: 1.,
            unfocused_opacity: 0.82,
            copy_on_select: false,
            scrollback_lines: 10_000,
            pane_titles: PaneTitles::Auto,
            confirm_quit: false,
            keys: BTreeMap::new(),
        }
    }
}
impl DesktopConfig {
    /// Reject out-of-range values and bad key bindings, naming the offending option.
    pub fn validate(&self) -> Result<()> {
        let check = |name: &str, v: f32, lo: f32, hi: f32| -> Result<()> {
            if !(lo..=hi).contains(&v) {
                bail!("desktop.{name} = {v} is outside {lo}..{hi}");
            }
            Ok(())
        };
        check("font_size", self.font_size, 6., 72.)?;
        check("line_height", self.line_height, 1., 2.5)?;
        check("padding", self.padding, 0., 48.)?;
        check("window_opacity", self.window_opacity, 0.3, 1.)?;
        check("unfocused_opacity", self.unfocused_opacity, 0.2, 1.)?;
        if !(100..=1_000_000).contains(&self.scrollback_lines) {
            bail!("desktop.scrollback_lines must be between 100 and 1000000");
        }
        let (_, problems) = crate::keymap::Keymap::build(&self.keys);
        if let Some(first) = problems.first() {
            bail!("desktop.keys: {first}");
        }
        Ok(())
    }
}
fn yes() -> bool {
    true
}
fn default_theme() -> String {
    "mac27-dark".into()
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub destination: String,
    pub remote_bin: Option<String>,
}
pub fn path() -> Result<PathBuf> {
    Ok(util::home()?.join(".config/teryon/config.toml"))
}
/// Pre-rename (ZYLINK) location, read only when no TERYON config exists.
fn legacy_path() -> Result<PathBuf> {
    Ok(util::home()?.join(".config/zylink/config.toml"))
}
pub fn load() -> Result<Config> {
    let mut path = path()?;
    if !path.exists() {
        path = legacy_path()?;
    }
    if !path.exists() {
        return Ok(Config {
            theme: default_theme(),
            splash: true,
            mouse: false,
            hosts: BTreeMap::new(),
            desktop: DesktopConfig::default(),
        });
    }
    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
}
/// Parse and validate the desktop section of `text` (a whole `config.toml`).
pub fn parse_desktop(text: &str) -> Result<DesktopConfig> {
    let cfg: Config = toml::from_str(text)?;
    cfg.desktop.validate()?;
    Ok(cfg.desktop)
}
/// The validated `[desktop]` options from the user's config file (defaults when there is none).
pub fn load_desktop() -> Result<DesktopConfig> {
    let cfg = load()?;
    cfg.desktop.validate()?;
    Ok(cfg.desktop)
}
/// Concrete `Host` aliases from OpenSSH config text; wildcards, negations and `Match` blocks are skipped.
pub fn ssh_hosts(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        if !key.eq_ignore_ascii_case("host") {
            continue;
        }
        for name in parts {
            let bare = name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
            if bare && !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
        }
    }
    out
}
/// Append `[hosts.NAME]` tables for names not already configured; existing text is left untouched.
pub fn add_hosts(names: &[String]) -> Result<Vec<String>> {
    let path = path()?;
    if !path.exists() {
        init()?;
    }
    let existing = load()?;
    let added: Vec<String> = names
        .iter()
        .filter(|n| !existing.hosts.contains_key(*n))
        .cloned()
        .collect();
    if added.is_empty() {
        return Ok(added);
    }
    let mut text = String::new();
    if !std::fs::read_to_string(&path)?.ends_with('\n') {
        text.push('\n');
    }
    for n in &added {
        text.push_str(&format!("\n[hosts.{n}]\ndestination = \"{n}\"\n"));
    }
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)?
        .write_all(text.as_bytes())?;
    Ok(added)
}
pub fn init() -> Result<PathBuf> {
    let path = path()?;
    if path.exists() {
        bail!("configuration already exists: {}", path.display());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    util::private_write(&path, include_bytes!("../docs/config.example.toml"))?;
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_section_defaults_validates_and_rejects_unknown_keys() {
        assert_eq!(parse_desktop("").unwrap(), DesktopConfig::default());
        let ok = parse_desktop("[desktop]\nfont_size = 15.0\ncursor_style = \"bar\"\n[desktop.keys]\npalette = \"cmd+p\"\n").unwrap();
        assert_eq!((ok.font_size, ok.cursor_style), (15., CursorStyle::Bar));
        assert!(parse_desktop("[desktop]\nfont_sise = 15.0\n").is_err());
        assert!(parse_desktop("[desktop]\nfont_size = 2.0\n").is_err());
        assert!(parse_desktop("[desktop]\ncursor_style = \"blob\"\n").is_err());
        assert!(parse_desktop("[desktop.keys]\nnope = \"cmd+x\"\n").is_err());
        assert!(parse_desktop("[desktop.keys]\npalette = \"cmd+d\"\n").is_err());
    }
    #[test]
    fn the_shipped_example_parses_and_its_desktop_defaults_hold() {
        let text = include_str!("../docs/config.example.toml");
        assert_eq!(parse_desktop(text).unwrap(), DesktopConfig::default());
        let enabled = text
            .replace("# [desktop", "[desktop")
            .replace("# palette", "palette");
        assert!(
            parse_desktop(&enabled).is_ok(),
            "uncommented example must stay valid"
        );
    }
    #[test]
    fn ssh_hosts_skips_wildcards_and_comments() {
        let cfg = "Host mini # home\n  HostName 10.0.0.2\nhost *\nHost a b !c *.x\nMatch all\nHost mini\n";
        assert_eq!(ssh_hosts(cfg), ["mini", "a", "b"]);
    }
}

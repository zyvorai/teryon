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
    /// What plain `teryon` opens after you pick a host: the terminal UI or the desktop app.
    #[serde(default)]
    pub start: StartMode,
    /// AI assistant options (`[ai]`).
    #[serde(default)]
    pub ai: AiConfig,
    /// Zsh integration switches (`[shell]`).
    #[serde(default)]
    pub shell: ShellConfig,
    /// Push notifications from the daemon while no client is attached (`[notify]`).
    #[serde(default)]
    pub notify: NotifyConfig,
}
/// `[notify]`: where the daemon sends "an agent needs you" and "a command finished" while a
/// workspace has no client attached. Messages name the agent or program, never screen text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotifyConfig {
    /// A macOS notification on the machine running the daemon.
    pub macos: bool,
    /// An ntfy topic URL (`https://ntfy.sh/your-topic`); the message is POSTed as the body.
    pub ntfy_url: Option<String>,
    /// A webhook that receives `{"title","message","event","workspace","pane"}` as JSON.
    pub webhook_url: Option<String>,
    /// Tell me when an AI agent needs me or finishes.
    pub agents: bool,
    /// Tell me when a command that ran at least `after_seconds` finishes (needs shell integration).
    pub commands: bool,
    pub after_seconds: u32,
}
impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            macos: false,
            ntfy_url: None,
            webhook_url: None,
            agents: true,
            commands: true,
            after_seconds: 30,
        }
    }
}
impl NotifyConfig {
    /// Whether any sink is configured.
    pub fn active(&self) -> bool {
        self.macos || self.ntfy_url.is_some() || self.webhook_url.is_some()
    }
    pub fn validate(&self) -> Result<()> {
        for (name, url) in [("ntfy_url", &self.ntfy_url), ("webhook_url", &self.webhook_url)] {
            if let Some(url) = url {
                if !(url.starts_with("https://") || url.starts_with("http://"))
                    || url.chars().any(|c| c.is_control() || c.is_whitespace())
                {
                    bail!("notify.{name} must be an http:// or https:// URL without spaces");
                }
            }
        }
        if self.after_seconds == 0 || self.after_seconds > 86_400 {
            bail!("notify.after_seconds = {} is outside 1..86400", self.after_seconds);
        }
        Ok(())
    }
}
/// `[shell]`: which parts of the zsh integration `teryon shell-init zsh` turns on. Every switch
/// can still be overridden per shell with its `TERYON_*` environment variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellConfig {
    /// Master switch for everything below (`TERYON_PLUS`).
    pub enabled: bool,
    /// Record commands with directory, exit status and duration (`TERYON_HISTORY`).
    pub history: bool,
    /// Ghost-text suggestions from history (`TERYON_SUGGEST`).
    pub suggest: bool,
    /// Colour the command line while typing (`TERYON_HIGHLIGHT`).
    pub highlight: bool,
    /// Ctrl-R history, Ctrl-T files and Alt-C directories (`TERYON_PICKERS`).
    pub pickers: bool,
    /// Fuzzy Tab completion (`TERYON_TAB`).
    pub tab: bool,
    /// The `z` and `zi` directory jump (`TERYON_JUMP`).
    pub jump: bool,
    /// The two-line prompt with git state; off unless you ask (`TERYON_PROMPT`).
    pub prompt: bool,
}
impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            history: true,
            suggest: true,
            highlight: true,
            pickers: true,
            tab: true,
            jump: true,
            prompt: false,
        }
    }
}
/// `[ai]`: the Claude-powered assistant. The API key is never stored here (`ANTHROPIC_API_KEY` or the Keychain).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AiConfig {
    pub enabled: bool,
    pub model: String,
    pub max_tokens: u32,
    /// API endpoint base; only for proxies and tests.
    pub base_url: Option<String>,
    /// Read-only tools that run without asking (changing tools always ask).
    pub auto_allow: Vec<String>,
}
impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: "claude-sonnet-5-5".into(),
            max_tokens: 2048,
            base_url: None,
            auto_allow: crate::ai::tools::READ_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}
impl AiConfig {
    pub fn validate(&self) -> Result<()> {
        if !(64..=16000).contains(&self.max_tokens) {
            bail!("ai.max_tokens = {} is outside 64..16000", self.max_tokens);
        }
        if self.model.trim().is_empty() || self.model.contains(char::is_whitespace) {
            bail!("ai.model must be a model id such as claude-sonnet-5-5");
        }
        if let Some(url) = &self.base_url {
            if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1")) {
                bail!("ai.base_url must be https:// (or http://127.0.0.1 for tests)");
            }
        }
        for name in &self.auto_allow {
            if !crate::ai::tools::READ_TOOLS.contains(&name.as_str()) {
                bail!(
                    "ai.auto_allow: \"{name}\" is not a read-only tool (allowed: {})",
                    crate::ai::tools::READ_TOOLS.join(", ")
                );
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartMode {
    #[default]
    Tui,
    Desktop,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}
/// When the desktop app draws a scrollbar along the right edge of a pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scrollbar {
    /// Only while you are scrolled back through history.
    #[default]
    Auto,
    /// Always, so its size shows how much history there is.
    Always,
}
/// How much window decoration the desktop app draws.
/// A strip of tabs along the bottom edge of a bare window (`chrome = "none"`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabBar {
    #[default]
    Off,
    Bottom,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chrome {
    /// The title bar with tabs and buttons.
    #[default]
    Full,
    /// Nothing but the terminal, like a bare terminal window: no title bar, no tab strip.
    None,
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
/// A saved command; `{{name}}` placeholders are asked for before it is typed into a pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snippet {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Press Enter after typing it (single-line commands only; multi-line snippets are never auto-run).
    #[serde(default)]
    pub run: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerAction {
    /// Tint the matching text on screen.
    #[default]
    Highlight,
    /// A system notification with the matching line.
    Notify,
    /// A short sound (macOS).
    Bell,
    /// A message in the corner of the window.
    Toast,
}
/// A rule applied to what appears on screen: when `pattern` matches a line, do `action`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    /// A regular expression (Rust `regex` syntax) matched against each screen line.
    pub pattern: String,
    #[serde(default)]
    pub action: TriggerAction,
    /// `#RRGGBB` tint for `highlight` (default: the theme's warning color).
    #[serde(default)]
    pub color: Option<String>,
    /// Only fire while the window is in the background (default: true for notify and bell).
    #[serde(default)]
    pub unfocused_only: Option<bool>,
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
    /// Overrides of `padding` for the left and right edges, the top and the bottom.
    pub padding_x: Option<f32>,
    pub padding_top: Option<f32>,
    pub padding_bottom: Option<f32>,
    /// Space between the window edge and the panes, in pixels (0 = edge to edge).
    pub pane_inset: f32,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    /// `#RRGGBB` cursor color (default: orange `#D97757`).
    pub cursor_color: Option<String>,
    /// Text weight: coverage gamma of the glyph atlas, 0.3 (heavy) to 1.0 (thin); 0.55 is egui's own.
    pub text_gamma: f32,
    /// Blur what shows through a transparent window, 0 (off) to 100 (macOS only; needs
    /// `window_opacity` below 1.0 and a restart).
    pub window_blur: u32,
    /// Two or more `#RRGGBB` colors painted across the terminal instead of the flat background.
    pub background_gradient: Vec<String>,
    /// Gradient direction in degrees as in CSS and WezTerm: 90 runs left to right, 180 top to
    /// bottom, 135 toward the bottom right.
    pub gradient_angle: f32,
    /// Window transparency, 0.3 (see-through) to 1.0 (opaque).
    pub window_opacity: f32,
    /// Brightness of unfocused panes, 0.2 (dim) to 1.0 (no dimming).
    pub unfocused_opacity: f32,
    /// Show the muted status line (PTYs held, last relink, receive rate) under the panes.
    pub status_line: bool,
    pub copy_on_select: bool,
    /// Notify when a command that took at least this many seconds finishes while the window is in
    /// the background (needs shell integration; 0 turns it off).
    pub notify_after_seconds: u32,
    /// Notify when an AI coding agent in the background needs you or finishes.
    pub agent_notifications: bool,
    /// Open the file browser sidebar when the app starts.
    pub file_browser: bool,
    /// Themes to switch to when macOS goes light or dark (either may be left out).
    pub theme_light: Option<String>,
    pub theme_dark: Option<String>,
    /// Lines of history shown when scrolling back.
    pub scrollback_lines: usize,
    pub pane_titles: PaneTitles,
    /// `"none"` draws only the terminal: no title bar and no tab strip (restart to apply).
    pub chrome: Chrome,
    /// `"bottom"` adds a WezTerm-style tab strip with a clock under a bare window.
    pub tab_bar: TabBar,
    /// `#RRGGBB` colors of the tab strip: its background, the active tab, inactive tabs.
    pub tab_bar_bg: Option<String>,
    pub tab_active_bg: Option<String>,
    pub tab_active_fg: Option<String>,
    pub tab_inactive_bg: Option<String>,
    pub tab_inactive_fg: Option<String>,
    /// Open windows maximized (it wins over `initial_cols`/`initial_rows`).
    pub maximized: bool,
    /// Open windows in native full screen (macOS: the Dock and menu bar hide; ⌘⌃F leaves it).
    pub fullscreen: bool,
    /// Round the corners of a bare window and give it a soft edge, in points (0 = square).
    pub corner_radius: f32,
    /// Fine grain over the background, 0 (none) to 100; WezTerm's `noise = 10` is about 10.
    pub background_noise: f32,
    /// Open windows this many columns by rows wide (both or neither), instead of the last size.
    pub initial_cols: Option<u16>,
    pub initial_rows: Option<u16>,
    /// Every `teryon desktop` window starts a new shell and ends it when the window closes, instead of
    /// reattaching to the shared `default` workspace (naming a workspace with `-w` still attaches to it).
    pub fresh_window: bool,
    pub scrollbar: Scrollbar,
    /// Ask before closing the window (shells keep running either way).
    pub confirm_quit: bool,
    /// Key bindings: action name -> chord such as "cmd+shift+d"; "none" unbinds.
    pub keys: BTreeMap<String, String>,
    /// Saved commands shown in the palette as "Snippet: NAME".
    pub snippets: Vec<Snippet>,
    /// Rules that highlight, notify or beep when output matches.
    pub triggers: Vec<Trigger>,
}
impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size: 13.,
            line_height: 1.45,
            padding: 8.,
            padding_x: None,
            padding_top: None,
            padding_bottom: None,
            pane_inset: 10.,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            cursor_color: None,
            text_gamma: 0.55,
            window_blur: 0,
            background_gradient: Vec::new(),
            gradient_angle: 135.,
            window_opacity: 1.,
            unfocused_opacity: 0.82,
            copy_on_select: false,
            notify_after_seconds: 10,
            agent_notifications: true,
            file_browser: false,
            theme_light: None,
            theme_dark: None,
            scrollback_lines: 10_000,
            pane_titles: PaneTitles::Auto,
            chrome: Chrome::Full,
            tab_bar: TabBar::Off,
            maximized: false,
            fullscreen: false,
            corner_radius: 0.,
            status_line: true,
            background_noise: 0.,
            tab_bar_bg: None,
            tab_active_bg: None,
            tab_active_fg: None,
            tab_inactive_bg: None,
            tab_inactive_fg: None,
            initial_cols: None,
            initial_rows: None,
            fresh_window: false,
            scrollbar: Scrollbar::Auto,
            confirm_quit: false,
            keys: BTreeMap::new(),
            snippets: Vec::new(),
            triggers: Vec::new(),
        }
    }
}
impl DesktopConfig {
    /// Text padding as `(left, top, right, bottom)` in pixels.
    pub fn insets(&self) -> (f32, f32, f32, f32) {
        let x = self.padding_x.unwrap_or(self.padding);
        (
            x,
            self.padding_top.unwrap_or(self.padding),
            x,
            self.padding_bottom.unwrap_or(self.padding),
        )
    }
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
        for (name, v) in [
            ("padding_x", self.padding_x),
            ("padding_top", self.padding_top),
            ("padding_bottom", self.padding_bottom),
        ] {
            if let Some(v) = v {
                check(name, v, 0., 48.)?;
            }
        }
        check("pane_inset", self.pane_inset, 0., 32.)?;
        check("window_opacity", self.window_opacity, 0.3, 1.)?;
        check("unfocused_opacity", self.unfocused_opacity, 0.2, 1.)?;
        check("corner_radius", self.corner_radius, 0., 24.)?;
        check("background_noise", self.background_noise, 0., 100.)?;
        check("text_gamma", self.text_gamma, 0.3, 1.)?;
        check("window_blur", self.window_blur as f32, 0., 100.)?;
        check("gradient_angle", self.gradient_angle, -360., 360.)?;
        let is_hex = |c: &str| {
            c.strip_prefix('#')
                .is_some_and(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        };
        let tab_colors = [
            &self.tab_bar_bg,
            &self.tab_active_bg,
            &self.tab_active_fg,
            &self.tab_inactive_bg,
            &self.tab_inactive_fg,
        ];
        for c in self
            .cursor_color
            .iter()
            .chain(&self.background_gradient)
            .chain(tab_colors.into_iter().flatten())
        {
            if !is_hex(c) {
                bail!("desktop colors must look like #RRGGBB, not {c:?}");
            }
        }
        if self.background_gradient.len() == 1 {
            bail!("desktop.background_gradient needs at least two colors");
        }
        match (self.initial_cols, self.initial_rows) {
            (None, None) => {}
            (Some(c), Some(r)) if (20..=500).contains(&c) && (5..=200).contains(&r) => {}
            (Some(_), Some(_)) => {
                bail!("desktop.initial_cols must be 20..500 and initial_rows 5..200")
            }
            _ => bail!("desktop.initial_cols and desktop.initial_rows go together"),
        }
        if self.notify_after_seconds > 86_400 {
            bail!("desktop.notify_after_seconds must be at most 86400");
        }
        if !(100..=1_000_000).contains(&self.scrollback_lines) {
            bail!("desktop.scrollback_lines must be between 100 and 1000000");
        }
        for (key, name) in [
            ("theme_light", &self.theme_light),
            ("theme_dark", &self.theme_dark),
        ] {
            if let Some(n) = name
                .as_deref()
                .filter(|n| crate::theme::Theme::load(n).is_err())
            {
                bail!("desktop.{key} = \"{n}\" is not a known theme");
            }
        }
        if self.snippets.len() > 200 {
            bail!("desktop.snippets: at most 200 snippets");
        }
        let mut seen = std::collections::BTreeSet::new();
        for sn in &self.snippets {
            if sn.name.trim().is_empty() || sn.name.len() > 60 {
                bail!("desktop.snippets: every snippet needs a name of 1 to 60 characters");
            }
            if !seen.insert(sn.name.clone()) {
                bail!("desktop.snippets: duplicate name \"{}\"", sn.name);
            }
            if sn.command.trim().is_empty() || sn.command.len() > 4000 {
                bail!(
                    "desktop.snippets: \"{}\" needs a command of up to 4000 characters",
                    sn.name
                );
            }
            crate::snippet::validate(&sn.command)
                .map_err(|e| anyhow::anyhow!("desktop.snippets: \"{}\": {e}", sn.name))?;
        }
        if self.triggers.len() > 30 {
            bail!("desktop.triggers: at most 30 triggers");
        }
        for t in &self.triggers {
            if t.pattern.is_empty() || t.pattern.len() > 500 {
                bail!("desktop.triggers: a pattern needs 1 to 500 characters");
            }
            regex::Regex::new(&t.pattern).map_err(|e| {
                anyhow::anyhow!(
                    "desktop.triggers: \"{}\": {}",
                    t.pattern,
                    e.to_string().lines().last().unwrap_or("invalid").trim()
                )
            })?;
            if let Some(c) = &t.color {
                let ok = c.len() == 7
                    && c.starts_with('#')
                    && c[1..].chars().all(|d| d.is_ascii_hexdigit());
                if !ok {
                    bail!("desktop.triggers: color \"{c}\" must be #RRGGBB");
                }
            }
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
    "ion".into()
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
            start: StartMode::Tui,
            ai: AiConfig::default(),
            shell: ShellConfig::default(),
            notify: NotifyConfig::default(),
        });
    }
    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
}
/// Parse and validate the desktop section of `text` (a whole `config.toml`).
pub fn parse_desktop(text: &str) -> Result<DesktopConfig> {
    let cfg: Config = toml::from_str(text)?;
    cfg.desktop.validate()?;
    cfg.ai.validate()?;
    Ok(cfg.desktop)
}
/// The validated `[desktop]` options from the user's config file (defaults when there is none).
pub fn load_desktop() -> Result<DesktopConfig> {
    load_desktop_theme().map(|(cfg, _)| cfg)
}
/// `load_desktop` plus the top-level `theme`, so a running app can follow edits to it.
pub fn load_desktop_theme() -> Result<(DesktopConfig, String)> {
    let cfg = load()?;
    cfg.desktop.validate()?;
    cfg.ai.validate()?;
    cfg.notify.validate()?;
    Ok((cfg.desktop, cfg.theme))
}
#[cfg(test)]
mod notify_tests {
    use super::*;
    #[test]
    fn notify_section_parses_and_is_validated() {
        let cfg: Config = toml::from_str(
            "[notify]\nntfy_url = \"https://ntfy.sh/mine\"\nafter_seconds = 60\n",
        )
        .unwrap();
        assert!(cfg.notify.active() && cfg.notify.agents);
        cfg.notify.validate().unwrap();
        assert!(!Config::default().notify.active());
        for bad in [
            "[notify]\nwebhook_url = \"ftp://x\"\n",
            "[notify]\nntfy_url = \"https://a b\"\n",
            "[notify]\nafter_seconds = 0\n",
        ] {
            let cfg: Config = toml::from_str(bad).unwrap();
            assert!(cfg.notify.validate().is_err(), "{bad}");
        }
        assert!(toml::from_str::<Config>("[notify]\nbogus = 1\n").is_err());
    }
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
        write_new(&format!("theme = \"{}\"\n", default_theme()))?;
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
/// Create the config file with `text`; refuses to replace an existing one.
fn write_new(text: &str) -> Result<PathBuf> {
    let path = path()?;
    if path.exists() {
        bail!("configuration already exists: {}", path.display());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    util::private_write(&path, text.as_bytes())?;
    Ok(path)
}
/// `text` with its top-level `theme = "..."` line set to `name` (added first if missing).
pub fn set_theme_text(text: &str, name: &str) -> String {
    let line = format!("theme = \"{name}\"");
    let mut done = false;
    let mut in_table = false;
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        if l.trim_start().starts_with('[') {
            in_table = true;
        }
        if !done && !in_table && l.trim_start().starts_with("theme") && l.contains('=') {
            out.push(line.clone());
            done = true;
        } else {
            out.push(l.to_string());
        }
    }
    if !done {
        out.insert(0, line);
    }
    out.join("\n") + "\n"
}
/// Persist `name` as the default theme in `config.toml`, creating the file if needed.
pub fn set_theme(name: &str) -> Result<()> {
    let path = path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    util::write_private_atomic(&path, set_theme_text(&text, name).as_bytes())
}
/// The target for `host`: a configured alias or a plain `[user@]host`.
pub fn target(host: &str, remote_bin: Option<String>) -> Result<crate::transport::Target> {
    let cfg = load()?;
    match cfg.hosts.get(host) {
        Some(h) => crate::transport::Target::new(
            h.destination.clone(),
            remote_bin.or_else(|| h.remote_bin.clone()),
        ),
        None => crate::transport::Target::new(host.to_string(), remote_bin),
    }
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
    fn chrome_and_initial_size_are_validated_together() {
        let ok =
            parse_desktop("[desktop]\nchrome = \"none\"\ninitial_cols = 124\ninitial_rows = 42\n")
                .unwrap();
        assert_eq!(
            (ok.chrome, ok.initial_cols, ok.initial_rows),
            (Chrome::None, Some(124), Some(42))
        );
        assert_eq!(parse_desktop("").unwrap().chrome, Chrome::Full);
        assert!(parse_desktop("[desktop]\ninitial_cols = 124\n").is_err());
        assert!(parse_desktop("[desktop]\ninitial_cols = 5\ninitial_rows = 42\n").is_err());
        assert!(parse_desktop("[desktop]\nchrome = \"minimal\"\n").is_err());
    }
    #[test]
    fn the_scrollbar_defaults_to_auto() {
        assert_eq!(parse_desktop("").unwrap().scrollbar, Scrollbar::Auto);
        let c = parse_desktop("[desktop]\nscrollbar = \"always\"\n").unwrap();
        assert_eq!(c.scrollbar, Scrollbar::Always);
        assert!(parse_desktop("[desktop]\nscrollbar = \"sometimes\"\n").is_err());
    }
    #[test]
    fn padding_can_differ_per_side() {
        assert_eq!(parse_desktop("").unwrap().insets(), (8., 8., 8., 8.));
        let c =
            parse_desktop("[desktop]\npadding = 10.0\npadding_top = 0.0\npadding_bottom = 5.0\n")
                .unwrap();
        assert_eq!(c.insets(), (10., 0., 10., 5.));
        let c = parse_desktop("[desktop]\npadding_x = 12.0\n").unwrap();
        assert_eq!(c.insets(), (12., 8., 12., 8.));
        assert!(parse_desktop("[desktop]\npadding_top = 99.0\n").is_err());
    }
    #[test]
    fn fresh_windows_are_opt_in() {
        assert!(!parse_desktop("").unwrap().fresh_window);
        assert!(
            parse_desktop("[desktop]\nfresh_window = true\n")
                .unwrap()
                .fresh_window
        );
    }
    #[test]
    fn the_shell_section_defaults_to_everything_on_except_the_prompt() {
        let d: Config = toml::from_str("").unwrap();
        assert!(d.shell.enabled && d.shell.tab && d.shell.jump && d.shell.history);
        assert!(!d.shell.prompt);
        let c: Config = toml::from_str("[shell]\nprompt = true\ntab = false\n").unwrap();
        assert!(c.shell.prompt && !c.shell.tab && c.shell.suggest);
        assert!(toml::from_str::<Config>("[shell]\nbogus = true\n").is_err());
    }
    #[test]
    fn desktop_section_defaults_validates_and_rejects_unknown_keys() {
        assert_eq!(parse_desktop("").unwrap(), DesktopConfig::default());
        let ok = parse_desktop("[desktop]\nfont_size = 15.0\ncursor_style = \"bar\"\n[desktop.keys]\npalette = \"cmd+p\"\n").unwrap();
        assert_eq!((ok.font_size, ok.cursor_style), (15., CursorStyle::Bar));
        assert!(parse_desktop("[desktop]\nfont_sise = 15.0\n").is_err());
        assert!(parse_desktop("[desktop]\nfont_size = 2.0\n").is_err());
        assert!(parse_desktop("[desktop]\ncursor_style = \"blob\"\n").is_err());
        assert!(parse_desktop("[desktop]\ntheme_dark = \"no-such-theme\"\n").is_err());
        assert!(
            parse_desktop("[desktop]\ntheme_dark = \"nord\"\ntheme_light = \"pearl\"\n").is_ok()
        );
        assert!(parse_desktop("[desktop.keys]\nnope = \"cmd+x\"\n").is_err());
        let trig = "[[desktop.triggers]]\npattern = \"ERROR|panic\"\naction = \"notify\"\n";
        assert_eq!(
            parse_desktop(trig).unwrap().triggers[0].action,
            TriggerAction::Notify
        );
        assert!(parse_desktop("[[desktop.triggers]]\npattern = \"(\"\n").is_err());
        assert!(parse_desktop("[[desktop.triggers]]\npattern = \"x\"\ncolor = \"red\"\n").is_err());
        assert!(
            parse_desktop("[[desktop.triggers]]\npattern = \"x\"\naction = \"explode\"\n").is_err()
        );
        let sn = "[[desktop.snippets]]\nname = \"logs\"\ncommand = \"tail -f {{file}}\"\n";
        assert_eq!(parse_desktop(sn).unwrap().snippets[0].name, "logs");
        assert!(
            parse_desktop(&format!("{sn}{sn}")).is_err(),
            "duplicate names"
        );
        assert!(
            parse_desktop("[[desktop.snippets]]\nname = \"x\"\ncommand = \"a {{b\"\n").is_err()
        );
        assert!(
            parse_desktop("[[desktop.snippets]]\nname = \"x\"\ncommand = \"ls\"\nbogus = 1\n")
                .is_err()
        );
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
    fn set_theme_text_replaces_or_prepends_without_touching_tables() {
        let a = set_theme_text("theme = \"nord\"\nsplash = true\n", "dracula");
        assert_eq!(a, "theme = \"dracula\"\nsplash = true\n");
        let b = set_theme_text("splash = true\n[hosts.x]\ntheme = \"inner\"\n", "nord");
        assert_eq!(
            b,
            "theme = \"nord\"\nsplash = true\n[hosts.x]\ntheme = \"inner\"\n"
        );
        assert_eq!(set_theme_text("", "nord"), "theme = \"nord\"\n");
    }
    #[test]
    fn ssh_hosts_skips_wildcards_and_comments() {
        let cfg = "Host mini # home\n  HostName 10.0.0.2\nhost *\nHost a b !c *.x\nMatch all\nHost mini\n";
        assert_eq!(ssh_hosts(cfg), ["mini", "a", "b"]);
    }
}

#[cfg(test)]
mod ai_tests {
    use super::*;
    #[test]
    fn ai_section_defaults_and_validation() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.ai.enabled && cfg.ai.validate().is_ok());
        assert_eq!(cfg.ai.model, "claude-sonnet-5-5");
        let ok: Config =
            toml::from_str("[ai]\nmodel = \"claude-opus-5-5\"\nauto_allow = [\"mac_info\"]\n")
                .unwrap();
        assert!(ok.ai.validate().is_ok());
        for bad in [
            "[ai]\nauto_allow = [\"run_command\"]\n",
            "[ai]\nmax_tokens = 5\n",
            "[ai]\nbase_url = \"http://evil.example\"\n",
            "[ai]\nmodel = \"has space\"\n",
        ] {
            let cfg: Config = toml::from_str(bad).unwrap();
            assert!(cfg.ai.validate().is_err(), "{bad}");
        }
        assert!(
            toml::from_str::<Config>("[ai]\nkey = \"sk-ant-x\"\n").is_err(),
            "a key must never be stored in config"
        );
    }
    #[test]
    fn colors_blur_and_gradient_are_validated() {
        let ok = parse_desktop(
            "[desktop]\ncursor_color = \"#82aaff\"\nwindow_blur = 40\nbackground_gradient = [\"#16161e\", \"#2a2b3b\"]\n",
        )
        .unwrap();
        assert_eq!((ok.window_blur, ok.background_gradient.len()), (40, 2));
        for bad in [
            "cursor_color = \"blue\"",
            "window_blur = 101",
            "background_gradient = [\"#16161e\"]",
            "background_gradient = [\"#16161e\", \"nope\"]",
        ] {
            assert!(
                parse_desktop(&format!("[desktop]\n{bad}\n")).is_err(),
                "{bad}"
            );
        }
    }
}

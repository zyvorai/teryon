use anyhow::{bail, Result};
use ratatui::style::Color;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSpec {
    pub name: String,
    pub background: String,
    pub panel: String,
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    pub success: String,
    pub warning: String,
    pub border: String,
    /// Solid fill for primary buttons; defaults to `accent`.
    #[serde(default)]
    pub primary: Option<String>,
    /// Sixteen `#RRGGBB` colors for ANSI 0-15 (what `ls`, `git` and editors use); derived when absent.
    #[serde(default)]
    pub ansi: Option<Vec<String>>,
}
#[derive(Clone)]
pub struct Theme {
    pub name: String,
    pub background: Color,
    pub panel: Color,
    pub foreground: Color,
    pub muted: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub border: Color,
    pub primary: Color,
    /// ANSI colors 0-15: black, red, green, yellow, blue, magenta, cyan, white, then bright variants.
    pub ansi: [Color; 16],
}
pub const BUILTINS: [&str; 20] = [
    "mac27-dark",
    "mac27-light",
    "apple-dark",
    "apple-light",
    "tokyo-night",
    "tokyo-night-storm",
    "catppuccin-mocha",
    "dracula",
    "gruvbox-dark",
    "nord",
    "rose-pine",
    "solarized-dark",
    "obsidian",
    "pearl",
    "aurora",
    "ember",
    "ocean",
    "sakura",
    "forest",
    "high-contrast",
];
/// Folder for themes the user added or imported (`teryon themes --import`).
pub fn user_dir() -> Option<std::path::PathBuf> {
    Some(crate::util::home().ok()?.join(".config/teryon/themes"))
}
fn simple_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
/// Where a bare theme name would live in the user folder (None for names with path characters).
pub fn user_theme_path(name: &str) -> Option<std::path::PathBuf> {
    simple_name(name).then(|| user_dir().map(|d| d.join(format!("{name}.toml"))))?
}
/// Names of the user's own themes, sorted.
pub fn user_themes() -> Vec<String> {
    let Some(dir) = user_dir() else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "toml").then(|| p.file_stem()?.to_str().map(str::to_string))?
        })
        .filter(|n| simple_name(n) && !BUILTINS.contains(&n.as_str()))
        .collect();
    names.sort();
    names
}
/// Built-in theme ids followed by the user's own.
pub fn all_names() -> Vec<String> {
    BUILTINS
        .iter()
        .map(|s| (*s).to_string())
        .chain(user_themes())
        .collect()
}
/// Whether `name` can be used as the file name of an imported theme.
pub fn valid_user_name(name: &str) -> bool {
    simple_name(name) && !BUILTINS.contains(&name)
}
fn hex(value: &str) -> Result<Color> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("theme colors must be #RRGGBB: {value}");
    }
    Ok(Color::Rgb(
        u8::from_str_radix(&value[1..3], 16)?,
        u8::from_str_radix(&value[3..5], 16)?,
        u8::from_str_radix(&value[5..7], 16)?,
    ))
}
fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    }
}
/// Linear blend of two colors; `t` = 0 gives `a`, 1 gives `b`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let (a, b) = (rgb(a), rgb(b));
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color::Rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}
fn is_light(c: Color) -> bool {
    let (r, g, b) = rgb(c);
    u16::from(r) + u16::from(g) + u16::from(b) > 450
}
/// ANSI palette built from the eight theme colors, for themes that do not define one.
fn derive_ansi(t: &Theme) -> [Color; 16] {
    let light = is_light(t.background);
    let red = if light {
        Color::Rgb(0xd7, 0x00, 0x15)
    } else {
        Color::Rgb(0xff, 0x6b, 0x6b)
    };
    let magenta = mix(t.accent, red, 0.45);
    let cyan = mix(t.accent, t.success, 0.5);
    let base = [
        t.border, red, t.success, t.warning, t.accent, magenta, cyan, t.muted,
    ];
    let mut out = [t.foreground; 16];
    for (i, c) in base.into_iter().enumerate() {
        out[i] = c;
        out[i + 8] = mix(c, t.foreground, 0.22);
    }
    out[8] = mix(t.muted, t.background, 0.4);
    out[15] = t.foreground;
    out
}
fn to_hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "#ffffff".into(),
    }
}
/// WCAG contrast of white text on `c`.
fn white_contrast(c: Color) -> f64 {
    let (r, g, b) = rgb(c);
    let ch = |n: u8| {
        let n = f64::from(n) / 255.;
        if n <= 0.04045 {
            n / 12.92
        } else {
            ((n + 0.055) / 1.055).powf(2.4)
        }
    };
    1.05 / (0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b) + 0.05)
}
/// macOS accent-color setting (`AppleAccentColor`) to a system color; `None` keeps the theme's blue.
pub fn accent_for_index(index: Option<i32>, light: bool) -> Option<Color> {
    let (dark, lit) = match index? {
        -1 => ((0x98, 0x98, 0x9d), (0x8e, 0x8e, 0x93)),
        0 => ((0xff, 0x45, 0x3a), (0xff, 0x3b, 0x30)),
        1 => ((0xff, 0x9f, 0x0a), (0xff, 0x95, 0x00)),
        2 => ((0xff, 0xd6, 0x0a), (0xff, 0xcc, 0x00)),
        3 => ((0x32, 0xd7, 0x4b), (0x34, 0xc7, 0x59)),
        5 => ((0xbf, 0x5a, 0xf2), (0xaf, 0x52, 0xde)),
        6 => ((0xff, 0x37, 0x5f), (0xff, 0x2d, 0x55)),
        _ => return None,
    };
    let (r, g, b) = if light { lit } else { dark };
    Some(Color::Rgb(r, g, b))
}
/// The user's macOS accent-color setting, if one is chosen (absent means the default blue).
pub fn system_accent_index() -> Option<i32> {
    static INDEX: std::sync::OnceLock<Option<i32>> = std::sync::OnceLock::new();
    *INDEX.get_or_init(|| {
        let out = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleAccentColor"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    })
}
impl Theme {
    /// Follow the macOS accent color; only the built-in Mac27 themes opt in.
    pub fn follow_system_accent(&mut self, index: Option<i32>) {
        if !self.name.starts_with("Mac27") {
            return;
        }
        let Some(accent) = accent_for_index(index, is_light(self.background)) else {
            return;
        };
        self.accent = accent;
        // Buttons carry white text, so darken the fill until it stays readable.
        let mut primary = accent;
        for _ in 0..12 {
            if white_contrast(primary) >= 4.5 {
                break;
            }
            primary = mix(primary, Color::Rgb(0, 0, 0), 0.1);
        }
        self.primary = primary;
    }
    /// A WezTerm `config.colors` block; paste it into wezterm.lua after `config` is created.
    pub fn to_wezterm_lua(&self) -> String {
        let list = |colors: &[Color]| {
            colors
                .iter()
                .map(|c| format!("\"{}\"", to_hex(*c)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let font = if self.name.starts_with("Apple") || self.name.starts_with("Mac27") {
            "-- config.font = wezterm.font(\"SF Mono\") -- install SF Mono from developer.apple.com/fonts; Menlo is built in\n"
        } else {
            ""
        };
        format!(
            "-- {name} (exported by teryon)\n{font}config.colors = {{\n  foreground = \"{fg}\",\n  background = \"{bg}\",\n  cursor_bg = \"{accent}\",\n  cursor_fg = \"{bg}\",\n  cursor_border = \"{accent}\",\n  selection_bg = \"{border}\",\n  selection_fg = \"{fg}\",\n  scrollbar_thumb = \"{border}\",\n  split = \"{border}\",\n  ansi = {{ {ansi} }},\n  brights = {{ {brights} }},\n}}\n",
            name = self.name,
            fg = to_hex(self.foreground),
            bg = to_hex(self.background),
            accent = to_hex(self.accent),
            border = to_hex(self.border),
            ansi = list(&self.ansi[..8]),
            brights = list(&self.ansi[8..]),
        )
    }
    /// Map a terminal color index to this theme; indexes above 15 keep the standard 256-color cube.
    pub fn indexed(&self, n: u8) -> Color {
        if n < 16 {
            self.ansi[usize::from(n)]
        } else {
            Color::Indexed(n)
        }
    }
    /// `load`, then follow the macOS accent color (Mac27 themes only).
    pub fn load_native(name: &str) -> Result<Self> {
        let mut theme = Self::load(name)?;
        theme.follow_system_accent(system_accent_index());
        Ok(theme)
    }
    pub fn load(name: &str) -> Result<Self> {
        let content = match name {
            "mac27-dark" => include_str!("../themes/mac27-dark.toml"),
            "mac27-light" => include_str!("../themes/mac27-light.toml"),
            "apple-dark" => include_str!("../themes/apple-dark.toml"),
            "apple-light" => include_str!("../themes/apple-light.toml"),
            "tokyo-night" => include_str!("../themes/tokyo-night.toml"),
            "tokyo-night-storm" => include_str!("../themes/tokyo-night-storm.toml"),
            "catppuccin-mocha" => include_str!("../themes/catppuccin-mocha.toml"),
            "dracula" => include_str!("../themes/dracula.toml"),
            "gruvbox-dark" => include_str!("../themes/gruvbox-dark.toml"),
            "nord" => include_str!("../themes/nord.toml"),
            "rose-pine" => include_str!("../themes/rose-pine.toml"),
            "solarized-dark" => include_str!("../themes/solarized-dark.toml"),
            "obsidian" => include_str!("../themes/obsidian.toml"),
            "pearl" => include_str!("../themes/pearl.toml"),
            "aurora" => include_str!("../themes/aurora.toml"),
            "ember" => include_str!("../themes/ember.toml"),
            "ocean" => include_str!("../themes/ocean.toml"),
            "sakura" => include_str!("../themes/sakura.toml"),
            "forest" => include_str!("../themes/forest.toml"),
            "high-contrast" => include_str!("../themes/high-contrast.toml"),
            other => {
                if let Some(path) = user_theme_path(other).filter(|p| p.is_file()) {
                    return Self::parse(&std::fs::read_to_string(path)?);
                }
                return Self::parse(&std::fs::read_to_string(other)?);
            }
        };
        Self::parse(content)
    }
    pub fn parse(content: &str) -> Result<Self> {
        let s: ThemeSpec = toml::from_str(content)?;
        let mut theme = Self {
            name: s.name,
            background: hex(&s.background)?,
            panel: hex(&s.panel)?,
            foreground: hex(&s.foreground)?,
            muted: hex(&s.muted)?,
            accent: hex(&s.accent)?,
            success: hex(&s.success)?,
            warning: hex(&s.warning)?,
            border: hex(&s.border)?,
            primary: match &s.primary {
                Some(p) => hex(p)?,
                None => hex(&s.accent)?,
            },
            ansi: [Color::Reset; 16],
        };
        theme.ansi = match &s.ansi {
            Some(list) => {
                if list.len() != 16 {
                    bail!("ansi needs exactly 16 colors, found {}", list.len());
                }
                let mut out = [Color::Reset; 16];
                for (slot, value) in out.iter_mut().zip(list) {
                    *slot = hex(value)?;
                }
                out
            }
            None => derive_ansi(&theme),
        };
        Ok(theme)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_builtin_themes_parse() {
        for name in BUILTINS {
            assert!(Theme::load(name).is_ok(), "{name}");
        }
    }
    #[test]
    fn wezterm_export_contains_theme_colors() {
        let lua = Theme::load("tokyo-night").unwrap().to_wezterm_lua();
        assert!(lua.contains("background = \"#1a1b26\""));
        assert!(lua.contains("ansi = { \"#414868\", \"#f7768e\""));
        assert!(!lua.contains("config.font"));
        let apple = Theme::load("apple-dark").unwrap().to_wezterm_lua();
        assert!(apple.contains("-- config.font = wezterm.font(\"SF Mono\")"));
        assert!(lua.contains("foreground = \"#c0caf5\""));
        assert!(lua.starts_with("-- Tokyo Night (exported by teryon)"));
    }
    #[test]
    fn primary_buttons_keep_white_text_readable() {
        fn lum(c: Color) -> f64 {
            let (r, g, b) = rgb(c);
            let ch = |n: u8| {
                let n = f64::from(n) / 255.;
                if n <= 0.04045 {
                    n / 12.92
                } else {
                    ((n + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
        }
        for name in ["mac27-dark", "mac27-light", "apple-dark", "apple-light"] {
            let t = Theme::load(name).unwrap();
            assert!(1.05 / (lum(t.primary) + 0.05) >= 4.5, "{name}");
        }
    }
    #[test]
    fn ansi_palette_is_complete_and_validated() {
        for name in BUILTINS {
            let t = Theme::load(name).unwrap();
            assert!(t.ansi.iter().all(|c| matches!(c, Color::Rgb(..))), "{name}");
            assert_eq!(t.indexed(4), t.ansi[4]);
            assert_eq!(t.indexed(200), Color::Indexed(200));
        }
        let bad = "name='x'\nbackground='#000000'\npanel='#000000'\nforeground='#ffffff'\nmuted='#ffffff'\naccent='#ffffff'\nsuccess='#ffffff'\nwarning='#ffffff'\nborder='#ffffff'\nansi=['#ffffff']";
        assert!(Theme::parse(bad).is_err());
    }
    #[test]
    fn system_accent_applies_only_to_mac27_and_keeps_buttons_readable() {
        for index in [-1, 0, 1, 2, 3, 5, 6] {
            let mut t = Theme::load("mac27-dark").unwrap();
            t.follow_system_accent(Some(index));
            assert!(white_contrast(t.primary) >= 4.5, "{index}");
        }
        let mut t = Theme::load("mac27-light").unwrap();
        t.follow_system_accent(Some(0));
        assert_eq!(t.accent, Color::Rgb(0xff, 0x3b, 0x30));
        let mut other = Theme::load("nord").unwrap();
        let before = other.accent;
        other.follow_system_accent(Some(0));
        assert_eq!(other.accent, before);
        let mut blue = Theme::load("mac27-dark").unwrap();
        blue.follow_system_accent(None);
        assert_eq!(blue.accent, Color::Rgb(0x0a, 0x84, 0xff));
    }
    #[test]
    fn colors_reject_bad_values() {
        for s in ["red", "#FFF", "#zz0000", "#☃☃"] {
            assert!(hex(s).is_err());
        }
    }
    #[test]
    fn theme_text_has_readable_contrast() {
        fn luminance(c: Color) -> f64 {
            let Color::Rgb(r, g, b) = c else { panic!() };
            let channel = |n: u8| {
                let n = f64::from(n) / 255.;
                if n <= 0.04045 {
                    n / 12.92
                } else {
                    ((n + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
        }
        for name in BUILTINS {
            let t = Theme::load(name).unwrap();
            for text in [t.foreground, t.muted] {
                for background in [t.background, t.panel] {
                    let a = luminance(text);
                    let b = luminance(background);
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(contrast >= 4.5, "{name}: {contrast}");
                }
            }
        }
    }
}

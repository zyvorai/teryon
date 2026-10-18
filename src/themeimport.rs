//! Convert color schemes from other terminals (iTerm2, Alacritty, Ghostty, Kitty) to TERYON themes.
use crate::theme::ThemeSpec;
use anyhow::{bail, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    ITerm,
    Alacritty,
    Ghostty,
    Kitty,
}
type Rgb = (u8, u8, u8);

/// Guess the format from the file name and content.
pub fn detect(file_name: &str, text: &str) -> Option<Format> {
    let lower = file_name.to_lowercase();
    if lower.ends_with(".itermcolors") || text.contains("<plist") {
        return Some(Format::ITerm);
    }
    if lower.ends_with(".toml") || text.contains("[colors") {
        return Some(Format::Alacritty);
    }
    if text.lines().any(|l| l.trim_start().starts_with("palette")) || lower.contains("ghostty") {
        return Some(Format::Ghostty);
    }
    if text.lines().any(|l| l.trim_start().starts_with("color0")) || lower.ends_with(".conf") {
        return Some(Format::Kitty);
    }
    None
}
fn hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}
fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_matches(['"', '\'']);
    let s = s
        .strip_prefix('#')
        .or_else(|| s.strip_prefix("0x"))
        .unwrap_or(s);
    let full = match s.len() {
        3 => s.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => s.to_string(),
        _ => return None,
    };
    let n = u32::from_str_radix(&full, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
fn lum(c: Rgb) -> f64 {
    let ch = |n: u8| {
        let n = f64::from(n) / 255.;
        if n <= 0.04045 {
            n / 12.92
        } else {
            ((n + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c.0) + 0.7152 * ch(c.1) + 0.0722 * ch(c.2)
}
fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let f = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}
/// xterm-style defaults for palette slots a scheme leaves out.
const DEFAULT_ANSI: [Rgb; 16] = [
    (0, 0, 0),
    (205, 49, 49),
    (13, 188, 121),
    (229, 229, 16),
    (36, 114, 200),
    (188, 63, 188),
    (17, 168, 205),
    (229, 229, 229),
    (102, 102, 102),
    (241, 76, 76),
    (35, 209, 139),
    (245, 245, 67),
    (59, 142, 234),
    (214, 112, 214),
    (41, 184, 219),
    (255, 255, 255),
];
#[derive(Default)]
struct Raw {
    background: Option<Rgb>,
    foreground: Option<Rgb>,
    cursor: Option<Rgb>,
    ansi: [Option<Rgb>; 16],
}

/// Pull `<key>NAME</key><dict>` color dictionaries out of an iTerm2 property list.
fn parse_iterm(text: &str) -> Raw {
    let mut raw = Raw::default();
    let color_of = |name: &str| -> Option<Rgb> {
        let at = text.find(&format!("<key>{name}</key>"))?;
        let rest = &text[at..];
        let end = rest.find("</dict>")?;
        let block = &rest[..end];
        let component = |key: &str| -> Option<u8> {
            let k = block.find(&format!("<key>{key}</key>"))?;
            let after = &block[k..];
            let v = after.split("<real>").nth(1)?.split("</real>").next()?;
            let f: f64 = v.trim().parse().ok()?;
            Some((f.clamp(0., 1.) * 255.).round() as u8)
        };
        Some((
            component("Red Component")?,
            component("Green Component")?,
            component("Blue Component")?,
        ))
    };
    raw.background = color_of("Background Color");
    raw.foreground = color_of("Foreground Color");
    raw.cursor = color_of("Cursor Color");
    for (i, slot) in raw.ansi.iter_mut().enumerate() {
        *slot = color_of(&format!("Ansi {i} Color"));
    }
    raw
}
fn parse_alacritty(text: &str) -> Result<Raw> {
    let v: toml::Value = toml::from_str(text)?;
    let colors = v.get("colors").unwrap_or(&v);
    let get = |path: &[&str]| -> Option<Rgb> {
        let mut cur = colors;
        for p in path {
            cur = cur.get(p)?;
        }
        parse_color(cur.as_str()?)
    };
    let mut raw = Raw {
        background: get(&["primary", "background"]),
        foreground: get(&["primary", "foreground"]),
        cursor: get(&["cursor", "cursor"]),
        ..Raw::default()
    };
    let names = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    for (i, n) in names.iter().enumerate() {
        raw.ansi[i] = get(&["normal", n]);
        raw.ansi[i + 8] = get(&["bright", n]);
    }
    Ok(raw)
}
fn parse_ghostty(text: &str) -> Raw {
    let mut raw = Raw::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "background" => raw.background = parse_color(v),
            "foreground" => raw.foreground = parse_color(v),
            "cursor-color" => raw.cursor = parse_color(v),
            "palette" => {
                if let Some((i, c)) = v.split_once('=') {
                    if let (Ok(i), Some(c)) = (i.trim().parse::<usize>(), parse_color(c)) {
                        if i < 16 {
                            raw.ansi[i] = Some(c);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    raw
}
fn parse_kitty(text: &str) -> Raw {
    let mut raw = Raw::default();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(k), Some(v)) = (parts.next(), parts.next()) else {
            continue;
        };
        match k {
            "background" => raw.background = parse_color(v),
            "foreground" => raw.foreground = parse_color(v),
            "cursor" => raw.cursor = parse_color(v),
            _ => {
                if let Some(i) = k
                    .strip_prefix("color")
                    .and_then(|d| d.parse::<usize>().ok())
                {
                    if i < 16 {
                        raw.ansi[i] = parse_color(v);
                    }
                }
            }
        }
    }
    raw
}

/// Build a theme from `text`; the returned strings are things worth telling the user.
pub fn import(text: &str, format: Format, name: &str) -> Result<(ThemeSpec, Vec<String>)> {
    let raw = match format {
        Format::ITerm => parse_iterm(text),
        Format::Alacritty => parse_alacritty(text)?,
        Format::Ghostty => parse_ghostty(text),
        Format::Kitty => parse_kitty(text),
    };
    let (Some(bg), Some(fg)) = (raw.background, raw.foreground) else {
        bail!("no background/foreground colors found; is this a {format:?} color scheme?");
    };
    let mut notes = Vec::new();
    let missing = raw.ansi.iter().filter(|c| c.is_none()).count();
    if missing > 0 {
        notes.push(format!(
            "{missing} of 16 palette colors were missing; xterm defaults fill the gaps"
        ));
    }
    let ansi: Vec<Rgb> = raw
        .ansi
        .iter()
        .enumerate()
        .map(|(i, c)| c.unwrap_or(DEFAULT_ANSI[i]))
        .collect();
    // Secondary text must stay readable on the background: move towards the foreground until it is.
    let mut muted = mix(bg, fg, 0.62);
    let mut t = 0.62;
    while contrast(muted, bg) < 4.5 && t < 1. {
        t += 0.05;
        muted = mix(bg, fg, t);
    }
    if contrast(fg, bg) < 4.5 {
        notes.push(format!(
            "foreground/background contrast is {:.1}:1 (below 4.5:1); text may be hard to read",
            contrast(fg, bg)
        ));
    }
    let spec = ThemeSpec {
        name: name.to_string(),
        background: hex(bg),
        panel: hex(mix(bg, fg, 0.07)),
        foreground: hex(fg),
        muted: hex(muted),
        accent: hex(raw
            .cursor
            .filter(|c| contrast(*c, bg) >= 3.)
            .unwrap_or(ansi[12])),
        success: hex(ansi[10]),
        warning: hex(ansi[11]),
        border: hex(mix(bg, fg, 0.2)),
        primary: None,
        ansi: Some(ansi.into_iter().map(hex).collect()),
    };
    Ok((spec, notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    fn load(spec: &ThemeSpec) -> Theme {
        Theme::parse(&toml::to_string(spec).unwrap()).unwrap()
    }
    #[test]
    fn colors_parse_in_common_spellings() {
        assert_eq!(parse_color("#1a1b26"), Some((0x1a, 0x1b, 0x26)));
        assert_eq!(parse_color("0xFFAA00"), Some((255, 170, 0)));
        assert_eq!(parse_color("\"#fff\""), Some((255, 255, 255)));
        assert_eq!(parse_color("nope"), None);
    }
    #[test]
    fn ghostty_and_kitty_files_import() {
        let g = "# Tokyo\nbackground = #1a1b26\nforeground = #c0caf5\ncursor-color = #c0caf5\npalette = 0=#15161e\npalette = 1=#f7768e\npalette = 4=#7aa2f7\n";
        let (spec, notes) = import(g, Format::Ghostty, "tn").unwrap();
        assert_eq!(spec.background, "#1A1B26");
        assert_eq!(spec.ansi.as_ref().unwrap()[1], "#F7768E");
        assert!(notes.iter().any(|n| n.contains("13 of 16")));
        assert_eq!(load(&spec).name, "tn");
        let k = "foreground #c0caf5\nbackground #1a1b26\ncolor0 #15161e\ncolor15 #c0caf5\n";
        let (spec, _) = import(k, Format::Kitty, "k").unwrap();
        assert_eq!(spec.ansi.as_ref().unwrap()[15], "#C0CAF5");
    }
    #[test]
    fn alacritty_toml_imports_with_and_without_the_colors_prefix() {
        let a = "[colors.primary]\nbackground = \"#282a36\"\nforeground = \"0xf8f8f2\"\n[colors.normal]\nred = \"#ff5555\"\n[colors.bright]\nblue = \"#d6acff\"\n";
        let (spec, _) = import(a, Format::Alacritty, "dr").unwrap();
        assert_eq!(spec.foreground, "#F8F8F2");
        assert_eq!(spec.ansi.as_ref().unwrap()[1], "#FF5555");
        assert_eq!(spec.ansi.as_ref().unwrap()[12], "#D6ACFF");
        assert_eq!(spec.accent, "#D6ACFF");
    }
    #[test]
    fn iterm_plists_import() {
        let comp = |name: &str, r: f64, g: f64, b: f64| {
            format!("<key>{name}</key><dict><key>Blue Component</key><real>{b}</real><key>Green Component</key><real>{g}</real><key>Red Component</key><real>{r}</real></dict>")
        };
        let plist = format!(
            "<plist><dict>{}{}{}</dict></plist>",
            comp("Ansi 1 Color", 1.0, 0.0, 0.0),
            comp("Background Color", 0.0, 0.0, 0.0),
            comp("Foreground Color", 1.0, 1.0, 1.0)
        );
        assert_eq!(detect("x.itermcolors", &plist), Some(Format::ITerm));
        let (spec, _) = import(&plist, Format::ITerm, "it").unwrap();
        assert_eq!(
            (spec.background.as_str(), spec.foreground.as_str()),
            ("#000000", "#FFFFFF")
        );
        assert_eq!(spec.ansi.as_ref().unwrap()[1], "#FF0000");
    }
    #[test]
    fn imported_themes_keep_secondary_text_readable_and_garbage_is_rejected() {
        let (spec, _) = import(
            "background = #000000\nforeground = #ffffff\n",
            Format::Ghostty,
            "t",
        )
        .unwrap();
        let theme = load(&spec);
        assert_eq!(theme.ansi.len(), 16);
        assert!(contrast(parse_color(&spec.muted).unwrap(), (0, 0, 0)) >= 4.5);
        assert!(import("hello", Format::Ghostty, "t").is_err());
        let (_, notes) = import(
            "background = #222222\nforeground = #333333\n",
            Format::Ghostty,
            "low",
        )
        .unwrap();
        assert!(notes.iter().any(|n| n.contains("contrast")));
    }
    #[test]
    fn formats_are_detected() {
        assert_eq!(detect("a.toml", ""), Some(Format::Alacritty));
        assert_eq!(
            detect("tokyo", "palette = 0=#000000"),
            Some(Format::Ghostty)
        );
        assert_eq!(detect("tokyo.conf", "color0 #000000"), Some(Format::Kitty));
        assert_eq!(detect("x", "plain"), None);
    }
}

//! Terminal branding: the gradient TERYON banner shown by `about`, `init` and `connect`.
use crate::theme::Theme;
use ratatui::style::Color;
use std::io::IsTerminal;

pub const TAGLINE: &str = "Your Mac mini. Wherever you work.";

const GLYPHS: [(char, [&str; 5]); 6] = [
    ('T', ["█████", "  █  ", "  █  ", "  █  ", "  █  "]),
    ('E', ["█████", "█    ", "████ ", "█    ", "█████"]),
    ('R', ["████ ", "█   █", "████ ", "█  █ ", "█   █"]),
    ('Y', ["█   █", " █ █ ", "  █  ", "  █  ", "  █  "]),
    ('O', [" ███ ", "█   █", "█   █", "█   █", " ███ "]),
    ('N', ["█   █", "██  █", "█ █ █", "█  ██", "█   █"]),
];

fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    }
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// Banner rows without color; every row has the same display width.
pub fn plain_rows() -> Vec<String> {
    (0..5)
        .map(|row| {
            GLYPHS
                .iter()
                .map(|(_, g)| g[row])
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Zyvor orange gradient endpoints (the Z mark's `#ff5a15`), identical in every theme.
const ORANGE_FROM: (u8, u8, u8) = (0xff, 0x5a, 0x15);
const ORANGE_TO: (u8, u8, u8) = (0xff, 0x9a, 0x5c);

/// The banner with a left-to-right Zyvor orange gradient, followed by the tagline in the theme's muted color.
pub fn banner(theme: &Theme, color: bool) -> String {
    let mut out = String::new();
    let rows = plain_rows();
    let width = rows[0].chars().count().max(2) - 1;
    let (from, to) = (ORANGE_FROM, ORANGE_TO);
    for row in &rows {
        for (i, ch) in row.chars().enumerate() {
            if color && ch != ' ' {
                let (r, g, b) = mix(from, to, i as f32 / width as f32);
                out.push_str(&format!("\x1b[38;2;{r};{g};{b}m{ch}"));
            } else {
                out.push(ch);
            }
        }
        if color {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
    if color {
        let (r, g, b) = rgb(theme.muted);
        out.push_str(&format!("\x1b[38;2;{r};{g};{b}m{TAGLINE}\x1b[0m\n"));
    } else {
        out.push_str(TAGLINE);
        out.push('\n');
    }
    out
}

/// Splash only on an interactive terminal, unless disabled by flag, config or `TERYON_NO_SPLASH`.
pub fn splash_wanted(disabled: bool) -> bool {
    !disabled && std::env::var_os("TERYON_NO_SPLASH").is_none() && std::io::stdout().is_terminal()
}

/// Color unless stdout is piped or `NO_COLOR` is set.
pub fn color_wanted() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_aligned() {
        let rows = plain_rows();
        assert_eq!(rows.len(), 5);
        assert!(rows
            .iter()
            .all(|r| r.chars().count() == rows[0].chars().count()));
    }

    #[test]
    fn banner_color_is_optional() {
        let t = Theme::load("tokyo-night").unwrap();
        assert!(!banner(&t, false).contains('\x1b'));
        assert!(banner(&t, true).contains("\x1b[38;2;255;90;21m"));
        assert!(banner(&t, false).ends_with("work.\n"));
    }
}

//! Desktop key bindings: chord parsing, the default table and user overrides.
use std::collections::BTreeMap;

/// Every action a key can trigger, by the name used in `[desktop.keys]`.
pub const ACTION_NAMES: &[&str] = &[
    "palette",
    "new_pane",
    "new_pane_right",
    "new_pane_below",
    "close_pane",
    "rename_pane",
    "zoom",
    "focus_left",
    "focus_right",
    "focus_up",
    "focus_down",
    "next_pane",
    "prev_pane",
    "resize_left",
    "resize_right",
    "resize_up",
    "resize_down",
    "equalize",
    "find",
    "find_next",
    "find_prev",
    "history",
    "new_tab",
    "close_tab",
    "tab_move_left",
    "tab_move_right",
    "tab_1",
    "tab_2",
    "tab_3",
    "tab_4",
    "tab_5",
    "tab_6",
    "tab_7",
    "tab_8",
    "tab_9",
    "font_up",
    "font_down",
    "font_reset",
    "reload_config",
    "fullscreen",
    "always_on_top",
    "next_theme",
    "prev_theme",
    "random_theme",
    "toggle_theme_variant",
    "scroll_page_up",
    "scroll_page_down",
    "scroll_top",
    "scroll_bottom",
    "copy_screen",
    "draft",
];

/// Default bindings as `(chord, action)`; an action may appear more than once.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("cmd+k", "palette"),
    ("cmd+d", "new_pane_right"),
    ("cmd+shift+d", "new_pane_below"),
    ("cmd+shift+enter", "zoom"),
    ("cmd+alt+left", "focus_left"),
    ("cmd+alt+right", "focus_right"),
    ("cmd+alt+up", "focus_up"),
    ("cmd+alt+down", "focus_down"),
    ("cmd+]", "next_pane"),
    ("cmd+[", "prev_pane"),
    ("cmd+ctrl+left", "resize_left"),
    ("cmd+ctrl+right", "resize_right"),
    ("cmd+ctrl+up", "resize_up"),
    ("cmd+ctrl+down", "resize_down"),
    ("cmd+ctrl+=", "equalize"),
    ("cmd+f", "find"),
    ("cmd+g", "find_next"),
    ("cmd+shift+g", "find_prev"),
    ("cmd+shift+f", "history"),
    ("cmd+t", "new_tab"),
    ("cmd+w", "close_tab"),
    ("cmd+shift+left", "tab_move_left"),
    ("cmd+shift+right", "tab_move_right"),
    ("cmd+1", "tab_1"),
    ("cmd+2", "tab_2"),
    ("cmd+3", "tab_3"),
    ("cmd+4", "tab_4"),
    ("cmd+5", "tab_5"),
    ("cmd+6", "tab_6"),
    ("cmd+7", "tab_7"),
    ("cmd+8", "tab_8"),
    ("cmd+9", "tab_9"),
    ("cmd+=", "font_up"),
    ("cmd+plus", "font_up"),
    ("cmd+-", "font_down"),
    ("cmd+0", "font_reset"),
    ("cmd+shift+,", "reload_config"),
    ("cmd+ctrl+f", "fullscreen"),
    ("cmd+up", "scroll_page_up"),
    ("cmd+down", "scroll_page_down"),
    ("cmd+home", "scroll_top"),
    ("cmd+end", "scroll_bottom"),
];

/// Modifier state of a key press, already normalized for the platform (see `desktop::chord_mods`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub mods: Mods,
    pub key: String,
}

/// Canonical key name (`a`, `1`, `left`, `enter`, `=`, `plus`, `f5`, ...) for a user or egui spelling.
pub fn canon_key(name: &str) -> Option<String> {
    let n = name.trim().to_lowercase();
    let key = match n.as_str() {
        "arrowleft" | "left" => "left",
        "arrowright" | "right" => "right",
        "arrowup" | "up" => "up",
        "arrowdown" | "down" => "down",
        "return" | "enter" => "enter",
        "esc" | "escape" => "escape",
        "equals" | "equal" | "=" => "=",
        "minus" | "-" => "-",
        "plus" | "+" => "plus",
        "comma" | "," => ",",
        "period" | "." => ".",
        "openbracket" | "[" => "[",
        "closebracket" | "]" => "]",
        "space" | " " => "space",
        "tab" | "backspace" | "delete" | "home" | "end" | "pageup" | "pagedown" | "insert" => {
            return Some(n)
        }
        other => {
            let mut chars = other.chars();
            let single = chars.next().filter(|_| chars.next().is_none());
            if single.is_some_and(|c| c.is_ascii_alphanumeric()) {
                return Some(other.to_string());
            }
            let f = other.strip_prefix('f').and_then(|d| d.parse::<u8>().ok());
            return f.filter(|d| (1..=12).contains(d)).map(|d| format!("f{d}"));
        }
    };
    Some(key.to_string())
}

impl Chord {
    /// Parse `cmd+shift+d` style text (modifiers `cmd`, `ctrl`, `alt`/`opt`, `shift`, any order).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut mods = Mods::default();
        // "cmd++" spells the plus key: peel it off before splitting on '+'.
        let (head, mut key) = match text.strip_suffix("++") {
            Some(head) => (head, Some("plus".to_string())),
            None => (text, None),
        };
        for part in head.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_lowercase().as_str() {
                "cmd" | "command" | "super" => mods.cmd = true,
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "opt" | "option" => mods.alt = true,
                "shift" => mods.shift = true,
                _ => {
                    if key.is_some() {
                        return Err(format!("more than one key in \"{text}\""));
                    }
                    key = Some(
                        canon_key(part)
                            .ok_or_else(|| format!("unknown key \"{part}\" in \"{text}\""))?,
                    );
                }
            }
        }
        let key = key.ok_or_else(|| format!("no key in \"{text}\""))?;
        Ok(Self { mods, key })
    }
    /// Display form with macOS symbols, e.g. `⇧⌘D`.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        for (on, sym) in [
            (self.mods.ctrl, "⌃"),
            (self.mods.alt, "⌥"),
            (self.mods.shift, "⇧"),
            (self.mods.cmd, "⌘"),
        ] {
            if on {
                out.push_str(sym);
            }
        }
        out.push_str(&match self.key.as_str() {
            "left" => "←".to_string(),
            "right" => "→".to_string(),
            "up" => "↑".to_string(),
            "down" => "↓".to_string(),
            "enter" => "↩".to_string(),
            "plus" => "+".to_string(),
            "escape" => "⎋".to_string(),
            "home" => "↖".to_string(),
            "end" => "↘".to_string(),
            "pageup" => "⇞".to_string(),
            "pagedown" => "⇟".to_string(),
            k if k.len() == 1 => k.to_uppercase(),
            k => k.to_string(),
        });
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    bindings: Vec<(Chord, String)>,
}

impl Keymap {
    /// Defaults plus `overrides` (action name -> chord text, `""`/`none` unbinds). Returns the map and
    /// human-readable problems; bad entries are skipped so the app always has a working keymap.
    pub fn build(overrides: &BTreeMap<String, String>) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut per_action: BTreeMap<String, Vec<Chord>> = BTreeMap::new();
        for (chord, action) in DEFAULTS {
            if let Ok(c) = Chord::parse(chord) {
                per_action.entry((*action).to_string()).or_default().push(c);
            }
        }
        for (action, chord) in overrides {
            if !ACTION_NAMES.contains(&action.as_str()) {
                problems.push(format!("unknown action \"{action}\" in [desktop.keys]"));
                continue;
            }
            let chord = chord.trim();
            if chord.is_empty() || chord.eq_ignore_ascii_case("none") {
                per_action.insert(action.clone(), Vec::new());
                continue;
            }
            match Chord::parse(chord) {
                Ok(c) => {
                    per_action.insert(action.clone(), vec![c]);
                }
                Err(e) => problems.push(format!("{action}: {e}")),
            }
        }
        let mut bindings: Vec<(Chord, String)> = Vec::new();
        for (action, chords) in per_action {
            for chord in chords {
                if let Some((_, other)) = bindings.iter().find(|(c, _)| *c == chord) {
                    problems.push(format!(
                        "{} is bound to both {other} and {action}",
                        chord.pretty()
                    ));
                    continue;
                }
                bindings.push((chord, action.clone()));
            }
        }
        (Self { bindings }, problems)
    }
    pub fn lookup(&self, mods: Mods, key: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|(c, _)| c.mods == mods && c.key == key)
            .map(|(_, a)| a.as_str())
    }
    /// First binding of `action`, formatted for display (empty when unbound).
    pub fn hint(&self, action: &str) -> String {
        self.bindings
            .iter()
            .find(|(_, a)| a == action)
            .map(|(c, _)| c.pretty())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chords_parse_in_any_order_and_spelling() {
        let a = Chord::parse("Cmd+Shift+D").unwrap();
        let b = Chord::parse("shift+command+d").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.pretty(), "⇧⌘D");
        assert_eq!(Chord::parse("cmd+alt+ArrowLeft").unwrap().key, "left");
        assert_eq!(Chord::parse("cmd++").unwrap().key, "plus");
        assert!(
            Chord::parse("cmd+").is_ok_and(|c| c.key == "plus") || Chord::parse("cmd+").is_err()
        );
        assert!(Chord::parse("cmd+shift").is_err());
        assert!(Chord::parse("cmd+d+e").is_err());
        assert!(Chord::parse("cmd+nonsense").is_err());
        assert_eq!(Chord::parse("ctrl+F5").unwrap().key, "f5");
    }
    #[test]
    fn defaults_are_valid_unique_and_cover_known_actions() {
        let (map, problems) = Keymap::build(&BTreeMap::new());
        assert!(problems.is_empty(), "{problems:?}");
        for (_, action) in DEFAULTS {
            assert!(ACTION_NAMES.contains(action), "{action}");
        }
        let cmd = Mods {
            cmd: true,
            ..Mods::default()
        };
        assert_eq!(map.lookup(cmd, "k"), Some("palette"));
        assert_eq!(map.hint("new_pane_below"), "⇧⌘D");
        assert_eq!(map.hint("close_pane"), "");
    }
    #[test]
    fn overrides_rebind_unbind_and_report_problems() {
        let mut o = BTreeMap::new();
        o.insert("palette".to_string(), "cmd+p".to_string());
        o.insert("find".to_string(), "none".to_string());
        o.insert("bogus".to_string(), "cmd+x".to_string());
        o.insert("zoom".to_string(), "cmd+d".to_string());
        o.insert("equalize".to_string(), "cmd+???".to_string());
        let (map, problems) = Keymap::build(&o);
        let cmd = Mods {
            cmd: true,
            ..Mods::default()
        };
        assert_eq!(map.lookup(cmd, "p"), Some("palette"));
        assert_eq!(map.lookup(cmd, "k"), None);
        assert_eq!(map.hint("find"), "");
        assert!(problems.iter().any(|p| p.contains("unknown action")));
        assert!(problems.iter().any(|p| p.contains("bound to both")));
        assert!(problems.iter().any(|p| p.starts_with("equalize")));
    }
}

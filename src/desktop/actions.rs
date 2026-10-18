//! Everything the palette and key bindings can trigger, with the names used in `[desktop.keys]`.
use super::*;
use crate::keymap::Keymap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scroll {
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// Things the command palette (and shortcuts) can do.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ActionKind {
    Palette,
    NewPane,
    NewPaneRight,
    NewPaneBelow,
    Focus(Dir),
    Resize(Dir),
    Equalize,
    NextPane,
    PrevPane,
    ClosePane,
    RenamePane,
    Layout(LayoutMode),
    Zoom,
    Find,
    FindNext,
    FindPrev,
    NewTab,
    CloseTab,
    TabJump(usize),
    TabMove(isize),
    Theme(&'static str),
    NextTheme,
    PrevTheme,
    RandomTheme,
    ThemeVariant,
    FontUp,
    FontDown,
    FontReset,
    ReloadConfig,
    Fullscreen,
    AlwaysOnTop,
    Scroll(Scroll),
    History,
    CopyScreen,
    Draft,
}

impl ActionKind {
    /// Name in `[desktop.keys]`; `None` for entries that exist only in the palette.
    pub(super) fn name(&self) -> Option<String> {
        Some(
            match self {
                Self::Palette => "palette",
                Self::NewPane => "new_pane",
                Self::NewPaneRight => "new_pane_right",
                Self::NewPaneBelow => "new_pane_below",
                Self::Focus(Dir::Left) => "focus_left",
                Self::Focus(Dir::Right) => "focus_right",
                Self::Focus(Dir::Up) => "focus_up",
                Self::Focus(Dir::Down) => "focus_down",
                Self::Resize(Dir::Left) => "resize_left",
                Self::Resize(Dir::Right) => "resize_right",
                Self::Resize(Dir::Up) => "resize_up",
                Self::Resize(Dir::Down) => "resize_down",
                Self::Equalize => "equalize",
                Self::NextPane => "next_pane",
                Self::PrevPane => "prev_pane",
                Self::ClosePane => "close_pane",
                Self::RenamePane => "rename_pane",
                Self::Zoom => "zoom",
                Self::Find => "find",
                Self::FindNext => "find_next",
                Self::FindPrev => "find_prev",
                Self::NewTab => "new_tab",
                Self::CloseTab => "close_tab",
                Self::TabMove(-1) => "tab_move_left",
                Self::TabMove(1) => "tab_move_right",
                Self::TabJump(n) => return Some(format!("tab_{}", n + 1)),
                Self::NextTheme => "next_theme",
                Self::PrevTheme => "prev_theme",
                Self::RandomTheme => "random_theme",
                Self::ThemeVariant => "toggle_theme_variant",
                Self::FontUp => "font_up",
                Self::FontDown => "font_down",
                Self::FontReset => "font_reset",
                Self::ReloadConfig => "reload_config",
                Self::Fullscreen => "fullscreen",
                Self::AlwaysOnTop => "always_on_top",
                Self::Scroll(Scroll::PageUp) => "scroll_page_up",
                Self::Scroll(Scroll::PageDown) => "scroll_page_down",
                Self::Scroll(Scroll::Top) => "scroll_top",
                Self::Scroll(Scroll::Bottom) => "scroll_bottom",
                Self::History => "history",
                Self::CopyScreen => "copy_screen",
                Self::Draft => "draft",
                Self::Layout(_) | Self::Theme(_) | Self::TabMove(_) => return None,
            }
            .to_string(),
        )
    }
    pub(super) fn from_name(name: &str) -> Option<Self> {
        if let Some(n) = name
            .strip_prefix("tab_")
            .and_then(|d| d.parse::<usize>().ok())
        {
            return (1..=9).contains(&n).then(|| Self::TabJump(n - 1));
        }
        Some(match name {
            "palette" => Self::Palette,
            "new_pane" => Self::NewPane,
            "new_pane_right" => Self::NewPaneRight,
            "new_pane_below" => Self::NewPaneBelow,
            "focus_left" => Self::Focus(Dir::Left),
            "focus_right" => Self::Focus(Dir::Right),
            "focus_up" => Self::Focus(Dir::Up),
            "focus_down" => Self::Focus(Dir::Down),
            "resize_left" => Self::Resize(Dir::Left),
            "resize_right" => Self::Resize(Dir::Right),
            "resize_up" => Self::Resize(Dir::Up),
            "resize_down" => Self::Resize(Dir::Down),
            "equalize" => Self::Equalize,
            "next_pane" => Self::NextPane,
            "prev_pane" => Self::PrevPane,
            "close_pane" => Self::ClosePane,
            "rename_pane" => Self::RenamePane,
            "zoom" => Self::Zoom,
            "find" => Self::Find,
            "find_next" => Self::FindNext,
            "find_prev" => Self::FindPrev,
            "new_tab" => Self::NewTab,
            "close_tab" => Self::CloseTab,
            "tab_move_left" => Self::TabMove(-1),
            "tab_move_right" => Self::TabMove(1),
            "next_theme" => Self::NextTheme,
            "prev_theme" => Self::PrevTheme,
            "random_theme" => Self::RandomTheme,
            "toggle_theme_variant" => Self::ThemeVariant,
            "font_up" => Self::FontUp,
            "font_down" => Self::FontDown,
            "font_reset" => Self::FontReset,
            "reload_config" => Self::ReloadConfig,
            "fullscreen" => Self::Fullscreen,
            "always_on_top" => Self::AlwaysOnTop,
            "scroll_page_up" => Self::Scroll(Scroll::PageUp),
            "scroll_page_down" => Self::Scroll(Scroll::PageDown),
            "scroll_top" => Self::Scroll(Scroll::Top),
            "scroll_bottom" => Self::Scroll(Scroll::Bottom),
            "history" => Self::History,
            "copy_screen" => Self::CopyScreen,
            "draft" => Self::Draft,
            _ => return None,
        })
    }
}

pub(super) struct Action {
    pub title: String,
    pub hint: String,
    pub kind: ActionKind,
}
/// Palette entries; shortcut hints come from the live keymap so rebinding shows up immediately.
pub(super) fn actions(keymap: &Keymap) -> Vec<Action> {
    let plain = [
        ("Command palette", ActionKind::Palette),
        ("New pane", ActionKind::NewPane),
        ("New pane to the right", ActionKind::NewPaneRight),
        ("New pane below", ActionKind::NewPaneBelow),
        ("Focus pane left", ActionKind::Focus(Dir::Left)),
        ("Focus pane right", ActionKind::Focus(Dir::Right)),
        ("Focus pane above", ActionKind::Focus(Dir::Up)),
        ("Focus pane below", ActionKind::Focus(Dir::Down)),
        ("Next pane", ActionKind::NextPane),
        ("Previous pane", ActionKind::PrevPane),
        ("Resize pane left", ActionKind::Resize(Dir::Left)),
        ("Resize pane right", ActionKind::Resize(Dir::Right)),
        ("Resize pane up", ActionKind::Resize(Dir::Up)),
        ("Resize pane down", ActionKind::Resize(Dir::Down)),
        ("Equalize panes", ActionKind::Equalize),
        ("Close pane", ActionKind::ClosePane),
        ("Rename pane", ActionKind::RenamePane),
        ("Zoom pane / restore", ActionKind::Zoom),
        ("Find in pane", ActionKind::Find),
        ("Find next", ActionKind::FindNext),
        ("Find previous", ActionKind::FindPrev),
        ("New tab", ActionKind::NewTab),
        ("Close tab", ActionKind::CloseTab),
        ("Move tab left", ActionKind::TabMove(-1)),
        ("Move tab right", ActionKind::TabMove(1)),
        ("Increase font size", ActionKind::FontUp),
        ("Decrease font size", ActionKind::FontDown),
        ("Reset font size", ActionKind::FontReset),
        ("Next theme", ActionKind::NextTheme),
        ("Previous theme", ActionKind::PrevTheme),
        ("Random theme", ActionKind::RandomTheme),
        ("Switch light / dark theme", ActionKind::ThemeVariant),
        ("Scroll up a page", ActionKind::Scroll(Scroll::PageUp)),
        ("Scroll down a page", ActionKind::Scroll(Scroll::PageDown)),
        ("Scroll to top", ActionKind::Scroll(Scroll::Top)),
        ("Scroll to bottom", ActionKind::Scroll(Scroll::Bottom)),
        ("Search retained output", ActionKind::History),
        ("Copy visible screen", ActionKind::CopyScreen),
        ("Offline draft", ActionKind::Draft),
        ("Reload config", ActionKind::ReloadConfig),
        ("Toggle fullscreen", ActionKind::Fullscreen),
        ("Toggle always on top", ActionKind::AlwaysOnTop),
    ];
    let mut list: Vec<Action> = plain
        .into_iter()
        .map(|(title, kind)| Action {
            hint: kind.name().map(|n| keymap.hint(&n)).unwrap_or_default(),
            title: title.into(),
            kind,
        })
        .collect();
    for mode in [
        LayoutMode::Single,
        LayoutMode::Vertical,
        LayoutMode::Horizontal,
        LayoutMode::Grid,
    ] {
        list.push(Action {
            title: format!("Layout: {}", mode.name()),
            hint: String::new(),
            kind: ActionKind::Layout(mode),
        });
    }
    for name in BUILTINS {
        list.push(Action {
            title: format!("Theme: {name}"),
            hint: String::new(),
            kind: ActionKind::Theme(name),
        });
    }
    list
}
/// Commands that take an argument and run through `execute` when one is typed.
pub(super) const ARG_COMMANDS: [&str; 7] = [
    "new", "rename", "theme", "search", "export", "tab", "layout",
];
/// The theme to switch to for `delta` steps through the built-ins (wrapping).
pub(super) fn theme_step(current: &str, delta: isize) -> &'static str {
    let at = BUILTINS.iter().position(|n| *n == current).unwrap_or(0) as isize;
    BUILTINS[(at + delta).rem_euclid(BUILTINS.len() as isize) as usize]
}
/// The light or dark sibling of a built-in theme (`mac27-dark` <-> `mac27-light`), if one exists.
pub(super) fn theme_counterpart(current: &str) -> Option<&'static str> {
    let other = if let Some(stem) = current.strip_suffix("-dark") {
        format!("{stem}-light")
    } else {
        format!("{}-dark", current.strip_suffix("-light")?)
    };
    BUILTINS.iter().copied().find(|n| *n == other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::ACTION_NAMES;
    #[test]
    fn every_key_name_maps_to_an_action_and_back() {
        for name in ACTION_NAMES {
            let kind = ActionKind::from_name(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(kind.name().as_deref(), Some(*name));
        }
        assert!(ActionKind::from_name("tab_0").is_none());
        assert!(ActionKind::from_name("tab_10").is_none());
    }
    #[test]
    fn palette_lists_shortcuts_from_the_keymap() {
        let (map, _) = Keymap::build(&Default::default());
        let list = actions(&map);
        let hint = |k: ActionKind| list.iter().find(|a| a.kind == k).unwrap().hint.clone();
        assert_eq!(hint(ActionKind::NewPaneBelow), "⇧⌘D");
        assert_eq!(hint(ActionKind::Focus(Dir::Left)), "⌥⌘←");
        assert_eq!(
            list.iter()
                .filter(|a| a.title.starts_with("Theme: "))
                .count(),
            BUILTINS.len()
        );
    }
    #[test]
    fn theme_stepping_wraps_and_finds_counterparts() {
        assert_eq!(theme_step("mac27-dark", -1), BUILTINS[BUILTINS.len() - 1]);
        assert_eq!(theme_step(BUILTINS[BUILTINS.len() - 1], 1), "mac27-dark");
        assert_eq!(theme_counterpart("mac27-dark"), Some("mac27-light"));
        assert_eq!(theme_counterpart("apple-light"), Some("apple-dark"));
        assert_eq!(theme_counterpart("nord"), None);
    }
}

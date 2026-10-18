//! Mouse menus: right-click in a pane and the ▾ dropdown in the bar (copy, paste, splits, view, help).
use super::*;
use std::process::{Command as Proc, Stdio};

/// What a menu entry asks for; applied at the start of the next frame, never while drawing.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum MenuPick {
    Act(ActionKind),
    Copy,
    Paste,
    SelectAll,
}

/// Current clipboard text, read through the platform tool (egui only delivers paste events for keystrokes).
pub(super) fn clipboard_text() -> Option<String> {
    let candidates: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["pbpaste"]]
    } else if cfg!(target_os = "windows") {
        &[&["powershell", "-NoProfile", "-Command", "Get-Clipboard -Raw"]]
    } else {
        &[
            &["wl-paste", "--no-newline"],
            &["xclip", "-selection", "clipboard", "-o"],
            &["xsel", "--clipboard", "--output"],
        ]
    };
    candidates.iter().find_map(|argv| {
        let out = Proc::new(argv[0])
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

/// Which entries a menu shows.
pub(super) struct MenuState {
    pub selection: bool,
    pub broadcast: bool,
    pub full: bool,
}

fn entry(
    ui: &mut egui::Ui,
    keys: &Keymap,
    label: &str,
    pick: MenuPick,
    enabled: bool,
    out: &mut Option<MenuPick>,
) {
    let hint = match &pick {
        MenuPick::Act(kind) => kind.name().map(|n| keys.hint(&n)).unwrap_or_default(),
        MenuPick::Copy if cfg!(target_os = "macos") => "⌘C".into(),
        MenuPick::Paste if cfg!(target_os = "macos") => "⌘V".into(),
        MenuPick::SelectAll => String::new(),
        MenuPick::Copy => "Ctrl+Shift+C".into(),
        MenuPick::Paste => "Ctrl+Shift+V".into(),
    };
    let mut button = egui::Button::new(label);
    if !hint.is_empty() {
        button = button.shortcut_text(hint);
    }
    if ui.add_enabled(enabled, button).clicked() {
        *out = Some(pick);
        ui.close_menu();
    }
}

/// The shared menu body. The right-click menu shows the first three groups; the dropdown shows all of them.
pub(super) fn menu_body(
    ui: &mut egui::Ui,
    keys: &Keymap,
    snippets: &[config::Snippet],
    st: &MenuState,
) -> Option<MenuPick> {
    use ActionKind as A;
    let mut out = None;
    ui.set_min_width(210.);
    if st.full {
        entry(
            ui,
            keys,
            "New tab",
            MenuPick::Act(A::NewTab),
            true,
            &mut out,
        );
        entry(
            ui,
            keys,
            "New pane",
            MenuPick::Act(A::NewPane),
            true,
            &mut out,
        );
        entry(
            ui,
            keys,
            "Open workspace…",
            MenuPick::Act(A::OpenWorkspace),
            true,
            &mut out,
        );
        ui.separator();
    }
    entry(ui, keys, "Copy", MenuPick::Copy, st.selection, &mut out);
    entry(ui, keys, "Paste", MenuPick::Paste, true, &mut out);
    entry(ui, keys, "Select all", MenuPick::SelectAll, true, &mut out);
    entry(
        ui,
        keys,
        "Copy last output",
        MenuPick::Act(A::CopyLastOutput),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Copy screen",
        MenuPick::Act(A::CopyScreen),
        true,
        &mut out,
    );
    ui.separator();
    entry(ui, keys, "Find…", MenuPick::Act(A::Find), true, &mut out);
    entry(
        ui,
        keys,
        "Quick select",
        MenuPick::Act(A::Hints),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Copy mode",
        MenuPick::Act(A::CopyMode),
        true,
        &mut out,
    );
    ui.separator();
    entry(
        ui,
        keys,
        "Split right",
        MenuPick::Act(A::NewPaneRight),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Split below",
        MenuPick::Act(A::NewPaneBelow),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Float / dock pane",
        MenuPick::Act(A::FloatPane),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Zoom pane",
        MenuPick::Act(A::Zoom),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Rename pane…",
        MenuPick::Act(A::RenamePane),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Close pane",
        MenuPick::Act(A::ClosePane),
        true,
        &mut out,
    );
    if !st.full {
        return out;
    }
    ui.separator();
    ui.menu_button("Layout", |ui| {
        for (name, mode) in [
            ("Single", LayoutMode::Single),
            ("Side by side", LayoutMode::Horizontal),
            ("Stacked", LayoutMode::Vertical),
            ("Grid", LayoutMode::Grid),
        ] {
            entry(
                ui,
                keys,
                name,
                MenuPick::Act(A::Layout(mode)),
                true,
                &mut out,
            );
        }
        entry(
            ui,
            keys,
            "Equalize panes",
            MenuPick::Act(A::Equalize),
            true,
            &mut out,
        );
    });
    if !snippets.is_empty() {
        ui.menu_button("Snippets", |ui| {
            for (i, s) in snippets.iter().enumerate() {
                entry(
                    ui,
                    keys,
                    &s.name,
                    MenuPick::Act(A::Snippet(i)),
                    true,
                    &mut out,
                );
            }
        });
    }
    ui.menu_button("View", |ui| {
        let label = if st.broadcast {
            "Stop broadcast input"
        } else {
            "Broadcast input"
        };
        entry(ui, keys, label, MenuPick::Act(A::Broadcast), true, &mut out);
        entry(
            ui,
            keys,
            "Bigger text",
            MenuPick::Act(A::FontUp),
            true,
            &mut out,
        );
        entry(
            ui,
            keys,
            "Smaller text",
            MenuPick::Act(A::FontDown),
            true,
            &mut out,
        );
        entry(
            ui,
            keys,
            "Reset text size",
            MenuPick::Act(A::FontReset),
            true,
            &mut out,
        );
        entry(
            ui,
            keys,
            "Next theme",
            MenuPick::Act(A::NextTheme),
            true,
            &mut out,
        );
        entry(
            ui,
            keys,
            "Full screen",
            MenuPick::Act(A::Fullscreen),
            true,
            &mut out,
        );
        entry(
            ui,
            keys,
            "Always on top",
            MenuPick::Act(A::AlwaysOnTop),
            true,
            &mut out,
        );
    });
    ui.separator();
    entry(
        ui,
        keys,
        "Command palette",
        MenuPick::Act(A::Palette),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Keyboard shortcuts",
        MenuPick::Act(A::CheatSheet),
        true,
        &mut out,
    );
    entry(
        ui,
        keys,
        "Reload config",
        MenuPick::Act(A::ReloadConfig),
        true,
        &mut out,
    );
    out
}

impl Desktop {
    /// Apply a menu choice made on `pane`.
    pub(super) fn run_menu(&mut self, pane: u64, pick: MenuPick) {
        if self.panes.contains_key(&pane) {
            self.focus = pane;
        }
        match pick {
            MenuPick::Act(kind) => self.run_action(kind),
            MenuPick::Copy => self.copy_selection(),
            MenuPick::Paste => match clipboard_text() {
                Some(text) if !text.is_empty() => {
                    let bracketed = self
                        .panes
                        .get(&self.focus)
                        .is_some_and(|p| p.parser.screen().bracketed_paste());
                    self.input(keys::paste(&text, bracketed));
                }
                _ => self.notice = "Clipboard is empty".into(),
            },
            MenuPick::SelectAll => {
                if let Some(p) = self.panes.get(&pane) {
                    let (rows, cols) = p.parser.screen().size();
                    self.selection = Some(Selection {
                        pane,
                        a: (0, 0),
                        b: (rows.saturating_sub(1), cols.saturating_sub(1)),
                        block: false,
                    });
                }
            }
        }
    }
    /// The ▾ button next to "+": a WezTerm-style dropdown with everything a mouse user needs.
    pub(super) fn dropdown_menu(&mut self, ui: &mut egui::Ui) {
        let t = self.theme.clone();
        let id = egui::Id::new("teryon-dropdown");
        let button = icon_button(ui, &t, Icon::Chevron, "Menu");
        if button.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        let st = MenuState {
            selection: self.selection.is_some(),
            broadcast: self.broadcast,
            full: true,
        };
        let mut pick = None;
        egui::popup::popup_below_widget(
            ui,
            id,
            &button,
            egui::PopupCloseBehavior::CloseOnClick,
            |ui| {
                pick = menu_body(ui, &self.keymap, &self.cfg.snippets, &st);
            },
        );
        if let Some(p) = pick {
            self.menu_pick = Some((self.focus, p));
        }
    }
}

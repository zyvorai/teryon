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

/// One menu row, macOS style: roomy, rounded accent highlight on hover, shortcut dimmed on the right.
fn entry(
    ui: &mut egui::Ui,
    cx: &Cx,
    label: &str,
    pick: MenuPick,
    enabled: bool,
    out: &mut Option<MenuPick>,
) {
    let hint = match &pick {
        MenuPick::Act(kind) => kind.name().map(|n| cx.keys.hint(&n)).unwrap_or_default(),
        MenuPick::Copy if cfg!(target_os = "macos") => "⌘C".into(),
        MenuPick::Paste if cfg!(target_os = "macos") => "⌘V".into(),
        MenuPick::SelectAll if cfg!(target_os = "macos") => "⌘A".into(),
        MenuPick::Copy => "Ctrl+Shift+C".into(),
        MenuPick::Paste => "Ctrl+Shift+V".into(),
        MenuPick::SelectAll => String::new(),
    };
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(220.), 26.),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hot = enabled && resp.hovered();
    if hot {
        ui.painter().rect_filled(rect, 6., color(cx.theme.primary));
    }
    let (text, dim) = if hot {
        (Color32::WHITE, Color32::from_white_alpha(190))
    } else if enabled {
        (color(cx.theme.foreground), color(cx.theme.muted))
    } else {
        (
            color(cx.theme.muted),
            color(cx.theme.muted).gamma_multiply(0.7),
        )
    };
    let p = ui.painter();
    p.text(
        rect.left_center() + Vec2::new(10., 0.),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.),
        text,
    );
    if !hint.is_empty() {
        p.text(
            rect.right_center() - Vec2::new(10., 0.),
            egui::Align2::RIGHT_CENTER,
            hint,
            FontId::proportional(12.),
            dim,
        );
    }
    if resp.clicked() {
        *out = Some(pick);
        ui.close_menu();
    }
}

/// A submenu row with a chevron, same look as `entry`.
fn submenu(ui: &mut egui::Ui, cx: &Cx, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        v.widgets.hovered.weak_bg_fill = color(cx.theme.primary);
        v.widgets.hovered.fg_stroke.color = Color32::WHITE;
        v.widgets.open.weak_bg_fill = color(cx.theme.primary);
        v.widgets.open.fg_stroke.color = Color32::WHITE;
        ui.set_min_width(220.);
        // Justified, so the row spans the menu and the submenu opens beside it instead of on top of it.
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            ui.menu_button(format!("{label}   ›"), |ui| {
                ui.set_min_width(220.);
                add(ui);
            });
        });
    });
}

fn rule(ui: &mut egui::Ui, cx: &Cx) {
    ui.add_space(2.);
    let (r, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(220.), 1.),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(r.shrink2(Vec2::new(8., 0.)), 0., color(cx.theme.border));
    ui.add_space(2.);
}

/// Everything a menu needs besides its entries.
pub(super) struct Cx<'a> {
    pub keys: &'a Keymap,
    pub theme: &'a Theme,
    pub snippets: &'a [config::Snippet],
}

/// The pane menu is short; the rest lives in submenus. The dropdown adds tabs, layout, snippets, view and help.
pub(super) fn menu_body(ui: &mut egui::Ui, cx: &Cx, st: &MenuState) -> Option<MenuPick> {
    use ActionKind as A;
    let mut out = None;
    ui.spacing_mut().item_spacing.y = 1.;
    if st.full {
        entry(ui, cx, "New Tab", MenuPick::Act(A::NewTab), true, &mut out);
        entry(
            ui,
            cx,
            "New Pane",
            MenuPick::Act(A::NewPane),
            true,
            &mut out,
        );
        entry(
            ui,
            cx,
            "Open Workspace…",
            MenuPick::Act(A::OpenWorkspace),
            true,
            &mut out,
        );
        rule(ui, cx);
    }
    entry(ui, cx, "Copy", MenuPick::Copy, st.selection, &mut out);
    entry(ui, cx, "Paste", MenuPick::Paste, true, &mut out);
    entry(ui, cx, "Select All", MenuPick::SelectAll, true, &mut out);
    rule(ui, cx);
    entry(ui, cx, "Find…", MenuPick::Act(A::Find), true, &mut out);
    submenu(ui, cx, "Copy Special", |ui| {
        entry(
            ui,
            cx,
            "Last Command Output",
            MenuPick::Act(A::CopyLastOutput),
            true,
            &mut out,
        );
        entry(
            ui,
            cx,
            "Whole Screen",
            MenuPick::Act(A::CopyScreen),
            true,
            &mut out,
        );
        entry(
            ui,
            cx,
            "Quick Select…",
            MenuPick::Act(A::Hints),
            true,
            &mut out,
        );
        entry(
            ui,
            cx,
            "Copy Mode",
            MenuPick::Act(A::CopyMode),
            true,
            &mut out,
        );
    });
    rule(ui, cx);
    entry(
        ui,
        cx,
        "Split Right",
        MenuPick::Act(A::NewPaneRight),
        true,
        &mut out,
    );
    entry(
        ui,
        cx,
        "Split Below",
        MenuPick::Act(A::NewPaneBelow),
        true,
        &mut out,
    );
    submenu(ui, cx, "Pane", |ui| {
        entry(ui, cx, "Zoom", MenuPick::Act(A::Zoom), true, &mut out);
        entry(
            ui,
            cx,
            "Float or Dock",
            MenuPick::Act(A::FloatPane),
            true,
            &mut out,
        );
        entry(
            ui,
            cx,
            "Rename…",
            MenuPick::Act(A::RenamePane),
            true,
            &mut out,
        );
        entry(
            ui,
            cx,
            "Equalize Sizes",
            MenuPick::Act(A::Equalize),
            true,
            &mut out,
        );
    });
    if st.full {
        submenu(ui, cx, "Layout", |ui| {
            for (name, mode) in [
                ("Single", LayoutMode::Single),
                ("Side by Side", LayoutMode::Horizontal),
                ("Stacked", LayoutMode::Vertical),
                ("Grid", LayoutMode::Grid),
            ] {
                entry(ui, cx, name, MenuPick::Act(A::Layout(mode)), true, &mut out);
            }
        });
        if !cx.snippets.is_empty() {
            submenu(ui, cx, "Snippets", |ui| {
                for (i, s) in cx.snippets.iter().enumerate() {
                    entry(
                        ui,
                        cx,
                        &s.name,
                        MenuPick::Act(A::Snippet(i)),
                        true,
                        &mut out,
                    );
                }
            });
        }
        submenu(ui, cx, "View", |ui| {
            let label = if st.broadcast {
                "Stop Broadcast Input"
            } else {
                "Broadcast Input"
            };
            entry(ui, cx, label, MenuPick::Act(A::Broadcast), true, &mut out);
            entry(
                ui,
                cx,
                "Bigger Text",
                MenuPick::Act(A::FontUp),
                true,
                &mut out,
            );
            entry(
                ui,
                cx,
                "Smaller Text",
                MenuPick::Act(A::FontDown),
                true,
                &mut out,
            );
            entry(
                ui,
                cx,
                "Actual Size",
                MenuPick::Act(A::FontReset),
                true,
                &mut out,
            );
            entry(
                ui,
                cx,
                "Next Theme",
                MenuPick::Act(A::NextTheme),
                true,
                &mut out,
            );
            entry(
                ui,
                cx,
                "Full Screen",
                MenuPick::Act(A::Fullscreen),
                true,
                &mut out,
            );
            entry(
                ui,
                cx,
                "Always on Top",
                MenuPick::Act(A::AlwaysOnTop),
                true,
                &mut out,
            );
        });
        rule(ui, cx);
        entry(
            ui,
            cx,
            "Command Palette",
            MenuPick::Act(A::Palette),
            true,
            &mut out,
        );
        entry(
            ui,
            cx,
            "Keyboard Shortcuts",
            MenuPick::Act(A::CheatSheet),
            true,
            &mut out,
        );
        entry(
            ui,
            cx,
            "Reload Config",
            MenuPick::Act(A::ReloadConfig),
            true,
            &mut out,
        );
    } else {
        rule(ui, cx);
    }
    if !st.full {
        entry(
            ui,
            cx,
            "Close Pane",
            MenuPick::Act(A::ClosePane),
            true,
            &mut out,
        );
    }
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
                pick = menu_body(
                    ui,
                    &Cx {
                        keys: &self.keymap,
                        theme: &t,
                        snippets: &self.cfg.snippets,
                    },
                    &st,
                );
            },
        );
        if let Some(p) = pick {
            self.menu_pick = Some((self.focus, p));
        }
    }
}

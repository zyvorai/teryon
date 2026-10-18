//! The welcome card, the shortcut cheat sheet and the host/workspace catalog behind "Open workspace".
use super::*;
use crate::recent;

const TIPS: [&str; 5] = [
    "⌘K finds every action; ↑/↓ over a theme previews it.",
    "⌘O opens another host or workspace as a new tab.",
    "Drag any divider, or use ⌥⌘ arrows to move between panes.",
    "Double-click a word to select it; ⌥-drag selects a block.",
    "Edit config.toml while TERYON runs: it reloads by itself.",
];
/// `(group, [(label, action)])` shown by the cheat sheet; bindings come from the live keymap.
const CHEAT: [(&str, &[(&str, &str)]); 5] = [
    (
        "Panes",
        &[
            ("New pane right", "new_pane_right"),
            ("New pane below", "new_pane_below"),
            ("Focus left", "focus_left"),
            ("Focus right", "focus_right"),
            ("Zoom / restore", "zoom"),
            ("Equalize", "equalize"),
            ("Broadcast input", "broadcast"),
            ("Float / unfloat pane", "float_pane"),
            ("New floating pane", "new_floating_pane"),
        ],
    ),
    (
        "Tabs & workspaces",
        &[
            ("New tab", "new_tab"),
            ("Open workspace…", "open_workspace"),
            ("Close tab", "close_tab"),
            ("Move tab left", "tab_move_left"),
            ("Move tab right", "tab_move_right"),
        ],
    ),
    (
        "Find & copy",
        &[
            ("Find in pane", "find"),
            ("Next match", "find_next"),
            ("Quick select (copy URLs, paths…)", "hints"),
            ("Copy mode (vi keys)", "copy_mode"),
            ("Regex / case in find", "find_regex"),
            ("Search retained output", "history"),
            ("Previous prompt", "prompt_prev"),
            ("Next prompt", "prompt_next"),
            ("Copy last command output", "copy_last_output"),
            ("Scroll a page up", "scroll_page_up"),
        ],
    ),
    (
        "View",
        &[
            ("Command palette", "palette"),
            ("Ask AI", "ai"),
            ("Bigger text", "font_up"),
            ("Smaller text", "font_down"),
            ("Reset text size", "font_reset"),
            ("Fullscreen", "fullscreen"),
        ],
    ),
    (
        "Setup",
        &[
            ("Reload config", "reload_config"),
            ("Shortcut cheat sheet", "cheatsheet"),
        ],
    ),
];

/// Recents first, then every configured host's default workspace, without repeats.
pub(super) fn open_catalog(recents: &[recent::Recent], hosts: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = recents
        .iter()
        .map(|r| (r.host.clone(), r.workspace.clone()))
        .collect();
    for h in hosts {
        if !out.iter().any(|(host, _)| host == h) {
            out.push((h.clone(), "default".into()));
        }
    }
    out.truncate(16);
    out
}

impl Desktop {
    /// Reload the lists the palette offers (themes, workspaces to open); called when it opens.
    pub(super) fn refresh_catalog(&mut self) {
        self.theme_names = crate::theme::all_names();
        let hosts: Vec<String> = config::load()
            .map(|c| c.hosts.keys().cloned().collect())
            .unwrap_or_default();
        self.open_entries = open_catalog(&recent::load(), &hosts);
    }
    /// Shown while the workspace has no panes: what is happening and what to do about it.
    pub(super) fn welcome_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme.clone();
        let ctx = ui.ctx().clone();
        let stalled = self
            .offline_since
            .is_some_and(|s| s.elapsed() > Duration::from_secs(3));
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.22).max(24.));
            let (mark, _) = ui.allocate_exact_size(Vec2::splat(64.), Sense::hover());
            paint_mark(ui.painter(), mark);
            ui.add_space(6.);
            ui.label(
                weighted(
                    &ctx,
                    if self.online {
                        format!("Starting {} / {}…", self.host, self.workspace)
                    } else if stalled {
                        format!("Can't reach {}", self.host)
                    } else {
                        format!("Connecting to {} / {}…", self.host, self.workspace)
                    },
                    "semibold",
                    18.,
                )
                .color(color(t.foreground)),
            );
            if stalled && !self.online {
                ui.add_space(4.);
                ui.label(
                    RichText::new(&self.notice)
                        .size(12.)
                        .color(color(t.warning)),
                );
                ui.add_space(8.);
                let fix = format!(
                    "teryon doctor {} --fix --binary PATH_TO_A_BUILD_FOR_THAT_MACHINE",
                    self.host
                );
                ui.label(
                    RichText::new("Check the host, or install/update TERYON there:")
                        .size(12.)
                        .color(color(t.muted)),
                );
                ui.label(RichText::new(&fix).monospace().size(12.));
                if ui.button("Copy command").clicked() {
                    self.copy_text = Some(fix);
                }
            }
            let others: Vec<(String, String)> = self
                .open_entries
                .iter()
                .filter(|(h, w)| !(*h == self.host && *w == self.workspace))
                .take(5)
                .cloned()
                .collect();
            if !others.is_empty() {
                ui.add_space(16.);
                ui.label(
                    RichText::new("Open instead")
                        .size(12.)
                        .color(color(t.muted)),
                );
                ui.horizontal_wrapped(|ui| {
                    for (host, ws) in others {
                        if ui.button(format!("{host} / {ws}")).clicked() {
                            self.open_tab(&host, &ws);
                        }
                    }
                });
            }
            ui.add_space(14.);
            ui.label(
                RichText::new(TIPS[self.tip_index % TIPS.len()])
                    .size(12.)
                    .color(color(t.muted)),
            );
        });
    }
    /// ⌘/ overlay: every shortcut grouped, from the live keymap (so rebinding shows up).
    pub(super) fn cheatsheet_ui(&mut self, ctx: &egui::Context) {
        let t = self.theme.clone();
        let mut open = true;
        egui::Window::new("Shortcuts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    for (group, rows) in CHEAT {
                        ui.vertical(|ui| {
                            ui.set_min_width(170.);
                            ui.label(weighted(ctx, group, "semibold", 12.).color(color(t.muted)));
                            for (label, action) in rows {
                                let hint = self.keymap.hint(action);
                                ui.horizontal(|ui| {
                                    ui.label(*label);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(if hint.is_empty() {
                                                    "—".into()
                                                } else {
                                                    hint
                                                })
                                                .monospace()
                                                .color(color(t.accent)),
                                            );
                                        },
                                    );
                                });
                            }
                        });
                        ui.add_space(14.);
                    }
                });
                ui.add_space(6.);
                ui.label(
                    RichText::new(format!("Tip: {}", TIPS[self.tip_index % TIPS.len()]))
                        .size(12.)
                        .color(color(t.muted)),
                );
            });
        if !open || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.cheatsheet = false;
            self.tip_index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_puts_recents_first_and_adds_each_host_once() {
        let r = vec![recent::Recent {
            host: "mini".into(),
            workspace: "fluxvm".into(),
            last_used: 5,
            count: 2,
        }];
        let c = open_catalog(&r, &["mini".into(), "box".into(), "box".into()]);
        assert_eq!(
            c,
            vec![
                ("mini".to_string(), "fluxvm".to_string()),
                ("box".to_string(), "default".to_string())
            ]
        );
    }
    #[test]
    fn every_cheat_sheet_row_names_a_real_action() {
        for (_, rows) in CHEAT {
            for (_, action) in rows {
                assert!(crate::keymap::ACTION_NAMES.contains(action), "{action}");
            }
        }
    }
}

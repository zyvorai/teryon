//! ⌘⇧T: choose a theme from one list, each row drawn in its own colors, with live preview.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum Filter {
    #[default]
    All,
    Dark,
    Light,
}

pub(super) struct Item {
    id: String,
    theme: Theme,
    dark: bool,
}

#[derive(Default)]
pub(super) struct ThemePicker {
    pub(super) open: bool,
    query: String,
    sel: usize,
    filter: Filter,
    items: Vec<Item>,
    /// Keep the selected row in view after the keyboard moved it.
    scroll: bool,
}

/// Whether a theme's background is dark (perceived brightness under half).
pub(super) fn is_dark(t: &Theme) -> bool {
    let (r, g, b) = match t.background {
        ratatui::style::Color::Rgb(r, g, b) => (f64::from(r), f64::from(g), f64::from(b)),
        _ => (0., 0., 0.),
    };
    0.299 * r + 0.587 * g + 0.114 * b < 128.
}

/// Indexes of `items` that pass `filter` and match `query`, best match first (list order when the query is empty).
pub(super) fn visible(items: &[Item], query: &str, filter: Filter) -> Vec<usize> {
    let mut hits: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| match filter {
            Filter::All => true,
            Filter::Dark => it.dark,
            Filter::Light => !it.dark,
        })
        .filter_map(|(i, it)| {
            uikit::fuzzy_score(query, &format!("{} {}", it.theme.name, it.id)).map(|s| (s, i))
        })
        .collect();
    if !query.trim().is_empty() {
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    hits.into_iter().map(|(_, i)| i).collect()
}

impl Desktop {
    pub(super) fn open_theme_picker(&mut self) {
        let items: Vec<Item> = crate::theme::all_names()
            .into_iter()
            .filter_map(|id| {
                let theme = Theme::load_native(&id).ok()?;
                Some(Item {
                    dark: is_dark(&theme),
                    id,
                    theme,
                })
            })
            .collect();
        let current = self.committed_theme.name.clone();
        let sel = items.iter().position(|i| i.theme.name == current).unwrap_or(0);
        self.theme_picker = ThemePicker {
            open: true,
            query: String::new(),
            sel,
            filter: Filter::All,
            items,
            scroll: true,
        };
        self.palette = false;
    }

    pub(super) fn theme_picker_ui(&mut self, ctx: &egui::Context) {
        let committed = self.committed_theme.name.clone();
        let shown = visible(
            &self.theme_picker.items,
            &self.theme_picker.query,
            self.theme_picker.filter,
        );
        let (down, up, enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
            )
        });
        let tp = &mut self.theme_picker;
        let mut pos = shown.iter().position(|i| *i == tp.sel).unwrap_or(0);
        if down && pos + 1 < shown.len() {
            pos += 1;
            tp.scroll = true;
        }
        if up && pos > 0 {
            pos -= 1;
            tp.scroll = true;
        }
        if let Some(i) = shown.get(pos) {
            tp.sel = *i;
        }
        // The selected row is previewed on the whole window; closing without choosing reverts it.
        let preview = shown.get(pos).map(|i| tp.items[*i].theme.clone());
        if let Some(p) = preview {
            if p.name != self.theme.name {
                self.theme = p;
            }
        }
        let t = self.theme.clone();
        let mut choose: Option<String> = None;
        let mut close = esc;
        egui::Window::new("themes")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0., 70.])
            .fixed_size([480., 0.])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.theme_picker.query)
                            .hint_text("Search themes…")
                            .desired_width(300.),
                    );
                    edit.request_focus();
                    if edit.changed() {
                        self.theme_picker.sel = usize::MAX;
                        self.theme_picker.scroll = true;
                    }
                    for (label, f) in [("All", Filter::All), ("Dark", Filter::Dark), ("Light", Filter::Light)] {
                        if ui
                            .selectable_label(self.theme_picker.filter == f, label)
                            .clicked()
                        {
                            self.theme_picker.filter = f;
                            self.theme_picker.sel = usize::MAX;
                            self.theme_picker.scroll = true;
                        }
                    }
                });
                ui.add_space(6.);
                egui::ScrollArea::vertical().max_height(380.).show(ui, |ui| {
                    for (row, idx) in shown.iter().enumerate() {
                        let it = &self.theme_picker.items[*idx];
                        let selected = row == pos;
                        let (rect, resp) = ui
                            .allocate_exact_size(Vec2::new(ui.available_width(), 34.), Sense::click());
                        if selected {
                            ui.painter()
                                .rect_filled(rect, 6., color(t.accent).gamma_multiply(0.3));
                            if self.theme_picker.scroll {
                                resp.scroll_to_me(None);
                            }
                        } else if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                6.,
                                color(mix(t.panel, t.foreground, 0.1)),
                            );
                        }
                        // The theme in its own colors: "Aa" and a strip of its palette.
                        let tile = Rect::from_min_size(
                            rect.left_top() + Vec2::new(6., 4.),
                            Vec2::new(150., 26.),
                        );
                        let th = &it.theme;
                        ui.painter().rect_filled(tile, 5., color(th.background));
                        ui.painter().text(
                            tile.left_center() + Vec2::new(8., 0.),
                            egui::Align2::LEFT_CENTER,
                            "Aa",
                            FontId::monospace(13.),
                            color(th.foreground),
                        );
                        let dots = [
                            th.accent,
                            th.success,
                            th.warning,
                            th.ansi[1],
                            th.ansi[4],
                            th.ansi[5],
                            th.ansi[6],
                        ];
                        for (n, c) in dots.iter().enumerate() {
                            ui.painter().circle_filled(
                                tile.left_center() + Vec2::new(40. + n as f32 * 15., 0.),
                                5.,
                                color(*c),
                            );
                        }
                        ui.painter().text(
                            rect.left_center() + Vec2::new(170., 0.),
                            egui::Align2::LEFT_CENTER,
                            &th.name,
                            FontId::proportional(13.),
                            color(t.foreground),
                        );
                        let tag = if th.name == committed {
                            "✓ current"
                        } else if it.dark {
                            "dark"
                        } else {
                            "light"
                        };
                        ui.painter().text(
                            rect.right_center() - Vec2::new(10., 0.),
                            egui::Align2::RIGHT_CENTER,
                            tag,
                            FontId::proportional(11.),
                            color(t.muted),
                        );
                        if resp.clicked() {
                            choose = Some(it.id.clone());
                        }
                    }
                    if shown.is_empty() {
                        ui.label(
                            RichText::new("No theme matches.")
                                .size(12.)
                                .color(color(t.muted)),
                        );
                    }
                });
                ui.add_space(4.);
                ui.label(
                    RichText::new("↑↓ preview · Enter use it · Esc cancel")
                        .size(11.)
                        .color(color(t.muted)),
                );
            });
        self.theme_picker.scroll = false;
        if enter && choose.is_none() {
            choose = shown.get(pos).map(|i| self.theme_picker.items[*i].id.clone());
        }
        if let Some(id) = choose {
            self.theme_picker.open = false;
            self.run_action(ActionKind::Theme(id));
        } else if close {
            self.theme_picker.open = false;
            close = false;
        }
        let _ = close;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;

    #[test]
    fn dark_and_light_themes_are_told_apart() {
        assert!(is_dark(&Theme::load("mac27-dark").unwrap()));
        assert!(!is_dark(&Theme::load("mac27-light").unwrap()));
        assert!(!is_dark(&Theme::load("tokyo-night-day").unwrap()));
    }
    #[test]
    fn the_list_filters_by_text_and_by_dark_or_light() {
        let items: Vec<Item> = ["mac27-dark", "mac27-light", "dracula", "nord"]
            .iter()
            .map(|id| {
                let theme = Theme::load(id).unwrap();
                Item {
                    id: id.to_string(),
                    dark: is_dark(&theme),
                    theme,
                }
            })
            .collect();
        assert_eq!(visible(&items, "", Filter::All).len(), 4);
        let light = visible(&items, "", Filter::Light);
        assert_eq!(light.len(), 1);
        assert_eq!(items[light[0]].id, "mac27-light");
        let hits = visible(&items, "drac", Filter::All);
        assert_eq!(items[hits[0]].id, "dracula");
        assert!(visible(&items, "zzzz", Filter::All).is_empty());
    }
    #[test]
    fn previewing_shows_a_theme_and_closing_reverts_while_choosing_keeps_it() {
        let (mut app, _rx) = app_with_screen("");
        let before = app.committed_theme.name.clone();
        app.open_theme_picker();
        assert!(app.theme_picker.open && !app.theme_picker.items.is_empty());
        let ctx = egui::Context::default();
        // Move the selection to another theme and run a frame: it is previewed, not committed.
        let other = app
            .theme_picker
            .items
            .iter()
            .position(|i| i.theme.name != before)
            .unwrap();
        app.theme_picker.sel = other;
        let preview = app.theme_picker.items[other].theme.name.clone();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
        assert_eq!(app.theme.name, preview);
        assert_eq!(app.committed_theme.name, before);
        // Escape closes and the next frame puts the committed theme back.
        let esc = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            ..Default::default()
        };
        let _ = ctx.run(esc, |ctx| app.frame(ctx));
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
        assert!(!app.theme_picker.open);
        assert_eq!(app.theme.name, before);
        // Choosing through the action commits it.
        app.open_theme_picker();
        let id = app.theme_picker.items[other].id.clone();
        app.run_action(ActionKind::Theme(id));
        assert_eq!(app.committed_theme.name, preview);
    }
}

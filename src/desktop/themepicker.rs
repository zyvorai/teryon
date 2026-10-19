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
        let sel = items
            .iter()
            .position(|i| i.theme.name == current)
            .unwrap_or(0);
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
        const COLS: usize = 2;
        let committed = self.committed_theme.name.clone();
        let shown = visible(
            &self.theme_picker.items,
            &self.theme_picker.query,
            self.theme_picker.filter,
        );
        let (down, up, left, right, enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
            )
        });
        let tp = &mut self.theme_picker;
        let mut pos = shown.iter().position(|i| *i == tp.sel).unwrap_or(0);
        let last = shown.len().saturating_sub(1);
        // ←/→ move by one only while the search box is empty (otherwise they move its caret).
        let before = pos;
        if down {
            pos = (pos + COLS).min(last);
        }
        if up {
            pos = pos.saturating_sub(COLS);
        }
        if right && tp.query.is_empty() {
            pos = (pos + 1).min(last);
        }
        if left && tp.query.is_empty() {
            pos = pos.saturating_sub(1);
        }
        if pos != before {
            tp.scroll = true;
        }
        if let Some(i) = shown.get(pos) {
            tp.sel = *i;
        }
        // The selected theme is previewed on the whole window; closing without choosing reverts it.
        let preview = shown.get(pos).map(|i| tp.items[*i].theme.clone());
        if let Some(p) = preview {
            if p.name != self.theme.name {
                self.theme = p;
            }
        }
        let t = self.theme.clone();
        let mut choose: Option<String> = None;
        let screen = ctx.screen_rect();
        let width = (screen.width() - 80.).clamp(340., 800.);
        let list_h = (screen.height() - 230.).clamp(180., 600.);
        let cols = if width >= 640. { COLS } else { 1 };
        egui::Window::new("themes")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0., 40.])
            .fixed_size([width, 0.])
            .frame(
                egui::Frame::window(&ctx.style())
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Choose a theme").size(20.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{} themes", shown.len()))
                                .size(12.)
                                .color(color(t.muted)),
                        );
                    });
                });
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.theme_picker.query)
                            .hint_text("Search…")
                            .font(FontId::proportional(15.))
                            .margin(egui::Margin::symmetric(10, 7))
                            .desired_width((width - 36.) - 210.),
                    );
                    edit.request_focus();
                    if edit.changed() {
                        self.theme_picker.sel = usize::MAX;
                        self.theme_picker.scroll = true;
                    }
                    for (label, f) in [
                        ("All", Filter::All),
                        ("Dark", Filter::Dark),
                        ("Light", Filter::Light),
                    ] {
                        let on = self.theme_picker.filter == f;
                        let chip = egui::Button::new(RichText::new(label).size(13.).color(if on {
                            color(t.background)
                        } else {
                            color(t.foreground)
                        }))
                        .fill(if on {
                            color(t.accent)
                        } else {
                            color(mix(t.panel, t.foreground, 0.08))
                        })
                        .corner_radius(egui::CornerRadius::same(14))
                        .min_size(Vec2::new(54., 30.));
                        if ui.add(chip).clicked() {
                            self.theme_picker.filter = f;
                            self.theme_picker.sel = usize::MAX;
                            self.theme_picker.scroll = true;
                        }
                    }
                });
                ui.add_space(12.);
                egui::ScrollArea::vertical()
                    .max_height(list_h)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(12.);
                        let gap = 12.;
                        let card_w =
                            (ui.available_width() - gap * (cols as f32 - 1.)) / cols as f32;
                        for chunk in shown.chunks(cols).enumerate().map(|(n, c)| (n, c.to_vec())) {
                            let (n, chunk) = chunk;
                            ui.horizontal(|ui| {
                                for (k, idx) in chunk.iter().enumerate() {
                                    let row = n * cols + k;
                                    let it = &self.theme_picker.items[*idx];
                                    let selected = row == pos;
                                    let (rect, resp) = ui.allocate_exact_size(
                                        Vec2::new(card_w, 142.),
                                        Sense::click(),
                                    );
                                    if selected && self.theme_picker.scroll {
                                        resp.scroll_to_me(None);
                                    }
                                    paint_card(
                                        ui,
                                        rect,
                                        it,
                                        selected,
                                        resp.hovered(),
                                        it.theme.name == committed,
                                        &t,
                                    );
                                    if resp.clicked() {
                                        choose = Some(it.id.clone());
                                    }
                                }
                            });
                        }
                        if shown.is_empty() {
                            ui.add_space(20.);
                            ui.label(
                                RichText::new("No theme matches.")
                                    .size(13.)
                                    .color(color(t.muted)),
                            );
                        }
                    });
                ui.add_space(10.);
                ui.label(
                    RichText::new("↑ ↓ ← → preview live   ·   Enter use it   ·   Esc cancel")
                        .size(12.)
                        .color(color(t.muted)),
                );
            });
        self.theme_picker.scroll = false;
        if enter && choose.is_none() {
            choose = shown
                .get(pos)
                .map(|i| self.theme_picker.items[*i].id.clone());
        }
        if let Some(id) = choose {
            self.theme_picker.open = false;
            self.run_action(ActionKind::Theme(id));
        } else if esc {
            self.theme_picker.open = false;
        }
    }
}

/// Text in several colors on one line starting at `pos`; returns where it ended.
fn segments(painter: &egui::Painter, mut pos: Pos2, font: &FontId, parts: &[(&str, Color32)]) {
    for (text, c) in parts {
        let galley = painter.layout_no_wrap((*text).to_string(), font.clone(), *c);
        let w = galley.size().x;
        painter.galley(pos, galley, *c);
        pos.x += w;
    }
}

/// One theme as a small terminal: its background, a few lines in its colors, and its palette.
fn paint_card(
    ui: &egui::Ui,
    rect: Rect,
    it: &Item,
    selected: bool,
    hovered: bool,
    current: bool,
    ui_theme: &Theme,
) {
    let th = &it.theme;
    let p = ui.painter();
    let radius = egui::CornerRadius::same(12);
    if selected {
        p.rect_filled(
            rect.expand(4.),
            egui::CornerRadius::same(15),
            color(ui_theme.accent).gamma_multiply(0.30),
        );
    }
    p.rect_filled(rect, radius, color(th.background));
    let edge = if selected {
        Stroke::new(2.5_f32, color(ui_theme.accent))
    } else if hovered {
        Stroke::new(1.5_f32, color(mix(th.background, th.foreground, 0.45)))
    } else {
        Stroke::new(1_f32, color(mix(th.background, th.foreground, 0.2)))
    };
    p.rect_stroke(rect, radius, edge, egui::StrokeKind::Inside);
    let fg = color(th.foreground);
    let mono = FontId::monospace(12.5);
    let x = rect.left() + 16.;
    p.text(
        Pos2::new(x, rect.top() + 14.),
        egui::Align2::LEFT_TOP,
        &th.name,
        FontId::proportional(15.),
        fg,
    );
    let tag = if current {
        "✓ current"
    } else if it.dark {
        "dark"
    } else {
        "light"
    };
    p.text(
        Pos2::new(rect.right() - 16., rect.top() + 16.),
        egui::Align2::RIGHT_TOP,
        tag,
        FontId::proportional(11.),
        color(mix(th.background, th.foreground, 0.6)),
    );
    let y = rect.top() + 42.;
    let lh = 17.;
    segments(
        p,
        Pos2::new(x, y),
        &mono,
        &[("❯ ", color(th.accent)), ("cargo test", fg)],
    );
    segments(
        p,
        Pos2::new(x, y + lh),
        &mono,
        &[
            ("✓ ", color(th.success)),
            ("12 passed  ", fg),
            ("✗ ", color(th.ansi[1])),
            ("1 failed", fg),
        ],
    );
    segments(
        p,
        Pos2::new(x, y + lh * 2.),
        &mono,
        &[("warning: ", color(th.warning)), ("unused variable", fg)],
    );
    segments(
        p,
        Pos2::new(x, y + lh * 3.),
        &mono,
        &[
            ("src/ ", color(th.ansi[4])),
            ("Cargo.toml ", color(th.ansi[6])),
            ("run.sh", color(th.ansi[2])),
        ],
    );
    // The palette along the bottom edge.
    let dots = [
        th.accent, th.success, th.warning, th.ansi[1], th.ansi[2], th.ansi[3], th.ansi[4],
        th.ansi[5], th.ansi[6],
    ];
    for (n, c) in dots.iter().enumerate() {
        p.circle_filled(
            Pos2::new(
                rect.right() - 20. - (dots.len() - 1 - n) as f32 * 14.,
                rect.bottom() - 14.,
            ),
            4.5,
            color(*c),
        );
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

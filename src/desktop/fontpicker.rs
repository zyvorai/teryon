//! Choose the terminal font from the monospaced fonts installed on this Mac, each drawn in its own typeface.
use super::*;
use crate::configedit::{self, Val};
use crate::fontscan::{self, FontFile};

pub(super) struct Item {
    /// What `font_family` becomes; `None` is the built-in SF Mono.
    family: Option<String>,
    label: String,
    /// Index of the `pick-N` family this item is drawn in (`None`: the built-in monospace).
    pick: Option<usize>,
}

#[derive(Default)]
pub(super) struct FontPicker {
    pub(super) open: bool,
    query: String,
    sel: usize,
    items: Vec<Item>,
    scroll: bool,
    /// Font file bytes, registered as `pick-N` families on the next frame that has a context.
    bytes: Vec<Vec<u8>>,
    /// The `pick-N` families exist (egui applies new fonts on the frame after they are set).
    ready: bool,
    installing: bool,
}

/// Items matching `query`, in list order unless the query ranks them.
pub(super) fn visible(items: &[Item], query: &str) -> Vec<usize> {
    let mut hits: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| uikit::fuzzy_score(query, &it.label).map(|s| (s, i)))
        .collect();
    if !query.trim().is_empty() {
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    hits.into_iter().map(|(_, i)| i).collect()
}

/// The picker's items and the bytes to register, from the fonts found.
fn build_items(files: &[FontFile], current: Option<&str>) -> (Vec<Item>, Vec<Vec<u8>>, usize) {
    let mut items = vec![Item {
        family: None,
        label: "Default (SF Mono)".into(),
        pick: None,
    }];
    let mut bytes: Vec<Vec<u8>> = Vec::new();
    for f in files.iter().take(60) {
        let Ok(data) = std::fs::read(&f.path) else {
            continue;
        };
        items.push(Item {
            family: Some(f.stem.clone()),
            label: f.label.clone(),
            pick: Some(bytes.len()),
        });
        bytes.push(data);
    }
    let sel = current
        .and_then(|c| items.iter().position(|i| i.family.as_deref() == Some(c)))
        .unwrap_or(0);
    (items, bytes, sel)
}

impl Desktop {
    pub(super) fn open_font_picker(&mut self) {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let files = fontscan::scan(&fontscan::font_dirs(home.as_deref()));
        let (items, bytes, sel) = build_items(&files, self.cfg.font_family.as_deref());
        self.font_picker = FontPicker {
            open: true,
            query: String::new(),
            sel,
            items,
            scroll: true,
            bytes: if self.persist { bytes } else { Vec::new() },
            ready: false,
            installing: false,
        };
        self.palette = false;
    }
    fn close_font_picker(&mut self) {
        self.font_picker.open = false;
        self.font_picker.items.clear();
        self.font_picker.bytes.clear();
        // Put the normal fonts back (the frame notices and reinstalls them).
        self.applied_font = None;
    }

    pub(super) fn font_picker_ui(&mut self, ctx: &egui::Context) {
        const COLS: usize = 2;
        if !self.font_picker.bytes.is_empty() {
            let bytes = std::mem::take(&mut self.font_picker.bytes);
            style::install_picker_fonts(ctx, &bytes);
            self.font_picker.installing = true;
            ctx.request_repaint();
            return;
        }
        if std::mem::take(&mut self.font_picker.installing) {
            self.font_picker.ready = true;
        }
        let shown = visible(&self.font_picker.items, &self.font_picker.query);
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
        let fp = &mut self.font_picker;
        let mut pos = shown.iter().position(|i| *i == fp.sel).unwrap_or(0);
        let last = shown.len().saturating_sub(1);
        let before = pos;
        if down {
            pos = (pos + COLS).min(last);
        }
        if up {
            pos = pos.saturating_sub(COLS);
        }
        if right && fp.query.is_empty() {
            pos = (pos + 1).min(last);
        }
        if left && fp.query.is_empty() {
            pos = pos.saturating_sub(1);
        }
        if pos != before {
            fp.scroll = true;
        }
        if let Some(i) = shown.get(pos) {
            fp.sel = *i;
        }
        let t = self.theme.clone();
        let screen = ctx.screen_rect();
        let width = (screen.width() - 80.).clamp(340., 800.);
        let list_h = (screen.height() - 230.).clamp(180., 600.);
        let cols = if width >= 640. { COLS } else { 1 };
        let mut choose: Option<usize> = None;
        egui::Window::new("fonts")
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
                    ui.label(RichText::new("Choose a font").size(20.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{} monospaced fonts", shown.len()))
                                .size(12.)
                                .color(color(t.muted)),
                        );
                    });
                });
                ui.add_space(8.);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.font_picker.query)
                        .hint_text("Search…")
                        .font(FontId::proportional(15.))
                        .margin(egui::Margin::symmetric(10, 7))
                        .desired_width(f32::INFINITY),
                );
                edit.request_focus();
                if edit.changed() {
                    self.font_picker.sel = usize::MAX;
                    self.font_picker.scroll = true;
                }
                ui.add_space(12.);
                egui::ScrollArea::vertical().max_height(list_h).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(12.);
                    let card_w = (ui.available_width() - 12. * (cols as f32 - 1.)) / cols as f32;
                    for (n, chunk) in shown.chunks(cols).enumerate() {
                        ui.horizontal(|ui| {
                            for (k, idx) in chunk.iter().enumerate() {
                                let selected = n * cols + k == pos;
                                let (rect, resp) =
                                    ui.allocate_exact_size(Vec2::new(card_w, 118.), Sense::click());
                                if selected && self.font_picker.scroll {
                                    resp.scroll_to_me(None);
                                }
                                let it = &self.font_picker.items[*idx];
                                let current = it.family == self.cfg.font_family;
                                let ready = self.font_picker.ready;
                                paint_card(ui, rect, it, selected, resp.hovered(), current, ready, &t);
                                if resp.clicked() {
                                    choose = Some(*idx);
                                }
                            }
                        });
                    }
                    if shown.is_empty() {
                        ui.add_space(20.);
                        ui.label(RichText::new("No font matches.").size(13.).color(color(t.muted)));
                    }
                });
                ui.add_space(10.);
                ui.label(
                    RichText::new("↑ ↓ ← → move   ·   Enter use it   ·   Esc cancel   ·   only monospaced fonts are listed")
                        .size(12.)
                        .color(color(t.muted)),
                );
            });
        self.font_picker.scroll = false;
        if enter && choose.is_none() {
            choose = shown.get(pos).copied();
        }
        if let Some(i) = choose {
            let family = self.font_picker.items[i].family.clone();
            self.close_font_picker();
            self.set_font_family(family);
        } else if esc {
            self.close_font_picker();
        }
    }

    /// Use `family` for the terminal (`None`: the built-in) and write it to the config file.
    pub(super) fn set_font_family(&mut self, family: Option<String>) {
        self.cfg.font_family = family.clone();
        let result = configedit::apply(|text| match &family {
            Some(f) => configedit::set_desktop(text, "font_family", &Val::Str(f.clone())),
            None => configedit::unset_desktop(text, "font_family"),
        });
        match result {
            Ok(()) => self.config_mtime = Desktop::config_stamp(),
            Err(e) => self.toast(format!("Could not save the font: {e:#}")),
        }
    }
}

/// One font as a card: its name and a few lines of code set in the font itself.
#[allow(clippy::too_many_arguments)]
fn paint_card(
    ui: &egui::Ui,
    rect: Rect,
    it: &Item,
    selected: bool,
    hovered: bool,
    current: bool,
    ready: bool,
    t: &Theme,
) {
    let p = ui.painter();
    let radius = egui::CornerRadius::same(12);
    if selected {
        p.rect_filled(
            rect.expand(4.),
            egui::CornerRadius::same(15),
            color(t.accent).gamma_multiply(0.30),
        );
    }
    p.rect_filled(rect, radius, color(mix(t.background, t.foreground, 0.05)));
    let edge = if selected {
        Stroke::new(2.5_f32, color(t.accent))
    } else if hovered {
        Stroke::new(1.5_f32, color(mix(t.background, t.foreground, 0.45)))
    } else {
        Stroke::new(1_f32, color(mix(t.background, t.foreground, 0.2)))
    };
    p.rect_stroke(rect, radius, edge, egui::StrokeKind::Inside);
    p.text(
        Pos2::new(rect.left() + 16., rect.top() + 12.),
        egui::Align2::LEFT_TOP,
        &it.label,
        FontId::proportional(14.),
        color(t.foreground),
    );
    if current {
        p.text(
            Pos2::new(rect.right() - 16., rect.top() + 14.),
            egui::Align2::RIGHT_TOP,
            "✓ in use",
            FontId::proportional(11.),
            color(t.muted),
        );
    }
    let family = match it.pick.filter(|_| ready) {
        Some(n) => egui::FontFamily::Name(format!("pick-{n}").into()),
        None => egui::FontFamily::Monospace,
    };
    let font = FontId::new(14., family);
    let x = rect.left() + 16.;
    let y = rect.top() + 38.;
    for (n, (text, c)) in [
        ("❯ cargo build --release", color(t.accent)),
        ("fn main() { 0O 1lI \"a\" != 'b' }", color(t.foreground)),
        ("→ ≥ ≠ ✓ ✗   { } [ ] ( ) ; : ::", color(t.muted)),
    ]
    .iter()
    .enumerate()
    {
        p.text(
            Pos2::new(x, y + n as f32 * 22.),
            egui::Align2::LEFT_TOP,
            *text,
            font.clone(),
            *c,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;

    fn files() -> Vec<FontFile> {
        ["Alpha Mono", "Beta Code", "Gamma"]
            .iter()
            .map(|n| FontFile {
                path: std::path::PathBuf::from("/nonexistent/x.ttf"),
                stem: format!("{n}-Regular").replace(' ', ""),
                label: n.to_string(),
            })
            .collect()
    }
    #[test]
    fn unreadable_files_are_skipped_and_the_current_font_is_preselected() {
        let (items, bytes, sel) = build_items(&files(), None);
        assert_eq!(
            items.len(),
            1,
            "only the default: none of the files can be read"
        );
        assert!(bytes.is_empty() && sel == 0);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Real-Regular.ttf");
        std::fs::write(&path, b"fontbytes").unwrap();
        let real = vec![FontFile {
            path,
            stem: "Real-Regular".into(),
            label: "Real".into(),
        }];
        let (items, bytes, sel) = build_items(&real, Some("Real-Regular"));
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].pick, Some(0));
        assert_eq!(bytes[0], b"fontbytes");
        assert_eq!(sel, 1);
    }
    #[test]
    fn the_list_filters_by_name() {
        let mk = |l: &str| Item {
            family: Some(l.into()),
            label: l.into(),
            pick: None,
        };
        let items = vec![mk("Menlo"), mk("Hack"), mk("JetBrains Mono")];
        assert_eq!(visible(&items, "").len(), 3);
        assert_eq!(visible(&items, "jet"), [2]);
        assert!(visible(&items, "zzz").is_empty());
    }
    #[test]
    fn the_picker_opens_renders_and_cancel_restores_the_fonts_while_choosing_saves_nothing_in_tests(
    ) {
        let (mut app, _rx) = app_with_screen("");
        let ctx = egui::Context::default();
        app.open_font_picker();
        assert!(app.font_picker.open && !app.font_picker.items.is_empty());
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
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
        assert!(!app.font_picker.open);
        assert_eq!(
            app.applied_font, None,
            "the normal fonts are reinstalled next frame"
        );
    }
}

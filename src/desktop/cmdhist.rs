//! ⌘⇧H: search the command history recorded by the shell integration and put the pick at the
//! prompt (Enter) or run it (⌥Enter). Same list as Ctrl-R, shown without leaving the window.
use super::*;
use crate::{history, picker};

/// Rows shown at once.
const ROWS: usize = 10;

#[derive(Default)]
pub(super) struct Bar {
    pub open: bool,
    input: String,
    focus: bool,
    state: Option<picker::State>,
    pane: u64,
}

impl Desktop {
    pub(super) fn cmdhist_toggle(&mut self) {
        if self.cmdhist.open {
            self.cmdhist.open = false;
            return;
        }
        if self.host != "local" {
            self.toast("Command history is kept on this Mac; open a local workspace to search it");
            return;
        }
        let cwd = self
            .panes
            .get(&self.focus)
            .map(|p| p.info.cwd.clone())
            .unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let home = std::env::var("HOME").unwrap_or_default();
        let entries = history::load_all();
        let items = picker::history_items(&entries, &cwd, &home, now);
        let runs = picker::history_runs(&entries, &cwd, &home, now);
        if items.is_empty() {
            self.toast("No command history yet: load the shell integration (teryon setup) and run a command");
            return;
        }
        self.cmdhist.state = Some(picker::State::with_all(items, runs, "", true));
        self.cmdhist.input.clear();
        self.cmdhist.pane = self.focus;
        self.cmdhist.focus = true;
        self.cmdhist.open = true;
    }

    pub(super) fn cmdhist_ui(&mut self, ctx: &egui::Context) {
        if !self.cmdhist.open {
            return;
        }
        let close = |this: &mut Self| {
            this.cmdhist.open = false;
            let pane = this.cmdhist.pane;
            ctx.memory_mut(|m| m.request_focus(egui::Id::new(("teryon-terminal", pane))));
        };
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            close(self);
            return;
        }
        let (up, down, tab) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::Tab),
            )
        });
        let Some(state) = self.cmdhist.state.as_mut() else {
            self.cmdhist.open = false;
            return;
        };
        if up {
            state.move_by(-1);
        }
        if down {
            state.move_by(1);
        }
        if tab {
            state.toggle_scope();
        }
        let t = self.theme.clone();
        let mut choose: Option<(String, bool)> = None;
        egui::Area::new(egui::Id::new("teryon-cmdhist"))
            .anchor(egui::Align2::CENTER_TOP, [0., bar::BAR_HEIGHT + 24.])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color(mix(t.panel, t.foreground, 0.08)))
                    .stroke(Stroke::new(1_f32, color(t.border)))
                    .corner_radius(egui::CornerRadius::same(14))
                    .shadow(egui::Shadow {
                        offset: [0, 12],
                        blur: 32,
                        spread: 0,
                        color: Color32::from_black_alpha(90),
                    })
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(640.);
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.cmdhist.input)
                                .desired_width(f32::INFINITY)
                                .font(FontId::proportional(15.))
                                .hint_text("⌕  Search command history…"),
                        );
                        if self.cmdhist.focus {
                            edit.request_focus();
                            self.cmdhist.focus = false;
                        }
                        state.set_query(&self.cmdhist.input);
                        let alt = ui.input(|i| i.modifiers.alt);
                        if edit.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            if let Some(text) = state.selected_text() {
                                choose = Some((text, alt));
                            }
                        }
                        ui.add_space(6.);
                        let first = state.selected.saturating_sub(ROWS - 1);
                        let shown: Vec<usize> =
                            state.hits().iter().copied().skip(first).take(ROWS).collect();
                        for (row, index) in shown.into_iter().enumerate() {
                            let item = state.item(index).clone();
                            let on = first + row == state.selected;
                            let text = item.text.replace('\n', "⏎");
                            let response = ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(if on { "▸" } else { " " })
                                        .color(color(t.accent)),
                                );
                                let label = RichText::new(text).monospace().size(13.).color(
                                    color(if on { t.foreground } else { mix(t.foreground, t.background, 0.25) }),
                                );
                                let clicked = ui.add(egui::Label::new(label).sense(egui::Sense::click())).clicked();
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(&item.detail).size(11.).color(color(t.muted)));
                                });
                                clicked
                            });
                            if response.inner {
                                choose = Some((item.text, alt));
                            }
                        }
                        if state.hits().is_empty() {
                            ui.label(RichText::new("No matches").size(12.).color(color(t.muted)));
                        }
                        ui.add_space(4.);
                        let scope = if state.only_local { " · this directory" } else { "" };
                        ui.label(
                            RichText::new(format!(
                                "{}/{}{scope} · Enter puts it at the prompt · ⌥Enter runs · Tab this directory · Esc closes",
                                state.hits().len(),
                                state.total()
                            ))
                            .size(11.)
                            .color(color(t.muted)),
                        );
                    });
            });
        if let Some((text, run)) = choose {
            self.insert_command(self.cmdhist.pane, &text, run);
            close(self);
        }
    }
}

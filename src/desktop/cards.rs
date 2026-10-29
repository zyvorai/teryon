//! A card in the top-right corner when an agent in another pane starts waiting for you while this
//! window is in front. (With the window in the background the macOS banner does that job.)
use super::*;
use crate::agents::{Kind, Status};

/// Cards shown at once; a newer one pushes the oldest out.
const MAX_CARDS: usize = 3;
/// A card nobody touched goes away after this long.
const CARD_LIFE: Duration = Duration::from_secs(120);
const CARD_WIDTH: f32 = 330.;
/// How long a new card takes to slide and fade in.
const SLIDE_SECS: f32 = 0.18;

pub(super) struct Card {
    pub pane: u64,
    pub kind: Kind,
    /// What the agent is asking, when it could be read.
    pub question: Option<String>,
    since: Instant,
}

enum Act {
    Answer(bool),
    Go,
    Dismiss,
}

impl Desktop {
    /// Show a card for the agent in `pane`, replacing the one it already has.
    pub(super) fn raise_card(&mut self, pane: u64, kind: Kind, question: Option<String>) {
        let cards = &mut self.extra.cards;
        cards.retain(|c| c.pane != pane);
        cards.push(Card {
            pane,
            kind,
            question,
            since: Instant::now(),
        });
        if cards.len() > MAX_CARDS {
            cards.remove(0);
        }
    }
    /// Drop cards whose agent was answered, left, or whose pane closed, and cards nobody touched.
    pub(super) fn prune_cards(&mut self) {
        let panes = &self.panes;
        self.extra.cards.retain(|c| {
            c.since.elapsed() < CARD_LIFE
                && panes
                    .get(&c.pane)
                    .is_some_and(|p| p.track.status == Some(Status::NeedsYou))
        });
    }
    pub(super) fn cards_ui(&mut self, ctx: &egui::Context) {
        if self.extra.cards.is_empty() {
            return;
        }
        let t = self.theme.clone();
        let can_answer = self.cfg.agent_quick_approve && !self.readonly;
        let accent = color(t.warning);
        let mut acts: Vec<(u64, Kind, Act)> = Vec::new();
        let mut animating = false;
        egui::Area::new(egui::Id::new("teryon-cards"))
            .anchor(egui::Align2::RIGHT_TOP, [-14., bar::BAR_HEIGHT + 12.])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(CARD_WIDTH);
                ui.spacing_mut().item_spacing.y = 8.;
                for card in &self.extra.cards {
                    let age = card.since.elapsed().as_secs_f32();
                    let fade = (age / SLIDE_SECS).clamp(0., 1.);
                    animating |= fade < 1.;
                    ui.add_space((1. - fade) * 14.);
                    let place = format!("{} · pane {}", self.workspace, card.pane);
                    let ask = card
                        .question
                        .clone()
                        .unwrap_or_else(|| "Waiting for your answer".into());
                    egui::Frame::new()
                        .fill(color(mix(t.panel, t.foreground, 0.08)).gamma_multiply(fade))
                        .stroke(Stroke::new(1_f32, accent.gamma_multiply(0.55 * fade)))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(14, 12))
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 6],
                            blur: 22,
                            spread: 0,
                            color: Color32::from_black_alpha((90. * fade) as u8),
                        })
                        .show(ui, |ui| {
                            ui.set_width(CARD_WIDTH - 28.);
                            ui.horizontal(|ui| {
                                let (dot, _) =
                                    ui.allocate_exact_size(Vec2::splat(9.), Sense::hover());
                                ui.painter().circle_filled(
                                    dot.center(),
                                    4.,
                                    accent.gamma_multiply(fade),
                                );
                                ui.label(
                                    weighted(
                                        ctx,
                                        format!("{} needs you", card.kind.name()),
                                        "semibold",
                                        13.5,
                                    )
                                    .color(color(t.foreground).gamma_multiply(fade)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let close = ui.add(
                                            egui::Button::new(
                                                RichText::new("✕").size(11.).color(color(t.muted)),
                                            )
                                            .frame(false),
                                        );
                                        if close.on_hover_text("Dismiss").clicked() {
                                            acts.push((card.pane, card.kind, Act::Dismiss));
                                        }
                                    },
                                );
                            });
                            ui.label(
                                RichText::new(place)
                                    .size(11.)
                                    .color(color(t.muted).gamma_multiply(fade)),
                            );
                            ui.add_space(2.);
                            ui.label(
                                RichText::new(ask)
                                    .size(12.5)
                                    .color(color(t.foreground).gamma_multiply(fade)),
                            );
                            ui.add_space(4.);
                            ui.horizontal(|ui| {
                                if can_answer {
                                    let yes = egui::Button::new(
                                        RichText::new("Approve").size(12.).color(color(t.panel)),
                                    )
                                    .fill(color(t.accent))
                                    .corner_radius(egui::CornerRadius::same(8));
                                    if ui.add(yes).clicked() {
                                        acts.push((card.pane, card.kind, Act::Answer(true)));
                                    }
                                    let no = egui::Button::new(
                                        RichText::new("Deny").size(12.).color(color(t.foreground)),
                                    )
                                    .corner_radius(egui::CornerRadius::same(8));
                                    if ui.add(no).clicked() {
                                        acts.push((card.pane, card.kind, Act::Answer(false)));
                                    }
                                }
                                let go = egui::Button::new(
                                    RichText::new("Go to pane").size(12.).color(color(t.muted)),
                                )
                                .frame(false);
                                if ui.add(go).clicked() {
                                    acts.push((card.pane, card.kind, Act::Go));
                                }
                            });
                        });
                }
            });
        for (pane, kind, act) in acts {
            match act {
                Act::Answer(yes) => self.answer_agent(pane, kind, yes),
                Act::Go => self.focus = pane,
                Act::Dismiss => {}
            }
            self.extra.cards.retain(|c| c.pane != pane);
        }
        if animating {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_app() -> Desktop {
        let (mut app, _rx) = crate::desktop::tests::app_with_screen("");
        app.window_focused = true;
        app.cfg.agent_notifications = true;
        app
    }

    #[test]
    fn a_card_replaces_its_panes_card_and_the_oldest_makes_room() {
        let mut app = card_app();
        app.raise_card(2, Kind::Claude, Some("one".into()));
        app.raise_card(2, Kind::Claude, Some("two".into()));
        assert_eq!(app.extra.cards.len(), 1);
        assert_eq!(app.extra.cards[0].question.as_deref(), Some("two"));
        for id in 3..8 {
            app.raise_card(id, Kind::Codex, None);
        }
        assert_eq!(app.extra.cards.len(), MAX_CARDS);
        assert_eq!(app.extra.cards[0].pane, 5, "the oldest were pushed out");
    }
}

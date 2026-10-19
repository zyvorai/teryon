//! Link health: the bottom status line and the "Relink" card shown when the connection to the
//! daemon drops while the shells keep running.
use super::*;

/// How long the link must be down before the card appears (short blips stay quiet).
const SHOW_AFTER: Duration = Duration::from_millis(1500);
/// Below this window width the status line is hidden.
const STATUS_MIN_WIDTH: f32 = 900.;
const STATUS_HEIGHT: f32 = 20.;

#[derive(Default)]
pub(super) struct Link {
    /// `arch · os` of the daemon's machine, once known.
    platform: String,
    /// When the daemon started (reported uptime counted back from the report).
    started: Option<Instant>,
    /// Reconnect attempt number and the planned wait before it.
    attempt: u32,
    retry_at: Option<Instant>,
    /// Last time the link came back after being down.
    relinked: Option<Instant>,
    /// The link has been down at least once since the last `Online`.
    was_down: bool,
    /// The user dismissed the card for this outage.
    hidden: bool,
    /// Screen bytes received, and a moving rate over about two seconds.
    rx_total: u64,
    rx_mark: Option<(Instant, u64)>,
    rx_rate: f64,
}

impl Link {
    pub(super) fn daemon(&mut self, platform: String, uptime: u64) {
        self.platform = platform;
        self.started = Instant::now().checked_sub(Duration::from_secs(uptime));
    }
    /// How long the daemon has held the shells, once it has said so.
    pub(super) fn uptime(&self) -> Option<String> {
        self.started.map(|s| span(s.elapsed()))
    }
    pub(super) fn retry(&mut self, attempt: u32, wait_ms: u64) {
        self.attempt = attempt;
        self.retry_at = Some(Instant::now() + Duration::from_millis(wait_ms));
        self.was_down = true;
    }
    pub(super) fn online(&mut self) {
        if self.was_down {
            self.relinked = Some(Instant::now());
        }
        self.was_down = false;
        self.hidden = false;
        self.attempt = 0;
        self.retry_at = None;
    }
    pub(super) fn received(&mut self, bytes: u64) {
        self.rx_total += bytes;
        let now = Instant::now();
        match self.rx_mark {
            None => self.rx_mark = Some((now, self.rx_total)),
            Some((at, total)) if at.elapsed() >= Duration::from_secs(2) => {
                self.rx_rate = (self.rx_total - total) as f64 / at.elapsed().as_secs_f64();
                self.rx_mark = Some((now, self.rx_total));
            }
            Some(_) => {}
        }
    }
}

/// The relink card shows once the link has been down long enough, while shells exist to reassure about.
fn card_visible(down_for: Option<Duration>, panes: usize, hidden: bool) -> bool {
    !hidden && panes > 0 && down_for.is_some_and(|d| d >= SHOW_AFTER)
}

/// `41s`, `12m 05s`, `2h 08m`, `3d 04h`.
fn span(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        3600..=86399 => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {:02}h", s / 86400, s % 86400 / 3600),
    }
}

fn rate(bytes_per_s: f64) -> String {
    if bytes_per_s >= 1_000_000. {
        format!("{:.1} MB/s", bytes_per_s / 1_000_000.)
    } else if bytes_per_s >= 1000. {
        format!("{:.1} kB/s", bytes_per_s / 1000.)
    } else {
        format!("{bytes_per_s:.0} B/s")
    }
}

impl Desktop {
    /// What the pane's agent is waiting on, if it is waiting on you.
    fn agent_status_of(&self, pane: u64) -> Option<&'static str> {
        self.agents_list()
            .into_iter()
            .find(|(id, _, s, ..)| *id == pane && *s == crate::agents::Status::NeedsYou)
            .map(|_| "waits for you")
    }
    /// Muted one-line summary under the panes: what the daemon holds and how the link behaved.
    pub(super) fn status_line(&mut self, ctx: &egui::Context) {
        if !self.cfg.status_line || ctx.screen_rect().width() < STATUS_MIN_WIDTH {
            return;
        }
        let t = self.theme.clone();
        let l = &self.link;
        let mut parts = vec![format!(
            "{} PTY{} held by teryond on {}",
            self.panes.len(),
            if self.panes.len() == 1 { "" } else { "s" },
            self.host
        )];
        parts.push(match l.relinked {
            Some(at) => format!("last relink {} ago · 0 inputs replayed", span(at.elapsed())),
            None => "no relink needed".to_string(),
        });
        if l.rx_total > 0 {
            parts.push(format!("rx {}", rate(l.rx_rate)));
        }
        if !l.platform.is_empty() {
            parts.push(l.platform.clone());
        }
        parts.push(self.theme.name.to_lowercase());
        let text = parts.join("  ·  ");
        egui::TopBottomPanel::bottom("teryon-status-line")
            .show_separator_line(false)
            .exact_height(STATUS_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(color(t.background).gamma_multiply(self.cfg.window_opacity))
                    .inner_margin(egui::Margin::symmetric(12, 2)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        RichText::new(text)
                            .monospace()
                            .size(10.5)
                            .color(color(t.muted)),
                    );
                });
            });
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    /// Centered card over a dimmed window while the link is down but the shells live on.
    pub(super) fn relink_ui(&mut self, ctx: &egui::Context) {
        let down_for = self.offline_since.map(|s| s.elapsed());
        if self.online {
            return;
        }
        if !card_visible(down_for, self.panes.len(), self.link.hidden) {
            return;
        }
        let t = self.theme.clone();
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.link.hidden = true;
            return;
        }
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("teryon-relink-scrim"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
                ui.painter()
                    .rect_filled(rect, 0., color(t.background).gamma_multiply(0.72));
            });
        let down = down_for.unwrap_or_default();
        let wait = self
            .link
            .retry_at
            .map(|at| at.saturating_duration_since(Instant::now()));
        let mut retry = false;
        let mut draft = false;
        egui::Area::new(egui::Id::new("teryon-relink"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color(mix(t.panel, t.background, 0.2)))
                    .stroke(Stroke::new(1_f32, color(t.border)))
                    .corner_radius(egui::CornerRadius::same(18))
                    .shadow(egui::Shadow {
                        offset: [0, 24],
                        blur: 60,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    })
                    .inner_margin(egui::Margin::same(26))
                    .show(ui, |ui| {
                        ui.set_width(470.);
                        ui.horizontal(|ui| {
                            brand_mark(ui, &t);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if let Some(w) = wait {
                                        ui.label(
                                            RichText::new(format!(
                                                "RETRY IN {:.1}s",
                                                w.as_secs_f32()
                                            ))
                                            .monospace()
                                            .size(11.)
                                            .color(color(t.warning)),
                                        );
                                    }
                                },
                            );
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} · link lost {} ago",
                                self.host,
                                span(down)
                            ))
                            .monospace()
                            .size(11.)
                            .color(color(t.muted)),
                        );
                        ui.label(
                            RichText::new("input paused · nothing will be replayed")
                                .monospace()
                                .size(11.)
                                .color(color(t.muted)),
                        );
                        ui.add_space(14.);
                        ui.label(
                            RichText::new(format!("RELINKING · ATTEMPT {}", self.link.attempt.max(1)))
                                .monospace()
                                .size(11.)
                                .color(color(t.accent)),
                        );
                        ui.add_space(4.);
                        ui.label(
                            weighted(ctx, "Your shells are still running.", "semibold", 20.)
                                .color(color(t.foreground)),
                        );
                        ui.add_space(4.);
                        ui.label(
                            RichText::new(format!(
                                "The connection dropped, not the work. teryond on {} is holding every PTY; you'll land exactly where you left off.",
                                self.host
                            ))
                            .size(13.)
                            .color(color(t.muted)),
                        );
                        ui.add_space(12.);
                        for (id, pane) in &self.panes {
                            let state = match (pane.info.exited, self.agent_status_of(*id)) {
                                (Some(_), _) => ("exited", t.warning),
                                (None, Some(s)) => (s, t.warning),
                                (None, None) => ("alive", t.success),
                            };
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").size(10.).color(color(state.1)));
                                ui.label(
                                    RichText::new(format!("{id:02}"))
                                        .monospace()
                                        .size(12.)
                                        .color(color(t.foreground)),
                                );
                                ui.label(
                                    RichText::new(&pane.info.title)
                                        .monospace()
                                        .size(12.)
                                        .color(color(t.muted)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(state.0)
                                                .monospace()
                                                .size(11.)
                                                .color(color(state.1)),
                                        );
                                    },
                                );
                            });
                        }
                        ui.add_space(16.);
                        ui.horizontal(|ui| {
                            retry = primary_button(ui, &t, "Retry now").clicked();
                            draft = ui.button("Open offline draft").clicked();
                        });
                    });
            });
        if retry {
            let _ = self.tx.try_send(Command::Reconnect);
        }
        if draft {
            self.link.hidden = true;
            self.draft_open = true;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn card_waits_for_a_real_outage_and_for_shells() {
        let long = Some(Duration::from_secs(3));
        assert!(card_visible(long, 2, false));
        assert!(!card_visible(Some(Duration::from_millis(300)), 2, false));
        assert!(!card_visible(None, 2, false));
        assert!(!card_visible(long, 0, false));
        assert!(!card_visible(long, 2, true));
    }
    #[test]
    fn spans_and_rates_read_naturally() {
        assert_eq!(span(Duration::from_secs(41)), "41s");
        assert_eq!(span(Duration::from_secs(12 * 60 + 5)), "12m 05s");
        assert_eq!(span(Duration::from_secs(2 * 3600 + 8 * 60)), "2h 08m");
        assert_eq!(span(Duration::from_secs(3 * 86400 + 4 * 3600)), "3d 04h");
        assert_eq!(rate(1200.), "1.2 kB/s");
        assert_eq!(rate(12.), "12 B/s");
    }
    #[test]
    fn link_remembers_a_relink_only_after_an_outage() {
        let mut l = Link::default();
        l.online();
        assert!(l.relinked.is_none());
        l.retry(1, 250);
        l.online();
        assert!(l.relinked.is_some());
        l.daemon("arm64 · macos".into(), 90);
        assert!(l.started.is_some());
    }
}

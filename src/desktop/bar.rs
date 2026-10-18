//! The single slim top bar (brand, tabs, pane chips, layout, status) and the toast overlay.
use super::*;

/// Height of the bar; the macOS traffic lights sit inside it.
pub(super) const BAR_HEIGHT: f32 = 30.;
/// Space kept free for the macOS window buttons when the title bar is merged into the bar.
const TRAFFIC_LIGHTS: f32 = 78.;
const TOAST_SECONDS: f32 = 4.;

impl Desktop {
    /// Turn changes of `notice` into toasts (the notice itself stays as the latest message).
    pub(super) fn sync_toast(&mut self) {
        if self.notice != self.toasted {
            self.toasted = self.notice.clone();
            if !self.notice.is_empty() {
                self.toast = Some((self.notice.clone(), Instant::now()));
            }
        }
    }
    pub(super) fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((text, since)) = self.toast.clone() else {
            return;
        };
        let age = since.elapsed().as_secs_f32();
        if age > TOAST_SECONDS {
            self.toast = None;
            return;
        }
        let fade = ((TOAST_SECONDS - age) / 0.8).clamp(0., 1.);
        let t = &self.theme;
        egui::Area::new(egui::Id::new("teryon-toast"))
            .anchor(egui::Align2::LEFT_BOTTOM, [12., -12.])
            .interactable(false)
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color(mix(t.panel, t.foreground, 0.08)).gamma_multiply(fade))
                    .stroke(Stroke::new(1_f32, color(t.border).gamma_multiply(fade)))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(text)
                                .size(12.)
                                .color(color(t.foreground).gamma_multiply(fade)),
                        );
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(60));
    }
    /// One row holding everything the old sidebar and status bar showed.
    pub(super) fn top_bar(&mut self, ctx: &egui::Context) {
        let t = self.theme.clone();
        let width = ctx.screen_rect().width();
        let wide = width >= 760.;
        let compact = width < 520.;
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let gap = if cfg!(target_os = "macos") && !fullscreen {
            TRAFFIC_LIGHTS
        } else {
            4.
        };
        egui::TopBottomPanel::top("chrome")
            .exact_height(BAR_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(color(t.panel).gamma_multiply(self.cfg.window_opacity))
                    .inner_margin(egui::Margin::symmetric(6, 3)),
            )
            .show(ctx, |ui| {
                // Empty bar space drags the window and double-click maximizes it.
                let drag = ui.interact(
                    ui.max_rect(),
                    egui::Id::new("teryon-bar-drag"),
                    Sense::click_and_drag(),
                );
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                ui.horizontal_centered(|ui| {
                    ui.add_space(gap - 6.);
                    brand_mark(ui, &t);
                    ui.add_space(2.);
                    if self.tabs.len() > 1 {
                        let many_hosts = self.tabs.iter().any(|t| t.host != self.tabs[0].host);
                        let items: Vec<(usize, String)> = self
                            .tabs
                            .iter()
                            .enumerate()
                            .map(|(i, tab)| {
                                let label = if many_hosts {
                                    format!("{}/{}", tab.host, tab.name)
                                } else {
                                    tab.name.clone()
                                };
                                (i, label)
                            })
                            .collect();
                        let mut pick = self.active;
                        segmented(ui, &t, &mut pick, &items);
                        if pick != self.active {
                            self.activate(pick);
                        }
                        if icon_button(ui, &t, Icon::Close, "Detach this tab").clicked() {
                            self.close_tab();
                        }
                    } else if !compact {
                        ui.label(
                            weighted(
                                ctx,
                                format!("{} / {}", self.host, self.workspace),
                                "semibold",
                                12.,
                            )
                            .color(color(t.foreground)),
                        );
                    }
                    if icon_button(ui, &t, Icon::Plus, "New tab").clicked() {
                        self.run_action(ActionKind::NewTab);
                    }
                    self.dropdown_menu(ui);
                    ui.add_space(4.);
                    self.pane_chips(ui, compact);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, &t, Icon::Spark, "Ask AI  ⌘J").clicked() {
                            self.run_action(ActionKind::Ai);
                        }
                        if icon_button(ui, &t, Icon::List, "Commands").clicked() {
                            self.palette = true;
                        }
                        if primary_button(ui, &t, if wide { "New pane" } else { "+" }).clicked() {
                            self.run_action(ActionKind::NewPane);
                        }
                        if wide {
                            layout_switch(ui, &t, &mut self.layout);
                        }
                        if self.broadcast {
                            let pill = egui::Button::new(
                                RichText::new("BROADCAST")
                                    .size(10.)
                                    .strong()
                                    .color(Color32::BLACK),
                            )
                            .fill(color(t.warning))
                            .corner_radius(egui::CornerRadius::same(8));
                            if ui
                                .add(pill)
                                .on_hover_text("Typing goes to every visible pane. Click to stop.")
                                .clicked()
                            {
                                self.run_action(ActionKind::Broadcast);
                            }
                        }
                        let tip = match (self.online, self.latency) {
                            (true, Some(ms)) => format!("Connected · {ms} ms"),
                            (true, None) => "Connected".to_string(),
                            (false, _) => "Reconnecting…".to_string(),
                        };
                        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(14.), Sense::hover());
                        ui.painter().circle_filled(
                            rect.center(),
                            3.5,
                            color(if self.online { t.success } else { t.warning }),
                        );
                        resp.on_hover_text(tip);
                        if wide && self.online {
                            if let Some(ms) = self.latency {
                                ui.label(
                                    RichText::new(format!("{ms} ms"))
                                        .size(11.)
                                        .color(color(t.muted)),
                                );
                            }
                        }
                        if self.readonly {
                            ui.label(RichText::new("Read only").size(11.).color(color(t.warning)));
                        }
                    });
                });
            });
    }
    /// Numbered chips for the panes of the active workspace (one cycling chip when cramped).
    fn pane_chips(&mut self, ui: &mut egui::Ui, compact: bool) {
        let t = self.theme.clone();
        let ids: Vec<u64> = self.panes.keys().copied().collect();
        if ids.is_empty() {
            return;
        }
        let chips: Vec<u64> = if compact {
            vec![self.focus]
        } else {
            ids.clone()
        };
        for id in chips {
            let Some(pane) = self.panes.get(&id) else {
                continue;
            };
            let focused = id == self.focus;
            let label = if compact {
                let at = ids.iter().position(|i| *i == id).map_or(1, |p| p + 1);
                format!("{at}/{}", ids.len())
            } else {
                id.to_string()
            };
            let (rect, resp) = ui.allocate_exact_size(
                Vec2::new(if compact { 40. } else { 24. }, 20.),
                Sense::click(),
            );
            ui.painter().rect_filled(
                rect,
                6.,
                if focused {
                    color(t.accent).gamma_multiply(0.35)
                } else if resp.hovered() {
                    color(mix(t.panel, t.foreground, 0.1))
                } else {
                    Color32::TRANSPARENT
                },
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                FontId::proportional(12.),
                color(if focused { t.foreground } else { t.muted }),
            );
            if pane.info.exited.is_some() {
                ui.painter().circle_filled(
                    rect.right_top() + Vec2::new(-4., 4.),
                    2.2,
                    color(t.warning),
                );
            }
            let tip = format!("{}  {}\n{}", id, pane.info.title, pane.info.cwd);
            let clicked = resp.on_hover_text(tip).clicked();
            if clicked {
                if compact {
                    let at = ids.iter().position(|i| *i == id).unwrap_or(0);
                    self.focus = ids[(at + 1) % ids.len()];
                } else {
                    self.focus = id;
                }
                let target = self.focus;
                ui.ctx()
                    .memory_mut(|m| m.request_focus(egui::Id::new(("teryon-terminal", target))));
            }
        }
    }
}

//! The single slim top bar (brand, tabs, pane chips, layout, status) and the toast overlay.
use super::*;

/// Height of the bar; the macOS traffic lights sit inside it.
pub(super) const BAR_HEIGHT: f32 = 34.;
pub(super) const TAB_STRIP_HEIGHT: f32 = 28.;
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
    /// The bottom tab strip of a bare window, styled like WezTerm's: rounded tabs (the active one
    /// filled and underlined), a new-tab button and a clock.
    pub(super) fn tab_strip(&mut self, ctx: &egui::Context) {
        if self.cfg.tab_bar != crate::config::TabBar::Bottom {
            return;
        }
        let t = self.theme.clone();
        let pick_color = |custom: &Option<String>, fallback: Color32| {
            custom
                .as_deref()
                .and_then(super::triggers::parse_hex)
                .unwrap_or(fallback)
        };
        let strip = pick_color(
            &self.cfg.tab_bar_bg,
            color(mix(t.background, ratatui::style::Color::Rgb(0, 0, 0), 0.35)),
        );
        let active_bg = pick_color(&self.cfg.tab_active_bg, color(t.accent));
        let active_fg = pick_color(&self.cfg.tab_active_fg, strip);
        let idle_bg = pick_color(
            &self.cfg.tab_inactive_bg,
            color(mix(t.background, t.foreground, 0.12)),
        );
        let idle_fg = pick_color(
            &self.cfg.tab_inactive_fg,
            color(mix(t.accent, t.foreground, 0.5)),
        );
        let hover_bg = color(t.border);
        let hover_fg = color(t.foreground);
        let mut pick = None;
        let mut new_tab = false;
        egui::TopBottomPanel::bottom("teryon-tab-strip")
            .exact_height(TAB_STRIP_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(strip.gamma_multiply(self.cfg.window_opacity))
                    .inner_margin(egui::Margin::symmetric(6, 2)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.;
                    // One rounded chip: `fill` and `ink` at rest, the hover pair under the pointer.
                    let chip = |ui: &mut egui::Ui,
                                text: String,
                                on: bool,
                                fill: Color32,
                                ink: Color32,
                                hover: (Color32, Color32)| {
                        let galley = ui
                            .painter()
                            .layout_no_wrap(text, FontId::monospace(13.), ink);
                        let size = Vec2::new(galley.size().x + 20., TAB_STRIP_HEIGHT - 6.);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        let hovered = response.hovered() && !on;
                        let (fill, ink) = if hovered { hover } else { (fill, ink) };
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same((size.y / 2.) as u8),
                            fill,
                        );
                        let origin = rect.center() - galley.size() / 2.;
                        ui.painter().galley(origin, galley.clone(), ink);
                        if on {
                            // Bold look and the underline WezTerm draws under the active tab.
                            ui.painter()
                                .galley(origin + Vec2::new(0.6, 0.), galley.clone(), ink);
                            let y = origin.y + galley.size().y - 1.;
                            ui.painter().line_segment(
                                [
                                    Pos2::new(origin.x, y),
                                    Pos2::new(origin.x + galley.size().x, y),
                                ],
                                Stroke::new(1_f32, ink),
                            );
                        }
                        response
                    };
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let on = i == self.active;
                        let name = match self.panes.get(&self.focus) {
                            Some(p) if on && !p.info.title.is_empty() => p.info.title.as_str(),
                            _ if tab.name.starts_with("win-") => "shell",
                            _ => tab.name.as_str(),
                        };
                        let name: String = name.chars().take(22).collect();
                        let text = format!(
                            "\u{f120} {} {name}",
                            if on { "\u{25cf}" } else { "\u{25cb}" }
                        );
                        let (fill, ink) = if on {
                            (active_bg, active_fg)
                        } else {
                            (idle_bg, idle_fg)
                        };
                        if chip(ui, text, on, fill, ink, (hover_bg, hover_fg)).clicked() {
                            pick = Some(i);
                        }
                    }
                    if chip(ui, "+".into(), false, strip, active_bg, (active_bg, strip))
                        .on_hover_text("New tab")
                        .clicked()
                    {
                        new_tab = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("\u{f017} {}", clock_now()))
                                .monospace()
                                .size(13.)
                                .color(active_bg),
                        );
                    });
                });
            });
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        if let Some(i) = pick {
            self.activate(i);
        }
        if new_tab {
            self.run_action(ActionKind::NewTab);
        }
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
                let edge = ui.max_rect().expand2(Vec2::new(6., 3.));
                ui.painter().line_segment(
                    [
                        edge.left_bottom() - Vec2::new(0., 0.5),
                        edge.right_bottom() - Vec2::new(0., 0.5),
                    ],
                    Stroke::new(1_f32, color(t.border).gamma_multiply(0.8)),
                );
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
                        // Quiet tools share one tray, so only "New pane" is filled.
                        egui::Frame::new()
                            .fill(color(mix(t.panel, t.foreground, 0.05)))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(1.)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.;
                                    if icon_button(ui, &t, Icon::List, "Commands  ⌘K").clicked() {
                                        self.palette = true;
                                    }
                                    if icon_button(ui, &t, Icon::Spark, "Ask AI  ⌘J").clicked() {
                                        self.run_action(ActionKind::Ai);
                                    }
                                });
                            });
                        self.agent_pill(ui);
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
                            if let Some(ms) = self.latency.filter(|ms| *ms > 150) {
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

/// Local time as `HH:MM:SS`.
fn clock_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as libc::time_t;
    // SAFETY: `tm` is plain data that localtime_r fully fills, and both pointers are valid.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm
    };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

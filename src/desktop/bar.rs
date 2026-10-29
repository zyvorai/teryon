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
        let now = ctx.input(|i| i.time);
        let mut spinning = false;
        let mut pick = None;
        let mut close = None;
        let mut new_tab = false;
        egui::TopBottomPanel::bottom("teryon-tab-strip")
            .show_separator_line(false)
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
                    // A closable chip ends in a dim ×; the bool returned says it was the × clicked.
                    let chip = |ui: &mut egui::Ui,
                                text: String,
                                on: bool,
                                closable: bool,
                                fill: Color32,
                                ink: Color32,
                                hover: (Color32, Color32)|
                     -> (egui::Response, bool) {
                        let galley = ui
                            .painter()
                            .layout_no_wrap(text, FontId::monospace(13.), ink);
                        let close_w = if closable { 22. } else { 0. };
                        let size =
                            Vec2::new(galley.size().x + 20. + close_w, TAB_STRIP_HEIGHT - 4.);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        let x_zone = Rect::from_min_max(
                            Pos2::new(rect.right() - close_w - 4., rect.top()),
                            rect.right_bottom(),
                        );
                        let over_x = closable
                            && ui
                                .input(|i| i.pointer.hover_pos())
                                .is_some_and(|p| x_zone.contains(p));
                        let hovered = response.hovered() && !on;
                        let (fill, ink) = if hovered { hover } else { (fill, ink) };
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same((rect.height() / 2.) as u8),
                            fill,
                        );
                        if on {
                            // Zellij-style: a short accent bar under the active tab's label.
                            let bar = Rect::from_center_size(
                                Pos2::new(rect.center().x, rect.bottom() - 3.),
                                Vec2::new((rect.width() - 28.).max(8.), 2.),
                            );
                            ui.painter().rect_filled(
                                bar,
                                egui::CornerRadius::same(1),
                                color(t.accent),
                            );
                        }
                        let origin =
                            Pos2::new(rect.left() + 10., rect.center().y - galley.size().y / 2.);
                        ui.painter().galley(origin, galley.clone(), ink);
                        if on {
                            // A second strike a hair to the right reads as bold.
                            ui.painter()
                                .galley(origin + Vec2::new(0.6, 0.), galley, ink);
                        }
                        if closable {
                            let mark = if over_x {
                                ink
                            } else {
                                ink.gamma_multiply(0.45)
                            };
                            ui.painter().text(
                                Pos2::new(rect.right() - 14., rect.center().y),
                                egui::Align2::CENTER_CENTER,
                                "\u{d7}",
                                FontId::monospace(14.),
                                mark,
                            );
                        }
                        let clicked_x = response.clicked() && over_x;
                        (response, clicked_x)
                    };
                    let many = self.tabs.len() > 1;
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let on = i == self.active;
                        let text = match (on, &tab.state) {
                            (true, _) | (_, None) => {
                                tab_label(&self.panes, self.focus, &tab.name, now)
                            }
                            (false, Some(sess)) => {
                                tab_label(&sess.panes, sess.focus, &tab.name, now)
                            }
                        };
                        let text = format!("{}{text}", tab.activity.prefix());
                        spinning |= text.starts_with(|c| ('\u{2800}'..='\u{28ff}').contains(&c));
                        // Tabs cross-fade between idle and active colors when focus moves.
                        let k = ctx.animate_bool_with_time(
                            egui::Id::new(("teryon-tab-fade", i)),
                            on,
                            0.15,
                        );
                        let (fill, ink) = (
                            idle_bg.lerp_to_gamma(active_bg, k),
                            idle_fg.lerp_to_gamma(active_fg, k),
                        );
                        let (response, clicked_x) =
                            chip(ui, text, on, many, fill, ink, (hover_bg, hover_fg));
                        if clicked_x {
                            close = Some(i);
                        } else if response.clicked() {
                            pick = Some(i);
                        }
                    }
                    if chip(
                        ui,
                        "+".into(),
                        false,
                        false,
                        Color32::TRANSPARENT,
                        color(t.accent),
                        (idle_bg, color(t.accent)),
                    )
                    .0
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
        ctx.request_repaint_after(std::time::Duration::from_millis(if spinning {
            80
        } else {
            1000
        }));
        if let Some(i) = close {
            self.activate(i);
            self.close_tab();
        } else if let Some(i) = pick {
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
                                    format!("{}{}/{}", tab.activity.prefix(), tab.host, tab.name)
                                } else {
                                    format!("{}{}", tab.activity.prefix(), tab.name)
                                };
                                (i, label)
                            })
                            .collect();
                        let mut pick = self.active;
                        segmented(ui, &t, &mut pick, &items);
                        if pick != self.active {
                            self.activate(pick);
                        }
                        if icon_button(
                            ui,
                            &t,
                            Icon::Close,
                            if self.tab_owns_shells(self.active) {
                                "Close this tab (ends its shells)"
                            } else {
                                "Detach this tab"
                            },
                        )
                        .clicked()
                        {
                            self.close_tab();
                        }
                    } else if !compact {
                        egui::Frame::new()
                            .fill(color(t.accent).gamma_multiply(0.08))
                            .stroke(Stroke::new(1_f32, color(t.accent).gamma_multiply(0.28)))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(9, 2))
                            .show(ui, |ui| {
                                let dot = if self.online { t.success } else { t.warning };
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(10.), Sense::hover());
                                ui.painter().circle_filled(
                                    rect.center(),
                                    6.,
                                    color(dot).gamma_multiply(0.18),
                                );
                                ui.painter().circle_filled(rect.center(), 3.5, color(dot));
                                ui.label(
                                    RichText::new(self.host.as_str())
                                        .monospace()
                                        .size(12.)
                                        .color(color(t.foreground)),
                                );
                                ui.label(RichText::new("/").size(12.).color(color(t.muted)));
                                ui.label(
                                    RichText::new(&self.workspace)
                                        .monospace()
                                        .size(12.)
                                        .color(color(t.accent)),
                                );
                            });
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
                                    if wide {
                                        ui.label(
                                            RichText::new("⌘K")
                                                .monospace()
                                                .size(10.5)
                                                .color(color(t.muted)),
                                        );
                                        ui.add_space(4.);
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
                        if width >= 1100. {
                            self.link_stats(ui);
                        } else if wide && self.online {
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
    /// `LINK 12 ms` and `SHELLS ALIVE 3d 04h` as small two-line labels.
    fn link_stats(&self, ui: &mut egui::Ui) {
        let t = &self.theme;
        let stat = |ui: &mut egui::Ui, name: &str, value: String, warn: bool| {
            ui.label(
                RichText::new(value)
                    .monospace()
                    .size(11.5)
                    .color(color(if warn { t.warning } else { t.foreground })),
            );
            ui.label(RichText::new(name).size(9.).color(color(t.muted)));
        };
        if let Some(up) = self.link.uptime() {
            stat(ui, "SHELLS ALIVE", up, false);
        }
        if let Some(ms) = self.latency.filter(|_| self.online) {
            stat(ui, "LINK", format!("{ms} ms"), link_is_slow(ms));
        }
        if let Some(s) = self.link.stats().filter(|_| self.online) {
            let (load, mem, heat) = health_labels(s);
            stat(ui, "LOAD", load, false);
            if let Some(m) = mem {
                stat(ui, "MEM", m, s.mem_used_pct.is_some_and(memory_is_high));
            }
            if let Some(h) = heat {
                ui.label(RichText::new(h).size(11.5).color(color(t.warning)));
                ui.label(RichText::new("HEAT").size(9.).color(color(t.muted)));
            }
        }
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
                format!("{id:02}")
            };
            let (rect, resp) = ui.allocate_exact_size(
                Vec2::new(if compact { 40. } else { 30. }, 22.),
                Sense::click(),
            );
            if focused {
                ui.painter().rect_stroke(
                    rect,
                    6.,
                    Stroke::new(1_f32, color(t.accent).gamma_multiply(0.55)),
                    egui::StrokeKind::Inside,
                );
            }
            ui.painter().rect_filled(
                rect,
                6.,
                if focused {
                    color(t.accent).gamma_multiply(0.22)
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

/// An icon for a foreground program, in Nerd Font glyphs.
fn program_icon(program: &str) -> &'static str {
    match program {
        "nvim" | "vim" | "vi" => "\u{e62b}",
        "ssh" | "mosh" => "\u{f0c2}",
        "git" | "lazygit" | "tig" => "\u{e702}",
        "cargo" | "rustc" | "rustup" => "\u{e7a8}",
        "python" | "python3" | "ipython" => "\u{e73c}",
        "node" | "npm" | "pnpm" | "yarn" | "bun" => "\u{e718}",
        "docker" | "podman" => "\u{f308}",
        "claude" | "codex" | "gemini" | "aider" => "\u{f0d0}",
        "htop" | "btop" | "top" => "\u{f080}",
        _ => "\u{f120}",
    }
}
/// A tab's label: an icon for the foreground program (a spinner while a command runs) and a title
/// (the program, or what the pane's title says).
fn tab_label(panes: &BTreeMap<u64, Pane>, focus: u64, fallback: &str, now: f64) -> String {
    let Some(p) = panes.get(&focus) else {
        let name = if fallback.starts_with("win-") {
            "Shell"
        } else {
            fallback
        };
        return format!("\u{f120}  {name}");
    };
    let program = p
        .info
        .fg
        .as_deref()
        .and_then(|c| c.split_whitespace().next())
        .and_then(|c| c.rsplit('/').next())
        .unwrap_or("");
    let is_shell = matches!(
        program,
        "" | "zsh" | "bash" | "fish" | "sh" | "-zsh" | "-bash"
    );
    let title = p.info.title.trim();
    let name = if !is_shell {
        program
    } else if !title.is_empty() {
        title
    } else {
        "Shell"
    };
    let running = p.marks.last().is_some_and(|m| m.kind == 'C');
    let icon = if running && is_shell {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[((now * 12.) as usize) % FRAMES.len()]
    } else {
        program_icon(program)
    };
    let name: String = name.chars().take(22).collect();
    format!("{icon}  {name}")
}
/// Round trips slower than this get the warning colour (the narrow bar already only shows them then).
const SLOW_LINK_MS: u64 = 150;
/// Memory in use at or above this percentage gets the warning colour.
const HIGH_MEMORY_PCT: u8 = 90;

/// Steady numbers stay quiet; the ones that mean trouble are called out, as heat already is.
fn link_is_slow(ms: u64) -> bool {
    ms > SLOW_LINK_MS
}
fn memory_is_high(pct: u8) -> bool {
    pct >= HIGH_MEMORY_PCT
}

/// Text for the host health labels: load, memory, and a heat warning only when throttled.
fn health_labels(s: &crate::hoststats::HostStats) -> (String, Option<String>, Option<String>) {
    (
        format!("{:.2}", s.load1),
        s.mem_used_pct.map(|p| format!("{p}%")),
        s.thermal.clone().filter(|t| t.as_str() != "nominal"),
    )
}

#[cfg(test)]
mod health_tests {
    use super::*;
    use crate::hoststats::HostStats;
    #[test]
    fn only_trouble_is_called_out() {
        assert!(!link_is_slow(8) && !link_is_slow(150));
        assert!(link_is_slow(151) && link_is_slow(900));
        assert!(!memory_is_high(65) && !memory_is_high(89));
        assert!(memory_is_high(90) && memory_is_high(98));
    }
    #[test]
    fn heat_shows_only_when_throttled() {
        let mut s = HostStats {
            load1: 1.5,
            mem_used_pct: Some(46),
            thermal: Some("nominal".into()),
        };
        assert_eq!(health_labels(&s), ("1.50".into(), Some("46%".into()), None));
        s.thermal = Some("throttled 80%".into());
        s.mem_used_pct = None;
        assert_eq!(
            health_labels(&s),
            ("1.50".into(), None, Some("throttled 80%".into()))
        );
    }
}

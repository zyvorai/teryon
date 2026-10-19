//! AI coding agents in panes: status chips, notifications when one needs you, and the ⌘⇧A overview.
use super::*;
use crate::agents::{self, Kind, Status};

/// What the app knows about the agent (if any) running in one pane.
#[derive(Clone)]
pub(super) struct Track {
    kind: Option<Kind>,
    /// An agent has run here, so a shell prompt coming back means it finished.
    seen: bool,
    rev: u64,
    changed: Instant,
    computed: Instant,
    pub(super) status: Option<Status>,
    since: Instant,
}
impl Default for Track {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            kind: None,
            seen: false,
            rev: 0,
            changed: now,
            computed: now - Duration::from_secs(5),
            status: None,
            since: now,
        }
    }
}

fn status_color(t: &Theme, s: Status) -> Color32 {
    match s {
        Status::Working => color(t.accent),
        Status::NeedsYou => color(t.warning),
        Status::Idle => color(t.muted),
        Status::Done => color(t.success),
    }
}

impl Desktop {
    /// Refresh every pane's agent state; notify when a background agent starts waiting for you or finishes.
    pub(super) fn agents_update(&mut self) {
        let now = Instant::now();
        let notify = self.cfg.agent_notifications && !self.window_focused;
        let mut messages: Vec<(&'static str, u64, String)> = Vec::new();
        for (id, pane) in self.panes.iter_mut() {
            let fg = pane
                .info
                .fg
                .clone()
                .unwrap_or_else(|| pane.info.title.clone());
            let found = agents::detect(&fg);
            let tr = &mut pane.track;
            if pane.info.revision != tr.rev {
                tr.rev = pane.info.revision;
                tr.changed = now;
            }
            if found.is_none() && !tr.seen {
                tr.status = None;
                continue;
            }
            if let Some(k) = found {
                tr.kind = Some(k);
                tr.seen = true;
            }
            // Re-read the screen only when it changed or the quiet period may have crossed a threshold.
            if now.duration_since(tr.computed) < Duration::from_millis(400) && tr.status.is_some() {
                continue;
            }
            tr.computed = now;
            let next = if pane.info.exited.is_some() || (found.is_none() && agents::is_shell(&fg)) {
                Status::Done
            } else if found.is_none() {
                // Something else is in front (a tool the agent started): the agent is still working.
                Status::Working
            } else {
                let screen = pane.parser.screen();
                let (_, cols) = screen.size();
                let rows: Vec<String> = screen.rows(0, cols).collect();
                agents::status(&rows, now.duration_since(tr.changed))
            };
            let prev = tr.status;
            if prev != Some(next) {
                tr.status = Some(next);
                tr.since = now;
                let name = tr.kind.map_or("Agent", Kind::name);
                if prev.is_some() && notify {
                    match next {
                        Status::NeedsYou => messages.push((
                            "agent_needs_you",
                            *id,
                            format!("{name} in pane {id} needs you"),
                        )),
                        Status::Done => messages.push((
                            "agent_done",
                            *id,
                            format!("{name} finished in pane {id}"),
                        )),
                        _ => {}
                    }
                }
            }
        }
        for (event, pane, message) in messages {
            crate::util::notify(&message);
            crate::notify::send_remote(
                &self.notify_cfg,
                crate::notify::Note {
                    event,
                    title: "TERYON".into(),
                    message,
                    workspace: self.workspace.clone(),
                    pane,
                },
            );
        }
    }

    /// Agent and status for a pane's bar chip.
    pub(super) fn agent_chip(&self, id: u64) -> Option<(Kind, Status)> {
        let tr = &self.panes.get(&id)?.track;
        Some((tr.kind?, tr.status?))
    }

    pub(super) fn draw_agent_chip(&self, ui: &mut egui::Ui, id: u64) {
        let Some((kind, status)) = self.agent_chip(id) else {
            return;
        };
        let t = &self.theme;
        let c = status_color(t, status);
        let text = format!("{} · {}", kind.name(), status.label());
        egui::Frame::new()
            .fill(c.gamma_multiply(0.18))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(6, 1))
            .show(ui, |ui| {
                ui.label(RichText::new(text).size(11.).color(c));
            });
    }

    /// A pill in the top bar for the most urgent agent, so the status is visible even when pane bars are hidden.
    pub(super) fn agent_pill(&mut self, ui: &mut egui::Ui) {
        let list = self.agents_list();
        let Some((_, kind, status, ..)) = list.first() else {
            return;
        };
        let t = self.theme.clone();
        let c = status_color(&t, *status);
        let more = if list.len() > 1 {
            format!(" +{}", list.len() - 1)
        } else {
            String::new()
        };
        let needs = *status == Status::NeedsYou;
        let text = if needs {
            format!("{} needs you{more}", kind.name())
        } else {
            format!("{} · {}{more}", kind.name(), status.label())
        };
        let button = egui::Button::new(RichText::new(text).size(11.5).color(c))
            .fill(c.gamma_multiply(if needs { 0.1 } else { 0.16 }))
            .stroke(if needs {
                Stroke::new(1_f32, c.gamma_multiply(0.45))
            } else {
                Stroke::NONE
            })
            .corner_radius(egui::CornerRadius::same(9));
        if ui
            .add(button)
            .on_hover_text("Agents overview  ⌘⇧A")
            .clicked()
        {
            self.agents_open = true;
        }
    }

    /// Answer the approval prompt of the agent in pane `id`; only while it is still asking.
    pub(super) fn answer_agent(&mut self, id: u64, kind: Kind, yes: bool) {
        if self.readonly || !self.cfg.agent_quick_approve {
            return;
        }
        let asking = self
            .panes
            .get(&id)
            .is_some_and(|p| p.track.status == Some(Status::NeedsYou));
        if !asking {
            self.toast("That agent is no longer waiting");
            return;
        }
        let data = if yes {
            kind.approve_keys()
        } else {
            kind.deny_keys()
        }
        .to_vec();
        self.send(Command::Input { pane: id, data });
        self.toast(format!(
            "{} in pane {id}: {}",
            kind.name(),
            if yes { "approved" } else { "denied" }
        ));
    }

    /// Every agent in the active workspace, needing-you first.
    pub(super) fn agents_list(&self) -> Vec<(u64, Kind, Status, String, String)> {
        let mut out: Vec<_> = self
            .panes
            .iter()
            .filter_map(|(id, p)| {
                Some((
                    *id,
                    p.track.kind?,
                    p.track.status?,
                    p.info.title.clone(),
                    p.info.cwd.clone(),
                ))
            })
            .collect();
        out.sort_by_key(|(id, _, s, ..)| (*s != Status::NeedsYou, *id));
        out
    }

    pub(super) fn agents_overview(&mut self, ctx: &egui::Context) {
        // Quiet periods and shell returns are noticed by polling, so keep frames coming while an agent is around.
        if self.panes.values().any(|p| p.track.status.is_some()) {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if !self.agents_open {
            return;
        }
        let list = self.agents_list();
        let t = self.theme.clone();
        let mut jump = None;
        let mut answer: Option<(u64, Kind, bool)> = None;
        let quick = self.cfg.agent_quick_approve && !self.readonly;
        let mut open = true;
        egui::Window::new("Agents")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0., 60.])
            .show(ctx, |ui| {
                ui.set_min_width(380.);
                if list.is_empty() {
                    ui.label(
                        RichText::new("No AI agents running here. Start Claude Code, Codex, Gemini CLI or aider in a pane and it shows up with its status.")
                            .size(12.)
                            .color(color(t.muted)),
                    );
                }
                for (id, kind, status, title, cwd) in &list {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.), Sense::click());
                    let asking = quick && *status == Status::NeedsYou;
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 8., color(mix(t.panel, t.foreground, 0.08)));
                    }
                    let c = status_color(&t, *status);
                    ui.painter().circle_filled(rect.left_center() + Vec2::new(12., 0.), 4., c);
                    ui.painter().text(
                        rect.left_top() + Vec2::new(26., 6.),
                        egui::Align2::LEFT_TOP,
                        format!("{} · pane {id}", kind.name()),
                        FontId::proportional(13.),
                        color(t.foreground),
                    );
                    ui.painter().text(
                        rect.left_top() + Vec2::new(26., 22.),
                        egui::Align2::LEFT_TOP,
                        format!("{title}  {cwd}"),
                        FontId::proportional(11.),
                        color(t.muted),
                    );
                    let label_x = if asking { 150. } else { 10. };
                    ui.painter().text(rect.right_center() - Vec2::new(label_x, 0.), egui::Align2::RIGHT_CENTER, status.label(), FontId::proportional(12.), c);
                    if asking {
                        let deny = egui::Rect::from_center_size(rect.right_center() - Vec2::new(34., 0.), Vec2::new(60., 24.));
                        let approve = egui::Rect::from_center_size(rect.right_center() - Vec2::new(100., 0.), Vec2::new(60., 24.));
                        let a = ui.put(approve, egui::Button::new("Approve")).on_hover_text("Answers the prompt: yes");
                        let d = ui.put(deny, egui::Button::new("Deny")).on_hover_text("Answers the prompt: no");
                        if a.clicked() {
                            answer = Some((*id, *kind, true));
                        } else if d.clicked() {
                            answer = Some((*id, *kind, false));
                        } else if resp.clicked() {
                            jump = Some(*id);
                        }
                    } else if resp.clicked() {
                        jump = Some(*id);
                    }
                }
            });
        if let Some((id, kind, yes)) = answer {
            self.answer_agent(id, kind, yes);
        }
        if let Some(id) = jump {
            self.focus = id;
            self.agents_open = false;
        }
        if !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.agents_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;

    fn agent_pane(app: &mut Desktop, id: u64, fg: Option<&str>, text: &str) {
        let (rows, cols) = (10, 60);
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(text.as_bytes());
        let view = PaneView {
            id,
            title: "Shell".into(),
            cwd: "/p".into(),
            rows,
            cols,
            revision: 1,
            fg: fg.map(str::to_string),
            ..Default::default()
        };
        app.panes.insert(id, Pane::new(view, parser));
    }

    #[test]
    fn a_waiting_agent_is_flagged_and_listed_first() {
        let (mut app, _rx) = app_with_screen("");
        app.window_focused = true;
        agent_pane(
            &mut app,
            2,
            Some("node /x/@anthropic-ai/claude-code/cli.js"),
            "Edit file\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n",
        );
        agent_pane(&mut app, 3, Some("codex"), "compiling 3/40\r\n");
        agent_pane(&mut app, 4, Some("-zsh"), "$ ");
        app.agents_update();
        assert_eq!(app.agent_chip(2), Some((Kind::Claude, Status::NeedsYou)));
        assert_eq!(app.agent_chip(3), Some((Kind::Codex, Status::Working)));
        assert_eq!(app.agent_chip(4), None);
        let list = app.agents_list();
        assert_eq!(list[0].0, 2);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn a_background_app_also_pushes_to_the_webhook() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let (mut app, _rx) = app_with_screen("");
        app.notify_cfg.webhook_url = Some(format!("http://{}/h", listener.local_addr().unwrap()));
        app.window_focused = false;
        agent_pane(&mut app, 2, Some("claude"), "esc to interrupt\r\n");
        app.agents_update();
        let mut parser = vt100::Parser::new(10, 60, 0);
        parser.process("Do you want to proceed?\r\n❯ 1. Yes\r\n".as_bytes());
        let pane = app.panes.get_mut(&2).unwrap();
        pane.parser = parser;
        pane.track.computed -= Duration::from_secs(5);
        app.agents_update();
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = [0u8; 4096];
        let mut got = String::new();
        while !got.contains("needs you") {
            let n = s.read(&mut buf).unwrap();
            assert!(n > 0, "closed early: {got}");
            got.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        assert!(got.contains("agent_needs_you"), "{got}");
    }

    #[test]
    fn approve_and_deny_type_into_a_waiting_agent_only_when_enabled() {
        let (mut app, rx) = app_with_screen("");
        app.window_focused = true;
        agent_pane(
            &mut app,
            2,
            Some("claude"),
            "Do you want to proceed?\r\n❯ 1. Yes\r\n",
        );
        app.agents_update();
        while rx.try_recv().is_ok() {}
        app.answer_agent(2, Kind::Claude, true);
        assert!(rx.try_recv().is_err(), "off by default");
        app.cfg.agent_quick_approve = true;
        app.answer_agent(2, Kind::Claude, true);
        assert!(matches!(rx.try_recv(), Ok(Command::Input { pane: 2, data }) if data == b"\r"));
        app.answer_agent(2, Kind::Claude, false);
        assert!(matches!(rx.try_recv(), Ok(Command::Input { pane: 2, data }) if data == b"\x1b"));
        app.readonly = true;
        app.answer_agent(2, Kind::Claude, true);
        assert!(rx.try_recv().is_err(), "nothing is sent when read only");
        app.readonly = false;
        app.panes.get_mut(&2).unwrap().track.status = Some(Status::Working);
        app.answer_agent(2, Kind::Claude, true);
        assert!(
            rx.try_recv().is_err(),
            "a stale click does not answer a later prompt"
        );
    }

    #[test]
    fn an_agent_that_exits_back_to_the_shell_is_done() {
        let (mut app, _rx) = app_with_screen("");
        app.window_focused = true;
        agent_pane(&mut app, 2, Some("claude"), "working\r\n");
        app.agents_update();
        assert!(app.agent_chip(2).is_some());
        app.panes.get_mut(&2).unwrap().info.fg = Some("-zsh".into());
        app.panes.get_mut(&2).unwrap().track.computed -= Duration::from_secs(5);
        app.agents_update();
        assert_eq!(app.agent_chip(2), Some((Kind::Claude, Status::Done)));
    }

    #[test]
    fn the_overview_and_chip_draw() {
        let (mut app, _rx) = app_with_screen("");
        agent_pane(&mut app, 2, Some("claude"), "Continue? (y/n)\r\n");
        app.agents_update();
        app.agents_open = true;
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.agents_overview(ctx);
            egui::CentralPanel::default().show(ctx, |ui| app.draw_agent_chip(ui, 2));
        });
    }
}

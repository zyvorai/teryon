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
        let mut messages: Vec<String> = Vec::new();
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
                        Status::NeedsYou => messages.push(format!("{name} in pane {id} needs you")),
                        Status::Done => messages.push(format!("{name} finished in pane {id}")),
                        _ => {}
                    }
                }
            }
        }
        for m in messages {
            crate::util::notify(&m);
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
        let text = format!(
            "{} {} · {}",
            if status == Status::Working {
                "●"
            } else {
                "◉"
            },
            kind.name(),
            status.label()
        );
        egui::Frame::new()
            .fill(c.gamma_multiply(0.18))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(6, 1))
            .show(ui, |ui| {
                ui.label(RichText::new(text).size(11.).color(c));
            });
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
                    ui.painter().text(rect.right_center() - Vec2::new(10., 0.), egui::Align2::RIGHT_CENTER, status.label(), FontId::proportional(12.), c);
                    if resp.clicked() {
                        jump = Some(*id);
                    }
                }
            });
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

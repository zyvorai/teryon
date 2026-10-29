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
    /// Where the agent got to, from its screen the last time it was read.
    pub(super) summary: Option<String>,
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
            summary: None,
        }
    }
}

/// The "prompt all idle agents" dialog.
pub(super) struct AgentPrompt {
    text: String,
    focus: bool,
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
        let mut notes: Vec<crate::notify::Note> = Vec::new();
        // With the window in front, an agent in a pane you are not looking at gets a card instead.
        let carding = self.cfg.agent_notifications && self.window_focused;
        let mut cards: Vec<(u64, Kind, Option<String>)> = Vec::new();
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
            // What the agent is asking, read from the same rows the state comes from.
            let mut asked = None;
            let next = if pane.info.exited.is_some() || (found.is_none() && agents::is_shell(&fg)) {
                Status::Done
            } else if found.is_none() {
                // Something else is in front (a tool the agent started): the agent is still working.
                Status::Working
            } else {
                let screen = pane.parser.screen();
                let (_, cols) = screen.size();
                let rows: Vec<String> = screen.rows(0, cols).collect();
                if let Some(line) = agents::summary(&rows) {
                    tr.summary = Some(line);
                }
                asked = agents::question(&rows);
                agents::status(&rows, now.duration_since(tr.changed))
            };
            let prev = tr.status;
            if prev != Some(next) {
                tr.status = Some(next);
                tr.since = now;
                let name = tr.kind.map_or("Agent", Kind::name);
                if prev.is_some() && carding && next == Status::NeedsYou && *id != self.focus {
                    if let Some(kind) = tr.kind {
                        cards.push((*id, kind, asked.clone()));
                    }
                }
                if prev.is_some() && notify {
                    match next {
                        Status::NeedsYou => notes.push(crate::notify::Note::agent_needs_you(
                            name,
                            &self.workspace,
                            *id,
                            asked,
                        )),
                        Status::Done => notes.push(crate::notify::Note::agent_done(
                            name,
                            &self.workspace,
                            *id,
                            tr.summary.clone(),
                        )),
                        _ => {}
                    }
                }
            }
        }
        for (id, kind, question) in cards {
            self.raise_card(id, kind, question);
        }
        self.prune_cards();
        for note in notes {
            note.show_banner();
            crate::notify::send_remote(&self.notify_cfg, note);
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

    /// Start `kind` in a new pane next to the focused one (same directory).
    pub(super) fn spawn_agent(&mut self, kind: Kind) {
        let run = self.agent_run(kind);
        self.spawn_command(run);
    }
    /// What is typed to start `kind`: its command, inside the sandbox when that is configured.
    fn agent_run(&self, kind: Kind) -> String {
        if self.cfg.agent_sandbox {
            agents::sandboxed(kind.launch())
        } else {
            kind.launch().to_string()
        }
    }
    /// Start `kind` on its own branch in its own Git worktree next to the repository.
    pub(super) fn spawn_agent_worktree(&mut self, kind: Kind) {
        let run = self.agent_run(kind);
        self.spawn_command(agents::worktree_command_running(kind.launch(), &run));
    }
    /// Open a new pane in the directory an agent works in, listing what it changed there.
    pub(super) fn show_agent_changes(&mut self, id: u64) {
        let cwd = self.panes.get(&id).map(|p| p.info.cwd.clone());
        self.spawn_command_in(cwd, agents::changes_command());
    }
    fn spawn_command(&mut self, command: String) {
        let cwd = self.focus_cwd();
        self.spawn_command_in(cwd, command);
    }
    fn spawn_command_in(&mut self, cwd: Option<String>, command: String) {
        if self.readonly || !self.online {
            self.toast("Can't open a pane: no writable connection");
            return;
        }
        self.spawn_cmd = Some(command);
        self.send(Command::New(cwd));
    }
    /// A pane that was just created for `spawn_agent` gets its command typed in.
    pub(super) fn run_spawned(&mut self, pane: u64) {
        if let Some(cmd) = self.spawn_cmd.take() {
            let mut data = cmd.into_bytes();
            data.push(b'\r');
            self.send(Command::Input { pane, data });
        }
    }
    /// Agents sitting at their own prompt: the only ones a broadcast prompt is typed into
    /// (typing into a working agent queues text, into an approval prompt answers it).
    pub(super) fn prompt_targets(&self) -> Vec<u64> {
        self.panes
            .iter()
            .filter(|(_, p)| p.track.kind.is_some() && p.track.status == Some(Status::Idle))
            .map(|(id, _)| *id)
            .collect()
    }
    pub(super) fn open_agent_prompt(&mut self) {
        if self.readonly {
            self.toast("Read only");
        } else {
            self.agent_prompt = Some(AgentPrompt {
                text: String::new(),
                focus: true,
            });
        }
    }
    /// Type `text` and Enter into every idle agent; returns how many got it.
    pub(super) fn send_agent_prompt(&mut self, text: &str) -> usize {
        let text = text.trim();
        let targets = self.prompt_targets();
        if text.is_empty() || targets.is_empty() {
            return 0;
        }
        for pane in &targets {
            let bracketed = self
                .panes
                .get(pane)
                .is_some_and(|p| p.parser.screen().bracketed_paste());
            let mut data = keys::paste(text, bracketed || text.contains('\n'));
            data.push(b'\r');
            self.send(Command::Input { pane: *pane, data });
        }
        targets.len()
    }
    pub(super) fn agent_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(mut prompt) = self.agent_prompt.take() else {
            return;
        };
        let n = self.prompt_targets().len();
        let (mut send, mut cancel) = (false, false);
        egui::Window::new("Prompt agents")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0., 90.])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "Typed and sent to {n} idle agent(s). Working agents and ones waiting for approval are skipped."
                    ))
                    .size(12.),
                );
                let edit = ui.add(
                    egui::TextEdit::multiline(&mut prompt.text)
                        .desired_width(420.)
                        .desired_rows(3),
                );
                if prompt.focus {
                    edit.request_focus();
                    prompt.focus = false;
                }
                if edit.has_focus()
                    && ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.command)
                {
                    send = true;
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(n > 0 && !prompt.text.trim().is_empty(), egui::Button::new("Send  ⌘↩"))
                        .clicked()
                    {
                        send = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                }
            });
        if send {
            let sent = self.send_agent_prompt(&prompt.text);
            self.toast(format!("Prompt sent to {sent} agent(s)"));
        } else if !cancel {
            self.agent_prompt = Some(prompt);
        }
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
        let summaries: BTreeMap<u64, String> = self
            .panes
            .iter()
            .filter_map(|(id, p)| Some((*id, p.track.summary.clone()?)))
            .collect();
        let idle = self.prompt_targets().len();
        let mut spawn: Option<Kind> = None;
        let mut prompt_all = false;
        let mut jump = None;
        let mut changes: Option<u64> = None;
        let can_open = !self.readonly && self.online;
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
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 58.), Sense::click());
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
                    if let Some(line) = summaries.get(id) {
                        let shown: String = line.chars().take(60).collect();
                        ui.painter().text(
                            rect.left_top() + Vec2::new(26., 37.),
                            egui::Align2::LEFT_TOP,
                            shown,
                            FontId::proportional(11.),
                            color(t.foreground).gamma_multiply(0.75),
                        );
                    }
                    let label_x = if asking { 150. } else { 10. };
                    ui.painter().text(rect.right_center() - Vec2::new(label_x, 0.), egui::Align2::RIGHT_CENTER, status.label(), FontId::proportional(12.), c);
                    let button = egui::Rect::from_center_size(rect.right_bottom() - Vec2::new(44., 11.), Vec2::new(72., 16.));
                    let shown = ui.put(button, egui::Button::new(RichText::new("Changes").size(11.)).small());
                    if shown.clicked() && can_open {
                        changes = Some(*id);
                    }
                    shown.on_hover_text("Open a pane listing the files this agent changed (git status and diff --stat)");
                    if changes == Some(*id) {
                        // The button was pressed: do not also treat the click as selecting the row.
                    } else if asking {
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
                if !self.readonly {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Start").size(11.).color(color(t.muted)));
                        for kind in Kind::ALL {
                            if ui.small_button(kind.name()).on_hover_text(format!("Run `{}` in a new pane here", kind.launch())).clicked() {
                                spawn = Some(kind);
                            }
                        }
                    });
                    let label = if idle == 0 { "Send a prompt to idle agents…".to_string() } else { format!("Send a prompt to {idle} idle agent(s)…") };
                    if ui.add_enabled(idle > 0, egui::Button::new(label)).clicked() {
                        prompt_all = true;
                    }
                }
            });
        if let Some(kind) = spawn {
            self.spawn_agent(kind);
            self.agents_open = false;
        }
        if prompt_all {
            self.open_agent_prompt();
        }
        if let Some(id) = changes {
            self.show_agent_changes(id);
            self.agents_open = false;
        }
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
    fn a_card_appears_for_an_agent_in_another_pane_and_goes_when_it_is_answered() {
        let (mut app, _rx) = app_with_screen("");
        app.window_focused = true;
        app.cfg.agent_notifications = true;
        app.focus = 1;
        agent_pane(&mut app, 2, Some("claude"), "esc to interrupt\r\n");
        app.agents_update();
        assert!(app.extra.cards.is_empty(), "nothing asks yet");
        let mut parser = vt100::Parser::new(10, 60, 0);
        parser.process("Apply this edit?\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n".as_bytes());
        let pane = app.panes.get_mut(&2).unwrap();
        pane.parser = parser;
        pane.track.computed -= Duration::from_secs(5);
        app.agents_update();
        assert_eq!(app.extra.cards.len(), 1);
        assert_eq!(app.extra.cards[0].pane, 2);
        assert_eq!(
            app.extra.cards[0].question.as_deref(),
            Some("Do you want to proceed?")
        );
        // The card draws, then goes once the agent stops asking.
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.cards_ui(ctx));
        assert_eq!(app.extra.cards.len(), 1);
        let pane = app.panes.get_mut(&2).unwrap();
        pane.parser = vt100::Parser::new(10, 60, 0);
        pane.track.changed = Instant::now();
        pane.track.computed -= Duration::from_secs(5);
        app.agents_update();
        assert!(app.extra.cards.is_empty());
    }

    #[test]
    fn no_card_for_the_pane_you_are_in_or_when_the_window_is_behind() {
        for (focused, focus) in [(true, 2), (false, 1)] {
            let (mut app, _rx) = app_with_screen("");
            app.window_focused = focused;
            app.cfg.agent_notifications = true;
            app.focus = focus;
            agent_pane(&mut app, 2, Some("claude"), "esc to interrupt\r\n");
            app.agents_update();
            let mut parser = vt100::Parser::new(10, 60, 0);
            parser.process("Do you want to proceed?\r\n❯ 1. Yes\r\n".as_bytes());
            let pane = app.panes.get_mut(&2).unwrap();
            pane.parser = parser;
            pane.track.computed -= Duration::from_secs(5);
            app.agents_update();
            assert!(
                app.extra.cards.is_empty(),
                "focused={focused} focus={focus}"
            );
        }
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
    fn spawn_types_the_command_into_the_new_pane_and_prompts_reach_only_idle_agents() {
        let (mut app, rx) = app_with_screen("");
        app.online = true;
        app.window_focused = true;
        app.spawn_agent(Kind::Codex);
        assert!(matches!(rx.try_recv(), Ok(Command::New(_))));
        app.run_spawned(7);
        assert!(
            matches!(rx.try_recv(), Ok(Command::Input { pane: 7, data }) if data == b"codex\r")
        );
        app.run_spawned(8);
        assert!(rx.try_recv().is_err(), "only the pane made for the spawn");
        agent_pane(&mut app, 2, Some("claude"), "all done\r\n");
        agent_pane(
            &mut app,
            3,
            Some("claude"),
            "Do you want to proceed?\r\n❯ 1. Yes\r\n",
        );
        app.agents_update();
        for id in [2, 3] {
            app.panes.get_mut(&id).unwrap().track.changed -= Duration::from_secs(10);
            app.panes.get_mut(&id).unwrap().track.computed -= Duration::from_secs(5);
        }
        app.agents_update();
        assert_eq!(app.prompt_targets(), vec![2]);
        while rx.try_recv().is_ok() {}
        assert_eq!(app.send_agent_prompt("  run the tests "), 1);
        assert!(
            matches!(rx.try_recv(), Ok(Command::Input { pane: 2, data }) if data == b"run the tests\r")
        );
        assert_eq!(app.send_agent_prompt("   "), 0);
        // In a worktree: the same flow, with the worktree script typed instead of the bare command.
        app.spawn_agent_worktree(Kind::Claude);
        assert!(matches!(rx.try_recv(), Ok(Command::New(_))));
        app.run_spawned(9);
        let Ok(Command::Input { pane: 9, data }) = rx.try_recv() else {
            panic!("the script is typed into the new pane")
        };
        let typed = String::from_utf8(data).unwrap();
        assert!(typed.starts_with("sh -c 'set -e;") && typed.contains("git worktree add"));
        assert!(typed.contains("exec claude'") && typed.ends_with('\r'));
        app.readonly = true;
        app.spawn_agent(Kind::Claude);
        assert!(app.spawn_cmd.is_none());
    }

    #[test]
    fn a_finished_agent_notification_carries_where_it_got_to() {
        let (mut app, _rx) = app_with_screen("");
        app.window_focused = true;
        agent_pane(
            &mut app,
            2,
            Some("claude"),
            "\u{23fa} Refactored the parser, 12 tests pass\r\n",
        );
        app.agents_update();
        assert_eq!(
            app.panes[&2].track.summary.as_deref(),
            Some("Refactored the parser, 12 tests pass")
        );
    }

    #[test]
    fn sandboxed_agents_are_started_through_teryon_sandbox() {
        let (mut app, rx) = app_with_screen("");
        app.online = true;
        app.spawn_agent(Kind::Codex);
        assert!(matches!(rx.try_recv(), Ok(Command::New(_))));
        app.run_spawned(5);
        assert!(matches!(rx.try_recv(), Ok(Command::Input { data, .. }) if data == b"codex\r"));
        app.cfg.agent_sandbox = true;
        app.spawn_agent(Kind::Codex);
        assert!(matches!(rx.try_recv(), Ok(Command::New(_))));
        app.run_spawned(6);
        assert!(
            matches!(rx.try_recv(), Ok(Command::Input { data, .. }) if data == b"teryon sandbox -- codex\r")
        );
        // In a worktree: named after the agent, running the sandboxed command.
        app.spawn_agent_worktree(Kind::Claude);
        assert!(matches!(rx.try_recv(), Ok(Command::New(_))));
        app.run_spawned(7);
        let Ok(Command::Input { data, .. }) = rx.try_recv() else {
            panic!("typed")
        };
        let typed = String::from_utf8(data).unwrap();
        assert!(
            typed.contains("agent/claude-") && typed.contains("exec teryon sandbox -- claude'"),
            "{typed}"
        );
    }

    #[test]
    fn changes_open_a_pane_in_the_agents_directory() {
        let (mut app, rx) = app_with_screen("");
        app.online = true;
        agent_pane(&mut app, 4, Some("claude"), "hi\r\n");
        app.panes.get_mut(&4).unwrap().info.cwd = "/srv/app".into();
        app.show_agent_changes(4);
        assert!(matches!(rx.try_recv(), Ok(Command::New(Some(c))) if c == "/srv/app"));
        app.run_spawned(9);
        let Ok(Command::Input { pane: 9, data }) = rx.try_recv() else {
            panic!("the command is typed into the new pane")
        };
        assert!(String::from_utf8(data)
            .unwrap()
            .contains("git status --short"));
        app.readonly = true;
        app.show_agent_changes(4);
        assert!(rx.try_recv().is_err());
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

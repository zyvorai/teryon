//! The ✦ assistant panel (⌘J): chat with Claude, with approval cards before anything changes.
use super::*;
use crate::ai::{self, api, key, tools, Agent, AgentEvent, Decision};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

const PANEL_WIDTH: f32 = 380.;
const EXAMPLES: [&str; 4] = [
    "Which apps are running?",
    "How much battery and disk is left?",
    "Open Calculator",
    "Open a terminal to my first SSH host",
];

pub(super) enum EnvOp {
    ListPanes,
    ReadPane(u64),
    RunInPane(u64, String, bool),
    ClosePane(u64),
    OpenSsh(String, String),
}
pub(super) struct EnvReq {
    op: EnvOp,
    reply: Sender<Result<String, String>>,
}

/// The agent thread's window into the app; every call is answered by the UI thread in `ai_pump`.
struct DesktopEnv {
    tx: Sender<EnvReq>,
    context: Arc<Mutex<String>>,
}
impl DesktopEnv {
    fn call(&self, op: EnvOp) -> Result<String, String> {
        let (reply, rx) = channel();
        self.tx
            .send(EnvReq { op, reply })
            .map_err(|_| "the app is closing")?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "the app did not answer".to_string())?
    }
}
impl tools::Env for DesktopEnv {
    fn context(&self) -> String {
        self.context.lock().map(|c| c.clone()).unwrap_or_default()
    }
    fn list_panes(&mut self) -> Result<String, String> {
        self.call(EnvOp::ListPanes)
    }
    fn read_pane(&mut self, pane: u64) -> Result<String, String> {
        self.call(EnvOp::ReadPane(pane))
    }
    fn run_in_pane(&mut self, pane: u64, text: &str, run: bool) -> Result<String, String> {
        self.call(EnvOp::RunInPane(pane, text.into(), run))
    }
    fn close_pane(&mut self, pane: u64) -> Result<String, String> {
        self.call(EnvOp::ClosePane(pane))
    }
    fn open_ssh_pane(&mut self, host: &str, workspace: &str) -> Result<String, String> {
        self.call(EnvOp::OpenSsh(host.into(), workspace.into()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ToolState {
    Waiting,
    Running,
    Done(bool, String),
}
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Item {
    User(String),
    Assistant(String),
    Tool {
        id: String,
        summary: String,
        state: ToolState,
    },
    Error(String),
}

pub(super) struct Panel {
    pub open: bool,
    pub(super) agent: Option<Agent>,
    pub(super) items: Vec<Item>,
    input: String,
    pub(super) busy: bool,
    /// `None` until first opened; then whether a key was found.
    key_found: Option<bool>,
    env_rx: Option<Receiver<EnvReq>>,
    context: Arc<Mutex<String>>,
    pub(super) focus_input: bool,
}
impl Panel {
    #[cfg(test)]
    pub(super) fn set_key_found(&mut self, found: bool) {
        self.key_found = Some(found);
    }
    pub(super) fn new() -> Self {
        Self {
            open: false,
            agent: None,
            items: Vec::new(),
            input: String::new(),
            busy: false,
            key_found: None,
            env_rx: None,
            context: Arc::new(Mutex::new(String::new())),
            focus_input: false,
        }
    }
}

/// Split a reply into prose and fenced code blocks.
fn blocks(text: &str) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    for (i, part) in text.split("```").enumerate() {
        let code = i % 2 == 1;
        let part = if code {
            // Drop the language tag on the first line.
            part.split_once('\n').map_or(part, |(_, rest)| rest)
        } else {
            part
        };
        let part = part.trim_matches('\n');
        if !part.trim().is_empty() {
            out.push((code, part.to_string()));
        }
    }
    out
}

impl Desktop {
    pub(super) fn ai_toggle(&mut self) {
        self.ai.open = !self.ai.open;
        if self.ai.open {
            self.ai.focus_input = true;
            if self.ai.key_found.is_none() {
                self.ai.key_found = Some(key::find().is_some());
            }
        }
    }

    /// Start the agent thread with the real API (needs a key).
    fn ai_start(&mut self) -> bool {
        let cfg = crate::config::load().map(|c| c.ai).unwrap_or_default();
        if !cfg.enabled {
            self.ai.items.push(Item::Error(
                "The assistant is disabled ([ai] enabled = false).".into(),
            ));
            return false;
        }
        let Some(api_key) = key::find() else {
            self.ai.key_found = Some(false);
            return false;
        };
        let model = api::Curl {
            settings: ai::cli::settings(&cfg),
            key: api_key,
        };
        self.ai_attach(Box::new(model), Arc::new(tools::System), cfg.auto_allow);
        true
    }

    /// Connect an agent built from `model`; split out so tests can script the model.
    pub(super) fn ai_attach(
        &mut self,
        model: Box<dyn api::Model>,
        runner: Arc<dyn tools::Runner>,
        auto: Vec<String>,
    ) {
        let (tx, rx) = channel();
        let env = DesktopEnv {
            tx,
            context: self.ai.context.clone(),
        };
        self.ai.env_rx = Some(rx);
        self.ai.agent = Some(Agent::spawn(model, runner, Box::new(env), auto));
        self.ai.key_found = Some(true);
    }

    pub(super) fn ai_send(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || self.ai.busy {
            return;
        }
        if self.ai.agent.is_none() && !self.ai_start() {
            return;
        }
        self.ai.items.push(Item::User(text.to_string()));
        self.ai.busy = true;
        if let Some(a) = &self.ai.agent {
            a.send(text);
        }
    }

    /// Answer the agent's pane requests and fold its events into the transcript (once per frame).
    pub(super) fn ai_pump(&mut self) {
        let ctx_text = format!(
            "host: {}\nworkspace: {}\nfocused pane: {}\npanes: {}",
            self.host,
            self.workspace,
            self.panes
                .get(&self.focus)
                .map_or("none".into(), |p| format!(
                    "{} “{}” in {}",
                    self.focus, p.info.title, p.info.cwd
                )),
            self.panes
                .keys()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        if let Ok(mut c) = self.ai.context.lock() {
            *c = ctx_text;
        }
        while let Some(req) = self.ai.env_rx.as_ref().and_then(|r| r.try_recv().ok()) {
            let reply = self.ai_env(req.op);
            let _ = req.reply.send(reply);
        }
        let mut events = Vec::new();
        if let Some(a) = &self.ai.agent {
            while let Ok(ev) = a.events.try_recv() {
                events.push(ev);
            }
        }
        for ev in events {
            match ev {
                AgentEvent::Text(t) => match self.ai.items.last_mut() {
                    Some(Item::Assistant(s)) => s.push_str(&t),
                    _ => self.ai.items.push(Item::Assistant(t)),
                },
                AgentEvent::ToolRequest {
                    id,
                    summary,
                    needs_approval,
                    ..
                } => self.ai.items.push(Item::Tool {
                    id,
                    summary,
                    state: if needs_approval {
                        ToolState::Waiting
                    } else {
                        ToolState::Running
                    },
                }),
                AgentEvent::ToolDone { id, ok, output } => {
                    let line = output
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(120)
                        .collect::<String>();
                    for item in &mut self.ai.items {
                        if let Item::Tool { id: i, state, .. } = item {
                            if *i == id {
                                *state = ToolState::Done(ok, line.clone());
                            }
                        }
                    }
                }
                AgentEvent::Error(m) => self.ai.items.push(Item::Error(m)),
                AgentEvent::Done => self.ai.busy = false,
            }
        }
    }

    fn ai_env(&mut self, op: EnvOp) -> Result<String, String> {
        match op {
            EnvOp::ListPanes => Ok(self
                .panes
                .iter()
                .map(|(id, p)| format!("{id}\t{}\t{}", p.info.title, p.info.cwd))
                .collect::<Vec<_>>()
                .join("\n")),
            EnvOp::ReadPane(id) => {
                let p = self.panes.get(&id).ok_or(format!("no pane {id}"))?;
                let text = p.parser.screen().contents();
                let text: String = text.chars().take(8000).collect();
                Ok(text)
            }
            EnvOp::RunInPane(id, text, run) => {
                let p = self.panes.get(&id).ok_or(format!("no pane {id}"))?;
                let bracketed = p.parser.screen().bracketed_paste() || text.contains('\n');
                let mut data = keys::paste(&text, bracketed);
                if run {
                    data.push(b'\r');
                }
                self.send(Command::Input { pane: id, data });
                Ok(format!(
                    "Typed into pane {id}{}",
                    if run { " and pressed Enter" } else { "" }
                ))
            }
            EnvOp::ClosePane(id) => {
                if !self.panes.contains_key(&id) {
                    return Err(format!("no pane {id}"));
                }
                self.send(Command::Close(id));
                Ok(format!("Closed pane {id}"))
            }
            EnvOp::OpenSsh(host, ws) => {
                self.run_action(ActionKind::Open(host.clone(), ws.clone()));
                Ok(format!("Opening {host}/{ws} in a new tab"))
            }
        }
    }

    pub(super) fn ai_panel(&mut self, ctx: &egui::Context) {
        self.ai_pump();
        if self.ai.busy || self.ai.env_rx.is_some() && self.ai.agent.is_some() && self.ai.open {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        if !self.ai.open {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::J))
            || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            self.ai.open = false;
            ctx.memory_mut(|m| m.request_focus(egui::Id::new(("teryon-terminal", self.focus))));
            return;
        }
        let t = self.theme.clone();
        let mut decisions: Vec<(String, Decision)> = Vec::new();
        let mut send: Option<String> = None;
        let mut close = false;
        let mut reset = false;
        egui::SidePanel::right("teryon-ai")
            .exact_width(PANEL_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(color(mix(t.panel, t.background, 0.4)))
                    .stroke(Stroke::new(1_f32, color(t.border)))
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        weighted(ctx, "✦  Assistant", "semibold", 14.).color(color(t.foreground)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, &t, Icon::Close, "Close  Esc").clicked() {
                            close = true;
                        }
                        if !self.ai.items.is_empty() && ui.small_button("New chat").clicked() {
                            reset = true;
                        }
                    });
                });
                ui.add_space(6.);
                egui::TopBottomPanel::bottom("teryon-ai-input")
                    .frame(egui::Frame::new().inner_margin(egui::Margin {
                        top: 8,
                        ..Default::default()
                    }))
                    .show_inside(ui, |ui| self.ai_input(ui, &t, &mut send));
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show_inside(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .stick_to_bottom(true)
                            .auto_shrink(false)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                self.ai_transcript(ui, ctx, &t, &mut decisions, &mut send);
                            });
                    });
            });
        if close {
            self.ai.open = false;
        }
        if reset {
            self.ai.agent = None;
            self.ai.env_rx = None;
            self.ai.items.clear();
            self.ai.busy = false;
        }
        for (id, d) in decisions {
            if let Some(a) = &self.ai.agent {
                a.decide(&id, d);
            }
            for item in &mut self.ai.items {
                if let Item::Tool { id: i, state, .. } = item {
                    if *i == id {
                        *state = if d == Decision::Deny {
                            ToolState::Done(false, "Denied".into())
                        } else {
                            ToolState::Running
                        };
                    }
                }
            }
        }
        if let Some(text) = send {
            self.ai_send(&text);
        }
    }

    fn ai_input(&mut self, ui: &mut egui::Ui, t: &Theme, send: &mut Option<String>) {
        let id = egui::Id::new("teryon-ai-input");
        // Enter sends, Shift+Enter inserts a newline.
        if ui.memory(|m| m.has_focus(id))
            && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift)
        {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
            if !self.ai.input.trim().is_empty() && !self.ai.busy {
                *send = Some(std::mem::take(&mut self.ai.input));
            }
        }
        let edit = ui.add(
            egui::TextEdit::multiline(&mut self.ai.input)
                .id(id)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("Ask anything, or tell me what to do…"),
        );
        if self.ai.focus_input {
            edit.request_focus();
            self.ai.focus_input = false;
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if self.ai.busy {
                    "Working…"
                } else {
                    "Enter to send · ⇧Enter for a new line"
                })
                .size(11.)
                .color(color(t.muted)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can = !self.ai.input.trim().is_empty() && !self.ai.busy;
                if ui.add_enabled(can, egui::Button::new("Send")).clicked() {
                    *send = Some(std::mem::take(&mut self.ai.input));
                }
            });
        });
    }

    fn ai_transcript(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        t: &Theme,
        decisions: &mut Vec<(String, Decision)>,
        send: &mut Option<String>,
    ) {
        if self.ai.key_found == Some(false) && self.ai.agent.is_none() {
            card(ui, t, |ui| {
                ui.label(weighted(ctx, "Add your API key", "semibold", 13.));
                ui.label(
                    RichText::new("Create a key in the Claude console, then in a terminal run:")
                        .size(12.)
                        .color(color(t.muted)),
                );
                let cmd = "export ANTHROPIC_API_KEY=sk-ant-…";
                ui.label(RichText::new(cmd).monospace().size(12.));
                ui.label(
                    RichText::new(
                        "or `teryon ai --set-key` to keep it in the Keychain, then restart TERYON.",
                    )
                    .size(12.)
                    .color(color(t.muted)),
                );
                if ui.small_button("Check again").clicked() {
                    self.ai.key_found = Some(key::find().is_some());
                }
            });
            return;
        }
        if self.ai.items.is_empty() {
            ui.add_space(8.);
            ui.label(weighted(ctx, "What can I do?", "semibold", 15.));
            ui.label(
                RichText::new("Chat, ask about this Mac, or have me open apps and SSH terminals. I ask before anything changes.")
                    .size(12.)
                    .color(color(t.muted)),
            );
            ui.add_space(6.);
            for ex in EXAMPLES {
                if ui
                    .add(
                        egui::Button::new(RichText::new(ex).size(12.))
                            .corner_radius(egui::CornerRadius::same(8)),
                    )
                    .clicked()
                {
                    *send = Some(ex.to_string());
                }
            }
            return;
        }
        for (n, item) in self.ai.items.clone().iter().enumerate() {
            match item {
                Item::User(text) => {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                        egui::Frame::new()
                            .fill(color(t.primary).gamma_multiply(0.85))
                            .corner_radius(egui::CornerRadius::same(12))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.set_max_width(PANEL_WIDTH - 70.);
                                ui.label(RichText::new(text).color(Color32::WHITE).size(13.));
                            });
                    });
                }
                Item::Assistant(text) => {
                    for (code, part) in blocks(text) {
                        if code {
                            egui::Frame::new()
                                .fill(color(t.background))
                                .stroke(Stroke::new(1_f32, color(t.border)))
                                .corner_radius(egui::CornerRadius::same(8))
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(&part).monospace().size(12.));
                                    if ui.small_button("Copy").clicked() {
                                        self.copy_text = Some(part.clone());
                                    }
                                });
                        } else {
                            ui.label(RichText::new(&part).size(13.).color(color(t.foreground)));
                        }
                    }
                }
                Item::Tool { id, summary, state } => {
                    card(ui, t, |ui| {
                        ui.label(RichText::new(summary).size(12.5).color(color(t.foreground)));
                        match state {
                            ToolState::Waiting => {
                                ui.horizontal(|ui| {
                                    if primary_button(ui, t, "Allow").clicked() {
                                        decisions.push((id.clone(), Decision::Allow));
                                    }
                                    if ui.button("Always this session").clicked() {
                                        decisions.push((id.clone(), Decision::AllowSession));
                                    }
                                    if ui.button("Deny").clicked() {
                                        decisions.push((id.clone(), Decision::Deny));
                                    }
                                });
                            }
                            ToolState::Running => {
                                ui.label(RichText::new("Running…").size(11.).color(color(t.muted)));
                            }
                            ToolState::Done(ok, line) => {
                                ui.label(
                                    RichText::new(format!(
                                        "{} {line}",
                                        if *ok { "✓" } else { "✗" }
                                    ))
                                    .size(11.)
                                    .color(color(if *ok {
                                        t.success
                                    } else {
                                        t.warning
                                    })),
                                );
                            }
                        }
                    });
                }
                Item::Error(m) => {
                    ui.label(
                        RichText::new(format!("⚠ {m}"))
                            .size(12.)
                            .color(color(t.warning)),
                    );
                }
            }
            if n + 1 < self.ai.items.len() {
                ui.add_space(6.);
            }
        }
        if self.ai.busy {
            ui.add_space(4.);
            ui.label(RichText::new("…").color(color(t.muted)));
        }
    }
}

fn card(ui: &mut egui::Ui, t: &Theme, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(color(mix(t.panel, t.foreground, 0.06)))
        .stroke(Stroke::new(1_f32, color(t.border)))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_split_into_prose_and_code() {
        let parts = blocks("Run this:\n```sh\nls -la\n```\nThen relax.");
        assert_eq!(
            parts,
            vec![
                (false, "Run this:".into()),
                (true, "ls -la".into()),
                (false, "Then relax.".into())
            ]
        );
        assert_eq!(blocks("plain"), vec![(false, "plain".into())]);
        // An unterminated fence still renders as code rather than vanishing.
        assert_eq!(
            blocks("a\n```\nb"),
            vec![(false, "a".into()), (true, "b".into())]
        );
    }
}

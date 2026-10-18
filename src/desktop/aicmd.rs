//! The ⌘⇧K command bar: describe what you want, get one command, insert it at the prompt (never auto-run).
use super::*;
use crate::ai::{self, api, key, redact, suggest::Suggestion};
use std::sync::mpsc::{channel, Receiver};

#[derive(Default)]
pub(super) struct Bar {
    pub open: bool,
    input: String,
    focus: bool,
    busy: bool,
    result: Option<Suggestion>,
    error: Option<String>,
    rx: Option<Receiver<Result<Suggestion, String>>>,
    pane: u64,
}

/// A best guess at the OS behind a pane, for hosts the daemon cannot tell us about.
fn os_hint(host: &str, cwd: &str) -> &'static str {
    if host == "local" {
        return std::env::consts::OS;
    }
    if cwd.starts_with("/Users/") {
        "macos"
    } else {
        "linux"
    }
}

impl Desktop {
    pub(super) fn aicmd_toggle(&mut self) {
        self.aicmd.open = !self.aicmd.open;
        if self.aicmd.open {
            self.aicmd.focus = true;
            self.aicmd.pane = self.focus;
            self.aicmd.error = None;
        }
    }

    /// Context sent with the request: host, OS, shell and directory of the focused pane (redacted).
    fn aicmd_context(&self) -> String {
        let Some(p) = self.panes.get(&self.aicmd.pane) else {
            return String::new();
        };
        let fg = p.info.fg.clone().unwrap_or_default();
        let shell = if crate::agents::is_shell(&fg) {
            fg.trim_start_matches('-').to_string()
        } else {
            "unknown".into()
        };
        redact::clean(&format!(
            "Host: {}\nOS: {}\nShell: {shell}\nDirectory: {}",
            self.host,
            os_hint(&self.host, &p.info.cwd),
            p.info.cwd
        ))
    }

    pub(super) fn aicmd_submit_with(&mut self, model: Box<dyn api::Model>) {
        let request = self.aicmd.input.trim().to_string();
        if request.is_empty() || self.aicmd.busy {
            return;
        }
        let context = self.aicmd_context();
        let (tx, rx) = channel();
        self.aicmd.rx = Some(rx);
        self.aicmd.busy = true;
        self.aicmd.result = None;
        self.aicmd.error = None;
        std::thread::spawn(move || {
            let _ = tx.send(ai::suggest::run(&*model, &context, &request));
        });
    }

    fn aicmd_submit(&mut self) {
        let cfg = crate::config::load().map(|c| c.ai).unwrap_or_default();
        let Some(api_key) = key::find() else {
            self.aicmd.error = Some(ai::cli::NO_KEY.into());
            return;
        };
        self.aicmd_submit_with(Box::new(api::Curl {
            settings: ai::cli::settings(&cfg),
            key: api_key,
        }));
    }

    /// Type `command` into `pane`; with `run` also press Enter. Never used for unchecked text without `run` being confirmed by the caller.
    pub(super) fn insert_command(&mut self, pane: u64, command: &str, run: bool) {
        let Some(p) = self.panes.get(&pane) else {
            return;
        };
        let bracketed = p.parser.screen().bracketed_paste();
        let mut data = keys::paste(command, bracketed);
        if run {
            data.push(b'\r');
        }
        self.send(Command::Input { pane, data });
    }

    pub(super) fn aicmd_ui(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.aicmd.rx {
            if let Ok(r) = rx.try_recv() {
                self.aicmd.busy = false;
                self.aicmd.rx = None;
                match r {
                    Ok(s) => self.aicmd.result = Some(s),
                    Err(e) => self.aicmd.error = Some(e),
                }
            }
        }
        if self.aicmd.busy {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        if !self.aicmd.open {
            return;
        }
        let shortcut = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::K)
        });
        if shortcut || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.aicmd.open = false;
            ctx.memory_mut(|m| m.request_focus(egui::Id::new(("teryon-terminal", self.focus))));
            return;
        }
        let t = self.theme.clone();
        let mut act: Option<bool> = None; // Some(false) insert, Some(true) run
        let mut submit = false;
        egui::Area::new(egui::Id::new("teryon-aicmd"))
            .anchor(egui::Align2::CENTER_TOP, [0., bar::BAR_HEIGHT + 24.])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color(mix(t.panel, t.foreground, 0.08)))
                    .stroke(Stroke::new(1_f32, color(t.border)))
                    .corner_radius(egui::CornerRadius::same(14))
                    .shadow(egui::Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(90) })
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(560.);
                        let id = egui::Id::new("teryon-aicmd-input");
                        let before = self.aicmd.input.clone();
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.aicmd.input)
                                .id(id)
                                .desired_width(f32::INFINITY)
                                .font(FontId::proportional(15.))
                                .hint_text("✦  Describe what you want to do…"),
                        );
                        if self.aicmd.focus {
                            edit.request_focus();
                            self.aicmd.focus = false;
                        }
                        if before != self.aicmd.input {
                            self.aicmd.result = None;
                            self.aicmd.error = None;
                        }
                        let enter = edit.has_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        let alt = ui.input(|i| i.modifiers.alt);
                        if enter {
                            match &self.aicmd.result {
                                Some(s) if alt && s.runnable() => act = Some(true),
                                Some(_) => act = Some(false),
                                None => submit = true,
                            }
                        }
                        if self.aicmd.busy {
                            ui.label(RichText::new("Thinking…").size(12.).color(color(t.muted)));
                        }
                        if let Some(e) = &self.aicmd.error {
                            ui.label(RichText::new(format!("⚠ {e}")).size(12.).color(color(t.warning)));
                        }
                        if let Some(s) = &self.aicmd.result {
                            ui.add_space(8.);
                            egui::Frame::new()
                                .fill(color(t.background))
                                .corner_radius(egui::CornerRadius::same(8))
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(&s.command).monospace().size(13.).color(color(t.foreground)));
                                });
                            if !s.explanation.is_empty() {
                                ui.label(RichText::new(&s.explanation).size(12.).color(color(t.muted)));
                            }
                            let rc = match (s.risk.as_str(), s.blocked.is_some()) {
                                (_, true) | ("high", _) => t.warning,
                                ("medium", _) => t.warning,
                                _ => t.success,
                            };
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("risk: {}", s.risk)).size(11.).color(color(rc)));
                                if let Some(why) = s.blocked {
                                    ui.label(RichText::new(format!("blocked: {why}")).size(11.).color(color(t.warning)));
                                }
                            });
                            ui.horizontal(|ui| {
                                if s.blocked.is_none() && primary_button(ui, &t, "Insert  ⏎").clicked() {
                                    act = Some(false);
                                }
                                if s.runnable() && ui.button("Run  ⌥⏎").clicked() {
                                    act = Some(true);
                                }
                                ui.label(RichText::new("Inserted at your prompt; you press Enter yourself.").size(11.).color(color(t.muted)));
                            });
                        } else if !self.aicmd.busy && self.aicmd.error.is_none() {
                            ui.label(RichText::new("Enter to translate · Esc to close. Nothing runs until you say so.").size(11.).color(color(t.muted)));
                        }
                    });
            });
        if submit {
            self.aicmd_submit();
        }
        if let (Some(run), Some(s)) = (act, self.aicmd.result.clone()) {
            if s.blocked.is_none() {
                self.insert_command(self.aicmd.pane, &s.command, run && s.runnable());
                self.aicmd.open = false;
                self.aicmd.result = None;
                self.aicmd.input.clear();
                ctx.memory_mut(|m| {
                    m.request_focus(egui::Id::new(("teryon-terminal", self.aicmd.pane)))
                });
            }
        }
    }

    /// "Explain / fix this" for a failed command: opens the assistant with a redacted summary of the failure.
    pub(super) fn ai_explain(&mut self, id: u64) {
        let Some(p) = self.panes.get(&id) else { return };
        let code = p
            .marks
            .iter()
            .rev()
            .find(|m| m.kind == 'D')
            .and_then(|m| m.exit);
        let screen = p.parser.screen();
        let (_, cols) = screen.size();
        let rows: Vec<String> = screen.rows(0, cols).collect();
        let used = rows
            .iter()
            .rposition(|r| !r.trim().is_empty())
            .map_or(0, |i| i + 1);
        let tail = rows[used.saturating_sub(40)..used].join("\n");
        let prompt = format!(
            "The last command in pane {id} failed{}. Host: {}, directory: {}. Explain briefly what went wrong and propose a fix with suggest_command.\n\n{}",
            code.map_or(String::new(), |c| format!(" with exit status {c}")),
            self.host,
            p.info.cwd,
            redact::untrusted(&tail)
        );
        self.ai.open = true;
        self.ai.focus_input = true;
        self.ai_send(&prompt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::api::{Event, Model, Turn};
    use crate::desktop::tests::{app_with_screen, drain};

    struct Canned;
    impl Model for Canned {
        fn stream(
            &self,
            _: &str,
            _: &serde_json::Value,
            m: &[serde_json::Value],
            on: &mut dyn FnMut(Event),
        ) -> Result<Turn, String> {
            let user = m[0]["content"].as_str().unwrap();
            assert!(user.contains("Directory: /p") && user.contains("Request: biggest files"));
            on(Event::Text("{\"command\": \"du -ah . | sort -rh | head -5\", \"explanation\": \"Top five\", \"risk\": \"low\"}".into()));
            Ok(Turn::default())
        }
    }

    #[test]
    fn a_request_becomes_a_command_that_is_inserted_but_not_run() {
        let (mut app, rx) = app_with_screen("");
        app.panes.get_mut(&1).unwrap().info.cwd = "/p".into();
        app.aicmd_toggle();
        app.aicmd.input = "biggest files".into();
        app.aicmd_submit_with(Box::new(Canned));
        let ctx = egui::Context::default();
        for _ in 0..200 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.aicmd_ui(ctx));
            if app.aicmd.result.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let s = app.aicmd.result.clone().expect("a suggestion");
        assert!(s.runnable());
        app.insert_command(1, &s.command, false);
        let sent = drain(&rx);
        assert!(
            matches!(&sent[..], [Command::Input { pane: 1, data }] if data == b"du -ah . | sort -rh | head -5")
        );
    }

    #[test]
    fn explaining_a_failure_sends_a_redacted_framed_prompt_to_the_assistant() {
        use std::sync::{Arc, Mutex};
        struct Spy(Arc<Mutex<Vec<String>>>);
        impl Model for Spy {
            fn stream(
                &self,
                _: &str,
                _: &serde_json::Value,
                m: &[serde_json::Value],
                _: &mut dyn FnMut(Event),
            ) -> Result<Turn, String> {
                self.0
                    .lock()
                    .unwrap()
                    .push(m.last().unwrap()["content"].to_string());
                Ok(Turn::default())
            }
        }
        struct NoRun;
        impl crate::ai::tools::Runner for NoRun {
            fn run(&self, _: &str, _: &[String], _: Duration) -> Result<String, String> {
                Ok(String::new())
            }
        }
        let secret = format!("sk-ant-{}", "aB3d".repeat(8));
        let (mut app, _rx) =
            app_with_screen(&format!("cargo test\r\nerror: boom with key {secret}\r\n"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        app.ai_attach(Box::new(Spy(seen.clone())), Arc::new(NoRun), vec![]);
        app.ai_explain(1);
        for _ in 0..200 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let got = seen.lock().unwrap().join("");
        assert!(
            got.contains("terminal_output") && got.contains("boom"),
            "{got}"
        );
        assert!(!got.contains(&secret) && got.contains("redacted:anthropic-key"));
    }
}

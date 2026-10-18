//! Running saved commands: ask for their `{{placeholders}}`, then type them into the focused pane.
use super::*;
use crate::snippet;

pub(super) struct SnippetPrompt {
    pub idx: usize,
    pub params: Vec<String>,
    pub values: Vec<String>,
    pub focus_first: bool,
}

impl Desktop {
    pub(super) fn run_snippet(&mut self, idx: usize) {
        let Some(sn) = self.cfg.snippets.get(idx) else {
            return;
        };
        let params = snippet::placeholders(&sn.command);
        if params.is_empty() {
            self.send_snippet(idx, &BTreeMap::new());
        } else {
            self.snippet_prompt = Some(SnippetPrompt {
                idx,
                values: vec![String::new(); params.len()],
                params,
                focus_first: true,
            });
        }
    }
    /// Type the snippet into the focused pane (every pane while broadcasting); Enter only for
    /// single-line snippets marked `run = true`.
    pub(super) fn send_snippet(&mut self, idx: usize, values: &BTreeMap<String, String>) {
        let Some(sn) = self.cfg.snippets.get(idx).cloned() else {
            return;
        };
        let text = snippet::fill(&sn.command, values);
        let multiline = text.contains('\n');
        let bracketed = self
            .panes
            .get(&self.focus)
            .is_some_and(|p| p.parser.screen().bracketed_paste());
        let mut data = keys::paste(&text, bracketed || multiline);
        let run = sn.run && !multiline;
        if run {
            data.push(b'\r');
        }
        self.input(data);
        if multiline && sn.run {
            self.toast("Multi-line snippet typed but not run: press Enter to run it");
        }
    }
    pub(super) fn snippet_ui(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.snippet_prompt.as_mut() else {
            return;
        };
        let name = self
            .cfg
            .snippets
            .get(prompt.idx)
            .map_or(String::new(), |s| s.name.clone());
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new(format!("Snippet: {name}"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0., 90.])
            .show(ctx, |ui| {
                for (i, param) in prompt.params.iter().enumerate() {
                    ui.label(RichText::new(param).size(12.));
                    let edit = ui
                        .add(egui::TextEdit::singleline(&mut prompt.values[i]).desired_width(360.));
                    if i == 0 && prompt.focus_first {
                        edit.request_focus();
                        prompt.focus_first = false;
                    }
                    if edit.lost_focus() && ui.input(|inp| inp.key_pressed(Key::Enter)) {
                        submit = true;
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Type it").clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|inp| inp.key_pressed(Key::Escape)) {
                    cancel = true;
                }
            });
        if submit {
            if let Some(p) = self.snippet_prompt.take() {
                let values: BTreeMap<String, String> = p.params.into_iter().zip(p.values).collect();
                self.send_snippet(p.idx, &values);
            }
        } else if cancel {
            self.snippet_prompt = None;
        }
    }
}

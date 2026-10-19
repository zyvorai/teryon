//! Floating panes: one pane lifted out of the layout into a draggable, resizable window on top of it.
use super::*;

impl Desktop {
    /// Pane ids in the tiled layout (everything except the floating pane).
    pub(super) fn tiled_ids(&self) -> Vec<u64> {
        self.panes
            .keys()
            .copied()
            .filter(|id| self.extra.floating != Some(*id))
            .collect()
    }
    /// The pane the tiled layout should treat as focused: the floating pane cannot be shown twice.
    pub(super) fn tiled_focus(&self, ids: &[u64]) -> u64 {
        if self.extra.floating == Some(self.focus) {
            ids.first().copied().unwrap_or(self.focus)
        } else {
            self.focus
        }
    }
    pub(super) fn toggle_float(&mut self) {
        if self.extra.floating.take().is_some() {
            return;
        }
        if self.panes.len() < 2 {
            self.toast("Open another pane first: a floating pane needs something to float over");
            return;
        }
        self.extra.floating = Some(self.focus);
    }
    /// ⌥⌘N: a new shell that appears already floating.
    pub(super) fn new_floating(&mut self) {
        self.pending_float = true;
        let cwd = self.focus_cwd();
        self.send(Command::New(cwd));
    }
    /// Apply what a pane's title bar or click asked for, and keep the shell's size in step.
    pub(super) fn handle_term_out(&mut self, id: u64, out: TermOut) {
        match out.action {
            Some(PaneAction::Rename(pane)) => {
                let title = self.panes.get(&pane).map(|p| p.info.title.clone());
                self.renaming = title.map(|title| (pane, title));
            }
            Some(PaneAction::Zoom(pane)) => {
                self.focus = pane;
                if self.extra.floating == Some(pane) {
                    self.extra.floating = None;
                } else {
                    self.run_action(ActionKind::Zoom);
                }
            }
            Some(PaneAction::Close(pane)) => {
                self.focus = pane;
                self.close_focused_pane();
            }
            None => {}
        }
        if id == self.focus {
            if self.online
                && !self.readonly
                && out.size != self.last_size
                && self
                    .tx
                    .try_send(Command::Resize(out.size.0, out.size.1))
                    .is_ok()
            {
                self.last_size = out.size;
            }
        } else if out.clicked {
            self.focus = id;
        }
    }
    pub(super) fn floating_ui(&mut self, ctx: &egui::Context) {
        let Some(id) = self.extra.floating else {
            return;
        };
        if !self.panes.contains_key(&id) || self.panes.len() < 2 {
            self.extra.floating = None;
            return;
        }
        let t = self.theme.clone();
        let screen = ctx.screen_rect();
        let size =
            Vec2::new(screen.width() * 0.62, screen.height() * 0.55).max(Vec2::new(320., 200.));
        let pos = Pos2::new(
            screen.center().x - size.x / 2.,
            screen.center().y - size.y / 2.,
        );
        let mut out: Option<TermOut> = None;
        egui::Window::new(format!("teryon-float-{id}"))
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_pos(pos)
            .default_size(size)
            .min_size(Vec2::new(280., 160.))
            .frame(
                egui::Frame::new()
                    .fill(color(t.background))
                    .stroke(Stroke::new(1.5_f32, color(t.accent)))
                    .corner_radius(egui::CornerRadius::same(10))
                    .shadow(egui::Shadow {
                        offset: [0, 12],
                        blur: 32,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    }),
            )
            .show(ctx, |ui| {
                let h = ui.available_height();
                out = self.draw_terminal(ui, id, h);
            });
        if let Some(out) = out {
            self.handle_term_out(id, out);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn floating_needs_two_panes_and_is_left_out_of_the_tiled_layout() {
        let (mut app, _rx) = super::super::tests::app_for_extras();
        app.toggle_float();
        assert!(app.extra.floating.is_none() && app.toast.is_some());
        app.add_test_pane(2);
        app.focus = 2;
        app.toggle_float();
        assert_eq!(app.extra.floating, Some(2));
        let tiled = app.tiled_ids();
        assert_eq!(tiled, vec![1]);
        assert_eq!(
            app.tiled_focus(&tiled),
            1,
            "the floating pane is never also tiled"
        );
        app.toggle_float();
        assert!(app.extra.floating.is_none());
    }
}

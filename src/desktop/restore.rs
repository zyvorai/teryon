//! Keeping each workspace's layout (titles and directories) so a rebooted host can be set up again.
use super::*;
use crate::layout::{self, Layout};

/// Whether to offer restoring `saved` for a workspace that currently has `views`.
pub(super) fn should_offer(views: &[PaneView], saved: Option<&Layout>) -> bool {
    layout::is_fresh(views) && saved.is_some_and(|l| l.panes.len() > 1)
}

impl Desktop {
    /// Called after every state update: offer a restore once, otherwise keep the saved layout current.
    pub(super) fn after_state(&mut self) {
        if !self.persist || self.panes.is_empty() {
            return;
        }
        let views: Vec<PaneView> = self.panes.values().map(|p| p.info.clone()).collect();
        let Ok(path) = layout::auto_path(&self.host, &self.workspace) else {
            return;
        };
        if !self.extra.restore_checked {
            self.extra.restore_checked = true;
            let saved = layout::load(&path).ok();
            if should_offer(&views, saved.as_ref()) {
                self.extra.restore_offer = saved;
            }
        }
        if self.extra.restore_offer.is_some() {
            return;
        }
        let now = Layout::from_panes(&views);
        let due = self
            .extra
            .last_layout_save
            .is_none_or(|t| t.elapsed() > Duration::from_secs(3));
        if due && self.extra.last_layout.as_ref() != Some(&now) {
            let _ = layout::save(&path, &now);
            self.extra.last_layout = Some(now);
            self.extra.last_layout_save = Some(Instant::now());
        }
    }
    /// Open the saved panes (directories and titles only; no command is run).
    pub(super) fn restore_layout(&mut self) {
        let Some(saved) = self.extra.restore_offer.take() else {
            return;
        };
        if let Some(first) = saved.panes.first() {
            if let Some(p) = self.panes.get(&self.focus) {
                if p.info.title != first.title {
                    let title = first.title.clone();
                    self.send(Command::Rename(self.focus, title));
                }
            }
        }
        for p in saved.panes.iter().skip(1) {
            self.extra
                .pending_titles
                .push((p.title.clone(), Instant::now()));
            self.send(Command::New(Some(p.cwd.clone())));
        }
        self.toast(format!("Restored {} panes", saved.panes.len()));
    }
    /// A pane was created: give it the next saved title, if one is still fresh.
    pub(super) fn title_new_pane(&mut self, id: u64) {
        self.extra
            .pending_titles
            .retain(|(_, at)| at.elapsed() < Duration::from_secs(15));
        if !self.extra.pending_titles.is_empty() {
            let (title, _) = self.extra.pending_titles.remove(0);
            self.send(Command::Rename(id, title));
        }
    }
    pub(super) fn restore_ui(&mut self, ctx: &egui::Context) {
        let Some(saved) = self.extra.restore_offer.clone() else {
            return;
        };
        let t = self.theme.clone();
        let mut action = 0;
        egui::Area::new(egui::Id::new("teryon-restore"))
            .anchor(egui::Align2::CENTER_TOP, [0., bar::BAR_HEIGHT + 12.])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color(mix(t.panel, t.foreground, 0.08)))
                    .stroke(Stroke::new(1_f32, color(t.border)))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.label(
                            weighted(ctx, format!("Restore your {} panes?", saved.panes.len()), "semibold", 13.)
                                .color(color(t.foreground)),
                        );
                        ui.label(
                            RichText::new("They open in their old directories with their old names. No commands are rerun.")
                                .size(12.)
                                .color(color(t.muted)),
                        );
                        ui.horizontal(|ui| {
                            if primary_button(ui, &t, "Restore").clicked() {
                                action = 1;
                            }
                            if ui.button("Not now").clicked() {
                                action = 2;
                            }
                        });
                    });
            });
        match action {
            1 => self.restore_layout(),
            2 => self.extra.restore_offer = None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn view(title: &str) -> PaneView {
        PaneView {
            title: title.into(),
            ..Default::default()
        }
    }
    #[test]
    fn the_offer_appears_only_for_a_fresh_workspace_with_a_richer_saved_layout() {
        let two = Layout::from_panes(&[view("Shell"), view("logs")]);
        let one = Layout::from_panes(&[view("Shell")]);
        assert!(should_offer(&[view("Shell")], Some(&two)));
        assert!(!should_offer(&[view("Shell")], Some(&one)));
        assert!(!should_offer(&[view("Shell")], None));
        assert!(!should_offer(&[view("Shell"), view("x")], Some(&two)));
        assert!(!should_offer(&[view("build")], Some(&two)));
    }
}

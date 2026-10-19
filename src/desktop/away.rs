//! "While you were away": what happened in a persistent workspace since the app last looked.
use super::*;
use crate::agents::{Kind, Status};

/// Shortest absence worth reporting.
const MIN_AWAY_SECS: u64 = 60;

fn seen_path(host: &str, workspace: &str) -> Option<std::path::PathBuf> {
    // Same name cleaning as saved layouts.
    let p = crate::layout::auto_path(host, workspace).ok()?;
    let name = p.file_name()?.to_string_lossy().replace(".json", ".seen");
    Some(p.parent()?.parent()?.join("seen").join(name))
}

/// Lines describing what finished or is waiting, from marks (`D` = command done) and agent states.
pub(super) fn digest(
    marks: &[(u64, &[Mark])],
    agents: &[(u64, Kind, Status)],
    since: u64,
) -> Vec<String> {
    let mut done = 0usize;
    let mut failed: Vec<(u64, i32)> = Vec::new();
    for (pane, ms) in marks {
        for m in ms.iter().filter(|m| m.kind == 'D' && m.at > since) {
            done += 1;
            if let Some(code) = m.exit.filter(|c| *c != 0) {
                failed.push((*pane, code));
            }
        }
    }
    let mut out = Vec::new();
    if done > 0 {
        let mut line = format!(
            "{done} command{} finished",
            if done == 1 { "" } else { "s" }
        );
        if !failed.is_empty() {
            let list: Vec<String> = failed
                .iter()
                .take(3)
                .map(|(p, c)| format!("pane {p} exit {c}"))
                .collect();
            line.push_str(&format!(", {} failed ({})", failed.len(), list.join(", ")));
        }
        out.push(line);
    }
    for (pane, kind, status) in agents {
        match status {
            Status::NeedsYou => out.push(format!("{} in pane {pane} needs you", kind.name())),
            Status::Done => out.push(format!("{} in pane {pane} finished", kind.name())),
            _ => {}
        }
    }
    out
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Desktop {
    /// Keep the "last seen" time fresh (every 20 s) while attached.
    pub(super) fn away_touch(&mut self) {
        if !self.persist
            || self.panes.is_empty()
            || self
                .extra
                .away_touched
                .is_some_and(|t| t.elapsed() < Duration::from_secs(20))
        {
            return;
        }
        self.extra.away_touched = Some(Instant::now());
        if let Some(path) = seen_path(&self.host, &self.workspace) {
            let _ = crate::util::write_private_atomic(&path, now_secs().to_string().as_bytes());
        }
    }

    /// Once per attach: if you were away for a while, build the card.
    pub(super) fn away_check(&mut self) {
        if self.extra.away_checked || self.panes.is_empty() || !self.persist {
            return;
        }
        self.extra.away_checked = true;
        let Some(path) = seen_path(&self.host, &self.workspace) else {
            return;
        };
        let Some(since) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
        else {
            return;
        };
        if now_secs().saturating_sub(since) < MIN_AWAY_SECS {
            return;
        }
        let marks: Vec<(u64, &[Mark])> = self
            .panes
            .iter()
            .map(|(id, p)| (*id, p.marks.as_slice()))
            .collect();
        let agents: Vec<(u64, Kind, Status)> = self
            .agents_list()
            .into_iter()
            .map(|(id, k, s, ..)| (id, k, s))
            .collect();
        let lines = digest(&marks, &agents, since);
        if !lines.is_empty() {
            self.extra.away = Some((lines, since));
        }
    }

    pub(super) fn away_ui(&mut self, ctx: &egui::Context) {
        let Some((lines, since)) = self.extra.away.clone() else {
            return;
        };
        let t = self.theme.clone();
        let mut action = 0;
        let ago = now_secs().saturating_sub(since);
        let ago = if ago >= 3600 {
            format!("{} h", ago / 3600)
        } else {
            format!("{} min", ago / 60)
        };
        egui::Area::new(egui::Id::new("teryon-away"))
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
                            weighted(ctx, format!("While you were away ({ago})"), "semibold", 13.)
                                .color(color(t.foreground)),
                        );
                        for l in &lines {
                            ui.label(
                                RichText::new(format!("• {l}"))
                                    .size(12.)
                                    .color(color(t.muted)),
                            );
                        }
                        ui.horizontal(|ui| {
                            if primary_button(ui, &t, "✦ Summarize").clicked() {
                                action = 1;
                            }
                            if ui.button("Dismiss").clicked() {
                                action = 2;
                            }
                        });
                    });
            });
        match action {
            1 => {
                self.extra.away = None;
                self.ai.open = true;
                self.ai.focus_input = true;
                self.ai_send("Summarize in three short lines what happened in this workspace while I was away. Use read_pane on panes that matter.");
            }
            2 => self.extra.away = None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;

    fn d(at: u64, exit: i32) -> Mark {
        Mark {
            kind: 'D',
            line: 1,
            exit: Some(exit),
            ms: Some(10),
            at,
            cmd: None,
        }
    }

    #[test]
    fn digest_counts_only_commands_after_the_last_visit() {
        let old = [d(50, 0), d(150, 0), d(160, 2), d(170, 1)];
        let lines = digest(
            &[(1, &old[..])],
            &[
                (2, Kind::Claude, Status::NeedsYou),
                (3, Kind::Codex, Status::Working),
            ],
            100,
        );
        assert_eq!(
            lines[0],
            "3 commands finished, 2 failed (pane 1 exit 2, pane 1 exit 1)"
        );
        assert_eq!(lines[1], "Claude in pane 2 needs you");
        assert_eq!(lines.len(), 2);
        assert!(digest(&[(1, &old[..1])], &[], 100).is_empty());
        assert_eq!(
            digest(&[(1, &old[1..2])], &[], 100),
            vec!["1 command finished".to_string()]
        );
    }

    #[test]
    fn the_card_draws_and_dismisses() {
        let (mut app, _rx) = app_with_screen("");
        app.extra.away = Some((vec!["2 commands finished".into()], now_secs() - 600));
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.away_ui(ctx));
        assert!(app.extra.away.is_some());
    }
}

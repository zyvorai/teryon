//! "While you were away": what happened in a persistent workspace since the app last looked.
use super::*;
use crate::agents::{Kind, Status};

/// Shortest absence worth reporting.
const MIN_AWAY_SECS: u64 = 60;
/// Most event rows the card shows (the newest ones).
const MAX_ROWS: usize = 8;

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

/// One thing that happened while you were away, as a row of the card.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AwayItem {
    /// Unix seconds it happened; 0 for something still true now (an agent waiting for you).
    pub at: u64,
    pub pane: u64,
    /// The absolute line to open in that pane: where the command's output starts.
    pub line: Option<u64>,
    pub text: String,
    pub bad: bool,
}

/// The events behind `digest`, oldest first, with the command line, result and where to look.
pub(super) fn timeline(
    marks: &[(u64, &[Mark])],
    agents: &[(u64, Kind, Status)],
    since: u64,
) -> Vec<AwayItem> {
    let mut out = Vec::new();
    for (pane, ms) in marks {
        for (i, d) in ms.iter().enumerate() {
            if d.kind != 'D' || d.at <= since {
                continue;
            }
            // The command is on the `C` mark that opened this output.
            // and only counts if no other command finished in between.
            let start = ms[..i]
                .iter()
                .rev()
                .take_while(|m| m.kind != 'D')
                .find(|m| m.kind == 'C');
            let cmd = start
                .and_then(|m| m.cmd.as_deref())
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map_or("a command".to_string(), |c| {
                    let short: String = c.chars().take(56).collect();
                    if c.chars().count() > 56 {
                        format!("{short}…")
                    } else {
                        short
                    }
                });
            let took = d.ms.map(super::term::format_duration);
            let (result, bad) = match (d.exit, took) {
                (Some(0), Some(t)) => (format!("✓ {t}"), false),
                (Some(0), None) => ("✓".to_string(), false),
                (Some(c), Some(t)) => (format!("✗ exit {c} · {t}"), true),
                (Some(c), None) => (format!("✗ exit {c}"), true),
                (None, Some(t)) => (format!("done · {t}"), false),
                (None, None) => ("done".to_string(), false),
            };
            out.push(AwayItem {
                at: d.at,
                pane: *pane,
                line: Some(start.map_or(d.line, |m| m.line)),
                text: format!("{cmd}  {result}"),
                bad,
            });
        }
    }
    out.sort_by_key(|i| (i.at, i.pane, i.line));
    for (pane, kind, status) in agents {
        let (text, bad) = match status {
            Status::NeedsYou => (format!("{} needs you", kind.name()), true),
            Status::Done => (format!("{} finished", kind.name()), false),
            _ => continue,
        };
        out.push(AwayItem {
            at: 0,
            pane: *pane,
            line: None,
            text,
            bad,
        });
    }
    out
}

/// "5 min ago", "2 h 10 min ago"; "now" for something still true.
fn ago(secs: u64) -> String {
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", secs / 60),
        _ => format!("{} h {} min ago", secs / 3600, secs % 3600 / 60),
    }
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
            self.extra.away_items = timeline(&marks, &agents, since);
            self.extra.away = Some((lines, since));
        }
    }

    /// A row was clicked: show that pane, and the command's output when the row has one.
    fn open_away_item(&mut self, pane: u64, line: Option<u64>) {
        if !self.panes.contains_key(&pane) {
            self.toast("That pane is gone");
            return;
        }
        self.focus = pane;
        if let Some(line) = line {
            self.goto_line(pane, line);
        }
    }

    pub(super) fn away_ui(&mut self, ctx: &egui::Context) {
        let Some((lines, since)) = self.extra.away.clone() else {
            return;
        };
        let items = self.extra.away_items.clone();
        let mut open: Option<(u64, Option<u64>)> = None;
        let t = self.theme.clone();
        let mut action = 0;
        let gone = now_secs().saturating_sub(since);
        let gone = if gone >= 3600 {
            format!("{} h", gone / 3600)
        } else {
            format!("{} min", gone / 60)
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
                            weighted(
                                ctx,
                                format!("While you were away ({gone})"),
                                "semibold",
                                13.,
                            )
                            .color(color(t.foreground)),
                        );
                        // Counts first, then each event as a row that takes you to it.
                        if let Some(first) = lines.first() {
                            ui.label(RichText::new(first).size(12.).color(color(t.muted)));
                        }
                        let now = now_secs();
                        let shown = items.len().min(MAX_ROWS);
                        if items.len() > shown {
                            ui.label(
                                RichText::new(format!("{} earlier not shown", items.len() - shown))
                                    .size(11.)
                                    .color(color(t.muted)),
                            );
                        }
                        for item in &items[items.len() - shown..] {
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width().max(420.), 22.),
                                Sense::click(),
                            );
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    6.,
                                    color(mix(t.panel, t.foreground, 0.1)),
                                );
                            }
                            let when = if item.at == 0 {
                                "now".to_string()
                            } else {
                                ago(now.saturating_sub(item.at))
                            };
                            let ink = if item.bad { t.warning } else { t.foreground };
                            ui.painter().text(
                                rect.left_center() + Vec2::new(8., 0.),
                                egui::Align2::LEFT_CENTER,
                                format!("pane {}", item.pane),
                                FontId::proportional(11.),
                                color(t.muted),
                            );
                            ui.painter().text(
                                rect.left_center() + Vec2::new(62., 0.),
                                egui::Align2::LEFT_CENTER,
                                &item.text,
                                FontId::proportional(12.),
                                color(ink),
                            );
                            ui.painter().text(
                                rect.right_center() - Vec2::new(8., 0.),
                                egui::Align2::RIGHT_CENTER,
                                when,
                                FontId::proportional(11.),
                                color(t.muted),
                            );
                            if resp.on_hover_text("Open it in the pane").clicked() {
                                open = Some((item.pane, item.line));
                            }
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
        if let Some((pane, line)) = open {
            self.open_away_item(pane, line);
        }
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

    fn mk(
        kind: char,
        line: u64,
        at: u64,
        exit: Option<i32>,
        ms: Option<u64>,
        cmd: Option<&str>,
    ) -> Mark {
        Mark {
            kind,
            line,
            exit,
            ms,
            at,
            cmd: cmd.map(str::to_string),
        }
    }

    #[test]
    fn the_timeline_names_each_command_its_result_and_where_to_look() {
        let marks = [
            mk('C', 10, 90, None, None, Some("old thing")),
            mk('D', 14, 95, Some(0), Some(100), None),
            mk('C', 20, 140, None, None, Some("cargo test --all")),
            mk('D', 31, 200, Some(101), Some(192_000), None),
            mk('C', 40, 205, None, None, Some("ls")),
            mk('D', 41, 206, Some(0), Some(30), None),
            mk('D', 50, 300, None, None, None),
        ];
        let items = timeline(
            &[(2, &marks[..])],
            &[
                (3, Kind::Claude, Status::NeedsYou),
                (4, Kind::Codex, Status::Working),
            ],
            100,
        );
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "cargo test --all  ✗ exit 101 · 3m 12s",
                "ls  ✓ 30 ms",
                "a command  done",
                "Claude needs you",
            ]
        );
        // Nothing from before the last visit; failures are flagged; each row opens its output.
        assert!(items[0].bad && !items[1].bad && items[3].bad);
        assert_eq!((items[0].pane, items[0].line), (2, Some(20)));
        assert_eq!((items[3].pane, items[3].line, items[3].at), (3, None, 0));
    }
    #[test]
    fn long_commands_are_shortened_and_the_clock_reads_naturally() {
        let long = "x".repeat(100);
        let marks = [
            mk('C', 1, 0, None, None, Some(&long)),
            mk('D', 2, 150, Some(0), None, None),
        ];
        let items = timeline(&[(1, &marks[..])], &[], 100);
        assert!(items[0].text.starts_with(&format!("{}…", "x".repeat(56))));
        assert_eq!(ago(10), "just now");
        assert_eq!(ago(300), "5 min ago");
        assert_eq!(ago(7_500), "2 h 5 min ago");
    }
    #[test]
    fn opening_a_row_focuses_the_pane_and_fetches_its_history() {
        let (mut app, rx) = app_with_screen("");
        app.add_test_pane(2);
        app.open_away_item(2, Some(20));
        assert_eq!(app.focus, 2);
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::Capture(2, CapturePurpose::Scrollback(2)))
        ));
        // A pane that is gone is said so, not silently ignored.
        app.open_away_item(9, Some(1));
        assert_eq!(app.focus, 2);
        assert!(app.toast.as_ref().is_some_and(|(t, _)| t.contains("gone")));
        // A row without a line (an agent) only shows the pane.
        app.open_away_item(1, None);
        assert_eq!(app.focus, 1);
    }
    #[test]
    fn the_card_draws_and_dismisses() {
        let (mut app, _rx) = app_with_screen("");
        app.extra.away = Some((vec!["2 commands finished".into()], now_secs() - 600));
        app.extra.away_items = vec![
            AwayItem {
                at: now_secs() - 300,
                pane: 1,
                line: Some(3),
                text: "cargo build  ✗ exit 101 · 4.0 s".into(),
                bad: true,
            },
            AwayItem {
                at: 0,
                pane: 1,
                line: None,
                text: "Claude needs you".into(),
                bad: true,
            },
        ];
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.away_ui(ctx));
        assert!(app.extra.away.is_some());
    }
}

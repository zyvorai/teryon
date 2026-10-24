//! "Host health": load and memory of the daemon's machine over the last half hour, as two small graphs.
use super::*;

/// Points for a line through `values` (oldest first) inside `rect`, scaled so `top` is the upper edge.
fn polyline(rect: Rect, values: &[f32], top: f32) -> Vec<Pos2> {
    let n = values.len();
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = if n < 2 {
                rect.right()
            } else {
                rect.left() + rect.width() * i as f32 / (n - 1) as f32
            };
            let y = rect.bottom() - rect.height() * (v / top.max(f32::EPSILON)).clamp(0., 1.);
            Pos2::new(x, y)
        })
        .collect()
}

impl Desktop {
    fn graph(
        &self,
        ui: &mut egui::Ui,
        name: &str,
        now: String,
        values: &[f32],
        top: f32,
        line: Color32,
    ) {
        let t = &self.theme;
        ui.horizontal(|ui| {
            ui.label(RichText::new(name).size(11.).color(color(t.muted)));
            ui.label(
                RichText::new(now)
                    .monospace()
                    .size(12.)
                    .color(color(t.foreground)),
            );
            ui.label(
                RichText::new(format!("max {top:.1}"))
                    .size(10.)
                    .color(color(t.muted)),
            );
        });
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(200.), 64.),
            Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, 6., color(mix(t.panel, t.foreground, 0.04)));
        if values.len() >= 2 {
            ui.painter().add(egui::Shape::line(
                polyline(rect.shrink(4.), values, top),
                Stroke::new(1.5_f32, line),
            ));
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "collecting…",
                egui::FontId::proportional(11.),
                color(t.muted),
            );
        }
    }
    pub(super) fn host_ui(&mut self, ctx: &egui::Context) {
        if !self.host_open {
            return;
        }
        let mut open = true;
        let history = self.link.history();
        let loads: Vec<f32> = history.iter().map(|h| h.load).collect();
        let mems: Vec<f32> = history
            .iter()
            .filter_map(|h| h.mem.map(f32::from))
            .collect();
        let minutes = history.front().map_or(0, |h| h.at.elapsed().as_secs() / 60);
        let t = self.theme.clone();
        egui::Window::new("Host health")
            .id(egui::Id::new("teryon-host"))
            .open(&mut open)
            .collapsible(false)
            .default_width(380.)
            .show(ctx, |ui| {
                if loads.is_empty() {
                    ui.label(
                        RichText::new("The daemon has not reported load yet.")
                            .color(color(t.muted)),
                    );
                    return;
                }
                let top = loads.iter().copied().fold(1_f32, f32::max);
                self.graph(
                    ui,
                    "LOAD (1 min)",
                    format!("{:.2}", loads.last().copied().unwrap_or(0.)),
                    &loads,
                    top,
                    color(t.accent),
                );
                if !mems.is_empty() {
                    ui.add_space(8.);
                    let now = format!("{}%", mems.last().copied().unwrap_or(0.) as u8);
                    self.graph(ui, "MEMORY", now, &mems, 100., color(t.success));
                }
                ui.add_space(6.);
                ui.label(
                    RichText::new(format!(
                        "Last {} min, one reading every few seconds. Kept in memory only.",
                        minutes.max(1)
                    ))
                    .size(11.)
                    .color(color(t.muted)),
                );
            });
        if !open {
            self.host_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;
    use crate::hoststats::HostStats;

    #[test]
    fn lines_scale_to_the_box_and_the_newest_reading_is_at_the_right_edge() {
        let r = Rect::from_min_size(Pos2::ZERO, Vec2::new(100., 50.));
        let p = polyline(r, &[0., 2., 4.], 4.);
        assert_eq!(p[0], Pos2::new(0., 50.));
        assert_eq!(p[2], Pos2::new(100., 0.));
        assert_eq!(p[1], Pos2::new(50., 25.));
        // A value past the top is clamped, not drawn outside the box.
        assert_eq!(polyline(r, &[9., 9.], 4.)[0].y, 0.);
    }

    #[test]
    fn readings_are_kept_spaced_out_and_the_window_draws_with_and_without_them() {
        let (mut app, _rx) = app_with_screen("");
        app.host_open = true;
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.host_ui(ctx));
        let stats = || {
            Some(HostStats {
                load1: 1.5,
                mem_used_pct: Some(40),
                thermal: None,
            })
        };
        app.link.daemon("arm64 · macos".into(), 5, stats());
        // A second reading right away is dropped: the history is one point per few seconds.
        app.link.daemon("arm64 · macos".into(), 5, stats());
        assert_eq!(app.link.history().len(), 1);
        let _ = ctx.run(Default::default(), |ctx| app.host_ui(ctx));
        assert!(app.host_open);
    }
}

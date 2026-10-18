//! Toolbar icons painted with lines, plus the icon buttons built from them.
use super::*;

/// Toolbar icons, painted with lines so they never depend on a font's glyph coverage.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Icon {
    Plus,
    List,
    Close,
    Zoom,
    Chevron,
    Spark,
    Layout(LayoutMode),
}
pub(super) fn paint_icon(p: &egui::Painter, area: Rect, icon: Icon, stroke: Stroke) {
    let c = area.center();
    let at = |x: f32, y: f32| c + Vec2::new(x, y);
    let line = |a: Pos2, b: Pos2| {
        p.line_segment([a, b], stroke);
    };
    let outline = |r: Rect, radius: u8| {
        p.rect_stroke(
            r,
            egui::CornerRadius::same(radius),
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    let body = Rect::from_center_size(c, Vec2::new(15., 12.));
    match icon {
        Icon::Plus => {
            line(at(-5., 0.), at(5., 0.));
            line(at(0., -5.), at(0., 5.));
        }
        Icon::List => {
            for y in [-4., 0., 4.] {
                line(at(-6., y), at(6., y));
            }
        }
        Icon::Spark => {
            // A four-pointed sparkle.
            for (a, b) in [
                ((0., -7.), (1.6, -1.6)),
                ((1.6, -1.6), (7., 0.)),
                ((7., 0.), (1.6, 1.6)),
                ((1.6, 1.6), (0., 7.)),
                ((0., 7.), (-1.6, 1.6)),
                ((-1.6, 1.6), (-7., 0.)),
                ((-7., 0.), (-1.6, -1.6)),
                ((-1.6, -1.6), (0., -7.)),
            ] {
                line(at(a.0, a.1), at(b.0, b.1));
            }
        }
        Icon::Chevron => {
            line(at(-4., -2.), at(0., 2.));
            line(at(0., 2.), at(4., -2.));
        }
        Icon::Close => {
            line(at(-4.5, -4.5), at(4.5, 4.5));
            line(at(-4.5, 4.5), at(4.5, -4.5));
        }
        Icon::Zoom => {
            for (sx, sy) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                line(at(sx * 6., sy * 6.), at(sx * 6., sy * 2.5));
                line(at(sx * 6., sy * 6.), at(sx * 2.5, sy * 6.));
            }
        }
        Icon::Layout(mode) => {
            outline(body, 3);
            if matches!(mode, LayoutMode::Vertical | LayoutMode::Grid) {
                line(at(-7.5, 0.), at(7.5, 0.));
            }
            if matches!(mode, LayoutMode::Horizontal | LayoutMode::Grid) {
                line(at(0., -6.), at(0., 6.));
            }
        }
    }
}
/// A 26x24 toolbar button showing `icon`; the tooltip carries the name and shortcut.
pub(super) fn icon_button(ui: &mut egui::Ui, t: &Theme, icon: Icon, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(26., 24.), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 6., color(mix(t.panel, t.foreground, 0.12)));
    }
    let ink = if resp.hovered() {
        t.foreground
    } else {
        mix(t.muted, t.foreground, 0.35)
    };
    paint_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(16.)),
        icon,
        Stroke::new(1.4_f32, color(ink)),
    );
    resp.on_hover_text(tip)
}
/// Four icon segments in a rounded tray; the selected layout is raised.
pub(super) fn layout_switch(ui: &mut egui::Ui, t: &Theme, current: &mut LayoutMode) {
    egui::Frame::new()
        .fill(color(mix(t.panel, t.foreground, 0.07)))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(2.)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.;
            for mode in [
                LayoutMode::Single,
                LayoutMode::Vertical,
                LayoutMode::Horizontal,
                LayoutMode::Grid,
            ] {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(28., 22.), Sense::click());
                if *current == mode {
                    ui.painter()
                        .rect_filled(rect, 6., color(mix(t.panel, t.foreground, 0.18)));
                } else if resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, 6., color(mix(t.panel, t.foreground, 0.1)));
                }
                let ink = if *current == mode {
                    t.foreground
                } else {
                    t.muted
                };
                paint_icon(
                    ui.painter(),
                    Rect::from_center_size(rect.center(), Vec2::splat(16.)),
                    Icon::Layout(mode),
                    Stroke::new(1.3_f32, color(ink)),
                );
                if resp
                    .on_hover_text(format!("{} layout", mode.name()))
                    .clicked()
                {
                    *current = mode;
                }
            }
        });
}

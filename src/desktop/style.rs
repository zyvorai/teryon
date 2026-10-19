//! Colors, fonts and the macOS-style widget chrome.
use super::*;

pub(super) fn color(c: ratatui::style::Color) -> Color32 {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        _ => Color32::WHITE,
    }
}
pub(super) fn indexed(n: u8) -> Color32 {
    let basic = [
        (0, 0, 0),
        (205, 49, 49),
        (13, 188, 121),
        (229, 229, 16),
        (36, 114, 200),
        (188, 63, 188),
        (17, 168, 205),
        (229, 229, 229),
        (102, 102, 102),
        (241, 76, 76),
        (35, 209, 139),
        (245, 245, 67),
        (59, 142, 234),
        (214, 112, 214),
        (41, 184, 219),
        (255, 255, 255),
    ];
    if n < 16 {
        let (r, g, b) = basic[n as usize];
        Color32::from_rgb(r, g, b)
    } else if n >= 232 {
        Color32::from_gray(8 + (n - 232) * 10)
    } else {
        let n = n - 16;
        let v = [0, 95, 135, 175, 215, 255];
        Color32::from_rgb(
            v[(n / 36) as usize],
            v[((n / 6) % 6) as usize],
            v[(n % 6) as usize],
        )
    }
}
pub(super) fn vt_color(c: vt100::Color, default: Color32, theme: &Theme) -> Color32 {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(n) if n < 16 => color(theme.ansi[usize::from(n)]),
        vt100::Color::Idx(n) => indexed(n),
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}
/// TERYON's own colors: the mark's gradient runs from `BRAND_CYAN` to `BRAND_BLUE`.
pub(super) const BRAND_BLUE: Color32 = Color32::from_rgb(0x0a, 0x84, 0xff);
pub(super) const BRAND_CYAN: Color32 = Color32::from_rgb(0x64, 0xd2, 0xff);

pub(super) const SYSTEM_FONTS: &str = "/System/Library/Fonts";

/// Font families for the UI: SF Pro at real weights (regular, `semibold`, `bold`), SF Pro
/// Rounded (`rounded`) and SF Mono, read through `read` at runtime and never bundled (only the small
/// Nerd Font symbols file below is bundled, so file and folder icons from eza, lsd or oh-my-posh render). SF is a
/// variable font, so each weight is the same file with a different `wght` axis value (applied
/// by the vendored epaint patch). Anything missing falls back to egui's built-in fonts.
pub(super) fn font_definitions(
    read: impl Fn(&str) -> Option<Vec<u8>>,
    user_mono: Option<Vec<u8>>,
) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let add = |fonts: &mut egui::FontDefinitions, key: &str, file: &str, weight: f32| {
        let bytes = read(&format!("{SYSTEM_FONTS}/{file}"))?;
        let data = egui::FontData::from_owned(bytes).variations(vec![(*b"wght", weight)]);
        fonts
            .font_data
            .insert(key.into(), std::sync::Arc::new(data));
        Some(key.to_string())
    };
    let base = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (name, key, file, weight) in [
        ("regular", "sf-400", "SFNS.ttf", 400.),
        ("semibold", "sf-600", "SFNS.ttf", 600.),
        ("bold", "sf-700", "SFNS.ttf", 700.),
        ("rounded", "sf-rounded", "SFNSRounded.ttf", 600.),
    ] {
        let mut list = base.clone();
        if let Some(key) = add(&mut fonts, key, file, weight) {
            list.insert(0, key);
        }
        if name == "regular" {
            fonts.families.insert(egui::FontFamily::Proportional, list);
        } else {
            fonts
                .families
                .insert(egui::FontFamily::Name(name.into()), list);
        }
    }
    if let Some(bytes) = user_mono {
        fonts.font_data.insert(
            "user-mono".into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "user-mono".into());
    }
    if let Some(key) = add(&mut fonts, "sf-mono", "SFNSMono.ttf", 400.) {
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, key);
    }
    // Last in the list, so it only supplies the private-use glyphs no other font has.
    fonts.font_data.insert(
        "nerd-symbols".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/SymbolsNerdFontMono-Regular.ttf"
        ))),
    );
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("nerd-symbols".into());
    // Menus put a small icon in front of each label.
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push("nerd-symbols".into());
    fonts
}

/// The normal fonts plus each of `picks` as its own family (`pick-0`, `pick-1`, ...), so the font
/// picker can draw every candidate in its own typeface at once.
pub(super) fn install_picker_fonts(ctx: &egui::Context, picks: &[Vec<u8>]) {
    let mut fonts = font_definitions(|path| std::fs::read(path).ok(), None);
    for (i, bytes) in picks.iter().enumerate() {
        let key = format!("pick-{i}");
        fonts.font_data.insert(
            key.clone(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes.clone())),
        );
        fonts.families.insert(
            egui::FontFamily::Name(key.clone().into()),
            vec![key, "nerd-symbols".into()],
        );
    }
    ctx.set_fonts(fonts);
}

pub(super) fn install_fonts(ctx: &egui::Context, family: Option<&str>) {
    let user = family.and_then(find_user_font);
    ctx.set_fonts(font_definitions(|path| std::fs::read(path).ok(), user));
}

/// Font bytes for a configured `font_family`: an absolute path, or a family/file name searched in
/// the system and user font folders (two levels deep).
fn find_user_font(family: &str) -> Option<Vec<u8>> {
    let path = std::path::Path::new(family);
    if path.is_absolute() {
        return std::fs::read(path).ok();
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut dirs: Vec<std::path::PathBuf> = vec![
        "/System/Library/Fonts".into(),
        "/Library/Fonts".into(),
        "/usr/share/fonts".into(),
    ];
    if let Some(h) = home {
        dirs.push(h.join("Library/Fonts"));
        dirs.push(h.join(".fonts"));
        dirs.push(h.join(".local/share/fonts"));
    }
    let mut found: Vec<std::path::PathBuf> = Vec::new();
    let mut stack: Vec<(std::path::PathBuf, u8)> = dirs.into_iter().map(|d| (d, 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && depth < 2 {
                stack.push((p, depth + 1));
            } else if p.is_file() {
                found.push(p);
            }
        }
    }
    let names: Vec<String> = found
        .iter()
        .map(|p| {
            p.file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned())
        })
        .collect();
    let at = uikit::match_font_file(family, &names)?;
    std::fs::read(&found[at]).ok()
}

/// Text in a named weight (`"semibold"`, `"bold"`, `"rounded"`); plain when the family is absent.
pub(super) fn weighted(
    ctx: &egui::Context,
    text: impl Into<String>,
    weight: &str,
    size: f32,
) -> RichText {
    let family = egui::FontFamily::Name(weight.into());
    let text = RichText::new(text);
    if ctx.fonts(|f| f.families()).contains(&family) {
        text.family(family).size(size)
    } else {
        text.size(size)
    }
}

/// Built-in Mac27 themes get macOS-style chrome (8px radii, filled buttons); others keep pills.
pub(super) fn is_native(t: &Theme) -> bool {
    t.name.starts_with("Mac27")
}

/// A macOS-style segmented control: a rounded tray with the selected segment raised.
pub(super) fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    t: &Theme,
    current: &mut T,
    items: &[(T, String)],
) {
    let tray = color(mix(t.panel, t.foreground, 0.07));
    let raised = color(mix(t.panel, t.foreground, 0.18));
    egui::Frame::new()
        .fill(tray)
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(2.)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            ui.spacing_mut().button_padding = Vec2::new(10., 2.);
            ui.spacing_mut().interact_size.y = 22.;
            ui.horizontal(|ui| {
                for (value, label) in items {
                    let selected = *current == *value;
                    let text = if selected {
                        weighted(ui.ctx(), label.clone(), "semibold", 13.)
                    } else {
                        RichText::new(label)
                    };
                    let button = egui::Button::new(text.color(color(t.foreground)))
                        .fill(if selected {
                            raised
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::NONE)
                        .corner_radius(egui::CornerRadius::same(7));
                    if ui.add(button).clicked() {
                        *current = *value;
                    }
                }
            });
        });
}

/// Chrome: pill buttons with hairlines (apple themes) or macOS-style rounded rects (Mac27).
pub(super) fn apply_style(ctx: &egui::Context, t: &Theme) {
    ctx.style_mut(|s| {
        s.text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(13.));
        s.text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(13.));
        s.text_styles
            .insert(egui::TextStyle::Small, FontId::proportional(11.));
        s.text_styles
            .insert(egui::TextStyle::Heading, FontId::proportional(20.));
        s.spacing.button_padding = Vec2::new(10., 4.);
        s.spacing.item_spacing = Vec2::new(6., 6.);
        s.spacing.interact_size.y = 24.;
    });
    let light = if let ratatui::style::Color::Rgb(r, g, b) = t.background {
        u16::from(r) + u16::from(g) + u16::from(b) > 450
    } else {
        false
    };
    let mut v = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let hairline = Stroke::new(1_f32, color(t.border));
    let strong = Stroke::new(1_f32, color(mix(t.border, t.foreground, 0.25)));
    let hover = color(mix(t.panel, t.foreground, 0.08));
    let pressed = color(mix(t.panel, t.foreground, 0.14));
    let rail = color(mix(t.panel, t.foreground, 0.16));
    v.panel_fill = color(t.panel);
    v.window_fill = color(mix(t.panel, t.foreground, 0.06));
    // Floating surfaces share one radius, a lighter edge and a soft shadow.
    v.window_stroke = Stroke::new(1_f32, color(mix(t.border, t.foreground, 0.1)));
    v.window_corner_radius = egui::CornerRadius::same(12);
    v.menu_corner_radius = egui::CornerRadius::same(12);
    v.override_text_color = Some(color(t.foreground));
    v.selection.bg_fill = color(t.accent).gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(2_f32, color(t.primary));
    v.extreme_bg_color = color(t.background);
    let native = is_native(t);
    let pill = egui::CornerRadius::same(if native { 8 } else { 255 });
    if native {
        v.window_corner_radius = egui::CornerRadius::same(12);
        v.menu_corner_radius = egui::CornerRadius::same(12);
        v.selection.bg_fill = color(t.accent).gamma_multiply(0.30);
        let shadow = egui::Shadow {
            offset: [0, 12],
            blur: 36,
            spread: 2,
            color: Color32::from_black_alpha(if light { 44 } else { 140 }),
        };
        v.window_shadow = shadow;
        v.popup_shadow = shadow;
    }
    v.widgets.noninteractive.bg_fill = color(t.panel);
    v.widgets.noninteractive.bg_stroke = hairline;
    v.widgets.noninteractive.corner_radius = egui::CornerRadius::same(14);
    // Mac27 buttons are filled at rest with a faint outline; apple themes are transparent pills.
    let (rest, strong) = if native {
        (
            color(mix(t.panel, t.foreground, 0.09)),
            Stroke::new(1_f32, color(mix(t.panel, t.foreground, 0.06))),
        )
    } else {
        (Color32::TRANSPARENT, strong)
    };
    for (w, fill, stroke) in [
        (&mut v.widgets.inactive, rest, strong),
        (&mut v.widgets.hovered, hover, strong),
        (&mut v.widgets.active, pressed, strong),
        (&mut v.widgets.open, hover, strong),
    ] {
        // weak_bg_fill paints buttons (transparent at rest); bg_fill paints slider rails and
        // other trays, which need a visible tint.
        w.bg_fill = if fill == Color32::TRANSPARENT || native {
            rail
        } else {
            fill
        };
        w.weak_bg_fill = fill;
        w.bg_stroke = stroke;
        w.corner_radius = pill;
        w.expansion = 0.;
    }
    ctx.set_visuals(v);
}

/// The one primary action of a view: solid blue pill with white text.
pub(super) fn primary_button(ui: &mut egui::Ui, t: &Theme, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE))
            .fill(color(t.primary))
            .stroke(Stroke::NONE)
            .corner_radius(egui::CornerRadius::same(if is_native(t) { 8 } else { 255 })),
    )
}

/// The TERYON mark (a T and the cursor waiting beside it, from `docs/brand/teryon-icon.svg`)
/// painted into `rect`; the geometry is the icon's 1024-unit design scaled to the rectangle.
pub(super) fn paint_mark(painter: &egui::Painter, rect: Rect) {
    let k = rect.width().min(rect.height()) / 1024.;
    let at = |x: f32, y: f32, w: f32, h: f32, r: f32, c: Color32| {
        let min = rect.min + Vec2::new(x, y) * k;
        painter.rect_filled(
            Rect::from_min_size(min, Vec2::new(w, h) * k),
            egui::CornerRadius::same((r * k).round().clamp(0., 255.) as u8),
            c,
        );
    };
    let bar = mix_color(BRAND_CYAN, BRAND_BLUE, 0.35);
    at(262., 246., 440., 116., 58., bar);
    at(
        444.,
        304.,
        116.,
        486.,
        58.,
        mix_color(BRAND_CYAN, BRAND_BLUE, 0.7),
    );
    at(
        614.,
        674.,
        116.,
        116.,
        30.,
        Color32::from_rgb(0xf5, 0xf5, 0xf7),
    );
}
fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}
/// The mark at 20px, followed by the product name.
pub(super) fn brand_mark(ui: &mut egui::Ui, t: &Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.), Sense::hover());
    paint_mark(ui.painter(), rect);
    ui.label(weighted(ui.ctx(), "TERYON", "bold", 14.).color(color(t.foreground)));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nerd_symbols_are_the_last_monospace_fallback() {
        let fonts = font_definitions(|_| None, None);
        let list = &fonts.families[&egui::FontFamily::Monospace];
        assert_eq!(list.last().map(String::as_str), Some("nerd-symbols"));
        let ui_list = &fonts.families[&egui::FontFamily::Proportional];
        assert_eq!(ui_list.last().map(String::as_str), Some("nerd-symbols"));
        let ctx = egui::Context::default();
        ctx.set_fonts(font_definitions(|_| None, None));
        let font = FontId::monospace(14.);
        // Fonts take effect at the start of a frame, so measure inside a second one.
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let width = |c: char| ctx.fonts(|f| f.glyph_width(&font, c));
            let m = width('M');
            // Folder, file and Rust icons (as printed by `eza --icons`) take up a cell, not zero.
            for icon in ['\u{f07b}', '\u{f15b}', '\u{e7a8}'] {
                assert!(width(icon) > m * 0.5, "{icon:?} is missing");
                assert!(width(icon) < m * 2.5);
            }
        });
    }
}

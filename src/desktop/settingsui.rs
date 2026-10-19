//! ⌘, : a settings window for the common `[desktop]` options. Changes apply live and are written
//! to `config.toml` with comments kept; the file stays the source of truth.
use super::*;
use crate::config::DesktopConfig;
use crate::configedit::{self, Val};

#[derive(Clone, Copy)]
pub(super) enum Ctl {
    Bool,
    Float(f32, f32),
    Int(u32, u32),
    Choice(&'static [&'static str]),
}
pub(super) struct Setting {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub ctl: Ctl,
    /// The running app cannot apply it; it takes effect the next time the app starts.
    pub restart: bool,
}
const fn s(key: &'static str, label: &'static str, help: &'static str, ctl: Ctl) -> Setting {
    Setting { key, label, help, ctl, restart: false }
}
const fn r(key: &'static str, label: &'static str, help: &'static str, ctl: Ctl) -> Setting {
    Setting { key, label, help, ctl, restart: true }
}

pub(super) const SECTIONS: &[(&str, &[Setting])] = &[
    (
        "Look",
        &[
            s("window_opacity", "Window opacity", "How see-through the window is (1 = solid).", Ctl::Float(0.3, 1.0)),
            s("unfocused_opacity", "Dim unfocused panes", "Brightness of panes that do not have focus.", Ctl::Float(0.2, 1.0)),
            r("window_blur", "Background blur", "Blur behind a see-through window (macOS).", Ctl::Int(0, 100)),
            s("corner_radius", "Rounded corners", "Window corner radius in points (bare window).", Ctl::Float(0., 24.)),
            s("background_noise", "Film grain", "Fine grain over the background, 0 to 100.", Ctl::Float(0., 100.)),
            s("pane_inset", "Space around panes", "Gap between the window edge and the panes.", Ctl::Float(0., 32.)),
        ],
    ),
    (
        "Text",
        &[
            s("font_size", "Font size", "Terminal text size in points.", Ctl::Float(8., 36.)),
            s("line_height", "Line height", "Multiple of the font size.", Ctl::Float(1., 2.5)),
            s("padding", "Text padding", "Space between the text and the pane edge.", Ctl::Float(0., 48.)),
            s("text_gamma", "Text weight", "Lower is heavier text, higher is thinner.", Ctl::Float(0.3, 1.0)),
            s("cursor_style", "Cursor shape", "Block, bar or underline.", Ctl::Choice(&["block", "bar", "underline"])),
            s("cursor_blink", "Blinking cursor", "Fade the cursor in and out.", Ctl::Bool),
        ],
    ),
    (
        "Window",
        &[
            r("chrome", "Window frame", "Full has a title bar and tabs; none is a bare terminal.", Ctl::Choice(&["full", "none"])),
            s("tab_bar", "Tab strip", "A tab strip along the bottom of a bare window.", Ctl::Choice(&["off", "bottom"])),
            s("pane_titles", "Pane title bars", "When each pane shows its title bar.", Ctl::Choice(&["auto", "always", "never"])),
            s("scrollbar", "Scrollbar", "Only while scrolled back, or always.", Ctl::Choice(&["auto", "always"])),
            s("status_line", "Status line", "The muted line under the panes.", Ctl::Bool),
            s("file_browser", "File browser at start", "Open the file sidebar when the app starts.", Ctl::Bool),
            r("maximized", "Open maximized", "New windows start maximized.", Ctl::Bool),
            r("fullscreen", "Open in full screen", "New windows start in native full screen.", Ctl::Bool),
            s("confirm_quit", "Confirm before closing", "Ask before closing the window (shells keep running).", Ctl::Bool),
        ],
    ),
    (
        "Behavior",
        &[
            s("copy_on_select", "Copy on select", "Copy as soon as you finish selecting.", Ctl::Bool),
            s("scrollback_lines", "Scrollback", "Lines of history shown when scrolling back.", Ctl::Int(1000, 100_000)),
            s("command_gutter", "Command gutter", "A thin bar beside each command (needs shell integration).", Ctl::Bool),
            s("fresh_window", "Fresh shell per window", "Each window starts its own shell instead of the shared one.", Ctl::Bool),
        ],
    ),
    (
        "Alerts",
        &[
            s("notify_after_seconds", "Long command alert", "Notify when a command ran this long (0 = off).", Ctl::Int(0, 3600)),
            s("agent_notifications", "AI agent alerts", "Notify when an agent needs you or finishes.", Ctl::Bool),
            s("agent_quick_approve", "Approve / Deny buttons", "Answer an agent's prompt from the Agents overview.", Ctl::Bool),
        ],
    ),
];

macro_rules! accessors {
    (bool: [$($b:ident),*]; float: [$($f:ident),*]; uint: [$($u:ident),*]; choice: [$($c:ident),*]) => {
        /// The current value of option `key`.
        pub(super) fn get(c: &DesktopConfig, key: &str) -> Option<Val> {
            match key {
                $( stringify!($b) => Some(Val::Bool(c.$b)), )*
                $( stringify!($f) => Some(Val::Float(f64::from(c.$f))), )*
                $( stringify!($u) => Some(Val::Int(c.$u as i64)), )*
                $( stringify!($c) => serde_json::to_value(c.$c).ok()
                    .and_then(|v| v.as_str().map(|s| Val::Str(s.to_string()))), )*
                _ => None,
            }
        }
        /// Set option `key` to `v`; false when the key or the value's type is wrong.
        pub(super) fn set(c: &mut DesktopConfig, key: &str, v: &Val) -> bool {
            match (key, v) {
                $( (stringify!($b), Val::Bool(x)) => { c.$b = *x; true } )*
                $( (stringify!($f), Val::Float(x)) => { c.$f = *x as f32; true } )*
                $( (stringify!($u), Val::Int(x)) => { c.$u = (*x).max(0) as _; true } )*
                $( (stringify!($c), Val::Str(x)) => {
                    match serde_json::from_value(serde_json::Value::String(x.clone())) {
                        Ok(parsed) => { c.$c = parsed; true }
                        Err(_) => false,
                    }
                } )*
                _ => false,
            }
        }
    };
}
accessors! {
    bool: [cursor_blink, status_line, file_browser, maximized, fullscreen, confirm_quit,
           copy_on_select, command_gutter, fresh_window, agent_notifications, agent_quick_approve];
    float: [window_opacity, unfocused_opacity, corner_radius, background_noise, pane_inset,
            font_size, line_height, padding, text_gamma];
    uint: [window_blur, scrollback_lines, notify_after_seconds];
    choice: [cursor_style, chrome, tab_bar, pane_titles, scrollbar]
}

#[derive(Default)]
pub(super) struct SettingsWindow {
    pub(super) open: bool,
    section: usize,
    query: String,
}

/// What a control reported this frame.
struct Edit {
    key: &'static str,
    val: Val,
    /// The user let go (or typed a value): write it to the file, not only apply it.
    commit: bool,
}

impl Desktop {
    pub(super) fn open_settings(&mut self) {
        self.settings_win.open = true;
        self.palette = false;
    }

    /// Apply a change now and, when `commit`, write it to `config.toml`.
    fn change_setting(&mut self, key: &str, val: &Val, commit: bool) {
        let mut cfg = self.cfg.clone();
        if !set(&mut cfg, key, val) {
            return;
        }
        if cfg.validate().is_err() {
            return;
        }
        self.apply_config(cfg);
        if commit {
            self.persist_setting(key, Some(val));
        }
    }
    /// Write `key` (or remove it with `None`) in the config file and remember its new stamp, so the file
    /// watcher does not announce our own edit.
    fn persist_setting(&mut self, key: &str, val: Option<&Val>) {
        let key = key.to_string();
        let val = val.cloned();
        let result = configedit::apply(|text| match &val {
            Some(v) => configedit::set_desktop(text, &key, v),
            None => configedit::unset_desktop(text, &key),
        });
        match result {
            Ok(()) => self.config_mtime = Desktop::config_stamp(),
            Err(e) => self.toast(format!("Could not save settings: {e:#}")),
        }
    }
    fn reset_setting(&mut self, key: &str) {
        let default = get(&DesktopConfig::default(), key);
        if let Some(v) = default {
            self.change_setting(key, &v, false);
            self.persist_setting(key, None);
        }
    }

    pub(super) fn settings_ui(&mut self, ctx: &egui::Context) {
        let t = self.theme.clone();
        let screen = ctx.screen_rect();
        let width = (screen.width() - 80.).clamp(360., 720.);
        let list_h = (screen.height() - 270.).clamp(160., 560.);
        let defaults = DesktopConfig::default();
        let mut edit: Option<Edit> = None;
        let mut reset: Option<&'static str> = None;
        let mut next: Option<&str> = None;
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        let query = self.settings_win.query.to_lowercase();
        egui::Window::new("settings")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0., 40.])
            .fixed_size([width, 0.])
            .frame(
                egui::Frame::window(&ctx.style())
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Settings").size(20.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕").clicked() {
                            close = true;
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings_win.query)
                                .hint_text("Search settings…")
                                .desired_width(190.),
                        );
                    });
                });
                ui.add_space(8.);
                if query.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for (n, (name, _)) in SECTIONS.iter().enumerate() {
                            let on = self.settings_win.section == n;
                            let chip = egui::Button::new(RichText::new(*name).size(13.).color(if on {
                                color(t.background)
                            } else {
                                color(t.foreground)
                            }))
                            .fill(if on { color(t.accent) } else { color(mix(t.panel, t.foreground, 0.08)) })
                            .corner_radius(egui::CornerRadius::same(14))
                            .min_size(Vec2::new(60., 28.));
                            if ui.add(chip).clicked() {
                                self.settings_win.section = n;
                            }
                        }
                    });
                    ui.add_space(8.);
                }
                egui::ScrollArea::vertical().max_height(list_h).show(ui, |ui| {
                    let rows: Vec<&'static Setting> = if query.is_empty() {
                        SECTIONS[self.settings_win.section.min(SECTIONS.len() - 1)].1.iter().collect()
                    } else {
                        SECTIONS
                            .iter()
                            .flat_map(|(_, list)| list.iter())
                            .filter(|st| {
                                st.label.to_lowercase().contains(&query)
                                    || st.help.to_lowercase().contains(&query)
                                    || st.key.contains(&query)
                            })
                            .collect()
                    };
                    egui::Grid::new("settings-grid")
                        .num_columns(3)
                        .spacing([14., 14.])
                        .min_col_width(0.)
                        .show(ui, |ui| {
                            for st in rows {
                                let Some(current) = get(&self.cfg, st.key) else { continue };
                                ui.vertical(|ui| {
                                    ui.set_min_width(width * 0.46);
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(st.label).size(14.));
                                        if st.restart {
                                            ui.label(RichText::new("restart").size(10.).color(color(t.warning)));
                                        }
                                    });
                                    ui.label(RichText::new(st.help).size(11.).color(color(t.muted)));
                                });
                                if let Some(e) = control(ui, st, &current, &t) {
                                    edit = Some(e);
                                }
                                if get(&defaults, st.key).is_some_and(|d| d != current)
                                    && ui.small_button("↺").on_hover_text("Back to the default").clicked()
                                {
                                    reset = Some(st.key);
                                }
                                ui.end_row();
                            }
                        });
                });
                ui.add_space(10.);
                ui.horizontal(|ui| {
                    if ui.button("🎨 Theme…").clicked() {
                        next = Some("theme");
                    }
                    if ui.button("𝐀 Font…").clicked() {
                        next = Some("font");
                    }
                    if ui.button("⌨ Shortcuts…").clicked() {
                        next = Some("keys");
                    }
                    if ui.button("Open config file").clicked() {
                        next = Some("file");
                    }
                });
            });
        if let Some(e) = edit {
            self.change_setting(e.key, &e.val, e.commit);
        }
        if let Some(key) = reset {
            self.reset_setting(key);
        }
        match next {
            Some("theme") => {
                self.settings_win.open = false;
                self.open_theme_picker();
            }
            Some("font") => {
                self.settings_win.open = false;
                self.open_font_picker();
            }
            Some("keys") => {
                self.settings_win.open = false;
                self.open_key_editor();
            }
            Some("file") => {
                if let Ok(p) = config::path() {
                    crate::uikit::open_in_editor(&p);
                }
            }
            _ => {}
        }
        if close {
            self.settings_win.open = false;
        }
    }
}

/// The control for one setting; returns the edit the user made this frame, if any.
fn control(ui: &mut egui::Ui, st: &'static Setting, current: &Val, t: &Theme) -> Option<Edit> {
    let mut edit: Option<Edit> = None;
    match (st.ctl, current) {
        (Ctl::Bool, Val::Bool(b)) => {
            let mut on = *b;
            if ui.add(toggle(&mut on, t)).changed() {
                edit = Some(Edit { key: st.key, val: Val::Bool(on), commit: true });
            }
        }
        (Ctl::Float(lo, hi), Val::Float(f)) => {
            let mut v = *f as f32;
            let step = if hi - lo > 20. { 1.0 } else { 0.01 };
            let resp = ui.add(
                egui::Slider::new(&mut v, lo..=hi)
                    .step_by(step)
                    .fixed_decimals(if step < 0.5 { 2 } else { 0 }),
            );
            if resp.changed() {
                edit = Some(Edit {
                    key: st.key,
                    val: Val::Float(f64::from(v)),
                    commit: !resp.dragged(),
                });
            } else if resp.drag_stopped() {
                edit = Some(Edit { key: st.key, val: Val::Float(f64::from(v)), commit: true });
            }
        }
        (Ctl::Int(lo, hi), Val::Int(n)) => {
            let mut v = (*n).clamp(i64::from(lo), i64::from(hi)) as i32;
            let resp = ui.add(egui::Slider::new(&mut v, lo as i32..=hi as i32));
            if resp.changed() {
                edit = Some(Edit { key: st.key, val: Val::Int(i64::from(v)), commit: !resp.dragged() });
            } else if resp.drag_stopped() {
                edit = Some(Edit { key: st.key, val: Val::Int(i64::from(v)), commit: true });
            }
        }
        (Ctl::Choice(options), Val::Str(cur)) => {
            ui.horizontal(|ui| {
                for o in options {
                    if ui.selectable_label(cur == o, *o).clicked() && cur != o {
                        edit = Some(Edit { key: st.key, val: Val::Str((*o).into()), commit: true });
                    }
                }
            });
        }
        _ => {
            ui.label("");
        }
    }

    edit
}

impl Desktop {
    /// A banner for a config file that cannot be loaded: what is wrong, and a way to fix it.
    pub(super) fn config_error_ui(&mut self, ctx: &egui::Context) {
        let Some(message) = self.config_error.clone() else {
            return;
        };
        let t = self.theme.clone();
        let mut dismiss = false;
        let mut retry = false;
        let mut open_file = false;
        let shown: String = message.lines().take(6).collect::<Vec<_>>().join("\n");
        egui::Window::new("config-error")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0., 12.])
            .fixed_size([(ctx.screen_rect().width() - 60.).clamp(320., 640.), 0.])
            .frame(
                egui::Frame::window(&ctx.style())
                    .corner_radius(egui::CornerRadius::same(12))
                    .stroke(Stroke::new(1.5_f32, color(t.warning)))
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("⚠ config.toml has a problem — keeping your previous settings")
                        .size(14.)
                        .strong()
                        .color(color(t.warning)),
                );
                ui.add_space(6.);
                ui.label(RichText::new(shown).monospace().size(11.5).color(color(t.foreground)));
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    if ui.button("Open config file").clicked() {
                        open_file = true;
                    }
                    if ui.button("Try again").clicked() {
                        retry = true;
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }
                });
            });
        if open_file {
            if let Ok(p) = config::path() {
                crate::uikit::open_in_editor(&p);
            }
        }
        if retry {
            self.reload_config();
        }
        if dismiss {
            self.config_error = None;
        }
    }
}

fn find(key: &str) -> Option<&'static Setting> {
    SECTIONS.iter().flat_map(|(_, l)| l.iter()).find(|s| s.key == key)
}

impl Desktop {
    /// A small floating panel with the settings people tweak by eye: opacity, dimming, size and spacing.
    pub(super) fn appearance_ui(&mut self, ctx: &egui::Context) {
        let t = self.theme.clone();
        let mut edit: Option<Edit> = None;
        let mut next: Option<&str> = None;
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        egui::Window::new("appearance")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, [-16., 48.])
            .fixed_size([320., 0.])
            .frame(
                egui::Frame::window(&ctx.style())
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Window look").size(16.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(8.);
                for key in ["window_opacity", "unfocused_opacity", "font_size", "line_height"] {
                    let (Some(st), Some(cur)) = (find(key), get(&self.cfg, key)) else {
                        continue;
                    };
                    ui.label(RichText::new(st.label).size(12.).color(color(t.muted)));
                    if let Some(e) = control(ui, st, &cur, &t) {
                        edit = Some(e);
                    }
                    ui.add_space(4.);
                }
                ui.add_space(6.);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Theme…").clicked() {
                        next = Some("theme");
                    }
                    if ui.button("Font…").clicked() {
                        next = Some("font");
                    }
                    if ui.button("All settings…").clicked() {
                        next = Some("settings");
                    }
                });
            });
        if let Some(e) = edit {
            self.change_setting(e.key, &e.val, e.commit);
        }
        match next {
            Some("theme") => {
                self.appearance_open = false;
                self.open_theme_picker();
            }
            Some("font") => {
                self.appearance_open = false;
                self.open_font_picker();
            }
            Some("settings") => {
                self.appearance_open = false;
                self.open_settings();
            }
            _ => {}
        }
        if close {
            self.appearance_open = false;
        }
    }
}

/// A rounded on/off switch.
pub(super) fn toggle<'a>(on: &'a mut bool, t: &Theme) -> impl egui::Widget + 'a {
    let (accent, off) = (color(t.accent), color(mix(t.panel, t.foreground, 0.2)));
    move |ui: &mut egui::Ui| {
        let size = Vec2::new(40., 22.);
        let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
        if resp.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        let k = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(11), off.lerp_to_gamma(accent, k));
        ui.painter().circle_filled(
            Pos2::new(rect.left() + 11. + k * 18., rect.center().y),
            8.,
            Color32::WHITE,
        );
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_reads_writes_and_stays_valid_at_both_ends() {
        let base = DesktopConfig::default();
        for (_, list) in SECTIONS {
            for st in *list {
                let current = get(&base, st.key).unwrap_or_else(|| panic!("no getter for {}", st.key));
                let mut c = base.clone();
                assert!(set(&mut c, st.key, &current), "{}", st.key);
                assert_eq!(c, base, "writing the current value changes nothing: {}", st.key);
                match st.ctl {
                    Ctl::Float(lo, hi) => {
                        for v in [lo, hi] {
                            let mut c = base.clone();
                            assert!(set(&mut c, st.key, &Val::Float(f64::from(v))));
                            c.validate().unwrap_or_else(|e| panic!("{} = {v}: {e}", st.key));
                        }
                    }
                    Ctl::Int(lo, hi) => {
                        for v in [lo, hi] {
                            let mut c = base.clone();
                            assert!(set(&mut c, st.key, &Val::Int(i64::from(v))));
                            c.validate().unwrap_or_else(|e| panic!("{} = {v}: {e}", st.key));
                        }
                    }
                    Ctl::Choice(options) => {
                        for o in options {
                            let mut c = base.clone();
                            assert!(set(&mut c, st.key, &Val::Str((*o).into())), "{} = {o}", st.key);
                            assert_eq!(get(&c, st.key), Some(Val::Str((*o).into())));
                        }
                    }
                    Ctl::Bool => {
                        let mut c = base.clone();
                        assert!(set(&mut c, st.key, &Val::Bool(true)));
                    }
                }
            }
        }
        assert!(!set(&mut base.clone(), "nope", &Val::Bool(true)));
        assert!(!set(&mut base.clone(), "cursor_style", &Val::Str("wavy".into())));
    }
    #[test]
    fn every_setting_has_a_documented_key_in_the_config() {
        // A typo in a key would write an option the config rejects (`deny_unknown_fields`).
        for (_, list) in SECTIONS {
            for st in *list {
                let text = configedit::set_desktop("", st.key, &get(&DesktopConfig::default(), st.key).unwrap()).unwrap();
                configedit::validate(&text).unwrap_or_else(|e| panic!("{}: {e}", st.key));
            }
        }
    }
    #[test]
    fn changing_a_setting_applies_it_and_a_bad_value_is_ignored() {
        let (mut app, _rx) = crate::desktop::tests::app_with_screen("");
        app.change_setting("window_opacity", &Val::Float(0.8), false);
        assert!((app.cfg.window_opacity - 0.8).abs() < 1e-6);
        assert!((app.unfocused_dim - (1. - app.cfg.unfocused_opacity)).abs() < 1e-6);
        app.change_setting("font_size", &Val::Float(500.), false);
        assert!(app.cfg.font_size < 100., "an out-of-range value never reaches the config");
        app.change_setting("font_size", &Val::Float(18.), false);
        assert_eq!(app.font_size, 18.);
    }
    #[test]
    fn a_config_error_shows_a_banner_that_can_be_dismissed() {
        let (mut app, _rx) = crate::desktop::tests::app_with_screen("");
        let ctx = egui::Context::default();
        app.config_error = Some("TOML parse error at line 3, column 1\n  |\n3 | oops\n".into());
        let out = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
        assert!(!out.shapes.is_empty());
        assert!(app.config_error.is_some());
        app.config_error = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
    }
    #[test]
    fn the_window_renders_in_every_section_and_when_searching() {
        let (mut app, _rx) = crate::desktop::tests::app_with_screen("");
        let ctx = egui::Context::default();
        app.open_settings();
        for n in 0..SECTIONS.len() {
            app.settings_win.section = n;
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
        }
        app.settings_win.query = "font".into();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
        assert!(app.settings_win.open);
    }
}

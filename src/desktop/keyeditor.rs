//! Keyboard shortcuts: every action with its key, rebound by pressing the new one. Saved in `[desktop.keys]`.
use super::*;
use crate::configedit;
use crate::keymap::{canon_key, Chord, Keymap, Mods, ACTION_NAMES};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct KeyEditor {
    pub(super) open: bool,
    query: String,
    /// The action waiting for a key press.
    capturing: Option<String>,
    message: String,
}

/// The chord for a pressed key, or why it cannot be used: a bare letter would steal typing from the shell.
pub(super) fn chord_for(key_name: &str, mods: Mods) -> Result<Chord, &'static str> {
    let key = canon_key(key_name).ok_or("That key cannot be bound")?;
    let function_key =
        key.len() >= 2 && key.starts_with('f') && key[1..].chars().all(|c| c.is_ascii_digit());
    if !(mods.cmd || mods.ctrl || mods.alt || function_key) {
        return Err("Hold ⌘, ⌃ or ⌥ too, so typing still reaches the shell");
    }
    Ok(Chord { mods, key })
}

/// `keys` with `action` bound to `chord` (`None` unbinds it), or the reason it cannot be.
pub(super) fn rebind(
    keys: &BTreeMap<String, String>,
    action: &str,
    chord: Option<&Chord>,
) -> Result<BTreeMap<String, String>, String> {
    if !ACTION_NAMES.contains(&action) {
        return Err(format!("unknown action {action}"));
    }
    let mut next = keys.clone();
    next.insert(
        action.to_string(),
        chord.map_or_else(|| "none".to_string(), Chord::text),
    );
    let (_, problems) = Keymap::build(&next);
    match problems.first() {
        Some(p) => Err(p.clone()),
        None => Ok(next),
    }
}

impl Desktop {
    pub(super) fn open_key_editor(&mut self) {
        self.key_editor = KeyEditor {
            open: true,
            ..Default::default()
        };
        self.palette = false;
    }

    /// Bind (or unbind with `None`, or restore the default with `reset`) and save.
    fn save_binding(&mut self, action: &str, chord: Option<Chord>, reset: bool) {
        let next = if reset {
            let mut k = self.cfg.keys.clone();
            k.remove(action);
            Ok(k)
        } else {
            rebind(&self.cfg.keys, action, chord.as_ref())
        };
        let keys = match next {
            Ok(k) => k,
            Err(e) => {
                self.key_editor.message = e;
                return;
            }
        };
        let (a, c) = (action.to_string(), chord.as_ref().map(Chord::text));
        let saved = configedit::apply(|text| {
            if reset {
                configedit::unset_key(text, &a)
            } else {
                configedit::set_key(text, &a, c.as_deref().unwrap_or("none"))
            }
        });
        match saved {
            Ok(()) => {
                self.config_mtime = Desktop::config_stamp();
                let mut cfg = self.cfg.clone();
                cfg.keys = keys;
                self.apply_config(cfg);
                self.key_editor.message.clear();
            }
            Err(e) => self.key_editor.message = format!("Could not save: {e:#}"),
        }
    }

    pub(super) fn key_editor_ui(&mut self, ctx: &egui::Context) {
        let t = self.theme.clone();
        // While waiting for a key, the first chord pressed is the new binding.
        if let Some(action) = self.key_editor.capturing.clone() {
            let pressed = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((
                        key.name().to_string(),
                        chord_mods(modifiers),
                        *key == Key::Escape,
                    )),
                    _ => None,
                })
            });
            if let Some((name, mods, escape)) = pressed {
                if escape {
                    self.key_editor.capturing = None;
                    self.key_editor.message.clear();
                } else {
                    match chord_for(&name, mods) {
                        Ok(chord) => {
                            self.save_binding(&action, Some(chord), false);
                            if self.key_editor.message.is_empty() {
                                self.key_editor.capturing = None;
                            }
                        }
                        Err(why) => self.key_editor.message = why.to_string(),
                    }
                }
            }
        }
        let list: Vec<(String, String, String, bool)> = actions(&self.keymap, &[], &[], &[])
            .into_iter()
            .filter_map(|a| {
                let name = a
                    .kind
                    .name()
                    .filter(|n| ACTION_NAMES.contains(&n.as_str()))?;
                let overridden = self.cfg.keys.contains_key(&name);
                Some((name, a.title, a.hint, overridden))
            })
            .collect();
        let query = self.key_editor.query.clone();
        let rows: Vec<&(String, String, String, bool)> = list
            .iter()
            .filter(|(name, title, ..)| {
                uikit::fuzzy_score(&query, &format!("{title} {name}")).is_some()
            })
            .collect();
        let screen = ctx.screen_rect();
        let width = (screen.width() - 80.).clamp(360., 720.);
        let list_h = (screen.height() - 250.).clamp(160., 560.);
        let mut start: Option<String> = None;
        let mut clear: Option<String> = None;
        let mut reset: Option<String> = None;
        let capturing = self.key_editor.capturing.clone();
        let mut close = capturing.is_none() && ctx.input(|i| i.key_pressed(Key::Escape));
        egui::Window::new("shortcuts")
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
                    ui.label(RichText::new("Keyboard shortcuts").size(20.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(8.);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.key_editor.query)
                        .hint_text("Search actions…")
                        .font(FontId::proportional(15.))
                        .margin(egui::Margin::symmetric(10, 7))
                        .desired_width(f32::INFINITY),
                );
                if capturing.is_none() {
                    edit.request_focus();
                }
                ui.add_space(10.);
                egui::ScrollArea::vertical()
                    .max_height(list_h)
                    .show(ui, |ui| {
                        for (name, title, hint, overridden) in &rows {
                            let waiting = capturing.as_deref() == Some(name.as_str());
                            ui.horizontal(|ui| {
                                ui.set_min_height(30.);
                                ui.label(RichText::new(title).size(14.));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if *overridden
                                            && ui
                                                .small_button("↺")
                                                .on_hover_text("Back to the default")
                                                .clicked()
                                        {
                                            reset = Some(name.clone());
                                        }
                                        if !hint.is_empty()
                                            && ui
                                                .small_button("✕")
                                                .on_hover_text("Remove this shortcut")
                                                .clicked()
                                        {
                                            clear = Some(name.clone());
                                        }
                                        let label = if waiting {
                                            "Press the new keys…".to_string()
                                        } else {
                                            "Change".to_string()
                                        };
                                        if ui
                                            .add(
                                                egui::Button::new(label)
                                                    .min_size(Vec2::new(70., 24.)),
                                            )
                                            .clicked()
                                        {
                                            start = Some(name.clone());
                                        }
                                        let cap = if hint.is_empty() {
                                            "—".to_string()
                                        } else {
                                            hint.clone()
                                        };
                                        egui::Frame::new()
                                            .fill(color(mix(t.panel, t.foreground, 0.1)))
                                            .corner_radius(egui::CornerRadius::same(6))
                                            .inner_margin(egui::Margin::symmetric(8, 3))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    RichText::new(cap)
                                                        .size(12.)
                                                        .color(color(t.muted)),
                                                );
                                            });
                                    },
                                );
                            });
                        }
                        if rows.is_empty() {
                            ui.label(
                                RichText::new("No action matches.")
                                    .size(13.)
                                    .color(color(t.muted)),
                            );
                        }
                    });
                ui.add_space(8.);
                let (text, c) = if !self.key_editor.message.is_empty() {
                    (self.key_editor.message.clone(), color(t.warning))
                } else if capturing.is_some() {
                    (
                        "Press the new shortcut (with ⌘, ⌃ or ⌥) · Esc cancels".to_string(),
                        color(t.accent),
                    )
                } else {
                    (
                        "Change · ✕ remove · ↺ back to the default · saved in config.toml"
                            .to_string(),
                        color(t.muted),
                    )
                };
                ui.label(RichText::new(text).size(12.).color(c));
            });
        if let Some(a) = start {
            self.key_editor.capturing = Some(a);
            self.key_editor.message.clear();
        }
        if let Some(a) = clear {
            self.save_binding(&a, None, false);
        }
        if let Some(a) = reset {
            self.save_binding(&a, None, true);
        }
        if close {
            self.key_editor.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;

    fn mods(cmd: bool, shift: bool) -> Mods {
        Mods {
            cmd,
            shift,
            ..Default::default()
        }
    }
    #[test]
    fn a_chord_needs_a_real_modifier_unless_it_is_a_function_key() {
        assert_eq!(chord_for("P", mods(true, false)).unwrap().text(), "cmd+p");
        assert_eq!(
            chord_for("ArrowLeft", mods(true, true)).unwrap().text(),
            "cmd+shift+left"
        );
        assert_eq!(chord_for("F5", mods(false, false)).unwrap().text(), "f5");
        assert!(
            chord_for("P", mods(false, false)).is_err(),
            "bare letters would steal typing"
        );
        assert!(
            chord_for("P", mods(false, true)).is_err(),
            "shift alone is not enough"
        );
        assert!(chord_for("Nonsense", mods(true, false)).is_err());
        let round = chord_for("=", mods(true, false)).unwrap();
        assert_eq!(Chord::parse(&round.text()).unwrap(), round);
    }
    #[test]
    fn rebinding_detects_a_clash_and_allows_free_chords_and_unbinding() {
        let keys = BTreeMap::new();
        let free = chord_for(
            "J",
            Mods {
                cmd: true,
                alt: true,
                ..Default::default()
            },
        )
        .unwrap();
        let next = rebind(&keys, "palette", Some(&free)).unwrap();
        assert_eq!(next["palette"], "cmd+alt+j");
        let taken = Chord::parse("cmd+d").unwrap();
        let err = rebind(&keys, "palette", Some(&taken)).unwrap_err();
        assert!(err.contains("bound to both"), "{err}");
        assert_eq!(rebind(&keys, "palette", None).unwrap()["palette"], "none");
        assert!(rebind(&keys, "no_such_action", None).is_err());
    }
    #[test]
    fn the_editor_lists_actions_and_rebinding_updates_the_live_keymap() {
        let (mut app, _rx) = app_with_screen("");
        let ctx = egui::Context::default();
        app.open_key_editor();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
        assert!(app.key_editor.open);
        // Apply a binding straight to the live state (the file write is exercised in configedit's tests).
        let next = rebind(
            &app.cfg.keys,
            "palette",
            Some(&Chord::parse("cmd+alt+j").unwrap()),
        )
        .unwrap();
        let mut cfg = app.cfg.clone();
        cfg.keys = next;
        app.apply_config(cfg);
        assert_eq!(app.keymap.hint("palette"), "⌥⌘J");
    }
}

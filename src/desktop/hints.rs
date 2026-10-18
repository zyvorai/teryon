//! Quick select: labels over URLs, paths, SHAs, IPs, UUIDs and emails; type a label to copy, paste or open it.
use super::*;
use crate::uikit::{hint_matches, labels, Hint, HintKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum HintAct {
    Copy,
    Paste,
    Open,
}
impl HintAct {
    fn next(self) -> Self {
        match self {
            Self::Copy => Self::Paste,
            Self::Paste => Self::Open,
            Self::Open => Self::Copy,
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::Open => "open",
        }
    }
}
pub(super) struct HintItem {
    pub row: u16,
    pub hint: Hint,
    pub label: String,
}
pub(super) struct HintMode {
    pub pane: u64,
    pub items: Vec<HintItem>,
    pub typed: String,
    pub act: HintAct,
}

/// One `char` per column of a screen row.
pub(super) fn screen_row(screen: &vt100::Screen, row: u16, cols: u16) -> Vec<char> {
    (0..cols)
        .map(|c| {
            screen
                .cell(row, c)
                .and_then(|cell| cell.contents().chars().next())
                .unwrap_or(' ')
        })
        .collect()
}

impl Desktop {
    pub(super) fn start_hints(&mut self) {
        let id = self.focus;
        let Some(pane) = self.panes.get(&id) else {
            return;
        };
        let screen = pane.parser.screen();
        let (rows, cols) = screen.size();
        let mut found: Vec<(u16, Hint)> = Vec::new();
        for row in 0..rows {
            for h in hint_matches(&screen_row(screen, row, cols)) {
                found.push((row, h));
            }
        }
        found.truncate(26 * 26);
        if found.is_empty() {
            self.toast("Nothing to pick on this screen");
            return;
        }
        let names = labels(found.len());
        let items = found
            .into_iter()
            .zip(names)
            .map(|((row, hint), label)| HintItem { row, hint, label })
            .collect();
        self.hints = Some(HintMode {
            pane: id,
            items,
            typed: String::new(),
            act: HintAct::Copy,
        });
    }
    /// Handle one event while hints are showing; returns whether it was consumed.
    pub(super) fn hints_event(&mut self, event: &egui::Event) -> bool {
        match event {
            egui::Event::Text(text) => {
                for ch in text.chars() {
                    self.hint_char(ch);
                }
                true
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if !modifiers.command => {
                match key {
                    Key::Escape => self.hints = None,
                    Key::Tab => {
                        if let Some(h) = self.hints.as_mut() {
                            h.act = h.act.next();
                        }
                    }
                    Key::Backspace => {
                        if let Some(h) = self.hints.as_mut() {
                            h.typed.pop();
                        }
                    }
                    _ => {}
                }
                true
            }
            _ => false,
        }
    }
    fn hint_char(&mut self, ch: char) {
        let Some(h) = self.hints.as_mut() else {
            return;
        };
        h.typed.push(ch.to_ascii_lowercase());
        if !h.items.iter().any(|i| i.label.starts_with(&h.typed)) {
            h.typed.pop();
            return;
        }
        let picked = h
            .items
            .iter()
            .find(|i| i.label == h.typed)
            .map(|i| (i.hint.clone(), h.act));
        if let Some((hint, act)) = picked {
            self.hints = None;
            self.apply_hint(&hint, act);
        }
    }
    fn apply_hint(&mut self, hint: &Hint, act: HintAct) {
        let local = self.host == "local";
        match act {
            HintAct::Copy => {
                self.copy_text = Some(hint.text.clone());
                self.toast(format!("Copied {}", shorten(&hint.text)));
            }
            HintAct::Paste => self.input(hint.text.clone().into_bytes()),
            HintAct::Open => match hint.kind {
                HintKind::Url => uikit::open_url(&hint.text),
                HintKind::Email => uikit::open_url(&format!("mailto:{}", hint.text)),
                HintKind::Path if local => {
                    // Strip a trailing :line[:col] before handing the path to the OS.
                    let path = hint.text.split(':').next().unwrap_or(&hint.text);
                    uikit::open_url(&expand_home(path));
                }
                _ => {
                    self.copy_text = Some(hint.text.clone());
                    self.toast(format!(
                        "Copied {} (only links and local paths open)",
                        shorten(&hint.text)
                    ));
                }
            },
        }
    }
}
fn shorten(text: &str) -> String {
    if text.chars().count() > 40 {
        format!("{}…", text.chars().take(39).collect::<String>())
    } else {
        text.to_string()
    }
}
fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), crate::util::home()) {
        (Some(rest), Ok(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortening_keeps_short_text_and_marks_cut_text() {
        assert_eq!(shorten("abc"), "abc");
        assert!(shorten(&"x".repeat(60)).ends_with('…'));
        assert_eq!(expand_home("/etc/hosts"), "/etc/hosts");
    }
    #[test]
    fn screen_rows_give_one_char_per_column() {
        let mut p = vt100::Parser::new(2, 10, 0);
        p.process(b"hi  there");
        let r = screen_row(p.screen(), 0, 10);
        assert_eq!(r.iter().collect::<String>(), "hi  there ");
    }
}

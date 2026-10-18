//! Triggers: rules from `[[desktop.triggers]]` that highlight, notify or beep when output matches.
use super::*;
use crate::config::{Trigger, TriggerAction};
use std::hash::{Hash, Hasher};

pub(super) struct CompiledTrigger {
    re: regex::Regex,
    action: TriggerAction,
    color: Option<Color32>,
    unfocused_only: bool,
}
/// A highlighted stretch of a screen row: `(row, column, length, color)`.
pub(super) type Span = (u16, usize, usize, Option<Color32>);

pub(super) fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
pub(super) fn compile(list: &[Trigger]) -> Vec<CompiledTrigger> {
    list.iter()
        .filter_map(|t| {
            Some(CompiledTrigger {
                re: regex::Regex::new(&t.pattern).ok()?,
                action: t.action,
                color: t.color.as_deref().and_then(parse_hex),
                unfocused_only: t.unfocused_only.unwrap_or(matches!(
                    t.action,
                    TriggerAction::Notify | TriggerAction::Bell
                )),
            })
        })
        .collect()
}
const REMEMBER: Duration = Duration::from_secs(300);

impl Desktop {
    /// Apply the triggers to pane `id`'s screen after it changed. Highlights are recomputed every time;
    /// notifications fire once per distinct line, and never for what was already on screen at first sight.
    pub(super) fn scan_triggers(&mut self, id: u64) {
        let Some(pane) = self.panes.get(&id) else {
            return;
        };
        if self.triggers.is_empty() {
            if let Some(p) = self.panes.get_mut(&id) {
                p.spans.clear();
                p.scanned = true;
            }
            return;
        }
        let first_scan = !pane.scanned;
        let screen = pane.parser.screen();
        let (rows, cols) = screen.size();
        let mut spans: Vec<Span> = Vec::new();
        let mut fires: Vec<(usize, String)> = Vec::new();
        for row in 0..rows {
            let chars = hints::screen_row(screen, row, cols);
            let text: String = chars.iter().collect();
            let line = text.trim_end();
            if line.is_empty() {
                continue;
            }
            for (i, t) in self.triggers.iter().enumerate() {
                let hits = find::regex_matches(&t.re, &chars);
                if hits.is_empty() {
                    continue;
                }
                if t.action == TriggerAction::Highlight {
                    spans.extend(hits.into_iter().map(|(c, l)| (row, c, l, t.color)));
                } else {
                    fires.push((i, line.to_string()));
                }
            }
        }
        if let Some(p) = self.panes.get_mut(&id) {
            p.spans = spans;
            p.scanned = true;
        }
        if first_scan {
            return;
        }
        let now = Instant::now();
        self.trigger_seen
            .retain(|_, at| now.duration_since(*at) < REMEMBER);
        for (i, line) in fires {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (id, i, &line).hash(&mut h);
            let key = h.finish();
            if self.trigger_seen.insert(key, now).is_some() {
                continue;
            }
            let (action, only_unfocused) =
                (self.triggers[i].action, self.triggers[i].unfocused_only);
            if only_unfocused && self.window_focused {
                continue;
            }
            match action {
                TriggerAction::Notify => crate::util::notify(&format!("Pane {id}: {line}")),
                TriggerAction::Bell => crate::util::beep(),
                TriggerAction::Toast => self.toast(line),
                TriggerAction::Highlight => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors_and_compilation() {
        assert_eq!(
            parse_hex("#ff453a"),
            Some(Color32::from_rgb(0xff, 0x45, 0x3a))
        );
        assert_eq!(parse_hex("ff453a"), None);
        let list = vec![
            Trigger {
                pattern: "err".into(),
                action: TriggerAction::Notify,
                color: None,
                unfocused_only: None,
            },
            Trigger {
                pattern: "ok".into(),
                action: TriggerAction::Toast,
                color: Some("#00ff00".into()),
                unfocused_only: Some(true),
            },
            Trigger {
                pattern: "(".into(),
                action: TriggerAction::Highlight,
                color: None,
                unfocused_only: None,
            },
        ];
        let c = compile(&list);
        assert_eq!(c.len(), 2, "an invalid pattern is skipped");
        assert!(c[0].unfocused_only, "notify defaults to background only");
        assert!(c[1].unfocused_only && c[1].color.is_some());
    }
}

//! Shell integration: prompt/command marks (OSC 133) and the working directory (OSC 7).
use crate::protocol::Mark;
use std::time::Instant;

const MAX_MARKS: usize = 500;

#[derive(Debug, PartialEq, Eq)]
pub enum OscEvent {
    /// 133;A: the prompt is about to be drawn.
    Prompt,
    /// 133;C: the command started producing output.
    Output,
    /// 133;D[;status]: the command finished.
    Done(Option<i32>),
    /// 7;file://host/path: the shell's working directory.
    Cwd(String),
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
/// Path part of a `file://host/path` URL, percent-decoded; `None` for anything else.
pub fn file_url_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let decoded = percent_decode(path);
    (decoded.starts_with('/') && !decoded.contains('\0') && decoded.len() <= 4096)
        .then_some(decoded)
}
/// Interpret the parameters of an OSC sequence the terminal parser did not handle itself.
pub fn parse_osc(params: &[&[u8]]) -> Option<OscEvent> {
    let first = std::str::from_utf8(params.first()?).ok()?;
    match first {
        "133" => {
            let kind = std::str::from_utf8(params.get(1)?).ok()?;
            match kind.chars().next()? {
                'A' => Some(OscEvent::Prompt),
                'C' => Some(OscEvent::Output),
                'D' => {
                    let status = params
                        .get(2)
                        .and_then(|p| std::str::from_utf8(p).ok())
                        .and_then(|s| s.trim().parse::<i32>().ok());
                    Some(OscEvent::Done(status))
                }
                _ => None,
            }
        }
        "7" => {
            // The URL may itself contain ';', which the parser split on.
            let joined = params[1..]
                .iter()
                .filter_map(|p| std::str::from_utf8(p).ok())
                .collect::<Vec<_>>()
                .join(";");
            file_url_path(&joined).map(OscEvent::Cwd)
        }
        _ => None,
    }
}

/// Marks and the live working directory of one pane.
#[derive(Default)]
pub struct MarkLog {
    pub marks: Vec<Mark>,
    pub cwd: Option<String>,
    ran_at: Option<Instant>,
}
impl MarkLog {
    /// Record `event` seen at absolute `line`; marks older than `floor` (no longer retained) are dropped.
    pub fn apply(&mut self, event: OscEvent, line: u64, floor: u64, now: Instant, unix: u64) {
        match event {
            OscEvent::Prompt => self.marks.push(Mark {
                kind: 'A',
                line,
                exit: None,
                ms: None,
                at: unix,
            }),
            OscEvent::Output => {
                self.ran_at = Some(now);
                self.marks.push(Mark {
                    kind: 'C',
                    line,
                    exit: None,
                    ms: None,
                    at: unix,
                });
            }
            OscEvent::Done(exit) => {
                let ms = self
                    .ran_at
                    .take()
                    .map(|t| u64::try_from(now.duration_since(t).as_millis()).unwrap_or(u64::MAX));
                self.marks.push(Mark {
                    kind: 'D',
                    line,
                    exit,
                    ms,
                    at: unix,
                });
            }
            OscEvent::Cwd(path) => self.cwd = Some(path),
        }
        self.marks.retain(|m| m.line >= floor);
        if self.marks.len() > MAX_MARKS {
            let drop = self.marks.len() - MAX_MARKS;
            self.marks.drain(..drop);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn osc(s: &str) -> Vec<&[u8]> {
        s.split(';').map(str::as_bytes).collect()
    }
    #[test]
    fn osc_sequences_parse() {
        assert_eq!(parse_osc(&osc("133;A")), Some(OscEvent::Prompt));
        assert_eq!(parse_osc(&osc("133;C")), Some(OscEvent::Output));
        assert_eq!(parse_osc(&osc("133;D;2")), Some(OscEvent::Done(Some(2))));
        assert_eq!(parse_osc(&osc("133;D")), Some(OscEvent::Done(None)));
        assert_eq!(parse_osc(&osc("133;B")), None);
        assert_eq!(
            parse_osc(&osc("7;file://mini.local/Users/me/my%20proj")),
            Some(OscEvent::Cwd("/Users/me/my proj".into()))
        );
        assert_eq!(
            parse_osc(&osc("7;file://h/a;b")),
            Some(OscEvent::Cwd("/a;b".into()))
        );
        assert_eq!(parse_osc(&osc("7;http://x/y")), None);
        assert_eq!(parse_osc(&osc("9;note")), None);
        assert_eq!(parse_osc(&[]), None);
    }
    #[test]
    fn urls_decode_safely() {
        assert_eq!(file_url_path("file:///tmp/a%2Fb"), Some("/tmp/a/b".into()));
        assert_eq!(file_url_path("file://host"), None);
        assert_eq!(file_url_path("file://h/%zz%4"), Some("/%zz%4".into()));
        assert_eq!(file_url_path("file://h/a%00b"), None);
    }
    #[test]
    fn the_log_times_commands_prunes_and_caps() {
        let mut log = MarkLog::default();
        let t0 = Instant::now();
        log.apply(OscEvent::Prompt, 10, 0, t0, 100);
        log.apply(OscEvent::Output, 11, 0, t0, 101);
        let later = t0 + std::time::Duration::from_millis(1500);
        log.apply(OscEvent::Done(Some(1)), 15, 0, later, 102);
        let d = log.marks.last().unwrap();
        assert_eq!((d.kind, d.exit, d.ms), ('D', Some(1), Some(1500)));
        log.apply(OscEvent::Done(Some(0)), 20, 0, later, 103);
        assert_eq!(
            log.marks.last().unwrap().ms,
            None,
            "a D without a C has no duration"
        );
        log.apply(OscEvent::Prompt, 30, 12, later, 104);
        assert!(
            log.marks.iter().all(|m| m.line >= 12),
            "marks below the floor are pruned"
        );
        for i in 0..600 {
            log.apply(OscEvent::Prompt, 100 + i, 0, later, 200);
        }
        assert_eq!(log.marks.len(), MAX_MARKS);
        log.apply(OscEvent::Cwd("/x".into()), 0, 0, later, 0);
        assert_eq!(log.cwd.as_deref(), Some("/x"));
    }
}

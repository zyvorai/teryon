// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `teryon play`: play an asciicast v2 file (as `teryon record` writes) in the terminal, at any speed,
//! with long pauses shortened. The recorded bytes go to your terminal, so by default sequences that
//! write the clipboard, report back to the program or run terminal-specific actions are dropped.
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::time::Duration;

/// Largest cast file read.
const MAX_BYTES: u64 = 256 << 20;

#[derive(Debug, PartialEq)]
pub struct Cast {
    pub width: u64,
    pub height: u64,
    /// (seconds since the start, text) of every output event.
    pub events: Vec<(f64, String)>,
}

pub fn parse(text: &str) -> Result<Cast> {
    let mut lines = text.lines();
    let header: serde_json::Value =
        serde_json::from_str(lines.next().context("the file is empty")?)
            .context("the first line is not an asciicast header")?;
    if header["version"] != 2 {
        bail!("only asciicast v2 files are supported");
    }
    let mut events = Vec::new();
    for (n, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let ev: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("line {} is not an event", n + 2))?;
        let (Some(t), Some(kind), Some(data)) = (ev[0].as_f64(), ev[1].as_str(), ev[2].as_str())
        else {
            bail!("line {} is not an event", n + 2);
        };
        if kind == "o" && t.is_finite() && t >= 0.0 {
            events.push((t, data.to_string()));
        }
    }
    Ok(Cast {
        width: header["width"].as_u64().unwrap_or(80),
        height: header["height"].as_u64().unwrap_or(24),
        events,
    })
}

pub fn read(path: &std::path::Path) -> Result<Cast> {
    let meta =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if meta.len() > MAX_BYTES {
        bail!("{} is larger than {} MB", path.display(), MAX_BYTES >> 20);
    }
    parse(
        &std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?,
    )
}

/// How long to wait before each event: its gap to the one before, at most `max_idle` seconds, divided
/// by `speed`.
pub fn delays(cast: &Cast, speed: f64, max_idle: f64) -> Vec<Duration> {
    let mut prev = 0.0;
    cast.events
        .iter()
        .map(|(t, _)| {
            let gap = (t - prev).clamp(0.0, max_idle);
            prev = *t;
            Duration::from_secs_f64(gap / speed)
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Csi,
    Osc,
    OscEsc,
    /// DCS, APC, PM or SOS: dropped until the string terminator.
    Str,
    StrEsc,
}

/// Removes what a recording must not be able to make your terminal do. It works across event
/// boundaries, since a sequence can be split between two events.
pub struct Filter {
    state: State,
    buf: String,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            state: State::Ground,
            buf: String::new(),
        }
    }
}

/// OSC numbers that only describe things: titles, the working directory, hyperlinks, prompt marks.
const SAFE_OSC: [&str; 6] = ["0", "1", "2", "7", "8", "133"];

fn csi_is_safe(body: &str) -> bool {
    let Some(final_byte) = body.chars().last() else {
        return true;
    };
    let params = &body[..body.len() - final_byte.len_utf8()];
    match final_byte {
        // Status, device-attribute and window reports; answers would arrive as typed input.
        'n' | 'c' | 't' | 'x' => false,
        'q' => !params.starts_with('>'),
        'p' => !params.contains('$'),
        _ => true,
    }
}

impl Filter {
    pub fn feed(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match self.state {
                State::Ground => {
                    if c == '\u{1b}' {
                        self.state = State::Esc;
                    } else if c == '\u{9b}' {
                        self.state = State::Csi;
                        self.buf.clear();
                    } else if c == '\u{9d}' {
                        self.state = State::Osc;
                        self.buf.clear();
                    } else {
                        out.push(c);
                    }
                }
                State::Esc => match c {
                    '[' => {
                        self.state = State::Csi;
                        self.buf.clear();
                    }
                    ']' => {
                        self.state = State::Osc;
                        self.buf.clear();
                    }
                    'P' | '_' | '^' | 'X' => self.state = State::Str,
                    '\u{1b}' => out.push('\u{1b}'),
                    // Other escapes (cursor save, charset, reset) are one or two characters.
                    _ => {
                        out.push('\u{1b}');
                        out.push(c);
                        self.state = State::Ground;
                    }
                },
                State::Csi => {
                    self.buf.push(c);
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        if csi_is_safe(&self.buf) {
                            out.push_str("\u{1b}[");
                            out.push_str(&self.buf);
                        }
                        self.state = State::Ground;
                    } else if self.buf.len() > 64 {
                        self.state = State::Ground;
                    }
                }
                State::Osc => match c {
                    '\u{7}' | '\u{9c}' => self.finish_osc(&mut out, "\u{7}"),
                    '\u{1b}' => self.state = State::OscEsc,
                    _ => {
                        if self.buf.len() < 4096 {
                            self.buf.push(c);
                        }
                    }
                },
                State::OscEsc => {
                    if c == '\\' {
                        self.finish_osc(&mut out, "\u{1b}\\");
                    } else {
                        // The ESC started something else; the OSC is abandoned.
                        self.state = State::Ground;
                        self.buf.clear();
                    }
                }
                State::Str => match c {
                    '\u{1b}' => self.state = State::StrEsc,
                    '\u{7}' | '\u{9c}' => self.state = State::Ground,
                    _ => {}
                },
                State::StrEsc => {
                    self.state = if c == '\\' { State::Ground } else { State::Str };
                }
            }
        }
        out
    }
    fn finish_osc(&mut self, out: &mut String, terminator: &str) {
        let number = self.buf.split(';').next().unwrap_or("");
        if SAFE_OSC.contains(&number)
            && !self
                .buf
                .chars()
                .any(|c| c.is_control() || crate::sanitize::is_invisible(c))
        {
            out.push_str("\u{1b}]");
            out.push_str(&self.buf);
            out.push_str(terminator);
        }
        self.buf.clear();
        self.state = State::Ground;
    }
}

/// Write the cast to `out`, calling `sleep` before each event.
pub fn play(
    cast: &Cast,
    speed: f64,
    max_idle: f64,
    raw: bool,
    out: &mut impl Write,
    sleep: &mut impl FnMut(Duration),
) -> Result<()> {
    let mut filter = Filter::default();
    for ((_, text), delay) in cast.events.iter().zip(delays(cast, speed, max_idle)) {
        if !delay.is_zero() {
            sleep(delay);
        }
        if raw {
            out.write_all(text.as_bytes())?;
        } else {
            out.write_all(filter.feed(text).as_bytes())?;
        }
        out.flush()?;
    }
    Ok(())
}

pub fn info(cast: &Cast) -> String {
    let seconds = cast.events.last().map_or(0.0, |e| e.0);
    let bytes: usize = cast.events.iter().map(|e| e.1.len()).sum();
    format!(
        "{}x{} terminal, {} frames, {:.1}s, {} bytes of output\n",
        cast.width,
        cast.height,
        cast.events.len(),
        seconds,
        bytes
    )
}

/// (seconds, event code, data)
type Event = (f64, String, String);

fn header_and_events(text: &str) -> Result<(String, Vec<Event>)> {
    let mut lines = text.lines();
    let head = lines.next().context("the file is empty")?;
    let header: serde_json::Value =
        serde_json::from_str(head).context("the first line is not an asciicast header")?;
    if header["version"] != 2 {
        bail!("only asciicast v2 files are supported");
    }
    let mut events = Vec::new();
    for (n, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let ev: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("line {} is not an event", n + 2))?;
        let (Some(t), Some(kind), Some(data)) = (ev[0].as_f64(), ev[1].as_str(), ev[2].as_str())
        else {
            bail!("line {} is not an event", n + 2);
        };
        if !t.is_finite() || t < 0.0 {
            bail!("line {} has an invalid time", n + 2);
        }
        events.push((t, kind.to_string(), data.to_string()));
    }
    Ok((head.to_string(), events))
}

fn join_events(head: &str, events: &[Event]) -> String {
    let mut out = format!("{head}\n");
    for (t, k, d) in events {
        let t = (t * 1e6).round() / 1e6;
        out.push_str(&serde_json::json!([t, k, d]).to_string());
        out.push('\n');
    }
    out
}

/// The recording with every pause longer than `max_idle` seconds shortened to `max_idle`, so the file
/// itself (not just `play`) is shorter. Every event kind is kept.
pub fn compact(text: &str, max_idle: f64) -> Result<String> {
    if !max_idle.is_finite() || max_idle < 0.0 {
        bail!("the longest pause must be zero or more seconds");
    }
    let (head, mut events) = header_and_events(text)?;
    let (mut prev, mut shift) = (0.0f64, 0.0f64);
    for ev in &mut events {
        let gap = (ev.0 - prev).max(0.0);
        prev = ev.0;
        if gap > max_idle {
            shift += gap - max_idle;
        }
        ev.0 -= shift;
    }
    Ok(join_events(&head, &events))
}

/// A copy of an existing recording made safe to share, like `record --redact`: output events lose
/// every sequence except styling and cursor moves and have secrets replaced by `[redacted:kind]`;
/// input (`i`) events are dropped. Returns the new file text and how many secrets were replaced.
pub fn redact(text: &str) -> Result<(String, usize)> {
    let (head, events) = header_and_events(text)?;
    let mut filter = Filter::default();
    let (mut out, mut n) = (Vec::new(), 0);
    for (t, kind, data) in events {
        match kind.as_str() {
            "i" => {}
            "o" => {
                let (clean, k) = crate::ai::redact::redact(&filter.feed(&data));
                n += k;
                out.push((t, kind, clean));
            }
            _ => out.push((t, kind, data)),
        }
    }
    Ok((join_events(&head, &out), n))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAST: &str = "{\"version\":2,\"width\":100,\"height\":30}\n[0.5,\"o\",\"hello \"]\n[1.0,\"r\",\"90x30\"]\n[10.0,\"o\",\"world\"]\n[10.2,\"o\",\"!\"]\n";

    #[test]
    fn parsing_keeps_output_events_and_the_terminal_size() {
        let c = parse(CAST).unwrap();
        assert_eq!((c.width, c.height), (100, 30));
        assert_eq!(c.events.len(), 3);
        assert!(parse("not json").is_err());
        assert!(parse("{\"version\":1}\n").is_err());
        assert!(parse("{\"version\":2}\n[1,2]\n").is_err());
        assert!(info(&c).starts_with("100x30 terminal, 3 frames, 10.2s"));
    }

    #[test]
    fn long_pauses_are_shortened_and_speed_divides_every_wait() {
        let c = parse(CAST).unwrap();
        let d = delays(&c, 2.0, 2.0);
        assert_eq!(
            d,
            [
                Duration::from_millis(250),
                Duration::from_millis(1000),
                Duration::from_millis(100)
            ]
        );
    }

    #[test]
    fn playing_writes_the_text_in_order_and_sleeps_between_events() {
        let c = parse(CAST).unwrap();
        let (mut out, mut slept) = (Vec::new(), Vec::new());
        play(&c, 1.0, 1.0, false, &mut out, &mut |d| slept.push(d)).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "hello world!");
        assert_eq!(slept.len(), 3);
        assert_eq!(slept.iter().sum::<Duration>(), Duration::from_millis(1700));
    }

    #[test]
    fn clipboard_queries_and_device_strings_are_dropped_but_colours_titles_and_links_stay() {
        let mut f = Filter::default();
        let text = "a\u{1b}[31mred\u{1b}[0m\u{1b}]0;title\u{7}\u{1b}]52;c;Zm9v\u{7}\u{1b}[6n\u{1b}[c\u{1b}[>q\u{1b}[2 q\u{1b}Pq#0;2;0;0;0\u{1b}\\\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\\u{1b}]1337;File=x:y\u{7}z\u{1b}[?25h";
        assert_eq!(
            f.feed(text),
            "a\u{1b}[31mred\u{1b}[0m\u{1b}]0;title\u{7}\u{1b}[2 q\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\z\u{1b}[?25h"
        );
    }

    #[test]
    fn a_sequence_split_between_events_is_still_filtered() {
        let mut f = Filter::default();
        let out = format!(
            "{}{}{}",
            f.feed("x\u{1b}]5"),
            f.feed("2;c;Zm9v"),
            f.feed("\u{7}y")
        );
        assert_eq!(out, "xy");
        let out = format!("{}{}", f.feed("a\u{1b}["), f.feed("6nb"));
        assert_eq!(out, "ab");
    }

    #[test]
    fn raw_mode_passes_everything_through() {
        let c = parse("{\"version\":2}\n[0,\"o\",\"\\u001b]52;c;Zm9v\\u0007\"]\n").unwrap();
        let mut out = Vec::new();
        play(&c, 1.0, 1.0, true, &mut out, &mut |_| {}).unwrap();
        assert_eq!(out, b"\x1b]52;c;Zm9v\x07");
    }

    #[test]
    fn a_title_or_link_with_control_characters_is_dropped_even_though_osc_0_and_8_are_allowed() {
        let mut f = Filter::default();
        assert_eq!(f.feed("\x1b]0;fine\x07"), "\x1b]0;fine\x07");
        assert_eq!(f.feed("\x1b]2;a\rb\x08c\x07"), "");
        assert_eq!(f.feed("\x1b]0;x\u{202E}y\x07"), "");
        assert_eq!(f.feed("\x1b]8;;http://h/\n\x07t"), "t");
    }

    #[test]
    fn compacting_shortens_long_pauses_in_the_file_and_keeps_other_events() {
        let out = compact(CAST, 2.0).unwrap();
        let c = parse(&out).unwrap();
        let times: Vec<f64> = c.events.iter().map(|e| e.0).collect();
        assert_eq!(times, [0.5, 3.0, 3.2]);
        assert!(out.contains("\"r\""), "{out}");
        assert!(compact(CAST, -1.0).is_err());
        assert!(compact("{\"version\":1}\n", 1.0).is_err());
    }

    #[test]
    fn redacting_a_recording_drops_input_clipboard_writes_and_secrets() {
        let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let text = format!(
            "{{\"version\":2,\"width\":80,\"height\":24}}\n[0.1,\"o\",\"\\u001b]52;c;QUJD\\u0007token {secret}\"]\n[0.2,\"i\",\"typed-secret\\n\"]\n[0.3,\"m\",\"mark\"]\n"
        );
        let (out, n) = redact(&text).unwrap();
        assert!(n >= 1, "{out}");
        assert!(
            !out.contains(secret) && !out.contains("typed-secret"),
            "{out}"
        );
        assert!(!out.contains("]52"), "{out}");
        assert!(out.contains("\"m\""), "{out}");
    }
}

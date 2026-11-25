// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `teryon play` and `teryon cast`: asciicast v2 (as `teryon record` writes) and v3 files (as
//! asciinema 3 writes). Playback runs at any speed with long pauses shortened. The recorded bytes go to
//! your terminal, so by default sequences that write the clipboard, report back to the program or run
//! terminal-specific actions are dropped.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::time::Duration;

/// Largest cast file read.
const MAX_BYTES: u64 = 256 << 20;

/// One event of a recording, with its time in seconds since the start (v3 files store intervals;
/// they are added up when read).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub t: f64,
    /// `o` output, `i` input, `m` marker, `r` resize, `x` exit status (v3).
    pub code: String,
    pub data: String,
}

/// A whole recording file: every event kind, and the header as it was written.
#[derive(Debug, Clone)]
pub struct Recording {
    pub version: u8,
    head: String,
    pub header: Value,
    pub events: Vec<Event>,
}

impl Recording {
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text.lines();
        let head = lines.next().context("the file is empty")?;
        let header: Value =
            serde_json::from_str(head).context("the first line is not an asciicast header")?;
        let version = match header["version"].as_u64() {
            Some(2) => 2,
            Some(3) => 3,
            _ => bail!("only asciicast v2 and v3 files are supported"),
        };
        let mut events = Vec::new();
        let mut clock = 0.0f64;
        for (n, line) in lines.enumerate() {
            if line.trim().is_empty() || (version == 3 && line.starts_with('#')) {
                continue;
            }
            let ev: Value = serde_json::from_str(line)
                .with_context(|| format!("line {} is not an event", n + 2))?;
            let (Some(t), Some(code), Some(data)) =
                (ev[0].as_f64(), ev[1].as_str(), ev[2].as_str())
            else {
                bail!("line {} is not an event", n + 2);
            };
            if !t.is_finite() || t < 0.0 {
                bail!("line {} has an invalid time", n + 2);
            }
            let t = if version == 3 {
                clock += t;
                clock
            } else {
                t
            };
            if t > MAX_SECONDS {
                bail!("line {} has an invalid time", n + 2);
            }
            events.push(Event {
                t,
                code: code.to_string(),
                data: data.to_string(),
            });
        }
        Ok(Self {
            version,
            head: head.to_string(),
            header,
            events,
        })
    }

    /// The file text, in the recording's own format. Comments of a v3 file are not kept.
    pub fn to_text(&self) -> String {
        let mut out = format!("{}\n", self.head);
        let mut prev = 0.0;
        for e in &self.events {
            // Round the time on the clock, not each interval, so rounding cannot add up.
            let t = micros(e.t);
            let written = if self.version == 3 {
                let gap = micros((t - prev).max(0.0));
                prev = t;
                gap
            } else {
                t
            };
            out.push_str(&json!([written, e.code, e.data]).to_string());
            out.push('\n');
        }
        out
    }

    fn set_header(&mut self, header: Value) {
        self.head = header.to_string();
        self.header = header;
    }

    /// (columns, rows) at the start.
    pub fn size(&self) -> (u64, u64) {
        let (c, r) = if self.version == 3 {
            (&self.header["term"]["cols"], &self.header["term"]["rows"])
        } else {
            (&self.header["width"], &self.header["height"])
        };
        (c.as_u64().unwrap_or(80), r.as_u64().unwrap_or(24))
    }

    fn duration(&self) -> f64 {
        self.events.iter().fold(0.0, |max, e| e.t.max(max))
    }
}

/// Longest recording accepted, in seconds (about 30 years); keeps every time arithmetic finite.
const MAX_SECONDS: f64 = 1e9;

fn micros(t: f64) -> f64 {
    (t * 1e6).round() / 1e6
}

/// What `play` needs from a recording.
#[derive(Debug, PartialEq)]
pub struct Cast {
    pub version: u8,
    pub width: u64,
    pub height: u64,
    /// (seconds since the start, text) of every output event.
    pub events: Vec<(f64, String)>,
    pub markers: Vec<Marker>,
    /// The header's `idle_time_limit`: the longest pause the author wants kept.
    pub idle_limit: Option<f64>,
}

#[derive(Debug, PartialEq)]
pub struct Marker {
    /// How many output events come before it.
    pub after: usize,
    pub t: f64,
    pub label: String,
}

pub fn parse(text: &str) -> Result<Cast> {
    let rec = Recording::parse(text)?;
    let (width, height) = rec.size();
    let idle_limit = rec.header["idle_time_limit"]
        .as_f64()
        .filter(|s| s.is_finite() && *s > 0.0);
    let version = rec.version;
    let (mut events, mut markers) = (Vec::new(), Vec::new());
    for e in rec.events {
        match e.code.as_str() {
            "o" => events.push((e.t, e.data)),
            "m" => markers.push(Marker {
                after: events.len(),
                t: e.t,
                label: e.data,
            }),
            _ => {}
        }
    }
    Ok(Cast {
        version,
        width,
        height,
        events,
        markers,
        idle_limit,
    })
}

/// The text of a recording file, refusing one that is too large to hold in memory.
pub fn read_text(path: &std::path::Path) -> Result<String> {
    let meta =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if meta.len() > MAX_BYTES {
        bail!("{} is larger than {} MB", path.display(), MAX_BYTES >> 20);
    }
    std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
}

pub fn read(path: &std::path::Path) -> Result<Cast> {
    parse(&read_text(path)?)
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
    /// Drop every OSC sequence, even titles and links.
    sharing: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            state: State::Ground,
            buf: String::new(),
            sharing: false,
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
    /// For files that will be shared: titles, working directories and hyperlinks can carry secrets and
    /// private paths, so no OSC sequence survives.
    pub fn sharing() -> Self {
        Self {
            sharing: true,
            ..Self::default()
        }
    }

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
        if !self.sharing
            && SAFE_OSC.contains(&number)
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

/// What to do at a marker: `false` stops playback.
pub type OnMarker<'a> = &'a mut dyn FnMut(&Marker) -> Result<bool>;

/// Write the cast to `out`, calling `sleep` before each event.
pub fn play(
    cast: &Cast,
    speed: f64,
    max_idle: f64,
    raw: bool,
    out: &mut impl Write,
    sleep: &mut impl FnMut(Duration),
) -> Result<()> {
    play_marked(cast, speed, max_idle, raw, out, sleep, None)
}

/// `play`, also calling `on_marker` when playback reaches each marker.
pub fn play_marked(
    cast: &Cast,
    speed: f64,
    max_idle: f64,
    raw: bool,
    out: &mut impl Write,
    sleep: &mut impl FnMut(Duration),
    mut on_marker: Option<OnMarker<'_>>,
) -> Result<()> {
    let mut filter = Filter::default();
    let mut prev = 0.0;
    let mut wait = |t: f64, sleep: &mut dyn FnMut(Duration)| {
        let delay = Duration::from_secs_f64((t - prev).clamp(0.0, max_idle) / speed);
        prev = t;
        if !delay.is_zero() {
            sleep(delay);
        }
    };
    let mut markers = cast.markers.iter().peekable();
    for i in 0..=cast.events.len() {
        if let Some(on_marker) = on_marker.as_mut() {
            while let Some(m) = markers.next_if(|m| m.after <= i) {
                wait(m.t, sleep);
                if !on_marker(m)? {
                    return Ok(());
                }
            }
        }
        let Some((t, text)) = cast.events.get(i) else {
            break;
        };
        wait(*t, sleep);
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
    let mut out = format!(
        "{}x{} terminal, {} frames, {:.1}s, {} bytes of output, asciicast v{}\n",
        cast.width,
        cast.height,
        cast.events.len(),
        seconds,
        bytes,
        cast.version
    );
    for m in &cast.markers {
        out.push_str(&format!(
            "marker at {:.1}s: {}\n",
            m.t,
            crate::sanitize::label(&m.label, 200)
        ));
    }
    out
}

/// The recording with every pause longer than `max_idle` seconds shortened to `max_idle`, so the file
/// itself (not just `play`) is shorter. Every event kind is kept.
pub fn compact(text: &str, max_idle: f64) -> Result<String> {
    if !max_idle.is_finite() || max_idle < 0.0 {
        bail!("the longest pause must be zero or more seconds");
    }
    let mut rec = Recording::parse(text)?;
    let (mut prev, mut shift) = (0.0f64, 0.0f64);
    for ev in &mut rec.events {
        let gap = (ev.t - prev).max(0.0);
        prev = ev.t;
        if gap > max_idle {
            shift += gap - max_idle;
        }
        ev.t -= shift;
    }
    Ok(rec.to_text())
}

/// Replace secrets in a plain string of a header or marker and strip anything that is not text.
fn scrub(text: &str, found: &mut usize) -> String {
    let (clean, n) = crate::ai::redact::redact(&crate::sanitize::label(text, 4000));
    *found += n;
    clean
}

/// A header with only known fields, each string scrubbed (a title or command line often holds secrets).
fn shareable_header(header: &Value, found: &mut usize) -> Value {
    fn walk(v: &Value, found: &mut usize) -> Value {
        match v {
            Value::String(s) => Value::String(scrub(s, found)),
            Value::Array(a) => Value::Array(a.iter().map(|v| walk(v, found)).collect()),
            Value::Object(o) => {
                Value::Object(o.iter().map(|(k, v)| (k.clone(), walk(v, found))).collect())
            }
            other => other.clone(),
        }
    }
    let mut out = serde_json::Map::new();
    for key in [
        "version",
        "width",
        "height",
        "term",
        "timestamp",
        "duration",
        "idle_time_limit",
        "command",
        "title",
        "tags",
        "theme",
    ] {
        if let Some(v) = header.get(key) {
            out.insert(key.to_string(), walk(v, found));
        }
    }
    // Only the two variables the recorders write; anything else could be a credential.
    let env: serde_json::Map<String, Value> = ["SHELL", "TERM"]
        .iter()
        .filter_map(|k| Some((k.to_string(), walk(header["env"].get(k)?, found))))
        .collect();
    if !env.is_empty() {
        out.insert("env".into(), Value::Object(env));
    }
    Value::Object(out)
}

fn size_event(data: &str) -> Option<(u16, u16)> {
    let (c, r) = data.split_once('x')?;
    Some((c.parse().ok()?, r.parse().ok()?))
}

/// A copy of an existing recording made safe to share, like `record --redact`: output events lose
/// every sequence except styling and cursor moves and have secrets replaced by `[redacted:kind]`;
/// the header and markers are scrubbed the same way; input (`i`) events, comments and unknown events
/// are dropped. Returns the new file text and how many secrets were replaced.
pub fn redact(text: &str) -> Result<(String, usize)> {
    let mut rec = Recording::parse(text)?;
    let mut filter = Filter::sharing();
    let mut n = 0;
    let header = shareable_header(&rec.header, &mut n);
    rec.set_header(header);
    let events = std::mem::take(&mut rec.events);
    for mut e in events {
        match e.code.as_str() {
            "o" => {
                let (clean, k) = crate::ai::redact::redact(&filter.feed(&e.data));
                n += k;
                e.data = clean;
            }
            "m" => e.data = scrub(&e.data, &mut n),
            "r" if size_event(&e.data).is_some() => {}
            "x" if e.data.parse::<i64>().is_ok() => {}
            _ => continue,
        }
        rec.events.push(e);
    }
    Ok((rec.to_text(), n))
}

/// Add a marker (a chapter point `play --pause-on-markers` stops at) `at` seconds into a recording.
pub fn mark(text: &str, at: f64, label: &str) -> Result<String> {
    let mut rec = Recording::parse(text)?;
    let end = rec.duration();
    if !at.is_finite() || at < 0.0 || at > end {
        bail!("the marker must be between 0 and {end:.3} seconds");
    }
    if label.chars().any(char::is_control) || label.len() > 200 {
        bail!("a marker label is at most 200 bytes with no control characters");
    }
    let place = rec
        .events
        .iter()
        .position(|e| e.t > at)
        .unwrap_or(rec.events.len());
    rec.events.insert(
        place,
        Event {
            t: at,
            code: "m".into(),
            data: label.into(),
        },
    );
    Ok(rec.to_text())
}

/// The recording rewritten as asciicast `version` 2 or 3.
pub fn convert(text: &str, version: u8) -> Result<String> {
    let mut rec = Recording::parse(text)?;
    if version != 2 && version != 3 {
        bail!("only asciicast v2 and v3 can be written");
    }
    if rec.version != version {
        let header = convert_header(&rec, version);
        rec.version = version;
        rec.set_header(header);
        // Version 2 has no exit event.
        if version == 2 {
            rec.events.retain(|e| e.code != "x");
        }
    }
    Ok(rec.to_text())
}

fn convert_header(rec: &Recording, version: u8) -> Value {
    let old = &rec.header;
    let (cols, rows) = rec.size();
    let mut out = serde_json::Map::new();
    out.insert("version".into(), json!(version));
    let mut env = old["env"].as_object().cloned().unwrap_or_default();
    if version == 3 {
        let mut term = serde_json::Map::new();
        term.insert("cols".into(), json!(cols));
        term.insert("rows".into(), json!(rows));
        if let Some(t) = env.remove("TERM") {
            term.insert("type".into(), t);
        }
        if let Some(theme) = old.get("theme") {
            term.insert("theme".into(), theme.clone());
        }
        out.insert("term".into(), Value::Object(term));
        if let Some(tags) = old.get("tags") {
            out.insert("tags".into(), tags.clone());
        }
    } else {
        out.insert("width".into(), json!(cols));
        out.insert("height".into(), json!(rows));
        if let Some(t) = old["term"].get("type") {
            env.insert("TERM".into(), t.clone());
        }
        if let Some(theme) = old["term"].get("theme") {
            out.insert("theme".into(), theme.clone());
        }
    }
    for key in ["timestamp", "idle_time_limit", "command", "title"] {
        if let Some(v) = old.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    if !env.is_empty() {
        out.insert("env".into(), Value::Object(env));
    }
    Value::Object(out)
}

/// Rows of history the plain-text export keeps.
const TXT_HISTORY: usize = 50_000;

/// The text a recording leaves in a terminal: its output is run through a terminal emulator and the
/// history (the newest 50,000 lines) and final screen are returned without colours. (A `teryon record` file redraws the whole
/// screen in every frame, so this is its last screen.)
pub fn to_plain(text: &str) -> Result<String> {
    let rec = Recording::parse(text)?;
    let clamp = |cols: u64, rows: u64| (rows.clamp(1, 500) as u16, cols.clamp(1, 500) as u16);
    let (cols, rows) = rec.size();
    let (rows, cols) = clamp(cols, rows);
    let mut parser = crate::vt::parser(rows, cols, TXT_HISTORY);
    for e in &rec.events {
        match e.code.as_str() {
            "o" => parser.process(e.data.as_bytes()),
            "r" => {
                if let Some((c, r)) = size_event(&e.data) {
                    let (r, c) = clamp(c.into(), r.into());
                    crate::vt::set_size(parser.screen_mut(), r, c);
                }
            }
            _ => {}
        }
    }
    let (rows, cols) = crate::vt::size(parser.screen());
    parser.screen_mut().set_scrollback(usize::MAX);
    let mut offset = parser.screen().scrollback();
    let mut lines: Vec<String> = Vec::new();
    while offset > 0 {
        parser.screen_mut().set_scrollback(offset);
        let take = offset.min(usize::from(rows));
        lines.extend(parser.screen().rows(0, cols).take(take));
        offset = offset.saturating_sub(usize::from(rows));
    }
    parser.screen_mut().set_scrollback(0);
    lines.extend(parser.screen().rows(0, cols));
    for line in &mut lines {
        line.retain(|c| !c.is_control() && !crate::sanitize::is_invisible(c));
        line.truncate(line.trim_end().len());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    Ok(lines.iter().map(|l| format!("{l}\n")).collect())
}

/// Recordings played one after another, as one file in the first one's format. Resize events are added
/// where the terminal size changes; exit events are kept only from the last file.
pub fn cat(texts: &[String]) -> Result<String> {
    if texts.len() < 2 {
        bail!("give at least two recordings");
    }
    let mut all = Recording::parse(&texts[0]).context("recording 1")?;
    let mut size = all.size();
    for e in &all.events {
        if let (true, Some((c, r))) = (e.code == "r", size_event(&e.data)) {
            size = (c.into(), r.into());
        }
    }
    for (i, text) in texts.iter().enumerate().skip(1) {
        let next = Recording::parse(text).with_context(|| format!("recording {}", i + 1))?;
        let offset = all.duration();
        all.events.retain(|e| e.code != "x");
        let starts = next.size();
        if starts != size {
            size = starts;
            all.events.push(Event {
                t: offset,
                code: "r".into(),
                data: format!("{}x{}", size.0, size.1),
            });
        }
        for mut e in next.events {
            if let (true, Some((c, r))) = (e.code == "r", size_event(&e.data)) {
                size = (c.into(), r.into());
            }
            e.t += offset;
            all.events.push(e);
        }
    }
    if all.version == 2 {
        all.events.retain(|e| e.code != "x");
    }
    Ok(all.to_text())
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
        let mut slept = Vec::new();
        play(&c, 2.0, 2.0, false, &mut Vec::new(), &mut |d| slept.push(d)).unwrap();
        assert_eq!(
            slept,
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

    const V3: &str = "{\"version\":3,\"term\":{\"cols\":90,\"rows\":20},\"idle_time_limit\":1.5}\n# comment\n[0.5,\"o\",\"a\"]\n[0.5,\"m\",\"here\"]\n[2.0,\"o\",\"b\"]\n[0.1,\"x\",\"0\"]\n";

    #[test]
    fn v3_intervals_are_added_up_and_written_back_as_intervals() {
        let c = parse(V3).unwrap();
        assert_eq!((c.version, c.width, c.height), (3, 90, 20));
        assert_eq!(c.events, [(0.5, "a".to_string()), (3.0, "b".to_string())]);
        assert_eq!(c.idle_limit, Some(1.5));
        assert_eq!(c.markers[0].after, 1);
        let again = Recording::parse(V3).unwrap().to_text();
        assert!(again.contains("[2.0,\"o\",\"b\"]"), "{again}");
        assert_eq!(parse(&again).unwrap(), c);
        // A comment is only allowed in v3.
        assert!(parse("{\"version\":2}\n# no\n").is_err());
        assert!(parse("{\"version\":3}\n[1e300,\"o\",\"x\"]\n").is_err());
    }

    #[test]
    fn rounding_v3_intervals_does_not_drift() {
        let mut text = String::from("{\"version\":3,\"term\":{\"cols\":1,\"rows\":1}}\n");
        for _ in 0..3000 {
            text.push_str("[0.0000004,\"o\",\"x\"]\n");
        }
        let out = Recording::parse(&text).unwrap().to_text();
        let end = parse(&out).unwrap().events.last().unwrap().0;
        assert!((end - 0.0012).abs() < 2e-6, "{end}");
    }

    #[test]
    fn markers_are_reached_in_order_after_the_pause_before_them() {
        let c = parse(V3).unwrap();
        let log = std::cell::RefCell::new(Vec::new());
        let mut out = Vec::new();
        play_marked(
            &c,
            1.0,
            10.0,
            false,
            &mut out,
            &mut |d| log.borrow_mut().push(format!("sleep {}", d.as_millis())),
            Some(&mut |m: &Marker| {
                log.borrow_mut().push(format!("marker {}", m.label));
                Ok(true)
            }),
        )
        .unwrap();
        assert_eq!(
            *log.borrow(),
            ["sleep 500", "sleep 500", "marker here", "sleep 2000"]
        );
        let mut out = Vec::new();
        play_marked(
            &c,
            1.0,
            10.0,
            false,
            &mut out,
            &mut |_| {},
            Some(&mut |_: &Marker| Ok(false)),
        )
        .unwrap();
        assert_eq!(out, b"a");
        // A marker after the last output is still reached.
        let c = parse(&mark(CAST, 10.2, "end").unwrap()).unwrap();
        let mut seen = 0;
        play_marked(
            &c,
            1.0,
            1.0,
            false,
            &mut Vec::new(),
            &mut |_| {},
            Some(&mut |_: &Marker| {
                seen += 1;
                Ok(true)
            }),
        )
        .unwrap();
        assert_eq!(seen, 1);
    }

    #[test]
    fn marking_places_the_event_by_time_and_checks_its_input() {
        let out = mark(V3, 2.0, "mid").unwrap();
        let c = parse(&out).unwrap();
        assert_eq!((c.markers[1].after, c.markers[1].t), (1, 2.0));
        assert_eq!(c.events, parse(V3).unwrap().events);
        for (at, label) in [(-1.0, "x"), (f64::NAN, "x"), (50.0, "x"), (1.0, "a\nb")] {
            assert!(mark(V3, at, label).is_err(), "{at} {label:?}");
        }
    }

    #[test]
    fn converting_moves_the_header_fields_between_the_formats() {
        let v2 = "{\"version\":2,\"width\":100,\"height\":30,\"timestamp\":5,\"title\":\"t\",\"env\":{\"TERM\":\"vt220\",\"SHELL\":\"sh\"},\"theme\":{\"fg\":\"#000000\"}}\n[1.0,\"o\",\"a\"]\n";
        let v3 = Recording::parse(&convert(v2, 3).unwrap()).unwrap();
        assert_eq!(v3.header["term"]["type"], "vt220");
        assert_eq!(v3.header["term"]["theme"]["fg"], "#000000");
        assert_eq!(v3.header["env"], serde_json::json!({"SHELL": "sh"}));
        let back = Recording::parse(&convert(&v3.to_text(), 2).unwrap()).unwrap();
        assert_eq!(back.header, Recording::parse(v2).unwrap().header);
        assert!(convert(v2, 1).is_err());
    }

    #[test]
    fn plain_text_export_keeps_history_follows_resizes_and_drops_hidden_characters() {
        let mut text = String::from("{\"version\":2,\"width\":20,\"height\":3}\n");
        for i in 0..8 {
            text.push_str(&format!(
                "[0.1,\"o\",\"\\u001b[31mrow{i}\\u001b[0m\\r\\n\"]\n"
            ));
        }
        text.push_str("[0.2,\"r\",\"40x10\"]\n[0.3,\"r\",\"junk\"]\n[0.4,\"o\",\"x\\u202Ey\"]\n");
        let plain = to_plain(&text).unwrap();
        assert_eq!(
            plain,
            "row0\nrow1\nrow2\nrow3\nrow4\nrow5\nrow6\nrow7\nxy\n"
        );
        // A hostile size is clamped, not allocated.
        let huge = "{\"version\":2,\"width\":99999999,\"height\":0}\n[0.1,\"o\",\"ok\"]\n";
        assert_eq!(to_plain(huge).unwrap(), "ok\n");
    }

    #[test]
    fn joining_recordings_shifts_times_and_notes_a_new_size() {
        let a = V3.to_string();
        let out = cat(&[a.clone(), CAST.to_string(), a.clone()]).unwrap();
        let rec = Recording::parse(&out).unwrap();
        assert_eq!(rec.version, 3);
        let codes: String = rec.events.iter().map(|e| e.code.as_str()).collect();
        // a without its exit, a resize, CAST (with its own resize), a resize back, a whole.
        assert_eq!(codes, "omororoorom ox".replace(' ', ""));
        assert!(rec.events.windows(2).all(|w| w[0].t <= w[1].t));
        assert!(cat(&[a]).is_err());
    }
}

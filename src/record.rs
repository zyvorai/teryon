// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! asciicast v2 writer for `teryon record`.
//!
//! The daemon exposes full-screen snapshots, not the raw byte stream, so each changed
//! snapshot becomes one "clear and redraw" event. Playback is faithful per frame, not per byte.
use anyhow::{bail, Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub struct CastWriter<W: Write> {
    out: W,
    redact: bool,
    redactions: usize,
}

impl CastWriter<std::fs::File> {
    /// Create `path` with mode 0600; an existing file is never overwritten.
    pub fn create(path: &Path, cols: u16, rows: u16, unix_time: u64) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("create {} (refusing to overwrite)", path.display()))?;
        CastWriter::new(file, cols, rows, unix_time)
    }
}

impl<W: Write> CastWriter<W> {
    pub fn new(mut out: W, cols: u16, rows: u16, unix_time: u64) -> Result<Self> {
        if cols == 0 || rows == 0 {
            bail!("terminal size must be non-zero");
        }
        let header = serde_json::json!({
            "version": 2,
            "width": cols,
            "height": rows,
            "timestamp": unix_time,
            "env": { "TERM": "xterm-256color" },
        });
        writeln!(out, "{header}")?;
        out.flush()?;
        Ok(Self {
            out,
            redact: false,
            redactions: 0,
        })
    }
    /// Make every later frame safe to share: escape sequences other than styling and cursor moves
    /// are dropped (a recording must not carry an OSC 52 clipboard write or a title into a viewer's
    /// terminal), and secrets matching the redaction rules are replaced by `[redacted:kind]`.
    /// Best effort: a secret split by a style change, or one with no recognisable shape, is not caught.
    pub fn redacting(mut self) -> Self {
        self.redact = true;
        self
    }
    /// How many secrets were replaced so far.
    pub fn redactions(&self) -> usize {
        self.redactions
    }
    /// Append one screen snapshot taken `seconds` after the start.
    pub fn frame(&mut self, seconds: f64, screen: &[u8]) -> Result<()> {
        let mut text = String::from_utf8_lossy(screen).into_owned();
        if self.redact {
            let filtered = crate::castplay::Filter::default().feed(&text);
            let (clean, n) = crate::ai::redact::redact(&filtered);
            self.redactions += n;
            text = clean;
        }
        let data = format!("\x1b[2J\x1b[H{text}");
        let event = serde_json::json!([seconds, "o", data]);
        writeln!(self.out, "{event}")?;
        self.out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writes_header_and_clearing_frames() {
        let mut buf = Vec::new();
        let mut w = CastWriter::new(&mut buf, 80, 24, 1_700_000_000).unwrap();
        w.frame(0.5, b"hi\r\n").unwrap();
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["version"], 2);
        assert_eq!(lines[0]["width"], 80);
        assert_eq!(lines[1][1], "o");
        assert_eq!(lines[1][2], "\u{1b}[2J\u{1b}[Hhi\r\n");
    }
    #[test]
    fn refuses_to_overwrite_and_keeps_file_private() {
        let dir = std::env::temp_dir().join(format!("teryon-cast-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.cast");
        CastWriter::create(&path, 80, 24, 0).unwrap();
        assert!(CastWriter::create(&path, 80, 24, 0).is_err());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
    fn frames(buf: Vec<u8>) -> Vec<String> {
        String::from_utf8(buf)
            .unwrap()
            .lines()
            .skip(1)
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()[2]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }
    #[test]
    fn redacting_hides_secrets_and_drops_clipboard_and_title_sequences() {
        let mut buf = Vec::new();
        let mut w = CastWriter::new(&mut buf, 80, 24, 0).unwrap().redacting();
        let screen = "$ export API_TOKEN=hunter2hunter2\r\n\x1b[1mbold\x1b[0m\x1b]52;c;cm0gLXJm\x07\x1b]0;t\x07\r\n";
        w.frame(0.1, screen.as_bytes()).unwrap();
        assert!(w.redactions() >= 1);
        let f = frames(buf);
        assert!(!f[0].contains("hunter2hunter2"), "{}", f[0]);
        assert!(f[0].contains("[redacted:"));
        assert!(f[0].contains("\x1b[1mbold\x1b[0m"));
        assert!(!f[0].contains("cm0gLXJm") && !f[0].contains("]52"));
    }
    #[test]
    fn without_redaction_frames_are_written_as_given() {
        let mut buf = Vec::new();
        let mut w = CastWriter::new(&mut buf, 80, 24, 0).unwrap();
        w.frame(0.1, b"API_TOKEN=hunter2hunter2").unwrap();
        assert_eq!(w.redactions(), 0);
        assert!(frames(buf)[0].contains("hunter2hunter2"));
    }
    #[test]
    fn a_huge_frame_with_redaction_stays_valid_json() {
        let mut buf = Vec::new();
        let mut w = CastWriter::new(&mut buf, 80, 24, 0).unwrap().redacting();
        let big = "x".repeat(2_000_000) + "\x1b[2";
        w.frame(0.1, big.as_bytes()).unwrap();
        assert_eq!(frames(buf)[0].len(), 2_000_000 + 7);
    }
    #[test]
    fn rejects_zero_size() {
        assert!(CastWriter::new(Vec::new(), 0, 24, 0).is_err());
    }
}

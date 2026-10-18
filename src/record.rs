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
        Ok(Self { out })
    }
    /// Append one screen snapshot taken `seconds` after the start.
    pub fn frame(&mut self, seconds: f64, screen: &[u8]) -> Result<()> {
        let mut data = b"\x1b[2J\x1b[H".to_vec();
        data.extend_from_slice(screen);
        let event = serde_json::json!([seconds, "o", String::from_utf8_lossy(&data)]);
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
    #[test]
    fn rejects_zero_size() {
        assert!(CastWriter::new(Vec::new(), 0, 24, 0).is_err());
    }
}

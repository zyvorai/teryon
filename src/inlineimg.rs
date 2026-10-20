//! Inline images: the iTerm2 `OSC 1337 ; File=...:BASE64` sequence that `imgcat` and many tools
//! print. The daemon decodes the payload, works out how many cells the image covers, moves the
//! cursor past it and keeps the bytes; desktop clients fetch the bytes once and draw them.
use crate::protocol::Placement;
use std::collections::HashMap;
use std::sync::Arc;

/// Pixel size assumed for one cell when turning an image's size into cells.
const CELL_W: f64 = 8.;
const CELL_H: f64 = 16.;
/// Largest image accepted, and the most image bytes kept per pane.
pub const MAX_IMAGE: usize = 4 << 20;
const MAX_KEPT: usize = 64 << 20;

/// What the sequence asked for.
#[derive(Debug, PartialEq)]
pub struct Request {
    pub bytes: Vec<u8>,
    pub width: Dim,
    pub height: Dim,
}
#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum Dim {
    #[default]
    Auto,
    Cells(u32),
    Pixels(u32),
    Percent(u32),
}
fn dim(value: &str) -> Dim {
    let value = value.trim();
    if let Some(n) = value.strip_suffix("px") {
        n.parse().map_or(Dim::Auto, Dim::Pixels)
    } else if let Some(n) = value.strip_suffix('%') {
        n.parse().map_or(Dim::Auto, Dim::Percent)
    } else {
        value.parse().map_or(Dim::Auto, Dim::Cells)
    }
}

/// Parse the OSC parameters of `1337;File=name=..;inline=1;width=..:BASE64`.
pub fn parse(params: &[&[u8]]) -> Option<Request> {
    if params.first()? != b"1337" {
        return None;
    }
    // The arguments are separated by ';', which the terminal parser split on.
    let joined = params[1..]
        .iter()
        .map(|p| String::from_utf8_lossy(p))
        .collect::<Vec<_>>()
        .join(";");
    let rest = joined.strip_prefix("File=")?;
    let (args, data) = rest.split_once(':')?;
    let (mut inline, mut width, mut height) = (false, Dim::Auto, Dim::Auto);
    for arg in args.split(';') {
        match arg.split_once('=') {
            Some(("inline", v)) => inline = v == "1",
            Some(("width", v)) => width = dim(v),
            Some(("height", v)) => height = dim(v),
            _ => {}
        }
    }
    if !inline || data.len() > MAX_IMAGE * 4 / 3 + 8 {
        return None;
    }
    let bytes = decode_base64(data)?;
    (bytes.len() <= MAX_IMAGE && !bytes.is_empty()).then_some(Request {
        bytes,
        width,
        height,
    })
}

pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u8);
    for b in text.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = acc << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Width and height in pixels of a PNG, GIF or JPEG, from its header.
pub fn pixel_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") && b.len() >= 24 {
        let w = u32::from_be_bytes(b[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(b[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if (b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) && b.len() >= 10 {
        let w = u32::from(u16::from_le_bytes([b[6], b[7]]));
        let h = u32::from(u16::from_le_bytes([b[8], b[9]]));
        return Some((w, h));
    }
    if b.starts_with(&[0xff, 0xd8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xff {
                i += 1;
                continue;
            }
            let marker = b[i + 1];
            if marker == 0xd8 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                i += 2;
                continue;
            }
            let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
            // Start-of-frame markers carry the size.
            if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                let h = u32::from(u16::from_be_bytes([b[i + 5], b[i + 6]]));
                let w = u32::from(u16::from_be_bytes([b[i + 7], b[i + 8]]));
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

/// Cells (columns, rows) the image covers on a screen of `cols` x `rows`.
pub fn span(req: &Request, size: (u32, u32), cols: u16, rows: u16) -> (u16, u16) {
    let (iw, ih) = (f64::from(size.0.max(1)), f64::from(size.1.max(1)));
    let cell = |d: Dim, total: u16, per_cell: f64| match d {
        Dim::Auto => None,
        Dim::Cells(n) => Some(f64::from(n)),
        Dim::Pixels(n) => Some(f64::from(n) / per_cell),
        Dim::Percent(n) => Some(f64::from(total) * f64::from(n) / 100.),
    };
    let max_w = f64::from(cols);
    let max_h = f64::from(rows.saturating_sub(1).max(1));
    let (mut w, mut h) = match (
        cell(req.width, cols, CELL_W),
        cell(req.height, rows, CELL_H),
    ) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * CELL_W * ih / iw / CELL_H),
        (None, Some(h)) => (h * CELL_H * iw / ih / CELL_W, h),
        (None, None) => (iw / CELL_W, ih / CELL_H),
    };
    if w > max_w {
        h *= max_w / w;
        w = max_w;
    }
    if h > max_h {
        w *= max_h / h;
        h = max_h;
    }
    (w.ceil().max(1.) as u16, h.ceil().max(1.) as u16)
}

/// Images shown in one pane: where they sit, and their bytes.
#[derive(Default)]
pub struct ImageLog {
    pub placements: Vec<Placement>,
    blobs: HashMap<u64, Arc<Vec<u8>>>,
    kept: usize,
    next: u64,
    /// The screen's erase count when images were last checked.
    seen_erases: u64,
}
impl ImageLog {
    /// Remember an image whose top-left cell is (`line`, `col`).
    pub fn add(&mut self, bytes: Vec<u8>, line: u64, col: u16, cols: u16, rows: u16) -> u64 {
        self.next += 1;
        let id = self.next;
        self.kept += bytes.len();
        self.blobs.insert(id, Arc::new(bytes));
        self.placements.push(Placement {
            id,
            line,
            col,
            cols,
            rows,
        });
        self.trim(0);
        id
    }
    /// Forget the placements with these ids.
    pub fn remove(&mut self, ids: &[u64]) {
        self.placements.retain(|p| !ids.contains(&p.id));
        self.trim(0);
    }
    /// Drop images that scrolled out of the retained history, then the oldest while over budget.
    pub fn trim(&mut self, floor: u64) {
        self.placements
            .retain(|p| p.line + u64::from(p.rows) > floor);
        while self.kept > MAX_KEPT && !self.placements.is_empty() {
            let gone = self.placements.remove(0).id;
            if !self.placements.iter().any(|p| p.id == gone) {
                if let Some(b) = self.blobs.remove(&gone) {
                    self.kept = self.kept.saturating_sub(b.len());
                }
            }
        }
        let live: std::collections::HashSet<u64> = self.placements.iter().map(|p| p.id).collect();
        self.blobs.retain(|id, _| live.contains(id));
        self.kept = self.blobs.values().map(|b| b.len()).sum();
    }
    /// Forget images the shell erased: when the display was cleared, those on screen (from line
    /// `top` down) go; they were drawn on cells that are now blank.
    pub fn after_output(&mut self, erases: u64, top: u64) {
        if erases != self.seen_erases {
            self.seen_erases = erases;
            self.placements
                .retain(|p| p.line + u64::from(p.rows) <= top);
            self.trim(0);
        }
    }
    /// Forget every image (the history was erased).
    pub fn clear(&mut self) {
        self.placements.clear();
        self.trim(0);
    }
    pub fn bytes(&self, id: u64) -> Option<Arc<Vec<u8>>> {
        self.blobs.get(&id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b
    }
    #[test]
    fn trim_evicts_only_the_oldest_images_over_budget() {
        let mut log = ImageLog::default();
        let third = MAX_KEPT / 2 + 1;
        let a = log.add(vec![0; third], 0, 0, 1, 1);
        let b = log.add(vec![0; third], 1, 0, 1, 1);
        assert_eq!(log.placements.len(), 1);
        assert_eq!(log.placements[0].id, b);
        assert!(log.bytes(a).is_none() && log.bytes(b).is_some());
    }
    #[test]
    fn base64_decodes_with_padding_and_line_breaks() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVs\nbG8=").unwrap(), b"hello");
        assert!(decode_base64("a$b").is_none());
    }
    #[test]
    fn the_iterm_sequence_is_parsed_across_split_parameters() {
        let params: Vec<&[u8]> = vec![
            b"1337",
            b"File=name=eC5wbmc=",
            b"inline=1",
            b"width=20:aGVsbG8=",
        ];
        let r = parse(&params).unwrap();
        assert_eq!(r.bytes, b"hello");
        assert_eq!((r.width, r.height), (Dim::Cells(20), Dim::Auto));
        let not_inline: Vec<&[u8]> = vec![b"1337", b"File=inline=0:aGVsbG8="];
        assert!(parse(&not_inline).is_none());
        assert!(parse(&[b"7", b"x"]).is_none());
    }
    #[test]
    fn sizes_come_from_headers() {
        assert_eq!(pixel_size(&png(640, 480)), Some((640, 480)));
        assert_eq!(pixel_size(b"GIF89a\x10\x00\x20\x00"), Some((16, 32)));
        let jpeg = [
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0xe0, 0x02, 0x80, 0x03, 0, 0, 0, 0, 0,
        ];
        assert_eq!(pixel_size(&jpeg), Some((640, 480)));
        assert_eq!(pixel_size(b"nope"), None);
    }
    #[test]
    fn spans_keep_the_aspect_ratio_and_fit_the_screen() {
        let req = |width, height| Request {
            bytes: Vec::new(),
            width,
            height,
        };
        // 160x160 px at 8x16 px cells is 20 columns by 10 rows.
        assert_eq!(
            span(&req(Dim::Auto, Dim::Auto), (160, 160), 80, 24),
            (20, 10)
        );
        // A huge image shrinks to the width, keeping its shape.
        let (w, h) = span(&req(Dim::Auto, Dim::Auto), (4000, 2000), 80, 40);
        assert_eq!(w, 80);
        assert!((19..=21).contains(&h), "{h}");
        assert_eq!(
            span(&req(Dim::Cells(10), Dim::Auto), (100, 100), 80, 24).0,
            10
        );
        assert_eq!(
            span(&req(Dim::Percent(50), Dim::Auto), (100, 100), 80, 24).0,
            40
        );
    }
    #[test]
    fn clearing_the_display_removes_the_images_on_screen_only() {
        let mut log = ImageLog::default();
        log.add(vec![1], 2, 0, 5, 3); // in history above the screen
        log.add(vec![2], 12, 0, 5, 3); // on screen
        log.after_output(0, 10);
        assert_eq!(log.placements.len(), 2);
        log.after_output(1, 10);
        assert_eq!(
            log.placements.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![1]
        );
        log.clear();
        assert!(log.placements.is_empty() && log.bytes(1).is_none());
    }
    #[test]
    fn the_log_drops_scrolled_out_images_and_serves_bytes() {
        let mut log = ImageLog::default();
        log.add(vec![1, 2, 3], 5, 0, 10, 3);
        log.add(vec![4], 100, 0, 10, 3);
        assert_eq!(log.bytes(1).unwrap().as_slice(), &[1, 2, 3]);
        log.trim(50);
        assert_eq!(log.placements.len(), 1);
        assert!(log.bytes(1).is_none() && log.bytes(2).is_some());
    }
}

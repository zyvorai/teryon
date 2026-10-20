//! Kitty graphics protocol (`ESC _ G key=value,… ; BASE64 ESC \`), the way `kitten icat`, `chafa` and
//! many TUIs draw pictures. The daemon cuts these APC sequences out of the byte stream (the terminal
//! parser would swallow them), keeps the images, reserves their cells and tells the program what it
//! asked to know; desktop clients draw them through the same path as iTerm2 inline images.
//!
//! Supported: PNG (`f=100`) and raw RGB/RGBA (`f=24`/`f=32`) sent directly (`t=d`), in chunks (`m=1`);
//! actions transmit (`t`), transmit and display (`T`), display a stored image (`p`), query (`q`) and
//! delete (`d` with `a`/`A` for everything, `i`/`I` for one image); `c`/`r` sizing, `C=1` to keep the
//! cursor, `q=1`/`q=2` to silence replies. Not supported (and answered with an error instead of
//! ignored): files and shared memory (`t=f`/`t=s`), zlib (`o=z`), animation, cropping, z-order and
//! Unicode placeholders.
use crate::inlineimg::{self, Dim, Request, MAX_IMAGE};
use std::collections::HashMap;
use std::sync::Arc;

/// Longest APC body kept while waiting for its terminator (a chunk is at most 4 KB of base64 in the
/// protocol; a bit more is tolerated).
const MAX_APC: usize = 8 << 20;
/// Stored (transmitted but not necessarily displayed) image bytes kept per pane.
const MAX_STORED: usize = 32 << 20;

/// Part of the stream: bytes for the terminal parser, or the body of an APC sequence.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    Data(Vec<u8>),
    Apc(Vec<u8>),
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    #[default]
    Ground,
    /// Saw ESC outside a sequence.
    Esc,
    /// Inside `ESC _ …`.
    Apc,
    /// Inside an APC, saw ESC (the next byte decides whether it ends).
    ApcEsc,
    /// An APC that grew too big: skipped up to its end.
    Skip,
    SkipEsc,
}

/// Splits a byte stream into plain data and APC bodies, across read boundaries.
#[derive(Default)]
pub struct ApcFilter {
    state: State,
    body: Vec<u8>,
}
impl ApcFilter {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
        let mut out: Vec<Piece> = Vec::new();
        let mut data: Vec<u8> = Vec::new();
        for &b in bytes {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Esc;
                    } else {
                        data.push(b);
                    }
                }
                State::Esc => {
                    if b == b'_' {
                        self.state = State::Apc;
                        self.body.clear();
                    } else {
                        data.push(0x1b);
                        if b == 0x1b {
                            // Still possibly the start of a sequence.
                        } else {
                            data.push(b);
                            self.state = State::Ground;
                        }
                    }
                }
                State::Apc => match b {
                    0x1b => self.state = State::ApcEsc,
                    _ => {
                        if self.body.len() >= MAX_APC {
                            self.body.clear();
                            self.state = State::Skip;
                        } else {
                            self.body.push(b);
                        }
                    }
                },
                State::ApcEsc => {
                    if b == b'\\' {
                        if !data.is_empty() {
                            out.push(Piece::Data(std::mem::take(&mut data)));
                        }
                        out.push(Piece::Apc(std::mem::take(&mut self.body)));
                        self.state = State::Ground;
                    } else {
                        // Not a terminator: the ESC belongs to the body.
                        self.body.push(0x1b);
                        self.body.push(b);
                        self.state = State::Apc;
                    }
                }
                State::Skip => {
                    if b == 0x1b {
                        self.state = State::SkipEsc;
                    }
                }
                State::SkipEsc => {
                    self.state = if b == b'\\' {
                        State::Ground
                    } else {
                        State::Skip
                    };
                }
            }
        }
        if !data.is_empty() {
            out.push(Piece::Data(data));
        }
        out
    }
}

/// The keys of one graphics command.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cmd {
    action: u8,
    format: u32,
    medium: u8,
    more: bool,
    id: u32,
    quiet: u8,
    cols: u32,
    rows: u32,
    width: u32,
    height: u32,
    keep_cursor: bool,
    delete: u8,
    compressed: bool,
}
impl Default for Cmd {
    fn default() -> Self {
        Self {
            action: b't',
            format: 32,
            medium: b'd',
            more: false,
            id: 0,
            quiet: 0,
            cols: 0,
            rows: 0,
            width: 0,
            height: 0,
            keep_cursor: false,
            delete: b'a',
            compressed: false,
        }
    }
}
/// Split `G<keys>;<payload>` into its keys and payload.
fn parse(body: &[u8]) -> Option<(Cmd, &[u8])> {
    let rest = body.strip_prefix(b"G")?;
    let (keys, payload) = match rest.iter().position(|b| *b == b';') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, &rest[rest.len()..]),
    };
    let mut c = Cmd::default();
    for kv in std::str::from_utf8(keys).ok()?.split(',') {
        let Some((k, v)) = kv.split_once('=') else {
            continue;
        };
        let n = || v.parse::<u32>().unwrap_or(0);
        let ch = v.bytes().next().unwrap_or(0);
        match k {
            "a" => c.action = ch,
            "f" => c.format = n(),
            "t" => c.medium = ch,
            "m" => c.more = n() == 1,
            "i" => c.id = n(),
            "q" => c.quiet = u8::try_from(n()).unwrap_or(0),
            "c" => c.cols = n(),
            "r" => c.rows = n(),
            "s" => c.width = n(),
            "v" => c.height = n(),
            "C" => c.keep_cursor = n() == 1,
            "d" => c.delete = ch,
            "o" => c.compressed = ch == b'z',
            _ => {}
        }
    }
    Some((c, payload))
}

/// What the daemon should do after a command.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// Show this image at the cursor; the bool says whether the cursor moves below it.
    pub place: Option<(Request, bool, Option<u32>)>,
    /// Forget the placements of this stored image (or all of them with `None`).
    pub delete: Option<Option<u32>>,
    /// Text for the program's input: `ESC _ G … ESC \`.
    pub reply: Option<String>,
}

struct Pending {
    cmd: Cmd,
    base64: Vec<u8>,
}
struct Stored {
    png: Arc<Vec<u8>>,
    seq: u64,
}
#[derive(Default)]
pub struct KittyGfx {
    pending: Option<Pending>,
    stored: HashMap<u32, Stored>,
    kept: usize,
    seq: u64,
}

fn reply(cmd: &Cmd, ok: bool, text: &str) -> Option<String> {
    // q=1 hides OK replies, q=2 hides errors too; a command without an id gets no reply.
    if cmd.id == 0 || cmd.quiet == 2 || (ok && cmd.quiet == 1) {
        return None;
    }
    Some(format!(
        "\x1b_Gi={};{}\x1b\\",
        cmd.id,
        if ok { "OK" } else { text }
    ))
}

impl KittyGfx {
    /// Run one APC body (starting with `G`). Bodies of other APC kinds are ignored.
    pub fn command(&mut self, body: &[u8]) -> Outcome {
        let Some((cmd, payload)) = parse(body) else {
            return Outcome::default();
        };
        if let Some(mut p) = self.pending.take() {
            // A later chunk: only its payload (and `m`) counts.
            p.base64.extend_from_slice(payload);
            if p.base64.len() > MAX_IMAGE * 4 / 3 + 16 {
                return Outcome {
                    reply: reply(&p.cmd, false, "EFBIG:image is too large"),
                    ..Default::default()
                };
            }
            if cmd.more {
                self.pending = Some(p);
                return Outcome::default();
            }
            return self.finish(p.cmd, &p.base64);
        }
        match cmd.action {
            b't' | b'T' | b'q' => {
                if cmd.more {
                    self.pending = Some(Pending {
                        cmd,
                        base64: payload.to_vec(),
                    });
                    return Outcome::default();
                }
                self.finish(cmd, payload)
            }
            b'p' => match self.stored.get(&cmd.id) {
                Some(s) => Outcome {
                    place: Some((request(&s.png, &cmd), !cmd.keep_cursor, Some(cmd.id))),
                    reply: reply(&cmd, true, ""),
                    ..Default::default()
                },
                None => Outcome {
                    reply: reply(&cmd, false, "ENOENT:no such image"),
                    ..Default::default()
                },
            },
            b'd' => {
                let all = matches!(cmd.delete, b'a' | b'A');
                let one = matches!(cmd.delete, b'i' | b'I');
                if one && cmd.delete == b'I' {
                    self.forget(cmd.id);
                }
                if all && cmd.delete == b'A' {
                    self.stored.clear();
                    self.kept = 0;
                }
                Outcome {
                    delete: (all || one).then_some(one.then_some(cmd.id)),
                    ..Default::default()
                }
            }
            _ => Outcome::default(),
        }
    }

    fn finish(&mut self, cmd: Cmd, base64: &[u8]) -> Outcome {
        let fail = |text: &str| Outcome {
            reply: reply(&cmd, false, text),
            ..Default::default()
        };
        if cmd.medium != b'd' {
            return fail("EINVAL:only direct transmission (t=d) is supported");
        }
        if cmd.compressed {
            return fail("EINVAL:compressed data (o=z) is not supported");
        }
        let Some(raw) = std::str::from_utf8(base64)
            .ok()
            .and_then(inlineimg::decode_base64)
        else {
            return fail("EINVAL:the data is not valid base64");
        };
        let png = match cmd.format {
            100 => (inlineimg::pixel_size(&raw).is_some() && raw.starts_with(b"\x89PNG"))
                .then_some(raw),
            24 | 32 => {
                let bpp = if cmd.format == 24 { 3 } else { 4 };
                (cmd.width > 0
                    && cmd.height > 0
                    && cmd.width.saturating_mul(cmd.height).saturating_mul(bpp) as usize
                        == raw.len())
                .then(|| png_from_raw(cmd.width, cmd.height, bpp as usize, &raw))
            }
            _ => None,
        };
        let Some(png) = png.filter(|p| p.len() <= MAX_IMAGE + 1024) else {
            return fail("EINVAL:unsupported or malformed image");
        };
        if cmd.action == b'q' {
            return Outcome {
                reply: reply(&cmd, true, ""),
                ..Default::default()
            };
        }
        let png = Arc::new(png);
        let req = request(&png, &cmd);
        let id = (cmd.id != 0).then_some(cmd.id);
        if let Some(id) = id {
            self.forget(id);
            self.seq += 1;
            self.kept += png.len();
            self.stored.insert(
                id,
                Stored {
                    png: png.clone(),
                    seq: self.seq,
                },
            );
            self.evict();
        }
        Outcome {
            place: (cmd.action == b'T').then_some((req, !cmd.keep_cursor, id)),
            reply: reply(&cmd, true, ""),
            ..Default::default()
        }
    }

    fn forget(&mut self, id: u32) {
        if let Some(old) = self.stored.remove(&id) {
            self.kept -= old.png.len();
        }
    }
    fn evict(&mut self) {
        while self.kept > MAX_STORED {
            let Some(oldest) = self
                .stored
                .iter()
                .min_by_key(|(_, s)| s.seq)
                .map(|(i, _)| *i)
            else {
                break;
            };
            self.forget(oldest);
        }
    }
}

fn request(png: &Arc<Vec<u8>>, cmd: &Cmd) -> Request {
    let cells = |n: u32| if n > 0 { Dim::Cells(n) } else { Dim::Auto };
    Request {
        bytes: png.as_ref().clone(),
        width: cells(cmd.cols),
        height: cells(cmd.rows),
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}
/// A PNG holding raw `bpp`-byte pixels (3: RGB, 4: RGBA), written with uncompressed deflate blocks.
pub fn png_from_raw(w: u32, h: u32, bpp: usize, pixels: &[u8]) -> Vec<u8> {
    let stride = w as usize * bpp;
    let mut scan = Vec::with_capacity((stride + 1) * h as usize);
    for row in pixels.chunks(stride) {
        scan.push(0);
        scan.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = scan.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend((block.len() as u16).to_le_bytes());
        z.extend((!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend(adler32(&scan).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend(crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, if bpp == 4 { 6 } else { 2 }, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    // A 1x1 PNG.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    fn apc(s: &str) -> Vec<u8> {
        format!("\x1b_G{s}\x1b\\").into_bytes()
    }
    #[test]
    fn apc_sequences_are_cut_out_even_across_reads() {
        let mut f = ApcFilter::default();
        let mut stream = b"ab".to_vec();
        stream.extend(apc("a=T;xyz"));
        stream.extend(b"cd\x1b[31mred");
        let mut got = Vec::new();
        // One byte at a time is the hardest split.
        for b in &stream {
            got.extend(f.feed(&[*b]));
        }
        let mut data = Vec::new();
        let mut bodies = Vec::new();
        for p in got {
            match p {
                Piece::Data(d) => data.extend(d),
                Piece::Apc(a) => bodies.push(a),
            }
        }
        assert_eq!(data, b"abcd\x1b[31mred");
        assert_eq!(bodies, [b"Ga=T;xyz".to_vec()]);
        // Whole-buffer feed gives the same pieces in order.
        let pieces = ApcFilter::default().feed(&stream);
        assert_eq!(
            pieces,
            [
                Piece::Data(b"ab".to_vec()),
                Piece::Apc(b"Ga=T;xyz".to_vec()),
                Piece::Data(b"cd\x1b[31mred".to_vec())
            ]
        );
    }
    #[test]
    fn other_escapes_and_double_escapes_pass_through_unchanged() {
        let mut f = ApcFilter::default();
        let pieces = f.feed(b"\x1b\x1b[1m\x1b]0;t\x07\x1b");
        let mut data = Vec::new();
        for p in pieces {
            if let Piece::Data(d) = p {
                data.extend(d);
            }
        }
        // The trailing ESC is held until the next byte says what it starts.
        assert_eq!(data, b"\x1b\x1b[1m\x1b]0;t\x07");
        assert_eq!(f.feed(b"[0m"), [Piece::Data(b"\x1b[0m".to_vec())]);
    }
    #[test]
    fn an_apc_that_is_too_big_is_skipped_to_its_end() {
        let mut f = ApcFilter::default();
        let mut stream = b"\x1b_G".to_vec();
        stream.extend(std::iter::repeat_n(b'x', MAX_APC + 10));
        stream.extend(b"\x1b\\ok");
        let pieces = f.feed(&stream);
        assert_eq!(pieces, [Piece::Data(b"ok".to_vec())]);
    }
    #[test]
    fn keys_are_parsed() {
        let (c, p) = parse(b"Ga=T,f=100,i=7,c=10,r=3,q=2,C=1;QUJD").unwrap();
        assert_eq!(
            (
                c.action,
                c.format,
                c.id,
                c.cols,
                c.rows,
                c.quiet,
                c.keep_cursor
            ),
            (b'T', 100, 7, 10, 3, 2, true)
        );
        assert_eq!(p, b"QUJD");
        assert!(parse(b"Xnope").is_none());
    }
    #[test]
    fn transmit_and_display_places_the_image_and_replies_ok() {
        let mut g = KittyGfx::default();
        let out = g.command(format!("Ga=T,f=100,i=5,c=12;{PNG}").as_bytes());
        let (req, move_cursor, id) = out.place.expect("placed");
        assert!(req.bytes.starts_with(b"\x89PNG") && move_cursor && id == Some(5));
        assert_eq!(req.width, Dim::Cells(12));
        assert_eq!(out.reply.as_deref(), Some("\x1b_Gi=5;OK\x1b\\"));
        // q=1 hides the OK; no id means no reply at all.
        assert!(g
            .command(format!("Ga=T,f=100,i=5,q=1;{PNG}").as_bytes())
            .reply
            .is_none());
        assert!(g
            .command(format!("Ga=T,f=100;{PNG}").as_bytes())
            .reply
            .is_none());
        // C=1 leaves the cursor alone.
        let out = g.command(format!("Ga=T,f=100,C=1;{PNG}").as_bytes());
        assert!(!out.place.unwrap().1);
    }
    #[test]
    fn transmit_then_put_and_delete() {
        let mut g = KittyGfx::default();
        let out = g.command(format!("Ga=t,f=100,i=9;{PNG}").as_bytes());
        assert!(out.place.is_none() && out.reply.is_some());
        let out = g.command(b"Ga=p,i=9,r=2");
        let (req, _, id) = out.place.unwrap();
        assert_eq!((req.height, id), (Dim::Cells(2), Some(9)));
        assert!(g.command(b"Ga=p,i=404").reply.unwrap().contains("ENOENT"));
        assert_eq!(g.command(b"Ga=d,d=i,i=9").delete, Some(Some(9)));
        assert_eq!(g.command(b"Ga=d").delete, Some(None));
        // Lower-case `d` keeps the stored image for later; upper-case frees it.
        assert!(g.command(b"Ga=p,i=9").place.is_some());
        g.command(b"Ga=d,d=I,i=9");
        assert!(g.command(b"Ga=p,i=9").place.is_none());
    }
    #[test]
    fn chunks_are_joined_and_only_the_last_one_acts() {
        let mut g = KittyGfx::default();
        let (a, b) = PNG.split_at(24);
        assert_eq!(
            g.command(format!("Ga=T,f=100,i=3,m=1;{a}").as_bytes()),
            Outcome::default()
        );
        let out = g.command(format!("Gm=0;{b}").as_bytes());
        assert!(out.place.is_some(), "{out:?}");
        assert_eq!(out.reply.as_deref(), Some("\x1b_Gi=3;OK\x1b\\"));
    }
    #[test]
    fn raw_pixels_become_a_png_and_bad_requests_get_an_error() {
        let mut g = KittyGfx::default();
        // 2x1 RGB: six bytes.
        let out = g.command(b"Ga=T,f=24,s=2,v=1,i=1;AAAA/wAA");
        let (req, ..) = out.place.expect("raw RGB is accepted");
        assert_eq!(inlineimg::pixel_size(&req.bytes), Some((2, 1)));
        // Wrong size for the stated dimensions.
        assert!(g
            .command(b"Ga=T,f=24,s=9,v=9,i=2;AAAA")
            .reply
            .unwrap()
            .contains("EINVAL"));
        // Files, shared memory and zlib are refused out loud.
        assert!(g
            .command(b"Ga=T,f=100,t=f,i=3;L3RtcC94")
            .reply
            .unwrap()
            .contains("only direct"));
        assert!(g
            .command(b"Ga=T,f=100,o=z,i=4;AAAA")
            .reply
            .unwrap()
            .contains("not supported"));
        assert!(g
            .command(b"Ga=T,f=100,i=5;***")
            .reply
            .unwrap()
            .contains("base64"));
        // A query answers OK for something it can show, without storing it.
        let out = g.command(format!("Ga=q,f=100,i=31;{PNG}").as_bytes());
        assert_eq!(out.reply.as_deref(), Some("\x1b_Gi=31;OK\x1b\\"));
        assert!(out.place.is_none() && g.command(b"Ga=p,i=31").place.is_none());
    }
    #[test]
    fn the_generated_png_has_valid_checksums() {
        let png = png_from_raw(2, 2, 4, &[255; 16]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // Every chunk's CRC matches its contents.
        let mut i = 8;
        let mut kinds = Vec::new();
        while i < png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let body = &png[i + 4..i + 8 + len];
            let crc = u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap());
            assert_eq!(crc32(body), crc);
            kinds.push(String::from_utf8_lossy(&body[..4]).into_owned());
            i += 12 + len;
        }
        assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}

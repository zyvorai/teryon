//! Sending a pane's screen as a difference from the last one the client received, instead of the
//! whole screen every time, with a checksum so either side notices if the two ever disagree and
//! falls back to a full snapshot.
use crate::protocol::PaneView;
use std::collections::BTreeMap;

/// A full snapshot is sent at least this often, so a bug cannot drift for long.
const KEYFRAME_EVERY: u32 = 500;

/// A checksum of what a client can reproduce: text and attributes (the formatted contents) and the
/// cursor. Whether the program is on the alternate screen is not sent, so it is not included.
pub fn digest(screen: &vt100::Screen) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(&screen.contents_formatted());
    let (row, col) = screen.cursor_position();
    eat(&row.to_le_bytes());
    eat(&col.to_le_bytes());
    eat(&[u8::from(screen.hide_cursor())]);
    h
}

/// How many lines `cur` is `prev` scrolled up by, if it looks like output scrolling past, so the
/// client can be told to scroll instead of being sent every moved row again. The shift under which
/// most non-empty rows line up wins (the rows near the bottom usually differ: the old cursor line
/// just gained text), and the ordinary diff then fixes whatever still differs.
fn scroll_hint(prev: &vt100::Screen, cur: &vt100::Screen) -> Option<u16> {
    let (rows, cols) = cur.size();
    let a: Vec<Vec<u8>> = prev.rows_formatted(0, cols).collect();
    let b: Vec<Vec<u8>> = cur.rows_formatted(0, cols).collect();
    let mut best: Option<(usize, u16)> = None;
    for k in 1..rows {
        let n = usize::from(rows - k);
        let matches = (0..n)
            .filter(|&i| !b[i].is_empty() && b[i] == a[i + usize::from(k)])
            .count();
        if matches >= 2 && matches * 2 >= n && best.is_none_or(|(m, _)| matches > m) {
            best = Some((matches, k));
        }
    }
    best.map(|(_, k)| k)
}

fn mirror_of(rows: u16, cols: u16, full: &[u8]) -> vt100::Parser {
    let mut p = vt100::Parser::new(rows, cols, 0);
    p.process(full);
    p
}

/// Everything the daemon remembers about what one client holds for one pane.
#[derive(Default)]
pub struct PaneSync {
    pub mirror: Option<ServerMirror>,
    meta: Option<String>,
}

impl PaneSync {
    /// Leave out the pane details (`same_meta`) when they are what this client was last sent;
    /// otherwise remember them as sent. A changed screen does not matter: the details are their own.
    pub fn trim(&mut self, view: &mut PaneView) {
        let key = format!(
            "{}\0{}\0{:?}\0{:?}\0{:?}\0{}\0{}",
            view.title, view.cwd, view.pid, view.fg, view.ports, view.first_line, view.top_line
        );
        if self.meta.as_deref() == Some(key.as_str()) {
            view.title.clear();
            view.cwd.clear();
            view.pid = None;
            view.fg = None;
            view.ports.clear();
            view.first_line = 0;
            view.top_line = 0;
            view.same_meta = true;
        } else {
            self.meta = Some(key);
        }
    }
}

/// What the daemon has sent one client for one pane: a copy of that client's screen.
pub struct ServerMirror {
    parser: vt100::Parser,
    revision: u64,
    since_keyframe: u32,
}

/// The bytes to send for a changed screen, and how the client should read them.
pub struct Encoded {
    pub bytes: Vec<u8>,
    /// Set when `bytes` is a diff to apply on top of this revision; `None` for a full snapshot.
    pub diff_from: Option<u64>,
    pub digest: Option<u64>,
}

impl ServerMirror {
    /// Encode `screen` (at `revision`) for a client that says it holds `known`.
    pub fn encode(
        mirror: &mut Option<Self>,
        screen: &vt100::Screen,
        revision: u64,
        known: Option<u64>,
    ) -> Encoded {
        if let Some(m) = mirror.as_mut() {
            let same_size = m.parser.screen().size() == screen.size();
            if same_size && known == Some(m.revision) && m.since_keyframe < KEYFRAME_EVERY {
                // Scrolling output moves every row up; say so once instead of resending each row.
                let mut diff = Vec::new();
                if let Some(k) = scroll_hint(m.parser.screen(), screen) {
                    diff.extend_from_slice(format!("\x1b[0m\x1b[{k}S").as_bytes());
                    m.parser.process(&diff);
                }
                let rest = screen.state_diff(m.parser.screen());
                m.parser.process(&rest);
                diff.extend_from_slice(&rest);
                let base = m.revision;
                let want = digest(screen);
                // A diff that does not reproduce the screen is never sent.
                if digest(m.parser.screen()) == want {
                    m.revision = revision;
                    m.since_keyframe += 1;
                    return Encoded {
                        bytes: diff,
                        diff_from: Some(base),
                        digest: Some(want),
                    };
                }
            }
        }
        let full = screen.state_formatted();
        let (rows, cols) = screen.size();
        *mirror = Some(Self {
            parser: mirror_of(rows, cols, &full),
            revision,
            since_keyframe: 0,
        });
        Encoded {
            bytes: full,
            diff_from: None,
            digest: None,
        }
    }
}

/// The pane details a client last received in full, to fill in the replies that leave them out.
#[derive(Default)]
pub struct MetaCache {
    panes: BTreeMap<u64, PaneView>,
}

impl MetaCache {
    /// Make `view` complete. `false` when it leaves details out that this client never received.
    pub fn complete(&mut self, view: &mut PaneView) -> bool {
        if !view.same_meta {
            let mut keep = PaneView {
                id: view.id,
                ..Default::default()
            };
            keep.title.clone_from(&view.title);
            keep.cwd.clone_from(&view.cwd);
            keep.pid = view.pid;
            keep.fg.clone_from(&view.fg);
            keep.ports.clone_from(&view.ports);
            keep.first_line = view.first_line;
            keep.top_line = view.top_line;
            self.panes.insert(view.id, keep);
            return true;
        }
        let Some(m) = self.panes.get(&view.id) else {
            return false;
        };
        view.title.clone_from(&m.title);
        view.cwd.clone_from(&m.cwd);
        view.pid = m.pid;
        view.fg.clone_from(&m.fg);
        view.ports.clone_from(&m.ports);
        view.first_line = m.first_line;
        view.top_line = m.top_line;
        view.same_meta = false;
        true
    }
    pub fn retain(&mut self, live: impl Fn(u64) -> bool) {
        self.panes.retain(|id, _| live(*id));
    }
}

/// The client side: one screen per pane, kept in step with the daemon's diffs.
#[derive(Default)]
pub struct ClientMirrors {
    panes: BTreeMap<u64, (u64, vt100::Parser)>,
}

/// What `ClientMirrors::apply` made of an update.
#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    /// The update carried no screen, or a full one that is now remembered.
    Plain,
    /// A diff was applied; the full screen to hand on is returned.
    Rebuilt(Vec<u8>),
    /// The diff did not fit what this client holds: forget the pane's revision so the next poll
    /// brings a full snapshot.
    Resync,
}

impl ClientMirrors {
    pub fn apply(
        &mut self,
        pane: u64,
        revision: u64,
        size: (u16, u16),
        screen: Option<&[u8]>,
        diff_from: Option<u64>,
        digest_want: Option<u64>,
    ) -> Applied {
        let Some(bytes) = screen else {
            return Applied::Plain;
        };
        let Some(base) = diff_from else {
            self.panes
                .insert(pane, (revision, mirror_of(size.0, size.1, bytes)));
            return Applied::Plain;
        };
        let ok = match self.panes.get_mut(&pane) {
            Some((rev, parser)) if *rev == base && parser.screen().size() == size => {
                parser.process(bytes);
                *rev = revision;
                digest_want.is_none_or(|d| d == digest(parser.screen()))
            }
            _ => false,
        };
        if !ok {
            self.panes.remove(&pane);
            return Applied::Resync;
        }
        Applied::Rebuilt(self.panes[&pane].1.screen().state_formatted())
    }
    pub fn retain(&mut self, live: impl Fn(u64) -> bool) {
        self.panes.retain(|id, _| live(*id));
    }
    pub fn clear(&mut self) {
        self.panes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `chunks` to a "daemon" screen and a client through diffs; the client must always match.
    fn run(chunks: &[&str], size: (u16, u16)) -> (usize, usize) {
        let mut daemon = vt100::Parser::new(size.0, size.1, 1000);
        let mut mirror = None;
        let mut client = ClientMirrors::default();
        let (mut sent, mut full) = (0, 0);
        let mut known = None;
        for (i, chunk) in chunks.iter().enumerate() {
            daemon.process(chunk.as_bytes());
            let rev = i as u64 + 1;
            let e = ServerMirror::encode(&mut mirror, daemon.screen(), rev, known);
            sent += e.bytes.len();
            full += daemon.screen().state_formatted().len();
            let got = client.apply(1, rev, size, Some(&e.bytes), e.diff_from, e.digest);
            let rebuilt = match got {
                Applied::Plain => e.bytes.clone(),
                Applied::Rebuilt(b) => b,
                Applied::Resync => panic!("diff {i} did not fit"),
            };
            let mut seen = vt100::Parser::new(size.0, size.1, 0);
            seen.process(&rebuilt);
            assert_eq!(
                digest(seen.screen()),
                digest(daemon.screen()),
                "step {i}: {chunk:?}"
            );
            assert_eq!(seen.screen().contents(), daemon.screen().contents());
            known = Some(rev);
        }
        (sent, full)
    }

    #[test]
    fn typing_scrolling_colors_and_full_screen_programs_round_trip() {
        let mut steps: Vec<String> = vec!["$ ".into()];
        steps.extend("ls -la\r\n".chars().map(|c| c.to_string()));
        for n in 0..60 {
            steps.push(format!("line {n} \x1b[3{}mcolored\x1b[0m text\r\n", n % 8));
        }
        steps.push("\x1b[?1049h\x1b[2J\x1b[H\x1b[7mmenu\x1b[0m\x1b[5;10Hbody\x1b[?25l".into());
        steps.push("\x1b[3;3H\x1b[1;4;38;2;10;200;30mxyz\x1b[0m".into());
        steps.push("\x1b[?1049l\x1b[?25h".into());
        steps.push("日本語 wide \x1b[1mbold\x1b[0m e\u{301}\r\n".into());
        steps.push("\x1b]0;a title\x07".into());
        steps.push("\x1b[2J\x1b[H".into());
        let refs: Vec<&str> = steps.iter().map(String::as_str).collect();
        run(&refs, (24, 80));
    }
    #[test]
    fn small_changes_cost_far_less_than_the_whole_screen() {
        let mut steps = vec!["\x1b[2J\x1b[H".to_string()];
        for n in 0..24 {
            steps.push(format!("row {n}: some text to fill the screen up well\r\n"));
        }
        for n in 0..100 {
            steps.push(format!("\x1b[24;1Hprompt {n}> "));
        }
        let refs: Vec<&str> = steps.iter().map(String::as_str).collect();
        let (sent, full) = run(&refs, (24, 80));
        assert!(
            sent * 4 < full,
            "sent {sent} bytes against {full} for full snapshots"
        );
    }
    #[test]
    fn scrolling_output_costs_a_scroll_and_the_new_rows_not_every_row_again() {
        let mut steps = vec!["\x1b[2J\x1b[H".to_string()];
        for n in 0..200 {
            steps.push(format!(
                "build step {n:04} \x1b[3{}mcompiling crate-{n} with some flags and a long path\x1b[0m\r\n",
                n % 8
            ));
        }
        let refs: Vec<&str> = steps.iter().map(String::as_str).collect();
        let (sent, full) = run(&refs, (24, 80));
        assert!(
            sent * 5 < full,
            "sent {sent} bytes against {full} for full snapshots"
        );
    }
    fn view(title: &str, fg: &str, revision: u64) -> PaneView {
        PaneView {
            id: 1,
            title: title.into(),
            cwd: "/work/project".into(),
            pid: Some(77),
            revision,
            fg: Some(fg.into()),
            ports: vec![3000],
            first_line: 4,
            top_line: 9,
            ..Default::default()
        }
    }
    #[test]
    fn unchanged_pane_details_are_left_out_and_filled_back_in() {
        let (mut sync, mut cache) = (PaneSync::default(), MetaCache::default());
        // The first reply carries everything.
        let mut v = view("zsh", "-zsh", 1);
        sync.trim(&mut v);
        assert!(!v.same_meta && v.title == "zsh");
        assert!(cache.complete(&mut v));
        // A quiet one carries almost nothing, and comes back complete.
        let mut quiet = view("zsh", "-zsh", 1);
        sync.trim(&mut quiet);
        assert!(quiet.same_meta && quiet.title.is_empty() && quiet.cwd.is_empty());
        assert!(quiet.fg.is_none() && quiet.ports.is_empty() && quiet.pid.is_none());
        let slim = serde_json::to_string(&quiet).unwrap().len();
        let whole = serde_json::to_string(&view("zsh", "-zsh", 1))
            .unwrap()
            .len();
        assert!(slim * 10 < whole * 7, "slim {slim} bytes against {whole}");
        assert!(cache.complete(&mut quiet));
        let want = view("zsh", "-zsh", 1);
        assert_eq!(
            (&quiet.title, &quiet.cwd, quiet.pid, &quiet.fg, &quiet.ports),
            (&want.title, &want.cwd, want.pid, &want.fg, &want.ports)
        );
        assert_eq!((quiet.first_line, quiet.top_line), (4, 9));
        assert!(!quiet.same_meta);
        // A change in any detail sends them all again; so does a changed screen.
        let mut moved = view("zsh", "vim notes.md", 1);
        sync.trim(&mut moved);
        assert!(!moved.same_meta);
        assert!(cache.complete(&mut moved));
        // And once sent, a changed screen on its own does not bring them back.
        let mut next = view("zsh", "vim notes.md", 2);
        sync.trim(&mut next);
        assert!(next.same_meta);
    }
    #[test]
    fn a_client_without_the_details_asks_to_start_over() {
        let mut cache = MetaCache::default();
        let mut quiet = PaneView {
            id: 5,
            same_meta: true,
            ..Default::default()
        };
        assert!(!cache.complete(&mut quiet));
    }
    #[test]
    fn a_client_that_missed_an_update_asks_for_a_full_snapshot() {
        let mut daemon = vt100::Parser::new(5, 20, 0);
        let mut mirror = None;
        let mut client = ClientMirrors::default();
        daemon.process(b"one");
        let e = ServerMirror::encode(&mut mirror, daemon.screen(), 1, None);
        assert_eq!(
            client.apply(1, 1, (5, 20), Some(&e.bytes), e.diff_from, e.digest),
            Applied::Plain
        );
        daemon.process(b"\r\ntwo");
        let _lost = ServerMirror::encode(&mut mirror, daemon.screen(), 2, Some(1));
        daemon.process(b"\r\nthree");
        let e = ServerMirror::encode(&mut mirror, daemon.screen(), 3, Some(2));
        assert_eq!(e.diff_from, Some(2));
        // The client never saw revision 2, so a diff on top of it cannot apply.
        assert_eq!(
            client.apply(1, 3, (5, 20), Some(&e.bytes), e.diff_from, e.digest),
            Applied::Resync
        );
        // With nothing known, the daemon sends everything and the client is whole again.
        let e = ServerMirror::encode(&mut mirror, daemon.screen(), 3, None);
        assert_eq!(e.diff_from, None);
        assert_eq!(
            client.apply(1, 3, (5, 20), Some(&e.bytes), e.diff_from, e.digest),
            Applied::Plain
        );
    }
    #[test]
    fn a_resize_or_a_wrong_checksum_forces_a_full_snapshot() {
        let mut daemon = vt100::Parser::new(5, 20, 0);
        let mut mirror = None;
        daemon.process(b"hello");
        let _ = ServerMirror::encode(&mut mirror, daemon.screen(), 1, None);
        daemon.screen_mut().set_size(6, 30);
        daemon.process(b" more");
        let e = ServerMirror::encode(&mut mirror, daemon.screen(), 2, Some(1));
        assert_eq!(e.diff_from, None, "a different size is never diffed");
        // A corrupted diff is detected by the checksum on the client.
        let mut client = ClientMirrors::default();
        let mut mirror = None;
        let e = ServerMirror::encode(&mut mirror, daemon.screen(), 3, None);
        client.apply(1, 3, (6, 30), Some(&e.bytes), e.diff_from, e.digest);
        daemon.process(b" tail");
        let e = ServerMirror::encode(&mut mirror, daemon.screen(), 4, Some(3));
        assert_eq!(
            client.apply(
                1,
                4,
                (6, 30),
                Some(&e.bytes),
                e.diff_from,
                Some(e.digest.unwrap() ^ 1)
            ),
            Applied::Resync
        );
    }
}

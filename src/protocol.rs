use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
pub const MAX_INPUT: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Hello {
        version: u32,
    },
    List,
    Remove {
        workspace: String,
        force: bool,
    },
    Attach {
        workspace: String,
        readonly: bool,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        /// Take the workspace's controller seat from whoever holds it. The previous controller keeps
        /// its connection but can only watch; older daemons ignore this and refuse the attach.
        #[serde(default)]
        takeover: bool,
    },
    Poll {
        known: BTreeMap<String, u64>,
    },
    Input {
        pane: u64,
        seq: u64,
        data: Vec<u8>,
    },
    Resize {
        pane: u64,
        rows: u16,
        cols: u16,
    },
    NewPane {
        cwd: Option<String>,
        command: Vec<String>,
        rows: u16,
        cols: u16,
    },
    Rename {
        pane: u64,
        title: String,
    },
    /// Close a pane but keep its shell alive for a short while, so `Reopen` can bring it back.
    SoftClose {
        pane: u64,
    },
    /// Bring back the pane closed last with `SoftClose`, if it is still within the grace period.
    Reopen,
    Close {
        pane: u64,
    },
    Capture {
        pane: u64,
    },
    /// The bytes of an inline image shown in a pane (see `PaneView::images`).
    Image {
        pane: u64,
        id: u64,
    },
    Info,
    Shutdown {
        force: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub name: String,
    pub panes: usize,
    pub controlled: bool,
}

/// A shell-integration event (OSC 133) at an absolute line: `A` prompt start, `C` command output
/// start, `D` command finished (with exit status and duration when known).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub kind: char,
    pub line: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// Unix time in seconds.
    #[serde(default)]
    pub at: u64,
    /// The command line, on a `C` mark when the shell integration sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PaneView {
    pub id: u64,
    pub title: String,
    pub cwd: String,
    pub pid: Option<u32>,
    pub rows: u16,
    pub cols: u16,
    pub revision: u64,
    /// Complete ANSI terminal state when changed. None means retain the old state.
    pub screen: Option<Vec<u8>>,
    pub exited: Option<u32>,
    /// Shell-integration marks, sent together with `screen`; old daemons never send them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marks: Option<Vec<Mark>>,
    /// Absolute number of the first retained history line, and of the top visible row.
    #[serde(default)]
    pub first_line: u64,
    #[serde(default)]
    pub top_line: u64,
    /// Command line of the pane's foreground process (e.g. `claude` or `node /…/codex`); old daemons send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    /// TCP ports the pane's processes listen on; old daemons send none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<u16>,
    /// Inline images on the screen and in history, sent together with `screen`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<Placement>>,
    /// OSC 8 hyperlinks on the visible screen, sent together with `screen`; old daemons send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<LinkSpan>>,
    /// Kitty keyboard protocol flags in force for the pane's current screen; old daemons send 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub kbd: u8,
}
fn is_zero(n: &u8) -> bool {
    *n == 0
}
/// A run of cells on one screen row that a program marked as a hyperlink (`end` is exclusive).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkSpan {
    pub row: u16,
    pub start: u16,
    pub end: u16,
    pub url: String,
}
/// An inline image: its id (for `Request::Image`), the absolute line of its top row, and the
/// cells it covers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub id: u64,
    pub line: u64,
    pub col: u16,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Hello {
        version: u32,
        build: String,
        instance: String,
    },
    Workspaces {
        items: Vec<WorkspaceInfo>,
    },
    Attached {
        workspace: String,
        readonly: bool,
    },
    State {
        panes: Vec<PaneView>,
    },
    Created {
        pane: u64,
    },
    Captured {
        text: String,
        /// Absolute number of the first line of `text` (0 from daemons that do not know).
        #[serde(default)]
        first_line: u64,
    },
    ImageData {
        id: u64,
        /// The image file, base64 encoded.
        data: String,
    },
    Info {
        os: String,
        arch: String,
        sessions: usize,
        panes: usize,
        pid: u32,
        /// Seconds the daemon has been running; old daemons send none.
        #[serde(default)]
        uptime: u64,
        /// Load, memory and thermal state of the daemon's machine; old daemons send none.
        #[serde(default)]
        stats: Option<crate::hoststats::HostStats>,
        /// The daemon understands `SoftClose` and `Reopen`.
        #[serde(default)]
        undo: bool,
    },
    Ack {
        seq: Option<u64>,
    },
    Error {
        message: String,
    },
}

pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<()> {
    let data = serde_json::to_vec(value)?;
    if data.len() > MAX_FRAME {
        bail!("protocol frame exceeds limit");
    }
    writer.write_all(&data)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Incremental bounded reading; never allocate an unbounded line from a peer.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            bail!("truncated protocol frame");
        }
        let end = available.iter().position(|b| *b == b'\n');
        let n = end.map_or(available.len(), |n| n + 1);
        if line.len() + n > MAX_FRAME {
            bail!("protocol frame exceeds limit");
        }
        line.extend_from_slice(&available[..n]);
        reader.consume(n);
        if end.is_some() {
            break;
        }
    }
    Ok(Some(
        serde_json::from_slice(&line).context("invalid protocol JSON")?,
    ))
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

pub fn valid_size(rows: u16, cols: u16) -> bool {
    (2..=160).contains(&rows) && (10..=400).contains(&cols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn frames_roundtrip_and_eof() {
        let mut buf = Vec::new();
        write_frame(
            &mut buf,
            &Request::Input {
                pane: 1,
                seq: 7,
                data: vec![0, 27, 255, 10],
            },
        )
        .unwrap();
        let mut reader = Cursor::new(buf);
        assert!(matches!(
            read_frame::<Request>(&mut reader).unwrap(),
            Some(Request::Input { seq: 7, .. })
        ));
        assert!(read_frame::<Request>(&mut reader).unwrap().is_none());
    }
    #[test]
    fn rejects_truncated_and_oversized() {
        assert!(read_frame::<Request>(&mut Cursor::new(b"{\"op\":\"list\"}")).is_err());
        assert!(read_frame::<Request>(&mut Cursor::new(vec![b'x'; MAX_FRAME + 1])).is_err());
    }
    #[test]
    fn validates_names_and_geometry() {
        assert!(valid_name("fluxvm-build_2"));
        for n in ["", "../../x", "a;whoami", "has space", "💥"] {
            assert!(!valid_name(n));
        }
        assert!(!valid_size(0, 80));
        assert!(!valid_size(500, 800));
        assert!(valid_size(24, 80));
    }
    #[test]
    fn pane_views_and_captures_stay_compatible_with_older_peers() {
        // A daemon from before shell integration: the new client sees empty extras.
        let old = r#"{"id":1,"title":"t","cwd":"/","pid":null,"rows":24,"cols":80,"revision":3,"screen":null,"exited":null}"#;
        let v: PaneView = serde_json::from_str(old).unwrap();
        assert!(v.marks.is_none() && v.first_line == 0 && v.top_line == 0 && v.fg.is_none());
        assert!(
            v.links.is_none(),
            "a daemon from before OSC 8 sends no links"
        );
        // A new daemon talking to an older client, whose struct knows none of the new fields.
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct OldPaneView {
            id: u64,
            title: String,
            cwd: String,
            pid: Option<u32>,
            rows: u16,
            cols: u16,
            revision: u64,
            screen: Option<Vec<u8>>,
            exited: Option<u32>,
        }
        let new = serde_json::to_string(&PaneView {
            marks: Some(vec![Mark {
                kind: 'D',
                line: 3,
                exit: Some(1),
                ms: Some(5),
                at: 9,
                cmd: None,
            }]),
            first_line: 2,
            top_line: 5,
            links: Some(vec![LinkSpan {
                row: 1,
                start: 2,
                end: 5,
                url: "https://a.example".into(),
            }]),
            ..Default::default()
        })
        .unwrap();
        serde_json::from_str::<OldPaneView>(&new).unwrap();
        let back: PaneView = serde_json::from_str(&new).unwrap();
        assert_eq!(back.marks.unwrap()[0].exit, Some(1));
        assert_eq!(back.links.unwrap()[0].url, "https://a.example");
        let captured: Response = serde_json::from_str(r#"{"type":"captured","text":"x"}"#).unwrap();
        assert!(matches!(captured, Response::Captured { first_line: 0, .. }));
        // Unchanged polls carry no marks at all.
        let quiet = serde_json::to_string(&PaneView::default()).unwrap();
        assert!(!quiet.contains("marks"));
    }
    #[test]
    fn an_old_daemons_info_without_stats_still_parses() {
        let old = r#"{"type":"info","os":"macos","arch":"arm64","sessions":1,"panes":2,"pid":3,"uptime":4}"#;
        match serde_json::from_str::<Response>(old).unwrap() {
            Response::Info { stats, uptime, .. } => assert!(stats.is_none() && uptime == 4),
            other => panic!("{other:?}"),
        }
    }
}

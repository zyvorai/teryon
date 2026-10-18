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
    Close {
        pane: u64,
    },
    Capture {
        pane: u64,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    },
    Info {
        os: String,
        arch: String,
        sessions: usize,
        panes: usize,
        pid: u32,
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
}

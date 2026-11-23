//! "Show me this pane": a notification click asks the running desktop app to switch to the workspace
//! tab and pane the notification was about. The request is one small file in the private runtime
//! directory (`~/.teryon/run`), written by `teryon notify open` and taken by the desktop app, which
//! looks for it a few times a second. A request nobody takes within `MAX_AGE` is stale and ignored,
//! so a click while TERYON was closed does not steal focus minutes later.
use crate::util::runtime_dir;
use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// How long a request stays good.
pub const MAX_AGE: Duration = Duration::from_secs(60);

/// Where to show: a tab (`host`/`workspace`) and a pane in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub workspace: String,
    pub pane: u64,
}

fn path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("focus"))
}

/// The file's text: three tab-separated fields. Names never hold a tab or newline (`valid_name`).
pub fn encode(t: &Target) -> String {
    format!("{}\t{}\t{}\n", t.host, t.workspace, t.pane)
}

/// Parse `encode`'s text; `None` for anything else.
pub fn decode(text: &str) -> Option<Target> {
    let mut f = text.trim_end_matches('\n').split('\t');
    let (host, workspace, pane) = (f.next()?, f.next()?, f.next()?);
    if f.next().is_some() || host.is_empty() || workspace.is_empty() {
        return None;
    }
    Some(Target {
        host: host.into(),
        workspace: workspace.into(),
        pane: pane.parse().ok()?,
    })
}

/// Ask the desktop app to show `target`. The file is written whole and renamed into place, so a
/// reader never sees half of it.
pub fn request(target: &Target) -> Result<()> {
    let p = path()?;
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, encode(target))?;
    fs::rename(&tmp, &p)?;
    Ok(())
}

/// Take the pending request, if there is a fresh one. A stale or unreadable file is removed.
pub fn take() -> Option<Target> {
    let p = path().ok()?;
    let age = fs::metadata(&p)
        .ok()?
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok());
    let text = fs::read_to_string(&p).ok();
    let _ = fs::remove_file(&p);
    if age? > MAX_AGE {
        return None;
    }
    decode(&text?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_target_survives_the_file() {
        let t = Target {
            host: "local".into(),
            workspace: "demo".into(),
            pane: 3,
        };
        assert_eq!(decode(&encode(&t)), Some(t));
    }
    #[test]
    fn nonsense_is_refused() {
        for bad in ["", "a\tb", "a\tb\tx", "\tb\t1", "a\t\t1", "a\tb\t1\tc"] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }
}

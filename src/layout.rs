//! Saved pane layouts (titles and directories, never commands): the `layout` command's files and the
//! automatic per-workspace copy the desktop keeps so a rebooted host can be set up again.
use crate::{protocol::PaneView, util};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 64 * 1024;
pub const MAX_PANES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub version: u32,
    pub panes: Vec<LayoutPane>,
    /// How the panes were split (desktop pane tree), when they were; absent for the grid layouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Kid>,
}
/// One split of a pane tree: children side by side (`row`) or stacked, with their shares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub row: bool,
    pub kids: Vec<Kid>,
    /// Each child's share in thousandths (integers keep `Layout` comparable).
    pub w: Vec<u32>,
}
/// A child of a split: the pane at this index of `Layout::panes`, or a nested split.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Kid {
    Pane(usize),
    Split(Shape),
}
const MAX_DEPTH: usize = 16;

impl Kid {
    /// Adds every pane index below this node to `seen`; false on a malformed split, an index out of
    /// range, one used twice, or nesting deeper than `MAX_DEPTH`.
    fn check(&self, n: usize, seen: &mut [bool], depth: usize) -> bool {
        match self {
            Kid::Pane(i) => *i < n && !std::mem::replace(&mut seen[*i], true),
            Kid::Split(sp) => {
                depth < MAX_DEPTH
                    && sp.kids.len() >= 2
                    && sp.kids.len() == sp.w.len()
                    && sp.w.iter().map(|&x| u64::from(x)).sum::<u64>() > 0
                    && sp.kids.iter().all(|k| k.check(n, seen, depth + 1))
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPane {
    pub title: String,
    pub cwd: String,
}

impl Layout {
    pub fn from_panes(panes: &[PaneView]) -> Self {
        Self {
            version: 1,
            panes: panes
                .iter()
                .map(|p| LayoutPane {
                    title: p.title.clone(),
                    cwd: p.cwd.clone(),
                })
                .collect(),
            shape: None,
        }
    }
    pub fn with_shape(mut self, shape: Option<Kid>) -> Self {
        self.shape = shape;
        self
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.panes.is_empty() || self.panes.len() > MAX_PANES {
            bail!("unsupported or oversized layout");
        }
        if let Some(shape) = &self.shape {
            let mut seen = vec![false; self.panes.len()];
            if !shape.check(self.panes.len(), &mut seen, 0) || seen.contains(&false) {
                bail!("layout shape does not match its panes");
            }
        }
        Ok(())
    }
}
pub fn load(path: &Path) -> Result<Layout> {
    if std::fs::metadata(path)?.len() > MAX_BYTES {
        bail!("layout exceeds 64 KiB");
    }
    let layout: Layout = serde_json::from_slice(&std::fs::read(path)?)?;
    layout.validate()?;
    Ok(layout)
}
/// `~/.teryon/layouts/HOST-WORKSPACE.json` with anything unsafe in the names replaced.
pub fn auto_path(host: &str, workspace: &str) -> Result<PathBuf> {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    Ok(util::home()?.join(".teryon/layouts").join(format!(
        "{}-{}.json",
        clean(host),
        clean(workspace)
    )))
}
pub fn save(path: &Path, layout: &Layout) -> Result<()> {
    layout.validate()?;
    util::write_private_atomic(path, &serde_json::to_vec_pretty(layout)?)
}
/// A workspace that has only its first, untouched shell: the moment to offer restoring a saved layout.
pub fn is_fresh(panes: &[PaneView]) -> bool {
    panes.len() == 1 && panes[0].title == "Shell" && panes[0].exited.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pane(title: &str, cwd: &str) -> PaneView {
        PaneView {
            title: title.into(),
            cwd: cwd.into(),
            ..Default::default()
        }
    }
    #[test]
    fn layouts_round_trip_validate_and_name_files_safely() {
        let l = Layout::from_panes(&[pane("Shell", "/a"), pane("logs", "/var/log")]);
        assert_eq!(l.panes.len(), 2);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x/l.json");
        save(&path, &l).unwrap();
        assert_eq!(load(&path).unwrap(), l);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(Layout {
            version: 2,
            panes: l.panes.clone(),
            shape: None
        }
        .validate()
        .is_err());
        assert!(Layout {
            version: 1,
            panes: vec![],
            shape: None
        }
        .validate()
        .is_err());
        std::fs::write(&path, vec![b' '; 70_000]).unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, r#"{"version":1,"panes":[],"extra":1}"#).unwrap();
        assert!(load(&path).is_err());
        let p = auto_path("me@mini.local:22", "../etc").unwrap();
        let name = p.file_name().unwrap().to_str().unwrap().to_string();
        assert_eq!(name, "me_mini.local_22-.._etc.json");
        assert!(p.parent().unwrap().ends_with(".teryon/layouts"));
    }
    fn split(row: bool, kids: Vec<Kid>) -> Kid {
        let w = vec![500; kids.len()];
        Kid::Split(Shape { row, kids, w })
    }
    #[test]
    fn a_shape_round_trips_and_must_cover_each_pane_exactly_once() {
        let panes = [pane("a", "/"), pane("b", "/"), pane("c", "/")];
        let good = split(
            true,
            vec![Kid::Pane(0), split(false, vec![Kid::Pane(1), Kid::Pane(2)])],
        );
        let l = Layout::from_panes(&panes).with_shape(Some(good));
        l.validate().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        save(&path, &l).unwrap();
        assert_eq!(load(&path).unwrap(), l);
        // An old file without a shape still loads.
        std::fs::write(&path, r#"{"version":1,"panes":[{"title":"a","cwd":"/"}]}"#).unwrap();
        assert_eq!(load(&path).unwrap().shape, None);
        for bad in [
            split(true, vec![Kid::Pane(0), Kid::Pane(1)]),
            split(
                true,
                vec![Kid::Pane(0), Kid::Pane(0), Kid::Pane(1), Kid::Pane(2)],
            ),
            split(true, vec![Kid::Pane(0), Kid::Pane(1), Kid::Pane(9)]),
            split(true, vec![Kid::Pane(0)]),
            Kid::Split(Shape {
                row: true,
                kids: vec![Kid::Pane(0), Kid::Pane(1), Kid::Pane(2)],
                w: vec![0, 0, 0],
            }),
            Kid::Split(Shape {
                row: true,
                kids: vec![Kid::Pane(0), Kid::Pane(1), Kid::Pane(2)],
                w: vec![1, 1],
            }),
        ] {
            assert!(Layout::from_panes(&panes)
                .with_shape(Some(bad))
                .validate()
                .is_err());
        }
        let mut deep = split(true, vec![Kid::Pane(0), Kid::Pane(1), Kid::Pane(2)]);
        for _ in 0..20 {
            deep = split(false, vec![deep, Kid::Pane(0)]);
        }
        assert!(Layout::from_panes(&panes)
            .with_shape(Some(deep))
            .validate()
            .is_err());
    }
    #[test]
    fn only_a_single_untouched_shell_is_fresh() {
        assert!(is_fresh(&[pane("Shell", "/")]));
        assert!(!is_fresh(&[pane("build", "/")]));
        assert!(!is_fresh(&[pane("Shell", "/"), pane("Shell", "/")]));
        assert!(!is_fresh(&[PaneView {
            exited: Some(0),
            ..pane("Shell", "/")
        }]));
    }
}

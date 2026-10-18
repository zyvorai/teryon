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
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.panes.is_empty() || self.panes.len() > MAX_PANES {
            bail!("unsupported or oversized layout");
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
            panes: l.panes.clone()
        }
        .validate()
        .is_err());
        assert!(Layout {
            version: 1,
            panes: vec![]
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

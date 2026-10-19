//! Workspace recipes: a `.teryon.toml` that describes the panes of a project, started with `teryon up`.
use crate::protocol::{valid_name, Request, Response};
use anyhow::{bail, Result};
use serde::Deserialize;
use std::path::Path;

const MAX_PANES: usize = 12;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Workspace name; defaults to the name of the directory holding the recipe.
    pub workspace: Option<String>,
    #[serde(default, rename = "pane")]
    pub panes: Vec<RecipePane>,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecipePane {
    /// Title shown for the pane.
    pub name: Option<String>,
    /// Directory the shell starts in; relative paths are relative to the recipe file.
    pub cwd: Option<String>,
    /// Typed into the shell and run; the shell stays open when it ends.
    pub command: Option<String>,
}

pub fn parse(text: &str) -> Result<Recipe> {
    let r: Recipe = toml::from_str(text)?;
    if r.panes.is_empty() {
        bail!("the recipe needs at least one [[pane]]");
    }
    if r.panes.len() > MAX_PANES {
        bail!("a recipe may have at most {MAX_PANES} panes");
    }
    if let Some(w) = &r.workspace {
        if !valid_name(w) {
            bail!("workspace names use 1-64 ASCII letters, digits, '-' or '_'");
        }
    }
    for p in &r.panes {
        for (what, v) in [("name", &p.name), ("cwd", &p.cwd), ("command", &p.command)] {
            if v.as_deref()
                .is_some_and(|s| s.chars().any(char::is_control))
            {
                bail!("a pane {what} must not contain control characters or line breaks");
            }
        }
    }
    Ok(r)
}

/// The workspace to use: `--workspace`, the recipe's, or the recipe directory's name made valid.
pub fn workspace_name(recipe: &Recipe, flag: Option<&str>, dir: &Path) -> Result<String> {
    if let Some(w) = flag.or(recipe.workspace.as_deref()) {
        if !valid_name(w) {
            bail!("workspace names use 1-64 ASCII letters, digits, '-' or '_'");
        }
        return Ok(w.to_string());
    }
    let name: String = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if valid_name(&name) {
        Ok(name)
    } else {
        bail!("cannot derive a workspace name from the directory; set `workspace` or pass --workspace")
    }
}

fn start_dir(cwd: Option<&str>, base: &Path, local: bool) -> Option<String> {
    let c = cwd?;
    if !local || Path::new(c).is_absolute() || c.starts_with('~') {
        return Some(c.to_string());
    }
    Some(base.join(c).to_string_lossy().into_owned())
}

/// Create the workspace and its panes through `call`; returns the pane ids. Refuses a workspace that
/// already exists, so running it twice never doubles up.
pub fn up(
    call: &mut dyn FnMut(Request) -> Result<Response>,
    recipe: &Recipe,
    workspace: &str,
    base: &Path,
    local: bool,
) -> Result<Vec<u64>> {
    if let Response::Workspaces { items } = call(Request::List)? {
        if items.iter().any(|w| w.name == workspace) {
            bail!("workspace `{workspace}` already exists; attach to it instead");
        }
    }
    let first = start_dir(recipe.panes[0].cwd.as_deref(), base, local);
    match call(Request::Attach {
        workspace: workspace.into(),
        readonly: false,
        rows: 40,
        cols: 120,
        cwd: first,
    })? {
        Response::Attached { .. } => {}
        other => bail!("unexpected response: {other:?}"),
    }
    let mut ids = Vec::new();
    for (i, p) in recipe.panes.iter().enumerate() {
        // A new workspace starts with pane 1 in the first pane's directory.
        let id = if i == 0 {
            1
        } else {
            match call(Request::NewPane {
                cwd: start_dir(p.cwd.as_deref(), base, local),
                command: vec![],
                rows: 40,
                cols: 120,
            })? {
                Response::Created { pane } => pane,
                other => bail!("unexpected response: {other:?}"),
            }
        };
        if let Some(name) = &p.name {
            call(Request::Rename {
                pane: id,
                title: name.clone(),
            })?;
        }
        if let Some(cmd) = &p.command {
            let mut data = cmd.as_bytes().to_vec();
            data.push(b'\r');
            call(Request::Input {
                pane: id,
                seq: i as u64 + 1,
                data,
            })?;
        }
        ids.push(id);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_recipe_parses_and_is_validated() {
        let r = parse(
            "workspace = \"proj\"\n[[pane]]\nname = \"server\"\ncommand = \"cargo run\"\n[[pane]]\ncwd = \"web\"\n",
        )
        .unwrap();
        assert_eq!(r.panes.len(), 2);
        assert_eq!(r.panes[0].name.as_deref(), Some("server"));
        for bad in [
            "workspace = \"x\"\n",
            "[[pane]]\ncommand = \"a\\nb\"\n",
            "workspace = \"bad name\"\n[[pane]]\n",
            "[[pane]]\nbogus = 1\n",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let many = "[[pane]]\n".repeat(13);
        assert!(parse(&many).is_err());
    }
    #[test]
    fn the_workspace_name_comes_from_the_flag_the_recipe_or_the_directory() {
        let r = parse("[[pane]]\n").unwrap();
        let dir = Path::new("/home/me/My Project");
        assert_eq!(workspace_name(&r, None, dir).unwrap(), "My_Project");
        assert_eq!(workspace_name(&r, Some("x1"), dir).unwrap(), "x1");
        assert!(workspace_name(&r, Some("a b"), dir).is_err());
        assert!(workspace_name(&r, None, Path::new("/")).is_err());
        let r = parse("workspace = \"named\"\n[[pane]]\n").unwrap();
        assert_eq!(workspace_name(&r, None, dir).unwrap(), "named");
    }
    #[test]
    fn relative_directories_resolve_against_the_recipe_only_on_this_machine() {
        let base = Path::new("/p");
        assert_eq!(
            start_dir(Some("web"), base, true).as_deref(),
            Some("/p/web")
        );
        assert_eq!(start_dir(Some("/x"), base, true).as_deref(), Some("/x"));
        assert_eq!(start_dir(Some("web"), base, false).as_deref(), Some("web"));
        assert_eq!(start_dir(None, base, true), None);
    }
}

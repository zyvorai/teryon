//! Workspace recipes: a `.teryon.toml` that describes the panes of a project, started with `teryon up`.
use crate::protocol::{valid_name, Request, Response};
use anyhow::{bail, Result};
use serde::Deserialize;
use std::path::Path;
use std::time::{Duration, Instant};

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
    /// Name of a pane that must be ready before this pane's command is typed.
    pub after: Option<String>,
    /// This pane is ready once this text shows up in its output. A line that is just the command
    /// as typed does not count, but the text should still be something the command prints.
    pub ready_text: Option<String>,
    /// This pane is ready once its processes listen on this TCP port.
    pub ready_port: Option<u16>,
    /// Seconds to wait for this pane to become ready (default 60, at most 600).
    pub ready_timeout: Option<u64>,
}

const DEFAULT_READY_SECS: u64 = 60;
const MAX_READY_SECS: u64 = 600;
/// How often readiness is checked.
const READY_POLL: Duration = Duration::from_millis(250);

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
        for (what, v) in [
            ("name", &p.name),
            ("cwd", &p.cwd),
            ("command", &p.command),
            ("after", &p.after),
            ("ready_text", &p.ready_text),
        ] {
            if v.as_deref()
                .is_some_and(|s| s.chars().any(char::is_control))
            {
                bail!("a pane {what} must not contain control characters or line breaks");
            }
        }
    }
    check_dependencies(&r)?;
    Ok(r)
}

/// `after` must name exactly one other pane, readiness settings must make sense, and the panes must
/// not wait on each other in a circle.
fn check_dependencies(r: &Recipe) -> Result<()> {
    for p in &r.panes {
        if p.ready_port == Some(0) {
            bail!("ready_port must be between 1 and 65535");
        }
        if p.ready_text.as_deref() == Some("") {
            bail!("ready_text must not be empty");
        }
        if p.ready_timeout
            .is_some_and(|s| s == 0 || s > MAX_READY_SECS)
        {
            bail!("ready_timeout must be between 1 and {MAX_READY_SECS} seconds");
        }
    }
    for p in &r.panes {
        let Some(dep) = &p.after else { continue };
        let named: Vec<usize> = (0..r.panes.len())
            .filter(|i| r.panes[*i].name.as_deref() == Some(dep.as_str()))
            .collect();
        match named.len() {
            0 => bail!("`after = \"{dep}\"` names no pane; give that pane `name = \"{dep}\"`"),
            1 => {}
            _ => bail!("`after = \"{dep}\"` is ambiguous: several panes are named that"),
        }
        if p.name.as_deref() == Some(dep.as_str()) {
            bail!("pane `{dep}` cannot come after itself");
        }
    }
    start_order(r).map(|_| ())
}

/// Pane indexes in the order their commands start: every pane after the one it waits for,
/// otherwise as written. Errors on a circle.
fn start_order(r: &Recipe) -> Result<Vec<usize>> {
    let dep_of = |i: usize| -> Option<usize> {
        let dep = r.panes[i].after.as_deref()?;
        (0..r.panes.len()).find(|j| r.panes[*j].name.as_deref() == Some(dep))
    };
    let mut order: Vec<usize> = Vec::new();
    let mut visiting: Vec<usize> = Vec::new();
    fn visit(
        i: usize,
        dep_of: &dyn Fn(usize) -> Option<usize>,
        order: &mut Vec<usize>,
        visiting: &mut Vec<usize>,
        r: &Recipe,
    ) -> Result<()> {
        if order.contains(&i) {
            return Ok(());
        }
        if visiting.contains(&i) {
            bail!(
                "panes wait on each other in a circle (through `{}`)",
                r.panes[i].name.as_deref().unwrap_or("?")
            );
        }
        visiting.push(i);
        if let Some(d) = dep_of(i) {
            visit(d, dep_of, order, visiting, r)?;
        }
        visiting.pop();
        order.push(i);
        Ok(())
    }
    for i in 0..r.panes.len() {
        visit(i, &dep_of, &mut order, &mut visiting, r)?;
    }
    Ok(order)
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
    up_with(call, recipe, workspace, base, local, &mut |_| {})
}

/// Whether `pane` is ready by its own recipe settings (ready at once when it sets none).
fn is_ready(
    call: &mut dyn FnMut(Request) -> Result<Response>,
    spec: &RecipePane,
    id: u64,
) -> Result<bool> {
    if let Some(text) = &spec.ready_text {
        // The shell echoes the command that was typed, so a line holding the whole command line
        // proves nothing (the text may well be in the command itself).
        let echoed = spec.command.as_deref().unwrap_or("\u{0}");
        match call(Request::Capture { pane: id })? {
            Response::Captured { text: shown, .. }
                if shown
                    .lines()
                    .any(|l| l.contains(text.as_str()) && !l.contains(echoed)) => {}
            _ => return Ok(false),
        }
    }
    if let Some(port) = spec.ready_port {
        let listening = match call(Request::Poll {
            known: Default::default(),
            diff: false,
        })? {
            Response::State { panes } => {
                panes.iter().any(|p| p.id == id && p.ports.contains(&port))
            }
            _ => false,
        };
        if !listening {
            return Ok(false);
        }
    }
    Ok(true)
}

fn describe_wait(spec: &RecipePane) -> String {
    match (&spec.ready_text, spec.ready_port) {
        (Some(t), Some(p)) => format!("port {p} and the text \"{t}\""),
        (Some(t), None) => format!("the text \"{t}\""),
        (None, Some(p)) => format!("port {p}"),
        (None, None) => "nothing".to_string(),
    }
}

/// Like `up`, telling `progress` what it is waiting for. Panes are all created first; their
/// commands then start in dependency order, each after the pane it waits for is ready, so a pane
/// that never becomes ready stops the rest (the ones already started keep running).
pub fn up_with(
    call: &mut dyn FnMut(Request) -> Result<Response>,
    recipe: &Recipe,
    workspace: &str,
    base: &Path,
    local: bool,
    progress: &mut dyn FnMut(&str),
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
        takeover: false,
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
        ids.push(id);
    }
    let mut seq = 0;
    for i in start_order(recipe)? {
        let p = &recipe.panes[i];
        if let Some(dep) = &p.after {
            let j = recipe
                .panes
                .iter()
                .position(|q| q.name.as_deref() == Some(dep.as_str()))
                .expect("checked when the recipe was parsed");
            let spec = &recipe.panes[j];
            let limit = Duration::from_secs(
                spec.ready_timeout
                    .unwrap_or(DEFAULT_READY_SECS)
                    .min(MAX_READY_SECS),
            );
            let started = Instant::now();
            if !is_ready(call, spec, ids[j])? {
                progress(&format!("waiting for `{dep}` ({})", describe_wait(spec)));
                loop {
                    if started.elapsed() >= limit {
                        bail!(
                            "`{dep}` was not ready after {} s (waiting for {}); `{}` was not started",
                            limit.as_secs(),
                            describe_wait(spec),
                            p.name.as_deref().unwrap_or("the next pane")
                        );
                    }
                    std::thread::sleep(READY_POLL);
                    if is_ready(call, spec, ids[j])? {
                        break;
                    }
                }
            }
        }
        if let Some(cmd) = &p.command {
            let mut data = cmd.as_bytes().to_vec();
            data.push(b'\r');
            seq += 1;
            call(Request::Input {
                pane: ids[i],
                seq,
                data,
            })?;
        }
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
    fn dependencies_must_name_one_other_pane_and_never_form_a_circle() {
        let ok = parse(
            "[[pane]]\nname = \"app\"\nafter = \"db\"\n[[pane]]\nname = \"db\"\nready_port = 5432\nready_timeout = 30\n",
        )
        .unwrap();
        // The database starts first even though the app is written first.
        assert_eq!(start_order(&ok).unwrap(), [1, 0]);
        for (bad, why) in [
            ("[[pane]]\nafter = \"nope\"\n", "names no pane"),
            ("[[pane]]\nname = \"a\"\nafter = \"a\"\n", "after itself"),
            (
                "[[pane]]\nname = \"a\"\nafter = \"b\"\n[[pane]]\nname = \"b\"\nafter = \"a\"\n",
                "circle",
            ),
            (
                "[[pane]]\nname = \"a\"\nafter = \"b\"\n[[pane]]\nname = \"b\"\n[[pane]]\nname = \"b\"\n",
                "ambiguous",
            ),
            ("[[pane]]\nname = \"a\"\nready_port = 0\n", "ready_port"),
            ("[[pane]]\nname = \"a\"\nready_text = \"\"\n", "ready_text"),
            ("[[pane]]\nname = \"a\"\nready_timeout = 0\n", "ready_timeout"),
            ("[[pane]]\nname = \"a\"\nready_timeout = 601\n", "ready_timeout"),
            ("[[pane]]\nname = \"a\"\nready_text = \"x\\ny\"\n", "control"),
        ] {
            let err = parse(bad).unwrap_err().to_string();
            assert!(err.contains(why), "{bad} -> {err}");
        }
        // Without dependencies the order is as written.
        let plain = parse("[[pane]]\n[[pane]]\n[[pane]]\n").unwrap();
        assert_eq!(start_order(&plain).unwrap(), [0, 1, 2]);
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

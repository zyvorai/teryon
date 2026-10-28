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
    /// Environment variables for the directory (`teryon env`); not used by `teryon up`.
    #[serde(default)]
    pub env: Option<toml::Value>,
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
    /// This pane is ready once this text shows up in its output. If the command itself contains the
    /// text, its echo is accounted for, but the text should still be something the command prints.
    pub ready_text: Option<String>,
    /// This pane is ready once its processes listen on this TCP port.
    pub ready_port: Option<u16>,
    /// Seconds to wait for this pane to become ready (default 60, at most 600).
    pub ready_timeout: Option<u64>,
    /// Run the command again when it ends: `"on-failure"` (exit status other than 0) or `"always"`.
    /// Default `"never"`. Ctrl-C in the pane stops the loop.
    pub restart: Option<String>,
    /// Seconds to wait before each restart (default 2, at most 3600).
    pub restart_delay: Option<u64>,
    /// Give up after this many restarts (default 10, at most 1000).
    pub restart_max: Option<u32>,
}

const DEFAULT_RESTART_DELAY: u64 = 2;
const MAX_RESTART_DELAY: u64 = 3600;
const DEFAULT_RESTART_MAX: u32 = 10;
const MAX_RESTART_MAX: u32 = 1000;
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
            ("restart", &p.restart),
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
        let policy = p.restart.as_deref();
        if !matches!(policy, None | Some("never" | "on-failure" | "always")) {
            bail!("restart must be \"never\", \"on-failure\" or \"always\"");
        }
        let restarts = matches!(policy, Some("on-failure" | "always"));
        if restarts && p.command.is_none() {
            bail!("restart needs a command to run again");
        }
        if !restarts && (p.restart_delay.is_some() || p.restart_max.is_some()) {
            bail!("restart_delay and restart_max need restart = \"on-failure\" or \"always\"");
        }
        if p.restart_delay.is_some_and(|s| s > MAX_RESTART_DELAY) {
            bail!("restart_delay must be at most {MAX_RESTART_DELAY} seconds");
        }
        if p.restart_max.is_some_and(|n| n == 0 || n > MAX_RESTART_MAX) {
            bail!("restart_max must be between 1 and {MAX_RESTART_MAX}");
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

/// How long a new pane's shell gets to show its prompt before a command is typed into it anyway.
const SHELL_WAIT: Duration = Duration::from_secs(4);

/// Wait until the pane has printed something and stopped changing (the shell's prompt), because a
/// shell that is still starting can throw away what is typed before its prompt appears. Never fails
/// on a slow or silent shell: after `SHELL_WAIT` the command is typed regardless.
fn wait_for_shell(call: &mut dyn FnMut(Request) -> Result<Response>, pane: u64) -> Result<()> {
    let end = Instant::now() + SHELL_WAIT;
    let mut last = String::new();
    loop {
        let Response::Captured { text, .. } = call(Request::Capture { pane })? else {
            return Ok(());
        };
        if !text.trim().is_empty() && text == last {
            return Ok(());
        }
        last = text;
        if Instant::now() >= end {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

/// `s` as one word for a POSIX shell (and fish, zsh, bash): in single quotes.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// What is typed into the pane for `spec`: its command, or, with a restart policy, a small `sh`
/// loop that runs the command again after it ends. The loop runs in `sh` whatever the user's shell
/// is, takes the command as an argument (so quoting inside it is never a problem), says what it is
/// doing in the pane, and ends on Ctrl-C because it shares the terminal's foreground process group.
fn typed_command(spec: &RecipePane) -> Option<String> {
    let cmd = spec.command.as_deref()?;
    let always = match spec.restart.as_deref() {
        Some("always") => true,
        Some("on-failure") => false,
        _ => return Some(cmd.to_string()),
    };
    let delay = spec.restart_delay.unwrap_or(DEFAULT_RESTART_DELAY);
    let max = spec.restart_max.unwrap_or(DEFAULT_RESTART_MAX);
    let clean = if always {
        ""
    } else {
        "[ $s -eq 0 ] && { echo \"[teryon] exited cleanly\"; exit 0; }; "
    };
    Some(format!(
        "sh -c 'n=0; while :; do sh -c \"$1\"; s=$?; {clean}n=$((n+1)); \
         [ $n -gt {max} ] && {{ echo \"[teryon] exit status $s; giving up after {max} restarts\"; exit $s; }}; \
         echo \"[teryon] exit status $s; restart $n of {max} in {delay} s (Ctrl-C stops)\"; sleep {delay}; done' sh {}",
        shell_quote(cmd)
    ))
}

/// Whether `pane` is ready by its own recipe settings (ready at once when it sets none).
fn is_ready(
    call: &mut dyn FnMut(Request) -> Result<Response>,
    spec: &RecipePane,
    id: u64,
) -> Result<bool> {
    if let Some(text) = &spec.ready_text {
        match call(Request::Capture { pane: id })? {
            Response::Captured { text: shown, .. }
                if ready_text_seen(&shown, text, typed_command(spec).as_deref()) => {}
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

/// Whether `text` appears in the pane's output (`shown` is the pane's text). The shell echoes the
/// command that was typed, perhaps more than once (prompt themes redraw it), perhaps only partway
/// while it is still being typed, and the text may be in the command itself. Output only comes after
/// the last echo, so when the command is on screen only what follows it counts; when the command
/// contains the text but is not fully on screen yet, nothing counts. Whitespace is ignored so a
/// command or text that wraps across rows still matches.
fn ready_text_seen(shown: &str, text: &str, command: Option<&str>) -> bool {
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let (shown, text) = (squash(shown), squash(text));
    let command = command.map(squash).filter(|c| !c.is_empty());
    let Some(command) = command else {
        return shown.contains(&text);
    };
    match shown.rfind(&command) {
        Some(at) => shown[at + command.len()..].contains(&text),
        None => !command.contains(&text) && shown.contains(&text),
    }
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
        if let Some(cmd) = typed_command(p) {
            wait_for_shell(call, ids[i])?;
            let mut data = cmd.into_bytes();
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
mod env_table_tests {
    use super::*;
    #[test]
    fn a_recipe_may_also_carry_an_env_table_for_the_shell_integration() {
        let r = parse("[env]\nA = \"1\"\n[[pane]]\nname = \"a\"\n").unwrap();
        assert_eq!(r.panes.len(), 1);
        assert!(r.env.is_some());
    }
}

#[cfg(test)]
mod tests {
    fn pane(extra: &str) -> RecipePane {
        parse(&format!("[[pane]]\nname = \"p\"\n{extra}"))
            .unwrap()
            .panes
            .remove(0)
    }
    /// Run what would be typed into the pane, in a plain `sh`, and return its output and status.
    fn run_typed(spec: &RecipePane) -> (String, Option<i32>) {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(typed_command(spec).unwrap())
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code(),
        )
    }
    #[test]
    fn restart_settings_are_validated() {
        for bad in [
            "command = \"x\"\nrestart = \"sometimes\"\n",
            "restart = \"always\"\n",
            "command = \"x\"\nrestart_delay = 1\n",
            "command = \"x\"\nrestart = \"always\"\nrestart_max = 0\n",
            "command = \"x\"\nrestart = \"always\"\nrestart_delay = 4000\n",
        ] {
            assert!(parse(&format!("[[pane]]\n{bad}")).is_err(), "{bad}");
        }
        assert!(parse("[[pane]]\ncommand = \"x\"\nrestart = \"on-failure\"\nrestart_delay = 0\nrestart_max = 3\n").is_ok());
        // Without a policy the command is typed as written.
        assert_eq!(
            typed_command(&pane("command = \"ls -l\"\n")).as_deref(),
            Some("ls -l")
        );
        assert_eq!(
            typed_command(&pane("command = \"ls\"\nrestart = \"never\"\n")).as_deref(),
            Some("ls")
        );
        assert_eq!(typed_command(&pane("")), None);
    }
    #[test]
    fn a_failing_command_is_run_again_until_the_limit() {
        let spec = pane("command = \"echo run; exit 3\"\nrestart = \"on-failure\"\nrestart_delay = 0\nrestart_max = 2\n");
        let (out, code) = run_typed(&spec);
        assert_eq!(out.matches("run\n").count(), 3, "{out}");
        assert!(
            out.contains("restart 1 of 2") && out.contains("giving up after 2 restarts"),
            "{out}"
        );
        assert_eq!(code, Some(3));
    }
    #[test]
    fn on_failure_stops_after_a_clean_exit_and_always_does_not() {
        let ok = pane("command = \"echo hi\"\nrestart = \"on-failure\"\nrestart_delay = 0\n");
        let (out, code) = run_typed(&ok);
        assert_eq!(out.matches("hi\n").count(), 1, "{out}");
        assert!(out.contains("exited cleanly") && code == Some(0), "{out}");
        let always = pane(
            "command = \"echo hi\"\nrestart = \"always\"\nrestart_delay = 0\nrestart_max = 2\n",
        );
        let (out, _) = run_typed(&always);
        assert_eq!(out.matches("hi\n").count(), 3, "{out}");
    }
    #[test]
    fn quotes_in_the_command_survive_the_restart_loop() {
        let spec =
            pane("command = \"echo \\\"it's $((1+1))\\\" 'a b'\"\nrestart = \"on-failure\"\n");
        let (out, _) = run_typed(&spec);
        assert!(out.starts_with("it's 2 a b\n"), "{out}");
    }

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
    fn the_echo_of_a_command_never_counts_as_its_ready_text() {
        let cmd = Some("sleep 2; echo DB-IS-READY; sleep 30");
        // Nothing yet, a partly typed echo, the whole echo, the echo twice (a prompt theme redraws
        // it), and the echo wrapped over rows: all still waiting.
        for shown in [
            "% ",
            "% sleep 2; echo DB-IS-READY",
            "% sleep 2; echo DB-IS-READY; sleep 30\n",
            "sleep 2; echo DB-IS-READY; sleep 30\n\n~/proj\n> sleep 2; echo DB-IS-READY; sleep 30",
            "> sleep 2; echo DB-IS-\nREADY; sleep 30\n",
        ] {
            assert!(!ready_text_seen(shown, "DB-IS-READY", cmd), "{shown:?}");
        }
        // The output comes after the last echo.
        for shown in [
            "% sleep 2; echo DB-IS-READY; sleep 30\nDB-IS-READY\n",
            "sleep 2; echo DB-IS-READY; sleep 30\n\n> sleep 2; echo DB-IS-READY; sleep 30\nDB-IS-READY\n",
            "> sleep 2; echo DB-IS-\nREADY; sleep 30\nDB-IS-READY\n",
        ] {
            assert!(ready_text_seen(shown, "DB-IS-READY", cmd), "{shown:?}");
        }
        // A command that prints its own text: the output after the echo still counts.
        assert!(ready_text_seen("> echo hi\nhi\n", "hi", Some("echo hi")));
        assert!(!ready_text_seen("> echo hi\n", "hi", Some("echo hi")));
        // A command that does not contain the text needs only the text, even if its echo has
        // scrolled away; a text with spaces matches across a wrap.
        assert!(ready_text_seen(
            "ready on :3000",
            "ready on",
            Some("npm start")
        ));
        assert!(ready_text_seen(
            "old stuff\nready\non :3000",
            "ready on",
            Some("npm start")
        ));
        assert!(!ready_text_seen("starting", "ready on", Some("npm start")));
        assert!(ready_text_seen("ready on", "ready on", None));
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

//! The daemon, never a connection handler, owns every PTY.
use crate::protocol::*;
use crate::{ipc, util};
use anyhow::{bail, Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_WORKSPACES: usize = 16;
const MAX_PANES: usize = 16;
/// Lines of history each pane keeps; `TERYON_SCROLLBACK` (100 to 50000) overrides the default.
/// Memory grows with lines x columns x panes, so raise it with care.
fn scrollback_lines() -> usize {
    static LINES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LINES.get_or_init(|| {
        std::env::var("TERYON_SCROLLBACK")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| (100..=50_000).contains(n))
            .unwrap_or(2000)
    })
}
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
struct TerminalCallbacks {
    input: std::sync::mpsc::SyncSender<Vec<u8>>,
    /// Shell-integration marks and the shell's live working directory (OSC 133 / OSC 7).
    log: crate::marks::MarkLog,
    /// Inline images (iTerm2 `OSC 1337 File=`) shown in this pane.
    images: crate::inlineimg::ImageLog,
}
impl TerminalCallbacks {
    /// Reserve room for an inline image at the cursor and remember it.
    fn place_image(&mut self, screen: &mut vt100::Screen, request: crate::inlineimg::Request) {
        // Full-screen programs draw their own pictures; only the shell's scrolling view holds images.
        if screen.alternate_screen() {
            return;
        }
        let Some(size) = crate::inlineimg::pixel_size(&request.bytes) else {
            return;
        };
        let (screen_rows, screen_cols) = screen.size();
        let (cols, rows) = crate::inlineimg::span(&request, size, screen_cols, screen_rows);
        let (row, col) = screen.cursor_position();
        let top = screen.scrolled_total();
        self.images
            .add(request.bytes, top + u64::from(row), col, cols, rows);
        screen.advance_rows(rows.saturating_sub(1));
        self.images.trim(
            screen
                .scrolled_total()
                .saturating_sub(scrollback_lines() as u64),
        );
    }
}
impl vt100::Callbacks for TerminalCallbacks {
    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        use crate::marks::OscEvent;
        if let Some(request) = crate::inlineimg::parse(params) {
            self.place_image(screen, request);
            return;
        }
        let Some(event) = crate::marks::parse_osc(params) else {
            return;
        };
        // Full-screen programs share the cursor row but not the history; only track the shell.
        if screen.alternate_screen() && !matches!(event, OscEvent::Cwd(_)) {
            return;
        }
        let top = screen.scrolled_total();
        let line = top + u64::from(screen.cursor_position().0);
        let floor = top.saturating_sub(scrollback_lines() as u64);
        self.log
            .apply(event, line, floor, Instant::now(), unix_now());
    }
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let n = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let response = match (c, i1, n) {
            ('n', None, 6) => {
                let (r, c) = screen.cursor_position();
                Some(format!("\x1b[{};{}R", r + 1, c + 1))
            }
            ('n', None, 5) => Some("\x1b[0n".into()),
            // Erasing the history takes the images in it along.
            ('J', None, 3) => {
                self.images.clear();
                None
            }
            ('c', None, 0) => Some("\x1b[?1;2c".into()),
            ('c', Some(b'>'), 0) => Some("\x1b[>0;1;0c".into()),
            _ => None,
        };
        if let Some(response) = response {
            let _ = self.input.try_send(response.into_bytes());
        }
    }
}
struct TerminalState {
    parser: vt100::Parser<TerminalCallbacks>,
    revision: u64,
    exited: Option<u32>,
}
struct Pane {
    id: u64,
    title: Mutex<String>,
    cwd: String,
    pid: Option<u32>,
    state: Arc<Mutex<TerminalState>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    input: std::sync::mpsc::SyncSender<Vec<u8>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// Foreground command line, refreshed at most once a second.
    fg: Mutex<(std::time::Instant, Option<String>)>,
    /// Listening ports of the pane's process tree, refreshed in the background every few seconds.
    ports: Arc<Mutex<PortCache>>,
}
struct PortCache {
    at: std::time::Instant,
    ports: Vec<u16>,
    busy: bool,
}

/// Friendly `ls`/`eza` colors for new shells: directories bold blue, executables bold green, symlinks
/// cyan, archives red, images magenta, video bold magenta, audio cyan, source code yellow, config and
/// data bright cyan, documents white, build leftovers dim. They use ANSI indexes, so every TERYON theme
/// repaints them. Variables the daemon's environment already sets are left alone, and a shell rc file
/// can still override these.
fn file_type_env(already_set: impl Fn(&str) -> bool) -> Vec<(&'static str, String)> {
    const BASE: &str = "di=1;34:ln=1;36:so=35:pi=33:ex=1;32:bd=34;46:cd=33;40:su=30;41:sg=30;46:tw=30;42:ow=34;42:or=1;31:mi=1;31";
    const GROUPS: &[(&str, &str)] = &[
        ("31", "tar tgz gz zip xz bz2 7z rar zst lz4 dmg iso pkg deb rpm jar war apk"),
        ("35", "png jpg jpeg gif svg webp bmp ico heic tiff psd avif"),
        ("1;35", "mp4 mov mkv avi webm m4v flv"),
        ("36", "mp3 wav flac aac ogg m4a opus"),
        ("93", "rs py js mjs ts tsx jsx go c h cc cpp hpp java kt swift rb php lua zig cs sql html css scss vue svelte"),
        ("1;32", "sh zsh bash fish"),
        ("96", "json toml yaml yml ini conf cfg env xml plist"),
        ("97", "md txt rst pdf doc docx pages epub"),
        ("90", "lock o a so dylib class pyc log tmp bak swp"),
    ];
    const NAMES: &str = "*Makefile=1;93:*Dockerfile=1;96:*README.md=1;97:*LICENSE=97:*Cargo.toml=1;96:*package.json=1;96";
    const EZA: &str = "ur=33:uw=31:ux=32:ue=32:gr=33:gw=31:gx=32:tr=33:tw=31:tx=32:sn=36:sb=36:uu=33:gu=33:un=31:gn=31:da=34:xx=90:hd=1;4";
    const LSCOLORS: &str = "Exgxfxdxcxegedabagacad";
    let mut ls = String::from(BASE);
    for (sgr, exts) in GROUPS {
        for ext in exts.split(' ') {
            ls.push_str(&format!(":*.{ext}={sgr}"));
        }
    }
    ls.push(':');
    ls.push_str(NAMES);
    [
        ("CLICOLOR", "1".to_string()),
        ("LSCOLORS", LSCOLORS.to_string()),
        ("LS_COLORS", ls),
        ("EZA_COLORS", EZA.to_string()),
    ]
    .into_iter()
    .filter(|(name, _)| !already_set(name))
    .collect()
}

impl Pane {
    fn spawn(
        id: u64,
        cwd: Option<String>,
        command: Vec<String>,
        rows: u16,
        cols: u16,
    ) -> Result<Arc<Self>> {
        if !valid_size(rows, cols) {
            bail!("terminal size outside supported bounds");
        }
        let cwd = cwd.map(std::path::PathBuf::from).unwrap_or(util::home()?);
        let cwd = cwd
            .canonicalize()
            .context("working directory does not exist")?;
        if !cwd.is_dir() {
            bail!("working directory must be a directory");
        }
        let pty = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let is_shell = command.is_empty();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut cmd = CommandBuilder::new(command.first().unwrap_or(&shell));
        if is_shell {
            cmd.arg("-l");
        } else {
            cmd.args(&command[1..]);
        }
        cmd.cwd(&cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERYON", "1");
        for (name, value) in file_type_env(|n| std::env::var_os(n).is_some()) {
            cmd.env(name, value);
        }
        let mut child = pty.slave.spawn_command(cmd).context("spawn PTY process")?;
        drop(pty.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pty.master.try_clone_reader()?;
        let mut writer = pty.master.take_writer()?;
        let (input, input_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
        thread::spawn(move || {
            while let Ok(data) = input_rx.recv() {
                if writer
                    .write_all(&data)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let state = Arc::new(Mutex::new(TerminalState {
            parser: vt100::Parser::new_with_callbacks(
                rows,
                cols,
                scrollback_lines(),
                TerminalCallbacks {
                    input: input.clone(),
                    log: crate::marks::MarkLog::default(),
                    images: crate::inlineimg::ImageLog::default(),
                },
            ),
            revision: 1,
            exited: None,
        }));
        let pane = Arc::new(Self {
            id,
            title: Mutex::new(if is_shell {
                "Shell".into()
            } else {
                command[0].clone()
            }),
            cwd: cwd.to_string_lossy().into(),
            pid,
            state: state.clone(),
            master: Mutex::new(pty.master),
            input,
            killer: Mutex::new(killer),
            fg: Mutex::new((std::time::Instant::now() - Duration::from_secs(10), None)),
            ports: Arc::new(Mutex::new(PortCache {
                at: std::time::Instant::now() - Duration::from_secs(60),
                ports: Vec::new(),
                busy: false,
            })),
        });
        let read_state = state.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 16384];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut st = read_state.lock().unwrap();
                        st.parser.process(&buf[..n]);
                        // Clearing the screen also removes the inline images drawn on it.
                        let (erases, top) = {
                            let screen = st.parser.screen();
                            (screen.erase_count(), screen.scrolled_total())
                        };
                        st.parser.callbacks_mut().images.after_output(erases, top);
                        st.revision += 1;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break, // Linux PTYs use EIO at EOF.
                }
            }
        });
        thread::spawn(move || {
            let exit = child.wait().map(|s| s.exit_code()).unwrap_or(255);
            let mut st = state.lock().unwrap();
            st.exited = Some(exit);
            st.revision += 1;
        });
        Ok(pane)
    }
    fn view(&self, known: Option<u64>) -> PaneView {
        let st = self.state.lock().unwrap();
        let (rows, cols) = st.parser.screen().size();
        let changed = known != Some(st.revision);
        let top = st.parser.screen().scrolled_total();
        let log = &st.parser.callbacks().log;
        PaneView {
            id: self.id,
            title: self.title.lock().unwrap().clone(),
            cwd: log.cwd.clone().unwrap_or_else(|| self.cwd.clone()),
            pid: self.pid,
            rows,
            cols,
            revision: st.revision,
            screen: changed.then(|| st.parser.screen().state_formatted()),
            exited: st.exited,
            marks: changed.then(|| log.marks.clone()),
            first_line: top.saturating_sub(scrollback_lines() as u64),
            top_line: top,
            fg: self.foreground_command(st.exited.is_some()),
            ports: self.listening_ports(st.exited.is_some()),
            images: changed.then(|| st.parser.callbacks().images.placements.clone()),
            links: changed.then(|| {
                st.parser
                    .screen()
                    .hyperlinks()
                    .into_iter()
                    .map(|h| crate::protocol::LinkSpan {
                        row: h.row,
                        start: h.start,
                        end: h.end,
                        url: h.url,
                    })
                    .collect()
            }),
        }
    }
    /// Listening ports of this pane's processes. Returns the last answer at once; when it is older
    /// than 5 seconds a background thread refreshes it (`lsof` can take a moment).
    fn listening_ports(&self, exited: bool) -> Vec<u16> {
        let Some(pid) = self.pid.filter(|_| !exited) else {
            return Vec::new();
        };
        let mut cache = self.ports.lock().unwrap();
        if !cache.busy && cache.at.elapsed() > Duration::from_secs(5) {
            cache.busy = true;
            let shared = self.ports.clone();
            thread::spawn(move || {
                let found = crate::ports::listening(pid);
                let mut c = shared.lock().unwrap();
                *c = PortCache {
                    at: std::time::Instant::now(),
                    ports: found,
                    busy: false,
                };
            });
        }
        cache.ports.clone()
    }
    /// The command line of the foreground process group (cached for a second; `ps` is only run when asked again).
    fn foreground_command(&self, exited: bool) -> Option<String> {
        if exited {
            return None;
        }
        let mut cache = self.fg.lock().unwrap();
        if cache.0.elapsed() < Duration::from_secs(1) {
            return cache.1.clone();
        }
        let leader = self.master.lock().unwrap().process_group_leader();
        let text = leader.filter(|p| *p > 1).and_then(|pid| {
            let out = std::process::Command::new("ps")
                .args(["-o", "command=", "-p", &pid.to_string()])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            let s = String::from_utf8_lossy(&out.stdout)
                .trim()
                .chars()
                .take(200)
                .collect::<String>();
            (!s.is_empty()).then_some(s)
        });
        *cache = (std::time::Instant::now(), text.clone());
        text
    }
    fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        if !valid_size(rows, cols) {
            bail!("terminal size outside supported bounds");
        }
        self.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut st = self.state.lock().unwrap();
        st.parser.screen_mut().set_size(rows, cols);
        st.revision += 1;
        Ok(())
    }
    /// Retained text, one line per terminal row (so line numbers are exact), and the absolute
    /// number of its first line.
    fn capture(&self) -> (String, u64) {
        let mut st = self.state.lock().unwrap();
        let (rows, cols) = st.parser.screen().size();
        let rows = usize::from(rows);
        // Query the actual bounded scrollback length, then walk oldest to newest.
        st.parser.screen_mut().set_scrollback(usize::MAX);
        let max = st.parser.screen().scrollback();
        let first = st
            .parser
            .screen()
            .scrolled_total()
            .saturating_sub(max as u64);
        let mut lines: Vec<String> = Vec::new();
        let mut offset = max;
        while offset > 0 {
            st.parser.screen_mut().set_scrollback(offset);
            let take = offset.min(rows);
            lines.extend(
                st.parser
                    .screen()
                    .rows(0, cols)
                    .take(take)
                    .map(|l| l.trim_end().to_string()),
            );
            offset = offset.saturating_sub(rows);
        }
        st.parser.screen_mut().set_scrollback(0);
        lines.extend(
            st.parser
                .screen()
                .rows(0, cols)
                .map(|l| l.trim_end().to_string()),
        );
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        (lines.join("\n"), first)
    }
    fn kill(&self) -> Result<()> {
        if self.state.lock().unwrap().exited.is_some() {
            return Ok(());
        }
        let foreground = self.master.lock().unwrap().process_group_leader();
        let result = self.killer.lock().unwrap().kill().context("terminate pane");
        // Explicit termination must also stop a foreground job that ignores HUP.
        // SAFETY: group IDs come from this pane's PTY and spawned child. Never
        // signal the daemon's own group, zero, or process group 1.
        let own_group = unsafe { libc::getpgrp() };
        for group in [foreground, self.pid.map(|p| p as i32)]
            .into_iter()
            .flatten()
        {
            if group > 1 && group != own_group {
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
            }
        }
        result
    }
}

struct Workspace {
    panes: BTreeMap<u64, Arc<Pane>>,
    controller: Option<String>,
    next_id: u64,
    last_seq: u64,
    /// Panes closed with `SoftClose`, newest last, still running until their grace period ends.
    graveyard: Vec<(std::time::Instant, Arc<Pane>)>,
}
/// How long a soft-closed pane can still be reopened (`TERYON_UNDO_SECS` shortens it for tests).
fn undo_window() -> Duration {
    Duration::from_secs(
        std::env::var("TERYON_UNDO_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30),
    )
}
const MAX_GRAVEYARD: usize = 5;
struct State {
    started: std::time::Instant,
    instance: String,
    workspaces: HashMap<String, Workspace>,
}
struct Connection {
    id: String,
    workspace: Option<String>,
    readonly: bool,
    hello: bool,
}

/// Host stats, sampled at most every 5 seconds however many clients ask.
fn host_stats() -> crate::hoststats::HostStats {
    static CACHE: Mutex<Option<(Instant, crate::hoststats::HostStats)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, stats)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(5) {
            return stats.clone();
        }
    }
    let stats = crate::hoststats::sample();
    *cache = Some((Instant::now(), stats.clone()));
    stats
}

fn process(req: Request, conn: &mut Connection, state: &mut State) -> Result<Response> {
    if let Request::Hello { version } = req {
        if version != VERSION {
            bail!("protocol version mismatch: client {version}, daemon {VERSION}");
        }
        conn.hello = true;
        return Ok(Response::Hello {
            version: VERSION,
            build: env!("CARGO_PKG_VERSION").into(),
            instance: state.instance.clone(),
        });
    }
    if !conn.hello {
        bail!("hello handshake required");
    }
    if conn.readonly && matches!(req, Request::Shutdown { .. } | Request::Remove { .. }) {
        bail!("read-only attachment cannot remove workspaces or stop the daemon");
    }
    match req {
        Request::List => {
            let mut items: Vec<_> = state
                .workspaces
                .iter()
                .map(|(name, w)| WorkspaceInfo {
                    name: name.clone(),
                    panes: w.panes.len(),
                    controlled: w.controller.is_some(),
                })
                .collect();
            items.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(Response::Workspaces { items })
        }
        Request::Remove { workspace, force } => {
            let ws = state
                .workspaces
                .get(&workspace)
                .context("workspace not found")?;
            if ws.controller.as_ref().is_some_and(|id| id != &conn.id) {
                bail!("detach the workspace controller before removing it");
            }
            let live = ws
                .panes
                .values()
                .any(|p| p.state.lock().unwrap().exited.is_none());
            if live && !force {
                bail!("workspace contains live processes; removal requires --force");
            }
            for p in ws.panes.values().chain(ws.graveyard.iter().map(|(_, p)| p)) {
                if p.state.lock().unwrap().exited.is_none() {
                    p.kill()?;
                }
            }
            state.workspaces.remove(&workspace);
            Ok(Response::Ack { seq: None })
        }
        Request::Info => Ok(Response::Info {
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            sessions: state.workspaces.len(),
            panes: state
                .workspaces
                .values()
                .map(|w| w.panes.len() + w.graveyard.len())
                .sum(),
            pid: std::process::id(),
            uptime: state.started.elapsed().as_secs(),
            stats: Some(host_stats()),
            undo: true,
        }),
        Request::Attach {
            workspace,
            readonly,
            rows,
            cols,
            cwd,
            takeover,
        } => {
            if !valid_name(&workspace) {
                bail!("workspace names use 1–64 ASCII letters, digits, '-' or '_'");
            }
            if !valid_size(rows, cols) {
                bail!("invalid terminal size");
            }
            if !state.workspaces.contains_key(&workspace) {
                if readonly {
                    bail!("workspace does not exist; a read-only attachment cannot create it");
                }
                if state.workspaces.len() >= MAX_WORKSPACES || total_panes(state) >= MAX_PANES {
                    bail!("workspace/pane limit reached");
                }
                let pane = Pane::spawn(1, cwd, vec![], rows, cols)?;
                state.workspaces.insert(
                    workspace.clone(),
                    Workspace {
                        panes: BTreeMap::from([(1, pane)]),
                        controller: None,
                        next_id: 2,
                        last_seq: 0,
                        graveyard: Vec::new(),
                    },
                );
            }
            let ws = state.workspaces.get(&workspace).unwrap();
            if !readonly && !takeover && ws.controller.as_ref().is_some_and(|id| id != &conn.id) {
                bail!(
                    "workspace already has a controller; use --readonly or detach the other client"
                );
            }
            release(conn, state);
            let ws = state.workspaces.get_mut(&workspace).unwrap();
            if !readonly {
                ws.controller = Some(conn.id.clone());
                ws.last_seq = 0;
            }
            conn.workspace = Some(workspace.clone());
            conn.readonly = readonly;
            Ok(Response::Attached {
                workspace,
                readonly,
            })
        }
        Request::Shutdown { force } => {
            let live = state
                .workspaces
                .values()
                .flat_map(|w| w.panes.values())
                .filter(|p| p.state.lock().unwrap().exited.is_none())
                .count();
            if live > 0 && !force {
                bail!("{live} live panes; refusing shutdown without --force");
            }
            for w in state.workspaces.values() {
                for p in w.panes.values().chain(w.graveyard.iter().map(|(_, p)| p)) {
                    let _ = p.kill();
                }
            }
            // Caller exits after the acknowledgment is flushed.
            Ok(Response::Ack { seq: None })
        }
        action => {
            let total = total_panes(state);
            let ws = state
                .workspaces
                .get_mut(
                    conn.workspace
                        .as_ref()
                        .context("attach to a workspace first")?,
                )
                .context("workspace no longer exists")?;
            if !matches!(
                action,
                Request::Poll { .. } | Request::Capture { .. } | Request::Image { .. }
            ) && (conn.readonly || ws.controller.as_ref() != Some(&conn.id))
            {
                // A client that lost its seat to a takeover is told so, rather than "read-only".
                if !conn.readonly && ws.controller.is_some() {
                    bail!("taken over: another client now controls this workspace");
                }
                bail!("read-only connection cannot modify the workspace");
            }
            match action {
                Request::Poll { known } => Ok(Response::State {
                    panes: ws
                        .panes
                        .values()
                        .map(|p| p.view(known.get(&p.id.to_string()).copied()))
                        .collect(),
                }),
                Request::Input { pane, seq, data } => {
                    if data.len() > MAX_INPUT {
                        bail!("input exceeds 64 KiB");
                    }
                    if seq <= ws.last_seq {
                        return Ok(Response::Ack { seq: Some(seq) });
                    }
                    if seq != ws.last_seq + 1 {
                        bail!("input sequence gap");
                    }
                    let p = ws.panes.get(&pane).context("pane not found")?;
                    if p.state.lock().unwrap().exited.is_some() {
                        bail!("pane has exited");
                    }
                    // Acceptance means queued, not executed. A blocked PTY writer
                    // never holds the global session lock or stalls other panes.
                    p.input
                        .try_send(data)
                        .context("pane input queue is full or closed")?;
                    ws.last_seq = seq;
                    Ok(Response::Ack { seq: Some(seq) })
                }
                Request::Resize { pane, rows, cols } => {
                    ws.panes
                        .get(&pane)
                        .context("pane not found")?
                        .resize(rows, cols)?;
                    Ok(Response::Ack { seq: None })
                }
                Request::NewPane {
                    cwd,
                    command,
                    rows,
                    cols,
                } => {
                    if total >= MAX_PANES || ws.panes.len() >= 8 {
                        bail!("pane limit reached (8 per workspace, 16 total)");
                    }
                    if command.len() > 128 || command.iter().any(|s| s.len() > 8192) {
                        bail!("command too large");
                    }
                    let id = ws.next_id;
                    let p = Pane::spawn(id, cwd, command, rows, cols)?;
                    ws.next_id += 1;
                    ws.panes.insert(id, p);
                    Ok(Response::Created { pane: id })
                }
                Request::Rename { pane, title } => {
                    if title.is_empty() || title.len() > 80 || title.chars().any(char::is_control) {
                        bail!("title must be 1–80 bytes with no control characters");
                    }
                    *ws.panes
                        .get(&pane)
                        .context("pane not found")?
                        .title
                        .lock()
                        .unwrap() = title;
                    Ok(Response::Ack { seq: None })
                }
                Request::SoftClose { pane } => {
                    if ws.panes.len() == 1 {
                        bail!("cannot remove the last pane; exit its shell instead");
                    }
                    let p = ws.panes.remove(&pane).context("pane not found")?;
                    ws.graveyard.push((Instant::now(), p));
                    while ws.graveyard.len() > MAX_GRAVEYARD {
                        let (_, old) = ws.graveyard.remove(0);
                        let _ = old.kill();
                    }
                    Ok(Response::Ack { seq: None })
                }
                Request::Reopen => {
                    while let Some((at, p)) = ws.graveyard.pop() {
                        if at.elapsed() < undo_window() {
                            let id = p.id;
                            ws.panes.insert(id, p);
                            return Ok(Response::Created { pane: id });
                        }
                        let _ = p.kill();
                    }
                    bail!("no closed pane to reopen")
                }
                Request::Close { pane } => {
                    if ws.panes.len() == 1 {
                        bail!("cannot remove the last pane; exit its shell instead");
                    }
                    let p = ws.panes.get(&pane).context("pane not found")?;
                    if p.state.lock().unwrap().exited.is_none() {
                        p.kill()?;
                    }
                    ws.panes.remove(&pane);
                    Ok(Response::Ack { seq: None })
                }
                Request::Capture { pane } => {
                    let (text, first_line) =
                        ws.panes.get(&pane).context("pane not found")?.capture();
                    Ok(Response::Captured { text, first_line })
                }
                Request::Image { pane, id } => {
                    let p = ws.panes.get(&pane).context("pane not found")?;
                    let bytes = p
                        .state
                        .lock()
                        .unwrap()
                        .parser
                        .callbacks()
                        .images
                        .bytes(id)
                        .context("image not found")?;
                    Ok(Response::ImageData {
                        id,
                        data: crate::ui::base64(&bytes),
                    })
                }
                _ => bail!("unsupported request"),
            }
        }
    }
}
fn total_panes(state: &State) -> usize {
    state.workspaces.values().map(|w| w.panes.len()).sum()
}
fn release(conn: &Connection, state: &mut State) {
    if let Some(name) = &conn.workspace {
        if let Some(ws) = state.workspaces.get_mut(name) {
            if ws.controller.as_ref() == Some(&conn.id) {
                ws.controller = None;
            }
        }
    }
}
fn serve(stream: ipc::Stream, state: Arc<Mutex<State>>, token: &str) -> Result<()> {
    stream.timeout(Duration::from_secs(45))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    ipc::authenticate(&mut reader, token)?;
    let mut conn = Connection {
        id: util::nonce()?,
        workspace: None,
        readonly: false,
        hello: false,
    };
    let result = (|| -> Result<()> {
        while let Some(req) = read_frame::<Request>(&mut reader)? {
            let shutdown = matches!(req, Request::Shutdown { .. });
            let response =
                process(req, &mut conn, &mut state.lock().unwrap()).unwrap_or_else(|e| {
                    Response::Error {
                        message: format!("{e:#}"),
                    }
                });
            let stop = shutdown && matches!(response, Response::Ack { .. });
            write_frame(&mut writer, &response)?;
            if stop {
                let _ = fs::remove_file(util::socket_path()?);
                std::process::exit(0);
            }
        }
        Ok(())
    })();
    release(&conn, &mut state.lock().unwrap());
    result
}

/// Tell the configured sinks when an agent needs you or a long command ends in a workspace that
/// has no client attached (an attached client notifies for itself).
/// End the shells of soft-closed panes whose grace period is over.
fn reap_graveyards(state: &Mutex<State>) {
    let mut st = state.lock().unwrap();
    for w in st.workspaces.values_mut() {
        let (expired, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut w.graveyard)
            .into_iter()
            .partition(|(at, _)| at.elapsed() >= undo_window());
        w.graveyard = keep;
        for (_, p) in expired {
            let _ = p.kill();
        }
    }
}

fn watch_panes(state: &Mutex<State>) {
    use crate::notify::{clean, send, Note};
    use crate::watch::{Event, Tracker, TICK};
    let mut trackers: HashMap<(String, u64), Tracker> = HashMap::new();
    let mut cfg = crate::config::NotifyConfig::default();
    let mut host_cfg = crate::config::HostConfig::default();
    let mut awake = KeepAwake::default();
    let mut cfg_loaded: Option<Instant> = None;
    loop {
        thread::sleep(TICK);
        reap_graveyards(state);
        let now = Instant::now();
        if cfg_loaded.is_none_or(|t| now.duration_since(t) > Duration::from_secs(15)) {
            cfg_loaded = Some(now);
            let loaded = crate::config::load()
                .ok()
                .filter(|c| c.notify.validate().is_ok() && c.host.validate().is_ok());
            (cfg, host_cfg) = loaded.map(|c| (c.notify, c.host)).unwrap_or_default();
        }
        if !cfg.active() && !host_cfg.keep_awake {
            trackers.clear();
            awake.set(false);
            continue;
        }
        let mut running: Vec<u64> = Vec::new();
        let mut agent_working = false;
        let snapshot: Vec<(String, bool, Vec<Arc<Pane>>)> = {
            let st = state.lock().unwrap();
            st.workspaces
                .iter()
                .map(|(name, w)| {
                    (
                        name.clone(),
                        w.controller.is_none(),
                        w.panes.values().cloned().collect(),
                    )
                })
                .collect()
        };
        let mut alive = std::collections::HashSet::new();
        for (ws, detached, panes) in snapshot {
            for pane in panes {
                let key = (ws.clone(), pane.id);
                alive.insert(key.clone());
                let (rev, exited, marks) = {
                    let st = pane.state.lock().unwrap();
                    (
                        st.revision,
                        st.exited.is_some(),
                        st.parser.callbacks().log.marks.clone(),
                    )
                };
                let fg = pane.foreground_command(exited).unwrap_or_default();
                let title = pane.title.lock().unwrap().clone();
                let fg = if fg.is_empty() { title } else { fg };
                let tracker = trackers.entry(key).or_insert_with(|| Tracker::new(now));
                let rows = || {
                    let st = pane.state.lock().unwrap();
                    let screen = st.parser.screen();
                    let (_, cols) = screen.size();
                    screen.rows(0, cols).collect::<Vec<String>>()
                };
                let long_ms = u64::from(cfg.after_seconds) * 1000;
                let events = tracker.observe(now, rev, &fg, exited, rows, &marks, long_ms);
                agent_working |= tracker.agent_working();
                running.extend(crate::watch::running_since(&marks));
                if !detached || !cfg.active() {
                    continue;
                }
                for event in events {
                    let (name, message) = match event {
                        Event::AgentNeedsYou(k) if cfg.agents => (
                            "agent_needs_you",
                            format!("{} needs you in {ws} (pane {})", k.name(), pane.id),
                        ),
                        Event::AgentDone(k) if cfg.agents => (
                            "agent_done",
                            format!("{} finished in {ws} (pane {})", k.name(), pane.id),
                        ),
                        Event::CommandDone {
                            program,
                            exit,
                            secs,
                        } if cfg.commands => (
                            "command_done",
                            format!(
                                "{} finished in {secs} s (exit {}) in {ws}",
                                clean(&program, 40),
                                exit.map_or("?".to_string(), |c| c.to_string())
                            ),
                        ),
                        _ => continue,
                    };
                    send(
                        &cfg,
                        Note {
                            event: name,
                            title: "TERYON".into(),
                            message: clean(&message, 200),
                            workspace: ws.clone(),
                            pane: pane.id,
                        },
                    );
                }
            }
        }
        trackers.retain(|k, _| alive.contains(k));
        awake.set(
            host_cfg.keep_awake
                && crate::watch::busy(
                    &running,
                    agent_working,
                    unix_now(),
                    u64::from(host_cfg.keep_awake_after_seconds),
                ),
        );
    }
}

/// Holds a `caffeinate` child (macOS) while the machine should not idle-sleep. It also exits on its
/// own when the daemon does.
#[derive(Default)]
struct KeepAwake {
    child: Option<std::process::Child>,
}
impl KeepAwake {
    fn set(&mut self, on: bool) {
        if on && self.child.is_none() {
            #[cfg(target_os = "macos")]
            {
                self.child = std::process::Command::new("caffeinate")
                    .args(["-i", "-w", &std::process::id().to_string()])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .ok();
            }
        } else if !on {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

pub fn run() -> Result<()> {
    let dir = util::runtime_dir()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(dir.join("daemon.lock"))?;
    // SAFETY: the file descriptor stays open for this daemon's entire lifetime.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("another daemon holds the runtime lock");
    }
    let (listener, token) = ipc::Listener::bind()?;
    let state = Arc::new(Mutex::new(State {
        started: std::time::Instant::now(),
        instance: util::nonce()?,
        workspaces: HashMap::new(),
    }));
    {
        let state = state.clone();
        thread::spawn(move || watch_panes(&state));
    }
    loop {
        let stream = listener.accept()?;
        let state = state.clone();
        let token = token.clone();
        thread::spawn(move || {
            let _ = serve(stream, state, &token);
        });
    }
}
pub fn ensure() -> Result<()> {
    if ipc::connect().is_ok() {
        return Ok(());
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(util::runtime_dir()?.join("daemon.log"))?;
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid is async-signal-safe and invoked only in the pre-exec child.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ipc::connect().is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(40));
    }
    bail!("daemon did not start; inspect ~/.teryon/run/daemon.log")
}
pub fn relay() -> Result<()> {
    ensure()?;
    let mut stream = ipc::connect()?;
    stream.timeout(Duration::from_secs(3600))?;
    let mut write = stream.try_clone()?;
    thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut write);
        write.shutdown_write();
    });
    std::io::copy(&mut stream, &mut std::io::stdout().lock())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::file_type_env;

    #[test]
    fn file_type_colors_never_override_user_settings() {
        let all = file_type_env(|_| false);
        assert_eq!(all.len(), 4);
        assert!(all.iter().any(|(n, v)| *n == "CLICOLOR" && v == "1"));
        for (name, value) in all.iter().filter(|(n, _)| n.ends_with("_COLORS")) {
            assert!(
                value.split(':').all(|e| e.split_once('=').is_some_and(
                    |(k, v)| !k.is_empty() && v.split(';').all(|n| n.parse::<u8>().is_ok())
                )),
                "{name} has a malformed entry"
            );
        }
        let none = file_type_env(|_| true);
        assert!(none.is_empty());
        let some = file_type_env(|n| n == "LS_COLORS");
        assert!(some.iter().all(|(n, _)| *n != "LS_COLORS") && some.len() == 3);
    }
}

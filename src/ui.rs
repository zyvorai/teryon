use crate::{
    keys,
    protocol::*,
    theme::{Theme, BUILTINS},
    transport::Target,
    util,
};
use anyhow::{bail, Result};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

/// Idle gap between polls, and the much shorter one right after typing or output, so an echo
/// appears within a few milliseconds instead of waiting out the idle gap.
const POLL_IDLE: Duration = Duration::from_millis(60);
const POLL_HOT: Duration = Duration::from_millis(6);
/// How long polling stays fast after input or a screen change.
const HOT_FOR: Duration = Duration::from_millis(400);

/// A worker loop that comes back after longer than this was suspended (laptop sleep, network
/// change), so its connection is assumed dead and replaced at once instead of waiting for ssh keepalives.
const RESUME_GAP: Duration = Duration::from_secs(20);

/// `wait_ms` spread by up to ±20% (from `seed`), so tabs that lost the same link do not all
/// reconnect in the same instant.
pub(crate) fn jittered(wait_ms: u64, seed: u32) -> u64 {
    let spread = wait_ms / 5;
    if spread == 0 {
        return wait_ms;
    }
    wait_ms - spread + u64::from(seed) % (2 * spread + 1)
}

/// How long the worker waits for a command before it polls again.
pub(crate) fn poll_wait(now: Instant, hot_until: Instant) -> Duration {
    if now < hot_until {
        POLL_HOT
    } else {
        POLL_IDLE
    }
}

/// Called by the worker right after new pane state is queued, so a GUI can draw it at once instead
/// of waiting for its own timer.
static WAKE: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
pub(crate) fn set_wake(f: impl Fn() + Send + Sync + 'static) {
    let _ = WAKE.set(Box::new(f));
}
fn wake() {
    if let Some(f) = WAKE.get() {
        f();
    }
}

/// Run `job` on a fresh read-only connection to `workspace` in a background thread. Failures are
/// dropped: the caller asked for optional data (history text, an image).
fn side_call(
    target: &Target,
    workspace: &str,
    size: (u16, u16),
    tx: &SyncSender<Update>,
    job: impl FnOnce(&mut crate::transport::Rpc, &SyncSender<Update>) -> Result<()> + Send + 'static,
) {
    let (target, workspace, tx) = (target.clone(), workspace.to_string(), tx.clone());
    thread::spawn(move || {
        let run = || -> Result<()> {
            let mut rpc = target.connect()?;
            rpc.call(Request::Attach {
                workspace,
                readonly: true,
                rows: size.0,
                cols: size.1,
                cwd: None,
                takeover: false,
            })?;
            job(&mut rpc, &tx)
        };
        let _ = run();
        wake();
    });
}

/// One-line hints rotated in the footer while the session is idle.
const TIPS: [&str; 6] = [
    "Ctrl+Space then z zooms a pane",
    "Ctrl+Space then w jumps to any pane",
    "Ctrl+Space then s cycles layouts: single, vertical, horizontal, grid",
    "Ctrl+Space then h opens retained output; y copies it",
    "Ctrl+Space then t tries another theme",
    "Closing this window only detaches: your shells keep running",
];
fn tip(elapsed: Duration) -> &'static str {
    TIPS[(elapsed.as_secs() / 12) as usize % TIPS.len()]
}

pub(crate) enum Command {
    Input {
        pane: u64,
        data: Vec<u8>,
    },
    Resize(u16, u16),
    New(Option<String>),
    Close(u64),
    /// Close a pane but keep it for a while so it can be reopened.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    SoftClose(u64),
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Reopen,
    Rename(u64, String),
    Capture(u64, CapturePurpose),
    /// Fetch an inline image's bytes for a pane (desktop only).
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Image(u64, u64),
    /// Skip the rest of the reconnect wait and try now.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Reconnect,
    Stop,
    /// Like `Stop`, but first remove this workspace and end its shells (a fresh desktop window).
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Discard,
    /// Take the controller seat from another client (it keeps watching, read-only).
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Takeover,
}
/// True for the daemon's answer to a request from a client that lost its seat to a takeover.
fn lost_seat(e: &anyhow::Error) -> bool {
    e.to_string().contains("taken over")
}
#[derive(Clone)]
pub(crate) enum CapturePurpose {
    View(String),
    Export(String),
    /// Full retained text for the desktop's scrollback view of this pane.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Scrollback(u64),
}
pub(crate) enum Update {
    Online,
    Offline(String),
    State(Vec<PaneView>),
    Message(String),
    Created(u64),
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Captured(String, u64, CapturePurpose),
    Latency(u64),
    /// The daemon's platform and how long it has been running (seconds).
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Daemon(String, u64, Option<crate::hoststats::HostStats>),
    /// Reconnect attempt number and how long the worker waits before it (milliseconds).
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Retry(u32, u64),
    /// An inline image's bytes: pane, image id, file contents.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Image(u64, u64, Vec<u8>),
    /// The worker finished a `Discard` (or had nothing to discard) and is about to exit.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Discarded,
    /// Another client took the controller seat; this one can only watch until it takes it back.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    TakenOver,
    /// The attach was refused because another client holds the controller seat.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    Controlled(String),
}

pub(crate) fn worker(
    target: Target,
    workspace: String,
    readonly: bool,
    cwd: Option<String>,
    rx: Receiver<Command>,
    tx: SyncSender<Update>,
    initial: (u16, u16),
) {
    let mut size = initial;
    let mut backoff = 250u64;
    let mut attempt = 0u32;
    let mut previous_instance: Option<String> = None;
    let mut takeover = false;
    loop {
        let connection = (|| -> Result<_> {
            let mut rpc = target.connect()?;
            rpc.call(Request::Attach {
                workspace: workspace.clone(),
                readonly,
                rows: size.0,
                cols: size.1,
                cwd: cwd.clone(),
                takeover,
            })?;
            Ok(rpc)
        })();
        match connection {
            Ok(mut rpc) => {
                takeover = false;
                let mut seat_lost = false;
                backoff = 250;
                attempt = 0;
                let _ = tx.send(Update::Online);
                let mut info_at = Instant::now();
                let mut undo_ok = false;
                // Screen diffs save bandwidth on a real link; a local socket has none to save.
                let mut diff_ok = false;
                let mut mirrors = crate::screensync::ClientMirrors::default();
                if let Ok(Response::Info {
                    os,
                    arch,
                    uptime,
                    stats,
                    undo,
                    diff,
                    ..
                }) = rpc.call(Request::Info)
                {
                    undo_ok = undo;
                    diff_ok = diff && !target.local();
                    let _ = tx.send(Update::Daemon(format!("{arch} · {os}"), uptime, stats));
                }
                if previous_instance
                    .as_ref()
                    .is_some_and(|old| old != &rpc.instance)
                {
                    let _=tx.send(Update::Message("Remote session daemon changed: previous live sessions are unavailable; this is a fresh attachment.".into()));
                }
                previous_instance = Some(rpc.instance.clone());
                let mut known = BTreeMap::new();
                let mut seq = 0;
                let mut hot_until = Instant::now();
                let mut last_sent = Instant::now();
                let result = (|| -> Result<()> {
                    let mut ticked = Instant::now();
                    loop {
                        if ticked.elapsed() > RESUME_GAP {
                            bail!("resumed after {} s", ticked.elapsed().as_secs());
                        }
                        ticked = Instant::now();
                        match rx.recv_timeout(poll_wait(Instant::now(), hot_until)) {
                            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                                return Ok(())
                            }
                            Ok(Command::Discard) => {
                                let _ = rpc.call(Request::Remove {
                                    workspace: workspace.clone(),
                                    force: true,
                                });
                                let _ = tx.send(Update::Discarded);
                                return Ok(());
                            }
                            Ok(command) => {
                                let req = match command {
                                    Command::Input { pane, data } => {
                                        hot_until = Instant::now() + HOT_FOR;
                                        seq += 1;
                                        Request::Input { pane, seq, data }
                                    }
                                    Command::Resize(rows, cols) => {
                                        size = (rows, cols);
                                        if !readonly {
                                            for &pane in known.keys() {
                                                match rpc.call(Request::Resize { pane, rows, cols })
                                                {
                                                    Err(e) if lost_seat(&e) => {
                                                        if !seat_lost {
                                                            seat_lost = true;
                                                            let _ = tx.send(Update::TakenOver);
                                                        }
                                                        break;
                                                    }
                                                    other => {
                                                        other?;
                                                    }
                                                }
                                            }
                                        }
                                        continue;
                                    }
                                    Command::New(cwd) => Request::NewPane {
                                        cwd,
                                        command: vec![],
                                        rows: size.0,
                                        cols: size.1,
                                    },
                                    Command::Close(pane) => Request::Close { pane },
                                    // A daemon from before this existed just closes the pane for good.
                                    Command::SoftClose(pane) if undo_ok => {
                                        Request::SoftClose { pane }
                                    }
                                    Command::SoftClose(pane) => Request::Close { pane },
                                    Command::Reopen if undo_ok => Request::Reopen,
                                    Command::Reopen => {
                                        let _ = tx.send(Update::Message(
                                            "This host's TERYON is too old to reopen panes; update it".into(),
                                        ));
                                        continue;
                                    }
                                    Command::Rename(pane, title) => Request::Rename { pane, title },
                                    // Big reads run on their own read-only connection so typing and
                                    // polling never wait behind a 15000-line capture.
                                    Command::Capture(pane, purpose) => {
                                        side_call(
                                            &target,
                                            &workspace,
                                            size,
                                            &tx,
                                            move |rpc, tx| {
                                                if let Response::Captured { text, first_line } =
                                                    rpc.call(Request::Capture { pane })?
                                                {
                                                    let _ = tx.send(Update::Captured(
                                                        text, first_line, purpose,
                                                    ));
                                                }
                                                Ok(())
                                            },
                                        );
                                        continue;
                                    }
                                    Command::Image(pane, id) => {
                                        // An image that already scrolled away is not an error.
                                        side_call(
                                            &target,
                                            &workspace,
                                            size,
                                            &tx,
                                            move |rpc, tx| {
                                                if let Ok(Response::ImageData { id, data }) =
                                                    rpc.call(Request::Image { pane, id })
                                                {
                                                    if let Some(bytes) =
                                                        crate::inlineimg::decode_base64(&data)
                                                    {
                                                        let _ =
                                                            tx.send(Update::Image(pane, id, bytes));
                                                    }
                                                }
                                                Ok(())
                                            },
                                        );
                                        continue;
                                    }
                                    Command::Reconnect => continue,
                                    Command::Takeover => {
                                        match rpc.call(Request::Attach {
                                            workspace: workspace.clone(),
                                            readonly,
                                            rows: size.0,
                                            cols: size.1,
                                            cwd: cwd.clone(),
                                            takeover: true,
                                        }) {
                                            Ok(_) => {
                                                seat_lost = false;
                                                let _ = tx.send(Update::Message(
                                                    "You have control of this workspace".into(),
                                                ));
                                            }
                                            Err(e) if e.to_string().starts_with("server:") => {
                                                let _ = tx.send(Update::Message(e.to_string()));
                                            }
                                            Err(e) => return Err(e),
                                        }
                                        continue;
                                    }
                                    Command::Stop | Command::Discard => unreachable!(),
                                };
                                let is_input = matches!(req, Request::Input { .. });
                                match rpc.call(req) {
                                    Ok(Response::Created { pane }) => {
                                        let _ = tx.send(Update::Created(pane));
                                    }
                                    Ok(_) => {}
                                    Err(e) if e.to_string().starts_with("server:") => {
                                        if is_input {
                                            seq = seq.saturating_sub(1);
                                        }
                                        if lost_seat(&e) && !seat_lost {
                                            seat_lost = true;
                                            let _ = tx.send(Update::TakenOver);
                                        }
                                        let _ = tx.send(Update::Message(e.to_string()));
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                        if info_at.elapsed() >= Duration::from_secs(60) {
                            info_at = Instant::now();
                            if let Ok(Response::Info {
                                os,
                                arch,
                                uptime,
                                stats,
                                ..
                            }) = rpc.call(Request::Info)
                            {
                                let _ = tx.send(Update::Daemon(
                                    format!("{arch} · {os}"),
                                    uptime,
                                    stats,
                                ));
                            }
                        }
                        let polled = Instant::now();
                        let state = rpc.call(Request::Poll {
                            known: known
                                .iter()
                                .map(|(id, revision)| (id.to_string(), *revision))
                                .collect(),
                            diff: diff_ok,
                        })?;
                        let poll_ms = polled.elapsed().as_millis() as u64;
                        if let Response::State { mut panes } = state {
                            // Diffs are turned back into whole screens here, so everything past this
                            // point (the UI, the change tracking) sees what it always saw.
                            let mut resync = Vec::new();
                            if diff_ok {
                                mirrors.retain(|id| panes.iter().any(|p| p.id == id));
                                for p in panes.iter_mut() {
                                    match mirrors.apply(
                                        p.id,
                                        p.revision,
                                        (p.rows, p.cols),
                                        p.screen.as_deref(),
                                        p.diff_from,
                                        p.digest,
                                    ) {
                                        crate::screensync::Applied::Plain => {}
                                        crate::screensync::Applied::Rebuilt(full) => {
                                            p.screen = Some(full);
                                            p.diff_from = None;
                                            p.digest = None;
                                        }
                                        crate::screensync::Applied::Resync => {
                                            p.screen = None;
                                            p.diff_from = None;
                                            p.digest = None;
                                            resync.push(p.id);
                                        }
                                    }
                                }
                            }
                            if !readonly && !seat_lost {
                                for pane in &panes {
                                    if (pane.rows, pane.cols) != size {
                                        match rpc.call(Request::Resize {
                                            pane: pane.id,
                                            rows: size.0,
                                            cols: size.1,
                                        }) {
                                            Err(e) if lost_seat(&e) => {
                                                seat_lost = true;
                                                let _ = tx.send(Update::TakenOver);
                                                break;
                                            }
                                            other => {
                                                other?;
                                            }
                                        }
                                    }
                                }
                            }
                            let changed =
                                panes.iter().any(|p| known.get(&p.id) != Some(&p.revision));
                            known = panes.iter().map(|p| (p.id, p.revision)).collect();
                            // A diff that did not fit: say we hold nothing, so the next poll is whole.
                            for id in &resync {
                                known.remove(id);
                            }
                            if changed {
                                hot_until = Instant::now() + HOT_FOR;
                            }
                            // Unchanged polls are only forwarded now and then (latency, titles).
                            if changed || last_sent.elapsed() >= POLL_IDLE {
                                last_sent = Instant::now();
                                let _ = tx.try_send(Update::Latency(poll_ms));
                                if tx.send(Update::State(panes)).is_err() {
                                    return Ok(());
                                }
                                wake();
                            }
                        }
                    }
                })();
                if result.is_ok() {
                    return;
                }
                // ssh's last words say why the link went (closed by the host, broken pipe, timeout).
                let why = rpc.stderr_tail();
                let _ = tx.send(Update::Offline(if why.is_empty() {
                    format!("{} · uncertain input discarded", result.unwrap_err())
                } else {
                    format!(
                        "{why} · {} · uncertain input discarded",
                        result.unwrap_err()
                    )
                }));
            }
            Err(e) => {
                if e.to_string().contains("already has a controller")
                    && tx.send(Update::Controlled(e.to_string())).is_err()
                {
                    return;
                }
                if tx.send(Update::Offline(format!("{e:#}"))).is_err() {
                    return;
                }
            }
        }
        // Discard stale actions throughout backoff, including buffered keystrokes.
        attempt += 1;
        let wait = jittered(
            backoff,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos()),
        );
        let _ = tx.send(Update::Retry(attempt, wait));
        wake();
        let until = Instant::now() + Duration::from_millis(wait);
        while Instant::now() < until {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Ok(Command::Discard) => {
                    let _ = tx.send(Update::Discarded);
                    return;
                }
                Ok(Command::Resize(r, c)) => size = (r, c),
                Ok(Command::Reconnect) => break,
                Ok(Command::Takeover) => {
                    takeover = true;
                    break;
                }
                _ => {}
            }
        }
        while let Ok(cmd) = rx.try_recv() {
            if matches!(cmd, Command::Stop) {
                return;
            }
            if matches!(cmd, Command::Discard) {
                let _ = tx.send(Update::Discarded);
                return;
            }
        }
        backoff = (backoff * 2).min(5000);
    }
}

struct Pane {
    info: PaneView,
    parser: vt100::Parser,
}
enum Mode {
    Terminal,
    Command(String),
    Help,
    History {
        lines: Vec<String>,
        offset: usize,
        query: String,
    },
    Draft,
    ConfirmDraft,
    Switch {
        query: String,
        selected: usize,
    },
}
struct App {
    target: String,
    workspace: String,
    readonly: bool,
    online: bool,
    notice: String,
    panes: BTreeMap<u64, Pane>,
    focus: u64,
    layout: LayoutMode,
    zoom: bool,
    latency: Option<u64>,
    theme: Theme,
    mode: Mode,
    prefix: bool,
    draft: String,
    tx: SyncSender<Command>,
    last_size: (u16, u16),
    started: Instant,
}
impl App {
    fn send(&mut self, command: Command) {
        if !self.online {
            self.notice =
                "Offline: action not sent. Use the draft editor to compose locally.".into();
            return;
        }
        if self.readonly && !matches!(command, Command::Capture(..) | Command::Image(..)) {
            self.notice = "Read-only attachment".into();
            return;
        }
        if self.tx.try_send(command).is_err() {
            self.notice = "Input queue full; action was not queued".into();
        }
    }
    fn input(&mut self, data: Vec<u8>) {
        if data.len() > MAX_INPUT {
            self.notice = "Paste exceeds 64 KiB; use file transfer".into();
            return;
        }
        self.send(Command::Input {
            pane: self.focus,
            data,
        });
    }
    fn command(&mut self, text: &str) -> bool {
        let (name, arg) = text.trim().split_once(' ').unwrap_or((text.trim(), ""));
        match name {
            "detach" | "quit" => return true,
            "new" => self.send(Command::New(if arg.is_empty() {
                None
            } else {
                Some(arg.into())
            })),
            "close" => {
                self.mode = Mode::Command("confirm-close".into());
                self.notice = "Press Enter again to terminate and remove this pane".into();
            }
            "confirm-close" => self.send(Command::Close(self.focus)),
            "rename" => self.send(Command::Rename(self.focus, arg.into())),
            "split" => {
                self.layout = if self.layout == LayoutMode::Single {
                    LayoutMode::Vertical
                } else {
                    LayoutMode::Single
                };
            }
            "layout" => match LayoutMode::parse(arg) {
                Some(mode) => self.layout = mode,
                None => self.notice = "Usage: layout single|vertical|horizontal|grid".into(),
            },
            "zoom" => self.zoom = !self.zoom,
            "switch" => {
                self.mode = Mode::Switch {
                    query: arg.into(),
                    selected: 0,
                }
            }
            "theme" => match Theme::load(arg) {
                Ok(t) => self.theme = t,
                Err(e) => self.notice = e.to_string(),
            },
            "capture" => self.send(Command::Capture(
                self.focus,
                CapturePurpose::View(String::new()),
            )),
            "search" => self.send(Command::Capture(
                self.focus,
                CapturePurpose::View(arg.into()),
            )),
            "export" => {
                if arg.is_empty() {
                    self.notice = "Usage: export /local/new-file.txt".into();
                } else {
                    self.send(Command::Capture(
                        self.focus,
                        CapturePurpose::Export(arg.into()),
                    ));
                }
            }
            "draft" => self.mode = Mode::Draft,
            "send-draft" => {
                if self.draft.is_empty() {
                    self.notice = "No saved draft".into();
                } else {
                    self.mode = Mode::ConfirmDraft;
                }
            }
            "help" => self.mode = Mode::Help,
            "focus" => {
                if let Ok(id) = arg.parse() {
                    if self.panes.contains_key(&id) {
                        self.focus = id;
                    }
                }
            }
            "" => {}
            _ => self.notice = format!("Unknown command: {name}. Use help."),
        }
        false
    }
    fn mode_now(&self) -> LayoutMode {
        if self.zoom {
            LayoutMode::Single
        } else {
            self.layout
        }
    }
    /// Panes on screen and their rectangles inside `area`.
    fn visible(&self, area: Rect) -> Vec<(u64, Rect)> {
        let ids: Vec<u64> = self.panes.keys().copied().collect();
        let mode = self.mode_now();
        let shown = visible_ids(&ids, self.focus, mode);
        let rects = pane_rects(area, mode, shown.len());
        shown.into_iter().zip(rects).collect()
    }
    fn cycle(&mut self) {
        let ids: Vec<_> = self.panes.keys().copied().collect();
        if !ids.is_empty() {
            let i = ids.iter().position(|id| *id == self.focus).unwrap_or(0);
            self.focus = ids[(i + 1) % ids.len()];
        }
    }
    fn update(&mut self, update: Update) {
        match update {
            Update::Online => {
                self.online = true;
                self.notice =
                    "Attached. Remote PTYs survived independently of the connection.".into();
                self.last_size = (0, 0);
            }
            Update::Offline(s) => {
                self.online = false;
                self.notice = s;
            }
            Update::Message(s) | Update::Controlled(s) => self.notice = s,
            Update::TakenOver => {
                self.notice = "Another client took over this workspace; you can only watch".into();
            }
            Update::Latency(ms) => self.latency = Some(ms),
            Update::Discarded | Update::Image(..) | Update::Daemon(..) | Update::Retry(..) => {}
            Update::Created(id) => self.focus = id,
            Update::Captured(text, _, purpose) => match purpose {
                CapturePurpose::View(query) => {
                    let lines: Vec<_> = text
                        .lines()
                        .filter(|l| {
                            query.is_empty() || l.to_lowercase().contains(&query.to_lowercase())
                        })
                        .map(str::to_owned)
                        .collect();
                    self.mode = Mode::History {
                        lines,
                        offset: 0,
                        query,
                    };
                }
                CapturePurpose::Scrollback(_) => {}
                CapturePurpose::Export(path) => {
                    self.notice =
                        match util::private_write(std::path::Path::new(&path), text.as_bytes()) {
                            Ok(()) => format!("Exported to {path}"),
                            Err(e) => format!("Export failed: {e}"),
                        };
                }
            },
            Update::State(views) => {
                self.panes.retain(|id, _| views.iter().any(|p| p.id == *id));
                for view in views {
                    let pane = self.panes.entry(view.id).or_insert_with(|| Pane {
                        parser: vt100::Parser::new(view.rows, view.cols, 0),
                        info: view.clone(),
                    });
                    if let Some(bytes) = &view.screen {
                        pane.parser = vt100::Parser::new(view.rows, view.cols, 0);
                        pane.parser.process(bytes);
                    }
                    if let (None, Some(code)) = (pane.info.exited, view.exited) {
                        self.notice = format!("Pane {} exited ({})", view.id, code);
                    }
                    pane.info = view;
                }
                if !self.panes.contains_key(&self.focus) {
                    self.focus = self.panes.keys().next().copied().unwrap_or(1);
                }
            }
        }
    }
}

fn terminal_area(size: Rect) -> Rect {
    let sidebar = if size.width >= 90 { 20 } else { 0 };
    Rect::new(
        size.x + sidebar,
        size.y + 3,
        size.width.saturating_sub(sidebar),
        size.height.saturating_sub(6),
    )
}
/// Pane ids whose id, title or directory contains `query` (case-insensitive).
fn switch_matches(panes: &BTreeMap<u64, Pane>, query: &str) -> Vec<u64> {
    let q = query.to_lowercase();
    panes
        .iter()
        .filter(|(id, p)| {
            q.is_empty()
                || format!("{id} {} {}", p.info.title, p.info.cwd)
                    .to_lowercase()
                    .contains(&q)
        })
        .map(|(id, _)| *id)
        .collect()
}
const MAX_VISIBLE: usize = 4;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LayoutMode {
    Single,
    Vertical,
    Horizontal,
    Grid,
}
impl LayoutMode {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name.trim() {
            "single" => Self::Single,
            "vertical" | "v" => Self::Vertical,
            "horizontal" | "h" => Self::Horizontal,
            "grid" => Self::Grid,
            _ => return None,
        })
    }
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Single => Self::Vertical,
            Self::Vertical => Self::Horizontal,
            Self::Horizontal => Self::Grid,
            Self::Grid => Self::Single,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::Grid => "grid",
        }
    }
    /// Cell grid `(rows, columns)` for `n` panes.
    pub(crate) fn dims(self, n: usize) -> (u16, u16) {
        let n = n.clamp(1, MAX_VISIBLE) as u16;
        match self {
            Self::Single => (1, 1),
            Self::Vertical => (n, 1),
            Self::Horizontal => (1, n),
            Self::Grid => {
                let cols = if n > 1 { 2 } else { 1 };
                (n.div_ceil(cols), cols)
            }
        }
    }
}
/// Pane ids shown for a layout: all of them when they fit, otherwise a window that contains the focus.
pub(crate) fn visible_ids(ids: &[u64], focus: u64, mode: LayoutMode) -> Vec<u64> {
    if mode == LayoutMode::Single || ids.is_empty() {
        return vec![focus];
    }
    if ids.len() <= MAX_VISIBLE {
        return ids.to_vec();
    }
    let at = ids.iter().position(|id| *id == focus).unwrap_or(0);
    let start = at.min(ids.len() - MAX_VISIBLE);
    ids[start..start + MAX_VISIBLE].to_vec()
}
/// Split `area` into `n` cells; a short last row stretches its panes across the full width.
fn pane_rects(area: Rect, mode: LayoutMode, n: usize) -> Vec<Rect> {
    let n = n.clamp(1, MAX_VISIBLE);
    let (rows, cols) = mode.dims(n);
    let (rows, cols) = (usize::from(rows), usize::from(cols));
    let span = |start: usize, count: usize, total: u16| {
        let t = usize::from(total);
        (
            (t * start / count) as u16,
            (t * (start + 1) / count - t * start / count) as u16,
        )
    };
    (0..n)
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            let in_row = (n - r * cols).min(cols);
            let (y, h) = span(r, rows, area.height);
            let (x, w) = span(c, in_row, area.width);
            Rect::new(area.x + x, area.y + y, w, h)
        })
        .collect()
}
fn desired_size(size: Rect, mode: LayoutMode, n: usize) -> (u16, u16) {
    let a = terminal_area(size);
    let (rows, cols) = mode.dims(n);
    (
        (a.height / rows).saturating_sub(2).clamp(2, 160),
        (a.width / cols).saturating_sub(2).clamp(10, 400),
    )
}
pub(crate) fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, b)| a | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
fn vt_color(c: vt100::Color, default: Color, theme: &Theme) -> Color {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(n) => theme.indexed(n),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
fn render_pane(f: &mut Frame, rect: Rect, pane: &Pane, theme: &Theme, focused: bool) {
    let title = format!(
        " {} / {}{} ",
        pane.info.id,
        pane.info.title,
        if pane.info.exited.is_some() {
            " · exited"
        } else {
            ""
        }
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focused { theme.accent } else { theme.border }))
        .style(Style::default().bg(theme.background).fg(theme.foreground));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let screen = pane.parser.screen();
    for row in 0..inner.height.min(pane.info.rows) {
        for col in 0..inner.width.min(pane.info.cols) {
            if let Some(cell) = screen.cell(row, col) {
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut style = Style::default()
                    .fg(vt_color(cell.fgcolor(), theme.foreground, theme))
                    .bg(vt_color(cell.bgcolor(), theme.background, theme));
                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.italic() {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse() {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let text = cell.contents();
                f.buffer_mut().set_stringn(
                    inner.x + col,
                    inner.y + row,
                    if text.is_empty() { " " } else { text },
                    usize::from(inner.width - col),
                    style,
                );
            }
        }
    }
    if focused && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        if row < inner.height && col < inner.width {
            f.set_cursor_position((inner.x + col, inner.y + row));
        }
    }
}
fn draw(f: &mut Frame, app: &App) {
    let size = f.area();
    let t = &app.theme;
    f.render_widget(
        Block::default().style(Style::default().bg(t.background).fg(t.foreground)),
        size,
    );
    let top = Rect::new(0, 0, size.width, 3.min(size.height));
    let (dot, status) = if app.online {
        ("● ", "CONNECTED")
    } else {
        ("◌ ", "RECONNECTING")
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  T",
                Style::default()
                    .fg(Color::Rgb(0x0a, 0x84, 0xff))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▪ ", Style::default().fg(Color::Rgb(0xf5, 0xf5, 0xf7))),
            Span::styled(
                "TERYON  ",
                Style::default()
                    .fg(t.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{} / {}  ", app.target, app.workspace)),
            Span::styled(
                dot,
                Style::default().fg(if app.online { t.success } else { t.warning }),
            ),
            Span::styled(status, Style::default().fg(t.muted)),
            Span::raw(if app.readonly { " · READ ONLY" } else { "" }),
            Span::styled(
                format!(
                    "  {}{} · {} pane{}{} · {}",
                    app.layout.name(),
                    if app.zoom { " (zoom)" } else { "" },
                    app.panes.len(),
                    if app.panes.len() == 1 { "" } else { "s" },
                    app.latency
                        .map_or(String::new(), |ms| format!(" · {ms} ms")),
                    util::clock_hhmm()
                ),
                Style::default().fg(t.muted),
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(t.border)),
        )
        .style(Style::default().bg(t.panel)),
        top,
    );
    let area = terminal_area(size);
    if size.width >= 90 {
        let mut lines = vec![
            Line::styled(" WORKSPACE", Style::default().fg(t.muted)),
            Line::raw(format!(" {}", app.workspace)),
            Line::raw(""),
            Line::styled(" PANES", Style::default().fg(t.muted)),
        ];
        for (id, p) in &app.panes {
            lines.push(Line::styled(
                format!(
                    " {} {} {}",
                    if *id == app.focus { "›" } else { " " },
                    id,
                    p.info.title
                ),
                Style::default().fg(if *id == app.focus {
                    t.accent
                } else {
                    t.foreground
                }),
            ));
        }
        lines.extend([
            Line::raw(""),
            Line::styled(" THEME", Style::default().fg(t.muted)),
            Line::raw(format!(" {}", t.name)),
            Line::raw(""),
            Line::styled(" Ctrl+Space → ?", Style::default().fg(t.muted)),
        ]);
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(t.panel)),
            Rect::new(0, 3, 20, size.height.saturating_sub(6)),
        );
    }
    for (id, rect) in app.visible(area) {
        if let Some(p) = app.panes.get(&id) {
            render_pane(f, rect, p, t, id == app.focus);
        }
    }
    if size.height >= 3 {
        let footer = Rect::new(0, size.height - 3, size.width, 3);
        let mut notice = app.notice.clone();
        if app.online && notice.starts_with("Attached.") {
            notice = format!("{notice}  ·  Tip: {}", tip(app.started.elapsed()));
        }
        if !app.online {
            notice = format!("Cached screen · {notice}");
        }
        f.render_widget(Paragraph::new(vec![Line::styled(notice,Style::default().fg(if app.online{t.muted}else{t.warning})),Line::raw(format!(" Ctrl+Space → n new · Tab focus · s layout · z zoom · w switch · t theme · : commands · d detach  |  {}",if app.prefix{"PREFIX: choose action"}else{"remote input"}))]).wrap(Wrap{trim:false}).style(Style::default().bg(t.panel)),footer);
    }
    let modal=match &app.mode{
        Mode::Terminal=>return,
        Mode::Command(text)=>("Command palette · Enter run / Esc cancel".into(),format!("❯ {text}\n\nnew [remote directory] · rename TITLE · close · split\nlayout single|vertical|horizontal|grid · zoom · switch [TEXT]\ntheme NAME · search TEXT · capture · export /local/file\ndraft · send-draft · focus ID · detach · help")),
        Mode::Help=>("Keyboard & commands · Esc close".into(),format!("Ctrl+Space opens the prefix layer\n n  New pane     Tab Next pane     s Cycle layout\n z  Zoom pane    w Switch pane     x Close pane\n t  Cycle theme  : Command palette d Detach\n /  Search       h History        e Draft editor\n ?  This help    Ctrl+Space again sends NUL\n\nNormal keys go to your remote program.\nRead-only viewers cannot type or resize.\n\nThemes: {}\n\nTransfers: teryon push/pull HOST SOURCE DEST\nPorts: teryon forward HOST LOCAL:REMOTE\nDisconnects never replay queued input.",BUILTINS.join(", "))),
        Mode::History{lines,offset,query}=>(format!("Retained output · {} matches · {} · ↑↓ PgUp/PgDn · y yank · Esc",lines.len(),query),lines.iter().skip(*offset).take(usize::from(size.height.saturating_sub(8))).cloned().collect::<Vec<_>>().join("\n")),
        Mode::Switch{query,selected}=>{
            let ids=switch_matches(&app.panes, query);
            let list=ids.iter().enumerate().filter_map(|(i,id)|app.panes.get(id).map(|p|format!("{} {} / {}  {}",if i==*selected{"›"}else{" "},id,p.info.title,p.info.cwd))).collect::<Vec<_>>().join("\n");
            ("Switch pane · type to filter · ↑↓ Enter / Esc".into(),format!("❯ {query}\n\n{list}"))
        }
        Mode::Draft=>("Offline draft · Enter save / Esc close".into(),format!("{}\n\nSaved locally in memory. Use send-draft to review and submit.",app.draft)),
        Mode::ConfirmDraft=>("Review draft · Enter SEND / Esc cancel".into(),format!("Destination: {} / {} / pane {}\n\n{}\n\nThis sends the draft followed by Enter to the remote shell.",app.target,app.workspace,app.focus,app.draft)),
    };
    let rect = Rect::new(
        size.width / 20,
        size.height / 6,
        size.width.saturating_sub(size.width / 10),
        size.height.saturating_sub(size.height / 3),
    );
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(modal.1)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(modal.0)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent)),
            )
            .style(Style::default().bg(t.panel).fg(t.foreground)),
        rect,
    );
}

pub fn run(
    target: Target,
    workspace: String,
    readonly: bool,
    cwd: Option<String>,
    theme: Theme,
    mouse: bool,
) -> Result<()> {
    if !valid_name(&workspace) {
        bail!("invalid workspace name");
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        bail!("interactive terminal required; use exec/capture for scripts");
    }
    let mut terminal = ratatui::init();
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = execute!(
                std::io::stdout(),
                DisableMouseCapture,
                DisableBracketedPaste
            );
            ratatui::restore();
        }
    }
    let _restore = Restore;
    execute!(std::io::stdout(), EnableBracketedPaste)?;
    if mouse {
        execute!(std::io::stdout(), EnableMouseCapture)?;
    }
    let size = terminal.size()?;
    let initial = desired_size(
        Rect::new(0, 0, size.width, size.height),
        LayoutMode::Single,
        1,
    );
    let (tx, rx) = mpsc::sync_channel(64);
    let (updates_tx, updates) = mpsc::sync_channel(32);
    let worker_target = target.clone();
    let worker_ws = workspace.clone();
    thread::spawn(move || {
        worker(
            worker_target,
            worker_ws,
            readonly,
            cwd,
            rx,
            updates_tx,
            initial,
        )
    });
    let mut app = App {
        target: target.host,
        workspace,
        readonly,
        online: false,
        notice: "Connecting…".into(),
        panes: BTreeMap::new(),
        focus: 1,
        layout: LayoutMode::Single,
        zoom: false,
        latency: None,
        theme,
        mode: Mode::Terminal,
        prefix: false,
        draft: String::new(),
        tx,
        last_size: initial,
        started: Instant::now(),
    };
    let result = (|| -> Result<()> {
        loop {
            while let Ok(u) = updates.try_recv() {
                app.update(u);
            }
            let size = terminal.size()?;
            let screen = Rect::new(0, 0, size.width, size.height);
            let wanted = desired_size(
                screen,
                app.mode_now(),
                app.visible(terminal_area(screen)).len(),
            );
            if app.online
                && wanted != app.last_size
                && app.tx.try_send(Command::Resize(wanted.0, wanted.1)).is_ok()
            {
                app.last_size = wanted;
            }
            terminal.draw(|f| draw(f, &app))?;
            if !event::poll(Duration::from_millis(33))? {
                continue;
            }
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    if k.code == KeyCode::Esc && !matches!(app.mode, Mode::Terminal) {
                        app.mode = Mode::Terminal;
                        continue;
                    }
                    match &mut app.mode {
                        Mode::Command(text) => match k.code {
                            KeyCode::Enter => {
                                let command = text.clone();
                                app.mode = Mode::Terminal;
                                if app.command(&command) {
                                    break;
                                }
                            }
                            KeyCode::Backspace => {
                                text.pop();
                            }
                            KeyCode::Char(c) => text.push(c),
                            _ => {}
                        },
                        Mode::Draft => match k.code {
                            KeyCode::Enter => {
                                app.mode = Mode::Terminal;
                                app.notice =
                                    "Draft saved in memory; use send-draft when ready".into();
                            }
                            KeyCode::Backspace => {
                                app.draft.pop();
                            }
                            KeyCode::Char(c) if app.draft.len() < 32768 => {
                                app.draft.push(c);
                            }
                            _ => {}
                        },
                        Mode::ConfirmDraft => {
                            if k.code == KeyCode::Enter {
                                if app.online && !app.readonly {
                                    let data = format!("{}\r", app.draft).into_bytes();
                                    if data.len() <= MAX_INPUT
                                        && app
                                            .tx
                                            .try_send(Command::Input {
                                                pane: app.focus,
                                                data,
                                            })
                                            .is_ok()
                                    {
                                        app.draft.clear();
                                        app.mode = Mode::Terminal;
                                    } else {
                                        app.notice =
                                            "Draft retained: input queue full or text too large"
                                                .into();
                                    }
                                } else {
                                    app.notice =
                                        "Connect with write access before submitting".into();
                                }
                            }
                        }
                        Mode::History { lines, offset, .. } => match k.code {
                            KeyCode::Up => *offset = offset.saturating_sub(1),
                            KeyCode::Down => {
                                *offset = (*offset + 1).min(lines.len().saturating_sub(1))
                            }
                            KeyCode::Char('y') => {
                                let mut text = lines.join("\n");
                                let mut end = text.len().min(MAX_INPUT);
                                while !text.is_char_boundary(end) {
                                    end -= 1;
                                }
                                text.truncate(end);
                                print!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
                                let _ = std::io::Write::flush(&mut std::io::stdout());
                                app.notice = format!(
                                    "Yanked {} line(s) via OSC 52 (terminal must allow clipboard writes)",
                                    lines.len()
                                );
                                app.mode = Mode::Terminal;
                            }
                            KeyCode::PageUp => *offset = offset.saturating_sub(10),
                            KeyCode::PageDown => {
                                *offset = (*offset + 10).min(lines.len().saturating_sub(1))
                            }
                            _ => {}
                        },
                        Mode::Switch { query, selected } => {
                            let q = query.clone();
                            match k.code {
                                KeyCode::Enter => {
                                    let pick =
                                        switch_matches(&app.panes, &q).get(*selected).copied();
                                    if let Some(id) = pick {
                                        app.focus = id;
                                    }
                                    app.mode = Mode::Terminal;
                                }
                                KeyCode::Up => *selected = selected.saturating_sub(1),
                                KeyCode::Down => *selected += 1,
                                KeyCode::Backspace => {
                                    query.pop();
                                    *selected = 0;
                                }
                                KeyCode::Char(c) if query.len() < 64 => {
                                    query.push(c);
                                    *selected = 0;
                                }
                                _ => {}
                            }
                            if let Mode::Switch { selected, .. } = &mut app.mode {
                                *selected = (*selected)
                                    .min(switch_matches(&app.panes, &q).len().saturating_sub(1));
                            }
                        }
                        Mode::Help => {}
                        Mode::Terminal => {
                            let prefix = k.code == KeyCode::Char(' ')
                                && k.modifiers.contains(KeyModifiers::CONTROL)
                                || k.code == KeyCode::Null;
                            if app.prefix {
                                app.prefix = false;
                                match k.code {
                                    KeyCode::Char('d') => break,
                                    KeyCode::Char('n') => app.send(Command::New(None)),
                                    KeyCode::Tab => app.cycle(),
                                    KeyCode::Char('s') => app.layout = app.layout.next(),
                                    KeyCode::Char('z') => app.zoom = !app.zoom,
                                    KeyCode::Char('w') => {
                                        app.mode = Mode::Switch {
                                            query: String::new(),
                                            selected: 0,
                                        }
                                    }
                                    KeyCode::Char('x') => {
                                        app.command("close");
                                    }
                                    KeyCode::Char('t') => {
                                        let i = BUILTINS
                                            .iter()
                                            .position(|n| {
                                                Theme::load(n)
                                                    .is_ok_and(|t| t.name == app.theme.name)
                                            })
                                            .unwrap_or(0);
                                        app.theme =
                                            Theme::load(BUILTINS[(i + 1) % BUILTINS.len()])?;
                                    }
                                    KeyCode::Char(':') => app.mode = Mode::Command(String::new()),
                                    KeyCode::Char('/') => {
                                        app.mode = Mode::Command("search ".into())
                                    }
                                    KeyCode::Char('h') => app.send(Command::Capture(
                                        app.focus,
                                        CapturePurpose::View(String::new()),
                                    )),
                                    KeyCode::Char('e') => app.mode = Mode::Draft,
                                    KeyCode::Char('?') => app.mode = Mode::Help,
                                    _ if prefix => app.input(vec![0]),
                                    _ => {}
                                }
                            } else if prefix {
                                app.prefix = true;
                            } else {
                                let app_cursor = app
                                    .panes
                                    .get(&app.focus)
                                    .is_some_and(|p| p.parser.screen().application_cursor());
                                if let Some(bytes) = keys::encode(k, app_cursor) {
                                    app.input(bytes);
                                }
                            }
                        }
                    }
                }
                Event::Mouse(m) if matches!(app.mode, Mode::Terminal) => {
                    let screen = Rect::new(0, 0, size.width, size.height);
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            let hit = app
                                .visible(terminal_area(screen))
                                .into_iter()
                                .find(|(_, r)| r.contains((m.column, m.row).into()))
                                .map(|(id, _)| id);
                            let listed = (size.width >= 90 && m.column < 20 && m.row >= 7)
                                .then(|| app.panes.keys().nth(usize::from(m.row - 7)).copied())
                                .flatten();
                            if let Some(id) = hit.or(listed) {
                                app.focus = id;
                            }
                        }
                        MouseEventKind::ScrollUp => app.send(Command::Capture(
                            app.focus,
                            CapturePurpose::View(String::new()),
                        )),
                        _ => {}
                    }
                }
                Event::Paste(text) => match &mut app.mode {
                    Mode::Terminal => {
                        let bracketed = app
                            .panes
                            .get(&app.focus)
                            .is_some_and(|p| p.parser.screen().bracketed_paste());
                        app.input(keys::paste(&text, bracketed));
                    }
                    Mode::Draft => {
                        if app.draft.len() + text.len() <= 32768 {
                            app.draft.push_str(&text.replace(['\r', '\n'], " "));
                        }
                    }
                    Mode::Command(s) if s.len() + text.len() < 8192 => {
                        s.push_str(&text.replace(['\r', '\n'], " "));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(())
    })();
    let _ = app.tx.try_send(Command::Stop);
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn reconnect_waits_are_spread_but_stay_near_the_backoff() {
        for seed in [0, 1, 7, 999_999_999, u32::MAX] {
            let w = jittered(1000, seed);
            assert!((800..=1200).contains(&w), "{w}");
        }
        assert_eq!(jittered(250, 3), 250 - 50 + 3);
        assert_eq!(jittered(3, 99), 3, "tiny waits are left alone");
        assert_ne!(jittered(5000, 1), jittered(5000, 2));
    }
    #[test]
    fn polling_is_fast_only_while_hot() {
        let now = std::time::Instant::now();
        let later = now + std::time::Duration::from_millis(100);
        assert_eq!(super::poll_wait(now, later), super::POLL_HOT);
        assert_eq!(super::poll_wait(later, now), super::POLL_IDLE);
        assert!(super::POLL_HOT < super::POLL_IDLE);
    }
    use super::*;
    #[test]
    fn layouts_tile_the_area_without_overlap() {
        let area = Rect::new(2, 3, 100, 40);
        for mode in [
            LayoutMode::Vertical,
            LayoutMode::Horizontal,
            LayoutMode::Grid,
        ] {
            for n in 1..=MAX_VISIBLE {
                let rects = pane_rects(area, mode, n);
                assert_eq!(rects.len(), n);
                let cells: u32 = rects
                    .iter()
                    .map(|r| u32::from(r.width) * u32::from(r.height))
                    .sum();
                assert_eq!(cells, 100 * 40, "{mode:?} {n}");
                for (i, a) in rects.iter().enumerate() {
                    assert!(area.contains(a.as_position()));
                    for b in &rects[i + 1..] {
                        assert!(!a.intersects(*b), "{mode:?} {n}");
                    }
                }
            }
        }
    }
    #[test]
    fn visible_window_keeps_focus_and_single_ignores_others() {
        let ids = [1, 2, 3, 4, 5, 6];
        assert_eq!(visible_ids(&ids, 6, LayoutMode::Grid), [3, 4, 5, 6]);
        assert_eq!(visible_ids(&ids, 1, LayoutMode::Vertical), [1, 2, 3, 4]);
        assert_eq!(visible_ids(&ids, 3, LayoutMode::Single), [3]);
        assert_eq!(visible_ids(&[], 1, LayoutMode::Grid), [1]);
    }
    #[test]
    fn pane_size_shrinks_with_more_cells() {
        let screen = Rect::new(0, 0, 120, 40);
        let one = desired_size(screen, LayoutMode::Single, 1);
        let grid = desired_size(screen, LayoutMode::Grid, 4);
        assert!(grid.0 < one.0 && grid.1 < one.1);
    }
    #[test]
    fn tips_rotate_and_wrap() {
        assert_eq!(tip(Duration::from_secs(0)), TIPS[0]);
        assert_eq!(tip(Duration::from_secs(12)), TIPS[1]);
        assert_eq!(tip(Duration::from_secs(12 * TIPS.len() as u64)), TIPS[0]);
    }
    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }
    #[test]
    fn terminal_ui_renders_small_and_large_light_dark_disconnected_views() {
        for theme in BUILTINS {
            for (width, height) in [(36, 12), (120, 40)] {
                let (tx, _rx) = mpsc::sync_channel(4);
                let info = PaneView {
                    id: 1,
                    title: "Shell".into(),
                    cwd: "/tmp".into(),
                    pid: Some(1),
                    rows: 24,
                    cols: 80,
                    revision: 1,
                    screen: None,
                    exited: None,
                    ..Default::default()
                };
                let mut parser = vt100::Parser::new(24, 80, 0);
                parser.process("\x1b[32mUnicode λ界\x1b[0m".as_bytes());
                let mut app = App {
                    target: "mini".into(),
                    workspace: "test".into(),
                    readonly: false,
                    online: false,
                    notice: "Connection interrupted".into(),
                    panes: BTreeMap::from([(1, Pane { info, parser })]),
                    focus: 1,
                    layout: LayoutMode::Grid,
                    zoom: false,
                    latency: Some(12),
                    theme: Theme::load(theme).unwrap(),
                    mode: Mode::Terminal,
                    prefix: false,
                    draft: String::new(),
                    tx,
                    last_size: (24, 80),
                    started: Instant::now(),
                };
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                        .unwrap();
                terminal.draw(|f| draw(f, &app)).unwrap();
                app.mode = Mode::Help;
                terminal.draw(|f| draw(f, &app)).unwrap();
                app.mode = Mode::Switch {
                    query: String::new(),
                    selected: 0,
                };
                terminal.draw(|f| draw(f, &app)).unwrap();
                app.mode = Mode::ConfirmDraft;
                app.draft = "echo reviewed".into();
                terminal.draw(|f| draw(f, &app)).unwrap();
            }
        }
    }
}

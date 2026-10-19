//! Optional native GPU front end. All networking stays off the UI thread.
use crate::{
    config::{self, DesktopConfig},
    keymap::{self, Keymap, Mods},
    keys,
    protocol::*,
    settings::DesktopSettings,
    theme::{mix, Theme, BUILTINS},
    transport::Target,
    ui::{visible_ids, worker, CapturePurpose, Command, LayoutMode, Update},
    uikit,
};
use anyhow::{bail, Result};
use eframe::egui::{self, Color32, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::{
    collections::{BTreeMap, HashMap},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

mod actions;
mod agents;
mod ai;
mod aicmd;
mod away;
mod bar;
mod blockmenu;
mod blur;
mod cmdhist;
mod dropzone;
mod files;
mod find;
mod fontpicker;
mod floating;
mod hints;
mod keyeditor;
mod icons;
mod menus;
mod panes;
mod portfwd;
mod relink;
mod settingsui;
mod restore;
mod snippets;
mod style;
mod tabs;
mod themepicker;
mod term;
mod triggers;
mod welcome;
use actions::{actions, theme_counterpart, theme_step, ActionKind, Scroll, ARG_COMMANDS};
use find::FindQuery;
use hints::HintMode;
use icons::*;
use panes::{arrange, Dir, Weights};
use snippets::SnippetPrompt;
use style::*;
use term::Scrollback;
use triggers::{CompiledTrigger, Span};

struct Pane {
    info: PaneView,
    parser: vt100::Parser,
    /// Shell-integration marks (kept between polls; the daemon sends them only when the screen changes).
    marks: Vec<Mark>,
    /// Exit-status notification bookkeeping: unix time of the last `D` mark already announced.
    announced: u64,
    /// Trigger highlights for the current screen, and whether a first scan has happened.
    spans: Vec<Span>,
    scanned: bool,
    /// AI agent detection and status for this pane.
    track: agents::Track,
    /// Inline images: where they sit, which ids were asked for, their bytes and decoded textures.
    placements: Vec<crate::protocol::Placement>,
    image_asked: std::collections::HashSet<u64>,
    image_raw: HashMap<u64, Vec<u8>>,
    image_tex: HashMap<u64, egui::TextureHandle>,
}
impl Pane {
    fn new(info: PaneView, parser: vt100::Parser) -> Self {
        Self {
            info,
            parser,
            marks: Vec::new(),
            announced: 0,
            spans: Vec::new(),
            scanned: false,
            track: agents::Track::default(),
            placements: Vec::new(),
            image_asked: Default::default(),
            image_raw: HashMap::new(),
            image_tex: HashMap::new(),
        }
    }
}
/// Per-workspace odds and ends: the floating pane and the saved-layout bookkeeping.
#[derive(Default)]
struct SessionExtra {
    floating: Option<u64>,
    restore_checked: bool,
    restore_offer: Option<crate::layout::Layout>,
    /// Titles for panes a restore is about to create, with when they were queued.
    pending_titles: Vec<(String, Instant)>,
    last_layout: Option<crate::layout::Layout>,
    last_layout_save: Option<Instant>,
    /// "While you were away": the card text and the time you last looked; checked once per attach.
    away: Option<(Vec<String>, u64)>,
    away_checked: bool,
    away_touched: Option<Instant>,
}
/// Everything that belongs to one attached workspace; parked tabs keep theirs here.
struct Session {
    extra: SessionExtra,
    host: String,
    target: Option<Target>,
    workspace: String,
    online: bool,
    notice: String,
    panes: BTreeMap<u64, Pane>,
    focus: u64,
    layout: LayoutMode,
    tx: SyncSender<Command>,
    rx: Receiver<Update>,
    history: Option<String>,
    query: String,
    last_size: (u16, u16),
}
/// A tab; the active one's state lives in `Desktop`, so `state` is None for it.
struct Tab {
    host: String,
    name: String,
    state: Option<Session>,
    /// Whether this tab, while in the background, has news for you.
    activity: tabs::Activity,
}
pub struct Desktop {
    host: String,
    workspace: String,
    readonly: bool,
    online: bool,
    notice: String,
    theme: Theme,
    panes: BTreeMap<u64, Pane>,
    focus: u64,
    layout: LayoutMode,
    target: Option<Target>,
    /// Row and column shares of the visible layout, set by dragging dividers.
    weights: Weights,
    /// Where each visible pane was drawn last frame (for directional focus).
    pane_rects: BTreeMap<u64, Rect>,
    /// How much unfocused panes are dimmed (0 = not at all).
    unfocused_dim: f32,
    /// Per pane: the finished-command text last shown as a toast and when it appeared (seconds).
    wake_set: bool,
    /// Focus state at the start of the frame, before Tab or the arrows can move it (egui hands
    /// focus to the next widget while the UI is built, and `keyboard` runs after that).
    focus_at_start: Option<(bool, bool)>,
    /// Tab or an arrow was pressed in the terminal: take focus back at the next frame.
    restore_focus: bool,
    /// When the active tab last changed, for the short cross-fade.
    tab_fade_at: Option<Instant>,
    chip_toasts: HashMap<u64, (String, f64)>,
    /// Grain texture for `background_noise`.
    noise_tex: Option<egui::TextureHandle>,
    /// Per pane: the scrolled-back position on screen, in lines, easing toward the real offset.
    scroll_visual: HashMap<u64, f32>,
    /// A running theme cross-fade: from, to, start time; and the theme currently on screen.
    theme_fade: Option<(Theme, Theme, f64)>,
    theme_shown: Theme,
    /// Enter native full screen on the first frame (`fullscreen` option).
    pending_fullscreen: bool,
    /// What the find bar (or the `search` command) looks for, and how.
    find_query: Option<FindQuery>,
    find_regex: bool,
    find_case: bool,
    find_error: Option<String>,
    /// Quick-select labels, the snippet being filled in, and broadcast input.
    hints: Option<HintMode>,
    snippet_prompt: Option<SnippetPrompt>,
    broadcast: bool,
    /// Scrolled-back views by pane, rows last drawn per pane, and mouse bookkeeping.
    scrollback: BTreeMap<u64, Scrollback>,
    view_rows: BTreeMap<u64, u16>,
    mouse_down: Option<(u64, u8)>,
    last_cell: Option<(u16, u16)>,
    wheel_accum: f32,
    window_focused: bool,
    /// Close-confirmation dialog state (see `confirm_quit`).
    quit_dialog: bool,
    quit_ok: bool,
    /// Desktop options from `config.toml` and the key bindings built from them.
    cfg: DesktopConfig,
    /// Font family last installed (None until the real app has done it).
    applied_font: Option<Option<String>>,
    keymap: Keymap,
    config_mtime: Option<std::time::SystemTime>,
    /// Top-level `theme` of `config.toml` as last read; a change to it is applied live.
    config_theme: Option<String>,
    /// `[notify]` sinks (ntfy, webhook) the app also uses when it notifies from the background.
    notify_cfg: crate::config::NotifyConfig,
    /// `ssh -L` tunnels started by "Open listening port", by port; ended when the app closes.
    forwards: HashMap<u16, std::process::Child>,
    config_checked: Instant,
    /// Window commands queued by actions, sent with the egui context on the next frame.
    pending: Vec<egui::ViewportCommand>,
    fullscreen: bool,
    on_top: bool,
    /// Palette catalogs (refreshed when it opens) and welcome/cheat-sheet state.
    theme_names: Vec<String>,
    open_entries: Vec<(String, String)>,
    palette_was_open: bool,
    cheatsheet: bool,
    tip_index: usize,
    offline_since: Option<Instant>,
    link: relink::Link,
    theme_id_cache: Option<(String, Option<String>)>,
    /// Last macOS appearance seen, so a manual theme choice sticks until it changes.
    last_system_dark: Option<bool>,
    /// Floating pane, saved-layout state, triggers and uploads.
    extra: SessionExtra,
    pending_float: bool,
    /// Columns and rows to size the window to on the first frame (`initial_cols`/`initial_rows`).
    pending_cells: Option<(u16, u16)>,
    /// Remove this window's own workspace (ending its shells) when the window closes.
    discard_workspace: bool,
    triggers: Vec<CompiledTrigger>,
    trigger_seen: HashMap<u64, Instant>,
    upload_tx: mpsc::Sender<(bool, String)>,
    upload_rx: Receiver<(bool, String)>,
    /// Message currently floating in the corner, and the last `notice` already turned into one.
    toast: Option<(String, Instant)>,
    toasted: String,
    window_title: String,
    latency: Option<u64>,
    /// Text queued for the clipboard; flushed with the egui context each frame.
    copy_text: Option<String>,
    ai: ai::Panel,
    files: files::Browser,
    agents_open: bool,
    aicmd: aicmd::Bar,
    cmdhist: cmdhist::Bar,
    /// A failed-command chip was clicked: explain this pane's failure with the assistant.
    explain_req: Option<u64>,
    /// A choice from the right-click or dropdown menu, applied next frame.
    menu_pick: Option<(u64, menus::MenuPick)>,
    renaming: Option<(u64, String)>,
    zoom_prev: Option<LayoutMode>,
    selection: Option<Selection>,
    find_open: bool,
    find_focus: bool,
    find_text: String,
    find_cur: usize,
    find_count: usize,
    palette_sel: usize,
    /// Write preferences to disk (off in tests).
    persist: bool,
    saved: DesktopSettings,
    last_save: Instant,
    win_size: Vec2,
    tabs: Vec<Tab>,
    active: usize,
    /// Theme the user chose; `theme` may differ while hovering previews in the picker.
    committed_theme: Theme,
    tx: SyncSender<Command>,
    rx: Receiver<Update>,
    palette: bool,
    theme_picker: themepicker::ThemePicker,
    font_picker: fontpicker::FontPicker,
    settings_win: settingsui::SettingsWindow,
    appearance_open: bool,
    /// The command block under the last right-click, offered in the pane's context menu.
    block_menu: Option<blockmenu::BlockRef>,
    /// The command and exit status waiting for a block's output before it goes to the assistant.
    block_ask: Option<(String, Option<i32>)>,
    /// Why `config.toml` could not be loaded; shown as a banner until it loads again or is dismissed.
    config_error: Option<String>,
    key_editor: keyeditor::KeyEditor,
    command: String,
    history: Option<String>,
    query: String,
    draft_open: bool,
    draft: String,
    review: bool,
    last_size: (u16, u16),
    font_size: f32,
    confirm_close: bool,
}
/// A drag selection on one pane, in screen cells (row, column).
#[derive(Clone, Copy, Debug)]
struct Selection {
    pane: u64,
    a: (u16, u16),
    b: (u16, u16),
    /// Rectangular (⌥-drag) rather than reading-order selection.
    block: bool,
}
impl Selection {
    fn contains(&self, row: u16, col: u16) -> bool {
        if self.block {
            return row >= self.a.0.min(self.b.0)
                && row <= self.a.0.max(self.b.0)
                && col >= self.a.1.min(self.b.1)
                && col <= self.a.1.max(self.b.1);
        }
        let (from, to) = if self.a <= self.b {
            (self.a, self.b)
        } else {
            (self.b, self.a)
        };
        (row, col) >= from && (row, col) <= to
    }
}

/// What a pane's title bar asked for; applied after drawing.
enum PaneAction {
    Rename(u64),
    Zoom(u64),
    Close(u64),
}
struct TermOut {
    size: (u16, u16),
    clicked: bool,
    action: Option<PaneAction>,
}

/// The id (as accepted by `--theme`) of the built-in or user theme with this display name.
fn find_theme_id(display_name: &str) -> Option<String> {
    crate::theme::all_names()
        .into_iter()
        .find(|id| Theme::load(id).is_ok_and(|t| t.name == display_name))
}

impl Desktop {
    fn new(
        host: String,
        workspace: String,
        readonly: bool,
        theme: Theme,
        tx: SyncSender<Command>,
        rx: Receiver<Update>,
    ) -> Self {
        let (upload_tx, upload_rx) = mpsc::channel();
        Self {
            tabs: vec![Tab {
                host: host.clone(),
                name: workspace.clone(),
                activity: Default::default(),
                state: None,
            }],
            host,
            active: 0,
            target: None,
            weights: Weights::default(),
            pane_rects: BTreeMap::new(),
            unfocused_dim: 0.18,
            wake_set: false,
            focus_at_start: None,
            restore_focus: false,
            tab_fade_at: None,
            chip_toasts: HashMap::new(),
            noise_tex: None,
            scroll_visual: HashMap::new(),
            theme_fade: None,
            theme_shown: theme.clone(),
            pending_fullscreen: false,
            find_query: None,
            find_regex: false,
            find_case: false,
            find_error: None,
            hints: None,
            snippet_prompt: None,
            broadcast: false,
            scrollback: BTreeMap::new(),
            view_rows: BTreeMap::new(),
            mouse_down: None,
            last_cell: None,
            wheel_accum: 0.,
            window_focused: true,
            quit_dialog: false,
            quit_ok: false,
            cfg: DesktopConfig::default(),
            applied_font: None,
            keymap: Keymap::build(&Default::default()).0,
            config_mtime: None,
            config_theme: None,
            notify_cfg: Default::default(),
            forwards: HashMap::new(),
            config_checked: Instant::now(),
            pending: Vec::new(),
            fullscreen: false,
            on_top: false,
            theme_names: BUILTINS.iter().map(|s| (*s).to_string()).collect(),
            open_entries: Vec::new(),
            palette_was_open: false,
            cheatsheet: false,
            tip_index: 0,
            offline_since: None,
            link: relink::Link::default(),
            theme_id_cache: None,
            last_system_dark: None,
            extra: SessionExtra::default(),
            pending_float: false,
            pending_cells: None,
            discard_workspace: false,
            triggers: Vec::new(),
            trigger_seen: HashMap::new(),
            upload_tx,
            upload_rx,
            toast: None,
            toasted: String::new(),
            window_title: String::new(),
            latency: None,
            copy_text: None,
            ai: ai::Panel::new(),
            files: files::Browser::new(),
            agents_open: false,
            aicmd: aicmd::Bar::default(),
            cmdhist: cmdhist::Bar::default(),
            explain_req: None,
            menu_pick: None,
            renaming: None,
            zoom_prev: None,
            selection: None,
            find_open: false,
            find_focus: false,
            find_text: String::new(),
            find_cur: 0,
            find_count: 0,
            palette_sel: 0,
            persist: false,
            saved: DesktopSettings::default(),
            last_save: Instant::now(),
            win_size: Vec2::ZERO,
            workspace,
            readonly,
            online: false,
            notice: "Connecting…".into(),
            committed_theme: theme.clone(),
            theme,
            panes: BTreeMap::new(),
            focus: 1,
            layout: LayoutMode::Single,
            tx,
            rx,
            palette: false,
            theme_picker: Default::default(),
            font_picker: Default::default(),
            settings_win: Default::default(),
            appearance_open: false,
            block_menu: None,
            block_ask: None,
            config_error: None,
            key_editor: Default::default(),
            command: String::new(),
            history: None,
            query: String::new(),
            draft_open: false,
            draft: String::new(),
            review: false,
            last_size: (0, 0),
            font_size: 13.,
            confirm_close: false,
        }
    }
    fn swap_session(&mut self, other: &mut Session) {
        std::mem::swap(&mut self.extra, &mut other.extra);
        std::mem::swap(&mut self.host, &mut other.host);
        std::mem::swap(&mut self.target, &mut other.target);
        std::mem::swap(&mut self.workspace, &mut other.workspace);
        std::mem::swap(&mut self.online, &mut other.online);
        std::mem::swap(&mut self.notice, &mut other.notice);
        std::mem::swap(&mut self.panes, &mut other.panes);
        std::mem::swap(&mut self.focus, &mut other.focus);
        std::mem::swap(&mut self.layout, &mut other.layout);
        std::mem::swap(&mut self.tx, &mut other.tx);
        std::mem::swap(&mut self.rx, &mut other.rx);
        std::mem::swap(&mut self.history, &mut other.history);
        std::mem::swap(&mut self.query, &mut other.query);
        std::mem::swap(&mut self.last_size, &mut other.last_size);
    }
    /// Make tab `i` the live one, parking the current state in its own slot.
    fn activate(&mut self, i: usize) {
        if i == self.active || i >= self.tabs.len() {
            return;
        }
        if let Some(mut incoming) = self.tabs[i].state.take() {
            self.swap_session(&mut incoming);
            self.tabs[self.active].state = Some(incoming);
            self.active = i;
            self.tab_fade_at = Some(Instant::now());
            self.last_size = (0, 0);
            self.toasted = self.notice.clone();
            self.broadcast = false;
            self.hints = None;
        }
    }
    /// Let background tabs drain their updates so their workers never block.
    fn pump_background(&mut self) {
        let home = self.active;
        let notice = self.notice.clone();
        for i in (0..self.tabs.len()).filter(|i| *i != home) {
            self.activate(i);
            self.updates();
            self.agents_update();
            let revisions: u64 = self.panes.values().map(|p| p.info.revision).sum();
            let waiting = self
                .panes
                .values()
                .any(|p| p.track.status == Some(crate::agents::Status::NeedsYou));
            self.activate(home);
            self.tabs[i].activity.observe(revisions, waiting);
        }
        // The tab on screen is being looked at.
        if let Some(t) = self.tabs.get_mut(home) {
            t.activity.seen();
        }
        self.notice = notice;
    }
    /// Switch to the tab for `host`/`name`, opening it (on that host) if it does not exist yet.
    fn open_tab(&mut self, host: &str, name: &str) {
        let active = self.active;
        let same = |tab: &Tab| tab.host == host && tab.name == name;
        if let Some(i) = self.tabs.iter().position(same) {
            self.activate(i);
            return;
        }
        let target = if host == self.host {
            self.target.clone()
        } else {
            config::target(host, None).ok()
        };
        let Some(target) = target else {
            self.toast(format!(
                "Cannot open {host}: check the host name or your config"
            ));
            return;
        };
        if !valid_name(name) {
            self.notice = "Workspace names use letters, digits, - and _".into();
            return;
        }
        let _ = active;
        let (tx, rx) = mpsc::sync_channel(64);
        let (utx, urx) = mpsc::sync_channel(32);
        let (ws, readonly, worker_target) = (name.to_string(), self.readonly, target.clone());
        thread::spawn(move || worker(worker_target, ws, readonly, None, rx, utx, (30, 100)));
        if self.persist {
            crate::recent::record(host, name);
        }
        self.tabs.push(Tab {
            host: host.into(),
            name: name.into(),
            activity: Default::default(),
            state: Some(Session {
                extra: SessionExtra::default(),
                host: host.into(),
                target: Some(target),
                workspace: name.into(),
                online: false,
                notice: "Connecting…".into(),
                panes: BTreeMap::new(),
                focus: 1,
                layout: LayoutMode::Single,
                tx,
                rx: urx,
                history: None,
                query: String::new(),
                last_size: (0, 0),
            }),
        });
        self.activate(self.tabs.len() - 1);
    }
    /// Detach the current tab (its shells keep running on the host, unless it is a fresh-window tab).
    fn close_tab(&mut self) {
        if self.tabs.len() < 2 {
            self.notice = "This is the only tab".into();
            return;
        }
        let closing = self.active;
        // A tab made by ⌘T in a fresh-window setup owns its shell: closing it ends the shell.
        let owned = self.cfg.fresh_window && self.tabs[closing].name.starts_with("win-");
        let _ = self.tx.try_send(if owned {
            Command::Discard
        } else {
            Command::Stop
        });
        let other = if closing > 0 { closing - 1 } else { 1 };
        self.activate(other);
        self.tabs.remove(closing);
        if other > closing {
            self.active = other - 1;
        }
    }
    fn send(&mut self, c: Command) {
        if !self.online {
            self.notice = "Offline: no input sent. Drafts remain local.".into();
            return;
        }
        if self.readonly && !matches!(c, Command::Capture(..) | Command::Image(..)) {
            self.notice = "Read-only attachment".into();
            return;
        }
        if self.tx.try_send(c).is_err() {
            self.notice = "Action queue full; action not queued".into();
        }
    }
    /// Panes that receive typed input: the focused one, or every visible live pane while broadcasting.
    fn input_targets(&self) -> Vec<u64> {
        if !self.broadcast {
            return vec![self.focus];
        }
        let mut t: Vec<u64> = self
            .pane_rects
            .keys()
            .copied()
            .filter(|id| self.panes.get(id).is_some_and(|p| p.info.exited.is_none()))
            .collect();
        if !t.contains(&self.focus) {
            t.push(self.focus);
        }
        t
    }
    fn input(&mut self, data: Vec<u8>) {
        self.scrollback.remove(&self.focus);
        if data.len() > MAX_INPUT {
            self.notice = "Paste too large; transfer a file instead".into();
            return;
        }
        for pane in self.input_targets() {
            self.send(Command::Input {
                pane,
                data: data.clone(),
            });
        }
    }
    fn updates(&mut self) {
        while let Ok(update) = self.rx.try_recv() {
            match update {
                Update::Online => {
                    self.online = true;
                    self.link.online();
                    self.last_size = (0, 0);
                    self.notice = "Attached to persistent remote shells".into();
                }
                Update::Offline(s) => {
                    self.online = false;
                    self.notice = s;
                }
                Update::Message(s) => self.notice = s,
                Update::Image(pane, id, bytes) => {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.image_raw.insert(id, bytes);
                    }
                }
                Update::Latency(ms) => self.latency = Some(ms),
                Update::Daemon(platform, uptime, stats) => self.link.daemon(platform, uptime, stats),
                Update::Retry(attempt, wait) => self.link.retry(attempt, wait),
                Update::Discarded => {}
                Update::Created(id) => {
                    self.focus = id;
                    if self.pending_float {
                        self.pending_float = false;
                        self.extra.floating = Some(id);
                    }
                    self.title_new_pane(id);
                }
                Update::Captured(text, first_line, purpose) => match purpose {
                    CapturePurpose::View(query) => {
                        self.history = Some(text);
                        self.query = query;
                    }
                    CapturePurpose::Scrollback(id) => self.finish_scrollback(id, &text, first_line),
                    CapturePurpose::Export(path) => {
                        self.notice = match crate::util::private_write(
                            std::path::Path::new(&path),
                            text.as_bytes(),
                        ) {
                            Ok(()) => format!("Exported {path}"),
                            Err(e) => format!("Export failed: {e}"),
                        }
                    }
                },
                Update::State(views) => {
                    self.panes.retain(|id, _| views.iter().any(|p| p.id == *id));
                    let changed: Vec<u64> = views
                        .iter()
                        .filter(|v| v.screen.is_some())
                        .map(|v| v.id)
                        .collect();
                    for view in views {
                        if let Some(bytes) = &view.screen {
                            self.link.received(bytes.len() as u64);
                        }
                        let pane = self.panes.entry(view.id).or_insert_with(|| {
                            Pane::new(view.clone(), vt100::Parser::new(view.rows, view.cols, 0))
                        });
                        if let Some(bytes) = &view.screen {
                            pane.parser = vt100::Parser::new(view.rows, view.cols, 0);
                            pane.parser.process(bytes);
                        }
                        if let Some(m) = &view.marks {
                            // A long command finished while the window is in the background.
                            if let Some(d) = m.iter().rev().find(|mk| mk.kind == 'D') {
                                if d.at > pane.announced {
                                    let limit = u64::from(self.cfg.notify_after_seconds) * 1000;
                                    let long = d.ms.is_some_and(|ms| limit > 0 && ms >= limit);
                                    if pane.announced != 0 && long && !self.window_focused {
                                        let message = format!(
                                            "Pane {} finished a command in {} s (exit {})",
                                            view.id,
                                            d.ms.unwrap_or(0) / 1000,
                                            d.exit.map_or("?".to_string(), |c| c.to_string())
                                        );
                                        crate::util::notify(&message);
                                        crate::notify::send_remote(
                                            &self.notify_cfg,
                                            crate::notify::Note {
                                                event: "command_done",
                                                title: "TERYON".into(),
                                                message,
                                                workspace: self.workspace.clone(),
                                                pane: view.id,
                                            },
                                        );
                                    }
                                    pane.announced = d.at;
                                }
                            }
                            pane.marks = m.clone();
                        }
                        if let Some(list) = &view.images {
                            let live: std::collections::HashSet<u64> =
                                list.iter().map(|p| p.id).collect();
                            pane.image_asked.retain(|id| live.contains(id));
                            pane.image_raw.retain(|id, _| live.contains(id));
                            pane.image_tex.retain(|id, _| live.contains(id));
                            pane.placements = list.clone();
                            for p in list {
                                if pane.image_asked.insert(p.id) {
                                    let _ = self.tx.try_send(Command::Image(view.id, p.id));
                                }
                            }
                        }
                        if let (None, Some(code)) = (pane.info.exited, view.exited) {
                            self.notice = format!("Pane {} finished with exit {}", view.id, code);
                            if !self.window_focused {
                                crate::util::notify(&self.notice);
                            }
                        }
                        pane.info = view;
                    }
                    if !self.panes.contains_key(&self.focus) {
                        self.focus = self.panes.keys().next().copied().unwrap_or(1);
                    }
                    for id in changed {
                        self.scan_triggers(id);
                    }
                    self.after_state();
                }
            }
        }
        self.agents_update();
        self.away_check();
        self.away_touch();
    }
    fn execute(&mut self) {
        let command = std::mem::take(&mut self.command);
        let (name, arg) = command
            .trim()
            .split_once(' ')
            .unwrap_or((command.trim(), ""));
        match name{
        "new"=>self.send(Command::New(if arg.is_empty(){None}else{Some(arg.into())})),
        "rename"=>self.send(Command::Rename(self.focus,arg.into())),"close"=>self.close_focused_pane(),
        "split"=>self.layout = if self.layout == LayoutMode::Single { LayoutMode::Vertical } else { LayoutMode::Single },
        "layout"=>match LayoutMode::parse(arg){Some(m)=>self.layout=m,None=>self.notice="Usage: layout single|vertical|horizontal|grid".into()},
        "theme"=>match Theme::load_native(arg){Ok(t)=>{self.committed_theme=t.clone();self.theme=t}Err(e)=>self.notice=e.to_string()},
        "search"=>{self.find_text=arg.to_string();self.rebuild_find();self.send(Command::Capture(self.focus,CapturePurpose::View(arg.into())))},
        "capture"=>self.send(Command::Capture(self.focus,CapturePurpose::View(String::new()))),
        "export"=>self.send(Command::Capture(self.focus,CapturePurpose::Export(arg.into()))),
        "draft"=>self.draft_open=true,
        "tab"=>{let host=self.host.clone();self.open_tab(&host,arg)},"open"=>{let mut p=arg.split_whitespace();match p.next(){Some(h)=>self.open_tab(h,p.next().unwrap_or("default")),None=>self.notice="Usage: open HOST [WORKSPACE]".into()}},"closetab"=>self.close_tab(),
        _=>self.notice="Commands: new [cwd], rename TITLE, close, split, layout NAME, theme NAME, search TEXT, capture, export /file, draft, tab NAME, closetab".into(),
    }
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        let (terminal_focus, wants_focus) = self.focus_at_start.take().unwrap_or_else(|| {
            let focused = ctx.memory(|m| m.focused());
            (
                focused == Some(egui::Id::new(("teryon-terminal", self.focus))),
                focused.is_some(),
            )
        });
        if terminal_focus {
            // Tab, Esc and the arrows belong to the shell, not to egui's focus walk.
            let term_id = egui::Id::new(("teryon-terminal", self.focus));
            ctx.memory_mut(|m| {
                m.set_focus_lock_filter(
                    term_id,
                    egui::EventFilter {
                        tab: true,
                        escape: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                    },
                );
            });
        }
        if (wants_focus && !terminal_focus)
            || self.palette
            || self.theme_picker.open
            || self.font_picker.open
            || self.settings_win.open
            || self.key_editor.open
            || self.draft_open
            || self.history.is_some()
            || self.confirm_close
            || self.files_capture_keys()
        {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        self.restore_focus |= terminal_focus
            && events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: Key::Tab
                            | Key::ArrowUp
                            | Key::ArrowDown
                            | Key::ArrowLeft
                            | Key::ArrowRight,
                        pressed: true,
                        ..
                    }
                )
            });
        // With ⌥ held the Key event already sent ESC + key; drop the composed text that follows.
        let alt_meta = ctx.input(|i| i.modifiers.alt && !i.modifiers.command);
        for event in events {
            let consumed = if self.hints.is_some() {
                self.hints_event(&event)
            } else if self.copy_active() {
                self.copy_event(&event)
            } else {
                false
            };
            if consumed {
                continue;
            }
            match event {
                egui::Event::Text(text) => {
                    if !alt_meta {
                        self.input(text.into_bytes());
                    }
                }
                egui::Event::Copy => {
                    if let Some(text) = self.selection.and_then(|s| self.selection_text(&s)) {
                        self.copy_text = Some(text);
                    }
                }
                egui::Event::Paste(text) => {
                    let bracketed = self
                        .panes
                        .get(&self.focus)
                        .is_some_and(|p| p.parser.screen().bracketed_paste());
                    self.input(keys::paste(&text, bracketed));
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let bound = keymap::canon_key(key.name())
                        .and_then(|k| {
                            self.keymap
                                .lookup(chord_mods(&modifiers), &k)
                                .map(str::to_string)
                        })
                        .and_then(|name| ActionKind::from_name(&name));
                    if let Some(kind) = bound {
                        self.run_action(kind);
                        continue;
                    }
                    if modifiers.mac_cmd {
                        continue;
                    }
                    let code = match key {
                        Key::ArrowUp => crossterm::event::KeyCode::Up,
                        Key::ArrowDown => crossterm::event::KeyCode::Down,
                        Key::ArrowLeft => crossterm::event::KeyCode::Left,
                        Key::ArrowRight => crossterm::event::KeyCode::Right,
                        Key::Enter => crossterm::event::KeyCode::Enter,
                        Key::Tab => {
                            if modifiers.shift {
                                crossterm::event::KeyCode::BackTab
                            } else {
                                crossterm::event::KeyCode::Tab
                            }
                        }
                        Key::Backspace => crossterm::event::KeyCode::Backspace,
                        Key::Escape => crossterm::event::KeyCode::Esc,
                        Key::Home => crossterm::event::KeyCode::Home,
                        Key::End => crossterm::event::KeyCode::End,
                        Key::PageUp => crossterm::event::KeyCode::PageUp,
                        Key::PageDown => crossterm::event::KeyCode::PageDown,
                        Key::Insert => crossterm::event::KeyCode::Insert,
                        Key::Delete => crossterm::event::KeyCode::Delete,
                        Key::F1 => crossterm::event::KeyCode::F(1),
                        Key::F2 => crossterm::event::KeyCode::F(2),
                        Key::F3 => crossterm::event::KeyCode::F(3),
                        Key::F4 => crossterm::event::KeyCode::F(4),
                        Key::F5 => crossterm::event::KeyCode::F(5),
                        Key::F6 => crossterm::event::KeyCode::F(6),
                        Key::F7 => crossterm::event::KeyCode::F(7),
                        Key::F8 => crossterm::event::KeyCode::F(8),
                        Key::F9 => crossterm::event::KeyCode::F(9),
                        Key::F10 => crossterm::event::KeyCode::F(10),
                        Key::F11 => crossterm::event::KeyCode::F(11),
                        Key::F12 => crossterm::event::KeyCode::F(12),
                        Key::Space if modifiers.ctrl => crossterm::event::KeyCode::Char(' '),
                        Key::Slash if modifiers.ctrl => crossterm::event::KeyCode::Char('_'),
                        // ⌥ is Meta like WezTerm's default: ⌥B / ⌥F / ⌥D / ⌥. send ESC + the key
                        // instead of the composed character the Mac layout would type.
                        k if modifiers.ctrl || modifiers.alt => {
                            let name = k.name();
                            let mut chars = name.chars();
                            let Some(c) = chars.next() else {
                                continue;
                            };
                            if chars.next().is_some() {
                                continue;
                            }
                            if modifiers.shift && !modifiers.ctrl && !c.is_ascii_alphabetic() {
                                continue;
                            }
                            crossterm::event::KeyCode::Char(if modifiers.shift && !modifiers.ctrl {
                                c.to_ascii_uppercase()
                            } else {
                                c.to_ascii_lowercase()
                            })
                        }
                        _ => continue,
                    };
                    let mut mods = crossterm::event::KeyModifiers::NONE;
                    if modifiers.ctrl {
                        mods |= crossterm::event::KeyModifiers::CONTROL;
                    }
                    if modifiers.alt {
                        mods |= crossterm::event::KeyModifiers::ALT;
                    }
                    if modifiers.shift {
                        mods |= crossterm::event::KeyModifiers::SHIFT;
                    }
                    let app_cursor = self
                        .panes
                        .get(&self.focus)
                        .is_some_and(|p| p.parser.screen().application_cursor());
                    if let Some(data) =
                        keys::encode(crossterm::event::KeyEvent::new(code, mods), app_cursor)
                    {
                        self.input(data);
                    }
                }
                _ => {}
            }
        }
    }
    fn frame(&mut self, ctx: &egui::Context) {
        if !self.wake_set {
            // The worker draws a frame the moment new pane state arrives.
            let repaint = ctx.clone();
            crate::ui::set_wake(move || repaint.request_repaint());
            self.wake_set = true;
        }
        let previous_focus = self.focus;
        {
            // Terminal first: after Tab, an arrow or Esc, or when nothing has focus (unless an
            // overlay owns the keyboard), so the next key reaches the shell.
            let term = egui::Id::new(("teryon-terminal", self.focus));
            let owned = self.palette
                || self.theme_picker.open
                || self.font_picker.open
                || self.settings_win.open
                || self.key_editor.open
                || self.draft_open
                || self.history.is_some()
                || self.confirm_close
                || self.files_capture_keys();
            if !owned
                && (std::mem::take(&mut self.restore_focus)
                    || ctx.memory(|m| m.focused()).is_none())
            {
                ctx.memory_mut(|m| m.request_focus(term));
            }
            let focused = ctx.memory(|m| m.focused());
            self.focus_at_start = Some((focused == Some(term), focused.is_some()));
        }
        self.pump_background();
        self.updates();
        if self.focus != previous_focus {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new(("teryon-terminal", self.focus))));
        }
        if self.theme.name != self.committed_theme.name
            && !self.palette
            && !self.theme_picker.open
            && !self.font_picker.open
            && !ctx.memory(|m| m.any_popup_open())
        {
            self.theme = self.committed_theme.clone();
        }
        let t = self.theme.clone();
        apply_style(ctx, &t);
        if let Some((pane, pick)) = self.menu_pick.take() {
            self.run_menu(pane, pick);
        }
        if let Some(text) = self.copy_text.take() {
            ctx.copy_text(text);
        }
        for cmd in self.pending.drain(..) {
            ctx.send_viewport_cmd(cmd);
        }
        if self.persist
            && !self.font_picker.open
            && self.applied_font.as_ref() != Some(&self.cfg.font_family)
        {
            install_fonts(ctx, self.cfg.font_family.as_deref());
            self.applied_font = Some(self.cfg.font_family.clone());
        }
        self.follow_system_theme(ctx);
        self.fade_theme(ctx);
        if std::mem::take(&mut self.pending_fullscreen) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        self.window_focused = ctx.input(|i| i.focused);
        if self.online {
            self.offline_since = None;
        } else if self.offline_since.is_none() {
            self.offline_since = Some(Instant::now());
        }
        self.fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        self.poll_config();
        self.sync_toast();
        let title = format!(
            "{} — {}",
            self.panes
                .get(&self.focus)
                .map_or("TERYON", |p| p.info.title.as_str()),
            self.workspace
        );
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
        if let Some((cols, rows)) = self.pending_cells.take() {
            self.size_window_to_cells(ctx, cols, rows);
        }
        if self.cfg.chrome == crate::config::Chrome::Full {
            self.top_bar(ctx);
        } else {
            self.chrome_drag(ctx);
            self.tab_strip(ctx);
        }
        self.status_line(ctx);
        self.files_ui(ctx);
        self.ai_panel(ctx);
        self.agents_overview(ctx);
        self.away_ui(ctx);
        if let Some(id) = self.explain_req.take() {
            self.ai_explain(id);
        }
        self.aicmd_ui(ctx);
        self.cmdhist_ui(ctx);
        self.relink_ui(ctx);
        let content_rect = ctx.available_rect();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(
                        color(if self.cfg.pane_inset > 0. {
                            mix(t.background, t.foreground, 0.04)
                        } else {
                            t.background
                        })
                        .gamma_multiply(self.cfg.window_opacity),
                    )
                    .inner_margin(egui::Margin::same(self.cfg.pane_inset.round() as i8)),
            )
            .show(ctx, |ui| {
                if self.find_open {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Find").size(12.).color(color(t.muted)));
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.find_text)
                                .id(egui::Id::new("teryon-find"))
                                .desired_width(220.),
                        );
                        if self.find_focus {
                            edit.request_focus();
                            self.find_focus = false;
                        }
                        if edit.changed() {
                            self.rebuild_find();
                        }
                        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            let back = ui.input(|i| i.modifiers.shift);
                            self.find_step(back);
                            edit.request_focus();
                        }
                        let mut toggled = false;
                        for (label, tip, on) in [
                            ("Aa", "Match case  ⌥⌘C", &mut self.find_case),
                            (".*", "Regular expression  ⌥⌘R", &mut self.find_regex),
                        ] {
                            let button = egui::Button::new(
                                RichText::new(label)
                                    .monospace()
                                    .size(11.)
                                    .color(color(if *on { t.foreground } else { t.muted })),
                            )
                            .fill(if *on {
                                color(t.accent).gamma_multiply(0.35)
                            } else {
                                Color32::TRANSPARENT
                            });
                            if ui.add(button).on_hover_text(tip).clicked() {
                                *on = !*on;
                                toggled = true;
                            }
                        }
                        if toggled {
                            self.rebuild_find();
                        }
                        let n = self.find_count;
                        if let Some(err) = &self.find_error {
                            ui.label(RichText::new(err).size(11.).color(color(t.warning)));
                        }
                        ui.label(
                            RichText::new(if n == 0 {
                                "no matches".to_string()
                            } else {
                                format!("{} of {n}", self.find_cur % n + 1)
                            })
                            .size(12.)
                            .color(color(t.muted)),
                        );
                        if ui
                            .small_button("‹")
                            .on_hover_text("Previous  ⇧⌘G")
                            .clicked()
                        {
                            self.find_step(true);
                        }
                        if ui.small_button("›").on_hover_text("Next  ⌘G").clicked() {
                            self.find_step(false);
                        }
                        if icon_button(ui, &t, Icon::Close, "Close  Esc").clicked()
                            || ui.input(|i| i.key_pressed(Key::Escape))
                        {
                            self.close_find();
                        }
                    });
                }
                if self.panes.is_empty() {
                    self.welcome_ui(ui);
                    return;
                }
                let ids: Vec<u64> = self.tiled_ids();
                let shown = visible_ids(&ids, self.tiled_focus(&ids), self.layout);
                let (grid_rows, grid_cols) = self.layout.dims(shown.len());
                let dims = (usize::from(grid_rows), usize::from(grid_cols));
                self.weights.fit(dims.0, dims.1);
                let area = ui.available_rect_before_wrap();
                let mut layout = arrange(area, dims, shown.len(), &self.weights);
                let mut moved = false;
                for d in &layout.dividers {
                    let handle = ui
                        .interact(
                            d.rect,
                            egui::Id::new(("teryon-divider", d.between_columns, d.index, d.row)),
                            Sense::drag(),
                        )
                        .on_hover_cursor(if d.between_columns {
                            egui::CursorIcon::ResizeHorizontal
                        } else {
                            egui::CursorIcon::ResizeVertical
                        });
                    if handle.dragged() {
                        let delta = if d.between_columns {
                            handle.drag_delta().x
                        } else {
                            handle.drag_delta().y
                        } / d.axis_len.max(1.);
                        let w = if d.between_columns {
                            &mut self.weights.cols
                        } else {
                            &mut self.weights.rows
                        };
                        panes::drag(w, d.index, delta);
                        moved = true;
                    }
                    let active = handle.hovered() || handle.dragged();
                    if !active && self.cfg.pane_inset > 0. {
                        continue;
                    }
                    let line = if active { t.accent } else { t.border };
                    let c = d.rect.center();
                    let seg = if d.between_columns {
                        [
                            Pos2::new(c.x, d.rect.top()),
                            Pos2::new(c.x, d.rect.bottom()),
                        ]
                    } else {
                        [
                            Pos2::new(d.rect.left(), c.y),
                            Pos2::new(d.rect.right(), c.y),
                        ]
                    };
                    ui.painter()
                        .line_segment(seg, Stroke::new(1_f32, color(line)));
                }
                if moved {
                    layout = arrange(area, dims, shown.len(), &self.weights);
                }
                self.pane_rects.clear();
                let mut drawn = Vec::new();
                for (id, r) in shown.iter().zip(&layout.rects) {
                    self.pane_rects.insert(*id, *r);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*r));
                    drawn.push((*id, self.draw_terminal(&mut child, *id, r.height())));
                }
                ui.advance_cursor_after_rect(area);
                for (id, result) in drawn {
                    if let Some(out) = result {
                        self.handle_term_out(id, out);
                    }
                }
            });
        self.floating_ui(ctx);
        self.handle_drops(ctx);
        self.restore_ui(ctx);
        if self.palette && !self.palette_was_open {
            self.refresh_catalog();
        }
        self.palette_was_open = self.palette;
        if self.palette {
            self.palette_ui(ctx);
        }
        if self.theme_picker.open {
            self.theme_picker_ui(ctx);
        }
        if self.font_picker.open {
            self.font_picker_ui(ctx);
        }
        if self.settings_win.open {
            self.settings_ui(ctx);
        }
        if self.key_editor.open {
            self.key_editor_ui(ctx);
        }
        if self.appearance_open {
            self.appearance_ui(ctx);
        }
        if self.config_error.is_some() {
            self.config_error_ui(ctx);
        }
        if self.cheatsheet {
            self.cheatsheet_ui(ctx);
        }
        self.snippet_ui(ctx);
        self.toast_ui(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && self.cfg.confirm_quit && !self.quit_ok {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_dialog = true;
        }
        if self.quit_dialog {
            egui::Window::new("Detach from this workspace?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                .show(ctx, |ui| {
                    ui.label("Closing the window detaches. Your shells keep running on the host.");
                    ui.horizontal(|ui| {
                        if primary_button(ui, &t, "Detach").clicked() {
                            self.quit_ok = true;
                            self.quit_dialog = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Cancel").clicked() {
                            self.quit_dialog = false;
                        }
                    });
                });
        }
        self.win_size = ctx.screen_rect().size();
        self.save_settings(false);
        if let Some(text) = self.history.clone() {
            let mut open = true;
            egui::Window::new("Retained output")
                .open(&mut open)
                .default_size(Vec2::new(780., 500.))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Search");
                        ui.text_edit_singleline(&mut self.query);
                        if ui.button("Copy all").clicked() {
                            ctx.copy_text(text.clone());
                        }
                    });
                    egui::ScrollArea::both().show(ui, |ui| {
                        for line in text.lines().filter(|line| {
                            self.query.is_empty()
                                || line.to_lowercase().contains(&self.query.to_lowercase())
                        }) {
                            ui.label(RichText::new(line).monospace());
                        }
                    });
                });
            if !open {
                self.history = None;
            }
        }
        if self.draft_open {
            let mut open = true;
            egui::Window::new("Local draft")
                .open(&mut open)
                .default_width(580.)
                .show(ctx, |ui| {
                    ui.label("Draft stays in local memory. It is never sent on reconnect.");
                    ui.add(egui::TextEdit::multiline(&mut self.draft).char_limit(32768));
                    if !self.review {
                        if ui.button("Review for sending").clicked() {
                            self.review = true;
                        }
                    } else {
                        ui.label(format!(
                            "Send to {} / {} / pane {} followed by Enter?",
                            self.host, self.workspace, self.focus
                        ));
                        if ui
                            .add_enabled(
                                self.online && !self.readonly && !self.draft.is_empty(),
                                egui::Button::new("Send reviewed draft"),
                            )
                            .clicked()
                        {
                            let data = format!("{}\r", self.draft).into_bytes();
                            if data.len() <= MAX_INPUT
                                && self
                                    .tx
                                    .try_send(Command::Input {
                                        pane: self.focus,
                                        data,
                                    })
                                    .is_ok()
                            {
                                self.draft.clear();
                                self.review = false;
                            } else {
                                self.notice =
                                    "Draft retained: input queue full or text too large".into();
                            }
                        }
                        if ui.button("Keep editing").clicked() {
                            self.review = false;
                        }
                    }
                });
            self.draft_open = open;
        }
        // A new tab's panes fade in over the old picture instead of cutting.
        if let Some(at) = self.tab_fade_at {
            let k = at.elapsed().as_secs_f32() / 0.14;
            if k >= 1. {
                self.tab_fade_at = None;
            } else {
                let veil = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("teryon-tab-fade"),
                ));
                let alpha = (1. - k) * (1. - k) * 0.55;
                veil.rect_filled(content_rect, 0., color(t.background).gamma_multiply(alpha));
                ctx.request_repaint();
            }
        }
        self.keyboard(ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
impl Desktop {
    /// Close the focused pane. Its shell keeps running for a moment, so ⇧⌘Z (or the palette) brings it back.
    fn close_focused_pane(&mut self) {
        if self.readonly {
            self.toast("Read only");
            return;
        }
        if self.panes.len() < 2 {
            self.toast("This is the last pane; exit its shell to end the workspace");
            return;
        }
        let id = self.focus;
        self.send(Command::SoftClose(id));
        let hint = self.keymap.hint("reopen_pane");
        self.toast(if hint.is_empty() {
            "Pane closed · \"Reopen closed pane\" in the palette brings it back".to_string()
        } else {
            format!("Pane closed · {hint} brings it back")
        });
    }
    fn run_action(&mut self, kind: ActionKind) {
        self.palette = false;
        match kind {
            ActionKind::Palette => self.palette = true,
            ActionKind::ThemePicker => self.open_theme_picker(),
            ActionKind::FontPicker => self.open_font_picker(),
            ActionKind::Settings => self.open_settings(),
            ActionKind::Appearance => {
                self.appearance_open = !self.appearance_open;
                self.palette = false;
            }
            ActionKind::KeyEditor => self.open_key_editor(),
            ActionKind::NewPane => {
                let cwd = self.focus_cwd();
                self.send(Command::New(cwd));
            }
            ActionKind::NewPaneRight => {
                if matches!(self.layout, LayoutMode::Single | LayoutMode::Vertical) {
                    self.layout = LayoutMode::Horizontal;
                }
                let cwd = self.focus_cwd();
                self.send(Command::New(cwd));
            }
            ActionKind::NewPaneBelow => {
                if matches!(self.layout, LayoutMode::Single | LayoutMode::Horizontal) {
                    self.layout = LayoutMode::Vertical;
                }
                let cwd = self.focus_cwd();
                self.send(Command::New(cwd));
            }
            ActionKind::Focus(dir) => self.focus_dir(dir),
            ActionKind::Resize(dir) => self.resize_focused(dir, 0.02),
            ActionKind::Equalize => self.weights.equalize(),
            ActionKind::NextPane => self.cycle_pane(false),
            ActionKind::PrevPane => self.cycle_pane(true),
            ActionKind::ClosePane => self.close_focused_pane(),
            ActionKind::ReopenPane => {
                self.send(Command::Reopen);
                self.toast("Reopening the pane you closed…");
            }
            ActionKind::RenamePane => {
                self.renaming = self
                    .panes
                    .get(&self.focus)
                    .map(|p| (self.focus, p.info.title.clone()));
            }
            ActionKind::Layout(mode) => self.layout = mode,
            ActionKind::Zoom => {
                if self.layout == LayoutMode::Single {
                    self.layout = self.zoom_prev.take().unwrap_or(LayoutMode::Vertical);
                } else {
                    self.zoom_prev = Some(self.layout);
                    self.layout = LayoutMode::Single;
                }
            }
            ActionKind::Find => {
                self.find_open = true;
                self.find_focus = true;
            }
            ActionKind::NewTab if self.cfg.fresh_window => {
                // Like a terminal's ⌘T: a new shell right away, named for you.
                let millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis());
                let host = self.host.clone();
                self.open_tab(&host, &format!("win-{millis}-{}", std::process::id()));
            }
            ActionKind::NewTab => {
                self.command = "tab ".into();
                self.palette = true;
            }
            ActionKind::CloseTab => self.close_tab(),
            ActionKind::Theme(name) => match Theme::load_native(&name) {
                Ok(t) => {
                    self.committed_theme = t.clone();
                    self.theme = t;
                }
                Err(e) => self.notice = e.to_string(),
            },
            ActionKind::OpenWorkspace => {
                self.command = "Open: ".into();
                self.palette = true;
            }
            ActionKind::Open(host, ws) => self.open_tab(&host, &ws),
            ActionKind::CheatSheet => self.cheatsheet = !self.cheatsheet,
            ActionKind::FindRegex => {
                self.find_regex = !self.find_regex;
                self.find_open = true;
                self.rebuild_find();
            }
            ActionKind::FindCase => {
                self.find_case = !self.find_case;
                self.find_open = true;
                self.rebuild_find();
            }
            ActionKind::FloatPane => self.toggle_float(),
            ActionKind::NewFloatingPane => self.new_floating(),
            ActionKind::Hints => self.start_hints(),
            ActionKind::CopyMode => self.start_copy_mode(),
            ActionKind::Broadcast => {
                self.broadcast = !self.broadcast;
                let n = self.input_targets().len();
                self.toast(if self.broadcast {
                    format!("Broadcast ON: typing goes to {n} pane(s)")
                } else {
                    "Broadcast off".to_string()
                });
            }
            ActionKind::Snippet(i) => self.run_snippet(i),
            ActionKind::FindNext => self.find_step(false),
            ActionKind::FindPrev => self.find_step(true),
            ActionKind::TabJump(n) => self.activate(n),
            ActionKind::TabMove(d) => self.move_tab(d),
            ActionKind::TabStep(d) => {
                let n = self.tabs.len() as isize;
                if n > 0 {
                    self.activate((self.active as isize + d).rem_euclid(n) as usize);
                }
            }
            ActionKind::NextTheme | ActionKind::PrevTheme | ActionKind::RandomTheme => {
                let now = self.theme_id().unwrap_or_else(|| "mac27-dark".into());
                let names = self.theme_names.clone();
                let next = match kind {
                    ActionKind::NextTheme => theme_step(&names, &now, 1),
                    ActionKind::PrevTheme => theme_step(&names, &now, -1),
                    _ => {
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.subsec_nanos() as usize);
                        names[nanos % names.len().max(1)].clone()
                    }
                };
                self.run_action(ActionKind::Theme(next));
            }
            ActionKind::ThemeVariant => {
                let now = self.theme_id().unwrap_or_else(|| "mac27-dark".into());
                match theme_counterpart(&now, &self.theme_names) {
                    Some(other) => self.run_action(ActionKind::Theme(other)),
                    None => self.toast("This theme has no light/dark counterpart"),
                }
            }
            ActionKind::FontUp => self.font_size = (self.font_size + 1.).min(36.),
            ActionKind::FontDown => self.font_size = (self.font_size - 1.).max(8.),
            ActionKind::FontReset => self.font_size = self.cfg.font_size,
            ActionKind::ReloadConfig => self.reload_config(),
            ActionKind::Fullscreen => self
                .pending
                .push(egui::ViewportCommand::Fullscreen(!self.fullscreen)),
            ActionKind::AlwaysOnTop => {
                self.on_top = !self.on_top;
                self.pending
                    .push(egui::ViewportCommand::WindowLevel(if self.on_top {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    }));
                self.toast(if self.on_top {
                    "Always on top"
                } else {
                    "Normal window level"
                });
            }
            ActionKind::Scroll(s) => self.scroll_action(s),
            ActionKind::PromptPrev => self.jump_prompt(true),
            ActionKind::PromptNext => self.jump_prompt(false),
            ActionKind::CopyLastOutput => self.copy_last_output(),
            ActionKind::CopyLastCommand => self.copy_last_command(),
            ActionKind::RerunLastCommand => self.rerun_last_command(),
            ActionKind::OpenPort => self.open_port(),
            ActionKind::AskAiLast => {
                let id = self.focus;
                self.ai_ask_last(id);
            }
            ActionKind::History => self.send(Command::Capture(
                self.focus,
                CapturePurpose::View(String::new()),
            )),
            ActionKind::CopyScreen => {
                if let Some(p) = self.panes.get(&self.focus) {
                    let text = p.parser.screen().contents();
                    self.copy_text = Some(text);
                }
            }
            ActionKind::Draft => self.draft_open = true,
            ActionKind::Ai => self.ai_toggle(),
            ActionKind::Files => self.files_toggle(),
            ActionKind::Agents => self.agents_open = !self.agents_open,
            ActionKind::AiCommand => self.aicmd_toggle(),
            ActionKind::CommandHistory => self.cmdhist_toggle(),
        }
    }
    fn palette_ui(&mut self, ctx: &egui::Context) {
        let list = actions(
            &self.keymap,
            &self.theme_names,
            &self.open_entries,
            &self.cfg.snippets,
        );
        let query = self.command.clone();
        let (word, arg) = query
            .trim_start()
            .split_once(' ')
            .unwrap_or((query.trim(), ""));
        let typed = ARG_COMMANDS.contains(&word) && !arg.trim().is_empty();
        let mut ranked: Vec<(i32, usize)> = list
            .iter()
            .enumerate()
            .filter_map(|(i, a)| uikit::fuzzy_score(&query, &a.title).map(|s| (s, i)))
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked.truncate(10);
        let mut chosen: Option<usize> = None;
        let mut run_typed = false;
        let mut close = false;
        egui::Window::new("palette")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0., 70.])
            .fixed_size([520., 0.])
            .show(ctx, |ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.command)
                        .hint_text("Type a command or search…")
                        .desired_width(f32::INFINITY),
                );
                edit.request_focus();
                if edit.changed() {
                    self.palette_sel = 0;
                }
                let (down, up, enter, esc) = ui.input(|i| {
                    (
                        i.key_pressed(Key::ArrowDown),
                        i.key_pressed(Key::ArrowUp),
                        i.key_pressed(Key::Enter),
                        i.key_pressed(Key::Escape),
                    )
                });
                if down && self.palette_sel + 1 < ranked.len() {
                    self.palette_sel += 1;
                }
                if up {
                    self.palette_sel = self.palette_sel.saturating_sub(1);
                }
                self.palette_sel = self.palette_sel.min(ranked.len().saturating_sub(1));
                // Arrowing over a "Theme: X" row previews it; anything else shows the real theme.
                let previewed = match ranked.get(self.palette_sel).map(|(_, i)| &list[*i].kind) {
                    Some(ActionKind::Theme(name)) if !typed => Theme::load_native(name).ok(),
                    _ => None,
                };
                let wanted = previewed.unwrap_or_else(|| self.committed_theme.clone());
                if wanted.name != self.theme.name {
                    self.theme = wanted;
                }
                ui.add_space(6.);
                if !ranked.is_empty() {
                    ui.label(
                        RichText::new(if typed { "RESULTS" } else { "SUGGESTED" })
                            .size(10.)
                            .strong()
                            .color(color(self.theme.muted)),
                    );
                }
                for (row, (_, i)) in ranked.iter().enumerate() {
                    let action = &list[*i];
                    let selected = row == self.palette_sel && !typed;
                    // Full-width rows, so the shortcut hint sits at the row's right edge instead of on the label.
                    let (row_rect, resp) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.), Sense::click());
                    let lit = selected || resp.hovered();
                    let k = ui.ctx().animate_bool_with_time(resp.id, lit, 0.12);
                    if k > 0. {
                        let fill = if selected {
                            color(self.theme.accent).gamma_multiply(0.35)
                        } else {
                            color(mix(self.theme.panel, self.theme.foreground, 0.1))
                        };
                        ui.painter().rect_filled(row_rect, 6., fill.gamma_multiply(k));
                    }
                    if let Some(g) = menus::glyph(&action.title) {
                        ui.painter().text(
                            row_rect.left_center() + Vec2::new(10., 0.),
                            egui::Align2::LEFT_CENTER,
                            g,
                            FontId::proportional(12.),
                            color(self.theme.muted),
                        );
                    }
                    ui.painter().text(
                        row_rect.left_center() + Vec2::new(10. + menus::ICON_COLUMN, 0.),
                        egui::Align2::LEFT_CENTER,
                        &action.title,
                        FontId::proportional(13.),
                        color(self.theme.foreground),
                    );
                    if !action.hint.is_empty() {
                        // The shortcut as a small keycap at the row's right edge.
                        let galley = ui.painter().layout_no_wrap(
                            action.hint.clone(),
                            FontId::proportional(11.),
                            color(self.theme.muted),
                        );
                        let cap = Rect::from_min_size(
                            Pos2::new(
                                resp.rect.right() - 8. - galley.size().x - 10.,
                                resp.rect.center().y - 9.,
                            ),
                            Vec2::new(galley.size().x + 10., 18.),
                        );
                        ui.painter().rect_filled(
                            cap,
                            5.,
                            color(mix(self.theme.panel, self.theme.foreground, 0.1)),
                        );
                        ui.painter().galley(
                            cap.min + Vec2::new(5., (18. - galley.size().y) / 2.),
                            galley,
                            color(self.theme.muted),
                        );
                    }
                    if resp.clicked() {
                        chosen = Some(*i);
                    }
                }
                if ranked.is_empty() && !typed {
                    ui.label(
                        RichText::new("No matching action. Enter runs it as a command: new [cwd] · rename TITLE · search TEXT · export /file")
                            .size(11.)
                            .color(color(self.theme.muted)),
                    );
                }
                if esc {
                    close = true;
                } else if enter {
                    if typed {
                        run_typed = true;
                    } else if let Some((_, i)) = ranked.get(self.palette_sel) {
                        chosen = Some(*i);
                    } else if !self.command.trim().is_empty() {
                        run_typed = true;
                    }
                }
            });
        if let Some(i) = chosen {
            self.command.clear();
            self.run_action(list[i].kind.clone());
        } else if run_typed {
            self.palette = false;
            self.execute();
        } else if close {
            self.palette = false;
        }
    }
    /// Id of the committed theme, remembered per display name (loading themes is not free).
    fn theme_id(&mut self) -> Option<String> {
        let name = self.committed_theme.name.clone();
        if self.theme_id_cache.as_ref().map(|(n, _)| n) != Some(&name) {
            self.theme_id_cache = Some((name.clone(), find_theme_id(&name)));
        }
        self.theme_id_cache.as_ref().and_then(|(_, id)| id.clone())
    }
    /// Switch to `theme_light` / `theme_dark` when macOS changes appearance (and once at start).
    fn follow_system_theme(&mut self, ctx: &egui::Context) {
        let (light, dark) = (self.cfg.theme_light.clone(), self.cfg.theme_dark.clone());
        if light.is_none() && dark.is_none() {
            return;
        }
        let Some(system) = ctx.system_theme() else {
            return;
        };
        let is_dark = system == egui::Theme::Dark;
        if self.last_system_dark == Some(is_dark) {
            return;
        }
        self.last_system_dark = Some(is_dark);
        if let Some(name) = if is_dark { dark } else { light } {
            self.run_action(ActionKind::Theme(name));
        }
    }
    /// The focused pane's working directory (live when shell integration reports it), for new panes.
    fn focus_cwd(&self) -> Option<String> {
        self.panes
            .get(&self.focus)
            .map(|p| p.info.cwd.clone())
            .filter(|c| !c.is_empty())
    }
    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }
    /// Adopt new `[desktop]` options and rebuild the keymap, reporting binding problems.
    fn apply_config(&mut self, cfg: DesktopConfig) {
        let (keymap, problems) = Keymap::build(&cfg.keys);
        self.keymap = keymap;
        self.unfocused_dim = 1. - cfg.unfocused_opacity;
        if cfg.font_size != self.cfg.font_size {
            self.font_size = cfg.font_size;
        }
        self.triggers = triggers::compile(&cfg.triggers);
        // Only a change of the option opens or closes the sidebar, so a reload never undoes ⌘B.
        if cfg.file_browser != self.cfg.file_browser {
            self.files.open = cfg.file_browser;
        }
        self.cfg = cfg;
        if let Some(first) = problems.first() {
            self.toast(format!("Config keys: {first}"));
        }
    }
    fn reload_config(&mut self) {
        match config::load_desktop_theme() {
            Ok((cfg, theme)) => {
                self.apply_config(cfg);
                self.notify_cfg = config::load().map(|c| c.notify).unwrap_or_default();
                let changed = theme_changed(self.config_theme.as_deref(), &theme);
                self.config_theme = Some(theme.clone());
                if changed {
                    self.run_action(ActionKind::Theme(theme));
                }
                self.config_error = None;
                self.toast("Config reloaded");
            }
            Err(e) => self.config_error = Some(format!("{e:#}")),
        }
    }
    fn config_stamp() -> Option<std::time::SystemTime> {
        std::fs::metadata(config::path().ok()?)
            .ok()?
            .modified()
            .ok()
    }
    /// Once a second, reload the config if its file changed (only in the real app).
    fn poll_config(&mut self) {
        if !self.persist || self.config_checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.config_checked = Instant::now();
        let stamp = Self::config_stamp();
        if stamp != self.config_mtime {
            self.config_mtime = stamp;
            self.reload_config();
        }
    }
    fn find_step(&mut self, back: bool) {
        let n = self.find_count.max(1);
        self.find_cur = if back {
            (self.find_cur + n - 1) % n
        } else {
            (self.find_cur + 1) % n
        };
    }
    fn close_find(&mut self) {
        self.find_open = false;
        self.find_query = None;
        self.find_error = None;
        self.find_count = 0;
    }
    fn move_tab(&mut self, delta: isize) {
        let to = self.active.checked_add_signed(delta);
        if let Some(to) = to.filter(|t| *t < self.tabs.len()) {
            self.tabs.swap(self.active, to);
            self.active = to;
        }
    }
    fn settings_snapshot(&mut self) -> DesktopSettings {
        DesktopSettings {
            width: Some(self.win_size.x.round()).filter(|w| *w > 0.),
            height: Some(self.win_size.y.round()).filter(|h| *h > 0.),
            theme: self.theme_id(),
            font_size: Some(self.font_size),
            layout: Some(self.layout.name().to_string()),
            welcomed: Some(true),
        }
    }
    /// Remember window size, theme, font and layout; throttled unless `force`.
    fn save_settings(&mut self, force: bool) {
        if !self.persist {
            return;
        }
        let now = self.settings_snapshot();
        if now != self.saved && (force || self.last_save.elapsed() > Duration::from_secs(2)) {
            let _ = now.save();
            self.saved = now;
            self.last_save = Instant::now();
        }
    }
}
impl eframe::App for Desktop {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.cfg.window_opacity < 1. {
            [0., 0., 0., 0.]
        } else {
            visuals.panel_fill.to_normalized_gamma_f32()
        }
    }
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
        self.window_edge(ctx);
    }
}
impl Desktop {
    /// Window size (points) for a terminal `rows` high and `text_width` wide, plus all margins.
    fn cells_to_window(&self, text_width: f32, rows: u16) -> (f32, f32) {
        let (left, top, right, bottom) = self.cfg.insets();
        let inset = 2. * self.cfg.pane_inset;
        let (margin_x, margin_y) = (left + right + inset, top + bottom + inset);
        let bar = if self.cfg.chrome == crate::config::Chrome::Full {
            bar::BAR_HEIGHT
        } else if self.cfg.tab_bar == crate::config::TabBar::Bottom {
            bar::TAB_STRIP_HEIGHT
        } else {
            0.
        };
        (
            text_width + margin_x,
            // The two points keep rounding from costing the last row.
            f32::from(rows) * self.font_size * self.cfg.line_height + margin_y + bar + 2.,
        )
    }
    fn size_window_to_cells(&mut self, ctx: &egui::Context, cols: u16, rows: u16) {
        let font = FontId::monospace(self.font_size);
        let cw = ctx.fonts(|f| f.glyph_width(&font, 'M'));
        let (w, h) = self.cells_to_window(f32::from(cols) * cw, rows);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(w, h)));
    }
    /// Cross-fade between themes over a quarter second whenever the theme changes.
    fn fade_theme(&mut self, ctx: &egui::Context) {
        const SECONDS: f64 = 0.25;
        let now = ctx.input(|i| i.time);
        let changed = match &self.theme_fade {
            None => self.theme.name != self.theme_shown.name,
            Some((_, to, _)) => self.theme.name != to.name,
        };
        if changed {
            self.theme_fade = Some((self.theme_shown.clone(), self.theme.clone(), now));
        }
        let Some((from, to, started)) = self.theme_fade.clone() else {
            return;
        };
        let k = ((now - started) / SECONDS).clamp(0., 1.) as f32;
        let eased = k * k * (3. - 2. * k);
        if k >= 1. {
            self.theme = to.clone();
            self.theme_shown = to;
            self.theme_fade = None;
        } else {
            self.theme = from.blend(&to, eased);
            self.theme_shown = self.theme.clone();
            ctx.request_repaint();
        }
    }
    /// A hairline around a rounded bare window, with a faint highlight along the top.
    fn window_edge(&self, ctx: &egui::Context) {
        if self.cfg.corner_radius <= 0. || self.cfg.chrome != crate::config::Chrome::None {
            return;
        }
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if fullscreen {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("teryon-window-edge"),
        ));
        let rect = ctx.screen_rect().shrink(0.5);
        let radius = self.cfg.corner_radius as u8;
        painter.rect_stroke(
            rect,
            egui::CornerRadius::same(radius),
            Stroke::new(1_f32, color(self.theme.border).gamma_multiply(0.7)),
            egui::StrokeKind::Inside,
        );
        let r = self.cfg.corner_radius;
        painter.line_segment(
            [
                Pos2::new(rect.left() + r, rect.top() + 1.),
                Pos2::new(rect.right() - r, rect.top() + 1.),
            ],
            Stroke::new(1_f32, Color32::from_white_alpha(14)),
        );
    }
    /// Without a title bar, a thin strip along the top edge moves the window (double-click maximizes).
    fn chrome_drag(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("teryon-drag-strip"))
            .fixed_pos(egui::Pos2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let size = Vec2::new(ctx.screen_rect().width(), 6.);
                let strip = ui.allocate_response(size, Sense::click_and_drag());
                if strip.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if strip.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
            });
    }
}
/// Whether a reloaded config names a different theme than the one last read (the first read is never a change).
fn theme_changed(previous: Option<&str>, next: &str) -> bool {
    previous.is_some_and(|p| p != next)
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.save_settings(true);
        if self.discard_workspace {
            // Let the worker remove the workspace before the process exits.
            let _ = self.tx.send(Command::Discard);
            let deadline = Instant::now() + Duration::from_secs(2);
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                if matches!(self.rx.recv_timeout(left), Ok(Update::Discarded) | Err(_)) {
                    break;
                }
            }
        }
        for (_, mut child) in self.forwards.drain() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.tx.try_send(Command::Stop);
        for tab in &self.tabs {
            if let Some(s) = &tab.state {
                let _ = s.tx.try_send(Command::Stop);
            }
        }
    }
}
/// On macOS the title bar is merged into TERYON's own top bar (window buttons stay).
/// Modifier state as the keymap sees it: `cmd` is ⌘ on macOS and Ctrl+Shift elsewhere (Ctrl alone
/// belongs to the shell), so on Linux the ⇧ variants of bindings are not available.
fn chord_mods(m: &egui::Modifiers) -> Mods {
    if cfg!(target_os = "macos") {
        Mods {
            cmd: m.mac_cmd,
            ctrl: m.ctrl,
            alt: m.alt,
            shift: m.shift,
        }
    } else {
        let cmd = m.ctrl && m.shift;
        Mods {
            cmd,
            ctrl: false,
            alt: m.alt,
            shift: m.shift && !cmd,
        }
    }
}
fn merged_titlebar(
    builder: egui::ViewportBuilder,
    chrome: crate::config::Chrome,
    native_fullscreen: bool,
) -> egui::ViewportBuilder {
    if chrome == crate::config::Chrome::None && !(native_fullscreen && cfg!(target_os = "macos")) {
        // A bare, still resizable window: no title bar, no buttons.
        builder.with_decorations(false)
    } else if cfg!(target_os = "macos") {
        builder
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
    } else {
        builder
    }
}
pub fn run(
    target: Target,
    workspace: String,
    readonly: bool,
    cwd: Option<String>,
    theme: Theme,
    settings: DesktopSettings,
    discard: bool,
) -> Result<()> {
    if !valid_name(&workspace) {
        bail!("invalid workspace name");
    }
    let (tx, rx) = mpsc::sync_channel(64);
    let (utx, urx) = mpsc::sync_channel(32);
    let host = target.host.clone();
    let target_for_tabs = target.clone();
    let ws = workspace.clone();
    thread::spawn(move || worker(target, ws, readonly, cwd, rx, utx, (30, 100)));
    let mut app = Desktop::new(host, workspace, readonly, theme, tx, urx);
    app.target = Some(target_for_tabs);
    app.config_mtime = Desktop::config_stamp();
    match config::load_desktop_theme() {
        Ok((cfg, theme)) => {
            app.apply_config(cfg);
            app.config_theme = Some(theme);
            app.notify_cfg = config::load().map(|c| c.notify).unwrap_or_default();
        }
        Err(e) => app.config_error = Some(format!("{e:#}")),
    }
    app.persist = true;
    app.discard_workspace = discard && !readonly;
    if let Some(mode) = settings.layout.as_deref().and_then(LayoutMode::parse) {
        app.layout = mode;
    }
    app.font_size = settings.font_size.unwrap_or(app.cfg.font_size);
    let transparent = app.cfg.window_opacity < 1.
        || (app.cfg.corner_radius > 0. && app.cfg.chrome == crate::config::Chrome::None);
    let family = app.cfg.font_family.clone();
    app.applied_font = Some(family.clone());
    let (mut width, mut height) = (
        settings.width.unwrap_or(1180.),
        settings.height.unwrap_or(760.),
    );
    if let (Some(cols), Some(rows), false) = (
        app.cfg.initial_cols,
        app.cfg.initial_rows,
        app.cfg.maximized,
    ) {
        // A first guess so the window opens close to its size; the first frame sets it exactly.
        app.pending_cells = Some((cols, rows));
        (width, height) = app.cells_to_window(f32::from(cols) * app.font_size * 0.6, rows);
    }
    if settings.welcomed != Some(true) {
        app.toast("Press ⌘K for everything, ⌘/ for shortcuts");
    }
    app.saved = settings;
    app.pending_fullscreen = app.cfg.fullscreen;
    let (maximized, fullscreen) = (app.cfg.maximized, app.cfg.fullscreen);
    eframe::egui_glow::set_font_gamma(app.cfg.text_gamma);
    eframe::run_native(
        "TERYON — Your Mac mini. Wherever you work.",
        eframe::NativeOptions {
            viewport: merged_titlebar(
                egui::ViewportBuilder::default(),
                app.cfg.chrome,
                app.cfg.fullscreen,
            )
            .with_transparent(transparent)
            .with_maximized(maximized && !fullscreen)
            .with_icon(egui::IconData {
                rgba: include_bytes!("../../docs/brand/teryon-icon-128.rgba").to_vec(),
                width: 128,
                height: 128,
            })
            .with_inner_size([width, height])
            .with_min_inner_size([400., 360.]),
            ..Default::default()
        },
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx, family.as_deref());
            if app.cfg.window_blur > 0 && transparent {
                blur::apply(cc, app.cfg.window_blur);
            }
            if app.cfg.corner_radius > 0. && app.cfg.chrome == crate::config::Chrome::None {
                blur::round_corners(cc, app.cfg.corner_radius);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canvas_focus_routes_input_while_editors_capture_it() {
        let (tx, rx) = mpsc::sync_channel(8);
        let (_utx, urx) = mpsc::sync_channel(8);
        let mut app = Desktop::new(
            "mini".into(),
            "test".into(),
            false,
            Theme::load("obsidian").unwrap(),
            tx,
            urx,
        );
        app.online = true;
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![egui::Event::Text("echo λ".into())],
            ..Default::default()
        };
        let _ = ctx.run(raw.clone(), |ctx| {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new(("teryon-terminal", 1u64))));
            app.keyboard(ctx);
        });
        assert!(
            matches!(rx.try_recv(),Ok(Command::Input{pane:1,data}) if data=="echo λ".as_bytes())
        );
        let _ = ctx.run(raw, |ctx| {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("draft-text-editor")));
            app.keyboard(ctx);
        });
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn terminal_focus_lock_sends_tab_esc_and_ctrl_u_to_the_pty() {
        let (tx, rx) = mpsc::sync_channel(8);
        let (_utx, urx) = mpsc::sync_channel(8);
        let mut app = Desktop::new(
            "mini".into(),
            "test".into(),
            false,
            Theme::load("obsidian").unwrap(),
            tx,
            urx,
        );
        app.online = true;
        let term_id = egui::Id::new(("teryon-terminal", 1u64));
        let ctx = egui::Context::default();
        let press = |key: Key, modifiers: egui::Modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        // The canvas must exist in used_ids each frame or egui's dead-man's-switch drops focus.
        let frame = |app: &mut Desktop, events: Vec<egui::Event>, grab: bool| {
            let mut still_focused = false;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let r = ui.interact(ui.max_rect(), term_id, egui::Sense::click_and_drag());
                        if grab {
                            r.request_focus();
                        }
                    });
                    still_focused = ctx.memory(|m| m.focused()) == Some(term_id);
                    app.keyboard(ctx);
                },
            );
            still_focused
        };
        // Frame 1: gain focus. Frame 2: keyboard() can apply the lock (had_focus_last_frame).
        frame(&mut app, vec![], true);
        frame(&mut app, vec![], false);
        for (events, want) in [
            (
                vec![press(Key::Tab, egui::Modifiers::NONE)],
                b"\t".as_slice(),
            ),
            (
                vec![press(Key::Escape, egui::Modifiers::NONE)],
                b"\x1b".as_slice(),
            ),
            (
                vec![press(Key::U, egui::Modifiers::CTRL)],
                b"\x15".as_slice(),
            ),
        ] {
            assert!(
                frame(&mut app, events, false),
                "EventFilter must keep focus on the terminal canvas"
            );
            assert!(
                matches!(rx.try_recv(), Ok(Command::Input { pane: 1, data }) if data == want),
                "expected {:?}",
                want
            );
        }
    }
    #[test]
    fn native_ui_renders_themes_sizes_and_disconnect_states() {
        for name in BUILTINS {
            for width in [420., 1180.] {
                let (tx, _rx) = mpsc::sync_channel(64);
                let (_utx, urx) = mpsc::sync_channel(32);
                let mut app = Desktop::new(
                    "mini".into(),
                    "test".into(),
                    false,
                    Theme::load(name).unwrap(),
                    tx,
                    urx,
                );
                let mut parser = vt100::Parser::new(24, 80, 0);
                parser.process(b"\x1b[32mHello\x1b[0m Unicode: \xe7\x95\x8c");
                let view = PaneView {
                    id: 1,
                    title: "Shell".into(),
                    cwd: "/tmp".into(),
                    pid: Some(123),
                    rows: 24,
                    cols: 80,
                    revision: 1,
                    screen: None,
                    exited: None,
                    ..Default::default()
                };
                app.panes.insert(1, Pane::new(view, parser));
                let ctx = egui::Context::default();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 680.))),
                        ..Default::default()
                    },
                    |ctx| app.frame(ctx),
                );
                assert!(!output.shapes.is_empty());
            }
        }
    }
    #[test]
    fn native_ui_renders_every_layout_with_several_panes() {
        for mode in [
            LayoutMode::Single,
            LayoutMode::Vertical,
            LayoutMode::Horizontal,
            LayoutMode::Grid,
        ] {
            for count in 1..=5u64 {
                let (tx, _rx) = mpsc::sync_channel(64);
                let (_utx, urx) = mpsc::sync_channel(32);
                let mut app = Desktop::new(
                    "mini".into(),
                    "test".into(),
                    false,
                    Theme::load("tokyo-night").unwrap(),
                    tx,
                    urx,
                );
                app.layout = mode;
                for id in 1..=count {
                    let view = PaneView {
                        id,
                        title: format!("Shell {id}"),
                        cwd: "/tmp".into(),
                        pid: Some(1),
                        rows: 24,
                        cols: 80,
                        revision: 1,
                        screen: None,
                        exited: None,
                        ..Default::default()
                    };
                    let parser = vt100::Parser::new(24, 80, 0);
                    app.panes.insert(id, Pane::new(view, parser));
                }
                let ctx = egui::Context::default();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 760.))),
                        ..Default::default()
                    },
                    |ctx| app.frame(ctx),
                );
                assert!(!output.shapes.is_empty(), "{mode:?} {count}");
            }
        }
    }
    #[test]
    fn hover_preview_reverts_to_the_committed_theme() {
        let (tx, _rx) = mpsc::sync_channel(4);
        let (_utx, urx) = mpsc::sync_channel(4);
        let mut app = Desktop::new(
            "mini".into(),
            "t".into(),
            false,
            Theme::load("nord").unwrap(),
            tx,
            urx,
        );
        app.theme = Theme::load("dracula").unwrap();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
        assert_eq!(app.theme.name, "Nord");
    }
    #[test]
    fn tabs_keep_separate_state_and_drain_in_the_background() {
        let (tx, _rx) = mpsc::sync_channel(4);
        let (_utx, urx) = mpsc::sync_channel(4);
        let mut app = Desktop::new(
            "mini".into(),
            "a".into(),
            false,
            Theme::load("nord").unwrap(),
            tx,
            urx,
        );
        let (btx, _brx) = mpsc::sync_channel(4);
        let (butx, burx) = mpsc::sync_channel(4);
        app.tabs.push(Tab {
            host: "m".into(),
            name: "b".into(),
            activity: Default::default(),
            state: Some(Session {
                extra: SessionExtra::default(),
                host: "m".into(),
                target: None,
                workspace: "b".into(),
                online: false,
                notice: String::new(),
                panes: BTreeMap::new(),
                focus: 7,
                layout: LayoutMode::Grid,
                tx: btx,
                rx: burx,
                history: None,
                query: String::new(),
                last_size: (0, 0),
            }),
        });
        app.notice = "mine".into();
        butx.send(Update::Online).unwrap();
        app.pump_background();
        assert_eq!((app.workspace.as_str(), app.notice.as_str()), ("a", "mine"));
        app.activate(1);
        assert_eq!(
            (app.workspace.as_str(), app.focus, app.layout),
            ("b", 7, LayoutMode::Grid)
        );
        assert!(app.online, "background update was applied to tab b");
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.frame(ctx));
        app.close_tab();
        assert_eq!(
            (app.tabs.len(), app.workspace.as_str(), app.active),
            (1, "a", 0)
        );
        app.close_tab();
        assert_eq!(app.tabs.len(), 1);
    }
    #[test]
    fn a_background_tab_gets_a_mark_when_its_panes_change_and_loses_it_when_opened() {
        let (tx, _rx) = mpsc::sync_channel(4);
        let (_utx, urx) = mpsc::sync_channel(4);
        let mut app = Desktop::new("mini".into(), "a".into(), false, Theme::load("nord").unwrap(), tx, urx);
        let (btx, _brx) = mpsc::sync_channel(4);
        let (butx, burx) = mpsc::sync_channel(8);
        app.tabs.push(Tab {
            host: "m".into(),
            name: "b".into(),
            activity: Default::default(),
            state: Some(Session {
                extra: SessionExtra::default(),
                host: "m".into(),
                target: None,
                workspace: "b".into(),
                online: false,
                notice: String::new(),
                panes: BTreeMap::new(),
                focus: 1,
                layout: LayoutMode::Single,
                tx: btx,
                rx: burx,
                history: None,
                query: String::new(),
                last_size: (0, 0),
            }),
        });
        let view = |revision| PaneView { id: 1, title: "sh".into(), rows: 5, cols: 20, revision, screen: Some(Vec::new()), ..Default::default() };
        butx.send(Update::State(vec![view(1)])).unwrap();
        app.pump_background();
        assert_eq!(app.tabs[1].activity.prefix(), "", "the first look is a baseline");
        butx.send(Update::State(vec![view(2)])).unwrap();
        app.pump_background();
        assert_eq!(app.tabs[1].activity.prefix(), "● ");
        app.activate(1);
        app.pump_background();
        assert_eq!(app.tabs[1].activity.prefix(), "", "the tab on screen is seen");
    }
    #[test]
    fn closing_a_pane_hides_it_with_an_undo_and_the_last_pane_is_kept() {
        let (mut app, rx) = app_with_screen("");
        app.close_focused_pane();
        assert!(drain(&rx).is_empty(), "a single pane is never closed");
        assert!(app.toast.as_ref().is_some_and(|(t, _)| t.contains("last pane")));
        app.add_test_pane(2);
        app.focus = 2;
        app.run_action(ActionKind::ClosePane);
        assert!(matches!(&drain(&rx)[..], [Command::SoftClose(2)]));
        assert!(app.toast.as_ref().is_some_and(|(t, _)| t.contains("brings it back")));
        app.run_action(ActionKind::ReopenPane);
        assert!(matches!(&drain(&rx)[..], [Command::Reopen]));
        app.readonly = true;
        app.close_focused_pane();
        assert!(drain(&rx).is_empty(), "read only attachments cannot close panes");
    }
    pub(super) fn app_for_extras() -> (Desktop, mpsc::Receiver<Command>) {
        app_with_screen("")
    }
    impl Desktop {
        pub(super) fn add_test_pane(&mut self, id: u64) {
            let info = PaneView {
                id,
                title: "sh".into(),
                rows: 10,
                cols: 60,
                ..Default::default()
            };
            self.panes
                .insert(id, Pane::new(info, vt100::Parser::new(10, 60, 0)));
        }
    }
    fn app_with(theme: &str) -> Desktop {
        let (tx, _rx) = mpsc::sync_channel(4);
        let (_utx, urx) = mpsc::sync_channel(4);
        Desktop::new(
            "m".into(),
            "a".into(),
            false,
            Theme::load(theme).unwrap(),
            tx,
            urx,
        )
    }
    fn render(app: &mut Desktop, ctx: &egui::Context, width: f32) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 700.))),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.frame(ctx));
    }
    #[test]
    fn every_theme_renders_at_every_width() {
        let ctx = egui::Context::default();
        for name in BUILTINS {
            let mut app = app_with(name);
            for width in [420., 500., 700., 1180.] {
                render(&mut app, &ctx, width);
            }
        }
    }
    #[test]
    fn palette_filters_runs_actions_and_keeps_typed_commands() {
        let mut app = app_with("nord");
        let ctx = egui::Context::default();
        app.run_action(ActionKind::Layout(LayoutMode::Grid));
        assert_eq!(app.layout, LayoutMode::Grid);
        app.run_action(ActionKind::Zoom);
        assert_eq!(app.layout, LayoutMode::Single);
        app.run_action(ActionKind::Zoom);
        assert_eq!(app.layout, LayoutMode::Grid);
        app.run_action(ActionKind::Theme("dracula".into()));
        assert_eq!(app.theme.name, "Dracula");
        app.run_action(ActionKind::FontUp);
        assert_eq!(app.font_size, 14.);
        app.run_action(ActionKind::NewTab);
        assert!(app.palette && app.command == "tab ");
        app.command = "lay gr".into();
        render(&mut app, &ctx, 900.);
        assert!(actions(&app.keymap, &app.theme_names, &[], &[])
            .iter()
            .any(|a| a.title == "Layout: grid"));
        assert!(uikit::fuzzy_score("lay gr", "Layout: grid").is_some());
    }
    #[test]
    fn tabs_reorder_and_find_wraps() {
        let mut app = app_with("nord");
        for name in ["b", "c"] {
            let (tx, _rx) = mpsc::sync_channel(4);
            let (_utx, urx) = mpsc::sync_channel(4);
            app.tabs.push(Tab {
                host: "m".into(),
                name: name.into(),
                activity: Default::default(),
                state: Some(Session {
                    extra: SessionExtra::default(),
                    host: "m".into(),
                    target: None,
                    workspace: name.into(),
                    online: false,
                    notice: String::new(),
                    panes: BTreeMap::new(),
                    focus: 1,
                    layout: LayoutMode::Single,
                    tx,
                    rx: urx,
                    history: None,
                    query: String::new(),
                    last_size: (0, 0),
                }),
            });
        }
        app.move_tab(1);
        assert_eq!((app.active, app.tabs[1].name.as_str()), (1, "a"));
        app.move_tab(-5);
        assert_eq!(app.active, 1);
        app.find_count = 3;
        app.find_step(true);
        assert_eq!(app.find_cur, 2);
        app.find_step(false);
        assert_eq!(app.find_cur, 0);
    }
    #[test]
    fn settings_snapshot_uses_builtin_ids() {
        let mut app = app_with("mac27-dark");
        app.win_size = Vec2::new(1000.4, 700.);
        let s = app.settings_snapshot();
        assert_eq!(s.theme.as_deref(), Some("mac27-dark"));
        assert_eq!(s.width, Some(1000.));
    }
    #[test]
    fn block_selection_is_rectangular_and_linear_selection_reads_in_order() {
        let block = Selection {
            pane: 1,
            a: (1, 2),
            b: (3, 5),
            block: true,
        };
        assert!(block.contains(2, 5) && !block.contains(2, 6) && !block.contains(0, 3));
        let linear = Selection {
            pane: 1,
            a: (1, 8),
            b: (2, 3),
            block: false,
        };
        assert!(linear.contains(1, 9) && linear.contains(2, 3) && !linear.contains(2, 4));
    }
    #[test]
    fn scrolling_back_fetches_history_once_and_returns_to_live() {
        let (tx, rx) = mpsc::sync_channel(8);
        let (_utx, urx) = mpsc::sync_channel(4);
        let mut app = Desktop::new(
            "m".into(),
            "a".into(),
            false,
            Theme::load("nord").unwrap(),
            tx,
            urx,
        );
        app.online = true;
        app.view_rows.insert(1, 10);
        app.scroll_by(1, 5);
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::Capture(1, CapturePurpose::Scrollback(1)))
        ));
        app.scroll_by(1, 3);
        assert!(rx.try_recv().is_err(), "second scroll must not refetch");
        let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
        app.finish_scrollback(1, &text, 0);
        assert_eq!(app.scrollback[&1].offset, 8);
        app.scroll_by(1, 1000);
        assert_eq!(app.scrollback[&1].offset, 90);
        app.scroll_by(1, -1000);
        assert!(!app.scrollback.contains_key(&1));
        app.cfg.scrollback_lines = 100;
        app.scroll_by(1, 5);
        app.finish_scrollback(
            1,
            &(0..500).map(|i| format!("{i}\n")).collect::<String>(),
            0,
        );
        assert_eq!(app.scrollback[&1].lines.len(), 110);
    }
    #[test]
    fn config_changes_apply_to_keys_dimming_and_font_size() {
        let mut app = app_with("nord");
        let mut cfg = DesktopConfig {
            unfocused_opacity: 0.5,
            font_size: 17.,
            ..Default::default()
        };
        cfg.keys.insert("palette".into(), "cmd+p".into());
        app.apply_config(cfg);
        assert_eq!((app.unfocused_dim, app.font_size), (0.5, 17.));
        assert_eq!(app.keymap.hint("palette"), "⌘P");
        let mut bad = DesktopConfig::default();
        bad.keys.insert("zoom".into(), "cmd+k".into());
        app.apply_config(bad);
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.contains("bound to both")));
    }
    #[test]
    fn command_history_search_is_local_only() {
        let (tx, _rx) = mpsc::sync_channel(8);
        let (_utx, urx) = mpsc::sync_channel(4);
        let mut app = Desktop::new(
            "mini".into(),
            "default".into(),
            false,
            Theme::load("mac27-dark").unwrap(),
            tx,
            urx,
        );
        app.cmdhist_toggle();
        assert!(!app.cmdhist.open);
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.contains("local workspace")));
    }
    #[test]
    fn option_is_meta_and_ctrl_space_is_nul() {
        let (tx, rx) = mpsc::sync_channel(8);
        let (_utx, urx) = mpsc::sync_channel(4);
        let mut app = Desktop::new(
            "m".into(),
            "a".into(),
            false,
            Theme::load("nord").unwrap(),
            tx,
            urx,
        );
        app.online = true;
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let raw = egui::RawInput {
            events: vec![
                key(Key::B, alt),
                egui::Event::Text("∫".into()),
                key(Key::Space, ctrl),
            ],
            modifiers: alt,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        let mut sent = Vec::new();
        while let Ok(Command::Input { data, .. }) = rx.try_recv() {
            sent.push(data);
        }
        assert_eq!(sent, vec![b"\x1bb".to_vec(), vec![0]]);
    }
    #[test]
    fn bound_keys_run_their_actions() {
        let (tx, rx) = mpsc::sync_channel(8);
        let (_utx, urx) = mpsc::sync_channel(4);
        let mut app = Desktop::new(
            "m".into(),
            "a".into(),
            false,
            Theme::load("nord").unwrap(),
            tx,
            urx,
        );
        app.online = true;
        let mods = if cfg!(target_os = "macos") {
            egui::Modifiers {
                mac_cmd: true,
                command: true,
                shift: true,
                ..Default::default()
            }
        } else {
            egui::Modifiers {
                ctrl: true,
                shift: true,
                command: true,
                ..Default::default()
            }
        };
        let key = if cfg!(target_os = "macos") {
            Key::D
        } else {
            Key::E
        };
        let _ = key;
        let raw = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::D,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            }],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        if cfg!(target_os = "macos") {
            assert_eq!(app.layout, LayoutMode::Vertical);
            assert!(matches!(rx.try_recv(), Ok(Command::New(None))));
        } else {
            // Ctrl+Shift stands in for the command key, so ⌘D is the only reachable binding.
            assert_eq!(app.layout, LayoutMode::Horizontal);
        }
    }
    #[test]
    fn palette_previews_themes_and_reverts_when_closed() {
        let mut app = app_with("nord");
        let ctx = egui::Context::default();
        app.palette = true;
        app.command = "Theme: dracula".into();
        render(&mut app, &ctx, 900.);
        assert_eq!(app.theme.name, "Dracula");
        assert_eq!(app.committed_theme.name, "Nord");
        app.palette = false;
        render(&mut app, &ctx, 900.);
        assert_eq!(app.theme.name, "Nord");
    }
    #[test]
    fn follows_the_system_appearance_and_keeps_manual_choices_until_it_changes() {
        let mut app = app_with("nord");
        app.cfg.theme_light = Some("pearl".into());
        app.cfg.theme_dark = Some("dracula".into());
        let ctx = egui::Context::default();
        let run = |app: &mut Desktop, dark: bool| {
            let raw = egui::RawInput {
                system_theme: Some(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                }),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| app.frame(ctx));
        };
        run(&mut app, true);
        assert_eq!(app.theme.name, "Dracula");
        app.run_action(ActionKind::Theme("nord".into()));
        run(&mut app, true);
        assert_eq!(
            app.theme.name, "Nord",
            "same appearance: manual choice stays"
        );
        run(&mut app, false);
        assert_eq!(app.theme.name, "Pearl");
    }
    #[test]
    fn opening_a_workspace_adds_a_tab_per_host_and_reuses_existing_ones() {
        let mut app = app_with("nord");
        app.target = Target::new("local".into(), None).ok();
        app.open_tab("m", "other");
        assert_eq!((app.tabs.len(), app.active), (2, 1));
        assert_eq!((app.host.as_str(), app.workspace.as_str()), ("m", "other"));
        app.open_tab("m", "a");
        assert_eq!((app.tabs.len(), app.active), (2, 0));
        app.close_tab();
        assert_eq!(app.tabs.len(), 1);
    }
    /// A desktop with one pane whose screen shows `text`, online, with a command receiver.
    pub(in crate::desktop) fn app_with_screen(text: &str) -> (Desktop, mpsc::Receiver<Command>) {
        let (tx, rx) = mpsc::sync_channel(64);
        let (_utx, urx) = mpsc::sync_channel(4);
        let mut app = Desktop::new(
            "local".into(),
            "a".into(),
            false,
            Theme::load("nord").unwrap(),
            tx,
            urx,
        );
        app.online = true;
        let mut parser = vt100::Parser::new(10, 60, 0);
        parser.process(text.as_bytes());
        app.panes.insert(
            1,
            Pane::new(
                PaneView {
                    id: 1,
                    title: "sh".into(),
                    cwd: "/tmp".into(),
                    pid: None,
                    rows: 10,
                    cols: 60,
                    revision: 1,
                    ..Default::default()
                },
                parser,
            ),
        );
        (app, rx)
    }
    fn key_event(key: Key, mods: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: mods,
        }
    }
    pub(in crate::desktop) fn drain(rx: &mpsc::Receiver<Command>) -> Vec<Command> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }
    #[test]
    fn broadcast_types_into_every_visible_live_pane_and_resets_on_tab_switch() {
        let (mut app, rx) = app_with_screen("");
        let info = app.panes[&1].info.clone();
        app.panes.insert(
            2,
            Pane::new(
                PaneView {
                    id: 2,
                    ..info.clone()
                },
                vt100::Parser::new(10, 60, 0),
            ),
        );
        app.panes.insert(
            3,
            Pane::new(
                PaneView {
                    id: 3,
                    exited: Some(0),
                    ..info
                },
                vt100::Parser::new(10, 60, 0),
            ),
        );
        for id in [1, 2, 3] {
            app.pane_rects
                .insert(id, Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.)));
        }
        app.input(b"x".to_vec());
        assert_eq!(drain(&rx).len(), 1, "no broadcast: focused pane only");
        app.run_action(ActionKind::Broadcast);
        assert!(app.broadcast);
        drain(&rx);
        app.input(b"ls\r".to_vec());
        let sent: Vec<u64> = drain(&rx)
            .into_iter()
            .filter_map(|c| {
                if let Command::Input { pane, .. } = c {
                    Some(pane)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(sent, vec![1, 2], "exited pane 3 is skipped");
        let (tx2, _rx2) = mpsc::sync_channel(4);
        let (_utx2, urx2) = mpsc::sync_channel(4);
        app.tabs.push(Tab {
            host: "m".into(),
            name: "b".into(),
            activity: Default::default(),
            state: Some(Session {
                extra: SessionExtra::default(),
                host: "m".into(),
                target: None,
                workspace: "b".into(),
                online: false,
                notice: String::new(),
                panes: BTreeMap::new(),
                focus: 1,
                layout: LayoutMode::Single,
                tx: tx2,
                rx: urx2,
                history: None,
                query: String::new(),
                last_size: (0, 0),
            }),
        });
        app.activate(1);
        assert!(!app.broadcast);
    }
    #[test]
    fn quick_select_labels_matches_and_acts_on_the_chosen_one() {
        let (mut app, rx) = app_with_screen("fix src/main.rs:42 and see https://example.com now");
        app.start_hints();
        let h = app.hints.as_ref().expect("hints started");
        assert_eq!(h.items.len(), 2);
        assert_eq!(h.items[0].label, "a");
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![egui::Event::Text("a".into())],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        assert!(app.hints.is_none());
        assert_eq!(app.copy_text.as_deref(), Some("src/main.rs:42"));
        // Tab cycles to paste, then the second label pastes it into the pane.
        app.start_hints();
        let raw = egui::RawInput {
            events: vec![
                key_event(Key::Tab, egui::Modifiers::NONE),
                egui::Event::Text("s".into()),
            ],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        let sent = drain(&rx);
        assert!(
            matches!(&sent[..], [Command::Input { pane: 1, data }] if data == b"https://example.com")
        );
        // Esc cancels, and an empty screen says so.
        app.start_hints();
        let raw = egui::RawInput {
            events: vec![key_event(Key::Escape, egui::Modifiers::NONE)],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        assert!(app.hints.is_none());
        let (mut empty, _rx) = app_with_screen("plain words only");
        empty.start_hints();
        assert!(empty.hints.is_none());
    }
    #[test]
    fn copy_mode_selects_with_vi_keys_and_yanks_history_text() {
        let (mut app, rx) = app_with_screen("one\r\ntwo\r\nthree");
        app.view_rows.insert(1, 10);
        app.start_copy_mode();
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::Capture(1, CapturePurpose::Scrollback(1)))
        ));
        app.finish_scrollback(1, "old line\none\ntwo\nthree", 0);
        assert!(app.copy_active());
        let ctx = egui::Context::default();
        let typed = |app: &mut Desktop, s: &str| {
            let raw = egui::RawInput {
                events: vec![egui::Event::Text(s.into())],
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        };
        typed(&mut app, "kk");
        typed(&mut app, "V");
        typed(&mut app, "j");
        typed(&mut app, "y");
        assert_eq!(app.copy_text.as_deref(), Some("one\ntwo"));
        assert!(!app.copy_active() && app.scrollback.is_empty());
        // Typing in a pane while copy mode is off still reaches the shell.
        typed(&mut app, "q");
        assert!(matches!(&drain(&rx)[..], [Command::Input { pane: 1, data }] if data == b"q"));
    }
    #[test]
    fn menu_choices_select_copy_and_run_actions() {
        let (mut app, rx) = app_with_screen("one\r\ntwo");
        app.run_menu(1, menus::MenuPick::SelectAll);
        assert!(app.selection.is_some());
        app.run_menu(1, menus::MenuPick::Copy);
        assert!(app
            .copy_text
            .as_deref()
            .is_some_and(|t| t.starts_with("one\ntwo")));
        app.run_menu(1, menus::MenuPick::Act(ActionKind::NewPane));
        assert!(matches!(&drain(&rx)[..], [Command::New(_)]));
        // The dropdown and the context menu both draw without panicking.
        let ctx = egui::Context::default();
        let st = |full| menus::MenuState {
            selection: true,
            broadcast: false,
            full,
        };
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(menus::menu_body(
                    ui,
                    &menus::Cx {
                        keys: &app.keymap,
                        theme: &app.theme,
                        snippets: &app.cfg.snippets
                    },
                    &st(true)
                )
                .is_none());
                assert!(menus::menu_body(
                    ui,
                    &menus::Cx {
                        keys: &app.keymap,
                        theme: &app.theme,
                        snippets: &app.cfg.snippets
                    },
                    &st(false)
                )
                .is_none());
            });
        });
    }
    #[test]
    fn assistant_panel_asks_before_acting_and_shows_the_result() {
        use crate::ai::api::{Event, Model, Turn};
        use serde_json::{json, Value};
        use std::sync::{Arc, Mutex};
        struct Script(Mutex<Vec<Turn>>);
        impl Model for Script {
            fn stream(
                &self,
                _: &str,
                _: &Value,
                _: &[Value],
                on: &mut dyn FnMut(Event),
            ) -> Result<Turn, String> {
                let t = self.0.lock().unwrap().remove(0);
                for b in &t.content {
                    if b["type"] == "text" {
                        on(Event::Text(b["text"].as_str().unwrap().into()));
                    }
                }
                Ok(t)
            }
        }
        struct Ran(Mutex<Vec<String>>);
        impl crate::ai::tools::Runner for Ran {
            fn run(&self, p: &str, a: &[String], _: Duration) -> Result<String, String> {
                self.0.lock().unwrap().push(format!("{p} {}", a.join(" ")));
                Ok(String::new())
            }
        }
        let tool = Turn {
            content: vec![
                json!({"type": "tool_use", "id": "t1", "name": "close_pane", "input": {"pane": 1}}),
            ],
            stop_reason: None,
        };
        let text = Turn {
            content: vec![json!({"type": "text", "text": "Closed it."})],
            stop_reason: None,
        };
        let (mut app, rx) = app_with_screen("hi");
        let ran = Arc::new(Ran(Mutex::new(Vec::new())));
        app.ai_attach(Box::new(Script(Mutex::new(vec![tool, text]))), ran, vec![]);
        app.ai.open = true;
        app.ai_send("close the first pane");
        let ctx = egui::Context::default();
        let frame = |app: &mut Desktop| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.ai_panel(ctx));
        };
        let wait = |app: &mut Desktop, done: &dyn Fn(&Desktop) -> bool| {
            for _ in 0..200 {
                frame(app);
                if done(app) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("timed out");
        };
        // The changing tool waits for approval and nothing has been sent to the daemon yet.
        wait(&mut app, &|a| {
            a.ai.items.iter().any(|i| {
                matches!(
                    i,
                    ai::Item::Tool {
                        state: ai::ToolState::Waiting,
                        ..
                    }
                )
            })
        });
        assert!(drain(&rx).is_empty());
        app.ai
            .agent
            .as_ref()
            .unwrap()
            .decide("t1", crate::ai::Decision::Allow);
        wait(&mut app, &|a| !a.ai.busy);
        assert!(matches!(&drain(&rx)[..], [Command::Close(1)]));
        assert!(app
            .ai
            .items
            .iter()
            .any(|i| matches!(i, ai::Item::Assistant(t) if t == "Closed it.")));
        assert!(app.ai.items.iter().any(|i| matches!(
            i,
            ai::Item::Tool {
                state: ai::ToolState::Done(true, _),
                ..
            }
        )));
        // The panel draws with a transcript.
        frame(&mut app);
    }
    #[test]
    fn assistant_panel_without_a_key_shows_setup_help_and_the_empty_state() {
        let (mut app, _rx) = app_with_screen("hi");
        let ctx = egui::Context::default();
        app.ai.open = true;
        app.ai.set_key_found(false);
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.ai_panel(ctx));
        app.ai.set_key_found(true);
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.ai_panel(ctx));
        // Sending without a key never starts a request.
        app.ai.set_key_found(false);
        assert!(app.ai.agent.is_none());
    }
    #[test]
    fn find_toggles_rebuild_the_query_and_report_bad_regexes() {
        let (mut app, _rx) = app_with_screen("Error 12 and error 7");
        app.find_text = "error".into();
        app.rebuild_find();
        assert!(app.find_query.is_some());
        app.run_action(ActionKind::FindCase);
        assert!(app.find_case && app.find_open);
        app.find_text = r"\d+".into();
        app.run_action(ActionKind::FindRegex);
        assert!(app.find_regex && app.find_error.is_none());
        app.find_text = "(".into();
        app.rebuild_find();
        assert!(app.find_error.is_some() && app.find_query.is_none());
        app.close_find();
        assert!(app.find_error.is_none());
    }
    #[test]
    fn snippets_prompt_for_placeholders_and_only_auto_run_single_lines() {
        let (mut app, rx) = app_with_screen("");
        app.cfg.snippets = vec![
            crate::config::Snippet {
                name: "logs".into(),
                command: "tail -f {{file}}".into(),
                description: None,
                run: true,
            },
            crate::config::Snippet {
                name: "multi".into(),
                command: "a\nb".into(),
                description: None,
                run: true,
            },
            crate::config::Snippet {
                name: "plain".into(),
                command: "ls".into(),
                description: None,
                run: false,
            },
        ];
        app.run_action(ActionKind::Snippet(0));
        assert!(app.snippet_prompt.is_some() && drain(&rx).is_empty());
        let mut values = BTreeMap::new();
        values.insert("file".to_string(), "/var/log/x".to_string());
        app.send_snippet(0, &values);
        assert!(
            matches!(&drain(&rx)[..], [Command::Input { data, .. }] if data == b"tail -f /var/log/x\r")
        );
        app.run_action(ActionKind::Snippet(2));
        assert!(matches!(&drain(&rx)[..], [Command::Input { data, .. }] if data == b"ls"));
        app.run_action(ActionKind::Snippet(1));
        let sent = drain(&rx);
        assert!(
            matches!(&sent[..], [Command::Input { data, .. }] if !data.ends_with(b"\r")),
            "multi-line never auto-runs"
        );
        assert!(app.toast.is_some());
    }
    #[test]
    fn power_tool_overlays_render_without_trouble() {
        let (mut app, _rx) = app_with_screen("open /etc/hosts or https://example.com");
        let ctx = egui::Context::default();
        app.start_hints();
        render(&mut app, &ctx, 900.);
        app.hints = None;
        app.view_rows.insert(1, 10);
        app.start_copy_mode();
        app.finish_scrollback(1, "alpha\nbeta\ngamma", 0);
        let typed = egui::RawInput {
            events: vec![egui::Event::Text("vj".into())],
            ..Default::default()
        };
        let _ = ctx.run(typed, |ctx| app.keyboard(ctx));
        render(&mut app, &ctx, 900.);
        app.scrollback.clear();
        app.broadcast = true;
        app.pane_rects
            .insert(1, Rect::from_min_size(Pos2::ZERO, Vec2::splat(100.)));
        app.find_text = "etc".into();
        app.find_open = true;
        app.rebuild_find();
        render(&mut app, &ctx, 420.);
        render(&mut app, &ctx, 1180.);
        assert!(app.find_count >= 1);
    }
    fn mark(kind: char, line: u64, exit: Option<i32>, ms: Option<u64>) -> Mark {
        Mark {
            kind,
            line,
            exit,
            ms,
            at: 100 + line,
            cmd: None,
        }
    }
    #[test]
    fn durations_and_chips_read_naturally() {
        use term::{command_chip, format_duration};
        assert_eq!(format_duration(340), "340 ms");
        assert_eq!(format_duration(1200), "1.2 s");
        assert_eq!(format_duration(125_000), "2m 05s");
        assert_eq!(command_chip(&[]), None);
        assert_eq!(
            command_chip(&[mark('A', 0, None, None), mark('C', 1, None, None)]),
            Some(("running…".into(), false))
        );
        assert_eq!(
            command_chip(&[mark('C', 1, None, None), mark('D', 3, Some(0), Some(400))]),
            Some(("✓ 400 ms".into(), false))
        );
        assert_eq!(
            command_chip(&[mark('D', 3, Some(2), Some(61_000))]),
            Some(("✗ 2 · 1m 01s".into(), true))
        );
        assert_eq!(
            command_chip(&[mark('D', 3, None, None)]),
            Some(("done".into(), false))
        );
    }
    #[test]
    fn prompts_can_be_jumped_to_and_the_last_output_copied() {
        let (mut app, rx) = app_with_screen("");
        app.view_rows.insert(1, 5);
        app.panes.get_mut(&1).unwrap().info.top_line = 40;
        app.panes.get_mut(&1).unwrap().marks = vec![
            mark('A', 10, None, None),
            mark('C', 11, None, None),
            mark('D', 14, Some(0), Some(50)),
            mark('A', 14, None, None),
            mark('C', 15, None, None),
            mark('D', 18, Some(1), Some(900)),
            mark('A', 18, None, None),
        ];
        // History: 50 lines with absolute numbers 0..50, so line i reads "line i".
        let text: String = (0..50).map(|i| format!("line {i}\n")).collect();
        app.jump_prompt(true);
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::Capture(1, CapturePurpose::Scrollback(1)))
        ));
        app.finish_scrollback(1, &text, 0);
        // The newest prompt above the live screen (top line 40) is 18: it is placed at the view's top.
        assert_eq!(app.view_top_line(1), 18);
        app.jump_prompt(true);
        assert_eq!(app.view_top_line(1), 14);
        app.jump_prompt(true);
        assert_eq!(app.view_top_line(1), 10);
        app.jump_prompt(true);
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.contains("No earlier prompt")));
        app.jump_prompt(false);
        assert_eq!(app.view_top_line(1), 14);
        // Last output = lines from the C mark up to the D mark of the last command.
        app.copy_last_output();
        assert_eq!(app.copy_text.as_deref(), Some("line 15\nline 16\nline 17"));
    }
    #[test]
    fn command_blocks_span_the_command_line_to_the_next_prompt() {
        use super::term::{block_spans, BlockState::*};
        let marks = vec![
            mark('A', 9, None, None),
            mark('C', 11, None, None),
            mark('D', 15, Some(0), Some(5)),
            mark('A', 15, None, None),
            mark('C', 17, None, None),
            mark('D', 20, Some(2), Some(5)),
            mark('C', 22, None, None),
        ];
        assert_eq!(
            block_spans(&marks, 30),
            vec![(10, 15, Ok), (16, 20, Failed), (21, 30, Running)]
        );
        // A command whose end never arrived is dropped when a newer one starts.
        let lost = vec![mark('C', 3, None, None), mark('C', 8, None, None), mark('D', 9, None, None)];
        assert_eq!(block_spans(&lost, 12), vec![(7, 9, Ok)]);
        assert!(block_spans(&[], 12).is_empty());
    }
    #[test]
    fn panes_with_command_marks_render_with_the_gutter_on_and_off() {
        let ctx = egui::Context::default();
        let (mut app, _rx) = app_with_screen("$ make\r\nerror\r\n$ ");
        app.panes.get_mut(&1).unwrap().marks = vec![
            mark('C', 1, None, None),
            mark('D', 2, Some(2), Some(5)),
            mark('C', 3, None, None),
        ];
        for gutter in [true, false] {
            app.cfg.command_gutter = gutter;
            for width in [500., 1180.] {
                render(&mut app, &ctx, width);
            }
        }
    }
    #[test]
    fn a_reload_switches_theme_only_when_the_config_names_another() {
        assert!(!theme_changed(None, "nord"), "the first read is not a change");
        assert!(!theme_changed(Some("nord"), "nord"));
        assert!(theme_changed(Some("nord"), "dracula"));
    }
    #[test]
    fn the_last_command_can_be_copied_and_run_again() {
        let (mut app, rx) = app_with_screen("");
        app.copy_last_command();
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.contains("shell integration")));
        app.panes.get_mut(&1).unwrap().marks = vec![mark('C', 2, None, None)];
        app.copy_last_command();
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.contains("Update the shell integration")));
        let mut with_cmd = mark('C', 5, None, None);
        with_cmd.cmd = Some("make test".into());
        app.panes.get_mut(&1).unwrap().marks.push(with_cmd);
        app.copy_last_command();
        assert_eq!(app.copy_text.as_deref(), Some("make test"));
        while rx.try_recv().is_ok() {}
        app.rerun_last_command();
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::Input { pane: 1, data }) if data == b"make test\r"
        ));
        app.readonly = true;
        app.rerun_last_command();
        assert!(rx.try_recv().is_err(), "nothing is sent when read only");
    }
    #[test]
    fn copying_last_output_fetches_history_when_needed_and_explains_missing_integration() {
        let (mut app, rx) = app_with_screen("");
        app.copy_last_output();
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.contains("shell integration")));
        app.panes.get_mut(&1).unwrap().marks =
            vec![mark('C', 2, None, None), mark('D', 4, Some(0), Some(5))];
        app.copy_last_output();
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::Capture(1, CapturePurpose::Scrollback(1)))
        ));
        app.finish_scrollback(1, "a\nb\nc\nd\ne", 0);
        assert_eq!(app.copy_text.as_deref(), Some("c\nd"));
        assert!(
            app.scrollback.is_empty(),
            "the view opened only for this is closed again"
        );
        // Output that has scrolled out of the retained history is reported, not guessed.
        app.panes.get_mut(&1).unwrap().marks =
            vec![mark('C', 2, None, None), mark('D', 4, Some(0), None)];
        app.copy_last_output();
        let _ = rx.try_recv();
        app.finish_scrollback(1, "x\ny", 100);
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.contains("no longer retained")));
    }
    #[test]
    fn new_panes_start_in_the_focused_panes_working_directory() {
        let (mut app, rx) = app_with_screen("");
        app.panes.get_mut(&1).unwrap().info.cwd = "/srv/app".into();
        app.run_action(ActionKind::NewPane);
        assert!(matches!(rx.try_recv(), Ok(Command::New(Some(c))) if c == "/srv/app"));
        app.run_action(ActionKind::NewPaneBelow);
        assert!(matches!(rx.try_recv(), Ok(Command::New(Some(_)))));
    }
    #[test]
    fn triggers_highlight_and_fire_once_per_new_line_but_never_for_first_sight() {
        let (mut app, _rx) = app_with_screen("old ERROR line");
        let cfg = DesktopConfig {
            triggers: vec![
                crate::config::Trigger {
                    pattern: "ERROR".into(),
                    action: crate::config::TriggerAction::Highlight,
                    color: Some("#ff0000".into()),
                    unfocused_only: None,
                },
                crate::config::Trigger {
                    pattern: "BOOM".into(),
                    action: crate::config::TriggerAction::Toast,
                    color: None,
                    unfocused_only: None,
                },
            ],
            ..Default::default()
        };
        app.apply_config(cfg);
        app.scan_triggers(1);
        let spans = &app.panes[&1].spans;
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].0, spans[0].1, spans[0].2), (0, 4, 5));
        assert!(app.toast.is_none(), "first sight of the screen is silent");
        app.panes
            .get_mut(&1)
            .unwrap()
            .parser
            .process(b"\r\nBOOM now");
        app.scan_triggers(1);
        assert!(app.toast.as_ref().is_some_and(|(t, _)| t == "BOOM now"));
        app.toast = None;
        app.scan_triggers(1);
        assert!(app.toast.is_none(), "the same line does not fire twice");
    }
    #[test]
    fn a_floating_pane_renders_and_a_new_floating_pane_floats_when_created() {
        let (tx, _rx) = mpsc::sync_channel(8);
        let (utx, urx) = mpsc::sync_channel(8);
        let mut app = Desktop::new(
            "local".into(),
            "a".into(),
            false,
            Theme::load("nord").unwrap(),
            tx,
            urx,
        );
        app.online = true;
        app.add_test_pane(1);
        app.new_floating();
        assert!(app.pending_float);
        app.add_test_pane(2);
        utx.send(Update::Created(2)).unwrap();
        let ctx = egui::Context::default();
        render(&mut app, &ctx, 900.);
        assert_eq!(app.extra.floating, Some(2));
        assert_eq!(app.focus, 2);
        render(&mut app, &ctx, 900.);
        render(&mut app, &ctx, 420.);
        // Removing the other pane ends floating: a lone pane is tiled.
        app.panes.remove(&1);
        render(&mut app, &ctx, 900.);
        assert!(app.extra.floating.is_none());
    }
    #[test]
    fn restoring_a_layout_renames_the_first_pane_creates_the_rest_and_titles_them_as_they_appear() {
        let (mut app, rx) = app_with_screen("");
        app.panes.get_mut(&1).unwrap().info.title = "Shell".into();
        app.extra.restore_offer = Some(crate::layout::Layout {
            version: 1,
            panes: vec![
                crate::layout::LayoutPane {
                    title: "editor".into(),
                    cwd: "/a".into(),
                },
                crate::layout::LayoutPane {
                    title: "logs".into(),
                    cwd: "/var/log".into(),
                },
            ],
        });
        app.restore_layout();
        let sent = drain(&rx);
        assert!(matches!(&sent[0], Command::Rename(1, t) if t == "editor"));
        assert!(matches!(&sent[1], Command::New(Some(c)) if c == "/var/log"));
        assert!(app.extra.restore_offer.is_none());
        app.title_new_pane(2);
        assert!(matches!(&drain(&rx)[..], [Command::Rename(2, t)] if t == "logs"));
        app.title_new_pane(3);
        assert!(
            drain(&rx).is_empty(),
            "no queued title left for a pane the user opens later"
        );
    }
}

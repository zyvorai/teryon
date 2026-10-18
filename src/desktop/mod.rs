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
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

mod actions;
mod bar;
mod icons;
mod panes;
mod style;
mod term;
use actions::{actions, theme_counterpart, theme_step, ActionKind, Scroll, ARG_COMMANDS};
use icons::*;
use panes::{arrange, Dir, Weights};
use style::*;
use term::Scrollback;

struct Pane {
    info: PaneView,
    parser: vt100::Parser,
}
/// Everything that belongs to one attached workspace; parked tabs keep theirs here.
struct Session {
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
    name: String,
    state: Option<Session>,
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
    /// Text from the last `search`, tinted wherever it appears on the visible screens.
    highlight: String,
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
    config_checked: Instant,
    /// Window commands queued by actions, sent with the egui context on the next frame.
    pending: Vec<egui::ViewportCommand>,
    fullscreen: bool,
    on_top: bool,
    /// Message currently floating in the corner, and the last `notice` already turned into one.
    toast: Option<(String, Instant)>,
    toasted: String,
    window_title: String,
    latency: Option<u64>,
    /// Text queued for the clipboard; flushed with the egui context each frame.
    copy_text: Option<String>,
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

/// Built-in theme ids (as accepted by `--theme`) matching a theme's display name.
fn builtin_id(display_name: &str) -> Option<&'static str> {
    BUILTINS
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
        Self {
            host,
            tabs: vec![Tab {
                name: workspace.clone(),
                state: None,
            }],
            active: 0,
            target: None,
            weights: Weights::default(),
            pane_rects: BTreeMap::new(),
            unfocused_dim: 0.18,
            highlight: String::new(),
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
            config_checked: Instant::now(),
            pending: Vec::new(),
            fullscreen: false,
            on_top: false,
            toast: None,
            toasted: String::new(),
            window_title: String::new(),
            latency: None,
            copy_text: None,
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
            self.last_size = (0, 0);
            self.toasted = self.notice.clone();
        }
    }
    /// Let background tabs drain their updates so their workers never block.
    fn pump_background(&mut self) {
        let home = self.active;
        let notice = self.notice.clone();
        for i in (0..self.tabs.len()).filter(|i| *i != home) {
            self.activate(i);
            self.updates();
            self.activate(home);
        }
        self.notice = notice;
    }
    fn open_tab(&mut self, name: &str) {
        if let Some(i) = self.tabs.iter().position(|t| t.name == name) {
            self.activate(i);
            return;
        }
        let Some(target) = self.target.clone() else {
            self.notice = "Tabs are unavailable in this session".into();
            return;
        };
        if !valid_name(name) {
            self.notice = "Usage: tab NAME (letters, digits, - and _)".into();
            return;
        }
        let (tx, rx) = mpsc::sync_channel(64);
        let (utx, urx) = mpsc::sync_channel(32);
        let (ws, readonly) = (name.to_string(), self.readonly);
        thread::spawn(move || worker(target, ws, readonly, None, rx, utx, (30, 100)));
        self.tabs.push(Tab {
            name: name.into(),
            state: Some(Session {
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
    /// Detach the current tab (its shells keep running on the host).
    fn close_tab(&mut self) {
        if self.tabs.len() < 2 {
            self.notice = "This is the only tab".into();
            return;
        }
        let closing = self.active;
        let _ = self.tx.try_send(Command::Stop);
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
        if self.readonly && !matches!(c, Command::Capture(..)) {
            self.notice = "Read-only attachment".into();
            return;
        }
        if self.tx.try_send(c).is_err() {
            self.notice = "Action queue full; action not queued".into();
        }
    }
    fn input(&mut self, data: Vec<u8>) {
        self.scrollback.remove(&self.focus);
        if data.len() > MAX_INPUT {
            self.notice = "Paste too large; transfer a file instead".into();
            return;
        }
        self.send(Command::Input {
            pane: self.focus,
            data,
        });
    }
    fn updates(&mut self) {
        while let Ok(update) = self.rx.try_recv() {
            match update {
                Update::Online => {
                    self.online = true;
                    self.last_size = (0, 0);
                    self.notice = "Attached to persistent remote shells".into();
                }
                Update::Offline(s) => {
                    self.online = false;
                    self.notice = s;
                }
                Update::Message(s) => self.notice = s,
                Update::Latency(ms) => self.latency = Some(ms),
                Update::Created(id) => self.focus = id,
                Update::Captured(text, purpose) => match purpose {
                    CapturePurpose::View(query) => {
                        self.history = Some(text);
                        self.query = query;
                    }
                    CapturePurpose::Scrollback(id) => self.finish_scrollback(id, &text),
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
                    for view in views {
                        let pane = self.panes.entry(view.id).or_insert_with(|| Pane {
                            info: view.clone(),
                            parser: vt100::Parser::new(view.rows, view.cols, 0),
                        });
                        if let Some(bytes) = &view.screen {
                            pane.parser = vt100::Parser::new(view.rows, view.cols, 0);
                            pane.parser.process(bytes);
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
                }
            }
        }
    }
    fn execute(&mut self) {
        let command = std::mem::take(&mut self.command);
        let (name, arg) = command
            .trim()
            .split_once(' ')
            .unwrap_or((command.trim(), ""));
        match name{
        "new"=>self.send(Command::New(if arg.is_empty(){None}else{Some(arg.into())})),
        "rename"=>self.send(Command::Rename(self.focus,arg.into())),"close"=>self.confirm_close=true,
        "split"=>self.layout = if self.layout == LayoutMode::Single { LayoutMode::Vertical } else { LayoutMode::Single },
        "layout"=>match LayoutMode::parse(arg){Some(m)=>self.layout=m,None=>self.notice="Usage: layout single|vertical|horizontal|grid".into()},
        "theme"=>match Theme::load_native(arg){Ok(t)=>{self.committed_theme=t.clone();self.theme=t}Err(e)=>self.notice=e.to_string()},
        "search"=>{self.highlight=arg.to_lowercase();self.send(Command::Capture(self.focus,CapturePurpose::View(arg.into())))},
        "capture"=>self.send(Command::Capture(self.focus,CapturePurpose::View(String::new()))),
        "export"=>self.send(Command::Capture(self.focus,CapturePurpose::Export(arg.into()))),
        "draft"=>self.draft_open=true,
        "tab"=>self.open_tab(arg),"closetab"=>self.close_tab(),
        _=>self.notice="Commands: new [cwd], rename TITLE, close, split, layout NAME, theme NAME, search TEXT, capture, export /file, draft, tab NAME, closetab".into(),
    }
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        let terminal_focus =
            ctx.memory(|m| m.focused()) == Some(egui::Id::new(("teryon-terminal", self.focus)));
        if (ctx.wants_keyboard_input() && !terminal_focus)
            || self.palette
            || self.draft_open
            || self.history.is_some()
            || self.confirm_close
        {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::Text(text) => self.input(text.into_bytes()),
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
                        k if modifiers.ctrl => {
                            let name = k.name();
                            let mut chars = name.chars();
                            let Some(c) = chars.next() else {
                                continue;
                            };
                            if chars.next().is_some() {
                                continue;
                            }
                            crossterm::event::KeyCode::Char(c.to_ascii_lowercase())
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
        let previous_focus = self.focus;
        self.pump_background();
        self.updates();
        if self.focus != previous_focus {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new(("teryon-terminal", self.focus))));
        }
        if self.theme.name != self.committed_theme.name
            && !self.palette
            && !ctx.memory(|m| m.any_popup_open())
        {
            self.theme = self.committed_theme.clone();
        }
        let t = self.theme.clone();
        apply_style(ctx, &t);
        if let Some(text) = self.copy_text.take() {
            ctx.copy_text(text);
        }
        for cmd in self.pending.drain(..) {
            ctx.send_viewport_cmd(cmd);
        }
        if self.persist && self.applied_font.as_ref() != Some(&self.cfg.font_family) {
            install_fonts(ctx, self.cfg.font_family.as_deref());
            self.applied_font = Some(self.cfg.font_family.clone());
        }
        self.window_focused = ctx.input(|i| i.focused);
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
        self.top_bar(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(color(t.background).gamma_multiply(self.cfg.window_opacity))
                    .inner_margin(egui::Margin::same(4)),
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
                            self.find_cur = 0;
                        }
                        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            let back = ui.input(|i| i.modifiers.shift);
                            self.find_step(back);
                            edit.request_focus();
                        }
                        let n = self.find_count;
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
                    self.highlight = self.find_text.to_lowercase();
                }
                let ids: Vec<u64> = self.panes.keys().copied().collect();
                let shown = visible_ids(&ids, self.focus, self.layout);
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
                    let line = if handle.hovered() || handle.dragged() {
                        t.accent
                    } else {
                        t.border
                    };
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
                    let Some(out) = result else {
                        continue;
                    };
                    match out.action {
                        Some(PaneAction::Rename(pane)) => {
                            let title = self.panes.get(&pane).map(|p| p.info.title.clone());
                            self.renaming = title.map(|title| (pane, title));
                        }
                        Some(PaneAction::Zoom(pane)) => {
                            self.focus = pane;
                            self.run_action(ActionKind::Zoom);
                        }
                        Some(PaneAction::Close(pane)) => {
                            self.focus = pane;
                            self.confirm_close = true;
                        }
                        None => {}
                    }
                    if id == self.focus {
                        if self.online
                            && !self.readonly
                            && out.size != self.last_size
                            && self
                                .tx
                                .try_send(Command::Resize(out.size.0, out.size.1))
                                .is_ok()
                        {
                            self.last_size = out.size;
                        }
                    } else if out.clicked {
                        self.focus = id;
                    }
                }
                if self.panes.is_empty() {
                    ui.label("Connecting to your persistent workspace…");
                }
            });
        if self.palette {
            self.palette_ui(ctx);
        }
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
        if self.confirm_close {
            egui::Window::new("Terminate pane?")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("The pane process will be terminated. This cannot be undone.");
                    if ui.button("Terminate pane").clicked() {
                        self.send(Command::Close(self.focus));
                        self.confirm_close = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_close = false;
                    }
                });
        }
        self.keyboard(ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
impl Desktop {
    fn run_action(&mut self, kind: ActionKind) {
        self.palette = false;
        match kind {
            ActionKind::Palette => self.palette = true,
            ActionKind::NewPane => self.send(Command::New(None)),
            ActionKind::NewPaneRight => {
                if matches!(self.layout, LayoutMode::Single | LayoutMode::Vertical) {
                    self.layout = LayoutMode::Horizontal;
                }
                self.send(Command::New(None));
            }
            ActionKind::NewPaneBelow => {
                if matches!(self.layout, LayoutMode::Single | LayoutMode::Horizontal) {
                    self.layout = LayoutMode::Vertical;
                }
                self.send(Command::New(None));
            }
            ActionKind::Focus(dir) => self.focus_dir(dir),
            ActionKind::Resize(dir) => self.resize_focused(dir, 0.02),
            ActionKind::Equalize => self.weights.equalize(),
            ActionKind::NextPane => self.cycle_pane(false),
            ActionKind::PrevPane => self.cycle_pane(true),
            ActionKind::ClosePane => self.confirm_close = true,
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
            ActionKind::NewTab => {
                self.command = "tab ".into();
                self.palette = true;
            }
            ActionKind::CloseTab => self.close_tab(),
            ActionKind::Theme(name) => match Theme::load_native(name) {
                Ok(t) => {
                    self.committed_theme = t.clone();
                    self.theme = t;
                }
                Err(e) => self.notice = e.to_string(),
            },
            ActionKind::FindNext => self.find_step(false),
            ActionKind::FindPrev => self.find_step(true),
            ActionKind::TabJump(n) => self.activate(n),
            ActionKind::TabMove(d) => self.move_tab(d),
            ActionKind::NextTheme | ActionKind::PrevTheme | ActionKind::RandomTheme => {
                let now = builtin_id(&self.committed_theme.name).unwrap_or("mac27-dark");
                let next = match kind {
                    ActionKind::NextTheme => theme_step(now, 1),
                    ActionKind::PrevTheme => theme_step(now, -1),
                    _ => {
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.subsec_nanos() as usize);
                        BUILTINS[nanos % BUILTINS.len()]
                    }
                };
                self.run_action(ActionKind::Theme(next));
            }
            ActionKind::ThemeVariant => {
                let now = builtin_id(&self.committed_theme.name).unwrap_or("mac27-dark");
                match theme_counterpart(now) {
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
        }
    }
    fn palette_ui(&mut self, ctx: &egui::Context) {
        let list = actions(&self.keymap);
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
                ui.add_space(4.);
                for (row, (_, i)) in ranked.iter().enumerate() {
                    let action = &list[*i];
                    let selected = row == self.palette_sel && !typed;
                    let resp = ui.add(egui::SelectableLabel::new(selected, &action.title));
                    if !action.hint.is_empty() {
                        ui.painter().text(
                            resp.rect.right_center() - Vec2::new(8., 0.),
                            egui::Align2::RIGHT_CENTER,
                            action.hint.clone(),
                            FontId::proportional(11.),
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
        self.cfg = cfg;
        if let Some(first) = problems.first() {
            self.toast(format!("Config keys: {first}"));
        }
    }
    fn reload_config(&mut self) {
        match config::load_desktop() {
            Ok(cfg) => {
                self.apply_config(cfg);
                self.toast("Config reloaded");
            }
            Err(e) => self.toast(format!(
                "Config error: {e:#} (keeping the previous settings)"
            )),
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
        self.highlight.clear();
        self.find_count = 0;
    }
    fn move_tab(&mut self, delta: isize) {
        let to = self.active.checked_add_signed(delta);
        if let Some(to) = to.filter(|t| *t < self.tabs.len()) {
            self.tabs.swap(self.active, to);
            self.active = to;
        }
    }
    fn settings_snapshot(&self) -> DesktopSettings {
        DesktopSettings {
            width: Some(self.win_size.x.round()).filter(|w| *w > 0.),
            height: Some(self.win_size.y.round()).filter(|h| *h > 0.),
            theme: builtin_id(&self.committed_theme.name).map(str::to_string),
            font_size: Some(self.font_size),
            layout: Some(self.layout.name().to_string()),
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
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.save_settings(true);
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
fn merged_titlebar(builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    if cfg!(target_os = "macos") {
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
    match config::load_desktop() {
        Ok(cfg) => app.apply_config(cfg),
        Err(e) => app.toast(format!("Config error: {e:#}")),
    }
    app.persist = true;
    if let Some(mode) = settings.layout.as_deref().and_then(LayoutMode::parse) {
        app.layout = mode;
    }
    app.font_size = settings.font_size.unwrap_or(app.cfg.font_size);
    let transparent = app.cfg.window_opacity < 1.;
    let family = app.cfg.font_family.clone();
    app.applied_font = Some(family.clone());
    let (width, height) = (
        settings.width.unwrap_or(1180.),
        settings.height.unwrap_or(760.),
    );
    app.saved = settings;
    eframe::run_native(
        "TERYON — Your Mac mini. Wherever you work.",
        eframe::NativeOptions {
            viewport: merged_titlebar(egui::ViewportBuilder::default())
                .with_transparent(transparent)
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
                };
                app.panes.insert(1, Pane { info: view, parser });
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
                    };
                    let parser = vt100::Parser::new(24, 80, 0);
                    app.panes.insert(id, Pane { info: view, parser });
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
            name: "b".into(),
            state: Some(Session {
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
        app.run_action(ActionKind::Theme("dracula"));
        assert_eq!(app.theme.name, "Dracula");
        app.run_action(ActionKind::FontUp);
        assert_eq!(app.font_size, 14.);
        app.run_action(ActionKind::NewTab);
        assert!(app.palette && app.command == "tab ");
        app.command = "lay gr".into();
        render(&mut app, &ctx, 900.);
        assert!(actions(&app.keymap)
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
                name: name.into(),
                state: Some(Session {
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
        app.finish_scrollback(1, &text);
        assert_eq!(app.scrollback[&1].offset, 8);
        app.scroll_by(1, 1000);
        assert_eq!(app.scrollback[&1].offset, 90);
        app.scroll_by(1, -1000);
        assert!(!app.scrollback.contains_key(&1));
        app.cfg.scrollback_lines = 100;
        app.scroll_by(1, 5);
        app.finish_scrollback(1, &(0..500).map(|i| format!("{i}\n")).collect::<String>());
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
}

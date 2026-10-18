//! The terminal canvas: drawing, selection, mouse reporting, cursor styles and the scrollback view.
use super::*;
use crate::config::{CursorStyle, PaneTitles};
use crate::copymode::{CopyMode, Key as CmKey, Outcome};
use crate::keys::{mouse_report, MouseKind};
use vt100::MouseProtocolMode as MouseMode;

/// Retained history of one pane, shown instead of the live screen while scrolled back.
pub(super) struct Scrollback {
    pub lines: Vec<String>,
    /// Lines above the live bottom; 0 never happens for a stored view.
    pub offset: usize,
    /// Waiting for the daemon's capture; `queued` is how far the user already scrolled.
    pub loading: bool,
    pub queued: usize,
    /// Keyboard selection over `buf` (the same text as `lines`, one `char` per cell).
    pub copy: Option<CopyMode>,
    /// Start copy mode as soon as the text arrives.
    pub copy_pending: bool,
    pub buf: Vec<Vec<char>>,
}

/// Scroll the view so the copy-mode cursor stays on screen.
fn follow_cursor(sb: &mut Scrollback, rows: usize) {
    let Some(c) = &sb.copy else {
        return;
    };
    let total = sb.buf.len();
    let mut end = total.saturating_sub(sb.offset);
    let start = end.saturating_sub(rows);
    if c.row < start {
        end = c.row + rows;
    } else if c.row >= end {
        end = c.row + 1;
    }
    sb.offset = total.saturating_sub(end.min(total));
}

impl Desktop {
    /// Whether panes get a title bar of their own.
    fn show_titles(&self) -> bool {
        match self.cfg.pane_titles {
            PaneTitles::Always => true,
            PaneTitles::Never => false,
            PaneTitles::Auto => self.panes.len() > 1,
        }
    }
    pub(super) fn selection_text(&self, sel: &Selection) -> Option<String> {
        let screen = self.panes.get(&sel.pane)?.parser.screen();
        Some(if sel.block {
            uikit::selected_block(screen, sel.a, sel.b)
        } else {
            uikit::selected_text(screen, sel.a, sel.b)
        })
    }
    fn copy_selection(&mut self) {
        if let Some(text) = self.selection.and_then(|s| self.selection_text(&s)) {
            if !text.is_empty() {
                self.copy_text = Some(text);
            }
        }
    }
    /// Scroll a pane's history: positive `lines` go back in time, negative return towards the live screen.
    pub(super) fn scroll_by(&mut self, id: u64, lines: isize) {
        let rows = usize::from(self.view_rows.get(&id).copied().unwrap_or(24));
        if lines > 0 {
            let step = lines.unsigned_abs();
            match self.scrollback.get_mut(&id) {
                Some(sb) if sb.loading => sb.queued = sb.queued.saturating_add(step),
                Some(sb) => {
                    let max = sb.lines.len().saturating_sub(rows);
                    sb.offset = sb.offset.saturating_add(step).min(max);
                }
                None if !self.online => self.notice = "Offline: no history to scroll".into(),
                None => {
                    self.scrollback.insert(
                        id,
                        Scrollback {
                            lines: Vec::new(),
                            offset: 0,
                            loading: true,
                            queued: step,
                            copy: None,
                            copy_pending: false,
                            buf: Vec::new(),
                        },
                    );
                    self.send(Command::Capture(id, CapturePurpose::Scrollback(id)));
                }
            }
        } else if let Some(sb) = self.scrollback.get_mut(&id) {
            if !sb.loading {
                sb.offset = sb.offset.saturating_sub(lines.unsigned_abs());
                if sb.offset == 0 && sb.copy.is_none() {
                    self.scrollback.remove(&id);
                }
            }
        }
    }
    pub(super) fn scroll_action(&mut self, s: Scroll) {
        let id = self.focus;
        let page = usize::from(self.view_rows.get(&id).copied().unwrap_or(24))
            .saturating_sub(1)
            .max(1);
        let page = isize::try_from(page).unwrap_or(1);
        match s {
            Scroll::PageUp => self.scroll_by(id, page),
            Scroll::PageDown => self.scroll_by(id, -page),
            Scroll::Top => self.scroll_by(id, isize::MAX / 2),
            Scroll::Bottom => {
                self.scrollback.remove(&id);
            }
        }
    }
    /// The daemon's text arrived: fill in the view the user already started scrolling.
    pub(super) fn finish_scrollback(&mut self, id: u64, text: &str) {
        let rows = usize::from(self.view_rows.get(&id).copied().unwrap_or(24));
        let Some(sb) = self.scrollback.get_mut(&id) else {
            return;
        };
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let keep = self.cfg.scrollback_lines + rows;
        if lines.len() > keep {
            lines.drain(..lines.len() - keep);
        }
        let max = lines.len().saturating_sub(rows);
        sb.offset = sb.queued.min(max);
        sb.lines = lines;
        sb.loading = false;
        if sb.copy_pending {
            sb.copy_pending = false;
            sb.buf = sb.lines.iter().map(|l| l.chars().collect()).collect();
            let last = sb.buf.len().saturating_sub(1);
            sb.copy = Some(CopyMode::new(last, 0, rows));
            sb.offset = 0;
        } else if sb.offset == 0 {
            self.scrollback.remove(&id);
        }
    }
    /// Enter vi-style copy mode over the focused pane's history and screen.
    pub(super) fn start_copy_mode(&mut self) {
        let id = self.focus;
        let Some(pane) = self.panes.get(&id) else {
            return;
        };
        if pane.parser.screen().alternate_screen() {
            self.toast("Copy mode works on the normal screen; use ⌥-drag in full-screen programs");
            return;
        }
        let rows = usize::from(self.view_rows.get(&id).copied().unwrap_or(24));
        match self.scrollback.get_mut(&id) {
            Some(sb) if !sb.loading => {
                sb.buf = sb.lines.iter().map(|l| l.chars().collect()).collect();
                let end = sb.lines.len().saturating_sub(sb.offset);
                sb.copy = Some(CopyMode::new(end.saturating_sub(1), 0, rows));
            }
            Some(sb) => sb.copy_pending = true,
            None if !self.online => self.toast("Offline: no history to copy from"),
            None => {
                self.scrollback.insert(
                    id,
                    Scrollback {
                        lines: Vec::new(),
                        offset: 0,
                        loading: true,
                        queued: 0,
                        copy: None,
                        copy_pending: true,
                        buf: Vec::new(),
                    },
                );
                self.send(Command::Capture(id, CapturePurpose::Scrollback(id)));
            }
        }
    }
    pub(super) fn copy_active(&self) -> bool {
        self.scrollback
            .get(&self.focus)
            .is_some_and(|s| !s.loading && s.copy.is_some())
    }
    /// Keys while copy mode is on; ⌘ shortcuts are left to the normal handling.
    pub(super) fn copy_event(&mut self, event: &egui::Event) -> bool {
        let keys: Vec<CmKey> = match event {
            egui::Event::Text(t) => t.chars().map(CmKey::Char).collect(),
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if !modifiers.command => match key {
                Key::Escape => vec![CmKey::Esc],
                Key::Enter => vec![CmKey::Enter],
                Key::Backspace => vec![CmKey::Backspace],
                Key::ArrowLeft => vec![CmKey::Char('h')],
                Key::ArrowRight => vec![CmKey::Char('l')],
                Key::ArrowUp => vec![CmKey::Char('k')],
                Key::ArrowDown => vec![CmKey::Char('j')],
                Key::PageUp => vec![CmKey::Ctrl('b')],
                Key::PageDown => vec![CmKey::Ctrl('f')],
                k if modifiers.ctrl => k
                    .name()
                    .chars()
                    .next()
                    .map(|c| CmKey::Ctrl(c.to_ascii_lowercase()))
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            },
            _ => return false,
        };
        for key in keys {
            if !self.copy_key(key) {
                break;
            }
        }
        true
    }
    /// One key into copy mode; false once copy mode has ended.
    fn copy_key(&mut self, key: CmKey) -> bool {
        let id = self.focus;
        let rows = usize::from(self.view_rows.get(&id).copied().unwrap_or(24));
        let Some(sb) = self.scrollback.get_mut(&id) else {
            return false;
        };
        let Some(mut cm) = sb.copy.take() else {
            return false;
        };
        match cm.key(key, &sb.buf) {
            Outcome::None => {
                sb.copy = Some(cm);
                follow_cursor(sb, rows);
                true
            }
            Outcome::Exit => {
                self.scrollback.remove(&id);
                false
            }
            Outcome::Yank(text) => {
                self.scrollback.remove(&id);
                let n = text.lines().count();
                self.copy_text = Some(text);
                self.toast(format!("Copied {n} line(s)"));
                false
            }
        }
    }
    /// Title bar above a pane: name (double-click renames), directory, status, zoom and close.
    pub(super) fn pane_bar(
        &mut self,
        ui: &mut egui::Ui,
        bar: Rect,
        id: u64,
        info: (&str, &str, Option<u32>),
    ) -> Option<PaneAction> {
        let (title, cwd, exited) = info;
        let t = self.theme.clone();
        let focused = id == self.focus;
        ui.painter().rect_filled(
            bar,
            egui::CornerRadius {
                nw: 6,
                ne: 6,
                sw: 0,
                se: 0,
            },
            color(mix(t.panel, t.background, if focused { 0. } else { 0.4 })),
        );
        if focused {
            ui.painter().line_segment(
                [bar.left_top(), bar.right_top()],
                Stroke::new(2_f32, color(t.accent)),
            );
        }
        let inner = bar.shrink2(Vec2::new(6., 0.));
        let mut out = None;
        let mut icons = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if icon_button(&mut icons, &t, Icon::Close, "Terminate pane").clicked() {
            out = Some(PaneAction::Close(id));
        }
        if icon_button(&mut icons, &t, Icon::Zoom, "Zoom / restore").clicked() {
            out = Some(PaneAction::Zoom(id));
        }
        let mut left = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if self.renaming.as_ref().is_some_and(|(r, _)| *r == id) {
            if let Some((_, text)) = self.renaming.as_mut() {
                let edit = left.add(egui::TextEdit::singleline(text).desired_width(150.));
                edit.request_focus();
                if edit.lost_focus() {
                    let enter = left.input(|i| i.key_pressed(Key::Enter));
                    if let (true, Some((pane, name))) = (enter, self.renaming.take()) {
                        self.send(Command::Rename(pane, name));
                    } else {
                        self.renaming = None;
                    }
                }
            }
        } else {
            let label = left.add(
                egui::Label::new(
                    weighted(ui.ctx(), format!("{id}  {title}"), "semibold", 12.)
                        .color(color(t.foreground)),
                )
                .sense(Sense::click()),
            );
            if label.double_clicked() {
                out = Some(PaneAction::Rename(id));
            } else if label.clicked() {
                self.focus = id;
            }
        }
        let tail: Vec<&str> = cwd.rsplit('/').filter(|c| !c.is_empty()).take(2).collect();
        if !tail.is_empty() {
            let short: Vec<&str> = tail.into_iter().rev().collect();
            left.label(
                RichText::new(short.join("/"))
                    .size(11.)
                    .color(color(t.muted)),
            );
        }
        if let Some(code) = exited {
            left.label(
                RichText::new(format!("exit {code}"))
                    .size(11.)
                    .color(color(t.warning)),
            );
        }
        out
    }
    fn send_to(&mut self, pane: u64, data: Vec<u8>) {
        if !data.is_empty() && data.len() <= MAX_INPUT {
            self.send(Command::Input { pane, data });
        }
    }
    pub(super) fn draw_terminal(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        height: f32,
    ) -> Option<TermOut> {
        let (title, cwd, exited) = {
            let p = self.panes.get(&id)?;
            (p.info.title.clone(), p.info.cwd.clone(), p.info.exited)
        };
        let t = self.theme.clone();
        let width = ui.available_width();
        let bar_h = if self.show_titles() { 24. } else { 0. };
        let (full, _) = ui.allocate_exact_size(Vec2::new(width, height.max(50.)), Sense::hover());
        let rect = Rect::from_min_max(full.min + Vec2::new(0., bar_h), full.max);
        let action = if bar_h > 0. {
            let bar = Rect::from_min_size(full.min, Vec2::new(width, bar_h));
            self.pane_bar(ui, bar, id, (&title, &cwd, exited))
        } else {
            None
        };
        let response = ui.interact(
            rect,
            egui::Id::new(("teryon-terminal", id)),
            Sense::click_and_drag(),
        );
        let font = FontId::monospace(self.font_size);
        let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
        let ch = self.font_size * self.cfg.line_height;
        let pad = self.cfg.padding;
        let rows = ((rect.height() - 2. * pad) / ch).floor().clamp(2., 200.) as u16;
        let cols = ((width - 2. * pad) / cw).floor().clamp(10., 500.) as u16;
        self.view_rows.insert(id, rows);
        let cell_at = move |p: Pos2| -> (u16, u16) {
            (
                ((p.y - rect.top() - pad) / ch)
                    .floor()
                    .clamp(0., f32::from(rows) - 1.) as u16,
                ((p.x - rect.left() - pad) / cw)
                    .floor()
                    .clamp(0., f32::from(cols) - 1.) as u16,
            )
        };
        let (mode, encoding, alt_screen, app_cursor) = {
            let sc = self.panes.get(&id)?.parser.screen();
            (
                sc.mouse_protocol_mode(),
                sc.mouse_protocol_encoding(),
                sc.alternate_screen(),
                sc.application_cursor(),
            )
        };
        let mods = ui.input(|i| i.modifiers);
        let command_held = mods.command;
        let scrolled = self
            .scrollback
            .get(&id)
            .is_some_and(|s| !s.loading && (s.offset > 0 || s.copy.is_some()));
        let reporting = mode != MouseMode::None && !mods.shift && !scrolled;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = pointer.is_some_and(|p| rect.contains(p));
        let row_chars = |screen: &vt100::Screen, row: u16| -> Vec<char> {
            (0..cols)
                .map(|c| {
                    screen
                        .cell(row, c)
                        .and_then(|cell| cell.contents().chars().next())
                        .unwrap_or(' ')
                })
                .collect()
        };
        // Mouse and wheel: forward to programs that ask for it, otherwise scroll history.
        let events = ui.input(|i| i.events.clone());
        let report = |kind, button, p: Pos2| {
            let (r, c) = cell_at(p);
            mouse_report(
                mode,
                encoding,
                kind,
                button,
                (mods.shift, mods.alt, mods.ctrl),
                r,
                c,
            )
        };
        let mut out_bytes: Vec<u8> = Vec::new();
        let mut scroll_lines: isize = 0;
        for event in &events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let b = match button {
                        egui::PointerButton::Primary => 0,
                        egui::PointerButton::Middle => 1,
                        egui::PointerButton::Secondary => 2,
                        _ => continue,
                    };
                    if *pressed && rect.contains(*pos) && reporting {
                        self.mouse_down = Some((id, b));
                        self.last_cell = Some(cell_at(*pos));
                        out_bytes
                            .extend(report(MouseKind::Press, Some(b), *pos).unwrap_or_default());
                    } else if !*pressed && self.mouse_down == Some((id, b)) {
                        self.mouse_down = None;
                        out_bytes
                            .extend(report(MouseKind::Release, Some(b), *pos).unwrap_or_default());
                    }
                }
                egui::Event::PointerMoved(pos) if reporting => {
                    let cell = cell_at(*pos);
                    let held = self.mouse_down.filter(|(p, _)| *p == id).map(|(_, b)| b);
                    if (held.is_some() || (mode == MouseMode::AnyMotion && rect.contains(*pos)))
                        && self.last_cell != Some(cell)
                    {
                        self.last_cell = Some(cell);
                        out_bytes.extend(report(MouseKind::Motion, held, *pos).unwrap_or_default());
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } if over => {
                    let lines = match unit {
                        egui::MouseWheelUnit::Point => delta.y / ch,
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * f32::from(rows),
                    };
                    self.wheel_accum += lines;
                    let steps = self.wheel_accum.trunc();
                    self.wheel_accum -= steps;
                    let steps = steps as isize;
                    if steps == 0 {
                        continue;
                    }
                    let at = pointer.unwrap_or(rect.center());
                    if reporting {
                        let kind = if steps > 0 {
                            MouseKind::WheelUp
                        } else {
                            MouseKind::WheelDown
                        };
                        for _ in 0..steps.unsigned_abs() {
                            out_bytes.extend(report(kind, None, at).unwrap_or_default());
                        }
                    } else if alt_screen && mode == MouseMode::None && !scrolled {
                        let arrow = match (steps > 0, app_cursor) {
                            (true, true) => &b"\x1bOA"[..],
                            (true, false) => b"\x1b[A",
                            (false, true) => b"\x1bOB",
                            (false, false) => b"\x1b[B",
                        };
                        for _ in 0..steps.unsigned_abs() * 3 {
                            out_bytes.extend_from_slice(arrow);
                        }
                    } else if !alt_screen || scrolled {
                        scroll_lines += steps * 3;
                    }
                }
                _ => {}
            }
        }
        self.send_to(id, out_bytes);
        if scroll_lines != 0 {
            self.scroll_by(id, scroll_lines);
        }
        // Local selection: drag, shift-click extend, double-click word, triple-click line, ⌥-drag block.
        let mut link: Option<(u16, usize, usize, String)> = None;
        if let Some(pos) = response.hover_pos() {
            let (row, col) = cell_at(pos);
            if let Some(pane) = self.panes.get(&id) {
                let chars = row_chars(pane.parser.screen(), row);
                if let Some((url, start, end)) = uikit::url_at(&chars, usize::from(col)) {
                    link = Some((row, start, end, url));
                }
            }
        }
        if reporting {
            if response.clicked() || response.drag_started() {
                response.request_focus();
            }
        } else if !scrolled {
            if response.drag_started() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let cell = cell_at(pos);
                    self.selection = Some(Selection {
                        pane: id,
                        a: cell,
                        b: cell,
                        block: mods.alt,
                    });
                    response.request_focus();
                }
            }
            if response.dragged() {
                if let (Some(pos), Some(sel)) =
                    (response.interact_pointer_pos(), self.selection.as_mut())
                {
                    if sel.pane == id {
                        sel.b = cell_at(pos);
                    }
                }
            }
            if response.drag_stopped() && self.cfg.copy_on_select {
                self.copy_selection();
            }
            let click_cell = response.interact_pointer_pos().map(cell_at);
            if response.triple_clicked() {
                if let Some((row, _)) = click_cell {
                    self.selection = Some(Selection {
                        pane: id,
                        a: (row, 0),
                        b: (row, cols - 1),
                        block: false,
                    });
                    if self.cfg.copy_on_select {
                        self.copy_selection();
                    }
                }
            } else if response.double_clicked() {
                if let (Some((row, col)), Some(pane)) = (click_cell, self.panes.get(&id)) {
                    let chars = row_chars(pane.parser.screen(), row);
                    let (s, e) = uikit::word_bounds(&chars, usize::from(col));
                    self.selection = Some(Selection {
                        pane: id,
                        a: (row, s as u16),
                        b: (row, e as u16),
                        block: false,
                    });
                    if self.cfg.copy_on_select {
                        self.copy_selection();
                    }
                }
            } else if response.clicked() {
                response.request_focus();
                match (&link, command_held, click_cell, self.selection) {
                    (Some((.., url)), true, ..) => uikit::open_url(url),
                    (_, _, Some(cell), Some(sel)) if mods.shift && sel.pane == id => {
                        self.selection = Some(Selection { b: cell, ..sel });
                    }
                    _ => self.selection = None,
                }
            }
        } else if response.clicked() {
            response.request_focus();
        }
        if command_held && link.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let painter = ui.painter_at(full);
        let opacity = self.cfg.window_opacity;
        let bg_default = color(t.background);
        painter.rect_filled(
            rect,
            if bar_h > 0. { 0. } else { 6. },
            bg_default.gamma_multiply(opacity),
        );
        if bar_h > 0. && id == self.focus {
            painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                Stroke::new(1_f32, color(t.accent).gamma_multiply(0.5)),
            );
        }
        if let Some(sb) = self
            .scrollback
            .get(&id)
            .filter(|s| !s.loading && (s.offset > 0 || s.copy.is_some()))
        {
            // Plain-text history (colors are not retained by the daemon).
            let total = sb.lines.len();
            let end = total.saturating_sub(sb.offset);
            let start = end.saturating_sub(usize::from(rows));
            for (i, line) in sb.lines[start..end].iter().enumerate() {
                let row = start + i;
                let y = rect.top() + pad + i as f32 * ch;
                if let Some(cm) = &sb.copy {
                    for col in 0..usize::from(cols) {
                        if cm.selected(row, col) {
                            painter.rect_filled(
                                Rect::from_min_size(
                                    Pos2::new(rect.left() + pad + col as f32 * cw, y),
                                    Vec2::new(cw, ch),
                                ),
                                0.,
                                color(t.accent).gamma_multiply(0.4),
                            );
                        }
                    }
                }
                let text: String = line.chars().take(usize::from(cols)).collect();
                painter.text(
                    Pos2::new(rect.left() + pad, y),
                    egui::Align2::LEFT_TOP,
                    text,
                    font.clone(),
                    color(t.foreground),
                );
                if let Some(cm) = sb.copy.as_ref().filter(|c| c.row == row) {
                    let x = rect.left() + pad + cm.col as f32 * cw;
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y), Vec2::new(cw, ch)),
                        0.,
                        color(t.accent),
                    );
                    if let Some(c) = line.chars().nth(cm.col).filter(|c| *c != ' ') {
                        painter.text(
                            Pos2::new(x, y),
                            egui::Align2::LEFT_TOP,
                            c,
                            font.clone(),
                            bg_default,
                        );
                    }
                }
            }
            let track = Rect::from_min_max(
                Pos2::new(rect.right() - 6., rect.top() + 4.),
                Pos2::new(rect.right() - 3., rect.bottom() - 4.),
            );
            let frac = f32::from(rows) / total.max(1) as f32;
            let top = start as f32 / total.max(1) as f32;
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(track.left(), track.top() + track.height() * top),
                    Vec2::new(track.width(), (track.height() * frac).max(12.)),
                ),
                2.,
                color(t.muted).gamma_multiply(0.6),
            );
            let label = match sb.copy.as_ref() {
                Some(cm) => match &cm.typing {
                    Some((text, fwd)) => format!("{}{text}▏", if *fwd { "/" } else { "?" }),
                    None => "copy mode · v V ^V select · y copy · / find · q quit".to_string(),
                },
                None => format!("history · {} lines up", sb.offset),
            };
            let pill = Rect::from_min_size(
                Pos2::new(rect.right() - 330., rect.top() + 6.),
                Vec2::new(316., 20.),
            );
            painter.rect_filled(pill, 10., color(mix(t.panel, t.foreground, 0.1)));
            painter.text(
                pill.center(),
                egui::Align2::CENTER_CENTER,
                label,
                FontId::proportional(11.),
                color(t.muted),
            );
            self.dim_if_unfocused(&painter, rect, id, &t);
            return Some(TermOut {
                size: (rows, cols),
                clicked: response.clicked(),
                action,
            });
        }
        let pane = self.panes.get(&id)?;
        let screen = pane.parser.screen();
        let mut starts: Vec<(u16, usize, usize)> = Vec::new();
        if id == self.focus {
            if let Some(q) = &self.find_query {
                for row in 0..rows.min(pane.info.rows) {
                    let chars = row_chars(screen, row.min(pane.info.rows - 1));
                    let width = chars.len().min(usize::from(pane.info.cols));
                    starts.extend(
                        q.matches(&chars[..width])
                            .into_iter()
                            .map(|(c, len)| (row, c, len)),
                    );
                }
                self.find_count = starts.len();
            }
        }
        let current = (!starts.is_empty()).then(|| starts[self.find_cur % starts.len()]);
        let selection = self.selection.filter(|s| s.pane == id);
        for row in 0..rows.min(pane.info.rows) {
            for col in 0..cols.min(pane.info.cols) {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut fg = vt_color(cell.fgcolor(), color(t.foreground), &t);
                let mut bg = vt_color(cell.bgcolor(), bg_default, &t);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let pos = Pos2::new(
                    rect.left() + pad + f32::from(col) * cw,
                    rect.top() + pad + f32::from(row) * ch,
                );
                let cell_rect = Rect::from_min_size(
                    pos,
                    Vec2::new(cw * if cell.is_wide() { 2. } else { 1. }, ch),
                );
                if bg != bg_default {
                    painter.rect_filled(cell_rect, 0., bg);
                }
                if let Some(&(r, c, len)) = starts.iter().find(|(r, c, len)| {
                    *r == row && usize::from(col) >= *c && usize::from(col) < *c + *len
                }) {
                    let strong = current == Some((r, c, len));
                    painter.rect_filled(
                        cell_rect,
                        0.,
                        color(t.warning).gamma_multiply(if strong { 0.85 } else { 0.4 }),
                    );
                }
                if selection.is_some_and(|s| s.contains(row, col)) {
                    painter.rect_filled(cell_rect, 0., color(t.accent).gamma_multiply(0.4));
                }
                let text = cell.contents();
                if !text.is_empty() {
                    painter.text(pos, egui::Align2::LEFT_TOP, text, font.clone(), fg);
                    if cell.bold() {
                        painter.text(
                            pos + Vec2::new(0.5, 0.),
                            egui::Align2::LEFT_TOP,
                            text,
                            font.clone(),
                            fg,
                        );
                    }
                }
                let linked = command_held
                    && link.as_ref().is_some_and(|(lr, s, e, _)| {
                        *lr == row && usize::from(col) >= *s && usize::from(col) < *e
                    });
                if cell.underline() || linked {
                    painter.line_segment(
                        [cell_rect.left_bottom(), cell_rect.right_bottom()],
                        Stroke::new(1_f32, if linked { color(t.accent) } else { fg }),
                    );
                }
            }
        }
        if !screen.hide_cursor() {
            let (r, c) = screen.cursor_position();
            if r < rows && c < cols {
                let active = self.focus == id && self.window_focused;
                let blink_on = !self.cfg.cursor_blink
                    || !active
                    || (ui.input(|i| i.time) * 2.) as i64 % 2 == 0;
                if self.cfg.cursor_blink && active {
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }
                let pos = Pos2::new(
                    rect.left() + pad + f32::from(c) * cw,
                    rect.top() + pad + f32::from(r) * ch,
                );
                let wide = screen.cell(r, c).is_some_and(|cell| cell.is_wide());
                let cell_rect =
                    Rect::from_min_size(pos, Vec2::new(cw * if wide { 2. } else { 1. }, ch));
                let ink = color(if self.online { t.accent } else { t.warning });
                if !active {
                    painter.rect_stroke(
                        cell_rect,
                        0.,
                        Stroke::new(1_f32, ink),
                        egui::StrokeKind::Inside,
                    );
                } else if blink_on {
                    match self.cfg.cursor_style {
                        CursorStyle::Block => {
                            painter.rect_filled(cell_rect, 0., ink);
                            if let Some(cell) = screen.cell(r, c).filter(|c| c.has_contents()) {
                                painter.text(
                                    pos,
                                    egui::Align2::LEFT_TOP,
                                    cell.contents(),
                                    font.clone(),
                                    bg_default,
                                );
                            };
                        }
                        CursorStyle::Bar => {
                            painter.rect_filled(
                                Rect::from_min_size(pos, Vec2::new(2., ch)),
                                0.,
                                ink,
                            );
                        }
                        CursorStyle::Underline => {
                            painter.rect_filled(
                                Rect::from_min_size(
                                    pos + Vec2::new(0., ch - 2.),
                                    Vec2::new(cell_rect.width(), 2.),
                                ),
                                0.,
                                ink,
                            );
                        }
                    }
                }
            }
        }
        if let Some(h) = self.hints.as_ref().filter(|h| h.pane == id) {
            for item in h.items.iter().filter(|i| i.label.starts_with(&h.typed)) {
                let x = rect.left() + pad + item.hint.start as f32 * cw;
                let y = rect.top() + pad + f32::from(item.row) * ch;
                let span = Rect::from_min_size(
                    Pos2::new(x, y),
                    Vec2::new((item.hint.end - item.hint.start) as f32 * cw, ch),
                );
                painter.rect_filled(span, 2., color(t.accent).gamma_multiply(0.25));
                let tag = Rect::from_min_size(
                    Pos2::new(x, y),
                    Vec2::new(item.label.chars().count() as f32 * cw + 6., ch),
                );
                painter.rect_filled(tag, 3., color(t.warning));
                painter.text(
                    tag.center(),
                    egui::Align2::CENTER_CENTER,
                    &item.label,
                    font.clone(),
                    bg_default,
                );
            }
            let pill = Rect::from_min_size(
                Pos2::new(rect.right() - 300., rect.top() + 6.),
                Vec2::new(286., 20.),
            );
            painter.rect_filled(pill, 10., color(mix(t.panel, t.foreground, 0.12)));
            painter.text(
                pill.center(),
                egui::Align2::CENTER_CENTER,
                format!(
                    "quick select · {} · Tab switches · Esc cancels",
                    h.act.name()
                ),
                FontId::proportional(11.),
                color(t.foreground),
            );
        }
        if self.broadcast && self.pane_rects.contains_key(&id) {
            painter.rect_stroke(
                rect.shrink(1.),
                0.,
                Stroke::new(2_f32, color(t.warning)),
                egui::StrokeKind::Inside,
            );
        }
        self.dim_if_unfocused(&painter, rect, id, &t);
        Some(TermOut {
            size: (rows, cols),
            clicked: response.clicked(),
            action,
        })
    }
    fn dim_if_unfocused(&self, painter: &egui::Painter, rect: Rect, id: u64, t: &Theme) {
        if id != self.focus && self.pane_rects.len() > 1 && self.unfocused_dim > 0. {
            painter.rect_filled(
                rect,
                0.,
                color(t.background).gamma_multiply(self.unfocused_dim),
            );
        }
    }
}

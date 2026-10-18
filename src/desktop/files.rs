//! A Finder-like file sidebar (⌘B): the focused pane's directory as a list, with Quick Look (Space).
//! Read-only: it lists and previews, and hands paths to the pane; it never changes files.
use super::*;
use crate::files::{self as fs, Entry, Preview};

/// Width of the sidebar.
const WIDTH: f32 = 270.;
/// Directory listings are reused for this long before the next automatic refresh.
const REFRESH: Duration = Duration::from_secs(4);

/// What Quick Look has to show.
#[derive(Clone)]
enum Shown {
    Doc(Preview),
    Image {
        width: usize,
        height: usize,
        rgba: std::sync::Arc<Vec<u8>>,
    },
}

/// A question the sidebar asks before changing something.
enum Dialog {
    Rename {
        name: String,
        text: String,
        focus: bool,
    },
    Trash {
        name: String,
        dir: bool,
    },
}

enum Msg {
    Listed(String, Result<Vec<Entry>, String>),
    Previewed(String, Result<Shown, String>),
    /// A rename, trash or download finished: what to tell the user, and whether to relist.
    Done(String, bool),
}

/// Largest image Quick Look reads.
const IMAGE_BYTES: usize = 8 * 1024 * 1024;

fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

pub(super) struct Browser {
    pub(super) open: bool,
    /// Directory shown; follows the focused pane unless the user navigated away.
    dir: String,
    follow: bool,
    entries: Vec<Entry>,
    error: Option<String>,
    loading: bool,
    listed_at: Instant,
    filter: String,
    hidden: bool,
    selected: Option<String>,
    /// Quick Look: file name and what was read, `None` while loading.
    look: Option<(String, Option<Result<Shown, String>>)>,
    /// The decoded image of the open Quick Look, with the name it belongs to.
    look_tex: Option<(String, egui::TextureHandle)>,
    dialog: Option<Dialog>,
    /// A row had keyboard focus last frame, so keys belong to the sidebar and not the pane.
    pub(super) has_focus: bool,
    tx: mpsc::Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Browser {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            open: false,
            dir: String::new(),
            follow: true,
            entries: Vec::new(),
            error: None,
            loading: false,
            listed_at: Instant::now(),
            filter: String::new(),
            hidden: false,
            selected: None,
            look: None,
            look_tex: None,
            dialog: None,
            has_focus: false,
            tx,
            rx,
        }
    }
}

/// Decode an image and shrink it so a big photo doesn't become a huge texture.
fn decode(bytes: &[u8]) -> Result<Shown, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("can't read this image: {e}"))?;
    let img = img.thumbnail(1400, 1400).to_rgba8();
    Ok(Shown::Image {
        width: img.width() as usize,
        height: img.height() as usize,
        rgba: std::sync::Arc::new(img.into_raw()),
    })
}

/// `'quoted path'` ready to type at a shell prompt.
fn quoted(path: &str) -> String {
    crate::util::shell_quote(path)
}

impl Desktop {
    /// Keys belong to the sidebar (not the pane) while a row has focus or a question is open.
    pub(super) fn files_capture_keys(&self) -> bool {
        self.files.has_focus || self.files.dialog.is_some()
    }

    pub(super) fn files_toggle(&mut self) {
        self.files.open = !self.files.open;
        if !self.files.open {
            self.files.has_focus = false;
            self.files.look = None;
            self.files.dialog = None;
        } else {
            self.files.follow = true;
        }
    }

    fn files_list(&mut self, dir: &str) {
        let Some(target) = self.target.clone() else {
            self.files.error = Some("Not connected".into());
            return;
        };
        self.files.dir = dir.to_string();
        self.files.loading = true;
        self.files.listed_at = Instant::now();
        let (tx, dir) = (self.files.tx.clone(), dir.to_string());
        std::thread::spawn(move || {
            let result = fs::list(&target, &dir).map_err(|e| e.to_string());
            let _ = tx.send(Msg::Listed(dir, result));
        });
    }

    fn files_look(&mut self, name: &str) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let path = fs::join(&self.files.dir, name);
        self.files.look = Some((name.to_string(), None));
        let tx = self.files.tx.clone();
        std::thread::spawn(move || {
            let result = if is_image(&path) {
                fs::read_bytes(&target, &path, IMAGE_BYTES)
                    .map_err(|e| e.to_string())
                    .and_then(|b| decode(&b))
            } else {
                fs::read_head(&target, &path)
                    .map(|b| Shown::Doc(fs::classify(&b)))
                    .map_err(|e| e.to_string())
            };
            let _ = tx.send(Msg::Previewed(path, result));
        });
    }

    /// Run a change on a worker thread and report back with a toast and a fresh listing.
    fn files_do(
        &mut self,
        what: &'static str,
        job: impl FnOnce(&crate::transport::Target) -> anyhow::Result<()> + Send + 'static,
    ) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let tx = self.files.tx.clone();
        std::thread::spawn(move || {
            let msg = match job(&target) {
                Ok(()) => (format!("{what} done"), true),
                Err(e) => (format!("{what} failed: {e}"), false),
            };
            let _ = tx.send(Msg::Done(msg.0, msg.1));
        });
    }

    /// Copy a file or folder from an SSH host into ~/Downloads, never replacing what is there.
    fn files_download(&mut self, name: &str) {
        let (dir, name) = (self.files.dir.clone(), name.to_string());
        self.toast(format!("Downloading {name}…"));
        self.files_do("Download", move |target| {
            let dest = crate::util::home()?.join("Downloads");
            fs::download(target, &dir, &name, &dest)
        });
    }

    /// Type a quoted path (or `cd` line) at the focused pane's prompt without pressing Enter.
    fn files_insert(&mut self, text: &str) {
        let bracketed = self
            .panes
            .get(&self.focus)
            .is_some_and(|p| p.parser.screen().bracketed_paste());
        self.input(keys::paste(text, bracketed));
    }

    fn files_pump(&mut self) {
        while let Ok(msg) = self.files.rx.try_recv() {
            match msg {
                Msg::Listed(dir, result) if dir == self.files.dir => {
                    self.files.loading = false;
                    match result {
                        Ok(entries) => {
                            self.files.entries = fs::arrange(entries, true);
                            self.files.error = None;
                        }
                        Err(e) => {
                            self.files.entries.clear();
                            self.files.error = Some(e);
                        }
                    }
                }
                Msg::Done(text, relist) => {
                    self.toast(text);
                    if relist && !self.files.dir.is_empty() {
                        let dir = self.files.dir.clone();
                        self.files_list(&dir);
                    }
                }
                Msg::Previewed(path, result) => {
                    let name = path.rsplit('/').next().unwrap_or_default();
                    if self.files.look.as_ref().is_some_and(|(n, _)| n == name) {
                        self.files.look = Some((name.to_string(), Some(result)));
                    }
                }
                Msg::Listed(..) => {}
            }
        }
    }

    /// The sidebar and the Quick Look window.
    pub(super) fn files_ui(&mut self, ctx: &egui::Context) {
        if !self.files.open {
            return;
        }
        self.files_pump();
        // Follow the focused pane's directory.
        let cwd = self
            .panes
            .get(&self.focus)
            .map(|p| p.info.cwd.clone())
            .filter(|c| c.starts_with('/'));
        if self.files.follow {
            if let Some(cwd) = cwd.as_ref().filter(|c| **c != self.files.dir) {
                self.files.filter.clear();
                self.files.selected = None;
                self.files_list(&cwd.clone());
            }
        }
        if !self.files.loading
            && self.files.listed_at.elapsed() > REFRESH
            && !self.files.dir.is_empty()
        {
            let dir = self.files.dir.clone();
            self.files_list(&dir);
        }
        ctx.request_repaint_after(Duration::from_millis(500));

        let t = self.theme.clone();
        let hidden = self.files.hidden;
        let needle = self.files.filter.to_lowercase();
        let shown: Vec<Entry> = self
            .files
            .entries
            .iter()
            .filter(|e| hidden || !e.name.starts_with('.'))
            .filter(|e| needle.is_empty() || e.name.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        let local = self.host == "local";
        let mut go: Option<String> = None;
        let mut act: Option<(String, Row)> = None;
        let mut focused = false;
        let mut close = false;
        egui::SidePanel::left("teryon-files")
            .exact_width(WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(color(mix(t.panel, t.background, 0.4)))
                    .stroke(Stroke::new(1_f32, color(t.border)))
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(weighted(ctx, "Files", "semibold", 13.).color(color(t.foreground)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, &t, Icon::Close, "Close  ⌘B").clicked() {
                            close = true;
                        }
                        let eye = if self.files.hidden {
                            "Hide dotfiles"
                        } else {
                            "Show dotfiles"
                        };
                        if ui
                            .add(
                                egui::Button::new(RichText::new(".*").monospace().size(11.).color(
                                    color(if self.files.hidden {
                                        t.foreground
                                    } else {
                                        t.muted
                                    }),
                                ))
                                .fill(if self.files.hidden {
                                    color(t.accent).gamma_multiply(0.3)
                                } else {
                                    Color32::TRANSPARENT
                                }),
                            )
                            .on_hover_text(eye)
                            .clicked()
                        {
                            self.files.hidden = !self.files.hidden;
                        }
                    });
                });
                // Breadcrumb: every component navigates there.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.;
                    for (label, path) in fs::crumbs(&self.files.dir) {
                        let last = path == self.files.dir;
                        let b = egui::Button::new(
                            RichText::new(label).size(11.5).color(color(if last {
                                t.foreground
                            } else {
                                t.muted
                            })),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE);
                        if ui.add(b).clicked() && !last {
                            go = Some(path);
                        }
                        ui.label(RichText::new("›").size(11.).color(color(t.muted)));
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.files.filter)
                        .hint_text("Filter")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.);
                if let Some(e) = &self.files.error {
                    ui.label(RichText::new(e).size(11.5).color(color(t.warning)));
                } else if self.files.loading && self.files.entries.is_empty() {
                    ui.label(RichText::new("Loading…").size(11.5).color(color(t.muted)));
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.;
                        if let Some(up) = fs::parent(&self.files.dir) {
                            if row(ui, &t, "..", true, false, None).clicked() {
                                go = Some(up);
                            }
                        }
                        for e in &shown {
                            let selected = self.files.selected.as_deref() == Some(e.name.as_str());
                            let resp = row(ui, &t, &e.name, e.dir, selected, Some(e));
                            focused |= resp.has_focus();
                            if resp.clicked() {
                                self.files.selected = Some(e.name.clone());
                                self.files.follow = true;
                            }
                            if resp.double_clicked() {
                                act = Some((e.name.clone(), Row::Open(e.dir)));
                            }
                            resp.context_menu(|ui| {
                                for (label, what) in [
                                    ("Quick Look", Some(Row::Look)),
                                    ("Insert path", Some(Row::Insert)),
                                    ("Copy path", Some(Row::CopyPath)),
                                    ("Download to Downloads", (!local).then_some(Row::Download)),
                                    ("Rename…", Some(Row::Rename)),
                                    ("Move to Trash…", Some(Row::Trash)),
                                    (if e.dir { "cd here" } else { "" }, e.dir.then_some(Row::Cd)),
                                    (
                                        "Reveal in Finder",
                                        (local && cfg!(target_os = "macos")).then_some(Row::Reveal),
                                    ),
                                ] {
                                    if let Some(what) = what {
                                        if ui.button(label).clicked() {
                                            act = Some((e.name.clone(), what));
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                        }
                        if shown.is_empty() && !self.files.loading && self.files.error.is_none() {
                            ui.label(
                                RichText::new("Nothing here")
                                    .size(11.5)
                                    .color(color(t.muted)),
                            );
                        }
                    });
            });

        // Keys while a row has focus: arrows move, Space looks, Enter opens, Backspace goes up.
        self.files.has_focus = focused;
        if focused {
            let (down, up, space, enter, back, esc) = ctx.input(|i| {
                (
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::Space),
                    i.key_pressed(Key::Enter),
                    i.key_pressed(Key::Backspace),
                    i.key_pressed(Key::Escape),
                )
            });
            let at = shown
                .iter()
                .position(|e| Some(e.name.as_str()) == self.files.selected.as_deref());
            if down || up {
                let next = match (at, down) {
                    (None, _) => 0,
                    (Some(i), true) => (i + 1).min(shown.len().saturating_sub(1)),
                    (Some(i), false) => i.saturating_sub(1),
                };
                if let Some(e) = shown.get(next) {
                    self.files.selected = Some(e.name.clone());
                    if self.files.look.is_some() {
                        act = Some((e.name.clone(), Row::Look));
                    }
                }
            }
            if let Some(e) = at.and_then(|i| shown.get(i)) {
                if space {
                    act = Some((e.name.clone(), Row::Look));
                } else if enter {
                    act = Some((e.name.clone(), Row::Open(e.dir)));
                }
            }
            if back {
                go = fs::parent(&self.files.dir);
            }
            if esc {
                if self.files.look.is_some() {
                    self.files.look = None;
                } else {
                    ctx.memory_mut(|m| {
                        m.request_focus(egui::Id::new(("teryon-terminal", self.focus)));
                    });
                }
            }
        }

        if let Some(path) = go {
            self.files.follow = false;
            self.files.filter.clear();
            self.files.selected = None;
            self.files_list(&path);
        }
        if let Some((name, what)) = act {
            let path = fs::join(&self.files.dir, &name);
            match what {
                Row::Open(true) => {
                    self.files.follow = false;
                    self.files.filter.clear();
                    self.files.selected = None;
                    self.files_list(&path);
                }
                Row::Open(false) | Row::Insert => self.files_insert(&format!("{} ", quoted(&path))),
                Row::Look => {
                    self.files.selected = Some(name.clone());
                    self.files_look(&name);
                }
                Row::CopyPath => {
                    ctx.copy_text(path.clone());
                    self.toast("Path copied".to_string());
                }
                Row::Download => self.files_download(&name),
                Row::Rename => {
                    self.files.dialog = Some(Dialog::Rename {
                        text: name.clone(),
                        name,
                        focus: true,
                    });
                }
                Row::Trash => {
                    let dir = self.files.entries.iter().any(|e| e.name == name && e.dir);
                    self.files.dialog = Some(Dialog::Trash { name, dir });
                }
                Row::Cd => self.files_insert(&format!("cd {}", quoted(&path))),
                Row::Reveal => {
                    let _ = std::process::Command::new("open")
                        .args(["-R", &path])
                        .spawn();
                }
            }
        }
        if close {
            self.files_toggle();
        }
        self.look_window(ctx);
        self.files_dialog(ctx);
    }

    /// Ask before renaming or moving to the Trash; nothing happens until the button is pressed.
    fn files_dialog(&mut self, ctx: &egui::Context) {
        let t = self.theme.clone();
        let local = self.host == "local";
        let dir = self.files.dir.clone();
        let mut run: Option<Dialog> = None;
        let mut cancel = false;
        match &mut self.files.dialog {
            None => return,
            Some(Dialog::Rename { name, text, focus }) => {
                egui::Window::new("Rename")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                    .show(ctx, |ui| {
                        ui.set_min_width(320.);
                        ui.label(
                            RichText::new(format!("New name for {name}")).color(color(t.muted)),
                        );
                        let edit =
                            ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                        if *focus {
                            edit.request_focus();
                            *focus = false;
                        }
                        let valid = fs::valid_name(text) && text != name;
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if !fs::valid_name(text) && !text.is_empty() {
                            ui.label(
                                RichText::new("A name can't contain / or line breaks")
                                    .size(11.5)
                                    .color(color(t.warning)),
                            );
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape))
                            {
                                cancel = true;
                            }
                            ui.add_enabled_ui(valid, |ui| {
                                if (primary_button(ui, &t, "Rename").clicked() || enter) && valid {
                                    run = Some(Dialog::Rename {
                                        name: name.clone(),
                                        text: text.clone(),
                                        focus: false,
                                    });
                                }
                            });
                        });
                    });
            }
            Some(Dialog::Trash { name, dir: is_dir }) => {
                let place = if local {
                    "the Trash"
                } else {
                    "~/.teryon-trash on the host"
                };
                egui::Window::new("Move to Trash?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                    .show(ctx, |ui| {
                        ui.set_min_width(320.);
                        let what = if *is_dir {
                            "folder and everything in it"
                        } else {
                            "file"
                        };
                        ui.label(format!(
                            "Move the {what} “{name}” to {place}? It can be put back from there."
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape))
                            {
                                cancel = true;
                            }
                            if primary_button(ui, &t, "Move to Trash").clicked() {
                                run = Some(Dialog::Trash {
                                    name: name.clone(),
                                    dir: *is_dir,
                                });
                            }
                        });
                    });
            }
        }
        if cancel {
            self.files.dialog = None;
        }
        match run {
            Some(Dialog::Rename { name, text, .. }) => {
                self.files.dialog = None;
                self.files.selected = Some(text.clone());
                self.files_do("Rename", move |target| {
                    fs::rename(target, &dir, &name, &text)
                });
            }
            Some(Dialog::Trash { name, .. }) => {
                self.files.dialog = None;
                self.files.selected = None;
                let path = fs::join(&dir, &name);
                self.files_do("Move to Trash", move |target| fs::trash(target, &path));
            }
            None => {}
        }
    }

    /// The Quick Look window for the file chosen with Space or the context menu.
    fn look_window(&mut self, ctx: &egui::Context) {
        let Some((name, state)) = self.files.look.clone() else {
            return;
        };
        let t = self.theme.clone();
        let mut open = true;
        // Upload a decoded image once per file.
        if let Some(Ok(Shown::Image {
            width,
            height,
            rgba,
        })) = &state
        {
            if self.files.look_tex.as_ref().map(|(n, _)| n) != Some(&name) {
                let image = egui::ColorImage::from_rgba_unmultiplied([*width, *height], rgba);
                let tex = ctx.load_texture("teryon-quicklook", image, egui::TextureOptions::LINEAR);
                self.files.look_tex = Some((name.clone(), tex));
            }
        }
        let tex = self.files.look_tex.clone();
        egui::Window::new(format!("Quick Look · {name}"))
            .id(egui::Id::new("teryon-quicklook"))
            .open(&mut open)
            .default_size([560., 420.])
            .show(ctx, |ui| match &state {
                None => {
                    ui.label(RichText::new("Reading…").color(color(t.muted)));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(color(t.warning)));
                }
                Some(Ok(Shown::Image { width, height, .. })) => {
                    ui.label(
                        RichText::new(format!("{width} × {height}"))
                            .size(11.)
                            .color(color(t.muted)),
                    );
                    if let Some((_, tex)) = tex.as_ref() {
                        let avail = ui.available_size();
                        let size = tex.size_vec2();
                        let scale = (avail.x / size.x).min(avail.y.max(120.) / size.y).min(1.);
                        ui.add(egui::Image::new((tex.id(), size * scale)));
                    }
                }
                Some(Ok(Shown::Doc(Preview::Binary { bytes }))) => {
                    ui.label(
                        RichText::new(format!(
                            "Binary file ({} read). Nothing to show as text.",
                            fs::human(*bytes)
                        ))
                        .color(color(t.muted)),
                    );
                }
                Some(Ok(Shown::Doc(Preview::Text { text, truncated }))) => {
                    if *truncated {
                        ui.label(
                            RichText::new(format!("First {} shown", fs::human(fs::PREVIEW_BYTES)))
                                .size(11.)
                                .color(color(t.muted)),
                        );
                    }
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(text).monospace().size(12.))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        });
                }
            });
        if !open {
            self.files.look = None;
            self.files.look_tex = None;
        }
    }
}

enum Row {
    Download,
    Rename,
    Trash,
    Open(bool),
    Look,
    Insert,
    CopyPath,
    Cd,
    Reveal,
}

/// One list row: icon, name, and an accent pill when selected.
fn row(
    ui: &mut egui::Ui,
    t: &Theme,
    name: &str,
    dir: bool,
    selected: bool,
    entry: Option<&Entry>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.), Sense::click());
    let lit = selected || resp.hovered() || resp.has_focus();
    let k = ui.ctx().animate_bool_with_time(resp.id, lit, 0.1);
    if k > 0. {
        let fill = if selected {
            color(t.accent).gamma_multiply(0.35)
        } else {
            color(mix(t.panel, t.foreground, 0.1))
        };
        ui.painter().rect_filled(rect, 6., fill.gamma_multiply(k));
    }
    let glyph = match entry {
        None => '\u{f148}',
        Some(e) if e.dir => '\u{f07b}',
        Some(e) => file_glyph(&e.name),
    };
    let tint = if dir { t.accent } else { t.muted };
    ui.painter().text(
        rect.left_center() + Vec2::new(8., 0.),
        egui::Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(13.),
        color(tint),
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(30., 0.),
        egui::Align2::LEFT_CENTER,
        name,
        FontId::proportional(12.5),
        color(t.foreground),
    );
    resp
}

/// A Font Awesome icon for a file by extension.
fn file_glyph(name: &str) -> char {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    match ext.as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "heic") => '\u{f1c5}',
        Some("zip" | "tar" | "gz" | "tgz" | "xz" | "7z" | "dmg") => '\u{f1c6}',
        Some("pdf") => '\u{f1c1}',
        Some("sh" | "zsh" | "bash" | "rs" | "py" | "js" | "ts" | "go" | "c" | "h" | "rb") => {
            '\u{f1c9}'
        }
        Some("mp3" | "wav" | "flac" | "m4a") => '\u{f1c7}',
        Some("mp4" | "mov" | "mkv" | "avi") => '\u{f1c8}',
        _ => '\u{f016}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;

    #[test]
    fn the_sidebar_lists_the_focused_panes_directory_and_draws() {
        let dir = std::env::temp_dir().join(format!("teryon-sidebar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("README.md"), "# hello\n").unwrap();
        let (mut app, _rx) = app_with_screen("");
        app.target = crate::transport::Target::new("local".into(), None).ok();
        let focus = app.focus;
        app.panes.get_mut(&focus).unwrap().info.cwd = dir.to_string_lossy().into_owned();
        app.files.open = true;
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.files_ui(ctx));
        // The listing arrives from a worker thread.
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(20));
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.files_ui(ctx));
            if !app.files.entries.is_empty() {
                break;
            }
        }
        let names: Vec<&str> = app.files.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "README.md"]);
        app.files_look("README.md");
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(20));
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.files_ui(ctx));
            if matches!(app.files.look, Some((_, Some(_)))) {
                break;
            }
        }
        assert!(matches!(
            &app.files.look,
            Some((n, Some(Ok(Shown::Doc(Preview::Text { text, .. }))))) if n == "README.md" && text.starts_with("# hello")
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn images_decode_and_shrink_and_junk_is_an_error() {
        let img = image::RgbaImage::from_pixel(3000, 1000, image::Rgba([10, 20, 30, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let Ok(Shown::Image {
            width,
            height,
            rgba,
        }) = decode(png.get_ref())
        else {
            panic!("png should decode");
        };
        assert_eq!((width, height), (1400, 467));
        assert_eq!(rgba.len(), width * height * 4);
        assert!(decode(b"not an image").is_err());
        assert!(is_image("Photo.JPG") && is_image("a.png") && !is_image("notes.txt"));
    }

    #[test]
    fn rename_and_trash_dialogs_draw_and_change_files_only_when_confirmed() {
        let dir = std::env::temp_dir().join(format!("teryon-dialog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.txt"), "x").unwrap();
        let (mut app, _rx) = app_with_screen("");
        app.target = crate::transport::Target::new("local".into(), None).ok();
        app.files.open = true;
        app.files.dir = dir.to_string_lossy().into_owned();
        app.files.follow = false;
        let ctx = egui::Context::default();
        app.files.dialog = Some(Dialog::Rename {
            name: "old.txt".into(),
            text: "new.txt".into(),
            focus: true,
        });
        assert!(app.files_capture_keys());
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.files_ui(ctx));
        // Drawing the question changes nothing on disk.
        assert!(dir.join("old.txt").exists() && !dir.join("new.txt").exists());
        app.files.dialog = Some(Dialog::Trash {
            name: "old.txt".into(),
            dir: false,
        });
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.files_ui(ctx));
        assert!(dir.join("old.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

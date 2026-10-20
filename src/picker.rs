//! `teryon pick`: an fzf-style fuzzy picker for the shell widgets (history, files, directories,
//! or lines from stdin). It draws on `/dev/tty` and prints the choice on stdout, so a shell can
//! capture it with `$(...)`. Exit status: 0 chosen, 10 chosen with Alt-Enter ("run it"), 1 cancelled.
use crate::{history, uikit};
use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use std::collections::{HashSet, VecDeque};
use std::path::Path;

/// Most entries a file or directory listing returns.
const MAX_ITEMS: usize = 50_000;
const MAX_DEPTH: usize = 10;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".cache",
    ".npm",
    ".cargo",
    ".rustup",
    ".Trash",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What is printed when chosen, and what the query matches.
    pub text: String,
    /// Dim context shown at the right.
    pub detail: String,
    /// Whether the item belongs to the current directory (Tab narrows to these).
    pub local: bool,
    /// What a history entry knows about its run, for the search filters.
    pub meta: Option<Meta>,
}
/// How, where and when a history command ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub exit: i32,
    pub ms: u64,
    pub cwd: String,
    pub branch: Option<String>,
    /// The other machine it ran on; `None` for this one.
    pub host: Option<String>,
}
impl Item {
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.into(),
            detail: String::new(),
            local: true,
            meta: None,
        }
    }
}

/// The structured part of a history search: words like `is:failed` or `dir:api`, which narrow the
/// list before the rest of the query is matched as fuzzy text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Filters {
    failed: bool,
    ok: bool,
    exit: Option<i32>,
    dirs: Vec<String>,
    branches: Vec<String>,
    hosts: Vec<String>,
    slow_ms: Option<u64>,
}
impl Filters {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
    fn matches(&self, meta: &Meta) -> bool {
        (!self.failed || meta.exit != 0)
            && (!self.ok || meta.exit == 0)
            && self.exit.is_none_or(|e| e == meta.exit)
            && self.slow_ms.is_none_or(|ms| meta.ms >= ms)
            && self
                .dirs
                .iter()
                .all(|d| meta.cwd.to_lowercase().contains(d))
            && self.hosts.iter().all(|h| {
                meta.host
                    .as_deref()
                    .unwrap_or("local")
                    .to_lowercase()
                    .contains(h)
            })
            && self.branches.iter().all(|b| {
                meta.branch
                    .as_deref()
                    .is_some_and(|m| m.to_lowercase().contains(b))
            })
    }
}
/// Split a history query into its filters and the text left to match. A word that is not a
/// recognized `name:value` stays part of the text, so `git commit -m "fix: x"` still searches.
pub fn parse_query(query: &str) -> (Filters, String) {
    let mut f = Filters::default();
    let mut text: Vec<&str> = Vec::new();
    for word in query.split_whitespace() {
        let lower = word.to_lowercase();
        let taken = match lower.split_once(':') {
            Some(("is", "failed")) => {
                f.failed = true;
                true
            }
            Some(("is", "ok")) => {
                f.ok = true;
                true
            }
            Some(("exit", n)) => n.parse().map(|n| f.exit = Some(n)).is_ok(),
            Some(("slow", n)) => n
                .parse::<f64>()
                .ok()
                .filter(|s| *s >= 0. && s.is_finite())
                .map(|s| f.slow_ms = Some((s * 1000.) as u64))
                .is_some(),
            Some(("dir", v)) if !v.is_empty() => {
                f.dirs.push(v.to_string());
                true
            }
            Some(("host", v)) if !v.is_empty() => {
                f.hosts.push(v.to_string());
                true
            }
            Some(("branch", v)) if !v.is_empty() => {
                f.branches.push(v.to_string());
                true
            }
            _ => false,
        };
        if !taken {
            text.push(word);
        }
    }
    (f, text.join(" "))
}
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    Cancel,
    Accept { text: String, run: bool },
}
pub struct State {
    items: Vec<Item>,
    pub query: String,
    pub selected: usize,
    pub only_local: bool,
    /// Whether Tab can narrow the list to this directory.
    scoped: bool,
    hits: Vec<usize>,
    /// Every run of every command, searched instead of `items` when a filter is in the query (the
    /// default list shows each distinct command once, its newest run).
    all: Vec<Item>,
    use_all: bool,
}
impl State {
    pub fn new(items: Vec<Item>, query: &str, scoped: bool) -> Self {
        Self::with_all(items, Vec::new(), query, scoped)
    }
    /// Like `new`, with a longer list (one item per run) for searches that use filters.
    pub fn with_all(items: Vec<Item>, all: Vec<Item>, query: &str, scoped: bool) -> Self {
        let mut s = Self {
            items,
            query: query.into(),
            selected: 0,
            only_local: false,
            scoped,
            hits: Vec::new(),
            all,
            use_all: false,
        };
        s.refresh();
        s
    }
    /// The list the current query searches.
    fn list(&self) -> &[Item] {
        if self.use_all {
            &self.all
        } else {
            &self.items
        }
    }
    /// The part of the query that is matched as text (what to highlight), without the filters.
    pub fn text_query(&self) -> String {
        if self.filterable() {
            parse_query(&self.query).1
        } else {
            self.query.clone()
        }
    }
    /// Only lists of history commands understand filter words.
    fn filterable(&self) -> bool {
        self.items.first().is_some_and(|i| i.meta.is_some())
    }
    /// Indexes of the items that match, best first (listing order when the query is empty).
    fn refresh(&mut self) {
        let (filters, text) = if self.filterable() {
            parse_query(&self.query)
        } else {
            (Filters::default(), self.query.clone())
        };
        self.use_all = !filters.is_empty() && !self.all.is_empty();
        let mut scored: Vec<(i32, usize)> = self
            .list()
            .iter()
            .enumerate()
            .filter(|(_, it)| !self.only_local || it.local)
            .filter(|(_, it)| it.meta.as_ref().is_none_or(|m| filters.matches(m)))
            .filter_map(|(i, it)| uikit::fuzzy_score(&text, &it.text).map(|s| (s, i)))
            .collect();
        if !text.trim().is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        self.hits = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
    }
    /// Replace the query (re-ranking only when it changed).
    pub fn set_query(&mut self, query: &str) {
        if self.query != query {
            self.query = query.into();
            self.refresh();
        }
    }
    /// Move the selection by `delta` rows, staying inside the list.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.hits.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }
    /// Narrow to this directory, or widen again (only for lists that can be scoped).
    pub fn toggle_scope(&mut self) {
        if self.scoped {
            self.only_local = !self.only_local;
            self.refresh();
        }
    }
    pub fn selected_text(&self) -> Option<String> {
        self.hits
            .get(self.selected)
            .map(|&i| self.list()[i].text.clone())
    }
    pub fn hits(&self) -> &[usize] {
        &self.hits
    }
    pub fn item(&self, index: usize) -> &Item {
        &self.list()[index]
    }
    pub fn total(&self) -> usize {
        self.list().len()
    }
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let last = self.hits.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Char('c' | 'g') if ctrl => return Outcome::Cancel,
            KeyCode::Enter => {
                return match self.hits.get(self.selected) {
                    Some(&i) => Outcome::Accept {
                        text: self.list()[i].text.clone(),
                        run: alt,
                    },
                    None => Outcome::Cancel,
                }
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char('p' | 'k') if ctrl => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(last),
            KeyCode::Char('n' | 'j') if ctrl => self.selected = (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown => self.selected = (self.selected + 10).min(last),
            KeyCode::Tab if self.scoped => {
                self.only_local = !self.only_local;
                self.refresh();
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refresh();
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refresh();
            }
            KeyCode::Char('w') if ctrl => {
                let trimmed = self.query.trim_end().len();
                let cut = self.query[..trimmed]
                    .rfind(char::is_whitespace)
                    .map_or(0, |i| i + 1);
                self.query.truncate(cut);
                self.refresh();
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                self.query.push(c);
                self.refresh();
            }
            _ => {}
        }
        Outcome::Pending
    }
}

/// Character positions in `text` matched by `query`, the way `uikit::fuzzy_score` matches it.
pub fn match_positions(query: &str, text: &str) -> Vec<usize> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut at = 0;
    let mut out = Vec::new();
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let Some(found) = (at..text.len()).find(|&i| text[i] == q) else {
            return Vec::new();
        };
        out.push(found);
        at = found + 1;
    }
    out
}

/// `3.4s`, `850ms`, `2m05s`.
pub fn short_duration(ms: u64) -> String {
    match ms {
        0..=999 => format!("{ms}ms"),
        1000..=59_999 => format!("{:.1}s", ms as f64 / 1000.0),
        _ => format!("{}m{:02}s", ms / 60_000, (ms / 1000) % 60),
    }
}
/// `now`-relative age: `5s`, `12m`, `3h`, `2d`.
pub fn ago(now: u64, at: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}
fn tilde(cwd: &str, home: &str) -> String {
    match cwd.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => cwd.to_string(),
    }
}
/// History as picker items: newest first, one per distinct command.
pub fn history_items(entries: &[history::Entry], cwd: &str, home: &str, now: u64) -> Vec<Item> {
    let mut seen = HashSet::new();
    entries
        .iter()
        .rev()
        .filter(|e| seen.insert(e.cmd.clone()))
        .map(|e| history_item(e, cwd, home, now))
        .collect()
}
fn history_item(e: &history::Entry, cwd: &str, home: &str, now: u64) -> Item {
    let status = if e.exit == 0 {
        "✓".to_string()
    } else {
        format!("✗{}", e.exit)
    };
    let branch = e
        .branch
        .as_deref()
        .map_or(String::new(), |b| format!(" ⎇ {b}"));
    let host = e
        .host
        .as_deref()
        .map_or(String::new(), |h| format!(" @{h}"));
    Item {
        text: e.cmd.clone(),
        detail: format!(
            "{status} {} {}{branch}{host} {}",
            short_duration(e.ms),
            tilde(&e.cwd, home),
            ago(now, e.at)
        ),
        local: e.host.is_none() && e.cwd == cwd,
        meta: Some(Meta {
            exit: e.exit,
            ms: e.ms,
            cwd: e.cwd.clone(),
            branch: e.branch.clone(),
            host: e.host.clone(),
        }),
    }
}
/// Every recorded run as its own item, newest first (no deduplication): what a filtered search reads.
pub fn history_runs(entries: &[history::Entry], cwd: &str, home: &str, now: u64) -> Vec<Item> {
    entries
        .iter()
        .rev()
        .map(|e| history_item(e, cwd, home, now))
        .collect()
}
/// Files and directories under `root` (breadth first, so shallow ones come first).
pub fn walk(root: &Path, dirs_only: bool) -> Vec<Item> {
    let mut out = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let Ok(kind) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = kind.is_dir();
            if is_dir && SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            if is_dir && depth + 1 < MAX_DEPTH {
                queue.push_back((e.path(), depth + 1));
            }
            if dirs_only && !is_dir {
                continue;
            }
            if let Ok(rel) = e.path().strip_prefix(root) {
                out.push(Item::plain(&rel.to_string_lossy()));
            }
            if out.len() >= MAX_ITEMS {
                return out;
            }
        }
    }
    out
}
/// Tracked and untracked-but-not-ignored files below `root`, if it is inside a git work tree.
pub fn git_files(root: &Path) -> Option<Vec<Item>> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-co", "--exclude-standard"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let items: Vec<Item> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .take(MAX_ITEMS)
        .map(Item::plain)
        .collect();
    (!items.is_empty()).then_some(items)
}

/// Run the picker on the controlling terminal. Returns what was chosen, or `None` if cancelled.
pub fn run(
    items: Vec<Item>,
    all: Vec<Item>,
    query: &str,
    scoped: bool,
    label: &str,
) -> Result<Option<(String, bool)>> {
    use crossterm::{
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("no terminal to draw the picker on")?;
    terminal::enable_raw_mode()?;
    let mut tty_out = tty.try_clone()?;
    execute!(tty_out, EnterAlternateScreen)?;
    struct Restore(std::fs::File);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = terminal::disable_raw_mode();
            let _ = crossterm::execute!(self.0, LeaveAlternateScreen);
        }
    }
    let _restore = Restore(tty.try_clone()?);
    let mut terminal = Terminal::new(CrosstermBackend::new(tty))?;
    let mut state = State::with_all(items, all, query, scoped);
    loop {
        terminal.draw(|f| draw(f, &state, label))?;
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => match state.key(k) {
                Outcome::Pending => {}
                Outcome::Cancel => return Ok(None),
                Outcome::Accept { text, run } => return Ok(Some((text, run))),
            },
            _ => {}
        }
    }
}
fn truncate(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    let mut s: String = chars[..width.saturating_sub(1)].iter().collect();
    s.push('…');
    s
}
fn draw(frame: &mut ratatui::Frame, state: &State, label: &str) {
    use ratatui::{
        layout::{Constraint, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::Paragraph,
    };
    let [input, list, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{label} ❯ "), Style::new().fg(Color::Cyan)),
            Span::raw(state.query.clone()),
            Span::styled("▏", Style::new().fg(Color::Cyan)),
        ])),
        input,
    );
    let height = usize::from(list.height);
    let width = usize::from(list.width);
    let first = state.selected.saturating_sub(height.saturating_sub(1));
    let lines: Vec<Line> = state
        .hits()
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(row, &i)| {
            let item = state.item(i);
            let on = row == state.selected;
            let avail = width.saturating_sub(3);
            let detail = truncate(&item.detail, avail / 3);
            let detail_w = detail.chars().count();
            let text_w = avail.saturating_sub(detail_w + 2);
            let shown = truncate(&item.text.replace('\n', "⏎"), text_w);
            let hit = match_positions(&state.text_query(), &shown);
            let base = if on {
                Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::new()
            };
            let mut spans = vec![Span::styled(
                if on { " ▸ " } else { "   " },
                Style::new().fg(Color::Cyan),
            )];
            for (pos, c) in shown.chars().enumerate() {
                let style = if hit.contains(&pos) {
                    base.fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    base
                };
                spans.push(Span::styled(c.to_string(), style));
            }
            let pad = text_w.saturating_sub(shown.chars().count()) + 2;
            spans.push(Span::styled(" ".repeat(pad), base));
            spans.push(Span::styled(detail, Style::new().fg(Color::DarkGray)));
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list);
    let scope = if state.only_local {
        " · this directory"
    } else {
        ""
    };
    let tab = if state.scoped {
        " · Tab this directory"
    } else {
        ""
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                " {}/{}{scope} · ↑↓ move · Enter choose · Alt-Enter run{tab} · Esc cancel",
                state.hits().len(),
                state.total()
            ),
            Style::new().fg(Color::DarkGray),
        )),
        foot,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn items(texts: &[&str]) -> Vec<Item> {
        texts.iter().map(|t| Item::plain(t)).collect()
    }
    fn type_str(s: &mut State, text: &str) {
        for c in text.chars() {
            s.key(press(KeyCode::Char(c)));
        }
    }
    fn texts(s: &State) -> Vec<String> {
        s.hits().iter().map(|&i| s.item(i).text.clone()).collect()
    }
    #[test]
    fn typing_filters_and_ranks_with_the_listing_order_as_the_tiebreak() {
        let mut s = State::new(
            items(&["cargo build", "git commit", "cargo test"]),
            "",
            false,
        );
        assert_eq!(texts(&s), ["cargo build", "git commit", "cargo test"]);
        type_str(&mut s, "ct");
        assert_eq!(texts(&s)[0], "cargo test");
        s.key(press(KeyCode::Backspace));
        s.key(press(KeyCode::Backspace));
        assert_eq!(s.hits().len(), 3);
        type_str(&mut s, "zzz");
        assert!(s.hits().is_empty());
    }
    #[test]
    fn keys_move_choose_run_and_cancel() {
        let mut s = State::new(items(&["a", "b", "c"]), "", false);
        s.key(press(KeyCode::Down));
        s.key(press(KeyCode::Down));
        s.key(press(KeyCode::Down));
        assert_eq!(s.selected, 2);
        s.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(
            s.key(press(KeyCode::Enter)),
            Outcome::Accept {
                text: "b".into(),
                run: false
            }
        );
        assert_eq!(
            s.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
            Outcome::Accept {
                text: "b".into(),
                run: true
            }
        );
        assert_eq!(s.key(press(KeyCode::Esc)), Outcome::Cancel);
        assert_eq!(
            s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Outcome::Cancel
        );
        let mut none = State::new(items(&["a"]), "zzz", false);
        assert_eq!(none.key(press(KeyCode::Enter)), Outcome::Cancel);
    }
    #[test]
    fn ctrl_u_and_ctrl_w_edit_the_query() {
        let mut s = State::new(items(&["a"]), "git sta", false);
        s.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(s.query, "git ");
        s.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(s.query, "");
    }
    #[test]
    fn tab_narrows_to_this_directory_only_when_scoped() {
        let mut here = Item::plain("ls");
        here.local = true;
        let mut there = Item::plain("make");
        there.local = false;
        let mut s = State::new(vec![there.clone(), here.clone()], "", true);
        s.key(press(KeyCode::Tab));
        assert_eq!(texts(&s), ["ls"]);
        s.key(press(KeyCode::Tab));
        assert_eq!(s.hits().len(), 2);
        let mut plain = State::new(vec![there, here], "", false);
        plain.key(press(KeyCode::Tab));
        assert_eq!(plain.hits().len(), 2);
    }
    #[test]
    fn the_desktop_bar_drives_the_state_without_key_events() {
        let mut s = State::new(
            items(&["cargo build", "git commit", "cargo test"]),
            "",
            true,
        );
        s.set_query("cargo");
        assert_eq!(s.hits().len(), 2);
        let order = texts(&s);
        s.move_by(5);
        assert_eq!(
            s.selected_text().as_deref(),
            order.last().map(String::as_str)
        );
        s.move_by(-9);
        assert_eq!(
            s.selected_text().as_deref(),
            order.first().map(String::as_str)
        );
        s.set_query("zzz");
        assert_eq!(s.selected_text(), None);
        let mut local = Item::plain("ls");
        local.local = false;
        let mut s = State::new(vec![local, Item::plain("make")], "", true);
        s.toggle_scope();
        assert_eq!(texts(&s), ["make"]);
    }
    #[test]
    fn match_positions_follow_the_score_rule() {
        assert_eq!(match_positions("gc", "git commit"), vec![0, 4]);
        assert!(match_positions("xyz", "git").is_empty());
    }
    #[test]
    fn durations_and_ages_read_naturally() {
        assert_eq!(short_duration(850), "850ms");
        assert_eq!(short_duration(3400), "3.4s");
        assert_eq!(short_duration(125_000), "2m05s");
        assert_eq!(ago(100, 95), "5s");
        assert_eq!(ago(10_000, 10_000 - 720), "12m");
        assert_eq!(ago(100_000, 100_000 - 3 * 3600), "3h");
        assert_eq!(ago(1_000_000, 1_000_000 - 2 * 86_400), "2d");
    }
    #[test]
    fn history_items_are_newest_first_deduped_and_marked_local() {
        let e = |cmd: &str, cwd: &str, exit: i32, at: u64| history::Entry {
            cmd: cmd.into(),
            cwd: cwd.into(),
            exit,
            ms: 1500,
            at,
            branch: None,
            host: None,
        };
        let list = vec![
            e("make", "/h/a", 0, 10),
            e("ls", "/h/b", 0, 20),
            e("make", "/h/b", 2, 30),
        ];
        let items = history_items(&list, "/h/b", "/h", 90);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "make");
        assert_eq!(items[0].detail, "✗2 1.5s ~/b 1m");
        assert!(items[0].local && items[1].local);
        assert!(!history_items(&list, "/h/a", "/h", 90)[1].local);
    }
    #[test]
    fn walking_skips_build_dirs_and_can_list_only_directories() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::create_dir_all(p.join("src/deep")).unwrap();
        std::fs::create_dir_all(p.join("target/debug")).unwrap();
        std::fs::create_dir_all(p.join(".git/objects")).unwrap();
        std::fs::write(p.join("README.md"), "").unwrap();
        std::fs::write(p.join("src/main.rs"), "").unwrap();
        std::fs::write(p.join("target/debug/x"), "").unwrap();
        let all: Vec<String> = walk(p, false).into_iter().map(|i| i.text).collect();
        assert!(all.contains(&"README.md".to_string()));
        assert!(all.contains(&"src/main.rs".to_string()));
        assert!(all.contains(&"src/deep".to_string()));
        assert!(!all
            .iter()
            .any(|t| t.starts_with("target") || t.starts_with(".git")));
        let dirs: Vec<String> = walk(p, true).into_iter().map(|i| i.text).collect();
        assert_eq!(dirs, ["src", "src/deep"]);
    }

    fn run_entry(
        cmd: &str,
        cwd: &str,
        exit: i32,
        ms: u64,
        branch: Option<&str>,
        at: u64,
    ) -> history::Entry {
        history::Entry {
            cmd: cmd.into(),
            cwd: cwd.into(),
            exit,
            ms,
            at,
            branch: branch.map(str::to_string),
            host: None,
        }
    }
    #[test]
    fn filter_words_are_split_from_the_text_and_unknown_words_stay_text() {
        let (f, text) = parse_query("cargo is:failed dir:API branch:Main slow:2.5 exit:101 test");
        assert_eq!(text, "cargo test");
        assert!(f.failed && !f.ok);
        assert_eq!((f.exit, f.slow_ms), (Some(101), Some(2500)));
        assert_eq!(
            (f.dirs, f.branches),
            (vec!["api".to_string()], vec!["main".to_string()])
        );
        // Things that only look like filters are searched as text.
        for q in [
            "git commit -m fix:x",
            "exit:abc",
            "slow:-1",
            "dir:",
            "is:weird",
            "http://x",
        ] {
            let (f, text) = parse_query(q);
            assert!(f.is_empty() && text == q, "{q}");
        }
        assert!(parse_query("").0.is_empty());
    }
    #[test]
    fn a_filtered_search_reads_every_run_not_just_the_newest() {
        let list = vec![
            run_entry("make test", "/h/api", 2, 90_000, Some("main"), 10),
            run_entry("make test", "/h/web", 0, 1_000, Some("fix-ui"), 20),
            run_entry("ls", "/h/api", 0, 5, None, 30),
            run_entry("make test", "/h/api", 0, 2_000, Some("main"), 40),
        ];
        let items = history_items(&list, "/h/api", "/h", 100);
        let runs = history_runs(&list, "/h/api", "/h", 100);
        assert_eq!((items.len(), runs.len()), (2, 4));
        let texts = |s: &State| -> Vec<String> {
            s.hits().iter().map(|&i| s.item(i).detail.clone()).collect()
        };
        // Without a filter the list is the usual one: each command once.
        let s = State::with_all(items.clone(), runs.clone(), "", true);
        assert_eq!(s.hits().len(), 2);
        // The newest `make test` succeeded, but an older run failed: the filter finds it.
        let s = State::with_all(items.clone(), runs.clone(), "is:failed", true);
        assert_eq!(texts(&s).len(), 1);
        assert!(
            texts(&s)[0].starts_with("✗2 1m30s ~/api ⎇ main"),
            "{:?}",
            texts(&s)
        );
        assert_eq!(s.selected_text().as_deref(), Some("make test"));
        assert_eq!(
            State::with_all(items.clone(), runs.clone(), "is:ok make", true)
                .hits()
                .len(),
            2
        );
        assert_eq!(
            State::with_all(items.clone(), runs.clone(), "branch:fix", true)
                .hits()
                .len(),
            1
        );
        assert_eq!(
            State::with_all(items.clone(), runs.clone(), "dir:api slow:60", true)
                .hits()
                .len(),
            1
        );
        assert_eq!(
            State::with_all(items.clone(), runs.clone(), "exit:2 branch:main", true)
                .hits()
                .len(),
            1
        );
        // Filters and text combine, and the text alone is what gets highlighted.
        let s = State::with_all(items.clone(), runs.clone(), "ls is:ok", true);
        assert_eq!((s.hits().len(), s.text_query()), (1, "ls".to_string()));
        // A command with no recorded branch never matches a branch filter.
        assert!(State::with_all(items, runs, "branch:main ls", true)
            .hits()
            .is_empty());
    }
    #[test]
    fn host_filter_separates_this_machine_from_pulled_ones() {
        let mk = |cmd: &str, host: Option<&str>| history::Entry {
            cmd: cmd.into(),
            cwd: "/srv".into(),
            exit: 0,
            ms: 1,
            at: 10,
            branch: None,
            host: host.map(str::to_string),
        };
        let entries = vec![mk("uptime", Some("Web1")), mk("ls", None)];
        let items = history_items(&entries, "/srv", "/home/me", 20);
        let runs = history_runs(&entries, "/srv", "/home/me", 20);
        let remote = items.iter().find(|i| i.text == "uptime").unwrap();
        assert!(remote.detail.contains("@Web1"));
        assert!(!remote.local, "another machine's directory is never `here`");
        assert!(items.iter().find(|i| i.text == "ls").unwrap().local);
        let texts = |q: &str| -> Vec<String> {
            let st = State::with_all(items.clone(), runs.clone(), q, true);
            st.hits().iter().map(|&i| st.item(i).text.clone()).collect()
        };
        assert_eq!(texts("host:web"), ["uptime"]);
        assert_eq!(texts("host:local"), ["ls"]);
        assert_eq!(texts("host:nowhere"), Vec::<String>::new());
    }
    #[test]
    fn lists_that_are_not_history_ignore_filter_words() {
        let mut s = State::new(
            vec![Item::plain("is:failed.txt"), Item::plain("a.txt")],
            "is:failed",
            false,
        );
        assert_eq!(s.hits().len(), 1, "searched as plain text");
        assert_eq!(s.text_query(), "is:failed");
        s.set_query("a");
        assert_eq!(s.hits().len(), 2);
    }
}

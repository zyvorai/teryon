//! `teryon` with no arguments: pick a host and workspace, then connect.
use crate::{protocol::valid_name, recent::Recent, theme::Theme, uikit};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub host: String,
    /// `None` for a bare host (its default workspace).
    pub workspace: Option<String>,
    pub note: String,
}
impl Entry {
    pub fn label(&self) -> String {
        match &self.workspace {
            Some(w) => format!("{} / {w}", self.host),
            None => self.host.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub host: String,
    pub workspace: String,
}

/// Recents first, then every known host once: configured aliases, `~/.ssh/config` names and `local`.
pub fn build_entries(
    config_hosts: &[String],
    recents: &[Recent],
    ssh_hosts: &[String],
) -> Vec<Entry> {
    let mut out: Vec<Entry> = recents
        .iter()
        .map(|r| Entry {
            host: r.host.clone(),
            workspace: Some(r.workspace.clone()),
            note: "recent".into(),
        })
        .collect();
    let sources = [
        ("local", vec!["local".to_string()]),
        ("configured", config_hosts.to_vec()),
        ("ssh config", ssh_hosts.to_vec()),
    ];
    for (note, hosts) in sources {
        for host in hosts {
            if !out.iter().any(|e| e.workspace.is_none() && e.host == host) {
                out.push(Entry {
                    host,
                    workspace: None,
                    note: note.into(),
                });
            }
        }
    }
    out
}
/// Indexes of `entries` matching `query`, best match first (original order when the query is empty).
pub fn filter(entries: &[Entry], query: &str) -> Vec<usize> {
    let mut hits: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            uikit::fuzzy_score(query, &format!("{} {}", e.label(), e.note)).map(|s| (s, i))
        })
        .collect();
    if !query.trim().is_empty() {
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    hits.into_iter().map(|(_, i)| i).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Choose(Choice),
    /// Tab on a host: the caller lists its workspaces and calls `show_workspaces`.
    Workspaces(String),
    /// Ctrl+W on a host: the caller sends it a wake-on-LAN packet.
    Wake(String),
}
enum Mode {
    Hosts,
    Workspaces { host: String, names: Vec<String> },
}
pub struct Picker {
    entries: Vec<Entry>,
    mode: Mode,
    pub query: String,
    pub selected: usize,
    pub message: String,
}
impl Picker {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            mode: Mode::Hosts,
            query: String::new(),
            selected: 0,
            message: String::new(),
        }
    }
    pub fn show_workspaces(&mut self, host: String, names: Vec<String>) {
        self.mode = Mode::Workspaces { host, names };
        self.query.clear();
        self.selected = 0;
        self.message.clear();
    }
    /// Rows currently listed as `(label, note)`.
    pub fn rows(&self) -> Vec<(String, String)> {
        match &self.mode {
            Mode::Hosts => filter(&self.entries, &self.query)
                .into_iter()
                .map(|i| (self.entries[i].label(), self.entries[i].note.clone()))
                .collect(),
            Mode::Workspaces { names, .. } => {
                let as_entries: Vec<Entry> = names
                    .iter()
                    .map(|n| Entry {
                        host: n.clone(),
                        workspace: None,
                        note: String::new(),
                    })
                    .collect();
                filter(&as_entries, &self.query)
                    .into_iter()
                    .map(|i| (names[i].clone(), String::new()))
                    .collect()
            }
        }
    }
    pub fn in_workspaces(&self) -> Option<&str> {
        match &self.mode {
            Mode::Workspaces { host, .. } => Some(host),
            Mode::Hosts => None,
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        let rows = self.rows();
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match (&self.mode, self.entry_at(self.selected)) {
                    (Mode::Hosts, Some(e)) => Action::Wake(e.host.clone()),
                    _ => Action::None,
                }
            }
            KeyCode::Esc => {
                if self.in_workspaces().is_some() {
                    self.mode = Mode::Hosts;
                    self.query.clear();
                    self.selected = 0;
                    Action::None
                } else {
                    Action::Quit
                }
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(rows.len().saturating_sub(1));
                Action::None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
                Action::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.selected = 0;
                Action::None
            }
            KeyCode::Tab => match (&self.mode, self.entry_at(self.selected)) {
                (Mode::Hosts, Some(e)) => Action::Workspaces(e.host.clone()),
                _ => Action::None,
            },
            KeyCode::Enter => self.enter(&rows),
            _ => Action::None,
        }
    }
    fn entry_at(&self, row: usize) -> Option<&Entry> {
        filter(&self.entries, &self.query)
            .get(row)
            .map(|i| &self.entries[*i])
    }
    fn enter(&mut self, rows: &[(String, String)]) -> Action {
        let typed = self.query.trim().to_string();
        match &self.mode {
            Mode::Hosts => match self.entry_at(self.selected) {
                Some(e) => Action::Choose(Choice {
                    host: e.host.clone(),
                    workspace: e.workspace.clone().unwrap_or_else(|| "default".into()),
                }),
                None if !typed.is_empty() => Action::Choose(Choice {
                    host: typed,
                    workspace: "default".into(),
                }),
                None => Action::None,
            },
            Mode::Workspaces { host, .. } => {
                if let Some((name, _)) = rows.get(self.selected) {
                    Action::Choose(Choice {
                        host: host.clone(),
                        workspace: name.clone(),
                    })
                } else if valid_name(&typed) {
                    Action::Choose(Choice {
                        host: host.clone(),
                        workspace: typed,
                    })
                } else {
                    self.message = "Workspace names use letters, digits, - and _".into();
                    Action::None
                }
            }
        }
    }
}

fn draw(frame: &mut Frame, picker: &Picker, theme: &Theme) {
    let [head, input, list, foot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let blue = ratatui::style::Color::Rgb(0x0a, 0x84, 0xff);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" T", Style::new().fg(blue).add_modifier(Modifier::BOLD)),
                Span::styled("▪ ", Style::new().fg(theme.foreground)),
                Span::styled(
                    "TERYON",
                    Style::new()
                        .fg(theme.foreground)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                match picker.in_workspaces() {
                    Some(h) => format!(" Workspaces on {h}"),
                    None => " Where to?".to_string(),
                },
                Style::new().fg(theme.muted),
            )),
        ])
        .style(Style::new().bg(theme.background)),
        head,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" › ", Style::new().fg(theme.accent)),
            Span::styled(picker.query.clone(), Style::new().fg(theme.foreground)),
            Span::styled("▏", Style::new().fg(theme.accent)),
        ]))
        .style(Style::new().bg(theme.background)),
        input,
    );
    let rows = picker.rows();
    let height = usize::from(list.height);
    let first = picker.selected.saturating_sub(height.saturating_sub(1));
    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, (label, note))| {
            let on = i == picker.selected;
            let style = if on {
                Style::new()
                    .fg(theme.foreground)
                    .bg(theme.panel)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.foreground)
            };
            Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::new().fg(theme.accent),
                ),
                Span::styled(label.clone(), style),
                Span::styled(format!("  {note}"), Style::new().fg(theme.muted)),
            ])
        })
        .collect();
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            if picker.in_workspaces().is_some() {
                "   Enter creates a workspace with this name"
            } else {
                "   Enter connects to what you typed (user@host works)"
            },
            Style::new().fg(theme.muted),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme.background)),
        list,
    );
    let hint = if picker.message.is_empty() {
        " ↑↓ move · Enter connect · Tab workspaces · Ctrl+W wake · Esc quit".to_string()
    } else {
        format!(" {}", picker.message)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hint, Style::new().fg(theme.muted)))
            .style(Style::new().bg(theme.background)),
        foot,
    );
}

/// Run the picker on the terminal. `workspaces` lists a host's workspaces when the user presses Tab;
/// `wake` sends a wake-on-LAN packet on Ctrl+W and returns the message to show.
pub fn pick(
    entries: Vec<Entry>,
    theme: &Theme,
    workspaces: &mut dyn FnMut(&str) -> Result<Vec<String>>,
    wake: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<Option<Choice>> {
    let mut terminal = ratatui::init();
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            ratatui::restore();
        }
    }
    let _restore = Restore;
    let mut picker = Picker::new(entries);
    loop {
        terminal.draw(|f| draw(f, &picker, theme))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match picker.key(key) {
            Action::None => {}
            Action::Quit => return Ok(None),
            Action::Choose(choice) => return Ok(Some(choice)),
            Action::Wake(host) => {
                picker.message = match wake(&host) {
                    Ok(m) => m,
                    Err(e) => format!("{host}: {e:#}"),
                };
            }
            Action::Workspaces(host) => {
                picker.message = format!("Asking {host} for its workspaces…");
                terminal.draw(|f| draw(f, &picker, theme))?;
                match workspaces(&host) {
                    Ok(names) => picker.show_workspaces(host, names),
                    Err(e) => picker.message = format!("{host}: {e:#}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn typed(p: &mut Picker, text: &str) {
        for c in text.chars() {
            p.key(key(KeyCode::Char(c)));
        }
    }
    fn sample() -> Vec<Entry> {
        build_entries(
            &["mini".into()],
            &[Recent {
                host: "mini".into(),
                workspace: "fluxvm".into(),
                last_used: 9,
                count: 3,
            }],
            &["mini".into(), "build-box".into()],
        )
    }
    #[test]
    fn entries_list_recents_first_and_each_host_once() {
        let e = sample();
        assert_eq!(e[0].label(), "mini / fluxvm");
        let hosts: Vec<String> = e
            .iter()
            .filter(|e| e.workspace.is_none())
            .map(|e| e.host.clone())
            .collect();
        assert_eq!(hosts, ["local", "mini", "build-box"]);
    }
    #[test]
    fn typing_filters_and_enter_chooses() {
        let mut p = Picker::new(sample());
        typed(&mut p, "build");
        assert_eq!(p.rows().len(), 1);
        assert_eq!(
            p.key(key(KeyCode::Enter)),
            Action::Choose(Choice {
                host: "build-box".into(),
                workspace: "default".into()
            })
        );
    }
    #[test]
    fn recents_connect_to_their_workspace_and_unknown_text_is_a_host() {
        let mut p = Picker::new(sample());
        assert_eq!(
            p.key(key(KeyCode::Enter)),
            Action::Choose(Choice {
                host: "mini".into(),
                workspace: "fluxvm".into()
            })
        );
        let mut p = Picker::new(sample());
        typed(&mut p, "zz@qq");
        assert_eq!(
            p.key(key(KeyCode::Enter)),
            Action::Choose(Choice {
                host: "zz@qq".into(),
                workspace: "default".into()
            })
        );
    }
    #[test]
    fn tab_lists_workspaces_and_enter_creates_new_ones() {
        let mut p = Picker::new(sample());
        typed(&mut p, "build");
        assert_eq!(
            p.key(key(KeyCode::Tab)),
            Action::Workspaces("build-box".into())
        );
        p.show_workspaces("build-box".into(), vec!["ci".into(), "dev".into()]);
        typed(&mut p, "de");
        assert_eq!(p.rows().len(), 1);
        assert_eq!(
            p.key(key(KeyCode::Enter)),
            Action::Choose(Choice {
                host: "build-box".into(),
                workspace: "dev".into()
            })
        );
        p.query = "bad name!".into();
        assert_eq!(p.key(key(KeyCode::Enter)), Action::None);
        assert!(!p.message.is_empty());
        p.query = "fresh".into();
        assert_eq!(
            p.key(key(KeyCode::Enter)),
            Action::Choose(Choice {
                host: "build-box".into(),
                workspace: "fresh".into()
            })
        );
        assert_eq!(p.key(key(KeyCode::Esc)), Action::None);
        assert!(p.in_workspaces().is_none());
        assert_eq!(p.key(key(KeyCode::Esc)), Action::Quit);
    }
    #[test]
    fn ctrl_w_wakes_the_selected_host_but_only_in_the_host_list() {
        let mut p = Picker::new(sample());
        typed(&mut p, "build");
        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(p.key(ctrl_w), Action::Wake("build-box".into()));
        assert!(p.query == "build", "Ctrl+W does not type a letter");
        p.show_workspaces("build-box".into(), vec!["a".into()]);
        assert_eq!(p.key(ctrl_w), Action::None);
    }
}

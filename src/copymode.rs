//! vi-style keyboard selection over a block of text (the pane's history plus its screen).
//! Pure state machine: the desktop feeds it keys and the buffer, and draws the result.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sel {
    Char,
    Line,
    Block,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Esc,
    Enter,
    Backspace,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Exit,
    /// Copy this text, then leave copy mode.
    Yank(String),
}
#[derive(Clone, Debug)]
pub struct CopyMode {
    pub row: usize,
    pub col: usize,
    pub anchor: Option<(usize, usize)>,
    pub sel: Sel,
    pub last_search: Option<(String, bool)>,
    /// Text being typed after `/` or `?` (`true` = forward).
    pub typing: Option<(String, bool)>,
    pending_g: bool,
    /// How far `^U`/`^D` move.
    pub page: usize,
}

#[derive(PartialEq, Clone, Copy)]
enum Class {
    Blank,
    Word,
    Punct,
}
fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

impl CopyMode {
    pub fn new(row: usize, col: usize, page: usize) -> Self {
        Self {
            row,
            col,
            anchor: None,
            sel: Sel::Char,
            last_search: None,
            typing: None,
            pending_g: false,
            page: page.max(2),
        }
    }
    fn line_len(buf: &[Vec<char>], row: usize) -> usize {
        buf.get(row).map_or(0, Vec::len)
    }
    fn clamp(&mut self, buf: &[Vec<char>]) {
        self.row = self.row.min(buf.len().saturating_sub(1));
        self.col = self
            .col
            .min(Self::line_len(buf, self.row).saturating_sub(1));
    }
    /// Whether the cell at (`row`, `col`) is inside the current selection.
    pub fn selected(&self, row: usize, col: usize) -> bool {
        let Some(a) = self.anchor else {
            return false;
        };
        let (r0, r1) = (a.0.min(self.row), a.0.max(self.row));
        if row < r0 || row > r1 {
            return false;
        }
        match self.sel {
            Sel::Line => true,
            Sel::Block => col >= a.1.min(self.col) && col <= a.1.max(self.col),
            Sel::Char => {
                let (from, to) = if a <= (self.row, self.col) {
                    (a, (self.row, self.col))
                } else {
                    ((self.row, self.col), a)
                };
                (row, col) >= from && (row, col) <= to
            }
        }
    }
    fn text(&self, buf: &[Vec<char>]) -> String {
        let Some(a) = self.anchor else {
            // Nothing selected: yank the cursor's line.
            return buf.get(self.row).map_or(String::new(), |l| {
                l.iter().collect::<String>().trim_end().to_string()
            });
        };
        let (r0, r1) = (a.0.min(self.row), a.0.max(self.row));
        let mut lines = Vec::new();
        for row in r0..=r1 {
            let line = buf.get(row).cloned().unwrap_or_default();
            let picked: String = line
                .iter()
                .enumerate()
                .filter(|(c, _)| self.sel == Sel::Line || self.selected(row, *c))
                .map(|(_, ch)| *ch)
                .collect();
            lines.push(picked.trim_end().to_string());
        }
        lines.join("\n")
    }
    fn find(&mut self, buf: &[Vec<char>], needle: &str, forward: bool) -> bool {
        if needle.is_empty() {
            return false;
        }
        let ci = !needle.chars().any(char::is_uppercase);
        let norm = |s: &str| if ci { s.to_lowercase() } else { s.to_string() };
        let needle = norm(needle);
        let hits: Vec<(usize, usize)> = buf
            .iter()
            .enumerate()
            .flat_map(|(r, line)| {
                let text: String = line.iter().collect();
                let hay = norm(&text);
                // Byte offsets in `hay` back to character columns.
                let cols: Vec<usize> = hay.char_indices().map(|(b, _)| b).collect();
                hay.match_indices(&needle)
                    .filter_map(|(b, _)| cols.iter().position(|x| *x == b).map(|c| (r, c)))
                    .collect::<Vec<_>>()
            })
            .collect();
        let here = (self.row, self.col);
        let next = if forward {
            hits.iter().find(|h| **h > here).or_else(|| hits.first())
        } else {
            hits.iter()
                .rev()
                .find(|h| **h < here)
                .or_else(|| hits.last())
        };
        match next {
            Some(&(r, c)) => {
                self.row = r;
                self.col = c;
                true
            }
            None => false,
        }
    }
    fn word_forward(&mut self, buf: &[Vec<char>], to_end: bool) {
        let at = |r: usize, c: usize| buf.get(r).and_then(|l| l.get(c)).copied().unwrap_or(' ');
        let (mut r, mut c) = (self.row, self.col);
        let last = buf.len().saturating_sub(1);
        let advance = |r: &mut usize, c: &mut usize| -> bool {
            if *c + 1 < Self::line_len(buf, *r) {
                *c += 1;
                true
            } else if *r < last {
                *r += 1;
                *c = 0;
                true
            } else {
                false
            }
        };
        if to_end {
            if !advance(&mut r, &mut c) {
                return;
            }
            while class(at(r, c)) == Class::Blank && advance(&mut r, &mut c) {}
            let k = class(at(r, c));
            while c + 1 < Self::line_len(buf, r) && class(at(r, c + 1)) == k {
                c += 1;
            }
        } else {
            let k = class(at(r, c));
            while k != Class::Blank && class(at(r, c)) == k && advance(&mut r, &mut c) && c != 0 {}
            while class(at(r, c)) == Class::Blank && advance(&mut r, &mut c) {}
        }
        self.row = r;
        self.col = c;
    }
    fn word_back(&mut self, buf: &[Vec<char>]) {
        let at = |r: usize, c: usize| buf.get(r).and_then(|l| l.get(c)).copied().unwrap_or(' ');
        let (mut r, mut c) = (self.row, self.col);
        let back = |r: &mut usize, c: &mut usize| -> bool {
            if *c > 0 {
                *c -= 1;
                true
            } else if *r > 0 {
                *r -= 1;
                *c = Self::line_len(buf, *r).saturating_sub(1);
                true
            } else {
                false
            }
        };
        if !back(&mut r, &mut c) {
            return;
        }
        while class(at(r, c)) == Class::Blank && back(&mut r, &mut c) {}
        let k = class(at(r, c));
        while c > 0 && class(at(r, c - 1)) == k {
            c -= 1;
        }
        self.row = r;
        self.col = c;
    }
    pub fn key(&mut self, key: Key, buf: &[Vec<char>]) -> Outcome {
        self.clamp(buf);
        if let Some((text, forward)) = self.typing.as_mut() {
            match key {
                Key::Esc => self.typing = None,
                Key::Backspace => {
                    text.pop();
                }
                Key::Char(c) => text.push(c),
                Key::Enter => {
                    let (needle, fwd) = (text.clone(), *forward);
                    self.typing = None;
                    self.find(buf, &needle, fwd);
                    self.last_search = Some((needle, fwd));
                }
                Key::Ctrl(_) => {}
            }
            return Outcome::None;
        }
        let g = std::mem::take(&mut self.pending_g);
        let last = buf.len().saturating_sub(1);
        match key {
            Key::Esc => {
                if self.anchor.is_some() {
                    self.anchor = None;
                } else {
                    return Outcome::Exit;
                }
            }
            Key::Char('q') => return Outcome::Exit,
            Key::Char('h') => self.col = self.col.saturating_sub(1),
            Key::Char('l') => self.col += 1,
            Key::Char('j') => self.row = (self.row + 1).min(last),
            Key::Char('k') => self.row = self.row.saturating_sub(1),
            Key::Char('0') => self.col = 0,
            Key::Char('^') => {
                self.col = buf
                    .get(self.row)
                    .and_then(|l| l.iter().position(|c| !c.is_whitespace()))
                    .unwrap_or(0);
            }
            Key::Char('$') => self.col = Self::line_len(buf, self.row).saturating_sub(1),
            Key::Char('w') => self.word_forward(buf, false),
            Key::Char('e') => self.word_forward(buf, true),
            Key::Char('b') => self.word_back(buf),
            Key::Char('g') => {
                if g {
                    self.row = 0;
                } else {
                    self.pending_g = true;
                }
            }
            Key::Char('G') => self.row = last,
            Key::Ctrl('u') => self.row = self.row.saturating_sub(self.page / 2),
            Key::Ctrl('d') => self.row = (self.row + self.page / 2).min(last),
            Key::Ctrl('b') => self.row = self.row.saturating_sub(self.page),
            Key::Ctrl('f') => self.row = (self.row + self.page).min(last),
            Key::Char('v') => self.toggle(Sel::Char),
            Key::Char('V') => self.toggle(Sel::Line),
            Key::Ctrl('v') => self.toggle(Sel::Block),
            Key::Char('/') => self.typing = Some((String::new(), true)),
            Key::Char('?') => self.typing = Some((String::new(), false)),
            Key::Char('n') | Key::Char('N') => {
                if let Some((needle, fwd)) = self.last_search.clone() {
                    let forward = if key == Key::Char('n') { fwd } else { !fwd };
                    self.find(buf, &needle, forward);
                }
            }
            Key::Char('y') | Key::Enter => return Outcome::Yank(self.text(buf)),
            _ => {}
        }
        self.clamp(buf);
        Outcome::None
    }
    fn toggle(&mut self, kind: Sel) {
        if self.anchor.is_some() && self.sel == kind {
            self.anchor = None;
        } else {
            if self.anchor.is_none() {
                self.anchor = Some((self.row, self.col));
            }
            self.sel = kind;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn buf(lines: &[&str]) -> Vec<Vec<char>> {
        lines.iter().map(|l| l.chars().collect()).collect()
    }
    fn press(m: &mut CopyMode, b: &[Vec<char>], keys: &str) -> Outcome {
        let mut out = Outcome::None;
        for c in keys.chars() {
            out = m.key(Key::Char(c), b);
        }
        out
    }
    #[test]
    fn motions_move_by_char_line_and_word() {
        let b = buf(&["hello brave new world", "second line", "third"]);
        let mut m = CopyMode::new(0, 0, 10);
        press(&mut m, &b, "ll");
        assert_eq!((m.row, m.col), (0, 2));
        press(&mut m, &b, "w");
        assert_eq!((m.row, m.col), (0, 6));
        press(&mut m, &b, "e");
        assert_eq!((m.row, m.col), (0, 10));
        press(&mut m, &b, "b");
        assert_eq!((m.row, m.col), (0, 6));
        press(&mut m, &b, "$");
        assert_eq!(m.col, 20);
        press(&mut m, &b, "j");
        assert_eq!((m.row, m.col), (1, 10));
        press(&mut m, &b, "G");
        assert_eq!(m.row, 2);
        press(&mut m, &b, "gg0");
        assert_eq!((m.row, m.col), (0, 0));
        press(&mut m, &b, "kk");
        assert_eq!(m.row, 0);
    }
    #[test]
    fn selections_yank_char_line_and_block_text() {
        let b = buf(&["abcdef", "ghijkl", "mnopqr"]);
        let mut m = CopyMode::new(0, 1, 10);
        press(&mut m, &b, "v");
        press(&mut m, &b, "lj");
        assert_eq!(press(&mut m, &b, "y"), Outcome::Yank("bcdef\nghi".into()));
        let mut m = CopyMode::new(0, 1, 10);
        press(&mut m, &b, "V");
        press(&mut m, &b, "j");
        assert_eq!(
            press(&mut m, &b, "y"),
            Outcome::Yank("abcdef\nghijkl".into())
        );
        let mut m = CopyMode::new(0, 1, 10);
        assert_eq!(m.key(Key::Ctrl('v'), &b), Outcome::None);
        press(&mut m, &b, "jjll");
        assert_eq!(
            press(&mut m, &b, "y"),
            Outcome::Yank("bcd\nhij\nnop".into())
        );
        let mut m = CopyMode::new(1, 0, 10);
        assert_eq!(press(&mut m, &b, "y"), Outcome::Yank("ghijkl".into()));
    }
    #[test]
    fn search_finds_forward_and_back_with_smartcase_and_repeats() {
        let b = buf(&["one Two", "three two", "Two again"]);
        let mut m = CopyMode::new(0, 0, 10);
        press(&mut m, &b, "/two");
        m.key(Key::Enter, &b);
        assert_eq!((m.row, m.col), (0, 4));
        press(&mut m, &b, "n");
        assert_eq!((m.row, m.col), (1, 6));
        press(&mut m, &b, "n");
        assert_eq!((m.row, m.col), (2, 0));
        press(&mut m, &b, "n");
        assert_eq!((m.row, m.col), (0, 4), "wraps");
        press(&mut m, &b, "N");
        assert_eq!((m.row, m.col), (2, 0));
        press(&mut m, &b, "?Two");
        m.key(Key::Enter, &b);
        assert_eq!((m.row, m.col), (0, 4), "uppercase query is case-sensitive");
    }
    #[test]
    fn escape_clears_the_selection_first_then_exits_and_blank_buffers_are_safe() {
        let b = buf(&["abc"]);
        let mut m = CopyMode::new(0, 0, 10);
        press(&mut m, &b, "v");
        assert_eq!(m.key(Key::Esc, &b), Outcome::None);
        assert_eq!(m.key(Key::Esc, &b), Outcome::Exit);
        let empty: Vec<Vec<char>> = Vec::new();
        let mut m = CopyMode::new(5, 5, 10);
        assert_eq!(
            press(&mut m, &empty, "jlwbe$y"),
            Outcome::Yank(String::new())
        );
        assert!(!m.selected(0, 0));
    }
}

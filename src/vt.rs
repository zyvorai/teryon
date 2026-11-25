// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Terminal sizes as plain `u16`s on top of `vt100`, which wants `NonZeroU16`. A zero (from a
//! corrupt reply or file) becomes 1 instead of a panic.
use std::num::NonZeroU16;

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

pub fn parser(rows: u16, cols: u16, scrollback: usize) -> vt100::Parser {
    vt100::Parser::new(nz(rows), nz(cols), scrollback)
}

pub fn parser_with<C: vt100::Callbacks>(
    rows: u16,
    cols: u16,
    scrollback: usize,
    callbacks: C,
) -> vt100::Parser<C> {
    vt100::Parser::new_with_callbacks(nz(rows), nz(cols), scrollback, callbacks)
}

/// `(rows, cols)`.
pub fn size(screen: &vt100::Screen) -> (u16, u16) {
    let (rows, cols) = screen.size();
    (rows.get(), cols.get())
}

pub fn set_size(screen: &mut vt100::Screen, rows: u16, cols: u16) {
    screen.set_size(nz(rows), nz(cols));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every retained line with its absolute number, the way the daemon's capture walks them.
    fn numbered(p: &mut vt100::Parser) -> Vec<(u64, String)> {
        let (rows, cols) = size(p.screen());
        p.screen_mut().set_scrollback(usize::MAX);
        let max = p.screen().scrollback();
        let first = p.screen().scrolled_total().saturating_sub(max as u64);
        let mut lines = Vec::new();
        let mut offset = max;
        while offset > 0 {
            p.screen_mut().set_scrollback(offset);
            let take = offset.min(usize::from(rows));
            lines.extend(p.screen().rows(0, cols).take(take));
            offset = offset.saturating_sub(usize::from(rows));
        }
        p.screen_mut().set_scrollback(0);
        lines.extend(p.screen().rows(0, cols));
        (first..).zip(lines).collect()
    }
    fn number_of(p: &mut vt100::Parser, text: &str) -> u64 {
        numbered(p)
            .into_iter()
            .find(|(_, l)| l.trim_end() == text)
            .unwrap_or_else(|| panic!("{text} is gone"))
            .0
    }

    #[test]
    fn a_zero_size_is_raised_to_one_instead_of_panicking() {
        let mut p = parser(0, 0, 0);
        assert_eq!(size(p.screen()), (1, 1));
        p.process(b"ab\r\ncd");
        set_size(p.screen_mut(), 0, 5);
        assert_eq!(size(p.screen()), (1, 5));
    }

    #[test]
    fn a_line_keeps_its_number_through_scrolling_resizing_and_clearing_the_scrollback() {
        let mut p = parser(5, 20, 100);
        for i in 0..30 {
            p.process(format!("line{i}\r\n").as_bytes());
        }
        let before = number_of(&mut p, "line20");
        assert_eq!(before, 20);
        for rows in [3, 12, 2, 40, 5] {
            set_size(p.screen_mut(), rows, 20);
            assert_eq!(number_of(&mut p, "line20"), before, "{rows} rows");
        }
        // CSI 3 J drops the history; later lines go on counting from where they were.
        let top = p.screen().scrolled_total();
        p.process(b"\x1b[3J");
        assert_eq!(p.screen().scrolled_total(), top);
        p.process(b"after\r\n\r\n\r\n\r\n\r\n\r\n");
        assert!(p.screen().scrolled_total() > top);
    }

    #[test]
    fn the_alternate_screen_does_not_count_lines() {
        let mut p = parser(3, 20, 100);
        p.process(b"\x1b[?1049h1\r\n2\r\n3\r\n4\r\n5");
        assert_eq!(p.screen().scrolled_total(), 0);
        p.process(b"\x1b[?1049l");
        assert_eq!(p.screen().scrolled_total(), 0);
    }

    #[test]
    fn links_survive_the_trip_through_the_scrollback_and_a_resize() {
        let mut p = parser(3, 20, 50);
        p.process(b"\x1b]8;;https://a.example/x\x1b\\link\x1b]8;;\x1b\\\r\n\r\n\r\n\r\n");
        assert!(p.screen().hyperlinks().is_empty());
        p.screen_mut().set_scrollback(usize::MAX);
        let links = p.screen().hyperlinks();
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!((links[0].row, links[0].start, links[0].end), (0, 0, 4));
        p.screen_mut().set_scrollback(0);
        set_size(p.screen_mut(), 10, 30);
        assert_eq!(p.screen().hyperlinks()[0].url, "https://a.example/x");
    }

    /// The daemon parses whatever a program prints while clients resize the pane: no mix of the two
    /// may panic. Deterministic pseudo-random fragments.
    #[test]
    fn random_output_and_resizes_never_panic() {
        const PIECES: [&str; 28] = [
            "a",
            "word ",
            "\u{4e2d}",
            "e\u{301}",
            "\u{2764}\u{fe0f}",
            "\r",
            "\n",
            "\t",
            "\x08",
            "\x1b[2J",
            "\x1b[3J",
            "\x1b[H",
            "\x1b[5;5H",
            "\x1b[999;999H",
            "\x1b[2;4r",
            "\x1b[r",
            "\x1b[3L",
            "\x1b[3M",
            "\x1b[9@",
            "\x1b[9P",
            "\x1b[9X",
            "\x1b[40S",
            "\x1b[40T",
            "\x1bM",
            "\x1b[?1049h",
            "\x1b[?1049l",
            "\x1b]8;;http://x\x1b\\",
            "\x1bc",
        ];
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..300 {
            let mut p = parser(2 + next(20) as u16, 10 + next(40) as u16, next(30) as usize);
            for _ in 0..400 {
                if next(25) == 0 {
                    set_size(p.screen_mut(), 2 + next(20) as u16, 10 + next(40) as u16);
                } else {
                    p.process(PIECES[next(PIECES.len() as u64) as usize].as_bytes());
                }
                if next(40) == 0 {
                    let prev = p.screen().clone();
                    p.process(b"x\r\n");
                    let _ = p.screen().state_formatted();
                    let _ = p.screen().state_diff(&prev);
                    let _ = p.screen().hyperlinks();
                    let _ = numbered(&mut p);
                }
            }
        }
    }
}

// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Neutralise untrusted terminal text before it is shown or handed to a tool.
//!
//! Anything a remote process printed (screen text, a working directory sent with OSC 7, a title) is
//! attacker-controlled. Printed raw into another terminal, an escape sequence can rewrite the screen,
//! set the title, write the clipboard (OSC 52) or hide text; handed to an AI agent, invisible Unicode
//! (tag characters, bidi controls, zero-width characters) can smuggle instructions the user never sees.
//!
//! [`text`] keeps printable text, newlines and tabs and drops everything else, whole escape
//! sequences included (so the base64 of an OSC 52 write disappears with its introducer). It is
//! bounded: one line is cut at [`Limits::max_line_chars`] and the whole text at
//! [`Limits::max_total_chars`], so a hostile producer cannot make the consumer allocate without limit.
//! [`label`] is the single-line variant for titles, directories and names.

/// Size limits for [`text`].
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_line_chars: usize,
    pub max_total_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_line_chars: 4_000,
            max_total_chars: 1_000_000,
        }
    }
}

/// What [`text`] removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Whole escape sequences (CSI, OSC, DCS, APC, ...) dropped.
    pub escapes: usize,
    /// Other control characters dropped (BEL, backspace, lone CR, NUL, C1, DEL).
    pub controls: usize,
    /// Invisible or direction-changing characters dropped.
    pub invisible: usize,
    /// Lines cut at the line limit.
    pub cut_lines: usize,
    /// Characters left out after the total limit was reached.
    pub cut_chars: usize,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        *self == Self::default()
    }
    /// A one-line note for the reader, or `None` when nothing was removed.
    pub fn note(&self) -> Option<String> {
        if self.is_clean() {
            return None;
        }
        let mut parts = Vec::new();
        for (n, what) in [
            (self.escapes, "escape sequence"),
            (self.controls, "control character"),
            (self.invisible, "invisible character"),
            (self.cut_lines, "overlong line cut"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {what}{}", if n == 1 { "" } else { "s" }));
            }
        }
        if self.cut_chars > 0 {
            parts.push(format!(
                "{} characters beyond the size limit",
                self.cut_chars
            ));
        }
        Some(format!("[sanitised: {} removed]", parts.join(", ")))
    }
}

/// Characters that show nothing or change reading order. Tag characters (U+E0000..U+E007F) and the
/// variation-selector supplement are the "ASCII smuggling" channel; bidi controls reorder text.
pub fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}'
        | '\u{061C}'
        | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}'
        | '\u{E0000}'..='\u{E007F}'
        | '\u{E0100}'..='\u{E01EF}')
}

#[derive(Clone, Copy, PartialEq)]
enum St {
    Ground,
    Esc,
    /// ESC with intermediate bytes (0x20..0x2F) seen; waiting for the final byte.
    EscInter,
    Csi,
    /// OSC or a string sequence (DCS, APC, PM, SOS): runs to BEL or ST.
    Str {
        osc: bool,
    },
    StrEsc {
        osc: bool,
    },
}

/// A sequence that never ends is abandoned after this many characters, so one unterminated OSC
/// cannot swallow the rest of the output.
const MAX_SEQUENCE: usize = 8_192;

/// `input` with escape sequences, control characters and invisible characters removed.
pub fn text(input: &str, limits: &Limits) -> (String, Report) {
    let mut out = String::with_capacity(input.len().min(limits.max_total_chars + 64));
    let mut rep = Report::default();
    let mut st = St::Ground;
    let mut seq_len = 0usize;
    let mut line_chars = 0usize;
    let mut line_cut = false;
    let mut total = 0usize;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match st {
            St::Ground => {
                match c {
                    '\u{1b}' => {
                        st = St::Esc;
                        seq_len = 0;
                        continue;
                    }
                    '\u{9b}' => {
                        st = St::Csi;
                        seq_len = 0;
                        continue;
                    }
                    '\u{9d}' | '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => {
                        st = St::Str { osc: c == '\u{9d}' };
                        seq_len = 0;
                        continue;
                    }
                    _ => {}
                }
                if c == '\r' && chars.peek() == Some(&'\n') {
                    continue; // CRLF is an ordinary line end
                }
                if c == '\n' {
                    if total >= limits.max_total_chars {
                        rep.cut_chars += 1;
                        continue;
                    }
                    out.push('\n');
                    total += 1;
                    line_chars = 0;
                    line_cut = false;
                } else if c == '\t' || !(c.is_control() || is_invisible(c)) {
                    if total >= limits.max_total_chars {
                        rep.cut_chars += 1;
                        continue;
                    }
                    if line_chars >= limits.max_line_chars {
                        if !line_cut {
                            line_cut = true;
                            rep.cut_lines += 1;
                            out.push('…');
                        }
                        continue;
                    }
                    out.push(c);
                    total += 1;
                    line_chars += 1;
                } else if c.is_control() {
                    rep.controls += 1;
                } else {
                    rep.invisible += 1;
                }
            }
            St::Esc => {
                seq_len += 1;
                match c {
                    '[' => st = St::Csi,
                    ']' => st = St::Str { osc: true },
                    'P' | '_' | '^' | 'X' => st = St::Str { osc: false },
                    '\u{20}'..='\u{2f}' => st = St::EscInter,
                    '\u{1b}' => seq_len = 0, // ESC ESC: the first was abandoned
                    _ => {
                        rep.escapes += 1;
                        st = St::Ground;
                    }
                }
            }
            St::EscInter => {
                seq_len += 1;
                if ('\u{30}'..='\u{7e}').contains(&c) || seq_len > 16 {
                    rep.escapes += 1;
                    st = St::Ground;
                }
            }
            St::Csi => {
                seq_len += 1;
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    rep.escapes += 1;
                    st = St::Ground;
                } else if seq_len > 256 || c == '\u{1b}' {
                    // Malformed: give up on it. A fresh ESC starts the next sequence.
                    rep.escapes += 1;
                    st = if c == '\u{1b}' { St::Esc } else { St::Ground };
                    seq_len = 0;
                }
            }
            St::Str { osc } => {
                seq_len += 1;
                match c {
                    '\u{7}' | '\u{9c}' => {
                        rep.escapes += 1;
                        st = St::Ground;
                    }
                    '\u{1b}' => st = St::StrEsc { osc },
                    _ if seq_len > MAX_SEQUENCE => {
                        rep.escapes += 1;
                        st = St::Ground;
                    }
                    _ => {}
                }
            }
            St::StrEsc { osc } => {
                if c == '\\' {
                    rep.escapes += 1;
                    st = St::Ground;
                } else if c == '\u{1b}' {
                    st = St::StrEsc { osc };
                } else {
                    // ESC inside a string that is not ST: the string ends and the ESC begins
                    // another sequence whose second byte is `c`.
                    rep.escapes += 1;
                    st = match c {
                        '[' => St::Csi,
                        ']' => St::Str { osc: true },
                        'P' | '_' | '^' | 'X' => St::Str { osc: false },
                        _ => St::Ground,
                    };
                    seq_len = 0;
                }
            }
        }
    }
    if st != St::Ground {
        rep.escapes += 1; // an unfinished sequence at the end
    }
    (out, rep)
}

/// A single-line, printable version of `input` of at most `max_chars` characters, for titles,
/// directories and names. Whatever [`text`] drops is dropped; newlines and tabs become spaces.
pub fn label(input: &str, max_chars: usize) -> String {
    let limits = Limits {
        max_line_chars: max_chars.max(1),
        max_total_chars: max_chars.max(1) * 4,
    };
    let (clean, _) = text(input, &limits);
    let flat: String = clean
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .collect();
    let flat = flat.trim();
    if flat.chars().count() > max_chars {
        let mut s: String = flat.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        flat.to_string()
    }
}

/// [`text`] with the default limits, followed by [`Report::note`] when something was removed.
pub fn text_with_note(input: &str) -> String {
    let (mut out, rep) = text(input, &Limits::default());
    if let Some(note) = rep.note() {
        out.push('\n');
        out.push_str(&note);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(s: &str) -> String {
        text(s, &Limits::default()).0
    }

    #[test]
    fn plain_text_newlines_and_tabs_pass_through() {
        let s = "héllo wörld 日本語 🚀\n\tindented\nlast";
        let (out, rep) = text(s, &Limits::default());
        assert_eq!(out, s);
        assert!(rep.is_clean());
        assert_eq!(rep.note(), None);
    }

    #[test]
    fn colours_and_cursor_moves_are_removed_with_their_arguments() {
        assert_eq!(clean("\x1b[1;31mred\x1b[0m \x1b[2J\x1b[Hdone"), "red done");
    }

    #[test]
    fn an_osc_52_clipboard_write_vanishes_with_its_payload() {
        let evil = "before\x1b]52;c;cm0gLXJmIH4K\x07after";
        assert_eq!(clean(evil), "beforeafter");
        let st = "a\x1b]52;c;cm0gLXJmIH4K\x1b\\b";
        assert_eq!(clean(st), "ab");
        // The 8-bit introducer does the same.
        assert_eq!(clean("a\u{9d}52;c;AAAA\u{9c}b"), "ab");
    }

    #[test]
    fn title_hyperlink_and_dcs_sequences_are_removed() {
        assert_eq!(clean("x\x1b]0;pwned\x07y"), "xy");
        assert_eq!(clean("\x1b]8;;http://evil/\x07click\x1b]8;;\x07"), "click");
        assert_eq!(clean("a\x1bP1$r0m\x1b\\b\x1b_Gf=100;AAAA\x1b\\c"), "abc");
    }

    #[test]
    fn answerback_style_and_misc_controls_are_removed() {
        assert_eq!(clean("a\x07b\x08c\x00d\x7fe\u{85}f"), "abcdef");
        // A carriage return would let a line overwrite the one before it; CRLF is just a newline.
        assert_eq!(clean("ok\rFAKE"), "okFAKE");
        assert_eq!(clean("one\r\ntwo"), "one\ntwo");
    }

    #[test]
    fn invisible_and_bidi_characters_are_removed() {
        // Tag characters spell "hi" invisibly; the bidi override flips what follows.
        let tags: String = "hi"
            .chars()
            .map(|c| char::from_u32(0xE0000 + c as u32).unwrap())
            .collect();
        let s = format!("a{tags}b\u{202E}c\u{200B}d\u{FEFF}e\u{2066}f");
        let (out, rep) = text(&s, &Limits::default());
        assert_eq!(out, "abcdef");
        assert_eq!(rep.invisible, 6);
    }

    #[test]
    fn emoji_variation_selector_and_joiners_inside_emoji_survive_where_needed() {
        // U+FE0F (emoji presentation) is kept; the supplement used for smuggling is not.
        assert_eq!(clean("\u{2764}\u{FE0F}"), "\u{2764}\u{FE0F}");
        assert_eq!(clean("a\u{E0100}b"), "ab");
    }

    #[test]
    fn a_truncated_or_malformed_sequence_does_not_eat_the_rest() {
        assert_eq!(clean("a\x1b[31"), "a");
        assert_eq!(clean("a\x1b[3\x1b[1mb"), "ab");
        assert_eq!(clean("a\x1b]0;title\x1b[1mb"), "ab");
        assert_eq!(clean("a\x1b"), "a");
        // An unterminated OSC is abandoned after the sequence limit, so later lines survive.
        let long = format!("\x1b]0;{}\nvisible", "x".repeat(MAX_SEQUENCE + 10));
        assert!(clean(&long).ends_with("\nvisible"));
    }

    #[test]
    fn a_huge_line_is_cut_once_and_the_total_is_bounded() {
        let limits = Limits {
            max_line_chars: 10,
            max_total_chars: 25,
        };
        let input = format!(
            "{}\nshort\n{}",
            "a".repeat(1_000_000),
            "b".repeat(1_000_000)
        );
        let (out, rep) = text(&input, &limits);
        assert!(out.chars().count() <= 25 + 3, "{}", out.chars().count());
        assert!(out.starts_with("aaaaaaaaaa…\nshort\n"));
        assert_eq!(rep.cut_lines, 1);
        assert!(rep.cut_chars > 0);
        assert!(rep.note().unwrap().contains("size limit"));
    }

    #[test]
    fn labels_are_one_line_printable_and_short() {
        assert_eq!(label("a\nb\tc", 20), "a b c");
        assert_eq!(label("\x1b]0;x\x07/home/u\x1b[31m\u{202E}", 20), "/home/u");
        assert_eq!(label("abcdefghij", 5), "abcd…");
        assert_eq!(label("\x1b[2J\x07", 5), "");
    }

    #[test]
    fn the_note_counts_what_was_removed() {
        let out = text_with_note("a\x1b[31mb\x07c\u{202E}");
        assert!(out.starts_with("abc\n[sanitised: "), "{out}");
        assert!(out.contains("1 escape sequence"));
        assert!(out.contains("1 control character"));
        assert!(out.contains("1 invisible character"));
    }

    #[test]
    fn output_never_contains_a_control_character_other_than_newline_and_tab() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..300 {
            let s: String = (0..200)
                .map(|_| match next() % 6 {
                    0 => '\u{1b}',
                    1 => char::from_u32((next() % 0x20) as u32).unwrap(),
                    2 => char::from_u32(0x80 + (next() % 0x20) as u32).unwrap(),
                    3 => '[',
                    4 => ']',
                    _ => char::from_u32(0x20 + (next() % 0x5f) as u32).unwrap(),
                })
                .collect();
            let out = clean(&s);
            assert!(
                out.chars()
                    .all(|c| c == '\n' || c == '\t' || !c.is_control()),
                "{s:?} -> {out:?}"
            );
        }
    }
}

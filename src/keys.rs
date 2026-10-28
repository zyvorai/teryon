//! Encode input for xterm-compatible remote programs. Local prefix is Ctrl+Space.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub fn encode(key: KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    encode_with(key, app_cursor, 0)
}
/// `Some(CSI code ; modifiers u)` for the keys the Kitty protocol's "disambiguate" flag changes:
/// Esc, and a letter, digit or symbol pressed with Ctrl or Alt. Plain text, Shift-only text and
/// the other special keys keep their usual encodings.
fn kitty_disambiguated(key: KeyEvent) -> Option<Vec<u8>> {
    let m = key.modifiers;
    let (shift, alt, ctrl) = (
        m.contains(KeyModifiers::SHIFT),
        m.contains(KeyModifiers::ALT),
        m.contains(KeyModifiers::CONTROL),
    );
    let mods = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let (code, mods) = match key.code {
        KeyCode::Esc => (27, mods),
        // Shift+Tab is Tab with the shift bit.
        KeyCode::BackTab => (9, ((mods - 1) | 1) + 1),
        KeyCode::Char(c) if alt || ctrl => {
            // The key is named by its unshifted character; the app maps Ctrl+/ to `_` for the shell.
            let c = if ctrl && c == '_' { '/' } else { c };
            (c.to_lowercase().next().unwrap_or(c) as u32, mods)
        }
        _ => return None,
    };
    Some(
        if mods == 1 {
            format!("\x1b[{code}u")
        } else {
            format!("\x1b[{code};{mods}u")
        }
        .into_bytes(),
    )
}
/// The control bytes xterm sends for Ctrl with a character outside `@`..`_`: Ctrl+/ and Ctrl+7 are
/// 0x1f (undo in readline), Ctrl+? and Ctrl+8 are DEL, Ctrl+2 and Ctrl+` are NUL, Ctrl+3..6 are
/// ESC, FS, GS and RS (as Ctrl+[, Ctrl+\\, Ctrl+] and Ctrl+^), Ctrl+~ is RS.
fn xterm_control(c: char) -> Option<u8> {
    match c {
        '/' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        '2' | '`' => Some(0),
        '3'..='6' => Some(0x1b + (c as u8 - b'3')),
        '~' => Some(0x1e),
        _ => None,
    }
}
/// Like `encode`, for a program that pushed Kitty keyboard `kitty` flags (see `kittykeys`).
pub fn encode_with(key: KeyEvent, app_cursor: bool, kitty: u8) -> Option<Vec<u8>> {
    if kitty & crate::kittykeys::DISAMBIGUATE != 0 {
        if let Some(bytes) = kitty_disambiguated(key) {
            return Some(bytes);
        }
    }
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut bytes = match key.code {
        KeyCode::Char(c) if control => {
            let upper = c.to_ascii_uppercase();
            if ('@'..='_').contains(&upper) {
                vec![upper as u8 & 0x1f]
            } else if c == ' ' {
                vec![0]
            } else {
                vec![xterm_control(c)?]
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![27],
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Right
        | KeyCode::Left
        | KeyCode::Home
        | KeyCode::End => {
            let code = match key.code {
                KeyCode::Up => 'A',
                KeyCode::Down => 'B',
                KeyCode::Right => 'C',
                KeyCode::Left => 'D',
                KeyCode::Home => 'H',
                _ => 'F',
            };
            let mods = 1
                + u8::from(key.modifiers.contains(KeyModifiers::SHIFT))
                + 2 * u8::from(alt)
                + 4 * u8::from(control);
            if mods > 1 {
                return Some(format!("\x1b[1;{mods}{code}").into_bytes());
            }
            format!("\x1b{}{code}", if app_cursor { 'O' } else { '[' }).into_bytes()
        }
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::F(n @ 1..=4) => vec![27, b'O', b'P' + n - 1],
        KeyCode::F(n @ 5..=12) => format!(
            "\x1b[{}~",
            [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]
        )
        .into_bytes(),
        _ => return None,
    };
    if alt {
        bytes.insert(0, 27);
    }
    Some(bytes)
}
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    // Untrusted clipboard data cannot terminate bracketed-paste mode early.
    let text = text.replace('\x1b', "");
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctrl_with_punctuation_and_digits_sends_the_xterm_bytes() {
        use crossterm::event::{KeyCode as K, KeyEvent as E, KeyModifiers as M};
        let enc = |c| encode(E::new(K::Char(c), M::CONTROL), false);
        assert_eq!(enc('/').unwrap(), [0x1f]);
        assert_eq!(enc('_').unwrap(), [0x1f]);
        assert_eq!(enc('7').unwrap(), [0x1f]);
        assert_eq!(enc('?').unwrap(), [0x7f]);
        assert_eq!(enc('8').unwrap(), [0x7f]);
        assert_eq!(enc('2').unwrap(), [0]);
        assert_eq!(enc('3').unwrap(), [0x1b]);
        assert_eq!(enc('4').unwrap(), [0x1c]);
        assert_eq!(enc('5').unwrap(), [0x1d]);
        assert_eq!(enc('6').unwrap(), [0x1e]);
        // Ctrl+Alt+/ is ESC then the byte; keys xterm has no byte for stay unsent.
        assert_eq!(
            encode(E::new(K::Char('/'), M::CONTROL | M::ALT), false).unwrap(),
            [27, 0x1f]
        );
        assert_eq!(enc('.'), None);
        assert_eq!(enc('9'), None);
    }
    #[test]
    fn kitty_disambiguate_changes_only_esc_and_modified_keys() {
        use crossterm::event::{KeyCode as K, KeyEvent as E, KeyModifiers as M};
        let enc = |code, mods| encode_with(E::new(code, mods), false, 1);
        assert_eq!(enc(K::Esc, M::NONE).unwrap(), b"\x1b[27u");
        assert_eq!(enc(K::Char('a'), M::CONTROL).unwrap(), b"\x1b[97;5u");
        assert_eq!(enc(K::Char('b'), M::ALT).unwrap(), b"\x1b[98;3u");
        // Shift is named by the unshifted key plus the shift bit.
        assert_eq!(enc(K::Char('B'), M::ALT | M::SHIFT).unwrap(), b"\x1b[98;4u");
        assert_eq!(
            enc(K::Char('a'), M::CONTROL | M::ALT).unwrap(),
            b"\x1b[97;7u"
        );
        assert_eq!(enc(K::Char(' '), M::CONTROL).unwrap(), b"\x1b[32;5u");
        // Combinations the legacy encoding could not express at all.
        assert_eq!(enc(K::Char('1'), M::CONTROL).unwrap(), b"\x1b[49;5u");
        // The app's Ctrl+/ arrives as Ctrl+_ and is reported as the key it is.
        assert_eq!(enc(K::Char('_'), M::CONTROL).unwrap(), b"\x1b[47;5u");
        assert_eq!(enc(K::BackTab, M::SHIFT).unwrap(), b"\x1b[9;2u");
        // Untouched: text, Shift-only text, Enter, Tab, Backspace, arrows, function keys.
        assert_eq!(enc(K::Char('a'), M::NONE).unwrap(), b"a");
        assert_eq!(enc(K::Char('A'), M::SHIFT).unwrap(), b"A");
        assert_eq!(enc(K::Enter, M::NONE).unwrap(), b"\r");
        assert_eq!(enc(K::Tab, M::NONE).unwrap(), b"\t");
        assert_eq!(enc(K::Backspace, M::NONE).unwrap(), [127]);
        assert_eq!(enc(K::Up, M::NONE).unwrap(), b"\x1b[A");
        assert_eq!(enc(K::Up, M::CONTROL).unwrap(), b"\x1b[1;5A");
        assert_eq!(enc(K::F(5), M::NONE).unwrap(), b"\x1b[15~");
    }
    #[test]
    fn without_the_flag_nothing_changes() {
        use crossterm::event::{KeyCode as K, KeyEvent as E, KeyModifiers as M};
        for (code, mods, want) in [
            (K::Esc, M::NONE, &b"\x1b"[..]),
            (K::Char('a'), M::CONTROL, &[1u8][..]),
            (K::Char('b'), M::ALT, &b"\x1bb"[..]),
            (K::BackTab, M::SHIFT, &b"\x1b[Z"[..]),
        ] {
            assert_eq!(encode(E::new(code, mods), false).unwrap(), want);
            assert_eq!(encode_with(E::new(code, mods), false, 0).unwrap(), want);
            // Flags this terminal does not honor change nothing either.
            assert_eq!(
                encode_with(E::new(code, mods), false, 0b1110).unwrap(),
                want
            );
        }
    }
    #[test]
    fn controls_unicode_and_arrows() {
        assert_eq!(
            encode(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                false
            ),
            Some(vec![3])
        );
        assert_eq!(
            encode(KeyEvent::new(KeyCode::Char('λ'), KeyModifiers::NONE), false),
            Some("λ".as_bytes().to_vec())
        );
        assert_eq!(
            encode(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL), false),
            Some(b"\x1b[1;5D".to_vec())
        );
    }
    #[test]
    fn clipboard_cannot_escape_bracketed_paste() {
        assert_eq!(
            paste("hi\x1b[201~bad", true),
            b"\x1b[200~hi[201~bad\x1b[201~"
        );
    }
}

/// What happened to the mouse, for `mouse_report`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    /// Movement; `button` says which button is held, if any.
    Motion,
    WheelUp,
    WheelDown,
}
/// Bytes an xterm-compatible program expects for a mouse event, or `None` if its current mouse
/// mode does not ask for this kind of event. `button`: 0 left, 1 middle, 2 right. `row`/`col` are
/// zero-based cells.
pub fn mouse_report(
    mode: vt100::MouseProtocolMode,
    encoding: vt100::MouseProtocolEncoding,
    kind: MouseKind,
    button: Option<u8>,
    mods: (bool, bool, bool),
    row: u16,
    col: u16,
) -> Option<Vec<u8>> {
    use vt100::MouseProtocolMode as M;
    let allowed = match (mode, kind) {
        (M::None, _) => false,
        (M::Press, MouseKind::Press) => true,
        (M::Press, _) => false,
        (_, MouseKind::Motion) => {
            mode == M::AnyMotion || (mode == M::ButtonMotion && button.is_some())
        }
        _ => true,
    };
    if !allowed {
        return None;
    }
    let (shift, alt, ctrl) = mods;
    let mut code: u16 = match kind {
        MouseKind::WheelUp => 64,
        MouseKind::WheelDown => 65,
        MouseKind::Motion => 32 + u16::from(button.unwrap_or(3)),
        _ => u16::from(button.unwrap_or(0)),
    };
    let sgr = encoding == vt100::MouseProtocolEncoding::Sgr;
    if kind == MouseKind::Release && !sgr {
        code = 3;
    }
    if mode != M::Press {
        code += u16::from(shift) * 4 + u16::from(alt) * 8 + u16::from(ctrl) * 16;
    }
    let (x, y) = (col + 1, row + 1);
    if sgr {
        let last = if kind == MouseKind::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{x};{y}{last}").into_bytes());
    }
    let utf8 = encoding == vt100::MouseProtocolEncoding::Utf8;
    let limit = if utf8 { 2015 } else { 223 };
    if x > limit || y > limit {
        return None;
    }
    let mut out = b"\x1b[M".to_vec();
    for v in [code, x, y] {
        let c = char::from_u32(u32::from(v) + 32)?;
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        if !utf8 && v + 32 > 255 {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod mouse_tests {
    use super::*;
    use vt100::{MouseProtocolEncoding as E, MouseProtocolMode as M};
    const NONE: (bool, bool, bool) = (false, false, false);
    #[test]
    fn sgr_press_release_wheel_and_modifiers() {
        let b = |k, btn, mods, r, c| mouse_report(M::PressRelease, E::Sgr, k, btn, mods, r, c);
        assert_eq!(
            b(MouseKind::Press, Some(0), NONE, 4, 9).unwrap(),
            b"\x1b[<0;10;5M"
        );
        assert_eq!(
            b(MouseKind::Release, Some(0), NONE, 4, 9).unwrap(),
            b"\x1b[<0;10;5m"
        );
        assert_eq!(
            b(MouseKind::Press, Some(2), (true, false, true), 0, 0).unwrap(),
            b"\x1b[<22;1;1M"
        );
        assert_eq!(
            b(MouseKind::WheelUp, None, NONE, 1, 1).unwrap(),
            b"\x1b[<64;2;2M"
        );
        assert_eq!(
            b(MouseKind::WheelDown, None, NONE, 1, 1).unwrap(),
            b"\x1b[<65;2;2M"
        );
    }
    #[test]
    fn default_encoding_uses_offset_bytes_and_release_code_3() {
        let press = mouse_report(
            M::PressRelease,
            E::Default,
            MouseKind::Press,
            Some(0),
            NONE,
            0,
            0,
        );
        assert_eq!(press.unwrap(), vec![0x1b, b'[', b'M', 32, 33, 33]);
        let release = mouse_report(
            M::PressRelease,
            E::Default,
            MouseKind::Release,
            Some(0),
            NONE,
            0,
            0,
        );
        assert_eq!(release.unwrap(), vec![0x1b, b'[', b'M', 35, 33, 33]);
        assert!(mouse_report(
            M::PressRelease,
            E::Default,
            MouseKind::Press,
            Some(0),
            NONE,
            0,
            300
        )
        .is_none());
        let wide = mouse_report(
            M::PressRelease,
            E::Utf8,
            MouseKind::Press,
            Some(0),
            NONE,
            0,
            300,
        )
        .unwrap();
        assert!(wide.len() > 6);
    }
    #[test]
    fn modes_filter_which_events_are_reported() {
        let r = |m, k, b| mouse_report(m, E::Sgr, k, b, NONE, 0, 0);
        assert!(r(M::None, MouseKind::Press, Some(0)).is_none());
        assert!(r(M::Press, MouseKind::Release, Some(0)).is_none());
        assert!(r(M::Press, MouseKind::WheelUp, None).is_none());
        assert!(r(M::PressRelease, MouseKind::Motion, Some(0)).is_none());
        assert_eq!(
            r(M::ButtonMotion, MouseKind::Motion, Some(0)).unwrap(),
            b"\x1b[<32;1;1M"
        );
        assert!(r(M::ButtonMotion, MouseKind::Motion, None).is_none());
        assert_eq!(
            r(M::AnyMotion, MouseKind::Motion, None).unwrap(),
            b"\x1b[<35;1;1M"
        );
    }
}

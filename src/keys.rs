//! Encode input for xterm-compatible remote programs. Local prefix is Ctrl+Space.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub fn encode(key: KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
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
                return None;
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

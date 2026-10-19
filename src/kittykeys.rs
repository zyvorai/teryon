//! The Kitty keyboard protocol's flag stack: which key encodings a program in a pane asked for.
//! Only the "disambiguate escape codes" flag is acted on; the others are accepted and not reported
//! back, so a program never believes it will get something this terminal does not send.

/// Flag 1: Esc and keys with modifiers are sent as `CSI code ; mods u`.
pub const DISAMBIGUATE: u8 = 1;
/// The flags this terminal honors.
const SUPPORTED: u8 = DISAMBIGUATE;
/// Entries kept per stack; a program that pushes more loses the oldest.
const MAX_DEPTH: usize = 32;

/// One stack per screen: the main screen and the alternate screen do not share flags.
#[derive(Clone, Debug, Default)]
pub struct KittyKeys {
    main: Vec<u8>,
    alt: Vec<u8>,
}

impl KittyKeys {
    fn stack(&mut self, alt: bool) -> &mut Vec<u8> {
        if alt {
            &mut self.alt
        } else {
            &mut self.main
        }
    }
    /// The flags in force on the given screen.
    pub fn flags(&self, alt: bool) -> u8 {
        let stack = if alt { &self.alt } else { &self.main };
        stack.last().copied().unwrap_or(0)
    }
    /// `CSI > flags u`
    pub fn push(&mut self, alt: bool, flags: u8) {
        let stack = self.stack(alt);
        if stack.len() >= MAX_DEPTH {
            stack.remove(0);
        }
        stack.push(flags & SUPPORTED);
    }
    /// `CSI < n u`
    pub fn pop(&mut self, alt: bool, n: usize) {
        let stack = self.stack(alt);
        let keep = stack.len().saturating_sub(n.max(1));
        stack.truncate(keep);
    }
    /// `CSI = flags ; mode u`: 1 sets, 2 adds, 3 removes. Changes the top entry.
    pub fn set(&mut self, alt: bool, flags: u8, mode: u8) {
        let stack = self.stack(alt);
        if stack.is_empty() {
            stack.push(0);
        }
        let top = stack.last_mut().unwrap();
        let flags = flags & SUPPORTED;
        match mode {
            2 => *top |= flags,
            3 => *top &= !flags,
            _ => *top = flags,
        }
    }
    /// Leaving the alternate screen ends whatever the full-screen program asked for, even if it
    /// crashed before popping.
    pub fn alt_screen_left(&mut self) {
        self.alt.clear();
    }
    /// The reply to `CSI ? u`.
    pub fn report(&self, alt: bool) -> String {
        format!("\x1b[?{}u", self.flags(alt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_and_query_follow_the_spec() {
        let mut k = KittyKeys::default();
        assert_eq!(k.report(false), "\x1b[?0u");
        k.push(false, 1);
        assert_eq!(k.report(false), "\x1b[?1u");
        k.push(false, 0);
        assert_eq!(k.flags(false), 0);
        k.pop(false, 1);
        assert_eq!(k.flags(false), 1);
        k.pop(false, 5);
        assert_eq!(k.flags(false), 0);
        k.pop(false, 1);
    }
    #[test]
    fn unsupported_flags_are_not_reported_back() {
        let mut k = KittyKeys::default();
        k.push(false, 0b1_1111);
        assert_eq!(k.flags(false), DISAMBIGUATE);
        k.push(false, 0b1_1110);
        assert_eq!(k.flags(false), 0);
    }
    #[test]
    fn set_can_replace_add_and_remove() {
        let mut k = KittyKeys::default();
        k.set(false, 1, 1);
        assert_eq!(k.flags(false), 1);
        k.set(false, 1, 3);
        assert_eq!(k.flags(false), 0);
        k.set(false, 1, 2);
        assert_eq!(k.flags(false), 1);
        k.pop(false, 1);
        assert_eq!(k.flags(false), 0, "set on an empty stack made one entry");
    }
    #[test]
    fn screens_have_separate_stacks_and_leaving_the_alt_screen_clears_its_own() {
        let mut k = KittyKeys::default();
        k.push(false, 1);
        k.push(true, 1);
        k.pop(true, 1);
        assert_eq!((k.flags(false), k.flags(true)), (1, 0));
        k.push(true, 1);
        k.alt_screen_left();
        assert_eq!((k.flags(false), k.flags(true)), (1, 0));
    }
    #[test]
    fn the_stack_is_bounded() {
        let mut k = KittyKeys::default();
        for _ in 0..200 {
            k.push(false, 1);
        }
        k.pop(false, MAX_DEPTH);
        assert_eq!(k.flags(false), 0);
    }
}

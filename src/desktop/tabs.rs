//! News from background tabs: a dot on a tab whose panes printed something, a different mark when an
//! AI agent in it needs you.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mark {
    #[default]
    None,
    /// New output since you last looked.
    Output,
    /// An AI agent is waiting for you.
    Attention,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Activity {
    pub mark: Mark,
    /// Sum of the panes' screen revisions at the last look; `None` until there is one to compare with.
    last: Option<u64>,
}
impl Activity {
    /// Note the background tab's current state; the first look only sets the baseline.
    pub fn observe(&mut self, revisions: u64, agent_waiting: bool) {
        if agent_waiting {
            self.mark = Mark::Attention;
        } else if self.last.is_some_and(|l| l != revisions) && self.mark == Mark::None {
            self.mark = Mark::Output;
        }
        self.last = Some(revisions);
    }
    /// The tab is on screen now: nothing is unseen and the next background look starts fresh.
    pub fn seen(&mut self) {
        *self = Self::default();
    }
    /// The text put in front of a tab's name.
    pub fn prefix(&self) -> &'static str {
        match self.mark {
            Mark::None => "",
            Mark::Output => "● ",
            Mark::Attention => "▲ ",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_marks_a_tab_after_the_first_look_and_attention_wins_until_seen() {
        let mut a = Activity::default();
        a.observe(5, false);
        assert_eq!(a.mark, Mark::None, "the first look is only a baseline");
        a.observe(5, false);
        assert_eq!(a.mark, Mark::None);
        a.observe(9, false);
        assert_eq!(a.mark, Mark::Output);
        assert_eq!(a.prefix(), "● ");
        a.observe(9, true);
        assert_eq!(a.mark, Mark::Attention);
        a.observe(12, false);
        assert_eq!(a.mark, Mark::Attention, "stays until the tab is looked at");
        a.seen();
        assert_eq!((a.mark, a.prefix()), (Mark::None, ""));
        a.observe(30, false);
        assert_eq!(a.mark, Mark::None, "looking restarts the baseline");
    }
}

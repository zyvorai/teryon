//! Right-click a command in the terminal: copy its command or output, run it again, or ask the assistant.
use super::*;
use super::term::Then;
use crate::protocol::Mark;

/// A command block found under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BlockRef {
    pub pane: u64,
    /// First output line (the `C` mark).
    pub c_line: u64,
    /// The line of the next prompt (the `D` mark); `None` while the command still runs.
    pub d_line: Option<u64>,
    pub cmd: Option<String>,
    pub exit: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BlockPick {
    CopyCommand,
    CopyOutput,
    Rerun,
    Ask,
}

/// The block whose command line or output covers absolute line `line`, from a pane's marks.
pub(super) fn block_at(pane: u64, marks: &[Mark], line: u64) -> Option<BlockRef> {
    let mut open: Option<&Mark> = None;
    let mut found: Option<BlockRef> = None;
    for m in marks {
        match m.kind {
            'C' => {
                // A newer command start drops one whose end was lost.
                open = Some(m);
            }
            'D' => {
                if let Some(c) = open.take() {
                    if c.line.saturating_sub(1) <= line && line < m.line && c.line < m.line {
                        found = Some(BlockRef {
                            pane,
                            c_line: c.line,
                            d_line: Some(m.line),
                            cmd: c.cmd.clone(),
                            exit: m.exit,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(c) = open {
        if c.line.saturating_sub(1) <= line {
            found = Some(BlockRef {
                pane,
                c_line: c.line,
                d_line: None,
                cmd: c.cmd.clone(),
                exit: None,
            });
        }
    }
    found
}

impl Desktop {
    /// Remember which block a right-click landed on, so the menu can offer it.
    pub(super) fn note_block_click(&mut self, id: u64, pos: Pos2, rect: Rect) {
        let ch = self.font_size * self.cfg.line_height;
        let (_, pad_top, _, _) = self.cfg.insets();
        let rows = usize::from(self.view_rows.get(&id).copied().unwrap_or(24));
        let row = (((pos.y - rect.top() - pad_top) / ch).floor().max(0.) as usize).min(rows.saturating_sub(1));
        let line = self.view_top_line(id) + row as u64;
        self.block_menu = self
            .panes
            .get(&id)
            .and_then(|p| block_at(id, &p.marks, line));
    }

    /// The menu entries for the block under the last right-click; `Some` when one was chosen.
    pub(super) fn block_menu_items(&self, ui: &mut egui::Ui, t: &Theme) -> Option<BlockPick> {
        let b = self.block_menu.as_ref()?;
        let mut pick = None;
        let (text, bad) = match (b.d_line, b.exit) {
            (None, _) => ("Command · running".to_string(), false),
            (Some(_), Some(0) | None) => ("Command · finished".to_string(), false),
            (Some(_), Some(c)) => (format!("Command · failed ({c})"), true),
        };
        ui.label(
            RichText::new(text)
                .size(11.)
                .color(color(if bad { t.warning } else { t.muted })),
        );
        let finished = b.d_line.is_some();
        for (label, p, enabled) in [
            ("Copy command", BlockPick::CopyCommand, b.cmd.is_some()),
            ("Copy output", BlockPick::CopyOutput, finished),
            ("Run it again", BlockPick::Rerun, b.cmd.is_some() && !self.readonly),
            ("Ask AI about it", BlockPick::Ask, finished),
        ] {
            if ui.add_enabled(enabled, egui::Button::new(label).frame(false)).clicked() {
                pick = Some(p);
                ui.close_menu();
            }
        }
        ui.separator();
        pick
    }

    pub(super) fn run_block_pick(&mut self, pick: BlockPick) {
        let Some(b) = self.block_menu.clone() else {
            return;
        };
        match pick {
            BlockPick::CopyCommand => {
                if let Some(cmd) = b.cmd {
                    self.copy_text = Some(cmd);
                    self.toast("Copied the command");
                }
            }
            BlockPick::Rerun => {
                if self.readonly {
                    self.toast("Read only");
                    return;
                }
                if let Some(cmd) = b.cmd {
                    self.insert_command(b.pane, &cmd, true);
                }
            }
            BlockPick::CopyOutput | BlockPick::Ask => {
                let Some(d) = b.d_line else { return };
                let ask = pick == BlockPick::Ask;
                if ask {
                    self.block_ask = Some((b.cmd.clone().unwrap_or_default(), b.exit));
                }
                if self.scrollback.get(&b.pane).is_some_and(|s| !s.loading) {
                    let text = self.block_text(b.pane, b.c_line, d);
                    self.deliver_block(b.pane, text, ask);
                } else if self.online {
                    let then = Then::Block(b.c_line, d, ask);
                    self.scrollback
                        .entry(b.pane)
                        .or_insert_with(|| Self::loading_view(Some(then)))
                        .then = Some(then);
                    self.send(Command::Capture(b.pane, CapturePurpose::Scrollback(b.pane)));
                } else {
                    self.toast("Offline: no history to read from");
                }
            }
        }
    }

    /// A block's output has arrived: copy it, or send it with its command to the assistant.
    pub(super) fn deliver_block(&mut self, id: u64, text: Result<String, &'static str>, ask: bool) {
        if !ask {
            match text {
                Ok(t) if !t.is_empty() => {
                    let n = t.lines().count();
                    self.copy_text = Some(t);
                    self.toast(format!("Copied the output ({n} line(s))"));
                }
                Ok(_) => self.toast("That command printed nothing"),
                Err(e) => self.toast(e),
            }
            return;
        }
        let (cmd, exit) = self.block_ask.take().unwrap_or_default();
        let output = text.unwrap_or_default();
        self.ai_ask_block(id, &cmd, exit, &output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(kind: char, line: u64, exit: Option<i32>, cmd: Option<&str>) -> Mark {
        Mark { kind, line, exit, cmd: cmd.map(str::to_string), ..Default::default() }
    }
    #[test]
    fn the_block_under_a_line_is_found_with_its_command_and_status() {
        let marks = vec![
            m('A', 9, None, None),
            m('C', 11, None, Some("make")),
            m('D', 15, Some(2), None),
            m('A', 15, None, None),
            m('C', 17, None, Some("ls")),
        ];
        let b = block_at(1, &marks, 12).unwrap();
        assert_eq!((b.c_line, b.d_line, b.exit), (11, Some(15), Some(2)));
        assert_eq!(b.cmd.as_deref(), Some("make"));
        assert_eq!(block_at(1, &marks, 10).unwrap().c_line, 11, "the command line belongs to its block");
        let running = block_at(1, &marks, 20).unwrap();
        assert_eq!((running.c_line, running.d_line), (17, None));
        assert_eq!(block_at(1, &marks, 3), None);
        assert_eq!(block_at(1, &[], 3), None);
    }
    #[test]
    fn menu_choices_copy_the_command_run_it_again_and_fetch_the_output_for_copying() {
        use crate::desktop::tests::{app_with_screen, drain};
        let (mut app, rx) = app_with_screen("");
        app.block_menu = Some(BlockRef {
            pane: 1,
            c_line: 3,
            d_line: Some(6),
            cmd: Some("make test".into()),
            exit: Some(1),
        });
        app.run_block_pick(BlockPick::CopyCommand);
        assert_eq!(app.copy_text.take().as_deref(), Some("make test"));
        drain(&rx);
        app.run_block_pick(BlockPick::Rerun);
        assert!(matches!(&drain(&rx)[..], [Command::Input { pane: 1, data }] if data == b"make test\r"));
        // The output is not loaded yet: it is requested, and copied when the history arrives.
        app.run_block_pick(BlockPick::CopyOutput);
        assert!(matches!(&drain(&rx)[..], [Command::Capture(1, CapturePurpose::Scrollback(1))]));
        let text: String = (0..10).map(|i| format!("line {i}\n")).collect();
        app.finish_scrollback(1, &text, 0);
        assert_eq!(app.copy_text.take().as_deref(), Some("line 3\nline 4\nline 5"));
        // Read only: nothing is typed.
        app.readonly = true;
        let enabled = app.block_menu.as_ref().is_some_and(|b| b.cmd.is_some() && !app.readonly);
        assert!(!enabled, "the menu greys out Run it again when read only");
    }
}

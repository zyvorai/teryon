# Shell integration

TERYON works with any shell. If the shell also tells TERYON where its prompts, commands and working
directory are, the desktop app can do more. The shell sends two standard escape sequences (OSC 133 prompt
marks as used by iTerm2, kitty, WezTerm and Ghostty, and OSC 7 for the directory); the daemon records them.

## Turn it on

On **the machine where the shell runs** (for a remote host, that is the remote host), add one line to the
shell's startup file:

```sh
# zsh (~/.zshrc) and bash (~/.bashrc, or ~/.bash_profile on macOS)
eval "$(teryon shell-init zsh)"      # or: teryon shell-init bash
# fish (~/.config/fish/conf.d/teryon.fish)
teryon shell-init fish | source
```

`teryon setup` can add the guarded block for your local shell after asking; it keeps a backup next to the file
and is safe to re-run. The block does nothing on a machine without `teryon`. Open a new pane afterwards.

## What you get (desktop app)

| Feature | How |
|---|---|
| Jump between prompts | ⌘↑ / ⌘↓ (page scrolling moves to ⇧⌘↑ / ⇧⌘↓). Without integration ⌘↑/⌘↓ scroll a page |
| Copy the last command's output | ⇧⌘O, or "Copy last command output" in the palette |
| Exit status and duration | A chip in the pane title bar: `✓ 0.4 s`, `✗ 1 · 12 s`, `running…` |
| "Command finished" notification | When a command that ran longer than `notify_after_seconds` (default 10) ends while the window is in the background |
| New panes start where you are | The focused pane's live working directory (OSC 7) is used for new panes |

## How it works

* The daemon parses the sequences with the terminal engine's unhandled-OSC hook and keeps up to 500 marks per
  pane, each at an **absolute line number**. The terminal engine is vendored with a small patch that counts
  every row that scrolls into history (`vendor/vt100/PATCH.md`), so a line keeps its number while history
  grows and is dropped once it falls out of the retained history.
* Marks and line numbers travel as extra optional fields on the existing protocol messages; older clients and
  daemons ignore or omit them, so nothing needs to be upgraded in lock step and the protocol version is unchanged.
* `Capture` now returns exactly one text line per terminal row and the number of its first line, which is how
  the desktop maps marks to text.
* History is retained per pane (2000 lines by default; set `TERYON_SCROLLBACK` between 100 and 50000 in the
  daemon's environment, remembering memory grows with lines × columns × panes).

## Limits

* Commands typed into a program that reads its own input (an editor, `ssh` into another machine) are not
  marked by the outer shell; marks resume when you are back at the prompt.
* Full-screen programs share the cursor but not the history, so their output is not marked.
* The "last output" and prompt jumps need the lines to still be in the retained history.
* The terminal UI (`teryon connect`) does not show the chips or jump between prompts yet.

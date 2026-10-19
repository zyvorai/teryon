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
| Copy the last command line | "Copy last command" in the palette |
| Run the last command again | "Re-run last command" in the palette |
| Ask the assistant about the last command | "Ask AI about last command" in the palette (needs `ANTHROPIC_API_KEY`) |
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

## Zsh power features

With the same `eval "$(teryon shell-init zsh)"` line, zsh also gets the everyday conveniences of the usual
plugins, built in (no plugin manager, nothing else to install):

| Feature | How it works |
|---|---|
| **History** | Every command is recorded in `~/.teryon/history.jsonl` (private, mode 0600) with its directory, exit status and duration. A command that starts with a space is never recorded. |
| **Suggestions** | The newest matching command shows as gray ghost text; → or End or Ctrl-E accepts it, Alt-F takes one word. |
| **Highlighting** | Commands (green, red when unknown), options, strings, variables, operators, comments and existing paths are coloured as you type. Aliases and functions count as known. |
| **Ctrl-R** | Fuzzy history with exit status, duration, directory and age. Enter puts the command on the line, Alt-Enter runs it, Tab narrows to this directory. |
| **Ctrl-T / Alt-C** | Fuzzy file picker (respects `.gitignore` inside a repository) and a directory picker that runs `cd`. On macOS, Alt-C needs Option to act as Meta in your terminal. |
| **Tab** | One match completes in place. Several open the fuzzy picker, for files and for anything zsh's own completion offers (`compinit` is required). It falls back to plain zsh Tab inside quotes, mid-word or when you type ahead. |
| **`z` / `zi`** | `z proj` jumps to the best matching directory (visits weighted by recency), `zi` picks from all of them. Directories are remembered in `~/.teryon/jump.jsonl`. An existing `z` function or alias is left alone. |
| **Prompt** | Off by default. A two-line prompt with the directory, git branch and state, language, the last command's duration and its exit status. |

`teryon-features` lists what is on and which feature is yielding to a plugin you already load
(zsh-syntax-highlighting, zsh-autosuggestions and fzf-tab take over their own job).

### Switches

Everything above is on except the prompt. Change it in `~/.config/teryon/config.toml`:

```toml
[shell]
enabled = true     # master switch
history = true
suggest = true
highlight = true
pickers = true     # Ctrl-R, Ctrl-T, Alt-C
tab = true
jump = true
prompt = false     # two-line prompt with git state
```

A variable set in the environment before the `eval` line wins over the file: `TERYON_PLUS`, `TERYON_HISTORY`,
`TERYON_SUGGEST`, `TERYON_HIGHLIGHT`, `TERYON_PICKERS`, `TERYON_TAB`, `TERYON_JUMP`, `TERYON_PROMPT` (`0` or `1`).

### Limits

History and the jump list are local to each machine; a remote host keeps its own if it runs `teryon` with the
integration. The highlighter is a tokenizer, not a full zsh parser, so `$(...)` and `case` patterns get basic
colouring and lines over 500 characters are left plain. The Tab menu shows no descriptions yet. bash and fish get the
prompt marks only.

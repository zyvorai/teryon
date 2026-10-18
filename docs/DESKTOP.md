# TERYON desktop app

`teryon desktop HOST -w NAME` (build with `--features desktop`) is a native GPU window over the same
persistent shells as the terminal UI. It has one slim top bar and no sidebar: everything else is the
terminal.

## Top bar

Window buttons (macOS: the title bar is merged into the bar), the TERYON mark, the workspace (or tabs when
you have several), `+` for a new tab, one numbered chip per pane, then on the right a connection dot
with latency, the layout switch, **New pane**, and the command palette. Messages appear as a short
toast in the bottom-left corner instead of a status bar. Drag empty bar space to move the window;
double-click it to maximize.

## Mouse menus

Right-click in a pane for Copy, Paste, Select all, Copy last output, Find, Quick select, split, float, zoom, rename
and close. The ▾ next to "+" in the bar opens the same menu plus New tab, Open workspace, Layout, Snippets, View
(broadcast, text size, theme, full screen) and Help. Shortcuts are shown beside each entry. When a program tracks
the mouse (vim, tmux, htop) it keeps the right button; hold Shift to open the menu anyway. Paste reads the system
clipboard through `pbpaste` (macOS), `wl-paste`/`xclip`/`xsel` (Linux) or PowerShell (Windows).

## AI agents in panes

Run Claude Code, Codex, Gemini CLI, aider, Cursor's agent or OpenCode in any pane (local or over SSH). TERYON sees the
pane's foreground command and shows a chip: **working**, **needs you** (an approval or y/n prompt is on screen),
**idle** or **done** (the shell is back). **⌘⇧A** lists every agent in the workspace, needing-you first. With the window
in the background you get a notification when one needs you or finishes (`[desktop] agent_notifications = false` turns
it off). Because the daemon keeps the agent running, you can close the app, reconnect later and see "While you were
away" with what finished or failed; ✦ Summarize asks the assistant for a short recap.

## Icons

Icons printed by `eza --icons`, `lsd`, oh-my-posh, starship and similar tools are Nerd Font glyphs. The desktop app
bundles the Symbols Nerd Font (license in `assets/fonts/`) as a fallback behind your terminal font, so they render
with nothing to install. The terminal UI (`teryon connect`) is drawn by the terminal you run it in and uses that
terminal's font.

## Getting around

* **Welcome card**: while a workspace has no panes the window explains what is happening; if the host cannot
  be reached it shows the error and the exact `teryon doctor HOST --fix --binary …` command with a copy button,
  plus your recent workspaces to open instead.
* **⌘O Open workspace…** lists recent workspaces and your configured hosts; choosing one opens it as a new tab,
  on that host (tabs of different hosts are labelled `host/workspace`). `open HOST [WORKSPACE]` works in the palette too.
* **⌘/ cheat sheet** lists every shortcut from the live keymap, with rotating tips. The first run shows a one-time hint.
* **Auto light/dark**: set `theme_light` and `theme_dark` in `[desktop]` to follow the macOS appearance; a theme
  you pick by hand stays until the appearance changes.

## Panes and splits

* Layouts: single, stacked, side-by-side, 2×2 grid. Drag any divider; shares are kept between 12 % and 88 %.
* Directional focus, resize and equalize (see the shortcut table). Unfocused panes are dimmed
  (`unfocused_opacity`) and show a hollow cursor.
* Pane title bars (double-click the name to rename, zoom and close buttons) appear when more than one
  pane is visible; set `pane_titles` to `always` or `never`.

## Terminal behavior

* **Selection**: drag; double-click selects a word, triple-click a line, ⌥-drag a rectangle,
  Shift-click extends. ⌘C copies; `copy_on_select = true` copies as soon as you finish selecting.
* **Links**: ⌘-click opens plain-text `http(s)://` URLs.
* **Scrollback**: scroll the wheel or trackpad (or ⇧⌘↑ / ⇧⌘↓, ⌘Home / ⌘End) in a normal-screen pane. The history
  comes from the daemon's retained text, so it is plain text without colors; typing returns to the live screen.
  Full-screen programs (vim, less) get the wheel as arrow keys.
* **Mouse reporting**: programs that ask for the mouse (vim, htop, tmux, ...) receive clicks, drags and the
  wheel in the standard xterm encodings. Hold Shift to select locally instead.
* **Cursor**: `block`, `bar` or `underline`, optional blinking.
* **Notifications**: when a pane's process exits while the window is in the background, macOS shows a notification.

## Power tools

* **Quick select** (⇧⌘E): labels appear over every URL, file path (with `:line`), git SHA, IP address, UUID and
  email on the screen. Type a label to copy it; Tab switches the action to paste (types it into the pane) or open
  (links and local paths). Esc cancels.
* **Copy mode** (⇧⌘X): move through the pane's whole history with the keyboard. `h j k l`, `w b e`, `0 ^ $`,
  `gg G`, `^U ^D ^B ^F`; `v` characters, `V` lines, `^V` a block; `/` and `?` search (smart case), `n N` repeat;
  `y` or Enter copies; `q` or Esc leaves. History is plain text, as in scrollback.
* **Snippets**: saved commands in the palette (`Snippet: NAME`). `{{name}}` placeholders are asked for first.
  With `run = true` single-line snippets also press Enter; multi-line snippets are only typed, never run.
* **Broadcast input** (⌥⌘I): everything you type or paste goes to every visible pane that is still running. The
  panes get an amber border and the bar shows BROADCAST; it switches itself off when you change tab.
* **Find**: ⌥⌘R toggles regular expressions and ⌥⌘C match case (also the `.*` and `Aa` buttons); a bad pattern is
  shown next to the field.

## Remote-native extras

* **Floating panes** (⌥⌘↩ floats the focused pane, ⌥⌘N opens a new floating one): the pane leaves the layout and
  becomes a draggable, resizable window on top of it, handy for a scratch shell or a log tail. ⌥⌘↩ (or the
  pane's expand button) puts it back. It needs at least one other pane.
* **Drag and drop**: drop files on a pane. On `local` the shell-quoted paths are typed at the cursor. On an SSH
  host the files are uploaded with rsync into the pane's current directory (live when shell integration is
  on); files that already exist there are left alone, and a toast reports the result. Up to 50 files per drop.
* **Triggers**: `[[desktop.triggers]]` rules match each screen line with a regular expression and
  `highlight` it (optionally with a `#RRGGBB` color), `notify`, `bell` or `toast`. Notifications and sounds fire once
  per distinct line, never for what was already on screen, and by default only while the window is in the background.
* **Saved layouts**: the app remembers each workspace's panes (names and directories, never commands) in
  `~/.teryon/layouts/`. When you attach to a brand-new workspace (for example after the host rebooted) it offers
  "Restore N panes". `teryon connect HOST -w NAME --restore` does the same for the terminal UI.

## Shell-aware features

With the one-line shell integration ([SHELL-INTEGRATION.md](SHELL-INTEGRATION.md)) the app knows where
prompts, commands and the working directory are: ⌘↑/⌘↓ jump between prompts, ⇧⌘O copies the last command's
output, pane title bars show `✓ 0.4 s` / `✗ 1 · 12 s` / `running…`, long commands that finish while the window
is in the background raise a notification (`notify_after_seconds`), and new panes start in the focused
pane's directory. Without the integration everything else keeps working and ⌘↑/⌘↓ just scroll a page.

## Configuration

Options live in the `[desktop]` table of `~/.config/teryon/config.toml` (see `docs/config.example.toml`).
The file is re-read about once a second while the app runs: a valid change applies at once and shows
"Config reloaded"; an invalid one is reported and the previous settings stay. Validate from a shell with
`teryon config --check`, show the effective values with `teryon config`, and list every action with its
binding using `teryon config --keys`.

| Option | Default | Meaning |
|---|---|---|
| `font_family` | SF Mono | Family or file name in the font folders, or an absolute path |
| `font_size`, `line_height`, `padding` | 13, 1.45, 8 | Terminal text metrics |
| `cursor_style`, `cursor_blink` | block, false | Cursor shape |
| `window_opacity` | 1.0 | 0.3 to 1.0; turning transparency on needs a restart |
| `unfocused_opacity` | 0.82 | Brightness of unfocused panes |
| `copy_on_select` | false | Copy a selection when it is finished |
| `scrollback_lines` | 10000 | History lines kept in the scrollback view |
| `pane_titles` | auto | `auto`, `always`, `never` |
| `notify_after_seconds` | 10 | Notify when a command this long finishes in the background (shell integration; 0 = off) |
| `confirm_quit` | false | Ask before the window closes (shells keep running either way) |
| `triggers` | none | `[[desktop.triggers]]` with `pattern`, `action` (`highlight`, `notify`, `bell`, `toast`), optional `color`, `unfocused_only` |
| `snippets` | none | `[[desktop.snippets]]` with `name`, `command`, optional `description` and `run` |

Remembered state (window size, theme, font size, layout) is stored separately in `~/.teryon/desktop.toml`.
Precedence for the theme: `--theme`, then the remembered choice, then `config.toml`.

### Key bindings

`[desktop.keys]` maps an action name to a chord such as `"cmd+shift+d"` (modifiers `cmd`, `ctrl`, `alt`,
`shift`; keys `a`-`z`, `0`-`9`, arrows, `enter`, `tab`, `f1`-`f12`, punctuation). Use `"none"` to unbind.
Conflicts and unknown actions are reported, and the bad entry is skipped. On Linux, Ctrl+Shift plays the
role of ⌘ (so ⇧ variants are not available there).

| Keys | Action |
|---|---|
| ⌘O | Open a workspace (new tab, any host) |
| ⌘/ | Shortcut cheat sheet |
| ⌘K | Command palette (fuzzy search of every action; ↑/↓ over a theme previews it) |
| ⌘D / ⇧⌘D | New pane to the right / below |
| ⌥⌘ arrows | Focus the pane in that direction |
| ⌃⌘ arrows, ⌃⌘= | Resize the focused pane, equalize all |
| ⌘] / ⌘[ | Next / previous pane |
| ⇧⌘↩ | Zoom or restore the focused pane |
| ⌘F, ⌘G, ⇧⌘G | Find in the pane, next, previous (⌥⌘R regex, ⌥⌘C case) |
| ⇧⌘E / ⇧⌘X | Quick select / copy mode |
| ⌥⌘I | Broadcast input to all visible panes |
| ⌥⌘↩ / ⌥⌘N | Float the focused pane / new floating pane |
| ⇧⌘F | Search the retained output |
| ⌘T, ⌘W | New tab, close tab |
| ⌘1 to ⌘9, ⇧⌘← / ⇧⌘→ | Jump to a tab, reorder tabs |
| ⌘+ / ⌘- / ⌘0 | Font size larger, smaller, reset |
| ⌘↑ / ⌘↓ | Previous / next prompt (a page scroll without shell integration) |
| ⇧⌘↑ / ⇧⌘↓, ⌘Home / ⌘End | Scroll history by a page, to the top, to the bottom |
| ⇧⌘O | Copy the last command's output |
| ⇧⌘, | Reload the config file |
| ⌃⌘F | Fullscreen |
| ⌘C | Copy the selection |

Palette-only actions: next/previous/random theme, switch between a theme's light and dark variant, layouts,
rename, copy visible screen, offline draft, always on top.

## What it does not do

* Font ligatures and complex text shaping (the renderer lays glyphs out one by one).
* Window background blur; plain `window_opacity` transparency is supported.
* The kitty graphics and keyboard protocols, and OSC 8 hyperlinks (the terminal parser ignores them;
  plain-text URLs still open with ⌘-click).
* Shell-integration features such as jumping between prompts (TERYON does not inject into your shell).
* Native macOS tabs (TERYON has its own tabs) and a global-hotkey dropdown window (a GUI toolkit cannot
  register global shortcuts).

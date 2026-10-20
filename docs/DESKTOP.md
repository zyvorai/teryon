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
away" with a row for each command that finished (its command line, ✓ or ✗ exit code, how long it took, and when) and for each agent waiting for you; click a row to open that pane at the command's output. ✦ Summarize asks the assistant for a short recap. The card shows the eight newest events and needs the shell integration for command rows.

Start one from the palette: "Agent: start Claude in a new pane" types its command into a new pane in the focused
pane's directory. "Agent: start Claude in a new Git worktree" does that on its own branch instead: it runs
`git worktree add -b agent/claude-<time>` next to the repository (`<repo>-claude-<time>`) and starts the agent inside it,
so the agent's edits stay out of your checkout. When the agent exits, the pane is back in the directory you started
from, and the worktree and branch stay for you to review, merge or remove (`git worktree remove`). Outside a Git
repository it stops with Git's error before starting anything. The script runs in `sh`, whatever your shell is.

`agent_sandbox = true` (also in Settings) starts agents from the overview and the palette inside `teryon sandbox`:
the agent can write only to the Git repository it starts in (for a worktree, that worktree and the repository's shared
Git directory, so commits work), to scratch space (`/tmp`, `$TMPDIR`) and to the settings and cache directories agents use
(`~/.claude`, `~/.codex`, `~/.gemini`, `~/.config`, `~/.cache`, `~/.npm`, …), and it cannot read `~/.ssh`, `~/.aws`,
`~/.gnupg` or `~/.kube`. The network stays open, since an agent needs its model API. You can use it by hand too:
`teryon sandbox [--allow-write DIR] -- claude`. It is a guard against mistakes and a leaked-credentials blast radius,
not a security boundary against a hostile program: macOS uses `sandbox-exec` (which Apple has deprecated but still ships),
Linux uses `bwrap` (bubblewrap, which must be installed). If that tool is missing the agent is not started, rather than
started unprotected. It needs `teryon` on the PATH of the host the pane runs on.

Each row of the Agents panel has a **Changes** button: it opens a new pane in that agent's directory (on the
same host, so it works over SSH) showing the branch, the changed and untracked files (`git status --short --branch`)
and the size of each change (`git diff HEAD --stat`), and leaves a shell open there for `git diff`. It reads the
repository, not the agent, so it shows every change in the directory, including yours. Outside a Git repository it says so.

## File browser

**⌘B** opens a sidebar listing the focused pane's directory; it follows `cd`, and works the same on a remote host (it runs `ls` and `head` over your SSH connection, nothing extra on the host). Click a row to select it, double-click to open a folder or type a quoted path at the prompt, **Space** for Quick Look (the first 64 KB of a text file), ↑/↓ to move, Enter to open, Backspace to go up, Esc to return to the pane. Right-click offers Quick Look, Insert path, Copy path, cd here and Reveal in Finder (local only). Quick Look also shows PNG and JPEG images. Remote rows add **Download to Downloads** (a file or folder, streamed with `tar` over SSH, never replacing an existing item). **Rename…** and **Move to Trash…** each ask first; a rename never replaces an existing name, and nothing is deleted: items move to the Trash (local) or `~/.teryon-trash` on an SSH host, so they can be put back. Nothing is run for you: paths and `cd` lines are typed without Enter.

## Describe a command

⌘⇧K opens a bar at the top: say what you want, review the command and its risk, press Enter to insert it at the
prompt (you still press Enter to run it). Click a red ✗ chip after a failed command to have the assistant explain it.
See `docs/AI.md`.

## Command history

⌘⇧H (palette: "Search command history…") opens the same list as Ctrl-R in the shell, without leaving the window:
Enter puts the command at the prompt, ⌥Enter runs it, Tab narrows to this directory. It needs the zsh integration
(`teryon setup`) and a local workspace, because the history is kept on this Mac.
The same filter words work here and in Ctrl-R: `is:failed`, `is:ok`, `exit:N`, `slow:SECONDS`, `dir:PART` and
`branch:PART`, combined with ordinary text (`cargo is:failed dir:api`). Each command is recorded with the Git branch it ran
on, read from `.git/HEAD` (commands recorded before that have none and never match a `branch:` filter).

Other machines' history can be copied here with `teryon hist pull HOST` (over SSH, using the same host names and
`--remote-bin` as the rest of teryon; run it again to refresh). Those commands show up in Ctrl-R and ⌘⇧H marked
`@HOST`, and `host:NAME` narrows to one machine (`host:local` is this one). They are kept apart in
`~/.teryon/history-hosts/HOST.jsonl` (private, newest 5000), never feed the ghost suggestions or directory jumps, and
commands with secrets are dropped on both ends. It copies a snapshot; nothing runs in the background.

Commands that contain a recognizable secret (a token, a `--password` flag, a URL with credentials) are not recorded,
so they are never suggested back or listed. History kept before that can be cleaned with `teryon hist scrub`; a
command you start with a space is also never recorded.

## A bare window

For a window that is only the terminal, like WezTerm with its tab bar and title bar off:

```toml
[desktop]
chrome = "none"          # no title bar, no tab strip, no empty-pane hints (restart to apply)
initial_cols = 124       # open windows this many columns and rows, instead of the last size
initial_rows = 42
fresh_window = true      # every window starts a new shell and ends it when the window closes
padding_x = 9.6          # per-side text padding in pixels (falls back to `padding`)
padding_top = 0.0
padding_bottom = 4.8
scrollbar = "always"     # auto (only while scrolled back) | always
```

With `fresh_window`, `teryon desktop` no longer reattaches to the shared `default` workspace: each window gets
its own `win-…` workspace, removed (and its shells ended) when you close the window. Workspaces left behind by a
crash are cleaned up the next time a window starts. `teryon desktop -w NAME` still attaches to a workspace you name,
and other tabs you open stay persistent. Leave it off if you rely on shells surviving a closed window.

The window stays resizable. A thin strip along the top edge moves it, and double-clicking that strip maximizes.
Tabs, panes, the palette (⌘K) and every shortcut still work; only their buttons are hidden.

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
* **Pane tree**: "Split pane right (pane tree)" and "Split pane below (pane tree)" in the palette (bindable as `split_right` and
  `split_down`) split the focused pane in two, as often and in any direction as you like, with no limit of four. Every divider
  drags, Resize and Equalize work on the tree, a closed pane gives its space to its neighbours, and a pane opened from
  elsewhere is split off the focused one. While a tree is active "New pane to the right / below" split too. Choosing any
  `Layout:` entry returns to the grid layouts. The shape is not saved: after a restart the panes come back side by side.
* Directional focus, resize and equalize (see the shortcut table). Unfocused panes are dimmed
  (`unfocused_opacity`) and show a hollow cursor.
* Pane title bars (double-click the name to rename, zoom and close buttons) appear when more than one
  pane is visible; set `pane_titles` to `always` or `never`.

## Terminal behavior

* **Selection**: drag; double-click selects a word, triple-click a line, ⌥-drag a rectangle,
  Shift-click extends. ⌘C copies; `copy_on_select = true` copies as soon as you finish selecting.
  Programs that track the mouse (Claude Code, vim) get your drags; hold ⇧ to select instead, or set
  `drag_selects = true` so a drag selects and only plain clicks reach the program.
* **Links**: ⌘-click opens plain-text `http(s)://` URLs and links programs mark with OSC 8 (`ls --hyperlink`,
  `eza --hyperlink`, `rg --hyperlink-format`, `delta`). Hovering an OSC 8 link shows where it really goes. Only `http`, `https` and
  `mailto` links open; `file:` and app links are ignored, because the text shown for a link is not its target.
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

* **Font picker** ("Choose font…" in the palette or Settings): the monospaced fonts installed on your Mac, each
  shown in its own typeface; Enter writes `font_family` to your config. **Keyboard shortcuts…** lists every action
  with its key: Change, then press the new keys (a ⌘, ⌃ or ⌥ is required so typing still reaches the shell);
  clashes are refused. **Window look…** is a small panel with opacity, dimming, font size and line height.
* **Right-click a command** (needs shell integration): when the click lands on a command or its output, the menu
  starts with Copy command, Copy output, Run it again and Ask AI about it for that block.
* **Tab marks**: a background tab shows `●` when its panes printed something and `▲` when an AI agent in it needs
  you; looking at the tab clears it.
* **Config problems** show as a banner with the parser's message, an Open config file button and Try again,
  and your previous settings stay in force.
* **Command gutter** (`command_gutter`, on by default): a thin bar beside each command and its output: red when
  it failed, the accent color while it runs, faint green when it succeeded.
* **Palette actions** for the last command: "Copy last command", "Re-run last command" and "Ask AI about last
  command" (the command and its redacted output go to the assistant).
* **Listening ports**: "Open listening port in browser" opens the first TCP port the focused pane's processes
  listen on. On an SSH host it starts an `ssh -L` tunnel first (closed when the app quits).
* **Agent overview** (⌘⇧A): lists the AI agents in this workspace. With `agent_quick_approve = true` a waiting
  agent gets Approve / Deny buttons. `teryon agents --all` lists agents across every workspace and host.
* **Host health** (window 1100 px or wider): load, memory and a heat warning for the machine the daemon runs on.
* **Notifications**: besides the macOS notification, a `[notify]` section sends the same events to ntfy or a
  webhook, from the app while it is in the background and from the daemon while no client is attached.

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
| `pane_inset` | 10 | Space around the pane cards; 0 runs panes edge to edge |
| `cursor_style`, `cursor_blink` | block, true | Cursor shape (blinks with an eased fade) |
| `window_opacity` | 1.0 | 0.3 to 1.0; turning transparency on needs a restart |
| `window_blur` | 0 | 0 to 100: blur behind a transparent window (macOS; needs `window_opacity` below 1.0) |
| `maximized` | false | open windows filling the screen (below the menu bar) |
| `fullscreen` | false | open windows in native full screen (macOS: Dock and menu bar hide; ⌘⌃F leaves it); it wins over `maximized` |
| `corner_radius` | 0 | round a bare window's corners (points, 0 to 24) and draw a hairline edge with a faint top highlight |
| `status_line` | true | muted line under the panes (PTYs held by the daemon, last relink, receive rate, platform, theme); hidden below 900 points wide |
| `background_noise` | 0 | fine grain over the background, 0 to 100 (WezTerm's `noise = 10`) |
| `text_gamma` | 0.55 | 0.3 (heavy) to 1.0 (thin) text weight; lower is closer to WezTerm's heavier look |
| `cursor_color` | orange `#D97757` | `#RRGGBB` |
| `background_gradient` | none | two or more `#RRGGBB` stops painted across the terminal |
| `gradient_angle` | 135 | degrees as in CSS and WezTerm: 90 left to right, 180 top to bottom, 135 toward the bottom right |
| `tab_bar` | `off` | `bottom` adds a strip of rounded tabs and a clock under a `chrome = "none"` window |
| `tab_bar_bg`, `tab_active_bg`, `tab_active_fg`, `tab_inactive_bg`, `tab_inactive_fg` | from the theme | `#RRGGBB` colors of the tab strip |
| `unfocused_opacity` | 0.82 | Brightness of unfocused panes |
| `copy_on_select` | false | Copy a selection when it is finished |
| `check_updates` | false | Once a day at start-up, ask GitHub whether a newer TERYON is out and say so in a toast ("Check for updates" in the palette asks any time and opens the release page; nothing is ever installed by the app) |
| `drag_selects` | false | Drag selects locally even when the program tracks the mouse |
| `quiet_poll_ms` | 250 | Gap between polls of a session with no input or output for 5 s (0 = always 60 ms). About 4× fewer requests while nothing happens; output after a quiet spell can show up this much later (typing and any screen change return to 6 ms polling at once) |
| `scrollback_lines` | 10000 | History lines kept in the scrollback view |
| `pane_titles` | auto | `auto`, `always`, `never` |
| `notify_after_seconds` | 10 | Notify when a command this long finishes in the background (shell integration; 0 = off) |
| `confirm_end_shells` | true | Ask before closing a tab or window that would end its running shells (`fresh_window` tabs and windows own theirs) |
| `confirm_quit` | false | Ask before the window closes; a normal window only detaches, its shells keep running |
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
| ⌘, | Settings window: Look, Text, Window, Behavior and Alerts, with sliders and switches that apply live and are written to `config.toml` (comments kept); search box; buttons for the theme, font and shortcut editors; ↺ puts an option back to its default |
| ⇧⌘Z | Reopen the pane you just closed (its shell keeps running for 30 seconds after you close it) |
| ⇧⌘T | Theme picker: every theme in one list, drawn in its own colors; type to search, All / Dark / Light filter, ↑/↓ previews it live, Enter keeps it, Esc puts the old one back (also under View in the top bar menu) |
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
* Window background blur uses a private macOS call and may stop working in a future macOS release.
* The kitty graphics protocol, and the parts of the kitty keyboard protocol beyond "disambiguate escape codes"
  (key release events, all keys as escapes, associated text). Programs can ask for the flag with `CSI > 1 u`;
  Esc and Ctrl/Alt combinations then arrive as `CSI code ; mods u`. Other flags are accepted and not reported
  back, so a program never expects something that is not sent. A program that dies without popping its flags
  leaves them on, except full-screen (alternate screen) programs, whose flags go when the screen is left.
* Shell-integration features such as jumping between prompts (TERYON does not inject into your shell).
* Native macOS tabs (TERYON has its own tabs) and a global-hotkey dropdown window (a GUI toolkit cannot
  register global shortcuts).


## Polish

* **Tabs** show an icon for the foreground program (`nvim`, `ssh`, `cargo`, `git`, `python`, `node`, `docker`, AI agents) and its name, a spinner while a command runs in a shell that has the integration loaded, an × on each tab, and cross-fade when focus moves.
* **Command chips**: in a bare window a finished command's status and time (`✓ 1.2 s`, `✗ 1 · 340 ms`) appear in the pane's top-right corner and fade out (needs the shell integration).
* **Cursor** blinks with an eased fade when `cursor_blink = true`.
* **Scrolling back** glides to the new position instead of jumping a line at a time.
* **Themes** cross-fade over a quarter second whenever they change. `carbon` (neutral dark) and `tokyo-night-day` are new built-ins.
* **Inline images**: programs that print iTerm2 images (`imgcat`, `OSC 1337 ; File=inline=1`) show them in the pane. The daemon reserves the rows, keeps up to 64 MB of images per pane, and clearing the screen removes them. PNG and JPEG are drawn; other formats leave blank space. Images do not show inside full-screen programs or in the scrollback text view.
  The **Kitty graphics protocol** (`kitten icat`, `chafa -f kitty`, `timg -p kitty`) works the same way, for a subset: PNG and raw RGB/RGBA sent directly in chunks, transmit/display/put/query/delete (`a=t|T|p|q|d`), `c`/`r` sizing, `C=1`, and replies to the program (so its detection query succeeds). Files and shared memory (`t=f|s`), zlib (`o=z`), animation, cropping, z-order and Unicode placeholders are answered with an error, so tools that offer a fallback use it. A daemon started before the upgrade needs `teryon stop --force`.

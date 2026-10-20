# Changelog

## Unreleased

- Fixed: a recipe pane could be treated as ready too early when its `ready_text` also appeared in its own command line (the shell echoes the command, and some prompt themes show it twice). Only output after the last echo of the command now counts, and a command or text that wraps across rows still matches.
- Commands that contain a secret are no longer written to the command history, so they are never suggested back or shown in the history picker. `teryon hist scrub` removes the ones already there. The redaction rules (also used before anything goes to the assistant) now catch `--password`/`--token`/`--api-key` flags, `user:password@` URLs and `Authorization: Basic` headers as well as the key shapes and `NAME=value` secrets they already knew. It is pattern-based and best-effort: a secret typed as a bare argument is not caught.
- Command history search takes filter words, in Ctrl-R and the desktop's ⌘⇧H alike: `is:failed`, `is:ok`, `exit:N`, `slow:SECONDS`, `dir:PART` and `branch:PART`, mixed with fuzzy text (`cargo is:failed dir:api`). With a filter, every recorded run is searched, not only each command's newest, so a command that failed last week still turns up after it later passed. Commands are now recorded with the Git branch they ran on (read from `.git/HEAD`, no extra process), shown in the list.
- Recipes can start things in order: a `.teryon.toml` pane can say `after = "db"` and the other pane can declare `ready_text`, `ready_port` and `ready_timeout`. `teryon up` creates all the panes, then types each command once the pane it waits for is ready, and stops with a clear error (leaving what already started running) if one never is. Unknown, ambiguous and circular `after` are rejected when the recipe is read.
- "Agent: start Claude (or Codex, Gemini, aider, Cursor, OpenCode) in a new Git worktree" in the palette: the agent gets its own branch (`agent/<name>-<time>`) and directory next to the repository, so its edits stay out of your checkout. Nothing is cleaned up for you: the worktree and branch stay until you merge or remove them.
- "While you were away" is now a timeline: one row per command that finished while the app was closed (the command line, ✓ or ✗ with exit code and duration, and how long ago), plus agents waiting for you. Click a row to open that pane at the command's output. The newest eight are shown, with a count of earlier ones; failures are highlighted. Command rows need the shell integration.
- Slimmer polls: a client that asks for diffs is sent the pane's title, directory, process, ports and line numbers once, and again only when one of them changes (the long foreground command line was being resent on every poll). Empty fields (`screen`, `exited`, zero line numbers) are no longer written at all. Together with diffs and compact text, a whole scrolling pane's reply stream came to about a seventh of what whole screens cost. What remains when nothing is changing is the per-poll framing and the polling rate itself.
- Compact screen bytes: a client that asks for diffs is sent the screen as base64 text instead of a JSON number array (about a third of the size). Measured over a whole scrolling pane including everything else in each reply, diffs plus compact text come to roughly a fifth of what whole screens cost. Pane details such as the title, directory and foreground command are still resent on every poll; that is now the largest remaining cost when nothing is changing.
- Screen diffs over SSH: the desktop app asks the daemon for the difference from the last screen it received (a scroll plus the changed rows) instead of the whole screen each time. In a test with a pane printing colored lines, 8.5 KB crossed the wire instead of 144 KB, and the rebuilt screen was identical. Each diff carries a checksum; if the two sides ever disagree, a size change intervenes, or 500 updates pass, a whole snapshot is sent. Old daemons and local sockets are unchanged, and the UI still receives whole screens.
- Closing a tab or window that would end running shells now asks first, naming what is running ("ends 2 running shells: vim, claude"). With `fresh_window`, a ⌘T tab and the window own their shells, and closing used to end them silently. "Keep it open" is the default button and Esc cancels. `confirm_end_shells = false` turns the question off. Tabs that only detach (shells keep running on the host) still close at once, and the tab's close button now says which of the two it does.
- Kitty keyboard protocol (the "disambiguate escape codes" flag): Neovim and other modern TUIs can tell Esc from Alt-prefixed keys and get Ctrl/Alt combinations that the legacy encoding cannot express (Ctrl+1, Ctrl+Alt+A…). The daemon tracks each pane's flag stack per screen and answers `CSI ? u`; the desktop app encodes Esc, Ctrl/Alt keys and Shift+Tab as `CSI code ; mods u` while a program has the flag on. Unsupported flags are not reported back. The terminal UI (`teryon connect`) is unchanged.
- Controller takeover: "Take control of this workspace from another device" in the palette takes the controller seat immediately instead of waiting for the other client to be released (up to 45 s when its link is half-open). The previous controller is told ("Another device took over") and keeps watching, read-only, until it takes the seat back. When an attach is refused because another client controls the workspace, the app says so and points to the action. Old daemons ignore the request and keep refusing.
- OSC 8 hyperlinks: links that programs mark (`ls --hyperlink`, `eza --hyperlink`, `rg --hyperlink-format`, `delta`) open with ⌘-click and show their real target on hover. Only `http`, `https` and `mailto` open. Links are tracked per cell by the daemon and sent beside the screen, so older clients simply do not see them. They apply to the live screen, not the scrolled-back view.
- Connection recovery inspector: the relink card now says why the link is down and what to do (Wi-Fi or DNS, host asleep, SSH not answering, key refused, changed host key, teryon missing or a different version, or a link that dropped), with ssh's own message underneath. Causes that need you show "ACTION NEEDED" instead of counting attempts. This works because ssh's stderr is now captured rather than lost; `teryon doctor HOST` errors include it too.
- Agents: the overview (⌘⇧A) shows a one-line summary of where each agent got to and can start Claude, Codex, Gemini, aider, Cursor or OpenCode in a new pane (also in the palette as "Agent: start …"). "Agent: send a prompt to all idle agents…" types one prompt into every agent sitting at its prompt, skipping working agents and ones waiting for approval. "Finished" notifications now include that last line.
- Remote link: reconnect waits are spread by up to ±20% so tabs do not retry in lockstep, and a link that comes back after the machine slept (20 s without a poll) is replaced at once instead of waiting for ssh keepalives to fail.
- Typing `exit` (or Ctrl-D) no longer leaves a dead pane: after a clean exit the pane closes, or the tab closes when it was the last pane, or the window closes when it was the only tab (the workspace is removed). A non-zero exit keeps the pane so you can read the output; the close-pane key then removes it.
- `drag_selects = true` makes a drag select (and honor `copy_on_select`) in panes whose program tracks the mouse, such as Claude Code; plain clicks and the wheel still reach the program. Shift-drag always selects.

## 0.19.0

- Settings window (⌘,): sliders, switches and choices for the common `[desktop]` options, applied live and saved to `config.toml` with your comments kept, with search, per-option reset, and a "restart" tag on the few that need one. "Window look…" is a small panel with opacity, dimming, font size and line height.
- Font picker: the monospaced fonts installed on your Mac, each shown in its own typeface.
- Keyboard shortcuts editor: every action with its key; click Change and press the new keys. Clashes are refused, and a ⌘, ⌃ or ⌥ is required so typing still reaches the shell.
- Closing a pane is undoable: its shell keeps running for 30 seconds and ⇧⌘Z (or "Reopen closed pane") brings it back. Older daemons just close it for good. The close confirmation is gone.
- Right-click a command or its output for Copy command, Copy output, Run it again and Ask AI about it.
- Background tabs show `●` for new output and `▲` when an AI agent in them needs you.
- A config file that cannot be loaded shows a banner with the message, Open config file and Try again, instead of a toast.
- The welcome card has Theme, Font and Settings buttons.

## 0.18.2

- Theme picker redesigned: a larger window (up to 800 px) of two columns of cards, each a miniature terminal in that theme's own colors (prompt, passed/failed, warning and file-color lines, and its palette). The chosen card glows; arrow keys move across the grid (← → only while the search box is empty); rounded All / Dark / Light filters and a bigger search box.

## 0.18.1

- Theme picker (⇧⌘T, "Choose theme…" in the palette, "Choose Theme…" in the menu): all themes, built-in and your own, in one searchable list. Each row is drawn in its own colors (a sample and its palette), with All / Dark / Light filters. ↑/↓ previews the theme on the whole window, Enter keeps it, Esc restores the one you had.

## 0.18.0

- `teryon agents [HOST...] [--all]` lists the AI agents in every workspace and whether each is working or waiting.
- Palette: "Ask AI about last command" sends the last command and its redacted output to the assistant.
- Host picker: Ctrl+W on a host sends it a wake-on-LAN packet.
- The desktop app also sends agent and long-command notifications to the ntfy / webhook sinks while its window is in the background.
- `.teryon.toml` recipes and `teryon up`: start a project's workspace (named panes, directories, commands); refuses a workspace that already exists.
- Listening ports: the daemon reports each pane's TCP listeners; "Open listening port in browser" opens one, through an `ssh -L` tunnel on remote hosts.

## 0.17.4

- Command gutter: a thin bar in the left padding beside each command and its output (red when it failed, the accent while it runs, faint green when it succeeded), in the live view and in scrollback. Needs shell integration; `[desktop] command_gutter = false` turns it off.

## 0.17.3

- Host health in the top bar (window 1100 px or wider): load, memory in use, and a heat warning only when the CPU is throttled, for the machine the daemon runs on. Old daemons simply show none; the protocol version is unchanged.
- `[host] keep_awake = true` makes the daemon hold `caffeinate` (macOS) while a command has run for `keep_awake_after_seconds` or an AI agent is working.
- `teryon wake HOST` sends a wake-on-LAN packet using `mac` (and optional `broadcast`) from `[hosts.HOST]`.

## 0.17.2

- Agents overview: with `[desktop] agent_quick_approve = true`, an agent waiting for approval gets Approve and Deny buttons (Enter or Esc, or `y`/`n` for aider). Off by default; never active when read only, and a stale click does not answer a later prompt.
- MCP: new read-only `list_agents` tool lists the AI agents in a workspace and whether each is working, waiting or idle.

## 0.17.1

- Daemon notifications: with a `[notify]` section (ntfy, webhook or macOS) the daemon tells you when an AI agent needs you or finishes, and when a long command ends, in any workspace with no client attached. Messages carry the agent or program name only. It needs `curl` for ntfy and webhooks. Restart the daemon (`teryon stop`) after updating.

## 0.17.0

- The shell integration sends each command line with its `133;C` mark (zsh, bash, fish), so the desktop app knows what ran. New palette actions: "Copy last command" and "Re-run last command". Re-source `teryon shell-init` in existing shells to get it.

## 0.16.5

- A running desktop app follows edits to the top-level `theme` in `config.toml`; the theme switches on config reload.

## 0.16.4

- Tab and the arrow keys reach the shell in a fresh window: focus is decided at the start of the frame and the terminal takes it when nothing has it (Tab used to move egui's focus first and was dropped).

## 0.16.3

- The background gradient is also painted under pane title bars (it was skipped, leaving a flat color).

## 0.16.2

- `ion` is the default theme for new installs; existing configs keep their theme. New `ion` theme (void navy, ion-blue accent, amber for "needs you"), with a matching header: host chip with a live dot, `01..` pane buttons, an amber "<agent> needs you" pill, `SHELLS ALIVE` / `LINK` stats, and a status line under the panes (`status_line = false` hides it).

- Relink card: when the connection to the daemon drops while panes exist, a card explains the shells are still running, counts reconnect attempts and offers "Retry now" and "Open offline draft" (Esc hides it). Needs no daemon change beyond an optional `uptime` in `Info`.

- Desktop responsiveness: the worker polls every 6 ms for 400 ms after typing or output (60 ms when idle) and wakes the UI the moment new screen state arrives, history/image reads run on their own read-only connection so they never delay keystrokes, switching tabs cross-fades, and the release build uses thin LTO.

- New shells get a richer `LS_COLORS` (source, config, docs, images, video, audio, archives, lock files, Makefile/Dockerfile/README) and an `EZA_COLORS` for `eza -l` columns.

- Desktop: the cursor is orange (`#D97757`) and blinks by default; set `cursor_color` / `cursor_blink = false` to change it.

## 0.16.1

- Desktop keys follow WezTerm: ⇧PageUp/⇧PageDown scroll, ⌘⇧[ / ⌘⇧] and Ctrl+Tab switch tabs (new `tab_next` / `tab_prev` actions), and the `'` key can be bound.

- Desktop keyboard: ⌥ acts as Meta like WezTerm (⌥B/⌥F/⌥D/⌥. send ESC + key, not the composed character), Ctrl+Space sends NUL and Ctrl+/ sends ^_.

## 0.16.0

- Desktop: inline images (iTerm2 `OSC 1337`), a command status chip in bare windows, tabs with program icons, a spinner and ×, eased cursor blink, smooth scroll-back, theme cross-fade with new `carbon` and `tokyo-night-day` themes, `fullscreen`, `corner_radius` and `background_noise` options.

- Desktop: a `maximized` start option, a WezTerm-style accent scrollbar flush with the right edge, and a × on every tab of the bottom strip.

- Desktop: ⌘T opens a new shell tab at once when `fresh_window` is on (closing it ends its shell); `text_gamma` sets text weight (uses a patched `vendor/egui_glow`).

- Desktop: `cursor_color`, `window_blur` (macOS background blur) and `background_gradient` with `gradient_angle`.

- Desktop: `tab_bar = "bottom"` adds a WezTerm-style tab strip (tab pills, a new-tab button and a clock) under a bare `chrome = "none"` window.

- **Zsh power features** in `teryon shell-init zsh`, no plugins needed: history in `~/.teryon/history.jsonl`, ghost-text suggestions, syntax highlighting, a fuzzy picker for Ctrl-R (history with status, duration and directory), Ctrl-T (files) and Alt-C (directories), fuzzy Tab completion, `z`/`zi` directory jump and an opt-in two-line prompt with git state. `[shell]` in `config.toml` switches each on or off; `teryon-features` shows what is active. See `docs/SHELL-INTEGRATION.md`.
- Desktop: ⌘⇧H searches the command history; `[desktop] chrome = "none"` draws only the terminal (no title bar, tab strip or empty-pane hints) and `initial_cols`/`initial_rows` size new windows, and `fresh_window = true` gives every window a new shell that ends when the window closes (leftovers from a crash are removed at the next start). Text padding can differ per side (`padding_x`, `padding_top`, `padding_bottom`) and `scrollbar = "always"` keeps a scrollbar thumb on the live screen.
- Signed and notarized `TERYON-X.Y.Z.dmg` and `.pkg` are attached to releases by `scripts/release-macos.sh` (built by hand on a Mac; CI still builds the tar.gz archives).

## 0.15.1 — Linux build fix, releases and the launch site

- Fixed the Linux build: an import used only by the macOS Keychain code failed `clippy -D warnings` on Linux.
- Release workflow now publishes a GitHub Release with the archives and these notes (`macos-arm64` and `linux-x86_64`).
- Website: absolute Open Graph and Twitter card tags with a 1200x630 social image, canonical URL, sitemap, robots.txt, 404 page, new sections for agents, AI and the file browser. Social card and launch copy in `docs/social/` (`build-social-card.sh` renders it with headless Chrome).

## 0.15.0 — a calmer display and a file browser

- **File browser** (⌘B, also in the ▾ menu → View and the palette): a Finder-like sidebar for the focused pane's directory, following `cd`, on this machine or over SSH. Breadcrumb, filter, dotfile toggle, icons by file type. Click selects, double-click opens a folder or types a quoted path at the prompt, **Space opens Quick Look** (first 64 KB of a text file, binary files are reported as such), ↑/↓/Enter/Backspace/Esc work from the keyboard. The context menu has Quick Look, Insert path, Copy path, cd here (types it, never presses Enter) and Reveal in Finder (local, macOS). Quick Look also shows PNG and JPEG images (up to 8 MB, shrunk to fit). **Download** (remote hosts) copies a file or folder to `~/Downloads` by streaming `tar` over SSH, so neither machine needs rsync, and never replaces an existing item. **Rename…** and **Move to Trash…** ask first, never replace an existing name, and delete nothing: items go to the Trash (local) or `~/.teryon-trash` (SSH host). `[desktop] file_browser = true` opens it at start. Remote access uses plain `ls` and `head` over the existing SSH transport, so no new daemon request and no protocol change.
- Transfers (`push`, `pull`, drag-and-drop upload) now work on stock macOS: TERYON skips macOS's built-in openrsync (it has no `--protect-args`) in favour of a Homebrew rsync 3 and asks the far side for one too (`--rsync-path` with Homebrew on PATH). Install it with `brew install rsync` on both machines.
- Panes are now rounded cards inset from the window edge (`[desktop] pane_inset`, default 10, 0 restores edge to edge) with a hairline border and a soft accent ring on the focused pane; dividers show only while hovered or dragged.
- Palette shortcuts are drawn as keycaps at the row's right edge, and menu and palette rows span the full width, so hints and submenus no longer overlap their labels.
- A lone, nearly empty pane shows a faint mark with the shortcuts for the palette, Ask AI and the command bar; it disappears once there is output.
- Menus gain an icon column (Font Awesome glyphs from the bundled Nerd Font, now also a fallback for the UI font), small group headers (Workspace, Edit, Help) in the ▾ menu, and a 120 ms ease on the row highlight.
- Top bar: 34 px with a bottom hairline, and Commands and Ask AI share one quiet tray so "New pane" is the only filled control. The focus ring fades with focus; the palette has icons, a SUGGESTED/RESULTS label and eased row hover; floating surfaces share a 12 px radius, a lighter edge and a softer shadow.
- The top bar shows latency only when it is above 150 ms (the status dot still has it in its tooltip).

## 0.14.0 — `teryon mcp`

- **MCP server for agents**: `teryon mcp [--allow-write] [--host H] [--workspace W]` lets Claude Code, Codex and other MCP clients use TERYON's persistent panes, local or over SSH. Read tools (`list_workspaces`, `list_panes`, `read_pane`, `search_scrollback`, `wait_for`) are always on; `new_pane`, `send_keys` and `close_pane` need `--allow-write` and only touch the server's own workspace. Hosts are limited to `local`, configured hosts and SSH aliases; returned text is redacted. `--print-config` prints the setup line for Claude Code and a Claude Desktop snippet. See `docs/MCP.md`.
- Verified end to end with Claude Code itself starting a pane, waiting for its output and reading it back.

## 0.13.0 — AI in the prompt

- **Command bar** (⌘⇧K, also in the ▾ menu and the palette): describe what you want ("show what listens on port 3000") and get one command with a one-line explanation and a risk label, using the pane's directory, shell and OS. **Enter inserts it at your prompt; nothing runs until you press Enter.** ⌥Enter runs it directly only when it is single-line, rated low risk and not on the denylist; blocked commands cannot be inserted at all.
- **Explain / fix**: click a failed-command chip (✗ 2) and the assistant explains the failure from the redacted tail of the pane and proposes a fix as a card with **Insert in pane**.
- New assistant tool `suggest_command`: proposes a command for you to review; it is never run by the tool.
- `teryon ai --command "find the 5 biggest files"` prints a command for scripts and shell aliases.

## 0.12.0 — Agent-aware workspaces

- AI coding agents (Claude Code, Codex, Gemini CLI, aider, Cursor, OpenCode) are recognized in panes from the pane's foreground command. Each shows a status chip (working / needs you / idle / done); **⌘⇧A** opens an Agents overview and jumps to a pane. When a background agent needs you or finishes you get a notification (`[desktop] agent_notifications`).
- "While you were away": reattaching to a persistent workspace after a few minutes shows what finished or failed meanwhile and which agents are waiting, with a ✦ Summarize button. It works from shell-integration marks, so it needs no AI unless you ask for the summary.
- Privacy guard: terminal text and command output sent to the assistant are scrubbed of API keys, tokens, JWTs, private keys and `NAME=secret` values (`[redacted:…]`), and are framed as untrusted data so instructions inside terminal output are not followed.
- The daemon reports each pane's foreground command (`fg`); additive protocol field, older clients and daemons keep working.

## 0.11.0 — AI assistant

- `teryon ai` and a ⌘J panel in the desktop app: chat with Claude and let it open or quit apps, open SSH terminals, answer questions about your Mac and work with your panes. Anything that changes something shows an approval card (Allow / Always this session / Deny); a few dangerous commands are refused outright. Needs `ANTHROPIC_API_KEY` (or `teryon ai --set-key` for the Keychain); `teryon ai --check` verifies it. See `docs/AI.md` for the permission and privacy model.
- `[ai]` config section (`model`, `max_tokens`, `auto_allow`); no key is ever stored in config.
- Requests go through the system `curl` (no new dependencies); the key is passed on stdin, not in the process list.
- File and folder icons work out of the box: the desktop bundles the Symbols Nerd Font (MIT, `assets/fonts/`) as the last monospace fallback, so `eza --icons`, `lsd`, oh-my-posh and similar tools show their glyphs without installing a patched font. The binary grows by about 2.5 MB.
- Mouse menus: right-click in a pane for Copy, Paste, Select all, Copy last output, Find, Quick select, splits, float, zoom, rename and close; a ▾ dropdown next to "+" in the bar holds the same, grouped into short submenus (Copy Special, Pane, Layout, Snippets, View), plus New tab, Open workspace, Layout, Snippets, View and Help. Programs that track the mouse keep the right button (Shift+right-click opens the menu).
- `Makefile`: `make run`, `make app`, `make check` and friends (`make help`).

## 0.10.0 — Remote-native extras

- Floating panes (⌥⌘↩ / ⌥⌘N): lift a pane into a draggable, resizable window over the layout.
- Drag and drop files onto a pane: quoted paths are typed on `local`; on SSH hosts the files are uploaded with rsync to the pane's directory without overwriting existing files.
- Triggers: `[[desktop.triggers]]` regex rules that highlight, notify, beep or toast; they fire once per new line and are silent for what was already on screen.
- Saved layouts: the desktop keeps each workspace's pane names and directories and offers "Restore N panes" on a fresh workspace (for example after a reboot); `teryon connect --restore` does it for the terminal UI. Commands are never rerun.
- `regex` is now a direct dependency (also used by `teryon config --check`); the layout file code moved into the library.

## 0.9.0 — Shell-aware features

- `teryon shell-init zsh|bash|fish`: optional OSC 133/7 integration (also offered by `teryon setup`, guarded and with a backup). Verified against real zsh, bash and fish sessions.
- The daemon records prompt/command/finished marks with exit status and duration at absolute line numbers, and the shell's live working directory. Additive protocol fields only: old clients and daemons keep working, no version bump.
- `vt100` is vendored with a small line-counter patch (`vendor/vt100/PATCH.md`); `Capture` now returns exactly one line per row plus the first line's number. `TERYON_SCROLLBACK` sets the daemon's history length.
- Desktop: ⌘↑/⌘↓ jump between prompts, ⇧⌘O copies the last command's output, exit-status/duration chips in pane title bars, notifications for long commands that finish while the window is in the background, new panes start in the focused pane's directory. Page scrolling moves to ⇧⌘↑/⇧⌘↓.
- The terminal UI does not show these yet.
- Tests: the two flaky `tests/sessions.rs` cases are fixed (an echoed command matched its own output marker; macOS `od` spacing and raw-mode line endings), and new real-PTY tests cover marks, exact line numbers and zsh/bash/fish integration.

## 0.8.0 — Keyboard power tools

- Quick select (⇧⌘E): labelled URLs, paths, SHAs, IPs, UUIDs and emails on screen; copy, paste or open with a keystroke.
- Copy mode (⇧⌘X): vi-style keyboard selection (characters, lines, blocks, search) over the pane's whole history.
- Snippets: `[[desktop.snippets]]` saved commands with `{{placeholders}}` in the palette; multi-line snippets are never auto-run.
- Broadcast input (⌥⌘I) to every visible live pane, with a visible BROADCAST marker and automatic reset on tab switch.
- Find gets regular-expression and match-case toggles (⌥⌘R, ⌥⌘C) with inline errors for bad patterns.

## 0.7.0 — Easy start and our own mark

- New TERYON logo: a blue T with the cursor waiting beside it (replaces the Zyvor Z everywhere: icon, wordmark, app, banner, site).
- `teryon` with no arguments opens a host/workspace picker (recents, config hosts, `~/.ssh/config`, `local`); `--desktop` or `start = "desktop"` opens the app instead. Recents are remembered in `~/.teryon/recent.toml`.
- `teryon setup`: a re-runnable guided check (tools, SSH host import, reachability with the exact fix command, theme).
- `teryon themes --import FILE` converts iTerm2, Alacritty, Ghostty and Kitty color schemes; user themes in `~/.config/teryon/themes/` appear everywhere themes are listed.
- Desktop: welcome card with the fix command when a host is unreachable, ⌘O to open any workspace as a new tab on any host, ⌘/ shortcut cheat sheet, a one-time first-run hint, and `theme_light` / `theme_dark` to follow macOS appearance.
- `import-ssh` no longer writes the example host when it has to create the config.

## 0.6.0 — One slim bar, Ghostty-style desktop

- The sidebar and status bar are gone: one 30 px top bar (window buttons merged into it on macOS) holds the workspace/tabs, pane chips, layout switch, New pane and the palette; messages are short toasts.
- Splits: weighted dividers for every layout, directional focus (⌥⌘ arrows), keyboard resize and equalize, new pane right/below, unfocused-pane dimming, exit notifications while the window is in the background.
- Terminal: word/line/block selection, shift-extend, copy-on-select, wheel scrollback from the daemon's retained text, xterm mouse reporting for vim/htop/tmux, wheel-as-arrows in full-screen programs, block/bar/underline cursor with blink.
- Config: typed `[desktop]` section in `config.toml` (font family and size, line height, padding, cursor, opacity, dimming, scrollback, pane titles, confirm quit) that reloads live; rebindable keys via `[desktop.keys]` with conflict detection; `teryon config`, `--check` and `--keys`.
- Palette shows live theme previews and gains next/previous/random theme, light/dark switch, fullscreen, always-on-top and reload-config actions. New `docs/DESKTOP.md`.
- Not supported (documented): ligatures, background blur, kitty protocols, OSC 8 links, shell-integration prompt jumps, native tabs and a global-hotkey quick terminal.

## 0.5.0 — Real macOS type, icon rail, compact bar

- Fonts: SF Pro now renders at true regular / semibold / bold weights (variable-font axis set through a small vendored `epaint` patch, see `vendor/epaint/PATCH.md`), SF Pro Rounded for section labels, SF Mono for terminals, and macOS sizes (13 px body, 11 px caption). Falls back to built-in fonts when the system files are absent.
- Sidebar has three states: full, a 46 px icon rail (pane chips plus new/find/copy/draft/theme icons) and hidden; ⌘B cycles them and narrow windows collapse automatically.
- One compact 36 px top bar with icon buttons and an icon layout switcher; per-pane title bars (double-click to rename, zoom, close, directory and exit chips); status bar shows latency, pane count, layout and font size.
- Command palette is a fuzzy-searchable action list with shortcut hints (themes, layouts, tabs, font size, find...); typed commands with arguments still work.
- Find bar (⌘F, ⌘G / ⇧⌘G, match counter), drag-to-select with ⌘C copy, ⌘-click opens http(s) links.
- Remembers window size, theme, font size, sidebar and layout in `~/.teryon/desktop.toml` (0600); ⌘1–9 jump between tabs, ⇧⌘←/→ reorder them, ⌘T / ⌘W open and close.

## 0.4.0 — Mac27 look

- New default `mac27-dark` (and `mac27-light`): macOS-style chrome in the desktop app (8px rounded filled buttons, source-list sidebar, segmented tab and layout controls, soft popup shadows, SF Pro Rounded section labels) and a system-color ANSI palette.
- Mac27 themes follow the macOS accent color (`AppleAccentColor`); primary buttons are darkened as needed to keep white text readable. Custom and other built-in themes are unchanged.
- Desktop sidebar can be hidden or shown (button or ⌘B) and resized by dragging its edge (140–420 px).
- GitHub Pages landing page in `site/` with a deploy workflow.
- `apple-dark` / `apple-light` remain available with their pill style.

## 0.3.0 — Apple look, Zyvor Z mark

- `apple-dark` (default) and `apple-light` themes from the netra apple.com contract; themes gain optional `primary` and 16-color `ansi` fields; every theme now paints ANSI 0-15 (derived when not defined), in both the TUI and the desktop.
- Desktop: SF Pro / SF Mono loaded from macOS at runtime when present, pill buttons with hairlines, a single blue primary action, raised popups, painted Zyvor Z mark.
- Logo and wordmark are now the Zyvor Z mark; the banner uses Zyvor orange.
- New shells get `CLICOLOR` and a tuned `LS_COLORS`/`LSCOLORS` when unset; the TUI footer rotates short tips; the header is graphite with a green status dot.
- `doctor HOST --fix --binary PATH [-y]` deploys a binary when no compatible daemon answers (asks first).
- `record HOST -w NAME --pane N -o FILE.cast` writes an asciicast v2 file (0600, never overwrites) from screen snapshots, not the raw byte stream.
- Desktop: workspace tabs (`tab NAME`, `closetab`, "New tab"), search hits tinted in the canvas, and a draggable divider for two-pane layouts.
- `themes --export wezterm` carries the 16 ANSI colors and, for Apple themes, an SF Mono font hint.

## 0.2.0 — renamed ZYLINK → TERYON

- New logo, wordmark, `.icns` app icon, and a gradient terminal banner (`teryon about`, `connect`; disable with `--no-splash`, `splash = false` or `TERYON_NO_SPLASH`).
- Tokyo Night (matching WezTerm's "Tokyo Night (Gogh)") is the default theme; added Tokyo Night Storm, Catppuccin Mocha, Dracula, Gruvbox Dark, Nord, Rosé Pine and Solarized Dark. `themes --export wezterm NAME` prints a WezTerm color block.
- TUI: single/vertical/horizontal/grid layouts (up to 4 panes visible), zoom, fuzzy pane switcher, close shortcut, OSC 52 yank from retained output, optional mouse focus, and a status line with layout, pane count, poll latency and clock.
- Desktop: layout selector for up to 4 panes, hover-to-preview themes, logo window icon and sidebar mark.
- New commands: `attach`, `dashboard`, `import-ssh`, `completions`, `about`; `exec --wait --notify`.
- Not included in this release: `doctor --fix`, session recording, desktop tabs, font-family picker, protocol ping.

- Crate, binary, environment variables (`TERYON_*`), config directory (`~/.config/teryon`) and runtime directory (`~/.teryon/run`) were renamed.
- `~/.config/zylink/config.toml` is still read when no TERYON config exists; it is never moved or modified.
- A pre-rename daemon and a renamed client use different sockets and will not attach to each other; restart remote services after upgrading.

## 0.1.0 — engineering preview

- Persistent per-user PTY service with authenticated local IPC.
- OpenSSH transport, reconnecting TUI and optional native GPU desktop client.
- Eight embedded themes and custom TOML theme files.
- Exclusive controllers, read-only viewers and ordered input acceptance.
- Pane controls, retained-output search/export and offline drafts.
- Durable tasks, directory/title layouts, rsync transfers and loopback forwards.
- Deployment/install tooling, Linux/macOS CI definitions and integration fixtures.

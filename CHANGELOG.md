# Changelog

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

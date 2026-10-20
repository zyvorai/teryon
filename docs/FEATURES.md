# TERYON features

Everything below is implemented in the current release. For limits and what is not done, read [LIMITATIONS.md](LIMITATIONS.md); for how each claim was checked, [TESTING.md](TESTING.md).

## Included features

| Area | Implemented behavior |
|---|---|
| Persistent sessions | Per-user daemon owns real shells and PTYs independently of clients |
| Reconnection | Interactive clients retry with capped backoff; connection failures discard queued client actions |
| Control | One controller per workspace, multiple server-enforced read-only viewers |
| Input integrity | Ordered sequence numbers suppress duplicates within one live connection; uncertain input is never automatically replayed |
| Terminal | ANSI colors, alternate screen, Unicode cells, cursor/application modes, bracketed paste, common function/control keys, cursor/status query responses |
| Pane management | Create, focus, rename, explicitly terminate; single, vertical, horizontal and 2×2 grid layouts (up to 4 panes visible), or a free-form tree of splits, zoom, fuzzy pane switcher, optional mouse focus |
| Native desktop | GPU canvas, pane navigation, command palette, theme chooser, font size control, copy visible screen |
| Desktop polish | Bare window with a bottom tab strip (program icons, busy spinner, ×), transparency and macOS background blur, gradient and grain backgrounds, rounded corners, native full screen, accent scrollbar, eased cursor, smooth scroll-back, theme cross-fade, inline images (iTerm2 `OSC 1337` and the Kitty graphics protocol, PNG/JPEG); see [DESKTOP.md](docs/DESKTOP.md) |
| Power tools | Quick select, vi-style copy mode, snippets, broadcast input, regex find, floating panes, drag-and-drop upload, triggers |
| Shell-aware | Optional shell integration: prompt jumping, last-command output, exit-status chips, long-command notifications |
| Zsh power features | Built into the shell integration: history suggestions, syntax highlighting, fuzzy Ctrl-R / Ctrl-T / Alt-C / Tab, `z` directory jump and an optional git prompt (`docs/SHELL-INTEGRATION.md`) |
| Remote transport | Over SSH the desktop app receives screen diffs (a scroll plus changed rows, checksummed, with automatic full resync) as compact text, polls quiet sessions less often, explains why a link is down (Wi-Fi/DNS, host asleep, key refused, changed host key, teryon missing) and can take over the controller seat from another device |
| Terminal compatibility | OSC 8 hyperlinks (⌘-click), Kitty keyboard protocol (disambiguate flag), Kitty graphics and iTerm2 inline images, mouse reporting; see [LIMITATIONS.md](docs/LIMITATIONS.md) for the gaps |
| AI agents | Status chips and an overview (⌘⇧A) for Claude Code, Codex, Gemini, aider, Cursor and OpenCode; start one in a pane or its own Git worktree, broadcast a prompt to idle agents, see what it changed, and run it in `teryon sandbox` (writes limited to its repository, credentials hidden) |
| Command history | Shell integration records each command with its exit status, duration, directory and Git branch, never keeps commands that contain a secret, searches with `is:failed`, `exit:`, `slow:`, `dir:`, `branch:` and `host:` filters, and can pull other hosts' history (`teryon hist pull HOST`) |
| Updates | `teryon update` and the palette's "Check for updates" say whether a newer release is out; nothing is installed automatically |
| Easy start | `teryon` picker, `teryon setup`, theme import from iTerm2/Alacritty/Ghostty/Kitty, saved layouts that come back after a reboot |
| File-type colors | New shells get `CLICOLOR`/`LS_COLORS`/`EZA_COLORS` (only if unset; separate colors for source, config, docs, media, archives), and every theme paints ANSI 0-15, so `ls` directories, executables, symlinks, archives and media look consistent |
| Terminal UI | Responsive sidebar, pane borders, status bar, help overlay, prefix shortcuts |
| Themes | Apple Dark (default, follows the netra/apple.com contract), Apple Light, Tokyo Night, Tokyo Night Storm, Tokyo Night Day, Carbon, Catppuccin Mocha, Dracula, Gruvbox, Nord, Rosé Pine, Solarized, Obsidian, Pearl, Aurora, Ember, Ocean, Sakura, Forest, High Contrast; custom TOML palettes; WezTerm export |
| Output | Bounded 2,000-line terminal scrollback, retained-output search, private text exports |
| Offline drafts | In-memory local draft editor with explicit review and submission |
| Durable tasks | Spawn an argv-based command in a persistent pane; optionally wait and return its exit status |
| Layouts | Save initial directories/titles; restore them as new shells without replaying old commands |
| File transfers | Resumable rsync uploads/downloads with protected path arguments and no automatic deletion |
| Ports | Loopback-only OpenSSH local forwarding; optional reconnect loop |
| Deployment | SCP upload, remote executable verification, atomic same-filesystem binary replacement, remote doctor |
| Projects | `.teryon.toml` recipes: `teryon up` starts a workspace's panes, directories and commands, in dependency order (`after`, `ready_text`, `ready_port`) and with restart policies |
| Operations | `dashboard` host overview, `agents` across workspaces, `wake` (wake-on-LAN), `import-ssh` aliases, shell completions, `exec --notify` desktop notifications |
| Diagnostics | Reachability, actual daemon OS/architecture, process ID and workspace/pane counts |
| Local IPC | Private Unix socket preferred; authenticated ephemeral loopback TCP fallback in restricted environments |

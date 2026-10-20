# TERYON Roadmap

What has shipped and what is still open, based on the current limitations (`docs/LIMITATIONS.md`), what users are likely to hit, and the Mac mini / AI-agent / infrastructure focus.

**Status**: Engineering preview v0.20.0. Each shipped item has tests; the desktop items have not yet all been checked in a real window (see the milestones in `docs/LIMITATIONS.md`).

## Shipped in 0.20.0

| Area | What |
|---|---|
| Transport | Screen diffs over SSH (a scroll plus the changed rows, checksummed, with automatic full resync), compact base64 screen bytes, slim polls that resend pane details only when they change, slower polling for quiet sessions |
| Remote reliability | Connection recovery inspector (why the link is down and what to do), reconnect jitter, resume-after-sleep detection, controller takeover |
| Terminal compatibility | OSC 8 hyperlinks, Kitty keyboard protocol (disambiguate flag), Kitty graphics protocol (PNG and raw pixels), iTerm2 inline images |
| Layouts | Free-form pane tree with draggable dividers, alongside single, columns, rows and grid |
| Agents | Overview panel with summaries, spawn in a pane or a Git worktree, broadcast a prompt to idle agents, finish notifications, **Changes** view, `teryon sandbox` and `agent_sandbox` |
| Projects | Recipes start panes in order (`after`, `ready_text`, `ready_port`), and can restart a command (`restart = "on-failure"`) |
| History | Secrets are never recorded (`hist scrub` for old ones), filter words (`is:failed`, `exit:`, `slow:`, `dir:`, `branch:`, `host:`), the Git branch of each command, `teryon hist pull HOST` for other machines |
| Safety | Confirmation before a tab or window closes running shells |
| Away | "While you were away" timeline of finished commands and waiting agents |
| Updates | `teryon update`, palette "Check for updates", optional daily check (off by default), nothing installed automatically |

## Still open

### Verification (do first)
- Real-window check of the 0.20 desktop features, on physical Apple Silicon, with laptop sleep, long builds and real SSH links.
- Published, reproducible latency, memory and bandwidth measurements against tmux, Zellij, WezTerm and Mosh.
- A long agent session through `teryon sandbox` on macOS and Linux (the Linux `bwrap` path has never been run).

### Layouts and windows
- Save the pane-tree shape across restarts.
- Cross-window tab dragging and multi-monitor workspaces.

### Terminal fidelity
- Sixel; Kitty graphics extras (files, shared memory, zlib, animation, Unicode placeholders); the rest of the Kitty keyboard flags; configurable font fallbacks.

### Packaging
- Build the signed macOS packages in CI without putting signing material in the repository.
- Decide whether the app should ever install updates itself (today it only tells you).

### Sessions and operations
- Persist working directory, environment and recent history across daemon restarts; optional tmux/Zellij backend for stronger process survival.
- Automatic port discovery with one-click forwarding in the UI (discovery and open-in-browser exist).
- Host metrics history graphs, a per-process view, Apple Silicon GPU and scheduling hints.
- Persistent offline drafts on disk.

### AI, agents and collaboration
- Agent workspace presets and richer MCP tools for long-running tasks; test-result views for agents.
- Multi-user shared workspaces (invitation links, roles, optional screen broadcasting).
- Bidirectional file sync and editor integration; delivery guarantees and a mobile companion for notifications.

### Not planned
- A native Windows client.

## Implementation notes

- Protocol changes need careful versioning and backward compatibility: new fields are optional on the wire and capabilities are negotiated (`Poll{diff}`, `Info{diff}`).
- Terminal fidelity work touches the vendored `vt100` and the daemon's reader thread; see `docs/ARCHITECTURE.md`.
- Packaging lives in `scripts/` and `.github/workflows/`.
- Every new feature includes tests and updates `docs/LIMITATIONS.md`.

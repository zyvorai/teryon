# TERYON Roadmap

Prioritized feature additions based on current limitations (`docs/LIMITATIONS.md`), user feedback potential, and alignment with Zyvor’s Mac mini / AI-agent / infrastructure focus.

**Status**: Engineering preview v0.19.0. These items move Teryon toward a reliable daily driver.

## P0 — Daily-driver foundations

### 1. Differential / incremental screen transport
- Replace full changed-screen snapshots with delta encoding.
- Reduce bandwidth and latency on high-output sessions.
- Improve controller heartbeat and force-takeover.

### 2. Mouse protocol forwarding + richer terminal compatibility
- Mouse reporting already works in the desktop app; remaining: TUI parity and a compatibility suite (Vim, Neovim, tmux, Claude Code).
- Kitty graphics protocol and/or Sixel support.
- Hyperlinks, better inline images, complete modern terminal queries.
- Configurable font fallbacks.

### 3. Automated signed/notarized macOS packaging + auto-update
- CI builds of signed `.dmg` / `.pkg`.
- Sparkle or equivalent auto-update mechanism.
- Optional App Store track later.

## P1 — Power-user and reliability

### 4. Arbitrary pane layouts
- Full pane tree (today: single, columns, rows or grid with draggable dividers).
- Multi-monitor workspace support.
- Cross-window tab dragging.

### 5. Improved session durability
- Persist cwd, environment, and recent history across daemon restarts.
- Optional tmux/Zellij backend for stronger process survival.
- Better layout restore semantics.

### 6. Automatic port discovery and one-click forwarding
- Listening ports are already detected and can be opened from the palette; remaining: one-click local forward and health checks.
- Failure and permission tests.

### 7. Persistent command history + secret redaction
- Durable, searchable command history store.
- Robust secret redaction before any external send (AI, notifications).
- Optional encrypted transcript archive.

## P2 — AI, agents, and ops

### 8. Deeper MCP and agent tooling
- Richer tools for long-running tasks, workspace templates.
- Agent workspace presets (Claude Code, Codex, custom).
- Integration hooks for FluxVM / Kairon sandboxes.

### 9. Enhanced host metrics dashboard
- History graphs for load, memory, temperature.
- Per-process view.
- Apple Silicon GPU utilization and scheduling hints.
- Better wake-on-LAN and power management.

### 10. Persistent offline drafts
- Save unsent drafts to disk (`~/.teryon/drafts/`).
- Survive client restarts.

## P3 — Reach and collaboration

### 11. Windows client
- Native desktop via egui (already cross-platform capable).
- Or high-quality WSL path.

### 12. Multi-user shared workspaces
- Invitation links.
- Role-based access (controller / editor / viewer).
- Optional screen broadcasting.

### 13. Bidirectional file sync and editor integration
- Continuous or on-demand sync beyond rsync CLI.
- Inline preview and “open in local editor” flows.

### 14. Mobile companion / stronger notifications
- Delivery guarantees and retries.
- Lightweight mobile monitoring for long builds and agents.

---

## Implementation notes

- Protocol changes (P0 transport) require careful versioning and backward compatibility.
- Terminal fidelity work touches `vt100` integration and desktop rendering.
- Packaging work lives in `scripts/` and `.github/workflows/`.
- All new features should include tests and updates to `docs/LIMITATIONS.md`.

See also the “Next engineering milestones” section in `docs/LIMITATIONS.md`.

# Preview limits and next milestones

Implemented features are documented in the README. This file names the gaps so a roadmap item cannot be mistaken for tested behavior.

A prioritized, higher-level roadmap lives in [ROADMAP.md](../ROADMAP.md).

## Limits in 0.19

- macOS/Linux only. WSL can run the Linux terminal build; there is no native Windows client.
- Reboots, daemon crashes and service termination lose live PTYs. Layouts recreate new shells. No process-memory checkpointing or transparent shell restoration is implemented.
- No QUIC, Mosh-compatible roaming protocol, predictive echo, automatic public relay, NAT traversal or zero-latency guarantee. OpenSSH reconnects after transport failure.
- A dead controller is released on EOF or the daemon's idle read timeout (45 seconds). During unusual half-open failures another controller may need to wait. There is no remote force-takeover button.
- Full changed-screen snapshots and request/response polling are simple but bandwidth/latency intensive. No benchmark proves this faster than Zellij, WezTerm, tmux or Mosh.
- Layouts are single, columns, rows or a grid with draggable dividers; there is no arbitrary pane tree, cross-window tab dragging or multi-monitor workspace layout.
- The desktop app forwards mouse reports to programs that ask for them and draws iTerm2-protocol inline images. There is no kitty graphics or keyboard protocol, sixel, complete modern terminal query support, or universal TUI compatibility. Validate your editor and shell. The TUI supports cell styling; desktop italic styling and complete font fallback remain limited.
- Remote files are transferred through CLI commands. There is no continuous bidirectional editor sync or inline browser preview. Listening ports are detected and can be opened in the browser from the palette, but are not forwarded automatically.
- Output search covers the retained terminal buffer, not an unlimited historical command database. No semantic command blocks, completion engine of its own (the zsh Tab menu uses zsh's completion), per-command telemetry, robust secret redaction or permanent encrypted transcript store.
- Offline drafts are memory-only. Closing the client loses unsent drafts. A saved draft can contain multiple commands; review it before submission.
- Notifications: the daemon can push agent and long-command events to ntfy or a webhook (`[notify]`), and the desktop app does the same from the background, but there is no delivery guarantee, no retry and no mobile app of our own. They need `curl`.
- Device handoff means detach one controller and attach another. No invitation links, separate-user RBAC, hosted collaboration, screen broadcasting or organization administration.
- The top bar shows load, memory and a heat warning for the daemon's host, with an opt-in `caffeinate` keep-awake and `teryon wake`. There is no history graph, per-process view, Apple GPU scheduling or GPU workload control, and wake-on-LAN only works over Ethernet to a host that is asleep, not off.
- SSH requires existing noninteractive key/agent authentication. Password onboarding and first-use host-key enrollment remain ordinary OpenSSH tasks.
- Signed and notarized `.dmg`/`.pkg` are built by hand on a Mac with `scripts/release-macos.sh` and attached to the GitHub Release; CI does not build them. App Store submission and automatic updates are not included.

## Next engineering milestones

1. Exercise the release on physical Apple Silicon hardware, including laptop sleep, mini sleep, long builds, zsh plugins, Vim/Neovim, SSH jump hosts and long-loss network conditions.
2. Measure latency, memory and bandwidth against established tools with published reproducible workloads.
3. Improve differential screen transport, controller heartbeat/takeover and reconnect diagnostics. Clients already warn when the daemon instance changes.
4. Add mouse support, richer terminal compatibility and configurable font fallbacks before claiming a daily-driver replacement.
5. Add integrated transfers, discovered ports and signed native packaging, each with failure and permission tests.

See [ROADMAP.md](../ROADMAP.md) for a prioritized breakdown of additional features (pane trees, durable history, AI/agent depth, Windows client, collaboration, etc.).

The goal is a strong remote workspace. “Best in the world” requires evidence from real users, compatibility testing and performance measurements; it is not a release flag.

* Inline images are iTerm2-protocol only (no Kitty or Sixel), draw PNG and JPEG, and assume 8×16 px cells when sizing; a daemon started before the upgrade keeps running the old code, so stop it (`teryon stop --force`, which ends its shells) after updating to get images.

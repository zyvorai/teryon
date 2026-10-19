# Preview limits and next milestones

Implemented features are documented in the README. This file names the gaps so a roadmap item cannot be mistaken for tested behavior.

## Limits in 0.1

- macOS/Linux only. WSL can run the Linux terminal build; there is no native Windows client.
- Reboots, daemon crashes and service termination lose live PTYs. Layouts recreate new shells. No process-memory checkpointing or transparent shell restoration is implemented.
- No QUIC, Mosh-compatible roaming protocol, predictive echo, automatic public relay, NAT traversal or zero-latency guarantee. OpenSSH reconnects after transport failure.
- A dead controller is released on EOF or the daemon's idle read timeout (45 seconds). During unusual half-open failures another controller may need to wait. There is no remote force-takeover button.
- Full changed-screen snapshots and request/response polling are simple but bandwidth/latency intensive. No benchmark proves this faster than Zellij, WezTerm, tmux or Mosh.
- Two visible panes in a vertical split; no arbitrary pane tree, drag resizing, horizontal split, cross-window tab dragging or multi-monitor workspace layout.
- No mouse-protocol forwarding, kitty graphics, sixel, hyperlinks, native inline images, complete modern terminal query support, or universal TUI compatibility. Validate your editor and shell. The TUI supports cell styling; desktop italic styling and complete font fallback remain limited.
- Remote files are transferred through CLI commands. There is no integrated file browser, drag/drop transfer UI, continuous bidirectional editor sync, automatic port detection or inline browser preview.
- Output search covers the retained terminal buffer, not an unlimited historical command database. No semantic command blocks, completion engine of its own (the zsh Tab menu uses zsh's completion), per-command telemetry, robust secret redaction or permanent encrypted transcript store.
- Offline drafts are memory-only. Closing the client loses unsent drafts. A saved draft can contain multiple commands; review it before submission.
- Per-pane completion notices exist only while the client observes state. No push service, mobile notifications, background desktop notification integration or guaranteed offline completion delivery.
- Device handoff means detach one controller and attach another. No invitation links, separate-user RBAC, hosted collaboration, screen broadcasting or organization administration.
- No CPU/memory/thermal dashboard, Apple GPU scheduling, GPU workload control, sleep prevention or wake-on-LAN integration.
- SSH requires existing noninteractive key/agent authentication. Password onboarding and first-use host-key enrollment remain ordinary OpenSSH tasks.
- Signed and notarized `.dmg`/`.pkg` are built by hand on a Mac with `scripts/release-macos.sh` and attached to the GitHub Release; CI does not build them. App Store submission and automatic updates are not included.

## Next engineering milestones

1. Exercise the release on physical Apple Silicon hardware, including laptop sleep, mini sleep, long builds, zsh plugins, Vim/Neovim, SSH jump hosts and long-loss network conditions.
2. Measure latency, memory and bandwidth against established tools with published reproducible workloads.
3. Improve differential screen transport, controller heartbeat/takeover and reconnect diagnostics. Clients already warn when the daemon instance changes.
4. Add mouse support, richer terminal compatibility and configurable font fallbacks before claiming a daily-driver replacement.
5. Add integrated transfers, discovered ports and signed native packaging, each with failure and permission tests.

The goal is a strong remote workspace. “Best in the world” requires evidence from real users, compatibility testing and performance measurements; it is not a release flag.

* Inline images are iTerm2-protocol only (no Kitty or Sixel), draw PNG and JPEG, and assume 8×16 px cells when sizing; a daemon started before the upgrade keeps running the old code, so stop it (`teryon stop --force`, which ends its shells) after updating to get images.

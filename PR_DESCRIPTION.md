# Add TERYON persistent remote workspaces

Add a Rust terminal workspace whose per-user remote daemon owns PTYs independently of SSH clients. Interactive TUI/native clients reconnect to existing shells, enforce a single controller with read-only viewers, and discard uncertain client actions rather than replaying input after a failure.

Includes eight themes, pane controls and split view, bounded retained-output search/export, offline drafts, durable argv tasks, fresh-shell layout restoration, protected/resumable rsync transfers with bandwidth limiting, loopback forwarding, deployment tools, tutorials, Linux/macOS workflow definitions, and an explicit compatibility/limitations document.

Validation executed on Linux: 25 desktop-enabled Rust tests, 23 default-build tests, eight real OpenSSH-client integration scenarios, native window launches in both Obsidian/Pearl, formatting and Clippy with warnings denied. The optimized Linux build succeeds with the pinned Rust 1.94.0 compiler; the Apple Silicon macOS target passes compile checking. Test logs and actual UI captures are included. See `docs/TESTING.md` for coverage and limits.

macOS hardware tests, service-manager installation, signing/notarization and hosted CI runs remain unverified. This is an engineering preview; no competitor performance claim is made.

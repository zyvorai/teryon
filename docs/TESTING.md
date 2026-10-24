# Verification report

Executed in an isolated Linux x86_64 environment on 2026-10-17 UTC with the committed lockfile. Initial runtime checks used Rust 1.99.1; the repository pins Rust 1.94.0 for reproducible release builds. No physical Mac mini or macOS runner was available in this session.

## Results

| Check | Result | Evidence |
|---|---|---|
| Rust tests with desktop enabled, pinned compiler | **25 passed**: 13 library/UI tests, 2 CLI tests, 10 subprocess/PTY tests | [rust-pinned.txt](test-results/rust-pinned.txt) |
| Rust tests without desktop | **23 passed** | [rust-terminal.txt](test-results/rust-terminal.txt) |
| Clippy, all targets/features, warnings denied | **Passed** | [clippy-pinned.txt](test-results/clippy-pinned.txt) |
| Optimized Linux build, pinned compiler | **Passed**, desktop included | [release-build.txt](test-results/release-build.txt) |
| OpenSSH/SFTP end-to-end integration | **8 scenarios passed**, including the optimized executable | [ssh-e2e-release.txt](test-results/ssh-e2e-release.txt) |
| Actual native GPU window launch | **Passed** with software OpenGL in Xvfb, Obsidian and Pearl, optimized executable | [native-smoke-release.txt](test-results/native-smoke-release.txt) |
| Apple Silicon macOS target | **Compile check passed**, all features; no macOS binary link or runtime test | [macos-cross-check.txt](test-results/macos-cross-check.txt) |
| Rust formatting, Python syntax and shell syntax | **Passed** | Reproduce using commands below |

Counts overlap across compiler/configuration runs; the totals should not be added together.

The runner's initial Rust 1.99.1 optimized build produced dependency archive/link errors. Pinning Rust 1.94.0 resolved that release-build issue; the pinned compiler is used by the repository and workflow definitions.

## Latest local run (macOS, 0.21.0)

Run on the Apple Silicon Mac the release was prepared on (2026-10-24), with the committed lockfile and the compiler installed there (not the pinned 1.94.0):

| Check | Result |
|---|---|
| `cargo test --features desktop` | **433 passed**: 390 library/UI, 2 CLI, 5 history/update CLI, 2 picker PTY, 1 recovery, 28 session, 5 zsh widget |
| `cargo test --no-default-features` | **273 passed** in the library and CLI test binaries (the desktop-only UI tests are not built) |
| `cargo fmt --check`, `cargo clippy --all-features --all-targets` | **Passed**, no warnings |

Notes on what these do and do not show:

- Run in parallel on a machine with a load average of about 11, three session tests (`a_recipe_starts_named_panes…`, `input_sequence_deduplicates_and_rejects_gaps`, `blocked_pane_input_does_not_stall_the_session_service`) failed across two runs, a different set each time; each passes alone, and the whole session and zsh-widget suites passed with `--test-threads=1`. The daemon was not changed in 0.21.

- The session tests start a real daemon and real shells. Two of them (recipe readiness and a recipe start) failed once each on a heavily loaded machine (load average 5 to 7) and passed on every other run, including eight consecutive runs on `main`; they depend on shell start-up time.
- `teryon sandbox` is tested against the real `sandbox-exec` on macOS (writes outside the repository are denied, a fake `~/.ssh` key is unreadable, the network and `claude --version` still work). The Linux `bwrap` path is only tested for the arguments it would receive; it has not been run.
- The Kitty graphics tests run a real daemon and check the reserved rows, deletion, the program's reply and the stored bytes; no real `kitten`, `chafa` or `timg` was available, and nothing was looked at in the desktop window.
- The desktop features added in 0.21 (the Quick Look code viewer, saved pane trees, the Ports and Host health windows) are covered by unit and app-level tests only; none was checked in a real window, and the release binary's size increase from `syntect` was not measured. The desktop features added in 0.20 (pane-tree dividers, the agent Changes button, Kitty images, the update toast, diffs in the app) are covered by unit and app-level tests but were not checked in a real window. The one real call to GitHub (`teryon update`) reported 0.19.0 as up to date.
- The 4x request reduction from quiet-session polling and the roughly 17x and 7x byte reductions from screen diffs are from arithmetic and one scrolling-output test, not from a benchmark against other tools.

The Linux table above is from an earlier session and its counts are no longer current; the evidence files under `test-results/` were not regenerated. The 0.16.0 `.dmg` and `.pkg` were signed and notarized; the 0.20.0 packages are built by hand on the signing Mac after the tag.

## Real-process coverage

The session tests start a real daemon process and real PTY-backed shells/tasks. They verify:

- Same shell PID and continuing job after client disconnect.
- Exclusive control, read-only viewing, and rejection of mutating read-only operations.
- Duplicate input suppression, sequence-gap rejection and subsequent valid input.
- Resize, Unicode/ANSI screen recovery and unchanged-screen suppression.
- Exit status and retained older output.
- Basic cursor-position replies without a real client terminal.
- Rejection of incorrect local IPC tokens.
- Bounded input backpressure without stalling the whole service.
- Explicit termination of a job that ignores hangup.
- Geometry, workspace-name, shutdown and removal validation.
- Screen diffs: a client that asks for diffs rebuilds a scrolling, colored screen identically, with a full snapshot on a mismatch.
- Kitty graphics: chunked transmit and display reserve the right rows, delete by id removes only that image, escape text never reaches the screen, and the capability query gets its reply.
- Kitty keyboard flags are answered, pushed, popped and left with the alternate screen.
- Recipes: panes start in dependency order, wait for `ready_text` or a listening port, stop with a clear error on a timeout, and refuse to start twice.
- Controller takeover, the connection recovery inspector's causes, and history `pull` over a stand-in `ssh` with a second home directory.

The test runner blocks Unix socket binding, so the Linux session tests exercised the authenticated loopback fallback. GitHub's Linux/macOS jobs use the same tests and normally exercise the preferred Unix-socket path. Those hosted jobs have not run merely because workflow files are present.

The SSH suite uses the real system `ssh`, `scp`, and `rsync` clients against an ephemeral Paramiko SSH/SFTP fixture bound to loopback. The fixture performs key authentication and executes the real Rust relay/daemon; it is not a production server dependency. This proves OpenSSH client interoperability in the fixture, not testing against every OpenSSH server configuration.

Its eight scenarios verify executable upload/installation and remote diagnostics, argv task execution/exit status, layout restoration, interactive TUI transport loss and reconnection to the same shell PID, special-character filename transfers with SHA-256 comparison, an intentionally interrupted upload followed by successful resumption, HTTP forwarding with tunnel reconnection, and explicit forced shutdown.

Native tests include egui frame generation at narrow/wide widths across all themes. The separate Xvfb test launches the actual native window and captures the framebuffer in both dark and light appearance. [Mac27 Dark screenshot](images/desktop-mac27-dark.png) · [Mac27 Light screenshot](images/desktop-mac27-light.png). These are macOS application captures, not image-generated mockups.

Theme tests validate every embedded palette and require at least 4.5:1 contrast for primary/secondary text against the default app surfaces. Explicit colors emitted by remote programs are not recolored by that test.

## Reproduce

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo test --all-targets --no-default-features --locked
cargo build --all-features --locked
python3 -m pip install -r tests/requirements.txt
python3 scripts/e2e.py --binary target/debug/teryon
python3 -m compileall -q scripts
for script in scripts/*.sh; do sh -n "$script"; done
```

Optional native window test on Linux with Xvfb, Pillow and software OpenGL available:

```bash
# Ensure target/debug/teryon was most recently built with --all-features.
python3 scripts/native-smoke.py --binary target/debug/teryon
```

Do not run default-feature rebuilds concurrently with native smoke tests; they share the same output executable. This is a build configuration concern, not a remote session constraint.

## Still requires validation

Broader macOS GUI behavior beyond the one Mac used, and every 0.20 desktop feature in a real window (see the notes above); laptop/mini sleep and wake; launchd/systemd installation on a real login session; notarized builds on other Macs; the 0.16.0 animations (smooth scroll, theme fade, eased cursor); long-lived reliability, latency and memory benchmarks; nontrivial jump-host/server policies; firewall and network blackhole cases; and compatibility with your exact Vim/Neovim, shell plugins and terminal applications.

Provided CI, app-bundle and service scripts are source deliverables, not evidence they were deployed to your account or hardware. These tests publish nothing; releases are cut by hand with `scripts/release-macos.sh`.

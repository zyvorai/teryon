# Contributing

Use the repository-pinned Rust toolchain and keep `Cargo.lock` committed. Format with `cargo fmt --all`. Run Clippy with warnings denied and tests with both default and desktop features before submitting.

Keep terminal process ownership in the daemon. Never put PTY lifetime under an SSH connection object. Never automatically replay uncertain terminal input. New control paths must enforce controller/read-only rules server-side, not only by disabling a UI button.

Add integration coverage for changes to persistence, input ordering, permissions, protocol parsing, process cleanup or reconnect state. UI-only polish should be verified at narrow and normal sizes in both light and dark themes.

Keep protocol changes explicit and versioned. Old/new client and daemon behavior must fail clearly rather than corrupt state. Do not silently migrate or rerun captured shell commands.

State exactly which OS/hardware and scenarios were tested in pull requests. A workflow file that has not run is not evidence of macOS compatibility. Avoid competitor superiority claims without published benchmarks.

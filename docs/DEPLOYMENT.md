# Deployment and releases

## Install from source

Build on each target OS and architecture:

```bash
cargo build --release --locked
./scripts/install.sh ./target/release/teryon
```

For the native desktop client, add `--features desktop`. The repository pins Rust 1.94.0 for reproducible builds. The lockfile is committed; build with `--locked`. Rust dependencies still need to be downloaded on the first build; this is not a vendored/offline archive.

Linux native graphics requires a working X11/Wayland desktop and OpenGL-capable software or hardware driver. macOS graphics is supplied by the OS through eframe's native backend. The headless remote session host does not require the desktop feature.

`scripts/install.sh` installs into `$HOME/.local/bin` by default; `TERYON_INSTALL_DIR` overrides it. Add that directory to your local PATH. Remote TERYON invokes the default path explicitly and does not depend on that PATH entry for its own binary.

## Deploy over SSH

When you already have a binary compiled for the target platform:

```bash
teryon deploy mini --binary ./target/release/teryon
```

Deployment reports `uname`, uploads to a random temporary filename using SCP, makes the upload executable, runs `--version` to check that it executes, then replaces the installed binary and runs `doctor`. It does not cross-compile automatically. A Linux binary cannot run on Apple Silicon; use a macOS build for the mini.

Do not replace a running installation with an untrusted executable. The deploy command deliberately executes the binary you supply. For a custom destination use global `--remote-bin /absolute/path`; its parent directory must exist and be writable.

## Upgrades

An existing daemon keeps running its old executable after a file replacement. A binary upgrade does not migrate live in-memory sessions. Before a protocol-changing upgrade, finish running jobs, detach clients, and explicitly stop the old daemon. Keep the previous binary so you can reinstall it if needed.

```bash
teryon sessions mini
# Finish or deliberately terminate remaining work first.
teryon stop mini --force
teryon deploy mini --binary ./new-build/teryon
```

`--force` terminates panes; it is not a zero-downtime session migration operation.

## Optional user services

`scripts/install-service.sh` writes a launchd LaunchAgent on macOS or a systemd user unit on Linux, without sudo. Run it explicitly only if you want service-manager ownership. It refuses to overwrite an existing unit file. Stop the automatically started daemon first, or the service's lock acquisition will fail.

macOS LaunchAgents normally operate in the logged-in user's context. Do not claim survival across logout or reboot. A Linux systemd user service depends on your system's login/user-manager policy. No system-level service or login-policy changes are made.

## Package for GitHub

The repository includes Linux/macOS check workflows and a tag-triggered release workflow. Source packages include `Cargo.lock`, source, themes, documentation and tests. No credential, runtime token, test key, build cache, or active session state belongs in a release.

```bash
./scripts/package.sh
```

To publish after reviewing the files:

```bash
git init -b main
git add .
git commit -m "Add TERYON persistent Rust workspaces"
# Create an empty repository in your GitHub organization, then:
git remote add origin git@github.com:YOUR_ORG/zyvor-teryon.git
git push -u origin main
```

Replace `YOUR_ORG` with the organization you control. The delivered archive is GitHub-ready source; no repository creation, remote push, Mac deployment, hosted workflow run, code signing or notarization is implied by its existence.

## macOS release (signed and notarized)

CI publishes only the tar.gz archives. The `.dmg` and `.pkg` are built on a Mac holding the Developer ID Application and Installer certificates and a `notarytool` keychain profile:

```sh
scripts/release-macos.sh   # env: MACORA_SIGN_IDENTITY, DEVELOPER_ID_INSTALLER, NOTARY_PROFILE
gh release upload vX.Y.Z dist/TERYON-X.Y.Z.dmg dist/TERYON-X.Y.Z.pkg dist/SHA256SUMS-macos-X.Y.Z
```

`pkgbuild --sign` needs a GUI login session; over plain ssh run the script from Terminal.app.

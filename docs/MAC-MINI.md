# MacBook → Mac mini: complete walkthrough

## 1. Prepare the mini

Enable Remote Login for your own account. Keep the mini reachable on your LAN or an existing VPN. For access from outside your LAN, configure your network/VPN independently; TERYON includes no public relay or NAT traversal service.

Use `hostname` and `whoami` on the mini to identify its host and your account. `mini.local` in these examples is a placeholder; your hostname may differ.

Install a stable Rust toolchain using the official rustup instructions, copy this repository to the mini, and run:

```bash
cargo build --release --locked
./scripts/install.sh ./target/release/teryon
~/.local/bin/teryon doctor local
```

For file transfers, install rsync **3+** on both machines. The older rsync shipped with some macOS releases may not support protected arguments. Ensure the new rsync is on the PATH of noninteractive SSH sessions, not only your interactive shell. An explicit `~/.ssh/environment` approach depends on server policy; alternatively configure your shell's noninteractive startup path.

## 2. Prepare the MacBook

Build locally with `cargo build --release --features desktop --locked`, then run `scripts/install.sh target/release/teryon`.

Create or reuse an SSH key. Establish normal SSH login and verify the fingerprint of the mini before trusting its key. Example `~/.ssh/config` entry:

```sshconfig
Host mini
    HostName mini.local
    User susant
    IdentityFile ~/.ssh/id_ed25519
    IdentitiesOnly yes
```

Test `ssh mini`. TERYON uses `BatchMode=yes` and `StrictHostKeyChecking=yes`: interactive passwords, first-use host-key prompts, or unattended key passphrase prompts will fail. Unlock your existing SSH agent before connecting.

```bash
teryon doctor mini
teryon desktop mini --workspace fluxvm
```

The daemon starts automatically through the authenticated SSH connection. You do not need a root service or an exposed application port.

## 3. Work and disconnect

Run a real build in the terminal:

```bash
cd ~/Developer/zyvor-fluxvm
cargo test --workspace
```

Close the desktop window, or detach from the terminal client with Ctrl+Space followed by `d`. Reopen the same workspace. The session host retains the real shell and PTY while it remains running.

If Wi-Fi drops, the client shows a cached screen and reconnects automatically. It discards pending client actions rather than guessing whether they were received. Check the terminal before retyping a command whose result is uncertain.

For a job started directly from a script:

```bash
teryon exec mini --workspace builds --cwd /Users/susant/Developer/zyvor-fluxvm -- cargo test --workspace
teryon connect mini --workspace builds
```

`--cwd` is a remote absolute path. The task runs in a new pane; it does not execute inside the existing shell or inherit that shell's current environment changes. Use explicit environment variables in your command or a deliberate shell wrapper when needed.

## 4. Development servers

Start a server on the mini, bound to its loopback interface, then forward it:

```bash
teryon forward mini 3000:3000 --reconnect
```

Open `http://127.0.0.1:3000` on the MacBook. Stop the foreground forwarder with Ctrl+C. Only loopback is exposed locally. Reconnection restarts the tunnel, but browsers may need to reload interrupted connections.

## 5. Files

```bash
teryon push mini ./project/ ./project/
teryon pull mini ./project/results/ ./results/
```

Repeat an interrupted command to resume. rsync owns file comparison and partial-transfer behavior. Uploads can replace destination files with the same names; TERYON does not add a delete operation. Do not use transfer commands as a concurrent bidirectional editor sync tool.

## 6. Review or share

```bash
teryon connect mini --workspace fluxvm --readonly
```

This is another attachment to the same account's workspace. It is not a multi-user permission system or an invitation link. Read-only viewing cannot modify pane input, sizes, titles or processes through the session API.

## 7. Clean up

Detach all controllers first, then explicitly terminate an unwanted workspace:

```bash
teryon remove mini --workspace builds --force
```

To stop every session and the daemon:

```bash
teryon stop mini --force
```

These commands terminate processes; use them only when the running work is no longer needed.

# Commands and controls

Run `teryon --help` or `teryon COMMAND --help` for the authoritative flags.

| Command | Purpose |
|---|---|
| `connect HOST -w NAME` | Reconnecting terminal UI; HOST defaults to `local` |
| `desktop HOST -w NAME` | Native app; requires the `desktop` Cargo feature |
| `sessions HOST` | JSON list of workspaces and controller status |
| `panes HOST -w NAME` | Pane IDs, titles, initial directories, PIDs and exit status |
| `capture HOST -w NAME --pane ID` | Plain retained output to stdout; read-only attachment |
| `exec HOST -w NAME -- COMMAND ARGS` | Durable command in a new pane |
| `exec … --wait --timeout SECONDS` | Wait for exit and return its status; timeout leaves job running |
| `exec … --wait --notify` | Also ring the bell, send OSC 9 and (macOS) a desktop notification on exit |
| `send HOST -w NAME --pane ID --enter TEXT` | Explicit input, requiring exclusive controller access |
| `push HOST SOURCE DESTINATION` | Upload via rsync 3+ (on macOS: `brew install rsync` on both machines; TERYON finds the Homebrew one itself, since macOS's built-in rsync is too old) |
| `pull HOST SOURCE DESTINATION` | Download via rsync 3+ |
| `forward HOST LOCAL:REMOTE --reconnect` | Foreground loopback tunnel with optional restart |
| `deploy HOST --binary PATH` | Verify and install an explicitly supplied target binary |
| `doctor HOST [--fix --binary PATH] [-y]` | Report daemon platform and reachability; with `--fix`, confirm then deploy the binary |
| `record HOST -w NAME --pane N -o FILE.cast` | Record a pane as asciicast v2 (screen snapshots; new 0600 file) |
| `layout save HOST -w NAME FILE` | Save initial directories and pane titles to a new local file |
| `layout open HOST -w NEW_NAME FILE` | Restore new shells in a new workspace |
| `attach NAME [--host HOST]` | Shorthand for `connect local -w NAME` |
| `dashboard [--local] [--watch SECS]` | Table of configured hosts: reachability, system, workspaces, panes, latency |
| `sandbox [--allow-write DIR] -- COMMAND…` | Run COMMAND (an AI agent, say) so it can only write to the Git repository it is started in and to scratch/settings directories, and cannot read `~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.kube`; the network stays open (`sandbox-exec` on macOS, `bwrap` on Linux) |
| `up [HOST] [--file PATH] [--workspace NAME]` | Start the workspace a project describes in `.teryon.toml` (`[[pane]]` with `name`, `cwd`, `command`, and optionally `after` / `ready_text` / `ready_port` / `ready_timeout` / `restart`, see below); refuses a workspace that already exists |
| `wake HOST` (or Ctrl+W on a host in the `teryon` picker) | Send a wake-on-LAN packet to a host that has `mac = "aa:bb:cc:dd:ee:ff"` (and optionally `broadcast`) in its `[hosts.NAME]` config; a Mac mini wakes this way over Ethernet only |
| `agents [HOST...] [--all]` | List the AI coding agents in every workspace (host, workspace, pane, agent, working / needs you / idle / done); `--all` covers this machine and every configured host |
| `import-ssh [--file PATH] [--dry-run]` | Add `Host` aliases from `~/.ssh/config` to the TERYON config (append-only) |
| `completions SHELL` | Shell completion script (bash, zsh, fish, elvish, powershell) |
| `about` | Banner, version and active theme |
| `config [--check] [--keys]` | Show or validate the `[desktop]` configuration, or list key bindings |
| `themes --preview` | Show built-in palettes |
| `themes --import FILE [--as NAME] [--force]` | Convert an iTerm2, Alacritty, Ghostty or Kitty color scheme to a TERYON theme in `~/.config/teryon/themes/` |
| `themes --export wezterm NAME` | Print a WezTerm `config.colors` block for a theme |
| `init` | Create config; never overwrite |
| `remove HOST -w NAME --force` | Remove a detached workspace, terminating live panes |
| `stop HOST --force` | Stop the daemon and all live panes |

`HOST` may be a TERYON config alias, an OpenSSH alias, or `user@host`. `local` means the current machine. Host values cannot contain options, whitespace, or shell commands. For a nonstandard SSH port, create an SSH alias rather than appending `:PORT`.

Transfers accept `--limit-kbps N` to limit bandwidth in KiB/s. Repeat the same command after interruption to reuse retained partial data.

Global `--remote-bin /absolute/path` overrides the remote executable path. `TERYON_SSH_CONFIG=/path/to/ssh_config` supplies an alternate OpenSSH configuration to connections, deployment, transfers, and forwards.

## Project recipes: starting things in order

`teryon up` normally starts every pane at once. To start one after another is ready, name the pane to wait for with
`after`, and say how to tell it is ready on that pane:

```toml
[[pane]]
name = "db"
command = "docker compose up db"
ready_text = "ready to accept connections"   # text its output prints, or
# ready_port = 5432                           # a TCP port its processes listen on (both may be set)
ready_timeout = 90                            # seconds, default 60, at most 600

[[pane]]
name = "app"
after = "db"                                  # typed only once `db` is ready
command = "cargo run"
```

All panes are created straight away; only the commands wait. A pane that never becomes ready within its timeout
stops `up` with an error naming what it waited for, and the panes already started keep running. `after` must name one
other pane, and panes may not wait on each other in a circle. Only output after the command's own echo counts as ready text (the shell echoes what was typed, and some prompt themes redraw it), so a command may safely contain its ready text, but `ready_text` should still be something the command prints.
`ready_port` is read from the pane's process tree, which the daemon refreshes every few seconds, so it can lag by
that much.

A command that should come back when it dies takes a restart policy:

```toml
[[pane]]
name = "api"
command = "cargo run"
restart = "on-failure"    # or "always"; default "never"
restart_delay = 2         # seconds before each restart, default 2, at most 3600
restart_max = 10          # give up after this many restarts, default 10, at most 1000
```

`on-failure` runs the command again when it exits with a status other than 0 and stops after a clean exit;
`always` runs it again either way. The pane prints what it is doing (`[teryon] exit status 1; restart 2 of 10 in 2 s`).
It is a small `sh` loop typed into the pane, not a supervisor: it keeps working when no client is connected, over SSH,
and for a command with quotes in it, and **Ctrl-C in the pane stops the loop**. It restarts only the command, not the
panes that wait on it. After `restart_max` restarts it stops and leaves the shell at its prompt.
`ready_text` of a restarting pane is still judged against the first run's output.

## Command palette

TUI: Ctrl+Space, release, then `:`. Desktop: visible Commands button, ⌘K, or Ctrl+Shift+K.

| Palette command | Action |
|---|---|
| `new` | Create a login shell at the remote user's home |
| `new /remote/directory` | Create a shell in an explicit remote directory |
| `rename Build logs` | Rename focused pane |
| `close` | Require confirmation, then kill/remove focused pane; last pane cannot be removed |
| `split` | Toggle between a single pane and a vertical split |
| `layout single\|vertical\|horizontal\|grid` | Choose how many panes (up to 4) are shown and how they tile |
| `zoom` | Temporarily show only the focused pane |
| `switch [TEXT]` | TUI: fuzzy pane switcher |
| `theme NAME_OR_PATH` | Change local appearance |
| `search TEXT` | Case-insensitive search in retained plain output |
| `capture` | Open retained-output viewer |
| `export /local/new-file.txt` | Write private output export; refuse overwrite |
| `draft` | Open local draft editor |
| `send-draft` | TUI: review saved draft before sending |
| `focus ID` | TUI: focus a pane by ID |
| `detach` | TUI: leave shells running |
| `help` | TUI: help overlay |

At most four panes are shown at once; with more, the visible window follows the focused pane. A short last row stretches its panes across the full width.

Mouse support is off by default (`mouse = true` in the config) because capturing the mouse replaces the terminal's own text selection. When on, click a pane or sidebar entry to focus it; the wheel opens retained output. In the retained-output viewer, `y` copies the shown lines through OSC 52, which needs a terminal that allows clipboard writes.

Close a desktop window to detach. Use its draft review controls to submit drafts. Ctrl+C normally goes to the remote foreground process in the TUI; use the prefix detach command to leave the client. Clipboard pastes strip ESC bytes to prevent bracketed-paste delimiter injection. Multiline pastes may execute commands when the receiving application does not enable bracketed paste; review pasted commands first.

## Retention and limits

16 workspaces, 16 total panes, 8 panes per workspace. Each pane retains up to 2,000 terminal scrollback lines. Terminal geometry is limited to 2–160 rows and 10–400 columns; larger views show a bounded terminal. Input is capped at 64 KiB per action. Use a file transfer for larger content. Pane titles accept 1–80 bytes without control characters; workspace names accept 1–64 ASCII letters, digits, `_` and `-`.

Pane directories represent the directory used at creation. `cd` in the shell does not update layout metadata in this release.

## Getting started

* `teryon` (no arguments) opens a small picker: recent workspaces first, then hosts from your config, your
  `~/.ssh/config` and `local`. Type to filter, Enter to connect, Tab to list a host's workspaces, or type
  `user@host` to connect to something new. Add `--desktop` (or `start = "desktop"` in `config.toml`) to open the
  desktop app instead.
* `teryon shell-init zsh|bash|fish` prints the shell integration script (see [SHELL-INTEGRATION.md](SHELL-INTEGRATION.md)).
* `teryon ai [PROMPT]`, `teryon ai --set-key`, `teryon ai --check`: chat with Claude and let it act with your approval (`docs/AI.md`).
* `teryon mcp [--allow-write] [--print-config]`: MCP server so agents can use persistent panes (`docs/MCP.md`).
* `teryon setup [-y]` is a short guided check: tools, importing SSH hosts, reachability (with the exact
  `doctor --fix` command when a host fails) and theme. Every step asks first and it is safe to re-run.

## Desktop

`teryon desktop HOST -w NAME` opens the native app. Shortcuts, options and limits are in [DESKTOP.md](DESKTOP.md);
`teryon config --keys` lists every binding and `teryon config --check` validates your `[desktop]` section.

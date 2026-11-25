# Research: remote shells, multiplexers and agent workspaces

Written for the 0.25 to 0.26 cycle. Every claim marked with a URL was read from that page during this
session; **[U]** marks something recalled from memory or only seen in a search snippet and not read from a fetched
page. Nothing here is a benchmark or a superiority claim.

## What the field does

| Tool | What it does (as read) | Source |
|---|---|---|
| mosh | Keeps a session alive across IP changes and sleep; predicts local echo; runs over UDP (SSP) and synchronises screen state, not bytes. Limits it states itself: only the visible screen is synchronised (no scrollback), needs UDP ports 60000-61000 and a UTF-8 locale. | <https://mosh.org/> |
| Eternal Terminal | Reconnects automatically after outages and roaming, starts through ssh, keeps native scrolling, works with tmux. | <https://eternalterminal.dev/> |
| tmux | Remote persistence by living on the host. `set-clipboard external` (default) lets only tmux set the outer clipboard; with `on`, "if a command can write text to a tmux pane, it can set the clipboard". | <https://github.com/tmux/tmux/wiki/Clipboard> |
| Zellij | Layouts, plugins, an "Inbox & Attention" chapter for what needs you across sessions. The fetched page was only an index, so nothing more is claimed. | <https://zellij.dev/documentation/introduction.html> |
| WezTerm mux | Domains (unix, ssh, TLS). TLS domains reconnect automatically with a saved certificate; the page does not say what is lost on disconnect for the others. SSH domains need a matching WezTerm on the remote. | <https://wezterm.org/multiplexing.html> |
| Wave Terminal | Terminal with widgets, SSH connections, a `wsh` CLI and an AI assistant that can read terminal output. | <https://docs.waveterm.dev/> |
| Warp | Groups a command and its output into a Block that can be copied, searched, shared. No SSH notes on the page read. | <https://docs.warp.dev/terminal/blocks> |
| tssh | OpenSSH-compatible client with ProxyJump, ProxyCommand, ControlMaster/ControlPath/ControlPersist and trz/tsz transfer. | <https://github.com/trzsz/trzsz-ssh> |
| Teleport | Records the PTY output of an SSH session as protobuf events, gzip-compressed, optionally encrypted; recording at the node (default) or proxy; sync mode kills a session that cannot be recorded. The page describes no hash or signature for tamper evidence and warns users can hide commands (encoded, scripts), pointing to BPF recording. | <https://goteleport.com/docs/reference/architecture/session-recording/> |
| Tabby | The fetched page had no feature text, so nothing is claimed. | <https://tabby.sh/> |
| asciicast v2 | Newline-delimited JSON: a header (`version`, `width`, `height`, optional `timestamp`, `idle_time_limit`, `title`, `env`) then `[time, code, data]` events: `o` output, `i` input, `m` marker, `r` resize. The recorder captures only `SHELL` and `TERM` and does not record input unless asked; other tools "shouldn't" either. | <https://docs.asciinema.org/manual/asciicast/v2/> |
| OpenSSH client config | `ProxyJump` takes `[user@]host[:port]` or an ssh URI. `ForwardAgent`: "should be enabled with caution"; users who can bypass file permissions on the remote can reach the agent. `Include` paths without a leading `/` are taken from `~/.ssh` in a user config. `Match exec` runs a command under the user's shell and "should not be used in the global /etc/ssh/ssh_config to control security-sensitive configuration". `ControlPath` for opportunistic sharing should include at least `%h`, `%p`, `%r`. | <https://man.openbsd.org/ssh_config> |
| OSC 8 hyperlinks | `OSC 8 ; params ; URI ST`; only `id` is defined; terminals decide which schemes to open; URIs are limited to bytes 32-126 and about 2000 bytes; mitigations suggested: Ctrl+click, show the URI, confirm, let admins disable. | <https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda> |
| MCP tools spec | Servers MUST validate inputs, enforce access control, rate limit and "sanitize tool outputs"; tool annotations are untrusted unless the server is trusted; a human SHOULD be able to deny invocations. | <https://modelcontextprotocol.io/specification/2025-06-18/server/tools> |
| Escape-injection advisories | A steady stream of CWE-150 reports against tools that print attacker-controlled text: OSC 0 / OSC 52 passing through prompts and `onefetch`/`broot`-style tools, plus iTerm2 CVE-2024-38396 (title report) and Ghostty CVE-2024-56803. Read from search results only, so treat the specifics as **[U]**; the class of bug is well established. Mitigation advice: disable clipboard writes and title reports, strip control characters from untrusted output. | search: "terminal escape sequence injection OSC 52", see also <https://dgl.cx/2024/12/ghostty-terminal-title> **[U]** |

## What TERYON already has (so these are not "ideas")

Persistent PTYs in a per-user daemon over plain OpenSSH, asciicast v2 `record` and a filtering `play`, output search
(`search`, `scrollsearch`), a paste guard for zsh, regex secret redaction, MCP with read-only default and
`--allow-write`, `doctor`, `hosts --check`, host health graphs, a sandbox for agents, read-only shares.

## What it lacked

1. **Untrusted terminal text reached other sinks unfiltered.** The MCP server redacted secrets but forwarded
   escape sequences, bidi controls and Unicode tag characters (the "ASCII smuggling" channel) to the model. The
   OSC 7 working directory was only checked for NUL. `play` let OSC 0/2/8 payloads carry control characters.
2. **Recordings could not be made shareable.** `record` wrote screen bytes as-is: no redaction, no sequence filter.
3. **No way to look at an SSH config before trusting it.** TERYON delegates to OpenSSH (a stated strength) but never
   told you what a `ProxyJump` chain really is, or that a config weakens host-key checking, forwards the agent
   through a jump host, or names a host like `-oProxyCommand=...`.
4. Not built (ranked below): time-stamped markers, `i` input events, link latency badge, scrollback index.

## Ranked ideas

| # | Idea | Why | Verdict |
|---|---|---|---|
| 1 | Output sanitiser for every consumer of pane text (MCP, `capture --plain`, listings) with counts, caps and tests | The MCP spec says servers MUST sanitise outputs; the advisory class above is exactly "attacker text reaches a terminal or an agent". Cheap, no new trust. | **Built** (`src/sanitize.rs`) |
| 2 | `record --redact`: drop non-style sequences (OSC 52, titles, queries) and redact secrets before the file is written | asciicast files get shared; Teleport shows recordings are audit artefacts; asciinema says to be careful with input and env. | **Built** (`record.rs`). Input (`i`) events are deliberately **not** recorded. |
| 3 | `ssh-audit`: parse the user's ssh config, resolve ProxyJump chains, flag loops, injection-shaped names, `ForwardAgent`, `StrictHostKeyChecking no`, `UserKnownHostsFile /dev/null`, `LocalCommand`, open forwards, unsafe `ControlPath`, `Match` blocks | The man page itself warns about ForwardAgent and Match exec; TERYON's whole model is "your ssh config", so it should be inspectable. Text-only: no connection, no key read. | **Built** (`src/sshaudit.rs`) |
| 4 | Fix OSC 7 cwd and `play` title payload hardening | Found while doing 1. | **Built** (bug fixes) |
| 5 | Link latency / health badge | mosh and WezTerm both show predicted echo / latency. TERYON already shows link latency and `hosts --check`. | Rejected: exists |
| 6 | Predictive local echo or UDP transport | mosh's core. A different product and a large attack surface; `docs/LIMITATIONS.md` already says it is not planned. | Rejected |
| 7 | Tamper-evident recordings (hash chain / signatures) | Teleport's page names no mechanism, so there is no reference design; honest signing needs key management this product avoids. | Deferred; roadmap |
| 8 | Block-based output (Warp), command-level search | Needs shell integration to be mandatory; OSC 133 marks exist already. | Deferred |
| 9 | OSC 8 scheme allowlist in the desktop app | The spec leaves schemes to the terminal. The app is not buildable on this host, so it cannot be tested here. | Deferred |
| 10 | Marker (`m`) events and `idle_time_limit` in casts | Trivial and spec-conformant. | Deferred (small) |

## Boundaries observed

* SSH: nothing here opens a connection, edits `~/.ssh`, reads key files or changes `StrictHostKeyChecking`/`BatchMode`.
  `ssh-audit` reads only the config text and reports each file it read.
* MCP: no new write tool; the new behaviour is on the output side of existing tools. The read-only/`--allow-write`
  split is unchanged.
* Honesty: redaction and sanitising are described as best effort. Nothing claims audit-grade recording.

## Third round: recordings and waiting on panes

Same rules: URLs below were fetched during this session unless marked **[U]** (seen only in search results).

| Source | What it says | Taken |
|---|---|---|
| <https://docs.asciinema.org/manual/asciicast/v3/> | v3 stores intervals, groups size under `term`, allows `#` comments, adds `x` exit events, asks tools to ignore unknown codes and to carry rounding error forward | read, write and convert v3 |
| <https://blog.asciinema.org/post/three-point-o/> | asciinema 3.0 introduced v3 | v3 matters: `play` refused it |
| <https://docs.rs/crate/asciinema/3.2.0/source/src/cli.rs> | `play --pause-on-markers` and `--idle-time-limit`; `cat` joins files; `convert` changes formats | `play --pause-on-markers`, `idle_time_limit`, `cast cat`, `cast convert`, `cast mark` |
| <https://zellij.dev/news/remote-sessions-windows-cli/> | Zellij 0.44 adds `subscribe` (stream a pane), `dump-screen`, and `run --blocking` | `capture --follow`, `wait --exit` |
| <https://github.com/tomfordweb/sidemux> | `wait` ends on exit, a regex or an idle pane; `since: "now"` avoids stale matches | `wait --for/--idle/--exit/--new`, MCP `idle_s` and `since` |
| <https://docs.atuin.sh/latest/reference/search/> | history search with `--before`, `--after`, `--format` | not built: `hist timeline --json` covers scripting |
| <https://raw.githubusercontent.com/tmux/tmux/3.6/CHANGES> (first part only) | 3.6 adds pane scrollbars, format changes | nothing taken |
| tmux MCP servers (mcp-tmux, libtmux-mcp) **[U]** | separate text and idle waits; a typed command can match its own echo | `--new` / `since` ignore old text; the echo of a command typed later can still match |

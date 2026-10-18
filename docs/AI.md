# The AI assistant

TERYON can chat with Claude and, with your approval, act for you: open or quit an app, open an SSH terminal,
answer questions about this Mac. It needs an Anthropic API key and the system `curl`; nothing else is installed.

## Set up

```bash
export ANTHROPIC_API_KEY=sk-ant-…      # or:  teryon ai --set-key   (stores it in the macOS Keychain)
teryon ai --check                      # one tiny request: key accepted, model, latency
```

The key is read from `ANTHROPIC_API_KEY`, then from the Keychain (service `teryon-anthropic`). It is never written to
`config.toml`, logs or the chat, and it is sent to `curl` on stdin, so it does not show up in `ps`.

## Use it

* Desktop: **⌘J** (or the ✦ button, or the palette's "Ask AI…") opens the panel on the right. Enter sends, ⇧Enter adds
  a line, Esc closes. "New chat" starts over.
* Terminal: `teryon ai "which apps are running?"` answers once; `teryon ai` alone is an interactive chat.

Try: *"open Calculator"*, *"quit Safari"*, *"how much battery is left?"*, *"open a terminal to my staging host"*,
*"what is using port 3000?"*, *"explain the last error in pane 1"*.

## What it may do, and when it asks

| Tool | Asks first? | What it does |
|---|---|---|
| `mac_info`, `list_apps`, `list_hosts`, `list_panes` | no | read-only facts (macOS version, battery, disk, running apps, SSH host names, panes) |
| `open_app`, `quit_app` | yes | `open -a NAME`; a graceful AppleScript quit (never `kill -9`) |
| `open_url` | yes | http(s) only |
| `open_ssh_pane` | yes | a new TERYON tab on that host (a Terminal window from the CLI) |
| `read_pane` | yes | reads a pane's visible text **and sends it to Anthropic** |
| `run_in_pane`, `close_pane` | yes | types into or closes one of your panes |
| `run_command` | yes | a shell command on this Mac: 30 s limit, output cut at 8 KB, no terminal |

The card shows the exact action. **Allow** runs it once, **Always this session** stops asking for that tool until you
start a new chat, **Deny** tells the model no. A few commands are refused outright without asking (`sudo`, recursive
deletes of `/`, `~` or system folders, formatting disks, piping downloads into a shell, anything touching `~/.ssh`,
shutdown/reboot). Names of apps, hosts and URLs are validated and passed as arguments, never pasted into a shell.
A reply can use at most 8 tool steps.

## What leaves your machine

Your messages, the tool results (for example the list of running apps) and a short context (host, workspace and the
focused pane's title and directory) go to the Anthropic API. Terminal text is sent only when you approve `read_pane`
or a command output. Conversations live in memory only and are gone when you quit.

## Config

```toml
[ai]
enabled = true
model = "claude-sonnet-5-5"
max_tokens = 2048
auto_allow = ["mac_info", "list_apps", "list_hosts", "list_panes"]   # read-only tools only
```

`teryon config --check` validates the section. There is no key option on purpose.

# TERYON as an MCP server (`teryon mcp`)

AI coding agents (Claude Code, Codex, Gemini CLI, …) are usually tied to the terminal they were started in: close it
and the job dies. `teryon mcp` gives them **persistent terminals**: panes run inside TERYON's daemon, on this Mac or on
any SSH host, and keep running when the agent, your laptop or the SSH link goes away. It speaks the Model Context
Protocol over stdio (newline-delimited JSON-RPC 2.0) and needs no network port.

## Connect an agent

```bash
teryon mcp --print-config --allow-write      # prints the lines below for your install path
claude mcp add teryon -- /path/to/teryon mcp --allow-write
```

Other MCP clients take the same command in their config (`{"mcpServers": {"teryon": {"command": "teryon", "args": ["mcp", "--allow-write"]}}}`).
Leave out `--allow-write` for a read-only server.

## Tools

| Tool | Needs `--allow-write` | What it does |
|---|---|---|
| `list_workspaces` | no | workspaces on a host, pane counts, whether a client controls them |
| `list_panes` | no | id, title, directory, foreground command (for example `claude`), exit status |
| `read_pane` | no | the last N lines (default 200, max 2000) of a pane's retained output |
| `search_scrollback` | no | regex over a pane's retained output, with absolute line numbers |
| `wait_for` | no | block until a regex appears (a server says "ready"), the pane exits, or a timeout (max 300 s) |
| `new_pane` | yes | start a command (argv list; `["sh","-c","…"]` for a shell line) in a new persistent pane |
| `send_keys` | yes | type text into a pane, optionally pressing Enter |
| `close_pane` | yes | terminate a pane |

`host` (default `--host`, usually `local`) may be `local`, a host from your TERYON config or an alias from
`~/.ssh/config`; nothing else is accepted, so an agent cannot make TERYON connect to arbitrary machines.

## Rules that keep it safe

* **Writes go only to the server's own workspace** (`--workspace`, default `mcp`). Reads can look at any workspace, so
  an agent can watch a build in your `default` workspace but cannot type into it.
* **One controller per workspace.** If another TERYON client (for example the desktop app) controls the workspace the
  server wants to write to, writes fail with a clear message while reads keep working.
* **Secrets are redacted** from everything returned (API keys, tokens, JWTs, private keys, `NAME=secret`), and the
  server tells the agent that terminal output is untrusted data. `--no-redact` turns the filter off.
* `--allow-write` is your decision that the agent may run commands as you. TERYON does not filter what an agent
  chooses to run in `new_pane`; use it with agents you already trust to run shell commands.

## Example

An agent starts the dev server in a pane, waits for it, and keeps working while you close the laptop:

```text
new_pane   {"command": ["sh", "-c", "npm run dev"]}       -> started pane 2 in workspace mcp on local
wait_for   {"pane": 2, "pattern": "ready in \\d+ms"}      -> MATCH: ready in 412ms
read_pane  {"pane": 2, "lines": 30}
```

Open the same workspace later with `teryon connect local --workspace mcp` (or the desktop app) to watch or take over.

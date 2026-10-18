# Launch copy

Ready to paste. Every claim is one the README, CHANGELOG or docs already make; nothing here has been posted.
Card: `docs/social/teryon-social-card.jpg` (1600x900). Alt text for the card:

> TERYON: "Close the laptop. Your shells stay." A desktop window with four terminal panes (a file tree, a git log,
> passing tests, and an AI coding agent asking for approval) beside the headline and feature chips.

## X: announcement and thread

Attach `teryon-social-card.jpg` to post 1, `docs/images/desktop-0.15-grid.png` to post 3 and
`docs/images/desktop-0.15-files.png` to post 7.

1.
> Introducing TERYON 🚀
>
> Close the laptop. Your shells stay.
>
> Persistent terminals on your Mac mini or Linux box, over plain OpenSSH. Native desktop app, agent-aware panes, a
> Finder-style file browser, an AI command bar and an MCP server.
>
> Rust. Open source. Apache-2.0.
>
> github.com/zyvorai/teryon 🧵

2.
> The idea: separate the life of your shell from the life of your connection. A small per-user service on your host owns
> the PTYs. Your laptop connects over the OpenSSH you already use. Sleep, switch networks, come back tomorrow: same live
> shells. No new ports, no new accounts.

3.
> Agents run for hours, so panes now know about them. Claude Code, Codex, Gemini CLI, aider, Cursor and OpenCode get a
> live status chip: working, needs you, idle, done. You get a notification when a background agent is waiting on you.
> ⌘⇧A lists them all.

4.
> Reattach and a "While you were away" card tells you what finished and what failed, built from shell marks, no AI needed.

5.
> Give your coding agent a terminal that survives it:
>
> `claude mcp add teryon -- teryon mcp --allow-write`
>
> Durable panes, local or over SSH. Reads are always on; writes are opt-in and only touch the server's own workspace.

6.
> AI in your prompt, without handing over control. ⌘⇧K: describe a command, get one command with a risk label. It's
> inserted at your prompt and never runs until you press Enter. Secrets in terminal text are scrubbed before they reach
> a model.

7.
> New in 0.15: a Finder-like file browser (⌘B), local or over SSH. Space is Quick Look, a double-click types a quoted
> path, and download, rename and move to Trash are one right-click away. Nothing is deleted outright.

8.
> Honest status: it's a 0.15 engineering preview. Input is acknowledged, not guaranteed executed, and a reboot ends live
> processes. The limits are written down in docs/LIMITATIONS.md.
>
> Try it, break it, tell me what's missing: github.com/zyvorai/teryon
> Site: zyvorai.github.io/teryon

For a single big post instead of a thread, use post 1 and put posts 2 and 8 in the first reply.

## LinkedIn

> I got tired of losing shells when a laptop sleeps or the Wi-Fi drops, so I built TERYON.
>
> A small per-user service on your Mac mini or Linux host owns the terminals. Your laptop connects over the OpenSSH you
> already use, so you can close the lid, switch networks and come back to the same live shells.
>
> 0.15 adds a Finder-style file browser with Quick Look, rounded pane cards, agent-aware panes (Claude Code, Codex,
> Gemini CLI, aider), an AI command bar that never runs anything without you pressing Enter, and an MCP server so coding
> agents can use durable terminals.
>
> It is an engineering preview, written in Rust, Apache-2.0. Feedback and issues welcome.
>
> https://github.com/zyvorai/teryon

## Hacker News

Title: `Show HN: TERYON – persistent remote shells over plain OpenSSH, with a native desktop app`

First comment:

> I built TERYON because tmux over SSH loses the thing I care about when the client side is flaky: a nice, local UI.
> A per-user daemon owns the PTYs on the host; the client (a ratatui TUI, or an egui desktop app) speaks a small
> protocol over `ssh`, so there is nothing to open on the network and no new accounts.
>
> What is different: agent-aware panes (status chips for Claude Code, Codex, aider and friends, plus a notification when
> one needs you), an MCP server so an agent can run a build in a pane that outlives it, and a file sidebar that follows your
> shell's directory locally or over SSH.
>
> What it is not: a production-hardened tmux replacement. Input is acknowledged, not guaranteed executed; a reboot ends
> live processes. The limits are written down in docs/LIMITATIONS.md. Rust, Apache-2.0, macOS and Linux.

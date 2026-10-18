# Launch copy

Ready to paste. Every claim is one the README, CHANGELOG or docs already make; nothing here has been posted.
Card: `docs/social/teryon-social-card.jpg` (1600x900). Alt text for the card:

> TERYON: "Close the laptop. Your shells stay." A desktop window with four terminal panes (a file tree, a git log,
> passing tests, and an AI coding agent asking for approval) beside the headline and feature chips.

## X

> Close the laptop. Your shells stay.
>
> TERYON keeps real PTYs alive on your Mac mini or Linux box and reattaches over plain OpenSSH. Native desktop app,
> an agent status chip for Claude Code/Codex/Gemini, an MCP server, and a Finder-style file browser. Rust, Apache-2.0.
>
> github.com/zyvorai/teryon

Thread follow-ups, one per post (attach a screenshot from `docs/images/`):
1. Agents: "A pane running Claude Code, Codex, Gemini CLI or aider gets a live chip: working, needs you, idle, done. Walk away; it tells you when one is waiting."
2. MCP: `claude mcp add teryon -- teryon mcp --allow-write` gives an agent a terminal that survives the agent and the SSH link.
3. Prompt: "Describe a command, get one command with a risk label. It is inserted at your prompt and never runs until you press Enter."
4. Honesty: "It is a 0.15 engineering preview. Known limits are in docs/LIMITATIONS.md."

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

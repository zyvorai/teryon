# Security model and reporting

TERYON 0.1 is an engineering preview. Review it before using it with sensitive production hosts.

## Trust boundaries

- OpenSSH provides network encryption, host verification and key authentication. TERYON requires `StrictHostKeyChecking=yes` and `BatchMode=yes`. It never auto-accepts a changed key or copies private keys to the remote host.
- Local IPC is restricted to the current account's private directory and authenticated by a random token. The fallback listener binds only to loopback.
- The daemon runs as your normal account. Anyone able to read that account's runtime token already has a path to terminal control. Read-only attachments are a session behavior, not account-level isolation or delegated authorization.
- A remote process's terminal output is parsed into cells; raw OSC clipboard instructions are not passed to the local terminal.
- Host destinations are validated. User-controlled remote executable paths are shell-quoted. Task argv is passed directly to the process launcher; shell interpretation occurs only when you explicitly invoke a shell.
- File transfers use rsync protected arguments; no automatic `--delete`, privilege escalation, or remote script downloads are used.
- Private exports and config creation use exclusive file creation with mode `0600`; existing files are not silently overwritten.

Do not run the daemon as root for everyday work. Use the same least-privilege account you normally use over SSH.

## Sensitive data

Retained terminal output may contain secrets, tokens, environment values or personal data. It resides in daemon memory and can be exported by explicit actions. TERYON does not implement reliable secret redaction, encrypted history on disk, audit-grade logging, or organization policies. Avoid exporting secrets and protect output files like other terminal logs.

## Resource and protocol bounds

Frames, input sizes, terminal dimensions, panes, workspaces, per-pane input queues, and scrollback are bounded. This does not make the daemon a hostile multi-tenant sandbox. It trusts the same user to launch programs and does not limit their CPU, disk, memory or process count. There are no cgroups, container sandbox, mandatory access controls or privilege separation inside the application.

## Persistence and destructive commands

Detach preserves sessions. `remove --force`, explicit pane termination and `stop --force` terminate work. A plain shutdown refuses live panes. Commands with uncertain delivery are not replayed after reconnect. An input acknowledgment is not proof that an application executed it.

## Reporting

If you publish this repository, enable GitHub private vulnerability reporting in repository settings and add a monitored security contact before accepting external reports. Do not file live credentials or exploitable sensitive details in a public issue. No unverified reporting email or security-response SLA is claimed here.

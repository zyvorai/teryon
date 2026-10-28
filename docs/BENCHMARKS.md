# Benchmarks

`scripts/bench.py` compares TERYON and tmux on one machine, driven the same way, with the standard
library only. It exists so a performance claim can be reproduced, not to rank tools.

```bash
cargo build --release
python3 scripts/bench.py --binary target/release/teryon --runs 30 --lines 200000 --json out.json
```

## What is measured

| Measurement | How |
|---|---|
| floor | One `capture` call on an idle pane. This is the cost of starting the CLI itself. |
| round trip | `send` a command, then poll `capture` until its output line appears. |
| throughput | Time from `send` of `seq 1 N; echo DONE` until `DONE` shows in `capture`. |
| memory | Resident size of the TERYON daemon / tmux server, idle and after the throughput run. |

Both tools use `/bin/sh`, an empty `$HOME`, private sockets and the same polling loop. Compare the
**round trip minus floor** row, not the absolute values: every number includes spawning the CLI.

## What is not measured

- The interactive client: desktop app rendering, TUI redraw, keystroke-to-glyph latency.
- A network. Nothing here uses SSH, so the screen-diff and quiet-poll work in 0.20 is not exercised.
- Mosh, Zellij or WezTerm. Add them as new classes in `bench.py` if you need them.
- Many panes, many workspaces, or long-running sessions.

So this does **not** show that TERYON is faster than tmux, Mosh or anything else over a real link.
A fair answer to that needs a network emulator (for example latency and loss via `pfctl`/`dnctl` on
macOS or `tc netem` on Linux) and the interactive client path.

## One recorded run

Single run, `--runs 30 --lines 200000`, TERYON 0.22.0 release build with the `desktop` feature,
tmux 3.8, Apple Silicon (10 cores) with a load average of about 3.5 at the start. Treat the
differences as indicative only; repeat on an idle machine before quoting them.

| | teryon | tmux |
|---|---|---|
| floor: idle `capture` (ms, median) | 6.1 | 4.3 |
| round trip (ms, median) | 11.1 | 13.0 |
| round trip (ms, p95) | 12.8 | 13.4 |
| round trip minus floor (ms, median) | 5.0 | 8.7 |
| `seq 1 200000` until last line visible (s) | 0.21 | 0.19 |
| daemon/server RSS idle (MiB) | 10.0 | 3.9 |
| daemon/server RSS after output (MiB) | 18.7 | 4.5 |

Reading it honestly: on this local path the two are close on latency and throughput, and TERYON's
daemon uses more memory (it keeps a parsed screen and scrollback per pane). Nothing here supports a
speed claim in either direction.

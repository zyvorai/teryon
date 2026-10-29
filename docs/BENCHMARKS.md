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

## Over an emulated slow link (`scripts/bench_net.py`)

This runs the real interactive clients, `teryon connect` and `ssh -tt host tmux new -A`, under a
pseudo-terminal, through a userspace TCP proxy that adds a fixed one-way delay plus up to 10 ms of
jitter. It starts a private `sshd` on a loopback port with its own keys and a private TERYON daemon
and tmux server, so it needs no sudo, changes no system setting and never touches your real sessions.

```bash
python3 scripts/bench_net.py --binary target/release/teryon --rtts 0,50,150,300 --samples 10 --outage
```

One run (10 samples per cell after 3 warm-ups, median / p95 in ms, load average about 2 on a
10-core Apple Silicon Mac, TERYON 0.22.0, tmux 3.8):

| Added RTT | teryon key echo | tmux key echo | teryon burst | tmux burst |
|---|---|---|---|---|
| 0 ms | 40 / 76 | 26 / 40 | 502 / 537 | 26 / 37 |
| 50 ms | 188 / 232 | 68 / 80 | 2492 / 2633 | 78 / 87 |
| 150 ms | 419 / 449 | 168 / 191 | 6364 / 6872 | 176 / 193 |
| 300 ms | 675 / 710 | 312 / 335 | 12272 / 13217 | 321 / 341 |

*Key echo* is one typed character until it is drawn. *Burst* is a 20-byte command written at once (a
paste or fast typing) until its output line is drawn.

What this shows:

- **TERYON handles keystrokes one round trip at a time.** A single key costs about 2.2 to 3.8 times
  the RTT, against about 1.1 times for tmux. A burst is that cost multiplied by its length, so a
  20-byte paste takes about 12 s at 300 ms RTT, while tmux takes the RTT. This matches the
  "request/response polling still adds latency" limit in `LIMITATIONS.md`, now with numbers.
- **On a link with no delay TERYON is fine.** 40 ms per key is not noticeable.
- **Outage recovery shows no difference.** After every connection is reset, both tools were
  answering again within the harness's 0.6 s retry window (0.65 s in all 3 trials each). The
  harness cannot resolve finer than that, and it restarts the tmux client instantly, which a
  person cannot do.
- **No claim about packet loss, Mosh or a real network.** A TCP proxy cannot lose packets, and UDP is
  not covered. Both daemons share this machine's CPU. Treat the figures as one machine, one run.

The method took three corrections before the numbers held, which is why the script's checks matter:
a "wait until quiet" helper that never returns for a client that repaints every 43 ms, a text
matcher that missed output interleaved with the top bar (it now keeps a small screen model), and
stale output from an earlier link setting matching new probes (every marker is now unique).

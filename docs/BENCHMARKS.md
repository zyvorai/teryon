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

The same harness was run three times on the same machine, once for each change to the client, so the
effect of each is visible. Median ms, 10 samples per cell after 3 warm-ups, TERYON 0.22.0 and tmux 3.8 on a
10-core Apple Silicon Mac. The load average was 2.0 to 3.9 across the runs, so a difference of a few
percent is within noise. The tmux column is from the last run; it varied by a few ms between runs.

*Key echo* is one typed character until it is drawn. *Paste* is a 20-byte command written at once (a
paste or fast typing) until its output line is drawn.

**Key echo**

| Added RTT | tmux | teryon, first measured (#54) | + keystroke merging (#55) | + held poll (#56) |
|---|---|---|---|---|
| 0 ms | 26 | 40 | 39 | 37 |
| 50 ms | 73 | 188 | 187 | 77 |
| 150 ms | 171 | 419 | 410 | 227 |
| 300 ms | 314 | 675 | 681 | 370 |

**Paste**

| Added RTT | tmux | teryon, first measured (#54) | + keystroke merging (#55) | + held poll (#56) |
|---|---|---|---|---|
| 0 ms | 28 | 502 | 80 | 81 |
| 50 ms | 75 | 2492 | 195 | 217 |
| 150 ms | 177 | 6364 | 427 | 517 |
| 300 ms | 315 | 12272 | 877 | 959 |

What this shows:

- **Key echo is now close to tmux.** The client used to send a key and then poll for it in a second round
  trip, about 2.3 times the RTT. The held poll (#56) brings that to about 1.2 times at 300 ms, against
  about 1.05 for tmux. At 50 ms it is level with tmux.
- **Pasting is no longer the problem it was.** Merging queued keystrokes (#55) made a 20-byte paste at
  300 ms about 14 times faster (12.3 s to 0.9 s). It is still about three round trips against tmux's one.
- **The held poll has a cost.** Pasting got 9 to 21 percent slower at 150 ms and above, probably because
  keystrokes sent while a poll is being held wait for the hold to end. That may be partly noise (the
  machine was busier in that run); `ECHO_HOLD_MS` in `src/ui.rs` trades echo speed against it.
- **On a link with no delay TERYON is fine.** 37 ms per key is not noticeable.
- **Outage recovery shows no difference.** After every connection is reset, both tools were answering
  again within the harness's 0.6 s retry window (0.65 to 0.67 s in every trial, in all three runs). The harness
  cannot resolve finer than that, and it restarts the tmux client instantly, which a person cannot do.
- **No claim about packet loss, Mosh or a real network.** A TCP proxy cannot lose packets, and UDP is
  not covered. Both daemons share this machine's CPU. Treat the figures as one machine, three runs.

The method took three corrections before the numbers held, which is why the script's checks matter:
a "wait until quiet" helper that never returns for a client that repaints every 43 ms, a text
matcher that missed output interleaved with the top bar (it now keeps a small screen model), and
stale output from an earlier link setting matching new probes (every marker is now unique).

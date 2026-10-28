#!/usr/bin/env python3
"""Reproducible local benchmark: TERYON vs tmux, driven the same way.

What it measures (same method for both tools, standard library only):
  floor        one `capture` call on an idle pane: the cost of spawning the CLI itself
  round trip   `send` a command, then poll `capture` until its output line appears
  throughput   time to print N lines and see the last one in `capture`
  memory       resident size of the daemon / server, idle and after the throughput run

What it does NOT measure: the interactive client path (desktop app, TUI redraw), a real network, SSH
or Mosh. Every number includes the process spawn of the CLI, so compare tools by the difference from
the floor, not by the absolute value. Run it on an otherwise idle machine; see docs/BENCHMARKS.md.

Everything runs in a private temp directory (own HOME, own sockets, same /bin/sh in both tools) and
never touches your real sessions. Processes are stopped by pid or by their own stop command.

  python3 scripts/bench.py --binary target/release/teryon
  python3 scripts/bench.py --binary target/release/teryon --runs 50 --lines 200000 --json out.json
"""
import argparse
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def run(cmd, env, check=True, timeout=60):
    return subprocess.run(
        cmd, env=env, capture_output=True, text=True, check=check, timeout=timeout
    )


def rss_kib(pid):
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True)
    return int(out.stdout.strip() or 0)


def pct(values, p):
    values = sorted(values)
    return values[min(len(values) - 1, int(round(p / 100 * (len(values) - 1))))]


class Teryon:
    name = "teryon"

    def __init__(self, binary, root):
        self.bin = str(Path(binary).resolve())
        self.env = clean_env(root)
        self.env["TERYON_RUNTIME_DIR"] = str(root / "rt")
        (root / "rt").mkdir(mode=0o700)
        self.daemon: subprocess.Popen | None = None

    def start(self):
        self.daemon = subprocess.Popen(
            [self.bin, "daemon"], env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
        self.send("true")  # creates the workspace and pane 1, and waits for the daemon socket
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if self.capture().strip():
                return
            time.sleep(0.05)
        raise RuntimeError("teryon pane never produced a prompt")

    def send(self, text):
        run([self.bin, "send", "local", "-w", "bench", "--pane", "1", text, "--enter"], self.env)

    def capture(self):
        return run([self.bin, "capture", "local", "-w", "bench", "--pane", "1"], self.env).stdout

    def pid(self):
        assert self.daemon is not None
        return self.daemon.pid

    def stop(self):
        if self.daemon is None:
            return
        run([self.bin, "stop", "--force", "local"], self.env, check=False)
        try:
            self.daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.daemon.terminate()  # our own child, known pid


class Tmux:
    name = "tmux"

    def __init__(self, root):
        self.env = clean_env(root)
        self.env["TMUX_TMPDIR"] = str(root)
        self.sock = "bench"

    def _tmux(self, *args, check=True):
        return run(["tmux", "-L", self.sock, "-f", "/dev/null", *args], self.env, check=check)

    def start(self):
        self._tmux("new-session", "-d", "-s", "b", "-x", "100", "-y", "30", "/bin/sh")
        self.send("true")
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if self.capture().strip():
                return
            time.sleep(0.05)
        raise RuntimeError("tmux pane never produced a prompt")

    def send(self, text):
        self._tmux("send-keys", "-t", "b", "-l", text)
        self._tmux("send-keys", "-t", "b", "Enter")

    def capture(self):
        return self._tmux("capture-pane", "-p", "-t", "b").stdout

    def pid(self):
        return int(self._tmux("display-message", "-p", "-t", "b", "#{pid}").stdout.strip())

    def stop(self):
        self._tmux("kill-server", check=False)


def clean_env(root):
    home = root / "home"
    home.mkdir(exist_ok=True)
    return {
        "HOME": str(home),
        "PATH": os.environ["PATH"],
        "SHELL": "/bin/sh",
        "TERM": "xterm-256color",
        "PS1": "$ ",
        "LANG": "en_US.UTF-8",
    }


def wait_for(tool, pattern, timeout=60):
    """Seconds from now until `pattern` matches a whole line of the pane; poll as fast as possible."""
    rx = re.compile(pattern, re.M)
    start = time.perf_counter()
    while True:
        if rx.search(tool.capture()):
            return time.perf_counter() - start
        if time.perf_counter() - start > timeout:
            raise TimeoutError(f"{tool.name}: {pattern!r} never appeared")


def measure(tool, runs, lines):
    tool.start()
    pid = tool.pid()
    result: dict = {"rss_idle_mib": rss_kib(pid) / 1024}

    floor = []
    for _ in range(runs):
        t = time.perf_counter()
        tool.capture()
        floor.append((time.perf_counter() - t) * 1000)
    result["floor_ms"] = floor

    trips = []
    for i in range(runs + 3):
        # `MA""RK` is typed but prints MARK, so only the command's output matches the pattern.
        t = time.perf_counter()
        tool.send(f'echo MA""RK{i}')
        wait_for(tool, rf"^MARK{i}$")
        if i >= 3:  # three warm-up runs
            trips.append((time.perf_counter() - t) * 1000)
    result["round_trip_ms"] = trips

    t = time.perf_counter()
    tool.send(f'seq 1 {lines}; echo DO""NE')
    wait_for(tool, r"^DONE$", timeout=300)
    result["throughput_s"] = time.perf_counter() - t
    result["rss_after_mib"] = rss_kib(pid) / 1024
    return result


def summarize(samples):
    return {
        "median": statistics.median(samples),
        "p95": pct(samples, 95),
        "min": min(samples),
        "max": max(samples),
    }


def table(results, lines):
    out = ["| | " + " | ".join(results) + " |", "|---|" + "---|" * len(results)]

    def row(label, fn):
        out.append(f"| {label} | " + " | ".join(fn(r) for r in results.values()) + " |")

    row("floor: idle `capture` (ms, median)", lambda r: f"{summarize(r['floor_ms'])['median']:.1f}")
    row("round trip (ms, median)", lambda r: f"{summarize(r['round_trip_ms'])['median']:.1f}")
    row("round trip (ms, p95)", lambda r: f"{summarize(r['round_trip_ms'])['p95']:.1f}")
    row(
        "round trip minus floor (ms, median)",
        lambda r: f"{summarize(r['round_trip_ms'])['median'] - summarize(r['floor_ms'])['median']:.1f}",
    )
    row(f"seq 1 {lines} until last line visible (s)", lambda r: f"{r['throughput_s']:.2f}")
    row("daemon/server RSS idle (MiB)", lambda r: f"{r['rss_idle_mib']:.1f}")
    row("daemon/server RSS after output (MiB)", lambda r: f"{r['rss_after_mib']:.1f}")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--binary", default="target/release/teryon")
    ap.add_argument("--runs", type=int, default=30, help="round-trip and floor samples")
    ap.add_argument("--lines", type=int, default=200000, help="lines printed for the throughput run")
    ap.add_argument("--json", help="also write raw samples to this file")
    ap.add_argument("--no-tmux", action="store_true")
    args = ap.parse_args()

    load1 = os.getloadavg()[0]
    cores = os.cpu_count() or 1
    if load1 > cores / 2:
        print(f"warning: load average {load1:.1f} on {cores} cores; results will be noisy")

    tools = []
    # Short path: unix socket paths are limited to about 100 bytes.
    base = Path(tempfile.mkdtemp(prefix="tb", dir=Path.home() / ".tb"
                                 if (Path.home() / ".tb").is_dir() else None))
    try:
        results = {}
        t_root = base / "t"
        t_root.mkdir()
        tools.append(Teryon(args.binary, t_root))
        if not args.no_tmux and shutil.which("tmux"):
            m_root = base / "m"
            m_root.mkdir()
            tools.append(Tmux(m_root))
        for tool in tools:
            print(f"measuring {tool.name} ...", flush=True)
            try:
                results[tool.name] = measure(tool, args.runs, args.lines)
            finally:
                tool.stop()
    finally:
        for tool in tools:
            tool.stop()
        shutil.rmtree(base, ignore_errors=True)

    env = {
        "machine": platform.machine(),
        "os": f"{platform.system()} {platform.release()}",
        "cores": cores,
        "load1_at_start": round(load1, 2),
        "tmux": subprocess.run(["tmux", "-V"], capture_output=True, text=True).stdout.strip()
        if shutil.which("tmux")
        else None,
        "teryon": run([str(Path(args.binary).resolve()), "--version"], None).stdout.strip(),
    }
    print()
    print(table(results, args.lines))
    print()
    print(f"environment: {json.dumps(env)}")
    if args.json:
        Path(args.json).write_text(json.dumps({"env": env, "results": results}, indent=2))


if __name__ == "__main__":
    main()

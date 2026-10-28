#!/usr/bin/env python3
"""TERYON vs tmux-over-SSH through an emulated slow link, using the real interactive clients.

Everything runs as you, in a private temp directory, with no system changes and no sudo:
  * a private OpenSSH `sshd` on a high loopback port (own host key, own client key)
  * a userspace TCP proxy in front of it that adds one-way delay and jitter, and can kill every
    connection (an outage) on demand
  * each client runs under a pseudo-terminal: `teryon connect` and `ssh -tt host tmux new -A`
    with the same ssh options, the same proxy and the same /bin/sh in the pane

Measured per link setting (median and p95 over --samples):
  key echo     type one character, wait until it is drawn
  burst        write `echo MA""RK<n>` + Enter in one go (a paste or fast typing), wait until the
               output line MARK<n> is drawn
Measured once per tool (--outage, 3 trials):
  recovery     kill every TCP connection, then retry a short command every 0.6 s until the shell
               answers it. The result is quantized to that 0.6 s window: it can show "under about
               0.65 s", not a precise figure. A bare echoed character is not used as the probe,
               because once tmux's ssh client dies the local tty echoes input itself. tmux's client
               is restarted by this script at once, which is faster than a person, so the comparison
               favours tmux.

Limits, stated plainly: a TCP proxy delays and reorders nothing, so it cannot model packet loss or
UDP (no Mosh here). For real loss use dnctl/pfctl (macOS) or `tc netem` (Linux) as root and point
this script at it with --no-proxy. Both daemons run on this machine, so CPU and scheduling noise is
shared. Read docs/BENCHMARKS.md before quoting a number.

  python3 scripts/bench_net.py --binary target/release/teryon --rtts 0,50,150,300 --samples 20 --outage
"""
import argparse
import asyncio
import fcntl
import json
import os
import pty
import random
import re
import select
import signal
import shutil
import statistics
import struct
import subprocess
import tempfile
import termios
import threading
import time
from pathlib import Path

ANSI = re.compile(rb"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][A-Za-z0-9]")
ECHO_CHARS = b"QXJZ"  # uppercase letters appear in neither client's status line


def pct(values, p):
    values = sorted(values)
    return values[min(len(values) - 1, int(round(p / 100 * (len(values) - 1))))]


# --- the emulated link --------------------------------------------------------------------------


class DelayProxy(threading.Thread):
    """Forward TCP to 127.0.0.1:upstream with a fixed one-way delay plus jitter, keeping order."""

    def __init__(self, upstream, one_way_ms=0.0, jitter_ms=0.0):
        super().__init__(daemon=True)
        self.upstream = upstream
        self.one_way = one_way_ms / 1000
        self.jitter = jitter_ms / 1000
        self.port = 0
        self.loop = asyncio.new_event_loop()
        self.ready = threading.Event()
        self.pairs = []

    def run(self):
        asyncio.set_event_loop(self.loop)
        server = self.loop.run_until_complete(asyncio.start_server(self._handle, "127.0.0.1", 0))
        self.port = server.sockets[0].getsockname()[1]
        self.ready.set()
        self.loop.run_forever()

    def start_and_wait(self):
        self.start()
        self.ready.wait(5)
        return self

    async def _handle(self, reader, writer):
        try:
            up_r, up_w = await asyncio.open_connection("127.0.0.1", self.upstream)
        except OSError:
            writer.close()
            return
        self.pairs.append((writer, up_w))
        await asyncio.gather(self._pump(reader, up_w), self._pump(up_r, writer), return_exceptions=True)
        for w in (writer, up_w):
            w.close()

    async def _pump(self, src, dst):
        queue = asyncio.Queue()
        last = [0.0]

        async def reader_side():
            while True:
                data = await src.read(65536)
                now = time.monotonic()
                at = max(last[0], now + self.one_way + random.uniform(0, self.jitter))
                last[0] = at
                await queue.put((at, data))
                if not data:
                    return

        task = asyncio.ensure_future(reader_side())
        try:
            while True:
                at, data = await queue.get()
                wait = at - time.monotonic()
                if wait > 0:
                    await asyncio.sleep(wait)
                if not data:
                    return
                dst.write(data)
                await dst.drain()
        finally:
            task.cancel()

    def set_delay(self, one_way_ms, jitter_ms):
        self.one_way, self.jitter = one_way_ms / 1000, jitter_ms / 1000

    def kill_all(self):
        """Reset every connection at once, like a dropped Wi-Fi network."""

        def _kill():
            for pair in self.pairs:
                for w in pair:
                    w.transport.abort()
            self.pairs.clear()

        self.loop.call_soon_threadsafe(_kill)


# --- the private ssh server ---------------------------------------------------------------------


class Fixture:
    def __init__(self, root: Path, binary: str):
        self.root = root
        self.binary = str(Path(binary).resolve())
        self.rt = root / "rt"
        self.home = root / "home"
        for d in (self.rt, self.home / ".ssh"):
            d.mkdir(parents=True)
        self.rt.chmod(0o700)
        self.port = 2000 + random.randint(300, 6000)
        self.sshd = None
        self.daemon = None
        self.proxy: DelayProxy | None = None
        self.tmux = None

    def _keygen(self, name):
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(self.root / name)], check=True)

    def start(self, no_proxy=False):
        self._keygen("hostkey")
        self._keygen("client")
        shutil.copy(self.root / "client.pub", self.root / "authorized_keys")
        (self.root / "authorized_keys").chmod(0o600)
        tmux = shutil.which("tmux")
        cfg = f"""Port {self.port}
ListenAddress 127.0.0.1
HostKey {self.root}/hostkey
PidFile {self.root}/sshd.pid
AuthorizedKeysFile {self.root}/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
LogLevel ERROR
SetEnv TERYON_RUNTIME_DIR={self.rt} TMUX_TMPDIR={self.rt}
"""
        (self.root / "sshd_config").write_text(cfg)
        self.sshd = subprocess.Popen(
            ["/usr/sbin/sshd", "-D", "-f", str(self.root / "sshd_config")],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self._wait_port(self.port)
        # Own daemon, started by us, so we know its pid, its shell and its runtime directory.
        env = {"PATH": os.environ["PATH"], "HOME": str(self.home), "SHELL": "/bin/sh",
               "TERYON_RUNTIME_DIR": str(self.rt), "PS1": "$ "}
        self.daemon = subprocess.Popen([self.binary, "daemon"], env=env,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.proxy = DelayProxy(self.port).start_and_wait()
        shown = self.port if no_proxy else self.proxy.port
        pub = (self.root / "hostkey.pub").read_text().split()[:2]
        (self.home / ".ssh" / "known_hosts").write_text(f"[127.0.0.1]:{shown} {pub[0]} {pub[1]}\n")
        (self.home / ".ssh" / "config").write_text(
            f"Host nb\n  HostName 127.0.0.1\n  Port {shown}\n  User {os.environ['USER']}\n"
            f"  IdentityFile {self.root}/client\n  IdentitiesOnly yes\n"
            f"  UserKnownHostsFile {self.home}/.ssh/known_hosts\n  StrictHostKeyChecking yes\n"
        )
        self.tmux = tmux

    @staticmethod
    def _wait_port(port):
        import socket

        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            with socket.socket() as s:
                if s.connect_ex(("127.0.0.1", port)) == 0:
                    return
            time.sleep(0.1)
        raise RuntimeError("sshd did not start")

    @property
    def ssh_config(self):
        return str(self.home / ".ssh" / "config")

    def client_env(self):
        return {"PATH": os.environ["PATH"], "HOME": str(self.home), "TERM": "xterm-256color",
                "TERYON_SSH_CONFIG": self.ssh_config, "LANG": "en_US.UTF-8", "USER": os.environ["USER"]}

    def stop(self):
        env = {"PATH": os.environ["PATH"], "HOME": str(self.home), "TERYON_RUNTIME_DIR": str(self.rt),
               "TMUX_TMPDIR": str(self.rt)}
        if self.tmux:
            subprocess.run([self.tmux, "-L", "bench", "kill-server"], env=env, capture_output=True)
        if self.daemon:
            subprocess.run([self.binary, "stop", "--force", "local"], env=env, capture_output=True)
            try:
                self.daemon.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.daemon.terminate()
        if self.sshd:
            self.sshd.terminate()  # our own child
            try:
                self.sshd.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.sshd.kill()


# --- the interactive clients --------------------------------------------------------------------

SSH_OPTS = ["-F", "{cfg}", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=3"]


def client_argv(tool, fx):
    cfg = fx.ssh_config
    if tool == "teryon":
        return [fx.binary, "connect", "nb", "-w", "bench", "--remote-bin", fx.binary, "--no-splash"]
    opts = [o.format(cfg=cfg) for o in SSH_OPTS]
    return ["ssh", "-tt", *opts, "nb", fx.tmux, "-L", "bench", "-f", "/dev/null",
            "new-session", "-A", "-s", "b", "/bin/sh"]



class Screen:
    """A minimal terminal screen: absolute cursor moves, printable text, CR/LF, line and screen erase.
    TERYON paints with absolute moves and updates its top bar between the letters of a word, so
    stripping escape codes from the byte stream cannot find text that is plainly visible."""

    TOKEN = re.compile(rb"\x1b\[([0-9;?]*)[ -/]*([@-~])|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][A-Za-z0-9]|\x1b.|([^\x1b]+)", re.S)

    def __init__(self, rows=30, cols=100):
        import codecs

        self.rows, self.cols = rows, cols
        self.grid = [[" "] * cols for _ in range(rows)]
        self.r = self.c = 0
        self.dec = codecs.getincrementaldecoder("utf-8")("ignore")

    def feed(self, data: bytes):
        for m in self.TOKEN.finditer(data):
            if m.group(3) is not None:
                self._text(self.dec.decode(m.group(3)))
            elif m.group(2) is not None:
                self._csi(m.group(1).decode(), m.group(2).decode())

    def _text(self, text):
        for ch in text:
            if ch == "\r":
                self.c = 0
            elif ch == "\n":
                self.r = min(self.r + 1, self.rows - 1)
            elif ch == "\b":
                self.c = max(0, self.c - 1)
            elif ch >= " ":
                if self.c >= self.cols:
                    self.c, self.r = 0, min(self.r + 1, self.rows - 1)
                self.grid[self.r][self.c] = ch
                self.c += 1

    def _csi(self, params, final):
        if params.startswith("?"):
            return
        nums = [int(x) if x else 0 for x in params.split(";")] if params else []
        n = nums[0] if nums and nums[0] else 1
        if final in "Hf":
            self.r = min(max((nums[0] if nums and nums[0] else 1) - 1, 0), self.rows - 1)
            self.c = min(max((nums[1] if len(nums) > 1 and nums[1] else 1) - 1, 0), self.cols - 1)
        elif final == "A":
            self.r = max(0, self.r - n)
        elif final == "B":
            self.r = min(self.rows - 1, self.r + n)
        elif final == "C":
            self.c = min(self.cols - 1, self.c + n)
        elif final == "D":
            self.c = max(0, self.c - n)
        elif final == "G":
            self.c = min(max(n - 1, 0), self.cols - 1)
        elif final == "K":
            mode = nums[0] if nums else 0
            lo, hi = {0: (self.c, self.cols), 1: (0, self.c + 1)}.get(mode, (0, self.cols))
            for x in range(lo, hi):
                self.grid[self.r][x] = " "
        elif final == "J" and (nums[0] if nums else 0) in (2, 3):
            self.grid = [[" "] * self.cols for _ in range(self.rows)]

    def lines(self):
        return ["".join(row) for row in self.grid]

    def contains(self, text):
        return any(text in line for line in self.lines())


class Client:
    """A program running under a pseudo-terminal, with timed reads."""

    def __init__(self, argv, env):
        pid, fd = pty.fork()
        if pid == 0:
            try:
                os.execvpe(argv[0], argv, env)
            finally:
                os._exit(127)
        self.pid, self.fd = pid, fd
        self.last_seen = b""
        self.screen = Screen()
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))

    def write(self, data: bytes):
        os.write(self.fd, data)

    def read_some(self, timeout):
        r, _, _ = select.select([self.fd], [], [], max(0, timeout))
        if not r:
            return b""
        try:
            data = os.read(self.fd, 65536)
        except OSError:
            return None  # the child closed the terminal
        self.screen.feed(data)
        return data

    def settle(self, seconds):
        """Read and discard output for a fixed time. TERYON's client repaints about every 43 ms even
        when idle, so "wait for quiet" never returns; a fixed settle time is the same for both tools."""
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if self.read_some(min(0.05, end - time.monotonic())) is None:
                return False
        return True

    def wait_for(self, needle: bytes, timeout, on_screen=False):
        """Seconds until `needle` shows in the output (escape sequences removed), or None."""
        start = time.perf_counter()
        buf = b""
        while time.perf_counter() - start < timeout:
            chunk = self.read_some(0.02)
            if chunk is None:
                return None
            buf = (buf + chunk)[-65536:]
            if needle in ANSI.sub(b"", buf) or (on_screen and self.screen.contains(needle.decode())):
                return time.perf_counter() - start
        self.last_seen = ANSI.sub(b"", buf)[-300:]
        return None

    def alive(self):
        try:
            done, _ = os.waitpid(self.pid, os.WNOHANG)
        except ChildProcessError:
            return False
        return done == 0

    def close(self):
        try:
            os.kill(self.pid, 15)  # our own child
            time.sleep(0.2)
            os.waitpid(self.pid, os.WNOHANG)
        except (ProcessLookupError, ChildProcessError):
            pass
        try:
            os.close(self.fd)
        except OSError:
            pass


def start_client(tool, fx, settle=4.0):
    c = Client(client_argv(tool, fx), fx.client_env())
    # No common prompt to match across tools, so give both the same fixed time to connect and paint.
    if (settle and not c.settle(settle)) or not c.alive():
        raise RuntimeError(f"{tool} client did not start")
    return c


def measure_link(tool, fx, samples, rtt_ms, timeout=30, verbose=False):
    settle = 0.6 + 5 * rtt_ms / 1000
    # Panes persist between link settings, so make every marker unique or old output would match.
    tok = "".join(random.choice("abcdefghijklmnopqrstuvwxyz") for _ in range(6))
    c = start_client(tool, fx)
    echo, cmd = [], []
    try:
        for i in range(samples + 3):
            ch = bytes([ECHO_CHARS[i % len(ECHO_CHARS)]])
            c.write(ch)
            t = c.wait_for(ch, timeout)
            c.write(b"\x15")  # Ctrl-U clears the line
            c.settle(settle)
            if verbose:
                print(f"  {tool} key {i}: {None if t is None else round(t * 1000)} ms", flush=True)
            if t is not None and i >= 3:
                echo.append(t * 1000)
            c.write(f'echo MA""RK{tok}{i}\r'.encode())
            t = c.wait_for(f"MARK{tok}{i}".encode(), timeout, on_screen=True)
            c.settle(settle)
            if verbose:
                print(f"  {tool} cmd {i}: {None if t is None else round(t * 1000)} ms", flush=True)
                if t is None:
                    print(f"    last output seen: {c.last_seen!r}", flush=True)
            if t is not None and i >= 3:
                cmd.append(t * 1000)
    finally:
        c.close()
    return {"key_echo_ms": echo, "command_ms": cmd}


def measure_outage(tool, fx, timeout=60, window=0.6):
    """Reset every connection, then time until a command typed afterwards is answered by the shell.
    A bare echoed character proves nothing: once ssh dies the local tty echoes it itself."""
    c = start_client(tool, fx)
    try:
        c.write(b'echo OK""0\r')
        if c.wait_for(b"OK0", 20) is None:
            return {"recovery_s": None, "note": "no answer before the outage"}
        c.settle(1.5)
        t0 = time.perf_counter()
        fx.proxy.kill_all()
        n = 0
        while time.perf_counter() - t0 < timeout:
            if not c.alive():
                # tmux path: the ssh client died with the connection, start a new one at once.
                c = start_client(tool, fx, settle=0)
            n += 1
            c.write(f'echo RC""{n}\r'.encode())
            if c.wait_for(f"RC{n}".encode(), window) is not None:
                return {"recovery_s": time.perf_counter() - t0, "attempts": n}
        return {"recovery_s": None, "note": f"no answer within {timeout}s"}
    finally:
        c.close()


# --- reporting ----------------------------------------------------------------------------------


def stat(values):
    if not values:
        return "n/a"
    return f"{statistics.median(values):.0f} / {pct(values, 95):.0f}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--binary", default="target/release/teryon")
    ap.add_argument("--rtts", default="0,50,150,300", help="round-trip times in ms, comma separated")
    ap.add_argument("--jitter", type=float, default=10, help="extra random one-way delay up to this many ms")
    ap.add_argument("--samples", type=int, default=10)
    ap.add_argument("--outage", action="store_true", help="also time recovery after all connections are reset")
    ap.add_argument("--outage-trials", type=int, default=3)
    ap.add_argument("--no-proxy", action="store_true",
                    help="skip the userspace proxy (you apply dnctl/pfctl or tc netem to loopback yourself)")
    ap.add_argument("--timeout", type=float, default=30, help="seconds to wait for each echo")
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--json")
    args = ap.parse_args()
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))

    if not shutil.which("tmux"):
        raise SystemExit("tmux is required for the comparison")
    base = Path(tempfile.mkdtemp(prefix="bn", dir=(Path.home() / ".tb") if (Path.home() / ".tb").is_dir() else None))
    fx = Fixture(base, args.binary)
    results = {}
    try:
        fx.start(no_proxy=args.no_proxy)
        assert fx.proxy is not None
        for rtt in [float(x) for x in args.rtts.split(",")]:
            if not args.no_proxy:
                fx.proxy.set_delay(rtt / 2, args.jitter)
            for tool in ("teryon", "tmux"):
                print(f"rtt {rtt:g} ms: {tool} ...", flush=True)
                results.setdefault(tool, {})[rtt] = measure_link(tool, fx, args.samples, rtt, args.timeout, args.verbose)
        if args.outage:
            fx.proxy.set_delay(0, 0)
            for tool in ("teryon", "tmux"):
                print(f"outage: {tool} ...", flush=True)
                results.setdefault(tool, {})["outage"] = [measure_outage(tool, fx) for _ in range(args.outage_trials)]
    finally:
        fx.stop()
        shutil.rmtree(base, ignore_errors=True)

    print("\nmedian / p95 in ms; link = added RTT with up to %g ms one-way jitter\n" % args.jitter)
    rtts = [float(x) for x in args.rtts.split(",")]
    print("| link RTT | teryon key echo | tmux key echo | teryon burst | tmux burst |")
    print("|---|---|---|---|---|")
    for rtt in rtts:
        t, m = results["teryon"][rtt], results["tmux"][rtt]
        print(f"| {rtt:g} ms | {stat(t['key_echo_ms'])} | {stat(m['key_echo_ms'])} | "
              f"{stat(t['command_ms'])} | {stat(m['command_ms'])} |")
    if args.outage:
        print()
        for tool in ("teryon", "tmux"):
            vals = [o.get("recovery_s") for o in results[tool]["outage"]]
            shown = ", ".join(f"{v:.2f} s" if v is not None else "none" for v in vals)
            print(f"outage recovery, {tool}: {shown}")
    print(f"\nsamples per cell: {args.samples} (3 warm-up runs discarded); load average {os.getloadavg()[0]:.1f}")
    if args.json:
        Path(args.json).write_text(json.dumps(results, indent=2, default=str))


if __name__ == "__main__":
    main()

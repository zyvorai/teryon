#!/usr/bin/env python3
"""Launch the actual native app on Xvfb and capture its framebuffer.

Linux optional visual smoke test. Requires Xvfb, Pillow and software OpenGL.
No browser, fabricated screenshot, or screenshot of personal application data.
"""
import argparse
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time
from PIL import Image


def capture(path, output):
    raw = path.read_bytes()
    header = struct.unpack(">25I", raw[:100])
    size, version, fmt, depth, width, height, xoffset, byte_order, unit, bitorder, pad, bpp, stride, visual, red, green, blue, bits, entries, ncolors, *_ = header
    assert version == 7 and fmt == 2 and bpp == 32, header
    start = size + ncolors * 12
    image = Image.frombytes("RGB", (width, height), raw[start:start + stride * height], "raw", "BGRX" if byte_order == 0 else "XRGB", stride)
    image.save(output)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/debug/teryon")
    parser.add_argument("--xvfb", default="Xvfb")
    parser.add_argument("--output", default="docs/images")
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="zl-native-") as temp:
        root = Path(temp)
        # The caller and Xvfb must share the same network namespace.
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        display = port - 6000
        assert display > 0
        auth = root / "auth"
        fields = [b"", str(display).encode(), b"MIT-MAGIC-COOKIE-1", os.urandom(16)]
        auth.write_bytes(struct.pack(">H", 65535) + b"".join(struct.pack(">H", len(f)) + f for f in fields))
        auth.chmod(0o600)
        env = dict(os.environ, DISPLAY=f"127.0.0.1:{display}", XAUTHORITY=str(auth), LIBGL_ALWAYS_SOFTWARE="1", TERYON_RUNTIME_DIR=str(root / "run"))
        server = subprocess.Popen([args.xvfb, f":{display}", "-nolisten", "unix", "-nolisten", "local", "-listen", "tcp", "-auth", str(auth), "-screen", "0", "1180x760x24", "-fbdir", str(root), "-noreset"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        app = None
        try:
            deadline = time.monotonic() + 10
            while True:
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                        break
                except OSError:
                    if server.poll() is not None:
                        raise RuntimeError(server.stderr.read().decode())
                    assert time.monotonic() < deadline
                    time.sleep(0.1)
            def run(*args):
                return subprocess.run([binary, *args], env=env, text=True, capture_output=True, timeout=10, check=True)
            run("exec", "local", "-w", "fluxvm", "--wait", "--", "/bin/sh", "-c", "printf 'Persistent Rust workspace ready\\n'")
            run("send", "local", "-w", "fluxvm", "--enter", "printf '\\033[36mTERYON native desktop\\033[0m\\n\\nYour shell lives here. Your connection can come and go.\\n'")
            for theme in ["obsidian", "pearl"]:
                app = subprocess.Popen([binary, "desktop", "local", "-w", "fluxvm", "--theme", theme], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                time.sleep(3)
                assert app.poll() is None, app.stderr.read().decode()
                capture(root / "Xvfb_screen0", output / f"desktop-{theme}.png")
                app.terminate()
                app.wait(timeout=5)
                app = None
                time.sleep(0.3)
                print("PASS native window launch and framebuffer capture:", theme, flush=True)
            run("stop", "local", "--force")
        finally:
            if app is not None:
                app.kill()
                app.wait()
            try:
                subprocess.run([binary, "stop", "local", "--force"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
            except Exception:
                pass
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    main()

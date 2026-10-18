#!/usr/bin/env python3
"""OpenSSH-client integration against an isolated, key-authenticated SSH fixture.

Uses Paramiko only as the *test server*: no production Python dependency.
Does not alter ~/.ssh, sshd, account settings, or system services.
Run: python -m pip install -r tests/requirements.txt
     python scripts/e2e.py --binary target/debug/teryon
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import pty
import select
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request

import paramiko


def eventually(fn, timeout=15):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            result = fn()
            if result:
                return result
        except Exception as error:
            last = error
        time.sleep(0.1)
    raise AssertionError(f"condition not reached: {last}")


class Files(paramiko.SFTPServerInterface):
    def __init__(self, server, *args, home, **kwargs):
        super().__init__(server, *args, **kwargs)
        self.home = home

    def path(self, path):
        p = (self.home / path).resolve() if not path.startswith("/") else Path(path).resolve()
        if not p.is_relative_to(self.home):
            raise PermissionError("outside fixture home")
        return p

    def stat(self, path):
        try:
            return paramiko.SFTPAttributes.from_stat(self.path(path).stat())
        except OSError as e:
            return paramiko.SFTPServer.convert_errno(e.errno)

    lstat = stat

    def canonicalize(self, path):
        return str(self.path(path))

    def open(self, path, flags, attr):
        try:
            fd = os.open(self.path(path), flags, attr.st_mode or 0o600)
            file = os.fdopen(fd, "r+b" if flags & os.O_RDWR else "wb" if flags & os.O_WRONLY else "rb", buffering=0)
            class Handle(paramiko.SFTPHandle):
                def stat(self):
                    return paramiko.SFTPAttributes.from_stat(os.fstat(file.fileno()))

                def chattr(self, attrs):
                    if attrs.st_mode is not None:
                        os.fchmod(file.fileno(), attrs.st_mode)
                    if attrs.st_size is not None:
                        os.ftruncate(file.fileno(), attrs.st_size)
                    return paramiko.SFTP_OK

            handle = Handle(flags)
            if flags & (os.O_WRONLY | os.O_RDWR):
                handle.writefile = file
            if not flags & os.O_WRONLY:
                handle.readfile = file
            return handle
        except OSError as e:
            return paramiko.SFTPServer.convert_errno(e.errno)


class Server(paramiko.ServerInterface):
    def __init__(self, fixture):
        self.fixture = fixture
        self.forwards = {}

    def check_auth_publickey(self, username, key):
        return paramiko.AUTH_SUCCESSFUL if key == self.fixture.client_key else paramiko.AUTH_FAILED

    def get_allowed_auths(self, username):
        return "publickey"

    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_direct_tcpip_request(self, chanid, origin, destination):
        if destination[0] != "127.0.0.1":
            return paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
        self.forwards[chanid] = destination
        return paramiko.OPEN_SUCCEEDED

    def check_channel_exec_request(self, channel, command):
        def execute():
            env = dict(os.environ, HOME=str(self.fixture.home), SHELL="/bin/sh", TERYON_RUNTIME_DIR=str(self.fixture.home / "run"))
            process = subprocess.Popen(["/bin/sh", "-c", command.decode()], cwd=self.fixture.home, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            self.fixture.processes.append(process)

            def stdin():
                try:
                    while chunk := channel.recv(32768):
                        process.stdin.write(chunk)
                        process.stdin.flush()
                except (OSError, EOFError):
                    pass
                finally:
                    try:
                        process.stdin.close()
                    except OSError:
                        pass

            def output(file, send):
                try:
                    while chunk := os.read(file.fileno(), 32768):
                        send(chunk)
                except (OSError, EOFError):
                    pass

            threading.Thread(target=stdin, daemon=True).start()
            out = threading.Thread(target=output, args=(process.stdout, channel.sendall), daemon=True)
            err = threading.Thread(target=output, args=(process.stderr, channel.sendall_stderr), daemon=True)
            out.start()
            err.start()
            code = process.wait()
            out.join(timeout=2)
            err.join(timeout=2)
            try:
                channel.send_exit_status(code)
                channel.close()
            except (OSError, EOFError):
                pass

        threading.Thread(target=execute, daemon=True).start()
        return True


class Fixture:
    def __init__(self, root):
        self.home = root / "remote"
        self.home.mkdir()
        self.client_key = paramiko.RSAKey.generate(2048)
        self.host_key = paramiko.RSAKey.generate(2048)
        self.key = root / "key"
        self.client_key.write_private_key_file(str(self.key))
        self.key.chmod(0o600)
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.port = self.listener.getsockname()[1]
        self.transports = []
        self.processes = []
        self.stop = False
        (root / "known_hosts").write_text(f"[127.0.0.1]:{self.port} {self.host_key.get_name()} {self.host_key.get_base64()}\n")
        self.config = root / "ssh_config"
        self.config.write_text(f"Host fixture\n HostName 127.0.0.1\n Port {self.port}\n User fixture\n IdentityFile {self.key}\n IdentitiesOnly yes\n UserKnownHostsFile {root / 'known_hosts'}\n StrictHostKeyChecking yes\n LogLevel ERROR\n")
        threading.Thread(target=self.accept, daemon=True).start()

    def accept(self):
        while not self.stop:
            try:
                client, _ = self.listener.accept()
            except OSError:
                break
            threading.Thread(target=self.serve, args=(client,), daemon=True).start()

    def serve(self, client):
        transport = paramiko.Transport(client)
        self.transports.append(transport)
        transport.add_server_key(self.host_key)
        class SFTP(paramiko.SFTPServer):
            def finish_subsystem(self):
                try:
                    self.sock.send_exit_status(0)
                except (OSError, EOFError):
                    pass
                super().finish_subsystem()

        transport.set_subsystem_handler("sftp", SFTP, Files, home=self.home)
        server = Server(self)
        try:
            transport.start_server(server=server)
            while transport.is_active():
                channel = transport.accept(timeout=0.2)
                if channel is not None and channel.chanid in server.forwards:
                    destination = server.forwards[channel.chanid]

                    def forward(channel=channel, destination=destination):
                        try:
                            with socket.create_connection(destination) as remote:
                                while True:
                                    ready, _, _ = select.select([channel, remote], [], [], 1)
                                    for source in ready:
                                        data = source.recv(65536)
                                        if not data:
                                            return
                                        (remote.sendall if source is channel else channel.sendall)(data)
                        finally:
                            channel.close()

                    threading.Thread(target=forward, daemon=True).start()
        except (EOFError, OSError, paramiko.SSHException):
            pass
        finally:
            transport.close()

    def drop_connections(self):
        for transport in list(self.transports):
            transport.close()

    def close(self):
        self.stop = True
        self.listener.close()
        self.drop_connections()
        for process in self.processes:
            if process.poll() is None:
                process.terminate()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/debug/teryon")
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    passed = []
    with tempfile.TemporaryDirectory(prefix="teryon-e2e-") as temp:
        root = Path(temp)
        fixture = Fixture(root)
        local_home = root / "client"
        local_home.mkdir()
        env = dict(os.environ, HOME=str(local_home), TERYON_SSH_CONFIG=str(fixture.config), TERM="xterm-256color")

        def run(*args, code=0):
            result = subprocess.run([binary, *args], env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return result.stdout

        def success(name):
            passed.append(name)
            print("PASS", name, flush=True)

        tui = tunnel = None
        try:
            assert "Deployment verified" in run("deploy", "fixture", "--binary", binary)
            success("OpenSSH/SFTP deployment and remote doctor")
            info = run("doctor", "fixture")
            assert '"os": "linux"' in info
            run("exec", "fixture", "-w", "jobs", "--wait", "--", "/bin/sh", "-c", "printf 'SSH_TASK_OK\\n'; exit 7", code=7)
            assert "SSH_TASK_OK" in run("capture", "fixture", "-w", "jobs", "--pane", "2")
            success("Remote task execution, captured output and exit status")
            layout = root / "layout.json"
            run("layout", "save", "fixture", "-w", "jobs", str(layout))
            run("layout", "open", "fixture", "-w", "restored", str(layout))
            assert "Shell" in run("panes", "fixture", "-w", "restored")
            success("Layout save and fresh-shell restoration")

            # A real terminal UI, not just a protocol client.
            master, slave = pty.openpty()
            import fcntl
            import struct
            import termios
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 120, 0, 0))
            tui = subprocess.Popen([binary, "connect", "fixture", "-w", "interactive"], env=env, stdin=slave, stdout=slave, stderr=slave)
            os.close(slave)
            terminal_output = bytearray()
            stop_reader = threading.Event()

            def drain():
                while not stop_reader.is_set():
                    try:
                        ready, _, _ = select.select([master], [], [], 0.1)
                        if ready:
                            terminal_output.extend(os.read(master, 65536))
                    except OSError:
                        return

            threading.Thread(target=drain, daemon=True).start()
            eventually(lambda: "interactive" in run("sessions", "fixture"))
            # Session creation precedes the asynchronous UI's Online update.
            # Never inject keystrokes until the actual client reports ready.
            eventually(lambda: b"CONNECTED" in terminal_output)
            os.write(master, b"printf 'BEFORE_DROP\\n'\r")
            eventually(lambda: "BEFORE_DROP" in run("capture", "fixture", "-w", "interactive"))
            before = run("panes", "fixture", "-w", "interactive").split("pid=")[1].split("\t")[0]
            fixture.drop_connections()
            eventually(lambda: b"RECONNECTING" in terminal_output)
            time.sleep(1)
            eventually(lambda: json.loads(run("sessions", "fixture"))["items"] and any(w["name"] == "interactive" and w["controlled"] for w in json.loads(run("sessions", "fixture"))["items"]))
            after = run("panes", "fixture", "-w", "interactive").split("pid=")[1].split("\t")[0]
            assert before == after
            os.write(master, b"printf 'AFTER_RECONNECT\\n'\r")
            eventually(lambda: "AFTER_RECONNECT" in run("capture", "fixture", "-w", "interactive"))
            os.write(master, b"\x00n")
            eventually(lambda: len(run("panes", "fixture", "-w", "interactive").strip().splitlines()) == 2)
            for action in [b"\x00t",b"\x00s",b"\x00?",b"\x1b",b"\x00d"]:
                os.write(master,action)
                time.sleep(0.3)
            tui.wait(timeout=10)
            assert tui.returncode == 0
            stop_reader.set()
            os.close(master)
            success("TUI reconnect after SSH transport loss with same shell PID, pane creation and keyboard controls")

            # Special filenames ensure paths cannot become remote shell commands.
            filename = "file with spaces $literal ' λ.txt"
            source = root / filename
            source.write_bytes(os.urandom(512 * 1024))
            run("push", "fixture", str(source), filename)
            downloaded = root / "downloaded.txt"
            run("pull", "fixture", filename, str(downloaded))
            assert hashlib.sha256(source.read_bytes()).digest() == hashlib.sha256(downloaded.read_bytes()).digest()
            success("Resumable-transfer path with special characters and SHA-256 round trip")

            # Force an interrupted, bandwidth-limited upload, then resume it.
            large=root/'large.bin'
            large.write_bytes(os.urandom(2*1024*1024))
            interrupted=subprocess.Popen([binary,'push','fixture',str(large),'resume.bin','--limit-kbps','128'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
            try:
                time.sleep(1.2)
                assert interrupted.poll() is None,'bandwidth-limited transfer finished unexpectedly'
                import signal
                os.killpg(interrupted.pid,signal.SIGTERM)
                interrupted.wait(timeout=5)
                eventually(lambda:(fixture.home/'.teryon-partial'/'resume.bin').exists())
                run('push','fixture',str(large),'resume.bin')
                assert hashlib.sha256(large.read_bytes()).digest()==hashlib.sha256((fixture.home/'resume.bin').read_bytes()).digest()
            finally:
                if interrupted.poll() is None:
                    os.killpg(interrupted.pid,signal.SIGKILL)
                    interrupted.wait()
            success('Interrupted upload retains partial data and resumes to the correct SHA-256')

            class Handler(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    self.send_response(200)
                    self.end_headers()
                    self.wfile.write(b"TERYON_FORWARD_OK")

                def log_message(self, *args):
                    pass

            httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            threading.Thread(target=httpd.serve_forever, daemon=True).start()
            with socket.socket() as s:
                s.bind(("127.0.0.1", 0))
                port = s.getsockname()[1]
            tunnel = subprocess.Popen([binary, "forward", "fixture", f"{port}:{httpd.server_port}", "--reconnect"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)

            def fetch():
                return urllib.request.urlopen(f"http://127.0.0.1:{port}", timeout=1).read() == b"TERYON_FORWARD_OK"

            eventually(fetch)
            fixture.drop_connections()
            time.sleep(0.4)
            eventually(fetch)
            success("Loopback HTTP port forwarding and automatic tunnel reconnection")
            httpd.shutdown()
            run("stop", "fixture", "--force")
            success("Explicit forced remote shutdown")
        finally:
            if tui is not None and tui.poll() is None:
                tui.kill()
                tui.wait()
            if tunnel is not None:
                import signal
                try:
                    os.killpg(tunnel.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                tunnel.wait()
            # A failed test must not leave its daemon or shells behind.
            try:
                run("stop", "fixture", "--force")
            except Exception:
                pass
            fixture.close()
    print(json.dumps({"passed": len(passed), "scenarios": passed}, indent=2))


if __name__ == "__main__":
    main()

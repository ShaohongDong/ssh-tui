#!/usr/bin/env python3
"""Isolated interactive acceptance: no real hosts, credentials or SSH config.

Requires paramiko, pexpect, pyte. All fixtures listen on loopback and disappear
after the run. The output contains screen snapshots and a machine-readable report.
"""
import argparse
import base64
import fcntl
import json
import os
from pathlib import Path
import platform
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import threading
import time
import tomllib

import paramiko
import pexpect
import pyte
from wcwidth import wcwidth

from e2e_support import config_revision, reap_shell, wait_for_config, write_pty

BASH_EXECUTABLE = shutil.which("bash")


def process_table():
    """Use POSIX ps options shared by GNU ps and macOS's BSD ps."""
    rows = subprocess.check_output(
        ["ps", "-axww", "-o", "pid=,ppid=,command="], text=True
    )
    return [line.strip().split(None, 2) for line in rows.splitlines() if line.strip()]


class Screen(pyte.Screen):
    @property
    def display(self):
        # pyte 0.8 can retain an empty wide-character stub after a popup
        # overwrites its leading cell. Render orphan stubs as blank cells.
        result = []
        for y in range(self.lines):
            text, continuation = [], False
            for x in range(self.columns):
                if continuation:
                    continuation = False
                    continue
                char = self.buffer[y][x].data or " "
                text.append(char)
                continuation = wcwidth(char[0]) == 2
            result.append("".join(text))
        return result


class SSHServer(paramiko.ServerInterface):
    def __init__(self, owner):
        self.owner = owner
        self.ready = threading.Event()
        self.master = None
        self.size = (29, 120)

    def get_allowed_auths(self, username):
        return "publickey,password"

    def check_auth_publickey(self, username, key):
        return paramiko.AUTH_SUCCESSFUL if key == self.owner.client_key else paramiko.AUTH_FAILED

    def check_auth_password(self, username, password):
        return paramiko.AUTH_SUCCESSFUL if password == "fixture-ssh-password" else paramiko.AUTH_FAILED

    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_pty_request(self, channel, term, width, height, *args):
        self.size = (height, width)
        return True

    def check_channel_window_change_request(self, channel, width, height, *args):
        self.size = (height, width)
        if self.master is not None:
            fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        return True

    def check_channel_exec_request(self, channel, command):
        if self.owner.reject_exec:
            return False
        # Image upload opens a second channel on the authenticated transport.
        def execute():
            process = None
            try:
                payload = bytearray()
                while True:
                    chunk = channel.recv(65536)
                    if not chunk:
                        break
                    payload.extend(chunk)
                    if len(payload) > 21 * 1024 * 1024:
                        return
                process = subprocess.Popen(
                    ["sh", "-c", command.decode("utf-8")],
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    cwd=self.owner.root, start_new_session=True,
                )
                out, err = process.communicate(bytes(payload), timeout=35)
                self.owner.exec_results.append((bytes(payload), out, process.returncode))
                channel.sendall(out)
                channel.sendall_stderr(err)
                channel.send_exit_status(process.returncode)
            except (OSError, EOFError, subprocess.TimeoutExpired):
                pass
            finally:
                if process is not None and process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                channel.close()
        threading.Thread(target=execute, daemon=True).start()
        return True

    def check_channel_shell_request(self, channel):
        self.ready.set()
        return True


class SSHFixture:
    def __init__(self, root, client_key):
        self.root, self.client_key = root, client_key
        self.host_key = paramiko.RSAKey.generate(2048)
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.listener.settimeout(0.2)
        self.port = self.listener.getsockname()[1]
        self.transports = []
        self.exec_results = []
        self.reject_exec = False
        self.shells = []
        self.workers = []
        self.errors = []
        self.closed = False
        self.accept_thread = threading.Thread(target=self.accept, daemon=True)
        self.accept_thread.start()

    def accept(self):
        while not self.closed:
            try:
                conn, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            worker = threading.Thread(target=self.serve, args=(conn,), daemon=True)
            self.workers.append(worker)
            worker.start()

    def serve(self, conn):
        transport = paramiko.Transport(conn)
        self.transports.append(transport)
        server = SSHServer(self)
        process = None
        master = None
        input_thread = None
        stopped = threading.Event()
        try:
            transport.add_server_key(self.host_key)
            transport.start_server(server=server)
            channel = transport.accept(15)
            if channel is None or not server.ready.wait(15):
                return
            master, slave = os.openpty()
            os.set_blocking(master, False)
            server.master = master
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", *server.size, 0, 0))

            def child_setup():
                os.setsid()
                fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

            env = dict(os.environ, HOME=str(self.root), TERM="xterm-256color", PS1="FIXTURE> ", HISTFILE="/dev/null")
            process = subprocess.Popen(
                [BASH_EXECUTABLE, "--noprofile", "--norc", "-i"],
                stdin=slave, stdout=slave, stderr=slave,
                env=env, cwd=self.root, preexec_fn=child_setup,
            )
            self.shells.append(process)
            os.close(slave)

            def input_loop():
                try:
                    while transport.is_active() and not stopped.is_set():
                        data = channel.recv(16384)
                        if not data:
                            break
                        write_pty(master, data, stopped)
                except (OSError, EOFError):
                    pass
                try:
                    os.killpg(process.pid, signal.SIGHUP)
                except ProcessLookupError:
                    pass

            input_thread = threading.Thread(target=input_loop, daemon=True)
            input_thread.start()
            while transport.is_active():
                transport.accept(0)
                if select.select([master], [], [], 0.1)[0]:
                    try:
                        data = os.read(master, 16384)
                    except BlockingIOError:
                        continue
                    except OSError:
                        break
                    if not data:
                        break
                    channel.sendall(data)
                if process.poll() is not None:
                    break
            if process.poll() is not None:
                channel.send_exit_status(max(0, process.returncode))
        except (EOFError, OSError, paramiko.SSHException):
            pass
        except Exception as error:
            self.errors.append(error)
        finally:
            stopped.set()
            transport.close()
            try:
                if input_thread is not None:
                    input_thread.join(timeout=5)
                    if input_thread.is_alive():
                        self.errors.append(AssertionError("SSH fixture input thread did not stop"))
                server.master = None
                descriptor, master = master, None
                if process is not None:
                    reap_shell(process, master=descriptor)
                elif descriptor is not None:
                    os.close(descriptor)
            except Exception as error:
                self.errors.append(error)
            finally:
                server.master = None
                if master is not None:
                    os.close(master)

    def close(self):
        self.closed = True
        self.listener.close()
        self.accept_thread.join(timeout=5)
        assert not self.accept_thread.is_alive(), "SSH fixture listener did not stop"
        for transport in self.transports:
            transport.close()
        for worker in self.workers:
            worker.join(timeout=10)
            assert not worker.is_alive(), "SSH fixture worker did not stop"
        assert not self.errors, f"SSH fixture cleanup failed: {self.errors!r}"
        assert all(shell.poll() is not None for shell in self.shells), "SSH fixture shell leaked"


class ProxyFixture:
    def __init__(self, auth=False, reject=False, family=socket.AF_INET):
        self.auth, self.reject = auth, reject
        self.requests = []
        self.listener = socket.socket(family)
        self.listener.bind(("::1" if family == socket.AF_INET6 else "127.0.0.1", 0))
        self.listener.listen()
        self.port = self.listener.getsockname()[1]
        self.host = "[::1]" if family == socket.AF_INET6 else "127.0.0.1"
        self.closed = False
        threading.Thread(target=self.accept, daemon=True).start()

    @property
    def url(self):
        return f"http://{self.host}:{self.port}"

    def accept(self):
        while not self.closed:
            try:
                conn, _ = self.listener.accept()
            except OSError:
                return
            threading.Thread(target=self.serve, args=(conn,), daemon=True).start()

    def serve(self, conn):
        remote = None
        try:
            conn.settimeout(5)
            data = b""
            while b"\r\n\r\n" not in data:
                block = conn.recv(4096)
                if not block:
                    return
                data += block
            # Record only request target, never the authorization header.
            self.requests.append(data.split(b"\r\n", 1)[0].decode())
            expected = base64.b64encode(b"fixture-proxy-user:fixture-proxy-password")
            if self.auth and b"Proxy-Authorization: Basic " + expected not in data:
                conn.sendall(b"HTTP/1.1 407 Auth\r\nProxy-Authenticate: Basic realm=fixture\r\n\r\n")
                return
            if self.reject:
                conn.sendall(b"HTTP/1.1 403 Forbidden\r\n\r\n")
                return
            target = data.split(b" ")[1].decode()
            host, port = target.rsplit(":", 1)
            remote = socket.create_connection((host.strip("[]"), int(port)), 5)
            banner = remote.recv(4096)
            # Deliberately coalesce the response headers and SSH banner.
            conn.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n" + banner)
            conn.settimeout(None)
            remote.settimeout(None)
            while True:
                readable, _, _ = select.select([conn, remote], [], [], 5)
                for source in readable:
                    block = source.recv(16384)
                    if not block:
                        return
                    (remote if source is conn else conn).sendall(block)
        except OSError:
            pass
        finally:
            conn.close()
            if remote:
                remote.close()

    def close(self):
        self.closed = True
        self.listener.close()


class Client:
    def __init__(self, binary, root, env, language, extra=()):
        self.root = root
        self.id = str(time.time_ns())
        self.before = root / f"tty-before-{self.id}"
        self.after = root / f"tty-after-{self.id}"
        args = [str(binary), "--config", str(root / "config.toml"), *extra]
        if language is not None:
            args.extend(["--language", language])
        script = f"stty -g > {shlex.quote(str(self.before))}; {shlex.join(args)}; result=$?; stty -g > {shlex.quote(str(self.after))}; exit $result"
        self.child = pexpect.spawn(BASH_EXECUTABLE, ["--noprofile", "--norc", "-c", script], env=env, encoding="utf-8", codec_errors="replace", dimensions=(31, 120), timeout=8)
        self.screen = Screen(120, 31)
        self.stream = pyte.Stream(self.screen)
        self.raw = ""

    def pump(self, seconds=0.15):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            try:
                data = self.child.read_nonblocking(65536, timeout=0.05)
                self.raw += data
                self.stream.feed(data)
            except pexpect.TIMEOUT:
                continue
            except pexpect.EOF:
                break

    @property
    def text(self):
        return "\n".join(self.screen.display)

    def wait(self, text, timeout=10):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.pump()
            if text in self.text:
                return
        raise AssertionError(f"Missing {text!r}\n{self.text}")

    def wait_line(self, text, timeout=10):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.pump()
            if text in [line.strip() for line in self.screen.display[1:-1]]:
                return
        raise AssertionError(f"Missing output line {text!r}\n{self.text}")

    def wait_prompt(self):
        end = time.monotonic() + 10
        while time.monotonic() < end:
            self.pump()
            rows = [line.strip() for line in self.screen.display[1:-1] if line.strip()]
            if rows and rows[-1] == "FIXTURE>":
                return
        raise AssertionError(f"Shell prompt did not return\n{self.text}")

    def send(self, text):
        self.child.send(text)
        self.pump()

    def save(self, text, predicate):
        path = self.root / "config.toml"
        previous = config_revision(path)
        self.send(text)
        try:
            return wait_for_config(path, predicate, previous=previous, pump=self.pump)
        except AssertionError as error:
            raise AssertionError(f"{error}\n{self.text}") from error

    def command(self, text):
        self.send(text + "\r")

    def prefix(self, text):
        self.send("\x1d" + text)

    def stop(self):
        self.prefix("q")
        self.pump()
        if "退出并关闭" in self.text or "Quit and close" in self.text:
            self.send("y")
        self.child.expect(pexpect.EOF, timeout=10)
        self.child.close()
        assert self.child.exitstatus == 0, self.child.exitstatus
        assert self.before.read_text() == self.after.read_text(), "terminal attributes not restored"

    def kill(self):
        if self.child.isalive():
            # Signal the executable, rather than the wrapper shell.
            kids = [int(pid) for pid, parent, *_ in process_table()
                    if int(parent) == self.child.pid]
            assert kids, "could not locate the executable for the SIGTERM test"
            for pid in kids:
                os.kill(pid, signal.SIGTERM)
            self.child.expect(pexpect.EOF, timeout=10)
            self.child.close()
        assert self.before.read_text() == self.after.read_text(), "signal exit did not restore terminal"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/release/ssh-tui"))
    parser.add_argument("--language", choices=("en", "zh"), default="en")
    parser.add_argument("--output", type=Path, default=Path("target/e2e"))
    args = parser.parse_args()
    binary = args.binary.resolve()
    assert platform.system() in ("Linux", "Darwin"), "acceptance requires Linux or macOS"
    assert binary.is_file(), f"missing binary: {binary}"
    commands = {name: shutil.which(name) for name in ("ssh", "bash", "top", "less", "vi", "python3", "ps", "stty", "seq")}
    assert all(commands.values()), f"missing commands: {[name for name, path in commands.items() if not path]}"
    args.output.mkdir(parents=True, exist_ok=True)
    report = []
    def t(zh, en):
        return en if args.language == "en" else zh

    def passed(name):
        report.append(name)
        print("PASS", name, flush=True)

    with tempfile.TemporaryDirectory(prefix="ssh-tui-fixture-") as tmp:
        root = Path(tmp)
        key = paramiko.RSAKey.generate(2048)
        key.write_private_key_file(str(root / "identity"))
        os.chmod(root / "identity", 0o600)
        ssh = SSHFixture(root, key)
        proxy = ProxyFixture()
        auth = ProxyFixture(auth=True)
        reject = ProxyFixture(reject=True)
        ipv6 = ProxyFixture(family=socket.AF_INET6)
        (root / "ssh_config").write_text(f"Host alias-target\n HostName 127.0.0.1\n Port {ssh.port}\n User fixture\nHost jump-target\n HostName 127.0.0.1\n Port {ssh.port}\n ProxyJump alias-target\nHost *\n User fixture\n IdentityFile {root}/identity\n IdentitiesOnly yes\n UserKnownHostsFile {root}/known_hosts\n GlobalKnownHostsFile /dev/null\n StrictHostKeyChecking ask\n")
        bindir = root / "bin"
        bindir.mkdir()
        (bindir / "ssh").write_text(f"#!/bin/sh\nexec {shlex.quote(commands['ssh'])} -F {shlex.quote(str(root / 'ssh_config'))} \"$@\"\n")
        (bindir / "ssh").chmod(0o700)
        env = dict(os.environ, HOME=str(root), XDG_CONFIG_HOME=str(root), PATH=f"{bindir}:{os.environ['PATH']}", TERM="xterm-256color", COLORTERM="truecolor", LC_ALL="en_US.UTF-8" if platform.system() == "Darwin" else "C.UTF-8")
        # Force password authentication for the password login scenario.
        (root / "config.toml").write_text(f'''theme = "Tokyo Night"
scrollback = 10000
[[connections]]
name = "direct"
host = "alias-target"
route = {{ mode = "direct" }}
[[connections]]
name = "http"
host = "alias-target"
route = {{ mode = "http", name = "local" }}
[[proxies]]
name = "local"
url = "{proxy.url}"
''')
        clients = []
        try:
            c = Client(binary, root, env, args.language, ["alias-target"])
            clients.append(c)
            c.wait("Are you sure")
            c.command("yes")
            c.wait("FIXTURE>")
            passed("new host fingerprint confirmation and public-key login")
            c.command("printf 'SIZE:'; stty size")
            c.wait("SIZE:29 120")
            c.child.setwinsize(35, 100)
            c.screen.resize(35, 100)
            c.pump()
            c.command("printf 'RESIZED:'; stty size")
            c.wait("RESIZED:33 100")
            passed("remote PTY initial dimensions and resize")
            c.command("sleep 20; printf 'CTRL_C_RETURNED\\n'")
            c.send("\x03")
            c.wait("FIXTURE>")
            c.command("printf 'INTERRUPT_ALIVE\\n'")
            c.wait_line("INTERRUPT_ALIVE")
            passed("Ctrl+C interrupts remote sleep and keeps client alive")
            c.command("python3 -c 'import os,sys,tty,termios; old=termios.tcgetattr(0); tty.setraw(0); os.write(1,b\"KEY_READY\\r\\n\"); b=sys.stdin.buffer.read(3); termios.tcsetattr(0,termios.TCSANOW,old); print(\"KEY_BYTES=\"+b.hex())'")
            c.wait_line("KEY_READY")
            c.prefix("\x1d")
            c.prefix("z")
            c.wait_line("KEY_BYTES=1d1d7a")
            passed("double prefix and unknown prefix combinations reach remote unchanged")
            c.send("\x1b[200~printf 'PASTE_KEYS_OK\\n' # \x1d q\x1b[201~")
            c.send("\r")
            c.wait_line("PASTE_KEYS_OK")
            c.send("\x1b[200~printf 'LONG_PASTE_OK\\n' # " + "x" * 18000 + "\x1b[201~")
            c.send("\r")
            c.wait_line("LONG_PASTE_OK")
            passed("bracketed paste cannot trigger local shortcuts and large paste remains responsive")
            c.command("printf '中文\\n'; printf '\\033[31mRED\\033[0m\\n'")
            c.wait("中文")
            c.wait("RED")
            passed("UTF-8 wide characters and ANSI colors")
            # Plain l must reach the remote terminal; only prefix-l is local.
            c.command("python3 -c 'import os,sys,tty,termios; old=termios.tcgetattr(0); tty.setraw(0); os.write(1,b\"LANG_KEY_READY\\r\\n\"); b=sys.stdin.buffer.read(1); termios.tcsetattr(0,termios.TCSANOW,old); print(\"LANG_KEY=\"+b.hex())'")
            c.wait_line("LANG_KEY_READY")
            c.send("l")
            c.wait_line("LANG_KEY=6c")
            other = "zh" if args.language == "en" else "en"
            other_title = "语言" if other == "zh" else "Language"
            current_title = t("语言", "Language")
            c.prefix("l")
            c.wait(current_title)
            c.send("\x1b[B")
            c.wait(other_title)
            c.send("\x1b")
            c.command("printf 'LANG_CANCEL_ALIVE\\n'")
            c.wait_line("LANG_CANCEL_ALIVE")
            c.prefix("l")
            c.wait(current_title)
            c.save("\x1b[B\r", lambda cfg: cfg.get("language") == other)
            c.command("printf 'LANG_SAVE_ALIVE\\n'")
            c.wait_line("LANG_SAVE_ALIVE")
            # A fresh process with no language override reads the saved language.
            restored = Client(binary, root, env, None)
            clients.append(restored)
            restored.wait("连接列表" if other == "zh" else "Connections")
            restored.stop()
            c.prefix("c")
            c.wait("连接列表" if other == "zh" else "Connections")
            c.save("l\x1b[B\r", lambda cfg: cfg.get("language") == args.language)
            c.wait(t("连接列表", "Connections"))
            # An override survives unrelated saves without replacing the stored language.
            override = Client(binary, root, env, other)
            clients.append(override)
            override.wait("连接列表" if other == "zh" else "Connections")
            override.save("p0", lambda cfg: cfg.get("language") == args.language)
            override.send("\x1b")
            override.stop()
            c.send("\t")
            c.command("printf 'LANG_RETURN_ALIVE\\n'")
            c.wait_line("LANG_RETURN_ALIVE")
            passed("language preview, cancel, save, restart, CLI override and active-session continuity; plain l forwarded")
            c.prefix("t")
            c.wait(t("主题预览", "Theme preview"))
            c.send("\x1b[B")
            c.send("\x1b")
            c.wait("Tokyo Night")
            assert "theme = \"Tokyo Night\"" in (root / "config.toml").read_text()
            c.prefix("t")
            c.send("\x1b[B")
            c.save("\r", lambda cfg: cfg.get("theme") == "Dracula")
            c.wait("Dracula")
            passed("theme preview cancel and persist")
            ubuntu_override = Client(binary, root, env, args.language, ["--theme", "Ubuntu 22.04"])
            clients.append(ubuntu_override)
            ubuntu_override.wait("Ubuntu 22.04")
            ubuntu_override.save("p0", lambda cfg: cfg.get("theme") == "Dracula")
            ubuntu_override.send("\x1b")
            ubuntu_override.stop()
            c.prefix("t")
            c.send("\x1b[H\x1b[5~")  # Home and PageUp stop at the first theme.
            c.wait("1/13")
            c.send("\x1b[6~")  # This wide picker displays ten list rows.
            c.wait("11/13 Everforest Dark")
            c.send("\x1b[5~")
            c.wait("1/13")
            c.send("\x1b[F")
            c.wait("13/13 Ubuntu 22.04")
            (args.output / "theme-picker-last.txt").write_text(c.text)
            c.send("\x1b[B")  # Single-row navigation still wraps.
            c.wait("1/13")
            c.send("\x1b[A")
            c.wait("13/13 Ubuntu 22.04")
            c.send("\x1b")
            assert tomllib.loads((root / "config.toml").read_text())["theme"] == "Dracula"
            c.prefix("t")
            c.send("\x1b[H" + "\x1b[B" * 6)
            for i, name in enumerate(("Catppuccin Mocha", "One Dark", "Monokai", "Kanagawa", "Everforest Dark", "Rose Pine", "Ubuntu 22.04")):
                c.wait(f"{i + 7}/13 {name}")
                if i < 6:
                    c.send("\x1b[B")
            c.save("\r", lambda cfg: cfg.get("theme") == "Ubuntu 22.04")
            c.command("printf 'THEME_SESSION_ALIVE\\n'")
            c.wait_line("THEME_SESSION_ALIVE")
            restarted = Client(binary, root, env, args.language)
            clients.append(restarted)
            restarted.wait("Ubuntu 22.04")
            restarted.stop()
            theme_override = Client(binary, root, env, args.language, ["--theme", "Catppuccin Mocha"])
            clients.append(theme_override)
            theme_override.wait("Catppuccin Mocha")
            theme_override.save("p0", lambda cfg: cfg.get("theme") == "Ubuntu 22.04")
            theme_override.send("\x1b")
            theme_override.stop()
            passed("all seven additional themes; paging, first/last, wrap, cancel, restart, CLI override and active-session continuity")
            c.command("(sleep 1; printf 'BACKGROUND_FIRST\\n') &")
            c.prefix("c")
            c.wait(t("连接列表", "Connections"))
            c.send("\x1b[B\r")
            c.wait("HTTP: local")
            c.wait("FIXTURE>")
            c.command("printf 'SECOND_SESSION\\n'")
            c.wait_line("SECOND_SESSION")
            c.prefix("p")
            c.wait_line("FIXTURE> BACKGROUND_FIRST")
            assert "SECOND_SESSION" not in "\n".join(c.screen.display[1:-1])
            c.prefix("n")
            c.wait("SECOND_SESSION")
            passed("HTTP CONNECT login, background output, tab input isolation")
            assert any(f"127.0.0.1:{ssh.port}" in req for req in proxy.requests)
            c.command("for n in $(seq 1 80); do echo HISTORY_$n; done")
            c.wait("HISTORY_80")
            c.prefix("[")
            c.send("\x1b[5~")
            c.wait(t("滚屏浏览", "History"))
            assert "HISTORY_80" not in "\n".join(c.screen.display[1:-1])
            c.send("\x1b")
            c.wait("HISTORY_80")
            passed("scrollback view and return to live terminal")
            c.command("top")
            c.wait("Load Avg" if platform.system() == "Darwin" else "load average")
            c.send("q")
            c.wait_prompt()
            c.command("seq 1 200 | less")
            c.wait_line(":")
            c.send("q")
            c.wait_prompt()
            c.command("vi e2e-file")
            c.wait("e2e-file")
            c.send("iVIM_EDIT_OK\x1b:wq\r")
            c.wait_prompt()
            assert "VIM_EDIT_OK" in (root / "e2e-file").read_text()
            passed("top, less and vi fullscreen interactions")
            (args.output / "terminal.txt").write_text(c.text)
            c.prefix("c")
            (args.output / "manager.txt").write_text(c.text)
            c.send("\t")
            c.command("exit")
            c.wait(t("已断开", "Disconnected"))
            c.send("r")
            c.wait("FIXTURE>")
            passed("disconnect retains screen and manual reconnect")
            c.stop()
            passed("normal exit restores terminal and closes child processes")

            c = Client(binary, root, env, args.language, ["alias-target", "--proxy", auth.url, "--proxy-user", "fixture-proxy-user"])
            clients.append(c)
            c.wait(t("HTTP 代理密码", "HTTP proxy password"))
            raw_start = len(c.raw)
            c.command("fixture-proxy-password")
            c.wait("FIXTURE>")
            assert "fixture-proxy-password" not in c.raw[raw_start:]
            c.prefix("t")
            c.save("\x1b[B\r", lambda cfg: cfg.get("theme") == "Tokyo Night"
                   and not any(p["name"] == "__command_line__" for p in cfg.get("proxies", [])))
            c.stop()
            assert "fixture-proxy-password" not in (root / "config.toml").read_text()
            passed("Basic proxy authentication, hidden input and no password persistence")

            c = Client(binary, root, env, args.language, ["alias-target", "--proxy", auth.url, "--proxy-user", "fixture-proxy-user"])
            clients.append(c)
            c.wait(t("HTTP 代理密码", "HTTP proxy password"))
            c.command("wrong-fixture-password")
            c.wait("407")
            assert "FIXTURE>" not in c.text
            c.stop()
            passed("invalid proxy password returns 407 and never falls back direct")

            c = Client(binary, root, env, args.language, ["alias-target", "--proxy", reject.url])
            clients.append(c)
            c.wait("403")
            assert "FIXTURE>" not in c.text
            c.stop()
            passed("proxy target rejection returns 403 and never falls back direct")

            c = Client(binary, root, env, args.language, ["jump-target", "--proxy", proxy.url])
            clients.append(c)
            c.wait(t("冲突", "conflicts"))
            c.stop()
            passed("effective OpenSSH ProxyJump conflict is rejected")

            c = Client(binary, root, env, args.language, ["alias-target", "--proxy", ipv6.url])
            clients.append(c)
            c.wait("FIXTURE>")
            c.kill()
            passed("IPv6 HTTP proxy and SIGTERM terminal restoration")

            c = Client(binary, root, env, args.language)
            clients.append(c)
            c.wait(t("连接列表", "Connections"))

            # Verify the compact UI through the real terminal event path too.
            def resize_ui(width, height):
                c.child.setwinsize(height, width)
                c.screen.resize(height, width)
                c.pump()

            resize_ui(80, 24)
            frame_rows = [i for i, line in enumerate(c.screen.display) if "╭" in line or "╰" in line]
            assert len(frame_rows) == 2 and frame_rows[1] - frame_rows[0] + 1 == 8, c.text
            assert "direct" in c.text and "http" in c.text
            resize_ui(48, 12)
            c.send("a\r")
            c.wait(t("连接名称不能为空", "Connection name cannot be empty"))
            c.send("\x1b[12~")  # F2 opens the complete validation error.
            c.wait(t("详细信息", "Details"))
            c.wait(t("连接名称不能为空", "Connection name cannot be empty"))
            c.send("\x1b")
            c.wait(t("编辑连接", "Edit connection"))
            c.send("\x1b")
            c.send("?")
            c.wait(t("快捷键帮助", "Keyboard help"))
            c.send("\x1b[F")  # End scrolls the help panel, not a remote shell.
            c.wait(t("认证提示见", "authentication prompts"))
            c.wait(t("远程终端", "terminal."))
            c.send("\x1b")
            c.wait(t("连接列表", "Connections"))
            saved_theme = tomllib.loads((root / "config.toml").read_text())["theme"]
            c.send("t\x1b[B\x1b")
            c.wait(t("连接列表", "Connections"))
            assert tomllib.loads((root / "config.toml").read_text())["theme"] == saved_theme
            resize_ui(80, 24)

            def fill(fields, saved=None):
                for i, field in enumerate(fields):
                    c.send("\x15" + field)
                    if i + 1 < len(fields):
                        c.send("\t")
                if saved is None:
                    c.send("\r")
                else:
                    c.save("\r", saved)

            c.send("a")
            fill(["created-ui", "alias-target", "", "", "", "", "direct"],
                 lambda cfg: any(p["name"] == "created-ui" for p in cfg["connections"]))
            c.send("/created-ui\r")
            c.send("e")
            c.save("\x15edited-ui\r", lambda cfg: any(p["name"] == "edited-ui" for p in cfg["connections"]))
            c.send("/edited-ui\r")
            c.save("dy", lambda cfg: not any(p["name"] in ["created-ui", "edited-ui"] for p in cfg["connections"]))
            passed("connection form create, search, edit, delete and persistence")
            c.send("p")
            c.send("a")
            fill(["created-proxy", proxy.url, ""],
                 lambda cfg: any(p["name"] == "created-proxy" for p in cfg["proxies"]))
            c.save("\x1b[B\r", lambda cfg: cfg.get("default_proxy") == "created-proxy")
            c.save("0", lambda cfg: "default_proxy" not in cfg)
            c.send("e")
            c.save("\x15edited-proxy\r", lambda cfg: any(p["name"] == "edited-proxy" for p in cfg["proxies"]))
            c.save("dy", lambda cfg: "default_proxy" not in cfg
                   and not any(p["name"] in ["created-proxy", "edited-proxy"] for p in cfg["proxies"]))
            passed("proxy form create, default selection, disable, edit and delete")
            c.send("\x1b")
            c.send("c")
            fill(["quick-ui", "alias-target", "", "", "", "", "direct"])
            c.wait("FIXTURE>")
            cfg = tomllib.loads((root / "config.toml").read_text())
            assert not any(p["name"] == "quick-ui" for p in cfg["connections"])
            c.prefix("x")
            c.send("n")
            c.command("printf 'CANCEL_CLOSE_OK\\n'")
            c.wait_line("CANCEL_CLOSE_OK")
            c.prefix("x")
            c.send("y")
            c.wait(t("连接列表", "Connections"))
            c.stop()
            passed("quick connection is not saved; close confirmation cancel and accept")

            # Use a separate identity configuration with no usable key.
            with (root / "ssh_config").open("a") as f:
                f.write("\nHost *\n PubkeyAuthentication no\n PreferredAuthentications password\n")
            c = Client(binary, root, env, args.language, ["alias-target"])
            clients.append(c)
            c.wait("password:")
            c.command("fixture-ssh-password")
            c.wait("FIXTURE>")
            c.stop()
            passed("SSH password authentication")

            ssh.host_key = paramiko.RSAKey.generate(2048)
            c = Client(binary, root, env, args.language, ["alias-target"])
            clients.append(c)
            c.wait("REMOTE HOST IDENTIFICATION HAS CHANGED")
            assert "FIXTURE>" not in c.text
            c.stop()
            passed("changed SSH host key is refused")

            time.sleep(0.3)
            processes = [row[2] for row in process_table() if len(row) == 3]
            assert not any(str(root / "ssh_config") in line for line in processes)
            proxy_urls = [p.url for p in (proxy, auth, reject, ipv6)]
            assert not any("__proxy-connect" in line and str(binary) in line
                           and any(url in line for url in proxy_urls) for line in processes)
            passed("no remaining SSH or proxy helper processes")
        finally:
            for c in clients:
                if c.child.isalive():
                    c.child.terminate(force=True)
            try:
                ssh.close()
            finally:
                for p in [proxy, auth, reject, ipv6]:
                    p.close()

    (args.output / "report.json").write_text(json.dumps({"status": "PASS", "platform": platform.system(), "architecture": platform.machine(), "language": args.language, "scenarios": report}, ensure_ascii=False, indent=2) + "\n")
    print(f"{len(report)} scenarios passed; artifacts: {args.output}", flush=True)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Linux clipboard/upload acceptance using isolated SSH and deterministic helpers.

Use --clipboard x11 or wayland with an isolated display server for native tests.
No personal SSH configuration, credentials or desktop clipboard is touched in
mock mode. Native mode requires the caller to provide an isolated display.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import tempfile
import time

import paramiko
from e2e import Client, SSHFixture, ProxyFixture

PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=")

JPEG = base64.b64decode('/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAABAAEDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDxGiiitjI//9k=')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/release/ssh-tui"))
    parser.add_argument("--language", choices=("en", "zh"), default="en")
    parser.add_argument("--clipboard", choices=("mock", "x11", "wayland"), default="mock")
    parser.add_argument("--codex", type=Path, help="Optional installed Codex CLI; attachment test uses an isolated home and dummy loopback provider")
    parser.add_argument("--output", type=Path, default=Path("target/e2e-images"))
    args = parser.parse_args()
    binary = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    report = []
    t = lambda en, zh: en if args.language == "en" else zh
    def passed(name):
        report.append(name)
        print("PASS", name, flush=True)

    with tempfile.TemporaryDirectory(prefix="ssh-tui-images-test-") as tmp:
        root = Path(tmp)
        key = paramiko.RSAKey.generate(2048)
        key.write_private_key_file(str(root / "identity"))
        (root / "identity").chmod(0o600)
        ssh = SSHFixture(root, key)
        proxy = ProxyFixture(auth=True)
        clipboard_owner = None
        client = None
        uploaded = []
        try:
            (root / "known_hosts").write_text(f"[127.0.0.1]:{ssh.port} {ssh.host_key.get_name()} {ssh.host_key.get_base64()}\n")
            # Force password auth: successful uploads must not authenticate again.
            (root / "ssh_config").write_text(f"Host target\n HostName 127.0.0.1\n Port {ssh.port}\n User fixture\nHost *\n PubkeyAuthentication no\n UserKnownHostsFile {root}/known_hosts\n GlobalKnownHostsFile /dev/null\n")
            bindir = root / "bin"
            bindir.mkdir()
            real_ssh = shutil.which("ssh")
            (bindir / "ssh").write_text(f'#!/bin/sh\nexec {shlex.quote(real_ssh)} -F {shlex.quote(str(root / "ssh_config"))} "$@"\n')
            (bindir / "ssh").chmod(0o700)
            (root / "image.png").write_bytes(PNG)
            (root / "image.jpg").write_bytes(JPEG)
            (root / "mode").write_text("image")
            if args.clipboard == "mock":
                helper = f'''#!/usr/bin/env python3
import pathlib,sys,time
root=pathlib.Path({str(root)!r})
mode=(root/'mode').read_text()
(root/'helper-used').write_text(pathlib.Path(sys.argv[0]).name)
if mode == 'timeout': time.sleep(10)
if mode == 'error': sys.stderr.write('fixture clipboard unreachable'); sys.exit(1)
if 'TARGETS' in sys.argv or '--list-types' in sys.argv:
 print('text/plain' if mode == 'text' else 'image/jpeg' if mode == 'jpeg' else 'image/png')
else:
 sys.stdout.buffer.write(b'x'*(20*1024*1024+1) if mode == 'large' else b'invalid' if mode == 'invalid' else (root/('image.jpg' if mode == 'jpeg' else 'image.png')).read_bytes())
'''
                for name in ("xclip", "wl-paste"):
                    (bindir / name).write_text(helper)
                    (bindir / name).chmod(0o700)
            env = dict(os.environ, HOME=str(root), XDG_CONFIG_HOME=str(root), PATH=f"{bindir}:{os.environ['PATH']}", TERM="xterm-256color", LC_ALL="C.UTF-8")
            if args.clipboard == "mock":
                env.update(XDG_SESSION_TYPE="x11", DISPLAY=":fixture")
                env.pop("WAYLAND_DISPLAY", None)
            else:
                env["XDG_SESSION_TYPE"] = args.clipboard
                if args.clipboard == "x11":
                    clipboard_owner = subprocess.Popen(["xclip", "-selection", "clipboard", "-in", "-target", "image/png", "-quiet"], stdin=subprocess.PIPE, start_new_session=True)
                else:
                    clipboard_owner = subprocess.Popen(["wl-copy", "--foreground", "--type", "image/png"], stdin=subprocess.PIPE, start_new_session=True)
                clipboard_owner.stdin.write(PNG)
                clipboard_owner.stdin.close()
                time.sleep(.3)
            (root / "config.toml").write_text(f'''[[connections]]
name = "images"
host = "target"
route = {{ mode = "http", name = "auth" }}
[[proxies]]
name = "auth"
url = "{proxy.url}"
username = "fixture-proxy-user"
''')
            client = Client(binary, root, env, args.language, extra=("target", "--proxy", proxy.url, "--proxy-user", "fixture-proxy-user"))
            client.wait(t("HTTP proxy password", "HTTP 代理密码"))
            client.send("fixture-proxy-password\r")
            client.wait("password:")
            client.send("fixture-ssh-password\r")
            client.wait("FIXTURE>")
            client.wait(t("Images OFF", "图片关闭"))
            transports = len(ssh.transports)
            requests = len(proxy.requests)
            client.prefix("v")
            client.wait(t("Images ON", "图片开启"))
            client.send("\x16")
            client.wait("/tmp/ssh-tui-image.")
            deadline = time.monotonic() + 5
            while not ssh.exec_results and time.monotonic() < deadline:
                client.pump()
            payload, out, rc = ssh.exec_results[-1]
            assert rc == 0 and payload == PNG, (rc, len(payload))
            path = Path(re.search(rb'SSH_TUI_IMAGE:(/tmp/ssh-tui-image\.[A-Za-z0-9]{10}/image.png)', out)[1].decode())
            uploaded.append(path.parent)
            assert path.read_bytes() == PNG
            assert path.stat().st_mode & 0o777 == 0o600
            assert path.parent.stat().st_mode & 0o777 == 0o700
            assert len(ssh.transports) == transports and len(proxy.requests) == requests
            client.command("\x15printf 'AFTER_IMAGE\\n'")
            client.wait_line("AFTER_IMAGE")
            passed("image bytes and private permissions; password and authenticated HTTP route reused; no auto Enter")
            if args.clipboard == "mock":
                for mode, expected in [("invalid", t("not valid", "不是有效")), ("error", t("Cannot read", "读取本机")), ("large", t("exceeds", "超限")), ("timeout", t("timed out", "超时"))]:
                    (root / "mode").write_text(mode)
                    client.send("\x16")
                    client.wait(expected)
                    assert len(ssh.exec_results) == 1
                passed("invalid image, clipboard failures, oversize and timeout do not upload or forward Ctrl+V")
                (root / "mode").write_text("timeout")
                client.send("\x16")
                client.send("\x1b")
                client.wait(t("cancelled", "已取消"))
                passed("Esc cancels a running clipboard helper")
                (root / "mode").write_text("text")
                client.command("python3 -c 'import os,sys,tty,termios; old=termios.tcgetattr(0); tty.setraw(0); os.write(1,b\"IMAGE_KEY_READY\\r\\n\"); b=sys.stdin.buffer.read(1); termios.tcsetattr(0,termios.TCSANOW,old); print(\"IMAGE_KEY=\"+b.hex())'")
                client.wait_line("IMAGE_KEY_READY")
                client.send("\x16")
                client.wait_line("IMAGE_KEY=16")
                passed("non-image clipboard forwards exactly one original Ctrl+V")
                (root / "mode").write_text("image")
                ssh.reject_exec = True
                client.send("\x16")
                client.wait(t("Image upload failed", "图片上传失败"))
                ssh.reject_exec = False
                assert len(ssh.transports) == transports and len(proxy.requests) == requests
                assert len(ssh.exec_results) == 1
                passed("server rejects upload channel without reauthentication or direct fallback")
                (root / "mode").write_text("jpeg")
                client.send("\x16")
                client.wait("/image.jpg")
                payload, out, rc = ssh.exec_results[-1]
                jpeg_path = Path(re.search(rb'SSH_TUI_IMAGE:(/tmp/ssh-tui-image\.[A-Za-z0-9]{10}/image.jpg)', out)[1].decode())
                uploaded.append(jpeg_path.parent)
                assert rc == 0 and payload == JPEG and jpeg_path.read_bytes() == JPEG
                client.send("\x15")
                passed("JPEG clipboard data retains its bytes and jpg extension")
                # A pending result must not be inserted into the manager view.
                (root / "mode").write_text("image")
                client.child.send("\x16\x1dc")
                client.wait(t("Connections", "连接"))
                client.pump(1)
                assert len(ssh.exec_results) == 3
                out = ssh.exec_results[-1][1]
                path2 = Path(re.search(rb'SSH_TUI_IMAGE:(/tmp/ssh-tui-image\.[A-Za-z0-9]{10}/image.png)', out)[1].decode())
                uploaded.append(path2.parent)
                # Open a second authenticated tab while the first result waits.
                client.send("\r")
                client.wait(t("HTTP proxy password", "HTTP 代理密码"))
                client.send("fixture-proxy-password\r")
                client.wait("password:")
                client.send("fixture-ssh-password\r")
                client.wait("FIXTURE>")
                client.wait(t("Images OFF", "图片关闭"))
                assert str(path2) not in client.text
                client.prefix("p")
                client.wait(str(path2))
                client.send("\x15")
                passed("completed image stays with its original tab while another authenticated tab is active")
            client.prefix("v")
            client.wait(t("Images OFF", "图片关闭"))
            if args.clipboard == "mock":
                (root / "mode").write_text("timeout")
                client.prefix("v")
                client.send("\x16")
                client.prefix("x")
                client.send("y")
                client.wait("FIXTURE>")
                client.wait(t("Images OFF", "图片关闭"))
                passed("closing a busy tab cancels its clipboard job and preserves the other tab")
                (root / "mode").write_text("image")
            client.stop()
            client = None
            passed("mode toggles off and shutdown restores terminal")
            if args.clipboard == "mock":
                # Select Wayland explicitly, even with DISPLAY inherited.
                env.update(XDG_SESSION_TYPE="wayland", WAYLAND_DISPLAY="fixture-wayland")
                client = Client(binary, root, env, args.language, extra=("target", "--no-proxy"))
                client.wait("password:")
                client.send("fixture-ssh-password\r")
                client.wait("FIXTURE>")
                client.wait(t("Images OFF", "图片关闭"))
                client.prefix("v")
                client.send("\x16")
                client.wait("/tmp/ssh-tui-image.")
                assert (root / "helper-used").read_text() == "wl-paste"
                out = ssh.exec_results[-1][1]
                uploaded.append(Path(re.search(rb'SSH_TUI_IMAGE:(/tmp/ssh-tui-image\.[A-Za-z0-9]{10}/image.png)', out)[1].decode()).parent)
                client.send("\x15")
                # A remote disconnect resets the mode; reconnect must get a new socket.
                client.command("exit")
                client.wait(t("Disconnected", "已断开"))
                client.send("r")
                client.wait("password:")
                client.send("fixture-ssh-password\r")
                client.wait("FIXTURE>")
                client.wait(t("Images OFF", "图片关闭"))
                client.stop()
                client = None
                passed("Wayland backend selection, new-session default off and reconnect reset")
            if args.codex:
                codex_home = root / "codex-home"
                codex_home.mkdir()
                (codex_home / "config.toml").write_text(f'''model = "fixture-model"
model_provider = "fixture"
[model_providers.fixture]
name = "fixture"
base_url = "http://127.0.0.1:9/v1"
env_key = "SSH_TUI_TEST_API_KEY"
wire_api = "responses"
[projects."{root}"]
trust_level = "trusted"
''')
                client = Client(binary, root, env, args.language, extra=("target", "--no-proxy"))
                client.wait("password:")
                client.send("fixture-ssh-password\r")
                client.wait("FIXTURE>")
                cmd = ["env", f"CODEX_HOME={codex_home}", "SSH_TUI_TEST_API_KEY=fixture", "DISPLAY=:unreachable", "WAYLAND_DISPLAY=", str(args.codex.resolve()), "--no-daemon", "--no-alt-screen", "-C", str(root)]
                client.command(shlex.join(cmd))
                client.wait("fixture-model", timeout=20)
                client.prefix("v")
                client.send("\x16")
                client.wait("[Image #1]", timeout=20)
                (args.output / "codex-attachment.txt").write_text(client.text)
                out = ssh.exec_results[-1][1]
                uploaded.append(Path(re.search(rb'SSH_TUI_IMAGE:(/tmp/ssh-tui-image\.[A-Za-z0-9]{10}/image.png)', out)[1].decode()).parent)
                client.send("\x03\x03")
                client.wait("FIXTURE>")
                client.stop()
                client = None
                passed("actual Codex CLI recognizes uploaded path as [Image #1] without remote clipboard; no prompt submitted")
            (args.output / "failure.txt").unlink(missing_ok=True)
            (args.output / "report.json").write_text(json.dumps({"language": args.language, "clipboard": args.clipboard, "passed": report}, indent=2) + "\n")
        finally:
            if client is not None:
                (args.output / "failure.txt").write_text(client.text)
                client.kill()
            if clipboard_owner is not None:
                os.killpg(clipboard_owner.pid, signal.SIGTERM)
                clipboard_owner.wait()
            proxy.close()
            ssh.close()
            for directory in uploaded:
                shutil.rmtree(directory, ignore_errors=True)


if __name__ == "__main__":
    main()

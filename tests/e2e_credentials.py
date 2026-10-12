"""Native credential acceptance with loopback SSH and a disposable system vault."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import platform
import select
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid

from e2e import Client, SSHFixture, ProxyFixture, SSHServer, process_table, paramiko


class JumpFixture(SSHFixture):
    """Public-key-only jump host; forwards one loopback direct-tcpip channel."""
    def serve(self, conn):
        transport = paramiko.Transport(conn)
        self.transports.append(transport)
        owner = self

        class Server(paramiko.ServerInterface):
            destination = None

            def get_allowed_auths(self, username):
                return "publickey"

            def check_auth_publickey(self, username, key):
                return paramiko.AUTH_SUCCESSFUL if key == owner.client_key else paramiko.AUTH_FAILED

            def check_channel_direct_tcpip_request(self, chanid, origin, destination):
                if destination[0] != "127.0.0.1":
                    return paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
                self.destination = destination
                return paramiko.OPEN_SUCCEEDED

        remote = None
        try:
            server = Server()
            transport.add_server_key(self.host_key)
            transport.start_server(server=server)
            channel = transport.accept(15)
            if channel is None:
                return
            remote = socket.create_connection(server.destination, timeout=5)
            while transport.is_active():
                for source in select.select([channel, remote], [], [], 0.1)[0]:
                    data = source.recv(8192)
                    if not data:
                        return
                    (remote if source is channel else channel).sendall(data)
        except (EOFError, OSError, paramiko.SSHException):
            pass
        finally:
            transport.close()
            if remote:
                remote.close()


@contextlib.contextmanager
def isolated_vault(root):
    env = dict(os.environ)
    if platform.system() == "Linux":
        env.update(HOME=str(root), XDG_DATA_HOME=str(root / "data"),
                   XDG_CONFIG_HOME=str(root), XDG_RUNTIME_DIR=str(root / "runtime"))
        (root / "runtime").mkdir(mode=0o700)
        control = root / "keyring-control"
        control.mkdir(mode=0o700)
        log = (root / "keyring.log").open("w")
        daemon = subprocess.Popen(
            ["gnome-keyring-daemon", "--foreground", "--unlock", "--components=secrets",
             f"--control-directory={control}"],
            env=env, stdin=subprocess.PIPE, stdout=log, stderr=log,
        )
        daemon.stdin.write(b"fixture-vault-password")
        daemon.stdin.close()
        try:
            # Wait for ownership of the Secret Service name on this private bus.
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                result = subprocess.run(
                    ["dbus-send", "--session", "--print-reply", "--dest=org.freedesktop.DBus",
                     "/org/freedesktop/DBus", "org.freedesktop.DBus.NameHasOwner",
                     "string:org.freedesktop.secrets"], env=env, capture_output=True, text=True,
                )
                if "boolean true" in result.stdout:
                    break
                time.sleep(0.05)
            else:
                raise AssertionError("isolated Secret Service did not start")
            yield env
        finally:
            daemon.terminate()
            try:
                daemon.wait(timeout=5)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait(timeout=5)
            log.close()
    else:
        def security(*args, check=True):
            return subprocess.run(["security", *args], check=check, capture_output=True, text=True)
        original = shlex.split(security("default-keychain", "-d", "user").stdout)[0]
        chain = str(root / "fixture.keychain-db")
        security("create-keychain", "-p", "fixture-vault-password", chain)
        try:
            security("set-keychain-settings", chain)
            security("unlock-keychain", "-p", "fixture-vault-password", chain)
            security("default-keychain", "-d", "user", "-s", chain)
            yield env
        finally:
            security("default-keychain", "-d", "user", "-s", original)
            security("delete-keychain", chain)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/release/ssh-tui"))
    parser.add_argument("--language", choices=("en", "zh"), default="en")
    parser.add_argument("--output", type=Path, default=Path("target/e2e-credentials"))
    parser.add_argument("--isolated-bus", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--native-test", type=Path, help="Optional compiled Rust native-vault test executable")
    args = parser.parse_args()
    if platform.system() == "Linux" and not args.isolated_bus:
        assert shutil.which("dbus-run-session") and shutil.which("gnome-keyring-daemon")
        os.execvp("dbus-run-session", ["dbus-run-session", "--", sys.executable,
                  str(Path(__file__).resolve()), *sys.argv[1:], "--isolated-bus"])
    binary = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    report = []

    def t(en, zh):
        return en if args.language == "en" else zh

    def passed(name):
        report.append(name)
        print("PASS", name, flush=True)

    with tempfile.TemporaryDirectory(prefix="ssh-tui-credentials-") as tmp:
        root = Path(tmp).resolve()
        with isolated_vault(root) as vault_env:
            if args.native_test:
                subprocess.run([str(args.native_test.resolve()), "--ignored"], env=vault_env, check=True)
            key = paramiko.RSAKey.generate(2048)
            ssh = SSHFixture(root, key)
            jump = JumpFixture(root, key)
            proxy = ProxyFixture()
            bindir = root / "bin"
            bindir.mkdir()
            ssh_config = root / "ssh_config"
            ssh_config.write_text(
                f"Host saved-target\n HostName 127.0.0.1\n Port {ssh.port}\n User fixture\n"
                f"Host *\n UserKnownHostsFile {root}/known_hosts\n GlobalKnownHostsFile /dev/null\n"
                " StrictHostKeyChecking ask\n PubkeyAuthentication no\n PreferredAuthentications password\n"
            )
            ssh_exe = shutil.which("ssh")
            (bindir / "ssh").write_text(f"#!/bin/sh\nexec {shlex.quote(ssh_exe)} -F {shlex.quote(str(ssh_config))} \"$@\"\n")
            (bindir / "ssh").chmod(0o700)
            env = dict(vault_env, HOME=str(root), XDG_CONFIG_HOME=str(root),
                       PATH=f"{bindir}:{os.environ['PATH']}", TERM="xterm-256color")
            (root / "config.toml").write_text(f'[[proxies]]\nname = "local"\nurl = "{proxy.url}"\n')
            clients = []

            def start(client_env=None):
                c = Client(binary, root, client_env or env, args.language)
                clients.append(c)
                c.wait(t("Connections", "连接列表"))
                return c

            def edit_password(c, password, route=None, enable=None, name=None):
                c.send("e")
                if name is not None:
                    c.send("\x15" + name)
                c.send("\t" * 6)
                if route is not None:
                    c.send("\x15" + route)
                c.send("\t\x15" + password + "\t")
                if enable is not None:
                    c.send(" ")
                c.save("\r", lambda cfg: bool(cfg["connections"][0].get("credential_id")) if enable is not False
                       else "credential_id" not in cfg["connections"][0])

            def no_leaks(c):
                c.pump()
                assert "fixture-ssh-password" not in c.raw
                assert "wrong-saved-password" not in c.raw
                config = (root / "config.toml").read_text()
                assert "fixture-ssh-password" not in config and "wrong-saved-password" not in config
                assert (root / "config.toml").stat().st_mode & 0o777 == 0o600
                for path in root.rglob("askpass"):
                    assert "fixture-ssh-password" not in path.read_text()
                rows = process_table()
                descendants = {c.child.pid}
                for _ in range(len(rows)):
                    descendants.update(int(pid) for pid, parent, *_ in rows if int(parent) in descendants)
                for pid, _, *command in rows:
                    if int(pid) not in descendants:
                        continue
                    assert "fixture-ssh-password" not in " ".join(command)
                    if platform.system() == "Linux":
                        try:
                            variables = Path(f"/proc/{pid}/environ").read_bytes()
                        except FileNotFoundError:
                            continue
                        assert b"fixture-ssh-password" not in variables
                        for variable in variables.split(b"\0"):
                            if variable.startswith(b"SSH_ASKPASS="):
                                helper = Path(os.fsdecode(variable.split(b"=", 1)[1]))
                                if helper.is_file():
                                    assert "fixture-ssh-password" not in helper.read_text()
                                    assert helper.stat().st_mode & 0o777 == 0o700

            try:
                c = start()
                c.send("a")
                fields = ["saved-ui", "saved-target", "fixture", "", "", "", "direct", "fixture-ssh-password"]
                for field in fields:
                    c.send("\x15" + field + "\t")
                c.send(" ")
                c.save("\r", lambda cfg: bool(cfg["connections"][0].get("credential_id")))
                no_leaks(c)
                c.stop()
                c = start()
                c.send("\r")
                c.wait("Are you sure")
                assert "FIXTURE>" not in c.text
                c.command("yes")
                c.wait("FIXTURE>")
                no_leaks(c)
                c.command("exit")
                c.wait(t("Disconnected", "已断开"))
                c.send("r")
                c.wait("FIXTURE>")
                no_leaks(c)
                c.stop()
                passed("saved username/password survives restart; fingerprint confirmation and reconnect")

                c = start()
                old = tomllib.loads((root / "config.toml").read_text())["connections"][0]["credential_id"]
                edit_password(c, "", name="renamed-ui")
                assert tomllib.loads((root / "config.toml").read_text())["connections"][0]["credential_id"] == old
                edit_password(c, "fixture-ssh-password", route="local")
                c.send("\r")
                c.wait("FIXTURE>")
                assert proxy.requests
                no_leaks(c)
                c.stop()
                passed("rename keeps saved password; replacement and HTTP CONNECT automatic login")

                c = start()
                edit_password(c, "wrong-saved-password", route="direct")
                c.send("\r")
                c.wait(t("Saved password rejected", "保存的密码被拒绝"))
                c.command("fixture-ssh-password")
                c.wait("FIXTURE>")
                no_leaks(c)
                c.stop()
                passed("wrong saved password is tried once; manual input remains hidden")

                c = start()
                c.send("m")
                c.wait("password:")
                assert t("Saved password rejected", "保存的密码被拒绝") not in c.text
                c.command("fixture-ssh-password")
                c.wait("FIXTURE>")
                no_leaks(c)
                c.stop()
                passed("one-time manual login ignores saved password")

                if platform.system() == "Linux":
                    unavailable = dict(env, DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent/ssh-tui-test-bus")
                    c = start(unavailable)
                    c.send("\r")
                    c.wait(t("System keyring unavailable", "系统密钥环不可用"))
                    c.command("fixture-ssh-password")
                    c.wait("FIXTURE>")
                    no_leaks(c)
                    c.stop()
                    passed("unavailable keyring falls back to manual authentication")

                # A valid reference whose vault entry has been removed elsewhere.
                config_path = root / "config.toml"
                config = config_path.read_text()
                original_id = tomllib.loads(config)["connections"][0]["credential_id"]
                config_path.write_text(config.replace(original_id, str(uuid.uuid4())))
                c = start()
                c.send("\r")
                c.wait(t("Saved SSH password is missing", "保存的 SSH 密码不存在"))
                c.command("fixture-ssh-password")
                c.wait("FIXTURE>")
                no_leaks(c)
                c.stop()
                config_path.write_text(config)
                passed("missing vault entry falls back to hidden manual input")

                # An inherited custom route must never receive a saved target password.
                original_ssh = ssh_config.read_text()
                relay = bindir / "relay.py"
                relay.write_text(
                    "import os,select,socket,sys\n"
                    f"s=socket.create_connection(('127.0.0.1',{ssh.port}))\n"
                    "while True:\n"
                    " for source in select.select([s,sys.stdin.buffer],[],[])[0]:\n"
                    "  data=s.recv(8192) if source is s else os.read(0,8192)\n"
                    "  if not data: sys.exit(0)\n"
                    "  s.sendall(data) if source is not s else os.write(1,data)\n"
                )
                route = shlex.join([sys.executable, str(relay)])
                ssh_config.write_text(original_ssh.replace("Host saved-target\n", f"Host saved-target\n ProxyCommand {route}\n"))
                c = start()
                c.send("\r")
                c.wait(t("Saved passwords are not used", "SSH 跳板连接不使用保存的密码"))
                c.wait("password:")
                c.command("fixture-ssh-password")
                c.wait("FIXTURE>")
                no_leaks(c)
                c.stop()
                ssh_config.write_text(original_ssh)
                passed("inherited OpenSSH route uses manual authentication without target password")

                key.write_private_key_file(str(root / "jump-identity"))
                (root / "jump-identity").chmod(0o600)
                # The first Host block keeps the jump's key auth separate from
                # the final target's password auth settings.
                jump_config = (
                    f"Host 127.0.0.1\n PubkeyAuthentication yes\n PreferredAuthentications publickey\n"
                    f" IdentityFile {root}/jump-identity\n IdentitiesOnly yes\n"
                ) + original_ssh.replace("Host saved-target\n", f"Host saved-target\n ProxyJump fixture@127.0.0.1:{jump.port}\n")
                ssh_config.write_text(jump_config)
                c = start()
                c.send("\r")
                c.wait(t("Saved passwords are not used", "SSH 跳板连接不使用保存的密码"))
                c.wait("Are you sure")
                c.command("yes")
                c.wait("password:")
                c.command("fixture-ssh-password")
                c.wait("FIXTURE>")
                no_leaks(c)
                c.stop()
                ssh_config.write_text(original_ssh)
                passed("SSH ProxyJump uses key auth for jump and manual password for target")

                encrypted_key = root / "encrypted-identity"
                key.write_private_key_file(str(encrypted_key), password="fixture-key-passphrase")
                encrypted_key.chmod(0o600)
                ssh_config.write_text(original_ssh.replace("PubkeyAuthentication no", "PubkeyAuthentication yes")
                                      .replace("PreferredAuthentications password", "PreferredAuthentications publickey,password")
                                      + f" IdentityFile {encrypted_key}\n IdentitiesOnly yes\n")
                c = start()
                c.send("\r")
                c.wait("Enter passphrase")
                c.command("fixture-key-passphrase")
                c.wait("FIXTURE>")
                assert "fixture-key-passphrase" not in c.raw
                no_leaks(c)
                c.stop()
                ssh_config.write_text(original_ssh)
                passed("encrypted private-key passphrase stays manual and hidden")

                # Even a server-crafted challenge identical to the native password
                # prompt must not obtain the stored secret through keyboard-interactive.
                allowed = SSHServer.get_allowed_auths
                interactive = getattr(SSHServer, "check_auth_interactive", None)
                responses = getattr(SSHServer, "check_auth_interactive_response", None)
                challenge_responses = []

                def challenge(server, username, submethods):
                    return paramiko.InteractiveQuery("", "", ("fixture@127.0.0.1's password: ", False))

                def answer(server, values):
                    challenge_responses.extend(values)
                    return paramiko.AUTH_SUCCESSFUL if values == ["fixture-otp-code"] else paramiko.AUTH_FAILED

                try:
                    SSHServer.get_allowed_auths = lambda server, username: "keyboard-interactive"
                    SSHServer.check_auth_interactive = challenge
                    SSHServer.check_auth_interactive_response = answer
                    ssh_config.write_text(original_ssh.replace("PreferredAuthentications password", "PreferredAuthentications password,keyboard-interactive"))
                    c = start()
                    c.send("\r")
                    c.wait(t("Disconnected", "已断开"))
                    assert not challenge_responses
                    c.send("m")
                    c.wait("password:")
                    c.command("fixture-otp-code")
                    c.wait("FIXTURE>")
                    assert challenge_responses == ["fixture-otp-code"]
                    no_leaks(c)
                    c.stop()
                finally:
                    SSHServer.get_allowed_auths = allowed
                    SSHServer.check_auth_interactive = interactive
                    SSHServer.check_auth_interactive_response = responses
                    ssh_config.write_text(original_ssh)
                passed("MFA cannot receive saved password; one-time manual login answers challenge")

                c = start()
                edit_password(c, "fixture-ssh-password")
                ssh.host_key = paramiko.RSAKey.generate(2048)
                c.send("\r")
                c.wait("REMOTE HOST IDENTIFICATION HAS CHANGED")
                assert "FIXTURE>" not in c.text
                c.wait(t("Disconnected", "已断开"))
                no_leaks(c)
                c.prefix("c")
                edit_password(c, "", enable=False)
                c.save("dy", lambda cfg: not cfg["connections"])
                assert not tomllib.loads((root / "config.toml").read_text()).get("pending_credential_cleanup")
                c.stop()
                passed("changed host key refused; clear password and delete profile clean up vault")
            finally:
                for c in clients:
                    if c.child.isalive():
                        c.kill()
                ssh.close()
                jump.close()
                proxy.close()
    (args.output / "report.json").write_text(json.dumps({
        "status": "PASS", "platform": platform.system(), "language": args.language, "scenarios": report,
    }, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()

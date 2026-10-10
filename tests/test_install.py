"""Exercise installation with isolated platform/download fixtures and native CI packages."""

import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "scripts/install.sh"
SH = shutil.which("sh")
REPOSITORY = "https://github.com/ShaohongDong/ssh-tui"
PLATFORMS = (
    ("Linux", "x86_64", "x86_64-unknown-linux-gnu"),
    ("Darwin", "x86_64", "x86_64-apple-darwin"),
    ("Darwin", "arm64", "aarch64-apple-darwin"),
)


class InstallerFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ssh-tui-install-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.tools = self.root / "tools"
        self.assets = self.root / "assets"
        self.home = self.root / "home"
        self.downloads = self.root / "temporary downloads"
        for directory in (self.tools, self.assets, self.home, self.downloads):
            directory.mkdir()
        for name in ("awk", "chmod", "cp", "gzip", "mkdir", "mktemp", "mv",
                     "rm", "sed", "tar", "sha256sum", "shasum", "ssh"):
            source = shutil.which(name)
            if source:
                (self.tools / name).symlink_to(source)
        self.write_tool("uname", """import os, sys
print(os.environ['INSTALL_TEST_OS' if sys.argv[-1] == '-s' else 'INSTALL_TEST_ARCH'])
""")
        self.write_tool("curl", """import json, os, pathlib, shutil, sys, time
args = sys.argv[1:]
url = args[-1]
with open(os.environ['INSTALL_TEST_LOG'], 'a') as log:
    log.write(json.dumps(args) + '\\n')
if os.environ.get('INSTALL_TEST_PAUSE') and url.endswith('.tar.gz'):
    pathlib.Path(os.environ['INSTALL_TEST_PAUSE']).touch()
    time.sleep(60)
if os.environ.get('INSTALL_TEST_FAIL', '\\0') in url:
    print('fixture: download failed', file=sys.stderr)
    sys.exit(22)
if url == 'https://github.com/ShaohongDong/ssh-tui/releases/latest':
    sys.stdout.write(os.environ.get('INSTALL_TEST_REDIRECT',
        'https://github.com/ShaohongDong/ssh-tui/releases/tag/' + os.environ['INSTALL_TEST_TAG']))
    sys.exit(0)
prefix = 'https://github.com/ShaohongDong/ssh-tui/releases/download/' + os.environ['INSTALL_TEST_TAG'] + '/'
if not url.startswith(prefix):
    sys.exit(22)
asset = pathlib.Path(os.environ['INSTALL_TEST_ASSETS']) / url[len(prefix):]
if not asset.is_file():
    sys.exit(22)
shutil.copyfile(asset, args[args.index('--output') + 1])
""")
        self.log = self.root / "curl.jsonl"
        self.env = {
            "PATH": str(self.tools), "HOME": str(self.home),
            "TMPDIR": str(self.downloads), "LC_ALL": "C",
            "INSTALL_TEST_OS": "Linux", "INSTALL_TEST_ARCH": "x86_64",
            "INSTALL_TEST_TAG": "v0.1.0", "INSTALL_TEST_ASSETS": str(self.assets),
            "INSTALL_TEST_LOG": str(self.log),
        }
        self.bin_dir = self.home / ".local/bin"
        self.binary = self.bin_dir / "ssh-tui"
        self.make_archive()

    def write_tool(self, name, source):
        path = self.tools / name
        if path.is_symlink():
            path.unlink()
        path.write_text(f"#!{sys.executable}\n{source}")
        path.chmod(0o755)

    def make_archive(self, target=PLATFORMS[0][2], version="0.1.0", binary=None,
                     member_type=tarfile.REGTYPE, include_binary=True, duplicate=False):
        prefix = f"ssh-tui-{version}-{target}"
        archive = self.assets / f"{prefix}.tar.gz"
        if binary is None:
            binary = f"#!/bin/sh\nprintf '%s\\n' 'ssh-tui {version}'\n".encode()
        with tarfile.open(archive, "w:gz") as tar:
            if include_binary:
                for _ in range(2 if duplicate else 1):
                    entry = tarfile.TarInfo(f"{prefix}/ssh-tui")
                    entry.type = member_type
                    entry.mode = 0o755
                    if member_type == tarfile.REGTYPE:
                        entry.size = len(binary)
                        tar.addfile(entry, io.BytesIO(binary))
                    else:
                        entry.linkname = "../outside"
                        tar.addfile(entry)
            # These paths must never be extracted by the installer.
            for name in (f"{prefix}/README.md", "../escaped"):
                entry = tarfile.TarInfo(name)
                entry.size = 4
                tar.addfile(entry, io.BytesIO(b"docs"))
        self.write_checksums()
        return archive

    def write_checksums(self):
        self.sums = "".join(
            f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n"
            for path in sorted(self.assets.glob("*.tar.gz"))
        )
        (self.assets / "SHA256SUMS").write_text(self.sums)

    def run_installer(self, *args, env=None, piped=False):
        process_env = self.env | (env or {})
        command = [SH, "-s", "--", *args] if piped else [SH, str(INSTALLER), *args]
        result = subprocess.run(
            command, input=INSTALLER.read_text() if piped else None,
            env=process_env, cwd=self.root, capture_output=True, text=True, timeout=20,
        )
        self.assertEqual(list(self.downloads.iterdir()), [], result.stderr)
        self.assertEqual(list(self.root.glob("**/.ssh-tui-install.*")), [], result.stderr)
        self.assertFalse((self.root / "escaped").exists())
        return result

    def requests(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def seed_old_binary(self):
        self.bin_dir.mkdir(parents=True, exist_ok=True)
        self.binary.write_bytes(b"old installed binary")
        self.binary.chmod(0o755)

    def assert_failed_without_replacement(self, result, error):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(error, result.stderr)
        self.assertEqual(self.binary.read_bytes(), b"old installed binary")


class InstallTests(InstallerFixture):
    def test_default_install_via_pipe_and_https_downloads(self):
        for name in (".profile", ".bashrc", ".zshrc"):
            (self.home / name).write_text("existing shell configuration\n")
        ssh_config = self.home / ".ssh/config"
        ssh_config.parent.mkdir()
        ssh_config.write_text("Host personal\n")
        result = self.run_installer(piped=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.binary.stat().st_mode & 0o777, 0o755)
        self.assertIn("Installed ssh-tui 0.1.0", result.stdout)
        self.assertIn("export PATH=", result.stdout)
        self.assertEqual(subprocess.check_output([self.binary, "--version"], text=True).strip(), "ssh-tui 0.1.0")
        self.assertEqual([args[-1] for args in self.requests()], [
            f"{REPOSITORY}/releases/latest",
            f"{REPOSITORY}/releases/download/v0.1.0/ssh-tui-0.1.0-{PLATFORMS[0][2]}.tar.gz",
            f"{REPOSITORY}/releases/download/v0.1.0/SHA256SUMS",
        ])
        for args in self.requests():
            self.assertIn("--fail", args)
            self.assertEqual(args[args.index("--proto") + 1], "=https")
            self.assertEqual(args[args.index("--proto-redir") + 1], "=https")
        for name in (".profile", ".bashrc", ".zshrc"):
            self.assertEqual((self.home / name).read_text(), "existing shell configuration\n")
        self.assertEqual(ssh_config.read_text(), "Host personal\n")
        self.assertEqual(list(self.bin_dir.iterdir()), [self.binary])

    def test_platform_selection(self):
        for os_name, arch, target in PLATFORMS:
            with self.subTest(os=os_name, arch=arch):
                self.make_archive(target)
                result = self.run_installer("--version", "0.1.0", env={
                    "INSTALL_TEST_OS": os_name, "INSTALL_TEST_ARCH": arch,
                })
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"-{target}.tar.gz", self.requests()[-2][-1])

    def test_pinned_version_and_upgrade(self):
        for version in ("0.2.0", "v0.2.0-rc.1"):
            with self.subTest(version=version):
                release_version = version.removeprefix("v")
                self.make_archive(version=release_version)
                self.seed_old_binary()
                result = self.run_installer("--version", version, env={
                    "INSTALL_TEST_TAG": f"v{release_version}",
                })
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"Installed ssh-tui {release_version}", result.stdout)
                self.assertNotEqual(self.binary.read_bytes(), b"old installed binary")
        self.assertFalse(any(args[-1].endswith("/latest") for args in self.requests()))

    def test_custom_relative_directory_and_shell_safe_path_hint(self):
        directory = "bin with 'quotes' $cash `literal` \\backslash"
        result = self.run_installer("--install-dir", directory)
        self.assertEqual(result.returncode, 0, result.stderr)
        destination = self.root / directory
        self.assertTrue((destination / "ssh-tui").is_file())
        export = next(line.strip() for line in result.stdout.splitlines() if line.strip().startswith("export PATH="))
        path = subprocess.check_output([SH, "-c", export + '; printf "%s" "$PATH"'], env=self.env, text=True)
        self.assertEqual(path, f"{destination}:{self.tools}")
        run_command = next(line.removeprefix("Run: ") for line in result.stdout.splitlines() if line.startswith("Run: "))
        self.assertEqual(shlex.split(run_command), [str(destination / "ssh-tui")])

    def test_no_path_hint_when_install_directory_is_on_path(self):
        result = self.run_installer(env={"PATH": f"{self.tools}:{self.bin_dir}"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("export PATH=", result.stdout)

    def test_shasum_fallback(self):
        if not (self.tools / "shasum").exists():
            self.skipTest("shasum is unavailable on this host")
        (self.tools / "sha256sum").unlink(missing_ok=True)
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_help_needs_no_dependencies_or_home(self):
        result = self.run_installer("--help", env={"PATH": "", "HOME": ""})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--install-dir", result.stdout)
        self.assertEqual(self.requests(), [])

    def test_invalid_arguments_and_versions(self):
        for args in (("--unknown",), ("--version",), ("--install-dir",),
                     ("--install-dir", ""), ("--version", "next"),
                     ("--version", "v0.1.0/other"), ("--version", "1.2.3\n4.5.6")):
            with self.subTest(args=args):
                result = self.run_installer(*args)
                self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.requests(), [])

    def test_home_unset_requires_explicit_directory(self):
        result = self.run_installer(env={"HOME": ""})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("HOME is unset", result.stderr)
        result = self.run_installer("--install-dir", str(self.bin_dir), env={"HOME": ""})
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_unsupported_platforms(self):
        for os_name, arch in (("Linux", "aarch64"), ("Linux", "i686"), ("FreeBSD", "x86_64")):
            with self.subTest(os=os_name, arch=arch):
                result = self.run_installer(env={"INSTALL_TEST_OS": os_name, "INSTALL_TEST_ARCH": arch})
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("unsupported platform", result.stderr)
        self.assertEqual(self.requests(), [])

    def test_missing_dependencies(self):
        for name in ("curl", "ssh"):
            with self.subTest(tool=name):
                path = self.tools / name
                hidden = self.tools / f"hidden-{name}"
                path.rename(hidden)
                try:
                    result = self.run_installer()
                    self.assertIn(f"missing required command: {name}", result.stderr)
                    self.assertNotEqual(result.returncode, 0)
                finally:
                    hidden.rename(path)
        for name in ("sha256sum", "shasum"):
            (self.tools / name).unlink(missing_ok=True)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing SHA-256 tool", result.stderr)
        self.assertEqual(self.requests(), [])

    def test_unexpected_latest_redirects(self):
        for url in (f"{REPOSITORY}/releases", "https://example.org/releases/tag/v0.1.0",
                    f"{REPOSITORY}/releases/tag/v0.1.0/1.2.3"):
            with self.subTest(url=url):
                result = self.run_installer(env={"INSTALL_TEST_REDIRECT": url})
                self.assertNotEqual(result.returncode, 0)
        self.assertTrue(all(args[-1].endswith("/latest") for args in self.requests()))

    def test_download_failures_preserve_old_installation(self):
        self.seed_old_binary()
        for phase in ("latest", ".tar.gz", "SHA256SUMS"):
            with self.subTest(phase=phase):
                result = self.run_installer(env={"INSTALL_TEST_FAIL": phase})
                self.assert_failed_without_replacement(result, "ssh-tui installer:")

    def test_checksum_failures_preserve_old_installation(self):
        self.seed_old_binary()
        archive_name = next(self.assets.glob("*.tar.gz")).name
        for contents, message in (
            ("", "missing, duplicate, or invalid checksum"),
            (self.sums * 2, "missing, duplicate, or invalid checksum"),
            (f"invalid  {archive_name}\n", "missing, duplicate, or invalid checksum"),
            (f"{'0' * 64}  {archive_name}\n", "SHA-256 mismatch"),
        ):
            with self.subTest(contents=contents):
                (self.assets / "SHA256SUMS").write_text(contents)
                self.assert_failed_without_replacement(self.run_installer(), message)

    def test_archive_failures_preserve_old_installation(self):
        self.seed_old_binary()
        archive = self.make_archive()
        archive.write_bytes(b"not a gzip archive")
        self.write_checksums()
        self.assert_failed_without_replacement(self.run_installer(), "invalid release archive")
        for options in ({"include_binary": False}, {"duplicate": True}):
            with self.subTest(options=options):
                self.make_archive(**options)
                self.assert_failed_without_replacement(self.run_installer(), "exactly one expected")

    def test_binary_execution_and_version_failures_preserve_old_installation(self):
        self.seed_old_binary()
        for binary, message in (
            (b"#!/bin/sh\nexit 1\n", "downloaded binary cannot run"),
            (b"#!/bin/sh\nprintf 'ssh-tui 9.9.9\\n'\n", "unexpected binary version"),
        ):
            with self.subTest(binary=binary):
                self.make_archive(binary=binary)
                self.assert_failed_without_replacement(self.run_installer(), message)
        self.make_archive(member_type=tarfile.SYMTYPE)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.binary.read_bytes(), b"old installed binary")

    def test_destination_directory_is_not_overwritten(self):
        self.binary.mkdir(parents=True)
        marker = self.binary / "keep"
        marker.write_text("preserve")
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("destination is a directory", result.stderr)
        self.assertEqual(marker.read_text(), "preserve")

    def test_existing_symlink_is_replaced_without_changing_its_target(self):
        outside = self.root / "outside"
        outside.write_bytes(b"keep external binary")
        self.bin_dir.mkdir(parents=True)
        self.binary.symlink_to(outside)
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.binary.is_symlink())
        self.assertEqual(outside.read_bytes(), b"keep external binary")

    def test_failed_atomic_replacement_preserves_old_installation(self):
        self.seed_old_binary()
        self.write_tool("mv", "import sys\nsys.exit(1)\n")
        self.assert_failed_without_replacement(self.run_installer(), "could not replace")

    def test_signal_cleans_download_directory(self):
        self.seed_old_binary()
        marker = self.root / "paused"
        process = subprocess.Popen(
            [SH, str(INSTALLER)], env=self.env | {"INSTALL_TEST_PAUSE": str(marker)},
            cwd=self.root, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, start_new_session=True,
        )
        try:
            deadline = time.monotonic() + 10
            while not marker.exists() and process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertTrue(marker.exists(), "installer did not begin download")
            os.killpg(process.pid, signal.SIGTERM)
            process.communicate(timeout=10)
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(self.binary.read_bytes(), b"old installed binary")
            self.assertEqual(list(self.downloads.iterdir()), [])
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()


@unittest.skipUnless(os.environ.get("SSH_TUI_INSTALL_TEST_PACKAGES"), "native package directory not supplied")
class NativeInstallTests(InstallerFixture):
    def test_native_package_install_and_reinstall(self):
        # Use actual native package bytes; only the network transport is replaced.
        directory = Path(os.environ["SSH_TUI_INSTALL_TEST_PACKAGES"]).resolve()
        packages = list(directory.glob("*.tar.gz"))
        self.assertEqual(len(packages), 1, "supply the current runner's native package only")
        archive = packages[0]
        match = re.fullmatch(r"ssh-tui-(.+)-(x86_64-unknown-linux-gnu|x86_64-apple-darwin|aarch64-apple-darwin)\.tar\.gz", archive.name)
        self.assertIsNotNone(match, archive.name)
        version, target = match.groups()
        for path in self.assets.iterdir():
            path.unlink()
        shutil.copyfile(archive, self.assets / archive.name)
        (self.assets / "SHA256SUMS").write_text(archive.with_name(archive.name + ".sha256").read_text())
        self.env.update({
            "INSTALL_TEST_OS": platform.system(), "INSTALL_TEST_ARCH": platform.machine(),
            "INSTALL_TEST_TAG": f"v{version}",
        })
        for args in ((), ("--version", version)):
            result = self.run_installer(*args)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(subprocess.check_output([self.binary, "--version"], text=True).strip(), f"ssh-tui {version}")
        help_text = subprocess.check_output([self.binary, "--help"], text=True)
        self.assertIn("--proxy", help_text)
        with tarfile.open(archive) as tar:
            original = tar.extractfile(f"ssh-tui-{version}-{target}/ssh-tui").read()
        self.assertEqual(self.binary.read_bytes(), original)
        self.assertEqual(self.binary.stat().st_mode & 0o777, 0o755)


if __name__ == "__main__":
    unittest.main()

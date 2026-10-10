"""Release gates: architecture, exact version tags, archives and checksum integrity."""

import importlib.util
import io
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location(
    "package_release", Path(__file__).resolve().parents[1] / "scripts/package_release.py"
)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


def header(target):
    if target == "x86_64-unknown-linux-gnu":
        value = bytearray(32)
        value[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", value, 18, 62)
        return bytes(value)
    return b"\xcf\xfa\xed\xfe" + struct.pack(
        "<I", 0x01000007 if target == "x86_64-apple-darwin" else 0x0100000C
    ) + bytes(24)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name, text in {
            "Cargo.toml": "[package]\nversion = '0.1.0'\n",
            "README.md": "Release instructions\n",
            "LICENSE": "MIT\n",
            "examples/config.toml": "language = 'en'\n",
            "examples/themes/amber.toml": "name = 'Amber'\n",
            "docs/ui-design.md": "UI documentation\n",
        }.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        self.output = self.root / "dist"
        # These tests inspect three target formats on any host. The workflow
        # separately runs every actual native binary before and after packaging.
        self.smoke = mock.patch.object(release, "smoke")
        self.smoke.start()
        self.addCleanup(self.smoke.stop)

    def package(self, target, tag="v0.1.0"):
        binary = self.root / f"binary-{target}"
        binary.write_bytes(header(target))
        binary.chmod(0o755)
        return release.package(binary, target, self.output, tag, root=self.root)

    def all_packages(self):
        return [self.package(target) for target in release.TARGETS]

    def test_complete_release_preserves_permissions_and_checksums(self):
        packages = self.all_packages()
        sums = release.collect(self.output, "v0.1.0", root=self.root)
        self.assertEqual(len(sums.read_text().splitlines()), 3)
        for archive, target in zip(packages, release.TARGETS):
            self.assertIn(f"{release.digest(archive)}  {archive.name}\n", sums.read_text())
            with tarfile.open(archive) as tar:
                prefix = release.basename("0.1.0", target)
                self.assertEqual(tar.getmember(f"{prefix}/ssh-tui").mode, 0o755)
                self.assertEqual(tar.getmember(f"{prefix}/README.md").mode, 0o644)
                self.assertIn(f"{prefix}/docs/ui-design.md", tar.getnames())

    def test_mismatched_tag_does_not_create_artifacts(self):
        with self.assertRaisesRegex(ValueError, "must match Cargo.toml"):
            self.package(release.TARGETS[0], "v9.9.9")
        self.assertFalse(self.output.exists())
        for tag in ("0.1.0", "v0.1.0/extra", "v0.1.0-rc.1"):
            with self.assertRaises(ValueError):
                release.check_tag(tag, "0.1.0")
        release.check_tag("v0.2.0-rc.1", "0.2.0-rc.1")

    def test_wrong_binary_architecture_is_rejected(self):
        binary = self.root / "wrong-binary"
        binary.write_bytes(header("x86_64-apple-darwin"))
        with self.assertRaisesRegex(ValueError, "architecture"):
            release.package(binary, "aarch64-apple-darwin", self.output, root=self.root)
        self.assertFalse(self.output.exists())

    def test_missing_platform_prevents_release(self):
        self.package(release.TARGETS[0])
        with self.assertRaises(FileNotFoundError):
            release.collect(self.output, "v0.1.0", root=self.root)
        self.assertFalse((self.output / "SHA256SUMS").exists())

    def test_corrupted_package_prevents_release(self):
        packages = self.all_packages()
        with packages[0].open("ab") as stream:
            stream.write(b"corruption")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            release.collect(self.output, "v0.1.0", root=self.root)
        self.assertFalse((self.output / "SHA256SUMS").exists())

    def test_unexpected_asset_prevents_release(self):
        self.all_packages()
        (self.output / "unexpected.tar.gz").write_bytes(b"extra")
        with self.assertRaisesRegex(ValueError, "unexpected release assets"):
            release.collect(self.output, "v0.1.0", root=self.root)

    def test_unsafe_archive_entries_are_rejected(self):
        archive = self.root / "unsafe.tar.gz"
        escaped = self.root.parent / f"{self.root.name}-escape"
        for name, kind in [(f"../{escaped.name}", tarfile.REGTYPE),
                           ("/absolute", tarfile.REGTYPE),
                           ("ssh-tui-0.1.0-x86_64-unknown-linux-gnu/link", tarfile.SYMTYPE)]:
            with self.subTest(name=name):
                with tarfile.open(archive, "w:gz") as tar:
                    entry = tarfile.TarInfo(name)
                    entry.type = kind
                    entry.linkname = "../outside"
                    tar.addfile(entry, io.BytesIO())
                with self.assertRaisesRegex(ValueError, "unsafe"):
                    release.verify_archive(archive, "0.1.0", release.TARGETS[0], run=True)
        self.assertFalse(escaped.exists())


if __name__ == "__main__":
    unittest.main()

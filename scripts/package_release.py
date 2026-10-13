#!/usr/bin/env python3
"""Package native release binaries and collect verified GitHub Release assets."""

import argparse
import gzip
import hashlib
import io
from pathlib import Path, PurePosixPath
import re
import struct
import subprocess
import tarfile
import tempfile
import tomllib


ROOT = Path(__file__).resolve().parents[1]
TARGETS = (
    "x86_64-unknown-linux-gnu",
)


def version(root=ROOT):
    value = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", value):
        raise ValueError(f"invalid package version: {value}")
    return value


def check_tag(tag, release_version):
    if tag is not None and tag != f"v{release_version}":
        raise ValueError(f"tag {tag!r} must match Cargo.toml: v{release_version}")


def basename(release_version, target):
    if target not in TARGETS:
        raise ValueError(f"unsupported target: {target}")
    return f"ssh-tui-{release_version}-{target}"


def check_binary_header(header, target):
    if target == "x86_64-unknown-linux-gnu":
        valid = (len(header) >= 20 and header[:6] == b"\x7fELF\x02\x01"
                 and struct.unpack_from("<H", header, 18)[0] == 62)
    else:
        raise ValueError(f"unsupported target: {target}")
    if not valid:
        raise ValueError(f"binary format/architecture does not match {target}")


def smoke(binary, release_version):
    for argument in ("--version", "--help"):
        result = subprocess.run(
            [str(binary), argument], capture_output=True, text=True,
            check=True, timeout=15,
        )
        if argument == "--version" and result.stdout.strip() != f"ssh-tui {release_version}":
            raise ValueError(f"unexpected binary version: {result.stdout.strip()}")
        if argument == "--help" and not all(
            option in result.stdout for option in ("--proxy", "--language", "--theme")
        ):
            raise ValueError("packaged binary is missing expected CLI options")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_archive(archive, release_version, target, run=False):
    prefix = basename(release_version, target)
    required = {f"{prefix}/{name}" for name in (
        "ssh-tui", "README.md", "LICENSE", "examples/config.toml",
        "examples/themes/amber.toml",
    )}
    names = set()
    with tempfile.TemporaryDirectory(prefix="ssh-tui-package-") as tmp:
        extracted = Path(tmp)
        with tarfile.open(archive, "r:gz") as tar:
            for member in tar:
                path = PurePosixPath(member.name)
                if (not member.isfile() or path.is_absolute() or ".." in path.parts
                        or path.as_posix() != member.name
                        or not path.parts or path.parts[0] != prefix
                        or member.name in names):
                    raise ValueError(f"unsafe or duplicate archive entry: {member.name}")
                names.add(member.name)
                source = tar.extractfile(member)
                if member.name == f"{prefix}/ssh-tui":
                    if member.mode != 0o755:
                        raise ValueError("packaged binary must have mode 0755")
                    check_binary_header(source.read(32), target)
                    source.seek(0)
                if run:
                    destination = extracted.joinpath(*path.parts)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with destination.open("wb") as out:
                        while block := source.read(1024 * 1024):
                            out.write(block)
                    destination.chmod(member.mode & 0o777)
        if not required.issubset(names):
            raise ValueError(f"archive is missing required files: {sorted(required - names)}")
        if run:
            smoke(extracted / prefix / "ssh-tui", release_version)


def package(binary, target, output, tag=None, root=ROOT):
    release_version = version(root)
    check_tag(tag, release_version)
    name = basename(release_version, target)
    binary = binary.resolve(strict=True)
    with binary.open("rb") as stream:
        check_binary_header(stream.read(32), target)
    smoke(binary, release_version)
    files = [(binary, "ssh-tui"), (root / "README.md", "README.md"),
             (root / "LICENSE", "LICENSE")]
    # Include documentation so README images and documentation links also work offline.
    for directory in ("examples", "docs"):
        files.extend((path, path.relative_to(root).as_posix())
                     for path in sorted((root / directory).rglob("*")) if path.is_file())
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{name}.tar.gz"
    with tempfile.TemporaryDirectory(prefix=".package-", dir=output) as tmp:
        pending = Path(tmp) / archive.name
        with pending.open("wb") as raw:
            with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w") as tar:
                    for source, relative in files:
                        if source.is_symlink():
                            raise ValueError(f"refusing symlink in package: {source}")
                        data = source.read_bytes()
                        entry = tarfile.TarInfo(f"{name}/{relative}")
                        entry.size = len(data)
                        entry.mode = 0o755 if relative == "ssh-tui" else 0o644
                        tar.addfile(entry, io.BytesIO(data))
        verify_archive(pending, release_version, target, run=True)
        pending.replace(archive)
    checksum = output / f"{archive.name}.sha256"
    checksum.write_text(f"{digest(archive)}  {archive.name}\n")
    print(f"Verified {archive.name} ({archive.stat().st_size} bytes)")
    return archive


def collect(directory, tag, root=ROOT):
    release_version = version(root)
    check_tag(tag, release_version)
    checksums = []
    expected = {"SHA256SUMS"}
    for target in TARGETS:
        archive = directory / f"{basename(release_version, target)}.tar.gz"
        checksum = directory / f"{archive.name}.sha256"
        expected.update((archive.name, checksum.name))
        if archive.is_symlink() or checksum.is_symlink():
            raise ValueError("release assets must be regular files")
        line = f"{digest(archive)}  {archive.name}\n"
        if checksum.read_text() != line:
            raise ValueError(f"checksum mismatch: {archive.name}")
        verify_archive(archive, release_version, target)
        checksums.append(line)
    unexpected = {path.name for path in directory.iterdir()} - expected
    if unexpected:
        raise ValueError(f"unexpected release assets: {sorted(unexpected)}")
    sums = directory / "SHA256SUMS"
    sums.write_text("".join(checksums))
    print(f"Verified all {len(TARGETS)} platform packages and wrote {sums.name}")
    return sums


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    pack = commands.add_parser("package", help="package and run a native binary")
    pack.add_argument("--binary", type=Path, required=True)
    pack.add_argument("--target", choices=TARGETS, required=True)
    pack.add_argument("--output", type=Path, default=Path("dist"))
    pack.add_argument("--tag", help="require an exact v<Cargo.toml version> tag")
    merge = commands.add_parser("collect", help="verify Linux release assets")
    merge.add_argument("--directory", type=Path, required=True)
    merge.add_argument("--tag", required=True)
    tag = commands.add_parser("check-tag", help="validate a release tag before building")
    tag.add_argument("--tag", required=True)
    args = parser.parse_args()
    try:
        if args.command == "package":
            package(args.binary, args.target, args.output, args.tag)
        elif args.command == "collect":
            collect(args.directory, args.tag)
        else:
            check_tag(args.tag, version())
    except (OSError, ValueError, tarfile.TarError, subprocess.SubprocessError) as error:
        parser.exit(1, f"release packaging: {error}\n")


if __name__ == "__main__":
    main()

#!/bin/sh
# Install a verified GitHub Release without requiring Rust or root privileges.
set -eu

repository=https://github.com/ShaohongDong/ssh-tui
version=
install_dir=
work_dir=
stage_dir=

usage() {
    printf '%s\n' 'Usage: install.sh [--version VERSION] [--install-dir DIR]

Install ssh-tui from GitHub Releases on Linux x64 or macOS Intel/Apple Silicon.

  --version VERSION  Install a specific version (with or without the v prefix).
                     Default: latest stable release.
  --install-dir DIR  Destination directory. Default: $HOME/.local/bin.
  -h, --help         Show this help.

Requires curl, tar, OpenSSH, and sha256sum or shasum. No sudo is invoked.
Run again to upgrade. Shell startup files and SSH configuration are untouched.'
}

fail() {
    printf 'ssh-tui installer: %s\n' "$*" >&2
    exit 1
}

cleanup() {
    if [ -n "$stage_dir" ]; then
        rm -rf "$stage_dir"
    fi
    if [ -n "$work_dir" ]; then
        rm -rf "$work_dir"
    fi
}

shell_quote() {
    printf "'"
    printf '%s' "$1" | sed "s/'/'\\\\''/g"
    printf "'"
}

download() {
    curl --fail --silent --show-error --location \
        --proto '=https' --proto-redir '=https' \
        --connect-timeout 10 --max-time 120 --output "$2" "$1"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] && [ -n "$2" ] || fail '--version requires a value'
            version=$2
            shift 2
            ;;
        --install-dir)
            [ "$#" -ge 2 ] && [ -n "$2" ] || fail '--install-dir requires a value'
            install_dir=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *) fail "unknown argument: $1 (see --help)" ;;
    esac
done

if [ -z "$install_dir" ]; then
    [ -n "${HOME:-}" ] || fail 'HOME is unset; specify --install-dir'
    install_dir=$HOME/.local/bin
fi
case "$install_dir" in
    /*) ;;
    *) install_dir=$PWD/$install_dir ;;
esac

for tool in curl tar uname mktemp awk chmod cp mkdir mv rm sed ssh; do
    command -v "$tool" >/dev/null 2>&1 || fail "missing required command: $tool"
done
if command -v sha256sum >/dev/null 2>&1; then
    checksum_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then
    checksum_tool=shasum
else
    fail 'missing SHA-256 tool: install sha256sum or shasum'
fi

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
    Linux/x86_64|Linux/amd64) target=x86_64-unknown-linux-gnu ;;
    Darwin/x86_64|Darwin/amd64) target=x86_64-apple-darwin ;;
    Darwin/arm64|Darwin/aarch64) target=aarch64-apple-darwin ;;
    *) fail "unsupported platform: $os/$arch; see README for source builds" ;;
esac

if [ -z "$version" ]; then
    latest=$(curl --fail --silent --show-error --location \
        --proto '=https' --proto-redir '=https' \
        --connect-timeout 10 --max-time 120 \
        --output /dev/null --write-out '%{url_effective}' \
        "$repository/releases/latest") || fail 'could not resolve the latest stable release'
    case "$latest" in
        "$repository/releases/tag/v"*) version=${latest#"$repository/releases/tag/"} ;;
        *) fail "unexpected latest release URL: $latest" ;;
    esac
fi
version=${version#v}
printf '%s\n' "$version" | awk '
    /^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/ { valid = 1 }
    END { exit !(NR == 1 && valid) }
' || fail "invalid version: $version"

trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
temp_base=${TMPDIR:-/tmp}
case "$temp_base" in
    /*) ;;
    *) temp_base=$PWD/$temp_base ;;
esac
work_dir=$(mktemp -d "$temp_base/ssh-tui-install.XXXXXX") || fail 'could not create a temporary directory'

prefix=ssh-tui-$version-$target
archive=$prefix.tar.gz
release=$repository/releases/download/v$version
printf 'Downloading ssh-tui %s for %s...\n' "$version" "$target"
download "$release/$archive" "$work_dir/$archive" || fail 'release archive download failed'
download "$release/SHA256SUMS" "$work_dir/SHA256SUMS" || fail 'release checksum download failed'

expected=$(awk -v name="$archive" '
    $2 == name {
        if (NF != 2 || length($1) != 64 || $1 ~ /[^0-9a-fA-F]/) exit 1
        hash = tolower($1)
        count++
    }
    END { if (count != 1) exit 1; print hash }
' "$work_dir/SHA256SUMS") || fail 'missing, duplicate, or invalid checksum for the selected archive'
if [ "$checksum_tool" = sha256sum ]; then
    actual=$(sha256sum < "$work_dir/$archive") || fail 'could not calculate SHA-256'
else
    actual=$(shasum -a 256 < "$work_dir/$archive") || fail 'could not calculate SHA-256'
fi
actual=${actual%% *}
[ "$actual" = "$expected" ] || fail 'SHA-256 mismatch; the existing installation was not changed'

member=$prefix/ssh-tui
tar -tzf "$work_dir/$archive" > "$work_dir/members" || fail 'invalid release archive'
count=$(awk -v name="$member" '$0 == name { count++ } END { print count + 0 }' "$work_dir/members")
[ "$count" = 1 ] || fail 'release archive must contain exactly one expected ssh-tui binary'
# Stream only the expected member; never extract archive paths into the filesystem.
tar -xOzf "$work_dir/$archive" "$member" > "$work_dir/ssh-tui" || fail 'could not extract the release binary'
mkdir -p "$install_dir" || fail "cannot create installation directory: $install_dir"
install_dir=$(CDPATH= cd "$install_dir" && pwd -P) || fail 'cannot access installation directory'
[ ! -d "$install_dir/ssh-tui" ] || fail "destination is a directory: $install_dir/ssh-tui"
stage_dir=$(mktemp -d "$install_dir/.ssh-tui-install.XXXXXX") || fail "cannot write to installation directory: $install_dir"
cp "$work_dir/ssh-tui" "$stage_dir/ssh-tui"
chmod 0755 "$stage_dir/ssh-tui"
reported_version=$("$stage_dir/ssh-tui" --version) || fail 'downloaded binary cannot run; check system runtime requirements in README'
[ "$reported_version" = "ssh-tui $version" ] || fail "unexpected binary version: $reported_version"
mv -f "$stage_dir/ssh-tui" "$install_dir/ssh-tui" || fail 'could not replace the installed binary'

printf 'Installed ssh-tui %s at %s\n' "$version" "$install_dir/ssh-tui"
printf 'Run: %s\n' "$(shell_quote "$install_dir/ssh-tui")"
case ":${PATH:-}:" in
    *":$install_dir:"*) ;;
    *)
        printf 'To run ssh-tui by name, add this directory to PATH:\n'
        printf '  export PATH=%s:"$PATH"\n' "$(shell_quote "$install_dir")"
        printf 'To make it persistent, add that line to your shell startup file.\n'
        ;;
esac

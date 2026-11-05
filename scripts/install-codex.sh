#!/bin/sh
# muster — one-line Codex installer for macOS / Linux.
#
#   curl -fsSL https://github.com/Castor-o7/muster/releases/latest/download/install-codex.sh \
#     | sh -s -- --daemon ws://<daemon-host>:6255
#
# Detects this machine's OS/arch, downloads the matching muster-shim from the
# latest release, and runs `codex-setup` — which installs the shim to a stable
# per-user path, wires ~/.codex (muster MCP server + hooks), and pre-trusts the
# hooks so they fire in the ChatGPT desktop app. Windows users run the .exe with
# `codex-setup` directly (see the release notes).
set -eu

REPO="https://github.com/Castor-o7/muster"
DAEMON="${MUSTER_DAEMON:-}"
while [ $# -gt 0 ]; do
    case "$1" in
        --daemon) DAEMON="${2:-}"; shift 2 ;;
        -h | --help)
            echo "usage: install-codex.sh --daemon ws://host:6255"
            exit 0
            ;;
        *)
            echo "install-codex: unknown argument '$1'" >&2
            exit 1
            ;;
    esac
done
if [ -z "$DAEMON" ]; then
    echo "error: pass the muster daemon address, e.g." >&2
    echo "  ... | sh -s -- --daemon ws://muster-01.example:6255" >&2
    exit 1
fi

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Darwin)
        case "$arch" in
            arm64) target="aarch64-apple-darwin" ;;
            x86_64) echo "unsupported macOS arch: $arch" >&2; exit 1 ;;
            *) echo "unsupported macOS arch: $arch" >&2; exit 1 ;;
        esac
        ;;
    Linux) target="x86_64-unknown-linux-gnu" ;;
    *)
        echo "unsupported OS: $os — on Windows, run the .exe with 'codex-setup'" >&2
        exit 1
        ;;
esac

asset="muster-shim-$target"
base="$REPO/releases/latest/download"
tmp=$(mktemp -d)
bin="$tmp/muster-shim"
trap 'rm -rf "$tmp"' EXIT INT TERM

# The SHA-256 of a file, with whichever tool this machine has (macOS ships shasum).
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        return 1
    fi
}

echo "Downloading muster-shim ($target)..."
curl -fSL --proto '=https' --tlsv1.2 -o "$bin" "$base/$asset"

# Check it against the release's SHA256SUMS before running it (MUS-95). Both come from the same
# release over HTTPS, so this catches a corrupt or swapped download, not a tampered release.
# Anything short of a match stops here, before the binary runs.
if ! curl -fsSL --proto '=https' --tlsv1.2 -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"; then
    echo "error: couldn't download SHA256SUMS from the release; not installing." >&2
    exit 1
fi
expected=$(awk -v f="$asset" '{ name = $2; sub(/^\*/, "", name) } name == f { print $1; exit }' "$tmp/SHA256SUMS")
if [ -z "$expected" ]; then
    echo "error: the release's SHA256SUMS has no entry for $asset; not installing." >&2
    exit 1
fi
if ! actual=$(sha256_of "$bin"); then
    echo "error: neither sha256sum nor shasum is installed, so the download can't be checked; not installing." >&2
    exit 1
fi
if [ "$actual" != "$expected" ]; then
    echo "error: $asset doesn't match the release's SHA256SUMS; not installing." >&2
    echo "  expected $expected" >&2
    echo "  got      $actual" >&2
    exit 1
fi
echo "Checked $asset against SHA256SUMS."
chmod +x "$bin"
# Clear the Gatekeeper quarantine in case this binary was fetched via a browser
# (curl downloads carry none). Harmless on Linux.
xattr -d com.apple.quarantine "$bin" 2>/dev/null || true

echo "Wiring muster into Codex..."
"$bin" codex-setup --daemon "$DAEMON"

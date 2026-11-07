#!/bin/sh
# muster: set up this machine in one command (MUS-98, the glue for MUS-15).
#
#   scripts/setup.sh [--daemon [--tls]] [--pair-file PATH|-] [--from-source] [--tag TAG | --rev SHA]
#                    [--prefix DIR] [--apply]
#
# Installs muster-shim (and, with --daemon, muster-daemon) into one place, PREFIX/bin, then hands
# over to the commands that already know how to wire a machine:
#   - with --daemon: `muster-daemon service [--tls]`, which writes the systemd user unit or launchd
#     plist that keeps the daemon running across reboots (#155)
#   - always: `muster-shim setup [--pair-file ...]`, which pairs, wires Claude Code through its own
#     `claude plugin` commands and ends with `muster-shim doctor` (#157)
#
# Like those two, it's a dry run unless given --apply: without it nothing is downloaded, installed
# or written, and each step says what it would do. With --apply it passes --apply on, and never
# --force or --drop-settings, so a unit that differs from the one `service` would write is refused
# for the operator to look at. It never starts, stops or restarts a daemon either: `service`
# prints the commands for that, and running them is the operator's call.
#
# Binaries come from the latest release, checked against its SHA256SUMS (as install-codex.sh
# does, MUS-96) once that is shown to be signed by the release key (MUS-110), or with `cargo install --locked` when there's no release yet or --from-source is
# given. Either way it's pinned to one release (MUS-111): the latest release's tag, looked up
# once, so the build is of the same release whose SHA256SUMS the download would check, or the
# release --tag names. --rev SHA builds that full commit from source instead. Only with no
# release at all does a build take the default branch, and it says so. A pairing code is only ever read from a file or stdin (--pair-file PATH, or - for stdin),
# never the command line, where shell history and `ps` would keep it.
set -eu

REPO="https://github.com/Castor-o7/muster"
# The key that signs releases (MUS-110), as release/allowed_signers has it; test-install-codex.sh
# checks the copies agree. Until the operator adds one, no release verifies, so none installs.
ALLOWED_SIGNERS=''
PREFIX="${MUSTER_PREFIX:-$HOME/.local}"
apply=0
daemon=0
tls=0
from_source=0
pair_file=""
tag=""
rev=""

usage() {
    echo "usage: setup.sh [--daemon [--tls]] [--pair-file PATH|-] [--from-source] [--tag TAG | --rev SHA] [--prefix DIR] [--apply]"
}
while [ $# -gt 0 ]; do
    case "$1" in
        --apply) apply=1; shift ;;
        --daemon) daemon=1; shift ;;
        --tls) tls=1; shift ;;
        --from-source) from_source=1; shift ;;
        --tag)
            [ $# -ge 2 ] || { usage >&2; exit 1; }
            tag=$2; shift 2 ;;
        --rev)
            [ $# -ge 2 ] || { usage >&2; exit 1; }
            rev=$2; shift 2 ;;
        --prefix)
            [ $# -ge 2 ] || { usage >&2; exit 1; }
            PREFIX=$2; shift 2 ;;
        --pair-file)
            [ $# -ge 2 ] || { echo "error: --pair-file needs a path, or - for stdin" >&2; exit 1; }
            pair_file=$2; shift 2 ;;
        -h | --help) usage; exit 0 ;;
        *)
            echo "error: unknown argument '$1'. A pairing code goes in a file (--pair-file PATH) or on stdin (--pair-file -), never on the command line." >&2
            usage >&2
            exit 1 ;;
    esac
done
if [ "$tls" = 1 ] && [ "$daemon" = 0 ]; then
    echo "error: --tls is for the daemon's unit; add --daemon" >&2
    exit 1
fi
# A tag is a plain name and a rev a full commit, so neither can be read as anything else.
valid_tag() { printf '%s' "$1" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$'; }
if [ -n "$tag" ] && [ -n "$rev" ]; then
    echo "error: give --tag or --rev, not both" >&2
    exit 1
fi
if [ -n "$tag" ] && ! valid_tag "$tag"; then
    echo "error: --tag '$tag' isn't a release tag such as v0.3.0" >&2
    exit 1
fi
if [ -n "$rev" ]; then
    printf '%s' "$rev" | grep -Eq '^([0-9a-fA-F]{40}|[0-9a-fA-F]{64})$' || {
        echo "error: --rev needs a full commit hash (40 hex digits), so the build is pinned" >&2
        exit 1
    }
    # A release's binaries are built from its tag, so a commit can only be built.
    from_source=1
fi
if [ -n "$pair_file" ] && [ "$pair_file" != - ] && [ ! -r "$pair_file" ]; then
    echo "error: can't read the pairing file $pair_file" >&2
    exit 1
fi

BIN="$PREFIX/bin"
bins="muster-shim"
[ "$daemon" = 1 ] && bins="muster-shim muster-daemon"
step() { echo; echo "== $*"; }
would() { echo "would: $*"; }

case "$(uname -s)/$(uname -m)" in
    Darwin/arm64) target="aarch64-apple-darwin" ;;
    Linux/x86_64) target="x86_64-unknown-linux-gnu" ;;
    *) target="" ;;
esac

[ "$apply" = 1 ] || echo "Dry run: nothing is downloaded, installed or written (it only asks GitHub which release is latest). Add --apply to do it."

# --- 1. Install the binaries into PREFIX/bin ------------------------------------------------------

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        return 1
    fi
}

# The latest release's tag, from where GitHub redirects releases/latest, or nothing when there's no
# release (MUS-111). It's looked up once, so the download and a build use the same release.
latest_tag() {
    url=$(curl -fsS --proto '=https' --tlsv1.2 -o /dev/null -w '%{redirect_url}' "$REPO/releases/latest" 2>/dev/null) || return 0
    case "$url" in
        */releases/tag/*)
            found=${url##*/releases/tag/}
            valid_tag "$found" && echo "$found" ;;
    esac
    return 0
}

# Whether $1/SHA256SUMS.sig is the release key's signature of $1/SHA256SUMS (MUS-110). 3 when
# ssh-keygen isn't here to check it.
signed_by_release_key() {
    command -v ssh-keygen >/dev/null 2>&1 || return 3
    printf '%s\n' "$ALLOWED_SIGNERS" >"$1/allowed_signers"
    ssh-keygen -Y verify -f "$1/allowed_signers" -I muster-release -n muster-release \
        -s "$1/SHA256SUMS.sig" <"$1/SHA256SUMS" >/dev/null 2>&1
}

# Downloads the release's binaries into $1, each checked against its SHA256SUMS, once that's shown
# to be signed by the release key. Returns 2 when the release has no SHA256SUMS (there's no release
# yet) or no SHA256SUMS.sig (it isn't signed), so the caller builds the pinned tag from source
# instead; anything else that goes wrong is fatal, since a bad signature or a mismatch must never
# fall through to an install.
fetch_release() {
    dir=$1
    base="$REPO/releases/download/$tag"
    [ -n "$tag" ] || base="$REPO/releases/latest/download"
    curl -fsSL --proto '=https' --tlsv1.2 -o "$dir/SHA256SUMS" "$base/SHA256SUMS" 2>/dev/null || return 2
    if ! curl -fsSL --proto '=https' --tlsv1.2 -o "$dir/SHA256SUMS.sig" "$base/SHA256SUMS.sig" 2>/dev/null; then
        echo "warning: the release has no SHA256SUMS.sig, so its binaries can't be shown to be the operator's; building from source instead" >&2
        return 2
    fi
    sig=0
    signed_by_release_key "$dir" || sig=$?
    if [ "$sig" = 3 ]; then
        echo "error: ssh-keygen (OpenSSH 8.1 or later) isn't installed, so the release's signature can't be checked; not installing. --from-source builds it instead." >&2
        exit 1
    elif [ "$sig" != 0 ]; then
        echo "error: SHA256SUMS.sig isn't the release key's signature of SHA256SUMS; not installing." >&2
        exit 1
    fi
    echo "checked SHA256SUMS against the release key's signature"
    for b in $bins; do
        asset="$b-$target"
        expected=$(awk -v f="$asset" '{ name = $2; sub(/^\*/, "", name) } name == f { print $1; exit }' "$dir/SHA256SUMS")
        [ -n "$expected" ] || { echo "error: the release's SHA256SUMS has no entry for $asset; not installing." >&2; exit 1; }
        curl -fSL --proto '=https' --tlsv1.2 -o "$dir/$b" "$base/$asset"
        actual=$(sha256_of "$dir/$b") || { echo "error: neither sha256sum nor shasum is installed, so downloads can't be checked; not installing." >&2; exit 1; }
        if [ "$actual" != "$expected" ]; then
            echo "error: $asset doesn't match the release's SHA256SUMS; not installing." >&2
            echo "  expected $expected" >&2
            echo "  got      $actual" >&2
            exit 1
        fi
        echo "checked $asset against SHA256SUMS"
    done
}

step "Install $bins into $BIN"
if [ "$from_source" = 0 ] && [ -z "$target" ]; then
    echo "no prebuilt binaries for $(uname -s)/$(uname -m); building from source"
    from_source=1
fi
# Which release, or commit, this installs (MUS-111).
release_is="release $tag"
if [ -z "$tag" ] && [ -z "$rev" ]; then
    tag=$(latest_tag)
    release_is="release $tag, the latest"
fi
if [ -n "$rev" ]; then
    ref="--rev $rev"
    ref_is="commit $rev"
elif [ -n "$tag" ]; then
    ref="--tag $tag"
    ref_is="tag $tag"
else
    ref=""
    ref_is="the default branch, unpinned: no release was found"
fi
build="cargo install --locked --root $PREFIX --git $REPO${ref:+ $ref} $bins"
if [ "$apply" = 0 ]; then
    if [ "$from_source" = 1 ]; then
        would "$build"
        echo "  (building $ref_is)"
    elif [ -n "$tag" ]; then
        would "download $bins for $target from $release_is, check its SHA256SUMS against the release key's signature and them against it, and copy them into $BIN"
        would "  (or, if it has no SHA256SUMS or isn't signed: $build)"
    else
        would "download $bins for $target from the latest release, check its SHA256SUMS against the release key's signature and them against it, and copy them into $BIN"
        would "  (or, if there's no release yet: $build, which builds $ref_is)"
    fi
else
    mkdir -p "$BIN"
    installed=0
    if [ "$from_source" = 0 ]; then
        tmp=$(mktemp -d)
        trap 'rm -rf "$tmp"' EXIT INT TERM
        status=0
        fetch_release "$tmp" || status=$?
        if [ "$status" = 0 ]; then
            for b in $bins; do
                chmod +x "$tmp/$b"
                # Clear the Gatekeeper quarantine in case one is set; harmless on Linux.
                xattr -d com.apple.quarantine "$tmp/$b" 2>/dev/null || true
                mv -f "$tmp/$b" "$BIN/$b"
                echo "installed $BIN/$b"
            done
            installed=1
        elif [ "$status" = 2 ]; then
            echo "no signed release with SHA256SUMS; building from source"
        else
            exit "$status"
        fi
    fi
    if [ "$installed" = 0 ]; then
        command -v cargo >/dev/null 2>&1 || { echo "error: there's no release to download and no cargo to build with; install Rust from https://rustup.rs" >&2; exit 1; }
        echo "building $ref_is"
        [ -n "$ref" ] || echo "warning: no release was found, so this builds the default branch, which isn't pinned; give --tag or --rev to pin it" >&2
        # shellcheck disable=SC2086 # $ref is a flag and its value, and $bins a list of crate names
        cargo install --locked --root "$PREFIX" --git "$REPO" $ref $bins
    fi
fi

# One copy of each, and that copy first on PATH: a stale one elsewhere (often ~/.cargo/bin) would
# be the one sessions run. Others are named, never deleted.
for b in $bins; do
    first=$(command -v "$b" 2>/dev/null || true)
    if [ "$apply" = 1 ] && [ "$first" != "$BIN/$b" ]; then
        if [ -z "$first" ]; then
            echo "warning: $BIN isn't on PATH, so '$b' won't be found. Add it to PATH in your shell's profile." >&2
        else
            echo "warning: '$b' on PATH is $first, not $BIN/$b. Put $BIN first on PATH, or remove $first." >&2
        fi
    fi
    others=$(which -a "$b" 2>/dev/null | grep -vx "$BIN/$b" || true)
    if [ -n "$others" ]; then
        echo "warning: other copies of $b on PATH (remove them so there's one):" >&2
        echo "$others" | sed 's/^/  /' >&2
    fi
done

# --- 2. The daemon's unit -------------------------------------------------------------------------

if [ "$daemon" = 1 ]; then
    step "The daemon's unit (muster-daemon service)"
    args=""
    [ "$tls" = 1 ] && args="--tls"
    [ "$apply" = 1 ] && args="$args --apply"
    if [ -x "$BIN/muster-daemon" ]; then
        # shellcheck disable=SC2086 # $args is a list of flags
        "$BIN/muster-daemon" service $args
    else
        would "$BIN/muster-daemon service $args"
    fi
fi

# --- 3. This machine as a client (muster-shim setup, which ends with doctor) -----------------------

step "Wire this machine (muster-shim setup)"
set --
[ -n "$pair_file" ] && set -- --pair-file "$pair_file"
[ "$apply" = 1 ] && set -- "$@" --apply
if [ -x "$BIN/muster-shim" ]; then
    "$BIN/muster-shim" setup "$@"
else
    would "$BIN/muster-shim setup $*"
fi

echo
if [ "$apply" = 1 ]; then
    echo "Done. Nothing was started or restarted: if a unit was written above, start it with the commands it printed."
else
    echo "Dry run done. Run again with --apply to do it."
fi

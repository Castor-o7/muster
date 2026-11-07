#!/bin/sh
# Check scripts/setup.sh (MUS-98) against fakes: a fake curl serving a fixture release, a fake cargo,
# and fake muster-shim and muster-daemon that record how they were run. systemctl and launchctl
# are fakes that record any call, so the test can show setup.sh never starts anything. Everything
# happens under a throwaway HOME and prefix; nothing is downloaded, built or wired. The releases
# and refs it pins (MUS-111) are checked too. Plain sh, so CI
# can run it (left out of ci.yml for now, during the MUS-97 freeze).
set -u
REPO=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM
pass=0
fail=0
check() {
    if eval "$2"; then echo "PASS  $1"; pass=$((pass + 1)); else echo "FAIL  $1"; fail=$((fail + 1)); fi
}

FAKE="$WORK/fake"
mkdir -p "$FAKE"
CALLS="$WORK/calls"
export CALLS

# The fake curl serves files from $FIXTURE by the URL's last part, or fails like `curl -f`. Asked
# for releases/latest's redirect (-w), it prints $FIXTURE/LATEST, as GitHub's Location would be.
cat >"$FAKE/curl" <<'EOF'
#!/bin/sh
out=""; url=""; write=""
while [ $# -gt 0 ]; do
    case "$1" in
        -o) out=$2; shift 2 ;;
        -w) write=$2; shift 2 ;;
        --proto) shift 2 ;;
        -*) shift ;;
        *) url=$1; shift ;;
    esac
done
echo "curl $url" >> "$CALLS"
if [ -n "$write" ]; then
    [ -f "$FIXTURE/LATEST" ] && cat "$FIXTURE/LATEST"
    exit 0
fi
src="$FIXTURE/${url##*/}"
[ -f "$src" ] || exit 22
cp "$src" "$out"
EOF
# The fake cargo installs fake binaries into --root/bin.
cat >"$FAKE/cargo" <<'EOF'
#!/bin/sh
echo "cargo $*" >> "$CALLS"
root=""
for a in "$@"; do [ "$prev" = --root ] && root=$a; prev=$a; done
mkdir -p "$root/bin"
for b in muster-shim muster-daemon; do
    case " $* " in *" $b "*) "$MAKE_FAKE" "$b" > "$root/bin/$b"; chmod +x "$root/bin/$b" ;; esac
done
EOF
for tool in systemctl launchctl; do printf '#!/bin/sh\necho "%s $*" >> "$CALLS"\n' "$tool" >"$FAKE/$tool"; done
# Makes a fake muster binary named $1 on stdout: it records "$1 <args>", and for
# `setup --pair-file -` also what came on stdin.
cat >"$WORK/make-fake" <<'EOF'
#!/bin/sh
name=$1
cat <<INNER
#!/bin/sh
echo "$name \$*" >> "\$CALLS"
if [ "\$1 \$2 \$3" = "setup --pair-file -" ]; then echo "stdin: \$(cat)" >> "\$CALLS"; fi
exit 0
INNER
EOF
chmod +x "$FAKE"/* "$WORK/make-fake"
MAKE_FAKE="$WORK/make-fake"
export MAKE_FAKE

case "$(uname -s)/$(uname -m)" in
    Darwin/arm64) TARGET="aarch64-apple-darwin" ;;
    *) TARGET="x86_64-unknown-linux-gnu" ;;
esac

# A throwaway release key, and a copy of setup.sh that trusts only it (MUS-110).
ssh-keygen -q -t ed25519 -N '' -C test-release -f "$WORK/key"
ssh-keygen -q -t ed25519 -N '' -C someone-else -f "$WORK/other"
TRUSTED="muster-release namespaces=\"muster-release\" $(cut -d' ' -f1,2 "$WORK/key.pub")"
SETUP="$WORK/setup.sh"
awk -v line="$TRUSTED" '/^ALLOWED_SIGNERS=/ { print "ALLOWED_SIGNERS='"'"'" line "'"'"'"; next } { print }' \
    "$REPO/scripts/setup.sh" >"$SETUP"
sign() { rm -f "$FIXTURE/SHA256SUMS.sig"; ssh-keygen -q -Y sign -f "$1" -n muster-release "$FIXTURE/SHA256SUMS" 2>/dev/null; }

# A fixture release with both binaries and a SHA256SUMS, signed with the release key. $1 names it.
release() {
    FIXTURE="$WORK/release-$1"
    rm -rf "$FIXTURE"; mkdir -p "$FIXTURE"
    for b in muster-shim muster-daemon; do "$MAKE_FAKE" "$b" >"$FIXTURE/$b-$TARGET"; done
    (cd "$FIXTURE" && sha256sum muster-* >SHA256SUMS)
    echo "https://github.com/Castor-o7/muster/releases/tag/v9.9.9" >"$FIXTURE/LATEST"
    export FIXTURE
    sign "$WORK/key"
}

# Runs setup.sh with a fresh HOME and prefix. Leaves $status, $WORK/out and $CALLS behind.
run() {
    export HOME="$WORK/home"; rm -rf "$HOME" "$WORK/prefix"; mkdir -p "$HOME"
    : >"$CALLS"
    PATH="$WORK/prefix/bin:$FAKE:$BASE_PATH" MUSTER_PREFIX="$WORK/prefix" sh "$SETUP" "$@" >"$WORK/out" 2>&1
    status=$?
}
called() { grep -q -- "$1" "$CALLS"; }
said() { grep -q -- "$1" "$WORK/out"; }
BASE_PATH=/usr/bin:/bin

release good
run --daemon
check "a dry run downloads, installs and builds nothing" '[ $status -eq 0 ] && [ ! -e "$WORK/prefix" ] && ! called "curl .*/download/" && ! called cargo'
check "and says what it would do, from which release" 'said "Dry run" && said "would: download muster-shim muster-daemon for $TARGET from release v9.9.9, the latest"'
check "and that a build would pin that release's tag" 'said "--git https://github.com/Castor-o7/muster --tag v9.9.9 muster-shim muster-daemon"'

run --daemon --tls --apply
check "--apply checks the release key's signature" 'said "checked SHA256SUMS against the release key.s signature"'
check "--apply installs both from the release, checked" '[ $status -eq 0 ] && [ -x "$WORK/prefix/bin/muster-shim" ] && [ -x "$WORK/prefix/bin/muster-daemon" ] && said "checked muster-daemon-$TARGET against SHA256SUMS"'
check "from the tag it looked up, not whatever latest is by then" 'called "curl https://github.com/Castor-o7/muster/releases/download/v9.9.9/SHA256SUMS" && ! called "latest/download"'
check "then writes the unit through muster-daemon service --tls --apply" 'called "^muster-daemon service --tls --apply$"'
check "and wires the client through muster-shim setup --apply" 'called "^muster-shim setup --apply$"'
check "never with --force or --drop-settings" '! called "--force" && ! called "--drop-settings"'
check "and never starts or restarts anything" '! called systemctl && ! called launchctl'

run
check "without --daemon, only the shim, and no unit" '[ $status -eq 0 ] && ! called "muster-daemon" && said "would: download muster-shim for"'

release tampered
echo "# changed" >>"$FIXTURE/muster-daemon-$TARGET"
run --daemon --apply
check "a tampered binary stops everything" '[ $status -ne 0 ] && said "doesn.t match the release.s SHA256SUMS" && [ ! -e "$WORK/prefix/bin/muster-daemon" ] && ! called "muster-daemon service" && ! called "muster-shim setup"'

release none
rm -f "$FIXTURE"/*
run --daemon --apply
check "with no release yet, it builds with cargo install --locked into the prefix" '[ $status -eq 0 ] && called "^cargo install --locked --root $WORK/prefix --git https://github.com/Castor-o7/muster muster-shim muster-daemon$"'
check "and warns that the default branch isn't pinned" 'said "builds the default branch, which isn.t pinned"'
check "and still hands over to service and setup" 'called "^muster-daemon service --apply$" && called "^muster-shim setup --apply$"'
run --daemon
check "which the dry run says too" 'said "builds the default branch, unpinned: no release was found"'

# MUS-110: with a release key, an unsigned release stops everything unless the operator asks for
# --from-source, since deleting SHA256SUMS.sig mustn't buy an unsigned build; a bad signature always
# stops everything.
BUILD_TAG="^cargo install --locked --root $WORK/prefix --git https://github.com/Castor-o7/muster --tag v9.9.9 muster-shim muster-daemon$"
release unsigned
rm "$FIXTURE/SHA256SUMS.sig"
run --daemon --apply
check "with a release key, an unsigned release is refused, not built" '[ $status -ne 0 ] && said "has no SHA256SUMS.sig" && said "Pass --from-source" && ! called cargo && [ ! -e "$WORK/prefix/bin/muster-daemon" ] && ! called "muster-shim setup"'
run --daemon --from-source --apply
check "but --from-source builds its tag" '[ $status -eq 0 ] && called "$BUILD_TAG" && ! called "curl .*/download/"'
release other-key
sign "$WORK/other"
run --daemon --apply
check "a release signed by another key stops everything" '[ $status -ne 0 ] && said "isn.t the release key.s signature" && [ ! -e "$WORK/prefix/bin/muster-daemon" ] && ! called cargo && ! called "muster-shim setup"'
# With no release key yet (the real script's placeholder), no release's binaries are installed,
# signed or not: the pinned tag is built, saying why.
PATCHED=$SETUP
SETUP="$REPO/scripts/setup.sh"
release good
run --daemon --apply
check "with no release key yet, a signed release's tag is built, not its binaries" '[ $status -eq 0 ] && said "no release key in this script yet" && called "$BUILD_TAG" && ! called "curl .*muster-shim-$TARGET" && ! called "SHA256SUMS.sig"'
release unsigned
rm "$FIXTURE/SHA256SUMS.sig"
run --daemon --apply
check "and so is an unsigned one's" '[ $status -eq 0 ] && called "$BUILD_TAG" && ! called "curl .*muster-shim-$TARGET"'
SETUP=$PATCHED

release good
run --from-source --apply
check "--from-source skips the release's binaries" '! called "curl .*/download/" && called "^cargo install"'
check "and builds the latest release's tag, not the default branch" 'called "^cargo install --locked --root $WORK/prefix --git https://github.com/Castor-o7/muster --tag v9.9.9 muster-shim$"'
run --from-source --tag v1.2.3 --apply
check "--tag picks the release to build" 'called "--tag v1.2.3 muster-shim$" && ! called "releases/latest"'
run --tag v1.2.3 --apply
check "and the release to download" 'called "curl https://github.com/Castor-o7/muster/releases/download/v1.2.3/SHA256SUMS"'
REV=0123456789abcdef0123456789abcdef01234567
run --rev $REV
check "--rev builds that commit, said in the dry run" '[ $status -eq 0 ] && said "--rev $REV muster-shim" && said "(building commit $REV)" && ! called curl'
run --rev $REV --apply
check "and with --apply, builds it from source" 'called "^cargo install --locked --root $WORK/prefix --git https://github.com/Castor-o7/muster --rev $REV muster-shim$"'
run --rev 0123abc
check "a short --rev is refused" '[ $status -ne 0 ] && said "full commit hash" && ! called cargo'
run --tag "v1;rm -rf /"
check "a --tag that isn't a plain name is refused" '[ $status -ne 0 ] && said "isn.t a release tag" && ! called curl'
run --tag v1 --rev $REV
check "--tag and --rev together are refused" '[ $status -ne 0 ] && said "not both"'

echo "the-code" >"$WORK/code"
run --pair-file "$WORK/code" --apply
check "a pairing file is passed on as a file" 'called "^muster-shim setup --pair-file $WORK/code --apply$"'
export HOME="$WORK/home"
echo "piped-code" | { rm -rf "$WORK/prefix"; : >"$CALLS"; PATH="$WORK/prefix/bin:$FAKE:$BASE_PATH" MUSTER_PREFIX="$WORK/prefix" sh "$SETUP" --pair-file - --apply >"$WORK/out" 2>&1; }
check "and --pair-file - passes stdin through to setup" 'called "^muster-shim setup --pair-file - --apply$" && called "^stdin: piped-code$"'
run ABCD-1234-code
check "a code on the command line is refused" '[ $status -ne 0 ] && said "never on the command line" && ! called curl'
run --tls
check "--tls without --daemon is refused" '[ $status -ne 0 ] && said "add --daemon"'

# A stale copy elsewhere on PATH is named, not deleted.
mkdir -p "$WORK/stale"
"$MAKE_FAKE" muster-shim >"$WORK/stale/muster-shim"; chmod +x "$WORK/stale/muster-shim"
BASE_PATH="$WORK/stale:/usr/bin:/bin"
run --apply
check "another copy on PATH is warned about by path" '[ $status -eq 0 ] && said "other copies of muster-shim" && said "$WORK/stale/muster-shim" && [ -e "$WORK/stale/muster-shim" ]'
BASE_PATH=/usr/bin:/bin

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

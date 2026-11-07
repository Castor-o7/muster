#!/bin/sh
# Check that scripts/install-codex.sh runs the shim only when it matches the release's SHA256SUMS
# (MUS-96), and only when that SHA256SUMS is signed by the release key (MUS-110). The fixture is
# signed with a throwaway key, and a copy of the installer is given that key in place of the real
# one; the real installer is checked to carry exactly release/allowed_signers' key. A fake `curl` serves a local fixture instead of GitHub, and the "shim" is a script that
# records how it was run, so nothing is downloaded and Codex is never touched. Runs in CI.
set -u
REPO=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM
pass=0
fail=0
check() {
    if eval "$2"; then echo "PASS  $1"; pass=$((pass + 1)); else echo "FAIL  $1"; fail=$((fail + 1)); fi
}

# The fake curl: `-o FILE ... URL` copies the fixture named like the URL's last part, or fails as
# `curl -f` does when the release has no such file.
mkdir -p "$WORK/fake"
cat >"$WORK/fake/curl" <<'EOF'
#!/bin/sh
out=""
url=""
while [ $# -gt 0 ]; do
    case "$1" in
        -o) out=$2; shift 2 ;;
        --proto | --tlsv1.2) [ "$1" = --proto ] && shift; shift ;;
        -*) shift ;;
        *) url=$1; shift ;;
    esac
done
src="$FIXTURE/${url##*/}"
[ -f "$src" ] || { echo "curl: (22) 404 for $url" >&2; exit 22; }
cp "$src" "$out"
EOF
chmod +x "$WORK/fake/curl"

# A throwaway release key, and a copy of the installer that trusts only it (MUS-110).
ssh-keygen -q -t ed25519 -N '' -C test-release -f "$WORK/key"
ssh-keygen -q -t ed25519 -N '' -C someone-else -f "$WORK/other"
TRUSTED="muster-release namespaces=\"muster-release\" $(cut -d' ' -f1,2 "$WORK/key.pub")"
INSTALLER="$WORK/install-codex.sh"
awk -v line="$TRUSTED" '/^ALLOWED_SIGNERS=/ { print "ALLOWED_SIGNERS='"'"'" line "'"'"'"; next } { print }' \
    "$REPO/scripts/install-codex.sh" >"$INSTALLER"
sign() { rm -f "$FIXTURE/SHA256SUMS.sig"; ssh-keygen -q -Y sign -f "$1" -n muster-release "$FIXTURE/SHA256SUMS" 2>/dev/null; }

# A fixture release: the "shim" under both targets' names, a SHA256SUMS listing them, and that
# signed with the release key.
release() {
    FIXTURE="$WORK/release-$1"
    rm -rf "$FIXTURE"
    mkdir -p "$FIXTURE"
    for target in aarch64-apple-darwin x86_64-unknown-linux-gnu; do
        printf '#!/bin/sh\necho "$*" > "$MARK"\n' >"$FIXTURE/muster-shim-$target"
    done
    (cd "$FIXTURE" && sha256sum muster-shim-* >SHA256SUMS)
    export FIXTURE
    sign "$WORK/key"
}

# Runs the installer with PATH=$1, and records its exit status, its output and what the shim saw.
install() {
    MARK="$WORK/shim-ran"
    rm -f "$MARK"
    export MARK
    PATH=$1 sh "${2:-$INSTALLER}" --daemon ws://test:6255 >"$WORK/out" 2>&1
    status=$?
}
ran() { [ -f "$MARK" ]; }
said() { grep -q "$1" "$WORK/out"; }

FULL="$WORK/fake:$PATH"

release good
install "$FULL"
check "a matching shim installs" '[ $status -eq 0 ] && said "Checked muster-shim-"'
check "and is run with codex-setup and the daemon" '[ "$(cat "$MARK")" = "codex-setup --daemon ws://test:6255" ]'

release tampered
for f in "$FIXTURE"/muster-shim-*; do echo "# one byte more" >>"$f"; done
install "$FULL"
check "a tampered shim is refused" '[ $status -ne 0 ] && said "doesn.t match the release.s SHA256SUMS"'
check "and never runs" '! ran'

release no-entry
: >"$FIXTURE/SHA256SUMS.new"
grep -v muster-shim "$FIXTURE/SHA256SUMS" >"$FIXTURE/SHA256SUMS.new"
mv "$FIXTURE/SHA256SUMS.new" "$FIXTURE/SHA256SUMS"
sign "$WORK/key"
install "$FULL"
check "a SHA256SUMS without the shim is refused" '[ $status -ne 0 ] && said "has no entry for muster-shim-" && ! ran'

release no-sums
rm "$FIXTURE/SHA256SUMS"
install "$FULL"
check "a release without SHA256SUMS is refused" '[ $status -ne 0 ] && said "couldn.t download SHA256SUMS" && ! ran'

# MUS-110: the signature.
release good
install "$FULL"
check "a signed release says it checked the signature" '[ $status -eq 0 ] && said "Checked SHA256SUMS against the release key.s signature" && ran'
release unsigned
rm "$FIXTURE/SHA256SUMS.sig"
install "$FULL"
check "an unsigned release is refused" '[ $status -ne 0 ] && said "has no SHA256SUMS.sig" && ! ran'
release other-key
sign "$WORK/other"
install "$FULL"
check "a release signed by another key is refused" '[ $status -ne 0 ] && said "isn.t the release key.s signature" && ! ran'
release swapped-sums
# A SHA256SUMS rewritten after signing, with the shim and its sum swapped together.
for f in "$FIXTURE"/muster-shim-*; do echo "# one byte more" >>"$f"; done
(cd "$FIXTURE" && sha256sum muster-shim-* >SHA256SUMS)
install "$FULL"
check "a SHA256SUMS changed after signing is refused, even when the shim matches it" '[ $status -ne 0 ] && said "isn.t the release key.s signature" && ! ran'
release good
install "$FULL" "$REPO/scripts/install-codex.sh"
check "the real installer, with no release key yet, refuses even a signed release" '[ $status -ne 0 ] && said "isn.t the release key.s signature" && ! ran'

# The key lines in release/allowed_signers, install-codex.sh and setup.sh are the same.
keys() { grep -v '^#' "$REPO/release/allowed_signers" | grep . || true; }
embedded() { sed -n "s/^ALLOWED_SIGNERS='\(.*\)'\$/\1/p" "$1"; }
check "install-codex.sh carries release/allowed_signers' key" '[ "$(embedded "$REPO/scripts/install-codex.sh")" = "$(keys)" ]'
check "and so does setup.sh" '[ "$(embedded "$REPO/scripts/setup.sh")" = "$(keys)" ]'

# macOS has shasum, not sha256sum. A PATH with only the tools the installer needs, a shasum that
# answers like the real one, and no sha256sum.
mkdir -p "$WORK/min"
for tool in sh awk uname mktemp chmod rm cp cat ssh-keygen; do ln -s "$(command -v "$tool")" "$WORK/min/$tool"; done
ln -s "$WORK/fake/curl" "$WORK/min/curl"
SHA256SUM=$(command -v sha256sum)
printf '#!/bin/sh\n[ "$1" = -a ] && [ "$2" = 256 ] || exit 2\nexec %s "$3"\n' "$SHA256SUM" >"$WORK/min/shasum"
chmod +x "$WORK/min/shasum"
release good
install "$WORK/min"
check "with only shasum, a matching shim installs" '[ $status -eq 0 ] && ran'
release tampered
for f in "$FIXTURE"/muster-shim-*; do echo "# one byte more" >>"$f"; done
install "$WORK/min"
check "with only shasum, a tampered shim is refused" '[ $status -ne 0 ] && ! ran'
rm "$WORK/min/shasum"
release good
install "$WORK/min"
check "with neither tool, it refuses rather than skip the check" '[ $status -ne 0 ] && said "neither sha256sum nor shasum" && ! ran'
rm "$WORK/min/ssh-keygen"
ln -s "$SHA256SUM" "$WORK/min/sha256sum"
install "$WORK/min"
check "without ssh-keygen, it refuses rather than skip the signature" '[ $status -ne 0 ] && said "ssh-keygen (OpenSSH 8.1 or later) isn.t installed" && ! ran'

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

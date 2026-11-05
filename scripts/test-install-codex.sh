#!/bin/sh
# Check that scripts/install-codex.sh runs the shim only when it matches the release's SHA256SUMS
# (MUS-96). A fake `curl` serves a local fixture instead of GitHub, and the "shim" is a script that
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

# A fixture release: the "shim" under both targets' names, and a SHA256SUMS listing them.
release() {
    FIXTURE="$WORK/release-$1"
    rm -rf "$FIXTURE"
    mkdir -p "$FIXTURE"
    for target in aarch64-apple-darwin x86_64-unknown-linux-gnu; do
        printf '#!/bin/sh\necho "$*" > "$MARK"\n' >"$FIXTURE/muster-shim-$target"
    done
    (cd "$FIXTURE" && sha256sum muster-shim-* >SHA256SUMS)
    export FIXTURE
}

# Runs the installer with PATH=$1, and records its exit status, its output and what the shim saw.
install() {
    MARK="$WORK/shim-ran"
    rm -f "$MARK"
    export MARK
    PATH=$1 sh "$REPO/scripts/install-codex.sh" --daemon ws://test:6255 >"$WORK/out" 2>&1
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
install "$FULL"
check "a SHA256SUMS without the shim is refused" '[ $status -ne 0 ] && said "has no entry for muster-shim-" && ! ran'

release no-sums
rm "$FIXTURE/SHA256SUMS"
install "$FULL"
check "a release without SHA256SUMS is refused" '[ $status -ne 0 ] && said "couldn.t download SHA256SUMS" && ! ran'

# macOS has shasum, not sha256sum. A PATH with only the tools the installer needs, a shasum that
# answers like the real one, and no sha256sum.
mkdir -p "$WORK/min"
for tool in sh awk uname mktemp chmod rm cp cat; do ln -s "$(command -v "$tool")" "$WORK/min/$tool"; done
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

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

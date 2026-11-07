#!/bin/sh
# Check scripts/sign-release.sh (MUS-110) against a fake gh serving a fixture draft release, with a
# throwaway release key. Nothing touches GitHub or the operator's keys. Plain sh.
set -u
REPO=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM
pass=0
fail=0
check() {
    if eval "$2"; then echo "PASS  $1"; pass=$((pass + 1)); else echo "FAIL  $1"; fail=$((fail + 1)); fi
}

# A checkout of just the script, beside its own release/allowed_signers.
mkdir -p "$WORK/root/scripts" "$WORK/root/release" "$WORK/fake" "$WORK/draft"
cp "$REPO/scripts/sign-release.sh" "$WORK/root/scripts/"
SIGNERS="$WORK/root/release/allowed_signers"
ssh-keygen -q -t ed25519 -N '' -C test-release -f "$WORK/key"
ssh-keygen -q -t ed25519 -N '' -C someone-else -f "$WORK/other"
trust() { printf '# test\nmuster-release namespaces="muster-release" %s\n' "$(cut -d' ' -f1,2 "$1")" >"$SIGNERS"; }

# The fake gh: `release view` answers $DRAFT, `release download` copies the fixture's SHA256SUMS,
# and `release upload` keeps what it was given. Every call is recorded.
CALLS="$WORK/calls"
export CALLS
cat >"$WORK/fake/gh" <<'EOF'
#!/bin/sh
echo "gh $*" >> "$CALLS"
case "$1 $2" in
    "release view") [ "$DRAFT" = none ] && exit 1; echo "$DRAFT" ;;
    "release download")
        dir=""; prev=""
        for a in "$@"; do [ "$prev" = -D ] && dir=$a; prev=$a; done
        cp "$FIXTURE/SHA256SUMS" "$dir/" ;;
    "release upload") cp "$4" "$UPLOADED" ;;
esac
EOF
chmod +x "$WORK/fake/gh"
FIXTURE="$WORK/draft"
UPLOADED="$WORK/uploaded.sig"
export FIXTURE UPLOADED
echo "0123  muster-shim-x86_64-unknown-linux-gnu" >"$FIXTURE/SHA256SUMS"

run() {
    : >"$CALLS"
    rm -f "$UPLOADED"
    PATH="$WORK/fake:$PATH" sh "$WORK/root/scripts/sign-release.sh" "$@" >"$WORK/out" 2>&1
    status=$?
}
said() { grep -q -- "$1" "$WORK/out"; }
called() { grep -q -- "$1" "$CALLS"; }

printf '# no key yet\n' >"$SIGNERS"
DRAFT=true; export DRAFT
run v1.0.0 --key "$WORK/key.pub" --apply
check "with no release key in allowed_signers, it refuses before anything" '[ $status -ne 0 ] && said "has no muster-release key yet" && ! called gh'

trust "$WORK/key.pub"
run v1.0.0 --key "$WORK/key.pub"
check "a dry run shows the draft's SHA256SUMS and signs and uploads nothing" '[ $status -eq 0 ] && said "muster-shim-x86_64" && said "Dry run" && [ ! -e "$UPLOADED" ] && ! called "release upload"'

run v1.0.0 --key "$WORK/key" --apply
check "--apply uploads a signature" '[ $status -eq 0 ] && [ -s "$UPLOADED" ] && called "^gh release upload v1.0.0 .*SHA256SUMS.sig -R Castor-o7/muster$"'
check "that the installers' check accepts" 'ssh-keygen -Y verify -f "$SIGNERS" -I muster-release -n muster-release -s "$UPLOADED" <"$FIXTURE/SHA256SUMS" >/dev/null 2>&1'
check "and it never publishes" '! called "release edit"'

run v1.0.0 --key "$WORK/other" --apply
check "a key other than the release key is refused before the upload" '[ $status -ne 0 ] && said "doesn.t verify against" && ! called "release upload"'

DRAFT=false
run v1.0.0 --key "$WORK/key" --apply
check "a release that's already published is refused" '[ $status -ne 0 ] && said "already published" && ! called "release upload"'
DRAFT=none
run v1.0.0 --key "$WORK/key" --apply
check "and one that doesn't exist" '[ $status -ne 0 ] && said "no release v1.0.0" && ! called "release upload"'
DRAFT=true

run "v1;rm" --key "$WORK/key" --apply
check "a tag that isn't a plain name is refused" '[ $status -ne 0 ] && said "isn.t a release tag" && ! called gh'
run v1.0.0
check "--key is required" '[ $status -ne 0 ] && said "usage"'

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

#!/bin/sh
# muster: sign a draft release's SHA256SUMS with the release key (MUS-110), then upload the
# signature, before the operator publishes the release.
#
#   scripts/sign-release.sh <tag> --key <public key file> [--apply]
#
# release.yml builds the binaries and SHA256SUMS into a draft release, and holds no key. This runs
# on the operator's machine: it downloads the draft's SHA256SUMS, signs it with
# `ssh-keygen -Y sign -n muster-release`, checks the signature against release/allowed_signers, and
# uploads SHA256SUMS.sig to the draft. --key names the public half; ssh-keygen then signs through
# the agent holding the private half (Secretive, say), so the key never leaves it. A private key
# file works too.
#
# Like setup.sh, it's a dry run unless given --apply: without it nothing is signed or uploaded, and
# the key is never asked to sign. It never publishes: that stays the operator's step.
set -eu

SLUG="Castor-o7/muster"
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SIGNERS="$ROOT/release/allowed_signers"
apply=0
tag=""
key=""

usage() { echo "usage: sign-release.sh <tag> --key <public key file> [--apply]"; }
while [ $# -gt 0 ]; do
    case "$1" in
        --apply) apply=1; shift ;;
        --key)
            [ $# -ge 2 ] || { usage >&2; exit 1; }
            key=$2; shift 2 ;;
        -h | --help) usage; exit 0 ;;
        -*) echo "error: unknown argument '$1'" >&2; usage >&2; exit 1 ;;
        *)
            [ -z "$tag" ] || { usage >&2; exit 1; }
            tag=$1; shift ;;
    esac
done
[ -n "$tag" ] && [ -n "$key" ] || { usage >&2; exit 1; }
printf '%s' "$tag" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$' || {
    echo "error: '$tag' isn't a release tag such as v0.3.0" >&2
    exit 1
}
[ -r "$key" ] || { echo "error: can't read the key file $key" >&2; exit 1; }
if ! grep -Eq '^muster-release ' "$SIGNERS"; then
    echo "error: $SIGNERS has no muster-release key yet, so no signature would verify." >&2
    echo "  Add your public key there, and in ALLOWED_SIGNERS in scripts/install-codex.sh and scripts/setup.sh, first." >&2
    exit 1
fi
for tool in gh ssh-keygen; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool isn't installed" >&2; exit 1; }
done

draft=$(gh release view "$tag" -R "$SLUG" --json isDraft -q .isDraft) || {
    echo "error: there's no release $tag on $SLUG; push the tag and let release.yml make the draft" >&2
    exit 1
}
if [ "$draft" != true ]; then
    echo "error: $tag is already published. Sign the draft before publishing it, so no one installs it unsigned." >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
gh release download "$tag" -R "$SLUG" -p SHA256SUMS -D "$tmp"
echo "SHA256SUMS of the draft $tag:"
sed 's/^/  /' "$tmp/SHA256SUMS"

if [ "$apply" = 0 ]; then
    echo "would: ssh-keygen -Y sign -f $key -n muster-release SHA256SUMS"
    echo "would: check it against $SIGNERS, then gh release upload $tag SHA256SUMS.sig -R $SLUG"
    echo "Dry run: nothing was signed or uploaded. Add --apply to do it."
    exit 0
fi

ssh-keygen -Y sign -f "$key" -n muster-release "$tmp/SHA256SUMS"
# The installers trust only the release key, so a signature by any other key is caught here
# rather than by everyone who installs.
if ! ssh-keygen -Y verify -f "$SIGNERS" -I muster-release -n muster-release \
    -s "$tmp/SHA256SUMS.sig" <"$tmp/SHA256SUMS" >/dev/null; then
    echo "error: that signature doesn't verify against $SIGNERS: is --key the release key?" >&2
    exit 1
fi
gh release upload "$tag" "$tmp/SHA256SUMS.sig" -R "$SLUG"
echo "Uploaded SHA256SUMS.sig to the draft $tag. Publish it when you're ready: gh release edit $tag --draft=false -R $SLUG"

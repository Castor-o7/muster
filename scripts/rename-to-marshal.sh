#!/usr/bin/env bash
# Undo the fork's rename (8cea618) in a source tree: muster -> marshal.
# Usage: rename-to-marshal.sh <tree>   (a checkout or an extracted archive)
# Renames paths first, then rewrites text in tracked-looking files.
# Leaves vendor/, target/, .git/ and binary files alone. Idempotent.
set -euo pipefail
tree=${1:?usage: rename-to-marshal.sh <tree>}
cd "$tree"

# 1. Paths with "muster" in them, deepest first so parents move last.
find . -depth -name '*muster*' -not -path './.git/*' -not -path './target/*' \
  -not -path './vendor/*' -print0 |
  while IFS= read -r -d '' p; do
    dir=$(dirname "$p"); base=$(basename "$p")
    new="$dir/${base//muster/marshal}"
    if [ -d .git ]; then git mv -k "$p" "$new" 2>/dev/null || mv "$p" "$new"; else mv "$p" "$new"; fi
  done

# 2. Text. Order matters: specific forms before the generic ones.
# NUL-separated, so paths with spaces survive. `--null`, not `-Z`: GNU grep
# reads -Z as --null, but BSD grep (macOS) reads it as --decompress and ugrep
# as --fuzzy, and then the whole list reached perl as one file name. The
# pattern has no \b, which BSD's basic regex lacks; it only picks the files,
# and perl's \b decides what changes.
# Caution: the \b625[5-8]\b rewrite hits any bounded 6255-6258, not just ports
# (e.g. a "-6255-" segment in a test UUID), so always read the resulting diff.
grep -rIl --null --exclude-dir=.git --exclude-dir=target --exclude-dir=vendor \
  -e muster -e Muster -e MUSTER -e '625[5-8]' . |
  xargs -0 -r perl -pi -e '
  s{github\.com/Castor-o7/muster}{github.com/ignition-is-go/marshal}g;
  s{Castor-o7/muster}{ignition-is-go/marshal}g;
  s/\b6255\b/6155/g; s/\b6256\b/6156/g; s/\b6257\b/6157/g; s/\b6258\b/6158/g;
  s/X-Muster-/X-Marshal-/g;
  s/MUSTER/MARSHAL/g;
  s/Muster/Marshal/g;
  s/muster/marshal/g;
'

# 3. Report anything left for a human to look at.
left=$(grep -rIn --exclude-dir=.git --exclude-dir=target --exclude-dir=vendor -i muster . || true)
if [ -n "$left" ]; then echo "still mentions muster:"; echo "$left"; exit 1; fi

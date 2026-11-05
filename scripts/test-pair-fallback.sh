#!/bin/zsh
# End-to-end check of MUS-16 option B (docs/remote-access.md): a pairing with
# several gate addresses, the first one unreachable, still reaches the daemon.
# Held on a branch until the operator picks option B; it fails until B is built.
# Build first (`cargo build`), then run this script. Needs zsh, nc and python3.
set -u
BIN=$(cd "$(dirname "$0")/.." && pwd)/target/debug
WORK=$(mktemp -d "${TMPDIR:-/tmp}/muster-fallback-e2e.XXXX")
export MUSTER_STATE_DIR=$WORK/state MUSTER_TLS=1
export MUSTER_BIND=127.0.0.1:16455 MUSTER_HOOK_BIND=127.0.0.1:16456 MUSTER_MYKO_BIND=127.0.0.1:16458
unset MUSTER_BASE_URL MUSTER_DAEMON_ADDRESS
CLOSED=127.0.0.1:16499   # nothing listens here
GATE=127.0.0.1:16455
T() { perl -e 'alarm 15; exec @ARGV' "$@"; }
pass=0; fail=0
check() { if eval "$2"; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1"; fail=$((fail+1)); fi; }

XDG_CONFIG_HOME=$WORK/daemon RUST_LOG=info $BIN/muster-daemon >$WORK/daemon.log 2>&1 &
DPID=$!
trap 'kill $DPID 2>/dev/null; wait $DPID 2>/dev/null; rm -rf $WORK' EXIT
for i in {1..50}; do nc -z 127.0.0.1 16455 2>/dev/null && break; sleep 0.2; done
check "nothing listens on the closed address" '! nc -z ${CLOSED%:*} ${CLOSED#*:} 2>/dev/null'

CODE=$(T env XDG_CONFIG_HOME=$WORK/daemon $BIN/muster-daemon pair --name fb --address $GATE 2>/dev/null | awk '{print $3}')
export XDG_CONFIG_HOME=$WORK/client HOME=$WORK/client
mkdir -p $WORK/client
T $BIN/muster-shim pair "$CODE" >/dev/null
PAIRING=$WORK/client/muster/pairing
check "paired" '[[ -s $PAIRING ]]'

# Rewrite the stored pairing: `address` (the first entry, as old pairings have it)
# is the closed port, and `addresses` lists it before the real gate.
REWRITE() {
  python3 -I - "$PAIRING" "$@" <<'PY'
import base64, json, sys
path, *addrs = sys.argv[1:]
prefix, body = open(path).read().strip().rsplit(":", 1)
p = json.loads(base64.urlsafe_b64decode(body + "=" * (-len(body) % 4)))
p["address"] = addrs[0]
if len(addrs) > 1:
    p["addresses"] = addrs
else:
    p.pop("addresses", None)
enc = base64.urlsafe_b64encode(json.dumps(p).encode()).decode().rstrip("=")
open(path, "w").write(f"{prefix}:{enc}\n")
PY
}
HOOK='{"session_id":"a0000000-0000-4000-8000-0000000000fb","cwd":"/tmp/fallback"}'

REWRITE $CLOSED
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "control: only the closed address reaches nothing" '[[ $OUT != *muster_session* ]]'

REWRITE $CLOSED $GATE
check "the pairing shows both addresses" '[[ $(T $BIN/muster-shim pairing) == *16499* && $(T $BIN/muster-shim pairing) == *16455* ]]'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "a hook falls through to the second address" '[[ $OUT == *muster_session* ]]'
check "the tunnel falls through too (doctor's handshake)" '[[ $(T $BIN/muster-shim doctor 2>&1) == *"ok   daemon"* ]]'
START=$(python3 -I -c 'import time; print(time.time())')
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
SECS=$(python3 -I -c "import time; print(time.time() - $START)")
# A refused port fails fast on loopback, so the time alone proves little: the
# hook must also have reached the daemon, which its session block shows.
check "the last good address is tried first next time (< 1 s)" '[[ $OUT == *muster_session* ]] && (( SECS < 1 ))'

# A blackhole (TEST-NET-1, RFC 5737): a connect there hangs until its timeout
# instead of being refused, as an away-from-home LAN address would. The first
# fall-through pays that timeout once; after that the last good address goes
# first, so a hook is quick again.
REWRITE 192.0.2.1:6255 $GATE
OUT=$(echo $HOOK | perl -e 'alarm 30; exec @ARGV' $BIN/muster-shim claude-hook session-start)
check "a hook falls through past a blackholed first address" '[[ $OUT == *muster_session* ]]'
START=$(python3 -I -c 'import time; print(time.time())')
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
SECS=$(python3 -I -c "import time; print(time.time() - $START)")
check "and next time it doesn't wait on the blackhole again (< 1 s)" '[[ $OUT == *muster_session* ]] && (( SECS < 1 ))'

echo "$pass passed, $fail failed"
(( fail == 0 ))

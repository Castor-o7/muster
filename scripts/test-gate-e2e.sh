#!/bin/zsh
# End-to-end check of the Muster gate with the real binaries, in throwaway
# dirs and spare ports. Build first (`cargo build`), then run this script.
# Needs zsh, curl, nc and python3.
set -u
BIN=$(cd "$(dirname "$0")/.." && pwd)/target/debug
WORK=$(mktemp -d "${TMPDIR:-/tmp}/muster-gate-e2e.XXXX")
export MUSTER_STATE_DIR=$WORK/state
export MUSTER_BIND=127.0.0.1:16255 MUSTER_HOOK_BIND=127.0.0.1:16256 MUSTER_INTERNAL_BIND=127.0.0.1:16257
unset MUSTER_BASE_URL MUSTER_DAEMON_ADDRESS
T() { perl -e 'alarm 10; exec @ARGV' "$@"; }
pass=0; fail=0
check() { if eval "$2"; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1"; fail=$((fail+1)); fi; }

RUST_LOG=info $BIN/muster-daemon >$WORK/daemon.log 2>&1 &
DPID=$!
trap 'kill $DPID 2>/dev/null; wait $DPID 2>/dev/null' EXIT
for i in {1..50}; do nc -z 127.0.0.1 16255 2>/dev/null && break; sleep 0.2; done

CODE=$(T $BIN/muster-daemon pair --name e2e 2>/dev/null | awk '{print $3}')
OTHER=$(T $BIN/muster-daemon pair --name revoked 2>/dev/null | awk '{print $3}')
export XDG_CONFIG_HOME=$WORK/client
T $BIN/muster-shim pair "$CODE" >/dev/null
check "pairing stored owner-only" '[[ $(python3 -I -c "import os,sys; print(oct(os.stat(sys.argv[1]).st_mode & 0o777))" $WORK/client/muster/pairing) == 0o600 ]]'

HOOK='{"session_id":"e2e-hook","cwd":"/tmp/demo"}'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "paired hook gets its session block over TLS" '[[ $OUT == *muster_session* ]]'

# Raw requests straight at the gate.
check "plain HTTP (no TLS) is refused" '! curl -s -m 3 http://127.0.0.1:16255/hook/session-start -d x | grep -q .'
check "TLS without a token gets 401" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'
check "TLS with a wrong token gets 401" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer nope" -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'

REVOKED_TOKEN=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])' "$OTHER")
check "a second machine's token works" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 200 ]]'
T $BIN/muster-daemon unpair revoked >/dev/null
check "and stops working once unpaired" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'

TOKEN=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])' "$CODE")
WS=$(curl -sk -m 2 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "https://127.0.0.1:16255/myko?token=$TOKEN" 2>/dev/null)
check "WebSocket upgrade with a valid token reaches myko (101)" '[[ $WS == 101 ]]'
WS=$(curl -sk -m 2 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "https://127.0.0.1:16255/myko?token=$REVOKED_TOKEN" 2>/dev/null)
check "WebSocket upgrade with a revoked token gets 401" '[[ $WS == 401 ]]'

# The MCP shim itself: connects through its loopback tunnel.
echo "... running the MCP shim for 6s"
(sleep 6) | env CLAUDE_CODE_SESSION_ID=e2e-ws-session RUST_LOG=info perl -e "alarm 10; exec @ARGV" $BIN/muster-shim >/dev/null 2>$WORK/shim.log
echo "... shim exited with $?"
check "MCP shim connects through the TLS tunnel" 'grep -q "connected to ws://127.0.0.1:[0-9]*/myko — (re)sending session" $WORK/shim.log'
check "token never appears in the shim's log" '! grep -q "token=" $WORK/shim.log'
check "token never appears in the daemon's log" '! grep -qF "$TOKEN" $WORK/daemon.log'

# A pairing whose fingerprint doesn't match: the hook must get nothing.
BAD=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; d=json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4))); d["fingerprint"]="0"*64; print("muster-pair:v1:"+base64.urlsafe_b64encode(json.dumps(d).encode()).decode().rstrip("="))' "$CODE")
T $BIN/muster-shim pair "$BAD" >/dev/null
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "wrong certificate fingerprint: hook gets nothing" '[[ -z $OUT ]]'

# Unpaired, on the daemon's own host: the shim falls back to myko's loopback port.
mkdir -p $WORK/unpaired
(sleep 4) | env XDG_CONFIG_HOME=$WORK/unpaired MUSTER_DAEMON_ADDRESS=ws://127.0.0.1:16257 CLAUDE_CODE_SESSION_ID=e2e-local RUST_LOG=info perl -e "alarm 8; exec @ARGV" $BIN/muster-shim >/dev/null 2>$WORK/local.log
check "unpaired shim on the daemon host connects to the internal port" 'grep -q "connected to ws://127.0.0.1:16257/myko" $WORK/local.log'

echo "--- $pass passed, $fail failed"
rm -rf "$WORK/client" "$WORK/state"
echo "--- daemon log (gate lines):"; grep -E "gate|rejected" $WORK/daemon.log | sed -E 's/fingerprint [0-9a-f]+/fingerprint …/' | head -12
[[ $fail == 0 ]]

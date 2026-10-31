#!/bin/zsh
# End-to-end check of the Muster gate with the real binaries, in throwaway
# dirs and spare ports. Build first (`cargo build`), then run this script.
# Needs zsh, curl, nc and python3.
set -u
BIN=$(cd "$(dirname "$0")/.." && pwd)/target/debug
WORK=$(mktemp -d "${TMPDIR:-/tmp}/muster-gate-e2e.XXXX")
export MUSTER_STATE_DIR=$WORK/state
export MUSTER_ACK_LEASE_SECS=2
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

HOOK='{"session_id":"a0000000-0000-4000-8000-000000000004","cwd":"/tmp/demo"}'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "paired hook gets its session block over TLS" '[[ $OUT == *muster_session* ]]'

# Raw requests straight at the gate.
check "plain HTTP (no TLS) is refused" '! curl -s -m 3 http://127.0.0.1:16255/hook/session-start -d x | grep -q .'
check "TLS without a token gets 401" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'
check "TLS with a wrong token gets 401" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer nope" -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'

REVOKED_TOKEN=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])' "$OTHER")
OTHER_HOOK='{"session_id":"a0000000-0000-4000-8000-000000000003","cwd":"/tmp/demo"}'
check "a second machine's token works" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST https://127.0.0.1:16255/hook/session-start -d "$OTHER_HOOK") == 200 ]]'
check "a second machine can't read the first machine's mail" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1" -d "$HOOK") == 403 ]]'
check "nor by forging a machine name" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1&machine=e2e" -d "$HOOK") == 403 ]]'
check "the first machine still can" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $(python3 -I -c '"'"'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])'"'"' "$CODE")" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1" -d "$HOOK") == 200 ]]'
T $BIN/muster-daemon unpair revoked >/dev/null
check "and stops working once unpaired" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'

TOKEN=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])' "$CODE")
WS=$(curl -sk -m 2 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "https://127.0.0.1:16255/myko?token=$TOKEN" 2>/dev/null)
check "WebSocket upgrade with a valid token reaches myko (101)" '[[ $WS == 101 ]]'
WS=$(curl -sk -m 2 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "https://127.0.0.1:16255/myko?token=$REVOKED_TOKEN" 2>/dev/null)
check "WebSocket upgrade with a revoked token gets 401" '[[ $WS == 401 ]]'

# The MCP shim itself: connects through its loopback tunnel.
echo "... running the MCP shim for 6s"
(sleep 6) | env CLAUDE_CODE_SESSION_ID=a0000000-0000-4000-8000-000000000001 RUST_LOG=info perl -e "alarm 10; exec @ARGV" $BIN/muster-shim >/dev/null 2>$WORK/shim.log
echo "... shim exited with $?"
check "MCP shim connects through the TLS tunnel" 'grep -q "connected to ws://127.0.0.1:[0-9]*/myko — (re)sending session" $WORK/shim.log'
check "token never appears in the shim's log" '! grep -q "token=" $WORK/shim.log'
check "token never appears in the daemon's log" '! grep -qF "$TOKEN" $WORK/daemon.log'

check "the shim's real traffic isn't refused" '! grep -q "refused e2e" $WORK/daemon.log'

# Raw frames from a second machine, over TLS, trying to bypass the commands.
ATTACK=$WORK/attack.py
cat >$ATTACK <<'PY'
import base64, json, os, socket, ssl, sys, time
token, frames = sys.argv[1], sys.argv[2:]
ctx = ssl.create_default_context(); ctx.check_hostname = False; ctx.verify_mode = ssl.CERT_NONE
s = ctx.wrap_socket(socket.create_connection(("127.0.0.1", 16255), 5))
s.sendall(("GET /myko?token=%s HTTP/1.1\r\nHost: h\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n"
           "Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n" % token).encode())
assert s.recv(4096).startswith(b"HTTP/1.1 101")
for f in frames:
    data = f.encode(); mask = os.urandom(4)
    head = bytes([0x81, 0x80 | 126]) + len(data).to_bytes(2, "big") + mask
    s.sendall(head + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))
time.sleep(1)
PY
T $BIN/muster-daemon pair --name attacker >$WORK/attacker.code 2>/dev/null
ATTACKER=$(python3 -I -c 'import sys,json,base64; s=open(sys.argv[1]).read().split()[2].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])' $WORK/attacker.code)
EVENT='{"event":"ws:m:event","data":{"item":{"id":"m1"},"changeType":"DEL","itemType":"Message","createdAt":"t"}}'
SEND='{"event":"ws:m:command","data":{"command":{"asSession":"a0000000-0000-4000-8000-000000000001","toSessionId":"x","body":"forged"},"commandId":"SendMessage"}}'
QUERY='{"event":"ws:m:query","data":{"query":{"tx":"t"},"queryId":"GetAllMessages","queryItemType":"Message"}}'
python3 -I $ATTACK "$ATTACKER" "$EVENT" "$SEND" "$QUERY"
check "a raw write to another entity is refused" 'grep -q "refused attacker: event on Message" $WORK/daemon.log'
check "sending as another machine's session is refused" 'grep -q "refused attacker: SendMessage for session a0000000-0000-4000-8000-000000000001" $WORK/daemon.log'
check "reading all messages is refused" 'grep -q "refused attacker: query GetAllMessages" $WORK/daemon.log'
curl -s -m 3 -o /dev/null -X POST http://127.0.0.1:16256/hook/session-start -d '{"session_id":"a0000000-0000-4000-8000-000000000002","cwd":"/tmp/demo"}'
check "a paired machine can't take a session on the daemon's own host" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1" -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000002\",\"cwd\":\"/tmp/demo\"}") == 403 ]]'
ARRAY='{"event":"ws:m:command","data":{"command":[null,null,null,false,false,true,null,null,"a0000000-0000-4000-8000-000000000001"],"commandId":"ReadMessages"}}'
python3 -I $ATTACK "$ATTACKER" "$ARRAY" '{"event":"ws:m:event","data":{"item":{"id":"m","id\u0000":"a0000000-0000-4000-8000-000000000001"},"changeType":"DEL","itemType":"Session","createdAt":"t"}}'
check "a positional command body is refused" 'grep -q "refused attacker: command ReadMessages body" $WORK/daemon.log'
check "a NUL-suffixed key is refused" 'grep -q "refused attacker: NUL" $WORK/daemon.log'
check "a session id posing as a nickname is refused" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST "https://127.0.0.1:16255/hook/session-register" -d "{\"session_id\":\"copper-komodo\",\"cwd\":\"/tmp\"}") == 403 ]]'
check "myko's other routes are closed to paired machines" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:16255/myko/mcp -d "{}") == 403 ]]'

# Messaging from a terminal: reaches a named team's idle session, labelled.
WAITSTATUS() { curl -sk -m 5 -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=2" -d "$HOOK"; }
OUT=$(T $BIN/muster-shim tell demo hello from e2e 2>&1); RC=$?
check "tell sends to a team and reports each session" '[[ $RC == 0 && $OUT == *" sent, 0 not sent"* ]]'
MAIL=$(WAITSTATUS)
check "the idle session receives it, labelled with the machine" '[[ $MAIL == *"from a terminal on e2e"* && $MAIL == *"hello from e2e"* ]]'
OUT=$(T $BIN/muster-shim tell nobody-here hi 2>&1); RC=$?
check "tell refuses an unknown name" '[[ $RC != 0 && $OUT == *"no team or session named"* ]]'
OUT=$(T $BIN/muster-shim all to everyone 2>&1); RC=$?
check "all sends to every active session" '[[ $RC == 0 && $OUT == *" sent, 0 not sent"* ]]'
MAIL=$(WAITSTATUS)
check "and that wakes the idle session too" '[[ $MAIL == *"to everyone"* ]]'
OUT=$(T $BIN/muster-shim all 2>&1); RC=$?
check "all refuses an empty message" '[[ $RC != 0 ]]'
sleep 1
OUT=$(T $BIN/muster-shim all last one 2>&1)
check "a terminal doesn't message itself" '[[ $OUT != *"no team"* ]]'

# A dropped path: mail handed out but never acked comes back, and an ack ends it.
RAWWAIT() { curl -sk -m 5 -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1&ack=client" -d "$HOOK"; }
sleep 3; while [[ -n $(WAITSTATUS) ]]; do :; done
T $BIN/muster-shim tell demo dropped path >/dev/null 2>&1
MAIL=$(RAWWAIT)
check "a client-acked wait hands out the mail" '[[ $MAIL == *"dropped path"* ]]'
MAIL=$(RAWWAIT)
check "another reader isn't offered it while that's in flight" '[[ $MAIL != *"dropped path"* ]]'
sleep 3
HEAD=$(curl -sk -m 5 -D - -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1&ack=client" -d "$HOOK")
check "with no ack the mail comes back" '[[ $HEAD == *"dropped path"* ]]'
ID=$(print -r -- "$HEAD" | tr -d '\r' | awk -F': ' 'tolower($1)=="x-muster-ack"{print $2}' | cut -d, -f1)
curl -sk -m 5 -o /dev/null -H "Authorization: Bearer $TOKEN" -X POST https://127.0.0.1:16255/hook/ack -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000004\",\"message_ids\":[\"$ID\"]}"
sleep 3
MAIL=$(RAWWAIT)
check "and an ack ends it" '[[ $MAIL != *"dropped path"* ]]'

# A hook port that can't bind stops the daemon, rather than leaving hooks dead.
env MUSTER_STATE_DIR=$WORK/state2 MUSTER_BIND=127.0.0.1:16355 MUSTER_INTERNAL_BIND=127.0.0.1:16357 RUST_LOG=error perl -e 'alarm 10; exec @ARGV' $BIN/muster-daemon >$WORK/bind.log 2>&1; RC=$?
check "a failed hook bind stops the daemon and says why" '[[ $RC != 0 && $RC != 142 ]] && grep -q "hook listener on 127.0.0.1:16256" $WORK/bind.log'

# A pairing whose fingerprint doesn't match: the hook must get nothing.
BAD=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; d=json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4))); d["fingerprint"]="0"*64; print("muster-pair:v1:"+base64.urlsafe_b64encode(json.dumps(d).encode()).decode().rstrip("="))' "$CODE")
T $BIN/muster-shim pair "$BAD" >/dev/null
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "wrong certificate fingerprint: hook gets nothing" '[[ -z $OUT ]]'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook wait 2>&1); RC=$?
check "wrong certificate fingerprint: waiter tells the session" '[[ $RC == 2 && $OUT == *"certificate"* ]]'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook wait 2>&1); RC=$?
check "and only once" '[[ $RC == 0 && -z $OUT ]]'

# Unpaired, on the daemon's own host: the shim falls back to myko's loopback port.
mkdir -p $WORK/unpaired
# HOME too, so a real pairing on the machine running this can't be picked up.
(sleep 4) | env HOME=$WORK/unpaired XDG_CONFIG_HOME=$WORK/unpaired MUSTER_DAEMON_ADDRESS=ws://127.0.0.1:16257 CLAUDE_CODE_SESSION_ID=e2e-local RUST_LOG=info perl -e "alarm 8; exec @ARGV" $BIN/muster-shim >/dev/null 2>$WORK/local.log
check "unpaired shim on the daemon host connects to the internal port" 'grep -q "connected to ws://127.0.0.1:16257/myko" $WORK/local.log'

echo "--- $pass passed, $fail failed"
rm -rf "$WORK/client" "$WORK/state"
echo "--- daemon log (gate lines):"; grep -E "gate|rejected" $WORK/daemon.log | sed -E 's/fingerprint [0-9a-f]+/fingerprint …/' | head -12
[[ $fail == 0 ]]

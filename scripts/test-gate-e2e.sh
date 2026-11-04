#!/bin/zsh
# End-to-end check of the Muster gate with the real binaries, in throwaway
# dirs and spare ports. Build first (`cargo build`), then run this script.
# Needs zsh, curl, nc and python3.
set -u
BIN=$(cd "$(dirname "$0")/.." && pwd)/target/debug
WORK=$(mktemp -d "${TMPDIR:-/tmp}/muster-gate-e2e.XXXX")
export MUSTER_STATE_DIR=$WORK/state
export MUSTER_ACK_LEASE_SECS=2
export MUSTER_BIND=127.0.0.1:16255 MUSTER_HOOK_BIND=127.0.0.1:16256 MUSTER_INTERNAL_BIND=127.0.0.1:16257 MUSTER_MYKO_BIND=127.0.0.1:16258
# A small per-machine cap, so the flood check below can reach it (MUS-9).
export MUSTER_MAX_CONN_PER_MACHINE=8
unset MUSTER_BASE_URL MUSTER_DAEMON_ADDRESS
T() { perl -e 'alarm 10; exec @ARGV' "$@"; }
pass=0; fail=0
check() { if eval "$2"; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1"; fail=$((fail+1)); fi; }

# The daemon writes its local token under its own config dir (MUS-7), so a
# run here plants nothing in the real one.
XDG_CONFIG_HOME=$WORK/daemon RUST_LOG=info $BIN/muster-daemon >$WORK/daemon.log 2>&1 &
DPID=$!
trap 'kill $DPID 2>/dev/null; wait $DPID 2>/dev/null' EXIT
for i in {1..50}; do nc -z 127.0.0.1 16255 2>/dev/null && break; sleep 0.2; done

LOCAL_TOKEN=$(<$WORK/daemon/muster/local-token)
check "local token stored owner-only" '[[ $(python3 -I -c "import os,sys; print(oct(os.stat(sys.argv[1]).st_mode & 0o777))" $WORK/daemon/muster/local-token) == 0o600 ]]'
CODE=$(T $BIN/muster-daemon pair --name e2e 2>/dev/null | awk '{print $3}')
OTHER=$(T $BIN/muster-daemon pair --name revoked 2>/dev/null | awk '{print $3}')
export XDG_CONFIG_HOME=$WORK/client
# A code is a one-time invite (MUS-8); this exchanges one at the gate, as
# `muster-shim pair` does, and prints the token it gets.
REDEEM() {
  local inv=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["invite"])' "$1")
  curl -sk -m 5 -X POST -H "Authorization: Bearer $inv" https://127.0.0.1:16255/pair | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["token"])'
}
check "a pairing code carries no token" '[[ $CODE == muster-pair:v2:* ]] && python3 -I -c "import sys,json,base64; s=sys.argv[1].split(\":\",2)[2]; sys.exit(\"token\" in json.loads(base64.urlsafe_b64decode(s+\"=\"*(-len(s)%4))))" "$CODE"'
T $BIN/muster-shim pair "$CODE" >/dev/null
check "a used code is refused" '! T $BIN/muster-shim pair "$CODE" >/dev/null 2>&1'
EXPIRING=$(T $BIN/muster-daemon pair --name expiring --expires 1s 2>/dev/null | awk '{print $3}')
sleep 2
mkdir -p $WORK/expiring
check "an expired code is refused" '! env XDG_CONFIG_HOME=$WORK/expiring HOME=$WORK/expiring T $BIN/muster-shim pair "$EXPIRING" >/dev/null 2>&1 && [[ ! -e $WORK/expiring/muster/pairing ]]'
check "and gets no token" '! T $BIN/muster-daemon pairs | grep -qx expiring'
check "pairing stored owner-only" '[[ $(python3 -I -c "import os,sys; print(oct(os.stat(sys.argv[1]).st_mode & 0o777))" $WORK/client/muster/pairing) == 0o600 ]]'

HOOK='{"session_id":"a0000000-0000-4000-8000-000000000004","cwd":"/tmp/demo"}'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "paired hook gets its session block over TLS" '[[ $OUT == *muster_session* ]]'

# Raw requests straight at the gate.
check "plain HTTP (no TLS) is refused" '! curl -s -m 3 http://127.0.0.1:16255/hook/session-start -d x | grep -q .'
check "TLS without a token gets 401" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'
check "TLS with a wrong token gets 401" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer nope" -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'

REVOKED_TOKEN=$(REDEEM "$OTHER")
OTHER_HOOK='{"session_id":"a0000000-0000-4000-8000-000000000003","cwd":"/tmp/demo"}'
check "a second machine's token works" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST https://127.0.0.1:16255/hook/session-start -d "$OTHER_HOOK") == 200 ]]'
check "a second machine can't read the first machine's mail" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1" -d "$HOOK") == 403 ]]'
check "nor by forging a machine name" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1&machine=e2e" -d "$HOOK") == 403 ]]'
check "the first machine still can" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $(python3 -I -c '"'"'import sys,json,base64; s=open(sys.argv[1]).read().strip().split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])'"'"' $WORK/client/muster/pairing)" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1" -d "$HOOK") == 200 ]]'
T $BIN/muster-daemon unpair revoked >/dev/null
check "and stops working once unpaired" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 401 ]]'

# The token now lives only in the stored pairing (a v1 code, owner-only).
TOKEN=$(python3 -I -c 'import sys,json,base64; s=open(sys.argv[1]).read().strip().split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])' $WORK/client/muster/pairing)
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
ATTACKER=$(REDEEM "$(awk '{print $3}' $WORK/attacker.code)")
EVENT='{"event":"ws:m:event","data":{"item":{"id":"m1"},"changeType":"DEL","itemType":"Message","createdAt":"t"}}'
SEND='{"event":"ws:m:command","data":{"command":{"asSession":"a0000000-0000-4000-8000-000000000001","toSessionId":"x","body":"forged"},"commandId":"SendMessage"}}'
QUERY='{"event":"ws:m:query","data":{"query":{"tx":"t"},"queryId":"GetAllMessages","queryItemType":"Message"}}'
python3 -I $ATTACK "$ATTACKER" "$EVENT" "$SEND" "$QUERY"
check "a raw write to another entity is refused" 'grep -q "refused attacker: event on Message" $WORK/daemon.log'
check "sending as another machine's session is refused" 'grep -q "refused attacker: SendMessage for session a0000000-0000-4000-8000-000000000001" $WORK/daemon.log'
check "reading all messages is refused" 'grep -q "refused attacker: query GetAllMessages" $WORK/daemon.log'
# The daemon's own host: a process without the local token can't act, a browser
# can't at all, and one with the token claims the session as local (MUS-7).
LOCAL_HOOK='{"session_id":"a0000000-0000-4000-8000-000000000002","cwd":"/tmp/demo"}'
check "a local process without the token gets 401" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -X POST http://127.0.0.1:16256/hook/session-start -d "$LOCAL_HOOK") == 401 ]]'
check "a browser origin gets 403 at the hook listener" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -H "Origin: https://evil.example" -H "Authorization: Bearer $LOCAL_TOKEN" -X POST http://127.0.0.1:16256/hook/session-start -d "$LOCAL_HOOK") == 403 ]]'
check "a websocket at the hook listener gets 403" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "http://127.0.0.1:16256/hook/wait?token=$LOCAL_TOKEN") == 403 ]]'
check "the local gate refuses a websocket without the token" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" http://127.0.0.1:16257/myko) == 401 ]]'
check "and one from a browser with the token" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Origin: https://evil.example" -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "http://127.0.0.1:16257/myko?token=$LOCAL_TOKEN") == 403 ]]'
check "but admits a local process with it (101)" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "http://127.0.0.1:16257/myko?token=$LOCAL_TOKEN") == 101 ]]'
# myko's own listener answers only what a gate forwarded (MUS-59).
check "myko's own port refuses a browser websocket" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Origin: https://evil.example" -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" http://127.0.0.1:16258/myko) == 403 ]]'
check "and a browser's MCP call" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -H "Origin: https://evil.example" -H "Content-Type: application/json" -X POST -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}" http://127.0.0.1:16258/myko/mcp) == 403 ]]'
check "and a local process that skips the gates" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" http://127.0.0.1:16258/myko) == 401 ]]'
check "a local process with the token acts" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $LOCAL_TOKEN" -X POST http://127.0.0.1:16256/hook/session-start -d "$LOCAL_HOOK") == 200 ]]'
check "a paired machine can't take a session on the daemon's own host" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1" -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000002\",\"cwd\":\"/tmp/demo\"}") == 403 ]]'
READAS='{"event":"ws:m:command","data":{"command":{"asSession":"a0000000-0000-4000-8000-000000000001","unread":true},"commandId":"ReadMessages"}}'
ACKAS='{"event":"ws:m:command","data":{"command":{"asSession":"a0000000-0000-4000-8000-000000000001","messageIds":["m1"]},"commandId":"AckMessages"}}'
python3 -I $ATTACK "$ATTACKER" "$READAS" "$ACKAS"
check "reading as another machine's session is refused" 'grep -q "refused attacker: ReadMessages for session a0000000-0000-4000-8000-000000000001" $WORK/daemon.log'
check "clearing another machine's mail is refused" 'grep -q "refused attacker: AckMessages for session a0000000-0000-4000-8000-000000000001" $WORK/daemon.log'
ARRAY='{"event":"ws:m:command","data":{"command":[null,null,null,false,false,true,null,null,"a0000000-0000-4000-8000-000000000001"],"commandId":"ReadMessages"}}'
python3 -I $ATTACK "$ATTACKER" "$ARRAY" '{"event":"ws:m:event","data":{"item":{"id":"m","id\u0000":"a0000000-0000-4000-8000-000000000001"},"changeType":"DEL","itemType":"Session","createdAt":"t"}}'
check "a positional command body is refused" 'grep -q "refused attacker: command ReadMessages body" $WORK/daemon.log'
check "a NUL-suffixed key is refused" 'grep -q "refused attacker: NUL" $WORK/daemon.log'
check "a session id posing as a nickname is refused" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST "https://127.0.0.1:16255/hook/session-register" -d "{\"session_id\":\"copper-komodo\",\"cwd\":\"/tmp\"}") == 403 ]]'
check "myko's other routes are closed to paired machines" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:16255/myko/mcp -d "{}") == 403 ]]'
# One machine holding all its connections is turned away; another isn't (MUS-9).
FLOOD=()
for n in {1..8}; do
  curl -sk -m 12 -o /dev/null -H "Authorization: Bearer $ATTACKER" -X POST "https://127.0.0.1:16255/hook/wait?timeout=8" -d "{\"session_id\":\"a0000000-0000-4000-8000-0000000009$(printf %02d $n)\",\"cwd\":\"/tmp/flood\"}" &
  FLOOD+=($!)
done
sleep 2
check "a machine past its connection cap gets 503" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:16255/hook/session-start -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000999\",\"cwd\":\"/tmp/flood\"}") == 503 ]]'
check "while another machine keeps working" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $TOKEN" -X POST https://127.0.0.1:16255/hook/session-start -d "$HOOK") == 200 ]]'
wait $FLOOD
check "and the flooding machine gets back in once its connections close" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:16255/hook/session-start -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000999\",\"cwd\":\"/tmp/flood\"}") == 200 ]]'

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

# Reading the roster from a program: read-only, and claims are kept apart.
ROSTER() { T $BIN/muster-shim roster 2>/dev/null; }
FIELD() { print -r -- "$1" | python3 -I -c 'import json,sys; r=[s for s in json.load(sys.stdin)["sessions"] if s["session_id"]==sys.argv[1]][0]; print(eval(sys.argv[2],{"r":r}))' $NEVER "$2"; }
SNAP() { print -r -- "$1" | python3 -I -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps(sorted(d["sessions"],key=lambda s:s["session_id"]),sort_keys=True))'; }
# A session with no mail that has never waited, because a wait with unread mail
# returns at once, and a wait that timed out still counts until it expires.
NEVER=a0000000-0000-4000-8000-0000000000ee
curl -sk -m 3 -o /dev/null -H "Authorization: Bearer $TOKEN" -X POST https://127.0.0.1:16255/hook/session-start -d "{\"session_id\":\"$NEVER\",\"cwd\":\"/tmp/demo\"}"
OUT=$(ROSTER)
check "roster reports a session with no waiter as none" '[[ $(FIELD "$OUT" "r[\"wake\"]") == none ]]'
check "and keeps what it declares apart" '[[ $(FIELD "$OUT" "\"current_task\" in r[\"declared\"] and \"current_task\" not in r[\"observed\"]") == True ]]'
AGAIN=$(ROSTER)
check "reading the roster changes nothing, and adds no session" '[[ -n $OUT && $(SNAP "$OUT") == $(SNAP "$AGAIN") ]]'
curl -sk -m 8 -o /dev/null -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=6" -d "{\"session_id\":\"$NEVER\",\"cwd\":\"/tmp/demo\"}" &
HELD=$!
sleep 1
OUT=$(ROSTER)
check "roster reports a held wait as observed" '[[ $(FIELD "$OUT" "r[\"wake\"]") == waiting && $(FIELD "$OUT" "r[\"observed\"][\"waiting\"]") == True ]]'
kill $HELD 2>/dev/null; wait $HELD 2>/dev/null

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

# An ack marks mail read, so a forged one would lose it.
T $BIN/muster-shim tell demo forged ack >/dev/null 2>&1
HEAD=$(curl -sk -m 5 -D - -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=1&ack=client" -d "$HOOK")
ID=$(print -r -- "$HEAD" | tr -d '\r' | awk -F': ' 'tolower($1)=="x-muster-ack"{print $2}' | cut -d, -f1)
check "another machine's ack is refused" '[[ -n $ID && $(curl -sk -m 5 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:16255/hook/ack -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000004\",\"message_ids\":[\"$ID\"]}") == 403 ]]'
sleep 3
MAIL=$(RAWWAIT)
check "and the mail still comes" '[[ $MAIL == *"forged ack"* ]]'

# Unpairing ends a machine's open long-poll, rather than leaving it running.
LATE=$(T $BIN/muster-daemon pair --name late 2>/dev/null | awk '{print $3}')
LATE_TOKEN=$(REDEEM "$LATE")
curl -sk -m 60 -o /dev/null -H "Authorization: Bearer $LATE_TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=50" -d '{"session_id":"a0000000-0000-4000-8000-000000000009","cwd":"/tmp/late"}' &
LATE_PID=$!
sleep 1; kill -0 $LATE_PID 2>/dev/null; ALIVE=$?
T $BIN/muster-daemon unpair late >/dev/null; sleep 7
check "unpair closes an open long-poll within seconds" '[[ $ALIVE == 0 ]] && ! kill -0 $LATE_PID 2>/dev/null'
kill $LATE_PID 2>/dev/null; wait $LATE_PID 2>/dev/null

# And its open WebSocket, the connection a revoked machine would keep longest.
LATE=$(T $BIN/muster-daemon pair --name late-ws 2>/dev/null | awk '{print $3}')
LATE_TOKEN=$(REDEEM "$LATE")
python3 -I -c '
import socket, ssl, sys
ctx = ssl.create_default_context(); ctx.check_hostname = False; ctx.verify_mode = ssl.CERT_NONE
s = ctx.wrap_socket(socket.create_connection(("127.0.0.1", 16255), 5))
s.sendall(("GET /myko?token=%s HTTP/1.1\r\nHost: h\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n" % sys.argv[1]).encode())
assert s.recv(4096).startswith(b"HTTP/1.1 101")
s.settimeout(20)
while s.recv(4096): pass
' "$LATE_TOKEN" 2>/dev/null &
LATE_PID=$!
sleep 1; kill -0 $LATE_PID 2>/dev/null; ALIVE=$?
T $BIN/muster-daemon unpair late-ws >/dev/null; sleep 7
check "unpair closes an open WebSocket within seconds" '[[ $ALIVE == 0 ]] && ! kill -0 $LATE_PID 2>/dev/null'
kill $LATE_PID 2>/dev/null; wait $LATE_PID 2>/dev/null

# An oversized body is refused before it's stored.
BIG=$(python3 -I -c 'print("x"*70000)')
OUT=$(T $BIN/muster-shim tell demo "$BIG" 2>&1); RC=$?
check "an oversized message is refused" '[[ $RC != 0 && $OUT == *"byte limit"* ]]'

# A lost reply: the send reaches the daemon, its response is dropped, the shim times out,
# and retrying with the key from the error doesn't deliver twice.
LOST=$WORK/lost.py
cat >$LOST <<'PY'
import json, os, re, select, socket, subprocess, sys, threading, time, urllib.request
shim, env_home = sys.argv[1], sys.argv[2]
drop = threading.Event()
def read_n(s, n):
    b = b""
    while len(b) < n:
        c = s.recv(n - len(b))
        if not c: raise EOFError
        b += c
    return b
def relay_up(c, u):
    try:
        while True:
            d = c.recv(65536)
            if not d: break
            u.sendall(d)
    except OSError: pass
def relay_down(u, c):
    # Whole frames, so dropping some leaves the stream valid.
    try:
        head = b""
        while b"\r\n\r\n" not in head: head += u.recv(1)
        c.sendall(head)
        while True:
            h = read_n(u, 2); n = h[1] & 127; ext = b""
            if n == 126: ext = read_n(u, 2); n = int.from_bytes(ext, "big")
            elif n == 127: ext = read_n(u, 8); n = int.from_bytes(ext, "big")
            body = read_n(u, n)
            if not drop.is_set(): c.sendall(h + ext + body)
    except (OSError, EOFError): pass
lst = socket.socket(); lst.bind(("127.0.0.1", 0)); lst.listen(); port = lst.getsockname()[1]
def serve():
    while True:
        c, _ = lst.accept()
        u = socket.create_connection(("127.0.0.1", 16257))
        threading.Thread(target=relay_up, args=(c, u), daemon=True).start()
        threading.Thread(target=relay_down, args=(u, c), daemon=True).start()
threading.Thread(target=serve, daemon=True).start()
env = dict(os.environ, HOME=env_home, XDG_CONFIG_HOME=env_home, MUSTER_DAEMON_ADDRESS="ws://127.0.0.1:%d" % port,
           CLAUDE_CODE_SESSION_ID="e2e-lost-sender")
p = subprocess.Popen([shim], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, text=True, bufsize=1)
def rpc(i, method, params, wait):
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}) + "\n"); p.stdin.flush()
    end = time.time() + wait
    while time.time() < end:
        if select.select([p.stdout], [], [], 1)[0]:
            m = json.loads(p.stdout.readline())
            if m.get("id") == i: return m
    raise SystemExit("no reply to " + method)
rpc(1, "initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}}, 20)
time.sleep(3)
send = {"to": "a0000000-0000-4000-8000-00000000000b", "body": "lost reply body"}
drop.set()
first = json.dumps(rpc(2, "tools/call", {"name": "send_message", "arguments": send}, 25))
drop.clear()
key = re.search(r"idempotency_key `([^`]+)`", first)
print("TIMEOUT" if "timed out" in first and key else "NO-TIMEOUT: " + first[:300])
if key:
    second = json.dumps(rpc(3, "tools/call", {"name": "send_message", "arguments": dict(send, idempotency_key=key.group(1))}, 15))
    print("RETRY-OK" if "message_id" in second else "RETRY-FAILED: " + second[:300])
p.kill()
PY
curl -s -m 3 -o /dev/null -H "Authorization: Bearer $LOCAL_TOKEN" -X POST http://127.0.0.1:16256/hook/session-start -d '{"session_id":"a0000000-0000-4000-8000-00000000000b","cwd":"/tmp/lost"}'
# An unpaired shim on the daemon's host reads the local token from its config dir (MUS-7).
mkdir -p $WORK/lost-home/muster && cp $WORK/daemon/muster/local-token $WORK/lost-home/muster/
OUT=$(perl -e 'alarm 60; exec @ARGV' python3 -I $LOST $BIN/muster-shim $WORK/lost-home 2>&1)
check "a dropped reply times out and gives a retry key" '[[ $OUT == *TIMEOUT* && $OUT != *NO-TIMEOUT* ]]'
check "and the retry with that key succeeds" '[[ $OUT == *RETRY-OK* ]]'
sleep 3
MAIL=$(curl -s -m 5 -H "Authorization: Bearer $LOCAL_TOKEN" -X POST "http://127.0.0.1:16256/hook/wait?timeout=1" -d '{"session_id":"a0000000-0000-4000-8000-00000000000b","cwd":"/tmp/lost"}')
check "and the recipient gets it once" '[[ $(print -r -- "$MAIL" | grep -c "lost reply body") == 1 ]]'
[[ $OUT == *RETRY-OK* ]] || print -r -- "$OUT"

# A session whose connection drops while it's still waiting stays reachable (MUS-54).
KEEP=a0000000-0000-4000-8000-0000000000c1
(sleep 120) | env CLAUDE_CODE_SESSION_ID=$KEEP RUST_LOG=info $BIN/muster-shim >/dev/null 2>$WORK/keep.log &
KEEP_SHIM=$!
for i in {1..50}; do grep -q "(re)sending session" $WORK/keep.log 2>/dev/null && break; sleep 0.2; done
curl -sk -m 110 -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:16255/hook/wait?timeout=100" -d "{\"session_id\":\"$KEEP\",\"cwd\":\"/tmp/keep\"}" >$WORK/keep-wait.out &
KEEP_WAIT=$!
sleep 1
kill -9 $KEEP_SHIM
echo "... waiting 70s for the sweeper's reconnect grace"
sleep 70
check "a waiting session isn't reaped when its connection drops" '[[ $(ROSTER) == *$KEEP* ]]'
T $BIN/muster-shim tell $KEEP kept and reachable >/dev/null 2>&1
wait $KEEP_WAIT
check "and a message sent after the drop reaches its held wait" 'grep -q "kept and reachable" $WORK/keep-wait.out'

# SIGTERM ends a held wait with an empty reply marked as a restart, then exits cleanly (MUS-55).
env XDG_CONFIG_HOME=$WORK/daemon MUSTER_STATE_DIR=$WORK/state3 MUSTER_BIND=127.0.0.1:16455 MUSTER_HOOK_BIND=127.0.0.1:16456 MUSTER_INTERNAL_BIND=127.0.0.1:16457 MUSTER_MYKO_BIND=127.0.0.1:16458 RUST_LOG=info $BIN/muster-daemon >$WORK/stop.log 2>&1 &
STOP_PID=$!
for i in {1..50}; do nc -z 127.0.0.1 16456 2>/dev/null && break; sleep 0.2; done
curl -s -m 20 -D $WORK/stop-head.txt -o $WORK/stop-body.txt -H "Authorization: Bearer $LOCAL_TOKEN" -X POST "http://127.0.0.1:16456/hook/wait?timeout=15" -d '{"session_id":"a0000000-0000-4000-8000-0000000000d1","cwd":"/tmp/stop"}' &
STOP_WAIT=$!
sleep 1
STOP_AT=$SECONDS
kill -TERM $STOP_PID
wait $STOP_WAIT
wait $STOP_PID; STOP_RC=$?
check "SIGTERM ends a held wait promptly, marked as a restart" '(( SECONDS - STOP_AT < 5 )) && grep -qi "x-muster-restarting: 1" $WORK/stop-head.txt && [[ ! -s $WORK/stop-body.txt ]]'
check "and the daemon then exits cleanly" '[[ $STOP_RC == 0 ]] && grep -q "SIGTERM; ending held waits" $WORK/stop.log'

# A hook port that can't bind stops the daemon, rather than leaving hooks dead.
env XDG_CONFIG_HOME=$WORK/daemon MUSTER_STATE_DIR=$WORK/state2 MUSTER_BIND=127.0.0.1:16355 MUSTER_INTERNAL_BIND=127.0.0.1:16357 MUSTER_MYKO_BIND=127.0.0.1:16358 RUST_LOG=error perl -e 'alarm 10; exec @ARGV' $BIN/muster-daemon >$WORK/bind.log 2>&1; RC=$?
check "a failed hook bind stops the daemon and says why" '[[ $RC != 0 && $RC != 142 ]] && grep -q "hook listener on 127.0.0.1:16256" $WORK/bind.log'

# A pairing whose fingerprint doesn't match: the hook must get nothing.
BAD=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; d=json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4))); d["fingerprint"]="0"*64; print("muster-pair:v1:"+base64.urlsafe_b64encode(json.dumps(d).encode()).decode().rstrip("="))' "$(<$WORK/client/muster/pairing)")
T $BIN/muster-shim pair "$BAD" >/dev/null
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "wrong certificate fingerprint: hook gets nothing" '[[ -z $OUT ]]'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook wait 2>&1); RC=$?
check "wrong certificate fingerprint: waiter tells the session" '[[ $RC == 2 && $OUT == *"certificate"* ]]'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook wait 2>&1); RC=$?
check "and only once" '[[ $RC == 0 && -z $OUT ]]'

# Unpaired, on the daemon's own host: the shim reaches the local gate with the local token (MUS-7).
mkdir -p $WORK/unpaired/muster && cp $WORK/daemon/muster/local-token $WORK/unpaired/muster/
# HOME too, so a real pairing on the machine running this can't be picked up.
(sleep 4) | env HOME=$WORK/unpaired XDG_CONFIG_HOME=$WORK/unpaired MUSTER_DAEMON_ADDRESS=ws://127.0.0.1:16257 CLAUDE_CODE_SESSION_ID=e2e-local RUST_LOG=info perl -e "alarm 8; exec @ARGV" $BIN/muster-shim >/dev/null 2>$WORK/local.log
check "unpaired shim on the daemon host connects through the local gate" 'grep -q "connected to ws://127.0.0.1:16257/myko" $WORK/local.log'
check "and its log never shows the local token" '! grep -qF "$LOCAL_TOKEN" $WORK/local.log'

echo "--- $pass passed, $fail failed"
rm -rf "$WORK/client" "$WORK/state"
echo "--- daemon log (gate lines):"; grep -E "gate|rejected" $WORK/daemon.log | sed -E 's/fingerprint [0-9a-f]+/fingerprint …/' | head -12
[[ $fail == 0 ]]

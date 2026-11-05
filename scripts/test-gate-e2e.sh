#!/bin/zsh
# End-to-end check of the Muster gate with the real binaries, in throwaway
# dirs and spare ports. Build first (`cargo build`), then run this script.
# Needs zsh, curl, nc and python3.
set -u
BIN=$(cd "$(dirname "$0")/.." && pwd)/target/debug
WORK=$(mktemp -d "${TMPDIR:-/tmp}/muster-gate-e2e.XXXX")
# Free ports for this run, picked together so they differ, and exported for
# the Python helpers below. Two runs on one host no longer collide (MUS-88).
FREE=($(python3 -I -c 'import socket; s=[socket.socket() for _ in range(13)]; [x.bind(("127.0.0.1", 0)) for x in s]; print(*[x.getsockname()[1] for x in s])'))
export P_GATE=$FREE[1]
export P_HOOK=$FREE[2]
export P_RETIRED=$FREE[3]
export P_MYKO=$FREE[4]
export P_BIND_GATE=$FREE[5]
export P_BIND_MYKO=$FREE[6]
export P_STOP_GATE=$FREE[7]
export P_STOP_HOOK=$FREE[8]
export P_STOP_MYKO=$FREE[9]
export P_PLAIN_GATE=$FREE[10]
export P_PLAIN_HOOK=$FREE[11]
export P_PLAIN_RETIRED=$FREE[12]
export P_PLAIN_MYKO=$FREE[13]
export MUSTER_STATE_DIR=$WORK/state
export MUSTER_ACK_LEASE_SECS=2
# Everything down to the plain-mode section runs with the gate on (MUS-64).
export MUSTER_TLS=1
export MUSTER_BIND=127.0.0.1:${P_GATE} MUSTER_HOOK_BIND=127.0.0.1:${P_HOOK} MUSTER_MYKO_BIND=127.0.0.1:${P_MYKO}
# The retired 6257 alias (MUS-70), set to show the daemon ignores it and says so.
export MUSTER_INTERNAL_BIND=127.0.0.1:${P_RETIRED}
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
for i in {1..50}; do nc -z 127.0.0.1 ${P_GATE} 2>/dev/null && break; sleep 0.2; done

LOCAL_TOKEN=$(<$WORK/daemon/muster/local-token)
check "local token stored owner-only" '[[ $(python3 -I -c "import os,sys; print(oct(os.stat(sys.argv[1]).st_mode & 0o777))" $WORK/daemon/muster/local-token) == 0o600 ]]'
CODE=$(T $BIN/muster-daemon pair --name e2e 2>/dev/null | awk '{print $3}')
OTHER=$(T $BIN/muster-daemon pair --name revoked 2>/dev/null | awk '{print $3}')
export XDG_CONFIG_HOME=$WORK/client
# A code is a one-time invite (MUS-8); this exchanges one at the gate, as
# `muster-shim pair` does, and prints the token it gets.
REDEEM() {
  local inv=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["invite"])' "$1")
  curl -sk -m 5 -X POST -H "Authorization: Bearer $inv" https://127.0.0.1:${P_GATE}/pair | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["token"])'
}
check "a pairing code carries no token" '[[ $CODE == muster-pair:v2:* ]] && python3 -I -c "import sys,json,base64; s=sys.argv[1].split(\":\",2)[2]; sys.exit(\"token\" in json.loads(base64.urlsafe_b64decode(s+\"=\"*(-len(s)%4))))" "$CODE"'
T $BIN/muster-shim pair "$CODE" >/dev/null
check "a used code is refused" '! T $BIN/muster-shim pair "$CODE" >/dev/null 2>&1'
EXPIRING=$(T $BIN/muster-daemon pair --name expiring --expires 1s 2>/dev/null | awk '{print $3}')
sleep 2
mkdir -p $WORK/expiring
check "an expired code is refused" '! T env XDG_CONFIG_HOME=$WORK/expiring HOME=$WORK/expiring $BIN/muster-shim pair "$EXPIRING" >/dev/null 2>&1 && [[ ! -e $WORK/expiring/muster/pairing ]]'
check "and gets no token" '! T $BIN/muster-daemon pairs | grep -qx expiring'
check "pairing stored owner-only" '[[ $(python3 -I -c "import os,sys; print(oct(os.stat(sys.argv[1]).st_mode & 0o777))" $WORK/client/muster/pairing) == 0o600 ]]'

HOOK='{"session_id":"a0000000-0000-4000-8000-000000000004","cwd":"/tmp/demo"}'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "paired hook gets its session block over TLS" '[[ $OUT == *muster_session* ]]'

# Raw requests straight at the gate.
check "plain HTTP without the local token is refused (401)" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} http://127.0.0.1:${P_GATE}/hook/session-start -d x) == 401 ]]'
UPGRADE=(-H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==")
check "the gate's port takes a local websocket with the local token (101)" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 $UPGRADE "http://127.0.0.1:${P_GATE}/myko?token=$LOCAL_TOKEN") == 101 ]]'
check "but not one from a browser (403)" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Origin: https://evil.example" $UPGRADE "http://127.0.0.1:${P_GATE}/myko?token=$LOCAL_TOKEN") == 403 ]]'
check "TLS without a token gets 401" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -X POST https://127.0.0.1:${P_GATE}/hook/session-start -d "$HOOK") == 401 ]]'
check "TLS with a wrong token gets 401" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer nope" -X POST https://127.0.0.1:${P_GATE}/hook/session-start -d "$HOOK") == 401 ]]'

REVOKED_TOKEN=$(REDEEM "$OTHER")
OTHER_HOOK='{"session_id":"a0000000-0000-4000-8000-000000000003","cwd":"/tmp/demo"}'
check "a second machine's token works" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST https://127.0.0.1:${P_GATE}/hook/session-start -d "$OTHER_HOOK") == 200 ]]'
check "a second machine can't read the first machine's mail" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=1" -d "$HOOK") == 403 ]]'
check "nor by forging a machine name" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=1&machine=e2e" -d "$HOOK") == 403 ]]'
check "the first machine still can" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $(python3 -I -c '"'"'import sys,json,base64; s=open(sys.argv[1]).read().strip().split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])'"'"' $WORK/client/muster/pairing)" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=1" -d "$HOOK") == 200 ]]'
T $BIN/muster-daemon unpair revoked >/dev/null
check "and stops working once unpaired" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $REVOKED_TOKEN" -X POST https://127.0.0.1:${P_GATE}/hook/session-start -d "$HOOK") == 401 ]]'

# The token now lives only in the stored pairing (a v1 code, owner-only).
TOKEN=$(python3 -I -c 'import sys,json,base64; s=open(sys.argv[1]).read().strip().split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])' $WORK/client/muster/pairing)
WS=$(curl -sk -m 2 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "https://127.0.0.1:${P_GATE}/myko?token=$TOKEN" 2>/dev/null)
check "WebSocket upgrade with a valid token reaches myko (101)" '[[ $WS == 101 ]]'
WS=$(curl -sk -m 2 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "https://127.0.0.1:${P_GATE}/myko?token=$REVOKED_TOKEN" 2>/dev/null)
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
s = ctx.wrap_socket(socket.create_connection(("127.0.0.1", int(os.environ["P_GATE"])), 5))
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
check "a local process without the token gets 401" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -X POST http://127.0.0.1:${P_HOOK}/hook/session-start -d "$LOCAL_HOOK") == 401 ]]'
check "a browser origin gets 403 at the hook listener" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -H "Origin: https://evil.example" -H "Authorization: Bearer $LOCAL_TOKEN" -X POST http://127.0.0.1:${P_HOOK}/hook/session-start -d "$LOCAL_HOOK") == 403 ]]'
check "a websocket at the hook listener gets 403" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "http://127.0.0.1:${P_HOOK}/hook/wait?token=$LOCAL_TOKEN") == 403 ]]'
check "the gate's port refuses a local websocket without the token" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" http://127.0.0.1:${P_GATE}/myko) == 401 ]]'
check "the retired 6257 alias isn't bound, and setting it says so" '! nc -z 127.0.0.1 ${P_RETIRED} 2>/dev/null && grep -q "MUSTER_INTERNAL_BIND is ignored" $WORK/daemon.log'
# myko's own listener answers only what a gate forwarded (MUS-59).
check "myko's own port refuses a browser websocket" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Origin: https://evil.example" -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" http://127.0.0.1:${P_MYKO}/myko) == 403 ]]'
check "and a browser's MCP call" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -H "Origin: https://evil.example" -H "Content-Type: application/json" -X POST -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}" http://127.0.0.1:${P_MYKO}/myko/mcp) == 403 ]]'
check "and a local process that skips the gates" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" http://127.0.0.1:${P_MYKO}/myko) == 401 ]]'
check "a local process with the token acts" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $LOCAL_TOKEN" -X POST http://127.0.0.1:${P_HOOK}/hook/session-start -d "$LOCAL_HOOK") == 200 ]]'
check "a paired machine can't take a session on the daemon's own host" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=1" -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000002\",\"cwd\":\"/tmp/demo\"}") == 403 ]]'
READAS='{"event":"ws:m:command","data":{"command":{"asSession":"a0000000-0000-4000-8000-000000000001","unread":true},"commandId":"ReadMessages"}}'
ACKAS='{"event":"ws:m:command","data":{"command":{"asSession":"a0000000-0000-4000-8000-000000000001","messageIds":["m1"]},"commandId":"AckMessages"}}'
python3 -I $ATTACK "$ATTACKER" "$READAS" "$ACKAS"
check "reading as another machine's session is refused" 'grep -q "refused attacker: ReadMessages for session a0000000-0000-4000-8000-000000000001" $WORK/daemon.log'
check "clearing another machine's mail is refused" 'grep -q "refused attacker: AckMessages for session a0000000-0000-4000-8000-000000000001" $WORK/daemon.log'
ARRAY='{"event":"ws:m:command","data":{"command":[null,null,null,false,false,true,null,null,"a0000000-0000-4000-8000-000000000001"],"commandId":"ReadMessages"}}'
python3 -I $ATTACK "$ATTACKER" "$ARRAY" '{"event":"ws:m:event","data":{"item":{"id":"m","id\u0000":"a0000000-0000-4000-8000-000000000001"},"changeType":"DEL","itemType":"Session","createdAt":"t"}}'
check "a positional command body is refused" 'grep -q "refused attacker: command ReadMessages body" $WORK/daemon.log'
check "a NUL-suffixed key is refused" 'grep -q "refused attacker: NUL" $WORK/daemon.log'
check "a session id posing as a nickname is refused" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST "https://127.0.0.1:${P_GATE}/hook/session-register" -d "{\"session_id\":\"copper-komodo\",\"cwd\":\"/tmp\"}") == 403 ]]'
check "myko's other routes are closed to paired machines" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:${P_GATE}/myko/mcp -d "{}") == 403 ]]'
# One machine holding all its connections is turned away; another isn't (MUS-9).
FLOOD=()
for n in {1..8}; do
  curl -sk -m 12 -o /dev/null -H "Authorization: Bearer $ATTACKER" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=8" -d "{\"session_id\":\"a0000000-0000-4000-8000-0000000009$(printf %02d $n)\",\"cwd\":\"/tmp/flood\"}" &
  FLOOD+=($!)
done
sleep 2
check "a machine past its connection cap gets 503" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:${P_GATE}/hook/session-start -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000999\",\"cwd\":\"/tmp/flood\"}") == 503 ]]'
check "while another machine keeps working" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $TOKEN" -X POST https://127.0.0.1:${P_GATE}/hook/session-start -d "$HOOK") == 200 ]]'
wait $FLOOD
check "and the flooding machine gets back in once its connections close" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:${P_GATE}/hook/session-start -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000999\",\"cwd\":\"/tmp/flood\"}") == 200 ]]'

# Messaging from a terminal: reaches a named team's idle session, labelled.
WAITSTATUS() { curl -sk -m 5 -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=2" -d "$HOOK"; }
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
curl -sk -m 3 -o /dev/null -H "Authorization: Bearer $TOKEN" -X POST https://127.0.0.1:${P_GATE}/hook/session-start -d "{\"session_id\":\"$NEVER\",\"cwd\":\"/tmp/demo\"}"
OUT=$(ROSTER)
check "roster reports a session with no waiter as none" '[[ $(FIELD "$OUT" "r[\"wake\"]") == none ]]'
check "and keeps what it declares apart" '[[ $(FIELD "$OUT" "\"current_task\" in r[\"declared\"] and \"current_task\" not in r[\"observed\"]") == True ]]'
AGAIN=$(ROSTER)
check "reading the roster changes nothing, and adds no session" '[[ -n $OUT && $(SNAP "$OUT") == $(SNAP "$AGAIN") ]]'
curl -sk -m 8 -o /dev/null -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=6" -d "{\"session_id\":\"$NEVER\",\"cwd\":\"/tmp/demo\"}" &
HELD=$!
sleep 1
OUT=$(ROSTER)
check "roster reports a held wait as observed" '[[ $(FIELD "$OUT" "r[\"wake\"]") == waiting && $(FIELD "$OUT" "r[\"observed\"][\"waiting\"]") == True ]]'
# The operator pauses wake-ups, and the roster says so (MUS-10).
OUT=$(T $BIN/muster-daemon pause 2>&1)
check "pause says what it does" '[[ $OUT == *"wake-ups are paused"* ]]'
sleep 3
PAUSED=$(ROSTER | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["wakes_paused"])')
check "and the roster shows wake-ups paused" '[[ $PAUSED == True ]]'
T $BIN/muster-daemon resume >/dev/null
sleep 3
PAUSED=$(ROSTER | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["wakes_paused"])')
check "and on again after resume" '[[ $PAUSED == False ]]'
kill $HELD 2>/dev/null; wait $HELD 2>/dev/null

# A dropped path: mail handed out but never acked comes back, and an ack ends it.
RAWWAIT() { curl -sk -m 5 -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=1&ack=client" -d "$HOOK"; }
sleep 3; while [[ -n $(WAITSTATUS) ]]; do :; done
T $BIN/muster-shim tell demo dropped path >/dev/null 2>&1
# Both are logged, by count and session, never by body (MUS-86).
HOOKLOG() { grep -c "\[hook\] a0000000-0000-4000-8000-000000000004: $1" $WORK/daemon.log; }
REOFFERS=$(HOOKLOG "offering 1 message(s) again"); ACKS=$(HOOKLOG "client acked 1 message(s)")
MAIL=$(RAWWAIT)
check "a client-acked wait hands out the mail" '[[ $MAIL == *"dropped path"* ]]'
MAIL=$(RAWWAIT)
check "another reader isn't offered it while that's in flight" '[[ $MAIL != *"dropped path"* ]]'
sleep 3
HEAD=$(curl -sk -m 5 -D - -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=1&ack=client" -d "$HOOK")
check "with no ack the mail comes back" '[[ $HEAD == *"dropped path"* ]]'
check "and the daemon logs the re-offer" '(( $(HOOKLOG "offering 1 message(s) again") == REOFFERS + 1 ))'
ID=$(print -r -- "$HEAD" | tr -d '\r' | awk -F': ' 'tolower($1)=="x-muster-ack"{print $2}' | cut -d, -f1)
curl -sk -m 5 -o /dev/null -H "Authorization: Bearer $TOKEN" -X POST https://127.0.0.1:${P_GATE}/hook/ack -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000004\",\"message_ids\":[\"$ID\"]}"
sleep 3
MAIL=$(RAWWAIT)
check "and an ack ends it" '[[ $MAIL != *"dropped path"* ]]'
check "and the daemon logs the client ack" '(( $(HOOKLOG "client acked 1 message(s)") == ACKS + 1 ))'
check "with no message body in either line" '! grep -E "\[hook\].*(offering|client acked)" $WORK/daemon.log | grep -q "dropped path"'

# An ack marks mail read, so a forged one would lose it.
T $BIN/muster-shim tell demo forged ack >/dev/null 2>&1
HEAD=$(curl -sk -m 5 -D - -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=1&ack=client" -d "$HOOK")
ID=$(print -r -- "$HEAD" | tr -d '\r' | awk -F': ' 'tolower($1)=="x-muster-ack"{print $2}' | cut -d, -f1)
check "another machine's ack is refused" '[[ -n $ID && $(curl -sk -m 5 -o /dev/null -w %{http_code} -H "Authorization: Bearer $ATTACKER" -X POST https://127.0.0.1:${P_GATE}/hook/ack -d "{\"session_id\":\"a0000000-0000-4000-8000-000000000004\",\"message_ids\":[\"$ID\"]}") == 403 ]]'
sleep 3
MAIL=$(RAWWAIT)
check "and the mail still comes" '[[ $MAIL == *"forged ack"* ]]'

# Unpairing ends a machine's open long-poll, rather than leaving it running.
LATE=$(T $BIN/muster-daemon pair --name late 2>/dev/null | awk '{print $3}')
LATE_TOKEN=$(REDEEM "$LATE")
curl -sk -m 60 -o /dev/null -H "Authorization: Bearer $LATE_TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=50" -d '{"session_id":"a0000000-0000-4000-8000-000000000009","cwd":"/tmp/late"}' &
LATE_PID=$!
sleep 1; kill -0 $LATE_PID 2>/dev/null; ALIVE=$?
T $BIN/muster-daemon unpair late >/dev/null; sleep 7
check "unpair closes an open long-poll within seconds" '[[ $ALIVE == 0 ]] && ! kill -0 $LATE_PID 2>/dev/null'
kill $LATE_PID 2>/dev/null; wait $LATE_PID 2>/dev/null

# And its open WebSocket, the connection a revoked machine would keep longest.
LATE=$(T $BIN/muster-daemon pair --name late-ws 2>/dev/null | awk '{print $3}')
LATE_TOKEN=$(REDEEM "$LATE")
python3 -I -c '
import os, socket, ssl, sys
ctx = ssl.create_default_context(); ctx.check_hostname = False; ctx.verify_mode = ssl.CERT_NONE
s = ctx.wrap_socket(socket.create_connection(("127.0.0.1", int(os.environ["P_GATE"])), 5))
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
        u = socket.create_connection(("127.0.0.1", int(os.environ["P_GATE"])))
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
curl -s -m 3 -o /dev/null -H "Authorization: Bearer $LOCAL_TOKEN" -X POST http://127.0.0.1:${P_HOOK}/hook/session-start -d '{"session_id":"a0000000-0000-4000-8000-00000000000b","cwd":"/tmp/lost"}'
# An unpaired shim on the daemon's host reads the local token from its config dir (MUS-7).
mkdir -p $WORK/lost-home/muster && cp $WORK/daemon/muster/local-token $WORK/lost-home/muster/
OUT=$(perl -e 'alarm 60; exec @ARGV' python3 -I $LOST $BIN/muster-shim $WORK/lost-home 2>&1)
check "a dropped reply times out and gives a retry key" '[[ $OUT == *TIMEOUT* && $OUT != *NO-TIMEOUT* ]]'
check "and the retry with that key succeeds" '[[ $OUT == *RETRY-OK* ]]'
sleep 3
MAIL=$(curl -s -m 5 -H "Authorization: Bearer $LOCAL_TOKEN" -X POST "http://127.0.0.1:${P_HOOK}/hook/wait?timeout=1" -d '{"session_id":"a0000000-0000-4000-8000-00000000000b","cwd":"/tmp/lost"}')
check "and the recipient gets it once" '[[ $(print -r -- "$MAIL" | grep -c "lost reply body") == 1 ]]'
[[ $OUT == *RETRY-OK* ]] || print -r -- "$OUT"

# A shim whose first Session SET is lost still gets its first write through
# (MUS-78): a relay drops that one frame, and the write must still succeed.
LOSTSET=$WORK/lostset.py
cat >$LOSTSET <<'PY'
import json, os, select, socket, subprocess, sys, threading, time
shim, env_home, upstream_port = sys.argv[1], sys.argv[2], int(sys.argv[3])
sid = sys.argv[4] if len(sys.argv) > 4 else "e2e-lost-set"
drop_first = (sys.argv[5] if len(sys.argv) > 5 else "drop") == "drop"
send_to = sys.argv[6] if len(sys.argv) > 6 else None
dropped = threading.Event()
def read_n(s, n):
    b = b""
    while len(b) < n:
        c = s.recv(n - len(b))
        if not c: raise EOFError
        b += c
    return b
def relay_up(c, u):
    # Client frames are masked; unmask to find the first Session SET and drop it.
    try:
        head = b""
        while b"\r\n\r\n" not in head: head += c.recv(1)
        u.sendall(head)
        while True:
            h = read_n(c, 2); n = h[1] & 127; ext = b""
            if n == 126: ext = read_n(c, 2); n = int.from_bytes(ext, "big")
            elif n == 127: ext = read_n(c, 8); n = int.from_bytes(ext, "big")
            mask = read_n(c, 4) if h[1] & 128 else b""
            body = read_n(c, n)
            plain = bytes(b ^ mask[i % 4] for i, b in enumerate(body)) if mask else body
            if drop_first and not dropped.is_set() and b'"Session"' in plain and b"SET" in plain:
                dropped.set(); continue
            u.sendall(h + ext + mask + body)
    except (OSError, EOFError): pass
def relay_down(u, c):
    try:
        while True:
            d = u.recv(65536)
            if not d: break
            c.sendall(d)
    except OSError: pass
lst = socket.socket(); lst.bind(("127.0.0.1", 0)); lst.listen(); port = lst.getsockname()[1]
def serve():
    while True:
        c, _ = lst.accept()
        u = socket.create_connection(("127.0.0.1", upstream_port))
        threading.Thread(target=relay_up, args=(c, u), daemon=True).start()
        threading.Thread(target=relay_down, args=(u, c), daemon=True).start()
threading.Thread(target=serve, daemon=True).start()
env = dict(os.environ, HOME=env_home, XDG_CONFIG_HOME=env_home, MUSTER_DAEMON_ADDRESS="ws://127.0.0.1:%d" % port,
           CLAUDE_CODE_SESSION_ID=sid, RUST_LOG="info")
p = subprocess.Popen([shim], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(env_home + "/shim.log", "a"), env=env, text=True, bufsize=1)
def rpc(i, method, params, wait):
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}) + "\n"); p.stdin.flush()
    end = time.time() + wait
    while time.time() < end:
        if select.select([p.stdout], [], [], 1)[0]:
            m = json.loads(p.stdout.readline())
            if m.get("id") == i: return m
    raise SystemExit("no reply to " + method)
rpc(1, "initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}}, 20)
start = time.time()
# send_message finds its caller by this connection's binding; set_status doesn't.
call = {"name": "send_message", "arguments": {"to": send_to, "body": "after a stale binding"}} if send_to else {"name": "set_status", "arguments": {"text": "after a lost SET"}}
r = rpc(2, "tools/call", call, 20)
took = time.time() - start
print("DROPPED" if dropped.is_set() else "NOT-DROPPED", "OK" if "error" not in r and not r.get("result", {}).get("isError") else "FAILED " + json.dumps(r)[:300], "%.1f" % took)
p.kill()  # SIGKILL: no deregistration, so the row stays bound to this dead connection
PY
mkdir -p $WORK/lostset-home/muster && cp $WORK/daemon/muster/local-token $WORK/lostset-home/muster/
OUT=$(perl -e 'alarm 40; exec @ARGV' python3 -I $LOSTSET $BIN/muster-shim $WORK/lostset-home ${P_GATE} 2>&1)
check "a shim whose first Session SET is lost still gets its first write through" '[[ $OUT == "DROPPED OK "* ]] && (( ${${OUT##* }%.*} < 5 ))'
check "and says it sent the session again" 'grep -q "not on the roster 1 s after a write began; sending it again" $WORK/lostset-home/shim.log'
[[ $OUT == "DROPPED OK "* ]] || print -r -- "$OUT"
# A row still bound to a dead connection for the same session (MUS-79): the
# hold sees a binding and passes, the daemon refuses, and the shim retries.
STALE=a0000000-0000-4000-8000-0000000000f7
perl -e 'alarm 40; exec @ARGV' python3 -I $LOSTSET $BIN/muster-shim $WORK/lostset-home ${P_GATE} $STALE keep >/dev/null 2>&1
OUT=$(perl -e 'alarm 40; exec @ARGV' python3 -I $LOSTSET $BIN/muster-shim $WORK/lostset-home ${P_GATE} $STALE drop e2e-lost-set 2>&1)
check "a shim whose session row is still bound to a dead connection gets its first write through" '[[ $OUT == "DROPPED OK "* ]]'
check "after one logged retry" 'grep -q "was refused as unregistered; sending the session again and retrying once" $WORK/lostset-home/shim.log'
[[ $OUT == "DROPPED OK "* ]] || print -r -- "$OUT"

# A session whose connection drops while it's still waiting stays reachable (MUS-54).
KEEP=a0000000-0000-4000-8000-0000000000c1
(sleep 120) | env CLAUDE_CODE_SESSION_ID=$KEEP RUST_LOG=info $BIN/muster-shim >/dev/null 2>$WORK/keep.log &
KEEP_SHIM=$!
for i in {1..50}; do grep -q "(re)sending session" $WORK/keep.log 2>/dev/null && break; sleep 0.2; done
curl -sk -m 110 -H "Authorization: Bearer $TOKEN" -X POST "https://127.0.0.1:${P_GATE}/hook/wait?timeout=100" -d "{\"session_id\":\"$KEEP\",\"cwd\":\"/tmp/keep\"}" >$WORK/keep-wait.out &
KEEP_WAIT=$!
sleep 1
kill -9 $KEEP_SHIM
echo "... waiting 70s for the sweeper's reconnect grace"
sleep 70
check "a waiting session isn't reaped when its connection drops" '[[ $(ROSTER) == *$KEEP* ]]'
T $BIN/muster-shim tell $KEEP kept and reachable >/dev/null 2>&1
wait $KEEP_WAIT
check "and a message sent after the drop reaches its held wait" 'grep -q "kept and reachable" $WORK/keep-wait.out'

# A Codex bridge on a paired machine (MUS-18): it reaches the daemon through the
# gate's tunnel, registers its thread through the pairing, finds the thread's mail
# with ReadMessages, and wakes it. Once the machine is unpaired it says why.
CODEX_THREAD=a0000000-0000-4000-8000-c0dec0dec0de
mkdir -p $WORK/codexbox
CODEX_CODE=$(T $BIN/muster-daemon pair --name codex-box 2>/dev/null | awk '{print $3}')
T env HOME=$WORK/codexbox XDG_CONFIG_HOME=$WORK/codexbox $BIN/muster-shim pair "$CODEX_CODE" >/dev/null
APPSERVER=$WORK/appserver.py
cat >$APPSERVER <<'PY'
# A Codex app-server, as far as the bridge needs one: initialize, a thread/started
# for one thread, and turn/start, whose thread ids it appends to a file.
import base64, hashlib, json, socket, sys, threading
thread_id, cwd, turns, port_file = sys.argv[1:5]
GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
def read_n(c, n):
    b = b""
    while len(b) < n:
        d = c.recv(n - len(b))
        if not d: raise EOFError
        b += d
    return b
def recv(c):
    while True:
        h = read_n(c, 2); op = h[0] & 15; n = h[1] & 127
        if n == 126: n = int.from_bytes(read_n(c, 2), "big")
        elif n == 127: n = int.from_bytes(read_n(c, 8), "big")
        mask = read_n(c, 4) if h[1] & 128 else b""
        body = read_n(c, n)
        if mask: body = bytes(x ^ mask[i % 4] for i, x in enumerate(body))
        if op == 8: raise EOFError
        if op == 9: send(c, body, 10); continue
        if op == 1: return json.loads(body)
def send(c, data, op=1):
    if isinstance(data, dict): data = json.dumps(data).encode()
    n = len(data)
    head = bytes([128 | op]) + (bytes([n]) if n < 126 else bytes([126]) + n.to_bytes(2, "big") if n < 65536 else bytes([127]) + n.to_bytes(8, "big"))
    c.sendall(head + data)
def serve(c):
    try:
        req = b""
        while b"\r\n\r\n" not in req: req += c.recv(1)
        key = [l.split(b":", 1)[1].strip() for l in req.split(b"\r\n") if l.lower().startswith(b"sec-websocket-key:")][0]
        accept = base64.b64encode(hashlib.sha1(key + GUID).digest())
        c.sendall(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " + accept + b"\r\n\r\n")
        while True:
            m = recv(c); method = m.get("method")
            if method == "initialize":
                send(c, {"id": m["id"], "result": {"userAgent": "e2e", "platformFamily": "unix", "platformOs": "linux", "codexHome": "/tmp"}})
            elif method == "initialized":
                send(c, {"method": "thread/started", "params": {"thread": {"id": thread_id, "sessionId": thread_id, "cwd": cwd}}})
            elif method == "turn/start":
                with open(turns, "a") as f: f.write(m["params"]["threadId"] + "\n")
                send(c, {"id": m["id"], "result": {"turn": {"id": "turn-1"}}})
            elif method == "thread/list":
                send(c, {"id": m["id"], "result": {"data": [], "nextCursor": None}})
            elif "id" in m:
                send(c, {"id": m["id"], "result": {}})
    except (OSError, EOFError, IndexError, ValueError): pass
    finally: c.close()
lst = socket.socket(); lst.bind(("127.0.0.1", 0)); lst.listen()
open(port_file, "w").write(str(lst.getsockname()[1]))
while True:
    c, _ = lst.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
PY
python3 -I $APPSERVER $CODEX_THREAD /tmp/codexbox $WORK/codex-turns $WORK/appserver.port &
APP_PID=$!
for i in {1..50}; do [[ -s $WORK/appserver.port ]] && break; sleep 0.1; done
env HOME=$WORK/codexbox XDG_CONFIG_HOME=$WORK/codexbox TMPDIR=$WORK/codexbox RUST_LOG=muster_shim=debug $BIN/muster-shim codex-bridge --endpoint ws://127.0.0.1:$(<$WORK/appserver.port) --poll-ms 200 --ready-file $WORK/codexbox/ready >/dev/null 2>$WORK/codex-bridge.log &
BRIDGE_PID=$!
for i in {1..100}; do grep -q "registered thread=$CODEX_THREAD" $WORK/codex-bridge.log 2>/dev/null && break; sleep 0.2; done
check "a paired Codex bridge connects through the gate" 'grep -q "(paired, through the gate)" $WORK/codex-bridge.log'
check "and registers its thread through the pairing" 'grep -q "registered thread=$CODEX_THREAD" $WORK/codex-bridge.log && [[ $(ROSTER) == *$CODEX_THREAD* ]]'
T $BIN/muster-shim tell $CODEX_THREAD wake the paired codex thread >/dev/null 2>&1
for i in {1..50}; do grep -q $CODEX_THREAD $WORK/codex-turns 2>/dev/null && break; sleep 0.2; done
check "a message to that thread wakes it through the gate" 'grep -q $CODEX_THREAD $WORK/codex-turns 2>/dev/null'
check "and the bridge's log never shows its token" '! grep -qF "$(python3 -I -c '"'"'import sys,json,base64; s=open(sys.argv[1]).read().strip().split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["token"])'"'"' $WORK/codexbox/muster/pairing)" $WORK/codex-bridge.log'
T $BIN/muster-daemon unpair codex-box >/dev/null
for i in {1..60}; do grep -q "refused this machine's token (401)" $WORK/codex-bridge.log && break; sleep 0.2; done
check "once unpaired, the bridge says the daemon refused its token" 'grep -q "the paired daemon refused this machine'"'"'s token (401)" $WORK/codex-bridge.log'
kill $BRIDGE_PID $APP_PID 2>/dev/null; wait $BRIDGE_PID $APP_PID 2>/dev/null

# SIGTERM ends a held wait with an empty reply marked as a restart, then exits cleanly (MUS-55).
env XDG_CONFIG_HOME=$WORK/daemon MUSTER_STATE_DIR=$WORK/state3 MUSTER_BIND=127.0.0.1:${P_STOP_GATE} MUSTER_HOOK_BIND=127.0.0.1:${P_STOP_HOOK} MUSTER_MYKO_BIND=127.0.0.1:${P_STOP_MYKO} RUST_LOG=info $BIN/muster-daemon >$WORK/stop.log 2>&1 &
STOP_PID=$!
for i in {1..50}; do nc -z 127.0.0.1 ${P_STOP_HOOK} 2>/dev/null && break; sleep 0.2; done
curl -s -m 20 -D $WORK/stop-head.txt -o $WORK/stop-body.txt -H "Authorization: Bearer $LOCAL_TOKEN" -X POST "http://127.0.0.1:${P_STOP_HOOK}/hook/wait?timeout=15" -d '{"session_id":"a0000000-0000-4000-8000-0000000000d1","cwd":"/tmp/stop"}' &
STOP_WAIT=$!
sleep 1
STOP_AT=$SECONDS
kill -TERM $STOP_PID
wait $STOP_WAIT
wait $STOP_PID; STOP_RC=$?
check "SIGTERM ends a held wait promptly, marked as a restart" '(( SECONDS - STOP_AT < 5 )) && grep -qi "x-muster-restarting: 1" $WORK/stop-head.txt && [[ ! -s $WORK/stop-body.txt ]]'
check "and the daemon then exits cleanly" '[[ $STOP_RC == 0 ]] && grep -q "SIGTERM; ending held waits" $WORK/stop.log'

# A hook port that can't bind stops the daemon, rather than leaving hooks dead.
env XDG_CONFIG_HOME=$WORK/daemon MUSTER_STATE_DIR=$WORK/state2 MUSTER_BIND=127.0.0.1:${P_BIND_GATE} MUSTER_MYKO_BIND=127.0.0.1:${P_BIND_MYKO} RUST_LOG=error perl -e 'alarm 10; exec @ARGV' $BIN/muster-daemon >$WORK/bind.log 2>&1; RC=$?
check "a failed hook bind stops the daemon and says why" '[[ $RC != 0 && $RC != 142 ]] && grep -q "hook listener on 127.0.0.1:${P_HOOK}" $WORK/bind.log'

# A pairing whose fingerprint doesn't match: the hook must get nothing.
BAD=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; d=json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4))); d["fingerprint"]="0"*64; print("muster-pair:v1:"+base64.urlsafe_b64encode(json.dumps(d).encode()).decode().rstrip("="))' "$(<$WORK/client/muster/pairing)")
T $BIN/muster-shim pair "$BAD" >/dev/null
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook session-start)
check "wrong certificate fingerprint: hook gets nothing" '[[ -z $OUT ]]'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook wait 2>&1); RC=$?
check "wrong certificate fingerprint: waiter tells the session" '[[ $RC == 2 && $OUT == *"certificate"* ]]'
OUT=$(echo $HOOK | T $BIN/muster-shim claude-hook wait 2>&1); RC=$?
check "and only once" '[[ $RC == 0 && -z $OUT ]]'

# Unpaired, on the daemon's own host: the shim reaches the gate's port with the local token (MUS-7, MUS-64).
mkdir -p $WORK/unpaired/muster && cp $WORK/daemon/muster/local-token $WORK/unpaired/muster/
# HOME too, so a real pairing on the machine running this can't be picked up.
(sleep 4) | env HOME=$WORK/unpaired XDG_CONFIG_HOME=$WORK/unpaired MUSTER_DAEMON_ADDRESS=ws://127.0.0.1:${P_GATE} CLAUDE_CODE_SESSION_ID=e2e-local RUST_LOG=info perl -e "alarm 8; exec @ARGV" $BIN/muster-shim >/dev/null 2>$WORK/local.log
check "unpaired shim on the daemon host connects through the gate's port" 'grep -q "connected to ws://127.0.0.1:${P_GATE}/myko" $WORK/local.log'
check "and its log never shows the local token" '! grep -qF "$LOCAL_TOKEN" $WORK/local.log'
check "with TLS on, the startup line says so" 'grep -q "TLS gate: on" $WORK/daemon.log'

# Plain mode (MUS-64): no MUSTER_TLS, fresh state, as upstream Marshal runs.
unset MUSTER_TLS
mkdir -p $WORK/plain-cfg
PLAIN_ENV=(XDG_CONFIG_HOME=$WORK/plain-cfg MUSTER_STATE_DIR=$WORK/plain-state MUSTER_BIND=127.0.0.1:${P_PLAIN_GATE} MUSTER_HOOK_BIND=127.0.0.1:${P_PLAIN_HOOK} MUSTER_INTERNAL_BIND=127.0.0.1:${P_PLAIN_RETIRED} MUSTER_MYKO_BIND=127.0.0.1:${P_PLAIN_MYKO})
env $PLAIN_ENV RUST_LOG=info $BIN/muster-daemon >$WORK/plain.log 2>&1 &
PLAIN_PID=$!
trap 'kill $DPID $PLAIN_PID 2>/dev/null; wait $DPID $PLAIN_PID 2>/dev/null' EXIT
for i in {1..50}; do nc -z 127.0.0.1 ${P_PLAIN_HOOK} 2>/dev/null && nc -z 127.0.0.1 ${P_PLAIN_GATE} 2>/dev/null && break; sleep 0.2; done
check "plain is the default, and the startup line says so" 'grep -q "TLS gate: off" $WORK/plain.log'
check "plain: myko takes a websocket without any token (101)" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} --http1.1 $UPGRADE http://127.0.0.1:${P_PLAIN_GATE}/myko) == 101 ]]'
check "plain: TLS isn't served" '! curl -sk -m 3 -o /dev/null https://127.0.0.1:${P_PLAIN_GATE}/myko'
check "plain: the hooks need no token" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -X POST http://127.0.0.1:${P_PLAIN_HOOK}/hook/session-start -d "{\"session_id\":\"a0000000-0000-4000-8000-0000000000e1\",\"cwd\":\"/tmp/plain\"}") == 200 ]]'
check "plain: but still refuse a browser origin" '[[ $(curl -s -m 3 -o /dev/null -w %{http_code} -H "Origin: https://evil.example" -X POST http://127.0.0.1:${P_PLAIN_HOOK}/hook/session-start -d x) == 403 ]]'
check "plain: no local gate and no separate myko port" '! nc -z 127.0.0.1 ${P_PLAIN_RETIRED} 2>/dev/null && ! nc -z 127.0.0.1 ${P_PLAIN_MYKO} 2>/dev/null'
check "plain: no certificate and no local token are made" '[[ ! -e $WORK/plain-state/gate-cert.der && ! -e $WORK/plain-cfg/muster/local-token ]]'
mkdir -p $WORK/plain-client
(sleep 6) | env HOME=$WORK/plain-client XDG_CONFIG_HOME=$WORK/plain-client MUSTER_DAEMON_ADDRESS=ws://127.0.0.1:${P_PLAIN_GATE} CLAUDE_CODE_SESSION_ID=a0000000-0000-4000-8000-0000000000e2 RUST_LOG=info perl -e "alarm 10; exec @ARGV" $BIN/muster-shim >/dev/null 2>$WORK/plain-shim.log &
PLAIN_SHIM=$!
for i in {1..50}; do grep -q "(re)sending session" $WORK/plain-shim.log 2>/dev/null && break; sleep 0.2; done
OUT=$(T env HOME=$WORK/plain-client XDG_CONFIG_HOME=$WORK/plain-client MUSTER_DAEMON_ADDRESS=ws://127.0.0.1:${P_PLAIN_GATE} $BIN/muster-shim tell a0000000-0000-4000-8000-0000000000e2 hello in plain mode 2>&1); RC=$?
check "plain: a message is sent with no pairing or token" '[[ $RC == 0 && $OUT == *" sent, 0 not sent"* ]]'
MAIL=$(curl -s -m 5 -X POST "http://127.0.0.1:${P_PLAIN_HOOK}/hook/wait?timeout=2" -d "{\"session_id\":\"a0000000-0000-4000-8000-0000000000e2\",\"cwd\":\"/tmp/plain\"}")
check "plain: and the recipient's wait gets it" '[[ $MAIL == *"hello in plain mode"* ]]'
wait $PLAIN_SHIM 2>/dev/null
# A machine still paired with a daemon that now runs plain is told so, not left retrying.
python3 -I -c 'import os,sys,json,base64; s=sys.argv[1].strip().split(":",2)[2]; d=json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4))); d["address"]="127.0.0.1:"+os.environ["P_PLAIN_GATE"]; print("muster-pair:v1:"+base64.urlsafe_b64encode(json.dumps(d).encode()).decode().rstrip("="))' "$(<$WORK/client/muster/pairing)" > $WORK/plain-pairing.code
mkdir -p $WORK/stale/muster && cp $WORK/plain-pairing.code $WORK/stale/muster/pairing
OUT=$(echo "{\"session_id\":\"a0000000-0000-4000-8000-0000000000e3\",\"cwd\":\"/tmp/stale\"}" | perl -e 'alarm 20; exec @ARGV' env HOME=$WORK/stale XDG_CONFIG_HOME=$WORK/stale $BIN/muster-shim claude-hook wait 2>&1); RC=$?
check "plain: a paired machine is told the daemon isn't serving TLS" '[[ $RC == 2 && $OUT == *"isn'"'"'t serving TLS"* ]]'
kill $PLAIN_PID 2>/dev/null; wait $PLAIN_PID 2>/dev/null

# The switch's refusals.
OUT=$(T env $PLAIN_ENV MUSTER_TLS=0 $BIN/muster-daemon --tls 2>&1); RC=$?
check "--tls with MUSTER_TLS=0 refuses to start and names both" '[[ $RC != 0 && $RC != 142 && $OUT == *"--tls and MUSTER_TLS=0 disagree; set one"* ]]'
OUT=$(T env $PLAIN_ENV MUSTER_TLS=maybe $BIN/muster-daemon 2>&1); RC=$?
check "an unknown MUSTER_TLS value refuses to start" '[[ $RC != 0 && $RC != 142 && $OUT == *"MUSTER_TLS=maybe"* ]]'
# The TLS run's state has paired machines: plain without an explicit off must refuse.
OUT=$(T env $PLAIN_ENV MUSTER_STATE_DIR=$WORK/state $BIN/muster-daemon 2>&1); RC=$?
check "paired machines and no MUSTER_TLS: refuses to run plain" '[[ $RC != 0 && $RC != 142 && $OUT == *"paired machines"* ]]'
env $PLAIN_ENV MUSTER_STATE_DIR=$WORK/state MUSTER_TLS=0 RUST_LOG=info $BIN/muster-daemon >$WORK/plain-off.log 2>&1 &
OFF_PID=$!
for i in {1..50}; do grep -q "TLS gate: off" $WORK/plain-off.log 2>/dev/null && break; sleep 0.2; done
check "but MUSTER_TLS=0 runs plain anyway" 'grep -q "TLS gate: off" $WORK/plain-off.log'
kill $OFF_PID 2>/dev/null; wait $OFF_PID 2>/dev/null
OUT=$(T env $PLAIN_ENV MUSTER_STATE_DIR=$WORK/plain-state $BIN/muster-daemon pair --name later --address 127.0.0.1:${P_PLAIN_GATE} 2>&1 >/dev/null)
check "pair in a shell without MUSTER_TLS says pairing needs TLS" '[[ $OUT == *"pairing takes effect only when the daemon runs with TLS"* ]]'

echo "--- $pass passed, $fail failed"
rm -rf "$WORK/client" "$WORK/state"
echo "--- daemon log (gate lines):"; grep -E "gate|rejected" $WORK/daemon.log | sed -E 's/fingerprint [0-9a-f]+/fingerprint …/' | head -12
[[ $fail == 0 ]]

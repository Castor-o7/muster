#!/bin/zsh
# End-to-end check of a daemon restart with no Postgres and MUSTER_PERSIST=file
# (MUS-17): unread mail, read state and an ad-hoc room membership survive it, and
# the store file is private. The Postgres version is test-restart-e2e.sh.
# Needs zsh, python3 and nc. Build first (`cargo build`), then run this script.
set -u
BIN=${MUSTER_E2E_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/debug}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/muster-restart-file-e2e.XXXX")
unset MUSTER_BASE_URL MUSTER_DAEMON_ADDRESS MUSTER_TLS MYKO_POSTGRES_URL
pass=0; fail=0
check() { if eval "$2"; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1"; fail=$((fail+1)); fi; }
waitfor() { for i in {1..100}; do eval "$1" && return 0; sleep 0.2; done; return 1; }

DPID=
trap 'kill $DPID 2>/dev/null; wait 2>/dev/null' EXIT
ENV=(XDG_CONFIG_HOME=$WORK/cfg MUSTER_STATE_DIR=$WORK/state MUSTER_HOST=rhost
     MUSTER_BIND=127.0.0.1:16865 MUSTER_HOOK_BIND=127.0.0.1:16866 MUSTER_PERSIST=file)
start() { env $ENV RUST_LOG=info $BIN/muster-daemon >>$WORK/daemon.log 2>&1 & DPID=$!; waitfor 'nc -z 127.0.0.1 16865 2>/dev/null && nc -z 127.0.0.1 16866 2>/dev/null'; }
start
check "the daemon keeps its store in a file" 'grep -q "persistence: file" $WORK/daemon.log'

# A shim speaking MCP: join a room, send, ack, or read.
MCP=$WORK/mcp.py
cat >$MCP <<'PY'
import json, os, select, subprocess, sys, time
shim, home, sid, action, room = sys.argv[1:6]
os.makedirs(home, exist_ok=True)
env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=home, MUSTER_DAEMON_ADDRESS="ws://127.0.0.1:16865", CLAUDE_CODE_SESSION_ID=sid)
p = subprocess.Popen([shim], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, text=True, bufsize=1)
n = [0]
def rpc(method, params, wait=20):
    n[0] += 1; i = n[0]
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}) + "\n"); p.stdin.flush()
    end = time.time() + wait
    while time.time() < end:
        if select.select([p.stdout], [], [], 1)[0]:
            m = json.loads(p.stdout.readline())
            if m.get("id") == i: return m
    raise SystemExit("no reply to " + method)
def read(uri):
    return json.loads(rpc("resources/read", {"uri": uri})["result"]["contents"][0]["text"])
rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
end = time.time() + 15
while time.time() < end and not any(r.get("is_self") for r in read("muster://roster").get("sessions", [])):
    time.sleep(0.2)
if action == "join":
    print(json.dumps(rpc("tools/call", {"name": "join_room", "arguments": {"name": room}}).get("result")))
elif action == "send":
    to, body = room.split("|", 1)
    print(json.dumps(rpc("tools/call", {"name": "send_message", "arguments": {"to": to, "body": body}}).get("result")))
elif action == "ack":
    mine = read("muster://messages?inbox=true&unread=true")["messages"]
    ids = [m["messageId"] for m in mine if room in m["body"]]
    print(json.dumps(rpc("tools/call", {"name": "ack_messages", "arguments": {"message_ids": ids}}).get("result")))
elif action == "unread":
    print("UNREAD " + json.dumps(sorted(m["body"] for m in read("muster://messages?inbox=true&unread=true")["messages"])))
elif action == "inbox":
    print("INBOX " + json.dumps(sorted(m["body"] for m in read("muster://messages?inbox=true")["messages"])))
else:
    rooms = read("muster://rooms")
    rooms = rooms.get("rooms", rooms) if isinstance(rooms, dict) else rooms
    found = [r for r in rooms if r.get("id") == room or r.get("name") == room]
    print("ROOM " + json.dumps(found[0] if found else None))
p.kill()
PY
mcp() { perl -e 'alarm 60; exec @ARGV' python3 -I $MCP $BIN/muster-shim "$@"; }

MEMBER=a0000000-0000-4000-8000-0000000017a1
SENDER=a0000000-0000-4000-8000-0000000017b1
OUT=$(mcp $WORK/member $MEMBER join keep-check)
check "a session joins an ad-hoc room" '[[ $OUT == *keep-check* ]]'
mcp $WORK/sender $SENDER send "$MEMBER|kept while you were away" >/dev/null
mcp $WORK/sender $SENDER send "$MEMBER|already read" >/dev/null
mcp $WORK/member $MEMBER ack "already read" >/dev/null
OUT=$(mcp $WORK/member $MEMBER unread x)
check "before the restart, one message is unread" '[[ $OUT == "UNREAD [\"kept while you were away\"]" ]]'

# Restart, the way systemd does.
kill -TERM $DPID; wait $DPID 2>/dev/null
start
check "after a restart, the store is loaded from the file" '[[ $(grep -c "persistence: file" $WORK/daemon.log) == 2 ]]'
check "and the sweeper starts once it is" 'waitfor "[[ \$(grep -c \"the store is loaded; starting the cleanup sweeper\" $WORK/daemon.log) == 2 ]]"'
sleep 8
OUT=$(mcp $WORK/member $MEMBER unread x)
check "the unread message survives the restart, still unread" '[[ $OUT == "UNREAD [\"kept while you were away\"]" ]]'
OUT=$(mcp $WORK/member $MEMBER inbox x)
check "and the read one is there, read" '[[ $OUT == *"already read"* && $OUT == *"kept while"* ]]'
OUT=$(mcp $WORK/sender $SENDER show keep-check)
check "the ad-hoc membership survives the restart" '[[ $OUT == *$MEMBER* ]]'
check "the store file is private" '[[ $(stat -c %a $WORK/state/store.jsonl 2>/dev/null || stat -f %Lp $WORK/state/store.jsonl) == 600 ]]'
check "nothing tried Postgres" '! grep -qi "postgres (events table" $WORK/daemon.log'

echo "--- $pass passed, $fail failed"
[[ $fail == 0 ]] || { echo "--- daemon log:"; tail -40 $WORK/daemon.log; }
[[ $fail == 0 ]]

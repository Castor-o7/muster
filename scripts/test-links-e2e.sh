#!/bin/zsh
# End-to-end check of links between daemons (MUS-66) with the real binaries:
# two TLS daemons on spare loopback ports, in throwaway dirs. Build first
# (`cargo build`), then run this script. Needs zsh, curl, nc and python3.
set -u
BIN=$(cd "$(dirname "$0")/.." && pwd)/target/debug
WORK=$(mktemp -d "${TMPDIR:-/tmp}/muster-links-e2e.XXXX")
unset MUSTER_BASE_URL MUSTER_DAEMON_ADDRESS MUSTER_TLS MYKO_POSTGRES_URL
T() { perl -e 'alarm 15; exec @ARGV' "$@"; }
pass=0; fail=0
check() { if eval "$2"; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1"; fail=$((fail+1)); fi; }
waitfor() { for i in {1..100}; do eval "$1" && return 0; sleep 0.2; done; return 1; }

# L listens (the far daemon, "lhost"); D dials it ("dhost").
L_ENV=(XDG_CONFIG_HOME=$WORK/L MUSTER_STATE_DIR=$WORK/L/state MUSTER_HOST=lhost MUSTER_TLS=1 MUSTER_ACK_LEASE_SECS=2
       MUSTER_BIND=127.0.0.1:16655 MUSTER_HOOK_BIND=127.0.0.1:16656 MUSTER_INTERNAL_BIND=127.0.0.1:16657 MUSTER_MYKO_BIND=127.0.0.1:16658)
D_ENV=(XDG_CONFIG_HOME=$WORK/D MUSTER_STATE_DIR=$WORK/D/state MUSTER_HOST=dhost MUSTER_TLS=1 MUSTER_ACK_LEASE_SECS=2
       MUSTER_BIND=127.0.0.1:16755 MUSTER_HOOK_BIND=127.0.0.1:16756 MUSTER_INTERNAL_BIND=127.0.0.1:16757 MUSTER_MYKO_BIND=127.0.0.1:16758)
start_l() { env $L_ENV RUST_LOG=info $BIN/muster-daemon >>$WORK/L.log 2>&1 & LPID=$!; waitfor 'nc -z 127.0.0.1 16656 2>/dev/null && nc -z 127.0.0.1 16655 2>/dev/null'; }
start_d() { env $D_ENV RUST_LOG=info $BIN/muster-daemon >>$WORK/D.log 2>&1 & DPID=$!; waitfor 'nc -z 127.0.0.1 16756 2>/dev/null && nc -z 127.0.0.1 16755 2>/dev/null'; }
start_l; start_d
trap 'kill $LPID $DPID 2>/dev/null; wait 2>/dev/null' EXIT
L_TOKEN=$(<$WORK/L/muster/local-token); D_TOKEN=$(<$WORK/D/muster/local-token)

# Sessions, registered the way the hooks register them.
SL=a0000000-0000-4000-8000-0000000001a1   # on L
SD=a0000000-0000-4000-8000-0000000001d1   # on D
register() { curl -s -m 5 -o /dev/null -w %{http_code} -H "Authorization: Bearer $2" -X POST "http://127.0.0.1:$1/hook/session-start" -d "{\"session_id\":\"$3\",\"cwd\":\"/tmp/$4\"}"; }
check "a session registers on each daemon" '[[ $(register 16656 $L_TOKEN $SL left) == 200 && $(register 16756 $D_TOKEN $SD right) == 200 ]]'
waitmail() { curl -s -m 12 -H "Authorization: Bearer $2" -X POST "http://127.0.0.1:$1/hook/wait?timeout=${4:-8}" -d "{\"session_id\":\"$3\",\"cwd\":\"/tmp/x\"}"; }

# A link is made with a one-time code, like a pairing (MUS-8).
OUT=$(T env $L_ENV $BIN/muster-daemon pair --name dhost --link --address 127.0.0.1:16655 2>/dev/null)
CODE=${OUT#muster-daemon link }
check "pair --link prints a muster-daemon link command" '[[ $OUT == "muster-daemon link muster-pair:v2:"* ]]'
OUT=$(T env HOME=$WORK/shim XDG_CONFIG_HOME=$WORK/shim $BIN/muster-shim pair "$CODE" 2>&1); RC=$?
check "muster-shim pair refuses a link code" '[[ $RC != 0 && $OUT == *"links two daemons"* ]]'
OUT=$(T env $D_ENV MUSTER_TLS=0 $BIN/muster-daemon link "$CODE" 2>&1); RC=$?
check "link refuses without TLS (and doesn't spend the code)" '[[ $RC != 0 && $OUT == *"need TLS on both daemons"* ]]'
OUT=$(T env $D_ENV $BIN/muster-daemon link "$CODE" 2>&1); RC=$?
check "link redeems the code and stores the link" '[[ $RC == 0 && $OUT == "linked to lhost at 127.0.0.1:16655"* ]]'
check "the stored link is owner-only" '[[ $(python3 -I -c "import os,sys; print(oct(os.stat(sys.argv[1]).st_mode & 0o777))" $WORK/D/state/links) == 0o600 ]]'
OUT=$(T env $D_ENV $BIN/muster-daemon link "$CODE" 2>&1); RC=$?
check "a used code is refused" '[[ $RC != 0 && $OUT == *"already used or has expired"* ]]'
check "links lists it on both sides" '[[ $(T env $D_ENV $BIN/muster-daemon links) == *"lhost (this daemon dials 127.0.0.1:16655)"* && $(T env $L_ENV $BIN/muster-daemon links) == *"dhost (dials this daemon)"* ]]'
check "the link comes up on both sides" 'waitfor "grep -q \"\\[link\\] lhost: up\" $WORK/D.log && grep -q \"\\[link\\] dhost: up\" $WORK/L.log"'


# A shim speaking MCP, as an agent would: read the roster, send messages.
MCP=$WORK/mcp.py
cat >$MCP <<'PY'
import json, os, select, subprocess, sys, time
shim, cfg, addr, sid = sys.argv[1:5]
actions = json.loads(sys.argv[5])
os.makedirs(cfg + "-home", exist_ok=True)
env = dict(os.environ, HOME=cfg + "-home", XDG_CONFIG_HOME=cfg, MUSTER_DAEMON_ADDRESS=addr, CLAUDE_CODE_SESSION_ID=sid)
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
def roster():
    r = rpc("resources/read", {"uri": "muster://roster"})
    return json.loads(r["result"]["contents"][0]["text"])
rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
# Wait for this session's own row, as an agent's first turn would (MUS-72).
end = time.time() + 15
while time.time() < end and not any(r.get("is_self") for r in roster().get("sessions", [])):
    time.sleep(0.2)
for a in actions:
    if a[0] == "wait_remote":
        end = time.time() + 20
        while time.time() < end:
            rs = [r for r in roster().get("remote_sessions", []) if r["session_id"] == a[1] and r["link_up"]]
            if rs: print("REMOTE " + json.dumps(rs[0])); break
            time.sleep(0.5)
        else: print("NO-REMOTE")
    elif a[0] == "send":
        r = rpc("tools/call", {"name": "send_message", "arguments": {"to": a[1], "body": a[2]}})
        print("SENT " + json.dumps(r.get("result", r)))
    elif a[0] == "sleep":
        time.sleep(a[1])
p.kill()
PY
mcp() { perl -e 'alarm 60; exec @ARGV' python3 -I $MCP $BIN/muster-shim "$@"; }

OUT=$(mcp $WORK/D ws://127.0.0.1:16755 $SD "[[\"wait_remote\",\"$SL\"]]")
check "D's roster lists L's session, via lhost" '[[ $OUT == *"\"via\": \"lhost\""* && $OUT == *"\"address\": "*"@lhost"* ]]'
check "and shares no cwd" '[[ $OUT != *"/tmp/left"* ]]'
SL_ADDR=$(print -r -- "$OUT" | python3 -I -c 'import json,sys; l=sys.stdin.read(); print(json.loads(l.split("REMOTE ",1)[1])["address"]) if "REMOTE " in l else print("")')
OUT=$(mcp $WORK/D ws://127.0.0.1:16755 $SD "[[\"wait_remote\",\"$SL\"],[\"send\",\"$SL_ADDR\",\"hello across the link\"]]")
check "a session on D sends to nickname@lhost" '[[ $OUT == *via_link*lhost* ]]'
[[ $OUT == *via_link*lhost* ]] || print -r -- "send from D: $OUT"
MAIL=$(waitmail 16656 $L_TOKEN $SL)
check "and L's session gets it, from nickname@dhost" '[[ $MAIL == *"hello across the link"* && $MAIL == *"@dhost"* ]]'
SD_ADDR=$(print -r -- "$MAIL" | grep -o "from [a-z]*-[a-z]*@dhost" | head -1 | cut -c6-)
OUT=$(mcp $WORK/L ws://127.0.0.1:16655 $SL "[[\"wait_remote\",\"$SD\"],[\"send\",\"$SD_ADDR\",\"and back again\"]]")
check "L's session answers to the address it was shown" '[[ -n $SD_ADDR && $OUT == *via_link*dhost* ]]'
MAIL=$(waitmail 16756 $D_TOKEN $SD)
check "and D's session gets the answer" '[[ $MAIL == *"and back again"* && $MAIL == *"@lhost"* ]]'
check "a message arrives once" '[[ $(waitmail 16756 $D_TOKEN $SD 2) != *"and back again"* ]]'

# The listener restarts while mail waits in D's outbox.
kill $LPID; wait $LPID 2>/dev/null
OUT=$(mcp $WORK/D ws://127.0.0.1:16755 $SD "[[\"sleep\",3],[\"send\",\"$SL_ADDR\",\"sent while lhost was down\"]]")
check "a send while the link is down queues and says so" '[[ $OUT == *via_link*lhost* && $OUT == *link_up*false* ]]'
start_l
L_TOKEN=$(<$WORK/L/muster/local-token)
register 16656 $L_TOKEN $SL left >/dev/null
MAIL=$(waitmail 16656 $L_TOKEN $SL 20)
check "after the restart, the queued message arrives" '[[ $MAIL == *"sent while lhost was down"* ]]'
check "once" '[[ $(waitmail 16656 $L_TOKEN $SL 3) != *"sent while lhost was down"* ]]'

PEER=$WORK/peer.py
cat >$PEER <<'PY'
import base64, json, os, socket, ssl, sys
token, scenario, local_id = sys.argv[1:4]
ctx = ssl.create_default_context(); ctx.check_hostname = False; ctx.verify_mode = ssl.CERT_NONE
s = ctx.wrap_socket(socket.create_connection(("127.0.0.1", 16655), timeout=10), server_hostname="muster-daemon")
s.sendall(("GET /link HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n"
           "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nAuthorization: Bearer %s\r\n\r\n" % token).encode())
head = b""
while b"\r\n\r\n" not in head: head += s.recv(1)
assert b" 101 " in head, head
def read_n(n):
    b = b""
    while len(b) < n:
        c = s.recv(n - len(b))
        if not c: raise EOFError
        b += c
    return b
def send_raw(data):
    mask = os.urandom(4); n = len(data)
    h = bytes([0x81]) + (bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + n.to_bytes(2, "big") if n < 65536 else bytes([0x80 | 127]) + n.to_bytes(8, "big"))
    s.sendall(h + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))
def send(obj): send_raw(json.dumps(obj).encode())
def recv():
    try:
        while True:
            h = read_n(2); op = h[0] & 15; n = h[1] & 127
            if n == 126: n = int.from_bytes(read_n(2), "big")
            elif n == 127: n = int.from_bytes(read_n(8), "big")
            body = read_n(n)
            if op == 8: return None
            if op == 1: return json.loads(body)
    except (EOFError, OSError, socket.timeout): return None
def until(kind):
    while True:
        f = recv()
        if f is None or f.get("type") == kind: return f
if scenario == "version":
    send({"type": "hello", "name": "dhost", "version": 99})
    print("CLOSED" if until("never") is None else "OPEN")
    sys.exit()
send({"type": "hello", "name": "dhost", "version": 1})
until("hello")
fake = os.environ.get("FAKE_ID", "f0000000-0000-4000-8000-0000000000ff")
send({"type": "roster", "sessions": [{"session_id": fake, "nickname": "fake-peer"}, {"session_id": local_id, "nickname": "evil-twin"}]})
if scenario == "forged":
    send({"type": "deliver", "id": "x1", "from": "a0000000-0000-4000-8000-00000000dead", "to": local_id, "body": "forged", "sent_at": 0})
    print("FORGED " + json.dumps(until("refused")))
    send({"type": "deliver", "id": "x2", "from": local_id, "to": local_id, "body": "as a local session", "sent_at": 0})
    print("TWIN " + json.dumps(until("refused")))
elif scenario == "flood":
    for i in range(30):
        send({"type": "deliver", "id": "flood-%d" % i, "from": fake, "to": local_id, "body": "flood %d" % i, "sent_at": 0})
    seen = {}
    for i in range(30):
        f = recv()
        if f is None: break
        seen[f["type"]] = seen.get(f["type"], 0) + 1
    print("FLOOD " + json.dumps(seen))
    # A resend of a stored message is stored again, not duplicated.
    send({"type": "deliver", "id": "flood-0", "from": fake, "to": local_id, "body": "flood 0", "sent_at": 0})
    print("RESEND " + json.dumps(until("stored")))
elif scenario == "answers":
    # Answer for a message queued for another link: every answer must be ignored.
    other = os.environ["OTHER_ID"]
    send({"type": "deferred", "id": other, "retry_after": 1})
    send({"type": "refused", "id": other, "why": "lies from another link"})
    send({"type": "stored", "id": other})
    import time; time.sleep(2)
    print("ANSWERED")
elif scenario == "big":
    send_raw(b'"' + b"x" * (200 * 1024) + b'"')
    print("CLOSED" if until("never") is None else "OPEN")
PY

# One link can't act on mail queued for another: a second link, ehost,
# answers for a message waiting in L's outbox for dhost.
kill $DPID; wait $DPID 2>/dev/null
OUT=$(mcp $WORK/L ws://127.0.0.1:16655 $SL "[[\"sleep\",3],[\"send\",\"$SD_ADDR\",\"queued for dhost only\"]]")
QUEUED_ID=$(print -r -- "$OUT" | grep -o '\\"message_id\\": \\"[^\\]*' | head -1 | sed 's/.*\\"//')
check "mail for dhost queues while D is down" '[[ -n $QUEUED_ID && $OUT == *link_up*false* ]]'
ECODE=$(T env $L_ENV $BIN/muster-daemon pair --name ehost --link --address 127.0.0.1:16655 2>/dev/null)
ECODE=${ECODE#muster-daemon link }
EINV=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["invite"])' "$ECODE")
E_TOKEN=$(curl -sk -m 5 -X POST -H "Authorization: Bearer $EINV" https://127.0.0.1:16655/pair | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["token"])')
OUT=$(FAKE_ID=e0000000-0000-4000-8000-0000000000ee OTHER_ID=$QUEUED_ID perl -e 'alarm 30; exec @ARGV' python3 -I $PEER $E_TOKEN answers $SL 2>&1)
check "ehost's answers about dhost's message are ignored" '[[ $OUT == *ANSWERED* ]] && [[ $(grep -c "ehost: ignored an answer about a message it wasn.t sent" $WORK/L.log) -ge 3 ]]'
check "and its sender gets no forged refusal" '[[ $(waitmail 16656 $L_TOKEN $SL 2) != *"lies from another link"* ]]'
start_d
D_TOKEN=$(<$WORK/D/muster/local-token)
register 16756 $D_TOKEN $SD right >/dev/null
MAIL=$(waitmail 16756 $D_TOKEN $SD 20)
check "and it still reaches dhost, not ehost, when D is back" '[[ $MAIL == *"queued for dhost only"* ]]'

# Token kinds are kept apart at the gate.
LINK_TOKEN=$(awk '{print $4}' $WORK/D/state/links)
MCODE=$(T env $L_ENV $BIN/muster-daemon pair --name m1 --address 127.0.0.1:16655 2>/dev/null | awk '{print $3}')
INV=$(python3 -I -c 'import sys,json,base64; s=sys.argv[1].split(":",2)[2]; print(json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))["invite"])' "$MCODE")
MTOKEN=$(curl -sk -m 5 -X POST -H "Authorization: Bearer $INV" https://127.0.0.1:16655/pair | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["token"])')
UPGRADE=(-H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==")
check "a machine's token can't open a link (403)" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} --http1.1 $UPGRADE -H "Authorization: Bearer $MTOKEN" https://127.0.0.1:16655/link) == 403 ]]'
check "a link's token can't open myko (403)" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} --http1.1 $UPGRADE -H "Authorization: Bearer $LINK_TOKEN" https://127.0.0.1:16655/myko) == 403 ]]'
check "or the hooks (403)" '[[ $(curl -sk -m 3 -o /dev/null -w %{http_code} -H "Authorization: Bearer $LINK_TOKEN" -X POST https://127.0.0.1:16655/hook/session-start -d "{}") == 403 ]]'

# A misbehaving peer holding D's link token. D is stopped first so it can't
# take the link back mid-test.
kill $DPID; wait $DPID 2>/dev/null
peer() { perl -e 'alarm 30; exec @ARGV' python3 -I $PEER $LINK_TOKEN "$@" $SL 2>&1; }
check "a hello with another protocol version closes the link" '[[ $(peer version) == CLOSED ]] && grep -q "speaks link protocol 99" $WORK/L.log'
OUT=$(peer forged)
check "a deliver from a session the peer never advertised is refused" '[[ $OUT == *"FORGED"*"\"type\": \"refused\""*"isn'"'"'t one of that daemon"* ]]'
check "a roster naming a local session id is refused" 'grep -q "refused $SL: that'"'"'s a local session'"'"'s id" $WORK/L.log'
check "so mail can't come from it either" '[[ $OUT == *"TWIN"*"\"type\": \"refused\""* ]]'
check "and the forged mail never reaches the session" '[[ $(waitmail 16656 $L_TOKEN $SL 2) != *forged* ]]'
OUT=$(peer flood)
check "a flood past the rate limit is deferred, not refused" '[[ $OUT == *"\"deferred\": "* && $OUT == *"\"stored\": "* && $OUT != *refused* ]]'
check "a resend of a stored message is answered stored" '[[ $OUT == *"RESEND"*"flood-0"* ]]'
MAIL=$(curl -s -m 12 -H "Authorization: Bearer $L_TOKEN" -X POST "http://127.0.0.1:16656/hook/wait?timeout=3" -d "{\"session_id\":\"$SL\",\"cwd\":\"/tmp/x\"}")
check "and flood 0 is in the inbox once" '[[ $(print -r -- "$MAIL" | grep -cw "flood 0") == 1 ]]'
check "a frame over 128 KiB closes the link" '[[ $(peer big) == CLOSED ]]'

# unpair on the listener ends the link and its token.
start_d
check "the link comes back when D restarts" 'waitfor "[[ \$(grep -c \"\\[link\\] lhost: up\" $WORK/D.log) -ge 2 ]]"'
D_LINES=$(wc -l <$WORK/D.log)
T env $L_ENV $BIN/muster-daemon unpair dhost >/dev/null
check "unpair on the listener closes the open link" 'waitfor "grep -q \"closing link:dhost.s connection: no longer paired\" $WORK/L.log"'
check "and D's next try is refused, and says why" 'waitfor "tail -n +\$((D_LINES+1)) $WORK/D.log | grep -q \"lhost: can.t connect.*refused this link.s token (401)\""'

# A plain daemon keeps its stored links idle.
kill $DPID; wait $DPID 2>/dev/null
env $D_ENV MUSTER_TLS=0 RUST_LOG=info $BIN/muster-daemon >$WORK/D-plain.log 2>&1 & DPID=$!
check "a plain daemon leaves its links idle, and says so" 'waitfor "grep -q \"stored link(s) stay idle: links need TLS\" $WORK/D-plain.log"'

echo "--- $pass passed, $fail failed"
[[ $fail == 0 ]]

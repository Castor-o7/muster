#!/bin/zsh
# End-to-end check of a daemon restart with Postgres (MUS-58): an ad-hoc room
# membership survives it. The sweeper removes memberships whose session has
# left the roster, so it must not run before myko has replayed the sessions.
# Needs zsh, python3, curl and nc, and either Postgres's initdb, pg_ctl and
# postgres on PATH, for a throwaway server, or RESTART_E2E_POSTGRES_URL naming
# an existing database. Build first (`cargo build`), then run this script.
set -u
BIN=${MUSTER_E2E_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/debug}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/muster-restart-e2e.XXXX")
unset MUSTER_BASE_URL MUSTER_DAEMON_ADDRESS MUSTER_TLS MYKO_POSTGRES_URL
pass=0; fail=0
check() { if eval "$2"; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1"; fail=$((fail+1)); fi; }
waitfor() { for i in {1..100}; do eval "$1" && return 0; sleep 0.2; done; return 1; }

DPID=
if [[ -n ${RESTART_E2E_POSTGRES_URL:-} ]]; then
  # An existing Postgres, as CI's service is: a table and channel of this run's
  # own, so nothing else using the database is touched.
  PG_URL=$RESTART_E2E_POSTGRES_URL
  PG_TABLE=restart_e2e_$$
  # On exit, drop what myko made for this run: the table, which takes its indexes
  # and trigger with it, and the trigger's function. Needs psql; without it they stay.
  drop_run_schema() {
    command -v psql >/dev/null || return 0
    psql -q -X "$PG_URL" -c "DROP TABLE IF EXISTS $PG_TABLE; DROP FUNCTION IF EXISTS ${PG_TABLE}_notify_insert_fn();" >/dev/null 2>&1 \
      || echo "note: couldn't drop $PG_TABLE"
  }
  trap 'kill $DPID 2>/dev/null; wait 2>/dev/null; drop_run_schema' EXIT
else
  # A throwaway Postgres on a Unix socket only, so nothing else can reach it. The
  # socket gets its own short directory: its path may be at most 103 bytes.
  PGPORT=16955
  SOCK=$(mktemp -d /tmp/muster-pg.XXXX)
  initdb -D $WORK/pg -U muster --auth=trust >$WORK/initdb.log 2>&1 || { echo "initdb failed"; cat $WORK/initdb.log; exit 1; }
  pg_ctl -D $WORK/pg -l $WORK/pg.log -o "-p $PGPORT -k $SOCK -c listen_addresses=''" -w start >/dev/null || { echo "postgres didn't start"; cat $WORK/pg.log; exit 1; }
  trap 'kill $DPID 2>/dev/null; wait 2>/dev/null; pg_ctl -D $WORK/pg -m immediate stop >/dev/null 2>&1; rm -rf $SOCK' EXIT
  PG_URL="host=$SOCK port=$PGPORT dbname=postgres user=muster"
  PG_TABLE=myko_events
fi

ENV=(XDG_CONFIG_HOME=$WORK/cfg MUSTER_STATE_DIR=$WORK/state MUSTER_HOST=rhost
     MUSTER_BIND=127.0.0.1:16855 MUSTER_HOOK_BIND=127.0.0.1:16856
     "MYKO_POSTGRES_URL=$PG_URL" MYKO_POSTGRES_TABLE=$PG_TABLE MYKO_POSTGRES_CHANNEL=${PG_TABLE}_notify)
start() { env $ENV RUST_LOG=info $BIN/muster-daemon >>$WORK/daemon.log 2>&1 & DPID=$!; waitfor 'nc -z 127.0.0.1 16855 2>/dev/null && nc -z 127.0.0.1 16856 2>/dev/null'; }
start
check "the daemon starts with Postgres" 'grep -q "persistence: postgres" $WORK/daemon.log'

# A shim speaking MCP: join a room, or print a room's members.
MCP=$WORK/mcp.py
cat >$MCP <<'PY'
import json, os, select, subprocess, sys, time
shim, home, sid, action, room = sys.argv[1:6]
os.makedirs(home, exist_ok=True)
env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=home, MUSTER_DAEMON_ADDRESS="ws://127.0.0.1:16855", CLAUDE_CODE_SESSION_ID=sid)
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
elif action == "status":
    print(json.dumps(rpc("tools/call", {"name": "set_status", "arguments": {"text": room}}).get("result")))
else:
    rooms = read("muster://rooms")
    rooms = rooms.get("rooms", rooms) if isinstance(rooms, dict) else rooms
    found = [r for r in rooms if r.get("id") == room or r.get("name") == room]
    print("ROOM " + json.dumps(found[0] if found else None))
p.kill()
PY
mcp() { perl -e 'alarm 60; exec @ARGV' python3 -I $MCP $BIN/muster-shim "$@"; }

MEMBER=a0000000-0000-4000-8000-0000000005a1
OBSERVER=a0000000-0000-4000-8000-0000000005b1
OUT=$(mcp $WORK/member $MEMBER join restart-check)
check "a session joins an ad-hoc room" '[[ $OUT == *restart-check* ]]'
OUT=$(mcp $WORK/observer $OBSERVER show restart-check)
check "and the room lists it" '[[ $OUT == *$MEMBER* ]]'

# Set up the order that makes the race possible. Replay loads each row's latest
# event, and a session that changed after it joined (here: its status) has a
# later event than its membership, as a session that reconnected does; a few
# thousand other sessions make the store more like production's. myko streams
# the whole snapshot first and then fills the stores one entity type at a time,
# so the unsafe window is only the gap between the RoomMember and Session
# inserts: too short to hit on purpose. So this pins the outcome, and that the
# sweeper starts only once the store is loaded, rather than the race itself.
seq 1 3000 | xargs -P 16 -I{} curl -s -m 5 -o /dev/null -X POST http://127.0.0.1:16856/hook/session-start -d '{"session_id":"b0000000-0000-4000-8000-{}","cwd":"/tmp/filler"}'
OUT=$(mcp $WORK/member $MEMBER status "changed after joining")
check "the member's session changes after it joined" '[[ $OUT == *changed* || $OUT == *ok* ]]'

# Restart, the way systemd does, and give the sweeper several ticks.
kill -TERM $DPID; wait $DPID 2>/dev/null
start
check "after a restart, the sweeper starts once the store is loaded" 'waitfor "[[ \$(grep -c \"the store is loaded; starting the cleanup sweeper\" $WORK/daemon.log) == 2 ]]"'
sleep 10
OUT=$(mcp $WORK/observer $OBSERVER show restart-check)
check "the ad-hoc membership survives the restart" '[[ $OUT == *$MEMBER* ]]'
check "and the sweeper didn't remove it" '! grep -q "$MEMBER left restart-check" $WORK/daemon.log'

echo "--- $pass passed, $fail failed"
[[ $fail == 0 ]] || { echo "--- daemon log:"; tail -40 $WORK/daemon.log; }
[[ $fail == 0 ]]

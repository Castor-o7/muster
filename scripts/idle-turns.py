#!/usr/bin/env python3
"""Count each session's turns per hour from the live roster (MUS-1).

MUS-1 is done when an idle session uses at most 2 model turns per hour, with 0
as the target, and a session idle overnight still wakes on a message. Neither
the daemon's log nor its Postgres store records turns: activity, lastActivityAt
and lastTool are left out of durable writes (liveness_filter.rs), and wait
records aren't persisted. The daemon's live state has them, and
`muster-shim roster` reads it without registering anything.

So this polls `muster-shim roster` every --interval seconds and keeps only
aggregates: per UTC hour, host and session (id prefix and nickname), the polls
seen, how many found the session not in a turn, turns (activity becoming
`busy`), lastActivityAt changes, and wait re-arms (observed waiting_until
moving). It never stores a roster, a cwd, a task or a message. The summary is
rewritten after every poll, to --out (default ~/.local/state/muster/idle-turns),
never inside the repository.

A turn shorter than the interval can fall between two polls, so turns can be
undercounted; the summary says so. Run with python3 -I, for example:

    python3 -I scripts/idle-turns.py --hours 12

`--self-test` checks the counting on made-up rosters and exits.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from collections import defaultdict
from datetime import datetime, timezone

# Claude's manifest says `busy` while a turn runs, `idle` at the prompt, and
# `shell` while a shell command runs, which on an idle session can last for hours
# (a background task). Only `busy` is a model turn.
BUSY = "busy"


class Counter:
    """Aggregates only. Nothing from a roster is kept beyond the last value of
    each counted field, per session."""

    def __init__(self):
        self.last = {}  # session key -> (activity, last_activity_at, waiting_until)
        self.hours = defaultdict(lambda: defaultdict(int))  # (hour, key) -> counts
        self.failed_polls = 0
        self.polls = 0

    def observe(self, roster, now_s):
        self.polls += 1
        hour = datetime.fromtimestamp(now_s, timezone.utc).strftime("%Y-%m-%dT%H")
        for s in roster.get("sessions", []):
            sid = s.get("session_id") or ""
            key = (
                (s.get("attested") or {}).get("machine") or "?",
                sid[:8],
                s.get("nickname") or "?",
            )
            declared = s.get("declared") or {}
            observed = s.get("observed") or {}
            activity = declared.get("activity")
            seen_at = declared.get("last_activity_at")
            until = observed.get("waiting_until")
            c = self.hours[(hour, key)]
            c["polls"] += 1
            if activity != BUSY:
                c["idle_polls"] += 1
            prev = self.last.get(key)
            if prev is not None:
                p_activity, p_seen, p_until = prev
                if p_activity != BUSY and activity == BUSY:
                    c["turns"] += 1
                if seen_at is not None and seen_at != p_seen:
                    c["activity_bumps"] += 1
                if until is not None and (p_until is None or until > p_until):
                    c["wait_rearms"] += 1
            self.last[key] = (activity, seen_at, until)

    def summary(self, interval):
        rows = []
        for (hour, (host, sid, nick)), c in sorted(self.hours.items()):
            idle_share = c["idle_polls"] / c["polls"] if c["polls"] else 0
            rows.append(
                {
                    "hour_utc": hour,
                    "host": host,
                    "session": f"{sid} {nick}",
                    "polls": c["polls"],
                    "idle_share": round(idle_share, 2),
                    "turns": c["turns"],
                    "activity_bumps": c["activity_bumps"],
                    "wait_rearms": c["wait_rearms"],
                }
            )
        # MUS-1's measure: the most turns any session took in an hour it spent
        # almost wholly idle.
        idle_hours = [r for r in rows if r["idle_share"] >= 0.9]
        worst = max(idle_hours, key=lambda r: r["turns"], default=None)
        return {
            "polls": self.polls,
            "failed_polls": self.failed_polls,
            "interval_s": interval,
            "note": f"a turn shorter than {interval} s can fall between polls, so turns "
            "may be undercounted; an idle hour is one with idle_share >= 0.9",
            "idle_hours": len(idle_hours),
            "max_turns_in_an_idle_hour": worst["turns"] if worst else None,
            "worst_idle_hour": worst,
            "rows": rows,
        }


def render(summary):
    out = [
        f"polls {summary['polls']}, failed {summary['failed_polls']}, "
        f"every {summary['interval_s']} s",
        f"note: {summary['note']}",
        f"idle hours: {summary['idle_hours']}, "
        f"max turns in an idle hour: {summary['max_turns_in_an_idle_hour']}",
        "",
        f"{'hour (UTC)':<14} {'host':<8} {'session':<28} {'polls':>5} {'idle':>5} "
        f"{'turns':>5} {'bumps':>5} {'waits':>5}",
    ]
    for r in summary["rows"]:
        out.append(
            f"{r['hour_utc']:<14} {r['host']:<8} {r['session']:<28} {r['polls']:>5} "
            f"{r['idle_share']:>5.2f} {r['turns']:>5} {r['activity_bumps']:>5} "
            f"{r['wait_rearms']:>5}"
        )
    return "\n".join(out) + "\n"


def write_atomic(path, text):
    d = os.path.dirname(path)
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d)
    with os.fdopen(fd, "w") as f:
        f.write(text)
    os.replace(tmp, path)


def read_roster(shim):
    done = subprocess.run(
        [shim, "roster"], capture_output=True, text=True, timeout=30, check=True
    )
    return json.loads(done.stdout)


def self_test():
    def roster(*rows):
        return {
            "sessions": [
                {
                    "session_id": f"{sid}-0000",
                    "nickname": nick,
                    "attested": {"machine": host},
                    "declared": {"activity": act, "last_activity_at": seen},
                    "observed": {"waiting_until": until},
                }
                for host, sid, nick, act, seen, until in rows
            ]
        }

    c = Counter()
    t = 1793916000  # 2026-11-05T22:00Z
    seq = [
        ("idle", 1, 100),
        ("idle", 1, 100),
        ("busy", 2, 100),  # a turn
        ("idle", 3, 200),  # wait re-armed after it
        ("idle", 3, 200),
        ("shell", 4, 200),  # a shell command, not a turn
        ("busy", 5, 200),  # another turn
        ("idle", 6, 300),
    ]
    for i, (act, seen, until) in enumerate(seq):
        c.observe(roster(("a1", "aaaaaaaa", "x-y", act, seen, until)), t + i * 10)
    # An idle session with a background shell command takes no turns.
    for i in range(3):
        c.observe(roster(("Murph", "bbbbbbbb", "p-q", "shell", 9, 50)), t + 100 + i * 10)
    s = c.summary(10)
    rows = {r["session"]: r for r in s["rows"]}
    a = rows["aaaaaaaa x-y"]
    assert a["turns"] == 2, a
    assert a["wait_rearms"] == 2, a
    assert a["activity_bumps"] == 5, a
    assert a["host"] == "a1" and rows["bbbbbbbb p-q"]["host"] == "Murph"
    assert rows["bbbbbbbb p-q"]["turns"] == 0
    assert s["idle_hours"] == 1 and s["max_turns_in_an_idle_hour"] == 0, s
    # Nothing but aggregates in the output.
    text = json.dumps(s)
    assert "cwd" not in text and "current_task" not in text
    print("self-test ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--interval", type=int, default=10, help="seconds between polls")
    ap.add_argument("--hours", type=float, default=12, help="how long to run")
    ap.add_argument(
        "--out",
        default=os.path.expanduser("~/.local/state/muster/idle-turns"),
        help="directory for the summary (never the repository)",
    )
    ap.add_argument("--shim", default="muster-shim")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%MZ")
    base = os.path.join(args.out, f"idle-turns-{stamp}")
    print(f"writing {base}.txt and {base}.json every poll", file=sys.stderr)
    c = Counter()
    end = time.time() + args.hours * 3600
    while time.time() < end:
        started = time.time()
        try:
            c.observe(read_roster(args.shim), started)
        except (subprocess.SubprocessError, OSError, ValueError):
            c.failed_polls += 1
        s = c.summary(args.interval)
        write_atomic(base + ".json", json.dumps(s, indent=1) + "\n")
        write_atomic(base + ".txt", render(s))
        time.sleep(max(0, args.interval - (time.time() - started)))


if __name__ == "__main__":
    main()

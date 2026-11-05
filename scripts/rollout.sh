#!/bin/zsh
# Roll a commit on main out to this machine's Muster daemon (MUS-55).
#
#   rollout.sh preflight <commit>                      build, test, stage, preview; changes nothing
#   rollout.sh apply <commit> --operator-approved      back up, install, restart, check, roll back
#
# `apply` restarts the daemon, so it needs the operator's direct OK, and refuses
# to run without --operator-approved. Run `preflight` first: `apply` installs only
# what `preflight` staged for that commit.
#
# Settings, from the environment:
#   MUSTER_UNIT     systemd user unit (default muster-daemon.service)
#   MUSTER_BIN_DIR  where the unit's binaries live (default ~/.cargo/bin)
#   ROLLOUT_DIR     build and staging area, on a real disk (default ~/.cache/muster-rollout)
#   BACKUP_DIR      backups (default ~/muster-backups)
#   HEALTH_WAIT     seconds to wait after a restart before checking (default 15)
setopt errexit nounset pipefail
SELF=$0
REPO=$(cd "$(dirname "$0")/.." && pwd)
UNIT=${MUSTER_UNIT:-muster-daemon.service}
BIN_DIR=${MUSTER_BIN_DIR:-$HOME/.cargo/bin}
ROLLOUT_DIR=${ROLLOUT_DIR:-$HOME/.cache/muster-rollout}
BACKUP_DIR=${BACKUP_DIR:-$HOME/muster-backups}
HEALTH_WAIT=${HEALTH_WAIT:-15}

say() { print -r -- "rollout: $*"; }
die() { print -ru2 -- "rollout: $*"; exit 1; }

# A value from the unit's environment, e.g. `unit_env MUSTER_BIND`. Values can
# hold spaces (a Postgres URL does), so parse systemd's quoting properly.
unit_env() {
  systemctl --user show "$UNIT" -p Environment --value | python3 -I -c '
import shlex, sys
for kv in shlex.split(sys.stdin.read()):
    k, _, v = kv.partition("=")
    if k == sys.argv[1]:
        print(v)
        break' "$1"
}
port_of() { print -r -- "${1##*:}"; }
# Whether the unit turns the TLS gate on (MUS-64). A binary from MUS-64 on runs
# plain without it, and refuses to start at all while it has paired machines.
unit_tls() { [[ ${$(unit_env MUSTER_TLS):l} == (1|true|on|yes) ]]; }
listening() { ss -Hltn "sport = :$1" | grep -q .; }

# The daemon is healthy when the unit is active, hasn't restarted since `$1`,
# runs the binary `$2`, and all its listeners accept connections. A binary from
# MUS-7 on also runs myko's own listener (`MUSTER_MYKO_BIND`, 6258), so that's
# checked only for a binary that knows the setting, which keeps a rollback to an
# older one checkable.
healthy() {
  local restarts_before=$1 expected=$2 now pid
  [[ $(systemctl --user is-active "$UNIT") == active ]] || { say "unit not active"; return 1; }
  now=$(systemctl --user show "$UNIT" -p NRestarts --value)
  (( now == restarts_before )) || { say "unit restarted itself ($restarts_before -> $now)"; return 1; }
  pid=$(systemctl --user show "$UNIT" -p MainPID --value)
  cmp -s "/proc/$pid/exe" "$expected" || { say "PID $pid isn't running $expected"; return 1; }
  local bind
  # Unset, each takes the daemon's default port.
  local gate=$(unit_env MUSTER_BIND) hook=$(unit_env MUSTER_HOOK_BIND) internal=$(unit_env MUSTER_INTERNAL_BIND)
  local -a binds=("${gate:-127.0.0.1:6255}" "${hook:-127.0.0.1:6256}")
  # A plain daemon (MUS-64) has no local gate and no separate myko port, and
  # a build from MUS-70 on has no 6257 local gate at all. An older build, as a
  # rollback target, still binds it.
  if grep -qa MUSTER_TLS "$expected" && ! unit_tls; then
    :
  elif grep -qa "MUSTER_INTERNAL_BIND is ignored" "$expected"; then
    :
  else
    binds+=("${internal:-127.0.0.1:6257}")
  fi
  if grep -qa MUSTER_MYKO_BIND "$expected" && { ! grep -qa MUSTER_TLS "$expected" || unit_tls; }; then
    local myko=$(unit_env MUSTER_MYKO_BIND)
    binds+=("${myko:-127.0.0.1:6258}")
  fi
  for bind in "${binds[@]}"; do
    listening "$(port_of "$bind")" || { say "nothing listening on $bind"; return 1; }
  done
  return 0
}

resolve() {
  git -C "$REPO" fetch -q origin
  local sha
  sha=$(git -C "$REPO" rev-parse --verify "$1^{commit}") || die "no such commit: $1"
  git -C "$REPO" merge-base --is-ancestor "$sha" origin/main || die "$sha isn't on origin/main"
  print -r -- "$sha"
}

preflight() {
  local sha=$1 area=$ROLLOUT_DIR/$1
  say "preflight $sha"
  # A fresh worktree means a fresh target directory, so a failed build can't
  # leave an older binary to be staged under this commit's name.
  [[ -d $area/src ]] && git -C "$REPO" worktree remove --force "$area/src"
  rm -rf "$area"; mkdir -p "$area/stage"
  git -C "$REPO" worktree add -q --detach "$area/src" "$sha"
  (
    cd "$area/src"
    say "building (debug) and running the gate e2e"
    cargo build --workspace --locked >"$area/build-debug.log" 2>&1 || die "debug build failed: $area/build-debug.log"
    local home; home=$(mktemp -d)
    env -u MYKO_POSTGRES_URL -u MUSTER_OPERATOR HOME="$home" zsh ./scripts/test-gate-e2e.sh >"$area/e2e.log" 2>&1 \
      || { rm -rf "$home"; die "gate e2e failed: $area/e2e.log"; }
    rm -rf "$home"
    say "$(grep -E '^--- [0-9]+ passed' "$area/e2e.log")"
    say "building (release)"
    cargo build --release --locked -p muster-daemon -p muster-shim >"$area/build-release.log" 2>&1 \
      || die "release build failed: $area/build-release.log"
    install -m 755 target/release/muster-daemon target/release/muster-shim "$area/stage/"
  )
  print -r -- "$sha" >"$area/stage/COMMIT"
  (cd "$area/stage" && sha256sum muster-daemon muster-shim >SHA256SUMS)
  git -C "$REPO" worktree remove --force "$area/src"
  local url; url=$(unit_env MYKO_POSTGRES_URL)
  if [[ -n $url ]]; then
    zsh "$REPO/scripts/restart-preview.sh" "$url" >"$area/restart-preview.txt" 2>&1 || say "restart-preview failed: $area/restart-preview.txt"
    say "restart preview:"; sed 's/^/  /' "$area/restart-preview.txt"
  fi
  if grep -qa MUSTER_TLS "$area/stage/muster-daemon" && ! unit_tls; then
    say "WARNING: $UNIT doesn't set MUSTER_TLS=1. This daemon runs plain without it (MUS-64),"
    say "  and refuses to start while it has paired machines. To keep the gate, add"
    say "  Environment=MUSTER_TLS=1 to the unit (systemctl --user edit $UNIT) before applying."
  fi
  say "staged in $area/stage; nothing installed. To deploy, with the operator's OK:"
  say "  $SELF apply $sha --operator-approved"
}

backup() {
  local sha=$1 dir=$2 url
  mkdir -p -m 700 "$dir"
  cp -p "$BIN_DIR/muster-daemon" "$dir/muster-daemon.old"
  [[ -e $BIN_DIR/muster-shim ]] && cp -p "$BIN_DIR/muster-shim" "$dir/muster-shim.old"
  systemctl --user cat "$UNIT" >"$dir/$UNIT.old"
  url=$(unit_env MYKO_POSTGRES_URL)
  if [[ -z $url ]]; then
    say "no MYKO_POSTGRES_URL on $UNIT; no database to back up"
    return
  fi
  zsh "$REPO/scripts/restart-preview.sh" "$url" >"$dir/restart-preview.txt" 2>&1 || true
  pg_dump -d "$url" -Fc -f "$dir/muster.dump"
  # Prove the dump restores, in a throwaway server with a short socket path
  # (a Unix socket path can't exceed 107 bytes).
  local data sock q got want max
  data=$(mktemp -d); sock=$(mktemp -d /tmp/mr.XXXXXX)
  initdb -D "$data" -U "$USER" --auth=trust -c listen_addresses='' -c unix_socket_directories="$sock" >/dev/null
  pg_ctl -D "$data" -w -l "$data/log" start >/dev/null
  {
    createdb -h "$sock" restore_test
    pg_restore -h "$sock" -d restore_test --no-owner --exit-on-error "$dir/muster.dump"
    q="select count(*), md5(string_agg(id::text||item_type||item_id||change_type||event::text, '|' order by id)) from myko_events"
    max=$(psql -h "$sock" -d restore_test -Atc "select coalesce(max(id), 0) from myko_events")
    got=$(psql -h "$sock" -d restore_test -Atc "$q")
    want=$(PGOPTIONS='-c default_transaction_read_only=on' psql -d "$url" -Atc "$q where id <= $max")
  } always {
    pg_ctl -D "$data" -w stop -m fast >/dev/null || true
    rm -rf "$data" "$sock"
  }
  [[ $got == "$want" ]] || die "backup didn't restore identically: $got vs $want"
  say "backup restored identically in a throwaway server: ${got%%|*} rows"
}

swap_in() {
  install -m 755 "$1" "$BIN_DIR/muster-daemon.new"
  mv -f "$BIN_DIR/muster-daemon.new" "$BIN_DIR/muster-daemon"
}

apply() {
  local sha=$1 stage=$ROLLOUT_DIR/$1/stage
  [[ -f $stage/COMMIT && $(<"$stage/COMMIT") == "$sha" ]] || die "nothing staged for $sha; run preflight first"
  (cd "$stage" && sha256sum --quiet -c SHA256SUMS) || die "staged binaries don't match their checksums"
  local dir=$BACKUP_DIR/rollout-${sha:0:7}-$(date +%Y%m%dT%H%M%S)
  say "apply $sha; backing up to $dir"
  backup "$sha" "$dir"
  local restarts started
  restarts=$(systemctl --user show "$UNIT" -p NRestarts --value)
  swap_in "$stage/muster-daemon"
  started=$(date '+%F %T')
  say "restarting $UNIT"
  systemctl --user restart "$UNIT" || say "restart failed"
  sleep "$HEALTH_WAIT"
  if ! healthy "$restarts" "$stage/muster-daemon"; then
    say "UNHEALTHY; rolling back to the previous binary"
    swap_in "$dir/muster-daemon.old"
    # A crashing binary can trip systemd's start limit, which would refuse the
    # restart below, so clear it first.
    systemctl --user reset-failed "$UNIT" 2>/dev/null || true
    restarts=$(systemctl --user show "$UNIT" -p NRestarts --value)
    systemctl --user restart "$UNIT" || say "restart of the previous binary failed"
    sleep "$HEALTH_WAIT"
    if healthy "$restarts" "$dir/muster-daemon.old"; then
      die "ROLLED BACK: $UNIT is running the previous binary again; backup in $dir"
    fi
    die "ROLLBACK FAILED: $UNIT is unhealthy on the previous binary too; backup in $dir"
  fi
  [[ -e $stage/muster-shim ]] && install -m 755 "$stage/muster-shim" "$BIN_DIR/muster-shim.new" && mv -f "$BIN_DIR/muster-shim.new" "$BIN_DIR/muster-shim"
  say "deployed $sha: $UNIT PID $(systemctl --user show "$UNIT" -p MainPID --value), healthy after ${HEALTH_WAIT}s"
  say "daemon log since $started:"
  journalctl --user -u "$UNIT" --since "$started" --no-pager -o cat | grep -v 'already exists' | sed 's/^/  /' | tail -20
  say "backup in $dir. Watch the log for a few minutes; running sessions keep their old shim until they restart."
}

case ${1:-} in
  preflight)
    [[ $# == 2 ]] || die "usage: $SELF preflight <commit>"
    sha=$(resolve "$2")
    preflight "$sha" ;;
  apply)
    [[ $# == 3 && $3 == --operator-approved ]] || die "usage: $SELF apply <commit> --operator-approved (needs the operator's direct OK)"
    sha=$(resolve "$2")
    apply "$sha" ;;
  *) die "usage: $SELF preflight <commit> | apply <commit> --operator-approved" ;;
esac

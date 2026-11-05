#!/bin/zsh
# Check that `rollout.sh apply` deploys a healthy daemon and rolls back one that
# fails its health check (MUS-55), against a throwaway systemd user unit on spare
# ports, never the real one. Needs systemd --user, ss and a debug build
# (`cargo build`), so it runs on a daemon host rather than in CI.
setopt nounset pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
GOOD=$REPO/target/debug/muster-daemon
[[ -x $GOOD ]] || { print -u2 "build first: cargo build"; exit 2; }
UNIT=muster-rollout-test-$$.service
WORK=$(mktemp -d "$HOME/.cache/muster-rollout-test.XXXX")
pass=0; fail=0
listening() { ss -Hltn "sport = :$1" | grep -q .; }
check() { if eval "$2"; then print "PASS  $1"; pass=$((pass+1)); else print "FAIL  $1"; fail=$((fail+1)); fi; }

mkdir -p "$WORK/bin" "$WORK/state" "$HOME/.config/systemd/user"
install -m 755 "$GOOD" "$WORK/bin/muster-daemon"
cat >"$HOME/.config/systemd/user/$UNIT" <<UNITFILE
[Unit]
Description=Muster rollout test (throwaway)

[Service]
ExecStart=$WORK/bin/muster-daemon
Environment=MUSTER_STATE_DIR=$WORK/state
Environment=MUSTER_TLS=1
Environment=MUSTER_BIND=127.0.0.1:18255
Environment=MUSTER_HOOK_BIND=127.0.0.1:18256
Environment=MUSTER_INTERNAL_BIND=127.0.0.1:18257
Environment=RUST_LOG=info
Restart=on-failure
RestartSec=1
UNITFILE
cleanup() {
  systemctl --user stop "$UNIT" 2>/dev/null
  rm -f "$HOME/.config/systemd/user/$UNIT"
  systemctl --user daemon-reload
  systemctl --user reset-failed "$UNIT" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT
systemctl --user daemon-reload
systemctl --user start "$UNIT"
for i in {1..50}; do listening 18256 && break; sleep 0.2; done
check "the throwaway daemon starts" 'listening 18256'

# Stage a binary for a real commit on main, as preflight would.
git -C "$REPO" fetch -q origin
SHA=$(git -C "$REPO" rev-parse origin/main)
stage() {
  mkdir -p "$WORK/rollout/$SHA/stage"
  install -m 755 "$1" "$WORK/rollout/$SHA/stage/muster-daemon"
  print -r -- "$SHA" >"$WORK/rollout/$SHA/stage/COMMIT"
  (cd "$WORK/rollout/$SHA/stage" && sha256sum muster-daemon >SHA256SUMS)
}
run() {
  env MUSTER_UNIT=$UNIT MUSTER_BIN_DIR=$WORK/bin ROLLOUT_DIR=$WORK/rollout BACKUP_DIR=$WORK/backups \
    HEALTH_WAIT=6 zsh "$REPO/scripts/rollout.sh" "$@" >"$WORK/out" 2>&1
}

run apply "$SHA"; RC=$?
check "apply refuses without --operator-approved" '[[ $RC != 0 ]] && grep -q "operator" $WORK/out'

stage "$GOOD"
run apply "$SHA" --operator-approved; RC=$?
check "a healthy binary deploys" '[[ $RC == 0 ]] && grep -q "deployed $SHA" $WORK/out'

# A binary that exits at once: systemd restarts it, so the health check fails.
print -r -- '#!/bin/sh
exit 1' >"$WORK/bad"; chmod 755 "$WORK/bad"
stage "$WORK/bad"
run apply "$SHA" --operator-approved; RC=$?
check "a failing binary is rolled back" '[[ $RC != 0 ]] && grep -q "ROLLED BACK" $WORK/out'
[[ $fail == 0 ]] || sed 's/^/    /' "$WORK/out"
check "and the previous binary is running again" 'cmp -s "$WORK/bin/muster-daemon" "$GOOD" && listening 18256 && [[ $(systemctl --user is-active $UNIT) == active ]]'

print -r -- "tampered" >>"$WORK/rollout/$SHA/stage/muster-daemon"
run apply "$SHA" --operator-approved; RC=$?
check "a staged binary that doesn't match its checksum isn't installed" '[[ $RC != 0 ]] && grep -q "checksums" $WORK/out && cmp -s "$WORK/bin/muster-daemon" "$GOOD"'

print -r -- "--- $pass passed, $fail failed"
[[ $fail == 0 ]]

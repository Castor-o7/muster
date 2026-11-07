#!/bin/sh
# End-to-end check of scripts/setup.sh (MUS-98) on a Linux daemon host: the unit it gets
# `muster-daemon service` to write really runs under systemd. Everything setup.sh touches is in a
# throwaway HOME and prefix; the unit is then run under a throwaway name on spare ports with its
# own state directory, and removed. The machine's own muster-daemon.service is never touched, and
# the test fails if its file changes. Needs systemd --user, ss, and release builds
# (`cargo build --release -p muster-daemon -p muster-shim`), so it runs on a host, not in CI.
set -u
REPO=$(cd "$(dirname "$0")/.." && pwd)
BUILT=${MUSTER_BUILT:-$REPO/target/release}
for b in muster-daemon muster-shim; do
    [ -x "$BUILT/$b" ] || { echo "build first: cargo build --release -p muster-daemon -p muster-shim" >&2; exit 2; }
done
REAL_HOME=$HOME
REAL_UNIT="$REAL_HOME/.config/systemd/user/muster-daemon.service"
real_before=$(sha256sum "$REAL_UNIT" 2>/dev/null || echo none)
WORK=$(mktemp -d "$REAL_HOME/.cache/muster-setup-test.XXXX")
TEST_UNIT="muster-setup-test-$$.service"
cleanup() {
    systemctl --user stop "$TEST_UNIT" 2>/dev/null
    rm -f "$REAL_HOME/.config/systemd/user/$TEST_UNIT"
    systemctl --user daemon-reload
    rm -rf "$WORK"
}
trap cleanup EXIT INT TERM
pass=0
fail=0
check() {
    if eval "$2"; then echo "PASS  $1"; pass=$((pass + 1)); else echo "FAIL  $1"; fail=$((fail + 1)); fi
}
listening() { ss -Hltn "sport = :$1" | grep -q .; }

# setup.sh builds from source here, pinned with --rev to this checkout's commit, so it doesn't ask
# GitHub for the latest release (MUS-111); this cargo "builds" by copying the release builds, and this
# claude stands in for Claude Code's CLI, so nothing is fetched and no real Claude config changes.
mkdir -p "$WORK/fake" "$WORK/home"
cat >"$WORK/fake/cargo" <<EOF
#!/bin/sh
root=""; prev=""
for a in "\$@"; do [ "\$prev" = --root ] && root=\$a; prev=\$a; done
mkdir -p "\$root/bin"
cp "$BUILT/muster-daemon" "$BUILT/muster-shim" "\$root/bin/"
EOF
printf '#!/bin/sh\necho "claude $*" >> "%s/claude-calls"\n' "$WORK" >"$WORK/fake/claude"
for tool in systemctl launchctl; do printf '#!/bin/sh\necho "%s $*" >> "%s/start-calls"\n' "$tool" "$WORK" >"$WORK/fake/$tool"; done
chmod +x "$WORK/fake"/*

# MUSTER_DAEMON_ADDRESS points the doctor at the spare port, so it never reaches the host's daemon.
HOME="$WORK/home" PATH="$WORK/prefix/bin:$WORK/fake:/usr/bin:/bin" MUSTER_PREFIX="$WORK/prefix" \
    MUSTER_DAEMON_ADDRESS=ws://127.0.0.1:18455 \
    sh "$REPO/scripts/setup.sh" --daemon --tls --rev "$(git -C "$REPO" rev-parse HEAD)" --apply >"$WORK/out" 2>&1
echo "setup.sh exited $?; its output:"
sed 's/^/  | /' "$WORK/out"
UNIT="$WORK/home/.config/systemd/user/muster-daemon.service"
check "setup.sh installed the daemon into the prefix" '[ -x "$WORK/prefix/bin/muster-daemon" ]'
check "and muster-daemon service --apply wrote a unit in that HOME" '[ -f "$UNIT" ]'
check "running the installed binary, with TLS" 'grep -qx "ExecStart=$WORK/prefix/bin/muster-daemon" "$UNIT" && grep -qx "Environment=MUSTER_TLS=1" "$UNIT"'
check "setup.sh started nothing" '[ ! -s "$WORK/start-calls" ]'
check "and the machine's own unit is unchanged" '[ "$(sha256sum "$REAL_UNIT" 2>/dev/null || echo none)" = "$real_before" ]'

# Run that unit for real, under a throwaway name, on spare ports and its own state.
mkdir -p "$WORK/state"
sed "/^\[Service\]/a Environment=MUSTER_STATE_DIR=$WORK/state\nEnvironment=MUSTER_BIND=127.0.0.1:18455\nEnvironment=MUSTER_HOOK_BIND=127.0.0.1:18456\nEnvironment=MUSTER_INTERNAL_BIND=127.0.0.1:18457\nEnvironment=MUSTER_MYKO_BIND=127.0.0.1:18458" \
    "$UNIT" >"$REAL_HOME/.config/systemd/user/$TEST_UNIT"
systemctl --user daemon-reload
systemctl --user start "$TEST_UNIT"
up=0
for _ in $(seq 1 30); do listening 18455 && { up=1; break; }; sleep 1; done
check "the written unit runs under systemd and the daemon listens" '[ $up = 1 ] && systemctl --user is-active --quiet "$TEST_UNIT"'
systemctl --user stop "$TEST_UNIT"
check "and stops cleanly" '! systemctl --user is-active --quiet "$TEST_UNIT" && ! listening 18455'
check "the machine's own daemon is still running" 'systemctl --user is-active --quiet muster-daemon.service'

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

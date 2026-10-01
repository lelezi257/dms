#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
TMP=${TMPDIR:-/tmp}/afs-deploy-selftest-$$
cleanup() {
  for pid in ${FAKE_PID:-} ${TERM_PID:-} ${MOUNT_RECOVERY_PIDS:-}; do
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      sleep 1
      kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null || true
    fi
  done
  rm -rf "$TMP"
}
trap cleanup EXIT

pass() { printf 'ok - %s\n' "$1"; }
fail() { printf 'not ok - %s\n' "$1" >&2; exit 1; }

mkdir -p "$TMP"

free_port() {
  python3 - <<'PY_PORT'
import socket
s = socket.socket()
s.bind(('127.0.0.1', 0))
print(s.getsockname()[1])
s.close()
PY_PORT
}

if "$ROOT/dep02-smoke.sh" --mount /tmp --name should-fail >"$TMP/smoke.out" 2>"$TMP/smoke.err"; then
  cat "$TMP/smoke.out" "$TMP/smoke.err" >&2
  fail "dep02-smoke rejects ordinary /tmp"
fi
if grep -Eq 'findmnt is required|not an exact mount point|not a FUSE filesystem|not an AFS' "$TMP/smoke.err"; then
  pass "dep02-smoke rejects ordinary /tmp"
else
  cat "$TMP/smoke.err" >&2
  fail "dep02-smoke reports mount validation failure"
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo 'skip - fake readiness check requires python3'
  exit 0
fi

READY_PORT=$(free_port)
IDENTITY_PORT=$(free_port)
mkdir -p "$TMP/prefix/bin" "$TMP/etc" "$TMP/run" "$TMP/log" "$TMP/state" "$TMP/mnt"
# Keep the readiness fixture a native executable, just like shipped AFS
# binaries, so executable/PID/start-tick identity checks remain meaningful.
cat > "$TMP/ready-fixture.c" <<'BIN'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
static void stop(int sig) { (void)sig; _exit(0); }
int main(int argc, char **argv) {
  if (argc != 3) return 2;
  FILE *f = fopen(argv[2], "r");
  if (!f) return 2;
  char line[512]; unsigned port = 0;
  while (fgets(line, sizeof line, f)) {
    if (sscanf(line, "rest_listen = \"127.0.0.1:%u", &port) != 1)
      sscanf(line, "rest_listen = '127.0.0.1:%u", &port);
  }
  fclose(f);
  signal(SIGTERM, stop);
  int s = socket(AF_INET, SOCK_STREAM, 0), one = 1;
  setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
  struct sockaddr_in addr = { .sin_family = AF_INET, .sin_port = htons(port), .sin_addr.s_addr = htonl(INADDR_LOOPBACK) };
  if (bind(s, (struct sockaddr*)&addr, sizeof addr) || listen(s, 8)) return 2;
  for (;;) {
    int c = accept(s, NULL, NULL);
    if (c < 0) continue;
    char request[1024]; (void)read(c, request, sizeof request);
    const char *body = "HTTP/1.1 200 OK\r\nContent-Length: 18\r\nConnection: close\r\n\r\n{\"status\":\"ready\"}";
    (void)write(c, body, strlen(body)); close(c);
  }
}
BIN
cc -Wall -Wextra -Werror "$TMP/ready-fixture.c" -o "$TMP/prefix/bin/afs-node"
cat > "$TMP/etc/node.toml" <<EOF
grpc_listen = "127.0.0.1:0"
rest_listen = "127.0.0.1:$READY_PORT"
dfs_mount = "$TMP/mnt"
ownerfs_mount = "$TMP/mnt-owner"
EOF

if "$ROOT/afs-processctl" \
  --prefix "$TMP/prefix" \
  --config-dir "$TMP/etc" \
  --run-dir "$TMP/run" \
  --log-dir "$TMP/log" \
  --timeout 2 \
  start dfs >"$TMP/start.out" 2>"$TMP/start.err"; then
  cat "$TMP/start.out" "$TMP/start.err" >&2
  fail "processctl rejects ready REST without AFS mount"
fi
if [ -f "$TMP/run/node.pid" ]; then
  fail "processctl removes pid file after failed readiness"
fi
if command -v ss >/dev/null 2>&1 && ss -H -ltn "sport = :$READY_PORT" | grep -q .; then
  fail "processctl cleans fake process after failed readiness"
fi
if grep -Eq 'mount is not an exact mount point|mount is not FUSE|mount source mismatch|did not become ready' "$TMP/start.err"; then
  pass "processctl rejects ready REST without AFS mount"
else
  cat "$TMP/start.err" >&2
  fail "processctl reports mount readiness failure"
fi


# Valid deployed TOML can use either string delimiter and trailing comments.
# Use Meta readiness so actual HTTP readiness is exercised without fake mounts.
mkdir -p "$TMP/quotes/prefix/bin" "$TMP/quotes/etc" "$TMP/quotes/run" "$TMP/quotes/log"
cp "$TMP/prefix/bin/afs-node" "$TMP/quotes/prefix/bin/afs-meta"
for delimiter in "'" '"'; do
  QUOTE_PORT=$(free_port)
  cat > "$TMP/quotes/etc/meta.toml" <<EOF
grpc_listen = ${delimiter}127.0.0.1:0${delimiter} # no grpc fixture
rest_listen = ${delimiter}127.0.0.1:$QUOTE_PORT${delimiter} # health endpoint
EOF
  "$ROOT/afs-processctl" --prefix "$TMP/quotes/prefix" --config-dir "$TMP/quotes/etc" \
    --run-dir "$TMP/quotes/run" --log-dir "$TMP/quotes/log" --timeout 2 \
    start meta >"$TMP/quotes/start.out" 2>"$TMP/quotes/start.err" || {
      cat "$TMP/quotes/start.out" "$TMP/quotes/start.err" >&2
      fail "processctl accepts TOML literal/basic listen strings with comments"
    }
  curl -fsS "http://127.0.0.1:$QUOTE_PORT/health" >/dev/null
  "$ROOT/afs-processctl" --prefix "$TMP/quotes/prefix" --config-dir "$TMP/quotes/etc" \
    --run-dir "$TMP/quotes/run" --log-dir "$TMP/quotes/log" --timeout 2 \
    stop meta >"$TMP/quotes/stop.out" 2>"$TMP/quotes/stop.err"
  if ss -H -ltn "sport = :$QUOTE_PORT" | grep -q .; then
    fail "quoted listen fixture has stopped"
  fi
done
pass "processctl accepts TOML literal/basic listen strings with comments"

# Local process health must not be routed through an inherited HTTP proxy.
PROXY_PORT=$(free_port)
cat > "$TMP/quotes/etc/meta.toml" <<EOF
grpc_listen = "127.0.0.1:0"
rest_listen = "127.0.0.1:$PROXY_PORT"
EOF
if ! env http_proxy=http://127.0.0.1:9 HTTP_PROXY=http://127.0.0.1:9 \
  https_proxy=http://127.0.0.1:9 HTTPS_PROXY=http://127.0.0.1:9 \
  all_proxy=http://127.0.0.1:9 ALL_PROXY=http://127.0.0.1:9 no_proxy= NO_PROXY= \
  "$ROOT/afs-processctl" --prefix "$TMP/quotes/prefix" --config-dir "$TMP/quotes/etc" \
    --run-dir "$TMP/quotes/run" --log-dir "$TMP/quotes/log" --timeout 2 \
    start meta >"$TMP/quotes/proxy-start.out" 2>"$TMP/quotes/proxy-start.err"; then
  cat "$TMP/quotes/proxy-start.out" "$TMP/quotes/proxy-start.err" >&2
  fail "processctl probes local readiness directly despite inherited proxy"
fi
"$ROOT/afs-processctl" --prefix "$TMP/quotes/prefix" --config-dir "$TMP/quotes/etc" \
  --run-dir "$TMP/quotes/run" --log-dir "$TMP/quotes/log" --timeout 2 \
  stop meta >"$TMP/quotes/proxy-stop.out" 2>"$TMP/quotes/proxy-stop.err"
if ss -H -ltn "sport = :$PROXY_PORT" | grep -q .; then
  fail "proxied readiness fixture has stopped"
fi
pass "processctl probes local readiness directly despite inherited proxy"

mkdir -p "$TMP/invalid/prefix/bin" "$TMP/invalid/etc" "$TMP/invalid/run" "$TMP/invalid/log"
cp "$TMP/prefix/bin/afs-node" "$TMP/invalid/prefix/bin/afs-meta"
for invalid in "'127.0.0.1:40000" '"127.0.0.1:70000"'; do
  printf 'rest_listen = %s\n' "$invalid" >"$TMP/invalid/etc/meta.toml"
  if "$ROOT/afs-processctl" --prefix "$TMP/invalid/prefix" --config-dir "$TMP/invalid/etc" \
    --run-dir "$TMP/invalid/run" --log-dir "$TMP/invalid/log" --timeout 2 \
    start meta >"$TMP/invalid/start.out" 2>"$TMP/invalid/start.err"; then
    fail "invalid listen configuration must fail before launching product"
  fi
  [ ! -f "$TMP/invalid/run/meta.pid" ] && [ ! -f "$TMP/invalid/run/meta.launch" ] || \
    fail "invalid listen configuration leaves no managed child"
  [ -z "$(ls -A "$TMP/invalid/run")" ] || fail "invalid listen configuration leaves no launch directory"
done
pass "invalid listen configuration fails before creating a managed launch"

mkdir -p "$TMP/mount-recovery/bin"
cat > "$TMP/mount-recovery/bin/findmnt" <<'FAKE_FINDMNT'
#!/usr/bin/env bash
set -euo pipefail
state=$(cat "$AFS_FAKE_MOUNT_STATE_FILE")
mount=
column=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --mountpoint) mount=$2; shift 2 ;;
    -o) column=$2; shift 2 ;;
    *) shift ;;
  esac
done
[ "$state" != absent ] || exit 1
[ -n "$mount" ] || exit 1
target=$mount
fstype=fuse
case "$mount" in
  *owner*) source=afs-ownerfs ;;
  *) source=afs-dfs ;;
esac
case "$state" in
  foreign-source) source=foreign ;;
  foreign-fstype) fstype=ext4 ;;
  wrong-target) target="${mount}-other" ;;
esac
case "$column" in
  TARGET) printf '%s\n' "$target" ;;
  FSTYPE) printf '%s\n' "$fstype" ;;
  SOURCE) printf '%s\n' "$source" ;;
  *) printf '%s\n' "$target" ;;
esac
FAKE_FINDMNT
cat > "$TMP/mount-recovery/bin/stat" <<'FAKE_STAT'
#!/usr/bin/env bash
set -euo pipefail
state=$(cat "$AFS_FAKE_MOUNT_STATE_FILE")
path=${@: -1}
case "$state" in
  dead) echo "stat: cannot statx '$path': Transport endpoint is not connected" >&2; exit 1 ;;
  timeout-enotconn) echo "stat: cannot statx '$path': Transport endpoint is not connected" >&2; exit 1 ;;
  unknown-stat) echo "stat: cannot statx '$path': Permission denied" >&2; exit 1 ;;
  *) echo directory; exit 0 ;;
esac
FAKE_STAT
cat > "$TMP/mount-recovery/bin/timeout" <<'FAKE_TIMEOUT'
#!/usr/bin/env bash
set -euo pipefail
state=$(cat "$AFS_FAKE_MOUNT_STATE_FILE")
case "${1:-}" in --kill-after=*) shift ;; esac
[ "$#" -gt 0 ] || exit 125
shift
[ "$#" -gt 0 ] || exit 125
if [ "$state" = timeout-enotconn ] && [ "$(basename "$1")" = stat ]; then
  path=${@: -1}
  echo "stat: cannot statx '$path': Transport endpoint is not connected" >&2
  exit 124
fi
exec "$@"
FAKE_TIMEOUT
cat > "$TMP/mount-recovery/bin/fusermount3" <<'FAKE_FUSERMOUNT'
#!/usr/bin/env bash
set -euo pipefail
printf 'fusermount3 %s\n' "$*" >> "$AFS_FAKE_UNMOUNT_LOG"
printf 'absent\n' > "$AFS_FAKE_MOUNT_STATE_FILE"
FAKE_FUSERMOUNT
cat > "$TMP/mount-recovery/bin/umount" <<'FAKE_UMOUNT'
#!/usr/bin/env bash
set -euo pipefail
printf 'umount %s\n' "$*" >> "$AFS_FAKE_UNMOUNT_LOG"
printf 'absent\n' > "$AFS_FAKE_MOUNT_STATE_FILE"
FAKE_UMOUNT
chmod +x "$TMP/mount-recovery/bin/findmnt" "$TMP/mount-recovery/bin/stat" "$TMP/mount-recovery/bin/timeout" "$TMP/mount-recovery/bin/fusermount3" "$TMP/mount-recovery/bin/umount"

mount_recovery_lane() {
  lane=$1
  port=$2
  dir="$TMP/mount-recovery/$lane"
  mkdir -p "$dir/prefix/bin" "$dir/etc" "$dir/run" "$dir/log" "$dir/mount-dfs" "$dir/mount-owner"
  cp "$TMP/prefix/bin/afs-node" "$dir/prefix/bin/afs-node"
  cat > "$dir/etc/node.toml" <<EOF
grpc_listen = "127.0.0.1:0"
rest_listen = "127.0.0.1:$port"
dfs_mount = "$dir/mount-dfs"
ownerfs_mount = "$dir/mount-owner"
EOF
  printf '999999\n' > "$dir/run/node.pid"
  cat > "$dir/run/node.identity" <<EOF
pid=999999
exe=$(readlink -f "$dir/prefix/bin/afs-node")
config=$(readlink -f "$dir/etc/node.toml")
start_ticks=1
boot_id=$(cat /proc/sys/kernel/random/boot_id 2>/dev/null || true)
lifecycle=$dir/run/node.lifecycle.old
cmdline=old
EOF
  printf '%s\n' "$dir"
}

run_mount_recovery_case() {
  mode=$1
  expected=$2
  lane=$3
  pid_state=${4:-with-pid}
  port=$(free_port)
  dir=$(mount_recovery_lane "$lane" "$port")
  if [ "$pid_state" = no-pid ]; then
    rm -f "$dir/run/node.pid" "$dir/run/node.identity"
  fi
  state="$dir/state"
  unmount_log="$dir/unmount.log"
  printf '%s\n' "$mode" > "$state"
  : > "$unmount_log"
  set +e
  PATH="$TMP/mount-recovery/bin:$PATH" \
    AFS_FAKE_MOUNT_STATE_FILE="$state" \
    AFS_FAKE_UNMOUNT_LOG="$unmount_log" \
    "$ROOT/afs-processctl" --prefix "$dir/prefix" --config-dir "$dir/etc" \
      --run-dir "$dir/run" --log-dir "$dir/log" --timeout 2 --no-readiness \
      start dfs >"$dir/start.out" 2>"$dir/start.err"
  rc=$?
  set -e
  if [ "$rc" -eq 0 ] && [ -f "$dir/run/node.pid" ]; then
    MOUNT_RECOVERY_PIDS="${MOUNT_RECOVERY_PIDS:-} $(cat "$dir/run/node.pid")"
  fi
  if [ "$expected" = pass ]; then
    [ "$rc" -eq 0 ] || { cat "$dir/start.out" "$dir/start.err" >&2; fail "stale disconnected AFS mount is recovered"; }
    grep -q 'fusermount3 -uz' "$unmount_log" || { cat "$unmount_log" >&2; fail "stale recovery uses bounded FUSE detach"; }
    [ "$(cat "$state")" = absent ] || fail "stale recovery clears fake mount state"
    "$ROOT/afs-processctl" --prefix "$dir/prefix" --config-dir "$dir/etc" \
      --run-dir "$dir/run" --log-dir "$dir/log" --timeout 2 --no-readiness \
      stop dfs >"$dir/stop.out" 2>"$dir/stop.err" || {
        cat "$dir/stop.out" "$dir/stop.err" >&2
        fail "stale recovery fixture stops cleanly"
      }
  else
    [ "$rc" -ne 0 ] || fail "$lane refuses unsafe stale mount recovery"
    if [ "$pid_state" = with-pid ]; then
      [ -f "$dir/run/node.pid" ] && [ -f "$dir/run/node.identity" ] || fail "$lane preserves stale evidence on refusal"
    fi
    [ ! -s "$unmount_log" ] || { cat "$unmount_log" >&2; fail "$lane does not unmount unsafe target"; }
  fi
}

run_mount_recovery_case dead pass disconnected
run_mount_recovery_case dead pass disconnected-no-pid no-pid
run_mount_recovery_case live fail healthy
run_mount_recovery_case wrong-target fail wrong-target
run_mount_recovery_case foreign-source fail foreign-source
run_mount_recovery_case foreign-fstype fail foreign-fstype
run_mount_recovery_case timeout-enotconn fail timeout-enotconn
run_mount_recovery_case unknown-stat fail unknown-stat
pass "processctl recovers only positively disconnected exact AFS FUSE mounts"


# PID identity must include more than executable path. A pid file pointing to a
# different live process using the same binary must not be stopped.
mkdir -p "$TMP/identity/prefix/bin" "$TMP/identity/etc" "$TMP/identity/run" "$TMP/identity/log"
cp "$TMP/prefix/bin/afs-node" "$TMP/identity/prefix/bin/afs-node"
cat > "$TMP/identity/etc/node.toml" <<EOF
rest_listen = "127.0.0.1:$IDENTITY_PORT"
grpc_listen = "127.0.0.1:0"
dfs_mount = "$TMP/identity/mnt-dfs"
ownerfs_mount = "$TMP/identity/mnt-owner"
EOF
"$TMP/identity/prefix/bin/afs-node" --config "$TMP/identity/etc/node.toml" >/dev/null 2>&1 &
FAKE_PID=$!
sleep 1
printf '%s
' "$FAKE_PID" > "$TMP/identity/run/node.pid"
cat > "$TMP/identity/run/node.identity" <<EOF
pid=$FAKE_PID
exe=$(readlink -f "$TMP/identity/prefix/bin/afs-node")
config=$(readlink -f "$TMP/identity/etc/node.toml")
start_ticks=1
cmdline=wrong
EOF
if "$ROOT/afs-processctl"   --prefix "$TMP/identity/prefix"   --config-dir "$TMP/identity/etc"   --run-dir "$TMP/identity/run"   --log-dir "$TMP/identity/log"   --timeout 1   stop node >"$TMP/identity-stop.out" 2>"$TMP/identity-stop.err"; then
  cat "$TMP/identity-stop.out" "$TMP/identity-stop.err" >&2
  fail "processctl refuses mismatched same-binary pid identity"
fi
if kill -0 "$FAKE_PID" 2>/dev/null && grep -q 'pid file points to another live process' "$TMP/identity-stop.err"; then
  pass "processctl refuses mismatched same-binary pid identity"
else
  cat "$TMP/identity-stop.out" "$TMP/identity-stop.err" >&2
  fail "processctl preserves mismatched same-binary process"
fi

# Detached controllers cannot wait on the product themselves. Exercise the real
# supervisor with native child wait statuses, not a shell trap or forged log.
python3 - "$ROOT/afs-processctl" "$TMP/lifecycle" <<'PY_LIFECYCLE'
import ctypes
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import time

assert platform.system() == 'Linux'
controller, root = sys.argv[1], Path(sys.argv[2])
root.mkdir()
# Reap orphaned supervisors and the deliberate supervisor-loss fixture ourselves.
assert ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) == 0
source = root / 'fixture.c'
source.write_text(r'''
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <unistd.h>
#include <sys/prctl.h>
static int code, delay;
static void stop(int sig) { (void)sig; usleep(delay * 1000); _exit(code); }
int main(int argc, char **argv) {
  int immediate;
  if (argc != 3) return 2;
  FILE *cfg = fopen(argv[2], "r");
  if (!cfg || fscanf(cfg, "%d %d %d", &code, &delay, &immediate) != 3) return 2;
  fclose(cfg);
  prctl(PR_SET_NAME, "afs (fixture)", 0, 0, 0);
  if (immediate) return code;
  signal(SIGTERM, code == 999 ? SIG_IGN : stop);
  puts("fixture-ready"); fflush(stdout);
  for (;;) pause();
}
''')
exe = root / 'fixture'
subprocess.run(['cc', '-Wall', '-Wextra', '-Werror', str(source), '-o', str(exe)], check=True)
children = []
records = []


def fields(path):
    return dict(line.split('=', 1) for line in Path(path).read_text().splitlines())


def lane(name, code=0, delay=0, immediate=0):
    p = root / name
    for d in ('prefix/bin', 'etc', 'run', 'log'):
        (p / d).mkdir(parents=True)
    shutil.copy2(exe, p / 'prefix/bin/afs-node')
    (p / 'etc/node.toml').write_text(f'{code} {delay} {immediate}\n')
    return p


def argv(p, *args, timeout=3):
    return [controller, '--prefix', str(p/'prefix'), '--config-dir', str(p/'etc'),
            '--run-dir', str(p/'run'), '--log-dir', str(p/'log'), '--timeout', str(timeout),
            '--no-readiness', *args]


def run(p, *args, timeout=3):
    result = subprocess.run(argv(p, *args, timeout=timeout), capture_output=True, text=True, timeout=8)
    records.append({'command': args, 'lane': p.name, 'returncode': result.returncode,
                    'stdout': result.stdout, 'stderr': result.stderr})
    return result


def start(p, alias='node'):
    result = run(p, 'start', alias)
    assert result.returncode == 0, records[-1]
    pid = int((p/'run/node.pid').read_text())
    children.append(pid)
    identity = fields(p/'run/node.identity')
    child = fields(Path(identity['lifecycle'])/'child')
    assert int(child['pid']) == pid
    assert identity['start_ticks'] == child['start_ticks'] and identity['start_ticks']
    assert identity['boot_id'] == child['boot_id']
    assert identity['exe'] == str(p/'prefix/bin/afs-node')
    return pid, identity


def status(p):
    r = run(p, '--json', 'status', 'node')
    assert r.returncode == 0
    return json.loads(r.stdout)


def wait_for(predicate):
    end = time.monotonic()+3
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(.01)
    raise AssertionError('bounded fixture wait failed')


try:
    # Never-started idempotency and genuine exit propagation, including repeats.
    p = lane('never')
    assert run(p, 'stop', 'node').returncode == 0
    assert status(p)['state'] == 'stopped'
    for code in (0, 1, 124):
        p = lane(f'exit-{code}', code)
        pid, identity = start(p)
        assert status(p)['state'] == 'running'
        assert run(p, 'stop', 'dfs').returncode == code
        assert run(p, 'stop', 'ownerfs').returncode == code
        receipt = fields(Path(identity['lifecycle'])/'exit')
        assert receipt['exit_code'] == str(code)
        s = status(p)
        assert s['state'] == ('stopped' if code == 0 else 'failed') and s['exit_code'] == str(code)
        print(f'ok - native exit {code} and repeated alias stop retain exact result')

    # New start establishes a distinct generation; the previous receipt cannot
    # qualify its termination, even if copied to the new receipt path.
    p = lane('generation', 0)
    _, old = start(p)
    assert run(p, 'stop', 'node').returncode == 0
    previous_receipt = (Path(old['lifecycle'])/'exit').read_bytes()
    (p/'etc/node.toml').write_text('1 0 0\n')
    _, new = start(p)
    assert new['lifecycle'] != old['lifecycle'] and not Path(old['lifecycle']).exists()
    assert run(p, 'stop', 'node').returncode == 1
    receipt = Path(new['lifecycle'])/'exit'
    good = receipt.read_bytes()
    receipt.write_bytes(previous_receipt)
    assert run(p, 'stop', 'node', timeout=1).returncode != 0
    assert status(p)['state'] == 'exit-unknown'
    receipt.write_text('exit_code=0\n')
    assert run(p, 'stop', 'node', timeout=1).returncode != 0
    assert status(p)['state'] == 'exit-unknown'
    receipt.unlink()
    assert run(p, 'stop', 'node', timeout=1).returncode != 0
    assert status(p)['state'] == 'exit-unknown'
    receipt.write_bytes(good)
    assert run(p, 'stop', 'node').returncode == 1
    assert run(p, 'restart', 'node').returncode == 1
    assert run(p, 'uninstall', 'node').returncode == 1
    assert (p/'prefix/bin/afs-node').exists()
    print('ok - stale, corrupt, missing receipts and failed restart/uninstall cannot become clean')

    # A watchdog can bypass Rust cleanup; lost supervision cannot fabricate 0.
    p = lane('supervisor-loss')
    pid, identity = start(p)
    child = fields(Path(identity['lifecycle'])/'child')
    os.kill(int(child['supervisor_pid']), signal.SIGKILL)
    wait_for(lambda: not Path(f"/proc/{child['supervisor_pid']}/exe").exists())
    assert run(p, 'stop', 'node', timeout=1).returncode != 0
    assert status(p)['state'] == 'exit-unknown'
    assert not (Path(identity['lifecycle'])/'exit').exists()
    print('ok - lost supervisor leaves explicit unknown instead of clean stop')

    # Kernel signal exit also propagates the actual wait code.
    p = lane('signal-loss')
    pid, identity = start(p)
    os.kill(pid, signal.SIGKILL)
    wait_for(lambda: (Path(identity['lifecycle'])/'exit').exists())
    assert run(p, 'stop', 'node').returncode == 137
    assert status(p)['exit_code'] == '137'
    print('ok - SIGKILL propagates exit 137')

    # Zero timeout retains evidence and does not kill or detach the wait owner.
    p = lane('zero-timeout', delay=300)
    _, identity = start(p)
    r = run(p, 'stop', 'node', timeout=0)
    assert r.returncode != 0 and 'did not stop within' in r.stderr
    assert run(p, 'stop', 'node').returncode == 0
    print('ok - timeout zero reports failure; subsequent stop observes actual completion')

    # All aliases lock the same real process, and restart keeps its lock through
    # both stop and start. Rejected calls cannot overwrite identity or launch.
    p = lane('alias-race', delay=300)
    first = subprocess.Popen(argv(p, 'start', 'dfs'), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    wait_for(lambda: (p/'run/node.start.lock').exists())
    assert run(p, 'start', 'ownerfs').returncode != 0
    assert run(p, 'stop', 'node').returncode != 0
    out, err = first.communicate(timeout=8)
    assert first.returncode == 0, (out, err)
    pid = int((p/'run/node.pid').read_text()); children.append(pid)
    old = fields(p/'run/node.identity')
    restart = subprocess.Popen(argv(p, 'restart', 'node'), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    wait_for(lambda: (p/'run/node.start.lock').exists())
    assert run(p, 'start', 'dfs').returncode != 0
    out, err = restart.communicate(timeout=8)
    assert restart.returncode == 0, (out, err)
    children.append(int((p/'run/node.pid').read_text()))
    assert fields(p/'run/node.identity')['lifecycle'] != old['lifecycle']
    assert run(p, 'stop', 'ownerfs').returncode == 0
    assert (p/'log/node.log').read_text().count('fixture-ready') == 2
    print('ok - canonical alias locks serialize start, stop and complete restart')

    p = lane('zero-start')
    assert run(p, 'start', 'node', timeout=0).returncode != 0
    assert not (p/'run/node.pid').exists() and not (p/'run/node.launch').exists()
    assert run(p, 'stop', 'node').returncode == 0
    print('ok - timeout zero start cannot create a detached product')

    original_controller = controller
    text = Path(controller).read_text()
    slow = root/'slow-bootstrap-controller'
    slow.write_text(text.replace('  printf \'supervisor_pid=%s', "  sleep 2\n  printf 'supervisor_pid=%s", 1))
    slow.chmod(0o755)
    assert slow.read_text() != text
    controller = str(slow)
    p = lane('bootstrap-timeout')
    r = run(p, 'start', 'node', timeout=1)
    assert r.returncode != 0 and 'bootstrap failed before product launch' in r.stderr
    assert not (p/'run/node.pid').exists() and not (p/'run/node.launch').exists()
    assert 'fixture-ready' not in (p/'log/node.log').read_text()
    assert run(p, 'stop', 'node').returncode == 0
    print('ok - bootstrap expiry before go cancels supervisor without launching product')

    paused = root/'paused-bootstrap-controller'
    paused.write_text(text.replace('  printf \'supervisor_pid=%s', "  kill -STOP \"$$\"\n  printf 'supervisor_pid=%s", 1))
    paused.chmod(0o755)
    assert paused.read_text() != text
    controller = str(paused)
    p = lane('paused-bootstrap')
    began = time.monotonic()
    r = run(p, 'start', 'node', timeout=1)
    assert r.returncode != 0 and 'bootstrap failed before product launch' in r.stderr
    assert time.monotonic()-began < 3
    assert not (p/'run/node.pid').exists() and not (p/'run/node.launch').exists()
    assert 'fixture-ready' not in (p/'log/node.log').read_text()
    assert run(p, 'stop', 'node').returncode == 0
    print('ok - SIGSTOP bootstrap cleanup stays bounded and cannot launch product')

    late = root/'late-child-controller'
    late.write_text(text.replace('  ticks=$(sed', '  sleep 2\n  ticks=$(sed', 1))
    late.chmod(0o755)
    assert late.read_text() != text
    controller = str(late)
    p = lane('late-publication')
    r = run(p, 'start', 'node', timeout=1)
    assert r.returncode != 0 and 'retained managed launch' in r.stderr
    assert not (p/'run/node.pid').exists() and (p/'run/node.launch').exists()
    assert status(p)['state'] == 'exit-unknown'
    lifecycle = Path((p/'run/node.launch').read_text().strip())
    wait_for(lambda: (lifecycle/'child').exists())
    child = fields(lifecycle/'child'); children.append(int(child['pid']))
    assert run(p, 'stop', 'node').returncode == 0
    assert fields(p/'run/node.identity')['start_ticks'] == child['start_ticks']
    assert status(p)['exit_code'] == '0'
    print('ok - post-go delayed publication retains intent and stop recovers original child')
    controller = original_controller

    p = lane('immediate-failure', code=1, immediate=1)
    assert run(p, 'start', 'node').returncode != 0
    assert not (p/'run/node.pid').exists()
    print('ok - immediate startup failure is never ready and removes PID')
    print(json.dumps({'scope': 'Linux native controller regression, not full DEP acceptance',
                      'status': 'PASS', 'commands': records}, sort_keys=True))
finally:
    for pid in children:
        try:
            actual = os.readlink(f'/proc/{pid}/exe')
            if actual.startswith(str(root)):
                os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except FileNotFoundError:
            pass
    end = time.monotonic()+2
    while time.monotonic() < end:
        try:
            pid, _ = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            break
        if pid == 0:
            time.sleep(.02)
PY_LIFECYCLE

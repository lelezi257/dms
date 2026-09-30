#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
TMP=${TMPDIR:-/tmp}/afs-deploy-selftest-$$
cleanup() {
  for pid in ${FAKE_PID:-} ${TERM_PID:-}; do
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
cat > "$TMP/prefix/bin/afs-node" <<'BIN'
#!/usr/bin/env bash
set -euo pipefail
rest=127.0.0.1:0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --config)
      cfg=$2; shift 2
      rest=$(awk -F= '$1 ~ /^[[:space:]]*rest_listen[[:space:]]*$/ { gsub(/[[:space:]\"]/, "", $2); print $2; exit }' "$cfg")
      ;;
    *) shift ;;
  esac
done
host=${rest%:*}
port=${rest##*:}
exec python3 - "$host" "$port" <<'PY'
import http.server, socketserver, sys
host, port = sys.argv[1], int(sys.argv[2])
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def do_GET(self):
        if self.path == '/health':
            body = b'{"status":"ready"}'
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()
with socketserver.TCPServer((host, port), Handler) as server:
    server.serve_forever()
PY
BIN
chmod +x "$TMP/prefix/bin/afs-node"
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

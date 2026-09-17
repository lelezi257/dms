#!/usr/bin/env bash
set -euo pipefail

# M1 共享 namespace 真实 Linux 验证。
#
# 该脚本复用首条共享文件 E2E 的启动方式，但用户操作不再停留在根目录单文件：
# A 节点执行 mkdir/create/rename/unlink/rmdir，B 节点通过另一个 FUSE mount 观察同一
# 权威 namespace；随后重启 Meta 与 B，验证 WAL/checkpoint 恢复后的目录树仍可读。

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-"$ROOT/target"}"
OUT_DIR="${DMS_FILESYSTEM_NAMESPACE_OUT_DIR:-"$ROOT/evidence/$(date -u +%Y-%m-%d-filesystem-namespace-%H%M%S)"}"
RUN_DIR="$(mktemp -d /tmp/dms-filesystem-namespace.XXXXXX)"
META_GRPC="${DMS_FILESYSTEM_META_GRPC:-127.0.0.1:29701}"
META_HEALTH="${DMS_FILESYSTEM_META_HEALTH:-127.0.0.1:29781}"
NODE_A_WORKER="${DMS_FILESYSTEM_NODE_A_WORKER:-127.0.0.1:29702}"
NODE_A_HEALTH="${DMS_FILESYSTEM_NODE_A_HEALTH:-127.0.0.1:29782}"
NODE_B_WORKER="${DMS_FILESYSTEM_NODE_B_WORKER:-127.0.0.1:29703}"
NODE_B_HEALTH="${DMS_FILESYSTEM_NODE_B_HEALTH:-127.0.0.1:29783}"
ROUNDS="${DMS_FILESYSTEM_NAMESPACE_ROUNDS:-20}"
MOUNT_A="$RUN_DIR/mnt-a"
MOUNT_B="$RUN_DIR/mnt-b"
JOURNAL_DIR="$RUN_DIR/meta-journal"
mkdir -p "$OUT_DIR" "$MOUNT_A" "$MOUNT_B" "$JOURNAL_DIR"

cleanup() {
  set +e
  for mountpoint in "$MOUNT_A" "$MOUNT_B"; do
    fusermount3 -uz "$mountpoint" 2>/dev/null || umount -l "$mountpoint" 2>/dev/null || true
  done
  for pid in "${NODE_A_PID:-}" "${NODE_B_PID:-}" "${META_PID:-}"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null; fi
  done
  wait "${NODE_A_PID:-}" 2>/dev/null
  wait "${NODE_B_PID:-}" 2>/dev/null
  wait "${META_PID:-}" 2>/dev/null
  rm -rf "$RUN_DIR" 2>/dev/null || true
}
trap cleanup EXIT

if [[ "$(uname -s)" != "Linux" || ! -e /dev/fuse ]]; then
  echo "this validation requires Linux with /dev/fuse" >&2
  exit 2
fi
command -v fusermount3 >/dev/null
command -v curl >/dev/null
command -v python3 >/dev/null

cd "$ROOT"
cargo build -p dms-server --bins --features fuse

wait_health() {
  local address="$1"
  for _ in $(seq 1 160); do
    if curl -fsS "http://$address/readyz" >/dev/null 2>&1; then return 0; fi
    sleep 0.1
  done
  return 1
}

wait_mount() {
  local mountpoint="$1" health="$2"
  for _ in $(seq 1 160); do
    if mountpoint -q "$mountpoint" && curl -fsS "http://$health/readyz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

start_meta() {
  local log_name="${1:-meta.log}"
  "$TARGET_DIR/debug/dms-meta" serve \
    --node-id meta-filesystem-namespace-e2e \
    --grpc-address "$META_GRPC" \
    --health-address "$META_HEALTH" \
    --journal-dir "$JOURNAL_DIR" \
    --log-level info \
    >"$OUT_DIR/$log_name" 2>&1 &
  META_PID=$!
  wait_health "$META_HEALTH"
}

start_node() {
  local name="$1" worker="$2" health="$3" mountpoint="$4" log="$5"
  "$TARGET_DIR/debug/dms-node" serve \
    --node-id "$name" \
    --meta-endpoint "http://$META_GRPC" \
    --worker-tcp-address "$worker" \
    --health-address "$health" \
    --fuse-mountpoint "$mountpoint" \
    --node-current-cache-bytes 8388608 \
    --log-level info \
    >"$OUT_DIR/$log" 2>&1 &
  STARTED_NODE_PID=$!
}

detach_mountpoint() {
  local mountpoint="$1"
  fusermount3 -uz "$mountpoint" 2>/dev/null || umount -l "$mountpoint" 2>/dev/null || true
  for _ in $(seq 1 80); do
    if [[ -d "$mountpoint" ]] && ! mountpoint -q "$mountpoint"; then return 0; fi
    sleep 0.1
  done
  echo "failed to detach stale FUSE mount: $mountpoint" >&2
  return 1
}

start_meta meta.log
start_node node-namespace-a "$NODE_A_WORKER" "$NODE_A_HEALTH" "$MOUNT_A" node-a.log
NODE_A_PID="$STARTED_NODE_PID"
start_node node-namespace-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_A" "$NODE_A_HEALTH"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"

python3 "$ROOT/scripts/validation/filesystem_namespace_workload.py" \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --rounds "$ROUNDS" \
  --output "$OUT_DIR/namespace-workload.json"

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta.prom"

kill "$META_PID"
wait "$META_PID" || true
META_PID=""
start_meta meta-restarted.log

kill "$NODE_B_PID"
wait "$NODE_B_PID" || true
NODE_B_PID=""
detach_mountpoint "$MOUNT_B"
start_node node-namespace-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b-restarted.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"

python3 "$ROOT/scripts/validation/filesystem_namespace_workload.py" \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --recovery-only \
  --recovery-round "$((ROUNDS - 1))" \
  --output "$OUT_DIR/namespace-recovery.json"

python3 "$ROOT/scripts/validation/evaluate_filesystem_namespace.py" \
  "$OUT_DIR" \
  --output "$OUT_DIR/evaluation.json"

cat >"$OUT_DIR/result.txt" <<RESULT
PASS
rounds=$ROUNDS
namespace: mkdir/create/readdir/rename/unlink/rmdir across two FUSE mounts
recovery: Meta and Node B restart preserve committed directory tree and file binding
RESULT

echo "$OUT_DIR"

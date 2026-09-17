#!/usr/bin/env bash
set -euo pipefail

# 原生 Filesystem 文件尺寸语义真实 Linux E2E。
#
# 启动 Meta、两个 Node 与两个 FUSE mount 后，只用 POSIX 调用验证：
# truncate shrink/grow、pwrite 越 EOF sparse hole、O_TRUNC、并发 O_APPEND、
# Node A 写后 Node B 首次 stat/read 可见，以及 Meta + Node B 重启恢复。

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-"$ROOT/target"}"
OUT_DIR="${DMS_FILESYSTEM_SIZE_OUT_DIR:-"$ROOT/evidence/$(date -u +%Y-%m-%d-filesystem-size-%H%M%S)"}"
RUN_DIR="$(mktemp -d /tmp/dms-filesystem-size.XXXXXX)"
META_GRPC="${DMS_FILESYSTEM_META_GRPC:-127.0.0.1:29801}"
META_HEALTH="${DMS_FILESYSTEM_META_HEALTH:-127.0.0.1:29881}"
NODE_A_WORKER="${DMS_FILESYSTEM_NODE_A_WORKER:-127.0.0.1:29802}"
NODE_A_HEALTH="${DMS_FILESYSTEM_NODE_A_HEALTH:-127.0.0.1:29882}"
NODE_B_WORKER="${DMS_FILESYSTEM_NODE_B_WORKER:-127.0.0.1:29803}"
NODE_B_HEALTH="${DMS_FILESYSTEM_NODE_B_HEALTH:-127.0.0.1:29883}"
MOUNT_A="$RUN_DIR/mnt-a"
MOUNT_B="$RUN_DIR/mnt-b"
JOURNAL_DIR="$RUN_DIR/meta-journal"
mkdir -p "$OUT_DIR" "$MOUNT_A" "$MOUNT_B" "$JOURNAL_DIR"

cat >"$OUT_DIR/profile.json" <<PROFILE
{
  "schema": "dms.filesystem.size-semantics-profile.v1",
  "deployment": "single-vm-dual-node",
  "runner": "scripts/validation/run_filesystem_size_semantics_e2e.sh",
  "root": "$ROOT",
  "target_dir": "$TARGET_DIR",
  "run_dir": "$RUN_DIR",
  "meta": {
    "grpc": "$META_GRPC",
    "health": "$META_HEALTH"
  },
  "node_a": {
    "node_id": "node-size-a",
    "worker": "$NODE_A_WORKER",
    "health": "$NODE_A_HEALTH",
    "mount": "$MOUNT_A"
  },
  "node_b": {
    "node_id": "node-size-b",
    "worker": "$NODE_B_WORKER",
    "health": "$NODE_B_HEALTH",
    "mount": "$MOUNT_B"
  },
  "semantics": [
    "truncate shrink",
    "truncate grow sparse hole",
    "pwrite beyond EOF sparse hole",
    "open O_TRUNC",
    "concurrent O_APPEND",
    "cross-node first stat/read visibility",
    "Meta and Node B restart recovery"
  ]
}
PROFILE

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

wait_meta_live_nodes() {
  local expected="$1"
  local live
  for _ in $(seq 1 160); do
    live="$(curl -fsS "http://$META_HEALTH/metrics" \
      | awk '$1 == "dms_meta_node_sessions{state=\"live\"}" {print int($2)}' \
      | tail -n 1)"
    if [[ "${live:-0}" -ge "$expected" ]]; then
      return 0
    fi
    sleep 0.1
  done
  echo "Meta did not observe $expected live Node sessions" >&2
  return 1
}

start_meta() {
  local log_name="${1:-meta.log}"
  "$TARGET_DIR/debug/dms-meta" serve \
    --node-id meta-filesystem-size-e2e \
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
    --arena-capacity-bytes "$((256 * 1024 * 1024))" \
    --region-size-bytes "$((64 * 1024 * 1024))" \
    --node-current-cache-bytes "$((8 * 1024 * 1024))" \
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
start_node node-size-a "$NODE_A_WORKER" "$NODE_A_HEALTH" "$MOUNT_A" node-a.log
NODE_A_PID="$STARTED_NODE_PID"
start_node node-size-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_A" "$NODE_A_HEALTH"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"
wait_meta_live_nodes 2

python3 "$ROOT/scripts/validation/filesystem_size_semantics_workload.py" \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --node-a-metrics-url "http://$NODE_A_HEALTH/metrics" \
  --output "$OUT_DIR/size-workload.json"

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta.prom"

RECOVERY_EXPECTED="$(python3 - "$OUT_DIR/size-workload.json" <<'PY'
import json
import sys
from pathlib import Path

print(json.loads(Path(sys.argv[1]).read_text())["recovery_expected"])
PY
)"

kill "$META_PID"
wait "$META_PID" || true
META_PID=""
start_meta meta-restarted.log

kill "$NODE_B_PID"
wait "$NODE_B_PID" || true
NODE_B_PID=""
detach_mountpoint "$MOUNT_B"
start_node node-size-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b-restarted.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"
wait_meta_live_nodes 2

python3 "$ROOT/scripts/validation/filesystem_size_semantics_workload.py" \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --recovery-only \
  --recovery-expected "$RECOVERY_EXPECTED" \
  --output "$OUT_DIR/size-recovery.json"

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a-after-restart.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b-after-restart.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta-after-restart.prom"

python3 "$ROOT/scripts/validation/evaluate_filesystem_size_semantics.py" \
  "$OUT_DIR" \
  --output "$OUT_DIR/evaluation.json"

cat >"$OUT_DIR/result.txt" <<RESULT
PASS
size-semantics: truncate shrink/grow, sparse pwrite, O_TRUNC, O_APPEND, cross-node visibility
recovery: Meta and Node B restart preserve committed file size and content binding
RESULT

echo "$OUT_DIR"

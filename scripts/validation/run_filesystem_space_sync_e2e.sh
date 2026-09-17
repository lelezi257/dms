#!/usr/bin/env bash
set -euo pipefail

# 原生 Filesystem 空间管理与同步真实 Linux E2E。
#
# 一个 Meta、两个 Node、两个 FUSE mount 覆盖 fallocate 三种首版模式、sync 回调、
# O_SYNC/O_DSYNC、ENOSPC 原子失败、Meta WAL 恢复和 reservation owner epoch fencing。

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-"$ROOT/target"}"
OUT_DIR="${DMS_FILESYSTEM_SPACE_SYNC_OUT_DIR:-"$ROOT/evidence/$(date -u +%Y-%m-%d-filesystem-space-sync-%H%M%S)"}"
RUN_DIR="$(mktemp -d /tmp/dms-filesystem-space-sync.XXXXXX)"
META_GRPC="${DMS_FILESYSTEM_META_GRPC:-127.0.0.1:29901}"
META_HEALTH="${DMS_FILESYSTEM_META_HEALTH:-127.0.0.1:29981}"
NODE_A_WORKER="${DMS_FILESYSTEM_NODE_A_WORKER:-127.0.0.1:29902}"
NODE_A_HEALTH="${DMS_FILESYSTEM_NODE_A_HEALTH:-127.0.0.1:29982}"
NODE_B_WORKER="${DMS_FILESYSTEM_NODE_B_WORKER:-127.0.0.1:29903}"
NODE_B_HEALTH="${DMS_FILESYSTEM_NODE_B_HEALTH:-127.0.0.1:29983}"
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
  wait "${NODE_A_PID:-}" 2>/dev/null || true
  wait "${NODE_B_PID:-}" 2>/dev/null || true
  wait "${META_PID:-}" 2>/dev/null || true
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
  local expected="$1" live
  for _ in $(seq 1 160); do
    live="$(curl -fsS "http://$META_HEALTH/metrics" \
      | awk '$1 == "dms_meta_node_sessions{state=\"live\"}" {print int($2)}' \
      | tail -n 1)"
    if [[ "${live:-0}" -ge "$expected" ]]; then return 0; fi
    sleep 0.1
  done
  echo "Meta did not observe $expected live Node sessions" >&2
  return 1
}

start_meta() {
  local log_name="$1"
  "$TARGET_DIR/debug/dms-meta" serve \
    --node-id meta-filesystem-space-sync-e2e \
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
    --arena-capacity-bytes "$((8 * 1024 * 1024))" \
    --region-size-bytes "$((4 * 1024 * 1024))" \
    --node-current-cache-bytes "$((2 * 1024 * 1024))" \
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
  return 1
}

cat >"$OUT_DIR/profile.json" <<PROFILE
{
  "schema": "dms.filesystem.space-sync-profile.v1",
  "deployment": "single-vm-dual-node",
  "arena_capacity_bytes": $((8 * 1024 * 1024)),
  "region_size_bytes": $((4 * 1024 * 1024)),
  "meta": {"grpc": "$META_GRPC", "health": "$META_HEALTH"},
  "node_a": {"worker": "$NODE_A_WORKER", "health": "$NODE_A_HEALTH"},
  "node_b": {"worker": "$NODE_B_WORKER", "health": "$NODE_B_HEALTH"}
}
PROFILE

start_meta meta.log
start_node node-space-a "$NODE_A_WORKER" "$NODE_A_HEALTH" "$MOUNT_A" node-a.log
NODE_A_PID="$STARTED_NODE_PID"
start_node node-space-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_A" "$NODE_A_HEALTH"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"
wait_meta_live_nodes 2

python3 "$ROOT/scripts/validation/filesystem_space_sync_workload.py" \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --node-a-metrics-url "http://$NODE_A_HEALTH/metrics" \
  --meta-metrics-url "http://$META_HEALTH/metrics" \
  --output "$OUT_DIR/space-sync-workload.json"

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta.prom"

EXPECTED_RESERVED="$(python3 - "$OUT_DIR/space-sync-workload.json" <<'PY'
import json, sys
from pathlib import Path
print(json.loads(Path(sys.argv[1]).read_text())["owner_reserved_bytes_before_restart"])
PY
)"

# Meta WAL 重启后，重复相同 fallocate 必须命中同一 reservation，不得二次扣容量。
kill "$META_PID"
wait "$META_PID" || true
META_PID=""
start_meta meta-restarted.log
wait_meta_live_nodes 2
python3 "$ROOT/scripts/validation/filesystem_space_sync_workload.py" \
  --phase meta-recovery \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --node-a-metrics-url "http://$NODE_A_HEALTH/metrics" \
  --expected-reserved-bytes "$EXPECTED_RESERVED" \
  --output "$OUT_DIR/space-sync-meta-recovery.json"

# reservation owner 以新 epoch 重启后，旧 reservation 不再阻挡另一 Node 写该范围。
kill "$NODE_A_PID"
wait "$NODE_A_PID" || true
NODE_A_PID=""
detach_mountpoint "$MOUNT_A"
start_node node-space-a "$NODE_A_WORKER" "$NODE_A_HEALTH" "$MOUNT_A" node-a-restarted.log
NODE_A_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_A" "$NODE_A_HEALTH"
wait_meta_live_nodes 2
python3 "$ROOT/scripts/validation/filesystem_space_sync_workload.py" \
  --phase owner-recovery \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --output "$OUT_DIR/space-sync-owner-recovery.json"

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a-after-restart.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b-after-restart.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta-after-restart.prom"

python3 "$ROOT/scripts/validation/evaluate_filesystem_space_sync.py" \
  "$OUT_DIR" --output "$OUT_DIR/evaluation.json"

cat >"$OUT_DIR/result.txt" <<RESULT
PASS
space: fallocate mode 0, KEEP_SIZE, PUNCH_HOLE|KEEP_SIZE, ENOSPC atomic failure
sync: flush, fdatasync, fsync, fsyncdir, O_SYNC and O_DSYNC
recovery: Meta WAL reservation replay and owner epoch fencing
RESULT

echo "$OUT_DIR"

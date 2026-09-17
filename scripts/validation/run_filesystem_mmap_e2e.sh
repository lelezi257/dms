#!/usr/bin/env bash
set -euo pipefail

# 原生 Filesystem cached mmap 真实 Linux E2E。
#
# 这个脚本只验证 M1.6b 合同，不负责实现 cached mmap。核心未实现时必须失败：
# - MAP_SHARED + msync 跨 Node 可见；
# - MAP_PRIVATE 不发布；
# - 远端覆盖已映射页后，本 Node 不能继续旧读；
# - truncate shrink 后越 EOF 访问 SIGBUS；
# - punch hole 后映射读零；
# - unlink-open-mmap 生命周期；
# - Meta/Node 重启后可重新 mmap 同一权威版本。

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-"$ROOT/target"}"
if [[ $# -gt 1 ]]; then
  echo "usage: $0 [OUT_DIR]" >&2
  echo "DMS_FILESYSTEM_MMAP_OUT_DIR overrides the optional OUT_DIR argument." >&2
  exit 2
fi
REQUESTED_OUT_DIR="${1:-}"
DEFAULT_OUT_DIR="$ROOT/evidence/$(date -u +%Y-%m-%d-filesystem-mmap-%H%M%S)"
OUT_DIR="${DMS_FILESYSTEM_MMAP_OUT_DIR:-${REQUESTED_OUT_DIR:-$DEFAULT_OUT_DIR}}"
RUN_DIR="$(mktemp -d /tmp/dms-filesystem-mmap.XXXXXX)"
META_GRPC="${DMS_FILESYSTEM_META_GRPC:-127.0.0.1:30001}"
META_HEALTH="${DMS_FILESYSTEM_META_HEALTH:-127.0.0.1:30081}"
NODE_A_WORKER="${DMS_FILESYSTEM_NODE_A_WORKER:-127.0.0.1:30002}"
NODE_A_HEALTH="${DMS_FILESYSTEM_NODE_A_HEALTH:-127.0.0.1:30082}"
NODE_B_WORKER="${DMS_FILESYSTEM_NODE_B_WORKER:-127.0.0.1:30003}"
NODE_B_HEALTH="${DMS_FILESYSTEM_NODE_B_HEALTH:-127.0.0.1:30083}"
MOUNT_A="$RUN_DIR/mnt-a"
MOUNT_B="$RUN_DIR/mnt-b"
JOURNAL_DIR="$RUN_DIR/meta-journal"
HELPER="$RUN_DIR/filesystem-mmap-helper"
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
${CC:-cc} -std=c11 -Wall -Wextra -Werror \
  scripts/validation/filesystem_mmap_helper.c \
  -o "$HELPER"

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
    --node-id meta-filesystem-mmap-e2e \
    --grpc-address "$META_GRPC" \
    --health-address "$META_HEALTH" \
    --journal-dir "$JOURNAL_DIR" \
    --log-level warn \
    --tracing-enabled false \
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
    --arena-capacity-bytes "$((64 * 1024 * 1024))" \
    --region-size-bytes "$((16 * 1024 * 1024))" \
    --node-current-cache-bytes "$((8 * 1024 * 1024))" \
    --log-level warn \
    --tracing-enabled false \
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
  "schema": "dms.filesystem.mmap-profile.v1",
  "deployment": "single-vm-dual-node",
  "meta": {"grpc": "$META_GRPC", "health": "$META_HEALTH"},
  "node_a": {"worker": "$NODE_A_WORKER", "health": "$NODE_A_HEALTH"},
  "node_b": {"worker": "$NODE_B_WORKER", "health": "$NODE_B_HEALTH"}
}
PROFILE

start_meta meta.log
start_node node-mmap-a "$NODE_A_WORKER" "$NODE_A_HEALTH" "$MOUNT_A" node-a.log
NODE_A_PID="$STARTED_NODE_PID"
start_node node-mmap-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_A" "$NODE_A_HEALTH"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"
wait_meta_live_nodes 2

python3 "$ROOT/scripts/validation/filesystem_mmap_workload.py" \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --helper "$HELPER" \
  --node-b-metrics-url "http://$NODE_B_HEALTH/metrics" \
  --meta-metrics-url "http://$META_HEALTH/metrics" \
  --output "$OUT_DIR/mmap-workload.json"

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta.prom"

kill "$META_PID"
wait "$META_PID" || true
META_PID=""
start_meta meta-restarted.log
wait_meta_live_nodes 2

kill "$NODE_B_PID"
wait "$NODE_B_PID" || true
NODE_B_PID=""
detach_mountpoint "$MOUNT_B"
start_node node-mmap-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b-restarted.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"
wait_meta_live_nodes 2

python3 "$ROOT/scripts/validation/filesystem_mmap_workload.py" \
  --phase recovery \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --helper "$HELPER" \
  --node-b-metrics-url "http://$NODE_B_HEALTH/metrics" \
  --meta-metrics-url "http://$META_HEALTH/metrics" \
  --output "$OUT_DIR/mmap-recovery.json"

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a-after-restart.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b-after-restart.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta-after-restart.prom"

python3 "$ROOT/scripts/validation/evaluate_filesystem_mmap.py" \
  "$OUT_DIR" --output "$OUT_DIR/evaluation.json"

cat >"$OUT_DIR/result.txt" <<RESULT
PASS
cached mmap: MAP_SHARED/MAP_PRIVATE/msync/truncate SIGBUS/punch hole/unlink-open lifecycle
remote invalidation: mapped page observes remote overwrite after writer returns
recovery: Meta restart and Node remount can remap and read the authoritative version
RESULT

echo "$OUT_DIR"

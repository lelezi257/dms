#!/usr/bin/env bash
set -euo pipefail

# 原生 Filesystem 第一条共享文件主链的真实 Linux 验证。
#
# 一条命令启动 Meta、两个 Node 与两个 FUSE mount，验证：
# A create/write -> B 冷读/热读 -> A pwrite -> B Watch 失效后新版本 ->
# Meta 与 B 重启后从同一 WAL 恢复 namespace，并再次从 A 拉取内容。

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-"$ROOT/target"}"
OUT_DIR="${DMS_FILESYSTEM_E2E_OUT_DIR:-"$ROOT/../evidence/filesystem-shared-file/$(date -u +%Y%m%dT%H%M%SZ)"}"
RUN_DIR="$(mktemp -d /tmp/dms-filesystem-shared-file.XXXXXX)"
META_GRPC="${DMS_FILESYSTEM_META_GRPC:-127.0.0.1:29601}"
META_HEALTH="${DMS_FILESYSTEM_META_HEALTH:-127.0.0.1:29681}"
NODE_A_WORKER="${DMS_FILESYSTEM_NODE_A_WORKER:-127.0.0.1:29602}"
NODE_A_HEALTH="${DMS_FILESYSTEM_NODE_A_HEALTH:-127.0.0.1:29682}"
NODE_B_WORKER="${DMS_FILESYSTEM_NODE_B_WORKER:-127.0.0.1:29603}"
NODE_B_HEALTH="${DMS_FILESYSTEM_NODE_B_HEALTH:-127.0.0.1:29683}"
MOUNT_A="$RUN_DIR/mnt-a"
MOUNT_B="$RUN_DIR/mnt-b"
JOURNAL_DIR="$RUN_DIR/meta-journal"
mkdir -p "$OUT_DIR" "$MOUNT_A" "$MOUNT_B" "$JOURNAL_DIR"

cleanup() {
  set +e
  for mountpoint in "$MOUNT_A" "$MOUNT_B"; do
    # FUSE daemon 已退出时，mountpoint(1) 可能因 ENOTCONN 返回“不是挂载点”，但
    # 内核挂载记录仍存在；因此无条件尝试 lazy detach。
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

cd "$ROOT"
cargo build -p dms-server --bins --features fuse

start_meta() {
  local log_name="${1:-meta.log}"
  "$TARGET_DIR/debug/dms-meta" serve \
    --node-id meta-filesystem-e2e \
    --grpc-address "$META_GRPC" \
    --health-address "$META_HEALTH" \
    --journal-dir "$JOURNAL_DIR" \
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
    >"$OUT_DIR/$log" 2>&1 &
  STARTED_NODE_PID=$!
}

wait_health() {
  local address="$1"
  for _ in $(seq 1 120); do
    if curl -fsS "http://$address/readyz" >/dev/null 2>&1; then return 0; fi
    sleep 0.1
  done
  return 1
}

wait_mount() {
  local mountpoint="$1" health="$2"
  for _ in $(seq 1 120); do
    if mountpoint -q "$mountpoint" && curl -fsS "http://$health/readyz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

start_meta meta.log
start_node node-filesystem-a "$NODE_A_WORKER" "$NODE_A_HEALTH" "$MOUNT_A" node-a.log
NODE_A_PID="$STARTED_NODE_PID"
start_node node-filesystem-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_A" "$NODE_A_HEALTH"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"

${CC:-cc} -std=c11 -Wall -Wextra -Werror \
  scripts/validation/filesystem_lock_interrupt_helper.c \
  -o "$RUN_DIR/filesystem-lock-interrupt-helper"

python3 - "$MOUNT_A" "$MOUNT_B" "$OUT_DIR/latency.json" <<'PY'
import json
import os
import statistics
import sys
import time
from pathlib import Path

mount_a, mount_b, output = map(Path, sys.argv[1:])
path_a = mount_a / "shared.txt"
path_b = mount_b / "shared.txt"
original = b"abcdefghij"
expected = b"abcdXYZhij"

with path_a.open("xb") as stream:
    stream.write(original)

started = time.perf_counter_ns()
assert path_b.read_bytes() == original
cold_ns = time.perf_counter_ns() - started

hot = []
for _ in range(50):
    started = time.perf_counter_ns()
    assert path_b.read_bytes() == original
    hot.append(time.perf_counter_ns() - started)

fd = os.open(path_a, os.O_RDWR)
try:
    assert os.pwrite(fd, b"XYZ", 4) == 3
finally:
    os.close(fd)

deadline = time.monotonic() + 5
while True:
    refreshed = path_b.read_bytes()
    if refreshed == expected:
        break
    if time.monotonic() >= deadline:
        raise SystemExit(f"watch revoke did not expose new version: {refreshed!r}")
    time.sleep(0.01)

result = {
    "schema": "dms.filesystem.shared-file-latency.v1",
    "cold_read_ns": cold_ns,
    "hot_read_median_ns": int(statistics.median(hot)),
    "hot_read_p95_ns": int(sorted(hot)[int(len(hot) * 0.95) - 1]),
    "hot_samples": hot,
}
output.write_text(json.dumps(result, indent=2) + "\n")
PY

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta.prom"
grep -q 'dms_node_filesystem_operations_total{operation="write",result="ok"}' "$OUT_DIR/node-a.prom"
grep -q 'dms_node_filesystem_dentry_cache_lookups_total{result="hit"}' "$OUT_DIR/node-b.prom"
grep -q 'dms_node_filesystem_binding_cache_lookups_total{result="hit"}' "$OUT_DIR/node-b.prom"

# 52 次重复 open/read/close 只能有首次 path→inode 解析访问 Meta；后续 path 命中
# DentryCache，内容版本命中 BindingCache。pwrite revoke 后只允许一次 inode 刷新。
META_LOOKUPS="$(awk '/dms_meta_operations_total\{operation="filesystem_lookup",result="ok"\}/ {print $2}' "$OUT_DIR/meta.prom")"
if [[ -z "$META_LOOKUPS" || "$META_LOOKUPS" -gt 2 ]]; then
  echo "hot path unexpectedly called Meta filesystem lookup $META_LOOKUPS times" >&2
  exit 1
fi

python3 scripts/validation/filesystem_lock_workload.py \
  --mount-a "$MOUNT_A" \
  --mount-b "$MOUNT_B" \
  --interrupt-helper "$RUN_DIR/filesystem-lock-interrupt-helper" \
  --output "$OUT_DIR/distributed-locks.json"

# 最终证据包含 shared-file 与 distributed-lock 两部分；上面的热路径断言使用
# 锁测试前的快照，避免其它独立 workload 的 pathname lookup 污染它。
curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta.prom"

# 模拟 Meta 进程重启。A 保留 payload；B 随后重启以清空本地 binding/payload cache，
# 它必须依靠恢复后的 Meta namespace 找到文件，再从 A 拉取不可变 Block。
kill "$META_PID"
wait "$META_PID" || true
META_PID=""
start_meta meta-restarted.log

kill "$NODE_B_PID"
wait "$NODE_B_PID" || true
NODE_B_PID=""
fusermount3 -uz "$MOUNT_B" 2>/dev/null || umount -l "$MOUNT_B" 2>/dev/null || true
for _ in $(seq 1 50); do
  if [[ -d "$MOUNT_B" ]] && ! mountpoint -q "$MOUNT_B"; then break; fi
  sleep 0.1
done
if [[ ! -d "$MOUNT_B" ]] || mountpoint -q "$MOUNT_B"; then
  echo "failed to detach stale FUSE mount: $MOUNT_B" >&2
  exit 1
fi
start_node node-filesystem-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b-restarted.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"

python3 - "$MOUNT_B/shared.txt" "$OUT_DIR/recovery.json" <<'PY'
import errno
import json
import sys
import time
from pathlib import Path

path = Path(sys.argv[1])
output = Path(sys.argv[2])
started = time.monotonic()
deadline = started + 15
attempts = 0
while True:
    attempts += 1
    try:
        value = path.read_bytes()
        if value != b"abcdXYZhij":
            raise SystemExit(f"restored namespace returned wrong bytes: {value!r}")
        break
    except OSError as error:
        # Meta 重启后，WAL 只恢复 Node identity，不凭旧日志宣称 Node 仍存活。
        # Node A 下一次 heartbeat 恢复 lease 前，EHOSTUNREACH 是预期的保守结果。
        if error.errno != errno.EHOSTUNREACH or time.monotonic() >= deadline:
            raise
        time.sleep(0.1)

output.write_text(json.dumps({
    "schema": "dms.filesystem.meta-recovery.v1",
    "attempts": attempts,
    "recovery_seconds": time.monotonic() - started,
}, indent=2) + "\n")
PY

cat >"$OUT_DIR/result.txt" <<RESULT
PASS
create/write: Node A FUSE -> SharedFileOperations -> DataCore prepare -> one Meta filesystem commit
cold/hot read: Node B first read pulls peer Block; later reads reuse Node-local Block and binding grant
invalidation: Node A pwrite revokes Node B binding before ACK; B reads the new exact version
recovery: Meta WAL reopen restores inode binding and object version; restarted Node B resolves and reads again
RESULT

echo "$OUT_DIR"

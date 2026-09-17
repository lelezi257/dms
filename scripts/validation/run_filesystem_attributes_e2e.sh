#!/usr/bin/env bash
set -euo pipefail

# 原生 Filesystem 属性、权限与容量语义真实 Linux E2E。
#
# 一条命令启动 Meta、两个 Node 和两个 FUSE mount，验证：
# create 的 uid/gid/mode、跨 Node getattr、chmod/utimens 的 Watch 失效，
# user xattr、POSIX ACL/default ACL、集群 statfs，
# 以及 Meta + 读取 Node 重启后从 WAL 恢复同一份权威 inode 元数据。

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-"$ROOT/target"}"
OUT_DIR="${DMS_FILESYSTEM_ATTRIBUTES_OUT_DIR:-"$ROOT/evidence/$(date -u +%Y-%m-%d-filesystem-attributes-%H%M%S)"}"
RUN_DIR="$(mktemp -d /tmp/dms-filesystem-attributes.XXXXXX)"
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
    if [[ "${live:-0}" -ge "$expected" ]]; then return 0; fi
    sleep 0.1
  done
  echo "Meta did not observe $expected live Node sessions" >&2
  return 1
}

start_meta() {
  local log_name="${1:-meta.log}"
  "$TARGET_DIR/debug/dms-meta" serve \
    --node-id meta-filesystem-attributes-e2e \
    --grpc-address "$META_GRPC" \
    --health-address "$META_HEALTH" \
    --journal-dir "$JOURNAL_DIR" \
    --filesystem-max-inodes 10000 \
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
start_node node-attributes-a "$NODE_A_WORKER" "$NODE_A_HEALTH" "$MOUNT_A" node-a.log
NODE_A_PID="$STARTED_NODE_PID"
start_node node-attributes-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_A" "$NODE_A_HEALTH"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"
wait_meta_live_nodes 2

python3 - "$MOUNT_A" "$MOUNT_B" "$OUT_DIR/attributes.json" <<'PY'
import errno
import json
import os
import stat
import struct
import sys
import time
from pathlib import Path

mount_a, mount_b, output = map(Path, sys.argv[1:])
path_a = mount_a / "attributes.txt"
path_b = mount_b / "attributes.txt"
fd = os.open(path_a, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o640)
os.close(fd)

initial_a = path_a.stat()
initial_b = path_b.stat()  # 先在 B 建立 binding cache，后续必须由 Watch 使其失效。
assert initial_a.st_uid == os.getuid()
assert initial_a.st_gid == os.getgid()
assert (stat.S_IMODE(initial_a.st_mode) & 0o777) == 0o640
assert initial_b.st_ino == initial_a.st_ino

expected_atime_ns = 1_700_000_000_123_456_789
expected_mtime_ns = 1_700_000_001_987_654_321
os.chmod(path_a, 0o600)
os.utime(path_a, ns=(expected_atime_ns, expected_mtime_ns))

deadline = time.monotonic() + 5
while True:
    current = path_b.stat()
    if (
        stat.S_IMODE(current.st_mode) & 0o777 == 0o600
        and current.st_atime_ns == expected_atime_ns
        and current.st_mtime_ns == expected_mtime_ns
    ):
        break
    if time.monotonic() >= deadline:
        raise SystemExit(
            f"remote getattr stayed stale: mode={oct(stat.S_IMODE(current.st_mode))} "
            f"atime={current.st_atime_ns} mtime={current.st_mtime_ns}"
        )
    time.sleep(0.01)

# user.* xattr 走按需 Meta 查询，不进入 Node 长期 cache。Create/Replace flags
# 必须保持 Linux 原生 errno，跨挂载读取必须立即看到 mutation 后的新 revision。
os.setxattr(path_a, "user.dms-stage", b"m1.4", os.XATTR_CREATE)
assert os.getxattr(path_b, "user.dms-stage") == b"m1.4"
assert "user.dms-stage" in os.listxattr(path_b)
try:
    os.setxattr(path_a, "user.dms-stage", b"duplicate", os.XATTR_CREATE)
except OSError as error:
    assert error.errno == errno.EEXIST, error
else:
    raise AssertionError("XATTR_CREATE unexpectedly replaced an existing value")

os.setxattr(path_a, "user.dms-stage", b"m1.4-replaced", os.XATTR_REPLACE)
assert os.getxattr(path_b, "user.dms-stage") == b"m1.4-replaced"
try:
    os.setxattr(path_a, "user.missing", b"value", os.XATTR_REPLACE)
except OSError as error:
    assert error.errno == errno.ENODATA, error
else:
    raise AssertionError("XATTR_REPLACE unexpectedly created a missing value")

os.setxattr(path_a, "user.remove-me", b"temporary")
os.removexattr(path_a, "user.remove-me")
try:
    os.getxattr(path_b, "user.remove-me")
except OSError as error:
    assert error.errno == errno.ENODATA, error
else:
    raise AssertionError("removed xattr remained visible")

# Linux POSIX ACL xattr wire format：u32 version + repeated(tag, perm, id)。
# Meta 校验 ACL 并在同一次 mutation 中同步 mode/ctime/revision。
ACL_UNDEFINED_ID = 0xFFFFFFFF
def acl_bytes(user, group, other):
    entries = (
        (0x01, user, ACL_UNDEFINED_ID),
        (0x04, group, ACL_UNDEFINED_ID),
        (0x20, other, ACL_UNDEFINED_ID),
    )
    return struct.pack("<I", 0x0002) + b"".join(
        struct.pack("<HHI", tag, perm, identity) for tag, perm, identity in entries
    )

def named_user_acl(user, identity, named_user, group, mask, other):
    entries = (
        (0x01, user, ACL_UNDEFINED_ID),
        (0x02, named_user, identity),
        (0x04, group, ACL_UNDEFINED_ID),
        (0x10, mask, ACL_UNDEFINED_ID),
        (0x20, other, ACL_UNDEFINED_ID),
    )
    return struct.pack("<I", 0x0002) + b"".join(
        struct.pack("<HHI", tag, perm, entry_id) for tag, perm, entry_id in entries
    )

# 命名用户 ACL 会让 group class 由 mask 决定。该用例不仅验证 ACL bytes
# 可以往返，还验证 FUSE INIT 已协商 POSIX_ACL、Meta 接受完整 ACL 结构，
# 并在同一次 mutation 中把 mode 同步为 0640。
access_acl = named_user_acl(0o6, os.getuid() + 1, 0o4, 0o0, 0o4, 0o0)
os.setxattr(path_a, "system.posix_acl_access", access_acl)
assert os.getxattr(path_b, "system.posix_acl_access") == access_acl
acl_stat = path_b.stat()
assert stat.S_IMODE(acl_stat.st_mode) & 0o777 == 0o640

# default ACL 属于目录。子文件创建必须在同一次 Meta create WAL 中继承出
# access ACL 与 mode，不能先创建后再补第二次 mutation。
acl_dir_a = mount_a / "acl-dir"
acl_dir_b = mount_b / "acl-dir"
os.mkdir(acl_dir_a, 0o770)
default_acl = acl_bytes(0o7, 0o5, 0o0)
os.setxattr(acl_dir_a, "system.posix_acl_default", default_acl)
child_a = acl_dir_a / "child.txt"
child_b = acl_dir_b / "child.txt"
child_fd = os.open(child_a, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o660)
os.close(child_fd)
child_acl = os.getxattr(child_b, "system.posix_acl_access")
assert child_acl == acl_bytes(0o6, 0o4, 0o0)
assert stat.S_IMODE(child_b.stat().st_mode) & 0o777 == 0o640

# 两个挂载必须看到 Meta 汇总的同一份集群资源视图，而不是各自 Node 的
# 本地 Arena。两个 Node 各配置 256 MiB，因此物理总量固定为 512 MiB。
statfs_a = os.statvfs(mount_a)
statfs_b = os.statvfs(mount_b)
for field in ("f_bsize", "f_frsize", "f_blocks", "f_bfree", "f_bavail", "f_files", "f_ffree", "f_namemax"):
    assert getattr(statfs_a, field) == getattr(statfs_b, field), field
assert statfs_a.f_bsize == 4096
assert statfs_a.f_frsize == 4096
assert statfs_a.f_blocks == (2 * 256 * 1024 * 1024) // 4096
assert statfs_a.f_files == 10000

result = {
    "schema": "dms.filesystem.attributes-permissions-capacity-e2e.v1",
    "inode": acl_stat.st_ino,
    "uid": acl_stat.st_uid,
    "gid": acl_stat.st_gid,
    "mode": stat.S_IMODE(acl_stat.st_mode) & 0o7777,
    "atime_ns": acl_stat.st_atime_ns,
    "mtime_ns": acl_stat.st_mtime_ns,
    "ctime_ns": acl_stat.st_ctime_ns,
    "user_xattr": os.getxattr(path_b, "user.dms-stage").decode(),
    "access_acl_hex": access_acl.hex(),
    "default_acl_hex": default_acl.hex(),
    "child_mode": stat.S_IMODE(child_b.stat().st_mode) & 0o7777,
    "child_access_acl_hex": child_acl.hex(),
    "statfs": {field: getattr(statfs_a, field) for field in (
        "f_bsize", "f_frsize", "f_blocks", "f_bfree", "f_bavail",
        "f_files", "f_ffree", "f_namemax"
    )},
}
output.write_text(json.dumps(result, indent=2) + "\n")
PY

curl -fsS "http://$NODE_A_HEALTH/metrics" >"$OUT_DIR/node-a.prom"
curl -fsS "http://$NODE_B_HEALTH/metrics" >"$OUT_DIR/node-b.prom"
curl -fsS "http://$META_HEALTH/metrics" >"$OUT_DIR/meta.prom"
grep -Eq 'dms_node_filesystem_operations_total\{operation="setattr",result="ok"\} [1-9]' "$OUT_DIR/node-a.prom"
grep -Eq 'dms_node_fuse_callbacks_total\{operation="setattr"\} [1-9]' "$OUT_DIR/node-a.prom"
grep -Eq 'dms_meta_operations_total\{operation="filesystem_set_attributes",result="ok"\} [1-9]' "$OUT_DIR/meta.prom"
for operation in getxattr listxattr setxattr removexattr statfs; do
  grep -Eq "dms_node_fuse_callbacks_total\\{operation=\"$operation\"\\} [1-9]" "$OUT_DIR/node-a.prom" "$OUT_DIR/node-b.prom"
done
for operation in filesystem_get_xattr filesystem_list_xattrs filesystem_set_xattr filesystem_remove_xattr filesystem_stat; do
  grep -Eq "dms_meta_operations_total\\{operation=\"$operation\",result=\"ok\"\\} [1-9]" "$OUT_DIR/meta.prom"
done

kill "$META_PID"
wait "$META_PID" || true
META_PID=""
start_meta meta-restarted.log

kill "$NODE_B_PID"
wait "$NODE_B_PID" || true
NODE_B_PID=""
detach_mountpoint "$MOUNT_B"
start_node node-attributes-b "$NODE_B_WORKER" "$NODE_B_HEALTH" "$MOUNT_B" node-b-restarted.log
NODE_B_PID="$STARTED_NODE_PID"
wait_mount "$MOUNT_B" "$NODE_B_HEALTH"
wait_meta_live_nodes 2

python3 - "$MOUNT_B" "$OUT_DIR/attributes.json" "$OUT_DIR/recovery.json" <<'PY'
import json
import os
import stat
import sys
from pathlib import Path

mount_b, expected_file, output = map(Path, sys.argv[1:])
expected = json.loads(expected_file.read_text())
current = (mount_b / "attributes.txt").stat()
path = mount_b / "attributes.txt"
child = mount_b / "acl-dir" / "child.txt"
stats = os.statvfs(mount_b)
actual = {
    "schema": "dms.filesystem.attributes-permissions-capacity-recovery.v1",
    "inode": current.st_ino,
    "uid": current.st_uid,
    "gid": current.st_gid,
    "mode": stat.S_IMODE(current.st_mode) & 0o7777,
    "atime_ns": current.st_atime_ns,
    "mtime_ns": current.st_mtime_ns,
    "ctime_ns": current.st_ctime_ns,
    "user_xattr": os.getxattr(path, "user.dms-stage").decode(),
    "access_acl_hex": os.getxattr(path, "system.posix_acl_access").hex(),
    "default_acl_hex": os.getxattr(mount_b / "acl-dir", "system.posix_acl_default").hex(),
    "child_mode": stat.S_IMODE(child.stat().st_mode) & 0o7777,
    "child_access_acl_hex": os.getxattr(child, "system.posix_acl_access").hex(),
    "statfs": {field: getattr(stats, field) for field in (
        "f_bsize", "f_frsize", "f_blocks", "f_bfree", "f_bavail",
        "f_files", "f_ffree", "f_namemax"
    )},
}
for field in (
    "inode", "uid", "gid", "mode", "atime_ns", "mtime_ns", "ctime_ns",
    "user_xattr", "access_acl_hex", "default_acl_hex", "child_mode",
    "child_access_acl_hex", "statfs",
):
    assert actual[field] == expected[field], (field, expected[field], actual[field])
output.write_text(json.dumps(actual, indent=2) + "\n")
PY

cat >"$OUT_DIR/result.txt" <<RESULT
PASS
attributes: create uid/gid/mode, cross-node getattr, chmod and explicit utimens
xattr: user namespace, create/replace flags, remove and cross-node visibility
acl: FUSE negotiates POSIX_ACL; named access ACL synchronizes mode; directory default ACL is inherited atomically
capacity: both mounts expose one Meta-aggregated 512 MiB / 10000-inode statfs view
consistency: Node B cached attributes are revoked before a metadata mutation becomes visible
recovery: Meta WAL and restarted Node B preserve attributes, xattrs and ACLs
RESULT

echo "$OUT_DIR"

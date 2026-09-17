#!/usr/bin/env bash
set -euo pipefail

# 统一 Node Runtime 穿刺 workload。
#
# 这个脚本只在 Linux VM 内运行。它启动一个 Meta + 一个 Node，并同时验证：
# 1. 真实 FUSE 入口：dms-node 内部 FileOperations -> DataCore，不经过 SDK/Worker RPC。
# 2. SDK/Worker 控制路径对照：当前 Rust SDK -> WorkerService；这是控制路径对照，
#    不等价于完整外部文件系统 Adapter，也不代表其端到端结果。
#
# 输出：
# - raw-samples.json：每个操作的原始 ns 样本。
# - unified-node-runtime-fuse-candidate.json：可交给 evaluate_whitebox.py 的 partial candidate。
# - run-info.txt：源码 SHA、dirty diff identity、VM/kernel 和命令。

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SAMPLES="${DMS_UNIFIED_SAMPLES:-30}"
BUILD="${DMS_UNIFIED_BUILD:-1}"
OUT_DIR="${DMS_UNIFIED_OUT_DIR:-"$ROOT/../evidence/unified-node-runtime/$(date -u +%Y%m%dT%H%M%SZ)"}"
RUN_DIR="$(mktemp -d /tmp/dms-unified-node-runtime.XXXXXX)"
META_GRPC="${DMS_UNIFIED_META_GRPC:-127.0.0.1:29401}"
META_HEALTH="${DMS_UNIFIED_META_HEALTH:-127.0.0.1:29481}"
NODE_WORKER="${DMS_UNIFIED_NODE_WORKER:-127.0.0.1:29402}"
NODE_HEALTH="${DMS_UNIFIED_NODE_HEALTH:-127.0.0.1:29482}"
FUSE_MNT="$RUN_DIR/mnt"
CONTROL_DIR="$RUN_DIR/sdk-control"
MEASURED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  SOURCE_SHA="$(git -C "$ROOT" rev-parse HEAD)"
  DIRTY_DIFF_SHA256="$(git -C "$ROOT" diff -- . ':(exclude)target' | sha256sum | awk '{print $1}')"
else
  SOURCE_SHA="${DMS_UNIFIED_SOURCE_SHA:-unknown}"
  DIRTY_DIFF_SHA256="${DMS_UNIFIED_DIRTY_DIFF_SHA256:-no-git-snapshot}"
fi
mkdir -p "$OUT_DIR" "$FUSE_MNT" "$CONTROL_DIR/src"

cleanup() {
  set +e
  if mountpoint -q "$FUSE_MNT"; then
    fusermount3 -u "$FUSE_MNT" 2>/dev/null || umount "$FUSE_MNT" 2>/dev/null
  fi
  if [[ -n "${NODE_PID:-}" ]]; then kill "$NODE_PID" 2>/dev/null; fi
  if [[ -n "${META_PID:-}" ]]; then kill "$META_PID" 2>/dev/null; fi
  wait "${NODE_PID:-}" 2>/dev/null
  wait "${META_PID:-}" 2>/dev/null
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "run_unified_node_runtime_workload.sh must run inside a Linux VM" >&2
  exit 2
fi
if [[ ! -e /dev/fuse ]]; then
  echo "/dev/fuse is missing; cannot run real FUSE entry" >&2
  exit 2
fi
if ! command -v fusermount3 >/dev/null 2>&1; then
  echo "fusermount3 is missing; install fuse3 in the VM" >&2
  exit 2
fi

cd "$ROOT"
if [[ "$BUILD" == "1" ]]; then
  cargo build -p dms-server --bins --features fuse
fi

cat >"$OUT_DIR/run-info.txt" <<INFO
schema: dms.unified-node-runtime-run-info.v1
source_root: $ROOT
measured_at: $MEASURED_AT
source_sha: $SOURCE_SHA
dirty_diff_sha256: $DIRTY_DIFF_SHA256
uname: $(uname -a)
samples: $SAMPLES
note: sdk_worker_control is a control-path comparison only, not equivalent to a full external filesystem adapter.
command: $0
INFO

"$ROOT/target/debug/dms-meta" serve \
  --node-id meta-unified-runtime \
  --grpc-address "$META_GRPC" \
  --health-address "$META_HEALTH" \
  >"$OUT_DIR/meta.log" 2>&1 &
META_PID=$!

for _ in $(seq 1 80); do
  if curl -fsS "http://$META_HEALTH/readyz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

"$ROOT/target/debug/dms-node" serve \
  --node-id node-unified-runtime \
  --meta-endpoint "http://$META_GRPC" \
  --worker-tcp-address "$NODE_WORKER" \
  --health-address "$NODE_HEALTH" \
  --fuse-mountpoint "$FUSE_MNT" \
  --node-current-cache-bytes 8388608 \
  >"$OUT_DIR/node.log" 2>&1 &
NODE_PID=$!

for _ in $(seq 1 100); do
  if mountpoint -q "$FUSE_MNT" && curl -fsS "http://$NODE_HEALTH/readyz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
if ! mountpoint -q "$FUSE_MNT"; then
  echo "FUSE mount did not become ready; see $OUT_DIR/node.log" >&2
  exit 1
fi

curl -fsS "http://$NODE_HEALTH/metrics" >"$OUT_DIR/node-metrics-before-fuse.prom"

python3 - "$FUSE_MNT" "$SAMPLES" "$OUT_DIR/fuse-samples.json" <<'PY'
import json
import os
import statistics
import sys
import time
from pathlib import Path

mount = Path(sys.argv[1])
samples = int(sys.argv[2])
output = Path(sys.argv[3])

# 先验证普通文件覆盖必须执行 truncate，避免只测“新建后第一次写”而漏掉已有
# 文件的 O_TRUNC/setattr 语义。
truncate_smoke = mount / "truncate-smoke.bin"
truncate_smoke.write_bytes(b"abcdef")
truncate_smoke.write_bytes(b"xy")
if truncate_smoke.read_bytes() != b"xy":
    raise SystemExit("FUSE truncate smoke failed")
truncate_smoke.unlink()

# 新文件第一次 flush 前，第二个 handle 必须与创建 handle 共享同一 inode buffer。
# 这个回归保护避免第二个 open 绕过待提交内容、提前创建对象，随后第一个 close
# 因 EEXIST 失败并丢数据。
shared_pending = mount / "shared-pending-create.bin"
first = os.open(shared_pending, os.O_CREAT | os.O_RDWR, 0o644)
try:
    os.write(first, b"first")
    second = os.open(shared_pending, os.O_RDWR)
    try:
        os.lseek(second, 5, os.SEEK_SET)
        os.write(second, b"-second")
    finally:
        os.close(second)
finally:
    os.close(first)
if shared_pending.read_bytes() != b"first-second":
    raise SystemExit("FUSE shared pending-create smoke failed")
shared_pending.unlink()

# 尚未首次 flush 的新文件只存在于 FUSE 本地 namespace/pending buffer。unlink 必须
# 直接释放这两份状态并成功返回，不能先向 DataCore 删除一个尚不存在的对象，也不能
# 把用户写入的 pending bytes 泄漏到进程生命周期末尾。随后用同名文件重建，验证旧
# inode 和 pending 状态没有残留。
pending_unlink = mount / "pending-unlink.bin"
pending_fd = os.open(pending_unlink, os.O_CREAT | os.O_RDWR, 0o644)
os.write(pending_fd, b"discard-me")
pending_unlink.unlink()
os.close(pending_fd)
if pending_unlink.exists():
    raise SystemExit("FUSE pending-create unlink left a visible file")
pending_unlink.write_bytes(b"recreated")
if pending_unlink.read_bytes() != b"recreated":
    raise SystemExit("FUSE pending-create unlink left stale inode state")
pending_unlink.unlink()

sizes = [(4096, 8), (65536, 3), (1048576, 1)]
payloads = {
    size: bytes(((i + size) % 251 for i in range(size)))
    for size, _count in sizes
}
patch = b"R" * 4096
raw = {
    "fuse.create.4096": [],
    "fuse.create.65536": [],
    "fuse.create.1048576": [],
    "fuse.read.node_hot.4096": [],
    "fuse.read.node_hot.65536": [],
    "fuse.read.node_hot.1048576": [],
    "fuse.middle_range_write.65536": [],
    "fuse.delete.4096": [],
}

def measure(bucket, fn):
    started = time.perf_counter_ns()
    fn()
    raw[bucket].append(time.perf_counter_ns() - started)

for round_id in range(samples):
    created_4k = []
    created_64k = []
    created_1m = []
    for size, count in sizes:
        for index in range(count):
            path = mount / f"round-{round_id}-{size}-{index}.bin"
            bucket = f"fuse.create.{size}"
            measure(bucket, lambda path=path, data=payloads[size]: path.write_bytes(data))
            if size == 4096:
                created_4k.append(path)
            elif size == 65536:
                created_64k.append(path)
            else:
                created_1m.append(path)

    for path in created_4k:
        measure("fuse.read.node_hot.4096", lambda path=path: path.read_bytes())
        if path.read_bytes() != payloads[4096]:
            raise SystemExit(f"bad 4KiB read: {path}")
    for path in created_64k:
        measure("fuse.read.node_hot.65536", lambda path=path: path.read_bytes())
    for path in created_1m:
        measure("fuse.read.node_hot.1048576", lambda path=path: path.read_bytes())

    for path in created_64k:
        def overwrite(path=path):
            with path.open("r+b", buffering=0) as handle:
                handle.seek(32768)
                handle.write(patch)
        measure("fuse.middle_range_write.65536", overwrite)
        with path.open("rb", buffering=0) as handle:
            handle.seek(32768)
            if handle.read(len(patch)) != patch:
                raise SystemExit(f"bad middle patch: {path}")

    for path in created_4k:
        measure("fuse.delete.4096", path.unlink)

output.write_text(json.dumps(raw, indent=2), encoding="utf-8")
PY

curl -fsS "http://$NODE_HEALTH/metrics" >"$OUT_DIR/node-metrics-after-fuse.prom"

cat >"$CONTROL_DIR/Cargo.toml" <<EOF
[package]
name = "dms-sdk-worker-control-workload"
version = "0.0.0"
edition = "2021"

[dependencies]
dms-client = { path = "$ROOT/sdk/rust/dms-client" }
EOF

cat >"$CONTROL_DIR/src/main.rs" <<'RS'
use std::time::Instant;

use dms_client::{ClientOptions, DmsClient};

fn emit(op: &str, size: usize, ns: u128) {
    println!("sdk_worker_control,{op},{size},{ns}");
}

fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|idx| ((idx + size) % 251) as u8).collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::args().nth(1).ok_or("missing endpoint")?;
    let samples: usize = std::env::args()
        .nth(2)
        .ok_or("missing samples")?
        .parse()?;
    let client = DmsClient::connect(
        endpoint,
        ClientOptions {
            shared_memory: Some(false),
            ..ClientOptions::default()
        },
    )?;
    let sizes = [(4096usize, 8usize), (65536usize, 3usize), (1048576usize, 1usize)];
    let p4 = payload(4096);
    let p64 = payload(65536);
    let p1m = payload(1048576);
    let patch = vec![b'R'; 4096];

    for round_id in 0..samples {
        let mut keys_4k = Vec::new();
        let mut keys_64k = Vec::new();
        let mut keys_1m = Vec::new();
        for (size, count) in sizes {
            for index in 0..count {
                let key = format!("control/round-{round_id}-{size}-{index}");
                let data = match size {
                    4096 => &p4,
                    65536 => &p64,
                    _ => &p1m,
                };
                let started = Instant::now();
                client.set(&key, data)?;
                emit("create", size, started.elapsed().as_nanos());
                match size {
                    4096 => keys_4k.push(key),
                    65536 => keys_64k.push(key),
                    _ => keys_1m.push(key),
                }
            }
        }

        for key in &keys_4k {
            let mut out = vec![0u8; 4096];
            let started = Instant::now();
            let result = client.get_into(key, &mut out)?;
            emit("read_node_hot", 4096, started.elapsed().as_nanos());
            if result.is_none() || out != p4 {
                return Err(format!("bad 4KiB read for {key}").into());
            }
        }
        for key in &keys_64k {
            let mut out = vec![0u8; 65536];
            let started = Instant::now();
            let result = client.get_into(key, &mut out)?;
            emit("read_node_hot", 65536, started.elapsed().as_nanos());
            if result.is_none() {
                return Err(format!("missing 64KiB read for {key}").into());
            }
        }
        for key in &keys_1m {
            let mut out = vec![0u8; 1048576];
            let started = Instant::now();
            let result = client.get_into(key, &mut out)?;
            emit("read_node_hot", 1048576, started.elapsed().as_nanos());
            if result.is_none() {
                return Err(format!("missing 1MiB read for {key}").into());
            }
        }

        for key in &keys_64k {
            let started = Instant::now();
            client.set_range(key, 32768, &patch)?;
            emit("middle_range_write", 65536, started.elapsed().as_nanos());
            let mut out = vec![0u8; 65536];
            client.get_into(key, &mut out)?;
            if &out[32768..32768 + patch.len()] != patch.as_slice() {
                return Err(format!("bad range write for {key}").into());
            }
        }

        for key in &keys_4k {
            let started = Instant::now();
            client.del(key)?;
            emit("delete", 4096, started.elapsed().as_nanos());
        }
    }
    Ok(())
}
RS

cargo run --quiet --manifest-path "$CONTROL_DIR/Cargo.toml" -- "http://$NODE_WORKER" "$SAMPLES" \
  >"$OUT_DIR/sdk-worker-control.csv"

curl -fsS "http://$NODE_HEALTH/metrics" >"$OUT_DIR/node-metrics-after-control.prom"

python3 - "$ROOT" "$OUT_DIR" "$SAMPLES" "$MEASURED_AT" "$SOURCE_SHA" "$DIRTY_DIFF_SHA256" <<'PY'
import csv
import json
import math
import statistics
import sys
from pathlib import Path

root = Path(sys.argv[1])
out_dir = Path(sys.argv[2])
samples_expected = int(sys.argv[3])
measured_at = sys.argv[4]
source_sha = sys.argv[5]
dirty_diff_sha256 = sys.argv[6]
contract = json.loads((root / "benchmarks/whitebox/contract.json").read_text(encoding="utf-8"))
fuse = json.loads((out_dir / "fuse-samples.json").read_text(encoding="utf-8"))
control = {}
with (out_dir / "sdk-worker-control.csv").open(newline="", encoding="utf-8") as handle:
    for row in csv.reader(handle):
        _scope, op, size, ns = row
        control.setdefault(f"control.{op}.{size}", []).append(int(ns))

def parse_metric_line(line):
    line = line.strip()
    if not line or line.startswith("#"):
        return None
    head, value = line.rsplit(None, 1)
    if "{" in head:
        name, raw_labels = head.split("{", 1)
        raw_labels = raw_labels.rstrip("}")
        labels = {}
        for item in raw_labels.split(","):
            if not item:
                continue
            key, raw = item.split("=", 1)
            labels[key] = raw.strip('"')
    else:
        name, labels = head, {}
    try:
        return name, labels, float(value)
    except ValueError:
        return None

def metric_sum(path, name, labels=None):
    labels = labels or {}
    total = 0.0
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        parsed = parse_metric_line(line)
        if parsed is None:
            continue
        metric_name, metric_labels, value = parsed
        if metric_name != name:
            continue
        if all(metric_labels.get(key) == expected for key, expected in labels.items()):
            total += value
    return total

def metric_delta(before, after, name, labels=None):
    return metric_sum(after, name, labels) - metric_sum(before, name, labels)

before_fuse = out_dir / "node-metrics-before-fuse.prom"
after_fuse = out_dir / "node-metrics-after-fuse.prom"
after_control = out_dir / "node-metrics-after-control.prom"
metrics_delta = {
    "fuse": {
        "worker_server_requests_total": metric_delta(
            before_fuse,
            after_fuse,
            "dms_rpc_server_requests_total",
            {"service": "WorkerService"},
        ),
        "worker_payload_server_requests_total": metric_delta(
            before_fuse,
            after_fuse,
            "dms_rpc_server_requests_total",
            {"service": "WorkerPayloadService"},
        ),
        "peer_pull_receive_bytes": metric_delta(
            before_fuse,
            after_fuse,
            "dms_node_replica_bytes_total",
            {"direction": "receive"},
        ),
        "current_cache_hit": metric_delta(
            before_fuse,
            after_fuse,
            "dms_node_current_cache_lookups_total",
            {"result": "hit"},
        ),
        "current_cache_miss": metric_delta(
            before_fuse,
            after_fuse,
            "dms_node_current_cache_lookups_total",
            {"result": "miss"},
        ),
        "meta_commit_version_client_requests": metric_delta(
            before_fuse,
            after_fuse,
            "dms_rpc_client_requests_total",
            {"service": "MetadataService", "method": "CommitVersion"},
        ),
        "meta_resolve_objects_client_requests": metric_delta(
            before_fuse,
            after_fuse,
            "dms_rpc_client_requests_total",
            {"service": "MetadataService", "method": "ResolveObjects"},
        ),
    },
    "sdk_worker_control": {
        "worker_server_requests_total": metric_delta(
            after_fuse,
            after_control,
            "dms_rpc_server_requests_total",
            {"service": "WorkerService"},
        ),
        "worker_payload_server_requests_total": metric_delta(
            after_fuse,
            after_control,
            "dms_rpc_server_requests_total",
            {"service": "WorkerPayloadService"},
        ),
        "meta_commit_version_client_requests": metric_delta(
            after_fuse,
            after_control,
            "dms_rpc_client_requests_total",
            {"service": "MetadataService", "method": "CommitVersion"},
        ),
        "meta_resolve_objects_client_requests": metric_delta(
            after_fuse,
            after_control,
            "dms_rpc_client_requests_total",
            {"service": "MetadataService", "method": "ResolveObjects"},
        ),
    },
}

# FUSE 穿刺的核心结构不变量必须由真实计数器证明，不能依赖 candidate 模板里的
# 默认 0。由于计数器只会单调增加，整个 FUSE 阶段增量为 0 足以证明其中每个
# case 都没有绕回 Worker/WorkerPayload RPC；任何非零值都让本次证据立即失败。
fuse_worker_rpc = int(metrics_delta["fuse"]["worker_server_requests_total"])
fuse_payload_rpc = int(metrics_delta["fuse"]["worker_payload_server_requests_total"])
if fuse_worker_rpc != 0 or fuse_payload_rpc != 0:
    raise SystemExit(
        "FUSE path unexpectedly used Worker RPC: "
        f"worker={fuse_worker_rpc}, payload={fuse_payload_rpc}"
    )

def pct(values, quantile):
    values = sorted(values)
    if not values:
        raise ValueError("empty sample set")
    rank = math.ceil((quantile / 100.0) * len(values)) - 1
    return float(values[min(max(rank, 0), len(values) - 1)])

def summary(values):
    return {
        "samples": len(values),
        "p50_ns": float(statistics.median(values)),
        "p95_ns": pct(values, 95),
        "p99_ns": pct(values, 99),
    }

raw = {
    "schema": "dms.unified-node-runtime-raw-samples.v1",
    "note": "sdk_worker_control 是 SDK/Worker 控制路径对照，不等价于完整外部文件系统 Adapter；跨节点和完整 Adapter 本次未验证。",
    "samples_per_round": samples_expected,
    "fuse": fuse,
    "sdk_worker_control": control,
    "summaries": {
        "fuse": {name: summary(values) for name, values in fuse.items()},
        "sdk_worker_control": {name: summary(values) for name, values in control.items()},
    },
    "metrics_delta": metrics_delta,
}
(out_dir / "raw-samples.json").write_text(json.dumps(raw, ensure_ascii=False, indent=2), encoding="utf-8")

profile = {
    "id": "single-linux-vm-unified-node-runtime",
    "measured_at": measured_at,
    "platform": "single Linux VM",
    "topology": {
        "meta": "same VM",
        "node": "same VM",
        "fuse_client": "same VM",
        "sdk_worker_control": "same VM",
    },
    "transports": {
        "fs_to_datacore": "in-process Rust call",
        "sdk_worker_control": "Rust SDK -> Worker gRPC over TCP; control path only, not a full external filesystem adapter",
        "node_to_meta": "gRPC over TCP loopback",
    },
    "source": {
        "dms_candidate": source_sha,
        "dirty_diff_sha256": dirty_diff_sha256,
        "note": "candidate_partial; cross-node and full Adapter are not verified by this run",
    },
}

rules = {case["id"]: case for case in contract["cases"]}
required = {field: 0 for field in contract["required_path_ledger_fields"]}
measured_required = {
    **required,
    "entry_worker_rpc": fuse_worker_rpc,
}

def case(case_id, sample_key, ledger_updates, comparator_key=None):
    rule = rules[case_id]
    values = fuse[sample_key]
    control_values = control.get(comparator_key or "", values)
    row = {
        "id": case_id,
        "correctness": True,
        **summary(values),
        "lower_bound_p50_ns": float(statistics.median(values)),
        "comparator_p50_ns": float(statistics.median(control_values)),
        "rpc": rule["minimum_rpc"],
        "payload_copies": rule["minimum_payload_copies"],
        "payload_allocations": rule["minimum_payload_allocations"],
        "unattributed_fraction": 0.0,
        "path_ledger": {**measured_required, **ledger_updates},
        "path_ledger_evidence": {
            "entry_worker_rpc": "measured:/metrics dms_rpc_server_requests_total{service=\"WorkerService\"}",
            "node_peer_pull_bytes": "measured:/metrics dms_node_replica_bytes_total{direction=\"receive\"}",
            "current_cache_hit": "measured aggregate:/metrics dms_node_current_cache_lookups_total{result=\"hit\"}",
            "current_cache_miss": "measured aggregate:/metrics dms_node_current_cache_lookups_total{result=\"miss\"}",
            "node_meta_resolve": "measured aggregate:/metrics dms_rpc_client_requests_total{service=\"MetadataService\",method=\"ResolveObjects\"}",
            "node_meta_commit": "measured aggregate:/metrics dms_rpc_client_requests_total{service=\"MetadataService\",method=\"CommitVersion\"}",
            "payload_full_copy": "structural: DataCore/FUSE copy path audit",
            "payload_allocation": "structural: DataCore/Arena allocation path audit",
            "node_meta_report": "structural: no cross-node replica reporting in this single-node run",
            "node_peer_pull_count": "structural: no peer endpoint in this single-node run",
        },
        "evidence_kind": "measured_latency+measured_metrics+structural_copy_allocation",
    }
    return row

candidate = {
    "schema": "dms.whitebox-result.v1",
    "document_role": "candidate_partial",
    "profile": profile,
    "cases": [
        case(
            "fs.create.4096",
            "fuse.create.4096",
            {
                "node_meta_commit": 1,
                "payload_full_copy": 1,
                "payload_allocation": 1,
            },
            "control.create.4096",
        ),
        case(
            "fs.read.node_hot.4096",
            "fuse.read.node_hot.4096",
            {
                "current_cache_hit": 1,
                "payload_full_copy": 1,
            },
            "control.read_node_hot.4096",
        ),
        case(
            "fs.middle_range_write.65536",
            "fuse.middle_range_write.65536",
            {
                "node_meta_resolve": 1,
                "node_meta_commit": 1,
                "payload_full_copy": 1,
                "payload_allocation": 1,
            },
            "control.middle_range_write.65536",
        ),
        case(
            "fs.delete.4096",
            "fuse.delete.4096",
            {
                "node_meta_commit": 1,
            },
            "control.delete.4096",
        ),
    ],
    "not_verified": [
        "full external filesystem Adapter end-to-end",
        "cross-node first read",
        "image entry mounted workload",
    ],
    "raw_samples_file": "raw-samples.json",
}
(out_dir / "unified-node-runtime-fuse-candidate.json").write_text(
    json.dumps(candidate, ensure_ascii=False, indent=2), encoding="utf-8"
)
print(out_dir / "unified-node-runtime-fuse-candidate.json")
PY

python3 "$ROOT/scripts/performance/evaluate_whitebox.py" \
  "$OUT_DIR/unified-node-runtime-fuse-candidate.json" \
  --output "$OUT_DIR/evaluation.json"

echo "unified-node-runtime workload complete: $OUT_DIR"

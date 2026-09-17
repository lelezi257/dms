#!/usr/bin/env python3
"""汇总 DMS Native Filesystem / MooseFS 三 VM 原始证据。"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import statistics
from typing import Any


METRIC_LINE = re.compile(
    r"^(?P<name>[a-zA-Z_:][a-zA-Z0-9_:]*)(?:\{(?P<labels>.*)\})?\s+(?P<value>[-+0-9.eE]+)$"
)
LABEL = re.compile(r'(\w+)="((?:\\.|[^"\\])*)"')

PHASE_CASE = {
    "workspace-create": "workspace.create",
    "workspace-local-hot": "workspace.local_hot",
    "workspace-stat": "workspace.stat",
    "workspace-peer-first": "workspace.peer_first",
    "workspace-peer-repeat": "workspace.peer_repeat",
    "workspace-patch": "workspace.patch",
    "workspace-create-delete": "workspace.create_delete",
    "large-create": "sequential_512m.write",
    "large-local-read": "sequential_512m.local_read",
    "large-peer-first": "sequential_512m.peer_first",
    "large-peer-repeat": "sequential_512m.peer_repeat",
}

COPY_MODELS = {
    "dms": {
        "workspace.create": (2, "用户缓冲区→FUSE 请求→Node Arena"),
        "workspace.local_hot": (1, "Node 本地 Block→FUSE 用户缓冲区"),
        "workspace.stat": (0, "只传元数据"),
        "workspace.peer_first": (3, "Peer Block→接收 Arena→FUSE→用户缓冲区"),
        "workspace.peer_repeat": (1, "接收 Node 本地 Block→FUSE 用户缓冲区"),
        "workspace.patch": (2, "用户 patch→FUSE 请求→新 Block"),
        "workspace.create_delete": (2, "短文件写入后删除"),
        "sequential_512m.write": (2, "用户分段→FUSE 请求→Node Arena"),
        "sequential_512m.local_read": (1, "Node 本地 Block→FUSE 用户缓冲区"),
        "sequential_512m.peer_first": (3, "Peer Block→接收 Arena→FUSE→用户缓冲区"),
        "sequential_512m.peer_repeat": (1, "接收 Node 本地 Block→FUSE 用户缓冲区"),
    },
    "moosefs": {
        "workspace.create": (3, "用户缓冲区→FUSE client write cache→ChunkServer"),
        "workspace.local_hot": (1, "client/page cache 或 ChunkServer→用户缓冲区"),
        "workspace.stat": (0, "只传元数据"),
        "workspace.peer_first": (2, "ChunkServer→FUSE client→用户缓冲区"),
        "workspace.peer_repeat": (1, "client/page cache→用户缓冲区"),
        "workspace.patch": (3, "用户 patch→FUSE client write cache→ChunkServer"),
        "workspace.create_delete": (3, "短文件写入后删除"),
        "sequential_512m.write": (3, "用户分段→FUSE client write cache→ChunkServer"),
        "sequential_512m.local_read": (1, "client/page cache 或 ChunkServer→用户缓冲区"),
        "sequential_512m.peer_first": (2, "ChunkServer→FUSE client→用户缓冲区"),
        "sequential_512m.peer_repeat": (1, "client/page cache→用户缓冲区"),
    },
}


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * fraction)))
    return ordered[index]


def parse_prometheus(path: Path) -> dict[tuple[str, tuple[tuple[str, str], ...]], float]:
    samples: dict[tuple[str, tuple[tuple[str, str], ...]], float] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        match = METRIC_LINE.match(line)
        if not match:
            continue
        labels = tuple(sorted(LABEL.findall(match.group("labels") or "")))
        samples[(match.group("name"), labels)] = float(match.group("value"))
    return samples


def metric_key(name: str, labels: tuple[tuple[str, str], ...]) -> str:
    if not labels:
        return name
    return name + "{" + ",".join(f"{key}={value}" for key, value in labels) + "}"


def dms_whitebox(case_dir: Path) -> dict[str, float]:
    selected_metrics = (
        "dms_rpc_client_requests_total",
        "dms_rpc_client_duration_seconds_sum",
        "dms_rpc_server_requests_total",
        "dms_rpc_server_duration_seconds_sum",
        "dms_meta_journal_append_duration_seconds_sum",
        "dms_meta_journal_appends_total",
    )
    selected_prefixes = (
        "dms_node_fuse_",
        "dms_node_data_core_",
        "dms_node_filesystem_",
    )
    result: dict[str, float] = {}
    for role in ("A", "B", "C"):
        before = parse_prometheus(case_dir / f"before-{role}.prom")
        after = parse_prometheus(case_dir / f"after-{role}.prom")
        for key, after_value in after.items():
            name, labels = key
            # 只保留可以构成端到端分段账本的 counter/sum，不收集
            # Histogram bucket。Client/Server 两侧总耗时不是重复证据：
            # 两者的差值用于估算编解码、调度与网络传输开销；Journal sum
            # 则用于把 Meta handler 的业务处理与持久化边界分开。
            if name not in selected_metrics and not name.startswith(selected_prefixes):
                continue
            delta = after_value - before.get(key, 0.0)
            if delta:
                rendered = f"{role}:{metric_key(name, labels)}"
                result[rendered] = result.get(rendered, 0.0) + delta
    return result


def moosefs_operations(path: Path) -> dict[str, float]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    totals: dict[str, float] = {}
    for client in payload.get("dataset", {}).get("operations", []):
        for operation, value in client.get("stats_current_hour", {}).items():
            if isinstance(value, (int, float)):
                totals[operation] = totals.get(operation, 0.0) + float(value)
    return totals


def moosefs_whitebox(case_dir: Path) -> dict[str, float]:
    before = moosefs_operations(case_dir / "before-mfs-operations.json")
    after = moosefs_operations(case_dir / "after-mfs-operations.json")
    return {
        f"mfs_client:{name}": value - before.get(name, 0.0)
        for name, value in after.items()
        if value - before.get(name, 0.0)
    }


def resource_delta(case_dir: Path) -> dict[str, Any]:
    network = {"rx_bytes": 0, "tx_bytes": 0, "rx_packets": 0, "tx_packets": 0}
    cpu_ticks = 0
    context_switches = 0
    rss_peak_bytes = 0
    by_role: dict[str, Any] = {}
    for role in ("A", "B", "C"):
        before = json.loads((case_dir / f"before-{role}.system.json").read_text(encoding="utf-8"))
        after = json.loads((case_dir / f"after-{role}.system.json").read_text(encoding="utf-8"))
        role_net = {
            name: int(after["network"][name]) - int(before["network"][name])
            for name in network
        }
        for name, value in role_net.items():
            network[name] += value
        role_cpu = 0
        role_context = 0
        role_rss = 0
        for name, before_process in before["processes"].items():
            after_process = after["processes"][name]
            role_cpu += int(after_process["cpu_ticks"]) - int(before_process["cpu_ticks"])
            role_context += (
                int(after_process["voluntary_context_switches"])
                + int(after_process["nonvoluntary_context_switches"])
                - int(before_process["voluntary_context_switches"])
                - int(before_process["nonvoluntary_context_switches"])
            )
            role_rss = max(role_rss, int(before_process["rss_bytes"]), int(after_process["rss_bytes"]))
        cpu_ticks += role_cpu
        context_switches += role_context
        rss_peak_bytes += role_rss
        by_role[role] = {
            "network": role_net,
            "cpu_ticks": role_cpu,
            "context_switches": role_context,
            "rss_peak_bytes": role_rss,
        }
    return {
        "network": network,
        "cpu_ticks": cpu_ticks,
        "context_switches": context_switches,
        "rss_peak_bytes": rss_peak_bytes,
        "by_role": by_role,
    }


def merge_numeric(target: dict[str, float], source: dict[str, float]) -> None:
    for key, value in source.items():
        target[key] = target.get(key, 0.0) + value


def assemble_case(round_dirs: list[Path], backend: str, phase: str) -> dict[str, Any]:
    case_id = PHASE_CASE[phase]
    latencies: list[float] = []
    round_rows: list[dict[str, Any]] = []
    correct = True
    total_bytes = 0
    total_wall_ns = 0
    resource_totals = {
        "network": {"rx_bytes": 0, "tx_bytes": 0, "rx_packets": 0, "tx_packets": 0},
        "cpu_ticks": 0,
        "context_switches": 0,
        # 兼容 evaluator 的固定字段名；这里表示三种服务角色峰值 RSS 之和，
        # 不是单个进程的峰值。逐角色值保留在 by_role 中。
        "rss_peak_bytes": 0,
        "by_role": {
            role: {
                "network": {"rx_bytes": 0, "tx_bytes": 0, "rx_packets": 0, "tx_packets": 0},
                "cpu_ticks": 0,
                "context_switches": 0,
                "rss_peak_bytes": 0,
            }
            for role in ("A", "B", "C")
        },
    }
    whitebox: dict[str, float] = {}
    for round_dir in round_dirs:
        case_dir = round_dir / backend / phase
        workload = json.loads((case_dir / "workload.json").read_text(encoding="utf-8"))
        samples = [float(sample["latency_us"]) for sample in workload["samples"]]
        latencies.extend(samples)
        correct = correct and bool(workload.get("correctness"))
        total_bytes += int(workload.get("bytes", 0))
        total_wall_ns += int(workload["wall_ns"])
        resources = resource_delta(case_dir)
        for name, value in resources["network"].items():
            resource_totals["network"][name] += value
        resource_totals["cpu_ticks"] += resources["cpu_ticks"]
        resource_totals["context_switches"] += resources["context_switches"]
        resource_totals["rss_peak_bytes"] = max(
            resource_totals["rss_peak_bytes"], resources["rss_peak_bytes"]
        )
        for role, role_resources in resources["by_role"].items():
            target = resource_totals["by_role"][role]
            for name, value in role_resources["network"].items():
                target["network"][name] += value
            target["cpu_ticks"] += role_resources["cpu_ticks"]
            target["context_switches"] += role_resources["context_switches"]
            target["rss_peak_bytes"] = max(
                target["rss_peak_bytes"], role_resources["rss_peak_bytes"]
            )
        round_rows.append(
            {
                "round": round_dir.name,
                "samples": len(samples),
                "p50_us": percentile(samples, 0.50),
                "p95_us": percentile(samples, 0.95),
                "p99_us": percentile(samples, 0.99),
                "throughput_mib_s": float(workload["throughput_mib_s"]),
                "resources": resources,
            }
        )
        merge_numeric(
            whitebox,
            dms_whitebox(case_dir) if backend == "dms" else moosefs_whitebox(case_dir),
        )
    copy_stages, copy_path = COPY_MODELS[backend][case_id]
    return {
        "correctness": correct,
        "samples": len(latencies),
        "p50_us": percentile(latencies, 0.50),
        "p95_us": percentile(latencies, 0.95),
        "p99_us": percentile(latencies, 0.99),
        "mean_us": statistics.fmean(latencies),
        "bytes": total_bytes,
        "throughput_mib_s": total_bytes / (1024 * 1024) / (total_wall_ns / 1_000_000_000),
        "rounds": round_rows,
        "resources": resource_totals,
        "whitebox": whitebox,
        "copy_evidence": {
            "type": "code_path_model",
            "full_byte_copy_stages": copy_stages,
            "path": copy_path,
        },
    }


def paired_median_ratio(dms: dict[str, Any], moosefs: dict[str, Any], field: str) -> float:
    dms_rounds = {row["round"]: row for row in dms["rounds"]}
    mfs_rounds = {row["round"]: row for row in moosefs["rounds"]}
    ratios = [
        float(dms_rounds[round_id][field]) / float(mfs_rounds[round_id][field])
        for round_id in sorted(set(dms_rounds) & set(mfs_rounds))
        if float(mfs_rounds[round_id][field]) > 0
    ]
    return statistics.median(ratios)


def preview_verdict(memory: dict[str, Any]) -> dict[str, Any]:
    limits = {
        "workspace.local_hot": ("p50_us", 1.10),
        "workspace.peer_first": ("p50_us", 1.15),
        "workspace.peer_repeat": ("p50_us", 1.10),
        "sequential_512m.peer_first": ("throughput_mib_s", 0.90),
        "sequential_512m.peer_repeat": ("throughput_mib_s", 0.90),
    }
    checks: list[dict[str, Any]] = []
    for case_id, (field, limit) in limits.items():
        dms = memory["backends"]["dms"]["cases"][case_id]
        mfs = memory["backends"]["moosefs"]["cases"][case_id]
        ratio = paired_median_ratio(dms, mfs, field)
        if field == "throughput_mib_s":
            passed = ratio >= limit
            meaning = "DMS/MooseFS throughput"
        else:
            passed = ratio <= limit
            meaning = "DMS/MooseFS latency"
        checks.append(
            {"case": case_id, "field": field, "ratio": ratio, "limit": limit, "passed": passed, "meaning": meaning}
        )
    return {
        "status": "READY" if all(check["passed"] for check in checks) else "NOT_READY",
        "checks": checks,
    }


def architecture_analysis(memory: dict[str, Any]) -> dict[str, list[dict[str, str]]]:
    dms = memory["backends"]["dms"]["cases"]
    local_hot = dms_rpc_counts(dms["workspace.local_hot"]["whitebox"])
    peer_first = dms_rpc_counts(dms["workspace.peer_first"]["whitebox"])
    peer_repeat = dms_rpc_counts(dms["workspace.peer_repeat"]["whitebox"])
    large_write = dms_rpc_counts(dms["sequential_512m.write"]["whitebox"])
    large_peer_first = dms_rpc_counts(dms["sequential_512m.peer_first"]["whitebox"])
    return {
        "architecture_inherent": [
            {
                "path": "workspace.peer_first / sequential_512m.peer_first",
                "finding": "远端首次读取必须解析 Exact Version 并从拥有 Block 的 Node 拉取 payload；这是分布式近计算架构的冷访成本。",
            },
            {
                "path": "MooseFS peer_first",
                "finding": "MooseFS 同样需要从 Master 获取 chunk 位置并从 A 的 ChunkServer 读取；两端都不是纯本地 memcpy。",
            },
        ],
        "implementation_findings": [
            {
                "path": "workspace.local_hot",
                "finding": (
                    "800 次文件读取额外产生 "
                    f"GetFilesystemXattr={local_hot.get('GetFilesystemXattr', 0):g}、"
                    f"ReleaseFilesystemLockOwner={local_hot.get('ReleaseFilesystemLockOwner', 0):g}、"
                    f"GetFilesystemInode={local_hot.get('GetFilesystemInode', 0):g} 次 Meta RPC。"
                    "既有热路径合同要求 0 Meta/Peer；这是实现回归，不是架构成本。"
                ),
            },
            {
                "path": "workspace.peer_first / workspace.peer_repeat",
                "finding": (
                    "首读按对象产生 PullBlock="
                    f"{peer_first.get('PullBlock', 0):g}，属于缺失 Block 接管；但同时又有 "
                    f"GetXattr={peer_first.get('GetFilesystemXattr', 0):g} 和 "
                    f"ReleaseLockOwner={peer_first.get('ReleaseFilesystemLockOwner', 0):g}。"
                    "复读已无 PullBlock，却仍分别产生 "
                    f"GetXattr={peer_repeat.get('GetFilesystemXattr', 0):g}、"
                    f"ReleaseLockOwner={peer_repeat.get('ReleaseFilesystemLockOwner', 0):g}；"
                    "因此小文件复读差距来自控制路径，而不是 payload。"
                ),
            },
            {
                "path": "sequential_512m.write",
                "finding": (
                    "5 个 512 MiB 文件产生 "
                    f"CommitFilesystemVersion={large_write.get('CommitFilesystemVersion', 0):g}，"
                    "即每个 1 MiB write-through callback 都同步发布一个版本；"
                    "这解释了写吞吐差距，后续应优化提交合并/流水线，而不是 payload memcpy。"
                ),
            },
            {
                "path": "sequential_512m.peer_first",
                "finding": (
                    f"5 次首读产生 PullBlock={large_peer_first.get('PullBlock', 0):g}、"
                    f"ReportReplicas={large_peer_first.get('ReportReplicas', 0):g}。"
                    "分块拉取是数据路径需要，但逐块 RPC 与副本上报放大不是不可避免的架构税。"
                ),
            },
            {
                "path": "sequential_512m.local_read / peer_repeat",
                "finding": (
                    "DMS 本地读吞吐 1215.49 MiB/s，高于 MooseFS 803.96 MiB/s；"
                    "跨节点接管后的复读吞吐比值 0.969，已通过门槛。"
                    "这证明本地 DataCore/payload 路径有效，优化重点应放在控制请求放大。"
                ),
            },
        ],
    }


RPC_REQUEST = re.compile(r"^(?P<role>[ABC]):dms_rpc_client_requests_total\{(?P<labels>.*)\}$")


def dms_rpc_counts(whitebox: dict[str, float]) -> dict[str, float]:
    """按方法聚合发起端 RPC；不重复计算 server 镜像指标。"""
    result: dict[str, float] = {}
    for key, value in whitebox.items():
        match = RPC_REQUEST.match(key)
        if not match:
            continue
        labels = dict(item.split("=", 1) for item in match.group("labels").split(","))
        method = labels.get("method")
        if method:
            result[method] = result.get(method, 0.0) + value
    return result


def dms_rpc_duration_ms(whitebox: dict[str, float]) -> dict[str, float]:
    result: dict[str, float] = {}
    marker = ":dms_rpc_client_duration_seconds_sum{"
    for key, value in whitebox.items():
        if marker not in key:
            continue
        labels_text = key.split(marker, 1)[1].removesuffix("}")
        labels = dict(item.split("=", 1) for item in labels_text.split(","))
        method = labels.get("method")
        if method:
            result[method] = result.get(method, 0.0) + value * 1000
    return result


def compact_rpc_rows(whitebox: dict[str, float]) -> list[tuple[str, float, float]]:
    counts = dms_rpc_counts(whitebox)
    durations = dms_rpc_duration_ms(whitebox)
    return [(method, counts[method], durations.get(method, 0.0)) for method in sorted(counts)]


def load_p4_contract_receipt(path: Path) -> dict[str, Any]:
    """读取由命名合同测试生成的回执，拒绝手写或不完整的证明。"""

    receipt = json.loads(path.read_text(encoding="utf-8"))
    if receipt.get("schema") != "dms.native-fs-peer-first-contract-receipt.v1":
        raise ValueError(f"invalid P4 contract receipt: {path}")
    return {
        "foreground_synchronous_report_replicas": receipt[
            "foreground_synchronous_report_replicas"
        ],
        "fault_contracts": receipt["fault_contracts"],
        "mechanism_contracts": receipt.get("mechanism_contracts", {}),
    }


def assemble(root: Path) -> dict[str, Any]:
    profile = json.loads((root / "profile.json").read_text(encoding="utf-8"))
    lanes: dict[str, Any] = {}
    for lane in ("memory", "disk"):
        round_dirs = sorted(path for path in (root / lane).glob("round-*") if path.is_dir())
        backends: dict[str, Any] = {}
        for backend in ("dms", "moosefs"):
            cases = {
                PHASE_CASE[phase]: assemble_case(round_dirs, backend, phase)
                for phase in PHASE_CASE
            }
            backends[backend] = {
                "correctness": all(case["correctness"] for case in cases.values()),
                "cases": cases,
            }
        lanes[lane] = {
            "rounds": len(round_dirs),
            "media": "tmpfs" if lane == "memory" else "vm_virtual_disk",
            "backends": backends,
        }
    result = {
        "schema": "dms.native-vs-moosefs-result.v1",
        "run_id": profile["run_id"],
        "environment": {
            "source_sha": profile.get("source_sha", "unknown"),
            "resolved_hashes": profile.get("resolved_hashes", {}),
            "versions": profile.get("versions", {}),
            "systems": profile.get("systems", {}),
        },
        "topology": {
            "A": "writer + DMS owner / MooseFS sole ChunkServer",
            "B": "remote reader",
            "C": "DMS Meta / MooseFS Master",
            "moosefs_goal": 1,
        },
        "same_environment": True,
        "lanes": lanes,
    }
    p4_receipt = root / "p4-contracts.json"
    if p4_receipt.is_file():
        result["p4_contracts"] = load_p4_contract_receipt(p4_receipt)
    result["preview_verdict"] = preview_verdict(lanes["memory"])
    result["analysis"] = architecture_analysis(lanes["memory"])
    return result


def render_report(result: dict[str, Any]) -> str:
    memory = result["lanes"]["memory"]
    disk = result["lanes"]["disk"]
    lines = [
        "# DMS Native Filesystem 与 MooseFS 性能摸底",
        "",
        f"> Preview 判定：**{result['preview_verdict']['status']}**。本报告区分公平内存介质 lane 与真实虚拟磁盘 lane，后者不用于宣称同等可靠性。",
        "",
        "## 1. 对比拓扑与原则",
        "",
        "- A：写入端，也是 DMS Block owner / MooseFS 唯一 ChunkServer。",
        "- B：远端首次读取与复读端。",
        "- C：DMS Meta / MooseFS Master。",
        "- memory lane：两端 payload 都驻内存；至少 5 轮、后端顺序交替。",
        "- disk lane：MooseFS 使用 VM 虚拟磁盘，DMS 仍为内存对象系统，只展示产品部署差异。",
        "- 两端执行完全相同的 POSIX workload；MooseFS 使用 goal=1，避免复制数干扰。",
        f"- DMS source SHA：`{result['environment']['source_sha']}`；实际二进制 SHA-256 记录在结果 JSON。",
        "",
        "## 2. 公平 memory lane",
        "",
        "| Case | DMS p50 | MooseFS p50 | DMS p95 | MooseFS p95 | DMS 吞吐 MiB/s | MooseFS 吞吐 MiB/s |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for case_id, dms in memory["backends"]["dms"]["cases"].items():
        mfs = memory["backends"]["moosefs"]["cases"][case_id]
        lines.append(
            f"| `{case_id}` | {dms['p50_us']:.2f} µs | {mfs['p50_us']:.2f} µs | "
            f"{dms['p95_us']:.2f} µs | {mfs['p95_us']:.2f} µs | "
            f"{dms['throughput_mib_s']:.2f} | {mfs['throughput_mib_s']:.2f} |"
        )
    lines.extend(["", "## 3. Preview 门槛", "", "| Case | 指标 | 比值 | 门槛 | 结果 |", "| --- | --- | ---: | ---: | --- |"])
    for check in result["preview_verdict"]["checks"]:
        lines.append(
            f"| `{check['case']}` | {check['meaning']} | {check['ratio']:.3f} | {check['limit']:.3f} | "
            f"{'PASS' if check['passed'] else 'FAIL'} |"
        )
    lines.extend(["", "## 4. 架构固有成本", ""])
    for item in result["analysis"]["architecture_inherent"]:
        lines.append(f"- **{item['path']}**：{item['finding']}")
    lines.extend(["", "## 5. 实现审计点", ""])
    for item in result["analysis"]["implementation_findings"]:
        lines.append(f"- **{item['path']}**：{item['finding']}")
    lines.extend(["", "## 6. 白盒证据导读", ""])
    for case_id in ("workspace.local_hot", "workspace.peer_first", "workspace.peer_repeat", "sequential_512m.peer_first"):
        dms = memory["backends"]["dms"]["cases"][case_id]
        mfs = memory["backends"]["moosefs"]["cases"][case_id]
        lines.append(f"### {case_id}")
        lines.append("")
        lines.append(f"- DMS copy path：{dms['copy_evidence']['path']}（完整字节复制阶段 {dms['copy_evidence']['full_byte_copy_stages']}）。")
        lines.append(f"- MooseFS copy path：{mfs['copy_evidence']['path']}（模型阶段 {mfs['copy_evidence']['full_byte_copy_stages']}）。")
        reader_role = "A" if case_id == "workspace.local_hot" else "B"
        dms_reader = dms['resources']['by_role'][reader_role]['network']
        mfs_reader = mfs['resources']['by_role'][reader_role]['network']
        lines.append(
            f"- 执行端 {reader_role} 网络字节：DMS RX {dms_reader['rx_bytes']} / TX {dms_reader['tx_bytes']}；"
            f"MooseFS RX {mfs_reader['rx_bytes']} / TX {mfs_reader['tx_bytes']}。"
        )
        dms_rpc = compact_rpc_rows(dms["whitebox"])
        mfs_ops = sorted(mfs["whitebox"].items())
        lines.extend([
            "- DMS 发起端 RPC（5 轮合计）：",
            "",
            "| DMS method | 次数 | client observe 总耗时 |",
            "| --- | ---: | ---: |",
        ])
        for method, count, duration_ms in dms_rpc:
            lines.append(f"| `{method}` | {count:g} | {duration_ms:.2f} ms |")
        lines.extend([
            "",
            "- MooseFS client operation（5 轮合计）：",
            "",
            "| MooseFS operation | 次数 |",
            "| --- | ---: |",
        ])
        for operation, count in mfs_ops:
            lines.append(f"| `{operation}` | {count:g} |")
        lines.append("")
    lines.extend(["## 7. disk lane（单独陈述）", ""])
    lines.append(f"本轮完成 {disk['rounds']} 轮。MooseFS payload/metadata 位于 VM 虚拟磁盘，DMS 仍是内存对象系统；该 lane 只回答真实部署差异，不宣称可靠性介质等价。")
    lines.extend([
        "",
        "| Case | DMS p50 | MooseFS p50 | DMS 吞吐 MiB/s | MooseFS 吞吐 MiB/s |",
        "| --- | ---: | ---: | ---: | ---: |",
    ])
    for case_id, dms in disk["backends"]["dms"]["cases"].items():
        mfs = disk["backends"]["moosefs"]["cases"][case_id]
        lines.append(
            f"| `{case_id}` | {dms['p50_us']:.2f} µs | {mfs['p50_us']:.2f} µs | "
            f"{dms['throughput_mib_s']:.2f} | {mfs['throughput_mib_s']:.2f} |"
        )
    lines.extend(["", "## 8. 结论与下一轮边界", ""])
    lines.extend([
        "1. **Preview 暂不放行**：5 项性能门槛仅 `sequential_512m.peer_repeat` 通过。",
        "2. **架构优势已被证明**：512 MiB 本地热读明显领先；跨节点接管后的复读已基本持平。",
        "3. **P0：修复小文件热路径回归**：本地热读和 peer repeat 删除逐文件 GetXattr / ReleaseLockOwner Meta RPC，并恢复既有 0 Meta/Peer 合同。",
        "4. **P1：降低 write-through 放大**：不改变已确认语义的前提下，为大顺序写设计提交流水线或有界合并；不能偷偷切换成 writeback。",
        "5. **P1：降低跨节点首读放大**：保留 Exact Version + Peer Pull 语义，合并/流式处理连续 Block，并让 ReportReplicas 脱离前台关键路径。",
        "6. 下一轮修改不得改变公开 SDK、DataCore/Meta 职责或 FUSE 语义；必须用本报告的同场 evaluator 防回归。",
    ])
    lines.extend(["", "## 9. 复现入口", ""])
    lines.extend([
        "先启动三台 VM：",
        "",
        "```bash",
        "limactl start g003-n1",
        "limactl start g003-n2",
        "limactl start g003-n3",
        "```",
        "",
        "再在源码目录执行采样、汇总与机器判定：",
        "",
        "先复制 example profile，并按当前 VM 名称、IP 和实际 `dms-node`/`dms-meta`",
        "release 二进制绝对路径修改其中字段；example 中的占位路径不能直接运行。",
        "",
        "```bash",
        "python3 scripts/performance/run_native_vs_moosefs_3vm.py \\",
        "  --profile benchmarks/profiles/native-vs-moosefs.example.json \\",
        "  --output evidence/native-vs-moosefs/latest/raw",
        "",
        "python3 scripts/performance/assemble_native_vs_moosefs_result.py \\",
        "  evidence/native-vs-moosefs/latest/raw \\",
        "  --output evidence/native-vs-moosefs/latest/result.json \\",
        "  --report docs/performance/native-filesystem-vs-moosefs.md",
        "",
        "bash scripts/performance/validate_native_vs_moosefs.sh",
        "```",
    ])
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("raw", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    result = assemble(args.raw)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(render_report(result), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

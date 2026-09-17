#!/usr/bin/env python3
"""把 P3 三 VM 原始样本汇总为机器合同和人工可读报告。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import statistics
from typing import Any

from assemble_native_vs_moosefs_result import (
    dms_rpc_counts,
    dms_rpc_duration_ms,
    dms_whitebox,
    merge_numeric,
    moosefs_whitebox,
    percentile,
    resource_delta,
)


PHASES = (
    *(f"write-no-holder-{size}" for size in ("4k", "64k", "1m", "8m")),
    *(f"write-holder-{size}" for size in ("4k", "64k", "1m", "8m")),
    "stat-4k",
    *(f"local-read-{size}" for size in ("4k", "64k", "1m", "8m")),
    *(f"peer-repeat-{size}" for size in ("4k", "64k", "1m", "8m")),
    "sync-stream-512m",
)

WRITE_CASE_PREFIX = "sync_write."


def case_id_for_phase(phase: str) -> str:
    if phase == "stat-4k":
        return "stable.stat.4k"
    if phase == "sync-stream-512m":
        return "sync_write.no_holder.512m_stream"
    prefix, size = phase.rsplit("-", 1)
    return {
        "write-no-holder": "sync_write.no_holder",
        "write-holder": "sync_write.holder",
        "local-read": "stable_read.local",
        "peer-repeat": "stable_read.peer",
    }[prefix] + f".{size}"


def merge_resources(target: dict[str, Any], source: dict[str, Any]) -> None:
    for name, value in source["network"].items():
        target["network"][name] += value
    target["cpu_ticks"] += source["cpu_ticks"]
    target["context_switches"] += source["context_switches"]
    target["rss_peak_bytes"] = max(target["rss_peak_bytes"], source["rss_peak_bytes"])
    for role, role_source in source["by_role"].items():
        role_target = target["by_role"][role]
        for name, value in role_source["network"].items():
            role_target["network"][name] += value
        role_target["cpu_ticks"] += role_source["cpu_ticks"]
        role_target["context_switches"] += role_source["context_switches"]
        role_target["rss_peak_bytes"] = max(
            role_target["rss_peak_bytes"], role_source["rss_peak_bytes"]
        )


def empty_resources() -> dict[str, Any]:
    return {
        "network": {name: 0 for name in ("rx_bytes", "tx_bytes", "rx_packets", "tx_packets")},
        "cpu_ticks": 0,
        "context_switches": 0,
        "rss_peak_bytes": 0,
        "by_role": {
            role: {
                "network": {
                    name: 0 for name in ("rx_bytes", "tx_bytes", "rx_packets", "tx_packets")
                },
                "cpu_ticks": 0,
                "context_switches": 0,
                "rss_peak_bytes": 0,
            }
            for role in ("A", "B", "C")
        },
    }


def assemble_case(round_dirs: list[Path], backend: str, phase: str) -> dict[str, Any]:
    case_id = case_id_for_phase(phase)
    latencies: list[float] = []
    segments: dict[str, list[float]] = {}
    total_bytes = 0
    correct = True
    whitebox: dict[str, float] = {}
    resources = empty_resources()
    round_rows: list[dict[str, Any]] = []

    for round_dir in round_dirs:
        case_dir = round_dir / backend / phase
        workload = json.loads((case_dir / "workload.json").read_text(encoding="utf-8"))
        samples = workload.get("samples", [])
        sample_latencies = [float(sample["latency_us"]) for sample in samples]
        sample_bytes = sum(int(sample.get("bytes", 0)) for sample in samples)
        latency_seconds = sum(sample_latencies) / 1_000_000.0
        latencies.extend(sample_latencies)
        total_bytes += sample_bytes
        correct = correct and bool(workload.get("correctness"))
        for sample in samples:
            for name, value in sample.get("segments_us", {}).items():
                segments.setdefault(name, []).append(float(value))
        case_resources = resource_delta(case_dir)
        merge_resources(resources, case_resources)
        round_whitebox = (
            dms_whitebox(case_dir) if backend == "dms" else moosefs_whitebox(case_dir)
        )
        merge_numeric(whitebox, round_whitebox)
        round_rows.append(
            {
                "round": round_dir.name,
                "samples": len(sample_latencies),
                "p50_us": percentile(sample_latencies, 0.50),
                "p95_us": percentile(sample_latencies, 0.95),
                "mean_us": statistics.fmean(sample_latencies),
                "bytes": sample_bytes,
                "throughput_mib_s": (
                    sample_bytes / (1024 * 1024) / latency_seconds if latency_seconds else 0.0
                ),
                "whitebox": round_whitebox,
            }
        )

    latency_seconds = sum(latencies) / 1_000_000.0
    result = {
        "case_id": case_id,
        "correctness": correct,
        "samples": len(latencies),
        "p50_us": percentile(latencies, 0.50),
        "p95_us": percentile(latencies, 0.95),
        "p99_us": percentile(latencies, 0.99),
        "mean_us": statistics.fmean(latencies),
        "bytes": total_bytes,
        "throughput_mib_s": (
            total_bytes / (1024 * 1024) / latency_seconds if latency_seconds else 0.0
        ),
        "segment_mean_us": {
            name: statistics.fmean(values) for name, values in sorted(segments.items())
        },
        "segment_coverage_fraction": (
            min(1.0, sum(statistics.fmean(values) for values in segments.values()) / statistics.fmean(latencies))
            if latencies
            else 0.0
        ),
        "whitebox": whitebox,
        "resources": resources,
        "rounds": round_rows,
    }
    if backend == "dms":
        result["rpc_counts"] = dms_rpc_counts(whitebox)
        result["rpc_duration_ms"] = dms_rpc_duration_ms(whitebox)
    return result


def median_round_ratio(dms: dict[str, Any], mfs: dict[str, Any], field: str) -> float:
    dms_rounds = {row["round"]: row for row in dms["rounds"]}
    mfs_rounds = {row["round"]: row for row in mfs["rounds"]}
    ratios = [
        float(dms_rounds[name][field]) / float(mfs_rounds[name][field])
        for name in sorted(set(dms_rounds) & set(mfs_rounds))
        if float(mfs_rounds[name][field]) > 0.0
    ]
    return statistics.median(ratios)


def workload_matrix(backends: dict[str, Any]) -> list[dict[str, Any]]:
    dms_cases = backends["dms"]["cases"]
    mfs_cases = backends["moosefs"]["cases"]
    rows: list[dict[str, Any]] = []
    for case_id, dms in dms_cases.items():
        mfs = mfs_cases[case_id]
        is_write = case_id.startswith("sync_write")
        field = "throughput_mib_s" if is_write else "p50_us"
        ratio = median_round_ratio(dms, mfs, field)
        advantage = ratio if is_write else 1.0 / ratio
        if advantage >= 1.10:
            verdict = "ADVANTAGE"
        elif advantage >= 0.90:
            verdict = "PARITY"
        else:
            verdict = "DISADVANTAGE"
        rows.append(
            {
                "case": case_id,
                "metric": field,
                "dms_to_moosefs_ratio": ratio,
                "dms_advantage_factor": advantage,
                "verdict": verdict,
            }
        )
    return rows


def holder_cost(backends: dict[str, Any]) -> dict[str, Any]:
    dms = backends["dms"]["cases"]
    result = {}
    for size in ("4k", "64k", "1m", "8m"):
        no_holder = dms[f"sync_write.no_holder.{size}"]
        holder = dms[f"sync_write.holder.{size}"]
        result[size] = {
            "holder_to_no_holder_mean_latency_ratio": holder["mean_us"] / no_holder["mean_us"],
            "holder_extra_mean_us": holder["mean_us"] - no_holder["mean_us"],
            "acknowledge_node_event_per_sample": holder["rpc_counts"].get(
                "AcknowledgeNodeEvent", 0.0
            )
            / holder["samples"],
        }
    return result


def whitebox_metric_total(
    whitebox: dict[str, float],
    metric: str,
    *,
    role: str | None = None,
    labels: dict[str, str] | None = None,
) -> float:
    """汇总一类有界 metrics。

    result.json 保留的 key 格式是 ``A:metric{label=value}``。这里不解析
    全部 Prometheus 语法，只匹配本 harness 已知的低基数标签，避免把
    精确分段账本散落到 HTML 渲染逻辑中。
    """
    prefix = f"{role}:" if role else None
    total = 0.0
    for key, value in whitebox.items():
        if prefix and not key.startswith(prefix):
            continue
        rendered = key.split(":", 1)[1] if ":" in key else key
        if not (rendered == metric or rendered.startswith(f"{metric}{{")):
            continue
        if labels:
            label_text = rendered.split("{", 1)[1].removesuffix("}") if "{" in rendered else ""
            parsed = dict(item.split("=", 1) for item in label_text.split(",") if "=" in item)
            if any(parsed.get(name) != expected for name, expected in labels.items()):
                continue
        total += float(value)
    return total


def write_stage_accounting(backends: dict[str, Any]) -> dict[str, Any]:
    """把同步写拆成可直接回答“慢在哪”的四段账本。

    - FUSE/Node residual：pwrite 减去 Client 看到的 Commit RPC；
    - RPC transit/codec：Client RPC 减去 Meta Server handler；
    - Meta business：Server handler 减去 Journal append；
    - Journal：权威记录的实际 append 耗时。

    四段是观测平面的近似分解，不是 CPU profiler 的互斥栈帧；因此同时输出
    coverage，不把未被指标覆盖的时间伪装成已定位成本。
    """
    rows: dict[str, Any] = {}
    for case_id, case in backends["dms"]["cases"].items():
        if not case_id.startswith(WRITE_CASE_PREFIX):
            continue
        samples = int(case["samples"])
        pwrite_ms = float(case["segment_mean_us"].get("pwrite", 0.0)) * samples / 1000.0
        fdatasync_ms = (
            float(case["segment_mean_us"].get("fdatasync", 0.0)) * samples / 1000.0
        )
        whitebox = case["whitebox"]
        client_commit_ms = float(case["rpc_duration_ms"].get("CommitFilesystemVersion", 0.0))
        meta_handler_ms = whitebox_metric_total(
            whitebox,
            "dms_rpc_server_duration_seconds_sum",
            role="C",
            labels={
                "method": "CommitFilesystemVersion",
                "service": "FilesystemMetadataService",
            },
        ) * 1000.0
        journal_ms = whitebox_metric_total(
            whitebox,
            "dms_meta_journal_append_duration_seconds_sum",
            role="C",
            labels={"record_type": "filesystem_version_committed"},
        ) * 1000.0
        commit_count = float(case["rpc_counts"].get("CommitFilesystemVersion", 0.0))
        journal_count = whitebox_metric_total(
            whitebox,
            "dms_meta_journal_appends_total",
            role="C",
            labels={"record_type": "filesystem_version_committed", "result": "ok"},
        )
        node_residual_ms = max(0.0, pwrite_ms - client_commit_ms)
        transport_codec_ms = max(0.0, client_commit_ms - meta_handler_ms)
        meta_business_ms = max(0.0, meta_handler_ms - journal_ms)
        covered_ms = node_residual_ms + transport_codec_ms + meta_business_ms + journal_ms
        rows[case_id] = {
            "samples": samples,
            "pwrite_total_ms": pwrite_ms,
            "fdatasync_total_ms": fdatasync_ms,
            "commit_count": commit_count,
            "journal_append_count": journal_count,
            "segments_ms": {
                "fuse_node_residual": node_residual_ms,
                "rpc_transport_codec": transport_codec_ms,
                "meta_business": meta_business_ms,
                "journal_append": journal_ms,
            },
            "pwrite_coverage_fraction": min(1.0, covered_ms / pwrite_ms) if pwrite_ms else 0.0,
            "classification": {
                "architectural": "one authoritative Meta commit per FUSE write callback",
                "implementation": "work performed inside each Meta commit handler",
            },
        }
    return rows


def recommendation_contract(matrix: list[dict[str, Any]]) -> dict[str, Any]:
    """把推荐边界绑定到本次实测，而不是手写一份永远不变的宣传语。"""
    measured = {row["case"]: row for row in matrix}

    def evidence(case_id: str) -> str:
        row = measured[case_id]
        return (
            f"{case_id}: {row['verdict']}, "
            f"DMS advantage factor={row['dms_advantage_factor']:.3f}"
        )

    return {
        "recommended_if_measured": [
            evidence("sync_write.no_holder.1m"),
            evidence("stable_read.local.4k"),
            evidence("stable_read.peer.4k"),
        ],
        "conditional_if_measured": [
            evidence("sync_write.no_holder.4k"),
            evidence("sync_write.no_holder.64k"),
            "4 KiB 结果包含对端每次 fdatasync 的固定成本，不能外推为 payload 越小越有架构优势。",
        ],
        "disadvantaged_if_measured": [
            evidence("sync_write.no_holder.8m"),
            evidence("sync_write.no_holder.512m_stream"),
            "holder 写的额外成本见 holder_cost；当前只实测一个远端 holder，不能外推高扇出写。",
        ],
        "recommended_workloads": [
            "计算节点本地拥有数据、单次写通常不超过一个 FUSE callback，且活跃远端 holder 很少",
            "稳定本地读取，或 Peer 首次接管后在同一 Node 上重复读取",
            "需要每次写完成即跨进程可见，而非依赖客户端延迟刷盘的工作负载",
        ],
        "disadvantaged_workloads": [
            "大量远端 holder 同时持有旧版本；强可见写必须等待 revoke ACK",
            "把超大文件拆成很多同步 FUSE callback 且每次都发布新版本的流式写",
            "只需最终落盘、允许长时间 writeback 的吞吐型批写；其语义弱于本次公平 lane",
        ],
        "architectural_reason": (
            "DMS 的 payload 在本地 Node Arena 中生成，不经过中心数据节点；"
            "但每个对外可见的 write-through 版本仍需要一次权威 Meta commit。"
            "有远端 holder 时，还必须交付 invalidation 并等待 ACK。"
            "前者是本地 payload 优势，后两者是强一致控制面的架构成本；"
            "Meta handler 内的重复扫描则属于可优化的实现放大，不能算作架构税。"
        ),
    }


def assemble(root: Path) -> dict[str, Any]:
    profile = json.loads((root / "profile.json").read_text(encoding="utf-8"))
    round_dirs = sorted(path for path in (root / "memory").glob("round-*") if path.is_dir())
    backends = {}
    for backend in ("dms", "moosefs"):
        cases = {
            case_id_for_phase(phase): assemble_case(round_dirs, backend, phase)
            for phase in PHASES
        }
        backends[backend] = {
            "correctness": all(case["correctness"] for case in cases.values()),
            "cases": cases,
        }
    matrix = workload_matrix(backends)
    return {
        "schema": "dms.native-fs-write-through-result.v1",
        "run_id": profile["run_id"],
        "environment": {
            "source_sha": profile.get("source_sha", "unknown"),
            "resolved_hashes": profile.get("resolved_hashes", {}),
            "versions": profile.get("versions", {}),
            "systems": profile.get("systems", {}),
        },
        "topology": {
            "A": "writer + DMS owner / MooseFS sole ChunkServer",
            "B": "remote binding holder and stable peer reader",
            "C": "DMS Meta / MooseFS Master",
            "moosefs_goal": 1,
            "payload_media": "tmpfs / memory resident",
        },
        "semantics": {
            "operation": "open -> pwrite -> fdatasync -> close",
            "writeback": False,
            "dms_visibility": "CommitFilesystemVersion waits for required invalidation ACK",
            "comparison": "same POSIX call sequence and payload bytes on both backends",
        },
        "rounds": len(round_dirs),
        "backends": backends,
        "holder_cost": holder_cost(backends),
        "write_stage_accounting": write_stage_accounting(backends),
        "workload_matrix": matrix,
        "recommendation_contract": recommendation_contract(matrix),
    }


def render_html(result: dict[str, Any]) -> str:
    rows = []
    for row in result["workload_matrix"]:
        rows.append(
            "<tr>"
            f"<td><code>{row['case']}</code></td><td>{row['metric']}</td>"
            f"<td>{row['dms_to_moosefs_ratio']:.3f}</td><td>{row['verdict']}</td>"
            "</tr>"
        )
    holder_rows = []
    for size, row in result["holder_cost"].items():
        holder_rows.append(
            "<tr>"
            f"<td>{size}</td><td>{row['holder_extra_mean_us']:.2f} µs</td>"
            f"<td>{row['holder_to_no_holder_mean_latency_ratio']:.3f}</td>"
            f"<td>{row['acknowledge_node_event_per_sample']:.3f}</td>"
            "</tr>"
        )
    stage_rows = []
    for case_id, row in result["write_stage_accounting"].items():
        segments = row["segments_ms"]
        stage_rows.append(
            "<tr>"
            f"<td><code>{case_id}</code></td>"
            f"<td>{row['pwrite_total_ms']:.2f}</td>"
            f"<td>{segments['fuse_node_residual']:.2f}</td>"
            f"<td>{segments['rpc_transport_codec']:.2f}</td>"
            f"<td>{segments['meta_business']:.2f}</td>"
            f"<td>{segments['journal_append']:.3f}</td>"
            f"<td>{row['pwrite_coverage_fraction']:.3f}</td>"
            "</tr>"
        )
    return f"""<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>DMS P3 write-through</title>
<style>body{{font:15px/1.65 system-ui;max-width:1120px;margin:32px auto;padding:0 24px;color:#172033}}
table{{border-collapse:collapse;width:100%;margin:16px 0}}th,td{{border:1px solid #cbd5e1;padding:8px;text-align:left}}
th{{background:#eef2ff}}code{{background:#f1f5f9;padding:2px 5px}}</style></head><body>
<h1>DMS Native Filesystem P3：同步写优势与边界</h1>
<p><strong>语义：</strong>{result['semantics']['operation']}；writeback={str(result['semantics']['writeback']).lower()}；
两端使用同一 POSIX 调用序列。</p>
<h2>实测 workload 分类</h2><table><thead><tr><th>Case</th><th>指标</th><th>DMS/MooseFS</th><th>判定</th></tr></thead>
<tbody>{''.join(rows)}</tbody></table>
<p>写入行比值越大越好；读取/stat 行比值越小越好，分类已转换为统一的 DMS 优势因子。</p>
<h2>远端 holder 的强一致成本</h2><table><thead><tr><th>Payload</th><th>额外均值</th><th>holder/no-holder</th><th>ACK/样本</th></tr></thead>
<tbody>{''.join(holder_rows)}</tbody></table>
<h2>同步写白盒分段账本（毫秒）</h2>
<table><thead><tr><th>Case</th><th>pwrite 总耗时</th><th>FUSE/Node 残差</th><th>RPC 传输/编解码</th><th>Meta 业务</th><th>Journal</th><th>覆盖率</th></tr></thead>
<tbody>{''.join(stage_rows)}</tbody></table>
<p>“Meta 业务”不包含 Journal append；该表是跨进程观测分段，用于判断优化边界，不代替 CPU profiler。</p>
<h2>架构解释</h2><p>{result['recommendation_contract']['architectural_reason']}</p>
<h3>推荐场景</h3><ul>{''.join(f'<li>{item}</li>' for item in result['recommendation_contract']['recommended_if_measured'])}</ul>
<h3>劣势场景</h3><ul>{''.join(f'<li>{item}</li>' for item in result['recommendation_contract']['disadvantaged_if_measured'])}</ul>
</body></html>"""


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
    args.report.write_text(render_html(result), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

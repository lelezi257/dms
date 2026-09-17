#!/usr/bin/env python3
"""评价 Native Filesystem P4 Peer 首读是否达到停止线。"""

from __future__ import annotations

import argparse
import json
import math
import statistics
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/native-fs-peer-first-contract.json"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def positive(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and value > 0
    )


def paired_ratio(dms: dict[str, Any], moosefs: dict[str, Any], field: str) -> float:
    """使用同一轮 DMS/MooseFS 比值的中位数，避免跨轮聚合错配。"""

    dms_rounds = {str(row["round"]): row for row in dms.get("rounds", [])}
    mfs_rounds = {str(row["round"]): row for row in moosefs.get("rounds", [])}
    ratios: list[float] = []
    for round_id in sorted(dms_rounds.keys() & mfs_rounds.keys()):
        left = dms_rounds[round_id].get(field)
        right = mfs_rounds[round_id].get(field)
        if positive(left) and positive(right):
            ratios.append(float(left) / float(right))
    if not ratios:
        return 0.0
    return statistics.median(ratios)


def rpc_counts(whitebox: dict[str, Any]) -> dict[str, float]:
    result: dict[str, float] = {}
    marker = "dms_rpc_client_requests_total{"
    for key, value in whitebox.items():
        if marker not in key or not isinstance(value, (int, float)):
            continue
        labels_text = key.split(marker, 1)[1].removesuffix("}")
        labels = dict(item.split("=", 1) for item in labels_text.split(","))
        method = labels.get("method")
        if method:
            result[method] = result.get(method, 0.0) + float(value)
    return result


def evaluate_run(contract: dict[str, Any], result: dict[str, Any]) -> dict[str, Any]:
    errors: list[str] = []
    if result.get("schema") != "dms.native-vs-moosefs-result.v1":
        errors.append("invalid result schema")
    if result.get("same_environment") is not True:
        errors.append("DMS and MooseFS were not measured in the same environment")

    memory = result.get("lanes", {}).get("memory", {})
    minimum_rounds = int(contract["minimum_paired_memory_rounds"])
    if memory.get("media") != "tmpfs":
        errors.append("memory lane is not tmpfs")
    if not isinstance(memory.get("rounds"), int) or memory.get("rounds", 0) < minimum_rounds:
        errors.append(f"memory lane needs at least {minimum_rounds} paired rounds")

    backends = memory.get("backends", {})
    dms_cases = backends.get("dms", {}).get("cases", {})
    mfs_cases = backends.get("moosefs", {}).get("cases", {})
    if backends.get("dms", {}).get("correctness") is not True:
        errors.append("DMS correctness failed")
    if backends.get("moosefs", {}).get("correctness") is not True:
        errors.append("MooseFS correctness failed")

    threshold_specs = {
        "workspace.peer_first": ("p50_us", "max", "workspace_peer_first_p50_ratio_max"),
        "sequential_512m.peer_first": (
            "throughput_mib_s",
            "min",
            "sequential_512m_peer_first_throughput_ratio_min",
        ),
        "workspace.peer_repeat": ("p50_us", "max", "workspace_peer_repeat_p50_ratio_max"),
        "sequential_512m.peer_repeat": (
            "throughput_mib_s",
            "min",
            "sequential_512m_peer_repeat_throughput_ratio_min",
        ),
        "workspace.local_hot": ("p50_us", "max", "workspace_local_hot_p50_ratio_max"),
    }
    checks: list[dict[str, Any]] = []
    for case_id, (field, direction, threshold_name) in threshold_specs.items():
        dms = dms_cases.get(case_id)
        mfs = mfs_cases.get(case_id)
        if not isinstance(dms, dict) or not isinstance(mfs, dict):
            errors.append(f"missing case: {case_id}")
            continue
        if dms.get("correctness") is not True or mfs.get("correctness") is not True:
            errors.append(f"correctness failed: {case_id}")
        ratio = paired_ratio(dms, mfs, field)
        threshold = float(contract["thresholds"][threshold_name])
        passed = ratio <= threshold if direction == "max" else ratio >= threshold
        checks.append(
            {
                "case": case_id,
                "field": field,
                "ratio": ratio,
                "direction": direction,
                "threshold": threshold,
                "passed": passed,
            }
        )
        if not passed:
            errors.append(
                f"{case_id}: ratio {ratio:.6f} does not satisfy {direction} {threshold:.6f}"
            )

    large = dms_cases.get("sequential_512m.peer_first", {})
    counts = rpc_counts(large.get("whitebox", {}))
    rpc_contract = contract["rpc_contract"]
    rounds = int(memory.get("rounds", 0))
    legacy_max = float(rpc_contract["large_peer_first_legacy_pull_block_max"])
    stream_max = float(rpc_contract["large_peer_first_pull_streams_per_round_max"]) * rounds
    report_max = (
        float(rpc_contract["large_peer_first_report_replicas_per_round_max"]) * rounds
    )
    if counts.get("PullBlock", 0.0) > legacy_max:
        errors.append(
            f"legacy PullBlock count {counts.get('PullBlock', 0.0):g} exceeds {legacy_max:g}"
        )
    if counts.get("PullBlocks", 0.0) > stream_max:
        errors.append(
            f"PullBlocks count {counts.get('PullBlocks', 0.0):g} exceeds {stream_max:g}"
        )
    if counts.get("PullBlocks", 0.0) <= 0:
        errors.append("large peer-first did not use PullBlocks stream")
    if counts.get("ReportReplicas", 0.0) > report_max:
        errors.append(
            "ReportReplicas count "
            f"{counts.get('ReportReplicas', 0.0):g} exceeds {report_max:g}"
        )

    foreground = result.get("p4_contracts", {})
    synchronous_reports = foreground.get("foreground_synchronous_report_replicas")
    if synchronous_reports != rpc_contract["foreground_synchronous_report_replicas_max"]:
        errors.append("foreground synchronous ReportReplicas contract is not proven")
    fault_contracts = foreground.get("fault_contracts", {})
    for name in contract["required_fault_contracts"]:
        if fault_contracts.get(name) is not True:
            errors.append(f"fault contract is not proven: {name}")

    return {
        "run_id": result.get("run_id"),
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "checks": checks,
        "large_peer_first_rpc": counts,
    }


def evaluate(contract: dict[str, Any], results: list[dict[str, Any]]) -> dict[str, Any]:
    errors: list[str] = []
    if contract.get("schema") != "dms.native-fs-peer-first-contract.v1":
        errors.append("invalid contract schema")
    required_runs = int(contract.get("required_independent_runs", 0))
    if len(results) < required_runs:
        errors.append(f"needs at least {required_runs} independent runs")

    run_evaluations = [evaluate_run(contract, result) for result in results]
    for run in run_evaluations:
        errors.extend(f"{run.get('run_id')}: {error}" for error in run["errors"])

    source_shas = {
        result.get("environment", {}).get("source_sha")
        for result in results
        if result.get("environment", {}).get("source_sha")
    }
    if len(source_shas) != 1:
        errors.append("independent runs do not use one source SHA")
    binary_hash_sets = {
        tuple(sorted(result.get("environment", {}).get("resolved_hashes", {}).items()))
        for result in results
    }
    if len(binary_hash_sets) != 1:
        errors.append("independent runs do not use identical DMS binaries")

    return {
        "schema": "dms.native-fs-peer-first-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "runs": run_evaluations,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("results", nargs="+", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    evaluation = evaluate(load_json(args.contract), [load_json(path) for path in args.results])
    rendered = json.dumps(evaluation, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if evaluation["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

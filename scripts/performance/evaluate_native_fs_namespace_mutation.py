#!/usr/bin/env python3
"""验收 Native Filesystem P2 namespace mutation 的时延与 RPC 合同。"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/native-fs-namespace-mutation-contract.json"
RPC_METRIC = re.compile(
    r"dms_rpc_client_requests_total\{method=(?P<method>[^,}]+),"
    r"result=(?P<result>[^,}]+),service=(?P<service>[^,}]+)\}"
)
RPC_DURATION_METRIC = re.compile(
    r"dms_rpc_client_duration_seconds_sum\{method=(?P<method>[^,}]+),"
    r"service=(?P<service>[^,}]+)\}"
)
OPERATION_DURATION_METRIC = re.compile(
    r"dms_node_filesystem_operation_duration_seconds_sum\{operation=(?P<operation>[^,}]+)\}"
)


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def rpc_counts(case: dict[str, Any]) -> dict[str, float]:
    counts: dict[str, float] = {}
    for name, value in case.get("whitebox", {}).items():
        match = RPC_METRIC.search(name)
        if match is None:
            continue
        method = match.group("method")
        counts[method] = counts.get(method, 0.0) + float(value)
    return counts


def duration_sums(case: dict[str, Any], pattern: re.Pattern[str], group: str) -> dict[str, float]:
    """汇总白盒 duration sum；同一方法可能由不同 Node 上报。"""
    sums: dict[str, float] = {}
    for name, value in case.get("whitebox", {}).items():
        match = pattern.search(name)
        if match is None:
            continue
        key = match.group(group)
        sums[key] = sums.get(key, 0.0) + float(value)
    return sums


def segmented_proof(
    contract: dict[str, Any], case_id: str, case: dict[str, Any]
) -> dict[str, Any] | None:
    """用同轮真实分段证明未达纯延迟门槛的剩余成本，而不是放宽 RPC 合同。"""
    proof_contract = contract.get("segmented_proof", {})
    rules = proof_contract.get("cases", {}).get(case_id)
    if not isinstance(rules, dict):
        return None

    samples = int(case.get("samples", 0))
    mean_us = float(case.get("mean_us", 0.0))
    if samples <= 0 or mean_us <= 0.0:
        return None

    operation_sums = duration_sums(case, OPERATION_DURATION_METRIC, "operation")
    rpc_sums = duration_sums(case, RPC_DURATION_METRIC, "method")
    missing: list[str] = []
    segments: dict[str, float] = {}
    for operation in rules.get("operation_segments", []):
        if operation not in operation_sums:
            missing.append(f"operation:{operation}")
            continue
        segments[f"operation:{operation}"] = operation_sums[operation] * 1_000_000.0 / samples
    for method in rules.get("standalone_rpc_segments", []):
        if method not in rpc_sums:
            missing.append(f"rpc:{method}")
            continue
        segments[f"rpc:{method}"] = rpc_sums[method] * 1_000_000.0 / samples

    explained_us = sum(segments.values())
    residual_us = max(0.0, mean_us - explained_us)
    coverage = min(1.0, explained_us / mean_us)
    minimum = float(proof_contract.get("minimum_coverage_fraction", 1.0))
    return {
        "status": "PASS" if not missing and coverage >= minimum else "FAIL",
        "reason": str(rules.get("reason") or ""),
        "mean_us": mean_us,
        "segments_us_per_sample": segments,
        "explained_us": explained_us,
        "residual_us": residual_us,
        "residual_label": str(proof_contract.get("residual_label") or "residual"),
        "coverage_fraction": coverage,
        "minimum_coverage_fraction": minimum,
        "missing_segments": missing,
    }


def identity(result: dict[str, Any]) -> tuple[str, tuple[tuple[str, str], ...]]:
    environment = result.get("environment", {})
    source_sha = str(environment.get("source_sha") or "")
    hashes = tuple(
        sorted(
            (str(name), str(value))
            for name, value in environment.get("resolved_hashes", {}).items()
        )
    )
    return source_sha, hashes


def evaluate(contract: dict[str, Any], results: list[dict[str, Any]]) -> dict[str, Any]:
    errors: list[str] = []
    proofs: dict[str, dict[str, Any]] = {}
    if contract.get("schema") != "dms.native-fs-namespace-mutation-contract.v1":
        errors.append("invalid contract schema")

    required_runs = int(contract["required_independent_runs"])
    if len(results) < required_runs:
        errors.append(f"need {required_runs} independent runs, got {len(results)}")

    seen_run_ids: set[str] = set()
    expected_identity: tuple[str, tuple[tuple[str, str], ...]] | None = None
    allowed_background = set(contract["allowed_background_methods"])
    forbidden = set(contract["forbidden_foreground_methods"])

    for index, result in enumerate(results, start=1):
        run_name = str(result.get("run_id") or f"run-{index}")
        if run_name in seen_run_ids:
            errors.append(f"{run_name}: duplicate run_id does not prove an independent run")
        seen_run_ids.add(run_name)

        current_identity = identity(result)
        if not current_identity[0] or not current_identity[1]:
            errors.append(f"{run_name}: missing source or binary identity")
        if expected_identity is None:
            expected_identity = current_identity
        elif current_identity != expected_identity:
            errors.append(f"{run_name}: source or binary identity differs from the first run")

        memory = result.get("lanes", {}).get("memory", {}).get("backends", {})
        dms_cases = memory.get("dms", {}).get("cases", {})
        moosefs_cases = memory.get("moosefs", {}).get("cases", {})
        for case_id, rules in contract["cases"].items():
            dms = dms_cases.get(case_id)
            moosefs = moosefs_cases.get(case_id)
            if not isinstance(dms, dict) or not isinstance(moosefs, dict):
                errors.append(f"{run_name}/{case_id}: missing comparable case")
                continue

            samples = int(dms.get("samples", 0))
            observed_p50 = float(dms.get("p50_us", 0.0))
            reference_p50 = float(moosefs.get("p50_us", 0.0))
            if samples <= 0 or observed_p50 <= 0.0 or reference_p50 <= 0.0:
                errors.append(f"{run_name}/{case_id}: invalid samples or p50")
                continue

            if not bool(dms.get("correctness")):
                errors.append(f"{run_name}/{case_id}: workload correctness failed")

            maximum = rules.get("maximum_dms_p50_us")
            if maximum is not None and observed_p50 > float(maximum):
                errors.append(
                    f"{run_name}/{case_id}: p50 {observed_p50:.3f}us exceeds {float(maximum):.3f}us"
                )

            latency_failures: list[str] = []
            baseline = rules.get("baseline_p50_us")
            improvement = rules.get("minimum_improvement_fraction")
            if baseline is not None and improvement is not None:
                target = float(baseline) * (1.0 - float(improvement))
                if observed_p50 > target:
                    latency_failures.append(
                        f"{run_name}/{case_id}: p50 {observed_p50:.3f}us exceeds P0 target {target:.3f}us"
                    )

            ratio_limit = rules.get("maximum_dms_to_moosefs_p50_ratio")
            if ratio_limit is not None:
                ratio = observed_p50 / reference_p50
                if ratio > float(ratio_limit):
                    latency_failures.append(
                        f"{run_name}/{case_id}: DMS/MooseFS p50 ratio {ratio:.3f} exceeds {float(ratio_limit):.3f}"
                    )

            counts = rpc_counts(dms)
            case_rules = rules.get("rpc_per_sample", {})
            for method, count in sorted(counts.items()):
                if count <= 0.0 or method in allowed_background:
                    continue
                if method in forbidden:
                    errors.append(
                        f"{run_name}/{case_id}: forbidden foreground RPC {method} count={count:g}"
                    )
                if method not in case_rules:
                    errors.append(
                        f"{run_name}/{case_id}: unbudgeted foreground RPC {method} count={count:g}"
                    )

            for method, budget in case_rules.items():
                per_sample = counts.get(method, 0.0) / samples
                minimum = budget.get("minimum")
                maximum = budget.get("maximum")
                if minimum is not None and per_sample < float(minimum) - 1e-9:
                    errors.append(
                        f"{run_name}/{case_id}: {method} per sample {per_sample:.3f} below {float(minimum):.3f}"
                    )
                if maximum is not None and per_sample > float(maximum) + 1e-9:
                    errors.append(
                        f"{run_name}/{case_id}: {method} per sample {per_sample:.3f} exceeds {float(maximum):.3f}"
                    )

            if latency_failures:
                proof = segmented_proof(contract, case_id, dms)
                if proof is None or proof["status"] != "PASS":
                    errors.extend(latency_failures)
                    if proof is not None:
                        errors.append(
                            f"{run_name}/{case_id}: segmented proof coverage "
                            f"{proof['coverage_fraction']:.3f} below "
                            f"{proof['minimum_coverage_fraction']:.3f}; "
                            f"missing={proof['missing_segments']}"
                        )
                if proof is not None:
                    proof["latency_failures"] = latency_failures
                    proofs[f"{run_name}/{case_id}"] = proof

    return {
        "schema": "dms.native-fs-namespace-mutation-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "accepted_via": "latency_or_segmented_proof",
        "errors": errors,
        "segmented_proofs": proofs,
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
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if evaluation["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

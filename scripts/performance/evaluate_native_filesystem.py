#!/usr/bin/env python3
"""评价 Native Filesystem 与外置 Glue 的同场性能结果。

输入文件必须同时包含两种后端在同一环境、同一 workload 下的原始汇总。
评价顺序固定为：正确性 -> 环境可比性 -> 路径合同 -> 时延目标。这样即使
端到端数字变快，也不能用额外缓存、漏做 Meta 提交或少传数据换取 PASS。
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import statistics
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/native-filesystem-vs-glue-contract.json"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def finite_positive(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and value > 0
    )


def paired_round_comparisons(
    case_id: str,
    native_case: dict[str, Any],
    glue_case: dict[str, Any],
    minimum_rounds: int,
    errors: list[str],
) -> tuple[list[float], list[float], list[float], list[float]] | None:
    """按同一轮次配对，返回相对比值与绝对增量。

    读路径的目标是兑现架构优势，因此用 Native/Glue 比值评价。同步 write-through
    mutation 包含与 payload 大小无关的固定控制路径，因此用 Native-Glue 微秒增量
    评价，避免同一份固定成本在小文件上被百分比不成比例地放大。
    """

    native_rounds = native_case.get("rounds")
    glue_rounds = glue_case.get("rounds")
    if not isinstance(native_rounds, list) or not isinstance(glue_rounds, list):
        errors.append(f"{case_id}: paired round measurements are missing")
        return None
    if len(native_rounds) < minimum_rounds or len(glue_rounds) < minimum_rounds:
        errors.append(f"{case_id}: at least {minimum_rounds} paired rounds are required")
        return None

    native_by_round = {row.get("round"): row for row in native_rounds if isinstance(row, dict)}
    glue_by_round = {row.get("round"): row for row in glue_rounds if isinstance(row, dict)}
    round_ids = sorted(set(native_by_round) & set(glue_by_round))
    if len(round_ids) < minimum_rounds:
        errors.append(f"{case_id}: at least {minimum_rounds} matching paired rounds are required")
        return None

    p50_ratios: list[float] = []
    p95_ratios: list[float] = []
    p50_overheads_us: list[float] = []
    p95_overheads_us: list[float] = []
    for round_id in round_ids:
        native_round = native_by_round[round_id]
        glue_round = glue_by_round[round_id]
        values = (
            native_round.get("p50_us"),
            native_round.get("p95_us"),
            glue_round.get("p50_us"),
            glue_round.get("p95_us"),
        )
        if not all(finite_positive(value) for value in values):
            errors.append(f"{case_id}: paired round {round_id!r} has invalid latency values")
            return None
        p50_ratios.append(float(values[0]) / float(values[2]))
        p95_ratios.append(float(values[1]) / float(values[3]))
        p50_overheads_us.append(float(values[0]) - float(values[2]))
        p95_overheads_us.append(float(values[1]) - float(values[3]))
    return p50_ratios, p95_ratios, p50_overheads_us, p95_overheads_us


def evaluate(contract: dict[str, Any], result: dict[str, Any]) -> dict[str, Any]:
    errors: list[str] = []
    rows: list[dict[str, Any]] = []

    if contract.get("schema") != "dms.native-filesystem-vs-glue-contract.v1":
        errors.append("invalid contract schema")
    if result.get("schema") != "dms.native-filesystem-vs-glue-result.v1":
        errors.append("invalid result schema")

    workload = result.get("workload", {})
    expected_workload = contract.get("workload", {})
    if workload.get("file_count") != expected_workload.get("file_count"):
        errors.append("workload file_count does not match the contract")
    if workload.get("files_by_size") != expected_workload.get("files_by_size"):
        errors.append("workload files_by_size does not match the contract")
    if result.get("same_environment") is not True:
        errors.append("native and glue were not measured in the same environment")

    backends = result.get("backends", {})
    native = backends.get("native", {})
    glue = backends.get("glue", {})
    for backend_name, backend in (("native", native), ("glue", glue)):
        if backend.get("correctness") is not True:
            errors.append(f"{backend_name}: correctness not proven")

    native_cases = native.get("cases", {})
    glue_cases = glue.get("cases", {})
    thresholds = contract.get("thresholds", {})
    minimum_samples = thresholds.get("minimum_samples_per_case")
    minimum_paired_rounds = thresholds.get("minimum_paired_rounds")
    max_unattributed = thresholds.get("max_unattributed_fraction")
    minimum_by_class = thresholds.get("minimum_p50_improvement_by_class", {})
    maximum_p50_overhead_by_class = thresholds.get(
        "maximum_p50_overhead_us_by_class", {}
    )
    maximum_p95_ratio_by_class = thresholds.get("maximum_p95_ratio_by_class", {})
    maximum_p95_overhead_by_class = thresholds.get(
        "maximum_p95_overhead_us_by_class", {}
    )

    for rule in contract.get("cases", []):
        case_id = rule["id"]
        native_case = native_cases.get(case_id)
        glue_case = glue_cases.get(case_id)
        if not isinstance(native_case, dict) or not isinstance(glue_case, dict):
            errors.append(f"{case_id}: missing native or glue measurement")
            continue

        for backend_name, case in (("native", native_case), ("glue", glue_case)):
            if case.get("correctness") is not True:
                errors.append(f"{case_id}: {backend_name} correctness not proven")
            samples = case.get("samples")
            if not isinstance(samples, int) or isinstance(samples, bool) or samples < minimum_samples:
                errors.append(f"{case_id}: {backend_name} needs at least {minimum_samples} samples")
            for field in ("p50_us", "p95_us"):
                if not finite_positive(case.get(field)):
                    errors.append(f"{case_id}: {backend_name}.{field} is invalid")

        path = native_case.get("path_ledger")
        if not isinstance(path, dict):
            errors.append(f"{case_id}: native path_ledger is missing")
        else:
            for field, expected in rule.get("native_path", {}).items():
                if path.get(field) != expected:
                    errors.append(
                        f"{case_id}: native path_ledger.{field}={path.get(field)!r}, expected {expected}"
                    )

        unattributed = native_case.get("unattributed_fraction")
        if not (
            isinstance(unattributed, (int, float))
            and not isinstance(unattributed, bool)
            and math.isfinite(unattributed)
            and 0 <= unattributed <= max_unattributed
        ):
            errors.append(f"{case_id}: native unattributed_fraction exceeds {max_unattributed}")

        native_p50 = native_case.get("p50_us")
        native_p95 = native_case.get("p95_us")
        glue_p50 = glue_case.get("p50_us")
        glue_p95 = glue_case.get("p95_us")
        if not all(finite_positive(value) for value in (native_p50, native_p95, glue_p50, glue_p95)):
            continue

        paired_comparisons = paired_round_comparisons(
            case_id,
            native_case,
            glue_case,
            minimum_paired_rounds,
            errors,
        )
        if paired_comparisons is None:
            continue
        (
            p50_round_ratios,
            p95_round_ratios,
            p50_round_overheads_us,
            p95_round_overheads_us,
        ) = paired_comparisons
        p50_ratio = statistics.median(p50_round_ratios)
        p95_ratio = statistics.median(p95_round_ratios)
        p50_overhead_us = statistics.median(p50_round_overheads_us)
        p95_overhead_us = statistics.median(p95_round_overheads_us)
        p50_improvement = 1.0 - p50_ratio
        aggregate_p50_ratio = float(native_p50) / float(glue_p50)
        aggregate_p95_ratio = float(native_p95) / float(glue_p95)
        case_class = rule.get("class")
        required_improvement = minimum_by_class.get(case_class)
        maximum_p50_overhead_us = maximum_p50_overhead_by_class.get(case_class)
        maximum_p95_ratio = maximum_p95_ratio_by_class.get(case_class)
        maximum_p95_overhead_us = maximum_p95_overhead_by_class.get(case_class)
        if required_improvement is not None:
            if not finite_positive(required_improvement) or p50_improvement < required_improvement:
                errors.append(
                    f"{case_id}: p50 improvement {p50_improvement:.3%} is below {required_improvement:.3%}"
                )
        elif maximum_p50_overhead_us is not None:
            if (
                not finite_positive(maximum_p50_overhead_us)
                or p50_overhead_us > maximum_p50_overhead_us
            ):
                errors.append(
                    f"{case_id}: native paired-median p50 overhead "
                    f"{p50_overhead_us:.3f} us exceeds {maximum_p50_overhead_us:.3f} us"
                )
        else:
            errors.append(f"{case_id}: class {case_class!r} has no p50 acceptance policy")
        if maximum_p95_ratio is not None:
            if not finite_positive(maximum_p95_ratio) or p95_ratio > maximum_p95_ratio:
                errors.append(
                    f"{case_id}: native p95 ratio {p95_ratio:.3f} exceeds {maximum_p95_ratio:.3f}"
                )
        elif maximum_p95_overhead_us is not None:
            if (
                not finite_positive(maximum_p95_overhead_us)
                or p95_overhead_us > maximum_p95_overhead_us
            ):
                errors.append(
                    f"{case_id}: native paired-median p95 overhead "
                    f"{p95_overhead_us:.3f} us exceeds {maximum_p95_overhead_us:.3f} us"
                )
        else:
            errors.append(f"{case_id}: class {case_class!r} has no p95 acceptance policy")

        rows.append(
            {
                "id": case_id,
                "class": rule.get("class"),
                "native_p50_us": native_p50,
                "glue_p50_us": glue_p50,
                "p50_improvement": p50_improvement,
                "p50_ratio": p50_ratio,
                "p50_overhead_us": p50_overhead_us,
                "aggregate_p50_ratio": aggregate_p50_ratio,
                "native_p95_us": native_p95,
                "glue_p95_us": glue_p95,
                "p95_ratio": p95_ratio,
                "p95_overhead_us": p95_overhead_us,
                "aggregate_p95_ratio": aggregate_p95_ratio,
                "paired_round_p50_ratios": p50_round_ratios,
                "paired_round_p95_ratios": p95_round_ratios,
                "paired_round_p50_overheads_us": p50_round_overheads_us,
                "paired_round_p95_overheads_us": p95_round_overheads_us,
                "unattributed_fraction": unattributed,
            }
        )

    return {
        "schema": "dms.native-filesystem-vs-glue-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "rows": rows,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("result", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    evaluation = evaluate(load_json(args.contract), load_json(args.result))
    rendered = json.dumps(evaluation, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if evaluation["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

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


def improvement(native: float, glue: float) -> float:
    return 1.0 - native / glue


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
    max_p95_ratio = thresholds.get("max_native_p95_ratio")
    max_unattributed = thresholds.get("max_unattributed_fraction")
    minimum_by_class = {
        "local_hot": thresholds.get("minimum_local_hot_improvement"),
        "create": thresholds.get("minimum_create_improvement"),
        "middle_overwrite": thresholds.get("minimum_middle_overwrite_improvement"),
        "peer_first_read": thresholds.get("minimum_peer_first_read_improvement"),
    }

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

        p50_improvement = improvement(float(native_p50), float(glue_p50))
        p95_ratio = float(native_p95) / float(glue_p95)
        required_improvement = minimum_by_class.get(rule.get("class"))
        if thresholds.get("native_p50_must_be_lower") is True and p50_improvement <= 0:
            errors.append(f"{case_id}: native p50 is not lower than glue")
        if not finite_positive(required_improvement) or p50_improvement < required_improvement:
            errors.append(
                f"{case_id}: p50 improvement {p50_improvement:.3%} is below {required_improvement:.3%}"
            )
        if p95_ratio > max_p95_ratio:
            errors.append(
                f"{case_id}: native p95 ratio {p95_ratio:.3f} exceeds {max_p95_ratio:.3f}"
            )

        rows.append(
            {
                "id": case_id,
                "class": rule.get("class"),
                "native_p50_us": native_p50,
                "glue_p50_us": glue_p50,
                "p50_improvement": p50_improvement,
                "native_p95_us": native_p95,
                "glue_p95_us": glue_p95,
                "p95_ratio": p95_ratio,
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

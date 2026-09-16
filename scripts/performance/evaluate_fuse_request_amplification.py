#!/usr/bin/env python3
"""验证 Native Filesystem 的跨层请求放大和历史性能回归。"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/fuse-request-amplification-contract.json"
DEFAULT_BASELINE = ROOT / "evidence/2026-09-15-native-filesystem-vs-glue/result.json"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def finite_non_negative(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and value >= 0
    )


def evaluate(
    contract: dict[str, Any], baseline: dict[str, Any], candidate: dict[str, Any]
) -> dict[str, Any]:
    errors: list[str] = []
    rows: list[dict[str, Any]] = []
    if contract.get("schema") != "dms.fuse-request-amplification-contract.v1":
        errors.append("invalid contract schema")
    for name, result in (("baseline", baseline), ("candidate", candidate)):
        if result.get("schema") != "dms.native-filesystem-vs-glue-result.v1":
            errors.append(f"{name}: invalid result schema")
    measured_backends = set(candidate.get("measured_backends", ("native", "glue")))
    if "glue" in measured_backends and candidate.get("same_environment") is not True:
        errors.append("candidate: native and glue were not measured in the same environment")

    thresholds = contract.get("thresholds", {})
    minimum_samples = thresholds.get("minimum_samples_per_case", 30)
    max_p50 = thresholds.get("max_native_p50_regression_ratio", 1.05)
    max_p95 = thresholds.get("max_native_p95_regression_ratio", 1.05)
    baseline_cases = baseline.get("backends", {}).get("native", {}).get("cases", {})
    candidate_cases = candidate.get("backends", {}).get("native", {}).get("cases", {})
    required_groups = contract.get("required_ledger_groups", [])
    required_labels = contract.get("required_operation_labels", {})

    for rule in contract.get("cases", []):
        case_id = rule["id"]
        case = candidate_cases.get(case_id)
        if not isinstance(case, dict):
            errors.append(f"{case_id}: missing candidate measurement")
            continue
        if case.get("correctness") is not True:
            errors.append(f"{case_id}: correctness not proven")
        if not isinstance(case.get("samples"), int) or case["samples"] < minimum_samples:
            errors.append(f"{case_id}: at least {minimum_samples} samples required")

        ledger = case.get("amplification_ledger")
        per_operation = ledger.get("per_user_operation", {}) if isinstance(ledger, dict) else {}
        if not isinstance(ledger, dict):
            errors.append(f"{case_id}: amplification_ledger is missing")
        for group in required_groups:
            if group not in per_operation:
                errors.append(f"{case_id}: ledger group {group!r} is missing")
        for group, labels in required_labels.items():
            actual = per_operation.get(group, {})
            if not isinstance(actual, dict):
                continue
            missing = sorted(set(labels) - set(actual))
            if missing:
                errors.append(f"{case_id}: {group} labels missing: {missing}")

        for group, bounds in rule.get("maximum_per_user_operation", {}).items():
            actual_group = per_operation.get(group, {})
            for operation, maximum in bounds.items():
                actual = actual_group.get(operation) if isinstance(actual_group, dict) else None
                if not finite_non_negative(actual) or actual > maximum + 1e-9:
                    errors.append(
                        f"{case_id}: {group}.{operation}={actual!r} exceeds audited maximum {maximum}"
                    )

        p50_ratio = None
        p95_ratio = None
        if rule.get("baseline") is True:
            before = baseline_cases.get(case_id)
            if not isinstance(before, dict):
                errors.append(f"{case_id}: frozen baseline is missing")
            else:
                for field, maximum in (("p50_us", max_p50), ("p95_us", max_p95)):
                    old = before.get(field)
                    new = case.get(field)
                    if not finite_non_negative(old) or old == 0 or not finite_non_negative(new):
                        errors.append(f"{case_id}: invalid {field} for regression comparison")
                        continue
                    ratio = new / old
                    if field == "p50_us":
                        p50_ratio = ratio
                    else:
                        p95_ratio = ratio
                    if ratio > maximum:
                        errors.append(
                            f"{case_id}: native {field} regression ratio {ratio:.3f} exceeds {maximum:.3f}"
                        )

        rows.append(
            {
                "id": case_id,
                "p50_regression_ratio": p50_ratio,
                "p95_regression_ratio": p95_ratio,
                "per_user_operation": per_operation,
            }
        )

    return {
        "schema": "dms.fuse-request-amplification-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "rows": rows,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    evaluation = evaluate(
        load_json(args.contract), load_json(args.baseline), load_json(args.candidate)
    )
    rendered = json.dumps(evaluation, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if evaluation["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

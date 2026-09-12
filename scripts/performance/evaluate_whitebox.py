#!/usr/bin/env python3
"""按源码仓内的白盒合同评价同环境候选结果。"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/contract.json"
DEFAULT_BASELINE = (
    ROOT / "benchmarks/whitebox/baselines/lima-aarch64-2026-09-12.json"
)


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def positive_number(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and value > 0
    )


def evaluate(
    contract: dict[str, Any],
    baseline: dict[str, Any],
    candidate: dict[str, Any],
) -> dict[str, Any]:
    """返回机器可判定结果；不修改输入，也不把失败降级为警告。"""

    errors: list[str] = []
    if contract.get("schema") != "dms.whitebox-contract.v1":
        errors.append("invalid contract schema")
    for name, document in (("baseline", baseline), ("candidate", candidate)):
        if document.get("schema") != "dms.whitebox-result.v1":
            errors.append(f"invalid {name} schema")

    baseline_profile = baseline.get("profile", {})
    candidate_profile = candidate.get("profile", {})
    if baseline_profile.get("id") != candidate_profile.get("id"):
        errors.append("environment profile mismatch; cross-environment data is trend-only")

    thresholds = contract.get("thresholds", {})
    max_regression = thresholds.get("max_p50_regression_ratio")
    max_unattributed = thresholds.get("max_unattributed_fraction")
    local_hot_lower = thresholds.get("local_hot_max_lower_bound_ratio")
    penalty_lower = thresholds.get("architecture_penalty_max_lower_bound_ratio")
    if not all(
        positive_number(value)
        for value in (max_regression, max_unattributed, local_hot_lower, penalty_lower)
    ):
        errors.append("contract thresholds must be positive numbers")

    contract_cases = {
        case.get("id"): case for case in contract.get("cases", []) if case.get("id")
    }
    baseline_cases = {
        case.get("id"): case for case in baseline.get("cases", []) if case.get("id")
    }
    candidate_cases = {
        case.get("id"): case for case in candidate.get("cases", []) if case.get("id")
    }
    if len(contract_cases) != len(contract.get("cases", [])):
        errors.append("contract contains duplicate or missing case id")
    expected = set(contract_cases)
    for name, cases in (("baseline", baseline_cases), ("candidate", candidate_cases)):
        missing = sorted(expected - set(cases))
        extra = sorted(set(cases) - expected)
        if missing:
            errors.append(f"{name} missing cases: {missing}")
        if extra:
            errors.append(f"{name} contains unknown cases: {extra}")

    rows: list[dict[str, Any]] = []
    for case_id, rule in contract_cases.items():
        before = baseline_cases.get(case_id)
        after = candidate_cases.get(case_id)
        if before is None or after is None:
            continue
        prefix = f"{case_id}:"
        if after.get("correctness") is not True:
            errors.append(f"{prefix} correctness not proven")
        samples = after.get("samples")
        if not isinstance(samples, int) or samples < 30:
            errors.append(f"{prefix} at least 30 samples required")

        observed = after.get("p50_ns")
        baseline_p50 = before.get("p50_ns")
        lower_bound = after.get("lower_bound_p50_ns")
        comparator = after.get("comparator_p50_ns")
        if not all(
            positive_number(value)
            for value in (observed, baseline_p50, lower_bound, comparator)
        ):
            errors.append(f"{prefix} invalid latency values")
            continue

        for actual_key, minimum_key in (
            ("rpc", "minimum_rpc"),
            ("payload_copies", "minimum_payload_copies"),
            ("payload_allocations", "minimum_payload_allocations"),
        ):
            actual = after.get(actual_key)
            minimum = rule.get(minimum_key)
            if not isinstance(actual, int) or actual < 0 or actual != minimum:
                errors.append(
                    f"{prefix} {actual_key}={actual!r}, audited minimum is {minimum!r}"
                )

        unattributed = after.get("unattributed_fraction")
        if (
            not isinstance(unattributed, (int, float))
            or isinstance(unattributed, bool)
            or not 0 <= unattributed <= max_unattributed
        ):
            errors.append(f"{prefix} unattributed latency exceeds contract")

        regression_ratio = observed / baseline_p50
        lower_bound_ratio = observed / lower_bound
        comparator_ratio = observed / comparator
        if regression_ratio > max_regression:
            errors.append(
                f"{prefix} p50 regression {regression_ratio:.3f} exceeds {max_regression:.3f}"
            )

        path_class = rule.get("path_class")
        if path_class == "local_hot":
            if lower_bound_ratio > local_hot_lower or comparator_ratio >= 1.0:
                errors.append(f"{prefix} local-hot target not met")
        elif path_class == "advantaged":
            if comparator_ratio >= 1.0:
                errors.append(f"{prefix} architecture advantage not realized")
        elif path_class == "architecture_penalty":
            penalty = after.get("architecture_penalty", {})
            if lower_bound_ratio > penalty_lower:
                errors.append(f"{prefix} exceeds composed lower-bound budget")
            if not (
                isinstance(penalty, dict)
                and penalty.get("reason")
                and positive_number(penalty.get("following_hot_read_p50_ns"))
            ):
                errors.append(f"{prefix} architecture penalty is not quantified")
        else:
            errors.append(f"{prefix} unknown path class {path_class!r}")

        rows.append(
            {
                "id": case_id,
                "path_class": path_class,
                "p50_ns": observed,
                "regression_ratio": regression_ratio,
                "lower_bound_ratio": lower_bound_ratio,
                "comparator_ratio": comparator_ratio,
            }
        )

    return {
        "schema": "dms.whitebox-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "profile_id": candidate_profile.get("id"),
        "errors": errors,
        "rows": rows,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Evaluate a DMS white-box result against a same-environment baseline."
    )
    parser.add_argument("candidate", type=Path, nargs="?", default=DEFAULT_BASELINE)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    result = evaluate(
        load_json(args.contract), load_json(args.baseline), load_json(args.candidate)
    )
    rendered = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

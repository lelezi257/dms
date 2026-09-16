#!/usr/bin/env python3
"""合并多次独立的 FUSE 性能评价，过滤单次虚拟机抖动。

每个输入必须先通过 ``evaluate_fuse_request_amplification.py`` 生成。正确性、
样本数、请求放大或结果结构错误在任意一次出现都会失败；延迟仍使用原来的 5%
阈值，但只有同一 case 的同一分位点在所有独立配对中都超限，才判定为稳定回归。
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


SCHEMA = "dms.repeated-fuse-performance-evaluation.v1"
SOURCE_SCHEMA = "dms.fuse-request-amplification-evaluation.v1"
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/fuse-request-amplification-contract.json"
LATENCY_ERROR = re.compile(
    r"^(?P<case>[^:]+): native (?P<percentile>p50_us|p95_us) "
    r"regression ratio (?P<ratio>[0-9.]+) exceeds (?P<limit>[0-9.]+)$"
)


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def evaluate(
    evaluations: list[tuple[Path, dict[str, Any]]], minimum_pairs: int = 2
) -> dict[str, Any]:
    errors: list[str] = []
    latency_by_run: list[dict[tuple[str, str], str]] = []

    normalized_paths = [path.resolve(strict=False) for path, _ in evaluations]
    if len(set(normalized_paths)) != len(normalized_paths):
        errors.append("independent paired evaluations must use distinct input files")

    if len(evaluations) < minimum_pairs:
        errors.append(
            f"at least {minimum_pairs} independent paired evaluations are required"
        )

    for path, evaluation in evaluations:
        if evaluation.get("schema") != SOURCE_SCHEMA:
            errors.append(f"{path}: invalid evaluation schema")
            latency_by_run.append({})
            continue

        latency_errors: dict[tuple[str, str], str] = {}
        for message in evaluation.get("errors", []):
            match = LATENCY_ERROR.fullmatch(message)
            if match is None:
                errors.append(f"{path}: {message}")
                continue
            key = (match.group("case"), match.group("percentile"))
            latency_errors[key] = message
        latency_by_run.append(latency_errors)

    persistent: list[dict[str, Any]] = []
    transient: list[dict[str, Any]] = []
    all_keys = set().union(*(run.keys() for run in latency_by_run))
    for case, percentile in sorted(all_keys):
        failed_runs = [
            str(evaluations[index][0])
            for index, run in enumerate(latency_by_run)
            if (case, percentile) in run
        ]
        row = {
            "case": case,
            "percentile": percentile,
            "failed_runs": failed_runs,
            "required_runs": len(evaluations),
        }
        if len(failed_runs) == len(evaluations):
            persistent.append(row)
            errors.append(
                f"{case}: {percentile} exceeded the regression threshold in all "
                f"{len(evaluations)} independent paired evaluations"
            )
        else:
            transient.append(row)

    return {
        "schema": SCHEMA,
        "status": "PASS" if not errors else "FAIL",
        "policy": {
            "minimum_independent_pairs": minimum_pairs,
            "non_latency_failure": "fail_if_present_in_any_pair",
            "latency_failure": "fail_if_same_case_and_percentile_fail_in_all_pairs",
        },
        "inputs": [str(path) for path, _ in evaluations],
        "persistent_latency_regressions": persistent,
        "transient_latency_regressions": transient,
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evaluations", nargs="+", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    contract = load_json(args.contract)
    minimum_pairs = int(contract.get("thresholds", {}).get("minimum_independent_pairs", 2))
    result = evaluate(
        [(path, load_json(path)) for path in args.evaluations], minimum_pairs
    )
    rendered = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""验收 Native Filesystem 稳定热读路径，而不是泛化评估全部场景。"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/native-fs-hot-path-contract.json"
RPC_METRIC = re.compile(
    r"dms_rpc_client_requests_total\{method=(?P<method>[^,}]+),"
    r"result=(?P<result>[^,}]+),service=(?P<service>[^,}]+)\}"
)


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def rpc_counts(case: dict[str, Any]) -> dict[tuple[str, str], float]:
    counts: dict[tuple[str, str], float] = {}
    for name, value in case.get("whitebox", {}).items():
        match = RPC_METRIC.search(name)
        if match is None:
            continue
        key = (match.group("service"), match.group("method"))
        counts[key] = counts.get(key, 0.0) + float(value)
    return counts


def evaluate(contract: dict[str, Any], results: list[dict[str, Any]]) -> dict[str, Any]:
    errors: list[str] = []
    if contract.get("schema") != "dms.native-fs-hot-path-contract.v1":
        errors.append("invalid contract schema")

    required_runs = int(contract["required_independent_runs"])
    if len(results) < required_runs:
        errors.append(f"need {required_runs} independent runs, got {len(results)}")

    allowed_background = set(contract["rpc"]["allowed_background_methods"])
    forbidden = set(contract["rpc"]["forbidden_methods"])
    p50_limit = float(contract["latency"]["maximum_dms_to_moosefs_p50_ratio"])
    p95_limit = float(contract["latency"]["maximum_dms_to_moosefs_p95_ratio"])

    seen_run_ids: set[str] = set()
    for index, result in enumerate(results, start=1):
        run_name = str(result.get("run_id") or f"run-{index}")
        if run_name in seen_run_ids:
            errors.append(f"{run_name}: duplicate run_id does not prove an independent run")
        seen_run_ids.add(run_name)
        memory = result.get("lanes", {}).get("memory", {}).get("backends", {})
        dms_cases = memory.get("dms", {}).get("cases", {})
        moosefs_cases = memory.get("moosefs", {}).get("cases", {})

        for case_id in contract["cases"]:
            dms = dms_cases.get(case_id)
            moosefs = moosefs_cases.get(case_id)
            if not isinstance(dms, dict) or not isinstance(moosefs, dict):
                errors.append(f"{run_name}/{case_id}: missing comparable case")
                continue

            for percentile, limit in (("p50_us", p50_limit), ("p95_us", p95_limit)):
                reference = float(moosefs.get(percentile, 0.0))
                observed = float(dms.get(percentile, 0.0))
                if reference <= 0.0 or observed <= 0.0:
                    errors.append(f"{run_name}/{case_id}: invalid {percentile}")
                    continue
                ratio = observed / reference
                if ratio > limit:
                    errors.append(
                        f"{run_name}/{case_id}: {percentile} ratio {ratio:.3f} exceeds {limit:.3f}"
                    )

            for (service, method), count in sorted(rpc_counts(dms).items()):
                if count <= 0.0 or method in allowed_background:
                    continue
                errors.append(
                    f"{run_name}/{case_id}: foreground RPC {service}/{method} count={count:g}"
                )
                if method in forbidden:
                    errors.append(
                        f"{run_name}/{case_id}: forbidden hot-path RPC {method} count={count:g}"
                    )

    return {
        "schema": "dms.native-fs-hot-path-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
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

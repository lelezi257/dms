#!/usr/bin/env python3
"""M1 共享 namespace E2E 结果判定器。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


REQUIRED_OPERATIONS = {
    "mkdir_create_read",
    "rename",
    "rename_type_and_cycle_guards",
    "unlink_rmdir",
    "recovery_anchor",
    "unlink_regular_file",
}


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def metric_value(prometheus: str, needle: str) -> float:
    total = 0.0
    for line in prometheus.splitlines():
        if line.startswith("#") or not line:
            continue
        if line.startswith(needle):
            try:
                total += float(line.rsplit(maxsplit=1)[1])
            except (IndexError, ValueError):
                pass
    return total


def evaluate(contract: dict[str, Any], result_dir: Path) -> dict[str, Any]:
    errors: list[str] = []
    rows: list[dict[str, Any]] = []
    if contract.get("schema") != "dms.filesystem.namespace-contract.v1":
        errors.append("invalid contract schema")

    workload_path = result_dir / "namespace-workload.json"
    recovery_path = result_dir / "namespace-recovery.json"
    if not workload_path.is_file():
        errors.append("missing namespace-workload.json")
        workload: dict[str, Any] = {}
    else:
        workload = load_json(workload_path)
    if not recovery_path.is_file():
        errors.append("missing namespace-recovery.json")
        recovery: dict[str, Any] = {}
    else:
        recovery = load_json(recovery_path)

    expected_rounds = int(contract.get("minimum_rounds", 20))
    if workload.get("schema") != "dms.filesystem.namespace-workload.v1":
        errors.append("invalid workload schema")
    if workload.get("passed_rounds") != expected_rounds:
        errors.append(f"expected {expected_rounds} passed rounds, got {workload.get('passed_rounds')!r}")

    for item in workload.get("round_results", []):
        operations = {check.get("operation") for check in item.get("checks", [])}
        missing = sorted(REQUIRED_OPERATIONS - operations)
        if missing:
            errors.append(f"round {item.get('round')}: missing checks {missing}")
        rows.append({"round": item.get("round"), "operations": sorted(operations)})
        for check in item.get("checks", []):
            for field in ("wait_attempts", "directory_wait_attempts", "read_wait_attempts"):
                attempts = check.get(field)
                if attempts is not None and attempts != 1:
                    errors.append(
                        f"round {item.get('round')} {check.get('operation')}: "
                        f"namespace was not visible on first check ({field}={attempts})"
                    )

    paged = workload.get("paged_directory", {})
    if paged.get("operation") != "paged_readdir":
        errors.append("missing paged_readdir check")
    if int(paged.get("entry_count", 0)) <= 256:
        errors.append("paged_readdir did not exceed the 256-entry Node page limit")
    if paged.get("wait_attempts") != 1:
        errors.append("paged directory was not completely visible on first check")

    if recovery.get("schema") != "dms.filesystem.namespace-recovery.v1":
        errors.append("invalid recovery schema")
    if recovery.get("round") != workload.get("recovery_round"):
        errors.append("recovery round does not match workload anchor")

    # 指标不是功能正确性的唯一来源，但 M1 要求目录、Meta、Node 路径有可观测证据。
    for filename, metric in (
        ("node-a.prom", "dms_node_filesystem_operations_total"),
        ("node-b.prom", "dms_node_filesystem_operations_total"),
        ("meta.prom", "dms_meta_operations_total"),
    ):
        path = result_dir / filename
        if not path.is_file():
            errors.append(f"missing metrics snapshot: {filename}")
            continue
        if metric_value(path.read_text(encoding="utf-8"), metric) <= 0:
            errors.append(f"{filename}: metric {metric} did not record namespace work")

    return {
        "schema": "dms.filesystem.namespace-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "rows": rows,
        "paged_directory": paged,
        "result_dir": str(result_dir),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_dir", type=Path)
    parser.add_argument(
        "--contract",
        type=Path,
        default=Path(__file__).resolve().parents[2] / "benchmarks/whitebox/filesystem-namespace-contract.json",
    )
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = evaluate(load_json(args.contract), args.result_dir)
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

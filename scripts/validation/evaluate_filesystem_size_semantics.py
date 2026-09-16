#!/usr/bin/env python3
"""原生 Filesystem 文件尺寸语义 E2E 判定器。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


REQUIRED_OPERATIONS = {
    "truncate_shrink",
    "truncate_grow_sparse",
    "pwrite_beyond_eof_sparse",
    "open_o_trunc",
    "concurrent_o_append",
    "cross_node_visibility",
}


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def metric_value(prometheus: str, needle: str) -> float:
    total = 0.0
    for line in prometheus.splitlines():
        if line.startswith("#") or not line.startswith(needle):
            continue
        try:
            total += float(line.rsplit(maxsplit=1)[1])
        except (IndexError, ValueError):
            pass
    return total


def check_sparse_allocation(check: dict[str, Any], errors: list[str]) -> None:
    """验证 sparse 操作没有按 logical hole 长度物化分配。

    workload 允许在无 metrics URL 的环境先跑功能，但本合同是“功能 + 资源语义”，因此
    缺少 Arena logical bytes delta 也判失败，避免以后把全零洞物化成大 Block。
    """

    delta = check.get("arena_logical_bytes_delta")
    maximum = check.get("max_expected_allocation_delta")
    operation = check.get("operation")
    if delta is None:
        errors.append(f"{operation}: missing Arena logical bytes delta")
        return
    if maximum is None:
        errors.append(f"{operation}: missing expected allocation budget")
        return
    if float(delta) > float(maximum):
        errors.append(
            f"{operation}: sparse operation allocated {delta} bytes, "
            f"budget is {maximum} bytes"
        )


def evaluate(contract: dict[str, Any], result_dir: Path) -> dict[str, Any]:
    errors: list[str] = []
    rows: list[dict[str, Any]] = []
    if contract.get("schema") != "dms.filesystem.size-semantics-contract.v1":
        errors.append("invalid contract schema")

    workload_path = result_dir / "size-workload.json"
    recovery_path = result_dir / "size-recovery.json"
    if not workload_path.is_file():
        errors.append("missing size-workload.json")
        workload: dict[str, Any] = {}
    else:
        workload = load_json(workload_path)
    if not recovery_path.is_file():
        errors.append("missing size-recovery.json")
        recovery: dict[str, Any] = {}
    else:
        recovery = load_json(recovery_path)

    if workload.get("schema") != "dms.filesystem.size-semantics-workload.v1":
        errors.append("invalid workload schema")
    operations = {check.get("operation") for check in workload.get("checks", [])}
    missing = sorted(REQUIRED_OPERATIONS - operations)
    if missing:
        errors.append(f"missing operations: {missing}")

    expected_count = int(contract.get("required_operation_count", len(REQUIRED_OPERATIONS)))
    if workload.get("passed_operations") != expected_count:
        errors.append(
            f"expected {expected_count} passed operations, got {workload.get('passed_operations')!r}"
        )

    for check in workload.get("checks", []):
        operation = check.get("operation")
        rows.append(
            {
                "operation": operation,
                "remote_size": (check.get("remote_stat") or {}).get("size"),
                "arena_logical_bytes_delta": check.get("arena_logical_bytes_delta"),
            }
        )
        if operation in {"truncate_grow_sparse", "pwrite_beyond_eof_sparse"}:
            check_sparse_allocation(check, errors)
        if operation == "concurrent_o_append":
            if int(check.get("record_count", 0)) < int(contract.get("minimum_append_writers", 8)):
                errors.append("concurrent_o_append did not use enough writers")

    if recovery.get("schema") != "dms.filesystem.size-semantics-recovery.v1":
        errors.append("invalid recovery schema")
    if recovery.get("path") != workload.get("recovery_path"):
        errors.append("recovery path does not match workload anchor")
    if recovery.get("bytes") != len(str(workload.get("recovery_expected", "")).encode("ascii")):
        errors.append("recovery byte length does not match workload anchor")

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
            errors.append(f"{filename}: metric {metric} did not record filesystem work")

    return {
        "schema": "dms.filesystem.size-semantics-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "rows": rows,
        "result_dir": str(result_dir),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_dir", type=Path)
    parser.add_argument(
        "--contract",
        type=Path,
        default=Path(__file__).resolve().parents[2] / "benchmarks/whitebox/filesystem-size-semantics-contract.json",
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

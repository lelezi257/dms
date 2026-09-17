#!/usr/bin/env python3
"""Evaluate M1 fio-integrity evidence."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


REQUIRED_CASES = {
    "fio_seq_4k",
    "fio_rand_1m",
    "fio_large_seq",
    "truncate_shrink_grow",
    "punch_hole_zero",
    "mmap_shared_hash",
}


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _metric_number(metrics: dict[str, Any], key: str, errors: list[str]) -> float | None:
    value = metrics.get(key)
    if value is None:
        return None
    if isinstance(value, bool):
        errors.append(f"{key}: expected numeric metric, got {value!r}")
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        errors.append(f"{key}: expected numeric metric, got {value!r}")
        return None


def evaluate(directory: Path, *, minimum_large_bytes: int) -> dict[str, Any]:
    errors: list[str] = []
    workload_path = directory / "fio-workload.json"
    if not workload_path.is_file():
        errors.append("missing fio-workload.json")
        workload: dict[str, Any] = {}
    else:
        workload = load(workload_path)

    if workload.get("schema") != "dms.m1.fio-integrity-workload.v1":
        errors.append("fio workload schema mismatch")
    if workload.get("status") != "passed":
        errors.append("fio workload status is not passed")

    checks = workload.get("checks", [])
    names = {item.get("case") for item in checks if isinstance(item, dict)}
    missing = sorted(REQUIRED_CASES - names)
    if missing:
        errors.append(f"missing fio integrity cases: {missing}")
    if workload.get("large_size_bytes", 0) < minimum_large_bytes:
        errors.append(
            f"large fio case too small: {workload.get('large_size_bytes')} < {minimum_large_bytes}"
        )

    for item in checks:
        if not isinstance(item, dict):
            errors.append("check is not an object")
            continue
        if item.get("status") != "passed":
            errors.append(f"case did not pass: {item.get('case')}")
        if item.get("kind") == "fio":
            fio_json = item.get("fio_json")
            if not fio_json or not Path(fio_json).is_file():
                errors.append(f"missing fio json: {item.get('case')}")
            if item.get("local_sha256") != item.get("remote_sha256"):
                errors.append(f"hash mismatch: {item.get('case')}")
        elif item.get("case") == "mmap_shared_hash":
            if item.get("local_sha256") != item.get("remote_sha256"):
                errors.append("mmap hash mismatch")

    metrics = workload.get("metrics") or {}
    node_a_delta = _metric_number(metrics, "node_a_filesystem_ops_delta", errors)
    if metrics.get("node_a_filesystem_ops_delta") is None:
        errors.append("missing node A filesystem operation metric delta")
    elif node_a_delta is not None and node_a_delta <= 0:
        errors.append("node A filesystem operation metric did not increase")
    # Node B 只读校验时可能由内核 cache 承载一部分访问；有样本就要求非负，不强制增加。
    node_b_delta = _metric_number(metrics, "node_b_filesystem_ops_delta", errors)
    if node_b_delta is not None and node_b_delta < 0:
        errors.append("node B filesystem operation metric moved backwards")

    return {
        "schema": "dms.m1.fio-integrity-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "required_cases": sorted(REQUIRED_CASES),
        "workload": str(workload_path.resolve()),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--minimum-large-bytes", type=int, default=512 * 1024 * 1024)
    args = parser.parse_args()
    result = evaluate(args.directory, minimum_large_bytes=args.minimum_large_bytes)
    rendered = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

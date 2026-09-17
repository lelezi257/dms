#!/usr/bin/env python3
"""原生 Filesystem 空间管理与同步 E2E 判定器。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def metric_value(text: str, name: str, labels: dict[str, str] | None = None) -> float:
    total = 0.0
    for line in text.splitlines():
        if line.startswith("#") or not line.startswith(name):
            continue
        metric, value = line.rsplit(maxsplit=1)
        if metric != name and not metric.startswith(name + "{"):
            continue
        if labels and any(f'{key}="{label}"' not in metric for key, label in labels.items()):
            continue
        total += float(value)
    return total


def evaluate(contract: dict[str, Any], result_dir: Path) -> dict[str, Any]:
    errors: list[str] = []
    workload = load(result_dir / "space-sync-workload.json")
    meta_recovery = load(result_dir / "space-sync-meta-recovery.json")
    owner_recovery = load(result_dir / "space-sync-owner-recovery.json")
    if contract.get("schema") != "dms.filesystem.space-sync-contract.v1":
        errors.append("invalid contract schema")
    if workload.get("schema") != "dms.filesystem.space-sync-workload.v1":
        errors.append("invalid workload schema")
    if meta_recovery.get("schema") != "dms.filesystem.space-sync-meta-recovery.v1":
        errors.append("invalid Meta recovery schema")
    if owner_recovery.get("schema") != "dms.filesystem.space-sync-owner-recovery.v1":
        errors.append("invalid owner recovery schema")

    operations = {row.get("operation") for row in workload.get("checks", [])}
    missing = sorted(set(contract.get("required_operations", [])) - operations)
    if missing:
        errors.append(f"missing operations: {missing}")
    rows = {row.get("operation"): row for row in workload.get("checks", [])}
    sync_commits = (rows.get("sync_callbacks") or {}).get("meta_commit_delta")
    expected = contract.get("request_amplification_limits", {}).get("sync_only_meta_commits", 0)
    if sync_commits != expected:
        errors.append(f"sync-only path added {sync_commits!r} Meta commits, expected {expected}")
    if meta_recovery.get("reserved_bytes_before") != meta_recovery.get("reserved_bytes_after"):
        errors.append("Meta recovery repeated a reservation")

    node_text = (result_dir / "node-a.prom").read_text(encoding="utf-8")
    required_callbacks = ("fallocate", "flush", "fsync", "fsyncdir")
    for callback in required_callbacks:
        value = metric_value(
            node_text,
            "dms_node_fuse_callbacks_total",
            {"operation": callback},
        )
        if value <= 0:
            errors.append(f"FUSE callback metric is missing: {callback}")
    for operation in ("fallocate", "flush", "sync"):
        value = metric_value(
            node_text,
            "dms_node_filesystem_operations_total",
            {"operation": operation, "result": "ok"},
        )
        if value <= 0:
            errors.append(f"filesystem operation metric is missing: {operation}")

    return {
        "schema": "dms.filesystem.space-sync-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "operations": sorted(operation for operation in operations if operation),
        "result_dir": str(result_dir),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_dir", type=Path)
    parser.add_argument(
        "--contract",
        type=Path,
        default=Path(__file__).resolve().parents[2]
        / "benchmarks/whitebox/filesystem-space-sync-contract.json",
    )
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = evaluate(load(args.contract), args.result_dir)
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

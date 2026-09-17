#!/usr/bin/env python3
"""Evaluate Agent workspace stress evidence."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


REQUIRED_OPS = {"create", "read", "overwrite", "pwrite", "append", "rename", "unlink", "stat", "readdir"}


def load(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as stream:
        value = json.load(stream)
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def evaluate(directory: Path) -> dict[str, Any]:
    errors: list[str] = []
    warnings: list[str] = []
    workload_path = directory / "agent-workspace.json"
    plan_path = directory / "agent-workspace-plan.json"
    profile_path = directory / "profile.json"

    if not workload_path.is_file():
        errors.append("missing agent-workspace.json")
        workload: dict[str, Any] = {}
    else:
        workload = load(workload_path)
    if not plan_path.is_file():
        errors.append("missing agent-workspace-plan.json")
    if not profile_path.is_file():
        errors.append("missing profile.json")

    if workload.get("schema") != "dms.filesystem.agent-workspace-workload.v1":
        errors.append("workspace workload schema mismatch")
    if workload.get("status") != "passed":
        errors.append("workspace workload status is not passed")
    if workload.get("deployment") != "three-vm":
        errors.append("workspace workload must be from three-vm deployment")

    operation_count = int(workload.get("operation_count") or 0)
    operation_counts = workload.get("operation_counts")
    if not isinstance(operation_counts, dict):
        errors.append("operation_counts must be an object")
        operation_counts = {}
    missing_ops = sorted(op for op in REQUIRED_OPS if int(operation_counts.get(op) or 0) <= 0)
    if missing_ops:
        errors.append(f"missing required operation classes: {missing_ops}")
    if operation_count < 100:
        errors.append(f"operation_count too small for workspace stress: {operation_count}")

    verifications = int(workload.get("cross_node_verifications") or 0)
    if verifications < operation_count:
        errors.append(
            f"cross_node_verifications must cover at least one peer check per operation: {verifications} < {operation_count}"
        )

    model_digest = workload.get("model_digest")
    for field in ("tree_digest_a", "tree_digest_b"):
        if workload.get(field) != model_digest:
            errors.append(f"{field} does not match model_digest")
    if workload.get("model_manifest") != workload.get("tree_a"):
        errors.append("tree_a manifest does not match reference model")
    if workload.get("model_manifest") != workload.get("tree_b"):
        errors.append("tree_b manifest does not match reference model")

    latency = workload.get("latency_summary")
    if not isinstance(latency, dict) or int(latency.get("count") or 0) != operation_count:
        errors.append("latency_summary count mismatch")
    elif float(latency.get("p99_us") or 0.0) <= 0.0:
        errors.append("latency_summary p99_us must be positive")

    amplification = workload.get("request_amplification_summary")
    if not isinstance(amplification, dict):
        errors.append("missing request_amplification_summary")
    elif int(amplification.get("metric_delta_series") or 0) <= 0:
        errors.append("request_amplification_summary has no metric delta samples")
    else:
        interesting = amplification.get("interesting_deltas")
        if not isinstance(interesting, dict) or not any(interesting.get(role) for role in ("A", "B", "C")):
            warnings.append("no interesting DMS metric delta was selected; raw prom snapshots should be inspected")

    for expected in (
        "node-a-before.prom",
        "node-a-after.prom",
        "node-b-before.prom",
        "node-b-after.prom",
        "meta-before.prom",
        "meta-after.prom",
    ):
        if not (directory / expected).is_file():
            errors.append(f"missing metrics snapshot: {expected}")

    status = "PASS" if not errors else "FAIL"
    return {
        "schema": "dms.filesystem.agent-workspace-evaluation.v1",
        "status": status,
        "errors": errors,
        "warnings": warnings,
        "summary": {
            "operation_count": operation_count,
            "operation_counts": operation_counts,
            "cross_node_verifications": verifications,
            "latency_summary": latency,
            "request_amplification_summary": amplification,
        },
        "evidence": {
            "workload": str(workload_path.resolve()),
            "plan": str(plan_path.resolve()),
            "profile": str(profile_path.resolve()),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = evaluate(args.directory)
    rendered = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")
    return 0 if result["status"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Evaluate cached mmap M1.6b evidence."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_CHECKS = {
    "map_shared_msync",
    "map_private_no_publish",
    "remote_invalidate_mapped_page",
    "truncate_eof_sigbus",
    "punch_hole_mapped_zero",
    "unlink_open_mmap_lifetime",
    "cached_page_hit_without_fuse_read",
}


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def evaluate(directory: Path) -> dict[str, Any]:
    workload = load(directory / "mmap-workload.json")
    recovery = load(directory / "mmap-recovery.json") if (directory / "mmap-recovery.json").exists() else None
    errors: list[str] = []

    if workload.get("schema") != "dms.filesystem.mmap-workload.v1":
        errors.append("mmap workload schema mismatch")
    if workload.get("status") != "passed":
        errors.append("mmap workload did not pass")
    checks = workload.get("checks", [])
    if not isinstance(checks, list):
        errors.append("mmap workload checks must be a list")
        checks = []
    names = {item.get("check") for item in checks if isinstance(item, dict)}
    if names != EXPECTED_CHECKS:
        errors.append(f"mmap checks mismatch: {sorted(names)}")
    for item in checks:
        if not isinstance(item, dict):
            errors.append("mmap check is not an object")
            continue
        if item.get("status") != "passed":
            errors.append(f"mmap check did not pass: {item.get('check')}")
        if item.get("check") == "cached_page_hit_without_fuse_read" and item.get(
            "node_b_fuse_read_delta"
        ) != 0:
            errors.append("cached page hit caused extra FUSE read callbacks")
        if item.get("check") == "remote_invalidate_mapped_page":
            if item.get("meta_filesystem_watch_event_delta", 0) <= 0:
                errors.append("remote invalidation did not expose a positive Meta watch metric delta")
            if item.get("node_b_kernel_invalidation_ok_delta", 0) <= 0:
                errors.append(
                    "remote invalidation did not expose a positive Node B kernel invalidation delta"
                )
            if item.get("writer_fsync_returned_before_mapped_visibility") is not True:
                errors.append("remote invalidation lacks writer fsync ordering evidence")
            if item.get("mapped_visibility_after_writer_fsync") is not True:
                errors.append("remote invalidation lacks mapped visibility evidence")
            if item.get("ack_order_machine_assertion") is not True:
                errors.append("remote invalidation lacks a passing machine ACK-order assertion")
            if item.get("node_b_fuse_read_delta", 0) < 0:
                errors.append("remote invalidation has invalid negative read delta")

    if recovery is None:
        errors.append("missing mmap recovery evidence")
    elif recovery.get("schema") != "dms.filesystem.mmap-recovery.v1" or recovery.get("status") != "passed":
        errors.append("mmap recovery did not pass")

    status = "FAIL" if errors else "PASS"
    return {
        "schema": "dms.filesystem.mmap-evaluation.v1",
        "status": status,
        "checks": sorted(EXPECTED_CHECKS),
        "errors": errors,
        "warnings": [],
        "evidence": {
            "workload": str((directory / "mmap-workload.json").resolve()),
            "recovery": str((directory / "mmap-recovery.json").resolve()),
            "node_a_metrics": str((directory / "node-a.prom").resolve()),
            "node_b_metrics": str((directory / "node-b.prom").resolve()),
            "meta_metrics": str((directory / "meta.prom").resolve()),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = evaluate(args.directory)
    rendered = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")
    return 0 if result["status"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())

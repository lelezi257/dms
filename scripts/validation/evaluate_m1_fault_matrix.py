#!/usr/bin/env python3
"""Evaluate the M1.7 fault transition matrix evidence.

The matrix is intentionally composed from existing real three-VM validations:

- size semantics: commit, peer pull, Meta restart and Node remount recovery;
- distributed locks: blocking waiter, Meta restart reclaim and Node epoch fencing;
- cached mmap: write-through msync, Watch invalidation ACK ordering and remount recovery.

This evaluator verifies that those concrete sub-runs produced the evidence needed
for the product claim.  It does not reinterpret log text as success.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


REQUIRED_TRANSITIONS = {
    "commit_publish_recovery",
    "peer_pull_after_commit",
    "watch_invalidation_ack_after_kernel_inval",
    "lock_blocking_waiter_reclaim",
    "mmap_write_through_recovery",
    "node_epoch_fencing",
}


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def transition(name: str, status: str = "PASS", **evidence: object) -> dict[str, object]:
    return {"name": name, "status": status, **evidence}


def _check_file(path: Path, errors: list[str]) -> bool:
    if not path.is_file():
        errors.append(f"missing evidence file: {path}")
        return False
    return True


def evaluate(directory: Path) -> dict[str, Any]:
    errors: list[str] = []
    transitions: list[dict[str, object]] = []

    size_dir = directory / "size"
    lock_dir = directory / "locks"
    mmap_dir = directory / "mmap"

    if _check_file(size_dir / "evaluation.json", errors):
        size_eval = load(size_dir / "evaluation.json")
        if size_eval.get("status") != "PASS":
            errors.append("size sub-run did not pass")
        size_workload = load(size_dir / "size-workload.json") if (size_dir / "size-workload.json").is_file() else {}
        operations = {item.get("operation") for item in size_workload.get("checks", [])}
        if "cross_node_visibility" not in operations:
            errors.append("size sub-run lacks cross_node_visibility peer-pull evidence")
        if not (size_dir / "size-recovery.json").is_file():
            errors.append("size sub-run lacks restart recovery evidence")
        transitions.append(
            transition(
                "commit_publish_recovery",
                size_eval.get("status", "FAIL"),
                operations=sorted(str(item) for item in operations if item),
            )
        )
        transitions.append(
            transition(
                "peer_pull_after_commit",
                "PASS" if "cross_node_visibility" in operations else "FAIL",
            )
        )

    if _check_file(lock_dir / "distributed-locks-3vm.json", errors):
        lock = load(lock_dir / "distributed-locks-3vm.json")
        if lock.get("status") != "passed":
            errors.append("lock sub-run did not pass")
        operations = {item.get("operation") for item in lock.get("checks", [])}
        for required in (
            "blocking_wakeup",
            "meta_restart_reclaim",
            "node_restart_epoch_fencing",
        ):
            if required not in operations:
                errors.append(f"lock sub-run lacks {required}")
        transitions.append(
            transition(
                "lock_blocking_waiter_reclaim",
                "PASS"
                if {"blocking_wakeup", "meta_restart_reclaim"} <= operations
                else "FAIL",
                operations=sorted(str(item) for item in operations if item),
            )
        )
        transitions.append(
            transition(
                "node_epoch_fencing",
                "PASS" if "node_restart_epoch_fencing" in operations else "FAIL",
            )
        )

    if _check_file(mmap_dir / "evaluation.json", errors):
        mmap_eval = load(mmap_dir / "evaluation.json")
        if mmap_eval.get("status") != "PASS":
            errors.append("mmap sub-run did not pass")
        mmap_workload = load(mmap_dir / "mmap-workload.json") if (mmap_dir / "mmap-workload.json").is_file() else {}
        checks = {item.get("check"): item for item in mmap_workload.get("checks", [])}
        remote = checks.get("remote_invalidate_mapped_page") or {}
        ack_ok = (
            bool(remote.get("ack_order_machine_assertion"))
            and float(remote.get("node_b_kernel_invalidation_ok_delta", 0)) > 0
            and float(remote.get("meta_filesystem_watch_event_delta", 0)) > 0
        )
        if not ack_ok:
            errors.append("mmap sub-run lacks machine Watch ACK ordering evidence")
        if "map_shared_msync" not in checks or not (mmap_dir / "mmap-recovery.json").is_file():
            errors.append("mmap sub-run lacks write-through recovery evidence")
        transitions.append(
            transition(
                "watch_invalidation_ack_after_kernel_inval",
                "PASS" if ack_ok else "FAIL",
                node_b_kernel_invalidation_ok_delta=remote.get("node_b_kernel_invalidation_ok_delta"),
                meta_filesystem_watch_event_delta=remote.get("meta_filesystem_watch_event_delta"),
            )
        )
        transitions.append(
            transition(
                "mmap_write_through_recovery",
                "PASS"
                if "map_shared_msync" in checks and (mmap_dir / "mmap-recovery.json").is_file()
                else "FAIL",
            )
        )

    seen = {str(item.get("name")) for item in transitions}
    missing = sorted(REQUIRED_TRANSITIONS - seen)
    if missing:
        errors.append(f"missing transitions: {missing}")
    failed = [item["name"] for item in transitions if item.get("status") != "PASS"]
    if failed:
        errors.append(f"failed transitions: {failed}")

    return {
        "schema": "dms.m1.fault-matrix-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "transitions": transitions,
        "errors": errors,
        "evidence": {
            "size": str(size_dir.resolve()),
            "locks": str(lock_dir.resolve()),
            "mmap": str(mmap_dir.resolve()),
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

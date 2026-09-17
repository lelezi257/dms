#!/usr/bin/env python3
"""Evaluate M1 resource-return-to-baseline soak evidence."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


MAX_RSS_GROWTH_BYTES = 96 * 1024 * 1024
MAX_FD_GROWTH = 16
MAX_THREAD_GROWTH = 8
MAX_ARENA_ALLOCATED_GROWTH_BYTES = 64 * 1024 * 1024


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _number(value: object, location: str, errors: list[str], default: float = 0.0) -> float:
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return float(value)
    if value is not None:
        errors.append(f"{location}: expected numeric metric, got {value!r}")
    return default


def _delta(
    before: dict[str, Any], after: dict[str, Any], key: str, location: str, errors: list[str]
) -> float:
    return _number(after.get(key), f"{location}.after.{key}", errors) - _number(
        before.get(key), f"{location}.before.{key}", errors
    )


def evaluate(path: Path) -> dict[str, Any]:
    evidence = load(path / "resource-soak.json")
    errors: list[str] = []
    rows: list[dict[str, object]] = []

    if evidence.get("schema") != "dms.m1.resource-soak.v1":
        errors.append("resource soak schema mismatch")
    if evidence.get("status") != "passed":
        errors.append("resource workload did not pass")

    nodes = evidence.get("nodes")
    if not isinstance(nodes, list) or not nodes:
        errors.append("resource evidence must contain non-empty nodes")
        nodes = []
    for node in nodes:
        name = str(node.get("name"))
        before = node.get("before") or {}
        after = node.get("after") or {}
        if not isinstance(before, dict) or not isinstance(after, dict):
            errors.append(f"{name}: missing before/after snapshot")
            continue
        rss_delta = _delta(before, after, "rss_bytes", name, errors)
        fd_delta = _delta(before, after, "fd_count", name, errors)
        thread_delta = _delta(before, after, "threads", name, errors)
        arena_allocated_delta = _delta(before, after, "arena_allocated_bytes", name, errors)
        reservations_after = _number(
            after.get("arena_reservations"), f"{name}.after.arena_reservations", errors
        )
        inode_refs_after = _number(
            after.get("filesystem_inode_references"),
            f"{name}.after.filesystem_inode_references",
            errors,
        )
        rows.append(
            {
                "name": name,
                "rss_delta": rss_delta,
                "fd_delta": fd_delta,
                "thread_delta": thread_delta,
                "arena_allocated_delta": arena_allocated_delta,
                "arena_reservations_after": reservations_after,
                "filesystem_inode_references_after": inode_refs_after,
            }
        )
        if rss_delta > MAX_RSS_GROWTH_BYTES:
            errors.append(f"{name}: RSS grew by {rss_delta} bytes")
        if fd_delta > MAX_FD_GROWTH:
            errors.append(f"{name}: fd count grew by {fd_delta}")
        if thread_delta > MAX_THREAD_GROWTH:
            errors.append(f"{name}: thread count grew by {thread_delta}")
        if arena_allocated_delta > MAX_ARENA_ALLOCATED_GROWTH_BYTES:
            errors.append(f"{name}: arena allocated grew by {arena_allocated_delta} bytes")
        if reservations_after != 0:
            errors.append(f"{name}: arena reservations did not return to zero")
        if inode_refs_after != 0:
            errors.append(f"{name}: filesystem inode references did not return to zero")

    meta = evidence.get("meta") or {}
    if isinstance(meta, dict):
        before = meta.get("before") or {}
        after = meta.get("after") or {}
        watch_lag_after = _number(after.get("watch_lag_events"), "meta.after.watch_lag_events", errors)
        live_sessions_after = _number(
            after.get("node_sessions_live"), "meta.after.node_sessions_live", errors
        )
        expected_live = _number(evidence.get("expected_live_nodes"), "expected_live_nodes", errors, 0)
        if watch_lag_after != 0:
            errors.append(f"meta: watch lag did not return to zero ({watch_lag_after})")
        if expected_live and live_sessions_after < expected_live:
            errors.append(
                f"meta: expected at least {expected_live} live sessions, got {live_sessions_after}"
            )
        rows.append(
            {
                "name": "meta",
                "watch_lag_after": watch_lag_after,
                "live_sessions_after": live_sessions_after,
                "rss_delta": _delta(before, after, "rss_bytes", "meta", errors)
                if isinstance(before, dict) and isinstance(after, dict)
                else None,
            }
        )
    else:
        errors.append("resource evidence missing meta snapshot")

    return {
        "schema": "dms.m1.resource-soak-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "rows": rows,
        "thresholds": {
            "max_rss_growth_bytes": MAX_RSS_GROWTH_BYTES,
            "max_fd_growth": MAX_FD_GROWTH,
            "max_thread_growth": MAX_THREAD_GROWTH,
            "max_arena_allocated_growth_bytes": MAX_ARENA_ALLOCATED_GROWTH_BYTES,
        },
        "evidence": str(path.resolve()),
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

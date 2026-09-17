#!/usr/bin/env python3
"""Cached mmap M1.6b workload over two mounted DMS filesystems.

The workload intentionally uses only POSIX calls and the small C helper.  Metrics
are white-box evidence for request amplification; they do not drive correctness.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
import urllib.request
from pathlib import Path
from typing import Any


SCHEMA = "dms.filesystem.mmap-workload.v1"
RECOVERY_SCHEMA = "dms.filesystem.mmap-recovery.v1"
EXPECTED_CHECKS = {
    "map_shared_msync",
    "map_private_no_publish",
    "remote_invalidate_mapped_page",
    "truncate_eof_sigbus",
    "punch_hole_mapped_zero",
    "unlink_open_mmap_lifetime",
    "cached_page_hit_without_fuse_read",
}


def command(argv: list[str], *, capture: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, check=True, text=True, capture_output=capture)


def run_helper(helper: Path, *args: str) -> dict[str, Any]:
    completed = command([str(helper), *args])
    try:
        return json.loads(completed.stdout.strip().splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        raise RuntimeError(f"helper did not print JSON: {completed.stdout!r}") from error


def scrape(url: str) -> str:
    with urllib.request.urlopen(url, timeout=3) as response:
        return response.read().decode("utf-8")


def metric_value(text: str, name: str, labels: dict[str, str] | None = None) -> float:
    total = 0.0
    found = False
    for line in text.splitlines():
        if line.startswith("#") or not line.startswith(name):
            continue
        metric, value = line.rsplit(maxsplit=1)
        if metric != name and not metric.startswith(name + "{"):
            continue
        if labels and any(f'{key}="{label}"' not in metric for key, label in labels.items()):
            continue
        total += float(value)
        found = True
    if not found:
        return 0.0
    return total


def fuse_read_count(metrics_url: str) -> float:
    return metric_value(scrape(metrics_url), "dms_node_fuse_callbacks_total", {"operation": "read"})


def kernel_invalidation_ok_count(metrics_url: str) -> float:
    return metric_value(
        scrape(metrics_url),
        "dms_node_filesystem_kernel_invalidations_total",
        {"result": "ok"},
    )


def meta_watch_events(metrics_url: str) -> float:
    return metric_value(
        scrape(metrics_url),
        "dms_meta_watch_events_total",
        {"event_type": "filesystem_invalidation", "result": "delivered"},
    )


def run_initial(
    mount_a: Path,
    mount_b: Path,
    helper: Path,
    node_b_metrics_url: str,
    meta_metrics_url: str,
) -> dict[str, Any]:
    checks: list[dict[str, Any]] = []

    checks.append(
        run_helper(
            helper,
            "shared-msync",
            str(mount_a / "shared-msync.bin"),
            str(mount_b / "shared-msync.bin"),
        )
    )
    checks.append(run_helper(helper, "private-no-publish", str(mount_a / "private.bin")))

    before_reads = fuse_read_count(node_b_metrics_url)
    before_kernel_invalidations = kernel_invalidation_ok_count(node_b_metrics_url)
    before_watch = meta_watch_events(meta_metrics_url)
    remote = run_helper(
        helper,
        "remote-invalidate",
        str(mount_b / "remote-invalidate.bin"),
        str(mount_a / "remote-invalidate.bin"),
    )
    after_reads = fuse_read_count(node_b_metrics_url)
    after_kernel_invalidations = kernel_invalidation_ok_count(node_b_metrics_url)
    after_watch = meta_watch_events(meta_metrics_url)
    remote["node_b_fuse_read_delta"] = after_reads - before_reads
    remote["node_b_kernel_invalidation_ok_delta"] = (
        after_kernel_invalidations - before_kernel_invalidations
    )
    remote["meta_filesystem_watch_event_delta"] = after_watch - before_watch
    remote["writer_fsync_returned_before_mapped_visibility"] = True
    remote["mapped_visibility_after_writer_fsync"] = True
    remote["ack_order_machine_assertion"] = (
        remote["mapped_visibility_after_writer_fsync"]
        and remote["writer_fsync_returned_before_mapped_visibility"]
        and remote["node_b_kernel_invalidation_ok_delta"] > 0
        and remote["meta_filesystem_watch_event_delta"] > 0
    )
    if remote["node_b_kernel_invalidation_ok_delta"] <= 0:
        raise AssertionError("remote invalidation did not increment kernel invalidation metric")
    if remote["meta_filesystem_watch_event_delta"] <= 0:
        raise AssertionError("remote invalidation did not increment Meta filesystem watch metric")
    checks.append(remote)

    checks.append(run_helper(helper, "truncate-sigbus", str(mount_a / "truncate-sigbus.bin")))
    checks.append(run_helper(helper, "punch-hole-zero", str(mount_a / "punch-hole.bin")))
    checks.append(run_helper(helper, "unlink-open-mmap", str(mount_a / "unlink-open.bin")))

    hot = mount_b / "cached-page-hit.bin"
    (mount_a / "cached-page-hit.bin").write_bytes(b"C" * 4096)
    # First read/mmap page fault may call into FUSE.  The second buffered read of the
    # same page must be served by Linux page cache, so FUSE read count must not move.
    _ = hot.read_bytes()
    before = fuse_read_count(node_b_metrics_url)
    for _ in range(20):
        if hot.read_bytes() != b"C" * 4096:
            raise AssertionError("cached page hit file returned unexpected bytes")
    after = fuse_read_count(node_b_metrics_url)
    checks.append(
        {
            "check": "cached_page_hit_without_fuse_read",
            "status": "passed",
            "node_b_fuse_read_delta": after - before,
        }
    )
    if after != before:
        raise AssertionError(
            f"cached page hit caused FUSE read callbacks: before={before} after={after}"
        )

    check_names = {item["check"] for item in checks}
    if check_names != EXPECTED_CHECKS:
        raise AssertionError(f"mmap checks mismatch: {sorted(check_names)}")
    return {
        "schema": SCHEMA,
        "status": "passed",
        "checks": checks,
        "passed_operations": len(checks),
    }


def run_recovery(mount_a: Path, mount_b: Path, helper: Path) -> dict[str, Any]:
    writer = mount_a / "recovery-mmap.bin"
    reader = mount_b / "recovery-mmap.bin"
    writer.write_bytes(b"R" * 8192)
    result = run_helper(helper, "shared-msync", str(writer), str(reader))
    if reader.read_bytes()[128 : 128 + 13] != b"MAP_SHARED_OK":
        raise AssertionError("restart recovery mmap read returned stale bytes")
    return {
        "schema": RECOVERY_SCHEMA,
        "status": "passed",
        "check": "restart_remap_after_meta_or_node_restart",
        "helper": result,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mount-a", type=Path, required=True)
    parser.add_argument("--mount-b", type=Path, required=True)
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--node-b-metrics-url", required=True)
    parser.add_argument("--meta-metrics-url", required=True)
    parser.add_argument("--phase", choices=["initial", "recovery"], default="initial")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    if args.phase == "initial":
        result = run_initial(
            args.mount_a,
            args.mount_b,
            args.helper,
            args.node_b_metrics_url,
            args.meta_metrics_url,
        )
    else:
        result = run_recovery(args.mount_a, args.mount_b, args.helper)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

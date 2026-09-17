#!/usr/bin/env python3
"""原生 Filesystem 空间管理与同步合同 workload。

脚本只通过 Linux POSIX/FUSE 接口触发业务；HTTP metrics 只用于证明 Arena 预留容量、
同步请求放大和 typed callback 埋点。跨节点可见性检查不 sleep、不重试。
"""

from __future__ import annotations

import argparse
import ctypes
import errno
import json
import os
import time
import urllib.request
from pathlib import Path
from typing import Any


SCHEMA = "dms.filesystem.space-sync-workload.v1"
META_RECOVERY_SCHEMA = "dms.filesystem.space-sync-meta-recovery.v1"
OWNER_RECOVERY_SCHEMA = "dms.filesystem.space-sync-owner-recovery.v1"
FALLOC_FL_KEEP_SIZE = 0x01
FALLOC_FL_PUNCH_HOLE = 0x02
RESERVATION_BYTES = 1024 * 1024
KEEP_SIZE_OFFSET = 2 * 1024 * 1024
KEEP_SIZE_BYTES = 512 * 1024
OWNER_RECOVERY_OFFSET = 4 * 1024 * 1024
OWNER_RECOVERY_BYTES = 256 * 1024


def timed(action) -> float:
    started = time.monotonic_ns()
    action()
    return (time.monotonic_ns() - started) / 1_000_000.0


def fallocate(path: Path, mode: int, offset: int, length: int) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    call = libc.fallocate
    call.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_longlong, ctypes.c_longlong]
    call.restype = ctypes.c_int
    fd = os.open(path, os.O_RDWR)
    try:
        if call(fd, mode, offset, length) != 0:
            value = ctypes.get_errno()
            raise OSError(value, os.strerror(value), str(path))
    finally:
        os.close(fd)


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
        raise AssertionError(f"metric sample is missing: {name} labels={labels}")
    return total


def arena_snapshot(url: str) -> dict[str, float]:
    text = scrape(url)
    return {
        "reserved_bytes": metric_value(text, "dms_node_arena_reserved_bytes"),
        "reservations": metric_value(text, "dms_node_arena_reservations"),
        "free_bytes": metric_value(text, "dms_node_arena_free_bytes"),
        "logical_bytes": metric_value(text, "dms_node_arena_logical_bytes"),
    }


def assert_remote_bytes(path: Path, expected: bytes, label: str) -> None:
    actual = path.read_bytes()
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")


def assert_zero_range(path: Path, offset: int, length: int, label: str) -> None:
    with path.open("rb") as stream:
        stream.seek(offset)
        data = stream.read(length)
    if data != b"\0" * length:
        raise AssertionError(f"{label}: range is not zero: {data[:64]!r}")


def initial_run(
    mount_a: Path,
    mount_b: Path,
    node_metrics_url: str,
    meta_metrics_url: str,
) -> dict[str, Any]:
    checks: list[dict[str, Any]] = []

    extend_a = mount_a / "preallocate-extend.bin"
    extend_b = mount_b / "preallocate-extend.bin"
    extend_a.write_bytes(b"")
    before = arena_snapshot(node_metrics_url)
    elapsed = timed(lambda: fallocate(extend_a, 0, 0, RESERVATION_BYTES))
    after = arena_snapshot(node_metrics_url)
    if extend_b.stat().st_size != RESERVATION_BYTES:
        raise AssertionError("mode-0 fallocate did not publish the extended EOF")
    assert_zero_range(extend_b, 0, 4096, "mode-0 preallocated range")
    if after["reserved_bytes"] - before["reserved_bytes"] < RESERVATION_BYTES:
        raise AssertionError("mode-0 fallocate did not reserve Arena capacity")
    checks.append(
        {
            "operation": "preallocate_extend",
            "latency_ms": elapsed,
            "size": extend_b.stat().st_size,
            "arena_before": before,
            "arena_after": after,
        }
    )

    keep_a = mount_a / "preallocate-keep-size.bin"
    keep_b = mount_b / "preallocate-keep-size.bin"
    keep_a.write_bytes(b"seed")
    before = arena_snapshot(node_metrics_url)
    elapsed = timed(
        lambda: fallocate(keep_a, FALLOC_FL_KEEP_SIZE, KEEP_SIZE_OFFSET, KEEP_SIZE_BYTES)
    )
    after = arena_snapshot(node_metrics_url)
    if keep_b.stat().st_size != 4:
        raise AssertionError("KEEP_SIZE changed EOF")
    if after["reserved_bytes"] - before["reserved_bytes"] < KEEP_SIZE_BYTES:
        raise AssertionError("KEEP_SIZE did not reserve Arena capacity")
    checks.append(
        {
            "operation": "preallocate_keep_size",
            "latency_ms": elapsed,
            "size": keep_b.stat().st_size,
            "arena_before": before,
            "arena_after": after,
        }
    )

    # 使用完整页大小，避免 64-byte Arena 对齐掩盖很小写入带来的 reservation 扣减。
    patch = b"R" * 4096
    before = arena_snapshot(node_metrics_url)
    fd = os.open(extend_a, os.O_RDWR)
    try:
        elapsed = timed(lambda: os.pwrite(fd, patch, 0))
    finally:
        os.close(fd)
    after = arena_snapshot(node_metrics_url)
    with extend_b.open("rb") as stream:
        if stream.read(len(patch)) != patch:
            raise AssertionError("remote first read did not see data written into reservation")
    if after["reserved_bytes"] >= before["reserved_bytes"]:
        raise AssertionError("write did not consume reserved Arena capacity")
    checks.append(
        {
            "operation": "write_consumes_reservation",
            "latency_ms": elapsed,
            "bytes": len(patch),
            "arena_before": before,
            "arena_after": after,
        }
    )

    punch_offset = 0
    punch_length = 4096
    size_before = extend_b.stat().st_size
    elapsed = timed(
        lambda: fallocate(
            extend_a,
            FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
            punch_offset,
            punch_length,
        )
    )
    if extend_b.stat().st_size != size_before:
        raise AssertionError("PUNCH_HOLE changed EOF")
    assert_zero_range(extend_b, punch_offset, punch_length, "punched range")
    checks.append(
        {
            "operation": "punch_hole",
            "latency_ms": elapsed,
            "size": extend_b.stat().st_size,
        }
    )

    sync_a = mount_a / "sync-callbacks.bin"
    sync_a.write_bytes(b"sync")
    meta_before = scrape(meta_metrics_url)
    fd = os.open(sync_a, os.O_RDWR)
    directory_fd = os.open(mount_a, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        flush_ms = timed(lambda: os.close(os.dup(fd)))
        fdatasync_ms = timed(lambda: os.fdatasync(fd))
        fsync_ms = timed(lambda: os.fsync(fd))
        fsyncdir_ms = timed(lambda: os.fsync(directory_fd))
    finally:
        os.close(fd)
        os.close(directory_fd)
    meta_after = scrape(meta_metrics_url)
    commit_labels = {"operation": "filesystem_commit_version", "result": "ok"}
    commit_delta = metric_value(
        meta_after, "dms_meta_operations_total", commit_labels
    ) - metric_value(meta_before, "dms_meta_operations_total", commit_labels)
    checks.append(
        {
            "operation": "sync_callbacks",
            "latency_ms": {
                "flush": flush_ms,
                "fdatasync": fdatasync_ms,
                "fsync": fsync_ms,
                "fsyncdir": fsyncdir_ms,
            },
            "meta_commit_delta": commit_delta,
        }
    )

    flag_results: dict[str, Any] = {}
    for name, flag in (("o_sync", os.O_SYNC), ("o_dsync", getattr(os, "O_DSYNC", os.O_SYNC))):
        path_a = mount_a / f"{name}.bin"
        path_b = mount_b / f"{name}.bin"
        path_a.write_bytes(b"")
        payload = f"{name}-payload".encode("ascii")
        fd = os.open(path_a, os.O_WRONLY | flag)
        try:
            elapsed = timed(lambda: os.write(fd, payload))
        finally:
            os.close(fd)
        assert_remote_bytes(path_b, payload, f"{name} remote first read")
        flag_results[name] = {"latency_ms": elapsed, "bytes": len(payload)}
    checks.append({"operation": "sync_open_flags", "modes": flag_results})

    full_a = mount_a / "capacity-exhaustion.bin"
    full_a.write_bytes(b"")
    before = arena_snapshot(node_metrics_url)
    size_before = full_a.stat().st_size
    observed_errno = None
    started = time.monotonic_ns()
    try:
        fallocate(full_a, FALLOC_FL_KEEP_SIZE, 0, 64 * 1024 * 1024)
    except OSError as error:
        observed_errno = error.errno
    elapsed = (time.monotonic_ns() - started) / 1_000_000.0
    after = arena_snapshot(node_metrics_url)
    if observed_errno != errno.ENOSPC:
        raise AssertionError(f"capacity failure must be ENOSPC, got {observed_errno}")
    if full_a.stat().st_size != size_before:
        raise AssertionError("failed fallocate changed file size")
    if after["reserved_bytes"] != before["reserved_bytes"]:
        raise AssertionError("failed fallocate leaked reserved Arena capacity")
    checks.append(
        {
            "operation": "capacity_exhaustion",
            "latency_ms": elapsed,
            "errno": observed_errno,
            "arena_before": before,
            "arena_after": after,
        }
    )

    owner_a = mount_a / "owner-recovery.bin"
    owner_a.write_bytes(b"")
    fallocate(owner_a, FALLOC_FL_KEEP_SIZE, OWNER_RECOVERY_OFFSET, OWNER_RECOVERY_BYTES)
    owner_reserved = arena_snapshot(node_metrics_url)
    return {
        "schema": SCHEMA,
        "checks": checks,
        "owner_recovery_path": "/owner-recovery.bin",
        "owner_recovery_offset": OWNER_RECOVERY_OFFSET,
        "owner_recovery_bytes": OWNER_RECOVERY_BYTES,
        "owner_reserved_bytes_before_restart": owner_reserved["reserved_bytes"],
    }


def verify_meta_recovery(mount_a: Path, node_metrics_url: str, expected_reserved: float) -> dict[str, Any]:
    path = mount_a / "owner-recovery.bin"
    before = arena_snapshot(node_metrics_url)
    fallocate(path, FALLOC_FL_KEEP_SIZE, OWNER_RECOVERY_OFFSET, OWNER_RECOVERY_BYTES)
    after = arena_snapshot(node_metrics_url)
    if before["reserved_bytes"] != expected_reserved:
        raise AssertionError(
            f"Meta restart lost reservation view: {before['reserved_bytes']} != {expected_reserved}"
        )
    if after["reserved_bytes"] != before["reserved_bytes"]:
        raise AssertionError("repeating recovered fallocate reserved capacity twice")
    return {
        "schema": META_RECOVERY_SCHEMA,
        "reserved_bytes_before": before["reserved_bytes"],
        "reserved_bytes_after": after["reserved_bytes"],
    }


def verify_owner_recovery(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    path_a = mount_a / "owner-recovery.bin"
    path_b = mount_b / "owner-recovery.bin"
    payload = b"new-owner-after-restart"
    fd = os.open(path_b, os.O_RDWR)
    try:
        written = os.pwrite(fd, payload, OWNER_RECOVERY_OFFSET)
        if written != len(payload):
            raise AssertionError(f"short owner recovery write: {written}/{len(payload)}")
    finally:
        os.close(fd)
    with path_a.open("rb") as stream:
        stream.seek(OWNER_RECOVERY_OFFSET)
        actual = stream.read(len(payload))
    if actual != payload:
        raise AssertionError(f"restarted owner first read mismatch: {actual!r}")
    return {
        "schema": OWNER_RECOVERY_SCHEMA,
        "path": "/owner-recovery.bin",
        "offset": OWNER_RECOVERY_OFFSET,
        "bytes": len(payload),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mount-a", type=Path, required=True)
    parser.add_argument("--mount-b", type=Path, required=True)
    parser.add_argument("--node-a-metrics-url")
    parser.add_argument("--meta-metrics-url")
    parser.add_argument("--expected-reserved-bytes", type=float)
    parser.add_argument("--phase", choices=("initial", "meta-recovery", "owner-recovery"), default="initial")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    if args.phase == "initial":
        if not args.node_a_metrics_url or not args.meta_metrics_url:
            raise ValueError("initial phase requires Node and Meta metrics URLs")
        result = initial_run(
            args.mount_a, args.mount_b, args.node_a_metrics_url, args.meta_metrics_url
        )
    elif args.phase == "meta-recovery":
        if not args.node_a_metrics_url or args.expected_reserved_bytes is None:
            raise ValueError("meta-recovery requires Node metrics and expected reserved bytes")
        result = verify_meta_recovery(
            args.mount_a, args.node_a_metrics_url, args.expected_reserved_bytes
        )
    else:
        result = verify_owner_recovery(args.mount_a, args.mount_b)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""原生 Filesystem 文件尺寸语义 workload。

这个脚本只通过 POSIX 文件接口访问两个 FUSE mount。它不调用 DMS SDK 或内部调试
接口，目的就是把用户真正关心的文件尺寸行为固定下来：

* truncate shrink 只保留前缀；
* truncate grow 和 pwrite 越 EOF 形成 sparse hole，读洞必须返回 0；
* open(..., O_TRUNC) 在 open 返回前完成清空发布；
* O_APPEND 并发写由服务端原子选择末尾 offset，记录不能互相覆盖；
* Node A 写完后，Node B 第一次 stat/read 立即看到结果；
* 重启后首次业务读仍能从 Meta/Node 恢复路径和内容。

等待只用于 runner 启动进程和挂载点 ready；本文件里的跨节点业务断言不做 sleep 或
重试。失败时直接暴露为 E2E 合同失败。
"""

from __future__ import annotations

import argparse
import json
import os
import threading
import urllib.request
from pathlib import Path
from typing import Any


SCHEMA = "dms.filesystem.size-semantics-workload.v1"
RECOVERY_SCHEMA = "dms.filesystem.size-semantics-recovery.v1"
PAYLOAD_PREFIX = b"dms-size:"
SPARSE_OFFSET = 1024 * 1024 + 7
REQUIRED_OPERATIONS = {
    "truncate_shrink",
    "truncate_grow_sparse",
    "pwrite_beyond_eof_sparse",
    "open_o_trunc",
    "concurrent_o_append",
    "cross_node_visibility",
}


def deterministic_payload(label: str) -> bytes:
    return PAYLOAD_PREFIX + label.encode("ascii")


def assert_bytes(path: Path, expected: bytes, label: str) -> None:
    actual = path.read_bytes()
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")


def assert_zeroes(data: bytes, label: str) -> None:
    if any(data):
        raise AssertionError(f"{label}: sparse hole returned non-zero bytes: {data[:64]!r}")


def stat_summary(path: Path) -> dict[str, int]:
    """记录用户可见 stat 字段。

    st_blocks 在当前 FUSE 实现中可能按 logical size 估算，不能单独证明物理分配；
    evaluator 会优先使用 metrics/whitebox 的 Arena logical bytes 证据。
    """

    stat = path.stat()
    return {
        "size": int(stat.st_size),
        "blocks": int(getattr(stat, "st_blocks", 0)),
        "block_size": int(getattr(stat, "st_blksize", 0)),
    }


def scrape_metric(url: str, metric: str) -> float | None:
    """从 /metrics 抓一个不带 label 的 Gauge。

    该函数只用于可选白盒证据：验证 sparse hole 不随洞长度分配 Arena bytes。没有
    metrics URL 时 workload 仍能验证用户语义，但 evaluator 会把资源语义标为失败。
    """

    try:
        with urllib.request.urlopen(url, timeout=3) as response:
            text = response.read().decode("utf-8")
    except OSError:
        return None
    for line in text.splitlines():
        if line.startswith("#") or not line.startswith(metric + " "):
            continue
        try:
            return float(line.rsplit(maxsplit=1)[1])
        except (IndexError, ValueError):
            return None
    return None


class MetricProbe:
    def __init__(self, node_a_metrics_url: str | None) -> None:
        self.node_a_metrics_url = node_a_metrics_url

    def value(self) -> float | None:
        if not self.node_a_metrics_url:
            return None
        return scrape_metric(self.node_a_metrics_url, "dms_node_arena_logical_bytes")

    def delta_around(self, action) -> tuple[float | None, float | None, float | None]:
        before = self.value()
        action()
        after = self.value()
        if before is None or after is None:
            return before, after, None
        return before, after, after - before


def truncate_shrink(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    path_a = mount_a / "truncate-shrink.txt"
    path_b = mount_b / "truncate-shrink.txt"
    path_a.write_bytes(b"0123456789")
    os.truncate(path_a, 4)

    assert path_b.stat().st_size == 4
    assert_bytes(path_b, b"0123", "truncate shrink remote read")
    return {
        "operation": "truncate_shrink",
        "remote_stat": stat_summary(path_b),
        "expected_bytes": 4,
    }


def truncate_grow_sparse(mount_a: Path, mount_b: Path, metrics: MetricProbe) -> dict[str, Any]:
    path_a = mount_a / "truncate-grow-sparse.txt"
    path_b = mount_b / "truncate-grow-sparse.txt"
    path_a.write_bytes(b"abc")
    target_size = SPARSE_OFFSET

    def action() -> None:
        os.truncate(path_a, target_size)

    before, after, delta = metrics.delta_around(action)
    if path_b.stat().st_size != target_size:
        raise AssertionError("truncate grow did not publish target size")
    with path_b.open("rb") as stream:
        stream.seek(0)
        assert stream.read(3) == b"abc"
        stream.seek(3)
        assert_zeroes(stream.read(4096), "truncate grow sparse hole")
        stream.seek(target_size - 16)
        assert_zeroes(stream.read(16), "truncate grow sparse tail")
    return {
        "operation": "truncate_grow_sparse",
        "target_size": target_size,
        "remote_stat": stat_summary(path_b),
        "arena_logical_bytes_before": before,
        "arena_logical_bytes_after": after,
        "arena_logical_bytes_delta": delta,
        "max_expected_allocation_delta": 4096,
    }


def pwrite_beyond_eof_sparse(mount_a: Path, mount_b: Path, metrics: MetricProbe) -> dict[str, Any]:
    path_a = mount_a / "pwrite-beyond-eof.txt"
    path_b = mount_b / "pwrite-beyond-eof.txt"
    path_a.write_bytes(b"head")
    patch = b"tail"
    expected_size = SPARSE_OFFSET + len(patch)

    def action() -> None:
        fd = os.open(path_a, os.O_RDWR)
        try:
            written = os.pwrite(fd, patch, SPARSE_OFFSET)
            if written != len(patch):
                raise AssertionError(f"short pwrite: {written}/{len(patch)}")
        finally:
            os.close(fd)

    before, after, delta = metrics.delta_around(action)
    if path_b.stat().st_size != expected_size:
        raise AssertionError("pwrite beyond EOF did not publish sparse target size")
    with path_b.open("rb") as stream:
        assert stream.read(4) == b"head"
        stream.seek(4)
        assert_zeroes(stream.read(4096), "pwrite sparse middle hole")
        stream.seek(SPARSE_OFFSET)
        assert stream.read(len(patch)) == patch
    return {
        "operation": "pwrite_beyond_eof_sparse",
        "offset": SPARSE_OFFSET,
        "patch_bytes": len(patch),
        "remote_stat": stat_summary(path_b),
        "arena_logical_bytes_before": before,
        "arena_logical_bytes_after": after,
        "arena_logical_bytes_delta": delta,
        "max_expected_allocation_delta": 4096 + len(patch),
    }


def open_o_trunc(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    path_a = mount_a / "open-o-trunc.txt"
    path_b = mount_b / "open-o-trunc.txt"
    path_a.write_bytes(deterministic_payload("o-trunc-before"))
    fd = os.open(path_a, os.O_WRONLY | os.O_TRUNC)
    os.close(fd)

    if path_b.stat().st_size != 0:
        raise AssertionError("O_TRUNC open returned before remote size became zero")
    assert_bytes(path_b, b"", "O_TRUNC remote read")
    return {"operation": "open_o_trunc", "remote_stat": stat_summary(path_b)}


def concurrent_o_append(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    path_a = mount_a / "append.txt"
    path_b = mount_b / "append.txt"
    path_a.write_bytes(b"")
    writer_count = 16
    records = [f"record-{index:04d}\n".encode("ascii") for index in range(writer_count)]
    barrier = threading.Barrier(writer_count)
    errors: list[str] = []

    def writer(record: bytes) -> None:
        try:
            fd = os.open(path_a, os.O_WRONLY | os.O_APPEND)
            try:
                barrier.wait(timeout=5)
                written = os.write(fd, record)
                if written != len(record):
                    errors.append(f"short append for {record!r}: {written}")
            finally:
                os.close(fd)
        except Exception as error:  # pragma: no cover - surfaced by assertion below.
            errors.append(str(error))

    threads = [threading.Thread(target=writer, args=(record,)) for record in records]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=10)
    if errors:
        raise AssertionError("; ".join(errors))

    actual = path_b.read_bytes().splitlines(keepends=True)
    if sorted(actual) != sorted(records):
        raise AssertionError(f"O_APPEND records overlapped or were lost: {actual!r}")
    return {
        "operation": "concurrent_o_append",
        "record_count": writer_count,
        "bytes": len(b"".join(records)),
        "remote_stat": stat_summary(path_b),
    }


def cross_node_visibility(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    path_a = mount_a / "visibility.txt"
    path_b = mount_b / "visibility.txt"
    payload = deterministic_payload("cross-node")
    path_a.write_bytes(payload)

    # mutation 返回后的第一次远端业务检查必须成功；这里没有 wait/retry。
    if path_b.stat().st_size != len(payload):
        raise AssertionError("remote stat did not see size on first check")
    assert_bytes(path_b, payload, "remote read first check")
    return {
        "operation": "cross_node_visibility",
        "remote_stat": stat_summary(path_b),
        "bytes": len(payload),
        "path": "/visibility.txt",
    }


def run(mount_a: Path, mount_b: Path, metrics_url: str | None) -> dict[str, Any]:
    metrics = MetricProbe(metrics_url)
    checks = [
        truncate_shrink(mount_a, mount_b),
        truncate_grow_sparse(mount_a, mount_b, metrics),
        pwrite_beyond_eof_sparse(mount_a, mount_b, metrics),
        open_o_trunc(mount_a, mount_b),
        concurrent_o_append(mount_a, mount_b),
        cross_node_visibility(mount_a, mount_b),
    ]
    return {
        "schema": SCHEMA,
        "passed_operations": len(checks),
        "checks": checks,
        "recovery_path": "/visibility.txt",
        "recovery_expected": deterministic_payload("cross-node").decode("ascii"),
    }


def verify_recovery(mount_b: Path, expected_text: str) -> dict[str, Any]:
    path = mount_b / "visibility.txt"
    expected = expected_text.encode("ascii")
    if path.stat().st_size != len(expected):
        raise AssertionError("recovered stat did not expose committed file size")
    assert_bytes(path, expected, "recovered first read")
    return {
        "schema": RECOVERY_SCHEMA,
        "path": "/visibility.txt",
        "bytes": len(expected),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mount-a", type=Path, required=True)
    parser.add_argument("--mount-b", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--node-a-metrics-url")
    parser.add_argument("--recovery-only", action="store_true")
    parser.add_argument("--recovery-expected")
    args = parser.parse_args()

    if args.recovery_only:
        if args.recovery_expected is None:
            raise ValueError("--recovery-only requires --recovery-expected")
        result = verify_recovery(args.mount_b, args.recovery_expected)
    else:
        for mount in (args.mount_a, args.mount_b):
            if not mount.is_dir():
                raise FileNotFoundError(mount)
        result = run(args.mount_a, args.mount_b, args.node_a_metrics_url)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

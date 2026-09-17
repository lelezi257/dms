#!/usr/bin/env python3
"""M1 fio 数据完整性 workload。

这个脚本只通过已经挂载好的 DMS FUSE 目录执行业务操作。fio 负责写后 verify，
脚本再从另一挂载点逐字节/哈希校验同一个权威版本，避免把“fio 自己读回正确”
误当成跨 Node 文件系统一致性正确。
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import mmap
import os
from pathlib import Path
import shutil
import subprocess
import time
import urllib.request
from typing import Any


SCHEMA = "dms.m1.fio-integrity-workload.v1"
FALLOC_FL_KEEP_SIZE = 0x01
FALLOC_FL_PUNCH_HOLE = 0x02
REQUIRED_CASES = {
    "fio_seq_4k",
    "fio_rand_1m",
    "fio_large_seq",
    "truncate_shrink_grow",
    "punch_hole_zero",
    "mmap_shared_hash",
}


def parse_size(value: str) -> int:
    suffixes = {"k": 1024, "m": 1024**2, "g": 1024**3}
    raw = value.strip().lower()
    if raw[-1:] in suffixes:
        return int(raw[:-1]) * suffixes[raw[-1]]
    return int(raw)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def scrape(url: str | None) -> str:
    if not url:
        return ""
    with urllib.request.urlopen(url, timeout=3) as response:
        return response.read().decode("utf-8")


def metric_value(text: str, name: str) -> float | None:
    if not text:
        return None
    total = 0.0
    found = False
    for line in text.splitlines():
        if line.startswith("#") or not line.startswith(name):
            continue
        metric, value = line.rsplit(maxsplit=1)
        if metric != name and not metric.startswith(name + "{"):
            continue
        total += float(value)
        found = True
    return total if found else None


def run(argv: list[str]) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, text=True, capture_output=True, check=False)
    if completed.returncode:
        detail = (completed.stdout or "") + (completed.stderr or "")
        raise RuntimeError(f"command failed ({completed.returncode}): {' '.join(argv)}\n{detail[-4000:]}")
    return completed


def timed(action) -> tuple[Any, float]:
    started = time.monotonic_ns()
    result = action()
    return result, (time.monotonic_ns() - started) / 1_000_000.0


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


def assert_remote_hash(local: Path, remote: Path) -> dict[str, str]:
    local_hash = sha256(local)
    remote_hash = sha256(remote)
    if local_hash != remote_hash:
        raise AssertionError(f"hash mismatch: {local}={local_hash} {remote}={remote_hash}")
    return {"local_sha256": local_hash, "remote_sha256": remote_hash}


def run_fio_case(
    *,
    name: str,
    filename: str,
    size: str,
    rw: str,
    bs: str,
    mount_a: Path,
    mount_b: Path,
    output_dir: Path,
) -> dict[str, Any]:
    fio_output = output_dir / f"{name}.fio.json"
    argv = [
        "fio",
        "--name",
        name,
        "--directory",
        str(mount_a),
        "--filename",
        filename,
        "--size",
        size,
        "--rw",
        rw,
        "--bs",
        bs,
        "--ioengine",
        "sync",
        "--verify",
        "crc32c",
        "--do_verify",
        "1",
        "--verify_fatal",
        "1",
        "--verify_dump",
        "0",
        "--verify_state_save",
        "0",
        "--aux-path",
        str(output_dir),
        "--randrepeat",
        "1",
        "--refill_buffers",
        "--end_fsync",
        "1",
        "--output-format",
        "json",
        "--output",
        str(fio_output),
    ]
    _, elapsed_ms = timed(lambda: run(argv))
    report = json.loads(fio_output.read_text(encoding="utf-8"))
    jobs = report.get("jobs", [])
    if not jobs or any(int(job.get("error", 0)) != 0 for job in jobs):
        raise AssertionError(f"fio reported error for {name}: {jobs!r}")
    hashes = assert_remote_hash(mount_a / filename, mount_b / filename)
    return {
        "case": name,
        "status": "passed",
        "kind": "fio",
        "rw": rw,
        "bs": bs,
        "size": parse_size(size),
        "elapsed_ms": elapsed_ms,
        "fio_json": str(fio_output),
        **hashes,
    }


def truncate_case(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    path_a = mount_a / "truncate-integrity.bin"
    path_b = mount_b / "truncate-integrity.bin"
    payload = (b"0123456789abcdef" * 4096)
    path_a.write_bytes(payload)
    os.truncate(path_a, 4096)
    os.truncate(path_a, 16384)
    with path_a.open("r+b") as stream:
        stream.seek(12288)
        stream.write(b"T" * 4096)
        stream.flush()
        os.fsync(stream.fileno())
    expected = payload[:4096] + (b"\0" * 8192) + (b"T" * 4096)
    actual = path_b.read_bytes()
    if actual != expected:
        raise AssertionError("truncate shrink/grow produced unexpected remote bytes")
    return {
        "case": "truncate_shrink_grow",
        "status": "passed",
        "kind": "posix",
        "size": len(actual),
        "sha256": hashlib.sha256(actual).hexdigest(),
    }


def punch_case(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    path_a = mount_a / "punch-integrity.bin"
    path_b = mount_b / "punch-integrity.bin"
    path_a.write_bytes(b"A" * 4096 + b"B" * 4096 + b"C" * 4096)
    fallocate(path_a, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE, 4096, 4096)
    data = path_b.read_bytes()
    expected = b"A" * 4096 + b"\0" * 4096 + b"C" * 4096
    if data != expected:
        raise AssertionError("punched range is not zero or adjacent bytes changed")
    return {
        "case": "punch_hole_zero",
        "status": "passed",
        "kind": "posix",
        "size": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def mmap_case(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    path_a = mount_a / "mmap-integrity.bin"
    path_b = mount_b / "mmap-integrity.bin"
    length = 64 * 1024
    path_a.write_bytes(b"\0" * length)
    fd = os.open(path_a, os.O_RDWR)
    try:
        mapping = mmap.mmap(fd, length, access=mmap.ACCESS_WRITE)
        try:
            for offset in range(0, length, 4096):
                mapping[offset : offset + 4096] = bytes([(offset // 4096) % 251]) * 4096
            mapping.flush()
        finally:
            mapping.close()
        os.fsync(fd)
    finally:
        os.close(fd)
    hashes = assert_remote_hash(path_a, path_b)
    return {
        "case": "mmap_shared_hash",
        "status": "passed",
        "kind": "mmap",
        "size": length,
        **hashes,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mount-a", type=Path, required=True)
    parser.add_argument("--mount-b", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--large-size", default="512m")
    parser.add_argument("--node-a-metrics-url")
    parser.add_argument("--node-b-metrics-url")
    args = parser.parse_args()

    if shutil.which("fio") is None:
        raise SystemExit("fio is required for M1 fio-integrity; install fio instead of skipping this case")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    start_a = scrape(args.node_a_metrics_url)
    start_b = scrape(args.node_b_metrics_url)
    checks: list[dict[str, Any]] = []

    checks.append(
        run_fio_case(
            name="fio_seq_4k",
            filename="fio-seq-4k.bin",
            size="4k",
            rw="write",
            bs="4k",
            mount_a=args.mount_a,
            mount_b=args.mount_b,
            output_dir=args.output.parent,
        )
    )
    checks.append(
        run_fio_case(
            name="fio_rand_1m",
            filename="fio-rand-1m.bin",
            size="1m",
            rw="randwrite",
            bs="4k",
            mount_a=args.mount_a,
            mount_b=args.mount_b,
            output_dir=args.output.parent,
        )
    )
    checks.append(
        run_fio_case(
            name="fio_large_seq",
            filename="fio-large-seq.bin",
            size=args.large_size,
            rw="write",
            bs="1m",
            mount_a=args.mount_a,
            mount_b=args.mount_b,
            output_dir=args.output.parent,
        )
    )
    checks.append(truncate_case(args.mount_a, args.mount_b))
    checks.append(punch_case(args.mount_a, args.mount_b))
    checks.append(mmap_case(args.mount_a, args.mount_b))

    end_a = scrape(args.node_a_metrics_url)
    end_b = scrape(args.node_b_metrics_url)
    result = {
        "schema": SCHEMA,
        "status": "passed",
        "large_size_bytes": parse_size(args.large_size),
        "checks": checks,
        "metrics": {
            "node_a_filesystem_ops_delta": None
            if metric_value(start_a, "dms_node_filesystem_operations_total") is None
            else metric_value(end_a, "dms_node_filesystem_operations_total")
            - metric_value(start_a, "dms_node_filesystem_operations_total"),
            "node_b_filesystem_ops_delta": None
            if metric_value(start_b, "dms_node_filesystem_operations_total") is None
            else metric_value(end_b, "dms_node_filesystem_operations_total")
            - metric_value(start_b, "dms_node_filesystem_operations_total"),
        },
    }
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

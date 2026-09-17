#!/usr/bin/env python3
"""P3 等语义 write-through workload。

两种后端都只通过 POSIX 接口访问挂载目录。每个同步写样本固定执行
``open -> pwrite -> fdatasync -> close``，从而避免把 DMS write-through
与对端 buffered write 混为同一语义。holder 文件会先由远端挂载读取，
用于量化 DMS 强一致失效屏障的固定成本。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import statistics
import time
from typing import Callable


MIB = 1024 * 1024
WRITE_MATRIX = ((4096, 120), (65536, 80), (MIB, 30), (8 * MIB, 6))
STREAM_SIZE = 512 * MIB
STREAM_CHUNK = MIB
STREAM_FILE = "sync-stream-512m.bin"


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * fraction)))
    return ordered[index]


def deterministic_bytes(label: str, size: int, seed: int) -> bytes:
    digest = hashlib.sha256(f"{seed}:{label}".encode()).digest()
    return (digest * ((size + len(digest) - 1) // len(digest)))[:size]


def matrix_path(root: Path, holder: bool, size: int, index: int) -> Path:
    group = "holder" if holder else "no-holder"
    return root / "p3" / group / f"file-{size}-{index:04d}.bin"


def phase_size(phase: str) -> int:
    suffix = phase.rsplit("-", 1)[-1]
    aliases = {"4k": 4096, "64k": 65536, "1m": MIB, "8m": 8 * MIB}
    if suffix not in aliases:
        raise ValueError(f"phase has no matrix size: {phase}")
    return aliases[suffix]


def size_name(size: int) -> str:
    return {4096: "4k", 65536: "64k", MIB: "1m", 8 * MIB: "8m"}[size]


class Recorder:
    def __init__(self, phase: str) -> None:
        self.phase = phase
        self.samples: list[dict[str, object]] = []
        self.bytes = 0

    def add(
        self,
        case_id: str,
        size: int,
        path: Path,
        elapsed_us: float,
        byte_count: int,
        segments_us: dict[str, float],
    ) -> None:
        self.bytes += byte_count
        self.samples.append(
            {
                "case_id": case_id,
                "size": size,
                "path": path.name,
                "latency_us": elapsed_us,
                "bytes": byte_count,
                "segments_us": segments_us,
                "ok": True,
            }
        )

    def measure_simple(
        self, case_id: str, size: int, path: Path, action: Callable[[], int]
    ) -> None:
        started = time.perf_counter_ns()
        byte_count = action()
        elapsed_us = (time.perf_counter_ns() - started) / 1000.0
        self.add(case_id, size, path, elapsed_us, byte_count, {"operation": elapsed_us})

    def summary(self) -> dict[str, object]:
        grouped: dict[str, list[dict[str, object]]] = {}
        for sample in self.samples:
            grouped.setdefault(str(sample["case_id"]), []).append(sample)
        result: dict[str, object] = {}
        for case_id, samples in sorted(grouped.items()):
            latencies = [float(sample["latency_us"]) for sample in samples]
            total_bytes = sum(int(sample["bytes"]) for sample in samples)
            total_seconds = sum(latencies) / 1_000_000.0
            segment_names = {
                name
                for sample in samples
                for name in dict(sample.get("segments_us", {}))
            }
            result[case_id] = {
                "samples": len(samples),
                "p50_us": percentile(latencies, 0.50),
                "p95_us": percentile(latencies, 0.95),
                "p99_us": percentile(latencies, 0.99),
                "mean_us": statistics.fmean(latencies),
                "bytes": total_bytes,
                "throughput_mib_s": (
                    total_bytes / MIB / total_seconds if total_seconds else 0.0
                ),
                "segment_mean_us": {
                    name: statistics.fmean(
                        float(dict(sample.get("segments_us", {})).get(name, 0.0))
                        for sample in samples
                    )
                    for name in sorted(segment_names)
                },
            }
        return result


def prepare(root: Path, seed: int) -> None:
    for holder in (False, True):
        for size, count in WRITE_MATRIX:
            directory = matrix_path(root, holder, size, 0).parent
            directory.mkdir(parents=True, exist_ok=True)
            initial = deterministic_bytes(f"initial:{holder}:{size}", size, seed)
            for index in range(count):
                path = matrix_path(root, holder, size, index)
                descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o664)
                try:
                    view = memoryview(initial)
                    while view:
                        written = os.write(descriptor, view)
                        view = view[written:]
                    os.fdatasync(descriptor)
                finally:
                    os.close(descriptor)
    stream = root / "p3" / STREAM_FILE
    descriptor = os.open(stream, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o664)
    try:
        os.ftruncate(descriptor, STREAM_SIZE)
        os.fdatasync(descriptor)
    finally:
        os.close(descriptor)


def read_exact(path: Path, expected: bytes) -> int:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        chunks: list[bytes] = []
        remaining = len(expected)
        while remaining:
            data = os.read(descriptor, remaining)
            if not data:
                break
            chunks.append(data)
            remaining -= len(data)
    finally:
        os.close(descriptor)
    actual = b"".join(chunks)
    if actual != expected:
        raise RuntimeError(f"content mismatch: {path}: {len(actual)}/{len(expected)}")
    return len(actual)


def warm_holder(root: Path, seed: int) -> None:
    for size, count in WRITE_MATRIX:
        expected = deterministic_bytes(f"initial:True:{size}", size, seed)
        for index in range(count):
            read_exact(matrix_path(root, True, size, index), expected)


def warm_holder_size(root: Path, seed: int, size: int) -> None:
    expected = deterministic_bytes(f"initial:True:{size}", size, seed)
    for index in range(dict(WRITE_MATRIX)[size]):
        read_exact(matrix_path(root, True, size, index), expected)


def pwrite_all(descriptor: int, data: bytes, offset: int) -> int:
    """把完整 payload 写到指定偏移，并正确处理 POSIX 允许的短写。"""
    view = memoryview(data)
    written_total = 0
    while view:
        written = os.pwrite(descriptor, view, offset + written_total)
        if written <= 0:
            raise RuntimeError(f"pwrite made no progress at offset {offset + written_total}")
        written_total += written
        view = view[written:]
    return written_total


def synchronized_write_sample(path: Path, data: bytes) -> tuple[float, dict[str, float]]:
    total_started = time.perf_counter_ns()
    started = time.perf_counter_ns()
    descriptor = os.open(path, os.O_RDWR)
    open_us = (time.perf_counter_ns() - started) / 1000.0
    try:
        started = time.perf_counter_ns()
        written = pwrite_all(descriptor, data, 0)
        write_us = (time.perf_counter_ns() - started) / 1000.0
        if written != len(data):
            raise RuntimeError(f"short pwrite: {path}: {written}/{len(data)}")
        started = time.perf_counter_ns()
        os.fdatasync(descriptor)
        sync_us = (time.perf_counter_ns() - started) / 1000.0
    finally:
        started = time.perf_counter_ns()
        os.close(descriptor)
        close_us = (time.perf_counter_ns() - started) / 1000.0
    elapsed_us = (time.perf_counter_ns() - total_started) / 1000.0
    return elapsed_us, {
        "open": open_us,
        "pwrite": write_us,
        "fdatasync": sync_us,
        "close": close_us,
    }


def write_matrix(root: Path, recorder: Recorder, seed: int, holder: bool, size: int) -> None:
    count = dict(WRITE_MATRIX)[size]
    case = f"sync_write.{'holder' if holder else 'no_holder'}.{size_name(size)}"
    for index in range(count):
        path = matrix_path(root, holder, size, index)
        data = deterministic_bytes(f"updated:{holder}:{size}:{index}", size, seed)
        elapsed_us, segments = synchronized_write_sample(path, data)
        recorder.add(case, size, path, elapsed_us, len(data), segments)
        read_exact(path, data)


def read_matrix(
    root: Path,
    recorder: Recorder | None,
    seed: int,
    holder: bool,
    size: int,
    case_prefix: str,
) -> None:
    count = dict(WRITE_MATRIX)[size]
    case = f"{case_prefix}.{size_name(size)}"
    for index in range(count):
        path = matrix_path(root, holder, size, index)
        expected = deterministic_bytes(f"updated:{holder}:{size}:{index}", size, seed)
        if recorder is None:
            read_exact(path, expected)
        else:
            recorder.measure_simple(
                case,
                size,
                path,
                lambda path=path, expected=expected: read_exact(path, expected),
            )


def stat_matrix(root: Path, recorder: Recorder) -> None:
    size = 4096
    for index in range(dict(WRITE_MATRIX)[size]):
        path = matrix_path(root, False, size, index)

        def action(path: Path = path, size: int = size) -> int:
            if os.stat(path).st_size != size:
                raise RuntimeError(f"unexpected size: {path}")
            return 0

        recorder.measure_simple("stable.stat.4k", size, path, action)


def synchronized_stream(root: Path, recorder: Recorder, seed: int) -> None:
    path = root / "p3" / STREAM_FILE
    descriptor = os.open(path, os.O_RDWR)
    segment_ns = {"pwrite": 0, "fdatasync": 0}
    total_started = time.perf_counter_ns()
    try:
        for offset in range(0, STREAM_SIZE, STREAM_CHUNK):
            data = deterministic_bytes(f"stream:{offset}", STREAM_CHUNK, seed)
            started = time.perf_counter_ns()
            written = pwrite_all(descriptor, data, offset)
            segment_ns["pwrite"] += time.perf_counter_ns() - started
            if written != len(data):
                raise RuntimeError(f"short stream pwrite: {written}/{len(data)}")
            started = time.perf_counter_ns()
            os.fdatasync(descriptor)
            segment_ns["fdatasync"] += time.perf_counter_ns() - started
    finally:
        os.close(descriptor)
    elapsed_us = (time.perf_counter_ns() - total_started) / 1000.0
    recorder.add(
        "sync_write.no_holder.512m_stream",
        STREAM_SIZE,
        path,
        elapsed_us,
        STREAM_SIZE,
        {name: value / 1000.0 for name, value in segment_ns.items()},
    )

    descriptor = os.open(path, os.O_RDONLY)
    try:
        for offset in range(0, STREAM_SIZE, STREAM_CHUNK):
            expected = deterministic_bytes(f"stream:{offset}", STREAM_CHUNK, seed)
            actual = os.pread(descriptor, STREAM_CHUNK, offset)
            if actual != expected:
                raise RuntimeError(f"stream content mismatch at {offset}")
    finally:
        os.close(descriptor)


def parse_args() -> argparse.Namespace:
    phases = ["prepare", "holder-warm", "peer-warm", "stat-4k", "sync-stream-512m"]
    for prefix in ("write-no-holder", "write-holder", "local-read", "peer-repeat"):
        phases.extend(f"{prefix}-{name}" for name in ("4k", "64k", "1m", "8m"))
    phases.extend(f"holder-warm-{name}" for name in ("4k", "64k", "1m", "8m"))
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--phase", choices=tuple(phases), required=True)
    parser.add_argument("--seed", type=int, default=7703)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    recorder = Recorder(args.phase)
    cpu_before = resource.getrusage(resource.RUSAGE_SELF)
    wall_started = time.perf_counter_ns()

    if args.phase == "prepare":
        prepare(args.root, args.seed)
    elif args.phase == "holder-warm":
        warm_holder(args.root, args.seed)
    elif args.phase.startswith("holder-warm-"):
        warm_holder_size(args.root, args.seed, phase_size(args.phase))
    elif args.phase == "peer-warm":
        for size, _ in WRITE_MATRIX:
            read_matrix(args.root, None, args.seed, False, size, "unused")
    elif args.phase == "stat-4k":
        stat_matrix(args.root, recorder)
    elif args.phase == "sync-stream-512m":
        synchronized_stream(args.root, recorder, args.seed)
    else:
        size = phase_size(args.phase)
        if args.phase.startswith("write-no-holder-"):
            write_matrix(args.root, recorder, args.seed, False, size)
        elif args.phase.startswith("write-holder-"):
            write_matrix(args.root, recorder, args.seed, True, size)
        elif args.phase.startswith("local-read-"):
            read_matrix(args.root, recorder, args.seed, False, size, "stable_read.local")
        elif args.phase.startswith("peer-repeat-"):
            read_matrix(args.root, recorder, args.seed, False, size, "stable_read.peer")

    cpu_after = resource.getrusage(resource.RUSAGE_SELF)
    wall_ns = time.perf_counter_ns() - wall_started
    result = {
        "schema": "dms.native-fs-write-through-workload.v1",
        "phase": args.phase,
        "seed": args.seed,
        "root": str(args.root),
        "wall_ns": wall_ns,
        "process_user_cpu_ns": int((cpu_after.ru_utime - cpu_before.ru_utime) * 1_000_000_000),
        "process_system_cpu_ns": int((cpu_after.ru_stime - cpu_before.ru_stime) * 1_000_000_000),
        "bytes": recorder.bytes,
        "correctness": True,
        "summary": recorder.summary(),
        "samples": recorder.samples,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""DMS Native Filesystem 与 MooseFS 共用的 POSIX workload。

本脚本不知道后端类型，只通过挂载目录执行普通文件操作。每次调用只运行一个
phase，三 VM 编排器会在 phase 前后采集服务计数、网络和进程资源，从而把端到端
时延与内部成本对应起来。
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
from typing import Callable, Iterable


WORKSPACE_LAYOUT = ((4096, 120), (65536, 40))
PATCH_OFFSET = 2048
PATCH_LENGTH = 1024
LARGE_SIZE = 512 * 1024 * 1024
LARGE_CHUNK = 8 * 1024 * 1024
LARGE_FILE = "dms-large-512m.bin"


def deterministic_bytes(label: str, size: int, seed: int) -> bytes:
    digest = hashlib.sha256(f"{seed}:{label}".encode()).digest()
    return (digest * ((size + len(digest) - 1) // len(digest)))[:size]


def workspace_path(root: Path, size: int, index: int) -> Path:
    group = index % 8
    return root / "workspace" / f"task-{group:02d}" / f"file-{size}-{index:04d}.bin"


def workspace_bytes(size: int, index: int, seed: int, *, patched: bool = False) -> bytes:
    data = bytearray(deterministic_bytes(f"workspace:{size}:{index}", size, seed))
    if patched and size == 4096:
        data[PATCH_OFFSET : PATCH_OFFSET + PATCH_LENGTH] = deterministic_bytes(
            f"patch:{index}", PATCH_LENGTH, seed
        )
    return bytes(data)


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * fraction)))
    return ordered[index]


class Recorder:
    def __init__(self, phase: str) -> None:
        self.phase = phase
        self.samples: list[dict[str, object]] = []
        self.bytes = 0

    def measure(self, case_id: str, size: int, path: Path, action: Callable[[], int]) -> None:
        started = time.perf_counter_ns()
        byte_count = action()
        elapsed_us = (time.perf_counter_ns() - started) / 1000.0
        self.bytes += byte_count
        self.samples.append(
            {
                "case_id": case_id,
                "size": size,
                "path": path.name,
                "latency_us": elapsed_us,
                "bytes": byte_count,
                "ok": True,
            }
        )

    def summary(self) -> dict[str, object]:
        grouped: dict[str, list[dict[str, object]]] = {}
        for sample in self.samples:
            grouped.setdefault(str(sample["case_id"]), []).append(sample)
        result: dict[str, object] = {}
        for case_id, samples in sorted(grouped.items()):
            latencies = [float(sample["latency_us"]) for sample in samples]
            total_bytes = sum(int(sample["bytes"]) for sample in samples)
            total_seconds = sum(latencies) / 1_000_000.0
            result[case_id] = {
                "samples": len(samples),
                "p50_us": percentile(latencies, 0.50),
                "p95_us": percentile(latencies, 0.95),
                "p99_us": percentile(latencies, 0.99),
                "mean_us": statistics.fmean(latencies),
                "bytes": total_bytes,
                "throughput_mib_s": (
                    total_bytes / (1024 * 1024) / total_seconds if total_seconds else 0.0
                ),
            }
        return result


def workspace_files() -> Iterable[tuple[int, int]]:
    for size, count in WORKSPACE_LAYOUT:
        for index in range(count):
            yield size, index


def prepare_workspace_directories(root: Path) -> None:
    """在计时和 RPC 快照外创建 workload 共用的父目录。

    create/create-delete 的样本只表达单个文件操作；父目录初始化属于测试夹具，
    如果留在 before/after 快照内，会把 mkdir 的 Meta RPC 错算成文件 create RPC。
    """

    for group in range(8):
        (root / "workspace" / f"task-{group:02d}").mkdir(parents=True, exist_ok=True)
    (root / "workspace" / "ephemeral").mkdir(parents=True, exist_ok=True)


def create_workspace(root: Path, recorder: Recorder, seed: int) -> None:
    for size, index in workspace_files():
        path = workspace_path(root, size, index)
        data = workspace_bytes(size, index, seed)

        def action(path: Path = path, data: bytes = data) -> int:
            descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o664)
            try:
                written = os.write(descriptor, data)
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            if written != len(data):
                raise RuntimeError(f"short write: {path}: {written}/{len(data)}")
            return written

        recorder.measure("workspace.create", size, path, action)


def read_workspace(root: Path, recorder: Recorder, seed: int, case_id: str) -> None:
    for size, index in workspace_files():
        path = workspace_path(root, size, index)
        expected = workspace_bytes(size, index, seed, patched=False)

        def action(path: Path = path, expected: bytes = expected) -> int:
            descriptor = os.open(path, os.O_RDONLY)
            try:
                actual = os.read(descriptor, len(expected) + 1)
            finally:
                os.close(descriptor)
            if actual != expected:
                raise RuntimeError(f"content mismatch: {path}")
            return len(actual)

        recorder.measure(case_id, size, path, action)


def stat_workspace(root: Path, recorder: Recorder) -> None:
    for size, index in workspace_files():
        path = workspace_path(root, size, index)

        def action(path: Path = path, size: int = size) -> int:
            if os.stat(path).st_size != size:
                raise RuntimeError(f"unexpected size: {path}")
            return 0

        recorder.measure("workspace.stat", size, path, action)


def patch_workspace(root: Path, recorder: Recorder, seed: int) -> None:
    size = 4096
    count = dict(WORKSPACE_LAYOUT)[size]
    for index in range(count):
        path = workspace_path(root, size, index)
        patch = deterministic_bytes(f"patch:{index}", PATCH_LENGTH, seed)

        def action(path: Path = path, patch: bytes = patch) -> int:
            descriptor = os.open(path, os.O_RDWR)
            try:
                written = os.pwrite(descriptor, patch, PATCH_OFFSET)
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            if written != len(patch):
                raise RuntimeError(f"short pwrite: {path}: {written}/{len(patch)}")
            return written

        recorder.measure("workspace.patch", size, path, action)


def create_delete_workspace(root: Path, recorder: Recorder, seed: int) -> None:
    directory = root / "workspace" / "ephemeral"
    for index in range(80):
        path = directory / f"temp-{index:04d}.txt"
        data = deterministic_bytes(f"ephemeral:{index}", 1024, seed)

        def action(path: Path = path, data: bytes = data) -> int:
            descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o664)
            try:
                written = os.write(descriptor, data)
            finally:
                os.close(descriptor)
            os.unlink(path)
            if written != len(data):
                raise RuntimeError(f"short ephemeral write: {path}")
            return written

        recorder.measure("workspace.create_delete", len(data), path, action)


def large_chunk(seed: int, offset: int, length: int) -> bytes:
    return deterministic_bytes(f"large:{offset}", length, seed)


def large_digest(seed: int) -> str:
    digest = hashlib.sha256()
    for offset in range(0, LARGE_SIZE, LARGE_CHUNK):
        length = min(LARGE_CHUNK, LARGE_SIZE - offset)
        digest.update(large_chunk(seed, offset, length))
    return digest.hexdigest()


def create_large(root: Path, recorder: Recorder, seed: int) -> None:
    path = root / LARGE_FILE

    def action() -> int:
        descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o664)
        total = 0
        try:
            for offset in range(0, LARGE_SIZE, LARGE_CHUNK):
                data = large_chunk(seed, offset, min(LARGE_CHUNK, LARGE_SIZE - offset))
                view = memoryview(data)
                while view:
                    written = os.write(descriptor, view)
                    total += written
                    view = view[written:]
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        if total != LARGE_SIZE:
            raise RuntimeError(f"short large write: {total}/{LARGE_SIZE}")
        return total

    recorder.measure("sequential_512m.write", LARGE_SIZE, path, action)


def read_large(root: Path, recorder: Recorder, seed: int, case_id: str) -> None:
    path = root / LARGE_FILE
    expected_digest = large_digest(seed)

    def action() -> int:
        digest = hashlib.sha256()
        total = 0
        descriptor = os.open(path, os.O_RDONLY)
        try:
            while True:
                data = os.read(descriptor, LARGE_CHUNK)
                if not data:
                    break
                total += len(data)
                digest.update(data)
        finally:
            os.close(descriptor)
        if total != LARGE_SIZE or digest.hexdigest() != expected_digest:
            raise RuntimeError(f"large content mismatch: bytes={total}")
        return total

    recorder.measure(case_id, LARGE_SIZE, path, action)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument(
        "--phase",
        choices=(
            "workspace-prepare",
            "workspace-create",
            "workspace-local-hot",
            "workspace-stat",
            "workspace-peer-first",
            "workspace-peer-repeat",
            "workspace-patch",
            "workspace-create-delete",
            "large-create",
            "large-local-read",
            "large-peer-first",
            "large-peer-repeat",
        ),
        required=True,
    )
    parser.add_argument("--seed", type=int, default=6701)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    recorder = Recorder(args.phase)
    cpu_before = resource.getrusage(resource.RUSAGE_SELF)
    wall_started = time.perf_counter_ns()

    if args.phase == "workspace-prepare":
        prepare_workspace_directories(args.root)
    elif args.phase == "workspace-create":
        create_workspace(args.root, recorder, args.seed)
    elif args.phase == "workspace-local-hot":
        read_workspace(args.root, recorder, args.seed, "workspace.local_hot")
    elif args.phase == "workspace-stat":
        stat_workspace(args.root, recorder)
    elif args.phase == "workspace-peer-first":
        read_workspace(args.root, recorder, args.seed, "workspace.peer_first")
    elif args.phase == "workspace-peer-repeat":
        read_workspace(args.root, recorder, args.seed, "workspace.peer_repeat")
    elif args.phase == "workspace-patch":
        patch_workspace(args.root, recorder, args.seed)
    elif args.phase == "workspace-create-delete":
        create_delete_workspace(args.root, recorder, args.seed)
    elif args.phase == "large-create":
        create_large(args.root, recorder, args.seed)
    elif args.phase == "large-local-read":
        read_large(args.root, recorder, args.seed, "sequential_512m.local_read")
    elif args.phase == "large-peer-first":
        read_large(args.root, recorder, args.seed, "sequential_512m.peer_first")
    else:
        read_large(args.root, recorder, args.seed, "sequential_512m.peer_repeat")

    cpu_after = resource.getrusage(resource.RUSAGE_SELF)
    wall_ns = time.perf_counter_ns() - wall_started
    result = {
        "schema": "dms.native-vs-moosefs-workload.v1",
        "phase": args.phase,
        "seed": args.seed,
        "root": str(args.root),
        "wall_ns": wall_ns,
        "process_user_cpu_ns": int((cpu_after.ru_utime - cpu_before.ru_utime) * 1_000_000_000),
        "process_system_cpu_ns": int((cpu_after.ru_stime - cpu_before.ru_stime) * 1_000_000_000),
        "bytes": recorder.bytes,
        "throughput_mib_s": recorder.bytes / (1024 * 1024) / (wall_ns / 1_000_000_000),
        "correctness": True,
        "summary": recorder.summary(),
        "samples": recorder.samples,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

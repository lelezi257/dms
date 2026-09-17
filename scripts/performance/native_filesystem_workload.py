#!/usr/bin/env python3
"""Native Filesystem 与 JuiceFS+DMS 共用的扁平文件 workload。

脚本只通过普通 POSIX 文件接口访问挂载点。数据、文件名和修改内容都由固定
seed 推导，因此 A/B 两台 VM 不需要共享额外 manifest，也不会把 Adapter 私有
行为混入对比。每次只执行一个 phase，便于在 phase 前后抓取 RPC/Metrics 差值。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import resource
import statistics
import time
from pathlib import Path
from typing import Callable


FILES_BY_SIZE = {4096: 140, 65536: 50, 1048576: 30}
MIDDLE_OFFSET = 32 * 1024
MIDDLE_LENGTH = 4 * 1024


def file_name(size: int, index: int) -> str:
    return f"dms-perf-{size}-{index:04d}.bin"


def deterministic_bytes(label: str, size: int, seed: int) -> bytes:
    prefix = hashlib.sha256(f"{seed}:{label}:{size}".encode()).digest()
    repeated = prefix * ((size + len(prefix) - 1) // len(prefix))
    return repeated[:size]


def original_bytes(size: int, index: int, seed: int) -> bytes:
    return deterministic_bytes(file_name(size, index), size, seed)


def patch_bytes(index: int, seed: int) -> bytes:
    return deterministic_bytes(f"patch:{index}", MIDDLE_LENGTH, seed)


def expected_bytes(size: int, index: int, seed: int, patched: bool) -> bytes:
    data = bytearray(original_bytes(size, index, seed))
    if patched and size == 65536:
        data[MIDDLE_OFFSET : MIDDLE_OFFSET + MIDDLE_LENGTH] = patch_bytes(index, seed)
    return bytes(data)


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * fraction)))
    return ordered[index]


class Recorder:
    def __init__(self, phase: str) -> None:
        self.phase = phase
        self.samples: list[dict[str, object]] = []

    def measure(self, case_id: str, size: int, path: Path, action: Callable[[], None]) -> None:
        started = time.perf_counter_ns()
        action()
        elapsed_us = (time.perf_counter_ns() - started) / 1000.0
        self.samples.append(
            {
                "case_id": case_id,
                "size": size,
                "path": path.name,
                "latency_us": elapsed_us,
                "ok": True,
            }
        )

    def summary(self) -> dict[str, dict[str, float | int]]:
        grouped: dict[str, list[float]] = {}
        for sample in self.samples:
            grouped.setdefault(str(sample["case_id"]), []).append(float(sample["latency_us"]))
        return {
            case_id: {
                "samples": len(values),
                "p50_us": percentile(values, 0.50),
                "p95_us": percentile(values, 0.95),
                "p99_us": percentile(values, 0.99),
            }
            for case_id, values in sorted(grouped.items())
        }


def selected_files(size_filter: int | None):
    for size, count in FILES_BY_SIZE.items():
        if size_filter is None or size == size_filter:
            yield size, count


def create(root: Path, recorder: Recorder, seed: int, size_filter: int | None = None) -> None:
    for size, count in selected_files(size_filter):
        for index in range(count):
            path = root / file_name(size, index)
            data = original_bytes(size, index, seed)

            def action(path: Path = path, data: bytes = data) -> None:
                with path.open("xb", buffering=0) as stream:
                    written = stream.write(data)
                    if written != len(data):
                        raise RuntimeError(f"short write: {path}: {written}/{len(data)}")

            recorder.measure(f"create_write.{size}", size, path, action)


def read_all(
    root: Path,
    recorder: Recorder,
    seed: int,
    case_prefix: str,
    patched: bool,
    size_filter: int | None = None,
) -> None:
    for size, count in selected_files(size_filter):
        for index in range(count):
            path = root / file_name(size, index)
            expected = expected_bytes(size, index, seed, patched)

            def action(path: Path = path, expected: bytes = expected) -> None:
                with path.open("rb", buffering=0) as stream:
                    actual = stream.read()
                if actual != expected:
                    raise RuntimeError(f"content mismatch: {path}")

            recorder.measure(f"{case_prefix}.{size}", size, path, action)


def overwrite(root: Path, recorder: Recorder, seed: int) -> None:
    size = 65536
    for index in range(FILES_BY_SIZE[size]):
        path = root / file_name(size, index)
        patch = patch_bytes(index, seed)

        def action(path: Path = path, patch: bytes = patch) -> None:
            descriptor = os.open(path, os.O_RDWR)
            try:
                written = os.pwrite(descriptor, patch, MIDDLE_OFFSET)
            finally:
                os.close(descriptor)
            if written != len(patch):
                raise RuntimeError(f"short pwrite: {path}: {written}/{len(patch)}")

        recorder.measure("middle_overwrite.65536", size, path, action)


def metadata_hot(root: Path, recorder: Recorder, size: int) -> None:
    """反复查询已存在文件属性，单独观察 lookup/getattr 放大。"""
    for index in range(FILES_BY_SIZE[size]):
        path = root / file_name(size, index)

        def action(path: Path = path, size: int = size) -> None:
            attributes = os.stat(path)
            if attributes.st_size != size:
                raise RuntimeError(f"unexpected size: {path}: {attributes.st_size}/{size}")

        recorder.measure(f"metadata_hot.{size}", size, path, action)


def open_close(root: Path, recorder: Recorder, size: int) -> None:
    """只打开并关闭已存在文件，不读取 payload。"""
    for index in range(FILES_BY_SIZE[size]):
        path = root / file_name(size, index)

        def action(path: Path = path) -> None:
            descriptor = os.open(path, os.O_RDONLY)
            os.close(descriptor)

        recorder.measure(f"open_close.{size}", size, path, action)


def readdir(root: Path, recorder: Recorder) -> None:
    """重复枚举根目录，验证单次目录读取的 callback 与 RPC 数。"""
    # 某些对象存储后端会在挂载根目录暴露少量管理文件。这里验证的是同一批
    # 业务文件能否被完整枚举，不能把“后端没有额外目录项”误当成 POSIX 合同。
    expected = {
        file_name(size, index)
        for size, count in FILES_BY_SIZE.items()
        for index in range(count)
    }
    # 目录枚举没有文件大小维度；40 次足以跨过 evaluator 的最小样本门槛。
    for index in range(40):

        def action() -> None:
            entries = {entry.name for entry in os.scandir(root) if entry.is_file()}
            missing = expected - entries
            if missing:
                raise RuntimeError(f"directory misses {len(missing)} workload files")

        recorder.measure("readdir.root", 0, root / f"scan-{index}", action)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument(
        "--phase",
        choices=(
            "create",
            "local-hot",
            "metadata-hot",
            "open-close",
            "readdir",
            "peer-first",
            "peer-hot",
            "overwrite",
            "remote-after-overwrite",
        ),
        required=True,
    )
    parser.add_argument("--seed", type=int, default=6701)
    parser.add_argument("--size", type=int, choices=tuple(FILES_BY_SIZE))
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    # 挂载点由三 VM harness 在计数快照前创建并验证。这里不能再次 resolve/mkdir：
    # 它们会在真正 workload 开始前触发根 inode 的 getattr，使 FUSE callback
    # 账本混入固定的脚本启动开销。
    recorder = Recorder(args.phase)
    cpu_before = resource.getrusage(resource.RUSAGE_SELF)
    wall_started = time.perf_counter_ns()

    if args.phase == "create":
        create(args.root, recorder, args.seed, args.size)
    elif args.phase == "local-hot":
        read_all(root=args.root, recorder=recorder, seed=args.seed, case_prefix="local_hot_read", patched=False, size_filter=args.size)
    elif args.phase == "metadata-hot":
        if args.size is None:
            raise ValueError("metadata-hot requires --size")
        metadata_hot(args.root, recorder, args.size)
    elif args.phase == "open-close":
        if args.size is None:
            raise ValueError("open-close requires --size")
        open_close(args.root, recorder, args.size)
    elif args.phase == "readdir":
        readdir(args.root, recorder)
    elif args.phase == "peer-first":
        read_all(root=args.root, recorder=recorder, seed=args.seed, case_prefix="peer_first_read", patched=False, size_filter=args.size)
    elif args.phase == "peer-hot":
        read_all(root=args.root, recorder=recorder, seed=args.seed, case_prefix="peer_hot_read", patched=False, size_filter=args.size)
    elif args.phase == "overwrite":
        overwrite(args.root, recorder, args.seed)
    else:
        read_all(root=args.root, recorder=recorder, seed=args.seed, case_prefix="remote_after_overwrite", patched=True, size_filter=args.size)

    cpu_after = resource.getrusage(resource.RUSAGE_SELF)
    result = {
        "schema": "dms.native-filesystem-workload-phase.v1",
        "phase": args.phase,
        "seed": args.seed,
        "root": str(args.root),
        "file_count": sum(count for _, count in selected_files(args.size)),
        "files_by_size": {str(size): count for size, count in selected_files(args.size)},
        "wall_ns": time.perf_counter_ns() - wall_started,
        "process_user_cpu_ns": int((cpu_after.ru_utime - cpu_before.ru_utime) * 1_000_000_000),
        "process_system_cpu_ns": int((cpu_after.ru_stime - cpu_before.ru_stime) * 1_000_000_000),
        "correctness": True,
        "summary": recorder.summary(),
        "samples": recorder.samples,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

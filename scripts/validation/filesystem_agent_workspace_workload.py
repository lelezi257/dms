#!/usr/bin/env python3
"""Agent workspace 小文件混合压力 workload。

这个文件刻意不直接依赖 DMS 内部接口，只描述可重放的 POSIX 操作序列和
reference model。真实三 VM 执行由 ``run_m1_agent_workspace.py`` 负责把这些
操作发到不同 FUSE mount 上；这里保留一个本地双 mount 执行入口，便于缩小失败。
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import random
import statistics
import time
from typing import Any


SCHEMA = "dms.filesystem.agent-workspace-workload.v1"
PLAN_SCHEMA = "dms.filesystem.agent-workspace-plan.v1"
DEFAULT_OPERATIONS = 180


def _payload(rng: random.Random, size: int) -> bytes:
    return bytes(rng.getrandbits(8) for _ in range(size))


def _encode(data: bytes) -> str:
    return base64.b64encode(data).decode("ascii")


def decode_payload(value: str) -> bytes:
    return base64.b64decode(value.encode("ascii"), validate=True)


def _candidate_path(index: int, rng: random.Random) -> str:
    shard = rng.randrange(0, 16)
    return f"workspace/pkg-{shard:02d}/task-{index:05d}.bin"


def _rename_path(index: int, rng: random.Random) -> str:
    shard = rng.randrange(0, 16)
    return f"workspace/pkg-{shard:02d}/renamed-{index:05d}.bin"


def build_plan(seed: int, operations: int = DEFAULT_OPERATIONS) -> list[dict[str, Any]]:
    """生成确定性的 workspace 操作序列。

    计划以小文件为主：大多数 payload 低于 8 KiB，少量 64 KiB 文件用来覆盖
    Peer pull 与分段读取，但不把 fio 大文件完整性职责混入本 workload。
    """

    rng = random.Random(seed)
    live_paths: list[str] = []
    next_id = 0
    plan: list[dict[str, Any]] = []

    def choose_live() -> str:
        return live_paths[rng.randrange(0, len(live_paths))]

    for step in range(operations):
        if not live_paths:
            kind = "create"
        else:
            kind = rng.choices(
                ["create", "read", "overwrite", "pwrite", "append", "rename", "unlink", "stat", "readdir"],
                weights=[22, 18, 14, 10, 8, 9, 8, 6, 5],
                k=1,
            )[0]

        if kind == "create":
            path = _candidate_path(next_id, rng)
            next_id += 1
            while path in live_paths:
                path = _candidate_path(next_id, rng)
                next_id += 1
            size = rng.choice([0, 1, 7, 32, 128, 1024, 4096, 8192, 64 * 1024])
            live_paths.append(path)
            plan.append({"op": "create", "path": path, "data": _encode(_payload(rng, size))})
        elif kind == "read":
            plan.append({"op": "read", "path": choose_live()})
        elif kind == "overwrite":
            path = choose_live()
            size = rng.choice([0, 16, 256, 2048, 8192])
            plan.append({"op": "overwrite", "path": path, "data": _encode(_payload(rng, size))})
        elif kind == "pwrite":
            path = choose_live()
            offset = rng.randrange(0, 12 * 1024)
            size = rng.choice([1, 3, 17, 128, 1024])
            plan.append({"op": "pwrite", "path": path, "offset": offset, "data": _encode(_payload(rng, size))})
        elif kind == "append":
            path = choose_live()
            size = rng.choice([1, 15, 128, 1024])
            plan.append({"op": "append", "path": path, "data": _encode(_payload(rng, size))})
        elif kind == "rename":
            src = choose_live()
            dst = _rename_path(next_id, rng)
            next_id += 1
            while dst in live_paths:
                dst = _rename_path(next_id, rng)
                next_id += 1
            live_paths[live_paths.index(src)] = dst
            plan.append({"op": "rename", "src": src, "dst": dst})
        elif kind == "unlink":
            path = choose_live()
            live_paths.remove(path)
            plan.append({"op": "unlink", "path": path})
        elif kind == "stat":
            plan.append({"op": "stat", "path": choose_live()})
        elif kind == "readdir":
            shard = rng.randrange(0, 16)
            plan.append({"op": "readdir", "path": f"workspace/pkg-{shard:02d}"})
        else:  # pragma: no cover - defensive guard for future operation additions.
            raise AssertionError(f"unknown operation: {kind}")
    return plan


def apply_operation(model: dict[str, bytes], operation: dict[str, Any]) -> None:
    op = operation["op"]
    if op in {"create", "overwrite"}:
        model[operation["path"]] = decode_payload(operation["data"])
    elif op == "pwrite":
        path = operation["path"]
        old = bytearray(model[path])
        offset = int(operation["offset"])
        payload = decode_payload(operation["data"])
        if len(old) < offset:
            old.extend(b"\0" * (offset - len(old)))
        end = offset + len(payload)
        if len(old) < end:
            old.extend(b"\0" * (end - len(old)))
        old[offset:end] = payload
        model[path] = bytes(old)
    elif op == "append":
        model[operation["path"]] += decode_payload(operation["data"])
    elif op == "rename":
        model[operation["dst"]] = model.pop(operation["src"])
    elif op == "unlink":
        model.pop(operation["path"])
    elif op in {"read", "stat", "readdir"}:
        return
    else:
        raise AssertionError(f"unknown operation: {op}")


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def model_manifest(model: dict[str, bytes]) -> dict[str, dict[str, Any]]:
    return {
        path: {"size": len(data), "sha256": digest_bytes(data)}
        for path, data in sorted(model.items())
    }


def model_digest(model: dict[str, bytes]) -> str:
    hasher = hashlib.sha256()
    for path, metadata in model_manifest(model).items():
        hasher.update(path.encode())
        hasher.update(b"\0")
        hasher.update(str(metadata["size"]).encode())
        hasher.update(b"\0")
        hasher.update(metadata["sha256"].encode())
        hasher.update(b"\n")
    return hasher.hexdigest()


def scan_tree(root: Path) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    if not root.exists():
        return result
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        rel = path.relative_to(root).as_posix()
        result[rel] = {"size": path.stat().st_size, "sha256": digest_bytes(path.read_bytes())}
    return result


def tree_digest(manifest: dict[str, dict[str, Any]]) -> str:
    hasher = hashlib.sha256()
    for path, metadata in sorted(manifest.items()):
        hasher.update(path.encode())
        hasher.update(b"\0")
        hasher.update(str(metadata["size"]).encode())
        hasher.update(b"\0")
        hasher.update(str(metadata["sha256"]).encode())
        hasher.update(b"\n")
    return hasher.hexdigest()


def latency_summary(samples_ns: list[int]) -> dict[str, float]:
    if not samples_ns:
        return {"count": 0, "p50_us": 0.0, "p95_us": 0.0, "p99_us": 0.0, "max_us": 0.0}
    ordered = sorted(samples_ns)

    def percentile(p: float) -> float:
        if len(ordered) == 1:
            return ordered[0] / 1000.0
        index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * p)))
        return ordered[index] / 1000.0

    return {
        "count": len(samples_ns),
        "p50_us": percentile(0.50),
        "p95_us": percentile(0.95),
        "p99_us": percentile(0.99),
        "max_us": max(ordered) / 1000.0,
        "mean_us": statistics.fmean(ordered) / 1000.0,
    }


class LocalMountRunner:
    """在同一台 Linux VM 的两个 mount 上执行 workload。

    三 VM runner 使用同一份 plan/model，但操作通过 limactl 发到不同 VM；这个本地
    runner 主要用于快速复现和单元测试，不承担最终三 VM 结论。
    """

    def __init__(self, mount_a: Path, mount_b: Path, plan: list[dict[str, Any]]) -> None:
        self.mount_a = mount_a
        self.mount_b = mount_b
        self.plan = plan
        self.model: dict[str, bytes] = {}
        self.latencies: list[dict[str, Any]] = []
        self.operation_counts: dict[str, int] = {}
        self.cross_node_verifications = 0

    def _path(self, mount: Path, rel: str) -> Path:
        path = mount / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def _verify_path_on_b(self, rel: str) -> None:
        expected = self.model.get(rel)
        path = self.mount_b / rel
        if expected is None:
            if path.exists():
                raise AssertionError(f"{rel} should be absent on peer mount")
        elif path.read_bytes() != expected:
            raise AssertionError(f"{rel} peer bytes mismatch")
        self.cross_node_verifications += 1

    def run(self) -> dict[str, Any]:
        for index, operation in enumerate(self.plan):
            started = time.perf_counter_ns()
            op = operation["op"]
            if op in {"create", "overwrite"}:
                self._path(self.mount_a, operation["path"]).write_bytes(decode_payload(operation["data"]))
            elif op == "pwrite":
                path = self._path(self.mount_a, operation["path"])
                with path.open("r+b") as stream:
                    stream.seek(int(operation["offset"]))
                    stream.write(decode_payload(operation["data"]))
            elif op == "append":
                with self._path(self.mount_a, operation["path"]).open("ab") as stream:
                    stream.write(decode_payload(operation["data"]))
            elif op == "rename":
                dst = self._path(self.mount_a, operation["dst"])
                os.replace(self.mount_a / operation["src"], dst)
            elif op == "unlink":
                (self.mount_a / operation["path"]).unlink()
            elif op == "read":
                _ = (self.mount_b / operation["path"]).read_bytes()
            elif op == "stat":
                _ = (self.mount_b / operation["path"]).stat()
            elif op == "readdir":
                directory = self.mount_b / operation["path"]
                if directory.exists():
                    _ = sorted(child.name for child in directory.iterdir())
            else:
                raise AssertionError(f"unknown operation: {op}")

            apply_operation(self.model, operation)
            if op in {"create", "overwrite", "pwrite", "append", "read", "stat"}:
                self._verify_path_on_b(operation["path"])
            elif op == "rename":
                self._verify_path_on_b(operation["src"])
                self._verify_path_on_b(operation["dst"])
            elif op == "unlink":
                self._verify_path_on_b(operation["path"])
            elif op == "readdir":
                self.cross_node_verifications += 1

            elapsed = time.perf_counter_ns() - started
            self.latencies.append({"index": index, "operation": op, "elapsed_ns": elapsed})
            self.operation_counts[op] = self.operation_counts.get(op, 0) + 1

        model_manifest_value = model_manifest(self.model)
        tree_a = scan_tree(self.mount_a)
        tree_b = scan_tree(self.mount_b)
        return {
            "schema": SCHEMA,
            "status": "passed",
            "deployment": "single-process",
            "operation_count": len(self.plan),
            "operation_counts": self.operation_counts,
            "cross_node_verifications": self.cross_node_verifications,
            "latency_summary": latency_summary([item["elapsed_ns"] for item in self.latencies]),
            "model_digest": model_digest(self.model),
            "tree_digest_a": tree_digest(tree_a),
            "tree_digest_b": tree_digest(tree_b),
            "model_manifest": model_manifest_value,
            "tree_a": tree_a,
            "tree_b": tree_b,
            "latencies": self.latencies,
        }


def write_plan(path: Path, seed: int, operations: int, plan: list[dict[str, Any]]) -> None:
    path.write_text(
        json.dumps(
            {
                "schema": PLAN_SCHEMA,
                "seed": seed,
                "operation_count": operations,
                "plan": plan,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mount-a", type=Path, required=True)
    parser.add_argument("--mount-b", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=20260916)
    parser.add_argument("--operations", type=int, default=DEFAULT_OPERATIONS)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--plan-output", type=Path)
    args = parser.parse_args()

    plan = build_plan(args.seed, args.operations)
    if args.plan_output:
        write_plan(args.plan_output, args.seed, args.operations, plan)
    result = LocalMountRunner(args.mount_a, args.mount_b, plan).run()
    result.update({"seed": args.seed, "plan_schema": PLAN_SCHEMA})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

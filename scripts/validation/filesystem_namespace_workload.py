#!/usr/bin/env python3
"""M1 共享 namespace 的 POSIX 语义 workload。

这个脚本只通过普通文件系统调用访问两个挂载点，不依赖 DMS 内部 API。它的作用是把
“A 节点修改目录树、B 节点按同一 namespace 观察到变化”固定成可重复验证的合同。
实现侧可以调整 Meta、Node、FUSE 的内部结构，但不能让这里的用户语义退化。
"""

from __future__ import annotations

import argparse
import errno
import json
import os
import time
from pathlib import Path
from typing import Callable


PAYLOAD_PREFIX = b"dms-m1-namespace:"
REQUIRED_OPERATIONS = {
    "mkdir_create_read",
    "rename",
    "rename_type_and_cycle_guards",
    "unlink_rmdir",
    "recovery_anchor",
    "unlink_regular_file",
}


def deterministic_payload(round_id: int, suffix: str) -> bytes:
    return PAYLOAD_PREFIX + f"{round_id}:{suffix}".encode()


def wait_until(name: str, deadline_seconds: float, check: Callable[[], bool]) -> int:
    """轮询直到跨节点异步可见。

    FUSE/Meta Watch 传播不是 Python 语言语义的一部分；这里显式等待，是为了把系统
    “最终在租约/失效边界内收敛”这件事转成稳定 E2E，不用固定 sleep 掩盖慢路径。
    """

    deadline = time.monotonic() + deadline_seconds
    attempts = 0
    while True:
        attempts += 1
        if check():
            return attempts
        if time.monotonic() >= deadline:
            raise TimeoutError(f"timed out waiting for {name}")
        time.sleep(0.02)


def names(path: Path) -> set[str]:
    try:
        return {entry.name for entry in os.scandir(path)}
    except FileNotFoundError:
        return set()


def read_matches_during_recovery(path: Path, expected: bytes) -> bool:
    """在 Node 重新向 Meta 续租的窗口内，把暂时不可达视为“尚未恢复完成”。

    Meta 恢复目录与副本目录后，存活 Node 仍需通过下一次 heartbeat 重新证明可达性。
    `EHOSTUNREACH` 因此可以在 deadline 内重试；其它 I/O 错误仍立即暴露，避免验证器
    把真实数据损坏误判成普通恢复延迟。
    """

    try:
        return path.exists() and path.read_bytes() == expected
    except OSError as error:
        if error.errno in (errno.ENOENT, errno.EHOSTUNREACH, errno.ESTALE):
            return False
        raise


def assert_errno(operation: Callable[[], object], expected: int, label: str) -> None:
    try:
        operation()
    except OSError as error:
        if error.errno == expected:
            return
        raise AssertionError(f"{label}: expected errno {expected}, got {error.errno}") from error
    raise AssertionError(f"{label}: expected errno {expected}, operation succeeded")


def create_nested_file(mount_a: Path, mount_b: Path, round_id: int) -> dict[str, object]:
    """验证 mkdir + create/write + readdir + cross-node read."""

    project_a = mount_a / f"project-{round_id:03d}"
    src_a = project_a / "src"
    pkg_a = src_a / "pkg"
    for directory in (project_a, src_a, pkg_a):
        directory.mkdir()

    wait_attempts = wait_until(
        "B sees nested directory",
        10,
        lambda: "pkg" in names(mount_b / f"project-{round_id:03d}" / "src"),
    )

    payload = deterministic_payload(round_id, "created")
    file_a = pkg_a / "module.txt"
    file_a.write_bytes(payload)
    file_b = mount_b / f"project-{round_id:03d}" / "src" / "pkg" / "module.txt"
    read_attempts = wait_until(
        "B reads created file",
        10,
        lambda: file_b.exists() and file_b.read_bytes() == payload,
    )
    return {
        "operation": "mkdir_create_read",
        "directory_wait_attempts": wait_attempts,
        "read_wait_attempts": read_attempts,
        "bytes": len(payload),
    }


def rename_and_verify(mount_a: Path, mount_b: Path, round_id: int) -> dict[str, object]:
    """验证 rename 后旧名消失、新名可读。"""

    base_a = mount_a / f"project-{round_id:03d}" / "src" / "pkg"
    base_b = mount_b / f"project-{round_id:03d}" / "src" / "pkg"
    old_a = base_a / "module.txt"
    new_a = base_a / "renamed.txt"
    old_b = base_b / "module.txt"
    new_b = base_b / "renamed.txt"
    os.rename(old_a, new_a)

    attempts = wait_until(
        "B sees rename",
        10,
        lambda: (not old_b.exists()) and new_b.exists() and new_b.read_bytes().startswith(PAYLOAD_PREFIX),
    )
    return {"operation": "rename", "wait_attempts": attempts}


def verify_rename_boundaries(mount_a: Path, round_id: int) -> dict[str, object]:
    """验证目录环和 file/dir 交叉覆盖会返回准确的 POSIX errno。"""

    base = mount_a / f"project-{round_id:03d}" / "rename-boundaries"
    base.mkdir()

    cycle_parent = base / "cycle-parent"
    cycle_child = cycle_parent / "child"
    cycle_child.mkdir(parents=True)
    assert_errno(
        lambda: os.rename(cycle_parent, cycle_child / "cycle-parent"),
        errno.EINVAL,
        "rename directory into its descendant",
    )

    source_file = base / "source-file"
    target_dir = base / "target-dir"
    source_file.write_bytes(deterministic_payload(round_id, "rename-file"))
    target_dir.mkdir()
    assert_errno(
        lambda: os.replace(source_file, target_dir),
        errno.EISDIR,
        "replace directory with regular file",
    )

    source_dir = base / "source-dir"
    target_file = base / "target-file"
    source_dir.mkdir()
    target_file.write_bytes(deterministic_payload(round_id, "rename-target-file"))
    assert_errno(
        lambda: os.replace(source_dir, target_file),
        errno.ENOTDIR,
        "replace regular file with directory",
    )

    source_file.unlink()
    target_dir.rmdir()
    source_dir.rmdir()
    target_file.unlink()
    cycle_child.rmdir()
    cycle_parent.rmdir()
    base.rmdir()
    return {"operation": "rename_type_and_cycle_guards"}


def remove_and_rmdir(mount_a: Path, mount_b: Path, round_id: int) -> dict[str, object]:
    """验证 unlink、非空 rmdir=ENOTEMPTY、空目录 rmdir 成功。"""

    sandbox_a = mount_a / f"project-{round_id:03d}" / "tmp"
    sandbox_b = mount_b / f"project-{round_id:03d}" / "tmp"
    sandbox_a.mkdir()
    child_a = sandbox_a / "child.txt"
    child_b = sandbox_b / "child.txt"
    child_a.write_bytes(deterministic_payload(round_id, "tmp-child"))

    wait_until("B sees tmp child", 10, lambda: child_b.exists())
    assert_errno(lambda: sandbox_a.rmdir(), errno.ENOTEMPTY, "rmdir non-empty directory")
    child_a.unlink()
    wait_until("B sees unlinked child disappear", 10, lambda: not child_b.exists())
    sandbox_a.rmdir()
    attempts = wait_until("B sees empty directory removed", 10, lambda: not sandbox_b.exists())
    return {"operation": "unlink_rmdir", "wait_attempts": attempts}


def keep_recovery_anchor(mount_a: Path, mount_b: Path, round_id: int) -> dict[str, object]:
    """留下一个重启后仍需通过 Meta namespace 找到的锚点文件。"""

    stable_a = mount_a / "stable"
    stable_a.mkdir(exist_ok=True)
    round_dir_a = stable_a / f"round-{round_id:03d}"
    round_dir_a.mkdir()
    anchor_a = round_dir_a / "anchor.txt"
    payload = deterministic_payload(round_id, "anchor")
    anchor_a.write_bytes(payload)
    anchor_b = mount_b / "stable" / f"round-{round_id:03d}" / "anchor.txt"
    attempts = wait_until(
        "B reads recovery anchor",
        10,
        lambda: anchor_b.exists() and anchor_b.read_bytes() == payload,
    )
    return {
        "operation": "recovery_anchor",
        "wait_attempts": attempts,
        "path": f"/stable/round-{round_id:03d}/anchor.txt",
        "bytes": len(payload),
    }


def cleanup_round(mount_a: Path, mount_b: Path, round_id: int) -> dict[str, object]:
    """验证 renamed 文件 unlink 后目录项消失，保留 stable 锚点给重启恢复。"""

    renamed_a = mount_a / f"project-{round_id:03d}" / "src" / "pkg" / "renamed.txt"
    renamed_b = mount_b / f"project-{round_id:03d}" / "src" / "pkg" / "renamed.txt"
    renamed_a.unlink()
    attempts = wait_until("B sees renamed file unlinked", 10, lambda: not renamed_b.exists())
    return {"operation": "unlink_regular_file", "wait_attempts": attempts}


def run_round(mount_a: Path, mount_b: Path, round_id: int) -> dict[str, object]:
    started = time.perf_counter_ns()
    checks = [
        create_nested_file(mount_a, mount_b, round_id),
        rename_and_verify(mount_a, mount_b, round_id),
        verify_rename_boundaries(mount_a, round_id),
        remove_and_rmdir(mount_a, mount_b, round_id),
        keep_recovery_anchor(mount_a, mount_b, round_id),
        cleanup_round(mount_a, mount_b, round_id),
    ]
    return {
        "round": round_id,
        "checks": checks,
        "elapsed_ms": (time.perf_counter_ns() - started) / 1_000_000,
    }


def verify_recovery(mount_b: Path, round_id: int) -> dict[str, object]:
    anchor = mount_b / "stable" / f"round-{round_id:03d}" / "anchor.txt"
    expected = deterministic_payload(round_id, "anchor")
    attempts = wait_until(
        "B reads anchor after Meta/Node restart",
        15,
        lambda: read_matches_during_recovery(anchor, expected),
    )
    return {
        "schema": "dms.filesystem.namespace-recovery.v1",
        "round": round_id,
        "path": f"/stable/round-{round_id:03d}/anchor.txt",
        "wait_attempts": attempts,
        "bytes": len(expected),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mount-a", type=Path, required=True)
    parser.add_argument("--mount-b", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=20)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--recovery-only", action="store_true")
    parser.add_argument("--recovery-round", type=int)
    args = parser.parse_args()

    if args.recovery_only:
        if args.recovery_round is None:
            raise ValueError("--recovery-only requires --recovery-round")
        result = verify_recovery(args.mount_b, args.recovery_round)
    else:
        for mount in (args.mount_a, args.mount_b):
            if not mount.is_dir():
                raise FileNotFoundError(mount)
        rounds = [run_round(args.mount_a, args.mount_b, round_id) for round_id in range(args.rounds)]
        result = {
            "schema": "dms.filesystem.namespace-workload.v1",
            "rounds": args.rounds,
            "passed_rounds": len(rounds),
            "round_results": rounds,
            "recovery_round": args.rounds - 1,
        }

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

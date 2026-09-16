#!/usr/bin/env python3
"""M1.3 文件身份与生命周期 POSIX workload。

脚本只通过普通文件系统调用访问 FUSE mount，不调用 DMS 内部接口。它覆盖文件身份
相关的用户语义：hardlink 共享 inode、unlink 一个名字不删除另一个名字、unlink-open
保护已打开文件、symlink/readlink 保留精确 target bytes，以及最后一个名字消失后的
orphan 用户可见行为。
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any


SCHEMA = "dms.filesystem.identity-lifecycle-workload.v1"
RECOVERY_SCHEMA = "dms.filesystem.identity-lifecycle-recovery.v1"
PAYLOAD = b"dms-identity-v1"
RECOVERY_PAYLOAD = b"dms-recovery-v1"
SYMLINK_TARGET = "target.txt"
RECOVERY_SYMLINK_TARGET = "recovery-target.txt"


def stat_summary(path: Path) -> dict[str, int]:
    stat = path.lstat()
    return {
        "inode": int(stat.st_ino),
        "mode": int(stat.st_mode),
        "nlink": int(stat.st_nlink),
        "size": int(stat.st_size),
    }


def hardlink_same_inode(root_a: Path, root_b: Path) -> dict[str, Any]:
    base_a = root_a / "hardlink"
    base_b = root_b / "hardlink"
    base_a.mkdir(exist_ok=True)
    source_a = base_a / "a.txt"
    linked_a = base_a / "b.txt"
    source_b = base_b / "a.txt"
    linked_b = base_b / "b.txt"
    source_a.write_bytes(PAYLOAD)
    os.link(source_a, linked_a)

    source_stat = source_b.stat()
    linked_stat = linked_b.stat()
    if source_stat.st_ino != linked_stat.st_ino:
        raise AssertionError("hardlink did not preserve inode identity")
    if source_stat.st_nlink != 2 or linked_stat.st_nlink != 2:
        raise AssertionError("hardlink nlink is not 2")
    if linked_b.read_bytes() != PAYLOAD:
        raise AssertionError("hardlink target did not read source payload")
    return {
        "operation": "hardlink_same_inode",
        "source_inode": int(source_stat.st_ino),
        "linked_inode": int(linked_stat.st_ino),
        "same_inode": True,
        "source_nlink": int(source_stat.st_nlink),
        "linked_nlink": int(linked_stat.st_nlink),
        "remote_stat_attempts": 1,
        "remote_read_bytes": len(PAYLOAD),
    }


def unlink_one_link_keeps_other_link(root_a: Path, root_b: Path) -> dict[str, Any]:
    removed_a = root_a / "hardlink" / "a.txt"
    removed_b = root_b / "hardlink" / "a.txt"
    remaining_b = root_b / "hardlink" / "b.txt"
    os.unlink(removed_a)
    if removed_b.exists():
        raise AssertionError("removed hardlink name is still visible")
    if remaining_b.read_bytes() != PAYLOAD:
        raise AssertionError("remaining hardlink did not retain payload")
    stat = remaining_b.stat()
    if stat.st_nlink != 1:
        raise AssertionError("remaining hardlink nlink is not 1")
    return {
        "operation": "unlink_one_link_keeps_other_link",
        "removed_exists": False,
        "remaining_inode": int(stat.st_ino),
        "remaining_nlink": int(stat.st_nlink),
        "remaining_bytes": len(PAYLOAD),
    }


def unlink_open_keeps_file_readable_until_close(root_a: Path, root_b: Path) -> dict[str, Any]:
    base_a = root_a / "unlink-open"
    base_b = root_b / "unlink-open"
    base_a.mkdir(exist_ok=True)
    path_a = base_a / "open.txt"
    path_b = base_b / "open.txt"
    path_a.write_bytes(PAYLOAD)
    fd = os.open(path_a, os.O_RDONLY)
    try:
        os.unlink(path_a)
        namespace_visible = path_b.exists()
        os.lseek(fd, 0, os.SEEK_SET)
        data = os.read(fd, len(PAYLOAD))
        if data != PAYLOAD:
            raise AssertionError("opened fd did not retain bytes after unlink")
        if namespace_visible:
            raise AssertionError("unlinked open file is still visible by name")
    finally:
        os.close(fd)
    return {
        "operation": "unlink_open_keeps_file_readable_until_close",
        "namespace_visible_after_unlink": False,
        "fd_read_bytes": len(PAYLOAD),
        "close_completed": True,
    }


def symlink_readlink_exact_target(root_a: Path, root_b: Path) -> dict[str, Any]:
    base_a = root_a / "symlink"
    base_b = root_b / "symlink"
    base_a.mkdir(exist_ok=True)
    target_a = base_a / SYMLINK_TARGET
    target_b = base_b / SYMLINK_TARGET
    link_a = base_a / "link"
    link_b = base_b / "link"
    target_a.write_bytes(PAYLOAD)
    os.symlink(SYMLINK_TARGET, link_a)
    observed = os.readlink(link_b)
    if observed != SYMLINK_TARGET:
        raise AssertionError(f"readlink mismatch: {observed!r}")
    if link_b.read_bytes() != PAYLOAD:
        raise AssertionError("symlink did not resolve to target payload")
    os.unlink(link_a)
    if not target_b.exists() or target_b.read_bytes() != PAYLOAD:
        raise AssertionError("unlinking symlink affected target")
    return {
        "operation": "symlink_readlink_exact_target",
        "readlink": observed,
        "readlink_matches": True,
        "readlink_bytes": len(observed.encode()),
        "target_survived_unlink": True,
        "target_bytes": len(PAYLOAD),
    }


def orphan_lifecycle_observed(root_a: Path, root_b: Path) -> dict[str, Any]:
    base_a = root_a / "orphan"
    base_b = root_b / "orphan"
    base_a.mkdir(exist_ok=True)
    path_a = base_a / "last.txt"
    path_b = base_b / "last.txt"
    path_a.write_bytes(PAYLOAD)
    fd = os.open(path_a, os.O_RDONLY)
    try:
        inode = int(os.fstat(fd).st_ino)
        os.unlink(path_a)
        namespace_visible = path_b.exists()
        os.lseek(fd, 0, os.SEEK_SET)
        protected = os.read(fd, len(PAYLOAD)) == PAYLOAD
    finally:
        os.close(fd)
    if namespace_visible:
        raise AssertionError("last-link unlink left a namespace entry")
    if not protected:
        raise AssertionError("open reference did not protect orphan bytes")
    return {
        "operation": "orphan_lifecycle_observed",
        "namespace_visible_after_last_unlink": False,
        "open_ref_protected_read": True,
        # runner 以该 inode 关联 Node 引用迁移日志与 Meta durable reap 日志。
        "inode": inode,
    }


def create_recovery_anchors(root_a: Path, root_b: Path) -> dict[str, Any]:
    base_a = root_a / "identity"
    base_b = root_b / "identity"
    base_a.mkdir(exist_ok=True)
    hardlink_a = base_a / "recovery-hardlink-a"
    hardlink_b = base_a / "recovery-hardlink-b"
    remote_hardlink_b = base_b / "recovery-hardlink-b"
    hardlink_a.write_bytes(RECOVERY_PAYLOAD)
    os.link(hardlink_a, hardlink_b)
    os.unlink(hardlink_a)
    if remote_hardlink_b.read_bytes() != RECOVERY_PAYLOAD:
        raise AssertionError("recovery hardlink anchor is not remotely visible")

    target_a = base_a / RECOVERY_SYMLINK_TARGET
    symlink_a = base_a / "recovery-symlink"
    symlink_b = base_b / "recovery-symlink"
    target_a.write_bytes(RECOVERY_PAYLOAD)
    os.symlink(RECOVERY_SYMLINK_TARGET, symlink_a)
    if os.readlink(symlink_b) != RECOVERY_SYMLINK_TARGET:
        raise AssertionError("recovery symlink anchor is not remotely visible")

    orphan_a = base_a / "recovery-orphan"
    orphan_b = base_b / "recovery-orphan"
    orphan_a.write_bytes(RECOVERY_PAYLOAD)
    os.unlink(orphan_a)
    if orphan_b.exists():
        raise AssertionError("recovery orphan is visible by name")

    return {
        "recovery_hardlink_path": "/identity/recovery-hardlink-b",
        "recovery_hardlink_bytes": len(RECOVERY_PAYLOAD),
        "recovery_symlink_path": "/identity/recovery-symlink",
        "recovery_symlink_target": RECOVERY_SYMLINK_TARGET,
    }


def run(mount_a: Path, mount_b: Path) -> dict[str, Any]:
    checks = [
        hardlink_same_inode(mount_a, mount_b),
        unlink_one_link_keeps_other_link(mount_a, mount_b),
        unlink_open_keeps_file_readable_until_close(mount_a, mount_b),
        symlink_readlink_exact_target(mount_a, mount_b),
        orphan_lifecycle_observed(mount_a, mount_b),
    ]
    recovery = create_recovery_anchors(mount_a, mount_b)
    return {
        "schema": SCHEMA,
        "passed_operations": len(checks),
        "checks": checks,
        **recovery,
    }


def verify_recovery(mount_b: Path) -> dict[str, Any]:
    hardlink = mount_b / "identity" / "recovery-hardlink-b"
    symlink = mount_b / "identity" / "recovery-symlink"
    orphan = mount_b / "identity" / "recovery-orphan"
    if hardlink.read_bytes() != RECOVERY_PAYLOAD:
        raise AssertionError("recovered hardlink bytes mismatch")
    observed = os.readlink(symlink)
    if observed != RECOVERY_SYMLINK_TARGET:
        raise AssertionError("recovered symlink target mismatch")
    if orphan.exists():
        raise AssertionError("recovered orphan resurrected in namespace")
    return {
        "schema": RECOVERY_SCHEMA,
        "hardlink_remaining_path": "/identity/recovery-hardlink-b",
        "hardlink_bytes": len(RECOVERY_PAYLOAD),
        "symlink_path": "/identity/recovery-symlink",
        "readlink": observed,
        "orphan_namespace_visible": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mount-a", type=Path, required=True)
    parser.add_argument("--mount-b", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--recovery-only", action="store_true")
    args = parser.parse_args()

    if args.recovery_only:
        result = verify_recovery(args.mount_b)
    else:
        for mount in (args.mount_a, args.mount_b):
            if not mount.is_dir():
                raise FileNotFoundError(mount)
        result = run(args.mount_a, args.mount_b)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

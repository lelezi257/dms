#!/usr/bin/env python3
"""通过两个真实 FUSE mount 验证跨 Node POSIX record lock 与 flock。"""

from __future__ import annotations

import argparse
import errno
import fcntl
import json
import multiprocessing
import os
import select
import signal
import struct
import subprocess
import time
from pathlib import Path


FLOCK_FORMAT = "@hhqqi"


def blocking_lock(path: str, connection) -> None:
    fd = os.open(path, os.O_RDWR)
    try:
        connection.send("waiting")
        fcntl.lockf(fd, fcntl.LOCK_EX)
        connection.send("acquired")
        fcntl.lockf(fd, fcntl.LOCK_UN)
    finally:
        os.close(fd)
        connection.close()


def expect_conflict(call) -> None:
    try:
        call()
    except OSError as error:
        if error.errno not in (errno.EACCES, errno.EAGAIN):
            raise
    else:
        raise AssertionError("conflicting lock unexpectedly succeeded")


def receive(connection, expected: str, timeout: float = 5.0) -> None:
    if not connection.poll(timeout):
        raise TimeoutError(f"timed out waiting for child state {expected!r}")
    actual = connection.recv()
    if actual != expected:
        raise AssertionError(f"expected child state {expected!r}, got {actual!r}")


def receive_line(process: subprocess.Popen[str], expected: str, timeout: float = 5.0) -> None:
    assert process.stdout is not None
    readable, _, _ = select.select([process.stdout], [], [], timeout)
    if not readable:
        raise TimeoutError(f"timed out waiting for helper state {expected!r}")
    actual = process.stdout.readline().strip()
    if actual != expected:
        stderr = process.stderr.read() if process.poll() is not None and process.stderr else ""
        raise AssertionError(
            f"expected helper state {expected!r}, got {actual!r}; stderr={stderr!r}"
        )


def run(mount_a: Path, mount_b: Path, interrupt_helper: Path) -> dict[str, object]:
    path_a = mount_a / "distributed-locks.bin"
    path_b = mount_b / "distributed-locks.bin"
    path_a.write_bytes(b"x" * 4096)
    deadline = time.monotonic() + 5
    while not path_b.exists():
        if time.monotonic() >= deadline:
            raise TimeoutError("lock test file did not become visible on peer mount")
        time.sleep(0.01)

    fd_a = os.open(path_a, os.O_RDWR)
    fd_b = os.open(path_b, os.O_RDWR)
    checks: list[str] = []
    try:
        # F_SETLK + F_GETLK：另一个 Node 必须看到精确冲突，而不是只在本机生效。
        fcntl.lockf(fd_a, fcntl.LOCK_EX | fcntl.LOCK_NB, 100, 0)
        expect_conflict(lambda: fcntl.lockf(fd_b, fcntl.LOCK_EX | fcntl.LOCK_NB, 100, 0))
        query = struct.pack(FLOCK_FORMAT, fcntl.F_WRLCK, os.SEEK_SET, 0, 100, 0)
        lock_type, _, start, length, owner_pid = struct.unpack(
            FLOCK_FORMAT, fcntl.fcntl(fd_b, fcntl.F_GETLK, query)
        )
        assert lock_type == fcntl.F_WRLCK and start == 0 and length == 100
        assert owner_pid != 0
        checks.append("cross_node_getlk")

        # 区间解锁必须切开原锁，不能误放开前后两段。
        fcntl.lockf(fd_a, fcntl.LOCK_UN, 20, 40)
        fcntl.lockf(fd_b, fcntl.LOCK_EX | fcntl.LOCK_NB, 20, 40)
        expect_conflict(lambda: fcntl.lockf(fd_b, fcntl.LOCK_EX | fcntl.LOCK_NB, 20, 0))
        fcntl.lockf(fd_b, fcntl.LOCK_UN, 20, 40)
        fcntl.lockf(fd_a, fcntl.LOCK_UN)
        checks.append("split_unlock")

        # 多个共享锁兼容，排它锁与它们冲突。
        fcntl.lockf(fd_a, fcntl.LOCK_SH | fcntl.LOCK_NB)
        fcntl.lockf(fd_b, fcntl.LOCK_SH | fcntl.LOCK_NB)
        expect_conflict(lambda: fcntl.lockf(fd_b, fcntl.LOCK_EX | fcntl.LOCK_NB))
        fcntl.lockf(fd_b, fcntl.LOCK_UN)
        fcntl.lockf(fd_a, fcntl.LOCK_UN)
        checks.append("shared_compatibility")

        # flock 复用同一权威冲突表，但使用 whole-file owner 生命周期。
        fcntl.flock(fd_a, fcntl.LOCK_EX | fcntl.LOCK_NB)
        expect_conflict(lambda: fcntl.flock(fd_b, fcntl.LOCK_EX | fcntl.LOCK_NB))
        fcntl.flock(fd_a, fcntl.LOCK_UN)
        checks.append("cross_node_flock")

        context = multiprocessing.get_context("spawn")
        parent, child = context.Pipe()
        process = context.Process(target=blocking_lock, args=(str(path_b), child))
        fcntl.lockf(fd_a, fcntl.LOCK_EX | fcntl.LOCK_NB)
        process.start()
        child.close()
        receive(parent, "waiting")
        assert not parent.poll(0.2), "blocking lock acquired before unlock"
        fcntl.lockf(fd_a, fcntl.LOCK_UN)
        receive(parent, "acquired")
        process.join(5)
        assert process.exitcode == 0
        parent.close()
        checks.append("blocking_wakeup")

        # FUSE_INTERRUPT 必须从 Node 取消 Meta waiter；子进程保持 fd 打开，确保
        # 后续成功不是依赖 close/release 的兜底清理。
        process = subprocess.Popen(
            [str(interrupt_helper), str(path_b)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        fcntl.lockf(fd_a, fcntl.LOCK_EX | fcntl.LOCK_NB)
        receive_line(process, "waiting")
        time.sleep(0.1)
        os.kill(process.pid, signal.SIGUSR1)
        receive_line(process, "interrupted")
        fcntl.lockf(fd_a, fcntl.LOCK_UN)
        time.sleep(0.2)
        fcntl.lockf(fd_b, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.lockf(fd_b, fcntl.LOCK_UN)
        assert process.stdin is not None
        process.stdin.write("exit\n")
        process.stdin.flush()
        assert process.wait(5) == 0
        checks.append("interrupt_cancels_waiter")
    finally:
        os.close(fd_b)
        os.close(fd_a)

    return {
        "schema": "dms.filesystem.distributed-locks.v1",
        "status": "passed",
        "checks": checks,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mount-a", type=Path, required=True)
    parser.add_argument("--mount-b", type=Path, required=True)
    parser.add_argument("--interrupt-helper", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = run(args.mount_a, args.mount_b, args.interrupt_helper)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

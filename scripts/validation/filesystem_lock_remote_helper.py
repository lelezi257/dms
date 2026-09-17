#!/usr/bin/env python3
"""三 VM 文件锁验收使用的最小远端进程。

每个动作都通过真实挂载点调用 POSIX fcntl；状态文件只负责让控制器知道进程已经
进入哪个阶段，不参与 DMS 协议或锁判定。
"""

from __future__ import annotations

import argparse
import errno
import fcntl
import json
import os
import struct
import time
from pathlib import Path


FLOCK_FORMAT = "@hhqqi"


def wait_for(path: Path, timeout: float = 30.0) -> None:
    deadline = time.monotonic() + timeout
    while not path.exists():
        if time.monotonic() >= deadline:
            raise TimeoutError(f"timed out waiting for {path}")
        time.sleep(0.02)


def acquire(fd: int, *, wait: bool, start: int, length: int) -> None:
    flags = fcntl.LOCK_EX | (0 if wait else fcntl.LOCK_NB)
    fcntl.lockf(fd, flags, length, start)


def hold(args: argparse.Namespace) -> int:
    fd = os.open(args.path, os.O_RDWR)
    try:
        acquire(fd, wait=True, start=args.start, length=args.length)
        args.ready.write_text("ready\n", encoding="utf-8")
        wait_for(args.release)
        fcntl.lockf(fd, fcntl.LOCK_UN, args.length, args.start)
    finally:
        os.close(fd)
    return 0


def blocking_wait(args: argparse.Namespace) -> int:
    fd = os.open(args.path, os.O_RDWR)
    try:
        args.ready.write_text("waiting\n", encoding="utf-8")
        acquire(fd, wait=True, start=args.start, length=args.length)
        args.acquired.write_text("acquired\n", encoding="utf-8")
        wait_for(args.release)
        fcntl.lockf(fd, fcntl.LOCK_UN, args.length, args.start)
    finally:
        os.close(fd)
    return 0


def try_lock(args: argparse.Namespace) -> int:
    fd = os.open(args.path, os.O_RDWR)
    result: dict[str, object]
    try:
        try:
            acquire(fd, wait=False, start=args.start, length=args.length)
        except OSError as error:
            result = {"status": "error", "errno": error.errno}
        else:
            result = {"status": "acquired"}
            fcntl.lockf(fd, fcntl.LOCK_UN, args.length, args.start)
    finally:
        os.close(fd)
    print(json.dumps(result, sort_keys=True))
    return 0


def query(args: argparse.Namespace) -> int:
    fd = os.open(args.path, os.O_RDWR)
    try:
        request = struct.pack(
            FLOCK_FORMAT,
            fcntl.F_WRLCK,
            os.SEEK_SET,
            args.start,
            args.length,
            0,
        )
        lock_type, _, start, length, pid = struct.unpack(
            FLOCK_FORMAT, fcntl.fcntl(fd, fcntl.F_GETLK, request)
        )
    finally:
        os.close(fd)
    print(
        json.dumps(
            {
                "status": "unlocked" if lock_type == fcntl.F_UNLCK else "conflict",
                "start": start,
                "length": length,
                "pid": pid,
            },
            sort_keys=True,
        )
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    for name in ("hold", "wait", "try", "query"):
        child = subparsers.add_parser(name)
        child.add_argument("path")
        child.add_argument("--start", type=int, default=0)
        child.add_argument("--length", type=int, default=0)
    hold_parser = subparsers.choices["hold"]
    hold_parser.add_argument("--ready", type=Path, required=True)
    hold_parser.add_argument("--release", type=Path, required=True)
    wait_parser = subparsers.choices["wait"]
    wait_parser.add_argument("--ready", type=Path, required=True)
    wait_parser.add_argument("--acquired", type=Path, required=True)
    wait_parser.add_argument("--release", type=Path, required=True)
    args = parser.parse_args()
    if args.start < 0 or args.length < 0:
        parser.error("start and length must be non-negative")
    return {
        "hold": hold,
        "wait": blocking_wait,
        "try": try_lock,
        "query": query,
    }[args.command](args)


if __name__ == "__main__":
    raise SystemExit(main())

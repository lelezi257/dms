#!/usr/bin/env python3
"""Linux-only diagnostic: count home-server sync syscalls for remote read closes."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time

import accept


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--workdir", type=Path, required=True)
    parser.add_argument("--count", type=int, default=200)
    parser.add_argument("--mode", choices=("read", "write-sync", "write-nosync", "write-after-sync"), default="read")
    parser.add_argument("--expect-home-sync-calls", type=int)
    args = parser.parse_args()
    options = accept.parse_args([
        "--source-root", str(Path(__file__).resolve().parents[2]),
        "--binary", str(args.binary),
        "--backend", "p2p",
        "--workdir", str(args.workdir),
        "--keep",
    ])
    run = accept.AcceptanceRun(options)
    tracer = None
    try:
        run.preflight()
        run.start_cluster()
        run.verify_root_and_management_api()
        expected = b"agent-home-remote-read-probe"
        for index in range(args.count):
            accept.write_fsync_close(run.mount_a / "job-42" / f"file-{index}", expected)

        trace_path = args.workdir / "home-sync.trace"
        tracer = subprocess.Popen([
            "sudo", "-n", "strace", "-f", "-qq", "-e", "trace=fsync,fdatasync",
            "-p", str(run.process_by_name("node-A").pid), "-o", str(trace_path),
        ], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        time.sleep(0.5)
        if tracer.poll() is not None:
            raise RuntimeError(f"strace exited: {tracer.stderr.read().decode()}")
        times = []
        for _ in range(2 if args.mode == "read" else 1):
            started = time.perf_counter_ns()
            for index in range(args.count):
                path = run.mount_b / "job-42" / f"file-{index}"
                if args.mode == "read":
                    if path.read_bytes() != expected:
                        raise RuntimeError(f"wrong bytes for file-{index}")
                else:
                    fd = os.open(path, os.O_WRONLY)
                    try:
                        os.pwrite(fd, b"B", 0)
                        if args.mode != "write-nosync":
                            os.fdatasync(fd)
                        if args.mode == "write-after-sync":
                            os.pwrite(fd, b"C", 1)
                    finally:
                        os.close(fd)
                    expected_prefix = b"BC" if args.mode == "write-after-sync" else b"B"
                    if path.read_bytes()[:len(expected_prefix)] != expected_prefix:
                        raise RuntimeError(f"wrong bytes after close for file-{index}")
            times.append((time.perf_counter_ns() - started) / 1e6)
        tracer.send_signal(signal.SIGINT)
        tracer.communicate(timeout=5)
        calls = re.findall(r"\b(fsync|fdatasync)\(", trace_path.read_text())
        print(json.dumps({"count": args.count, "mode": args.mode, "pass_ms": times,
                          "home_sync_calls": len(calls),
                          "fsync": calls.count("fsync"), "fdatasync": calls.count("fdatasync"),
                          "trace": str(trace_path)}))
        if args.expect_home_sync_calls is not None and len(calls) != args.expect_home_sync_calls:
            raise AssertionError(f"expected {args.expect_home_sync_calls} home sync calls, got {len(calls)}")
    finally:
        if tracer is not None and tracer.poll() is None:
            tracer.send_signal(signal.SIGINT)
            tracer.communicate(timeout=5)
        errors = run.cleanup()
        if errors:
            raise RuntimeError(f"cleanup errors: {errors}")


if __name__ == "__main__":
    main()

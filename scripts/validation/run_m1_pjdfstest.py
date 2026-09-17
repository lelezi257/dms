#!/usr/bin/env python3
"""Run the frozen DMS M1 pjdfstest supported subset."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import time
from typing import Any

from posix_suite_runner import PosixSuiteRunner, ROOT, SuiteError, read_allowlist, suite_identity, write_json


CASE_ID = "posix-pjdfstest-supported"
DEFAULT_ALLOWLIST = ROOT / "scripts/validation/m1/posix/pjdfstest-supported.txt"


def _run_one(
    suite_dir: Path,
    mountpoint: Path,
    case_id: str,
    command_text: str,
    output_dir: Path,
    run_as_root: bool,
) -> dict[str, Any]:
    case_dir = output_dir / "cases" / case_id.replace("/", "_")
    case_dir.mkdir(parents=True, exist_ok=True)
    command_path = suite_dir / command_text
    started = time.monotonic_ns()
    if not command_path.exists():
        return {
            "id": case_id,
            "command": command_text,
            "status": "FAIL",
            "returncode": 127,
            "duration_ms": 0.0,
            "message": f"allowlisted pjdfstest case is missing: {command_text}",
            "stdout": "",
            "stderr": "",
        }
    env = os.environ.copy()
    env.update(
        {
            "PJD_TEST_PATH": str(mountpoint),
            "PJDFSTEST_TEST_PATH": str(mountpoint),
            "TMPDIR": str(mountpoint),
        }
    )
    argv = ["prove", "-v", str(command_path)] if command_path.suffix == ".t" else [str(command_path)]
    if run_as_root:
        # pjdfstest 内部还会继续切换 uid/gid；从 root 启动才能让身份矩阵有效。
        argv = ["sudo", "-n", "-E", *argv]
    completed = subprocess.run(argv, cwd=suite_dir, env=env, text=True, capture_output=True, check=False)
    (case_dir / "stdout.txt").write_text(completed.stdout or "", encoding="utf-8")
    (case_dir / "stderr.txt").write_text(completed.stderr or "", encoding="utf-8")
    return {
        "id": case_id,
        "command": shlex.join(argv),
        "status": "PASS" if completed.returncode == 0 else "FAIL",
        "returncode": completed.returncode,
        "duration_ms": (time.monotonic_ns() - started) / 1_000_000.0,
        "stdout": str((case_dir / "stdout.txt").resolve()),
        "stderr": str((case_dir / "stderr.txt").resolve()),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--suite-dir",
        type=Path,
        default=Path(os.environ.get("DMS_PJDFSTEST_DIR", "/nonexistent/dms-pjdfstest")),
    )
    parser.add_argument("--allowlist", type=Path, default=DEFAULT_ALLOWLIST)
    parser.add_argument("--mountpoint", type=Path, help="已挂载的 DMS FUSE mount，仅用于 wrapper 单测或手工调试")
    parser.add_argument("--purpose", choices=("discovery", "release"), default="discovery")
    parser.add_argument("--port-base", type=int, default=32500)
    args = parser.parse_args()

    args.output.mkdir(parents=True, exist_ok=True)
    runner = PosixSuiteRunner(
        case_id=CASE_ID,
        suite_name="pjdfstest",
        output=args.output,
        purpose=args.purpose,
        allowlist=args.allowlist,
        suite_dir=args.suite_dir,
        mountpoint=args.mountpoint,
        port_base=args.port_base,
    )
    evidence = [args.allowlist, args.output / "suite-result.json"]
    try:
        cases = read_allowlist(args.allowlist)
        required = ["prove", "python3"]
        if args.mountpoint is None:
            required.append("sudo")
        runner.preflight(required)
        if args.mountpoint is None:
            completed = subprocess.run(
                ["sudo", "-n", "true"], text=True, capture_output=True, check=False
            )
            if completed.returncode:
                raise SuiteError("pjdfstest requires passwordless sudo for its identity matrix")
        mountpoint = runner.dms_mountpoint()
        results = [
            _run_one(
                args.suite_dir,
                mountpoint,
                case_id,
                command_text,
                args.output,
                args.mountpoint is None,
            )
            for case_id, command_text in cases
        ]
        failures = [case for case in results if case["status"] != "PASS"]
        summary = {
            "schema": "dms.m1.pjdfstest-result.v1",
            "suite_dir": str(args.suite_dir.resolve()),
            "suite_identity": suite_identity(args.suite_dir),
            "allowlist": str(args.allowlist.resolve()),
            "mountpoint": str(mountpoint.resolve()),
            "total": len(results),
            "passed": len(results) - len(failures),
            "failed": len(failures),
            "cases": results,
        }
        write_json(args.output / "suite-result.json", summary)
        status = "PASS" if not failures else "FAIL"
        return runner.finish_case(
            status=status,
            message="pjdfstest supported subset passed" if status == "PASS" else f"pjdfstest failures={len(failures)}",
            evidence=evidence,
            extra={"total": len(results), "failed": len(failures)},
        )
    except SuiteError as error:
        write_json(
            args.output / "suite-result.json",
            {
                "schema": "dms.m1.pjdfstest-result.v1",
                "status": "preflight-failed",
                "error": str(error),
                "allowlist": str(args.allowlist.resolve()),
                "suite_dir": str(args.suite_dir),
                "suite_identity": suite_identity(args.suite_dir) if args.suite_dir.is_dir() else None,
            },
        )
        return runner.finish_case(status="FAIL", message=f"preflight failed: {error}", evidence=evidence)
    finally:
        runner.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())

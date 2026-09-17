#!/usr/bin/env python3
"""Run the frozen DMS M1 fstests generic supported subset."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
from typing import Any

from posix_suite_runner import PosixSuiteRunner, ROOT, SuiteError, read_allowlist, suite_identity, write_json


CASE_ID = "fstests-generic-supported"
DEFAULT_ALLOWLIST = ROOT / "scripts/validation/m1/posix/fstests-generic-supported.txt"


def _root_prefix() -> list[str]:
    """只在执行上游 suite 时取得 root 身份。

    DMS 进程和 FUSE mount 仍由当前普通用户持有；xfstests 需要 root 来切换
    uid/gid 并检查权限语义。把提权限制在 ``./check`` 边界，可以避免 Cargo、
    日志与证据目录变成 root 所有，也不会把身份不足误判成产品缺陷。
    """

    if os.geteuid() == 0:
        return []
    sudo = shutil.which("sudo")
    if sudo is None:
        raise SuiteError("xfstests requires root capabilities, but sudo is unavailable")
    completed = subprocess.run(
        [sudo, "-n", "true"],
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        raise SuiteError("xfstests requires passwordless sudo for its uid/gid permission matrix")
    return [sudo, "-n", "-E"]


def _write_local_config(path: Path, mountpoint: Path) -> None:
    path.write_text(
        "\n".join(
            [
                "# DMS FUSE is already mounted by the wrapper. For xfstests,",
                "# fuse TEST_DEV must match the mounted source reported by df(1).",
                "# DMS FUSE uses MountOption::FSName(\"dms-node\"), while TEST_DIR",
                "# is the real mountpoint used by the frozen generic cases.",
                "# SCRATCH_* is intentionally omitted: the frozen M1 allowlist must",
                "# not include tests that call _require_scratch.",
                "export TEST_DEV=dms-node",
                f"export TEST_DIR={shlex.quote(str(mountpoint))}",
                "export FSTYP=fuse",
                "",
            ]
        ),
        encoding="utf-8",
    )


def _run_batch(
    suite_dir: Path,
    mountpoint: Path,
    cases: list[tuple[str, str]],
    output_dir: Path,
    timeout_sec: int,
    root_prefix: list[str],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    case_dir = output_dir / "cases" / "fstests_batch"
    case_dir.mkdir(parents=True, exist_ok=True)
    started = time.monotonic_ns()
    missing = [
        f"{case_id}: generic/{generic_number}"
        for case_id, generic_number in cases
        if not (suite_dir / "tests/generic" / generic_number).exists()
    ]
    if missing:
        results = []
        for case_id, generic_number in cases:
            status = "FAIL" if f"{case_id}: generic/{generic_number}" in missing else "NOT_RUN"
            results.append(
                {
                    "id": case_id,
                    "generic": generic_number,
                    "status": status,
                    "returncode": 127 if status == "FAIL" else None,
                    "duration_ms": 0.0,
                    "message": f"allowlisted fstests generic case is missing: generic/{generic_number}"
                    if status == "FAIL"
                    else "not run because another allowlisted case is missing",
                }
            )
        return results, {
            "command": "",
            "returncode": 127,
            "duration_ms": 0.0,
            "stdout": "",
            "stderr": "",
            "message": "; ".join(missing),
        }
    local_config = case_dir / "local.config"
    _write_local_config(local_config, mountpoint)
    argv = [*root_prefix, "./check", "-fuse", *[f"generic/{generic_number}" for _, generic_number in cases]]
    env = os.environ.copy()
    env["RESULT_BASE"] = str((case_dir / "results").resolve())
    env["HOST_OPTIONS"] = str(local_config.resolve())
    stdout = ""
    stderr = ""
    returncode = 0
    timed_out = False
    try:
        completed = subprocess.run(
            argv,
            cwd=suite_dir,
            env=env,
            text=True,
            capture_output=True,
            check=False,
            timeout=timeout_sec,
        )
        stdout = completed.stdout or ""
        stderr = completed.stderr or ""
        returncode = completed.returncode
    except subprocess.TimeoutExpired as error:
        timed_out = True
        returncode = 124
        stdout = error.stdout or ""
        stderr = error.stderr or ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode("utf-8", errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
    (case_dir / "stdout.txt").write_text(stdout, encoding="utf-8")
    (case_dir / "stderr.txt").write_text(stderr, encoding="utf-8")
    combined_output = f"{stdout}\n{stderr}"
    unsupported_markers = ("Not run:", "[not run]", "not run")
    unsupported = any(marker in combined_output for marker in unsupported_markers)
    failed = timed_out or returncode != 0 or unsupported
    message = None
    if timed_out:
        message = f"xfstests batch timed out after {timeout_sec}s"
    elif unsupported:
        message = "xfstests reported at least one case as not run; DMS treats that as FAIL, not SKIP"
    result_common = {
        "command": shlex.join(argv),
        "returncode": returncode,
        "duration_ms": (time.monotonic_ns() - started) / 1_000_000.0,
        "message": message,
        "config": str(local_config.resolve()),
        "stdout": str((case_dir / "stdout.txt").resolve()),
        "stderr": str((case_dir / "stderr.txt").resolve()),
        "results_dir": str((case_dir / "results").resolve()),
    }
    results = [
        {
            "id": case_id,
            "generic": generic_number,
            "status": "FAIL" if failed else "PASS",
            **result_common,
        }
        for case_id, generic_number in cases
    ]
    return results, result_common


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--suite-dir",
        type=Path,
        default=Path(os.environ.get("DMS_FSTESTS_DIR", "/nonexistent/dms-xfstests")),
    )
    parser.add_argument("--allowlist", type=Path, default=DEFAULT_ALLOWLIST)
    parser.add_argument("--mountpoint", type=Path, help="已挂载的 DMS FUSE mount，仅用于 wrapper 单测或手工调试")
    parser.add_argument("--purpose", choices=("discovery", "release"), default="discovery")
    parser.add_argument("--port-base", type=int, default=32600)
    # generic/075 包含四轮 fsx，默认 180 秒只够进入最后一轮，强杀进程会留下
    # 类似内容不一致的中间证据。900 秒是整个冻结 batch 的上限，不是单 syscall 阈值。
    parser.add_argument("--case-timeout-sec", type=int, default=900)
    args = parser.parse_args()

    args.output.mkdir(parents=True, exist_ok=True)
    runner = PosixSuiteRunner(
        case_id=CASE_ID,
        suite_name="fstests",
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
        runner.preflight(["python3"])
        if not (args.suite_dir / "check").is_file():
            raise SuiteError(f"xfstests check entry is missing: {args.suite_dir / 'check'}")
        root_prefix = _root_prefix()
        mountpoint = runner.dms_mountpoint()
        results, batch = _run_batch(
            args.suite_dir,
            mountpoint,
            cases,
            args.output,
            args.case_timeout_sec,
            root_prefix,
        )
        failures = [case for case in results if case["status"] != "PASS"]
        summary = {
            "schema": "dms.m1.fstests-result.v1",
            "suite_dir": str(args.suite_dir.resolve()),
            "suite_identity": suite_identity(args.suite_dir),
            "allowlist": str(args.allowlist.resolve()),
            "mountpoint": str(mountpoint.resolve()),
            "batch": batch,
            "total": len(results),
            "passed": len(results) - len(failures),
            "failed": len(failures),
            "cases": results,
        }
        write_json(args.output / "suite-result.json", summary)
        status = "PASS" if not failures else "FAIL"
        return runner.finish_case(
            status=status,
            message="fstests generic supported subset passed" if status == "PASS" else f"fstests failures={len(failures)}",
            evidence=evidence,
            extra={"total": len(results), "failed": len(failures)},
        )
    except SuiteError as error:
        write_json(
            args.output / "suite-result.json",
            {
                "schema": "dms.m1.fstests-result.v1",
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

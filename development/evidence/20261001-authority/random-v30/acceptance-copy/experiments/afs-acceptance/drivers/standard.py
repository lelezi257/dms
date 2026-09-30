#!/usr/bin/env python3
"""Standard-suite acceptance drivers.

Currently implements STD-01 (pjdfstest) only. The driver runs the pinned
upstream pjdfstest root harness against a caller supplied mount directory,
records raw TAP/prove output, and emits the runner proof JSON as the final
stdout object.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import subprocess
import uuid
import sys
import time
import traceback
from pathlib import Path
from typing import Any

from target_identity import target_checks

PJDFS_REV = "d25636a227606f8960e5179741d8f4ad7030ef41"
DEFAULT_SUITE_ROOT = Path("/mnt/lima-afsctlstate/afs-acceptance/suites-reference/src/pjdfstest")
SMOKE_TESTS = ["open/00.t", "mkdir/00.t", "rename/00.t", "mknod/00.t"]


def utc() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_text(argv: list[str], timeout: int = 10) -> dict[str, Any]:
    try:
        proc = subprocess.run(argv, shell=False, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, check=False)
        return {"argv": argv, "returncode": proc.returncode, "stdout": proc.stdout, "stderr": proc.stderr}
    except Exception as exc:  # noqa: BLE001 - evidence path records exact failure
        return {"argv": argv, "returncode": None, "exception": type(exc).__name__, "message": str(exc)}


def run_bounded(argv: list[str], cwd: Path, timeout: int, stdout_path: Path, stderr_path: Path) -> dict[str, Any]:
    started = time.time()
    stdout_path.parent.mkdir(parents=True, exist_ok=True)
    stderr_path.parent.mkdir(parents=True, exist_ok=True)
    timed_out = False
    with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
        process = subprocess.Popen(argv, cwd=str(cwd), shell=False, stdout=out, stderr=err, start_new_session=True)
        try:
            returncode = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                returncode = process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                returncode = process.wait()
            err.write(f"pjdfstest driver timed out after {timeout}s\n".encode())
    return {"argv": argv, "cwd": str(cwd), "returncode": returncode, "timeout_seconds": timeout, "timed_out": timed_out, "duration_seconds": round(time.time() - started, 3)}


def discover_tests(suite_root: Path) -> dict[str, Any]:
    tests_root = suite_root / "tests"
    tests = sorted(str(path.relative_to(tests_root)) for path in tests_root.rglob("*.t") if path.is_file())
    uid_drop_tests: list[str] = []
    todo_source_files: list[str] = []
    for rel in tests:
        text = (tests_root / rel).read_text(errors="replace")
        if re.search(r"\s-u\s+\d+", text) or re.search(r"\s-g\s+\d+", text):
            uid_drop_tests.append(rel)
        if "TODO" in text:
            todo_source_files.append(rel)
    misc = tests_root / "misc.sh"
    misc_text = misc.read_text(errors="replace") if misc.exists() else ""
    return {
        "tests_root": str(tests_root),
        "discovered_files": len(tests),
        "tests": tests,
        "uid_drop_test_files": uid_drop_tests,
        "uid_drop_test_file_count": len(uid_drop_tests),
        "todo_source_files": todo_source_files,
        "todo_source_file_count": len(todo_source_files),
        "misc_defines_todo": "TODO" in misc_text,
    }


def parse_tap_and_prove(stdout_path: Path, stderr_path: Path, selected_tests: list[str]) -> dict[str, Any]:
    text = stdout_path.read_text(errors="replace") if stdout_path.exists() else ""
    stderr = stderr_path.read_text(errors="replace") if stderr_path.exists() else ""
    tap_ok = tap_not_ok = tap_skip = tap_todo = tap_unexpected_fail = 0
    todo_not_ok = 0
    planned = 0
    not_ok_lines: list[str] = []
    unexpected_fail_lines: list[str] = []
    todo_lines: list[str] = []
    skip_lines: list[str] = []
    for line in text.splitlines():
        stripped = line.strip()
        if re.match(r"^1\.\.\d+", stripped):
            try:
                planned += int(stripped.split("..", 1)[1].split()[0])
            except Exception:  # noqa: BLE001
                pass
        if re.match(r"^ok\s+\d+", stripped):
            tap_ok += 1
            if re.search(r"#\s*SKIP", stripped, re.I):
                tap_skip += 1
                skip_lines.append(stripped)
            if re.search(r"#\s*TODO", stripped, re.I):
                tap_todo += 1
                todo_lines.append(stripped)
        elif re.match(r"^not ok\s+\d+", stripped):
            tap_not_ok += 1
            not_ok_lines.append(stripped)
            if re.search(r"#\s*TODO", stripped, re.I):
                tap_todo += 1
                todo_not_ok += 1
                todo_lines.append(stripped)
            else:
                tap_unexpected_fail += 1
                unexpected_fail_lines.append(stripped)
    prove_files = prove_tests = None
    prove_result = None
    match = re.search(r"Files=(\d+),\s+Tests=(\d+).+?Result:\s+(\w+)", text, re.S)
    if match:
        prove_files = int(match.group(1))
        prove_tests = int(match.group(2))
        prove_result = match.group(3)
    return {
        "selected_files": len(selected_tests),
        "selected_tests": selected_tests,
        "stdout_bytes": len(text.encode()),
        "stderr_bytes": len(stderr.encode()),
        "tap_ok": tap_ok,
        "tap_not_ok": tap_not_ok,
        "tap_skip": tap_skip,
        "tap_todo": tap_todo,
        "tap_todo_not_ok": todo_not_ok,
        "tap_unexpected_fail": tap_unexpected_fail,
        "tap_planned": planned,
        "not_ok_lines": not_ok_lines,
        "unexpected_fail_lines": unexpected_fail_lines,
        "todo_lines": todo_lines,
        "skip_lines": skip_lines,
        "prove_files": prove_files,
        "prove_tests": prove_tests,
        "prove_result": prove_result,
        "accounting_identity": {
            "tap_total_observed": tap_ok + tap_not_ok,
            "tap_total_accounted": tap_ok + tap_not_ok,
            "no_post_failure_filtering": True,
            "todo_is_counted_not_filtered": True,
        },
    }


def mount_identity(mount: Path) -> dict[str, Any]:
    return run_text(["findmnt", "-T", str(mount), "-o", "TARGET,SOURCE,FSTYPE,OPTIONS", "--json"])


def complete_accounting(accounting: dict[str, Any], selected_files: int) -> bool:
    observed = accounting["tap_ok"] + accounting["tap_not_ok"]
    return (
        selected_files > 0
        and accounting["prove_files"] == selected_files
        and observed > 0
        and accounting["prove_tests"] == observed
        and accounting["tap_planned"] == observed
    )


def resolve_base_dir(mount: Path, base_dir: Path | None) -> Path:
    if base_dir is None:
        return mount
    return base_dir if base_dir.is_absolute() else mount / base_dir


def is_under(child: Path, parent: Path) -> bool:
    try:
        child.resolve().relative_to(parent.resolve())
        return True
    except ValueError:
        return False


def process_identity(pid: str | None) -> dict[str, Any] | None:
    if not pid:
        return None
    proc = Path("/proc") / pid
    result: dict[str, Any] = {"pid": pid, "exists": proc.exists()}
    try:
        exe = proc.joinpath("exe").resolve()
        result["exe"] = str(exe)
        if exe.is_file():
            result["exe_sha256"] = sha256_file(exe)
    except Exception as exc:  # noqa: BLE001
        result["exe_error"] = f"{type(exc).__name__}: {exc}"
    try:
        result["cmdline"] = proc.joinpath("cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
    except Exception as exc:  # noqa: BLE001
        result["cmdline_error"] = f"{type(exc).__name__}: {exc}"
    return result


def suite_identity(suite_root: Path) -> dict[str, Any]:
    head = run_text(["git", "-C", str(suite_root), "rev-parse", "HEAD"])
    status = run_text(["git", "-C", str(suite_root), "status", "--porcelain"])
    exe = suite_root / "pjdfstest"
    return {
        "suite_root": str(suite_root),
        "expected_revision": PJDFS_REV,
        "git_head": head.get("stdout", "").strip(),
        "git_head_command": head,
        "git_status_porcelain": status.get("stdout", ""),
        "git_status_command": status,
        "executable": str(exe),
        "executable_exists": exe.is_file() and os.access(exe, os.X_OK),
        "executable_sha256": sha256_file(exe) if exe.is_file() else None,
    }


def rel(path: Path, base: Path) -> str:
    return str(path.resolve().relative_to(base.resolve()))


def build_check(name: str, status: str, evidence: Any, artifact: str | None = None) -> dict[str, Any]:
    check = {"name": name, "status": status, "evidence": evidence}
    if artifact:
        check["artifact"] = artifact
    return check


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run AFS STD-01 pjdfstest driver")
    parser.add_argument("--case-id", default=os.environ.get("AFS_ACCEPTANCE_CASE_ID", "STD-01"))
    parser.add_argument("--profile", choices=["smoke", "full"], default=os.environ.get("AFS_ACCEPTANCE_PROFILE", "smoke"))
    parser.add_argument("--matrix-json", default=os.environ.get("AFS_ACCEPTANCE_MATRIX", "{}"))
    parser.add_argument("--run-dir", type=Path, default=Path(os.environ.get("AFS_ACCEPTANCE_RUN_DIR", "results/std-01-driver")))
    parser.add_argument("--mount", type=Path, default=Path(os.environ["AFS_ACCEPTANCE_MOUNT"]) if os.environ.get("AFS_ACCEPTANCE_MOUNT") else None)
    parser.add_argument("--base-dir", type=Path, default=Path(os.environ["AFS_ACCEPTANCE_BASE_DIR"]) if os.environ.get("AFS_ACCEPTANCE_BASE_DIR") else None, help="Directory under --mount that contains the temporary pjdfstest fixture. Relative paths are resolved below --mount.")
    parser.add_argument("--suite-root", type=Path, default=DEFAULT_SUITE_ROOT)
    parser.add_argument("--timeout", type=int, default=None, help="pjdfstest subprocess timeout in seconds")
    parser.add_argument("--process-pid", default=os.environ.get("AFS_ACCEPTANCE_PROCESS_PID"))
    parser.add_argument("--meta-process-pid", default=os.environ.get("AFS_ACCEPTANCE_META_PROCESS_PID"))
    parser.add_argument("--backend", default=os.environ.get("AFS_ACCEPTANCE_BACKEND"), help="Observed product backend label, for proof identity only.")
    parser.add_argument("--meta", default=os.environ.get("AFS_ACCEPTANCE_META"), help="Observed Meta backend label, for proof identity only.")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    run_dir = args.run_dir.resolve()
    artifacts = run_dir / "artifacts" / "std-01-pjdfstest"
    artifacts.mkdir(parents=True, exist_ok=True)
    matrix = json.loads(args.matrix_json) if args.matrix_json.strip() else {}
    if not isinstance(matrix, dict):
        matrix = {}
    matrix.setdefault("reference", "ext4")
    matrix.setdefault("suite_sha", f"sanwan/pjdfstest {PJDFS_REV}")

    checks: list[dict[str, Any]] = []
    status = "PASS"
    reason = ""
    selected_tests: list[str] = []
    accounting: dict[str, Any] = {}
    command_result: dict[str, Any] | None = None
    fixture: Path | None = None
    fixture_kept: bool | None = None

    try:
        if args.case_id != "STD-01":
            raise RuntimeError(f"standard.py currently implements STD-01 only, got {args.case_id}")
        if args.mount is None:
            raise RuntimeError("--mount or AFS_ACCEPTANCE_MOUNT is required")

        suite = suite_identity(args.suite_root)
        discovery = discover_tests(args.suite_root)
        base_dir = resolve_base_dir(args.mount, args.base_dir)
        mnt = mount_identity(args.mount)
        base_mnt = mount_identity(base_dir)
        proc = process_identity(args.process_pid)
        meta_proc = process_identity(args.meta_process_pid)
        product_identity = {
            "backend": args.backend or matrix.get("backend"),
            "meta": args.meta or matrix.get("meta"),
            "transport": matrix.get("transport"),
            "single_backend_selection": bool(args.backend or matrix.get("backend")),
            "mount_path": str(args.mount),
            "base_dir": str(base_dir),
            "process_pid": args.process_pid,
            "meta_process_pid": args.meta_process_pid,
        }
        identity = {
            "created_at": utc(),
            "host": run_text(["hostname"]),
            "uname": run_text(["uname", "-a"]),
            "platform": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(), "python": platform.python_version(), "uid": os.geteuid(), "gid": os.getegid()},
            "suite": suite,
            "mount": mnt,
            "base_mount": base_mnt,
            "process": proc,
            "meta_process": meta_proc,
            "product": product_identity,
            "lock_state_note": "acceptance.lock.json may remain PREPARING; driver readiness is not a release PASS.",
        }
        write_json(artifacts / "identity.json", identity)
        write_json(artifacts / "discovery.json", discovery)

        suite_ok = suite["git_head"] == PJDFS_REV and suite["executable_exists"]
        checks.append(build_check("pinned-suite-identity", "PASS" if suite_ok else "FAIL", {"expected": PJDFS_REV, "observed": suite.get("git_head"), "executable_exists": suite.get("executable_exists")}, rel(artifacts / "identity.json", run_dir)))
        root_ok = os.geteuid() == 0
        checks.append(build_check("root-harness", "PASS" if root_ok else "BLOCKED", {"euid": os.geteuid(), "uid_drop_test_file_count": discovery["uid_drop_test_file_count"], "note": "pjdfstest must run as root; internal -u/-g tests provide non-root coverage."}, rel(artifacts / "discovery.json", run_dir)))
        mount_ok = mnt.get("returncode") == 0 and bool(mnt.get("stdout", "").strip())
        base_dir_ok = base_dir.is_dir() and is_under(base_dir, args.mount)
        checks.append(build_check("mount-identity", "PASS" if mount_ok else "BLOCKED", {"mount": str(args.mount), "findmnt_returncode": mnt.get("returncode")}, rel(artifacts / "identity.json", run_dir)))
        checks.append(build_check("base-dir-scope", "PASS" if base_dir_ok else "BLOCKED", {"mount": str(args.mount), "base_dir": str(base_dir), "exists": base_dir.exists(), "is_dir": base_dir.is_dir(), "under_mount": is_under(base_dir, args.mount)}, rel(artifacts / "identity.json", run_dir)))
        backend_ok = bool(product_identity["backend"])
        checks.append(build_check("backend-selection", "PASS" if backend_ok else "BLOCKED", product_identity, rel(artifacts / "identity.json", run_dir)))
        observed_checks = target_checks(platform.system(), product_identity["backend"], mnt, base_mnt, proc, meta_proc)
        for name, passed in observed_checks.items():
            checks.append(build_check(name, "PASS" if passed else "BLOCKED", {"observed": passed}, rel(artifacts / "identity.json", run_dir)))
        target_ok = all(observed_checks.values())
        discovery_ok = discovery["discovered_files"] > 0
        checks.append(build_check("discovery", "PASS" if discovery_ok else "BLOCKED", {"discovered_files": discovery["discovered_files"], "uid_drop_test_file_count": discovery["uid_drop_test_file_count"], "todo_source_file_count": discovery["todo_source_file_count"]}, rel(artifacts / "discovery.json", run_dir)))

        if not suite_ok or not root_ok or not mount_ok or not base_dir_ok or not backend_ok or not discovery_ok or not target_ok:
            status = "BLOCKED" if not (root_ok and mount_ok and base_dir_ok and backend_ok and discovery_ok and target_ok) else "FAIL"
            reason = "pjdfstest preflight failed"
        else:
            tests_root = args.suite_root / "tests"
            if args.profile == "smoke":
                selected_tests = SMOKE_TESTS
                timeout = args.timeout or 180
            else:
                selected_tests = discovery["tests"]
                timeout = args.timeout or 1800
            test_paths = [str(tests_root / rel_test) for rel_test in selected_tests]
            fixture = base_dir / f".afs-std01-pjdfstest-{dt.datetime.now(dt.timezone.utc).strftime('%Y%m%dT%H%M%SZ')}-{uuid.uuid4().hex[:8]}"
            fixture.mkdir(mode=0o755)
            # pjdfstest runs as root but many upstream checks drop to numeric
            # non-root users through the harness. Those child processes must be
            # able to traverse the harness root; a 0700 root-owned fixture makes
            # valid non-root relative-path cases fail with EACCES on ext4.
            os.chmod(fixture, 0o755)
            stdout = artifacts / "pjdfstest.stdout.tap"
            stderr = artifacts / "pjdfstest.stderr.log"
            command = ["prove", "-e", "/bin/sh", "-rv", *test_paths]
            command_result = run_bounded(command, cwd=fixture, timeout=timeout, stdout_path=stdout, stderr_path=stderr)
            write_json(artifacts / "command.json", command_result)
            accounting = parse_tap_and_prove(stdout, stderr, selected_tests)
            accounting.update({
                "profile": args.profile,
                "discovered_files": discovery["discovered_files"],
                "executed_files": len(selected_tests),
                "incomplete_files": max(0, discovery["discovered_files"] - len(selected_tests)),
                "upstream_todo_source_file_count": discovery["todo_source_file_count"],
                "uid_drop_test_file_count": discovery["uid_drop_test_file_count"],
            })
            write_json(artifacts / "tap-accounting.json", accounting)
            checks.append(build_check("subprocess-bound", "PASS" if not command_result["timed_out"] else "BLOCKED", {"timeout_seconds": command_result["timeout_seconds"], "timed_out": command_result["timed_out"], "returncode": command_result["returncode"]}, rel(artifacts / "command.json", run_dir)))
            accounting_ok = complete_accounting(accounting, len(selected_tests))
            checks.append(build_check("tap-accounting", "PASS" if accounting_ok else "FAIL", accounting, rel(artifacts / "tap-accounting.json", run_dir)))
            uid_drop_executed = bool(set(selected_tests) & set(discovery["uid_drop_test_files"]))
            uid_drop_status = "PASS" if uid_drop_executed or args.profile == "smoke" else "BLOCKED"
            checks.append(build_check("uid-drop-coverage", uid_drop_status, {"executed_uid_drop_tests": sorted(set(selected_tests) & set(discovery["uid_drop_test_files"])), "total_uid_drop_test_files": discovery["uid_drop_test_file_count"], "profile": args.profile, "note": "smoke verifies harness wiring; full profile must include upstream uid/gid drop tests."}, rel(artifacts / "tap-accounting.json", run_dir)))
            result_ok = accounting_ok and command_result["returncode"] == 0 and not command_result["timed_out"] and accounting["tap_unexpected_fail"] == 0 and accounting.get("prove_result") == "PASS"
            checks.append(build_check("pjdfstest-result", "PASS" if result_ok else "FAIL", {"returncode": command_result["returncode"], "prove_result": accounting.get("prove_result"), "tap_unexpected_fail": accounting.get("tap_unexpected_fail"), "tap_todo": accounting.get("tap_todo")}, rel(artifacts / "pjdfstest.stdout.tap", run_dir)))
            if command_result["timed_out"]:
                status = "BLOCKED"
                reason = f"pjdfstest timed out after {command_result['timeout_seconds']}s"
            elif not result_ok:
                status = "FAIL"
                reason = "pjdfstest reported failures; raw TAP was preserved without filtering"
            if status == "PASS" and fixture.exists():
                try:
                    shutil.rmtree(fixture)
                    fixture_kept = False
                    checks.append(build_check("cleanup-fixture", "PASS", {"fixture": str(fixture), "kept": False}))
                except Exception as cleanup_exc:  # noqa: BLE001 - preserve suite accounting and report cleanup separately
                    fixture_kept = True
                    cleanup_artifact = artifacts / "cleanup-error.json"
                    write_json(cleanup_artifact, {"fixture": str(fixture), "exception": type(cleanup_exc).__name__, "message": str(cleanup_exc), "traceback": traceback.format_exc()})
                    checks.append(build_check("cleanup-fixture", "FAIL", {"fixture": str(fixture), "kept": True, "error": f"{type(cleanup_exc).__name__}: {cleanup_exc}"}, rel(cleanup_artifact, run_dir)))
                    status = "FAIL"
                    reason = "pjdfstest passed but fixture cleanup failed; accounting was preserved"
            else:
                fixture_kept = bool(fixture and fixture.exists())
    except Exception as exc:  # noqa: BLE001 - proof must preserve setup failure
        status = "BLOCKED"
        reason = f"driver setup failed: {type(exc).__name__}: {exc}"
        write_json(artifacts / "setup-error.json", {"exception": type(exc).__name__, "message": str(exc), "traceback": traceback.format_exc()})
        checks.append(build_check("driver-setup", "BLOCKED", reason, rel(artifacts / "setup-error.json", run_dir)))

    coverage_axes = {
        "reference": {"values": [str(matrix.get("reference", "ext4"))], "checks": {str(matrix.get("reference", "ext4")): "mount-identity"}},
        "suite_sha": {"values": [str(matrix.get("suite_sha", f"sanwan/pjdfstest {PJDFS_REV}"))], "checks": {str(matrix.get("suite_sha", f"sanwan/pjdfstest {PJDFS_REV}")): "pinned-suite-identity"}},
    }
    proof = {
        "case_id": args.case_id,
        "profile": args.profile,
        "matrix": matrix,
        "status": status,
        "reason": reason,
        "checks": checks,
        "coverage": {"profile": args.profile, "axes": coverage_axes},
        "artifacts": {"root": rel(artifacts, run_dir)},
        "fixture": {"path": str(fixture) if fixture else None, "base_dir": str(resolve_base_dir(args.mount, args.base_dir)) if args.mount else None, "kept": fixture_kept},
        "identity": {"artifact": rel(artifacts / "identity.json", run_dir), "product": (identity.get("product") if "identity" in locals() else None)},
        "accounting": accounting,
        "command": command_result,
        "notes": [
            "STD-01 driver READY means the pjdfstest harness can run and report proof; it is not a release acceptance PASS by itself.",
            "No failures are filtered after execution. TODO and SKIP are counted in TAP accounting.",
        ],
    }
    write_json(artifacts / "proof.json", proof)
    print(json.dumps(proof, sort_keys=True))
    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

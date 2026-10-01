#!/usr/bin/env python3
"""Small Linux resource snapshot for round3 AFS diagnostics.

This helper is intentionally observational. It reads selected kernel/procfs
state and optionally writes one JSON report. It does not read process cmdline or
environ because those fields may contain secrets.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import Iterable

REQUIRED_SYSTEM = "Linux"
REQUIRED_MACHINE = "aarch64"
COMMAND_TIMEOUT_SECONDS = 5
PROC_ROOT = pathlib.Path("/proc")
CGROUP_ROOT = pathlib.Path("/sys/fs/cgroup")
RDMA_RES_TYPES = ("qp", "mr", "cq", "pd", "ctx")


class ProbeError(RuntimeError):
    pass


@dataclass(frozen=True)
class ProcessTarget:
    role: str
    pid_file: pathlib.Path
    pid: int


def unavailable(reason: str, **extra: object) -> dict[str, object]:
    record: dict[str, object] = {"status": "UNAVAILABLE", "reason": reason}
    record.update(extra)
    return record


def read_text(path: pathlib.Path) -> dict[str, object]:
    try:
        return {"status": "OBSERVED", "path": str(path), "value": path.read_text(encoding="utf-8").strip()}
    except OSError as error:
        return unavailable(str(error), path=str(path))


def require_linux_aarch64_root() -> None:
    if platform.system() != REQUIRED_SYSTEM:
        raise ProbeError(f"{REQUIRED_SYSTEM} required, got {platform.system()}")
    if platform.machine() != REQUIRED_MACHINE:
        raise ProbeError(f"{REQUIRED_MACHINE} required, got {platform.machine()}")
    if os.geteuid() != 0:
        raise ProbeError("root privileges required for stable process/resource capture")


def parse_pid_file(value: str) -> ProcessTarget:
    if "=" not in value:
        raise ProbeError("--pid-file expects ROLE=PATH")
    role, path_text = value.split("=", 1)
    if not role or any(ch in role for ch in "/\0"):
        raise ProbeError(f"invalid role {role!r}")
    path = pathlib.Path(path_text)
    try:
        pid_text = path.read_text(encoding="utf-8").strip()
    except OSError as error:
        raise ProbeError(f"cannot read pid file {path}: {error}") from error
    if not pid_text.isdecimal() or int(pid_text) <= 0:
        raise ProbeError(f"pid file {path} must contain a positive decimal PID")
    return ProcessTarget(role=role, pid_file=path, pid=int(pid_text))


def parse_proc_stat(stat_text: str) -> dict[str, int]:
    close = stat_text.rfind(")")
    if close == -1:
        raise ProbeError("malformed /proc stat: missing comm terminator")
    fields = stat_text[close + 2 :].split()
    if len(fields) <= 19:
        raise ProbeError("malformed /proc stat: missing timing fields")
    try:
        return {
            "utime_ticks": int(fields[11]),
            "stime_ticks": int(fields[12]),
            "start_ticks": int(fields[19]),
        }
    except ValueError as error:
        raise ProbeError("malformed /proc stat: non-numeric timing field") from error


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_identity(pid: int) -> dict[str, object]:
    proc_dir = PROC_ROOT / str(pid)
    stat_path = proc_dir / "stat"
    exe_path = proc_dir / "exe"
    try:
        stat_fields = parse_proc_stat(stat_path.read_text(encoding="utf-8"))
        exe_target = os.readlink(exe_path)
        exe_stat = exe_path.stat()
    except FileNotFoundError as error:
        raise ProbeError(f"pid {pid} is not alive") from error
    except OSError as error:
        raise ProbeError(f"pid {pid} identity is unreadable: {error}") from error
    return {
        "pid": pid,
        "start_ticks": stat_fields["start_ticks"],
        "exe_path": exe_target,
        "exe_dev": exe_stat.st_dev,
        "exe_inode": exe_stat.st_ino,
        "utime_ticks": stat_fields["utime_ticks"],
        "stime_ticks": stat_fields["stime_ticks"],
    }


def assert_same_identity(role: str, before: dict[str, object], after: dict[str, object]) -> None:
    for key in ("pid", "start_ticks", "exe_dev", "exe_inode"):
        if before.get(key) != after.get(key):
            raise ProbeError(f"{role} process identity changed during capture at {key}")


def parse_status_fields(text: str) -> dict[str, str]:
    wanted = {"VmSize", "VmRSS", "VmHWM", "Threads", "State", "Name"}
    values: dict[str, str] = {}
    for line in text.splitlines():
        key, sep, value = line.partition(":")
        if sep and key in wanted:
            values[key] = value.strip()
    return values


def read_proc_io(pid: int) -> dict[str, object]:
    path = PROC_ROOT / str(pid) / "io"
    observation = read_text(path)
    if observation["status"] != "OBSERVED":
        return observation
    parsed: dict[str, int] = {}
    value = observation.get("value")
    assert isinstance(value, str)
    for line in value.splitlines():
        key, sep, raw = line.partition(":")
        if sep:
            try:
                parsed[key.strip()] = int(raw.strip())
            except ValueError:
                return unavailable("non-numeric /proc io field", path=str(path), raw=value)
    return {"status": "OBSERVED", "path": str(path), "fields": parsed}


def count_dir_entries(path: pathlib.Path) -> dict[str, object]:
    try:
        return {"status": "OBSERVED", "path": str(path), "count": sum(1 for _ in path.iterdir())}
    except OSError as error:
        return unavailable(str(error), path=str(path))


def cgroup_path_for_pid(pid: int) -> pathlib.Path | None:
    observation = read_text(PROC_ROOT / str(pid) / "cgroup")
    value = observation.get("value")
    if not isinstance(value, str):
        return None
    for line in value.splitlines():
        if line.startswith("0::"):
            return CGROUP_ROOT / line[3:].lstrip("/")
    return None


def collect_cgroup_ancestors(pid: int) -> dict[str, object]:
    leaf = cgroup_path_for_pid(pid)
    if leaf is None:
        return unavailable("unified cgroup membership not observed")
    try:
        root = CGROUP_ROOT.resolve()
        current = leaf.resolve()
    except OSError as error:
        return unavailable(str(error), path=str(leaf))
    try:
        current.relative_to(root)
    except ValueError:
        return unavailable("cgroup path escapes visible hierarchy", path=str(current))
    ancestors: list[dict[str, object]] = []
    while True:
        ancestors.append({
            "path": str(current),
            "fields": {
                name: read_text(current / name)
                for name in ("memory.current", "memory.max", "cpu.max", "pids.current", "pids.max")
            },
        })
        if current == root:
            break
        current = current.parent
    return {"status": "OBSERVED", "version": 2, "ancestors": ancestors}


def collect_process(target: ProcessTarget) -> dict[str, object]:
    before = read_identity(target.pid)
    try:
        exe_sha256 = sha256_file(PROC_ROOT / str(target.pid) / "exe")
    except OSError as error:
        raise ProbeError(f"{target.role} executable hash failed: {error}") from error
    status_observation = read_text(PROC_ROOT / str(target.pid) / "status")
    status_fields: dict[str, str] | dict[str, object]
    if status_observation["status"] == "OBSERVED":
        status_fields = parse_status_fields(str(status_observation["value"]))
    else:
        status_fields = status_observation
    io = read_proc_io(target.pid)
    fd_count = count_dir_entries(PROC_ROOT / str(target.pid) / "fd")
    task_count = count_dir_entries(PROC_ROOT / str(target.pid) / "task")
    cgroup = collect_cgroup_ancestors(target.pid)
    after = read_identity(target.pid)
    assert_same_identity(target.role, before, after)
    return {
        "role": target.role,
        "pid_file": str(target.pid_file),
        "identity": {
            "pid": before["pid"],
            "start_ticks": before["start_ticks"],
            "exe_path": before["exe_path"],
            "exe_dev": before["exe_dev"],
            "exe_inode": before["exe_inode"],
            "exe_sha256": exe_sha256,
            "stable_pre_post": True,
        },
        "status_fields": status_fields,
        "stat_ticks": {"utime": after["utime_ticks"], "stime": after["stime_ticks"]},
        "io": io,
        "fd_count": fd_count,
        "task_count": task_count,
        "cgroup": cgroup,
    }


def run_command(argv: list[str]) -> dict[str, object]:
    started = time.monotonic()
    try:
        result = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=COMMAND_TIMEOUT_SECONDS,
            check=False,
        )
        duration = time.monotonic() - started
        return {
            "argv": argv,
            "exit_code": result.returncode,
            "duration_seconds": duration,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "status": "OBSERVED" if result.returncode == 0 else "ERROR",
        }
    except subprocess.TimeoutExpired as error:
        return {
            "argv": argv,
            "exit_code": None,
            "duration_seconds": time.monotonic() - started,
            "stdout": error.stdout or "",
            "stderr": error.stderr or "",
            "status": "TIMEOUT",
            "timeout_seconds": COMMAND_TIMEOUT_SECONDS,
        }
    except OSError as error:
        return {
            "argv": argv,
            "exit_code": 127,
            "duration_seconds": time.monotonic() - started,
            "stdout": "",
            "stderr": str(error),
            "status": "ERROR",
        }


def collect_rdma() -> dict[str, object]:
    if shutil.which("rdma") is None:
        return unavailable("rdma command not found")
    resources: dict[str, object] = {}
    error_types: list[str] = []
    for resource_type in RDMA_RES_TYPES:
        command = run_command(["rdma", "res", "show", resource_type, "-j"])
        parsed: object
        if command["status"] == "OBSERVED":
            try:
                parsed = json.loads(str(command["stdout"]))
            except json.JSONDecodeError as error:
                command["status"] = "ERROR"
                command["parse_error"] = str(error)
                parsed = None
        else:
            parsed = None
        if command["status"] != "OBSERVED":
            error_types.append(resource_type)
        resources[resource_type] = {"command": command, "parsed": parsed}
    return {
        "status": "OBSERVED" if not error_types else "PARTIAL",
        "error_types": error_types,
        "resources": resources,
    }


def collect_machine() -> dict[str, object]:
    return {
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "boot": {"uptime": read_text(PROC_ROOT / "uptime"), "boot_id": read_text(PROC_ROOT / "sys" / "kernel" / "random" / "boot_id")},
        "kernel": {"release": platform.release(), "version": platform.version()},
        "machine": {
            "hostname": platform.node(),
            "architecture": platform.machine(),
            "cpu_count": os.cpu_count(),
            "loadavg": os.getloadavg(),
            "meminfo": read_text(PROC_ROOT / "meminfo"),
        },
    }


def mount_metadata_for(path: pathlib.Path) -> dict[str, object]:
    target = path.resolve(strict=False)
    best: tuple[int, dict[str, str]] | None = None
    mounts = read_text(PROC_ROOT / "self" / "mountinfo")
    value = mounts.get("value")
    if not isinstance(value, str):
        return mounts
    for line in value.splitlines():
        left, sep, right = line.partition(" - ")
        if not sep:
            continue
        left_fields = left.split()
        right_fields = right.split()
        if len(left_fields) < 5 or len(right_fields) < 3:
            continue
        mount_point = pathlib.Path(left_fields[4].replace("\\040", " "))
        try:
            target.relative_to(mount_point)
        except ValueError:
            continue
        score = len(str(mount_point))
        record = {
            "mount_id": left_fields[0],
            "parent_id": left_fields[1],
            "major_minor": left_fields[2],
            "root": left_fields[3],
            "mount_point": str(mount_point),
            "mount_options": left_fields[5],
            "fs_type": right_fields[0],
            "source": right_fields[1],
            "super_options": right_fields[2],
        }
        if best is None or score > best[0]:
            best = (score, record)
    if best is None:
        return unavailable("no mountinfo entry matched path", path=str(path), resolved=str(target))
    return {"status": "OBSERVED", **best[1]}


def collect_volume(path: pathlib.Path) -> dict[str, object]:
    try:
        stat = os.statvfs(path)
        statvfs = {
            "f_bsize": stat.f_bsize,
            "f_frsize": stat.f_frsize,
            "f_blocks": stat.f_blocks,
            "f_bfree": stat.f_bfree,
            "f_bavail": stat.f_bavail,
            "f_files": stat.f_files,
            "f_ffree": stat.f_ffree,
            "f_favail": stat.f_favail,
            "f_flag": stat.f_flag,
            "f_namemax": stat.f_namemax,
        }
    except OSError as error:
        return unavailable(str(error), path=str(path))
    return {"status": "OBSERVED", "path": str(path), "statvfs": statvfs, "mount": mount_metadata_for(path)}


def collect_report(targets: Iterable[ProcessTarget], volumes: Iterable[pathlib.Path]) -> dict[str, object]:
    process_records = {target.role: collect_process(target) for target in targets}
    if process_records and all(
        record.get("identity", {}).get("stable_pre_post") is True for record in process_records.values()
    ):
        status = "PASS"
    elif process_records:
        status = "FAIL"
    else:
        status = "NO_PROCESS_TARGETS"
    return {
        "schema": "afs-round3-resource-probe/v1",
        "status": status,
        "formal_acceptance": "NOT_RUN",
        "environment": "PREPARING",
        "scope": "unqualified development resource snapshot",
        "qualification_limits": [
            "top-level PASS only means requested process identities stayed stable while sampled",
            "partial RDMA resource command failures remain explicit and do not fail stable process capture",
            "volume observations are filesystem metadata snapshots, not durability or performance qualification",
        ],
        "collector": {
            "argv": sys.argv,
            "pid": os.getpid(),
            "uid": os.geteuid(),
            "timeout_seconds_per_subprocess": COMMAND_TIMEOUT_SECONDS,
        },
        "machine": collect_machine(),
        "processes": process_records,
        "rdma": collect_rdma(),
        "volumes": {str(path): collect_volume(path) for path in volumes},
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__,
        epilog=(
            "Typical topology roles are afs-accept-ctl and node-a/node-b/node-c; "
            "typical volumes include /mnt/lima-afsctlstate and AFS mount paths."
        ),
    )
    parser.add_argument("--pid-file", action="append", default=[], metavar="ROLE=PATH")
    parser.add_argument("--volume", action="append", default=[], metavar="PATH")
    parser.add_argument("--output", metavar="PATH", help="write JSON report to PATH instead of stdout")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        require_linux_aarch64_root()
        targets = [parse_pid_file(value) for value in args.pid_file]
        volumes = [pathlib.Path(value) for value in args.volume]
        report = collect_report(targets, volumes)
        encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if args.output:
            with pathlib.Path(args.output).open("x", encoding="utf-8") as handle:
                handle.write(encoded)
        else:
            sys.stdout.write(encoded)
    except ProbeError as error:
        print(f"round3 resource probe error: {error}", file=sys.stderr)
        return 2
    except BrokenPipeError:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

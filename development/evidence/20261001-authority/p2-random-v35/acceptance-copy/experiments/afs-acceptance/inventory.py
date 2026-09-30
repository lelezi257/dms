#!/usr/bin/env python3
"""Collect observed Linux guest state, never expected readiness values.

Process identity probes are evidence only. They do not imply readiness.
They intentionally avoid /proc/<pid>/cmdline and environ because those may
contain secrets.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import Iterable

REQUIRED_SYSTEM = "Linux"
REQUIRED_MACHINE = "aarch64"
PROC_ROOT = pathlib.Path("/proc")
PROCESS_NAME_RE = re.compile(r"^[A-Za-z0-9_.-]+$")


class InventoryError(RuntimeError):
    pass


@dataclass(frozen=True)
class ProcessProbe:
    name: str
    pid: int
    source: str


def require_linux_arm64() -> None:
    if platform.system() != REQUIRED_SYSTEM or platform.machine() != REQUIRED_MACHINE:
        raise InventoryError("dedicated ARM64 Linux required")


def command(argv: list[str]) -> dict[str, object]:
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=15)
    except FileNotFoundError as error:
        return {
            "argv": argv,
            "returncode": 127,
            "stdout": "",
            "stderr": str(error),
        }
    return {
        "argv": argv,
        "returncode": result.returncode,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_pid(value: str, source: str) -> int:
    text = value.strip()
    if not text or not text.isdecimal():
        raise InventoryError(f"{source} must contain a positive decimal PID")
    pid = int(text)
    if pid <= 0:
        raise InventoryError(f"{source} must contain a positive decimal PID")
    return pid


def validate_process_name(name: str) -> str:
    if not PROCESS_NAME_RE.fullmatch(name):
        raise InventoryError(
            f"process name {name!r} must match {PROCESS_NAME_RE.pattern}; do not use paths"
        )
    return name


def parse_process_assignment(value: str) -> ProcessProbe:
    if "=" not in value:
        raise InventoryError("--process expects NAME=PID")
    name, pid_text = value.split("=", 1)
    name = validate_process_name(name)
    return ProcessProbe(name=name, pid=parse_pid(pid_text, f"--process {name}"), source="argv")


def parse_pid_file_assignment(value: str) -> ProcessProbe:
    if "=" not in value:
        raise InventoryError("--pid-file expects NAME=PATH")
    name, path_text = value.split("=", 1)
    name = validate_process_name(name)
    path = pathlib.Path(path_text)
    pid = parse_pid(path.read_text(encoding="utf-8"), str(path))
    return ProcessProbe(name=name, pid=pid, source=str(path))


def parse_start_ticks(stat_text: str) -> int:
    close = stat_text.rfind(")")
    if close == -1:
        raise InventoryError("/proc stat is malformed: missing comm terminator")
    tail = stat_text[close + 2 :].split()
    # proc_pid_stat(5): fields after comm start at field 3, so starttime field
    # 22 is tail index 19.
    if len(tail) <= 19:
        raise InventoryError("/proc stat is malformed: missing starttime")
    try:
        return int(tail[19])
    except ValueError as exc:
        raise InventoryError("/proc stat has non-numeric starttime") from exc


def read_process_fingerprint(pid: int, proc_root: pathlib.Path = PROC_ROOT) -> dict[str, object]:
    proc_dir = proc_root / str(pid)
    stat_path = proc_dir / "stat"
    exe_path = proc_dir / "exe"
    try:
        stat_text = stat_path.read_text(encoding="utf-8")
        exe_target = os.readlink(exe_path)
        exe_stat = exe_path.stat()
    except FileNotFoundError as exc:
        raise InventoryError(f"pid {pid} is not alive") from exc
    except ProcessLookupError as exc:
        raise InventoryError(f"pid {pid} disappeared while reading identity") from exc
    except PermissionError as exc:
        raise InventoryError(f"pid {pid} identity is not readable: {exc}") from exc
    return {
        "pid": pid,
        "start_ticks": parse_start_ticks(stat_text),
        "exe_path": exe_target,
        "exe_dev": exe_stat.st_dev,
        "exe_inode": exe_stat.st_ino,
    }


def assert_same_process(before: dict[str, object], after: dict[str, object], name: str) -> None:
    for key in ("pid", "start_ticks", "exe_dev", "exe_inode"):
        if before[key] != after[key]:
            raise InventoryError(
                f"process {name} changed while probing: {key} {before[key]!r} -> {after[key]!r}"
            )


def collect_process_identity(probe: ProcessProbe, proc_root: pathlib.Path = PROC_ROOT) -> dict[str, object]:
    before = read_process_fingerprint(probe.pid, proc_root)
    exe_proc_path = proc_root / str(probe.pid) / "exe"
    try:
        digest = sha256_file(exe_proc_path)
    except FileNotFoundError as exc:
        raise InventoryError(f"pid {probe.pid} exited before binary hash completed") from exc
    except PermissionError as exc:
        raise InventoryError(f"pid {probe.pid} executable is not readable: {exc}") from exc
    after = read_process_fingerprint(probe.pid, proc_root)
    assert_same_process(before, after, probe.name)
    return {
        "name": probe.name,
        "pid": probe.pid,
        "pid_source": probe.source,
        "exe_path": before["exe_path"],
        "sha256": digest,
        "start_ticks": before["start_ticks"],
        "exe_dev": before["exe_dev"],
        "exe_inode": before["exe_inode"],
    }


def collect_processes(probes: Iterable[ProcessProbe]) -> dict[str, dict[str, object]]:
    processes: dict[str, dict[str, object]] = {}
    for probe in probes:
        if probe.name in processes:
            raise InventoryError(f"duplicate process name {probe.name!r}")
        processes[probe.name] = collect_process_identity(probe)
    return processes


def collect_guest_state(probes: Iterable[ProcessProbe]) -> dict[str, object]:
    rdma = pathlib.Path("/var/lib/afs-acceptance")
    state: dict[str, object] = {
        "observed_at_unix": time.time(),
        "hostname": platform.node(),
        "architecture": platform.machine(),
        "kernel": platform.release(),
        "cpu_count": os.cpu_count(),
        "meminfo": pathlib.Path("/proc/meminfo").read_text(encoding="utf-8"),
        "swap": pathlib.Path("/proc/swaps").read_text(encoding="utf-8"),
        "os_release": pathlib.Path("/etc/os-release").read_text(encoding="utf-8"),
        "fuse_present": pathlib.Path("/dev/fuse").exists(),
    }
    for name, argv in {
        "addresses": ["ip", "-j", "address"],
        "routes": ["ip", "-j", "route"],
        "volumes": ["findmnt", "--json", "--real"],
        "disks": ["lsblk", "-J", "-b", "-o", "NAME,SIZE,FSTYPE,MOUNTPOINTS"],
        "rdma_links": ["rdma", "link", "show"],
        "rdma_device": ["ibv_devinfo", "-d", "rxe0", "-v"],
        "packages": ["dpkg-query", "-W", "-f=${Package} ${Version}\n"],
    }.items():
        state[name] = command(argv)
    for name in ["gids.txt", "ibv-devinfo.txt"]:
        path = rdma / name
        state[name] = path.read_text(encoding="utf-8") if path.exists() else None
    packages = state["packages"]
    assert isinstance(packages, dict)
    state["packages_sha256"] = hashlib.sha256(str(packages["stdout"]).encode()).hexdigest()
    process_identities = collect_processes(probes)
    if process_identities:
        state["processes"] = process_identities
    return state


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--process",
        action="append",
        default=[],
        metavar="NAME=PID",
        help="capture a live process binary identity from /proc without reading cmdline/environ",
    )
    parser.add_argument(
        "--pid-file",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="read PID from PATH and capture that live process identity",
    )
    return parser


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    return build_parser().parse_args(argv)


def probes_from_args(args: argparse.Namespace) -> list[ProcessProbe]:
    probes = [parse_process_assignment(value) for value in args.process]
    probes.extend(parse_pid_file_assignment(value) for value in args.pid_file)
    return probes


def main(argv: list[str] | None = None) -> int:
    try:
        require_linux_arm64()
        args = parse_args(argv)
        state = collect_guest_state(probes_from_args(args))
    except InventoryError as error:
        print(f"inventory error: {error}", file=sys.stderr)
        return 2
    print(json.dumps(state, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

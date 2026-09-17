#!/usr/bin/env python3
"""DMS M1 POSIX 上游子集 runner 共用逻辑。

这个模块只服务 M1 验收脚本，不进入产品代码。它负责：

1. 读取冻结 allowlist；
2. 启动一个真实 DMS 单 VM FUSE mount，或在单测中使用传入 mountpoint；
3. 逐条运行上游 suite；
4. 把 preflight、stdout/stderr、机器结果落到证据目录。

注意：上游 suite 不存在时结果是 FAIL/preflight，不是 SKIP。这样总验收会如实暴露
环境缺口，避免把“没有跑”误报成“通过”。
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import tempfile
import time
from typing import Any, Iterable
import urllib.request


ROOT = Path(__file__).resolve().parents[2]


class SuiteError(RuntimeError):
    """POSIX suite 配置、环境或执行失败。"""


def command(
    argv: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    capture: bool = True,
    check: bool = False,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        argv,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        check=False,
    )
    if check and completed.returncode:
        detail = ((completed.stdout or "") + (completed.stderr or ""))[-4000:]
        raise SuiteError(f"command failed ({completed.returncode}): {shlex.join(argv)}\n{detail}")
    return completed


def git_value(*args: str, cwd: Path = ROOT) -> str:
    return command(["git", *args], cwd=cwd, check=True).stdout.strip()


def optional_git_value(*args: str, cwd: Path) -> str | None:
    try:
        return git_value(*args, cwd=cwd)
    except (SuiteError, FileNotFoundError):
        return None


def source_identity() -> dict[str, Any]:
    commit = optional_git_value("rev-parse", "HEAD", cwd=ROOT) or os.environ.get("DMS_SOURCE_COMMIT") or "unknown"
    branch = optional_git_value("branch", "--show-current", cwd=ROOT) or os.environ.get("DMS_SOURCE_BRANCH") or "unknown"
    dirty_raw = optional_git_value("status", "--porcelain", cwd=ROOT)
    return {
        "commit": commit,
        "branch": branch,
        "dirty": bool(dirty_raw) if dirty_raw is not None else None,
    }


def suite_identity(suite_dir: Path) -> dict[str, Any]:
    """记录上游 suite 身份，保证 evidence 可复现。

    suite 目录通常是 git clone；如果维护者传入的是源码包，也要诚实记录为
    unknown，而不是阻断 preflight。真实 case 是否执行由 runner 后续判断。
    """

    head = optional_git_value("rev-parse", "HEAD", cwd=suite_dir)
    status = optional_git_value("status", "--porcelain", cwd=suite_dir)
    origin = optional_git_value("remote", "get-url", "origin", cwd=suite_dir)
    return {
        "path": str(suite_dir.resolve()),
        "vcs": "git" if head else "unknown",
        "commit": head or "unknown",
        "origin": origin or "unknown",
        "dirty": bool(status) if status is not None else None,
    }


def environment(profile: str) -> dict[str, str]:
    return {"kernel": platform.release(), "arch": platform.machine(), "profile": profile}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_allowlist(path: Path) -> list[tuple[str, str]]:
    cases: list[tuple[str, str]] = []
    for number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if "|" not in line:
            raise SuiteError(f"{path}:{number}: allowlist line must be '<id>|<command>'")
        case_id, command_text = (part.strip() for part in line.split("|", 1))
        if not case_id or not command_text:
            raise SuiteError(f"{path}:{number}: allowlist id and command must be non-empty")
        cases.append((case_id, command_text))
    if not cases:
        raise SuiteError(f"allowlist is empty: {path}")
    return cases


def write_text(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value, encoding="utf-8")


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


class DmsSingleMount:
    """单 VM DMS FUSE mount 生命周期。

    POSIX 上游 suite 只需要一个挂载点；两个 Node 的跨节点一致性已经由其它 M1.7
    case 覆盖。这里专门验证“内核 POSIX 调用进入 DMS FUSE 后语义正确”。
    """

    def __init__(self, output: Path, port_base: int) -> None:
        self.output = output
        self.port_base = port_base
        self.target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        self.run_dir = Path(tempfile.mkdtemp(prefix="dms-posix-suite."))
        self.mountpoint = self.run_dir / "mnt"
        self.journal = self.run_dir / "meta-journal"
        self.meta_grpc = f"127.0.0.1:{port_base + 1}"
        self.meta_health = f"127.0.0.1:{port_base + 2}"
        self.node_worker = f"127.0.0.1:{port_base + 3}"
        self.node_health = f"127.0.0.1:{port_base + 4}"
        self.processes: list[subprocess.Popen[str]] = []

    def cleanup(self) -> None:
        try:
            command(["fusermount3", "-uz", str(self.mountpoint)], check=False)
        except FileNotFoundError:
            pass
        try:
            command(["umount", "-l", str(self.mountpoint)], check=False)
        except FileNotFoundError:
            pass
        for process in self.processes:
            process.terminate()
        for process in self.processes:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
        shutil.rmtree(self.run_dir)

    def wait_http(self, address: str) -> None:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(f"http://{address}/readyz", timeout=1):
                    return
            except OSError:
                time.sleep(0.1)
        raise SuiteError(f"timed out waiting for http://{address}/readyz")

    def wait_mount(self) -> None:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if command(["mountpoint", "-q", str(self.mountpoint)]).returncode == 0:
                self.wait_http(self.node_health)
                return
            time.sleep(0.1)
        raise SuiteError(f"timed out waiting for DMS FUSE mount: {self.mountpoint}")

    def start_process(self, argv: list[str], log_name: str) -> None:
        log = (self.output / log_name).open("w", encoding="utf-8")
        process = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, text=True)
        self.processes.append(process)

    def start(self) -> Path:
        if platform.system() != "Linux" or not Path("/dev/fuse").exists():
            raise SuiteError("POSIX suite requires Linux with /dev/fuse")
        for tool in ("cargo", "fusermount3", "mountpoint", "python3"):
            if shutil.which(tool) is None:
                raise SuiteError(f"required tool is missing: {tool}")
        self.mountpoint.mkdir(parents=True)
        self.journal.mkdir(parents=True)
        if os.environ.get("DMS_POSIX_SKIP_BUILD") != "1":
            command(["cargo", "build", "-p", "dms-server", "--bins", "--features", "fuse"], cwd=ROOT, check=True)
        bin_dir = Path(os.environ.get("DMS_SERVER_BIN_DIR", str(self.target_dir / "debug")))
        meta = bin_dir / "dms-meta"
        node = bin_dir / "dms-node"
        for binary in (meta, node):
            if not binary.is_file():
                raise SuiteError(f"DMS server binary is missing: {binary}")
        self.start_process(
            [
                str(meta),
                "serve",
                "--node-id",
                "meta-m1-posix-suite",
                "--grpc-address",
                self.meta_grpc,
                "--health-address",
                self.meta_health,
                "--journal-dir",
                str(self.journal),
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ],
            "meta.log",
        )
        self.wait_http(self.meta_health)
        self.start_process(
            [
                str(node),
                "serve",
                "--node-id",
                "node-m1-posix-suite",
                "--meta-endpoint",
                f"http://{self.meta_grpc}",
                "--worker-tcp-address",
                self.node_worker,
                "--health-address",
                self.node_health,
                "--fuse-mountpoint",
                str(self.mountpoint),
                "--arena-capacity-bytes",
                str(256 * 1024 * 1024),
                "--region-size-bytes",
                str(64 * 1024 * 1024),
                "--node-current-cache-bytes",
                str(8 * 1024 * 1024),
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ],
            "node.log",
        )
        self.wait_mount()
        return self.mountpoint


class PosixSuiteRunner:
    def __init__(
        self,
        *,
        case_id: str,
        suite_name: str,
        output: Path,
        purpose: str,
        allowlist: Path,
        suite_dir: Path,
        mountpoint: Path | None,
        port_base: int,
    ) -> None:
        self.case_id = case_id
        self.suite_name = suite_name
        self.output = output.resolve()
        self.purpose = purpose
        self.allowlist = allowlist
        self.suite_dir = suite_dir
        self.mountpoint = mountpoint
        self.port_base = port_base
        self.started_ns = time.monotonic_ns()
        self._mount: DmsSingleMount | None = None

    def preflight(self, commands: Iterable[str]) -> None:
        if not self.suite_dir.is_dir():
            raise SuiteError(f"{self.suite_name} suite dir is missing: {self.suite_dir}")
        for tool in commands:
            if shutil.which(tool) is None:
                raise SuiteError(f"required tool is missing: {tool}")

    def dms_mountpoint(self) -> Path:
        if self.mountpoint is not None:
            if not self.mountpoint.is_dir():
                raise SuiteError(f"provided mountpoint is not a directory: {self.mountpoint}")
            return self.mountpoint
        self._mount = DmsSingleMount(self.output, self.port_base)
        return self._mount.start()

    def finish_case(
        self,
        *,
        status: str,
        message: str,
        evidence: list[Path],
        extra: dict[str, Any] | None = None,
    ) -> int:
        result = {
            "schema": "dms.m1.acceptance-result.v1",
            "purpose": self.purpose,
            "tier": "full",
            "topology": "single-vm",
            "source": source_identity(),
            "environment": environment(f"m1-{self.suite_name}"),
            "cases": [
                {
                    "id": self.case_id,
                    "status": status,
                    "duration_ms": (time.monotonic_ns() - self.started_ns) / 1_000_000.0,
                    "message": message,
                    "evidence": [str(path.resolve()) for path in evidence],
                }
            ],
        }
        if extra:
            result["cases"][0]["extra"] = extra
        write_json(self.output / "m1-result.json", result)
        write_text(self.output / "result.txt", f"{status}\n{message}\n")
        return 0 if status == "PASS" else 2

    def cleanup(self) -> None:
        if self._mount is not None:
            self._mount.cleanup()

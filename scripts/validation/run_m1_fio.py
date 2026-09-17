#!/usr/bin/env python3
"""Run the M1 fio-integrity acceptance case.

默认单 VM 模式会启动一个 Meta、两个 Node 和两个 FUSE mount。三 VM 模式复用
同一套 workload 语义，但把写入和校验分别放到两个独立 VM 的挂载点上。
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
WORKLOAD = ROOT / "scripts/validation/fio_integrity_workload.py"
EVALUATOR = ROOT / "scripts/validation/evaluate_m1_fio.py"
CASE_ID = "fio-integrity"
RUN_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}")


def command(argv: list[str], *, capture: bool = False, check: bool = True, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, text=True, capture_output=capture, check=False, cwd=cwd)
    if check and completed.returncode:
        detail = (completed.stdout or "") + (completed.stderr or "")
        raise RuntimeError(f"command failed ({completed.returncode}): {shlex.join(argv)}\n{detail[-4000:]}")
    return completed


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validated_run_id(value: str) -> str:
    if RUN_ID_PATTERN.fullmatch(value) is None:
        raise argparse.ArgumentTypeError(
            "run-id must be 1-64 ASCII letters, digits, '_' or '-', and start alphanumeric"
        )
    return value


def parse_size(value: str) -> int:
    raw = value.strip().lower()
    suffixes = {"k": 1024, "m": 1024**2, "g": 1024**3}
    if raw[-1:] in suffixes:
        return int(raw[:-1]) * suffixes[raw[-1]]
    return int(raw)


def arena_capacity_for_workload(large_size: str) -> int:
    """为完整 workload 保留大对象之外的验证对象和版本空间。

    同一 Node 会依次保存 4 KiB、1 MiB 与 large object，随后还会执行 truncate、
    打洞和 mmap。Arena 若恰好等于 large object 大小，测到的只是人工配置导致的
    ENOSPC，而不是 512 MiB 数据完整性。这里至少留 256 MiB 余量，并设置 1 GiB
    验收下限；不改变产品默认配置，也不放宽数据校验阈值。
    """

    return max(1024**3, parse_size(large_size) + 256 * 1024**2)


def git_value(*args: str) -> str:
    return command(["git", *args], capture=True, cwd=ROOT).stdout.strip()


def source_identity() -> dict[str, Any]:
    return {
        "commit": git_value("rev-parse", "HEAD"),
        "branch": git_value("branch", "--show-current"),
        "dirty": bool(git_value("status", "--porcelain")),
    }


def environment(profile: str) -> dict[str, str]:
    return {"kernel": platform.release(), "arch": platform.machine(), "profile": profile}


def write_m1_result(
    output: Path,
    *,
    purpose: str,
    topology: str,
    status: str,
    message: str,
    evidence: list[Path],
    started_ns: int,
) -> None:
    result = {
        "schema": "dms.m1.acceptance-result.v1",
        "purpose": purpose,
        "tier": "full",
        "topology": topology,
        "source": source_identity(),
        "environment": environment(f"m1-fio-{topology}"),
        "cases": [
            {
                "id": CASE_ID,
                "status": status,
                "duration_ms": (time.monotonic_ns() - started_ns) / 1_000_000.0,
                "message": message,
                "evidence": [str(path.resolve()) for path in evidence],
            }
        ],
    }
    (output / "m1-result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


class SingleVmHarness:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        self.output = args.output.resolve()
        self.run_dir = Path(tempfile.mkdtemp(prefix="dms-m1-fio."))
        self.mount_a = self.run_dir / "mnt-a"
        self.mount_b = self.run_dir / "mnt-b"
        self.journal = self.run_dir / "meta-journal"
        self.meta_grpc = args.meta_grpc
        self.meta_health = args.meta_health
        self.node_a_worker = args.node_a_worker
        self.node_a_health = args.node_a_health
        self.node_b_worker = args.node_b_worker
        self.node_b_health = args.node_b_health
        self.pids: list[subprocess.Popen[str]] = []

    def cleanup(self) -> None:
        for mountpoint in (self.mount_a, self.mount_b):
            if command(["mountpoint", "-q", str(mountpoint)], check=False).returncode == 0:
                command(["fusermount3", "-uz", str(mountpoint)], check=False)
                if command(["mountpoint", "-q", str(mountpoint)], check=False).returncode == 0:
                    command(["umount", "-l", str(mountpoint)], check=False)
        for process in self.pids:
            process.terminate()
        for process in self.pids:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
        if not self.args.keep_run_dir:
            shutil.rmtree(self.run_dir)

    def wait_http(self, address: str, path: str = "readyz") -> None:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(f"http://{address}/{path}", timeout=1):
                    return
            except OSError:
                time.sleep(0.1)
        raise TimeoutError(f"timed out waiting for http://{address}/{path}")

    def wait_mount(self, mountpoint: Path, health: str) -> None:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if command(["mountpoint", "-q", str(mountpoint)], check=False).returncode == 0:
                self.wait_http(health)
                return
            time.sleep(0.1)
        raise TimeoutError(f"timed out waiting for FUSE mount: {mountpoint}")

    def start(self, argv: list[str], log_name: str) -> subprocess.Popen[str]:
        log = (self.output / log_name).open("w", encoding="utf-8")
        process = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, text=True)
        self.pids.append(process)
        return process

    def run(self) -> list[Path]:
        if platform.system() != "Linux" or not Path("/dev/fuse").exists():
            raise RuntimeError("single-vm fio-integrity requires Linux with /dev/fuse")
        for tool in ("fio", "fusermount3", "curl", "python3"):
            if shutil.which(tool) is None:
                raise RuntimeError(f"{tool} is required for fio-integrity")
        self.output.mkdir(parents=True, exist_ok=True)
        self.mount_a.mkdir(parents=True)
        self.mount_b.mkdir(parents=True)
        self.journal.mkdir(parents=True)
        command(["cargo", "build", "-p", "dms-server", "--bins", "--features", "fuse"], cwd=ROOT)
        dms_meta = self.target_dir / "debug/dms-meta"
        dms_node = self.target_dir / "debug/dms-node"

        profile = {
            "schema": "dms.m1.fio-integrity-profile.v1",
            "topology": "single-vm",
            "run_dir": str(self.run_dir),
            "large_size": self.args.large_size,
            "binaries": {"dms_meta": str(dms_meta), "dms_node": str(dms_node)},
        }
        (self.output / "profile.json").write_text(json.dumps(profile, ensure_ascii=False, indent=2) + "\n")

        self.start(
            [
                str(dms_meta),
                "serve",
                "--node-id",
                "meta-m1-fio",
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
        for name, worker, health, mount, log in (
            ("node-m1-fio-a", self.node_a_worker, self.node_a_health, self.mount_a, "node-a.log"),
            ("node-m1-fio-b", self.node_b_worker, self.node_b_health, self.mount_b, "node-b.log"),
        ):
            self.start(
                [
                    str(dms_node),
                    "serve",
                    "--node-id",
                    name,
                    "--meta-endpoint",
                    f"http://{self.meta_grpc}",
                    "--worker-tcp-address",
                    worker,
                    "--health-address",
                    health,
                    "--fuse-mountpoint",
                    str(mount),
                    "--arena-capacity-bytes",
                    str(arena_capacity_for_workload(self.args.large_size)),
                    "--region-size-bytes",
                    str(64 * 1024 * 1024),
                    "--node-current-cache-bytes",
                    str(16 * 1024 * 1024),
                    "--log-level",
                    "warn",
                    "--tracing-enabled",
                    "false",
                ],
                log,
            )
            self.wait_mount(mount, health)

        command(
            [
                sys.executable,
                str(WORKLOAD),
                "--mount-a",
                str(self.mount_a),
                "--mount-b",
                str(self.mount_b),
                "--large-size",
                self.args.large_size,
                "--node-a-metrics-url",
                f"http://{self.node_a_health}/metrics",
                "--node-b-metrics-url",
                f"http://{self.node_b_health}/metrics",
                "--output",
                str(self.output / "fio-workload.json"),
            ],
            cwd=ROOT,
        )
        for name, url in (
            ("node-a.prom", self.node_a_health),
            ("node-b.prom", self.node_b_health),
            ("meta.prom", self.meta_health),
        ):
            content = urllib.request.urlopen(f"http://{url}/metrics", timeout=3).read().decode("utf-8")
            (self.output / name).write_text(content, encoding="utf-8")
        command(
            [
                sys.executable,
                str(EVALUATOR),
                str(self.output),
                "--minimum-large-bytes",
                str(parse_size(self.args.large_size)),
                "--output",
                str(self.output / "evaluation.json"),
            ],
            cwd=ROOT,
        )
        return [
            self.output / "profile.json",
            self.output / "fio-workload.json",
            self.output / "evaluation.json",
            self.output / "node-a.prom",
            self.output / "node-b.prom",
            self.output / "meta.prom",
        ]


class ThreeVmHarness:
    """三 VM fio-integrity 编排器。

    这里只处理 fio-integrity 的最小合同：A 写，B 第一次读取校验，C 为 Meta。
    它不依赖宿主能看到远端挂载点，因此不能复用双挂载 workload。
    """

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.limactl = shutil.which("limactl")
        if self.limactl is None:
            raise RuntimeError("limactl is required for three-vm fio-integrity")
        self.vms = {"A": args.vm_a, "B": args.vm_b, "C": args.vm_c}
        self.ips = {"A": args.ip_a, "B": args.ip_b, "C": args.ip_c}
        self.remote = f"/tmp/dms-m1-fio-{args.run_id}"
        self.mount = {"A": self.remote + "/mnt", "B": self.remote + "/mnt"}
        self.output = args.output.resolve()

    def shell(self, role: str, script: str, *, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
        clean_env = "export NO_PROXY='*'; unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy; "
        return command(
            [self.limactl, "shell", "--workdir", "/tmp", self.vms[role], "--", "bash", "-lc", clean_env + script],
            capture=capture,
            check=check,
        )

    def copy_to(self, role: str, source: Path, destination: str) -> None:
        command([self.limactl, "copy", str(source), f"{self.vms[role]}:{destination}"])

    def copy_from(self, role: str, source: str, destination: Path) -> None:
        destination.parent.mkdir(parents=True, exist_ok=True)
        command([self.limactl, "copy", f"{self.vms[role]}:{source}", str(destination)])

    def wait(self, role: str, description: str, script: str, timeout: float = 40.0) -> None:
        deadline = time.monotonic() + timeout
        last = ""
        while time.monotonic() < deadline:
            result = self.shell(role, script, capture=True, check=False)
            if result.returncode == 0:
                return
            last = ((result.stdout or "") + (result.stderr or ""))[-1000:]
            time.sleep(0.1)
        raise TimeoutError(f"timed out waiting for {description} on {role}: {last!r}")

    def spawn(self, role: str, name: str, argv: list[str]) -> None:
        self.shell(
            role,
            f"nohup {shlex.join(argv)} >{shlex.quote(self.remote + '/' + name + '.log')} 2>&1 </dev/null & "
            f"echo $! >{shlex.quote(self.remote + '/' + name + '.pid')}",
        )

    def prepare(self) -> None:
        for tool, path in (("dms-node", self.args.dms_node), ("dms-meta", self.args.dms_meta)):
            if not path or not path.is_file():
                raise RuntimeError(f"{tool} binary is required for three-vm fio-integrity")
        self.output.mkdir(parents=True, exist_ok=True)
        for role in ("A", "B", "C"):
            self.shell(role, f"test ! -e {shlex.quote(self.remote)} && mkdir -p {shlex.quote(self.remote + '/bin')}")
        for role in ("A", "B"):
            self.copy_to(role, self.args.dms_node, self.remote + "/bin/dms-node")
            self.shell(role, f"chmod 755 {shlex.quote(self.remote + '/bin/dms-node')}; mkdir -p {shlex.quote(self.mount[role])}")
        self.copy_to("C", self.args.dms_meta, self.remote + "/bin/dms-meta")
        self.shell("C", f"chmod 755 {shlex.quote(self.remote + '/bin/dms-meta')}; mkdir -p {shlex.quote(self.remote + '/journal')}")

    def start_services(self) -> None:
        self.spawn(
            "C",
            "meta",
            [
                self.remote + "/bin/dms-meta",
                "serve",
                "--node-id",
                f"{self.args.run_id}-meta",
                "--grpc-address",
                f"{self.ips['C']}:{self.args.meta_port}",
                "--health-address",
                f"{self.ips['C']}:{self.args.meta_health_port}",
                "--journal-dir",
                self.remote + "/journal",
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait("C", "Meta ready", f"curl -fsS http://{self.ips['C']}:{self.args.meta_health_port}/readyz >/dev/null")
        for role in ("A", "B"):
            self.spawn(
                role,
                "node",
                [
                    self.remote + "/bin/dms-node",
                    "serve",
                    "--node-id",
                    f"{self.args.run_id}-node-{role.lower()}",
                    "--meta-endpoint",
                    f"http://{self.ips['C']}:{self.args.meta_port}",
                    "--worker-tcp-address",
                    f"{self.ips[role]}:{self.args.worker_port}",
                    "--health-address",
                    f"{self.ips[role]}:{self.args.node_health_port}",
                    "--fuse-mountpoint",
                    self.mount[role],
                    "--arena-capacity-bytes",
                    str(2 * 1024 * 1024 * 1024),
                    "--region-size-bytes",
                    str(256 * 1024 * 1024),
                    "--node-current-cache-bytes",
                    str(64 * 1024 * 1024),
                    "--log-level",
                    "warn",
                    "--tracing-enabled",
                    "false",
                ],
            )
            self.wait(role, f"Node {role} ready", f"curl -fsS http://{self.ips[role]}:{self.args.node_health_port}/readyz >/dev/null")
            self.wait(role, f"Node {role} mount", f"mountpoint -q {shlex.quote(self.mount[role])}")

    def remote_hash(self, role: str, path: str) -> str:
        script = (
            "python3 - <<'PY'\n"
            "import hashlib\n"
            f"path={path!r}\n"
            "h=hashlib.sha256()\n"
            "with open(path,'rb') as f:\n"
            "    while True:\n"
            "        b=f.read(1024*1024)\n"
            "        if not b: break\n"
            "        h.update(b)\n"
            "print(h.hexdigest())\n"
            "PY"
        )
        return self.shell(role, script, capture=True).stdout.strip().splitlines()[-1]

    def metric(self, role: str, name: str) -> float:
        ip = self.ips[role]
        script = f"curl -fsS http://{ip}:{self.args.node_health_port}/metrics | awk '/^{name}/ {{sum += $NF}} END {{print sum+0}}'"
        return float(self.shell(role, script, capture=True).stdout.strip().splitlines()[-1])

    def run_fio_remote(self, name: str, filename: str, size: str, rw: str, bs: str) -> dict[str, Any]:
        output = self.remote + f"/{name}.fio.json"
        remote_path_a = self.mount["A"] + "/" + filename
        remote_path_b = self.mount["B"] + "/" + filename
        argv = [
            "fio",
            "--name",
            name,
            "--directory",
            self.mount["A"],
            "--filename",
            filename,
            "--size",
            size,
            "--rw",
            rw,
            "--bs",
            bs,
            "--ioengine",
            "sync",
            "--verify",
            "crc32c",
            "--do_verify",
            "1",
            "--verify_fatal",
            "1",
            "--verify_dump",
            "0",
            "--verify_state_save",
            "0",
            "--aux-path",
            self.remote,
            "--randrepeat",
            "1",
            "--refill_buffers",
            "--end_fsync",
            "1",
            "--output-format",
            "json",
            "--output",
            output,
        ]
        started = time.monotonic_ns()
        self.shell("A", shlex.join(argv))
        elapsed_ms = (time.monotonic_ns() - started) / 1_000_000.0
        local_copy = self.output / f"{name}.fio.json"
        self.copy_from("A", output, local_copy)
        report = json.loads(local_copy.read_text(encoding="utf-8"))
        if any(int(job.get("error", 0)) != 0 for job in report.get("jobs", [])):
            raise AssertionError(f"fio reported error for {name}")
        local_hash = self.remote_hash("A", remote_path_a)
        remote_hash = self.remote_hash("B", remote_path_b)
        if local_hash != remote_hash:
            raise AssertionError(f"remote hash mismatch for {name}")
        return {
            "case": name,
            "status": "passed",
            "kind": "fio",
            "rw": rw,
            "bs": bs,
            "size": parse_size(size),
            "elapsed_ms": elapsed_ms,
            "fio_json": str(local_copy),
            "local_sha256": local_hash,
            "remote_sha256": remote_hash,
        }

    def run_posix_remote(self) -> list[dict[str, Any]]:
        # 复用 Python/ctypes 直接在 A 执行 mutation，在 B 做第一次业务读校验。
        self.shell(
            "A",
            "python3 - <<'PY'\n"
            "import os\n"
            "from pathlib import Path\n"
            f"p=Path({self.mount['A'] + '/truncate-integrity.bin'!r})\n"
            "payload=(b'0123456789abcdef'*4096)\n"
            "p.write_bytes(payload)\n"
            "os.truncate(p, 4096); os.truncate(p, 16384)\n"
            "with p.open('r+b') as f:\n"
            "    f.seek(12288); f.write(b'T'*4096); f.flush()\n"
            "PY"
        )
        expected_truncate = hashlib.sha256((b"0123456789abcdef" * 256) + (b"\0" * 8192) + (b"T" * 4096)).hexdigest()
        truncate_hash = self.remote_hash("B", self.mount["B"] + "/truncate-integrity.bin")
        if truncate_hash != expected_truncate:
            raise AssertionError("truncate hash mismatch on B")

        self.shell(
            "A",
            "python3 - <<'PY'\n"
            "import ctypes, os\n"
            "from pathlib import Path\n"
            f"p=Path({self.mount['A'] + '/punch-integrity.bin'!r})\n"
            "p.write_bytes(b'A'*4096+b'B'*4096+b'C'*4096)\n"
            "libc=ctypes.CDLL(None, use_errno=True); call=libc.fallocate\n"
            "call.argtypes=[ctypes.c_int,ctypes.c_int,ctypes.c_longlong,ctypes.c_longlong]\n"
            "fd=os.open(p, os.O_RDWR)\n"
            "try:\n"
            "    rc=call(fd, 0x02|0x01, 4096, 4096)\n"
            "    assert rc == 0, ctypes.get_errno()\n"
            "finally:\n"
            "    os.close(fd)\n"
            "PY"
        )
        expected_punch = hashlib.sha256(b"A" * 4096 + b"\0" * 4096 + b"C" * 4096).hexdigest()
        punch_hash = self.remote_hash("B", self.mount["B"] + "/punch-integrity.bin")
        if punch_hash != expected_punch:
            raise AssertionError("punch hash mismatch on B")

        self.shell(
            "A",
            "python3 - <<'PY'\n"
            "import mmap, os\n"
            "from pathlib import Path\n"
            f"p=Path({self.mount['A'] + '/mmap-integrity.bin'!r})\n"
            "length=64*1024; p.write_bytes(b'\\0'*length)\n"
            "fd=os.open(p, os.O_RDWR)\n"
            "try:\n"
            "    m=mmap.mmap(fd, length, access=mmap.ACCESS_WRITE)\n"
            "    try:\n"
            "        for off in range(0,length,4096): m[off:off+4096]=bytes([(off//4096)%251])*4096\n"
            "        m.flush()\n"
            "    finally: m.close()\n"
            "    os.fsync(fd)\n"
            "finally: os.close(fd)\n"
            "PY"
        )
        mmap_a = self.remote_hash("A", self.mount["A"] + "/mmap-integrity.bin")
        mmap_b = self.remote_hash("B", self.mount["B"] + "/mmap-integrity.bin")
        if mmap_a != mmap_b:
            raise AssertionError("mmap hash mismatch on B")
        return [
            {
                "case": "truncate_shrink_grow",
                "status": "passed",
                "kind": "posix",
                "size": 16384,
                "sha256": truncate_hash,
            },
            {"case": "punch_hole_zero", "status": "passed", "kind": "posix", "size": 12288, "sha256": punch_hash},
            {
                "case": "mmap_shared_hash",
                "status": "passed",
                "kind": "mmap",
                "size": 64 * 1024,
                "local_sha256": mmap_a,
                "remote_sha256": mmap_b,
            },
        ]

    def cleanup(self) -> None:
        for role in ("A", "B"):
            self.shell(role, f"fusermount3 -uz {shlex.quote(self.mount[role])} 2>/dev/null || umount -l {shlex.quote(self.mount[role])} 2>/dev/null || true", check=False)
        for role in ("A", "B", "C"):
            self.shell(role, f"for f in {shlex.quote(self.remote)}/*.pid; do test -f \"$f\" && kill $(cat \"$f\") 2>/dev/null || true; done", check=False)
        for role in ("A", "B", "C"):
            # 结果、指标和校验值均已复制到控制端，远端大文件不属于验收证据。
            self.shell(role, f"rm -rf -- {shlex.quote(self.remote)}", check=False)

    def run(self) -> list[Path]:
        self.prepare()
        self.shell("A", "command -v fio >/dev/null || { echo 'fio is required on writer VM A' >&2; exit 2; }")
        self.start_services()
        start_a = self.metric("A", "dms_node_filesystem_operations_total")
        checks = [
            self.run_fio_remote("fio_seq_4k", "fio-seq-4k.bin", "4k", "write", "4k"),
            self.run_fio_remote("fio_rand_1m", "fio-rand-1m.bin", "1m", "randwrite", "4k"),
            self.run_fio_remote("fio_large_seq", "fio-large-seq.bin", self.args.large_size, "write", "1m"),
            *self.run_posix_remote(),
        ]
        end_a = self.metric("A", "dms_node_filesystem_operations_total")
        workload = {
            "schema": "dms.m1.fio-integrity-workload.v1",
            "status": "passed",
            "large_size_bytes": parse_size(self.args.large_size),
            "checks": checks,
            "metrics": {"node_a_filesystem_ops_delta": end_a - start_a, "node_b_filesystem_ops_delta": None},
        }
        (self.output / "fio-workload.json").write_text(json.dumps(workload, ensure_ascii=False, indent=2) + "\n")
        profile = {
            "schema": "dms.m1.fio-integrity-profile.v1",
            "topology": "three-vm",
            "run_id": self.args.run_id,
            "vms": self.vms,
            "ips": self.ips,
            "large_size": self.args.large_size,
        }
        (self.output / "profile.json").write_text(json.dumps(profile, ensure_ascii=False, indent=2) + "\n")
        command(
            [
                sys.executable,
                str(EVALUATOR),
                str(self.output),
                "--minimum-large-bytes",
                str(parse_size(self.args.large_size)),
                "--output",
                str(self.output / "evaluation.json"),
            ],
            cwd=ROOT,
        )
        return [self.output / "profile.json", self.output / "fio-workload.json", self.output / "evaluation.json"]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--topology", choices=["single-vm", "three-vm"], default="single-vm")
    parser.add_argument("--purpose", choices=["discovery", "release"], default="discovery")
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT
        / f"evidence/{dt.datetime.now(dt.timezone.utc):%Y-%m-%d-m1-fio-%H%M%S}",
    )
    parser.add_argument("--large-size", default=os.environ.get("DMS_M1_FIO_LARGE_SIZE", "512m"))
    parser.add_argument("--keep-run-dir", action="store_true")
    parser.add_argument("--meta-grpc", default="127.0.0.1:30701")
    parser.add_argument("--meta-health", default="127.0.0.1:30781")
    parser.add_argument("--node-a-worker", default="127.0.0.1:30702")
    parser.add_argument("--node-a-health", default="127.0.0.1:30782")
    parser.add_argument("--node-b-worker", default="127.0.0.1:30703")
    parser.add_argument("--node-b-health", default="127.0.0.1:30783")
    parser.add_argument("--run-id", type=validated_run_id, default=f"fio-{int(time.time())}")
    parser.add_argument("--vm-a")
    parser.add_argument("--vm-b")
    parser.add_argument("--vm-c")
    parser.add_argument("--ip-a")
    parser.add_argument("--ip-b")
    parser.add_argument("--ip-c")
    parser.add_argument("--dms-node", type=Path)
    parser.add_argument("--dms-meta", type=Path)
    parser.add_argument("--meta-port", type=int, default=30701)
    parser.add_argument("--meta-health-port", type=int, default=30781)
    parser.add_argument("--worker-port", type=int, default=30702)
    parser.add_argument("--node-health-port", type=int, default=30782)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    started_ns = time.monotonic_ns()
    args.output.mkdir(parents=True, exist_ok=True)
    harness: SingleVmHarness | ThreeVmHarness
    if args.topology == "single-vm":
        harness = SingleVmHarness(args)
    else:
        required = ("vm_a", "vm_b", "vm_c", "ip_a", "ip_b", "ip_c", "dms_node", "dms_meta")
        missing = [name for name in required if getattr(args, name) in (None, "")]
        if missing:
            raise SystemExit(f"three-vm mode missing required arguments: {', '.join(missing)}")
        harness = ThreeVmHarness(args)
    try:
        evidence = harness.run()
        write_m1_result(
            args.output,
            purpose=args.purpose,
            topology=args.topology,
            status="PASS",
            message="fio verify, cross-node hash, truncate, punch and mmap integrity passed",
            evidence=evidence + [args.output / "m1-result.json"],
            started_ns=started_ns,
        )
        print(args.output)
        return 0
    except Exception as error:
        write_m1_result(
            args.output,
            purpose=args.purpose,
            topology=args.topology,
            status="FAIL",
            message=str(error),
            evidence=[args.output / "m1-result.json"],
            started_ns=started_ns,
        )
        print(f"fio-integrity failed: {error}", file=sys.stderr)
        return 1
    finally:
        harness.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Run M1 resource-return-to-baseline soak.

The case is declared for both single-vm and three-vm topologies.  Both modes
drive the same real FUSE workload through two Nodes, then record process
resources and DMS metrics before/after cleanup.  The evaluator decides whether
resources returned to the accepted budget; the runner never reuses one topology
as evidence for the other.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import tempfile
import time
from typing import Any
import urllib.request


ROOT = Path(__file__).resolve().parents[2]
BASE_PATH = Path(__file__).with_name("run_filesystem_size_semantics_3vm.py")
SPEC = importlib.util.spec_from_file_location("filesystem_size_3vm", BASE_PATH)
assert SPEC is not None and SPEC.loader is not None
BASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASE)
EVALUATOR = Path(__file__).with_name("evaluate_m1_resource_soak.py")
CASE_ID = "resource-return-to-baseline"


def command(
    argv: list[str],
    *,
    capture: bool = False,
    check: bool = True,
    cwd: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, cwd=cwd, text=True, capture_output=capture, check=False)
    if check and completed.returncode:
        detail = ((completed.stdout or "") + (completed.stderr or ""))[-4000:]
        raise RuntimeError(f"command failed ({completed.returncode}): {shlex.join(argv)}\n{detail}")
    return completed


def git_value(*args: str) -> str:
    return command(["git", *args], capture=True, cwd=ROOT).stdout.strip()


def source_identity() -> dict[str, Any]:
    return {
        "commit": git_value("rev-parse", "HEAD"),
        "branch": git_value("branch", "--show-current"),
        "dirty": bool(git_value("status", "--porcelain")),
    }


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
        "environment": {
            "kernel": platform.release(),
            "arch": platform.machine(),
            "profile": f"m1-resource-soak-{topology}",
        },
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


def metric_value(text: str, name: str) -> float:
    total = 0.0
    for line in text.splitlines():
        if line.startswith("#") or not line.startswith(name):
            continue
        metric, value = line.rsplit(maxsplit=1)
        if metric != name and not metric.startswith(name + "{"):
            continue
        try:
            total += float(value)
        except ValueError as error:
            raise ValueError(f"malformed metric value for {name}: {line}") from error
    return total


def process_snapshot_by_pid(pid: int) -> dict[str, Any]:
    status: dict[str, str] = {}
    with Path(f"/proc/{pid}/status").open(encoding="utf-8") as stream:
        for line in stream:
            if ":" in line:
                key, value = line.split(":", 1)
                status[key] = value.strip()
    rss_kb = int(status.get("VmRSS", "0 kB").split()[0])
    threads = int(status.get("Threads", "0").split()[0])
    fd_count = len(list(Path(f"/proc/{pid}/fd").iterdir()))
    return {"pid": pid, "rss_bytes": rss_kb * 1024, "threads": threads, "fd_count": fd_count}


def workload_writer_script(root: str, rounds: int, files_per_round: int) -> str:
    return (
        "import fcntl, mmap\n"
        "from pathlib import Path\n"
        f"root=Path({root!r})\n"
        "root.mkdir(exist_ok=True)\n"
        f"rounds={rounds}; files_per_round={files_per_round}\n"
        "for round_id in range(rounds):\n"
        "    batch=root / f'round-{round_id:03d}'\n"
        "    batch.mkdir()\n"
        "    for index in range(files_per_round):\n"
        "        path=batch / f'file-{index:04d}.bin'\n"
        "        data=(f'dms-resource-{round_id}-{index}\\n'.encode()) * 64\n"
        "        path.write_bytes(data)\n"
        "        with path.open('r+b') as stream:\n"
        "            fcntl.flock(stream.fileno(), fcntl.LOCK_EX)\n"
        "            size=max(4096, len(data))\n"
        "            stream.truncate(size)\n"
        "            mapping=mmap.mmap(stream.fileno(), size)\n"
        "            mapping[128:132]=b'DMS!'\n"
        "            mapping.flush(); mapping.close()\n"
        "            fcntl.flock(stream.fileno(), fcntl.LOCK_UN)\n"
    )


def workload_reader_script(root: str, rounds: int, files_per_round: int) -> str:
    return (
        "from pathlib import Path\n"
        f"root=Path({root!r})\n"
        f"rounds={rounds}; files_per_round={files_per_round}\n"
        "for round_id in range(rounds):\n"
        "    batch=root / f'round-{round_id:03d}'\n"
        "    for index in range(files_per_round):\n"
        "        path=batch / f'file-{index:04d}.bin'\n"
        "        data=path.read_bytes()\n"
        "        if data[128:132] != b'DMS!':\n"
        "            raise SystemExit(f'bad mmap payload in {path}')\n"
    )


def workload_cleanup_script(root: str) -> str:
    return (
        "from pathlib import Path\n"
        f"root=Path({root!r})\n"
        "for path in sorted(root.rglob('*'), reverse=True):\n"
        "    if path.is_file() or path.is_symlink(): path.unlink()\n"
        "    elif path.is_dir(): path.rmdir()\n"
        "root.rmdir()\n"
    )


class Harness(BASE.Harness):
    def __init__(self, args: argparse.Namespace) -> None:
        super().__init__(args)
        self.remote = f"/tmp/dms-m1-resource-{args.run_id}"
        self.mount = {role: f"{self.remote}/mnt" for role in ("A", "B")}

    def prepare(self) -> None:
        super().prepare()
        profile = json.loads((self.output / "profile.json").read_text(encoding="utf-8"))
        profile["schema"] = "dms.m1.resource-soak-profile.v1"
        profile["rounds"] = self.args.rounds
        profile["files_per_round"] = self.args.files_per_round
        (self.output / "profile.json").write_text(
            json.dumps(profile, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    def process_snapshot(self, role: str, service: str) -> dict[str, Any]:
        script = (
            f"pid=$(sed -n '1p' {shlex.quote(self.remote + '/' + service + '.pid')}); "
            "python3 - \"$pid\" <<'PY'\n"
            "import json, os, sys\n"
            "pid=sys.argv[1]\n"
            "status={}\n"
            "with open(f'/proc/{pid}/status', encoding='utf-8') as stream:\n"
            "    for line in stream:\n"
            "        if ':' in line:\n"
            "            k,v=line.split(':',1); status[k]=v.strip()\n"
            "rss_kb=int(status.get('VmRSS','0 kB').split()[0])\n"
            "threads=int(status.get('Threads','0').split()[0])\n"
            "fd_count=len(os.listdir(f'/proc/{pid}/fd'))\n"
            "print(json.dumps({'pid': int(pid), 'rss_bytes': rss_kb*1024, 'threads': threads, 'fd_count': fd_count}))\n"
            "PY"
        )
        result = self.shell(role, script, capture=True)
        return json.loads(result.stdout.strip().splitlines()[-1])

    def node_snapshot(self, role: str, service: str) -> dict[str, Any]:
        snapshot = self.process_snapshot(role, service)
        metrics = self.shell(
            role,
            f"curl -fsS http://{self.ips[role]}:{self.args.node_health_port}/metrics",
            capture=True,
        ).stdout
        snapshot.update(
            {
                "arena_allocated_bytes": metric_value(metrics, "dms_node_arena_allocated_bytes"),
                "arena_reserved_bytes": metric_value(metrics, "dms_node_arena_reserved_bytes"),
                "arena_reservations": metric_value(metrics, "dms_node_arena_reservations"),
                "filesystem_inode_references": metric_value(metrics, "dms_node_filesystem_inode_references"),
            }
        )
        (self.output / f"node-{role.lower()}-{service}-metrics.prom").write_text(metrics, encoding="utf-8")
        return snapshot

    def meta_snapshot(self) -> dict[str, Any]:
        snapshot = self.process_snapshot("C", "meta")
        metrics = self.shell(
            "C",
            f"curl -fsS http://{self.ips['C']}:{self.args.meta_health_port}/metrics",
            capture=True,
        ).stdout
        snapshot.update(
            {
                "watch_lag_events": metric_value(metrics, "dms_meta_watch_lag_events"),
                "node_sessions_live": 0.0,
            }
        )
        for line in metrics.splitlines():
            if line.startswith('dms_meta_node_sessions{state="live"} '):
                snapshot["node_sessions_live"] += float(line.rsplit(maxsplit=1)[1])
        (self.output / "meta-metrics.prom").write_text(metrics, encoding="utf-8")
        return snapshot

    def run_soak_rounds(self) -> None:
        files_per_round = self.args.files_per_round
        rounds = self.args.rounds
        self.python("A", workload_writer_script(self.mount["A"] + "/resource-soak", rounds, files_per_round))
        self.python("B", workload_reader_script(self.mount["B"] + "/resource-soak", rounds, files_per_round))
        self.python("A", workload_cleanup_script(self.mount["A"] + "/resource-soak"))
        self.wait("B", "resource soak directory removed", f"test ! -e {shlex.quote(self.mount['B'] + '/resource-soak')}")

    def execute(self) -> None:
        started_ns = time.monotonic_ns()
        self.prepare()
        completed = False
        try:
            self.start_meta()
            self.start_node("A")
            self.start_node("B")
            self.wait_meta_live_nodes(2)
            time.sleep(self.args.settle_seconds)
            before = {
                "node-a": self.node_snapshot("A", "node"),
                "node-b": self.node_snapshot("B", "node"),
                "meta": self.meta_snapshot(),
            }
            self.run_soak_rounds()
            time.sleep(self.args.settle_seconds)
            after = {
                "node-a": self.node_snapshot("A", "node"),
                "node-b": self.node_snapshot("B", "node"),
                "meta": self.meta_snapshot(),
            }
            evidence = {
                "schema": "dms.m1.resource-soak.v1",
                "status": "passed",
                "deployment": "three-vm",
                "rounds": self.args.rounds,
                "files_per_round": self.args.files_per_round,
                "expected_live_nodes": 2,
                "nodes": [
                    {"name": "node-a", "before": before["node-a"], "after": after["node-a"]},
                    {"name": "node-b", "before": before["node-b"], "after": after["node-b"]},
                ],
                "meta": {"before": before["meta"], "after": after["meta"]},
            }
            (self.output / "resource-soak.json").write_text(
                json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
            )
            BASE.command(["python3", str(EVALUATOR), str(self.output), "--output", str(self.output / "evaluation.json")])
            (self.output / "result.txt").write_text("PASS\nresource soak verified\n", encoding="utf-8")
            write_m1_result(
                self.output,
                purpose=self.args.purpose,
                topology="three-vm",
                status="PASS",
                message="resource soak verified",
                evidence=[
                    self.output / "resource-soak.json",
                    self.output / "evaluation.json",
                    self.output / "result.txt",
                ],
                started_ns=started_ns,
            )
            completed = True
        finally:
            self.collect_logs()
            self.cleanup()
            if not completed:
                raise


class SingleVmHarness:
    """单 VM 双 Node 资源回落验收。

    它启动本机 Meta、两个 Node 和两个独立 FUSE mount，拓扑与 three-vm 相同
    （A 写、B 读、Meta 独立进程），但所有进程在同一 Linux VM 内，适合本地快速
    验收 single-vm 合同。
    """

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.output = args.output.resolve()
        self.target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        self.run_dir = Path(tempfile.mkdtemp(prefix="dms-m1-resource."))
        self.mount_a = self.run_dir / "mnt-a"
        self.mount_b = self.run_dir / "mnt-b"
        self.journal = self.run_dir / "meta-journal"
        self.processes: dict[str, subprocess.Popen[str]] = {}

    def cleanup(self) -> None:
        for mountpoint in (self.mount_a, self.mount_b):
            command(["fusermount3", "-uz", str(mountpoint)], check=False)
            command(["umount", "-l", str(mountpoint)], check=False)
        for process in self.processes.values():
            process.terminate()
        for process in self.processes.values():
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
        if not self.args.keep_run_dir:
            shutil.rmtree(self.run_dir)

    def wait_http(self, address: str, path: str = "readyz") -> None:
        deadline = time.monotonic() + 30
        last = ""
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(f"http://{address}/{path}", timeout=1):
                    return
            except OSError as error:
                last = str(error)
                time.sleep(0.1)
        raise TimeoutError(f"timed out waiting for http://{address}/{path}: {last}")

    def wait_mount(self, mountpoint: Path, health: str) -> None:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if command(["mountpoint", "-q", str(mountpoint)], check=False).returncode == 0:
                self.wait_http(health)
                return
            time.sleep(0.1)
        raise TimeoutError(f"timed out waiting for FUSE mount: {mountpoint}")

    def wait_meta_live_nodes(self, expected: int) -> None:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            metrics = urllib.request.urlopen(f"http://{self.args.meta_health}/metrics", timeout=1).read().decode("utf-8")
            live = 0.0
            for line in metrics.splitlines():
                if line.startswith('dms_meta_node_sessions{state="live"} '):
                    live += float(line.rsplit(maxsplit=1)[1])
            if live >= expected:
                return
            time.sleep(0.1)
        raise TimeoutError(f"timed out waiting for {expected} live Meta node sessions")

    def start(self, name: str, argv: list[str]) -> None:
        log = (self.output / f"{name}.log").open("w", encoding="utf-8")
        self.processes[name] = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, text=True)

    def prepare(self) -> None:
        if platform.system() != "Linux" or not Path("/dev/fuse").exists():
            raise RuntimeError("single-vm resource soak requires Linux with /dev/fuse")
        for tool in ("fusermount3", "mountpoint", "python3"):
            if shutil.which(tool) is None:
                raise RuntimeError(f"{tool} is required for single-vm resource soak")
        self.output.mkdir(parents=True, exist_ok=True)
        self.mount_a.mkdir(parents=True)
        self.mount_b.mkdir(parents=True)
        self.journal.mkdir(parents=True)
        command(["cargo", "build", "-p", "dms-server", "--bins", "--features", "fuse"], cwd=ROOT)
        profile = {
            "schema": "dms.m1.resource-soak-profile.v1",
            "topology": "single-vm",
            "run_dir": str(self.run_dir),
            "rounds": self.args.rounds,
            "files_per_round": self.args.files_per_round,
        }
        (self.output / "profile.json").write_text(json.dumps(profile, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def start_services(self) -> None:
        dms_meta = self.args.dms_meta or self.target_dir / "debug/dms-meta"
        dms_node = self.args.dms_node or self.target_dir / "debug/dms-node"
        if not dms_meta.is_file() or not dms_node.is_file():
            raise RuntimeError(f"built binaries are missing: {dms_meta}, {dms_node}")
        self.start(
            "meta",
            [
                str(dms_meta),
                "serve",
                "--node-id",
                f"{self.args.run_id}-meta",
                "--grpc-address",
                self.args.meta_grpc,
                "--health-address",
                self.args.meta_health,
                "--journal-dir",
                str(self.journal),
                "--log-level",
                self.args.log_level,
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait_http(self.args.meta_health)
        for name, worker, health, mount in (
            ("node-a", self.args.node_a_worker, self.args.node_a_health, self.mount_a),
            ("node-b", self.args.node_b_worker, self.args.node_b_health, self.mount_b),
        ):
            self.start(
                name,
                [
                    str(dms_node),
                    "serve",
                    "--node-id",
                    f"{self.args.run_id}-{name}",
                    "--meta-endpoint",
                    f"http://{self.args.meta_grpc}",
                    "--worker-tcp-address",
                    worker,
                    "--health-address",
                    health,
                    "--fuse-mountpoint",
                    str(mount),
                    "--arena-capacity-bytes",
                    str(256 * 1024 * 1024),
                    "--region-size-bytes",
                    str(64 * 1024 * 1024),
                    "--node-current-cache-bytes",
                    str(8 * 1024 * 1024),
                    "--log-level",
                    self.args.log_level,
                    "--tracing-enabled",
                    "false",
                ],
            )
            self.wait_mount(mount, health)
        self.wait_meta_live_nodes(2)

    def node_snapshot(self, name: str, health: str) -> dict[str, Any]:
        process = self.processes[name]
        assert process.pid is not None
        snapshot = process_snapshot_by_pid(process.pid)
        metrics = urllib.request.urlopen(f"http://{health}/metrics", timeout=3).read().decode("utf-8")
        snapshot.update(
            {
                "arena_allocated_bytes": metric_value(metrics, "dms_node_arena_allocated_bytes"),
                "arena_reserved_bytes": metric_value(metrics, "dms_node_arena_reserved_bytes"),
                "arena_reservations": metric_value(metrics, "dms_node_arena_reservations"),
                "filesystem_inode_references": metric_value(metrics, "dms_node_filesystem_inode_references"),
            }
        )
        (self.output / f"{name}-metrics.prom").write_text(metrics, encoding="utf-8")
        return snapshot

    def meta_snapshot(self) -> dict[str, Any]:
        process = self.processes["meta"]
        assert process.pid is not None
        snapshot = process_snapshot_by_pid(process.pid)
        metrics = urllib.request.urlopen(f"http://{self.args.meta_health}/metrics", timeout=3).read().decode("utf-8")
        snapshot.update({"watch_lag_events": metric_value(metrics, "dms_meta_watch_lag_events"), "node_sessions_live": 0.0})
        for line in metrics.splitlines():
            if line.startswith('dms_meta_node_sessions{state="live"} '):
                snapshot["node_sessions_live"] += float(line.rsplit(maxsplit=1)[1])
        (self.output / "meta-metrics.prom").write_text(metrics, encoding="utf-8")
        return snapshot

    def run_soak_rounds(self) -> None:
        command(["python3", "-c", workload_writer_script(str(self.mount_a / "resource-soak"), self.args.rounds, self.args.files_per_round)])
        command(["python3", "-c", workload_reader_script(str(self.mount_b / "resource-soak"), self.args.rounds, self.args.files_per_round)])
        command(["python3", "-c", workload_cleanup_script(str(self.mount_a / "resource-soak"))])
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if not (self.mount_b / "resource-soak").exists():
                return
            time.sleep(0.1)
        raise TimeoutError("timed out waiting for resource soak directory removal on node-b")

    def execute(self) -> None:
        started_ns = time.monotonic_ns()
        self.prepare()
        completed = False
        try:
            self.start_services()
            time.sleep(self.args.settle_seconds)
            before = {
                "node-a": self.node_snapshot("node-a", self.args.node_a_health),
                "node-b": self.node_snapshot("node-b", self.args.node_b_health),
                "meta": self.meta_snapshot(),
            }
            self.run_soak_rounds()
            time.sleep(self.args.settle_seconds)
            after = {
                "node-a": self.node_snapshot("node-a", self.args.node_a_health),
                "node-b": self.node_snapshot("node-b", self.args.node_b_health),
                "meta": self.meta_snapshot(),
            }
            evidence = {
                "schema": "dms.m1.resource-soak.v1",
                "status": "passed",
                "deployment": "single-vm",
                "rounds": self.args.rounds,
                "files_per_round": self.args.files_per_round,
                "expected_live_nodes": 2,
                "nodes": [
                    {"name": "node-a", "before": before["node-a"], "after": after["node-a"]},
                    {"name": "node-b", "before": before["node-b"], "after": after["node-b"]},
                ],
                "meta": {"before": before["meta"], "after": after["meta"]},
            }
            (self.output / "resource-soak.json").write_text(
                json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
            )
            command(["python3", str(EVALUATOR), str(self.output), "--output", str(self.output / "evaluation.json")], cwd=ROOT)
            (self.output / "result.txt").write_text("PASS\nresource soak verified\n", encoding="utf-8")
            write_m1_result(
                self.output,
                purpose=self.args.purpose,
                topology="single-vm",
                status="PASS",
                message="resource soak verified",
                evidence=[
                    self.output / "resource-soak.json",
                    self.output / "evaluation.json",
                    self.output / "result.txt",
                ],
                started_ns=started_ns,
            )
            completed = True
        finally:
            self.cleanup()
            if not completed:
                raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--topology", choices=["single-vm", "three-vm"], default="single-vm")
    parser.add_argument("--purpose", choices=["discovery", "release"], default="discovery")
    parser.add_argument("--dms-node", type=Path)
    parser.add_argument("--dms-meta", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--run-id", type=BASE.validated_run_id, default=BASE.validated_run_id(f"resource-{int(time.time())}"))
    parser.add_argument("--keep-run-dir", action="store_true")
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--worker-port", type=int, default=32277)
    parser.add_argument("--node-health-port", type=int, default=32278)
    parser.add_argument("--meta-port", type=int, default=32377)
    parser.add_argument("--meta-health-port", type=int, default=32378)
    parser.add_argument("--meta-grpc", default="127.0.0.1:32377")
    parser.add_argument("--meta-health", default="127.0.0.1:32378")
    parser.add_argument("--node-a-worker", default="127.0.0.1:32277")
    parser.add_argument("--node-a-health", default="127.0.0.1:32278")
    parser.add_argument("--node-b-worker", default="127.0.0.1:32279")
    parser.add_argument("--node-b-health", default="127.0.0.1:32280")
    parser.add_argument("--rounds", type=int, default=8)
    parser.add_argument("--files-per-round", type=int, default=32)
    parser.add_argument("--settle-seconds", type=float, default=1.0)
    parser.add_argument("--log-level", default="warn")
    args = parser.parse_args()
    if args.topology == "three-vm":
        if args.dms_node is None or args.dms_meta is None:
            parser.error("--dms-node and --dms-meta are required for --topology three-vm")
        Harness(args).execute()
    else:
        SingleVmHarness(args).execute()
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

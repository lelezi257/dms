#!/usr/bin/env python3
"""在同一组三台 Lima VM 上运行 DMS Native Filesystem / MooseFS 对比。

拓扑固定为：A 是数据拥有者和写入客户端，B 是远端读取客户端，C 是元数据节点。
MooseFS 使用 goal=1，且只在 A 启动 ChunkServer，避免调度随机性把 B 的“远端首次
读取”偶然变成本地读取。memory lane 把 MooseFS 元数据和 chunk 目录放在 tmpfs；
disk lane 放在 VM 虚拟磁盘，两条 lane 的结果不得混为同一可靠性结论。
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import time
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
WORKLOAD = ROOT / "scripts/performance/native_vs_moosefs_workload.py"
PHASES = (
    ("workspace-create", "A"),
    ("workspace-local-hot", "A"),
    ("workspace-stat", "A"),
    ("workspace-peer-first", "B"),
    ("workspace-peer-repeat", "B"),
    ("workspace-patch", "A"),
    ("workspace-create-delete", "A"),
    ("large-create", "A"),
    ("large-local-read", "A"),
    ("large-peer-first", "B"),
    ("large-peer-repeat", "B"),
)

# `workspace-local-hot` 验收的是稳定热路径，而不是创建阶段耗时超过授权租约后
# 第一次重新遍历目录的冷启动成本。DMS 和 MooseFS 都在采集 before snapshot 前
# 完整读取同一工作集一次；warmup 结果单独落盘，不能混入正式延迟样本。
WARMUP_PHASES = {"workspace-local-hot"}


def command(argv: list[str], *, capture: bool = False) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, text=True, capture_output=True, check=False)
    if completed.returncode:
        detail = (completed.stdout + completed.stderr)[-6000:]
        raise RuntimeError(f"command failed ({completed.returncode}): {shlex.join(argv)}\n{detail}")
    if not capture and completed.stdout:
        print(completed.stdout, end="")
    return completed


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class Harness:
    def __init__(self, profile: dict[str, Any], output: Path) -> None:
        self.profile = profile
        self.output = output
        self.limactl = shutil.which("limactl")
        if self.limactl is None:
            raise RuntimeError("limactl is required on the macOS controller")
        self.run_id = str(profile["run_id"])
        self.vm = profile["vms"]
        self.ip = profile["ips"]
        self.port = profile["ports"]
        self.remote_base = f"/tmp/dms-mfs-{self.run_id}"

    def shell(self, role: str, script: str, *, capture: bool = False) -> subprocess.CompletedProcess[str]:
        return command(
            [
                self.limactl,
                "shell",
                "--workdir",
                "/tmp",
                self.vm[role],
                "--",
                "bash",
                "-lc",
                script,
            ],
            capture=capture,
        )

    def copy_to(self, role: str, source: Path, destination: str) -> None:
        command([self.limactl, "copy", str(source), f"{self.vm[role]}:{destination}"])

    def copy_from(self, role: str, source: str, destination: Path) -> None:
        destination.parent.mkdir(parents=True, exist_ok=True)
        command([self.limactl, "copy", f"{self.vm[role]}:{source}", str(destination)])

    def prepare(self) -> None:
        artifacts = {name: Path(value).resolve() for name, value in self.profile["artifacts"].items()}
        for name, path in artifacts.items():
            if not path.is_file():
                raise RuntimeError(f"artifact is missing: {name}: {path}")
        hashes = {name: sha256(path) for name, path in artifacts.items()}
        self.output.mkdir(parents=True, exist_ok=False)
        versions = {}
        systems = {}
        for role in ("A", "B", "C"):
            # MooseFS daemon 与 mount helper 的版本参数不同：master 使用 -v，
            # mfsmount 使用 --version。按角色采集，避免把 CLI 用法错误写进证据。
            moosefs_version = "mfsmaster -v" if role == "C" else "mfsmount --version"
            versions[role] = self.shell(
                role,
                f"printf 'moosefs='; ({moosefs_version} 2>&1 || true) | head -1; "
                "printf 'kernel='; uname -r; printf 'arch='; uname -m",
                capture=True,
            ).stdout
            systems[role] = json.loads(
                self.shell(
                    role,
                    "python3 - <<'PY'\n"
                    "import json, os, pathlib, platform, subprocess\n"
                    "meminfo = {}\n"
                    "for line in pathlib.Path('/proc/meminfo').read_text().splitlines():\n"
                    "    key, value = line.split(':', 1)\n"
                    "    meminfo[key] = value.strip()\n"
                    "disk_bytes = int(subprocess.check_output("
                    "['df', '-B1', '--output=size', '/'], text=True).splitlines()[-1].strip())\n"
                    "print(json.dumps({\n"
                    "    'cpu_count': os.cpu_count(),\n"
                    "    'memory_bytes': int(meminfo['MemTotal'].split()[0]) * 1024,\n"
                    "    'root_disk_bytes': disk_bytes,\n"
                    "    'kernel': platform.release(),\n"
                    "    'arch': platform.machine(),\n"
                    "}))\n"
                    "PY",
                    capture=True,
                ).stdout
            )
        source_sha = command(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture=True
        ).stdout.strip()
        resolved = {
            **self.profile,
            "source_sha": source_sha,
            "resolved_hashes": hashes,
            "versions": versions,
            "systems": systems,
        }
        (self.output / "profile.json").write_text(
            json.dumps(resolved, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        for role in ("A", "B", "C"):
            self.shell(role, f"test ! -e {shlex.quote(self.remote_base)}; mkdir -p {shlex.quote(self.remote_base + '/bin')}")
            self.copy_to(role, WORKLOAD, self.remote_base + "/workload.py")
        for role in ("A", "B"):
            self.copy_to(role, artifacts["dms_node"], self.remote_base + "/bin/dms-node")
        self.copy_to("C", artifacts["dms_meta"], self.remote_base + "/bin/dms-meta")

    def experiment_root(self, lane: str, round_id: int, backend: str, role: str) -> str:
        return f"{self.remote_base}/{lane}-round-{round_id}-{backend}-{role.lower()}"

    def spawn(
        self,
        role: str,
        root: str,
        name: str,
        argv: list[str],
        env: dict[str, str] | None = None,
    ) -> None:
        exports = " ".join(f"{key}={shlex.quote(value)}" for key, value in (env or {}).items())
        script = (
            f"mkdir -p {shlex.quote(root + '/services')}; "
            f"env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy NO_PROXY='*' {exports} "
            f"nohup {shlex.join(argv)} >{shlex.quote(root + '/services/' + name + '.log')} 2>&1 & "
            f"echo $! >{shlex.quote(root + '/' + name + '.pid')}"
        )
        self.shell(role, script)

    def wait_http(self, role: str, url: str) -> None:
        self.shell(
            role,
            "for i in $(seq 1 300); do "
            f"env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy NO_PROXY='*' "
            f"curl -fsS {shlex.quote(url)} >/dev/null && exit 0; sleep 0.1; done; exit 1",
        )

    def start_dms(self, lane: str, round_id: int) -> None:
        backend = "dms"
        for role in ("A", "B", "C"):
            root = self.experiment_root(lane, round_id, backend, role)
            self.shell(role, f"mkdir -p {shlex.quote(root)}")
        root_c = self.experiment_root(lane, round_id, backend, "C")
        self.spawn(
            "C",
            root_c,
            "meta",
            [
                self.remote_base + "/bin/dms-meta",
                "serve",
                "--node-id",
                f"{self.run_id}-{lane}-{round_id}-meta",
                "--grpc-address",
                f"{self.ip['C']}:{self.port['dms_meta']}",
                "--health-address",
                f"{self.ip['C']}:{self.port['dms_meta_health']}",
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait_http("C", f"http://{self.ip['C']}:{self.port['dms_meta_health']}/readyz")
        for role in ("A", "B"):
            root = self.experiment_root(lane, round_id, backend, role)
            mount = root + "/mnt"
            self.shell(role, f"mkdir -p {shlex.quote(mount)}")
            self.spawn(
                role,
                root,
                "node",
                [
                    self.remote_base + "/bin/dms-node",
                    "serve",
                    "--node-id",
                    f"{self.run_id}-{lane}-{round_id}-{role.lower()}",
                    "--meta-endpoint",
                    f"http://{self.ip['C']}:{self.port['dms_meta']}",
                    "--worker-tcp-address",
                    f"{self.ip[role]}:{self.port['dms_worker']}",
                    "--worker-uds-path",
                    root + "/worker.sock",
                    "--health-address",
                    f"{self.ip[role]}:{self.port['dms_node_health']}",
                    "--arena-capacity-bytes",
                    str(self.profile["dms"]["arena_capacity_bytes"]),
                    "--region-size-bytes",
                    str(self.profile["dms"]["region_size_bytes"]),
                    "--node-current-cache-bytes",
                    str(self.profile["dms"]["node_current_cache_bytes"]),
                    "--fuse-mountpoint",
                    mount,
                    "--log-level",
                    "warn",
                    "--tracing-enabled",
                    "false",
                ],
            )
            self.wait_http(role, f"http://{self.ip[role]}:{self.port['dms_node_health']}/readyz")
            self.shell(role, f"for i in $(seq 1 300); do mountpoint -q {shlex.quote(mount)} && exit 0; sleep 0.1; done; exit 1")

    def prepare_moosefs_storage(self, lane: str, round_id: int, role: str) -> str:
        root = self.experiment_root(lane, round_id, "moosefs", role)
        storage = root + "/storage"
        self.shell(role, f"mkdir -p {shlex.quote(root)} {shlex.quote(storage)}")
        if lane == "memory":
            size = "1536m" if role == "A" else "512m"
            self.shell(
                role,
                f"sudo mount -t tmpfs -o size={size} tmpfs {shlex.quote(storage)}; "
                f"sudo chown lzc:lzc {shlex.quote(storage)}",
            )
        return storage

    def start_moosefs(self, lane: str, round_id: int) -> None:
        backend = "moosefs"
        storage_c = self.prepare_moosefs_storage(lane, round_id, "C")
        root_c = self.experiment_root(lane, round_id, backend, "C")
        data_c = storage_c + "/master"
        self.shell(
            "C",
            f"mkdir -p {shlex.quote(data_c)}; cp /var/lib/mfs/metadata.mfs.empty {shlex.quote(data_c + '/metadata.mfs')}; "
            f"printf '%s\\n' '* / rw,alldirs,admin,maproot=0:0' > {shlex.quote(root_c + '/mfsexports.cfg')}; "
            "printf '%s\\n' "
            "'WORKING_USER = lzc' 'WORKING_GROUP = lzc' "
            f"'DATA_PATH = {data_c}' 'EXPORTS_FILENAME = {root_c}/mfsexports.cfg' "
            f"'MATOCS_LISTEN_HOST = {self.ip['C']}' 'MATOCS_LISTEN_PORT = {self.port['mfs_chunk_reg']}' "
            f"'MATOCL_LISTEN_HOST = {self.ip['C']}' 'MATOCL_LISTEN_PORT = {self.port['mfs_client']}' "
            f"> {shlex.quote(root_c + '/mfsmaster.cfg')}",
        )
        self.spawn("C", root_c, "master", ["mfsmaster", "-f", "-c", root_c + "/mfsmaster.cfg"])
        self.shell(
            "C",
            f"for i in $(seq 1 300); do grep -q 'daemon initialized properly' {shlex.quote(root_c + '/services/master.log')} && exit 0; sleep 0.1; done; exit 1",
        )

        storage_a = self.prepare_moosefs_storage(lane, round_id, "A")
        root_a = self.experiment_root(lane, round_id, backend, "A")
        data_a = storage_a + "/chunk-meta"
        chunks_a = storage_a + "/chunks"
        self.shell(
            "A",
            f"mkdir -p {shlex.quote(data_a)} {shlex.quote(chunks_a)}; "
            f"printf '%s\\n' {shlex.quote(chunks_a)} > {shlex.quote(root_a + '/mfshdd.cfg')}; "
            "printf '%s\\n' "
            "'WORKING_USER = lzc' 'WORKING_GROUP = lzc' "
            f"'DATA_PATH = {data_a}' 'HDD_CONF_FILENAME = {root_a}/mfshdd.cfg' "
            "'HDD_LEAVE_SPACE_DEFAULT = 64MiB' "
            f"'MASTER_HOST = {self.ip['C']}' 'MASTER_PORT = {self.port['mfs_chunk_reg']}' "
            f"'CSSERV_LISTEN_HOST = {self.ip['A']}' 'CSSERV_LISTEN_PORT = {self.port['mfs_chunk_data']}' "
            f"> {shlex.quote(root_a + '/mfschunkserver.cfg')}",
        )
        self.spawn("A", root_a, "chunk", ["mfschunkserver", "-f", "-c", root_a + "/mfschunkserver.cfg"])
        self.shell(
            "A",
            f"for i in $(seq 1 300); do grep -q 'connected to Master' {shlex.quote(root_a + '/services/chunk.log')} && exit 0; sleep 0.1; done; exit 1",
        )
        for role in ("A", "B"):
            root = self.experiment_root(lane, round_id, backend, role)
            self.shell(role, f"mkdir -p {shlex.quote(root + '/mnt')}")
            self.spawn(
                role,
                root,
                "mount",
                [
                    "mfsmount",
                    "-f",
                    "-H",
                    self.ip["C"],
                    "-P",
                    str(self.port["mfs_client"]),
                    "-o",
                    "allow_other",
                    "-o",
                    "mfslogminlevel=WARNING",
                    root + "/mnt",
                ],
            )
            self.shell(role, f"for i in $(seq 1 300); do mountpoint -q {shlex.quote(root + '/mnt')} && exit 0; sleep 0.1; done; exit 1")
        self.shell("A", f"mfssetgoal 1 {shlex.quote(root_a + '/mnt')} >/dev/null")
        status = self.shell(
            "C",
            f"mfscli -H {self.ip['C']} -P {self.port['mfs_client']} -SCS -j",
            capture=True,
        )
        connected = json.loads(status.stdout)["dataset"]["chunkservers"]
        if len(connected) != 1 or connected[0].get("ip") != self.ip["A"]:
            raise RuntimeError(f"MooseFS owner topology is not deterministic: {connected}")

    def dms_prometheus(self, role: str) -> str:
        url = (
            f"http://{self.ip['C']}:{self.port['dms_meta_health']}/metrics"
            if role == "C"
            else f"http://{self.ip[role]}:{self.port['dms_node_health']}/metrics"
        )
        return self.shell(
            role,
            f"env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy NO_PROXY='*' curl -fsS {shlex.quote(url)}",
            capture=True,
        ).stdout

    def pid_names(self, backend: str, role: str) -> tuple[str, ...]:
        if backend == "dms":
            return ("meta",) if role == "C" else ("node",)
        if role == "C":
            return ("master",)
        if role == "A":
            return ("chunk", "mount")
        return ("mount",)

    def system_snapshot(self, lane: str, round_id: int, backend: str, role: str) -> dict[str, Any]:
        root = self.experiment_root(lane, round_id, backend, role)
        names = self.pid_names(backend, role)
        script = """
import json, os, pathlib, sys
root = pathlib.Path(sys.argv[1])
names = sys.argv[2:]
processes = {}
for name in names:
    pid = int((root / f"{name}.pid").read_text().strip())
    raw = pathlib.Path(f"/proc/{pid}/stat").read_text()
    fields = raw[raw.rfind(")") + 2:].split()
    status = {}
    for line in pathlib.Path(f"/proc/{pid}/status").read_text().splitlines():
        if ":" in line:
            key, value = line.split(":", 1)
            status[key] = value.strip()
    processes[name] = {
        "pid": pid,
        "cpu_ticks": int(fields[11]) + int(fields[12]),
        "rss_bytes": int(fields[21]) * os.sysconf("SC_PAGE_SIZE"),
        "voluntary_context_switches": int(status.get("voluntary_ctxt_switches", "0")),
        "nonvoluntary_context_switches": int(status.get("nonvoluntary_ctxt_switches", "0")),
    }
net = {}
base = pathlib.Path("/sys/class/net/eth0/statistics")
for name in ("rx_bytes", "tx_bytes", "rx_packets", "tx_packets"):
    net[name] = int((base / name).read_text())
print(json.dumps({"processes": processes, "network": net}))
"""
        quoted = " ".join(shlex.quote(name) for name in names)
        result = self.shell(
            role,
            f"python3 -c {shlex.quote(script)} {shlex.quote(root)} {quoted}",
            capture=True,
        )
        return json.loads(result.stdout)

    def snapshot(
        self,
        lane: str,
        round_id: int,
        backend: str,
        case_dir: Path,
        suffix: str,
    ) -> None:
        for role in ("A", "B", "C"):
            (case_dir / f"{suffix}-{role}.system.json").write_text(
                json.dumps(self.system_snapshot(lane, round_id, backend, role), indent=2) + "\n",
                encoding="utf-8",
            )
            if backend == "dms":
                (case_dir / f"{suffix}-{role}.prom").write_text(
                    self.dms_prometheus(role), encoding="utf-8"
                )
        if backend == "moosefs":
            for flag, name in (("-SMO", "operations"), ("-SMC", "master"), ("-SCC", "chunk")):
                result = self.shell(
                    "C",
                    f"mfscli -H {self.ip['C']} -P {self.port['mfs_client']} {flag} -j",
                    capture=True,
                )
                (case_dir / f"{suffix}-mfs-{name}.json").write_text(result.stdout, encoding="utf-8")

    def run_cases(self, lane: str, round_id: int, backend: str) -> None:
        for phase, role in PHASES:
            case_dir = self.output / lane / f"round-{round_id}" / backend / phase
            case_dir.mkdir(parents=True)
            root = self.experiment_root(lane, round_id, backend, role)
            if phase in WARMUP_PHASES:
                remote_warmup = root + f"/{phase}.warmup.json"
                self.shell(
                    role,
                    shlex.join(
                        [
                            "python3",
                            self.remote_base + "/workload.py",
                            "--root",
                            root + "/mnt",
                            "--phase",
                            phase,
                            "--seed",
                            "6701",
                            "--output",
                            remote_warmup,
                        ]
                    ),
                )
                self.copy_from(role, remote_warmup, case_dir / "warmup.json")
            self.snapshot(lane, round_id, backend, case_dir, "before")
            remote_result = root + f"/{phase}.json"
            self.shell(
                role,
                shlex.join(
                    [
                        "python3",
                        self.remote_base + "/workload.py",
                        "--root",
                        root + "/mnt",
                        "--phase",
                        phase,
                        "--seed",
                        "6701",
                        "--output",
                        remote_result,
                    ]
                ),
            )
            self.snapshot(lane, round_id, backend, case_dir, "after")
            self.copy_from(role, remote_result, case_dir / "workload.json")

    def stop_pid(self, role: str, root: str, name: str) -> None:
        self.shell(
            role,
            f"file={shlex.quote(root + '/' + name + '.pid')}; "
            "if test -f \"$file\"; then pid=$(sed -n '1p' \"$file\"); kill \"$pid\" 2>/dev/null || true; fi",
        )

    def cleanup(self, lane: str, round_id: int, backend: str) -> None:
        for role in ("A", "B"):
            root = self.experiment_root(lane, round_id, backend, role)
            self.shell(role, f"fusermount3 -uz {shlex.quote(root + '/mnt')} 2>/dev/null || true")
            if backend == "moosefs" and role == "A":
                self.shell(
                    role,
                    f"mfschunkserver -c {shlex.quote(root + '/mfschunkserver.cfg')} stop >/dev/null 2>&1 || true",
                )
            for name in self.pid_names(backend, role):
                self.stop_pid(role, root, name)
        root_c = self.experiment_root(lane, round_id, backend, "C")
        if backend == "moosefs":
            self.shell(
                "C",
                f"mfsmaster -c {shlex.quote(root_c + '/mfsmaster.cfg')} stop >/dev/null 2>&1 || true",
            )
        for name in self.pid_names(backend, "C"):
            self.stop_pid("C", root_c, name)
        time.sleep(2)
        if backend == "moosefs" and lane == "memory":
            for role in ("A", "C"):
                root = self.experiment_root(lane, round_id, backend, role)
                self.shell(
                    role,
                    f"sudo umount {shlex.quote(root + '/storage')} 2>/dev/null || "
                    f"sudo umount -l {shlex.quote(root + '/storage')} 2>/dev/null || true",
                )

    def collect_logs(self) -> None:
        destination = self.output / "remote-logs"
        for role in ("A", "B", "C"):
            listing = self.shell(
                role,
                f"find {shlex.quote(self.remote_base)} -type f -path '*/services/*.log' -print 2>/dev/null || true",
                capture=True,
            )
            for remote in filter(None, listing.stdout.splitlines()):
                relative = Path(remote).relative_to(self.remote_base)
                self.copy_from(role, remote, destination / role.lower() / relative)

    def execute(self) -> None:
        lane_rounds = self.profile.get("lane_rounds", {"memory": 5, "disk": 1})
        orders = (("dms", "moosefs"), ("moosefs", "dms"))
        try:
            self.prepare()
            for lane in ("memory", "disk"):
                rounds = int(lane_rounds.get(lane, 0))
                for round_id in range(rounds):
                    for backend in orders[round_id % 2]:
                        try:
                            if backend == "dms":
                                self.start_dms(lane, round_id)
                            else:
                                self.start_moosefs(lane, round_id)
                            self.run_cases(lane, round_id, backend)
                        finally:
                            self.cleanup(lane, round_id, backend)
        finally:
            self.collect_logs()
            for role in ("A", "B", "C"):
                self.shell(
                    role,
                    f"findmnt -rn -o TARGET | grep '^{shlex.quote(self.remote_base)}' | "
                    "sort -r | while read -r target; do sudo umount -l \"$target\" 2>/dev/null || true; done; "
                    f"rm -rf -- {shlex.quote(self.remote_base)}",
                )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    harness = Harness(profile, args.output.resolve())
    harness.execute()
    print(json.dumps({"ok": True, "output": str(harness.output)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

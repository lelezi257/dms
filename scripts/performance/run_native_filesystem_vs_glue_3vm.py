#!/usr/bin/env python3
"""在同一组三台 Lima VM 上交替运行 Native Filesystem 与外置 Glue。

脚本只负责编排：每个 round/arm 使用独立 Meta WAL、Redis 和挂载目录；进程 PID、
启动命令、二进制 SHA256、每个 case 前后的 Prometheus 快照和 workload 原始样本都会
保留在输出目录。它不会按进程名清理，也不会修改 VM 的全局缓存或系统参数。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import time
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
WORKLOAD = ROOT / "scripts/performance/native_filesystem_workload.py"
SIZES = (4096, 65536, 1048576)
PHASES = (
    ("create", "A", SIZES),
    ("local-hot", "A", SIZES),
    ("metadata-hot", "A", (4096,)),
    ("open-close", "A", (4096,)),
    ("readdir", "A", (None,)),
    ("peer-first", "B", SIZES),
    ("peer-hot", "B", SIZES),
    ("overwrite", "A", (65536,)),
    ("remote-after-overwrite", "B", SIZES),
)

CASE_PREFIX = {
    "create": "create_write",
    "local-hot": "local_hot_read",
    "metadata-hot": "metadata_hot",
    "open-close": "open_close",
    "readdir": "readdir",
    "peer-first": "peer_first_read",
    "peer-hot": "peer_hot_read",
    "overwrite": "middle_overwrite",
    "remote-after-overwrite": "remote_after_overwrite",
}


def workload_case_id(phase: str, size: int | None) -> str:
    return CASE_PREFIX[phase] + (".root" if size is None else f".{size}")


def command(argv: list[str], *, capture: bool = False) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, text=True, capture_output=True, check=False)
    if completed.returncode:
        detail = (completed.stdout + completed.stderr)[-4000:]
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
        self.run_id = profile["run_id"]
        self.vm = profile["vms"]
        self.ip = profile["ips"]
        self.port = profile["ports"]
        self.remote_base = f"/tmp/dms-{self.run_id}"

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
        artifacts = {name: Path(path).resolve() for name, path in self.profile["artifacts"].items()}
        for name, path in artifacts.items():
            if not path.is_file():
                raise RuntimeError(f"artifact is missing: {name}: {path}")
        hashes = {name: sha256(path) for name, path in artifacts.items()}
        expected = self.profile.get("hashes", {})
        for name, digest in expected.items():
            if hashes.get(name) != digest:
                raise RuntimeError(f"artifact hash changed: {name}")

        self.output.mkdir(parents=True, exist_ok=False)
        (self.output / "profile.json").write_text(
            json.dumps({**self.profile, "resolved_hashes": hashes}, ensure_ascii=False, indent=2)
            + "\n",
            encoding="utf-8",
        )
        for role in ("A", "B", "C"):
            self.shell(role, f"test ! -e {shlex.quote(self.remote_base)}; mkdir -p {shlex.quote(self.remote_base + '/bin')}")
            self.copy_to(role, WORKLOAD, self.remote_base + "/workload.py")
        for role in ("A", "B"):
            self.copy_to(role, artifacts["dms_node"], self.remote_base + "/bin/dms-node")
            self.copy_to(role, artifacts["juicefs"], self.remote_base + "/bin/juicefs")
        self.copy_to("C", artifacts["dms_meta"], self.remote_base + "/bin/dms-meta")

    def wait_http(self, role: str, url: str) -> None:
        script = (
            "for i in $(seq 1 200); do "
            f"env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy NO_PROXY='*' curl -fsS {shlex.quote(url)} >/dev/null && exit 0; "
            "sleep 0.1; done; exit 1"
        )
        self.shell(role, script)

    def experiment_root(self, round_id: int, arm: str, role: str) -> str:
        return f"{self.remote_base}/round-{round_id}-{arm}-{role.lower()}"

    def spawn(self, role: str, root: str, name: str, argv: list[str], env: dict[str, str] | None = None) -> None:
        exports = " ".join(f"{key}={shlex.quote(value)}" for key, value in (env or {}).items())
        command_line = shlex.join(argv)
        script = (
            f"mkdir -p {shlex.quote(root + '/services')}; "
            f"env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy NO_PROXY='*' {exports} "
            f"nohup {command_line} >{shlex.quote(root + '/services/' + name + '.log')} 2>&1 & "
            f"echo $! >{shlex.quote(root + '/' + name + '.pid')}"
        )
        self.shell(role, script)

    def start(self, round_id: int, arm: str) -> None:
        for role in ("A", "B", "C"):
            root = self.experiment_root(round_id, arm, role)
            self.shell(role, f"mkdir -p {shlex.quote(root)}")

        root_c = self.experiment_root(round_id, arm, "C")
        meta_argv = [
            self.remote_base + "/bin/dms-meta",
            "serve",
            "--node-id",
            f"{self.run_id}-{round_id}-{arm}-meta",
            "--grpc-address",
            f"{self.ip['C']}:{self.port['meta']}",
            "--health-address",
            f"{self.ip['C']}:{self.port['meta_health']}",
            "--log-level",
            "warn",
            "--tracing-enabled",
            "false",
        ]
        if self.profile.get("metadata_mode", "memory") == "local_wal":
            meta_argv.extend(["--journal-dir", root_c + "/meta"])
        self.spawn(
            "C",
            root_c,
            "meta",
            meta_argv,
        )
        self.wait_http("C", f"http://{self.ip['C']}:{self.port['meta_health']}/readyz")

        if arm == "glue":
            redis_argv = [
                "redis-server",
                "--bind",
                self.ip["C"],
                "--port",
                str(self.port["redis"]),
                "--protected-mode",
                # 该 Redis 只监听隔离实验 VM 的私网地址，且生命周期受本 harness 管理。
                # protected-mode 会拒绝 A/B 两台 VM 的无密码连接，不能用于三机基线。
                "no",
                "--dir",
                root_c,
            ]
            if self.profile.get("redis_mode", "memory") == "aof_always":
                redis_argv.extend(["--save", "", "--appendonly", "yes", "--appendfsync", "always"])
            else:
                redis_argv.extend(["--save", "", "--appendonly", "no"])
            self.spawn(
                "C",
                root_c,
                "redis",
                redis_argv,
            )
            self.shell("C", f"for i in $(seq 1 100); do redis-cli -h {self.ip['C']} -p {self.port['redis']} ping | grep -q PONG && exit 0; sleep 0.1; done; exit 1")

        for role in ("A", "B"):
            root = self.experiment_root(round_id, arm, role)
            mount = root + "/mnt"
            self.shell(role, f"mkdir -p {shlex.quote(mount)}")
            argv = [
                self.remote_base + "/bin/dms-node",
                "serve",
                "--node-id",
                f"{self.run_id}-{round_id}-{arm}-{role.lower()}",
                "--meta-endpoint",
                f"http://{self.ip['C']}:{self.port['meta']}",
                "--worker-tcp-address",
                f"{self.ip[role]}:{self.port['worker']}",
                "--worker-uds-path",
                root + "/worker.sock",
                "--health-address",
                f"{self.ip[role]}:{self.port['node_health']}",
                "--arena-capacity-bytes",
                str(self.profile["arena_capacity_bytes"]),
                "--region-size-bytes",
                str(self.profile["region_size_bytes"]),
                "--node-current-cache-bytes",
                str(self.profile["node_current_cache_bytes"]),
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ]
            if arm == "native":
                argv.extend(["--fuse-mountpoint", mount])
            self.spawn(role, root, "node", argv)
            self.wait_http(role, f"http://{self.ip[role]}:{self.port['node_health']}/readyz")

        if arm == "glue":
            self.format_glue(round_id)
            for role in ("A", "B"):
                self.mount_glue(round_id, role)
        else:
            for role in ("A", "B"):
                root = self.experiment_root(round_id, arm, role)
                self.shell(role, f"mountpoint -q {shlex.quote(root + '/mnt')}")

    def glue_env(self, root: str) -> dict[str, str]:
        alias = f"{self.run_id}-store"
        return {
            "DMS_PROVIDER_ALIAS": alias,
            "DMS_SHARED_MEMORY": "true",
            "DMS_JUICEFS_ENDPOINTS": json.dumps({alias: f"unix://{root}/worker.sock"}),
        }

    def format_glue(self, round_id: int) -> None:
        root = self.experiment_root(round_id, "glue", "A")
        alias = f"{self.run_id}-store"
        env = " ".join(f"{key}={shlex.quote(value)}" for key, value in self.glue_env(root).items())
        argv = [
            self.remote_base + "/bin/juicefs",
            "--no-agent",
            "format",
            "--storage",
            "dms",
            "--bucket",
            f"dms://{alias}",
            "--block-size",
            "4M",
            "--compress",
            "none",
            "--trash-days",
            "0",
            f"redis://{self.ip['C']}:{self.port['redis']}/0",
            f"{self.run_id}-{round_id}",
        ]
        self.shell("A", f"env {env} {shlex.join(argv)} >{shlex.quote(root + '/format.log')} 2>&1")

    def mount_glue(self, round_id: int, role: str) -> None:
        root = self.experiment_root(round_id, "glue", role)
        env = self.glue_env(root)
        argv = [
            self.remote_base + "/bin/juicefs",
            "--no-agent",
            "mount",
            f"redis://{self.ip['C']}:{self.port['redis']}/0",
            root + "/mnt",
            "--backup-meta",
            "0",
            "--cache-size",
            "0",
            "--buffer-size",
            "32",
            "--max-readahead",
            "0",
            "--prefetch",
            "0",
            "--attr-cache",
            "0",
            "--entry-cache",
            "0",
            "--dir-entry-cache",
            "0",
            "--open-cache",
            "0",
            "--no-usage-report",
        ]
        self.spawn(role, root, "juicefs", argv, env)
        self.shell(role, f"for i in $(seq 1 200); do mountpoint -q {shlex.quote(root + '/mnt')} && exit 0; sleep 0.1; done; exit 1")

    def metrics(self, role: str) -> str:
        url = (
            f"http://{self.ip['C']}:{self.port['meta_health']}/metrics"
            if role == "C"
            else f"http://{self.ip[role]}:{self.port['node_health']}/metrics"
        )
        return self.shell(
            role,
            f"env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy NO_PROXY='*' curl -fsS {shlex.quote(url)}",
            capture=True,
        ).stdout

    def process_snapshot(self, round_id: int, arm: str, role: str) -> dict[str, Any]:
        root = self.experiment_root(round_id, arm, role)
        result = self.shell(
            role,
            (
                f"pid=$(sed -n '1p' {shlex.quote(root + '/' + ('meta' if role == 'C' else 'node') + '.pid')}); "
                "python3 -c 'import json,os,sys; p=sys.argv[1]; raw=open(f\"/proc/{p}/stat\").read(); f=raw[raw.rfind(\")\")+2:].split(); "
                "print(json.dumps({\"pid\":int(p),\"cpu_ticks\":int(f[11])+int(f[12]),\"rss_bytes\":int(f[21])*os.sysconf(\"SC_PAGE_SIZE\")}))' \"$pid\""
            ),
            capture=True,
        )
        return json.loads(result.stdout)

    def snapshot(self, round_id: int, arm: str, case_dir: Path, suffix: str) -> None:
        for role in ("A", "B", "C"):
            (case_dir / f"{suffix}-{role}.prom").write_text(self.metrics(role), encoding="utf-8")
            (case_dir / f"{suffix}-{role}.process.json").write_text(
                json.dumps(self.process_snapshot(round_id, arm, role), indent=2) + "\n",
                encoding="utf-8",
            )

    def run_cases(self, round_id: int, arm: str) -> None:
        selected_cases = set(self.profile.get("cases", ()))
        for phase, role, sizes in PHASES:
            for size in sizes:
                case_id = workload_case_id(phase, size)
                if selected_cases and case_id not in selected_cases:
                    continue
                case_dir = self.output / f"round-{round_id}" / arm / case_id
                case_dir.mkdir(parents=True)
                self.snapshot(round_id, arm, case_dir, "before")
                root = self.experiment_root(round_id, arm, role)
                remote_result = root + f"/{phase}-{size or 'root'}.json"
                argv = [
                    "python3",
                    self.remote_base + "/workload.py",
                    "--root",
                    root + "/mnt",
                    "--phase",
                    phase,
                    "--output",
                    remote_result,
                    "--seed",
                    "6701",
                ]
                if size is not None:
                    argv.extend(["--size", str(size)])
                self.shell(role, shlex.join(argv))
                self.snapshot(round_id, arm, case_dir, "after")
                self.copy_from(role, remote_result, case_dir / "workload.json")

    def cleanup(self, round_id: int, arm: str) -> None:
        for role in ("A", "B"):
            root = self.experiment_root(round_id, arm, role)
            script = (
                f"fusermount3 -uz {shlex.quote(root + '/mnt')} 2>/dev/null || true; "
                f"for name in juicefs node; do file={shlex.quote(root)}/$name.pid; "
                "if test -f \"$file\"; then pid=$(sed -n '1p' \"$file\"); kill \"$pid\" 2>/dev/null || true; fi; done"
            )
            self.shell(role, script)
        root_c = self.experiment_root(round_id, arm, "C")
        self.shell(
            "C",
            f"for name in redis meta; do file={shlex.quote(root_c)}/$name.pid; if test -f \"$file\"; then pid=$(sed -n '1p' \"$file\"); kill \"$pid\" 2>/dev/null || true; fi; done",
        )
        time.sleep(1)

    def execute(self) -> None:
        alternating_orders = (("native", "glue"), ("glue", "native"))
        rounds = int(self.profile.get("rounds", 3))
        if rounds < 1:
            raise RuntimeError("rounds must be positive")
        try:
            for round_id in range(rounds):
                order = alternating_orders[round_id % len(alternating_orders)]
                for arm in order:
                    try:
                        self.start(round_id, arm)
                        self.run_cases(round_id, arm)
                    finally:
                        # start() 中途失败时也必须按精确 PID 和挂载点收口。
                        self.cleanup(round_id, arm)
        finally:
            for role in ("A", "B", "C"):
                # 这里只删除本 harness 自己创建、且所有受管进程已按 PID 停止的目录。
                self.shell(role, f"test -d {shlex.quote(self.remote_base)} && true")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    harness = Harness(profile, args.output.resolve())
    harness.prepare()
    harness.execute()
    print(json.dumps({"ok": True, "output": str(harness.output)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

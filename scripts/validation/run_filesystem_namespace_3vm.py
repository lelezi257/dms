#!/usr/bin/env python3
"""在三台 Lima VM 上验证 M1 共享 namespace。

部署固定为：A=写入/挂载 Node，B=观察/挂载 Node，C=Meta。控制器只通过普通
POSIX 文件操作驱动两个挂载点，不调用 DMS 内部接口；服务均按精确 PID 和本次
run directory 管理，避免误杀其它实验。
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
EVALUATOR = ROOT / "scripts/validation/evaluate_filesystem_namespace.py"
REQUIRED_OPERATIONS = {
    "mkdir_create_read",
    "rename",
    "rename_type_and_cycle_guards",
    "unlink_rmdir",
    "recovery_anchor",
    "unlink_regular_file",
}


def command(argv: list[str], *, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, text=True, capture_output=capture, check=False)
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


class Harness:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.limactl = shutil.which("limactl")
        if self.limactl is None:
            raise RuntimeError("limactl is required")
        self.vms = {"A": args.vm_a, "B": args.vm_b, "C": args.vm_c}
        self.ips = {"A": args.ip_a, "B": args.ip_b, "C": args.ip_c}
        self.remote = f"/tmp/dms-filesystem-namespace-{args.run_id}"
        self.mount = {role: f"{self.remote}/mnt" for role in ("A", "B")}
        self.output = args.output.resolve()
        self.round_results: list[dict[str, Any]] = []

    def shell(self, role: str, script: str, *, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
        return command(
            [
                self.limactl,
                "shell",
                "--workdir",
                "/tmp",
                self.vms[role],
                "--",
                "bash",
                "-lc",
                script,
            ],
            capture=capture,
            check=check,
        )

    def copy_to(self, role: str, source: Path, destination: str) -> None:
        command([self.limactl, "copy", str(source), f"{self.vms[role]}:{destination}"])

    def copy_from(self, role: str, source: str, destination: Path) -> None:
        destination.parent.mkdir(parents=True, exist_ok=True)
        command([self.limactl, "copy", f"{self.vms[role]}:{source}", str(destination)])

    def wait(self, role: str, description: str, script: str, timeout: float = 15.0) -> int:
        deadline = time.monotonic() + timeout
        attempts = 0
        last_detail = ""
        while True:
            attempts += 1
            result = self.shell(role, script, capture=True, check=False)
            if result.returncode == 0:
                return attempts
            last_detail = ((result.stdout or "") + (result.stderr or ""))[-1000:]
            if time.monotonic() >= deadline:
                raise TimeoutError(
                    f"timed out waiting for {description} on VM {role}; last output: {last_detail!r}"
                )
            time.sleep(0.05)

    def spawn(self, role: str, name: str, argv: list[str]) -> None:
        root = self.remote
        script = (
            f"env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy NO_PROXY='*' "
            f"nohup {shlex.join(argv)} >{shlex.quote(root + '/' + name + '.log')} 2>&1 </dev/null & "
            f"echo $! >{shlex.quote(root + '/' + name + '.pid')}"
        )
        self.shell(role, script)

    def stop(self, role: str, name: str) -> None:
        pid_file = f"{self.remote}/{name}.pid"
        self.shell(
            role,
            f"if test -f {shlex.quote(pid_file)}; then pid=$(sed -n '1p' {shlex.quote(pid_file)}); "
            "kill \"$pid\" 2>/dev/null || true; wait \"$pid\" 2>/dev/null || true; fi",
            check=False,
        )

    def prepare(self) -> None:
        if self.output.exists():
            raise RuntimeError(f"output already exists: {self.output}")
        self.output.mkdir(parents=True)
        for binary in (self.args.dms_node, self.args.dms_meta):
            if not binary.is_file():
                raise RuntimeError(f"binary is missing: {binary}")
        manifest = {
            "schema": "dms.filesystem.namespace-3vm-profile.v1",
            "run_id": self.args.run_id,
            "vms": self.vms,
            "ips": self.ips,
            "rounds": self.args.rounds,
            "artifacts": {
                "dms_node": {"path": str(self.args.dms_node), "sha256": digest(self.args.dms_node)},
                "dms_meta": {"path": str(self.args.dms_meta), "sha256": digest(self.args.dms_meta)},
            },
        }
        (self.output / "profile.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
        for role in ("A", "B", "C"):
            self.shell(role, f"test ! -e {shlex.quote(self.remote)} && mkdir -p {shlex.quote(self.remote + '/bin')}")
        for role in ("A", "B"):
            self.shell(role, f"mkdir -p {shlex.quote(self.mount[role])}")
            self.copy_to(role, self.args.dms_node, self.remote + "/bin/dms-node")
            self.shell(role, f"chmod 755 {shlex.quote(self.remote + '/bin/dms-node')}")
        self.copy_to("C", self.args.dms_meta, self.remote + "/bin/dms-meta")
        self.shell("C", f"chmod 755 {shlex.quote(self.remote + '/bin/dms-meta')}; mkdir -p {shlex.quote(self.remote + '/journal')}")

    def start_meta(self, log_name: str) -> None:
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
        if log_name != "meta.log":
            self.shell("C", f"mv {shlex.quote(self.remote + '/meta.log')} {shlex.quote(self.remote + '/' + log_name)}")
            # spawn 固定写 meta.log；重启阶段重新指向独立文件。
            self.shell("C", f"ln -s {shlex.quote(log_name)} {shlex.quote(self.remote + '/meta.log')}")
        self.wait(
            "C",
            "Meta ready",
            "env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy NO_PROXY='*' "
            f"curl -fsS http://{self.ips['C']}:{self.args.meta_health_port}/readyz >/dev/null",
        )

    def start_node(self, role: str, log_name: str) -> None:
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
                str(512 * 1024 * 1024),
                "--region-size-bytes",
                str(64 * 1024 * 1024),
                "--node-current-cache-bytes",
                str(64 * 1024 * 1024),
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ],
        )
        if log_name != "node.log":
            self.shell(role, f"mv {shlex.quote(self.remote + '/node.log')} {shlex.quote(self.remote + '/' + log_name)}")
            self.shell(role, f"ln -s {shlex.quote(log_name)} {shlex.quote(self.remote + '/node.log')}")
        self.wait(
            role,
            "Node ready",
            "env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy NO_PROXY='*' "
            f"curl -fsS http://{self.ips[role]}:{self.args.node_health_port}/readyz >/dev/null",
        )
        self.wait(role, "FUSE mount", f"mountpoint -q {shlex.quote(self.mount[role])}")

    @staticmethod
    def payload(round_id: int, suffix: str) -> bytes:
        return f"dms-m1-namespace:{round_id}:{suffix}".encode()

    def python(self, role: str, source: str, *, check: bool = True) -> subprocess.CompletedProcess[str]:
        return self.shell(role, "python3 -c " + shlex.quote(source), capture=True, check=check)

    def read_check(self, path: str, expected: bytes) -> str:
        source = "from pathlib import Path; import sys; " + (
            f"sys.exit(0 if Path({path!r}).read_bytes() == {expected!r} else 1)"
        )
        return "python3 -c " + shlex.quote(source)

    def run_round(self, round_id: int) -> None:
        started = time.perf_counter_ns()
        project = f"project-{round_id:03d}"
        base_a = f"{self.mount['A']}/{project}"
        base_b = f"{self.mount['B']}/{project}"
        payload = self.payload(round_id, "created")
        self.python(
            "A",
            "from pathlib import Path; "
            f"p=Path({(base_a + '/src/pkg')!r}); p.mkdir(parents=True); "
            f"(p/'module.txt').write_bytes({payload!r})",
        )
        directory_attempts = self.wait("B", "nested directory", f"test -d {shlex.quote(base_b + '/src/pkg')}")
        read_attempts = self.wait("B", "created file", self.read_check(base_b + "/src/pkg/module.txt", payload))

        self.shell("A", f"mv {shlex.quote(base_a + '/src/pkg/module.txt')} {shlex.quote(base_a + '/src/pkg/renamed.txt')}")
        rename_attempts = self.wait(
            "B",
            "rename",
            f"test ! -e {shlex.quote(base_b + '/src/pkg/module.txt')} && test -f {shlex.quote(base_b + '/src/pkg/renamed.txt')}",
        )

        boundary_base = base_a + "/rename-boundaries"
        self.python(
            "A",
            "import errno, os\n"
            "from pathlib import Path\n"
            f"base=Path({boundary_base!r}); base.mkdir()\n"
            "cycle_parent=base/'cycle-parent'; cycle_child=cycle_parent/'child'; cycle_child.mkdir(parents=True)\n"
            "def expect(call, expected):\n"
            " try:\n  call()\n"
            " except OSError as error:\n"
            "  if error.errno == expected: return\n"
            "  raise\n"
            " raise AssertionError(f'expected errno {expected}')\n"
            "expect(lambda: os.rename(cycle_parent, cycle_child/'cycle-parent'), errno.EINVAL)\n"
            "source_file=base/'source-file'; target_dir=base/'target-dir'\n"
            "source_file.write_bytes(b'file'); target_dir.mkdir()\n"
            "expect(lambda: os.replace(source_file, target_dir), errno.EISDIR)\n"
            "source_dir=base/'source-dir'; target_file=base/'target-file'\n"
            "source_dir.mkdir(); target_file.write_bytes(b'file')\n"
            "expect(lambda: os.replace(source_dir, target_file), errno.ENOTDIR)\n"
            "source_file.unlink(); target_dir.rmdir(); source_dir.rmdir(); target_file.unlink()\n"
            "cycle_child.rmdir(); cycle_parent.rmdir(); base.rmdir()",
        )

        tmp_payload = self.payload(round_id, "tmp-child")
        self.python(
            "A",
            "from pathlib import Path; "
            f"p=Path({(base_a + '/tmp')!r}); p.mkdir(); (p/'child.txt').write_bytes({tmp_payload!r})",
        )
        self.wait("B", "temporary child", self.read_check(base_b + "/tmp/child.txt", tmp_payload))
        self.python(
            "A",
            "import errno; from pathlib import Path; "
            f"p=Path({(base_a + '/tmp')!r}); "
            "caught=False\ntry:\n p.rmdir()\nexcept OSError as e:\n caught=e.errno==errno.ENOTEMPTY\n"
            "raise SystemExit(0 if caught else 1)",
        )
        self.python("A", f"from pathlib import Path; p=Path({(base_a + '/tmp')!r}); (p/'child.txt').unlink(); p.rmdir()")
        remove_attempts = self.wait("B", "removed empty directory", f"test ! -e {shlex.quote(base_b + '/tmp')}")

        anchor_payload = self.payload(round_id, "anchor")
        anchor_a = f"{self.mount['A']}/stable/round-{round_id:03d}/anchor.txt"
        anchor_b = f"{self.mount['B']}/stable/round-{round_id:03d}/anchor.txt"
        self.python(
            "A",
            "from pathlib import Path; "
            f"p=Path({anchor_a!r}); p.parent.mkdir(parents=True); p.write_bytes({anchor_payload!r})",
        )
        anchor_attempts = self.wait("B", "recovery anchor", self.read_check(anchor_b, anchor_payload))

        self.shell("A", f"rm {shlex.quote(base_a + '/src/pkg/renamed.txt')}")
        unlink_attempts = self.wait("B", "unlinked regular file", f"test ! -e {shlex.quote(base_b + '/src/pkg/renamed.txt')}")
        self.round_results.append(
            {
                "round": round_id,
                "checks": [
                    {"operation": "mkdir_create_read", "directory_wait_attempts": directory_attempts, "read_wait_attempts": read_attempts},
                    {"operation": "rename", "wait_attempts": rename_attempts},
                    {"operation": "rename_type_and_cycle_guards"},
                    {"operation": "unlink_rmdir", "wait_attempts": remove_attempts},
                    {"operation": "recovery_anchor", "wait_attempts": anchor_attempts, "path": anchor_b},
                    {"operation": "unlink_regular_file", "wait_attempts": unlink_attempts},
                ],
                "elapsed_ms": (time.perf_counter_ns() - started) / 1_000_000,
            }
        )

    def snapshot_metrics(self) -> None:
        for role, url, filename in (
            ("A", f"http://{self.ips['A']}:{self.args.node_health_port}/metrics", "node-a.prom"),
            ("B", f"http://{self.ips['B']}:{self.args.node_health_port}/metrics", "node-b.prom"),
            ("C", f"http://{self.ips['C']}:{self.args.meta_health_port}/metrics", "meta.prom"),
        ):
            result = self.shell(
                role,
                "env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy NO_PROXY='*' "
                f"curl -fsS {shlex.quote(url)}",
                capture=True,
            )
            (self.output / filename).write_text(result.stdout)

    def restart_and_verify(self) -> None:
        self.stop("C", "meta")
        self.shell(
            "C",
            f"mv {shlex.quote(self.remote + '/meta.log')} {shlex.quote(self.remote + '/meta-initial.log')}",
        )
        self.start_meta("meta-restarted.log")
        self.stop("B", "node")
        self.shell("B", f"fusermount3 -uz {shlex.quote(self.mount['B'])} 2>/dev/null || true; rm -f {shlex.quote(self.remote + '/node.log')}")
        self.start_node("B", "node-b-restarted.log")
        round_id = self.args.rounds - 1
        expected = self.payload(round_id, "anchor")
        path = f"{self.mount['B']}/stable/round-{round_id:03d}/anchor.txt"
        attempts = self.wait("B", "recovered anchor", self.read_check(path, expected), timeout=20)
        recovery = {
            "schema": "dms.filesystem.namespace-recovery.v1",
            "round": round_id,
            "path": path,
            "wait_attempts": attempts,
            "bytes": len(expected),
        }
        (self.output / "namespace-recovery.json").write_text(json.dumps(recovery, ensure_ascii=False, indent=2) + "\n")

    def collect_logs(self) -> None:
        for role, remote_name, local_name in (
            ("A", "node-a.log", "node-a.log"),
            ("B", "node-b.log", "node-b.log"),
            ("B", "node-b-restarted.log", "node-b-restarted.log"),
            ("C", "meta-initial.log", "meta.log"),
            ("C", "meta-restarted.log", "meta-restarted.log"),
        ):
            self.copy_from(role, self.remote + "/" + remote_name, self.output / local_name)

    def collect_failure_logs(self) -> None:
        """失败收口前尽量带回服务日志，不能让 cleanup 抹掉根因证据。"""

        for role, remote_name, local_name in (
            ("A", "node-a.log", "failure-node-a.log"),
            ("B", "node-b.log", "failure-node-b.log"),
            ("C", "meta.log", "failure-meta.log"),
        ):
            if self.shell(role, f"test -f {shlex.quote(self.remote + '/' + remote_name)}", check=False).returncode == 0:
                try:
                    self.copy_from(role, self.remote + "/" + remote_name, self.output / local_name)
                except RuntimeError:
                    pass

    def cleanup(self) -> None:
        for role in ("A", "B"):
            self.shell(role, f"fusermount3 -uz {shlex.quote(self.mount[role])} 2>/dev/null || true", check=False)
            self.stop(role, "node")
        self.stop("C", "meta")
        for role in ("A", "B", "C"):
            self.shell(role, f"rm -rf {shlex.quote(self.remote)}", check=False)

    def execute(self) -> None:
        self.prepare()
        completed = False
        try:
            self.start_meta("meta.log")
            self.start_node("A", "node-a.log")
            self.start_node("B", "node-b.log")
            for round_id in range(self.args.rounds):
                self.run_round(round_id)
            workload = {
                "schema": "dms.filesystem.namespace-workload.v1",
                "deployment": "three-vm",
                "requested_rounds": self.args.rounds,
                "passed_rounds": len(self.round_results),
                "recovery_round": self.args.rounds - 1,
                "round_results": self.round_results,
            }
            (self.output / "namespace-workload.json").write_text(json.dumps(workload, ensure_ascii=False, indent=2) + "\n")
            self.snapshot_metrics()
            self.restart_and_verify()
            self.collect_logs()
            command(["python3", str(EVALUATOR), str(self.output), "--output", str(self.output / "evaluation.json")])
            (self.output / "result.txt").write_text(
                "PASS\ndeployment=three-vm\n"
                f"rounds={self.args.rounds}\n"
                "namespace=mkdir/create/readdir/rename/unlink/rmdir across Node A and Node B\n"
                "recovery=Meta C and Node B restart preserve namespace and file binding\n"
            )
            completed = True
        finally:
            if not completed:
                self.collect_failure_logs()
            self.cleanup()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dms-node", type=Path, required=True)
    parser.add_argument("--dms-meta", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--run-id", default=f"m1-{int(time.time())}")
    parser.add_argument("--rounds", type=int, default=20)
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--worker-port", type=int, default=30277)
    parser.add_argument("--node-health-port", type=int, default=30178)
    parser.add_argument("--meta-port", type=int, default=30377)
    parser.add_argument("--meta-health-port", type=int, default=30177)
    args = parser.parse_args()
    if args.rounds < 1:
        raise ValueError("--rounds must be positive")
    Harness(args).execute()
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

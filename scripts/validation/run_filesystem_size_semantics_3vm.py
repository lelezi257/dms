#!/usr/bin/env python3
"""三 VM 原生 Filesystem 文件尺寸语义验证入口。

默认拓扑：A=写入 Node，B=读取 Node，C=Meta。脚本复用单 VM 合同：只用 POSIX/FUSE
触发业务，metrics 只作为 sparse hole 是否物化分配的白盒证据。
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import time


ROOT = Path(__file__).resolve().parents[2]
WORKLOAD = ROOT / "scripts/validation/filesystem_size_semantics_workload.py"
EVALUATOR = ROOT / "scripts/validation/evaluate_filesystem_size_semantics.py"
RUN_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}")


def validated_run_id(value: str) -> str:
    """限制远端临时目录名，禁止路径分隔符、展开符和父目录片段。"""

    if RUN_ID_PATTERN.fullmatch(value) is None:
        raise argparse.ArgumentTypeError(
            "run-id must be 1-64 ASCII letters, digits, '_' or '-', and start alphanumeric"
        )
    return value


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
        self.remote = f"/tmp/dms-filesystem-size-{args.run_id}"
        self.mount = {role: f"{self.remote}/mnt" for role in ("A", "B")}
        self.output = args.output.resolve()

    def shell(self, role: str, script: str, *, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
        clean_env = (
            "export NO_PROXY='*'; "
            "unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy; "
        )
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

    def wait(self, role: str, description: str, script: str, timeout: float = 20.0) -> None:
        deadline = time.monotonic() + timeout
        last_detail = ""
        while True:
            result = self.shell(role, script, capture=True, check=False)
            if result.returncode == 0:
                return
            last_detail = ((result.stdout or "") + (result.stderr or ""))[-1000:]
            if time.monotonic() >= deadline:
                raise TimeoutError(f"timed out waiting for {description} on {role}; last output: {last_detail!r}")
            time.sleep(0.05)

    def wait_meta_live_nodes(self, expected: int) -> None:
        """等待 Meta 重新观察到指定数量的 live Node 会话。

        这是进程组 ready 条件，不是业务可见性重试：后面的 stat/read 仍然只执行一次，
        用来证明 mutation 返回后或重启恢复后的第一次业务访问即可成功。
        """

        script = (
            f"curl -fsS http://{self.ips['C']}:{self.args.meta_health_port}/metrics "
            "| awk '$1 == \"dms_meta_node_sessions{state=\\\"live\\\"}\" {print int($2)}' "
            f"| tail -n 1 | awk '{{exit !($1 >= {expected})}}'"
        )
        self.wait("C", f"Meta observes {expected} live Node sessions", script)

    def spawn(self, role: str, name: str, argv: list[str]) -> None:
        script = (
            f"env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy NO_PROXY='*' "
            f"nohup {shlex.join(argv)} >{shlex.quote(self.remote + '/' + name + '.log')} 2>&1 </dev/null & "
            f"echo $! >{shlex.quote(self.remote + '/' + name + '.pid')}"
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
        profile = {
            "schema": "dms.filesystem.size-semantics-3vm-profile.v1",
            "run_id": self.args.run_id,
            "vms": self.vms,
            "ips": self.ips,
            "artifacts": {
                "dms_node": {"path": str(self.args.dms_node), "sha256": digest(self.args.dms_node)},
                "dms_meta": {"path": str(self.args.dms_meta), "sha256": digest(self.args.dms_meta)},
            },
        }
        (self.output / "profile.json").write_text(json.dumps(profile, ensure_ascii=False, indent=2) + "\n")
        for role in ("A", "B", "C"):
            self.shell(role, f"test ! -e {shlex.quote(self.remote)} && mkdir -p {shlex.quote(self.remote + '/bin')}")
        for role in ("A", "B"):
            self.copy_to(role, self.args.dms_node, self.remote + "/bin/dms-node")
            self.copy_to(role, WORKLOAD, self.remote + "/filesystem_size_semantics_workload.py")
            self.shell(role, f"chmod 755 {shlex.quote(self.remote + '/bin/dms-node')}")
            self.shell(role, f"mkdir -p {shlex.quote(self.mount[role])}")
        self.copy_to("C", self.args.dms_meta, self.remote + "/bin/dms-meta")
        self.shell("C", f"chmod 755 {shlex.quote(self.remote + '/bin/dms-meta')}; mkdir -p {shlex.quote(self.remote + '/journal')}")

    def start_meta(self) -> None:
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
                getattr(self.args, "log_level", "warn"),
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait("C", "Meta ready", f"curl -fsS http://{self.ips['C']}:{self.args.meta_health_port}/readyz >/dev/null")

    def start_node(self, role: str) -> None:
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
                str(256 * 1024 * 1024),
                "--region-size-bytes",
                str(64 * 1024 * 1024),
                "--node-current-cache-bytes",
                str(8 * 1024 * 1024),
                "--log-level",
                getattr(self.args, "log_level", "warn"),
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait(role, "Node ready", f"curl -fsS http://{self.ips[role]}:{self.args.node_health_port}/readyz >/dev/null")
        self.wait(role, "FUSE mount", f"mountpoint -q {shlex.quote(self.mount[role])}")

    def python(self, role: str, source: str) -> None:
        self.shell(role, "python3 -c " + shlex.quote(source))

    def arena_logical_bytes(self) -> float | None:
        result = self.shell(
            "A",
            f"curl -fsS http://{self.ips['A']}:{self.args.node_health_port}/metrics "
            "| awk '/^dms_node_arena_logical_bytes / {print $2}'",
            capture=True,
            check=False,
        )
        if result.returncode != 0 or not result.stdout.strip():
            return None
        return float(result.stdout.strip().splitlines()[-1])

    def remote_stat_size(self, role: str, path: str, expected: int) -> None:
        self.python(
            role,
            "from pathlib import Path\n"
            f"path=Path({path!r})\n"
            f"expected={expected!r}\n"
            "actual=path.stat().st_size\n"
            "raise SystemExit(0 if actual == expected else f'size mismatch {actual} != {expected}')",
        )

    def remote_read_equals(self, role: str, path: str, expected: bytes) -> None:
        self.python(
            role,
            "from pathlib import Path\n"
            f"path=Path({path!r})\n"
            f"expected={expected!r}\n"
            "actual=path.read_bytes()\n"
            "raise SystemExit(0 if actual == expected else f'bytes mismatch {actual!r} != {expected!r}')",
        )

    def remote_range_zero(self, role: str, path: str, offset: int, length: int) -> None:
        self.python(
            role,
            "from pathlib import Path\n"
            f"path=Path({path!r})\n"
            f"offset={offset!r}\n"
            f"length={length!r}\n"
            "with path.open('rb') as stream:\n"
            "    stream.seek(offset)\n"
            "    data=stream.read(length)\n"
            "raise SystemExit(0 if data == b'\\0' * length else f'hole is not zero: {data[:64]!r}')",
        )

    def remote_range_equals(self, role: str, path: str, offset: int, expected: bytes) -> None:
        self.python(
            role,
            "from pathlib import Path\n"
            f"path=Path({path!r})\n"
            f"offset={offset!r}\n"
            f"expected={expected!r}\n"
            "with path.open('rb') as stream:\n"
            "    stream.seek(offset)\n"
            "    data=stream.read(len(expected))\n"
            "raise SystemExit(0 if data == expected else f'range mismatch {data!r} != {expected!r}')",
        )

    def stat_summary(self, role: str, path: str) -> dict[str, int]:
        result = self.shell(
            role,
            "python3 -c "
            + shlex.quote(
                "import json\n"
                "from pathlib import Path\n"
                f"stat=Path({path!r}).stat()\n"
                "print(json.dumps({'size': stat.st_size, 'blocks': getattr(stat, 'st_blocks', 0), "
                "'block_size': getattr(stat, 'st_blksize', 0)}))"
            ),
            capture=True,
        )
        return json.loads(result.stdout)

    def run_workload(self) -> str:
        checks = []
        sparse_offset = 1024 * 1024 + 7

        path_a = f"{self.mount['A']}/truncate-shrink.txt"
        path_b = f"{self.mount['B']}/truncate-shrink.txt"
        self.python("A", "import os\nfrom pathlib import Path\n" f"path=Path({path_a!r})\npath.write_bytes(b'0123456789')\nos.truncate(path, 4)")
        self.remote_stat_size("B", path_b, 4)
        self.remote_read_equals("B", path_b, b"0123")
        checks.append({"operation": "truncate_shrink", "remote_stat": self.stat_summary("B", path_b), "expected_bytes": 4})

        path_a = f"{self.mount['A']}/truncate-grow-sparse.txt"
        path_b = f"{self.mount['B']}/truncate-grow-sparse.txt"
        self.python("A", "from pathlib import Path\n" f"Path({path_a!r}).write_bytes(b'abc')")
        before = self.arena_logical_bytes()
        self.python("A", "import os\nfrom pathlib import Path\n" f"path=Path({path_a!r})\nos.truncate(path, {sparse_offset})")
        after = self.arena_logical_bytes()
        self.remote_stat_size("B", path_b, sparse_offset)
        self.remote_range_zero("B", path_b, 3, 4096)
        self.remote_range_zero("B", path_b, sparse_offset - 16, 16)
        checks.append(
            {
                "operation": "truncate_grow_sparse",
                "target_size": sparse_offset,
                "remote_stat": self.stat_summary("B", path_b),
                "arena_logical_bytes_before": before,
                "arena_logical_bytes_after": after,
                "arena_logical_bytes_delta": None if before is None or after is None else after - before,
                "max_expected_allocation_delta": 4096,
            }
        )

        path_a = f"{self.mount['A']}/pwrite-beyond-eof.txt"
        path_b = f"{self.mount['B']}/pwrite-beyond-eof.txt"
        self.python("A", "from pathlib import Path\n" f"Path({path_a!r}).write_bytes(b'head')")
        before = self.arena_logical_bytes()
        self.python(
            "A",
            "import os\n"
            f"path={path_a!r}\n"
            "fd=os.open(path, os.O_RDWR)\n"
            "try:\n"
            f"    written=os.pwrite(fd, b'tail', {sparse_offset})\n"
            "    assert written == 4\n"
            "finally:\n"
            "    os.close(fd)",
        )
        after = self.arena_logical_bytes()
        self.remote_stat_size("B", path_b, sparse_offset + 4)
        self.remote_range_zero("B", path_b, 4, 4096)
        self.remote_range_equals("B", path_b, 0, b"head")
        self.remote_range_equals("B", path_b, sparse_offset, b"tail")
        checks.append(
            {
                "operation": "pwrite_beyond_eof_sparse",
                "offset": sparse_offset,
                "patch_bytes": 4,
                "remote_stat": self.stat_summary("B", path_b),
                "arena_logical_bytes_before": before,
                "arena_logical_bytes_after": after,
                "arena_logical_bytes_delta": None if before is None or after is None else after - before,
                "max_expected_allocation_delta": 4100,
            }
        )

        path_a = f"{self.mount['A']}/open-o-trunc.txt"
        path_b = f"{self.mount['B']}/open-o-trunc.txt"
        self.python(
            "A",
            "import os\nfrom pathlib import Path\n"
            f"path=Path({path_a!r})\npath.write_bytes(b'before')\nfd=os.open(path, os.O_WRONLY | os.O_TRUNC)\nos.close(fd)",
        )
        self.remote_stat_size("B", path_b, 0)
        self.remote_read_equals("B", path_b, b"")
        checks.append({"operation": "open_o_trunc", "remote_stat": self.stat_summary("B", path_b)})

        path_a = f"{self.mount['A']}/append.txt"
        path_b = f"{self.mount['B']}/append.txt"
        self.python(
            "A",
            "import os, threading\n"
            "from pathlib import Path\n"
            f"path=Path({path_a!r})\npath.write_bytes(b'')\n"
            "records=[f'record-{index:04d}\\n'.encode() for index in range(16)]\n"
            "barrier=threading.Barrier(len(records)); errors=[]\n"
            "def writer(record):\n"
            "    try:\n"
            "        fd=os.open(path, os.O_WRONLY | os.O_APPEND)\n"
            "        try:\n"
            "            barrier.wait(timeout=5); assert os.write(fd, record) == len(record)\n"
            "        finally:\n"
            "            os.close(fd)\n"
            "    except Exception as error:\n"
            "        errors.append(str(error))\n"
            "threads=[threading.Thread(target=writer, args=(record,)) for record in records]\n"
            "[thread.start() for thread in threads]\n"
            "[thread.join(timeout=10) for thread in threads]\n"
            "if errors: raise SystemExit('; '.join(errors))\n",
        )
        self.python(
            "B",
            "from pathlib import Path\n"
            f"path=Path({path_b!r})\n"
            "expected=[f'record-{index:04d}\\n'.encode() for index in range(16)]\n"
            "actual=path.read_bytes().splitlines(keepends=True)\n"
            "raise SystemExit(0 if sorted(actual) == sorted(expected) else f'append mismatch {actual!r}')",
        )
        checks.append({"operation": "concurrent_o_append", "record_count": 16, "remote_stat": self.stat_summary("B", path_b)})

        path_a = f"{self.mount['A']}/visibility.txt"
        path_b = f"{self.mount['B']}/visibility.txt"
        expected = "dms-size:cross-node"
        self.python("A", "from pathlib import Path\n" f"Path({path_a!r}).write_bytes({expected.encode()!r})")
        self.remote_stat_size("B", path_b, len(expected.encode()))
        self.remote_read_equals("B", path_b, expected.encode())
        checks.append({"operation": "cross_node_visibility", "remote_stat": self.stat_summary("B", path_b), "bytes": len(expected), "path": "/visibility.txt"})

        result = {
            "schema": "dms.filesystem.size-semantics-workload.v1",
            "deployment": "three-vm",
            "passed_operations": len(checks),
            "checks": checks,
            "recovery_path": "/visibility.txt",
            "recovery_expected": expected,
        }
        (self.output / "size-workload.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        return expected

    def snapshot_metrics(self) -> None:
        for role, url, filename in (
            ("A", f"http://{self.ips['A']}:{self.args.node_health_port}/metrics", "node-a.prom"),
            ("B", f"http://{self.ips['B']}:{self.args.node_health_port}/metrics", "node-b.prom"),
            ("C", f"http://{self.ips['C']}:{self.args.meta_health_port}/metrics", "meta.prom"),
        ):
            result = self.shell(role, f"curl -fsS {shlex.quote(url)}", capture=True)
            (self.output / filename).write_text(result.stdout, encoding="utf-8")

    def snapshot_metrics_after_restart(self) -> None:
        for role, url, filename in (
            ("A", f"http://{self.ips['A']}:{self.args.node_health_port}/metrics", "node-a-after-restart.prom"),
            ("B", f"http://{self.ips['B']}:{self.args.node_health_port}/metrics", "node-b-after-restart.prom"),
            ("C", f"http://{self.ips['C']}:{self.args.meta_health_port}/metrics", "meta-after-restart.prom"),
        ):
            result = self.shell(role, f"curl -fsS {shlex.quote(url)}", capture=True)
            (self.output / filename).write_text(result.stdout, encoding="utf-8")

    def restart_and_verify(self, expected: str) -> None:
        self.stop("C", "meta")
        self.start_meta()
        self.stop("B", "node")
        self.shell("B", f"fusermount3 -uz {shlex.quote(self.mount['B'])} 2>/dev/null || true", check=False)
        self.start_node("B")
        self.wait_meta_live_nodes(2)
        output = self.remote + "/size-recovery.json"
        self.shell(
            "B",
            "python3 "
            + shlex.join(
                [
                    self.remote + "/filesystem_size_semantics_workload.py",
                    "--mount-a",
                    self.mount["A"],
                    "--mount-b",
                    self.mount["B"],
                    "--recovery-only",
                    "--recovery-expected",
                    expected,
                    "--output",
                    output,
                ]
            ),
        )
        self.copy_from("B", output, self.output / "size-recovery.json")

    def collect_logs(self) -> None:
        for role, remote_name, local_name in (
            ("A", "node.log", "node-a.log"),
            ("B", "node.log", "node-b.log"),
            ("C", "meta.log", "meta.log"),
        ):
            if self.shell(role, f"test -f {shlex.quote(self.remote + '/' + remote_name)}", check=False).returncode == 0:
                self.copy_from(role, self.remote + "/" + remote_name, self.output / local_name)

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
            self.start_meta()
            self.start_node("A")
            self.start_node("B")
            self.wait_meta_live_nodes(2)
            expected = self.run_workload()
            self.snapshot_metrics()
            self.restart_and_verify(expected)
            self.snapshot_metrics_after_restart()
            command(["python3", str(EVALUATOR), str(self.output), "--output", str(self.output / "evaluation.json")])
            (self.output / "result.txt").write_text("PASS\ndeployment=three-vm\nsize semantics verified\n", encoding="utf-8")
            completed = True
        finally:
            self.collect_logs()
            self.cleanup()
            if not completed:
                raise


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dms-node", type=Path, required=True)
    parser.add_argument("--dms-meta", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--run-id",
        type=validated_run_id,
        default=validated_run_id(f"size-{int(time.time())}"),
    )
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--worker-port", type=int, default=30477)
    parser.add_argument("--node-health-port", type=int, default=30478)
    parser.add_argument("--meta-port", type=int, default=30577)
    parser.add_argument("--meta-health-port", type=int, default=30578)
    args = parser.parse_args()
    Harness(args).execute()
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

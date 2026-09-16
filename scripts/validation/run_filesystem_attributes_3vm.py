#!/usr/bin/env python3
"""三 VM 原生 Filesystem 属性、ACL 与集群容量验证入口。

默认拓扑：A=写入 Node，B=读取 Node，C=Meta。业务只经 POSIX/FUSE 触发；
脚本验证 mutation 返回后的跨 Node 可见性、Meta 汇总 statfs，以及 Meta/Node
重启后的 WAL 恢复。临时进程和挂载由脚本精确清理。
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


RUN_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}")


def validated_run_id(value: str) -> str:
    """限制远端临时目录名，避免把 shell 元字符带入部署命令。"""

    if RUN_ID_PATTERN.fullmatch(value) is None:
        raise argparse.ArgumentTypeError(
            "run-id must be 1-64 ASCII letters, digits, '_' or '-', and start alphanumeric"
        )
    return value


def command(
    argv: list[str], *, capture: bool = False, check: bool = True
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, text=True, capture_output=capture, check=False)
    if check and completed.returncode:
        detail = (completed.stdout or "") + (completed.stderr or "")
        raise RuntimeError(
            f"command failed ({completed.returncode}): {shlex.join(argv)}\n{detail[-4000:]}"
        )
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
        self.remote = f"/tmp/dms-filesystem-attributes-{args.run_id}"
        self.mount = {role: f"{self.remote}/mnt" for role in ("A", "B")}
        self.output = args.output.resolve()

    def shell(
        self, role: str, script: str, *, capture: bool = False, check: bool = True
    ) -> subprocess.CompletedProcess[str]:
        clean_env = (
            "export NO_PROXY='*'; "
            "unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy; "
        )
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
                clean_env + script,
            ],
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
                raise TimeoutError(
                    f"timed out waiting for {description} on {role}; last output: {last_detail!r}"
                )
            time.sleep(0.05)

    def spawn(self, role: str, name: str, argv: list[str]) -> None:
        log = shlex.quote(f"{self.remote}/{name}.log")
        pid = shlex.quote(f"{self.remote}/{name}.pid")
        script = (
            "env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY "
            "-u http_proxy -u https_proxy -u all_proxy NO_PROXY='*' "
            f"nohup {shlex.join(argv)} >{log} 2>&1 </dev/null & echo $! >{pid}"
        )
        self.shell(role, script)

    def stop(self, role: str, name: str) -> None:
        pid_file = f"{self.remote}/{name}.pid"
        self.shell(
            role,
            f"if test -f {shlex.quote(pid_file)}; then "
            f"pid=$(sed -n '1p' {shlex.quote(pid_file)}); "
            'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi',
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
            "schema": "dms.filesystem.attributes-3vm-profile.v1",
            "run_id": self.args.run_id,
            "vms": self.vms,
            "ips": self.ips,
            "artifacts": {
                "dms_node": {
                    "path": str(self.args.dms_node),
                    "sha256": digest(self.args.dms_node),
                },
                "dms_meta": {
                    "path": str(self.args.dms_meta),
                    "sha256": digest(self.args.dms_meta),
                },
            },
        }
        (self.output / "profile.json").write_text(
            json.dumps(profile, ensure_ascii=False, indent=2) + "\n"
        )
        for role in ("A", "B", "C"):
            self.shell(
                role,
                f"test ! -e {shlex.quote(self.remote)} && "
                f"mkdir -p {shlex.quote(self.remote + '/bin')}",
            )
        for role in ("A", "B"):
            self.copy_to(role, self.args.dms_node, self.remote + "/bin/dms-node")
            self.shell(
                role,
                f"chmod 755 {shlex.quote(self.remote + '/bin/dms-node')}; "
                f"mkdir -p {shlex.quote(self.mount[role])}",
            )
        self.copy_to("C", self.args.dms_meta, self.remote + "/bin/dms-meta")
        self.shell(
            "C",
            f"chmod 755 {shlex.quote(self.remote + '/bin/dms-meta')}; "
            f"mkdir -p {shlex.quote(self.remote + '/journal')}",
        )

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
                "--filesystem-max-inodes",
                "10000",
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait(
            "C",
            "Meta ready",
            f"curl -fsS http://{self.ips['C']}:{self.args.meta_health_port}/readyz >/dev/null",
        )

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
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait(
            role,
            "Node ready",
            f"curl -fsS http://{self.ips[role]}:{self.args.node_health_port}/readyz >/dev/null",
        )
        self.wait(role, "FUSE mount", f"mountpoint -q {shlex.quote(self.mount[role])}")

    def wait_live_nodes(self, expected: int) -> None:
        script = (
            f"curl -fsS http://{self.ips['C']}:{self.args.meta_health_port}/metrics "
            "| awk '$1 == \"dms_meta_node_sessions{state=\\\"live\\\"}\" {print int($2)}' "
            f"| tail -n 1 | awk '{{exit !($1 >= {expected})}}'"
        )
        self.wait("C", f"Meta observes {expected} live Nodes", script)

    def python(self, role: str, source: str, *, capture: bool = False) -> subprocess.CompletedProcess[str]:
        return self.shell(role, "python3 -c " + shlex.quote(source), capture=capture)

    @staticmethod
    def acl_helpers() -> str:
        return (
            "import struct\n"
            "ACL_UNDEFINED_ID=0xFFFFFFFF\n"
            "def acl(entries):\n"
            "    return struct.pack('<I', 2) + b''.join(struct.pack('<HHI', *entry) for entry in entries)\n"
            "def named_access(uid):\n"
            "    return acl(((0x01, 6, ACL_UNDEFINED_ID), (0x02, 4, uid), "
            "(0x04, 0, ACL_UNDEFINED_ID), (0x10, 4, ACL_UNDEFINED_ID), "
            "(0x20, 0, ACL_UNDEFINED_ID)))\n"
            "def basic(user, group, other):\n"
            "    return acl(((0x01, user, ACL_UNDEFINED_ID), "
            "(0x04, group, ACL_UNDEFINED_ID), (0x20, other, ACL_UNDEFINED_ID)))\n"
        )

    def run_workload(self) -> dict[str, object]:
        path_a = f"{self.mount['A']}/attributes.txt"
        path_b = f"{self.mount['B']}/attributes.txt"
        directory_a = f"{self.mount['A']}/acl-dir"
        directory_b = f"{self.mount['B']}/acl-dir"
        self.python(
            "A",
            self.acl_helpers()
            + "import os\n"
            + f"path={path_a!r}\n"
            + "fd=os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o640); os.close(fd)\n"
            + "os.setxattr(path, 'user.dms-stage', b'three-vm')\n"
            + "os.setxattr(path, 'system.posix_acl_access', named_access(os.getuid()+1))\n"
            + f"directory={directory_a!r}\n"
            + "os.mkdir(directory, 0o770)\n"
            + "os.setxattr(directory, 'system.posix_acl_default', basic(7, 5, 0))\n"
            + "child=directory + '/child.txt'\n"
            + "fd=os.open(child, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o660); os.close(fd)\n",
        )
        result = self.python(
            "B",
            self.acl_helpers()
            + "import json, os, stat\n"
            + f"path={path_b!r}; directory={directory_b!r}; child=directory + '/child.txt'\n"
            + "st=os.stat(path); fs=os.statvfs(path)\n"
            + "expected=named_access(os.getuid()+1)\n"
            + "assert stat.S_IMODE(st.st_mode) & 0o777 == 0o640\n"
            + "assert os.getxattr(path, 'user.dms-stage') == b'three-vm'\n"
            + "assert os.getxattr(path, 'system.posix_acl_access') == expected\n"
            + "assert os.getxattr(child, 'system.posix_acl_access') == basic(6, 4, 0)\n"
            + "assert stat.S_IMODE(os.stat(child).st_mode) & 0o777 == 0o640\n"
            + "assert fs.f_bsize == 4096 and fs.f_blocks == (2*256*1024*1024)//4096\n"
            + "assert fs.f_files == 10000\n"
            + "print(json.dumps({'inode': st.st_ino, 'mode': stat.S_IMODE(st.st_mode)&0o7777, "
            + "'access_acl_hex': expected.hex(), 'default_acl_hex': os.getxattr(directory, "
            + "'system.posix_acl_default').hex(), 'blocks': fs.f_blocks, 'files': fs.f_files}))\n",
            capture=True,
        )
        payload = json.loads(result.stdout)
        (self.output / "attributes.json").write_text(
            json.dumps(payload, ensure_ascii=False, indent=2) + "\n"
        )
        return payload

    def verify_statfs_same_on_both_nodes(self) -> None:
        values = []
        for role in ("A", "B"):
            result = self.python(
                role,
                "import json, os\n"
                + f"value=os.statvfs({self.mount[role]!r})\n"
                + "print(json.dumps([value.f_bsize,value.f_frsize,value.f_blocks,value.f_bfree,"
                + "value.f_bavail,value.f_files,value.f_ffree,value.f_namemax]))",
                capture=True,
            )
            values.append(json.loads(result.stdout))
        if values[0] != values[1]:
            raise RuntimeError(f"statfs mismatch across Nodes: {values!r}")

    def restart_and_verify(self, expected: dict[str, object]) -> None:
        self.stop("C", "meta")
        self.start_meta()
        self.stop("B", "node")
        self.shell(
            "B",
            f"fusermount3 -uz {shlex.quote(self.mount['B'])} 2>/dev/null || true",
            check=False,
        )
        self.start_node("B")
        self.wait_live_nodes(2)
        result = self.python(
            "B",
            self.acl_helpers()
            + "import json, os, stat\n"
            + f"path={self.mount['B'] + '/attributes.txt'!r}\n"
            + f"directory={self.mount['B'] + '/acl-dir'!r}; child=directory + '/child.txt'\n"
            + "st=os.stat(path); fs=os.statvfs(path)\n"
            + "print(json.dumps({'inode': st.st_ino, 'mode': stat.S_IMODE(st.st_mode)&0o7777, "
            + "'access_acl_hex': os.getxattr(path, 'system.posix_acl_access').hex(), "
            + "'default_acl_hex': os.getxattr(directory, 'system.posix_acl_default').hex(), "
            + "'blocks': fs.f_blocks, 'files': fs.f_files}))\n",
            capture=True,
        )
        actual = json.loads(result.stdout)
        if actual != expected:
            raise RuntimeError(f"recovery mismatch: expected={expected!r} actual={actual!r}")
        (self.output / "recovery.json").write_text(
            json.dumps(actual, ensure_ascii=False, indent=2) + "\n"
        )

    def collect(self) -> None:
        for role, name, local in (
            ("A", "node", "node-a.log"),
            ("B", "node", "node-b.log"),
            ("C", "meta", "meta.log"),
        ):
            remote = f"{self.remote}/{name}.log"
            if self.shell(role, f"test -f {shlex.quote(remote)}", check=False).returncode == 0:
                self.copy_from(role, remote, self.output / local)

    def cleanup(self) -> None:
        for role in ("A", "B"):
            self.shell(
                role,
                f"fusermount3 -uz {shlex.quote(self.mount[role])} 2>/dev/null || true",
                check=False,
            )
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
            self.wait_live_nodes(2)
            expected = self.run_workload()
            self.verify_statfs_same_on_both_nodes()
            self.restart_and_verify(expected)
            (self.output / "result.txt").write_text(
                "PASS\ndeployment=three-vm\nattributes, ACL and statfs verified\n"
            )
            completed = True
        finally:
            self.collect()
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
        default=validated_run_id(f"attributes-{int(time.time())}"),
    )
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--worker-port", type=int, default=30877)
    parser.add_argument("--node-health-port", type=int, default=30878)
    parser.add_argument("--meta-port", type=int, default=30977)
    parser.add_argument("--meta-health-port", type=int, default=30978)
    args = parser.parse_args()
    Harness(args).execute()
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

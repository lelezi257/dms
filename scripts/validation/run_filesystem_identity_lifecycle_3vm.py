#!/usr/bin/env python3
"""三 VM M1.3 文件身份与生命周期验证入口。

默认拓扑：A=写入 Node，B=读取 Node，C=Meta。控制器通过 limactl 在远端启动服务，
业务语义仍由 POSIX/FUSE workload 驱动，评价逻辑复用单 VM evaluator。
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
EVALUATOR = ROOT / "scripts/validation/evaluate_filesystem_identity_lifecycle.py"
WHITEBOX = ROOT / "scripts/validation/extract_filesystem_identity_whitebox.py"
RUN_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}")


def validated_run_id(value: str) -> str:
    """限制远端临时目录名，禁止路径分隔符、shell 展开和父目录片段。"""

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
        self.remote = f"/tmp/dms-filesystem-identity-{args.run_id}"
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
            "schema": "dms.filesystem.identity-lifecycle-3vm-profile.v1",
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
                "info",
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
                "debug",
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait(role, "Node ready", f"curl -fsS http://{self.ips[role]}:{self.args.node_health_port}/readyz >/dev/null")
        self.wait(role, "FUSE mount", f"mountpoint -q {shlex.quote(self.mount[role])}")

    def python(self, role: str, source: str, *, check: bool = True) -> subprocess.CompletedProcess[str]:
        return self.shell(role, "python3 -c " + shlex.quote(source), capture=True, check=check)

    def run_workload(self) -> int:
        """由控制器分别驱动 A/B 的挂载点，避免把远端路径误当成本地路径。"""

        payload = b"dms-identity-v1"
        recovery_payload = b"dms-recovery-v1"
        mount_a = self.mount["A"]
        mount_b = self.mount["B"]

        self.python(
            "A",
            "import os\nfrom pathlib import Path\n"
            f"base=Path({mount_a!r})/'hardlink'; base.mkdir()\n"
            f"source=base/'a.txt'; source.write_bytes({payload!r}); os.link(source, base/'b.txt')",
        )
        self.wait("B", "hardlink visibility", f"test -f {shlex.quote(mount_b + '/hardlink/a.txt')} && test -f {shlex.quote(mount_b + '/hardlink/b.txt')}")
        hardlink = json.loads(
            self.python(
                "B",
                "import json\nfrom pathlib import Path\n"
                f"a=Path({(mount_b + '/hardlink/a.txt')!r}); b=Path({(mount_b + '/hardlink/b.txt')!r})\n"
                "sa=a.stat(); sb=b.stat()\n"
                "print(json.dumps({'source_inode':sa.st_ino,'linked_inode':sb.st_ino,'source_nlink':sa.st_nlink,'linked_nlink':sb.st_nlink,'bytes':len(b.read_bytes())}))",
            ).stdout
        )
        if hardlink["source_inode"] != hardlink["linked_inode"] or hardlink["source_nlink"] != 2:
            raise AssertionError("cross-node hardlink identity mismatch")
        remote_read_bytes = hardlink.pop("bytes")
        checks: list[dict[str, object]] = [
            {
                "operation": "hardlink_same_inode",
                **hardlink,
                "same_inode": True,
                "remote_stat_attempts": 1,
                "remote_read_bytes": remote_read_bytes,
            }
        ]

        self.shell("A", f"rm {shlex.quote(mount_a + '/hardlink/a.txt')}")
        self.wait("B", "one hardlink removed", f"test ! -e {shlex.quote(mount_b + '/hardlink/a.txt')} && test -f {shlex.quote(mount_b + '/hardlink/b.txt')}")
        remaining = json.loads(
            self.python(
                "B",
                "import json\nfrom pathlib import Path\n"
                f"p=Path({(mount_b + '/hardlink/b.txt')!r}); s=p.stat(); print(json.dumps({{'inode':s.st_ino,'nlink':s.st_nlink,'bytes':len(p.read_bytes())}}))",
            ).stdout
        )
        checks.append(
            {
                "operation": "unlink_one_link_keeps_other_link",
                "removed_exists": False,
                "remaining_inode": remaining["inode"],
                "remaining_nlink": remaining["nlink"],
                "remaining_bytes": remaining["bytes"],
            }
        )

        unlink_open = json.loads(
            self.python(
                "A",
                "import json, os\nfrom pathlib import Path\n"
                f"base=Path({mount_a!r})/'unlink-open'; base.mkdir(); p=base/'open.txt'; p.write_bytes({payload!r})\n"
                "fd=os.open(p, os.O_RDONLY)\n"
                "try:\n p.unlink(); data=os.read(fd, 4096)\nfinally:\n os.close(fd)\n"
                "print(json.dumps({'bytes':len(data)}))",
            ).stdout
        )
        self.wait("B", "unlink-open namespace removal", f"test ! -e {shlex.quote(mount_b + '/unlink-open/open.txt')}")
        checks.append(
            {
                "operation": "unlink_open_keeps_file_readable_until_close",
                "namespace_visible_after_unlink": False,
                "fd_read_bytes": unlink_open["bytes"],
                "close_completed": True,
            }
        )

        self.python(
            "A",
            "import os\nfrom pathlib import Path\n"
            f"base=Path({mount_a!r})/'symlink'; base.mkdir(); (base/'target.txt').write_bytes({payload!r}); os.symlink('target.txt', base/'link')",
        )
        self.wait("B", "symlink visibility", f"test -L {shlex.quote(mount_b + '/symlink/link')}")
        symlink = json.loads(
            self.python(
                "B",
                "import json, os\nfrom pathlib import Path\n"
                f"link=Path({(mount_b + '/symlink/link')!r}); print(json.dumps({{'target':os.readlink(link),'bytes':len(link.read_bytes())}}))",
            ).stdout
        )
        self.shell("A", f"rm {shlex.quote(mount_a + '/symlink/link')}")
        self.wait("B", "symlink removal keeps target", f"test ! -e {shlex.quote(mount_b + '/symlink/link')} && test -f {shlex.quote(mount_b + '/symlink/target.txt')}")
        checks.append(
            {
                "operation": "symlink_readlink_exact_target",
                "readlink": symlink["target"],
                "readlink_matches": symlink["target"] == "target.txt",
                "readlink_bytes": len(symlink["target"].encode()),
                "target_survived_unlink": True,
                "target_bytes": symlink["bytes"],
            }
        )

        orphan = json.loads(
            self.python(
                "A",
                "import json, os\nfrom pathlib import Path\n"
                f"base=Path({mount_a!r})/'orphan'; base.mkdir(); p=base/'last.txt'; p.write_bytes({payload!r})\n"
                "fd=os.open(p, os.O_RDONLY); inode=os.fstat(fd).st_ino\n"
                "try:\n p.unlink(); data=os.read(fd, 4096)\nfinally:\n os.close(fd)\n"
                "print(json.dumps({'inode':inode,'bytes':len(data)}))",
            ).stdout
        )
        self.wait("B", "orphan namespace removal", f"test ! -e {shlex.quote(mount_b + '/orphan/last.txt')}")
        checks.append(
            {
                "operation": "orphan_lifecycle_observed",
                "namespace_visible_after_last_unlink": False,
                "open_ref_protected_read": orphan["bytes"] == len(payload),
                "inode": orphan["inode"],
            }
        )

        self.python(
            "A",
            "import os\nfrom pathlib import Path\n"
            f"base=Path({mount_a!r})/'identity'; base.mkdir(); a=base/'recovery-hardlink-a'; b=base/'recovery-hardlink-b'; a.write_bytes({recovery_payload!r}); os.link(a,b); a.unlink(); "
            f"target=base/'recovery-target.txt'; target.write_bytes({recovery_payload!r}); os.symlink('recovery-target.txt', base/'recovery-symlink'); orphan=base/'recovery-orphan'; orphan.write_bytes({recovery_payload!r}); orphan.unlink()",
        )
        self.wait("B", "recovery anchors", f"test -f {shlex.quote(mount_b + '/identity/recovery-hardlink-b')} && test -L {shlex.quote(mount_b + '/identity/recovery-symlink')} && test ! -e {shlex.quote(mount_b + '/identity/recovery-orphan')}")

        workload = {
            "schema": "dms.filesystem.identity-lifecycle-workload.v1",
            "deployment": "three-vm",
            "passed_operations": len(checks),
            "checks": checks,
            "recovery_hardlink_path": "/identity/recovery-hardlink-b",
            "recovery_hardlink_bytes": len(recovery_payload),
            "recovery_symlink_path": "/identity/recovery-symlink",
            "recovery_symlink_target": "recovery-target.txt",
        }
        (self.output / "identity-workload.json").write_text(
            json.dumps(workload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        return int(orphan["inode"])

    def wait_orphan_reaped(self, inode: int) -> None:
        source = (
            "import json, sys\nfrom pathlib import Path\n"
            f"p=Path({(self.remote + '/meta.log')!r}); target={inode}\n"
            "found=False\n"
            "for line in p.read_text().splitlines() if p.is_file() else []:\n"
            " try: record=json.loads(line)\n"
            " except json.JSONDecodeError: continue\n"
            " if record.get('event')=='meta.filesystem.orphan.reaped' and int(record.get('inode',-1))==target: found=True\n"
            "sys.exit(0 if found else 1)"
        )
        # Meta 启动后有 30s crash-recovery grace；真实验收必须跨过该边界，不能
        # 为了缩短测试而弱化生产安全语义。
        self.wait("C", "durable orphan reap", "python3 -c " + shlex.quote(source), timeout=45)

    def run_recovery(self) -> None:
        hardlink = self.mount["B"] + "/identity/recovery-hardlink-b"
        symlink = self.mount["B"] + "/identity/recovery-symlink"
        orphan = self.mount["B"] + "/identity/recovery-orphan"
        self.wait(
            "B",
            "filesystem state restored after Meta and Node restart",
            (
                f"test -f {shlex.quote(hardlink)} "
                f"&& test -L {shlex.quote(symlink)} "
                f"&& test ! -e {shlex.quote(orphan)}"
            ),
            timeout=25,
        )
        result = json.loads(
            self.python(
                "B",
                "import json, os\nfrom pathlib import Path\n"
                f"hardlink=Path({hardlink!r}); symlink=Path({symlink!r}); orphan=Path({orphan!r})\n"
                "print(json.dumps({'hardlink_bytes':len(hardlink.read_bytes()),'readlink':os.readlink(symlink),'orphan_visible':orphan.exists()}))",
            ).stdout
        )
        recovery = {
            "schema": "dms.filesystem.identity-lifecycle-recovery.v1",
            "hardlink_remaining_path": "/identity/recovery-hardlink-b",
            "hardlink_bytes": result["hardlink_bytes"],
            "symlink_path": "/identity/recovery-symlink",
            "readlink": result["readlink"],
            "orphan_namespace_visible": result["orphan_visible"],
        }
        (self.output / "identity-recovery.json").write_text(
            json.dumps(recovery, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    def scrape_metrics(self, suffix: str = "") -> None:
        targets = {
            "node-a": ("A", self.ips["A"], self.args.node_health_port),
            "node-b": ("B", self.ips["B"], self.args.node_health_port),
            "meta": ("C", self.ips["C"], self.args.meta_health_port),
        }
        for name, (role, ip, port) in targets.items():
            remote_file = f"{self.remote}/{name}{suffix}.prom"
            self.shell(role, f"curl -fsS http://{ip}:{port}/metrics >{shlex.quote(remote_file)}")

    def restart_meta_and_node_b(self) -> None:
        self.stop("C", "meta")
        self.shell(
            "C",
            f"mv {shlex.quote(self.remote + '/meta.log')} {shlex.quote(self.remote + '/meta-initial.log')}",
        )
        self.start_meta()
        self.stop("B", "node")
        self.shell(
            "B",
            f"mv {shlex.quote(self.remote + '/node.log')} {shlex.quote(self.remote + '/node-initial.log')}",
        )
        self.shell("B", f"fusermount3 -uz {shlex.quote(self.mount['B'])} 2>/dev/null || umount -l {shlex.quote(self.mount['B'])} 2>/dev/null || true")
        self.start_node("B")
        self.wait_meta_live_nodes(2)

    def collect(self) -> None:
        for remote_name in (
            "node-a.prom",
            "node-b.prom",
            "meta.prom",
            "node-a-after-restart.prom",
            "node-b-after-restart.prom",
            "meta-after-restart.prom",
        ):
            role = "C" if remote_name.startswith("meta") else "A"
            if remote_name.startswith("node-b"):
                role = "B"
            self.copy_from(role, self.remote + "/" + remote_name, self.output / remote_name)
        for role, remote_name, local_name in (
            ("A", "node.log", "node-a.log"),
            ("B", "node-initial.log", "node-b.log"),
            ("B", "node.log", "node-b-restarted.log"),
            ("C", "meta-initial.log", "meta.log"),
            ("C", "meta.log", "meta-restarted.log"),
        ):
            self.copy_from(role, self.remote + "/" + remote_name, self.output / local_name)

    def collect_failure_evidence(self) -> None:
        """尽可能保留失败现场；单个文件不存在不能覆盖原始失败。"""

        failure = self.output / "failure"
        failure.mkdir(parents=True, exist_ok=True)
        for role, remote_name, local_name in (
            ("A", "node.log", "node-a.log"),
            ("B", "node-initial.log", "node-b-initial.log"),
            ("B", "node.log", "node-b.log"),
            ("C", "meta-initial.log", "meta-initial.log"),
            ("C", "meta.log", "meta.log"),
        ):
            exists = self.shell(
                role,
                f"test -f {shlex.quote(self.remote + '/' + remote_name)}",
                check=False,
            )
            if exists.returncode == 0:
                try:
                    self.copy_from(
                        role,
                        self.remote + "/" + remote_name,
                        failure / local_name,
                    )
                except Exception as error:  # noqa: BLE001 - 不能掩盖主失败
                    (failure / "collection-errors.txt").open("a", encoding="utf-8").write(
                        f"{role}:{remote_name}: {error}\n"
                    )

    def evaluate(self) -> None:
        completed = command(
            [
                "python3",
                str(EVALUATOR),
                str(self.output),
                "--output",
                str(self.output / "evaluation.json"),
            ],
            capture=True,
            check=False,
        )
        (self.output / "evaluation.stdout").write_text(completed.stdout + completed.stderr, encoding="utf-8")
        if completed.returncode:
            raise RuntimeError(f"evaluation failed\n{completed.stdout}\n{completed.stderr}")

    def cleanup(self) -> None:
        for role in ("A", "B"):
            self.shell(role, f"fusermount3 -uz {shlex.quote(self.mount[role])} 2>/dev/null || umount -l {shlex.quote(self.mount[role])} 2>/dev/null || true", check=False)
            self.stop(role, "node")
        self.stop("C", "meta")

    def remove_remote_workspace(self) -> None:
        for role in ("A", "B", "C"):
            self.shell(role, f"rm -rf -- {shlex.quote(self.remote)}", check=False)

    def run(self) -> None:
        succeeded = False
        try:
            self.prepare()
            self.start_meta()
            self.start_node("A")
            self.start_node("B")
            self.wait_meta_live_nodes(2)
            orphan_inode = self.run_workload()
            self.wait_orphan_reaped(orphan_inode)
            self.scrape_metrics()
            self.restart_meta_and_node_b()
            self.run_recovery()
            self.scrape_metrics("-after-restart")
            self.collect()
            command(
                [
                    "python3",
                    str(WHITEBOX),
                    str(self.output),
                    "--output",
                    str(self.output / "identity-whitebox.json"),
                ]
            )
            self.evaluate()
            (self.output / "result.txt").write_text("PASS\nidentity-lifecycle 3vm\n", encoding="utf-8")
            succeeded = True
            print(self.output)
        except Exception:
            self.collect_failure_evidence()
            raise
        finally:
            self.cleanup()
            if succeeded:
                self.remove_remote_workspace()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--vm-a", required=True)
    parser.add_argument("--vm-b", required=True)
    parser.add_argument("--vm-c", required=True)
    parser.add_argument("--ip-a", required=True)
    parser.add_argument("--ip-b", required=True)
    parser.add_argument("--ip-c", required=True)
    parser.add_argument("--run-id", type=validated_run_id, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--dms-node", type=Path, default=ROOT / "target/debug/dms-node")
    parser.add_argument("--dms-meta", type=Path, default=ROOT / "target/debug/dms-meta")
    parser.add_argument("--worker-port", type=int, default=30002)
    parser.add_argument("--node-health-port", type=int, default=30082)
    parser.add_argument("--meta-port", type=int, default=30001)
    parser.add_argument("--meta-health-port", type=int, default=30081)
    return parser.parse_args()


def main() -> int:
    Harness(parse_args()).run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

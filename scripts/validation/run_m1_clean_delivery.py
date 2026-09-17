#!/usr/bin/env python3
"""M1 交付包干净安装验收。

这个脚本验证的是“用户拿到 server tar 以后能不能脱离源码运行”，因此所有服务
都从解压后的发布包目录启动；源码目录只允许用于构建临时发布包、存放本次证据和
驱动 limactl。失败必须显式暴露，不能因为缺 Docker、缺 FUSE 或缺 VM 而跳过。
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
import sys
import tarfile
import time
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
CASE_ID = "clean-install-single-and-three-vm"


def command(
    argv: list[str],
    *,
    cwd: Path | None = None,
    capture: bool = False,
    check: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        argv,
        cwd=cwd,
        text=True,
        capture_output=capture,
        check=False,
        env=env,
    )
    if check and completed.returncode:
        detail = ((completed.stdout or "") + (completed.stderr or ""))[-4000:]
        raise RuntimeError(f"command failed ({completed.returncode}): {shlex.join(argv)}\n{detail}")
    return completed


def shell(
    script: str,
    *,
    cwd: Path | None = None,
    capture: bool = False,
    check: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return command(["bash", "-lc", script], cwd=cwd, capture=capture, check=check, env=env)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def safe_run_id(value: str) -> str:
    allowed = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-")
    cleaned = "".join(ch if ch in allowed else "-" for ch in value)
    return cleaned.strip(".-") or "clean-install"


def host_path_in_vm(path: Path, host_root: Path, vm_root: Path) -> Path:
    """把 Lima 共享源码树中的 Host 路径翻译成 VM 可见路径。"""

    resolved = path.resolve()
    resolved_root = host_root.resolve()
    try:
        relative = resolved.relative_to(resolved_root)
    except ValueError as error:
        raise RuntimeError(f"path is outside shared source root: {resolved}") from error
    return vm_root / relative


def render_config(example: str, overrides: dict[str, str]) -> str:
    """按包内 dms.env.example 生成节点配置。

    保留示例中的注释，便于失败时直接阅读现场；不存在的新 key 会追加在文件末尾。
    """

    remaining = dict(overrides)
    lines: list[str] = []
    for line in example.splitlines():
        if "=" not in line or line.lstrip().startswith("#"):
            lines.append(line)
            continue
        key, _ = line.split("=", 1)
        if key in remaining:
            lines.append(f"{key}={remaining.pop(key)}")
        else:
            lines.append(line)
    for key in sorted(remaining):
        lines.append(f"{key}={remaining[key]}")
    return "\n".join(lines) + "\n"


def install_archive(archive: Path, install_root: Path) -> Path:
    if install_root.exists():
        raise RuntimeError(f"install root already exists: {install_root}")
    install_root.mkdir(parents=True)
    with tarfile.open(archive, "r:gz") as tar:
        members = tar.getmembers()
        if not members:
            raise RuntimeError(f"archive is empty: {archive}")
        top_levels = {Path(member.name).parts[0] for member in members if member.name}
        if len(top_levels) != 1:
            raise RuntimeError(f"archive must contain exactly one top directory: {sorted(top_levels)}")
        resolved_root = install_root.resolve()
        for member in members:
            target = (install_root / member.name).resolve()
            if target != resolved_root and not target.is_relative_to(resolved_root):
                raise RuntimeError(f"archive member escapes install root: {member.name}")
        tar.extractall(install_root)
    package_dir = install_root / next(iter(top_levels))
    if not (package_dir / "SERVER-PACKAGE-MANIFEST.json").is_file():
        raise RuntimeError(f"missing SERVER-PACKAGE-MANIFEST.json in {package_dir}")
    command(["sha256sum", "-c", "SHA256SUMS"], cwd=package_dir, capture=True)
    return package_dir


def build_archive(
    output: Path,
    run_id: str,
    explicit_archive: Path | None,
    *,
    build_vm: str | None = None,
    build_vm_root: Path | None = None,
) -> dict[str, Any]:
    if explicit_archive is not None:
        archive = explicit_archive.resolve()
        if not archive.is_file():
            raise RuntimeError(f"server archive is missing: {archive}")
        return {"archive": archive, "built_from_source": False, "sha256": sha256(archive)}

    package_root = output / "package-build"
    third_party_dir = output / "third-party"
    package_root.mkdir(parents=True)
    build_id = safe_run_id(f"m1-clean-{run_id}")
    # source scripts/env.sh 是维护者构包入口；真正的安装和运行仍只使用 tar 内容。
    #
    # package.sh 明确要求正式包携带第三方许可材料。干净验收不能假设源码树已经
    # 预生成 THIRD-PARTY-LICENSES/，因此在构包前先生成本次专用清单目录，再
    # 通过 DMS_THIRD_PARTY_DIR 显式交给 package.sh。
    host_script = (
        "source scripts/env.sh && "
        f"python3 scripts/release/dependency_inventory.py --output {shlex.quote(str(third_party_dir))} && "
        "./scripts/build.sh && "
        f"DMS_THIRD_PARTY_DIR={shlex.quote(str(third_party_dir))} "
        f"DMS_PACKAGE_BUILD_ID={shlex.quote(build_id)} ./scripts/package.sh {shlex.quote(str(package_root))}"
    )
    if platform.system() == "Linux":
        shell(host_script, cwd=ROOT)
    else:
        if not build_vm or build_vm_root is None:
            raise RuntimeError(
                "building a clean-install package on a non-Linux host requires "
                "--build-vm and --build-vm-root, or an explicit --archive"
            )
        vm_output = host_path_in_vm(output, ROOT, build_vm_root)
        vm_package_root = vm_output / "package-build"
        vm_third_party_dir = vm_output / "third-party"
        vm_script = (
            "source scripts/env.sh && "
            f"python3 scripts/release/dependency_inventory.py --output {shlex.quote(str(vm_third_party_dir))} && "
            "./scripts/build.sh && "
            f"DMS_THIRD_PARTY_DIR={shlex.quote(str(vm_third_party_dir))} "
            f"DMS_PACKAGE_BUILD_ID={shlex.quote(build_id)} "
            f"./scripts/package.sh {shlex.quote(str(vm_package_root))}"
        )
        command(
            [
                "limactl",
                "shell",
                "--workdir",
                str(build_vm_root),
                build_vm,
                "--",
                "bash",
                "-lc",
                vm_script,
            ]
        )
    archives = sorted(package_root.rglob("dms-server-*.tar.gz"))
    if len(archives) != 1:
        raise RuntimeError(f"expected one server archive under {package_root}, got {archives}")
    archive = archives[0]
    return {"archive": archive, "built_from_source": True, "sha256": sha256(archive)}


class LocalPackage:
    def __init__(self, package_dir: Path) -> None:
        self.package_dir = package_dir
        self.config_path = package_dir / "config/dms.env"

    @property
    def manifest(self) -> dict[str, Any]:
        return json.loads((self.package_dir / "SERVER-PACKAGE-MANIFEST.json").read_text())

    def write_config(self, overrides: dict[str, str]) -> None:
        example = (self.package_dir / "config/dms.env.example").read_text()
        self.config_path.write_text(render_config(example, overrides), encoding="utf-8")

    def run(self, *args: str, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
        return command(["./scripts/cluster.sh", *args], cwd=self.package_dir, capture=capture, check=check)

    def stop_all(self) -> None:
        self.run("all", "stop", check=False)


def wait_for(predicate, description: str, timeout: float = 15.0) -> int:
    deadline = time.monotonic() + timeout
    attempts = 0
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        attempts += 1
        try:
            if predicate():
                return attempts
        except Exception as error:  # noqa: BLE001 - evidence keeps the last failure detail.
            last_error = error
        time.sleep(0.1)
    if last_error is not None:
        raise TimeoutError(f"timed out waiting for {description}: {last_error}")
    raise TimeoutError(f"timed out waiting for {description}")


def assert_no_pid_files(run_dir: Path) -> None:
    remaining = sorted(run_dir.glob("*.pid"))
    if remaining:
        raise RuntimeError(f"pid files remain after stop: {[path.name for path in remaining]}")


def run_single_vm(archive: Path, output: Path, run_id: str, port_base: int) -> dict[str, Any]:
    if platform.system() != "Linux":
        raise RuntimeError("single-vm clean install requires Linux")
    if not Path("/dev/fuse").exists():
        raise RuntimeError("single-vm clean install requires /dev/fuse")
    for tool in ("fusermount3", "curl", "sha256sum"):
        if shutil.which(tool) is None:
            raise RuntimeError(f"single-vm clean install requires {tool}")

    package = LocalPackage(install_archive(archive, output / "single-install"))
    mountpoint = package.package_dir / "mnt"
    run_dir = Path("/tmp") / f"dms-run-{run_id}-single"
    package.write_config(
        {
            "DMS_NODE_ID": f"{run_id}-single-node",
            "DMS_META_NODE_ID": f"{run_id}-single-meta",
            "DMS_META_BIND": f"0.0.0.0:{port_base + 300}",
            "DMS_META_STATUS_BIND": f"0.0.0.0:{port_base + 100}",
            "DMS_META_ENDPOINT": f"http://127.0.0.1:{port_base + 300}",
            "DMS_NODE_IP": "127.0.0.1",
            "DMS_WORKER_PORT": str(port_base + 200),
            "DMS_NODE_STATUS_BIND": f"0.0.0.0:{port_base}",
            "DMS_CLIENT_ENDPOINT": f"http://127.0.0.1:{port_base + 200}",
            "DMS_CLIENT_METRICS_BIND": f"0.0.0.0:{port_base + 400}",
            "DMS_RUN_DIR": str(run_dir),
            "DMS_FUSE_MOUNTPOINT": str(mountpoint),
            "DMS_TRACING_ENABLED": "false",
        }
    )

    evidence: dict[str, Any] = {
        "topology": "single-vm",
        "package_dir": str(package.package_dir),
        "package_version": package.manifest["version"],
        "mountpoint": str(mountpoint),
        "run_dir": str(run_dir),
        "steps": [],
    }
    try:
        package.run("meta", "start", capture=True)
        package.run("node", "start", capture=True)
        package.run("client", "start", capture=True)
        evidence["steps"].append("started_meta_node_client")
        package.run("meta", "status", capture=True)
        package.run("node", "status", capture=True)
        package.run("client", "status", capture=True)
        wait_for(lambda: command(["mountpoint", "-q", str(mountpoint)], check=False).returncode == 0, "single FUSE mount")
        (mountpoint / "clean-single.txt").write_bytes(b"dms-clean-single")
        if (mountpoint / "clean-single.txt").read_bytes() != b"dms-clean-single":
            raise RuntimeError("single-vm FUSE read returned wrong bytes")
        sdk_env = dict(os.environ)
        sdk_env["DMS_ENDPOINT"] = f"http://127.0.0.1:{port_base + 200}"
        sdk_get = command(
            [str(package.package_dir / "bin/sdk-kv"), "set", "clean/single", "ok"],
            cwd=package.package_dir,
            capture=True,
            env=sdk_env,
        )
        command(
            [str(package.package_dir / "bin/sdk-kv"), "get", "clean/single", "ok"],
            cwd=package.package_dir,
            capture=True,
            env=sdk_env,
        )
        evidence["sdk_kv_stdout"] = sdk_get.stdout.strip()
        evidence["steps"].append("fuse_create_read_and_sdk_kv")
    finally:
        package.stop_all()
        command(["fusermount3", "-uz", str(mountpoint)], check=False)
    wait_for(lambda: command(["mountpoint", "-q", str(mountpoint)], check=False).returncode != 0, "single FUSE unmount")
    assert_no_pid_files(run_dir)
    evidence["steps"].append("stopped_and_unmounted")
    evidence["status"] = "PASS"
    return evidence


class LimaHarness:
    def __init__(self, args: argparse.Namespace, archive: Path, output: Path, run_id: str) -> None:
        self.args = args
        self.archive = archive
        self.output = output
        self.run_id = run_id
        self.limactl = shutil.which("limactl")
        if self.limactl is None:
            raise RuntimeError("three-vm clean install requires limactl")
        self.vms = {"A": args.vm_a, "B": args.vm_b, "C": args.vm_c}
        self.ips = {"A": args.ip_a, "B": args.ip_b, "C": args.ip_c}
        self.remote = f"/tmp/dms-m1-clean-{run_id}"
        self.package_dir = self.remote + "/pkg"
        self.mount = {role: f"{self.package_dir}/mnt" for role in ("A", "B")}
        self.run_dir = {role: f"/tmp/dms-run-{run_id}-{role.lower()}" for role in ("A", "B", "C")}

    def lima(self, argv: list[str], *, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
        return command([self.limactl, *argv], capture=capture, check=check)

    def shell(self, role: str, script: str, *, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
        return self.lima(
            ["shell", "--workdir", "/tmp", self.vms[role], "--", "bash", "-lc", script],
            capture=capture,
            check=check,
        )

    def copy_to(self, role: str, source: Path, destination: str) -> None:
        self.lima(["copy", str(source), f"{self.vms[role]}:{destination}"])

    def prepare(self) -> None:
        for role in ("A", "B", "C"):
            self.shell(role, f"test ! -e {shlex.quote(self.remote)} && mkdir -p {shlex.quote(self.remote)}")
            self.copy_to(role, self.archive, self.remote + "/server.tar.gz")
            self.shell(
                role,
                f"tar -xzf {shlex.quote(self.remote + '/server.tar.gz')} -C {shlex.quote(self.remote)} && "
                f"mv {shlex.quote(self.remote)}/dms-server-* {shlex.quote(self.package_dir)} && "
                f"cd {shlex.quote(self.package_dir)} && sha256sum -c SHA256SUMS >/dev/null",
            )

    def write_config(self, role: str, overrides: dict[str, str]) -> None:
        payload = json.dumps(overrides, ensure_ascii=False)
        script = f"""
import json
from pathlib import Path
pkg = Path({self.package_dir!r})
overrides = json.loads({payload!r})
lines = []
remaining = dict(overrides)
for line in (pkg / 'config/dms.env.example').read_text().splitlines():
    if '=' not in line or line.lstrip().startswith('#'):
        lines.append(line)
        continue
    key = line.split('=', 1)[0]
    if key in remaining:
        lines.append(f"{{key}}={{remaining.pop(key)}}")
    else:
        lines.append(line)
for key in sorted(remaining):
    lines.append(f"{{key}}={{remaining[key]}}")
(pkg / 'config/dms.env').write_text('\\n'.join(lines) + '\\n')
"""
        self.shell(role, "python3 -c " + shlex.quote(script))

    def configure(self) -> None:
        meta_port = self.args.meta_port
        worker_port = self.args.worker_port
        meta_status_port = self.args.meta_status_port
        node_status_port = self.args.node_status_port
        client_port = self.args.client_metrics_port
        common = {"DMS_TRACING_ENABLED": "false"}
        self.write_config(
            "C",
            {
                **common,
                "DMS_NODE_ID": f"{self.run_id}-meta-node-unused",
                "DMS_META_NODE_ID": f"{self.run_id}-meta",
                "DMS_META_BIND": f"0.0.0.0:{meta_port}",
                "DMS_META_STATUS_BIND": f"0.0.0.0:{meta_status_port}",
                "DMS_META_ENDPOINT": f"http://{self.ips['C']}:{meta_port}",
                "DMS_RUN_DIR": self.run_dir["C"],
                "DMS_FUSE_MOUNTPOINT": "",
            },
        )
        for role in ("A", "B"):
            self.write_config(
                role,
                {
                    **common,
                    "DMS_NODE_ID": f"{self.run_id}-node-{role.lower()}",
                    "DMS_META_NODE_ID": f"{self.run_id}-meta",
                    "DMS_META_BIND": f"0.0.0.0:{meta_port}",
                    "DMS_META_STATUS_BIND": f"0.0.0.0:{meta_status_port}",
                    "DMS_META_ENDPOINT": f"http://{self.ips['C']}:{meta_port}",
                    "DMS_NODE_IP": self.ips[role],
                    "DMS_WORKER_PORT": str(worker_port),
                    "DMS_NODE_STATUS_BIND": f"0.0.0.0:{node_status_port}",
                    "DMS_CLIENT_ENDPOINT": f"http://{self.ips[role]}:{worker_port}",
                    "DMS_CLIENT_METRICS_BIND": f"0.0.0.0:{client_port}",
                    "DMS_RUN_DIR": self.run_dir[role],
                    "DMS_FUSE_MOUNTPOINT": self.mount[role],
                },
            )

    def pkg(self, role: str, *args: str, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
        return self.shell(
            role,
            f"cd {shlex.quote(self.package_dir)} && ./scripts/cluster.sh {shlex.join(list(args))}",
            capture=capture,
            check=check,
        )

    def wait_remote(self, role: str, description: str, script: str, timeout: float = 20.0) -> int:
        deadline = time.monotonic() + timeout
        attempts = 0
        last = ""
        while time.monotonic() < deadline:
            attempts += 1
            result = self.shell(role, script, capture=True, check=False)
            if result.returncode == 0:
                return attempts
            last = ((result.stdout or "") + (result.stderr or ""))[-1000:]
            time.sleep(0.1)
        raise TimeoutError(f"timed out waiting for {description} on {role}: {last!r}")

    def stop_all(self) -> None:
        for role in ("A", "B"):
            self.pkg(role, "all", "stop", check=False)
            self.shell(role, f"fusermount3 -uz {shlex.quote(self.mount[role])} 2>/dev/null || true", check=False)
        self.pkg("C", "all", "stop", check=False)

    def cleanup_workspace(self) -> None:
        """删除只服务于本轮 clean-install 的解包目录和运行目录。"""

        warnings: list[dict[str, str]] = []
        for role in ("A", "B", "C"):
            paths = [self.remote, self.run_dir[role]]
            command = "rm -rf -- " + " ".join(shlex.quote(path) for path in paths)
            completed = self.shell(role, command, check=False)
            if completed.returncode != 0:
                warnings.append(
                    {
                        "role": role,
                        "action": "cleanup_workspace",
                        "command": command,
                        "returncode": str(completed.returncode),
                        "stderr": completed.stderr[-2000:],
                    }
                )
        if warnings:
            (self.output / "cleanup-warnings.json").write_text(
                json.dumps(warnings, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )

    def run(self) -> dict[str, Any]:
        evidence: dict[str, Any] = {
            "topology": "three-vm",
            "vms": self.vms,
            "ips": self.ips,
            "remote_root": self.remote,
            "run_dirs": self.run_dir,
            "steps": [],
        }
        completed = False
        try:
            self.prepare()
            self.configure()
            self.pkg("C", "meta", "start", capture=True)
            self.pkg("A", "node", "start", capture=True)
            self.pkg("B", "node", "start", capture=True)
            self.pkg("A", "client", "start", capture=True)
            evidence["steps"].append("started_meta_two_nodes_and_client")
            for role in ("C", "A", "B"):
                if role == "C":
                    self.pkg(role, "meta", "status", capture=True)
                else:
                    self.pkg(role, "node", "status", capture=True)
            for role in ("A", "B"):
                self.wait_remote(role, f"FUSE mount {role}", f"mountpoint -q {shlex.quote(self.mount[role])}")
            payload = b"dms-clean-three"
            write_script = (
                "from pathlib import Path; "
                f"Path({(self.mount['A'] + '/clean-three.txt')!r}).write_bytes({payload!r})"
            )
            self.shell("A", "python3 -c " + shlex.quote(write_script))
            read_script = (
                "from pathlib import Path; import sys; "
                f"sys.exit(0 if Path({(self.mount['B'] + '/clean-three.txt')!r}).read_bytes() == {payload!r} else 1)"
            )
            attempts = self.wait_remote("B", "cross-node FUSE read", "python3 -c " + shlex.quote(read_script))
            evidence["cross_node_read_attempts"] = attempts
            evidence["steps"].append("cross_node_fuse_create_read")
            self.stop_all()
            for role in ("A", "B"):
                self.wait_remote(
                    role,
                    f"FUSE unmount {role}",
                    f"! mountpoint -q {shlex.quote(self.mount[role])}",
                )
            evidence["steps"].append("stopped_and_unmounted")
            evidence["status"] = "PASS"
            completed = True
            return evidence
        finally:
            if not completed:
                self.stop_all()
            self.cleanup_workspace()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="已生成的 dms-server tar.gz；不传则在 Linux 中临时构建")
    parser.add_argument("--build-vm", help="macOS 上构建当前源码制品所使用的 Linux Lima VM")
    parser.add_argument(
        "--build-vm-root",
        type=Path,
        help="源码根目录在 --build-vm 内的共享路径，例如 /workspace/dms/source",
    )
    parser.add_argument("--output", type=Path, default=ROOT / "evidence/m1/clean-install")
    parser.add_argument("--run-id", default=dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ"))
    parser.add_argument("--topology", choices=("auto", "single-vm", "three-vm", "both"), default="auto")
    parser.add_argument("--port-base", type=int, default=38600)
    parser.add_argument("--vm-a", default="dms-a")
    parser.add_argument("--vm-b", default="dms-b")
    parser.add_argument("--vm-c", default="dms-c")
    parser.add_argument("--ip-a", default="192.168.105.11")
    parser.add_argument("--ip-b", default="192.168.105.12")
    parser.add_argument("--ip-c", default="192.168.105.13")
    parser.add_argument("--meta-port", type=int, default=39300)
    parser.add_argument("--meta-status-port", type=int, default=39100)
    parser.add_argument("--worker-port", type=int, default=39200)
    parser.add_argument("--node-status-port", type=int, default=39000)
    parser.add_argument("--client-metrics-port", type=int, default=39400)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    run_id = safe_run_id(args.run_id)
    output = args.output.resolve() / run_id
    output.mkdir(parents=True, exist_ok=True)

    build = build_archive(
        output,
        run_id,
        args.archive,
        build_vm=args.build_vm,
        build_vm_root=args.build_vm_root,
    )
    archive = build["archive"]
    topologies: list[str]
    if args.topology == "auto":
        topologies = ["three-vm"] if shutil.which("limactl") else ["single-vm"]
    elif args.topology == "both":
        topologies = ["single-vm", "three-vm"]
    else:
        topologies = [args.topology]

    detail: dict[str, Any] = {
        "schema": "dms.m1.clean-install-result.v1",
        "generated_at": utc_now(),
        "case_id": CASE_ID,
        "run_id": run_id,
        "archive": {
            "path": str(archive),
            "sha256": build["sha256"],
            "built_from_source": build["built_from_source"],
        },
        "topologies": [],
        "evidence_files": [],
    }

    try:
        for topology in topologies:
            if topology == "single-vm":
                result = run_single_vm(archive, output, run_id, args.port_base)
            else:
                result = LimaHarness(args, archive, output, run_id).run()
            detail["topologies"].append(result)
            per_topology = output / f"{topology}.json"
            per_topology.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            detail["evidence_files"].append(str(per_topology))

        detail["status"] = "PASS"
        detailed_path = output / "clean-install-result.json"
        detailed_path.write_text(json.dumps(detail, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(detailed_path)
        return 0
    except Exception as error:  # noqa: BLE001 - machine evidence must include the top-level failure.
        detail["status"] = "FAIL"
        detail["error"] = str(error)
        failed_path = output / "clean-install-result.json"
        failed_path.write_text(json.dumps(detail, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"M1 clean install failed: {error}", file=sys.stderr)
        print(failed_path, file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

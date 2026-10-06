#!/usr/bin/env python3
"""Small paired real-container OwnerFs workspace performance diagnostic.

This records OFF/FUSE and experimental ON/native managed-container timings
against ordinary ext4 OCI containers. It is diagnostic evidence only: it does
not qualify G2.12, G2.13, or production native workspace enablement.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import stat
import subprocess
import time
import tomllib
from typing import Any


spec = importlib.util.spec_from_file_location(
    "installed_smoke", Path(__file__).with_name("installed-smoke-linux.py")
)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)

native_spec = importlib.util.spec_from_file_location(
    "native_workspace", Path(__file__).with_name("native-workspace-linux.py")
)
native_base = importlib.util.module_from_spec(native_spec)
native_spec.loader.exec_module(native_base)


DATA_BYTES = 64 * 1024 * 1024
BLOCK_BYTES = 1024 * 1024
PATTERN_BYTE = 90
MEASUREMENT_ROUNDS = 5
WARMUP_ROUNDS = 1
UID = 501
GID = 501


def sha(path: Path | str) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def proc_starttick(pid: int) -> int:
    return int(Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19])


def exact_mount(path: Path) -> dict[str, Any]:
    return json.loads(subprocess.check_output(
        ["findmnt", "-J", "-o", "TARGET,SOURCE,FSTYPE,OPTIONS,ID", "--mountpoint", str(path)],
        text=True,
    ))["filesystems"][0]


def snapshot_host(root: Path) -> dict[str, Any]:
    usage = shutil.disk_usage(root)
    return {
        "loadavg": Path("/proc/loadavg").read_text(encoding="utf-8"),
        "meminfo": Path("/proc/meminfo").read_text(encoding="utf-8"),
        "diskstats": Path("/proc/diskstats").read_text(encoding="utf-8"),
        "opt_usage": {"total": usage.total, "used": usage.used, "free": usage.free},
    }


def parse_prometheus(text: str) -> dict[str, float]:
    values: dict[str, float] = {}
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        parts = line.rsplit(" ", 1)
        if len(parts) != 2:
            continue
        try:
            values[parts[0]] = float(parts[1])
        except ValueError:
            continue
    return values


def verify_io_result(record: dict[str, Any], operation: str, barrier: str) -> None:
    required = {
        "operation": str,
        "file_bytes": int,
        "io_bytes": int,
        "block_bytes": int,
        "concurrency": int,
        "barrier": str,
        "pattern_byte": int,
        "operations": int,
        "wall_ns": int,
        "client_cpu_ns": int,
        "p50_ns": int,
        "p95_ns": int,
        "p99_ns": int,
        "content_ok": bool,
    }
    for name, kind in required.items():
        if not isinstance(record.get(name), kind):
            raise ValueError(f"payload result field {name} has wrong shape")
    if record["operation"] != operation or record["barrier"] != barrier:
        raise ValueError("unexpected operation/barrier")
    if record["file_bytes"] != DATA_BYTES or record["io_bytes"] != DATA_BYTES or record["block_bytes"] != BLOCK_BYTES:
        raise ValueError("unexpected workload size")
    if record["concurrency"] != 1 or record["operations"] != DATA_BYTES // BLOCK_BYTES:
        raise ValueError("unexpected C1 operation count")
    if record["pattern_byte"] != PATTERN_BYTE:
        raise ValueError("unexpected content pattern")
    if record.get("cache_requested") != "unobserved":
        raise ValueError("cache residency must remain unobserved")
    if record["wall_ns"] <= 0 or record["p95_ns"] < record["p50_ns"] or record["p99_ns"] < record["p95_ns"]:
        raise ValueError("invalid timing order")
    if not record["content_ok"]:
        raise ValueError("payload reported invalid content")


def verify_metadata_result(record: dict[str, Any]) -> None:
    if record.get("files") != 1000 or record.get("file_bytes") != 4096:
        raise ValueError("metadata workload shape mismatch")
    if record.get("concurrency") != 1 or record.get("path_form") != "absolute":
        raise ValueError("metadata must use absolute C1 paths")
    expected = ["create_write_close", "stat", "read_close", "readdir", "rename", "unlink"]
    phases = record.get("phases")
    if not isinstance(phases, list) or [phase.get("name") for phase in phases] != expected:
        raise ValueError("metadata benchmark must report the exact six phases")
    for phase in phases:
        if not isinstance(phase.get("wall_ns"), int) or phase["wall_ns"] <= 0:
            raise ValueError("metadata phase timing missing")


def validate_pair(round_record: dict[str, Any]) -> None:
    samples = round_record.get("samples", [])
    if len(samples) != 2:
        raise ValueError("round must contain exactly one experiment and one reference sample")
    names = {sample.get("target") for sample in samples}
    if names != {"experiment", "reference"}:
        raise ValueError(f"missing paired experiment/reference samples: {sorted(names)}")
    for sample in samples:
        for section in ("write", "read", "metadata"):
            if section not in sample:
                raise ValueError(f"sample missing {section}")
        verify_io_result(sample["write"]["result"], "seq-write", "fsync")
        verify_io_result(sample["read"]["result"], "seq-read", "close")
        verify_metadata_result(sample["metadata"]["result"])


def validate_summary(summary: dict[str, Any]) -> None:
    if summary.get("status") != "DATA_RECORDED":
        raise ValueError("performance diagnostic must not claim PASS")
    cohorts = summary.get("cohorts", [])
    if len(cohorts) != 2 or {cohort.get("name") for cohort in cohorts} != {"off", "on"}:
        raise ValueError("summary must contain exactly off and on cohorts")
    for cohort in cohorts:
        rounds = cohort.get("rounds", [])
        if len(rounds) != WARMUP_ROUNDS + MEASUREMENT_ROUNDS:
            raise ValueError("wrong round count")
        for item in rounds:
            validate_pair(item)


def newest_stdout_for_argv(control_dir: Path, argv: list[str], before: set[Path]) -> dict[str, Any]:
    matches: list[tuple[float, Path]] = []
    for command_path in control_dir.glob("command-*.command.json"):
        if command_path in before:
            continue
        try:
            record = read_json(command_path)
        except json.JSONDecodeError:
            continue
        if record.get("argv") == argv:
            stem = control_dir / command_path.name.split(".", 1)[0]
            stdout = stem.with_suffix(".stdout")
            exit_json = stem.with_suffix(".exit.json")
            if stdout.exists() and exit_json.exists():
                matches.append((command_path.stat().st_mtime, stem))
    if not matches:
        raise ValueError("no matching controller command artifact for exact argv")
    stem = sorted(matches)[-1][1]
    exit_record = read_json(stem.with_suffix(".exit.json"))
    if not exit_record.get("success") or exit_record.get("code") != 0:
        raise ValueError("matching controller command did not exit successfully")
    text = stem.with_suffix(".stdout").read_text(encoding="utf-8")
    try:
        payload = json.loads(text)
    except json.JSONDecodeError:
        payload = text
    return {"stem": str(stem), "stdout": payload, "exit": exit_record}


class Driver(base.Run):
    def __init__(self, args: argparse.Namespace) -> None:
        super().__init__(args)
        self.runtime_root = self.root / "runtime-state"
        self.native_container_id: str | None = None

    def raw_command(self, argv: list[Any], out_dir: Path, timeout: int = 300, allowed: tuple[int, ...] = (0,)) -> dict[str, Any]:
        out_dir.mkdir(parents=True, exist_ok=True)
        prefix = f"{len(self.commands):03d}"
        began = time.monotonic()
        timed_out = False
        with (out_dir / f"{prefix}.stdout").open("wb") as stdout, (out_dir / f"{prefix}.stderr").open("wb") as stderr:
            try:
                result = subprocess.run([str(x) for x in argv], stdout=stdout, stderr=stderr, timeout=timeout)
                code = result.returncode
            except subprocess.TimeoutExpired:
                code = 124
                timed_out = True
        record = {
            "argv": [str(x) for x in argv],
            "exit": code,
            "timeout": timed_out,
            "elapsed_seconds": time.monotonic() - began,
            "stdout": str(out_dir / f"{prefix}.stdout"),
            "stderr": str(out_dir / f"{prefix}.stderr"),
        }
        self.commands.append(record)
        self.save("commands.json", self.commands)
        if code not in allowed:
            raise RuntimeError(f"command {prefix} exit {code}: {' '.join(record['argv'])}")
        return record

    def run_text(self, argv: list[Any], out_dir: Path, timeout: int = 300, allowed: tuple[int, ...] = (0,)) -> str:
        record = self.raw_command(argv, out_dir, timeout, allowed)
        return Path(record["stdout"]).read_text(encoding="utf-8")

    def runc(self, argv: list[Any], out_dir: Path, timeout: int = 300, allowed: tuple[int, ...] = (0,)) -> str:
        return self.runc_at(self.runtime_root, argv, out_dir, timeout=timeout, allowed=allowed)

    def runc_at(self, root: Path, argv: list[Any], out_dir: Path, timeout: int = 300, allowed: tuple[int, ...] = (0,)) -> str:
        return self.run_text(
            ["/usr/local/sbin/runc", "--root", root, *argv],
            out_dir,
            timeout=timeout,
            allowed=allowed,
        )

    def native(self, ident: str, *action: str, error: bool = False) -> dict[str, Any]:
        text = self.run_text(
            ["python3", self.args.controller, "--socket", self.root / "on/control/control.sock", "--id", ident, *action],
            self.out / "controller-client",
            timeout=600,
            allowed=(1,) if error else (0,),
        )
        value = json.loads(text)["response"]
        self.check(
            ident + "-response",
            value.get("production_ready") is False and ((value.get("status") == "ERROR") == error),
            value,
        )
        return value

    def preflight(self) -> None:
        self.check(
            "Linux-root",
            platform.system() == "Linux" and platform.machine() == "aarch64" and os.geteuid() == 0,
            [platform.system(), platform.machine(), os.geteuid()],
        )
        self.check("new-opt-root", self.root.is_absolute() and self.root.parent == Path("/opt") and not self.root.exists(), str(self.root))
        deps = {
            name: shutil.which(name)
            for name in (
                "bash",
                "python3",
                "tar",
                "ldd",
                "openssl",
                "findmnt",
                "fusermount3",
                "curl",
                "flock",
                "ss",
                "awk",
                "sed",
                "mountpoint",
                "nsenter",
                "setsid",
            )
        }
        self.check("dependencies", all(deps.values()), deps)
        self.check("fuse", stat.S_ISCHR(os.stat("/dev/fuse").st_mode), "/dev/fuse")
        fs = json.loads(self.command(["findmnt", "-J", "-T", "/opt"]))
        self.check("opt-ext4", fs["filesystems"][0]["fstype"] == "ext4", fs)
        self.check("capacity", shutil.disk_usage("/opt").free >= 4 * 2**30, shutil.disk_usage("/opt").free)
        for port in (24400, 24401, 24500, 24501):
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", port))
        self.check("ports", True, [24400, 24401, 24500, 24501])
        self.check("package", sha(self.args.package) == self.args.package_sha256, sha(self.args.package))
        self.check("runtime", sha("/usr/local/sbin/runc") == self.args.runtime_sha256, sha("/usr/local/sbin/runc"))
        self.check("io-bin", self.args.io_bin.is_file() and os.access(self.args.io_bin, os.X_OK), str(self.args.io_bin))
        self.check("benchmark-bin", self.args.benchmark_bin.is_file() and os.access(self.args.benchmark_bin, os.X_OK), str(self.args.benchmark_bin))
        inputs = read_json(self.args.rootfs_inputs)
        for rel, expected in inputs.items():
            src = self.args.template_rootfs / rel
            self.check("template-" + rel, src.is_file() and not src.is_symlink() and sha(src) == expected["sha256"], expected)
        for name, binary in (("io", self.args.io_bin), ("benchmark", self.args.benchmark_bin)):
            self.check(name + "-trusted-sha", name in inputs and sha(binary) == inputs[name]["sha256"], inputs.get(name))
            libs = self.command(["ldd", binary])
            self.check(name + "-libraries", "not found" not in libs, libs)
        self.save("preflight.json", self.checks)

    def clone_rootfs(self, rootfs: Path) -> dict[str, Any]:
        inputs = read_json(self.args.rootfs_inputs)
        for rel, path in (("io", self.args.io_bin), ("benchmark", self.args.benchmark_bin)):
            if rel not in inputs:
                raise ValueError(f"trusted rootfs inputs missing {rel}")
            if sha(path) != inputs[rel]["sha256"]:
                raise ValueError(f"{rel} payload path does not match trusted input sha256")
        manifest: dict[str, Any] = {}
        rootfs.mkdir(mode=0o755, parents=True, exist_ok=False)
        for rel, expected in inputs.items():
            src = self.args.template_rootfs / rel
            self.check("input-" + rel, src.is_file() and not src.is_symlink() and sha(src) == expected["sha256"], expected)
            dst = rootfs / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(src, dst)
            dst.chmod(int(str(expected.get("mode", "0o755")), 8))
            manifest["/" + rel] = {"sha256": sha(dst), "source": str(src)}
        (rootfs / "proc").mkdir(exist_ok=True)
        (rootfs / "workspace").mkdir(exist_ok=True)
        return manifest

    def install_product(self, cohort: str, native_enabled: bool) -> Path:
        root = self.root / cohort
        root.mkdir(parents=True, mode=0o755, exist_ok=False)
        package_dir = root / "package"
        package_dir.mkdir()
        self.command(["tar", "-xzf", self.args.package, "-C", package_dir])
        packages = list(package_dir.iterdir())
        self.check(cohort + "-one-package", len(packages) == 1, [str(p) for p in packages])
        package = packages[0]
        manifest = read_json(package / "manifest.json")
        self.check(cohort + "-source-commit", manifest["source_commit"] == self.args.source_commit, manifest)
        self.command([
            package / "install.sh",
            "--prefix", root / "prefix",
            "--config-dir", root / "etc",
            "--state-dir", root / "state",
            "--run-dir", root / "run",
            "--log-dir", root / "logs",
            "--mount-root", root / "mount",
        ])
        for name in ("meta", "node"):
            binary = root / f"prefix/bin/afs-{name}"
            self.check(cohort + f"-{name}-ELF", sha(binary) == getattr(self.args, f"afs_{name}_sha256"), sha(binary))
            libs = self.command(["ldd", binary])
            self.check(cohort + f"-{name}-libraries", "not found" not in libs, libs)
        self.command([
            root / "prefix/bin/afs-trial-config",
            "single",
            "--backend",
            "local-file",
            "--config-dir",
            root / "etc",
            "--state-dir",
            root / "state",
            "--run-dir",
            root / "run",
            "--mount-root",
            root / "mount",
            "--meta-grpc-port",
            "24400",
            "--meta-rest-port",
            "24401",
            "--node-grpc-port",
            "24500",
            "--node-rest-port",
            "24501",
            "--force",
        ])
        rootfs = root / "rootfs"
        rootfs_manifest = self.clone_rootfs(rootfs)
        (root / "control").mkdir(mode=0o700, exist_ok=True)
        write_json(self.out / f"{cohort}-rootfs-manifest.json", rootfs_manifest)
        for name in ("meta", "node"):
            config_path = root / f"etc/{name}.toml"
            text = "\n".join(
                line
                for line in config_path.read_text(encoding="utf-8").replace('fs = "all"', 'fs = "ownerfs"').splitlines()
                if not line.startswith("dfs_mount =")
            ) + "\n"
            if name == "node":
                if native_enabled:
                    text = (
                        "experimental_native_workspace = true\n"
                        + text
                        + "\n[native_workspace]\n"
                        + f'control_dir = "{root}/control"\n'
                        + 'runtime = "/usr/local/sbin/runc"\n'
                        + f'rootfs = "{rootfs}"\n'
                        + f"workload_uid = {UID}\nworkload_gid = {GID}\n"
                    )
                else:
                    text = "experimental_native_workspace = false\n" + text
                    parsed = tomllib.loads(text)
                    self.check(cohort + "-native-explicit-false", parsed.get("experimental_native_workspace") is False, parsed)
                    self.check(cohort + "-native-table-absent", "native_workspace" not in parsed, parsed)
            config_path.write_text(text, encoding="utf-8")
            shutil.copyfile(config_path, self.out / f"{cohort}-{name}.toml")
        return root

    def ctl(self, root: Path, *action: str) -> str:
        return self.run_text(
            [
                root / "prefix/bin/afs-processctl",
                "--prefix",
                root / "prefix",
                "--config-dir",
                root / "etc",
                "--run-dir",
                root / "run",
                "--log-dir",
                root / "logs",
                *action,
            ],
            self.out / "processctl",
            timeout=180,
        )

    def product_identity(self, root: Path, cohort: str) -> dict[str, Any]:
        value: dict[str, Any] = {}
        for name in ("meta", "node"):
            pid = int((root / f"run/{name}.pid").read_text(encoding="utf-8"))
            proc = Path(f"/proc/{pid}")
            digest = sha(proc / "exe")
            self.check(cohort + f"-{name}-live-ELF", digest == getattr(self.args, f"afs_{name}_sha256"), digest)
            installed = root / f"prefix/bin/afs-{name}"
            executable = base.verify_executable(proc / "exe", installed)
            self.check(cohort + f"-{name}-installed-process", True, executable)
            value[name] = {"pid": pid, "starttick": proc_starttick(pid), "sha256": digest, "executable": executable}
        value["ownerfs_mount"] = exact_mount(root / "mount/ownerfs")
        self.check(cohort + "-ownerfs-mount", value["ownerfs_mount"]["source"] == "afs-ownerfs", value["ownerfs_mount"])
        write_json(self.out / f"{cohort}-running-identity.json", value)
        return value

    def oci_spec(self, rootfs: Path, workspace: Path, args: list[str], hostname: str) -> dict[str, Any]:
        return {
            "ociVersion": "1.0.2",
            "root": {"path": str(rootfs), "readonly": True},
            "hostname": hostname,
            "process": {
                "terminal": False,
                "cwd": "/",
                "args": args,
                "user": {"uid": UID, "gid": GID},
                "env": ["PATH=/bin:/usr/bin"],
                "noNewPrivileges": True,
                "capabilities": {key: [] for key in ("bounding", "effective", "inheritable", "permitted", "ambient")},
            },
            "mounts": [
                {"destination": "/proc", "type": "proc", "source": "proc", "options": ["nosuid", "nodev", "noexec"]},
                {"destination": "/workspace", "type": "bind", "source": str(workspace), "options": ["bind", "rw", "nosuid", "nodev"]},
            ],
            "linux": {
                "namespaces": [{"type": kind} for kind in ("mount", "pid", "network", "ipc", "uts", "cgroup")]
            },
        }

    def start_ordinary_container(self, name: str, workspace: Path, expect_fstype: str) -> dict[str, Any]:
        bundle = self.root / f"bundle-{name}"
        bundle.mkdir(parents=True)
        rootfs = self.root / f"rootfs-{name}"
        manifest = self.clone_rootfs(rootfs)
        spec_doc = self.oci_spec(rootfs, workspace, ["/afs-workspace-probe", "idle"], name)
        write_json(bundle / "config.json", spec_doc)
        self.runc(["run", "--detach", "--bundle", bundle, name], self.out / f"runc-{name}", timeout=60)
        state = json.loads(self.runc(["state", name], self.out / f"runc-{name}", timeout=30))
        pid = state["pid"]
        source = os.stat(workspace)
        final = os.stat(f"/proc/{pid}/root/workspace")
        self.check(name + "-source-match", (source.st_dev, source.st_ino) == (final.st_dev, final.st_ino), {
            "source": [source.st_dev, source.st_ino],
            "final": [final.st_dev, final.st_ino],
        })
        identity_text = self.runc(["exec", name, "/afs-workspace-probe", "identity"], self.out / f"runc-{name}", timeout=60)
        observed = json.loads(identity_text)
        native_base.verify_final(observed, {"dev": final.st_dev, "ino": final.st_ino}, {
            "dev": os.stat(f"/proc/{pid}/ns/mnt").st_dev,
            "ino": os.stat(f"/proc/{pid}/ns/mnt").st_ino,
        })
        mountinfo = Path(f"/proc/{pid}/mountinfo").read_text(encoding="utf-8")
        workspace_lines = [line for line in mountinfo.splitlines() if line.split()[4] == "/workspace"]
        self.check(name + "-one-workspace-mount", len(workspace_lines) == 1, workspace_lines)
        if expect_fstype == "fuse":
            self.check(name + "-mount-fuse", " - fuse" in workspace_lines[0], workspace_lines[0])
        else:
            self.check(name + "-mount-ext4", " - ext4 " in workspace_lines[0], workspace_lines[0])
        record = {
            "id": name,
            "state": state,
            "pid": pid,
            "rootfs_manifest": manifest,
            "source": str(workspace),
            "source_object": {"dev": source.st_dev, "ino": source.st_ino},
            "final_identity": observed,
            "mountinfo": mountinfo,
            "expect_fstype": expect_fstype,
        }
        write_json(self.out / f"{name}-container.json", record)
        return record

    def stop_ordinary_container(self, container: dict[str, Any]) -> dict[str, Any]:
        name = container["id"]
        cleanup: dict[str, Any] = {"id": name}
        self.runc(["kill", name, "TERM"], self.out / f"runc-{name}", timeout=30, allowed=(0, 1))
        deadline = time.monotonic() + 10
        state: dict[str, Any] = {}
        while time.monotonic() < deadline:
            state = json.loads(self.runc(["state", name], self.out / f"runc-{name}", timeout=30, allowed=(0, 1)))
            if state.get("status") == "stopped":
                break
            time.sleep(0.1)
        self.check(name + "-stopped", state.get("status") == "stopped", state)
        self.runc(["delete", name], self.out / f"runc-{name}", timeout=30)
        cleanup["stopped"] = True
        cleanup["deleted"] = True
        return cleanup

    def runtime_empty(self, root: Path, label: str) -> bool:
        text = self.runc_at(root, ["list", "--format", "json"], self.out / f"runc-list-{label}", timeout=30)
        value = json.loads(text) if text.strip() else None
        return value in (None, [])

    def collect_text_artifacts(self, product_root: Path, cohort: str) -> None:
        portable = self.out / f"{cohort}-portable"
        for name in ("logs", "run", "control"):
            source = product_root / name
            if not source.exists():
                continue
            for path in source.rglob("*"):
                if not path.is_file() or path.suffix in (".sock", ".key") or os.access(path, os.X_OK):
                    continue
                rel = path.relative_to(product_root)
                if "rootfs" in rel.parts or "prefix" in rel.parts or "package" in rel.parts:
                    continue
                if path.stat().st_size > 4 * 1024 * 1024:
                    continue
                dst = portable / rel
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, dst)

    def exec_ordinary(self, container: dict[str, Any], argv: list[str], name: str) -> dict[str, Any]:
        text = self.runc(["exec", container["id"], *argv], self.out / f"payload-{name}", timeout=900)
        payload = json.loads(text)
        return {"result": payload, "raw_stdout": text}

    def exec_native_payload(self, argv: list[str], name: str) -> dict[str, Any]:
        if self.native_container_id is None:
            raise ValueError("native container id unavailable")
        before = set((self.root / "on/control").glob("command-*.command.json"))
        response = self.native(name, "exec", "--", *argv)
        self.check(name + "-executed", response.get("status") == "Executed", response)
        command_argv = [
            "--root",
            str(self.root / "on/control/runtime-state"),
            "exec",
            self.native_container_id,
            *argv,
        ]
        artifact = newest_stdout_for_argv(self.root / "on/control", command_argv, before)
        payload = artifact["stdout"]
        if not isinstance(payload, dict):
            raise ValueError("native payload stdout must be JSON")
        return {"result": payload, "control_artifact": artifact, "controller_response": response}

    def node_metrics(self) -> dict[str, Any]:
        text = self.run_text(["curl", "-fsS", "http://127.0.0.1:24501/metrics"], self.out / "metrics", timeout=30)
        return {"raw": text, "parsed": parse_prometheus(text), "fuse_request_counts": "NOT_OBSERVED"}

    def run_sample(self, cohort: str, target: str, executor: Any, round_index: int, measured: bool) -> dict[str, Any]:
        suffix = f"{cohort}-{target}-r{round_index:02d}"
        path = f"/workspace/io-{suffix}"
        write = executor(["/io", path, "seq-write", str(DATA_BYTES), str(BLOCK_BYTES), "1", "fsync", str(DATA_BYTES), str(PATTERN_BYTE), "create", "unobserved"], f"{suffix}-write")
        verify_io_result(write["result"], "seq-write", "fsync")
        read = executor(["/io", path, "seq-read", str(DATA_BYTES), str(BLOCK_BYTES), "1", "close", str(DATA_BYTES), str(PATTERN_BYTE), "existing", "unobserved"], f"{suffix}-read")
        verify_io_result(read["result"], "seq-read", "close")
        metadata = executor(["/benchmark", f"/workspace/meta-{suffix}", "absolute", "1"], f"{suffix}-metadata")
        verify_metadata_result(metadata["result"])
        return {"target": target, "measured": measured, "write": write, "read": read, "metadata": metadata}

    def run_cohort(self, cohort: str, native_enabled: bool) -> dict[str, Any]:
        product_root = self.install_product(cohort, native_enabled)
        cleanup: list[dict[str, Any]] = []
        product_identity: dict[str, Any] | None = None
        experiment_container: dict[str, Any] | None = None
        reference_container: dict[str, Any] | None = None
        try:
            self.started = True
            self.ctl(product_root, "start", "all")
            product_identity = self.product_identity(product_root, cohort)
            workspace = product_root / "mount/ownerfs/workspace"
            workspace.mkdir(mode=0o700)
            os.chown(workspace, UID, GID)
            reference_dir = self.root / f"{cohort}-reference-workspace"
            reference_dir.mkdir(mode=0o700)
            os.chown(reference_dir, UID, GID)
            reference_container = self.start_ordinary_container(f"{cohort}-reference", reference_dir, "ext4")

            if native_enabled:
                start = self.native("perf-start", "start", "workspace")
                self.check(cohort + "-native-final", start.get("state") == "FinalVerified", start)
                self.native_container_id = start["container"]
                native_runtime_root = self.root / "on/control/runtime-state"
                state = json.loads(self.runc_at(native_runtime_root, ["state", start["container"]], self.out / "native-state", timeout=30))
                pid = state["pid"]
                ns = os.stat(f"/proc/{pid}/ns/mnt")
                final = os.stat(f"/proc/{pid}/root/workspace")
                observed = newest_stdout_for_argv(
                    self.root / "on/control",
                    [
                        "--root",
                        str(self.root / "on/control/runtime-state"),
                        "exec",
                        start["container"],
                        "/afs-workspace-probe",
                        "identity",
                    ],
                    set(),
                )["stdout"]
                native_base.verify_final(observed, {"dev": final.st_dev, "ino": final.st_ino}, {"dev": ns.st_dev, "ino": ns.st_ino})
                mountinfo = Path(f"/proc/{pid}/mountinfo").read_text(encoding="utf-8")
                lines = [line for line in mountinfo.splitlines() if line.split()[4] == "/workspace"]
                self.check(cohort + "-native-workspace-ext4", len(lines) == 1 and " - ext4 " in lines[0], lines)
                experiment_identity = {
                    "container": state,
                    "final_identity": observed,
                    "mountinfo": mountinfo,
                    "controller_start": start,
                }
                experiment_exec = self.exec_native_payload
            else:
                experiment_container = self.start_ordinary_container(f"{cohort}-experiment", workspace, "fuse")
                experiment_identity = experiment_container
                experiment_exec = lambda argv, name: self.exec_ordinary(experiment_container, argv, name)

            reference_exec = lambda argv, name: self.exec_ordinary(reference_container, argv, name)
            rounds: list[dict[str, Any]] = []
            for index in range(WARMUP_ROUNDS + MEASUREMENT_ROUNDS):
                measured = index >= WARMUP_ROUNDS
                order = ["experiment", "reference"] if index % 2 == 0 else ["reference", "experiment"]
                round_record: dict[str, Any] = {
                    "round": index,
                    "measured": measured,
                    "order": order,
                    "resources_before": {"host": snapshot_host(self.root), "node_metrics": self.node_metrics()},
                    "samples": [],
                }
                for target in order:
                    executor = experiment_exec if target == "experiment" else reference_exec
                    round_record["samples"].append(self.run_sample(cohort, target, executor, index, measured))
                round_record["resources_after"] = {"host": snapshot_host(self.root), "node_metrics": self.node_metrics()}
                validate_pair(round_record)
                rounds.append(round_record)
                write_json(self.out / f"{cohort}-round-{index:02d}.json", round_record)
            return {
                "name": cohort,
                "native_enabled": native_enabled,
                "product_identity": product_identity,
                "experiment_identity": experiment_identity,
                "reference_identity": reference_container,
                "rounds": rounds,
            }
        finally:
            cleanup_errors: list[str] = []
            if native_enabled and (product_root / "control/control.sock").exists():
                try:
                    self.native(cohort + "-finally-stop-native", "stop")
                except Exception as error:
                    cleanup.append({"native_stop_error": repr(error)})
                    cleanup_errors.append(repr(error))
            if experiment_container:
                try:
                    cleanup.append(self.stop_ordinary_container(experiment_container))
                except Exception as error:
                    cleanup.append({"experiment_cleanup_error": repr(error)})
                    cleanup_errors.append(repr(error))
            if reference_container:
                try:
                    cleanup.append(self.stop_ordinary_container(reference_container))
                except Exception as error:
                    cleanup.append({"reference_cleanup_error": repr(error)})
                    cleanup_errors.append(repr(error))
            try:
                if product_root.exists():
                    self.ctl(product_root, "stop", "all")
                    self.check(cohort + "-mount-removed", self.command(["findmnt", "-rn", "--mountpoint", product_root / "mount/ownerfs"], allowed=(1,)) == "", "absent")
                    if product_identity:
                        for name in ("meta", "node"):
                            self.check(cohort + f"-{name}-gone", not Path(f"/proc/{product_identity[name]['pid']}").exists(), product_identity[name])
                    self.check(cohort + "-control-clean", not (product_root / "control/control.sock").exists() and not (product_root / "control/controller.lock").exists(), "absent")
                    self.check(cohort + "-ordinary-runtime-empty", self.runtime_empty(self.runtime_root, cohort + "-ordinary"), "empty")
                    if native_enabled:
                        self.check(cohort + "-native-runtime-empty", self.runtime_empty(product_root / "control/runtime-state", cohort), "empty")
                    cleanup.append({"product_stop": "PASS"})
            except Exception as error:
                cleanup.append({"product_cleanup_error": repr(error)})
                cleanup_errors.append(repr(error))
            finally:
                if product_root.exists():
                    self.collect_text_artifacts(product_root, cohort)
                write_json(self.out / f"{cohort}-cleanup.json", cleanup)
            if cleanup_errors:
                raise RuntimeError("cleanup failed: " + "; ".join(cleanup_errors))

    def run(self) -> int:
        summary = {
            "status": "BLOCKED",
            "scope": "small real-container workspace performance diagnostic; not G2.12/G2.13 PASS",
            "source_commit": self.args.source_commit,
            "driver_sha256": sha(__file__),
            "payloads": {"io": sha(self.args.io_bin), "benchmark": sha(self.args.benchmark_bin)},
            "cohorts": [],
        }
        try:
            self.preflight()
            self.root.mkdir(mode=0o755)
            self.runtime_root.mkdir(mode=0o700)
            summary["status"] = "FAIL"
            for name, enabled in (("off", False), ("on", True)):
                summary["cohorts"].append(self.run_cohort(name, enabled))
            summary["status"] = "DATA_RECORDED"
            summary["claim"] = "diagnostic data only; fuse_request_counts are not observed by current metrics"
            validate_summary(summary)
        except Exception as error:
            if summary["status"] == "DATA_RECORDED":
                summary["status"] = "FAIL"
            summary["error"] = repr(error)
        finally:
            try:
                runtime_list = self.runc(["list", "--format", "json"], self.out / "final-runc", timeout=30, allowed=(0,))
                summary["final_runtime_list"] = json.loads(runtime_list) if runtime_list.strip() else None
                if summary["final_runtime_list"] not in (None, []):
                    summary["status"] = "FAIL"
                    summary["final_runtime_list_error"] = "ordinary runc runtime still has containers"
            except Exception as error:
                summary["final_runtime_list_error"] = repr(error)
                if summary["status"] == "DATA_RECORDED":
                    summary["status"] = "FAIL"
            self.save("checks.json", self.checks)
            write_json(self.out / "result.json", summary)
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 0 if summary["status"] == "DATA_RECORDED" else 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("root", "out", "package", "controller", "template-rootfs", "rootfs-inputs", "io-bin", "benchmark-bin"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("source-commit", "package-sha256", "runtime-sha256", "afs-meta-sha256", "afs-node-sha256"):
        parser.add_argument("--" + name, required=True)
    return parser


def main() -> int:
    return Driver(build_parser().parse_args()).run()


if __name__ == "__main__":
    raise SystemExit(main())

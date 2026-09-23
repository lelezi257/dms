#!/usr/bin/env python3
"""Linux E2E acceptance runner for the Agent-home filesystem preview."""

from __future__ import annotations

import argparse
import errno
import http.client
import json
import os
import platform
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import traceback
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


TOKEN = "acceptance-token-2026"
LOG_TAIL_LINES = 160


class AcceptanceError(RuntimeError):
    pass


@dataclass
class Step:
    name: str
    status: str = "PENDING"
    duration_ms: int = 0
    detail: dict[str, Any] = field(default_factory=dict)
    error: str | None = None


@dataclass
class ManagedProcess:
    name: str
    args: list[str]
    log: Path
    env: dict[str, str]
    process: subprocess.Popen[bytes] | None = None

    def start(self) -> None:
        self.log.parent.mkdir(parents=True, exist_ok=True)
        with self.log.open("ab") as output:
            self.process = subprocess.Popen(
                self.args,
                stdout=output,
                stderr=subprocess.STDOUT,
                env=self.env,
                start_new_session=True,
            )

    @property
    def pid(self) -> int | None:
        if self.process is None:
            return None
        return self.process.pid

    def poll(self) -> int | None:
        if self.process is None:
            return None
        return self.process.poll()

    def terminate(self, timeout: float = 5.0) -> None:
        if self.process is None or self.process.poll() is not None:
            return
        try:
            os.killpg(self.process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                return
            time.sleep(0.05)
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.process.wait(timeout=timeout)


@dataclass
class ReadFailure:
    errno: int | None
    stderr: str


class AcceptanceRun:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.source_root = args.source_root.resolve()
        self.keep = args.keep
        self.failed = False
        self.steps: list[Step] = []
        self.processes: list[ManagedProcess] = []
        self.tmp = Path(args.workdir).resolve() if args.workdir else Path(tempfile.mkdtemp(prefix="dms-home-accept-"))
        self.logs = self.tmp / "logs"
        self.center_rpc = f"127.0.0.1:{free_port()}"
        self.center_http_port = free_port()
        self.center_http = f"127.0.0.1:{self.center_http_port}"
        self.node_a_p2p = f"127.0.0.1:{free_port()}"
        self.node_b_p2p = f"127.0.0.1:{free_port()}"
        self.binary = args.binary.resolve() if args.binary else self.tmp / "target" / "release" / "dms-home"
        self.mount_a = self.tmp / "mount-a"
        self.mount_b = self.tmp / "mount-b"
        self.data_a = self.tmp / "data-a"
        self.data_b = self.tmp / "data-b"
        self.peers_a = self.tmp / "peers-a"
        self.peers_b = self.tmp / "peers-b"
        self.state = self.tmp / "center" / "state.txt"

    def run(self) -> dict[str, Any]:
        started = time.time()
        status = "FAIL"
        failure: str | None = None
        try:
            self.step("preflight", self.preflight)
            if self.args.binary is None:
                self.step("build dms-home", self.build_binary)
            self.step("start center and nodes", self.start_cluster)
            self.step("root mkdir and management lookup", self.verify_root_and_management_api)
            self.step("close-to-open bytes across A and B", self.verify_cross_node_bytes)
            self.step("remote namespace mutations", self.verify_remote_mutations)
            self.step("cross-root rename returns EXDEV", self.verify_cross_root_exdev)
            self.step("center restart preserves root ownership", self.verify_center_restart)
            self.step("home P2P restart and failure boundary", self.verify_home_p2p_restart)
            status = "PASS"
        except Exception as error:  # noqa: BLE001 - report exact acceptance failure.
            self.failed = True
            failure = f"{type(error).__name__}: {error}"
            if self.args.traceback:
                failure = f"{failure}\n{traceback.format_exc()}"
        finally:
            cleanup_errors = self.cleanup()
            if cleanup_errors and status == "PASS":
                status = "FAIL"
                failure = "; ".join(cleanup_errors)
                self.failed = True

        result = {
            "status": status,
            "backend": self.args.backend,
            "started_at": started,
            "duration_ms": int((time.time() - started) * 1000),
            "source_root": str(self.source_root),
            "binary": str(self.binary),
            "workdir": str(self.tmp),
            "center_rpc": self.center_rpc,
            "center_http": f"http://{self.center_http}",
            "nodes": {
                "A": {"mount": str(self.mount_a), "data": str(self.data_a), "p2p": self.node_a_p2p},
                "B": {"mount": str(self.mount_b), "data": str(self.data_b), "p2p": self.node_b_p2p},
            },
            "steps": [step.__dict__ for step in self.steps],
            "failure": failure,
            "logs": self.collect_log_tails() if status == "FAIL" else {},
        }
        if status == "PASS" and not self.keep:
            shutil.rmtree(self.tmp, ignore_errors=True)
            result["workdir_removed"] = True
        else:
            result["workdir_removed"] = False
        return result

    def step(self, name: str, action: Any) -> None:
        step = Step(name=name)
        self.steps.append(step)
        before = time.monotonic()
        try:
            detail = action()
            step.status = "PASS"
            if isinstance(detail, dict):
                step.detail = detail
        except Exception as error:
            step.status = "FAIL"
            step.error = f"{type(error).__name__}: {error}"
            raise
        finally:
            step.duration_ms = int((time.monotonic() - before) * 1000)

    def preflight(self) -> dict[str, Any]:
        if platform.system() != "Linux":
            raise AcceptanceError("Linux is required for real FUSE/NFS acceptance")
        if not self.source_root.joinpath("Cargo.toml").exists():
            raise AcceptanceError(f"source root does not look like a checkout: {self.source_root}")
        if self.args.backend == "nfs":
            if not self.args.nfs_a_endpoint or not self.args.nfs_b_endpoint:
                raise AcceptanceError("--backend nfs requires --nfs-a-endpoint and --nfs-b-endpoint")
            if shutil.which("mount") is None or shutil.which("mountpoint") is None:
                raise AcceptanceError("NFS backend requires mount and mountpoint commands")
        if shutil.which("fusermount3") is None and shutil.which("fusermount") is None and shutil.which("umount") is None:
            raise AcceptanceError("no FUSE unmount helper found")
        for path in [self.mount_a, self.mount_b, self.data_a, self.data_b, self.peers_a, self.peers_b, self.logs]:
            path.mkdir(parents=True, exist_ok=True)
        return {
            "kernel": platform.release(),
            "python": platform.python_version(),
            "workdir": str(self.tmp),
        }

    def build_binary(self) -> dict[str, Any]:
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(self.tmp / "target")
        command = ["cargo", "build", "-p", "dms-home", "--release", "--locked"]
        run_logged(command, cwd=self.source_root, env=env, log=self.logs / "build.log", timeout=self.args.build_timeout)
        if not self.binary.exists():
            raise AcceptanceError(f"cargo finished but binary is missing: {self.binary}")
        return {"command": command, "log": str(self.logs / "build.log")}

    def start_cluster(self) -> dict[str, Any]:
        self.start_center("center.log")
        wait_tcp(self.center_rpc, self.args.start_timeout)
        wait_http_json(self.center_http_port, "/v1/roots", self.args.start_timeout)
        self.start_node("A", "node-a.log")
        self.start_node("B", "node-b.log")
        wait_tcp(self.node_a_p2p, self.args.start_timeout)
        wait_tcp(self.node_b_p2p, self.args.start_timeout)
        wait_mount(self.mount_a, self.args.start_timeout)
        wait_mount(self.mount_b, self.args.start_timeout)
        return {
            "center_pid": self.process_by_name("center").pid,
            "node_a_pid": self.process_by_name("node-A").pid,
            "node_b_pid": self.process_by_name("node-B").pid,
        }

    def start_center(self, log_name: str) -> None:
        env = os.environ.copy()
        env["DMS_HOME_TOKEN"] = TOKEN
        process = ManagedProcess(
            name="center",
            args=[str(self.binary), "center", self.center_rpc, self.center_http, str(self.state)],
            log=self.logs / log_name,
            env=env,
        )
        process.start()
        self.replace_process(process)

    def start_node(self, node: str, log_name: str) -> None:
        if node == "A":
            p2p = self.node_a_p2p
            data = self.data_a
            peers = self.peers_a
            mount = self.mount_a
            nfs = self.args.nfs_a_endpoint or "127.0.0.1:/"
        elif node == "B":
            p2p = self.node_b_p2p
            data = self.data_b
            peers = self.peers_b
            mount = self.mount_b
            nfs = self.args.nfs_b_endpoint or "127.0.0.1:/"
        else:
            raise AssertionError(node)
        env = os.environ.copy()
        env["DMS_HOME_TOKEN"] = TOKEN
        process = ManagedProcess(
            name=f"node-{node}",
            args=[
                str(self.binary),
                "node",
                node,
                self.center_rpc,
                nfs,
                p2p,
                str(data),
                str(peers),
                str(mount),
                self.args.backend,
            ],
            log=self.logs / log_name,
            env=env,
        )
        process.start()
        self.replace_process(process)

    def replace_process(self, process: ManagedProcess) -> None:
        self.processes = [existing for existing in self.processes if existing.name != process.name]
        self.processes.append(process)

    def process_by_name(self, name: str) -> ManagedProcess:
        for process in self.processes:
            if process.name == name:
                return process
        raise AcceptanceError(f"process not found: {name}")

    def verify_root_and_management_api(self) -> dict[str, Any]:
        root = self.mount_a / "job-42"
        root.mkdir()
        assert_equal(self.locate("job-42"), "A", "locate job-42")
        body = http_json(self.center_http_port, "/v1/roots/job-42")
        assert_equal(body["owner"], "A", "management owner")
        assert_equal(body["status"], "active", "management status")
        assert_equal(body["node"]["p2p_endpoint"], self.node_a_p2p, "management p2p endpoint")
        roots = http_json(self.center_http_port, "/v1/roots")
        if not any(row.get("name") == "job-42" and row.get("owner") == "A" for row in roots["roots"]):
            raise AcceptanceError(f"/v1/roots did not include job-42 owned by A: {roots}")
        return {"root": "job-42", "generation": body["generation"]}

    def verify_cross_node_bytes(self) -> dict[str, Any]:
        note_a = self.mount_a / "job-42" / "note.txt"
        note_b = self.mount_b / "job-42" / "note.txt"
        write_fsync_close(note_a, b"hello-from-A-v1")
        assert_equal(note_b.read_bytes(), b"hello-from-A-v1", "B reopen reads A bytes")
        write_fsync_close(note_a, b"rewritten-by-A-v2")
        assert_equal(note_b.read_bytes(), b"rewritten-by-A-v2", "B reopen reads A rewrite")
        return {"path": "/job-42/note.txt", "bytes": len(b"rewritten-by-A-v2")}

    def verify_remote_mutations(self) -> dict[str, Any]:
        subdir = self.mount_b / "job-42" / "logs"
        subdir.mkdir()
        remote_file = subdir / "run.txt"
        write_fsync_close(remote_file, b"remote-create")
        renamed = subdir / "renamed.txt"
        remote_file.rename(renamed)
        assert_equal((self.mount_a / "job-42" / "logs" / "renamed.txt").read_bytes(), b"remote-create", "A sees B rename")
        renamed.unlink()
        if (self.mount_a / "job-42" / "logs" / "renamed.txt").exists():
            raise AcceptanceError("A still sees file after B unlink")
        subdir.rmdir()
        if (self.mount_a / "job-42" / "logs").exists():
            raise AcceptanceError("A still sees directory after B rmdir")
        return {"mutator": "B", "root_owner": "A"}

    def verify_cross_root_exdev(self) -> dict[str, Any]:
        left = self.mount_a / "left-root"
        right = self.mount_a / "right-root"
        left.mkdir()
        right.mkdir()
        source = left / "file.txt"
        write_fsync_close(source, b"cannot-cross")
        try:
            source.rename(right / "file.txt")
        except OSError as error:
            if error.errno != errno.EXDEV:
                raise AcceptanceError(f"cross-root rename returned errno {error.errno}, expected EXDEV")
        else:
            raise AcceptanceError("cross-root rename unexpectedly succeeded")
        source.unlink()
        left.rmdir()
        right.rmdir()
        return {"errno": errno.EXDEV}

    def verify_center_restart(self) -> dict[str, Any]:
        center = self.process_by_name("center")
        center.terminate()
        wait_process_exit(center, self.args.start_timeout)
        self.unmount_peer_nfs()
        self.start_center("center-restarted.log")
        wait_tcp(self.center_rpc, self.args.start_timeout)
        wait_http_json(self.center_http_port, "/v1/roots", self.args.start_timeout)
        assert_equal(self.locate("job-42"), "A", "locate after center restart")
        body = self.wait_management_node("job-42", "A", self.node_a_p2p)
        assert_equal(body["owner"], "A", "management owner after center restart")
        assert_equal(body["status"], "active", "management status after center restart")
        assert_equal(
            (self.mount_b / "job-42" / "note.txt").read_bytes(),
            b"rewritten-by-A-v2",
            "B reopen reads after center restart",
        )
        return {"owner_after_restart": body["owner"], "node_after_restart": body.get("node")}

    def wait_management_node(self, root: str, owner: str, p2p_endpoint: str) -> dict[str, Any]:
        observed: dict[str, Any] = {}

        def probe() -> bool:
            nonlocal observed
            observed = http_json(self.center_http_port, f"/v1/roots/{root}")
            node = observed.get("node")
            return (
                observed.get("owner") == owner
                and isinstance(node, dict)
                and node.get("p2p_endpoint") == p2p_endpoint
            )

        wait_for(probe, self.args.start_timeout)
        return observed

    def verify_home_p2p_restart(self) -> dict[str, Any]:
        if self.args.backend != "p2p":
            return {"skipped": "P2P failure semantics are only checked for --backend p2p"}
        service_file = self.mount_a / "job-42" / "service.txt"
        write_fsync_close(service_file, b"before-home-restart")
        assert_equal((self.mount_b / "job-42" / "service.txt").read_bytes(), b"before-home-restart", "B reads before A restart")

        node_a = self.process_by_name("node-A")
        node_a.terminate()
        wait_process_exit(node_a, self.args.start_timeout)
        unmount(self.mount_a)
        failure = read_should_fail(self.mount_b / "job-42" / "service.txt", self.args.command_timeout)

        self.start_node("A", "node-a-restarted.log")
        wait_tcp(self.node_a_p2p, self.args.start_timeout)
        wait_mount(self.mount_a, self.args.start_timeout)
        wait_for(
            lambda: read_bytes_bounded(
                self.mount_b / "job-42" / "service.txt",
                self.args.command_timeout,
            )
            == b"before-home-restart",
            self.args.restart_timeout,
        )
        return {"failure_errno": failure.errno, "restart_read": "ok"}

    def locate(self, root: str) -> str:
        completed = subprocess.run(
            [str(self.binary), "locate", root, self.center_rpc],
            cwd=self.source_root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=self.args.command_timeout,
            check=False,
        )
        if completed.returncode != 0:
            raise AcceptanceError(f"locate failed rc={completed.returncode}: {completed.stderr.strip()}")
        return completed.stdout.strip()

    def cleanup(self) -> list[str]:
        errors: list[str] = []
        for process in reversed(self.processes):
            try:
                process.terminate()
            except Exception as error:  # noqa: BLE001
                errors.append(f"terminate {process.name}: {error}")
        for path in self.all_mounts():
            try:
                unmount(path)
            except Exception as error:  # noqa: BLE001
                errors.append(f"final unmount {path}: {error}")
        return errors

    def all_mounts(self) -> list[Path]:
        mounts = [self.mount_b, self.mount_a]
        for peers in [self.peers_a, self.peers_b]:
            if peers.exists():
                mounts.extend(sorted([path for path in peers.glob("*") if path.is_dir()], reverse=True))
        return mounts

    def unmount_peer_nfs(self) -> None:
        if self.args.backend == "nfs":
            for peers in [self.peers_a, self.peers_b]:
                if peers.exists():
                    for path in peers.glob("*"):
                        if path.is_dir():
                            unmount(path)

    def collect_log_tails(self) -> dict[str, str]:
        tails: dict[str, str] = {}
        if not self.logs.exists():
            return tails
        for log in sorted(self.logs.glob("*.log")):
            tails[log.name] = tail(log, LOG_TAIL_LINES)
        return tails


def parse_args(argv: list[str]) -> argparse.Namespace:
    script = Path(__file__).resolve()
    default_source_root = script.parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, default=default_source_root)
    parser.add_argument("--binary", type=Path, help="Existing dms-home binary. When omitted, cargo builds one in the run workdir.")
    parser.add_argument("--backend", choices=["p2p", "nfs"], default="p2p")
    parser.add_argument("--nfs-a-endpoint", help="NFS endpoint advertised by node A, for explicit --backend nfs runs.")
    parser.add_argument("--nfs-b-endpoint", help="NFS endpoint advertised by node B, for explicit --backend nfs runs.")
    parser.add_argument("--workdir", type=Path, help="Run directory. Defaults to a new /tmp/dms-home-accept-* directory.")
    parser.add_argument("--keep", action="store_true", help="Keep workdir even when the run passes.")
    parser.add_argument("--traceback", action="store_true", help="Include Python traceback in the JSON failure field.")
    parser.add_argument("--start-timeout", type=float, default=20.0)
    parser.add_argument("--restart-timeout", type=float, default=35.0)
    parser.add_argument("--command-timeout", type=float, default=5.0)
    parser.add_argument("--build-timeout", type=float, default=300.0)
    return parser.parse_args(argv)


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_tcp(address: str, timeout: float) -> None:
    host, port_text = address.rsplit(":", 1)
    port = int(port_text)
    def probe() -> bool:
        with socket.create_connection((host, port), timeout=0.25):
            return True
    wait_for(probe, timeout)


def wait_http_json(port: int, path: str, timeout: float) -> None:
    def probe() -> bool:
        body = http_json(port, path)
        return isinstance(body, dict)
    wait_for(probe, timeout)


def wait_mount(path: Path, timeout: float) -> None:
    def probe() -> bool:
        if is_mount(path):
            list(path.iterdir())
            return True
        return False
    wait_for(probe, timeout)


def wait_process_exit(process: ManagedProcess, timeout: float) -> None:
    wait_for(lambda: process.poll() is not None, timeout)


def wait_for(probe: Any, timeout: float, interval: float = 0.1) -> None:
    deadline = time.monotonic() + timeout
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            if probe():
                return
        except Exception as error:  # noqa: BLE001 - readiness probes retry.
            last_error = error
        time.sleep(interval)
    if last_error is not None:
        raise AcceptanceError(f"timed out after {timeout:.1f}s; last error: {last_error}")
    raise AcceptanceError(f"timed out after {timeout:.1f}s")


def http_json(port: int, path: str) -> dict[str, Any]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
    try:
        connection.request("GET", path)
        response = connection.getresponse()
        data = response.read()
        if response.status != 200:
            raise AcceptanceError(f"HTTP {path} returned {response.status}: {data!r}")
        return json.loads(data.decode("utf-8"))
    finally:
        connection.close()


def run_logged(command: list[str], cwd: Path, env: dict[str, str], log: Path, timeout: float) -> None:
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("wb") as output:
        completed = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            stdout=output,
            stderr=subprocess.STDOUT,
            timeout=timeout,
            check=False,
        )
    if completed.returncode != 0:
        raise AcceptanceError(f"command failed rc={completed.returncode}: {' '.join(command)}; log={log}")


def write_fsync_close(path: Path, data: bytes) -> None:
    with path.open("wb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())


def read_should_fail(path: Path, timeout: float) -> ReadFailure:
    command = [
        sys.executable,
        "-c",
        (
            "import errno, pathlib, sys\n"
            "p = pathlib.Path(sys.argv[1])\n"
            "try:\n"
            "    p.read_bytes()\n"
            "except OSError as e:\n"
            "    print(e.errno if e.errno is not None else -1)\n"
            "    raise SystemExit(7)\n"
            "raise SystemExit(0)\n"
        ),
        str(path),
    ]
    try:
        completed = subprocess.run(
            command,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise AcceptanceError(f"read did not return within {timeout:.1f}s while home service was stopped: {error}") from error
    if completed.returncode == 7:
        stdout = completed.stdout.strip()
        parsed_errno = int(stdout) if stdout.lstrip("-").isdigit() and int(stdout) >= 0 else None
        return ReadFailure(errno=parsed_errno, stderr=completed.stderr.strip())
    if completed.returncode == 0:
        raise AcceptanceError(f"read unexpectedly succeeded while home service was stopped: {path}")
    return ReadFailure(errno=None, stderr=completed.stderr.strip())


def read_bytes_bounded(path: Path, timeout: float) -> bytes:
    command = [
        sys.executable,
        "-c",
        (
            "import pathlib, sys\n"
            "sys.stdout.buffer.write(pathlib.Path(sys.argv[1]).read_bytes())\n"
        ),
        str(path),
    ]
    completed = subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    if completed.returncode != 0:
        raise AcceptanceError(
            f"bounded read failed rc={completed.returncode}: {completed.stderr.decode(errors='replace').strip()}"
        )
    return completed.stdout


def assert_equal(actual: Any, expected: Any, label: str) -> None:
    if actual != expected:
        raise AcceptanceError(f"{label}: got {actual!r}, expected {expected!r}")


def is_mount(path: Path) -> bool:
    if shutil.which("mountpoint") is not None:
        completed = subprocess.run(
            ["mountpoint", "-q", str(path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if completed.returncode == 0:
            return True
    target = os.path.abspath(os.fspath(path))
    try:
        mountinfo_rows = Path("/proc/self/mountinfo").read_text(errors="replace").splitlines()
    except OSError:
        mountinfo_rows = []
    for line in mountinfo_rows:
        fields = line.split()
        if len(fields) > 4 and decode_mountinfo_path(fields[4]) == target:
            return True
    try:
        path_stat = path.stat()
        parent_stat = path.parent.stat()
    except OSError:
        return False
    return path_stat.st_dev != parent_stat.st_dev


def decode_mountinfo_path(value: str) -> str:
    decoded = bytearray()
    index = 0
    while index < len(value):
        if (
            value[index] == "\\"
            and index + 3 < len(value)
            and value[index + 1 : index + 4].isdigit()
        ):
            decoded.append(int(value[index + 1 : index + 4], 8))
            index += 4
        else:
            decoded.extend(value[index].encode())
            index += 1
    return decoded.decode(errors="replace")


def unmount(path: Path) -> None:
    if not is_mount(path):
        return
    helpers = [
        ["fusermount3", "-u", "-z", str(path)],
        ["fusermount", "-u", "-z", str(path)],
        ["umount", "-l", str(path)],
    ]
    errors: list[str] = []
    for command in helpers:
        if shutil.which(command[0]) is None:
            continue
        completed = subprocess.run(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
            timeout=10,
        )
        if completed.returncode == 0 or not is_mount(path):
            return
        errors.append(f"{' '.join(command)} rc={completed.returncode}: {completed.stderr.strip()}")
    raise AcceptanceError("; ".join(errors) or f"no unmount helper could unmount {path}")


def tail(path: Path, lines: int) -> str:
    try:
        content = path.read_text(errors="replace").splitlines()
    except OSError as error:
        return f"<unable to read log: {error}>"
    return "\n".join(content[-lines:])


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    runner = AcceptanceRun(args)
    result = runner.run()
    print(json.dumps(result, indent=2, sort_keys=True), flush=True)
    print(result["status"], flush=True)
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

#!/usr/bin/env python3
"""Run the DFS local R=1 create/write/fsync/reopen/read FUSE slice."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import signal
import socket
import subprocess
import tempfile
import time


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_port(port: int, process: subprocess.Popen[bytes], timeout: float = 15.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"process exited before port {port} became ready")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.1)
    raise RuntimeError(f"port {port} did not become ready")


def wait_mount(path: pathlib.Path, process: subprocess.Popen[bytes], timeout: float = 15.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("afs-node exited before the DFS mount became ready")
        if subprocess.run(
            ["mountpoint", "-q", str(path)], check=False, capture_output=True
        ).returncode == 0:
            return
        time.sleep(0.1)
    raise RuntimeError("DFS FUSE mount did not become ready")


def stop(process: subprocess.Popen[bytes] | None) -> None:
    if process is None or process.poll() is not None:
        return
    process.send_signal(signal.SIGINT)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def tail(path: pathlib.Path, limit: int = 4000) -> str:
    if not path.exists():
        return ""
    return path.read_text(errors="replace")[-limit:]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=pathlib.Path, required=True)
    parser.add_argument("--work-dir", type=pathlib.Path)
    args = parser.parse_args()

    owned_work_dir = args.work_dir is None
    work_dir = args.work_dir or pathlib.Path(tempfile.mkdtemp(prefix="afs-dfs-r1-"))
    work_dir.mkdir(parents=True, exist_ok=True)
    mount = work_dir / "mnt"
    mount.mkdir()
    meta_log = work_dir / "meta.log"
    node_log = work_dir / "node.log"
    meta_port, meta_rest, node_port, node_rest = (free_port() for _ in range(4))
    meta: subprocess.Popen[bytes] | None = None
    node: subprocess.Popen[bytes] | None = None
    result: dict[str, object] = {"work_dir": str(work_dir), "passed": False}

    try:
        with meta_log.open("wb") as meta_output, node_log.open("wb") as node_output:
            meta = subprocess.Popen(
                [
                    str(args.bin_dir / "afs-meta"),
                    "--id",
                    "dfs-meta-e2e",
                    "--fs",
                    "dfs",
                    "--meta-store",
                    "local-file",
                    "--data-dir",
                    str(work_dir / "meta"),
                    "--uds-path",
                    str(work_dir / "meta.sock"),
                    "--grpc-listen",
                    f"127.0.0.1:{meta_port}",
                    "--rest-listen",
                    f"127.0.0.1:{meta_rest}",
                ],
                stdout=meta_output,
                stderr=subprocess.STDOUT,
            )
            wait_port(meta_port, meta)
            node = subprocess.Popen(
                [
                    str(args.bin_dir / "afs-node"),
                    "--id",
                    "dfs-node-e2e",
                    "--fs",
                    "dfs",
                    "--meta-endpoint",
                    f"http://127.0.0.1:{meta_port}",
                    "--advertise-endpoint",
                    f"http://127.0.0.1:{node_port}",
                    "--data-dir",
                    str(work_dir / "node"),
                    "--uds-path",
                    str(work_dir / "node.sock"),
                    "--dfs-mount",
                    str(mount),
                    "--grpc-listen",
                    f"127.0.0.1:{node_port}",
                    "--rest-listen",
                    f"127.0.0.1:{node_rest}",
                ],
                stdout=node_output,
                stderr=subprocess.STDOUT,
            )
            wait_port(node_port, node)
            wait_mount(mount, node)

            path = mount / "hello.txt"
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o640)
            try:
                os.write(fd, b"hello-")
                os.write(fd, b"dfs-r1")

                # A second handle must observe the inode-level dirty overlay
                # before any FileVersion is committed.
                reader = os.open(path, os.O_RDONLY)
                try:
                    dirty_payload = os.pread(reader, 64, 0)
                finally:
                    os.close(reader)
                if dirty_payload != b"hello-dfs-r1":
                    raise RuntimeError(
                        f"dirty overlay was not shared across handles: {dirty_payload!r}"
                    )

                os.fdatasync(fd)
                first_chunks = sorted((work_dir / "node" / "dfs" / "chunks").iterdir())
                if len(first_chunks) != 1 or first_chunks[0].read_bytes() != dirty_payload:
                    raise RuntimeError("fdatasync did not commit the first immutable Chunk")

                os.pwrite(fd, b"DFS", 6)
                os.fsync(fd)
            finally:
                os.close(fd)

            with path.open("rb") as reopened:
                payload = reopened.read()
            stat = path.stat()
            chunks = sorted((work_dir / "node" / "dfs" / "chunks").iterdir())
            if payload != b"hello-DFS-r1":
                raise RuntimeError(f"reopened payload mismatch: {payload!r}")
            if stat.st_size != len(payload):
                raise RuntimeError(f"reopened size mismatch: {stat.st_size}")
            if len(chunks) != 2:
                raise RuntimeError(f"expected two immutable versions, found {len(chunks)} chunks")
            if payload not in [chunk.read_bytes() for chunk in chunks]:
                raise RuntimeError("latest immutable local Chunk does not match committed content")

            result.update(
                passed=True,
                payload=payload.decode(),
                file_size=stat.st_size,
                chunk_ids=[chunk.name for chunk in chunks],
                chunk_bytes=[chunk.stat().st_size for chunk in chunks],
                dirty_overlay_visible=True,
                fdatasync_then_fsync=True,
            )
    except Exception as error:  # noqa: BLE001 - emit bounded diagnostics for the E2E runner.
        result.update(error=str(error), meta_log=tail(meta_log), node_log=tail(node_log))
    finally:
        if subprocess.run(
            ["mountpoint", "-q", str(mount)], check=False, capture_output=True
        ).returncode == 0:
            fusermount = shutil.which("fusermount3") or shutil.which("fusermount")
            if fusermount:
                subprocess.run([fusermount, "-uz", str(mount)], check=False)
        stop(node)
        stop(meta)

    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    if result["passed"] and owned_work_dir:
        shutil.rmtree(work_dir)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

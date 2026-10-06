#!/usr/bin/env python3
"""Small DFS one-writer/many-reader diagnostic helper.

The parent orchestrates writer on one node and readers on other nodes. This tool
only prepares or reads a fixed DFS payload and records evidence; it does not
manage cluster lifecycle and does not qualify DFS against 3FS.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import time
from typing import Any

DATA_BYTES = 64 * 1024 * 1024
BLOCK_BYTES = 1024 * 1024
CONCURRENCY = 1
PATTERN_BYTE = 97
IO_TOOL_SHA256 = "70ac97c7634d406a177a74c783d446b62a2014ba198586132882a1d9228e55e8"
PRODUCT_SOURCE_COMMIT = "6d51aeb45c1ed8669d80f612b3817e6d1bdabe04"
COMPILER_INPUT_MAP = "66dbbe3e0071fcec1efc9cf370c709a99f025d39acd4f37ba57582c874429304"
WARMUP_ROUNDS = 1
MEASUREMENT_ROUNDS = 5
PAYLOAD_NAME = "payload64m.bin"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def expected_payload_sha() -> str:
    digest = hashlib.sha256()
    block = bytes([PATTERN_BYTE]) * BLOCK_BYTES
    for _ in range(DATA_BYTES // BLOCK_BYTES):
        digest.update(block)
    return digest.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def require_absolute_path(value: str, name: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise ValueError(f"{name} must be absolute: {value}")
    return path


def require_existing_directory(path: Path, name: str) -> None:
    if path.is_symlink() or not path.is_dir():
        raise ValueError(f"{name} must be an existing non-symlink directory: {path}")


def validate_output_path(path: Path) -> None:
    if path.is_symlink() or path.exists():
        raise ValueError(f"output must be fresh and non-symlink: {path}")
    if not path.parent.is_dir():
        raise ValueError(f"output parent must exist: {path.parent}")


def stat_identity(path: Path) -> dict[str, Any]:
    st = path.stat()
    return {"path": str(path), "device": st.st_dev, "inode": st.st_ino, "mode": stat.S_IMODE(st.st_mode), "uid": st.st_uid, "gid": st.st_gid}


def find_mount(path: Path) -> dict[str, Any]:
    result = subprocess.run(
        ["findmnt", "-J", "-T", str(path), "-o", "TARGET,SOURCE,FSTYPE,OPTIONS,ID"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    filesystems = json.loads(result.stdout).get("filesystems") or []
    if not filesystems:
        raise ValueError(f"no mount found for {path}")
    return filesystems[0]


def require_dfs_mount(mount: dict[str, Any], dfs_root: Path) -> None:
    fstype = str(mount.get("fstype", ""))
    source = str(mount.get("source", ""))
    target = str(mount.get("target", ""))
    if not (fstype == "fuse" or fstype.startswith("fuse.")):
        raise ValueError(f"DFS root must be on FUSE, got {fstype}")
    if source != "afs-dfs" or target != str(dfs_root):
        raise ValueError(f"DFS mount must be exact source=afs-dfs target={dfs_root}: {mount}")


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def verify_linux_aarch64_root() -> dict[str, Any]:
    identity = {"system": platform.system(), "machine": platform.machine(), "euid": os.geteuid()}
    if identity != {"system": "Linux", "machine": "aarch64", "euid": 0}:
        raise RuntimeError(f"requires Linux aarch64 root: {identity}")
    return identity


def validate_io_tool(path: Path) -> dict[str, Any]:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"io tool must be an existing non-symlink file: {path}")
    digest = sha256_file(path)
    if digest != IO_TOOL_SHA256:
        raise ValueError(f"io tool sha256 mismatch: {digest}")
    return {"path": str(path), "sha256": digest}


def validate_io_result(record: dict[str, Any], operation: str, barrier: str) -> None:
    expected = {
        "operation": operation,
        "file_bytes": DATA_BYTES,
        "io_bytes": DATA_BYTES,
        "block_bytes": BLOCK_BYTES,
        "concurrency": CONCURRENCY,
        "barrier": barrier,
        "pattern_byte": PATTERN_BYTE,
        "operations": DATA_BYTES // BLOCK_BYTES,
        "cache_requested": "unobserved",
        "content_ok": True,
    }
    for key, value in expected.items():
        if record.get(key) != value:
            raise ValueError(f"unexpected io result {key}: {record.get(key)!r}")
    if record.get("residency_observed") is not False:
        raise ValueError("cache residency must remain unobserved")
    if not isinstance(record.get("wall_ns"), int) or record["wall_ns"] <= 0:
        raise ValueError("io result missing positive wall_ns")


def run_io(io_tool: Path, payload: Path, operation: str, barrier: str, mode: str) -> dict[str, Any]:
    argv = [
        str(io_tool), str(payload), operation, str(DATA_BYTES), str(BLOCK_BYTES), str(CONCURRENCY),
        barrier, str(DATA_BYTES), str(PATTERN_BYTE), mode, "unobserved",
    ]
    began = time.monotonic_ns()
    completed = subprocess.run(argv, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    record: dict[str, Any] = {
        "argv": argv,
        "rc": completed.returncode,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
        "elapsed_ns": time.monotonic_ns() - began,
        "status": "FAIL",
    }
    try:
        payload_json = json.loads(completed.stdout)
        record["result"] = payload_json
        if completed.returncode != 0:
            raise ValueError(f"io exited {completed.returncode}")
        validate_io_result(payload_json, operation, barrier)
        record["verify"] = {"status": "PASS"}
        record["status"] = "PASS"
    except Exception as error:
        record["verify"] = {"status": "FAIL", "error": repr(error)}
    return record


def verify_payload(path: Path, expected_sha256: str) -> dict[str, Any]:
    st = path.stat()
    digest = sha256_file(path)
    status = "PASS" if st.st_size == DATA_BYTES and digest == expected_sha256 else "FAIL"
    return {"status": status, "path": str(path), "bytes": st.st_size, "sha256": digest, "expected_sha256": expected_sha256}


def writer_directory_name() -> str:
    return f"dfs-manyread-small-{int(time.time_ns())}-{os.getpid()}"


def writer(args: argparse.Namespace) -> dict[str, Any]:
    result: dict[str, Any] = {"role": "writer", "status": "BLOCKED", "scope": "small DFS one-write/many-read diagnostic"}
    output: Path | None = None
    output_created = False
    try:
        dfs_root = require_absolute_path(args.dfs_root, "dfs-root")
        io_tool = require_absolute_path(args.io_tool, "io-tool")
        output = require_absolute_path(args.output, "output")
        validate_output_path(output)
        output.mkdir(mode=0o700)
        output_created = True
        platform_identity = verify_linux_aarch64_root()
        require_existing_directory(dfs_root, "dfs-root")
        mount = find_mount(dfs_root)
        require_dfs_mount(mount, dfs_root)
        io_identity = validate_io_tool(io_tool)
        sample_dir = dfs_root / writer_directory_name()
        payload = sample_dir / PAYLOAD_NAME
        if sample_dir.exists() or payload.exists():
            raise FileExistsError(str(sample_dir))
        sample_dir.mkdir(mode=0o700)
        fsync_directory(dfs_root)
        if payload.exists():
            raise FileExistsError(str(payload))
        write_record = run_io(io_tool, payload, "seq-write", "fdatasync", "create")
        write_json(output / "write-command.json", write_record)
        fsync_directory(sample_dir)
        expected_sha = expected_payload_sha()
        content = verify_payload(payload, expected_sha)
        result.update({
            "status": "DATA_RECORDED" if write_record.get("status") == "PASS" and content.get("status") == "PASS" else "FAIL",
            "product_source_commit": PRODUCT_SOURCE_COMMIT,
            "source6d": PRODUCT_SOURCE_COMMIT[:7],
            "compiler_input_map": COMPILER_INPUT_MAP,
            "map66": COMPILER_INPUT_MAP[:8],
            "io_tool": io_identity,
            "fs": {"dfs_root": stat_identity(dfs_root), "sample_dir": stat_identity(sample_dir), "payload": stat_identity(payload)},
            "mount": mount,
            "platform": platform_identity,
            "payload": {"relative_dir": sample_dir.name, "name": PAYLOAD_NAME, "bytes": DATA_BYTES, "pattern_byte": PATTERN_BYTE, "sha256": expected_sha},
            "write": write_record,
            "content_verify": content,
            "dir_fsync": True,
            "parent_dir_fsync": True,
        })
    except Exception as error:
        result["error"] = repr(error)
    finally:
        if output_created and output is not None and output.exists() and output.is_dir():
            write_json(output / "confirmed.json", result)
            write_json(output / "summary.json", result)
    return result


def validate_manifest_shape(manifest: dict[str, Any]) -> dict[str, Any]:
    if manifest.get("status") != "DATA_RECORDED":
        raise ValueError("writer manifest is not DATA_RECORDED")
    if manifest.get("product_source_commit") != PRODUCT_SOURCE_COMMIT:
        raise ValueError("writer manifest full source identity mismatch")
    if manifest.get("compiler_input_map") != COMPILER_INPUT_MAP:
        raise ValueError("writer manifest full compiler input map mismatch")
    if manifest.get("source6d") != PRODUCT_SOURCE_COMMIT[:7] or manifest.get("map66") != COMPILER_INPUT_MAP[:8]:
        raise ValueError("writer manifest display source/map identity mismatch")
    io_tool = manifest.get("io_tool")
    if not isinstance(io_tool, dict) or io_tool.get("sha256") != IO_TOOL_SHA256:
        raise ValueError("writer manifest io tool identity mismatch")
    payload = manifest.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("writer manifest missing payload")
    if payload.get("name") != PAYLOAD_NAME or payload.get("bytes") != DATA_BYTES or payload.get("pattern_byte") != PATTERN_BYTE:
        raise ValueError("writer manifest payload shape mismatch")
    rel_dir = payload.get("relative_dir")
    digest = payload.get("sha256")
    if not isinstance(rel_dir, str) or rel_dir.startswith("/") or "/" in rel_dir or rel_dir in ("", ".", ".."):
        raise ValueError("writer manifest relative_dir must be one safe path component")
    if digest != expected_payload_sha():
        raise ValueError("writer manifest payload sha mismatch")
    write = manifest.get("write")
    if not isinstance(write, dict) or write.get("rc") != 0 or write.get("status") != "PASS":
        raise ValueError("writer manifest write command did not pass")
    result = write.get("result")
    if not isinstance(result, dict):
        raise ValueError("writer manifest missing write C result")
    validate_io_result(result, "seq-write", "fdatasync")
    verify = write.get("verify")
    if not isinstance(verify, dict) or verify.get("status") != "PASS":
        raise ValueError("writer manifest write verify did not pass")
    content = manifest.get("content_verify")
    if not isinstance(content, dict) or content.get("status") != "PASS" or content.get("bytes") != DATA_BYTES or content.get("sha256") != digest:
        raise ValueError("writer manifest content verification mismatch")
    if manifest.get("dir_fsync") is not True or manifest.get("parent_dir_fsync") is not True:
        raise ValueError("writer manifest missing directory fsync proof")
    return payload


def read_status(rounds: list[dict[str, Any]]) -> str:
    if len(rounds) != WARMUP_ROUNDS + MEASUREMENT_ROUNDS:
        return "FAIL"
    for round_record in rounds:
        samples = round_record.get("samples")
        if not isinstance(samples, list) or len(samples) != 1:
            return "FAIL"
        sample = samples[0]
        if sample.get("status") != "PASS" or sample.get("verify", {}).get("status") != "PASS":
            return "FAIL"
    return "DATA_RECORDED"


def reader_rounds(io_tool: Path, payload: Path, output: Path) -> list[dict[str, Any]]:
    rounds: list[dict[str, Any]] = []
    for index in range(WARMUP_ROUNDS + MEASUREMENT_ROUNDS):
        sample = run_io(io_tool, payload, "seq-read", "close", "existing")
        sample["round"] = index
        sample["measured"] = index >= WARMUP_ROUNDS
        sample_path = output / "read-samples" / f"round-{index:02d}.json"
        write_json(sample_path, sample)
        sample["artifact"] = str(sample_path)
        round_record = {"round": index, "measured": index >= WARMUP_ROUNDS, "samples": [sample]}
        write_json(output / f"read-round-{index:02d}.json", round_record)
        rounds.append(round_record)
    return rounds


def reader(args: argparse.Namespace) -> dict[str, Any]:
    result: dict[str, Any] = {"role": "reader", "status": "BLOCKED", "scope": "small DFS many-reader diagnostic; parent owns concurrency timing"}
    output: Path | None = None
    output_created = False
    try:
        dfs_root = require_absolute_path(args.dfs_root, "dfs-root")
        io_tool = require_absolute_path(args.io_tool, "io-tool")
        manifest_path = require_absolute_path(args.manifest, "manifest")
        output = require_absolute_path(args.output, "output")
        validate_output_path(output)
        output.mkdir(mode=0o700)
        output_created = True
        platform_identity = verify_linux_aarch64_root()
        require_existing_directory(dfs_root, "dfs-root")
        if manifest_path.is_symlink() or not manifest_path.is_file():
            raise ValueError(f"manifest must be an existing non-symlink file: {manifest_path}")
        manifest = read_json(manifest_path)
        payload_shape = validate_manifest_shape(manifest)
        mount = find_mount(dfs_root)
        require_dfs_mount(mount, dfs_root)
        io_identity = validate_io_tool(io_tool)
        payload = dfs_root / payload_shape["relative_dir"] / PAYLOAD_NAME
        content = verify_payload(payload, payload_shape["sha256"])
        write_json(output / "manifest-precheck.json", {"manifest": str(manifest_path), "content_verify": content})
        result.update({
            "platform": platform_identity,
            "manifest": {"path": str(manifest_path), "payload": payload_shape, "product_source_commit": manifest.get("product_source_commit"), "compiler_input_map": manifest.get("compiler_input_map"), "source6d": manifest.get("source6d"), "map66": manifest.get("map66")},
            "io_tool": io_identity,
            "fs": {"dfs_root": stat_identity(dfs_root), "payload": stat_identity(payload)},
            "mount": mount,
            "content_verify": content,
        })
        if content.get("status") != "PASS":
            raise ValueError("reader payload precheck failed")
        rounds = reader_rounds(io_tool, payload, output)
        result["read_rounds"] = rounds
        write_json(output / "read-rounds.json", rounds)
        result["status"] = read_status(rounds)
    except Exception as error:
        result["error"] = repr(error)
    finally:
        if output_created and output is not None and output.exists() and output.is_dir():
            write_json(output / "summary.json", result)
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="role", required=True)
    for name in ("writer", "reader"):
        item = sub.add_parser(name)
        item.add_argument("--dfs-root", required=True)
        item.add_argument("--io-tool", required=True)
        item.add_argument("--output", required=True)
    sub.choices["reader"].add_argument("--manifest", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    result = writer(args) if args.role == "writer" else reader(args)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result.get("status") == "DATA_RECORDED" else 1


if __name__ == "__main__":
    raise SystemExit(main())

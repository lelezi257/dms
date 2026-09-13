#!/usr/bin/env python3
"""生成 0.1.0 候选发布的机器可读总清单。

本脚本只做校验和汇总，不构建、不上传、不创建 tag。它把几个已经生成
的候选制品串成一份 JSON，方便后续验收、归档或 GitHub Release 草稿引用：

* Rust SDK `.crate`
* Go SDK module proxy 目录（开发预览）
* Server tar 包及 companion `.sha256`
* 可选的 juicefs-dms tar 包及 companion `.sha256`
* 第三方依赖清单
* 可选的隔离消费者验收结果
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path


SOURCE_ROOT = Path(__file__).resolve().parents[2]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def rel(path: Path, root: Path) -> str:
    resolved = path.resolve()
    try:
        return resolved.relative_to(root.resolve()).as_posix()
    except ValueError:
        return str(resolved)


def workspace_version(source_root: Path = SOURCE_ROOT) -> str:
    cargo = source_root / "Cargo.toml"
    in_workspace_package = False
    for line in cargo.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped == "[workspace.package]":
            in_workspace_package = True
            continue
        if stripped.startswith("[") and stripped.endswith("]"):
            in_workspace_package = False
        if in_workspace_package and stripped.startswith("version"):
            match = re.search(r'"([^"]+)"', stripped)
            if match:
                return match.group(1)
    raise ValueError(f"无法从 {cargo} 读取 workspace.package.version")


def source_commit(source_root: Path = SOURCE_ROOT) -> str | None:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=source_root,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def require_file(path: Path, label: str) -> Path:
    if not path.is_file():
        raise ValueError(f"{label} 不存在或不是普通文件：{path}")
    return path


def artifact_file_record(kind: str, path: Path, source_root: Path = SOURCE_ROOT) -> dict:
    path = require_file(path, kind)
    return {
        "kind": kind,
        "path": rel(path, source_root),
        "bytes": path.stat().st_size,
        "sha256": sha256_file(path),
    }


def rust_sdk_record(crate: Path, version: str, source_root: Path = SOURCE_ROOT) -> dict:
    record = artifact_file_record("rust-sdk-crate", crate, source_root)
    expected_name = f"dms-client-{version}.crate"
    if crate.name != expected_name:
        raise ValueError(f"Rust SDK crate 文件名应为 {expected_name}，实际为 {crate.name}")
    record["package"] = "dms-client"
    record["version"] = version
    record["publish_target"] = "cargo-registry"
    return record


def server_record(archive: Path, version: str, source_root: Path = SOURCE_ROOT) -> dict:
    record = artifact_file_record("server-archive", archive, source_root)
    if not archive.name.startswith(f"dms-server-{version}-linux-") or not archive.name.endswith(".tar.gz"):
        raise ValueError(f"Server 包名必须匹配 dms-server-{version}-linux-*.tar.gz：{archive.name}")
    checksum_file = archive.with_name(archive.name + ".sha256")
    if checksum_file.is_file():
        text = checksum_file.read_text(encoding="utf-8").strip().split()
        if not text or text[0] != record["sha256"]:
            raise ValueError(f"Server companion sha256 与归档不一致：{checksum_file}")
        record["companion_sha256"] = rel(checksum_file, source_root)
    else:
        record["companion_sha256"] = None
    record["package"] = "dms-server"
    record["version"] = version
    record["publish_target"] = "binary-tarball"
    return record


def companion_sha256(record: dict, archive: Path, source_root: Path) -> None:
    checksum_file = archive.with_name(archive.name + ".sha256")
    if checksum_file.is_file():
        text = checksum_file.read_text(encoding="utf-8").strip().split()
        if not text or text[0] != record["sha256"]:
            raise ValueError(f"companion sha256 与归档不一致：{checksum_file}")
        record["companion_sha256"] = rel(checksum_file, source_root)
    else:
        record["companion_sha256"] = None


def juicefs_dms_record(archive: Path, version: str, source_root: Path = SOURCE_ROOT) -> dict:
    record = artifact_file_record("juicefs-dms-archive", archive, source_root)
    expected_prefix = f"juicefs-dms-{version}-rc-linux-"
    if not archive.name.startswith(expected_prefix) or not archive.name.endswith(".tar.gz"):
        raise ValueError(f"juicefs-dms 包名必须匹配 {expected_prefix}*.tar.gz：{archive.name}")
    companion_sha256(record, archive, source_root)
    record["package"] = "juicefs-dms"
    record["version"] = f"{version}-rc"
    record["publish_target"] = "binary-tarball"
    return record


def go_proxy_record(proxy: Path, source_root: Path = SOURCE_ROOT) -> dict:
    if not proxy.is_dir():
        raise ValueError(f"Go module proxy 目录不存在：{proxy}")
    list_files = sorted(proxy.rglob("@v/list"))
    if len(list_files) != 1:
        raise ValueError(f"Go module proxy 应有且仅有一份 @v/list，实际 {len(list_files)}：{proxy}")
    version_lines = [line.strip() for line in list_files[0].read_text(encoding="utf-8").splitlines() if line.strip()]
    if not version_lines:
        raise ValueError(f"Go module proxy @v/list 为空：{list_files[0]}")
    version = version_lines[-1]
    module = list_files[0].parent.parent.relative_to(proxy).as_posix()
    archive = list_files[0].parent / f"{version}.zip"
    require_file(archive, "Go SDK zip")
    info = list_files[0].parent / f"{version}.info"
    mod = list_files[0].parent / f"{version}.mod"
    return {
        "kind": "go-sdk-module-proxy",
        "module": module,
        "version": version,
        "path": rel(proxy, source_root),
        "archive": rel(archive, source_root),
        "archive_bytes": archive.stat().st_size,
        "archive_sha256": sha256_file(archive),
        "info": rel(info, source_root) if info.is_file() else None,
        "mod": rel(mod, source_root) if mod.is_file() else None,
        "publish_target": "go-module-proxy",
        "status": "preview",
    }


def dependency_inventory_record(inventory: Path, source_root: Path = SOURCE_ROOT) -> dict:
    record = artifact_file_record("third-party-inventory", inventory, source_root)
    data = json.loads(inventory.read_text(encoding="utf-8"))
    record["package_count"] = data.get("package_count")
    record["packages_with_issues"] = data.get("packages_with_issues")
    record["license_text_count"] = data.get("license_text_count")
    return record


def acceptance_record(result: Path, source_root: Path = SOURCE_ROOT) -> dict:
    record = artifact_file_record("acceptance-result", result, source_root)
    data = json.loads(result.read_text(encoding="utf-8"))
    record["status"] = data.get("status")
    record["sdk_source"] = data.get("sdk_source")
    record["other_dms_packages"] = data.get("other_dms_packages")
    if data.get("status") != "passed":
        raise ValueError(f"验收结果不是 passed：{result}")
    return record


def generated_at() -> str:
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if epoch:
        return dt.datetime.fromtimestamp(int(epoch), tz=dt.timezone.utc).isoformat().replace("+00:00", "Z")
    return dt.datetime.now(tz=dt.timezone.utc).isoformat().replace("+00:00", "Z")


def build_manifest(args: argparse.Namespace) -> dict:
    source_root = args.source_root.resolve()
    version = workspace_version(source_root)
    artifacts = [
        rust_sdk_record(args.rust_sdk_crate, version, source_root),
        server_record(args.server_archive, version, source_root),
    ]
    if args.juicefs_dms_archive:
        artifacts.append(juicefs_dms_record(args.juicefs_dms_archive, version, source_root))
    if args.go_proxy:
        artifacts.append(go_proxy_record(args.go_proxy, source_root))
    if args.third_party_inventory:
        artifacts.append(dependency_inventory_record(args.third_party_inventory, source_root))
    if args.acceptance_result:
        artifacts.append(acceptance_record(args.acceptance_result, source_root))
    return {
        "schema_version": 1,
        "release": {
            "name": f"DMS {version} release candidate",
            "version": version,
            "channel": "rc",
            "remote_publish": False,
            "source_commit": source_commit(source_root),
            "generated_at_utc": generated_at(),
        },
        "contracts": {
            "public_products": ["rust-sdk", "server-linux-tarball"],
            "preview_products": ["go-sdk-module-proxy", "juicefs-dms-linux-tarball"],
            "not_in_0_1_0": ["python-sdk", "cpp-sdk", "multi-meta-ha", "rdma", "ub", "l2-storage"],
        },
        "artifacts": artifacts,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-sdk-crate", type=Path, required=True)
    parser.add_argument("--server-archive", type=Path, required=True)
    parser.add_argument("--juicefs-dms-archive", type=Path)
    parser.add_argument("--go-proxy", type=Path)
    parser.add_argument("--third-party-inventory", type=Path)
    parser.add_argument("--acceptance-result", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, default=SOURCE_ROOT)
    args = parser.parse_args()
    try:
        manifest = build_manifest(args)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    except Exception as exc:
        print(f"candidate_manifest.py: error: {exc}", file=sys.stderr)
        return 2
    print(f"候选发布清单已生成：{args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

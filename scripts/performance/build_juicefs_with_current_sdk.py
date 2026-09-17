#!/usr/bin/env python3
"""构建并校验与当前 DMS Go SDK 同源的 JuiceFS 验收制品。

M1 性能验收会同时执行 Native Filesystem 与 JuiceFS+DMS Glue。Glue 不是一个
独立黑盒：它在编译时链接 DMS Go SDK。如果复用历史二进制，Node 与 SDK 的协议或
数据路径可能已经发生变化，测试结果就不再代表当前源码。

本脚本用临时 ``-modfile`` 把 JuiceFS 的 DMS 依赖替换为当前 ``sdk/go``，不会修改
JuiceFS 工作树；构建后记录 SDK 内容哈希、两个仓库的 Git 身份和二进制哈希。验收
入口随后再次校验这些字段，拒绝来源不明或已经过期的 Glue 制品。
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shlex
import shutil
import subprocess
from typing import Any


SCHEMA = "dms.juicefs-current-sdk-build.v1"


class BuildError(RuntimeError):
    """构建配置、制品来源或外部命令不满足验收合同。"""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_tree_sha256(root: Path) -> str:
    """计算源码目录的稳定内容哈希，不依赖 Git 是否已经提交。"""

    digest = hashlib.sha256()
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        relative = path.relative_to(root)
        if any(part in {".git", "target", "__pycache__"} for part in relative.parts):
            continue
        digest.update(relative.as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def git_identity(root: Path) -> dict[str, Any]:
    head = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        text=True,
        capture_output=True,
        check=True,
    ).stdout.strip()
    dirty_lines = subprocess.run(
        ["git", "-C", str(root), "status", "--short"],
        text=True,
        capture_output=True,
        check=True,
    ).stdout.splitlines()
    return {"head": head, "dirty": bool(dirty_lines), "dirty_paths": dirty_lines}


def map_shared_path(path: Path, host_root: Path, vm_root: Path) -> Path:
    resolved = path.resolve()
    try:
        relative = resolved.relative_to(host_root.resolve())
    except ValueError as error:
        raise BuildError(f"路径不在共享宿主根目录内: {resolved}") from error
    return vm_root / relative


def run(argv: list[str]) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, text=True, capture_output=True, check=False)
    if completed.returncode:
        detail = (completed.stdout + completed.stderr)[-8000:]
        raise BuildError(f"command failed ({completed.returncode}): {shlex.join(argv)}\n{detail}")
    return completed


def build(args: argparse.Namespace) -> dict[str, Any]:
    juicefs_source = args.juicefs_source.resolve()
    dms_sdk_source = args.dms_sdk_source.resolve()
    output = args.output.resolve()
    host_root = args.shared_host_root.resolve()
    vm_root = Path(args.shared_vm_root)

    if output.exists():
        raise BuildError(f"输出目录已存在: {output}")
    for path, label in ((juicefs_source, "JuiceFS source"), (dms_sdk_source, "DMS Go SDK")):
        if not (path / "go.mod").is_file():
            raise BuildError(f"{label} 缺少 go.mod: {path}")

    vm_juicefs = map_shared_path(juicefs_source, host_root, vm_root)
    vm_sdk = map_shared_path(dms_sdk_source, host_root, vm_root)
    vm_output = map_shared_path(output, host_root, vm_root)
    output.mkdir(parents=True)
    shutil.copy2(juicefs_source / "go.mod", output / "acceptance.mod")
    shutil.copy2(juicefs_source / "go.sum", output / "acceptance.sum")

    env_assignments = {
        "GOMAXPROCS": str(args.go_max_procs),
        "GOWORK": "off",
        # Go 的全量 JuiceFS 编译临时文件可达数 GiB；必须放在共享数据盘，不能挤满
        # 小规格验收 VM 的根分区。
        "GOTMPDIR": str(vm_output / "go-tmp"),
    }
    if args.go_mod_cache:
        env_assignments["GOMODCACHE"] = args.go_mod_cache
    if args.go_cache:
        env_assignments["GOCACHE"] = args.go_cache
    exports = " ".join(f"{key}={shlex.quote(value)}" for key, value in env_assignments.items())
    go = shlex.quote(args.go_binary)
    modfile = shlex.quote(str(vm_output / "acceptance.mod"))
    script = "; ".join(
        [
            "set -euo pipefail",
            f"mkdir -p {shlex.quote(str(vm_output / 'go-tmp'))}",
            f"cd {shlex.quote(str(vm_juicefs))}",
            f"export {exports}",
            f"{go} mod edit -modfile={modfile} -replace=github.com/lelezi257/dms/sdk/go={shlex.quote(str(vm_sdk))}",
            f"{go} list -modfile={modfile} -m -json github.com/lelezi257/dms/sdk/go > {shlex.quote(str(vm_output / 'dms-sdk-module.json'))}",
            f"{go} test -modfile={modfile} -p {args.build_parallelism} ./pkg/object -run '^TestDMS' -count=1",
            f"{go} build -modfile={modfile} -p {args.build_parallelism} -o {shlex.quote(str(vm_output / 'juicefs'))} .",
            f"{go} version -m {shlex.quote(str(vm_output / 'juicefs'))} > {shlex.quote(str(vm_output / 'juicefs-buildinfo.txt'))}",
        ]
    )
    completed = run(
        [
            "limactl",
            "shell",
            "--workdir",
            "/tmp",
            args.vm,
            "--",
            "bash",
            "-lc",
            script,
        ]
    )
    (output / "build.stdout.log").write_text(completed.stdout, encoding="utf-8")
    (output / "build.stderr.log").write_text(completed.stderr, encoding="utf-8")

    artifact = output / "juicefs"
    if not artifact.is_file():
        raise BuildError(f"构建成功但制品缺失: {artifact}")
    provenance = {
        "schema": SCHEMA,
        "dependency_mode": "local-replace",
        "artifact": str(artifact),
        "artifact_sha256": sha256_file(artifact),
        "dms_sdk_source": str(dms_sdk_source),
        "dms_sdk_content_sha256": source_tree_sha256(dms_sdk_source),
        "dms_repository": git_identity(dms_sdk_source.parents[1]),
        "juicefs_source": str(juicefs_source),
        "juicefs_repository": git_identity(juicefs_source),
        "builder": {
            "vm": args.vm,
            "go_binary": args.go_binary,
            "go_max_procs": args.go_max_procs,
            "build_parallelism": args.build_parallelism,
        },
    }
    provenance_path = output / "build-provenance.json"
    provenance_path.write_text(
        json.dumps(provenance, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    validate_artifact(artifact, provenance_path, dms_sdk_source)
    return provenance


def validate_artifact(artifact: Path, provenance_path: Path, dms_sdk_source: Path) -> dict[str, Any]:
    if not artifact.is_file():
        raise BuildError(f"JuiceFS 制品不存在: {artifact}")
    if not provenance_path.is_file():
        raise BuildError(f"JuiceFS 构建来源不存在: {provenance_path}")
    try:
        provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BuildError(f"无法读取 JuiceFS 构建来源: {error}") from error
    if provenance.get("schema") != SCHEMA:
        raise BuildError(f"未知 JuiceFS 构建来源格式: {provenance.get('schema')!r}")
    if provenance.get("dependency_mode") != "local-replace":
        raise BuildError("JuiceFS 必须通过 local-replace 链接当前 DMS Go SDK")
    actual_artifact_hash = sha256_file(artifact)
    if provenance.get("artifact_sha256") != actual_artifact_hash:
        raise BuildError("JuiceFS 二进制哈希与构建来源不一致")
    actual_sdk_hash = source_tree_sha256(dms_sdk_source.resolve())
    if provenance.get("dms_sdk_content_sha256") != actual_sdk_hash:
        raise BuildError("JuiceFS 链接的 DMS Go SDK 已过期，请重新构建")
    return provenance


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    build_parser = subparsers.add_parser("build")
    build_parser.add_argument("--juicefs-source", type=Path, required=True)
    build_parser.add_argument("--dms-sdk-source", type=Path, required=True)
    build_parser.add_argument("--output", type=Path, required=True)
    build_parser.add_argument("--vm", default="dms-dev")
    build_parser.add_argument("--shared-host-root", type=Path, required=True)
    build_parser.add_argument("--shared-vm-root", default="/workspace/dms")
    build_parser.add_argument("--go-binary", default="go")
    build_parser.add_argument("--go-mod-cache")
    build_parser.add_argument("--go-cache")
    build_parser.add_argument("--go-max-procs", type=int, default=2)
    build_parser.add_argument("--build-parallelism", type=int, default=2)

    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("--juicefs", type=Path, required=True)
    validate_parser.add_argument("--provenance", type=Path, required=True)
    validate_parser.add_argument("--dms-sdk-source", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.command == "build":
            value = build(args)
        else:
            value = validate_artifact(args.juicefs.resolve(), args.provenance.resolve(), args.dms_sdk_source.resolve())
        print(json.dumps(value, ensure_ascii=False))
        return 0
    except (BuildError, OSError, subprocess.CalledProcessError) as error:
        print(f"JuiceFS current-SDK build failed: {error}", file=__import__("sys").stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

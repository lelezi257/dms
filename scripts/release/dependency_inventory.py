#!/usr/bin/env python3
"""在 Linux 从 Cargo.lock 对应元数据收集第三方许可，不作法律兼容性判断。"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import urllib.request


LICENSE_NAME = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)(?:$|[._-])", re.I)
CURATED_SOURCES = Path(__file__).with_name("license_sources.json")
CRATES_IO_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def load_path_patches(manifest_path):
    """读取根 Cargo.toml 中显式声明的 path patch，不把普通工作区包误作第三方。"""
    manifest_path = Path(manifest_path).resolve()
    document = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    patches = {}
    for registry, packages in document.get("patch", {}).items():
        for name, specification in packages.items():
            if not isinstance(specification, dict) or "path" not in specification:
                continue
            root = (manifest_path.parent / specification["path"]).resolve()
            patches[root] = {
                "name": name,
                "registry": registry,
                "original_source": CRATES_IO_SOURCE if registry == "crates-io" else f"registry+{registry}",
                "declared_path": root.relative_to(manifest_path.parent).as_posix(),
            }
    return patches


def tree_digest(root):
    """对 vendored 源码树做与绝对路径无关的确定性摘要。"""
    checksum = hashlib.sha256()
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        relative = path.relative_to(root).as_posix().encode("utf-8")
        checksum.update(len(relative).to_bytes(8, "big"))
        checksum.update(relative)
        checksum.update(hashlib.sha256(path.read_bytes()).digest())
    return checksum.hexdigest()


def output_paths(record):
    paths = {item["path"] for item in record.get("files", [])}
    patch = record.get("vendored_patch")
    if patch:
        paths.add(patch["path"])
    return paths


def load_curated_sources(path=CURATED_SOURCES):
    """加载精确到 package/version/source 的补充许可来源。

    某些 crates.io 包不会把仓库根目录的 LICENSE 一并打入 .crate。这里不
    根据包名猜许可证，而是只接受仓库内审阅过的精确映射，并在下载后校验
    SHA-256。这样既能补齐二进制分发材料，也不会把网络上的任意内容混入包。
    """
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        raise ValueError("license_sources.json 的 schema_version 不受支持")
    return data.get("packages", [])


def matching_curated_source(package, curated):
    for item in curated:
        if (
            item.get("name") == package["name"]
            and item.get("version") == package["version"]
            and item.get("source") == package.get("source")
        ):
            return item
    return None


def fetch_url(url):
    if not url.startswith("https://"):
        raise ValueError(f"补充许可只允许 HTTPS 来源: {url}")
    with urllib.request.urlopen(url, timeout=30) as response:
        return response.read()


def curated_license_files(package, root, output, folder, item, offline=False, fetcher=fetch_url):
    """复制或下载已经审阅的补充许可，并逐字节校验来源。"""
    files = []
    issues = []
    for material in item.get("materials", []):
        source_path = material["source_path"]
        package_path = material.get("package_path")
        if package_path:
            candidate = root / package_path
            if not candidate.is_file() or candidate.is_symlink() or not candidate.resolve().is_relative_to(root):
                issues.append(f"curated package file unavailable: {package_path}")
                continue
            data = candidate.read_bytes()
        elif offline:
            issues.append(f"curated license requires network in offline mode: {source_path}")
            continue
        else:
            try:
                data = fetcher(material["url"])
            except Exception as error:
                issues.append(f"curated license download failed: {source_path}: {error}")
                continue
        actual = digest(data)
        if actual != material["sha256"]:
            issues.append(f"curated license checksum mismatch: {source_path}")
            continue
        destination = output / "texts" / folder / source_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.is_symlink():
            raise ValueError(f"refusing output symlink: {destination}")
        destination.write_bytes(data)
        files.append({
            "source_path": source_path,
            "path": destination.relative_to(output).as_posix(),
            "sha256": actual,
            "bytes": len(data),
            "origin_url": material.get("url"),
            "curated": True,
        })
    return files, issues


def prepare_output(output):
    """重跑只能替换本工具已登记的文件，拒绝链接和混入的用户文件。"""
    # 只拒绝输出目录本身是链接。macOS 的 /var 等系统父目录本来就是链接，
    # 不能因此把安全的临时目录误判为非法；后续仍以 canonical root 做越界检查。
    if output.is_symlink():
        raise ValueError("output 不得是符号链接")
    resolved_output = output.resolve()
    if not output.exists() or not any(output.iterdir()):
        return set()
    marker = output / "inventory.json"
    if marker.is_symlink() or not marker.is_file():
        raise ValueError("output 非空且不是本工具的盘点目录")
    previous = json.loads(marker.read_text(encoding="utf-8"))
    if previous.get("schema_version") != 1:
        raise ValueError("output 的 schema_version 不受支持")
    owned = {path for package in previous["packages"] for path in output_paths(package)}
    for relative in owned:
        path = output / relative
        if not relative.startswith("texts/") or not path.resolve().is_relative_to(resolved_output):
            raise ValueError("已有清单包含非法输出路径")
    for path in output.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"output 包含链接: {path}")
        if path.is_file() and path.relative_to(output).as_posix() not in owned | {"inventory.json", "DEPENDENCIES.md"}:
            raise ValueError(f"output 包含非本工具文件: {path}")
    return owned


def license_paths(package, root):
    """保留嵌套 vendored 代码的声明；不跟随链接或允许 license_file 越界。"""
    paths = set()
    problems = []
    explicit = package.get("license_file")
    if explicit:
        candidate = root / explicit
        if candidate.is_absolute() and not candidate.is_relative_to(root):
            problems.append("license_file escapes package root")
        elif not candidate.is_file():
            problems.append(f"declared license_file missing: {explicit}")
        else:
            paths.add(candidate)
    for candidate in root.rglob("*"):
        if LICENSE_NAME.match(candidate.name) and candidate.is_file():
            paths.add(candidate)
    accepted = []
    for path in sorted(paths):
        # resolve 同时捕获父目录中的链接；不能把 registry 外的文件放入附件。
        if path.is_symlink() or path.resolve() != path or not path.resolve().is_relative_to(root):
            problems.append(f"unsafe license path: {path.relative_to(root)}")
        else:
            accepted.append(path)
    return accepted, problems


def copy_material(root, output, folder, source_path):
    path = root / source_path
    if not path.is_file() or path.is_symlink() or not path.resolve().is_relative_to(root):
        return None
    data = path.read_bytes()
    destination = output / "texts" / folder / source_path
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_symlink():
        raise ValueError(f"refusing output symlink: {destination}")
    destination.write_bytes(data)
    return {
        "source_path": source_path,
        "path": destination.relative_to(output).as_posix(),
        "sha256": digest(data),
        "bytes": len(data),
    }


def collect(metadata, output, lock_hash, curated=None, offline=False, fetcher=fetch_url, path_patches=None):
    output.mkdir(parents=True, exist_ok=True)
    curated = load_curated_sources() if curated is None else curated
    path_patches = {} if path_patches is None else path_patches
    workspace_members = set(metadata.get("workspace_members", []))
    records = []
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"], item["id"])):
        source = package.get("source")
        if not source and package["id"] in workspace_members:
            continue  # 本项目许可由根 LICENSE 交付；不能冒充第三方包。
        manifest = Path(package["manifest_path"]).resolve()
        root = manifest.parent
        path_patch = path_patches.get(root) if not source else None
        if not source and not path_patch:
            continue  # 非 workspace 的普通 path 依赖不在本工具的已审阅来源范围内。
        recorded_source = path_patch["original_source"] if path_patch else source
        record = {
            "name": package["name"], "version": package["version"],
            "package_id": package["id"], "source": recorded_source,
            "resolved_source": "vendored-path" if path_patch else "registry",
            "license_expression": package.get("license"),
            "license_file": package.get("license_file"),
            "files": [], "issues": [],
        }
        # 保持 SPDX 的 AND/OR/括号原样，不替用户选择许可分支。
        if not record["license_expression"] and not record["license_file"]:
            record["issues"].append("manifest declares neither license nor license_file")
        curated_source = matching_curated_source(package, curated)
        if source and not source.startswith("registry+") and not curated_source:
            record["issues"].append("non-registry dependency requires separate source review")
        if not manifest.is_file():
            record["issues"].append("dependency manifest unavailable")
        else:
            record["manifest_sha256"] = digest(manifest.read_bytes())
            candidates, problems = license_paths(package, root)
            record["issues"].extend(problems)
            # source hash 区分同名同版本的不同 registry，不使用机器绝对路径。
            folder = f"{package['name']}-{package['version']}-{digest(recorded_source.encode())[:12]}"
            for path in candidates:
                relative = path.relative_to(root)
                if not path.stat().st_size:
                    record["issues"].append(f"empty license file: {relative.as_posix()}")
                data = path.read_bytes()
                destination = output / "texts" / folder / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                if destination.is_symlink():
                    raise ValueError(f"refusing output symlink: {destination}")
                destination.write_bytes(data)
                record["files"].append({
                    "source_path": relative.as_posix(),
                    "path": destination.relative_to(output).as_posix(),
                    "sha256": digest(data), "bytes": len(data),
                })
            if not candidates and curated_source:
                files, issues = curated_license_files(
                    package, root, output, folder, curated_source, offline=offline, fetcher=fetcher
                )
                record["files"].extend(files)
                record["issues"].extend(issues)
            if not record["files"]:
                record["issues"].append("no license/notice text file found in downloaded package")
            if curated_source:
                record["source_review"] = {
                    "status": "exact-source-reviewed",
                    "reference": curated_source.get("reference"),
                }
            if path_patch:
                if path_patch["name"] != package["name"]:
                    record["issues"].append("path patch package name does not match resolved package")
                patch_material = copy_material(root, output, folder, "DMS-PATCH.md")
                if patch_material is None:
                    record["issues"].append("vendored path patch is missing DMS-PATCH.md")
                else:
                    record["vendored_patch"] = patch_material
                vcs_file = root / ".cargo_vcs_info.json"
                upstream_revision = None
                if vcs_file.is_file():
                    try:
                        upstream_revision = json.loads(vcs_file.read_text(encoding="utf-8"))["git"]["sha1"]
                    except (KeyError, TypeError, json.JSONDecodeError):
                        record["issues"].append("invalid .cargo_vcs_info.json in vendored dependency")
                record["source_review"] = {
                    "status": "vendored-patched",
                    "registry": path_patch["registry"],
                    "original_source": path_patch["original_source"],
                    "original_version": package["version"],
                    "upstream_repository": package.get("repository"),
                    "upstream_revision": upstream_revision,
                    "vendored_path": path_patch["declared_path"],
                    "vendored_tree_sha256": tree_digest(root),
                }
        records.append(record)
    report = {
        "schema_version": 1,
        "scope": "cargo metadata --locked --all-features workspace resolution; includes development/build and target-specific dependencies, not proof each is linked into the service",
        "legal_status": "inventory only; not legal approval or ownership verification",
        "cargo_lock_sha256": lock_hash,
        "package_count": len(records),
        "packages_with_issues": sum(bool(item["issues"]) for item in records),
        "license_text_count": sum(len(item["files"]) for item in records),
        "packages": records,
    }
    (output / "inventory.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    lines = [
        "# 第三方依赖许可盘点", "",
        "这是 Cargo locked 元数据与下载包内原始许可材料的工程盘点，不是法律批准或权利归属确认。",
        "范围含开发/构建及目标平台依赖，是保守附件集合，不表示每个依赖都链接进服务二进制。容器镜像不在本清单内。", "",
        f"Cargo.lock SHA-256：`{lock_hash}`", "",
        f"包：{report['package_count']}；原始文件：{report['license_text_count']}；有待确认项的包：{report['packages_with_issues']}。", "",
        "| 包 / 版本 | 来源 | 原始许可表达式 | 原文文件 / patch | 待确认 |",
        "| --- | --- | --- | --- | --- |",
    ]
    for item in records:
        files = ", ".join(f"[{row['source_path']}]({row['path']})" for row in item["files"]) or "未找到"
        patch = item.get("vendored_patch")
        if patch:
            files += f", [{patch['source_path']}]({patch['path']})"
        issues = "; ".join(item["issues"]) or "无自动发现项（不代表法律批准）"
        expression = item["license_expression"] or "见 license_file / 待确认"
        review = item.get("source_review", {})
        source_summary = item["source"]
        if review.get("status") == "vendored-patched":
            source_summary = (
                f"{review['registry']} 原版 {review['original_version']}；"
                f"vendored `{review['vendored_path']}`；tree `{review['vendored_tree_sha256']}`"
            )
        lines.append(
            f"| {item['name']} {item['version']} | {source_summary.replace('|', '&#124;')} | "
            f"{expression.replace('|', '&#124;')} | {files} | {issues} |"
        )
    (output / "DEPENDENCIES.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return report


def verify_output(output, required_vendored_patches=()):
    """验证已生成附件未被篡改，并确认运行包要求的 vendored patch 材料齐全。"""
    output = Path(output).resolve()
    report = json.loads((output / "inventory.json").read_text(encoding="utf-8"))
    if report.get("schema_version") != 1:
        raise ValueError("inventory.json 的 schema_version 不受支持")
    for package in report.get("packages", []):
        for material_path in output_paths(package):
            candidate = output / material_path
            if not candidate.is_file() or candidate.is_symlink() or not candidate.resolve().is_relative_to(output):
                raise ValueError(f"inventory material unavailable: {material_path}")
            expected = next(
                item["sha256"] for item in package.get("files", []) + [package.get("vendored_patch")]
                if item and item["path"] == material_path
            )
            if digest(candidate.read_bytes()) != expected:
                raise ValueError(f"inventory material checksum mismatch: {material_path}")
    by_name = {package["name"]: package for package in report.get("packages", [])}
    for name in required_vendored_patches:
        package = by_name.get(name)
        if not package:
            raise ValueError(f"required vendored dependency missing from inventory: {name}")
        if package.get("source_review", {}).get("status") != "vendored-patched":
            raise ValueError(f"required vendored dependency lacks patch provenance: {name}")
        if not package.get("files") or not package.get("vendored_patch"):
            raise ValueError(f"required vendored dependency lacks license or patch material: {name}")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest-path", type=Path, default=Path(__file__).resolve().parents[2] / "Cargo.toml")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--output", type=Path, help="专用生成目录；现有文件必须为空或由本工具生成")
    mode.add_argument("--verify-output", type=Path, help="验证既有清单及其附件，不重新访问 Cargo 或网络")
    parser.add_argument("--require-vendored-patch", action="append", default=[], help="验证模式下必须存在的 vendored patch 包名")
    parser.add_argument("--offline", action="store_true", help="禁止 Cargo 网络访问，缺依赖缓存时失败")
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("必须在 Linux VM/容器执行，macOS 只用于编辑和阅读")
    if args.verify_output:
        try:
            report = verify_output(args.verify_output, args.require_vendored_patch)
        except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
            parser.error(str(error))
        print(json.dumps({key: report[key] for key in ("package_count", "license_text_count", "packages_with_issues")}, ensure_ascii=False))
        return 2 if report["packages_with_issues"] else 0
    output = args.output.absolute()
    try:
        previous_files = prepare_output(output)
    except (ValueError, KeyError) as error:
        parser.error(str(error))
    command = [
        "cargo", "metadata", "--format-version", "1", "--locked", "--all-features",
        "--manifest-path", str(args.manifest_path.resolve()),
    ]
    if args.offline:
        command.append("--offline")
    result = subprocess.run(command, check=True, stdout=subprocess.PIPE)
    metadata = json.loads(result.stdout)
    lock = Path(metadata["workspace_root"]) / "Cargo.lock"
    report = collect(
        metadata,
        output,
        digest(lock.read_bytes()),
        offline=args.offline,
        path_patches=load_path_patches(args.manifest_path),
    )
    current_files = {path for package in report["packages"] for path in output_paths(package)}
    # 只删除旧清单中已核对的生成文件，避免依赖移除后留下过时许可附件。
    for relative in sorted(previous_files - current_files):
        (output / relative).unlink(missing_ok=True)
    print(json.dumps({key: report[key] for key in ("package_count", "license_text_count", "packages_with_issues")}, ensure_ascii=False))
    return 2 if report["packages_with_issues"] else 0


if __name__ == "__main__":
    sys.exit(main())

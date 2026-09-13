#!/usr/bin/env python3
"""为已经上传的正式 GitHub Release 生成机器可读资产清单。"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_commit(root):
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()


def build_manifest(root, tag, assets, repository):
    records = []
    for asset in sorted(assets, key=lambda path: path.name):
        if not asset.is_file():
            raise ValueError(f"资产不存在: {asset}")
        records.append({"name": asset.name, "bytes": asset.stat().st_size, "sha256": sha256(asset)})
    return {
        "schema_version": 1,
        "release": "DMS 0.1.0",
        "version": "0.1.0",
        "tag": tag,
        "source_commit": source_commit(root),
        "repository": repository,
        "remote_publish": True,
        "assets": records,
        "limitations": [
            "local-memory durability only",
            "single Meta process",
            "Linux host memory; no RDMA/UB/L2/device memory",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--tag", default="v0.1.0")
    parser.add_argument("--repository", default="lelezi257/dms")
    parser.add_argument("--asset", action="append", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        manifest = build_manifest(args.source_root.resolve(), args.tag, args.asset, args.repository)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except Exception as error:
        parser.error(str(error))
    print(f"正式发布清单已生成: {args.output}")


if __name__ == "__main__":
    main()

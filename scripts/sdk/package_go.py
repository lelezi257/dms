#!/usr/bin/env python3
"""生成 Go SDK 候选 module proxy；不发布、不使用本地 replace、不要求消费者有 protoc。

版本包含源码摘要，避免候选变化复用同一版本后被 Go 校验缓存拒绝。此工具只负责
打包；调用方仍须在独立目录执行 go test/build 和真实服务读写验收。
"""
import argparse
import hashlib
import json
import platform
from pathlib import Path
import zipfile

SOURCE = Path(__file__).resolve().parents[2]
MODULE = "github.com/lelezi257/dms/sdk/go"


def package(output: Path) -> dict:
    if platform.system() != "Linux":
        raise RuntimeError("run packaging inside Linux")
    sdk = SOURCE / "sdk/go"
    files = {
        str(path.relative_to(sdk)): path.read_bytes()
        for path in sorted(sdk.rglob("*"))
        if path.is_file() and (path.suffix == ".go" or path.name in ("go.mod", "go.sum", "README.md"))
    }
    files["LICENSE"] = (SOURCE / "LICENSE").read_bytes()
    if not any(name.endswith("_grpc.pb.go") for name in files):
        raise RuntimeError("generate SDK protobuf bindings before packaging")
    digest = hashlib.sha256()
    for name, data in sorted(files.items()):
        digest.update(name.encode() + b"\0" + data + b"\0")
    version = "v0.1.0-dev." + digest.hexdigest()[:16]
    target = output.resolve() / MODULE / "@v"
    target.mkdir(parents=True, exist_ok=True)
    archive = target / (version + ".zip")
    if not archive.exists():
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as packed:
            for name, data in sorted(files.items()):
                entry = zipfile.ZipInfo(f"{MODULE}@{version}/{name}", (2026, 9, 9, 0, 0, 0))
                entry.compress_type = zipfile.ZIP_DEFLATED
                entry.external_attr = 0o644 << 16
                packed.writestr(entry, data)
    (target / (version + ".mod")).write_bytes(files["go.mod"])
    (target / (version + ".info")).write_text(json.dumps({"Version": version, "Time": "2026-09-09T00:00:00Z"}) + "\n")
    versions = sorted(path.stem for path in target.glob("*.mod"))
    (target / "list").write_text("\n".join(versions) + "\n")
    return {"module": MODULE, "version": version, "proxy": output.resolve().as_uri(),
            "archive": str(archive), "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
            "files": sorted(files)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(package(args.output), ensure_ascii=False, indent=2))

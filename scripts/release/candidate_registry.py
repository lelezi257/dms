#!/usr/bin/env python3
"""把一个实际 .crate 放入只读候选仓库，供版本依赖验收；不上传公共平台。

只使用 Cargo sparse index 的文件格式和 Python HTTP server。发布方可用
此小工具验证“依赖版本 + 下载包”，不是把消费者指向原 SDK 源码目录。
"""

import argparse
import hashlib
import http.server
import json
from pathlib import Path
import tarfile
import tomllib


def index_entry(manifest, archive):
    package = manifest["package"]
    dependencies = []
    sections = [(None, manifest)] + list(manifest.get("target", {}).items())
    for target, tables in sections:
        for table, kind in (("dependencies", "normal"), ("build-dependencies", "build"),
                            ("dev-dependencies", "dev")):
            for name, spec in tables.get(table, {}).items():
                spec = {"version": spec} if isinstance(spec, str) else spec
                if "path" in spec or "git" in spec:
                    raise ValueError(f"候选包仍依赖源码路径或 Git：{name}")
                dependencies.append({
                    "name": name, "req": spec["version"],
                    "features": spec.get("features", []),
                    "optional": spec.get("optional", False),
                    "default_features": spec.get("default-features", True),
                    # sparse index 中 null 表示“与当前包同仓库”，不是 crates.io。
                    # 候选仓只托管 DMS；第三方必须显式指回标准仓库。
                    "target": target, "kind": kind,
                    "registry": "https://github.com/rust-lang/crates.io-index",
                    "package": spec.get("package"),
                })
    return {
        "name": package["name"], "vers": package["version"], "deps": dependencies,
        "cksum": hashlib.sha256(archive).hexdigest(), "features": {},
        "features2": manifest.get("features", {}), "v": 2, "yanked": False,
        "rust_version": package.get("rust-version"),
    }


def prepare(crate, directory, address):
    data = crate.read_bytes()
    # 只读取包内 manifest，不解压任意路径到磁盘。
    with tarfile.open(crate, "r:gz") as archive:
        members = [x for x in archive.getmembers()
                   if len(Path(x.name).parts) == 2 and x.name.endswith("/Cargo.toml")]
        if len(members) != 1:
            raise ValueError("包中必须有且仅有一份顶层 Cargo.toml")
        manifest = tomllib.loads(archive.extractfile(members[0]).read().decode())
    entry = index_entry(manifest, data)
    name, version = entry["name"], entry["vers"]
    if name != "dms-client":
        raise ValueError("本验收仓只接受 dms-client")
    directory.mkdir(parents=True, exist_ok=False)
    (directory / "config.json").write_text(json.dumps({
        "dl": f"http://{address}/crates/{{crate}}/{{version}}/download"
    }) + "\n")
    index = directory / name[:2] / name[2:4] / name
    index.parent.mkdir(parents=True)
    index.write_text(json.dumps(entry) + "\n")
    download = directory / "crates" / name / version / "download"
    download.parent.mkdir(parents=True)
    download.write_bytes(data)
    return entry


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("crate", type=Path)
    parser.add_argument("directory", type=Path, help="必须不存在的专用输出目录")
    parser.add_argument("--port", type=int, default=26880)
    args = parser.parse_args()
    entry = prepare(args.crate, args.directory, f"127.0.0.1:{args.port}")
    print(f"candidate {entry['name']} {entry['vers']} sha256={entry['cksum']}", flush=True)
    handler = lambda *a, **kw: http.server.SimpleHTTPRequestHandler(
        *a, directory=str(args.directory), **kw)
    # 默认仅回环监听，临时验证仓不暴露到其它机器。
    with http.server.ThreadingHTTPServer(("127.0.0.1", args.port), handler) as server:
        server.serve_forever()


if __name__ == "__main__":
    main()

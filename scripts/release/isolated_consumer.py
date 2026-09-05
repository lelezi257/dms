#!/usr/bin/env python3
"""在只含候选制品的 Linux 容器内执行；不读取原项目源码。

输入放 /input：sdk.crate、server.tar.gz、consumer.rs。/results 保存运行包、
日志与 JSON 结果。Rust 工具链可只读挂载 /opt/rust；第三方源码缓存可通过
--third-party-directory 复用，不将 DMS 自身替换成 path dependency。
"""

import argparse
import functools
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import threading

from candidate_registry import prepare


def run(args, cwd, env):
    print("$ " + " ".join(map(str, args)), flush=True)
    subprocess.run(args, cwd=cwd, env=env, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=Path("/input"))
    parser.add_argument("--output", type=Path, default=Path("/results"))
    parser.add_argument("--third-party-directory", type=Path)
    parser.add_argument("--port-base", type=int, default=26000)
    parser.add_argument("--otlp", default="http://127.0.0.1:24317")
    args = parser.parse_args()
    if Path("/workspace/dms/source").exists() or shutil.which("protoc"):
        raise RuntimeError("隔离条件不成立：能看到原源码或 protoc")
    args.output.mkdir(parents=True, exist_ok=True)
    app = args.output / "consumer"
    app.mkdir(exist_ok=False)
    (app / "src").mkdir()
    shutil.copy2(args.input / "consumer.rs", app / "src/main.rs")
    index = args.output / "registry"
    entry = prepare(args.input / "sdk.crate", index, "127.0.0.1:26880")
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(index))
    registry = http.server.ThreadingHTTPServer(("127.0.0.1", 26880), handler)
    threading.Thread(target=registry.serve_forever, daemon=True).start()
    (app / "Cargo.toml").write_text(
        '[package]\nname="candidate-consumer"\nversion="0.0.0"\nedition="2024"\n'
        f'[dependencies]\ndms-client={{ version="={entry["vers"]}", registry="dms-candidate" }}\n')
    (app / ".cargo").mkdir()
    config = '[registries.dms-candidate]\nindex="sparse+http://127.0.0.1:26880/"\n'
    if args.third_party_directory:
        # Cargo directory source 只用于第三方；SDK 仍真实从临时 HTTP registry 下载。
        config += ('[source.crates-io]\nreplace-with="cached-third-party"\n'
                   f'[source.cached-third-party]\ndirectory={json.dumps(str(args.third_party_directory))}\n')
    (app / ".cargo/config.toml").write_text(config)
    env = dict(os.environ, CARGO_HOME="/tmp/dms-candidate-cargo",
               CARGO_TARGET_DIR="/tmp/dms-candidate-target", CARGO_INCREMENTAL="0",
               PROTOC="/protoc-intentionally-absent", CARGO_BUILD_JOBS="2")
    env["PATH"] = "/opt/rust/bin:" + env["PATH"]
    run(["cargo", "build"], app, env)
    run(["cargo", "build", "--locked"], app, env)
    run(["cargo", "tree", "--locked", "-e", "normal"], app, env)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=app, env=env))
    sdk = next(p for p in metadata["packages"] if p["name"] == "dms-client")
    if sdk["source"] != "sparse+http://127.0.0.1:26880/":
        # Cargo 报告的 source 通常用 registry+ 前缀，索引 URL 仍应是本次测试仓。
        if sdk["source"] != "registry+sparse+http://127.0.0.1:26880/":
            raise RuntimeError(f"SDK 未来自候选仓：{sdk['source']}")
    leaked = [p["name"] for p in metadata["packages"]
              if p["name"].startswith("dms-") and p["name"] != "dms-client"]
    if leaked:
        raise RuntimeError(f"消费者仍依赖其它 DMS 包：{leaked}")
    (args.output / "consumer-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    (args.output / "container-mountinfo.txt").write_text(Path("/proc/self/mountinfo").read_text())
    with tarfile.open(args.input / "server.tar.gz", "r:gz") as archive:
        roots = {Path(m.name).parts[0] for m in archive.getmembers()}
        if len(roots) != 1:
            raise RuntimeError("服务归档不是单根目录")
        archive.extractall(args.output, filter="data")
    home = args.output / next(iter(roots))
    run(["sha256sum", "-c", "SHA256SUMS"], home, env)
    config = (home / "config/dms.env.example").read_text()
    b = args.port_base
    overrides = {
        "DMS_NODE_ID": "candidate-node", "DMS_META_NODE_ID": "candidate-meta",
        "DMS_META_BIND": f"0.0.0.0:{b+300}", "DMS_META_STATUS_BIND": f"0.0.0.0:{b+100}",
        "DMS_META_ENDPOINT": f"http://127.0.0.1:{b+300}",
        "DMS_NODE_IP": "127.0.0.1", "DMS_WORKER_PORT": str(b+200),
        "DMS_NODE_STATUS_BIND": f"0.0.0.0:{b}",
        "DMS_CLIENT_METRICS_BIND": f"0.0.0.0:{b+400}",
        "DMS_CLIENT_ENDPOINT": f"unix://{home}/run/dms-worker.sock",
        "DMS_ARENA_CAPACITY_BYTES": str(64 * 1024 * 1024),
        "DMS_LOG_LEVEL": "debug", "DMS_TRACING_ENABLED": "true",
        "DMS_TRACING_SAMPLE_RATIO": "1", "DMS_TRACING_OTLP_ENDPOINT": args.otlp,
    }
    # 后写覆盖只用于本次验收目录，原制品保留且其SHA已经验证。
    (home / "config/dms.env").write_text(config + "\n" + "\n".join(
        f'{k}="{v}"' for k, v in overrides.items()) + "\n")
    for role in ("meta", "node", "client"):
        run(["bash", "scripts/cluster.sh", role, "start"], home, env)
    executable = "/tmp/dms-candidate-target/debug/candidate-consumer"
    for endpoint, shm in ((f"http://127.0.0.1:{b+200}", False),
                          (f"unix://{home}/run/dms-worker.sock", True)):
        run([executable], app, dict(env, DMS_ENDPOINT=endpoint,
                                   DMS_SHARED_MEMORY=str(shm).lower()))
    report = {"status": "passed", "sdk_sha256": entry["cksum"],
              "sdk_dependency": f'dms-client={entry["vers"]} via sparse HTTP registry',
              "original_source_visible": False, "protoc_available": False,
              "sdk_source": sdk["source"], "other_dms_packages": leaked,
              "tests": ["TCP/SHM", "10-byte/128-KiB", "SET/GET/SET_RANGE/DEL",
                        "HSET/HGET/DEL", "public DmsError/ErrorKind"],
              "third_party_cache": str(args.third_party_directory),
              "server_home": str(home),
              "processes": "left running for observability acceptance"}
    (args.output / "consumer-result.json").write_text(json.dumps(report, indent=2) + "\n")
    registry.shutdown()
    print(json.dumps(report), flush=True)


if __name__ == "__main__":
    main()

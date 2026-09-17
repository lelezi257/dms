#!/usr/bin/env python3
"""生成并校验 M1.7 Native Filesystem 三 VM 性能与白盒证据。

这个入口把原先分散的四步收敛成一个可审计流程：先在同一组三台 VM 上交替运行
Native Filesystem 与 JuiceFS+DMS Glue，再汇总原始样本，最后分别执行端到端性能门禁
和 FUSE/DataCore/Meta/Peer 请求放大门禁。第二个总验收 case 可以使用
``--reuse-existing`` 复核同一份证据，避免重复运行六轮对称基准。
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]
PRODUCER = ROOT / "scripts/performance/run_native_filesystem_vs_glue_3vm.py"
ASSEMBLER = ROOT / "scripts/performance/assemble_native_filesystem_result.py"
PERFORMANCE_EVALUATOR = ROOT / "scripts/performance/evaluate_native_filesystem.py"
AMPLIFICATION_EVALUATOR = ROOT / "scripts/performance/evaluate_fuse_request_amplification.py"
JUICEFS_BUILDER = ROOT / "scripts/performance/build_juicefs_with_current_sdk.py"


def command(argv: list[str]) -> None:
    completed = subprocess.run(argv, cwd=ROOT, text=True, check=False)
    if completed.returncode:
        raise RuntimeError(f"command failed ({completed.returncode}): {' '.join(argv)}")


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def current_source_head() -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=True,
    ).stdout.strip()


def require_pass(path: Path, expected_schema: str) -> None:
    value = load_json(path)
    if value.get("schema") != expected_schema:
        raise RuntimeError(f"unexpected schema in {path}: {value.get('schema')!r}")
    if value.get("status") != "PASS":
        raise RuntimeError(f"evaluation did not pass: {path}")


def build_profile(args: argparse.Namespace, juicefs: Path) -> dict:
    if args.dms_node is None or args.dms_meta is None:
        raise RuntimeError("new performance run requires --dms-node and --dms-meta")
    artifacts = {
        "dms_node": args.dms_node.resolve(),
        "dms_meta": args.dms_meta.resolve(),
        "juicefs": juicefs.resolve(),
    }
    hashes = {name: sha256_file(path) for name, path in artifacts.items()}
    artifact_provenance = {
        name: {"path": str(path), "sha256": hashes[name]}
        for name, path in artifacts.items()
    }
    juicefs_provenance = juicefs.parent / "build-provenance.json"
    if args.juicefs_provenance is not None:
        juicefs_provenance = args.juicefs_provenance.resolve()
    if juicefs_provenance.is_file():
        artifact_provenance["juicefs"]["provenance"] = {
            "path": str(juicefs_provenance),
            "sha256": sha256_file(juicefs_provenance),
        }
    return {
        "run_id": args.run_id,
        "description": (
            "M1.7 同环境六轮对称验收：Native Filesystem 与 JuiceFS+DMS Glue "
            "交替执行，并记录端到端延迟、RPC、复制与资源路径。"
        ),
        "source_head": current_source_head(),
        "vms": {"A": args.vm_a, "B": args.vm_b, "C": args.vm_c},
        "ips": {"A": args.ip_a, "B": args.ip_b, "C": args.ip_c},
        "ports": {
            "worker": args.worker_port,
            "node_health": args.node_health_port,
            "meta": args.meta_port,
            "meta_health": args.meta_health_port,
            "redis": args.redis_port,
        },
        "artifacts": {
            "dms_node": str(artifacts["dms_node"]),
            "dms_meta": str(artifacts["dms_meta"]),
            "juicefs": str(artifacts["juicefs"]),
        },
        "hashes": hashes,
        "artifact_provenance": artifact_provenance,
        "rounds": args.rounds,
        "metadata_mode": "memory",
        "redis_mode": "memory",
        "arena_capacity_bytes": args.arena_capacity_bytes,
        "region_size_bytes": args.region_size_bytes,
        "node_current_cache_bytes": args.node_current_cache_bytes,
    }


def prepare_juicefs(args: argparse.Namespace, output: Path) -> Path:
    """取得与当前 SDK 同源、且可审计的 Glue 二进制。"""

    if args.juicefs_source is not None:
        build_output = output / "artifacts" / "juicefs-current-sdk"
        argv = [
            sys.executable,
            str(JUICEFS_BUILDER),
            "build",
            "--juicefs-source",
            str(args.juicefs_source.resolve()),
            "--dms-sdk-source",
            str((ROOT / "sdk/go").resolve()),
            "--output",
            str(build_output),
            "--vm",
            args.builder_vm,
            "--shared-host-root",
            str(args.shared_host_root.resolve()),
            "--shared-vm-root",
            args.shared_vm_root,
            "--go-binary",
            args.go_binary,
        ]
        if args.go_mod_cache:
            argv.extend(["--go-mod-cache", args.go_mod_cache])
        if args.go_cache:
            argv.extend(["--go-cache", args.go_cache])
        command(argv)
        return build_output / "juicefs"

    if args.juicefs is None or args.juicefs_provenance is None:
        raise RuntimeError(
            "new performance run requires --juicefs-source, or both --juicefs and --juicefs-provenance"
        )
    command(
        [
            sys.executable,
            str(JUICEFS_BUILDER),
            "validate",
            "--juicefs",
            str(args.juicefs.resolve()),
            "--provenance",
            str(args.juicefs_provenance.resolve()),
            "--dms-sdk-source",
            str((ROOT / "sdk/go").resolve()),
        ]
    )
    return args.juicefs.resolve()


def validate_existing_freshness(result: dict) -> None:
    environment = result.get("environment")
    if not isinstance(environment, dict):
        raise RuntimeError("performance evidence is missing result.environment")
    source_head = environment.get("source_head")
    if not isinstance(source_head, str) or not source_head:
        raise RuntimeError("performance evidence is missing result.environment.source_head")
    current_head = current_source_head()
    if source_head != current_head:
        raise RuntimeError(
            "stale performance evidence: result.environment.source_head does not match current HEAD"
        )

    artifacts = environment.get("artifacts")
    hashes = environment.get("hashes")
    if not isinstance(artifacts, dict) or not isinstance(hashes, dict):
        raise RuntimeError("performance evidence is missing artifact paths or hashes")
    for name in ("dms_node", "dms_meta", "juicefs"):
        path_value = artifacts.get(name)
        digest = hashes.get(name)
        if not isinstance(path_value, str) or not isinstance(digest, str) or not digest:
            raise RuntimeError(f"performance evidence is missing artifact hash for {name}")
        path = Path(path_value)
        if not path.is_file():
            raise RuntimeError(f"performance evidence artifact is not available: {name}: {path}")
        if sha256_file(path) != digest:
            raise RuntimeError(f"stale performance evidence: artifact hash changed: {name}")

    provenance = environment.get("artifact_provenance")
    if not isinstance(provenance, dict):
        raise RuntimeError("performance evidence is missing artifact_provenance")
    juicefs = provenance.get("juicefs")
    if not isinstance(juicefs, dict):
        raise RuntimeError("performance evidence is missing JuiceFS artifact provenance")
    recorded_juicefs_hash = juicefs.get("sha256")
    if recorded_juicefs_hash != hashes.get("juicefs"):
        raise RuntimeError("performance evidence JuiceFS provenance does not match artifact hash")
    provenance_file = juicefs.get("provenance")
    if provenance_file is None:
        raise RuntimeError("performance evidence is missing JuiceFS build provenance")
    if not isinstance(provenance_file, dict):
        raise RuntimeError("performance evidence JuiceFS build provenance must be an object")
    provenance_path = provenance_file.get("path")
    provenance_hash = provenance_file.get("sha256")
    if not isinstance(provenance_path, str) or not isinstance(provenance_hash, str):
        raise RuntimeError("performance evidence JuiceFS build provenance is incomplete")
    path = Path(provenance_path)
    if not path.is_file():
        raise RuntimeError(f"performance evidence JuiceFS build provenance is not available: {path}")
    if sha256_file(path) != provenance_hash:
        raise RuntimeError("stale performance evidence: JuiceFS build provenance hash changed")


def validate_existing(
    output: Path,
    rounds: int,
    *,
    allow_stale_evidence: bool = False,
    purpose: str = "discovery",
) -> None:
    if allow_stale_evidence and purpose == "release":
        raise RuntimeError("release M1 performance validation cannot use --allow-stale-evidence")
    result_path = output / "result.json"
    result = load_json(result_path)
    if result.get("schema") != "dms.native-filesystem-vs-glue-result.v1":
        raise RuntimeError(f"unexpected result schema: {result.get('schema')!r}")
    if result.get("same_environment") is not True:
        raise RuntimeError("performance evidence must contain native and glue from the same run")
    if result.get("workload", {}).get("rounds", 0) < rounds:
        raise RuntimeError(f"performance evidence must contain at least {rounds} rounds")
    if not allow_stale_evidence:
        validate_existing_freshness(result)
    require_pass(
        output / "performance-evaluation.json",
        "dms.native-filesystem-vs-glue-evaluation.v1",
    )
    require_pass(
        output / "amplification-evaluation.json",
        "dms.fuse-request-amplification-evaluation.v1",
    )


def validate_round_count(rounds: int) -> None:
    """保证两个 backend 获得相同次数的先跑与后跑机会。"""

    if rounds < 6:
        raise RuntimeError("M1 performance acceptance requires at least six rounds")
    if rounds % 2 != 0:
        raise RuntimeError("M1 performance acceptance requires an even number of rounds")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dms-node", type=Path)
    parser.add_argument("--dms-meta", type=Path)
    parser.add_argument("--juicefs", type=Path)
    parser.add_argument("--juicefs-provenance", type=Path)
    parser.add_argument("--juicefs-source", type=Path)
    parser.add_argument("--builder-vm", default="dms-dev")
    parser.add_argument("--shared-host-root", type=Path, default=ROOT.parent)
    parser.add_argument("--shared-vm-root", default="/workspace/dms")
    parser.add_argument("--go-binary", default="go")
    parser.add_argument("--go-mod-cache")
    parser.add_argument("--go-cache")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reuse-existing", action="store_true")
    parser.add_argument("--allow-stale-evidence", action="store_true")
    parser.add_argument("--purpose", choices=("discovery", "release"), default="discovery")
    parser.add_argument("--run-id", default=dt.datetime.now(dt.timezone.utc).strftime("m1-perf-%Y%m%dT%H%M%SZ"))
    parser.add_argument("--rounds", type=int, default=6)
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--worker-port", type=int, default=33277)
    parser.add_argument("--node-health-port", type=int, default=33278)
    parser.add_argument("--meta-port", type=int, default=33377)
    parser.add_argument("--meta-health-port", type=int, default=33378)
    parser.add_argument("--redis-port", type=int, default=26397)
    parser.add_argument("--arena-capacity-bytes", type=int, default=1024 * 1024 * 1024)
    parser.add_argument("--region-size-bytes", type=int, default=64 * 1024 * 1024)
    parser.add_argument("--node-current-cache-bytes", type=int, default=64 * 1024 * 1024)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        validate_round_count(args.rounds)
        output = args.output.resolve()
        if not args.reuse_existing:
            if output.exists():
                raise RuntimeError(f"output already exists: {output}")
            output.mkdir(parents=True)
            juicefs = prepare_juicefs(args, output)
            profile_path = output / "profile.json"
            profile_path.write_text(
                json.dumps(build_profile(args, juicefs), ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
            raw = output / "raw"
            result = output / "result.json"
            command([sys.executable, str(PRODUCER), "--profile", str(profile_path), "--output", str(raw)])
            command([sys.executable, str(ASSEMBLER), str(raw), "--output", str(result)])
            command(
                [
                    sys.executable,
                    str(PERFORMANCE_EVALUATOR),
                    str(result),
                    "--output",
                    str(output / "performance-evaluation.json"),
                ]
            )
            command(
                [
                    sys.executable,
                    str(AMPLIFICATION_EVALUATOR),
                    str(result),
                    "--output",
                    str(output / "amplification-evaluation.json"),
                ]
            )
            (output / "result.txt").write_text(
                (
                    "PASS\n"
                    f"{args.rounds} balanced alternating rounds; "
                    "performance and amplification gates passed\n"
                ),
                encoding="utf-8",
            )
        validate_existing(
            output,
            args.rounds,
            allow_stale_evidence=args.allow_stale_evidence,
            purpose=args.purpose,
        )
        print(output)
        return 0
    except (OSError, RuntimeError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        print(f"M1 performance validation failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

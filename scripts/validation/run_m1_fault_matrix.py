#!/usr/bin/env python3
"""Run the M1.7 fault-transition matrix over a real three-VM deployment."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import shlex
import subprocess
import time


ROOT = Path(__file__).resolve().parents[2]
SIZE = ROOT / "scripts/validation/run_filesystem_size_semantics_3vm.py"
LOCKS = ROOT / "scripts/validation/run_filesystem_lock_3vm.py"
MMAP = ROOT / "scripts/validation/run_filesystem_mmap_3vm.py"
EVALUATOR = ROOT / "scripts/validation/evaluate_m1_fault_matrix.py"


def command(argv: list[str]) -> None:
    completed = subprocess.run(argv, text=True, capture_output=True, check=False)
    if completed.returncode:
        detail = (completed.stdout or "") + (completed.stderr or "")
        raise RuntimeError(f"command failed ({completed.returncode}): {shlex.join(argv)}\n{detail[-6000:]}")


def run_case(script: Path, args: argparse.Namespace, output: Path, run_id: str, ports: tuple[int, int, int, int]) -> None:
    worker_port, node_health_port, meta_port, meta_health_port = ports
    argv = [
        "python3",
        str(script),
        "--dms-node",
        str(args.dms_node),
        "--dms-meta",
        str(args.dms_meta),
        "--output",
        str(output),
        "--run-id",
        run_id,
        "--vm-a",
        args.vm_a,
        "--vm-b",
        args.vm_b,
        "--vm-c",
        args.vm_c,
        "--ip-a",
        args.ip_a,
        "--ip-b",
        args.ip_b,
        "--ip-c",
        args.ip_c,
        "--worker-port",
        str(worker_port),
        "--node-health-port",
        str(node_health_port),
        "--meta-port",
        str(meta_port),
        "--meta-health-port",
        str(meta_health_port),
    ]
    if script in {LOCKS, MMAP}:
        argv.extend(["--log-level", args.log_level])
    if script == MMAP and args.helper_binary:
        argv.extend(["--helper-binary", str(args.helper_binary)])
    command(argv)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dms-node", type=Path, required=True)
    parser.add_argument("--dms-meta", type=Path, required=True)
    parser.add_argument("--helper-binary", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--run-id", default=f"fault-{int(time.time())}")
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--base-port", type=int, default=32100)
    parser.add_argument("--log-level", default="warn")
    args = parser.parse_args()

    args.output.mkdir(parents=True, exist_ok=False)
    profile = {
        "schema": "dms.m1.fault-matrix-profile.v1",
        "run_id": args.run_id,
        "cases": ["size", "locks", "mmap"],
        "topology": {"vm_a": args.vm_a, "vm_b": args.vm_b, "vm_c": args.vm_c},
    }
    (args.output / "profile.json").write_text(json.dumps(profile, ensure_ascii=False, indent=2) + "\n")

    run_case(SIZE, args, args.output / "size", args.run_id + "-size", (args.base_port + 1, args.base_port + 2, args.base_port + 3, args.base_port + 4))
    run_case(LOCKS, args, args.output / "locks", args.run_id + "-locks", (args.base_port + 11, args.base_port + 12, args.base_port + 13, args.base_port + 14))
    run_case(MMAP, args, args.output / "mmap", args.run_id + "-mmap", (args.base_port + 21, args.base_port + 22, args.base_port + 23, args.base_port + 24))
    command(["python3", str(EVALUATOR), str(args.output), "--output", str(args.output / "evaluation.json")])
    (args.output / "result.txt").write_text("PASS\nfault transition matrix verified\n", encoding="utf-8")
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

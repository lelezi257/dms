#!/usr/bin/env python3
"""Linux W1 v2 local-home comparison; two independent six-round sessions."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

from s5_posix_workload import nearest_rank, run_w1


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mounted(path: Path) -> bool:
    return subprocess.run(["mountpoint", "-q", str(path)], check=False).returncode == 0


def session(roots: dict[str, Path], number: int, seed: int) -> dict:
    names = list(roots)
    warmup = {}
    for index, name in enumerate(names):
        result = run_w1(roots[name], seed=seed + number * 1000 + index, drop_cache=lambda: None)
        if not result["correctness"] or result["timing_contract"] != "dms.s5-w1-timing.v2":
            raise RuntimeError(f"warmup failed: {name}")
        warmup[name] = result["batch_wall_us"]
    rounds = []
    for round_number in range(1, 7):
        order = names[(round_number - 1) % len(names):] + names[: (round_number - 1) % len(names)]
        if round_number > len(names):
            order.reverse()
        cases = {}
        for name in order:
            result = run_w1(
                roots[name], seed=seed + number * 1000 + 100 + round_number,
                drop_cache=lambda: None,
            )
            if not result["correctness"] or result["timing_contract"] != "dms.s5-w1-timing.v2":
                raise RuntimeError(f"round {round_number} failed: {name}")
            cases[name] = result
        rounds.append({"round": round_number, "order": order, "cases": cases})
        print(json.dumps({"session": number, "round": round_number,
                          "batch_wall_us": {name: cases[name]["batch_wall_us"] for name in order}}), flush=True)
    samples = {name: [row["cases"][name]["batch_wall_us"] for row in rounds] for name in names}
    p50 = {name: nearest_rank(values, .50) for name, values in samples.items()}
    p95 = {name: nearest_rank(values, .95) for name, values in samples.items()}
    p99 = {name: nearest_rank(values, .99) for name, values in samples.items()}
    ratio = p50["dms_home"] / p50["moosefs"]
    return {"session": number, "warmup_us": warmup, "rounds": rounds, "samples_us": samples,
            "p50_us": p50, "p95_us": p95, "p99_us": p99,
            "dms_to_moosefs_p50": ratio, "w1_value_pass": ratio <= .80}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dms-home", type=Path, required=True)
    parser.add_argument("--moosefs", type=Path, required=True)
    parser.add_argument("--thin-fuse", type=Path, required=True)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--dms-binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=6701)
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("Linux required")
    roots = {"dms_home": args.dms_home, "thin_fuse": args.thin_fuse,
             "moosefs": args.moosefs, "native_fs": args.native}
    resolved = {name: path.resolve() for name, path in roots.items()}
    if len(set(resolved.values())) != len(resolved):
        parser.error("all backend roots must be distinct")
    for name, path in roots.items():
        if not path.is_dir():
            parser.error(f"missing {name} root: {path}")
        if name != "native_fs" and not mounted(path):
            parser.error(f"{name} is not a mountpoint: {path}")
    if args.output.exists():
        parser.error(f"refusing to overwrite output: {args.output}")
    args.output.mkdir(parents=True)
    result = {
        "schema": "dms.home-preview-local-w1.v1", "status": "IN_PROGRESS",
        "timing_contract": "dms.s5-w1-timing.v2",
        "scope": "local home W1; default MooseFS engineering value, durability differences separate",
        "environment": {"uname": platform.uname()._asdict(), "python": sys.version,
                        "roots": {name: str(path) for name, path in roots.items()},
                        "mounts": subprocess.run(["findmnt", "-rn", "-o", "TARGET,SOURCE,FSTYPE,OPTIONS"],
                                                 capture_output=True, text=True, check=True).stdout},
        "sha256": {"runner": digest(Path(__file__)),
                   "workload": digest(Path(__file__).with_name("s5_posix_workload.py")),
                   "dms_binary": digest(args.dms_binary)},
        "sessions": [],
    }
    try:
        for number in (1, 2):
            item = session(roots, number, args.seed)
            result["sessions"].append(item)
            (args.output / f"session-{number}.json").write_text(json.dumps(item, indent=2) + "\n")
        result["status"] = "PASS" if all(item["w1_value_pass"] for item in result["sessions"]) else "FAIL"
    except Exception as error:
        result["status"] = "ERROR"
        result["error"] = str(error)
        raise
    finally:
        (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"status": result["status"],
                      "ratios": [item["dms_to_moosefs_p50"] for item in result["sessions"]]}))
    if result["status"] != "PASS":
        raise SystemExit(1)


if __name__ == "__main__":
    main()

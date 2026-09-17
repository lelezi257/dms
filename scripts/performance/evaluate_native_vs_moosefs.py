#!/usr/bin/env python3
"""验证 Native Filesystem / MooseFS 结果是否满足可审计摸底合同。"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/native-vs-moosefs-contract.json"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def positive(value: object) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value > 0


def evaluate(contract: dict[str, Any], result: dict[str, Any]) -> dict[str, Any]:
    errors: list[str] = []
    if contract.get("schema") != "dms.native-vs-moosefs-contract.v1":
        errors.append("invalid contract schema")
    if result.get("schema") != "dms.native-vs-moosefs-result.v1":
        errors.append("invalid result schema")
    if result.get("same_environment") is not True:
        errors.append("backends were not measured in the same environment")

    lanes = result.get("lanes", {})
    memory = lanes.get("memory", {})
    disk = lanes.get("disk", {})
    if memory.get("media") != "tmpfs":
        errors.append("memory lane is not identified as tmpfs")
    if disk.get("media") != "vm_virtual_disk":
        errors.append("disk lane is not separately identified as VM virtual disk")
    minimum_rounds = int(contract["evidence"]["minimum_paired_memory_rounds"])
    if not isinstance(memory.get("rounds"), int) or memory.get("rounds", 0) < minimum_rounds:
        errors.append(f"memory lane needs at least {minimum_rounds} paired rounds")
    if not isinstance(disk.get("rounds"), int) or disk.get("rounds", 0) < 1:
        errors.append("disk lane needs at least one separate observational round")

    required_cases = contract["workload"]["required_cases"]
    for lane_name, lane in (("memory", memory), ("disk", disk)):
        for backend_name in ("dms", "moosefs"):
            backend = lane.get("backends", {}).get(backend_name, {})
            if backend.get("correctness") is not True:
                errors.append(f"{lane_name}/{backend_name}: correctness is not proven")
            cases = backend.get("cases", {})
            for case_id in required_cases:
                case = cases.get(case_id)
                if not isinstance(case, dict):
                    errors.append(f"{lane_name}/{backend_name}/{case_id}: missing case")
                    continue
                if case.get("correctness") is not True:
                    errors.append(f"{lane_name}/{backend_name}/{case_id}: correctness failed")
                for field in ("p50_us", "p95_us", "p99_us"):
                    if not positive(case.get(field)):
                        errors.append(f"{lane_name}/{backend_name}/{case_id}: invalid {field}")
                if case_id != "workspace.stat" and not positive(case.get("throughput_mib_s")):
                    errors.append(f"{lane_name}/{backend_name}/{case_id}: invalid throughput")
                resources = case.get("resources", {})
                if not isinstance(resources.get("network"), dict):
                    errors.append(f"{lane_name}/{backend_name}/{case_id}: missing network evidence")
                for field in ("cpu_ticks", "context_switches", "rss_peak_bytes"):
                    if not isinstance(resources.get(field), (int, float)):
                        errors.append(f"{lane_name}/{backend_name}/{case_id}: missing {field}")
                copy = case.get("copy_evidence", {})
                if copy.get("type") != "code_path_model" or not isinstance(copy.get("path"), str):
                    errors.append(f"{lane_name}/{backend_name}/{case_id}: missing copy evidence")
                if not isinstance(case.get("whitebox"), dict):
                    errors.append(f"{lane_name}/{backend_name}/{case_id}: missing whitebox counters")
                if lane_name == "memory":
                    rounds = case.get("rounds")
                    if not isinstance(rounds, list) or len(rounds) < minimum_rounds:
                        errors.append(f"memory/{backend_name}/{case_id}: incomplete paired rounds")

    verdict = result.get("preview_verdict", {})
    if verdict.get("status") not in {"READY", "NOT_READY"}:
        errors.append("Preview verdict is missing")
    if not isinstance(verdict.get("checks"), list) or not verdict.get("checks"):
        errors.append("Preview verdict has no explicit checks")
    analysis = result.get("analysis", {})
    if not analysis.get("architecture_inherent"):
        errors.append("architecture-inherent cost analysis is missing")
    if not analysis.get("implementation_findings"):
        errors.append("implementation cost analysis is missing")

    return {
        "schema": "dms.native-vs-moosefs-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "preview_verdict": verdict.get("status"),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    evaluation = evaluate(load_json(args.contract), load_json(args.result))
    rendered = json.dumps(evaluation, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if evaluation["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

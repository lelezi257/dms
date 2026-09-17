#!/usr/bin/env python3
"""验收 P3 write-through 性能、RPC 合同与推荐 workload 产物。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "benchmarks/whitebox/native-fs-write-through-contract.json"
FUSE_WRITE = re.compile(r"dms_node_fuse_callbacks_total\{operation=write\}")


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def identity(result: dict[str, Any]) -> tuple[str, tuple[tuple[str, str], ...]]:
    environment = result.get("environment", {})
    return (
        str(environment.get("source_sha") or ""),
        tuple(
            sorted(
                (str(name), str(value))
                for name, value in environment.get("resolved_hashes", {}).items()
            )
        ),
    )


def fuse_write_callbacks(case: dict[str, Any]) -> float:
    return sum(
        float(value)
        for name, value in case.get("whitebox", {}).items()
        if FUSE_WRITE.search(name)
    )


def segmented_proof(case: dict[str, Any], contract: dict[str, Any]) -> dict[str, Any]:
    rules = contract["segmented_proof"]
    segments = case.get("segment_mean_us", {})
    missing = [name for name in rules["required_segments"] if name not in segments]
    coverage = float(case.get("segment_coverage_fraction", 0.0))
    commit_count = float(case.get("rpc_counts", {}).get("CommitFilesystemVersion", 0.0))
    callback_count = fuse_write_callbacks(case)
    exact_publish = callback_count > 0.0 and abs(commit_count - callback_count) < 1e-9
    passed = (
        not missing
        and coverage >= float(rules["minimum_coverage_fraction"])
        and exact_publish
    )
    return {
        "status": "PASS" if passed else "FAIL",
        "coverage_fraction": coverage,
        "minimum_coverage_fraction": float(rules["minimum_coverage_fraction"]),
        "missing_segments": missing,
        "commit_count": commit_count,
        "fuse_write_callbacks": callback_count,
        "one_commit_per_callback": exact_publish,
        "reason": rules["reason"],
    }


def evaluate(contract: dict[str, Any], results: list[dict[str, Any]]) -> dict[str, Any]:
    errors: list[str] = []
    proofs: dict[str, Any] = {}
    if contract.get("schema") != "dms.native-fs-write-through-contract.v1":
        errors.append("invalid contract schema")
    required_runs = int(contract["required_independent_runs"])
    if len(results) < required_runs:
        errors.append(f"need {required_runs} independent runs, got {len(results)}")

    seen_run_ids: set[str] = set()
    expected_identity = None
    allowed_background = set(contract["allowed_background_methods"])
    forbidden = set(contract["forbidden_foreground_methods"])
    minimum_ratio = float(contract["minimum_dms_to_moosefs_throughput_ratio"])

    for index, result in enumerate(results, start=1):
        run_id = str(result.get("run_id") or f"run-{index}")
        if run_id in seen_run_ids:
            errors.append(f"{run_id}: duplicate run_id")
        seen_run_ids.add(run_id)
        current_identity = identity(result)
        if not current_identity[0] or not current_identity[1]:
            errors.append(f"{run_id}: missing source or binary identity")
        if expected_identity is None:
            expected_identity = current_identity
        elif current_identity != expected_identity:
            errors.append(f"{run_id}: source or binary identity differs from first run")

        if result.get("schema") != "dms.native-fs-write-through-result.v1":
            errors.append(f"{run_id}: invalid result schema")
        semantics = result.get("semantics", {})
        if semantics.get("writeback") is not False:
            errors.append(f"{run_id}: writeback must remain disabled")
        if semantics.get("operation") != contract["semantics"]["operation"]:
            errors.append(f"{run_id}: comparison operation differs from contract")
        for section in contract["required_machine_sections"]:
            if not result.get(section):
                errors.append(f"{run_id}: missing machine section {section}")

        backends = result.get("backends", {})
        dms_cases = backends.get("dms", {}).get("cases", {})
        mfs_cases = backends.get("moosefs", {}).get("cases", {})
        if not backends.get("dms", {}).get("correctness"):
            errors.append(f"{run_id}: DMS correctness failed")
        if not backends.get("moosefs", {}).get("correctness"):
            errors.append(f"{run_id}: MooseFS correctness failed")

        for case_id in contract["large_write_cases"]:
            dms = dms_cases.get(case_id)
            mfs = mfs_cases.get(case_id)
            if not isinstance(dms, dict) or not isinstance(mfs, dict):
                errors.append(f"{run_id}/{case_id}: missing case")
                continue
            ratio = float(dms.get("throughput_mib_s", 0.0)) / max(
                float(mfs.get("throughput_mib_s", 0.0)), 1e-12
            )
            proof = segmented_proof(dms, contract)
            proof["throughput_ratio"] = ratio
            proofs[f"{run_id}/{case_id}"] = proof
            if ratio < minimum_ratio and proof["status"] != "PASS":
                errors.append(
                    f"{run_id}/{case_id}: throughput ratio {ratio:.3f} below {minimum_ratio:.3f} "
                    f"and segmented proof failed"
                )

        write_cases = [
            case_id
            for case_id in dms_cases
            if case_id.startswith("sync_write.no_holder")
            or case_id.startswith("sync_write.holder")
        ]
        for case_id in write_cases:
            case = dms_cases[case_id]
            callbacks = fuse_write_callbacks(case)
            commits = float(case.get("rpc_counts", {}).get("CommitFilesystemVersion", 0.0))
            if callbacks <= 0.0 or abs(callbacks - commits) > 1e-9:
                errors.append(
                    f"{run_id}/{case_id}: Fuse write callbacks {callbacks:g} != commits {commits:g}"
                )
            counts = case.get("rpc_counts", {})
            for method in forbidden:
                if float(counts.get(method, 0.0)) > 0.0:
                    errors.append(f"{run_id}/{case_id}: forbidden foreground RPC {method}")
            if ".no_holder." in case_id and float(
                counts.get("AcknowledgeNodeEvent", 0.0)
            ) > 0.0:
                errors.append(f"{run_id}/{case_id}: no-holder write unexpectedly waited for ACK")

        ack_limits = contract["holder_ack_per_logical_write"]
        for case_id in contract["holder_cases"]:
            case = dms_cases.get(case_id)
            if not isinstance(case, dict):
                errors.append(f"{run_id}/{case_id}: missing holder case")
                continue
            samples = max(int(case.get("samples", 0)), 1)
            per_sample = float(case.get("rpc_counts", {}).get("AcknowledgeNodeEvent", 0.0)) / samples
            if per_sample < float(ack_limits["minimum"]) or per_sample > float(
                ack_limits["maximum"]
            ):
                errors.append(
                    f"{run_id}/{case_id}: ACK per logical write {per_sample:.3f} outside "
                    f"[{ack_limits['minimum']}, {ack_limits['maximum']}]"
                )

        for case_id in contract["stable_cases"]:
            case = dms_cases.get(case_id)
            if not isinstance(case, dict):
                errors.append(f"{run_id}/{case_id}: missing stable case")
                continue
            counts = case.get("rpc_counts", {})
            foreground = {
                method: count
                for method, count in counts.items()
                if count > 0.0 and method not in allowed_background
            }
            if foreground:
                errors.append(f"{run_id}/{case_id}: stable path RPCs {foreground}")
    return {
        "schema": "dms.native-fs-write-through-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "accepted_via": "throughput_or_exact_write_through_segmented_proof",
        "errors": errors,
        "segmented_proofs": proofs,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("results", nargs="+", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    evaluation = evaluate(load_json(args.contract), [load_json(path) for path in args.results])
    rendered = json.dumps(evaluation, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if evaluation["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

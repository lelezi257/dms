#!/usr/bin/env python3
"""M1.3 文件身份与生命周期 E2E 判定器。

判定器只消费 workload/runner 产出的 JSON、metrics 快照和结构化日志证据，不调用
DMS 内部调试接口。功能语义、引用迁移或 durable reclaim 缺证据都会失败，避免把
“POSIX 表面成功”伪装成引用保护和持久回收已经同时成立。
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


DEFAULT_CONTRACT: dict[str, Any] = {
    "schema": "dms.filesystem.identity-lifecycle-contract.v1",
    "required_operations": [
        "hardlink_same_inode",
        "unlink_one_link_keeps_other_link",
        "unlink_open_keeps_file_readable_until_close",
        "symlink_readlink_exact_target",
        "orphan_lifecycle_observed",
    ],
    "require_reference_and_reap_evidence": True,
    "require_recovery": True,
}


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_contract(path: Path | None) -> dict[str, Any]:
    if path is None:
        return dict(DEFAULT_CONTRACT)
    return load_json(path)


def metric_value(prometheus: str, needle: str) -> float:
    total = 0.0
    for line in prometheus.splitlines():
        if line.startswith("#") or not line.startswith(needle):
            continue
        try:
            total += float(line.rsplit(maxsplit=1)[1])
        except (IndexError, ValueError):
            pass
    return total


def operation_map(workload: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        str(check.get("operation")): check
        for check in workload.get("checks", [])
        if check.get("operation") is not None
    }


def require_equals(
    check: dict[str, Any],
    field: str,
    expected: Any,
    errors: list[str],
    label: str,
) -> None:
    actual = check.get(field)
    if actual != expected:
        errors.append(f"{label}: expected {field}={expected!r}, got {actual!r}")


def check_functional_semantics(checks: dict[str, dict[str, Any]], errors: list[str]) -> None:
    hardlink = checks.get("hardlink_same_inode", {})
    require_equals(hardlink, "same_inode", True, errors, "hardlink_same_inode")
    require_equals(hardlink, "source_nlink", 2, errors, "hardlink_same_inode")
    require_equals(hardlink, "linked_nlink", 2, errors, "hardlink_same_inode")
    if hardlink.get("remote_stat_attempts") not in (None, 1):
        errors.append("hardlink_same_inode: remote visibility was not first-check")

    one_link = checks.get("unlink_one_link_keeps_other_link", {})
    require_equals(one_link, "removed_exists", False, errors, "unlink_one_link_keeps_other_link")
    require_equals(one_link, "remaining_nlink", 1, errors, "unlink_one_link_keeps_other_link")
    if int(one_link.get("remaining_bytes", 0)) <= 0:
        errors.append("unlink_one_link_keeps_other_link: remaining link did not read payload")

    unlink_open = checks.get("unlink_open_keeps_file_readable_until_close", {})
    require_equals(unlink_open, "namespace_visible_after_unlink", False, errors, "unlink_open")
    if int(unlink_open.get("fd_read_bytes", 0)) <= 0:
        errors.append("unlink_open: opened file handle did not return payload after unlink")
    require_equals(unlink_open, "close_completed", True, errors, "unlink_open")

    symlink = checks.get("symlink_readlink_exact_target", {})
    require_equals(symlink, "readlink_matches", True, errors, "symlink_readlink_exact_target")
    require_equals(symlink, "target_survived_unlink", True, errors, "symlink_readlink_exact_target")
    # 按已定设计，symlink target 作为 DataCore exact ObjectVersion 发布；readlink 读取
    # target bytes 是允许且必要的。这里不禁止 payload transfer，只检查结果语义。
    if int(symlink.get("readlink_bytes", 0)) <= 0:
        errors.append("symlink_readlink_exact_target: readlink returned an empty target")

    orphan = checks.get("orphan_lifecycle_observed", {})
    require_equals(orphan, "namespace_visible_after_last_unlink", False, errors, "orphan_lifecycle")
    if orphan.get("open_ref_protected_read") is not True:
        errors.append("orphan_lifecycle: open reference did not protect read after last unlink")


def check_recovery(
    workload: dict[str, Any],
    recovery: dict[str, Any],
    require_recovery: bool,
    errors: list[str],
) -> None:
    if not require_recovery:
        return
    if recovery.get("schema") != "dms.filesystem.identity-lifecycle-recovery.v1":
        errors.append("invalid recovery schema")
        return
    if recovery.get("hardlink_remaining_path") != workload.get("recovery_hardlink_path"):
        errors.append("recovery hardlink path does not match workload anchor")
    if recovery.get("hardlink_bytes") != workload.get("recovery_hardlink_bytes"):
        errors.append("recovery hardlink bytes do not match workload anchor")
    if recovery.get("symlink_path") != workload.get("recovery_symlink_path"):
        errors.append("recovery symlink path does not match workload anchor")
    if recovery.get("readlink") != workload.get("recovery_symlink_target"):
        errors.append("recovery symlink target does not match workload anchor")
    if recovery.get("orphan_namespace_visible") is not False:
        errors.append("recovery resurrected an unlinked orphan in namespace")


def collect_observability(
    result_dir: Path,
    require_reference_and_reap_evidence: bool,
    errors: list[str],
) -> dict[str, Any]:
    gaps: list[str] = []
    metrics_present = True
    for filename, metric in (
        ("node-a.prom", "dms_node_filesystem_operations_total"),
        ("node-b.prom", "dms_node_filesystem_operations_total"),
        ("meta.prom", "dms_meta_operations_total"),
    ):
        path = result_dir / filename
        if not path.is_file():
            metrics_present = False
            errors.append(f"missing metrics snapshot: {filename}")
            continue
        if metric_value(path.read_text(encoding="utf-8"), metric) <= 0:
            metrics_present = False
            errors.append(f"{filename}: metric {metric} did not record filesystem work")

    node_a = result_dir / "node-a.prom"
    if node_a.is_file():
        node_metrics = node_a.read_text(encoding="utf-8")
        if metric_value(
            node_metrics,
            'dms_node_filesystem_inode_reference_transitions_total{transition="release_final"}',
        ) <= 0:
            errors.append("node-a.prom: no final inode reference release was recorded")
    meta = result_dir / "meta.prom"
    if meta.is_file():
        meta_metrics = meta.read_text(encoding="utf-8")
        if metric_value(
            meta_metrics,
            'dms_meta_journal_appends_total{record_type="filesystem_orphan_reaped",result="ok"}',
        ) <= 0:
            errors.append("meta.prom: no durable filesystem orphan reap was recorded")

    whitebox_path = result_dir / "identity-whitebox.json"
    whitebox_prerequisites_seen = False
    if whitebox_path.is_file():
        whitebox = load_json(whitebox_path)
        whitebox_prerequisites_seen = bool(
            whitebox.get("reference_lifecycle_and_reap_seen")
        )
        if require_reference_and_reap_evidence and not whitebox_prerequisites_seen:
            errors.append("whitebox reference lifecycle and durable reap were not both observed")
    else:
        gaps.append(
            "missing identity-whitebox.json; reference lifecycle and durable reap not machine-proven"
        )
        if require_reference_and_reap_evidence:
            errors.append("missing required whitebox reference and reap evidence")

    for filename in ("node-a.log", "node-b.log", "meta.log"):
        if not (result_dir / filename).is_file():
            gaps.append(f"missing log snapshot: {filename}")

    return {
        "metrics_present": metrics_present,
        "whitebox_prerequisites_seen": whitebox_prerequisites_seen,
        "evidence_gaps": gaps,
    }


def evaluate(contract: dict[str, Any], result_dir: Path) -> dict[str, Any]:
    errors: list[str] = []
    if contract.get("schema") != "dms.filesystem.identity-lifecycle-contract.v1":
        errors.append("invalid contract schema")

    workload_path = result_dir / "identity-workload.json"
    recovery_path = result_dir / "identity-recovery.json"
    if not workload_path.is_file():
        errors.append("missing identity-workload.json")
        workload: dict[str, Any] = {}
    else:
        workload = load_json(workload_path)
    if not recovery_path.is_file():
        errors.append("missing identity-recovery.json")
        recovery: dict[str, Any] = {}
    else:
        recovery = load_json(recovery_path)

    if workload.get("schema") != "dms.filesystem.identity-lifecycle-workload.v1":
        errors.append("invalid workload schema")
    checks = operation_map(workload)
    required = set(contract.get("required_operations", DEFAULT_CONTRACT["required_operations"]))
    missing = sorted(required - set(checks))
    if missing:
        errors.append(f"missing operations: {missing}")

    expected_count = len(required)
    if workload.get("passed_operations") != expected_count:
        errors.append(
            f"expected {expected_count} passed operations, got {workload.get('passed_operations')!r}"
        )

    check_functional_semantics(checks, errors)
    check_recovery(
        workload,
        recovery,
        bool(contract.get("require_recovery", True)),
        errors,
    )
    observability = collect_observability(
        result_dir,
        bool(contract.get("require_reference_and_reap_evidence", False)),
        errors,
    )

    return {
        "schema": "dms.filesystem.identity-lifecycle-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "rows": [
            {
                "operation": operation,
                "summary": {key: value for key, value in check.items() if key != "operation"},
            }
            for operation, check in sorted(checks.items())
        ],
        "observability": observability,
        "result_dir": str(result_dir),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_dir", type=Path)
    parser.add_argument("--contract", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = evaluate(load_contract(args.contract), args.result_dir)
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

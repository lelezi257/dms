#!/usr/bin/env python3
"""M1 产品级总验收合同校验与结果评估器。

这个文件只负责编排和判定，不复制各阶段已有的业务测试逻辑。具体测试仍由
acceptance-manifest.json 中声明的脚本执行；本工具解决的是“测了什么、是否漏测、
能否据此宣称 M1 通过”三个问题。
"""

from __future__ import annotations

import argparse
import datetime as dt
import html
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
MANIFEST_PATH = HERE / "acceptance-manifest.json"
KNOWN_GAPS_PATH = HERE / "known-gaps.json"
RESULT_SCHEMA_PATH = HERE / "result.schema.json"

VALID_IMPLEMENTATION_STATUS = {"implemented", "planned"}
VALID_TIERS = {"fast", "full"}
VALID_TOPOLOGIES = {"single-vm", "three-vm"}
VALID_CASE_STATUS = {"PASS", "FAIL", "SKIP"}
VALID_PURPOSES = {"discovery", "release"}


class ContractError(ValueError):
    """验收合同本身不完整或互相矛盾。"""


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as stream:
        value = json.load(stream)
    if not isinstance(value, dict):
        raise ContractError(f"{path} 的顶层必须是 JSON object")
    return value


def load_contract() -> tuple[dict[str, Any], dict[str, Any]]:
    return load_json(MANIFEST_PATH), load_json(KNOWN_GAPS_PATH)


def _require_non_empty_string(value: Any, location: str) -> None:
    if not isinstance(value, str) or not value.strip():
        raise ContractError(f"{location} 必须是非空字符串")


def validate_contract(
    manifest: dict[str, Any],
    known_gaps: dict[str, Any],
    root: Path = ROOT,
) -> list[str]:
    """校验验收合同，并返回便于人阅读的摘要。"""

    requirements = manifest.get("requirements")
    cases = manifest.get("cases")
    policy = manifest.get("policy")
    if not isinstance(requirements, list) or not requirements:
        raise ContractError("requirements 必须是非空数组")
    if not isinstance(cases, list) or not cases:
        raise ContractError("cases 必须是非空数组")
    if not isinstance(policy, dict):
        raise ContractError("policy 必须是 object")

    requirement_ids: set[str] = set()
    for index, requirement in enumerate(requirements):
        if not isinstance(requirement, dict):
            raise ContractError(f"requirements[{index}] 必须是 object")
        requirement_id = requirement.get("id")
        _require_non_empty_string(requirement_id, f"requirements[{index}].id")
        if requirement_id in requirement_ids:
            raise ContractError(f"重复 requirement id: {requirement_id}")
        requirement_ids.add(requirement_id)
        _require_non_empty_string(requirement.get("stage"), f"{requirement_id}.stage")
        _require_non_empty_string(requirement.get("claim"), f"{requirement_id}.claim")

    case_ids: set[str] = set()
    covered_requirements: set[str] = set()
    for index, case in enumerate(cases):
        if not isinstance(case, dict):
            raise ContractError(f"cases[{index}] 必须是 object")
        case_id = case.get("id")
        _require_non_empty_string(case_id, f"cases[{index}].id")
        if case_id in case_ids:
            raise ContractError(f"重复 case id: {case_id}")
        case_ids.add(case_id)

        status = case.get("implementation_status")
        if status not in VALID_IMPLEMENTATION_STATUS:
            raise ContractError(f"{case_id}.implementation_status 非法: {status}")

        requirement_refs = case.get("requirement_ids")
        if not isinstance(requirement_refs, list) or not requirement_refs:
            raise ContractError(f"{case_id}.requirement_ids 必须是非空数组")
        unknown_requirements = set(requirement_refs) - requirement_ids
        if unknown_requirements:
            raise ContractError(f"{case_id} 引用了未知 requirement: {sorted(unknown_requirements)}")
        covered_requirements.update(requirement_refs)

        tiers = case.get("tiers")
        if not isinstance(tiers, list) or not tiers or not set(tiers) <= VALID_TIERS:
            raise ContractError(f"{case_id}.tiers 非法: {tiers}")
        topologies = case.get("topologies")
        if (
            not isinstance(topologies, list)
            or not topologies
            or not set(topologies) <= VALID_TOPOLOGIES
        ):
            raise ContractError(f"{case_id}.topologies 非法: {topologies}")

        executor = case.get("executor")
        _require_non_empty_string(executor, f"{case_id}.executor")
        if status == "implemented" and not (root / executor).is_file():
            raise ContractError(f"已实现用例缺少执行入口: {case_id} -> {executor}")
        _require_non_empty_string(case.get("expected"), f"{case_id}.expected")
        invariants = case.get("invariants")
        if not isinstance(invariants, list) or not invariants:
            raise ContractError(f"{case_id}.invariants 必须是非空数组")

    missing_coverage = requirement_ids - covered_requirements
    if missing_coverage:
        raise ContractError(f"没有测试覆盖的 requirement: {sorted(missing_coverage)}")

    exemptions = known_gaps.get("exemptions", [])
    if not isinstance(exemptions, list):
        raise ContractError("known-gaps.exemptions 必须是数组")
    for index, exemption in enumerate(exemptions):
        if not isinstance(exemption, dict):
            raise ContractError(f"exemptions[{index}] 必须是 object")
        case_id = exemption.get("case_id")
        if case_id not in case_ids:
            raise ContractError(f"exemption 引用了未知 case: {case_id}")
        for field in ("issue", "reason", "expires_at_milestone"):
            _require_non_empty_string(exemption.get(field), f"exemption[{case_id}].{field}")

    if not RESULT_SCHEMA_PATH.is_file():
        raise ContractError(f"缺少结果 schema: {RESULT_SCHEMA_PATH}")

    implemented = sum(case["implementation_status"] == "implemented" for case in cases)
    planned = len(cases) - implemented
    return [
        f"requirements={len(requirements)}",
        f"cases={len(cases)}",
        f"implemented={implemented}",
        f"planned={planned}",
        f"exemptions={len(exemptions)}",
    ]


def select_cases(manifest: dict[str, Any], tier: str, topology: str) -> list[dict[str, Any]]:
    if tier not in VALID_TIERS:
        raise ContractError(f"未知 tier: {tier}")
    if topology not in VALID_TOPOLOGIES:
        raise ContractError(f"未知 topology: {topology}")
    return [
        case
        for case in manifest["cases"]
        if tier in case["tiers"] and topology in case["topologies"]
    ]


def git_value(*args: str) -> str:
    completed = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return completed.stdout.strip()


def source_identity() -> dict[str, Any]:
    dirty = bool(git_value("status", "--porcelain"))
    return {
        "commit": git_value("rev-parse", "HEAD"),
        "branch": git_value("branch", "--show-current"),
        "dirty": dirty,
    }


def build_plan(manifest: dict[str, Any], tier: str, topology: str) -> dict[str, Any]:
    selected = select_cases(manifest, tier, topology)
    planned = [case["id"] for case in selected if case["implementation_status"] == "planned"]
    return {
        "schema": "dms.m1.acceptance-plan.v1",
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "milestone": manifest["milestone"],
        "tier": tier,
        "topology": topology,
        "ready": not planned,
        "planned_cases": planned,
        "cases": [
            {
                "id": case["id"],
                "title": case["title"],
                "category": case["category"],
                "implementation_status": case["implementation_status"],
                "executor": case["executor"],
                "expected": case["expected"],
            }
            for case in selected
        ],
    }


def _validate_result_shape(result: dict[str, Any]) -> None:
    if result.get("schema") != "dms.m1.acceptance-result.v1":
        raise ContractError("结果 schema 必须是 dms.m1.acceptance-result.v1")
    if result.get("purpose") not in VALID_PURPOSES:
        raise ContractError(f"结果 purpose 非法: {result.get('purpose')}")
    if result.get("tier") not in VALID_TIERS:
        raise ContractError(f"结果 tier 非法: {result.get('tier')}")
    if result.get("topology") not in VALID_TOPOLOGIES:
        raise ContractError(f"结果 topology 非法: {result.get('topology')}")
    source = result.get("source")
    if not isinstance(source, dict) or not isinstance(source.get("dirty"), bool):
        raise ContractError("结果 source.dirty 必须存在且为 bool")
    _require_non_empty_string(source.get("commit"), "result.source.commit")
    environment = result.get("environment")
    if not isinstance(environment, dict):
        raise ContractError("结果 environment 必须存在且为 object")
    for field in ("kernel", "arch", "profile"):
        _require_non_empty_string(environment.get(field), f"result.environment.{field}")
    cases = result.get("cases")
    if not isinstance(cases, list):
        raise ContractError("结果 cases 必须是数组")
    for index, case in enumerate(cases):
        if not isinstance(case, dict):
            raise ContractError(f"result.cases[{index}] 必须是 object")
        _require_non_empty_string(case.get("id"), f"result.cases[{index}].id")
        if case.get("status") not in VALID_CASE_STATUS:
            raise ContractError(f"result case {case.get('id')} status 非法")
        evidence = case.get("evidence")
        if not isinstance(evidence, list) or not evidence:
            raise ContractError(f"result case {case.get('id')} 必须提供 evidence")
        if not all(isinstance(item, str) and item.strip() for item in evidence):
            raise ContractError(f"result case {case.get('id')} evidence 必须是非空字符串数组")
        duration_ms = case.get("duration_ms")
        if duration_ms is not None and (
            not isinstance(duration_ms, (int, float)) or duration_ms < 0
        ):
            raise ContractError(f"result case {case.get('id')} duration_ms 非法")


def evaluate(
    manifest: dict[str, Any],
    known_gaps: dict[str, Any],
    result: dict[str, Any],
) -> dict[str, Any]:
    """把一次运行结果转换成 PASS / INCOMPLETE / FAIL。"""

    _validate_result_shape(result)
    tier = result["tier"]
    topology = result["topology"]
    purpose = result["purpose"]
    selected = select_cases(manifest, tier, topology)
    selected_by_id = {case["id"]: case for case in selected}
    implemented_ids = {
        case["id"] for case in selected if case["implementation_status"] == "implemented"
    }
    planned_ids = {
        case["id"] for case in selected if case["implementation_status"] == "planned"
    }
    exemption_by_case = {
        exemption["case_id"]: exemption for exemption in known_gaps.get("exemptions", [])
    }

    errors: list[str] = []
    warnings: list[str] = []
    seen: set[str] = set()
    counts = {"PASS": 0, "FAIL": 0, "SKIP": 0}

    for case_result in result["cases"]:
        case_id = case_result["id"]
        if case_id in seen:
            errors.append(f"重复结果: {case_id}")
            continue
        seen.add(case_id)
        if case_id not in selected_by_id:
            errors.append(f"本次计划之外的结果: {case_id}")
            continue
        status = case_result["status"]
        counts[status] += 1
        if status == "FAIL":
            errors.append(f"用例失败: {case_id}")
        elif status == "SKIP":
            exemption = exemption_by_case.get(case_id)
            if purpose == "release":
                errors.append(f"发布验收不允许跳过: {case_id}")
            elif exemption is None:
                errors.append(f"跳过项没有带 Issue 和到期里程碑的豁免: {case_id}")
            else:
                warnings.append(f"发现模式豁免跳过: {case_id} ({exemption['issue']})")

    missing = implemented_ids - seen
    if missing:
        errors.append(f"缺少已实现用例结果: {sorted(missing)}")

    if planned_ids:
        message = f"仍有未实现验收项: {sorted(planned_ids)}"
        if purpose == "release" or not manifest["policy"]["release_allows_planned_cases"]:
            if purpose == "release":
                errors.append(message)
            else:
                warnings.append(message)

    if purpose == "release" and manifest["policy"]["release_requires_clean_source"]:
        if result["source"]["dirty"]:
            errors.append("发布验收要求源码目录无未提交修改")

    status = "FAIL" if errors else ("INCOMPLETE" if warnings or planned_ids else "PASS")
    return {
        "schema": "dms.m1.acceptance-evaluation.v1",
        "evaluated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "status": status,
        "purpose": purpose,
        "tier": tier,
        "topology": topology,
        "source": result["source"],
        "counts": counts,
        "required_implemented_cases": len(implemented_ids),
        "planned_cases": sorted(planned_ids),
        "errors": errors,
        "warnings": warnings,
        "case_results": [
            {
                "id": item["id"],
                "status": item["status"],
                "duration_ms": item.get("duration_ms"),
                "message": item.get("message", ""),
                "evidence": item["evidence"],
            }
            for item in result["cases"]
        ],
    }


def render_html(evaluation: dict[str, Any]) -> str:
    status = html.escape(evaluation["status"])
    errors = "".join(f"<li>{html.escape(item)}</li>" for item in evaluation["errors"])
    warnings = "".join(f"<li>{html.escape(item)}</li>" for item in evaluation["warnings"])

    def duration_cell(item: dict[str, Any]) -> str:
        duration = item.get("duration_ms")
        if duration is None:
            return ""
        return html.escape(f"{duration:.3f} ms")

    case_rows = "".join(
        "<tr>"
        f"<td><code>{html.escape(item['id'])}</code></td>"
        f"<td class=\"{html.escape(item['status'])}\">{html.escape(item['status'])}</td>"
        f"<td>{duration_cell(item)}</td>"
        f"<td>{html.escape(item.get('message', ''))}</td>"
        f"<td>{len(item.get('evidence', []))}</td>"
        "</tr>"
        for item in evaluation.get("case_results", [])
    )
    raw = html.escape(json.dumps(evaluation, ensure_ascii=False, indent=2))
    return f"""<!doctype html>
<html lang=\"zh-CN\"><head><meta charset=\"utf-8\">
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">
<title>DMS M1 验收结果</title>
<style>body{{font:16px/1.6 system-ui;margin:40px;max-width:1200px}}code,pre{{background:#f5f5f5}}pre{{padding:16px;overflow:auto}}table{{border-collapse:collapse;width:100%}}th,td{{border:1px solid #ddd;padding:8px;text-align:left}}.status{{font-size:2rem;font-weight:700}}.FAIL{{color:#b42318}}.PASS{{color:#067647}}.SKIP,.INCOMPLETE{{color:#b54708}}</style>
</head><body><h1>DMS M1 验收结果</h1>
<p class=\"status {status}\">{status}</p>
<p>模式：{html.escape(evaluation['purpose'])} / {html.escape(evaluation['tier'])} / {html.escape(evaluation['topology'])}</p>
<h2>用例结果</h2><table><thead><tr><th>用例</th><th>状态</th><th>耗时</th><th>说明</th><th>证据数</th></tr></thead><tbody>{case_rows}</tbody></table>
<h2>错误</h2><ul>{errors or '<li>无</li>'}</ul>
<h2>警告</h2><ul>{warnings or '<li>无</li>'}</ul>
<h2>机器结果</h2><pre>{raw}</pre></body></html>"""


def write_json(path: Path | None, value: dict[str, Any]) -> None:
    rendered = json.dumps(value, ensure_ascii=False, indent=2) + "\n"
    if path is None:
        sys.stdout.write(rendered)
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(rendered, encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("validate", help="校验 manifest、known gaps 和执行入口")

    plan = subparsers.add_parser("plan", help="生成指定 tier/topology 的执行计划")
    plan.add_argument("--tier", choices=sorted(VALID_TIERS), required=True)
    plan.add_argument("--topology", choices=sorted(VALID_TOPOLOGIES), required=True)
    plan.add_argument("--output", type=Path)

    evaluate_parser = subparsers.add_parser("evaluate", help="评估一次机器结果")
    evaluate_parser.add_argument("result", type=Path)
    evaluate_parser.add_argument("--output", type=Path)

    render = subparsers.add_parser("render", help="把 evaluation JSON 渲染为 HTML")
    render.add_argument("evaluation", type=Path)
    render.add_argument("output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest, known_gaps = load_contract()
    try:
        summary = validate_contract(manifest, known_gaps)
        if args.command == "validate":
            print("M1 acceptance contract is valid: " + ", ".join(summary))
            return 0
        if args.command == "plan":
            plan = build_plan(manifest, args.tier, args.topology)
            write_json(args.output, plan)
            return 0 if plan["ready"] else 2
        if args.command == "evaluate":
            evaluation = evaluate(manifest, known_gaps, load_json(args.result))
            write_json(args.output, evaluation)
            return 0 if evaluation["status"] == "PASS" else 2
        if args.command == "render":
            evaluation = load_json(args.evaluation)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(render_html(evaluation), encoding="utf-8")
            return 0
    except (ContractError, json.JSONDecodeError, OSError, subprocess.CalledProcessError) as error:
        print(f"M1 acceptance error: {error}", file=sys.stderr)
        return 1
    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())

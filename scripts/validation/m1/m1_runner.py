#!/usr/bin/env python3
"""顺序执行一组 M1 验收用例，并生成统一、可审计的结果。

业务用例仍拥有自己的启动参数和证据格式。本文件只负责四件事：

1. 从 execution profile 取得明确的命令、环境和超时；
2. 保存每个用例的 stdout、stderr、退出码和耗时；
3. 校验 profile 声明的关键证据确实存在；
4. 生成 ``dms.m1.acceptance-result.v1``，再调用合同评估器判定。

这样总验收不会复制各业务脚本，也不会靠解析终端里的一句 ``PASS`` 猜结果。
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import platform
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Any

MODULE_DIR = Path(__file__).resolve().parent
if str(MODULE_DIR) not in sys.path:
    sys.path.insert(0, str(MODULE_DIR))

import m1_acceptance as acceptance


PROFILE_SCHEMA = "dms.m1.execution-profile.v1"


class ProfileError(ValueError):
    """Execution profile 缺字段、引用未知用例或包含不安全值。"""


def make_run_nonce() -> str:
    """为一次总验收生成可安全用于远端目录和进程名的唯一后缀。

    各业务脚本会在 VM 的 ``/tmp`` 下建立运行目录。若 profile 只使用固定
    ``case_id``，第二次执行同一验收就可能误撞上一次现场。时间戳便于人工定位，
    UUID 后缀保证同一微秒内并发启动也不会冲突。
    """

    # 下游还会把 case 名和 ``-node-a`` / ``-meta`` 拼到这个值上。这里保持短小，
    # 避免目录唯一性修复反而突破 DMS NodeId 的 64 字节协议上限。
    # 只使用小写字母、数字和短横线。这个 nonce 还会成为 JuiceFS volume、
    # DMS NodeId 和 Unix socket 路径的一部分，采用三方约束的最小交集可避免
    # 到具体用例才发现某一端不接受大写 ``T/Z``。
    timestamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%d-%H%M%S")
    return f"{timestamp}-{uuid.uuid4().hex[:6]}"


def load_profile(path: Path) -> dict[str, Any]:
    profile = acceptance.load_json(path)
    if profile.get("schema") != PROFILE_SCHEMA:
        raise ProfileError(f"profile schema 必须是 {PROFILE_SCHEMA}")
    if not isinstance(profile.get("name"), str) or not profile["name"].strip():
        raise ProfileError("profile.name 必须是非空字符串")
    commands = profile.get("commands")
    if not isinstance(commands, dict):
        raise ProfileError("profile.commands 必须是 object")
    variables = profile.get("variables", {})
    if not isinstance(variables, dict) or not all(
        isinstance(key, str) and isinstance(value, (str, int, float))
        for key, value in variables.items()
    ):
        raise ProfileError("profile.variables 只能包含字符串 key 和标量 value")
    return profile


def parse_variable_overrides(values: list[str]) -> dict[str, str]:
    """解析 ``--variable NAME=VALUE``，供不同 VM/制品路径复用同一 profile。"""

    result: dict[str, str] = {}
    for value in values:
        name, separator, rendered = value.partition("=")
        if not separator or not name.strip():
            raise ProfileError(f"变量覆盖必须使用 NAME=VALUE: {value!r}")
        result[name] = rendered
    return result


def _render(value: str, variables: dict[str, str]) -> str:
    try:
        return value.format_map(variables)
    except KeyError as error:
        raise ProfileError(f"未知模板变量: {error.args[0]}") from error


def resolve_variables(values: dict[str, str]) -> dict[str, str]:
    """解析 profile 变量之间的引用，并拒绝循环或未定义引用。"""

    resolved = dict(values)
    for _ in range(len(resolved) + 1):
        changed = False
        for key, value in tuple(resolved.items()):
            try:
                rendered = value.format_map(resolved)
            except KeyError:
                continue
            if rendered != value:
                resolved[key] = rendered
                changed = True
        if not changed:
            break
    for key, value in resolved.items():
        try:
            value.format_map(resolved)
        except KeyError as error:
            raise ProfileError(f"变量 {key!r} 引用了未知模板变量: {error.args[0]}") from error
        if "{" in value or "}" in value:
            raise ProfileError(f"变量 {key!r} 无法完全解析，可能存在循环引用: {value!r}")
    return resolved


def _case_command(
    case_id: str,
    raw: dict[str, Any],
    variables: dict[str, str],
) -> tuple[list[str], dict[str, str], Path, float, list[Path]]:
    argv = raw.get("argv")
    if not isinstance(argv, list) or not argv or not all(isinstance(item, str) for item in argv):
        raise ProfileError(f"{case_id}.argv 必须是非空字符串数组")
    env_value = raw.get("env", {})
    if not isinstance(env_value, dict) or not all(
        isinstance(key, str) and isinstance(value, str) for key, value in env_value.items()
    ):
        raise ProfileError(f"{case_id}.env 必须是 string -> string object")
    timeout = raw.get("timeout_seconds", 1800)
    if not isinstance(timeout, (int, float)) or timeout <= 0:
        raise ProfileError(f"{case_id}.timeout_seconds 必须大于 0")
    expected = raw.get("expected_evidence", [])
    if not isinstance(expected, list) or not all(isinstance(item, str) for item in expected):
        raise ProfileError(f"{case_id}.expected_evidence 必须是字符串数组")
    cwd_value = raw.get("cwd", "{root}")
    if not isinstance(cwd_value, str):
        raise ProfileError(f"{case_id}.cwd 必须是字符串")
    return (
        [_render(item, variables) for item in argv],
        {key: _render(value, variables) for key, value in env_value.items()},
        Path(_render(cwd_value, variables)),
        float(timeout),
        [Path(_render(item, variables)) for item in expected],
    )


def validate_profile(
    profile: dict[str, Any],
    manifest: dict[str, Any],
    tier: str,
    topology: str,
) -> list[dict[str, Any]]:
    selected = acceptance.select_cases(manifest, tier, topology)
    planned = [case["id"] for case in selected if case["implementation_status"] != "implemented"]
    if planned:
        raise ProfileError(f"仍有 planned 用例，不能执行完整验收: {planned}")
    selected_ids = {case["id"] for case in selected}
    command_ids = set(profile["commands"])
    missing = selected_ids - command_ids
    unknown = command_ids - {case["id"] for case in manifest["cases"]}
    if missing:
        raise ProfileError(f"profile 缺少命令: {sorted(missing)}")
    if unknown:
        raise ProfileError(f"profile 引用了未知用例: {sorted(unknown)}")
    return selected


def _write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def run_case(
    case: dict[str, Any],
    raw_command: dict[str, Any],
    common_variables: dict[str, str],
    run_dir: Path,
) -> dict[str, Any]:
    case_id = case["id"]
    case_dir = run_dir / "cases" / case_id
    case_dir.mkdir(parents=True, exist_ok=False)
    variables = dict(common_variables)
    try:
        case_rel_dir = case_dir.relative_to(acceptance.ROOT)
    except ValueError:
        case_rel_dir = case_dir
    variables.update(
        {
            "case_id": case_id,
            "case_dir": str(case_dir),
            "case_rel_dir": str(case_rel_dir),
        }
    )
    argv, env_overlay, cwd, timeout, expected = _case_command(case_id, raw_command, variables)
    command_record = {
        "argv": argv,
        "cwd": str(cwd),
        "env_overlay": env_overlay,
        "timeout_seconds": timeout,
    }
    _write_json(case_dir / "command.json", command_record)

    started_at = dt.datetime.now(dt.timezone.utc)
    started = time.monotonic()
    status = "PASS"
    message = "executor exited successfully and all declared evidence exists"
    returncode: int | None = None
    timed_out = False
    stdout = ""
    stderr = ""
    try:
        completed = subprocess.run(
            argv,
            cwd=cwd,
            env={**os.environ, **env_overlay},
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
        returncode = completed.returncode
        stdout = completed.stdout
        stderr = completed.stderr
        if returncode != 0:
            status = "FAIL"
            message = f"executor exited with {returncode}"
    except subprocess.TimeoutExpired as error:
        status = "FAIL"
        timed_out = True
        message = f"executor exceeded {timeout:g}s timeout"
        stdout = error.stdout or ""
        stderr = error.stderr or ""
    except OSError as error:
        status = "FAIL"
        message = f"executor could not start: {error}"
        stderr = str(error)

    duration_ms = (time.monotonic() - started) * 1000.0
    (case_dir / "stdout.log").write_text(stdout, encoding="utf-8")
    (case_dir / "stderr.log").write_text(stderr, encoding="utf-8")

    missing_evidence = [str(path) for path in expected if not path.exists()]
    if status == "PASS" and missing_evidence:
        status = "FAIL"
        message = f"executor succeeded but evidence is missing: {missing_evidence}"

    finished_at = dt.datetime.now(dt.timezone.utc)
    execution = {
        "case_id": case_id,
        "started_at": started_at.isoformat(),
        "finished_at": finished_at.isoformat(),
        "duration_ms": duration_ms,
        "returncode": returncode,
        "timed_out": timed_out,
        "status": status,
        "message": message,
        "expected_evidence": [str(path) for path in expected],
        "missing_evidence": missing_evidence,
    }
    _write_json(case_dir / "execution.json", execution)
    evidence = [
        str(case_dir / "command.json"),
        str(case_dir / "execution.json"),
        str(case_dir / "stdout.log"),
        str(case_dir / "stderr.log"),
        *[str(path) for path in expected if path.exists()],
    ]
    return {
        "id": case_id,
        "status": status,
        "duration_ms": round(duration_ms, 3),
        "message": message,
        "evidence": evidence,
    }


def run_acceptance(
    profile: dict[str, Any],
    tier: str,
    topology: str,
    purpose: str,
    run_dir: Path,
    variable_overrides: dict[str, str] | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    manifest, known_gaps = acceptance.load_contract()
    acceptance.validate_contract(manifest, known_gaps)
    selected = validate_profile(profile, manifest, tier, topology)
    if run_dir.exists():
        raise ProfileError(f"输出目录已存在: {run_dir}")
    run_dir.mkdir(parents=True)

    source = acceptance.source_identity()
    variables = resolve_variables({
        "root": str(acceptance.ROOT),
        "run_dir": str(run_dir),
        "run_nonce": make_run_nonce(),
        "tier": tier,
        "topology": topology,
        "purpose": purpose,
        **{key: str(value) for key, value in profile.get("variables", {}).items()},
        **(variable_overrides or {}),
    })
    results = [
        run_case(case, profile["commands"][case["id"]], variables, run_dir)
        for case in selected
    ]
    result = {
        "schema": "dms.m1.acceptance-result.v1",
        "purpose": purpose,
        "tier": tier,
        "topology": topology,
        "source": source,
        "environment": {
            "kernel": platform.release(),
            "arch": platform.machine(),
            "profile": profile["name"],
        },
        "cases": results,
    }
    evaluation = acceptance.evaluate(manifest, known_gaps, result)
    _write_json(run_dir / "result.json", result)
    _write_json(run_dir / "evaluation.json", evaluation)
    (run_dir / "evaluation.html").write_text(acceptance.render_html(evaluation), encoding="utf-8")
    return result, evaluation


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--tier", choices=sorted(acceptance.VALID_TIERS), required=True)
    parser.add_argument("--topology", choices=sorted(acceptance.VALID_TOPOLOGIES), required=True)
    parser.add_argument("--purpose", choices=sorted(acceptance.VALID_PURPOSES), default="discovery")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--variable",
        action="append",
        default=[],
        metavar="NAME=VALUE",
        help="覆盖 execution profile 变量；可重复指定。",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        _, evaluation = run_acceptance(
            load_profile(args.profile.resolve()),
            args.tier,
            args.topology,
            args.purpose,
            args.output.resolve(),
            parse_variable_overrides(args.variable),
        )
    except (ProfileError, acceptance.ContractError, json.JSONDecodeError, OSError) as error:
        print(f"M1 runner error: {error}", file=sys.stderr)
        return 1
    print(args.output.resolve())
    return 0 if evaluation["status"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())

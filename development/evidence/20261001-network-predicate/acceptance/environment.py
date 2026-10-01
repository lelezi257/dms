#!/usr/bin/env python3
"""Conservative AFS environment preparation evaluator."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import platform
import re
import shlex
import sys
from pathlib import Path
from typing import Any

STATUS_ORDER = {"PASS": 0, "BLOCKED": 1, "FAIL": 2}
EXPECTED_KERNEL = "6.8.0-142-generic"
EXPECTED_IMAGE_SHA = "1ea801e659d2f5035ac294e0faab0aac9b6ba66753df933ba5c7beab0c689bd0"
EXPECTED_NETWORK_PROBE_SHA = "3db932a4c1a72d450edbcc222ae8ae4010061fac84b5c012f061c91186e79612"
EXPECTED_NETWORK_FAULT_SOURCE_SHA = "a485c56bf184087f4cbdc2b3dc63b3b3085a537f43f743a2b992ba4c78813353"
GIB = 1024**3
EXPECTED_VMS = {
    "afs-accept-ctl": {"cpus": 2, "memory": 4 * GIB, "disk": 24 * GIB, "volume": "afsctlstate", "volume_gib": 8, "ip": "192.168.109.11", "inventory": "inventory-ctl.json"},
    "afs-accept-a": {"cpus": 2, "memory": 6 * GIB, "disk": 24 * GIB, "volume": "afsadata", "volume_gib": 32, "ip": "192.168.109.12", "inventory": "inventory-a.json"},
    "afs-accept-b": {"cpus": 2, "memory": 6 * GIB, "disk": 24 * GIB, "volume": "afsbdata", "volume_gib": 32, "ip": "192.168.109.13", "inventory": "inventory-b.json"},
    "afs-accept-c": {"cpus": 2, "memory": 6 * GIB, "disk": 24 * GIB, "volume": "afscdata", "volume_gib": 32, "ip": "192.168.109.14", "inventory": "inventory-c-rxe.json"},
}
NETWORK_NODES = {
    "ctl": {"vm": "afs-accept-ctl", "hostname": "lima-afs-accept-ctl", "ip": "192.168.109.11", "dns": "afs-env-ctl"},
    "a": {"vm": "afs-accept-a", "hostname": "lima-afs-accept-a", "ip": "192.168.109.12", "dns": "afs-env-a"},
    "b": {"vm": "afs-accept-b", "hostname": "lima-afs-accept-b", "ip": "192.168.109.13", "dns": "afs-env-b"},
    "c": {"vm": "afs-accept-c", "hostname": "lima-afs-accept-c", "ip": "192.168.109.14", "dns": "afs-env-c"},
}
DEFERRED = {
    "network-tls-fault-recovery": "complete four-way TCP/UDP, TLS negative and controlled fault recovery semantics are not validated",
    "durable-backend-restart": "etcd/Redis durable backend restart semantics are not validated",
    "cross-vm-verbs": "independent cross-VM verbs transfer is not validated here",
    "ext4-reference-accounting": "ext4 reference applicability and complete suite accounting are not validated",
    "actual-moosefs-mount-io": "stock MooseFS mount read/write evidence is not validated",
    "actual-3fs-mount-io": "stock 3FS mount read/write evidence is not validated",
    "complete-frozen-inputs": "complete source/binary/tool/suite/runner frozen-input contract is not validated",
    "run-contracts": "formal run contracts remain TODO and cannot be inferred from preparation receipts",
    "clock-accuracy": "clock synchronization is observed but accuracy bound has no evaluator yet",
    "cgroup-mount-cache-thin-allocation": "cgroup quotas, mount cache mode and host thin-allocation/cache semantics are not evaluated",
}


class InvalidEvidence(ValueError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def check(name: str, status: str, detail: str, evidence: Any = None) -> dict[str, Any]:
    item = {"name": name, "status": status, "detail": detail}
    if evidence is not None:
        item["evidence"] = evidence
    return item


def worst_status(checks: list[dict[str, Any]]) -> str:
    return max((item["status"] for item in checks), key=lambda status: STATUS_ORDER[status], default="BLOCKED")


def checked_path(root: Path, rel: str) -> Path:
    if not rel or "\x00" in rel or rel.startswith("/") or ".." in Path(rel).parts:
        raise InvalidEvidence(f"path escapes evidence root: {rel}")
    path = (root / rel).resolve()
    if not path.is_relative_to(root.resolve()):
        raise InvalidEvidence(f"path escapes evidence root: {rel}")
    return path


def refs_from_bundle(bundle: dict[str, Any]) -> dict[str, str]:
    refs = bundle.get("artifact_references")
    if refs is None:
        refs = bundle.get("files")
    if not isinstance(refs, dict):
        raise InvalidEvidence("bundle artifact_references/files must be an object")
    clean: dict[str, str] = {}
    for rel, digest in refs.items():
        if not isinstance(rel, str) or not isinstance(digest, str):
            raise InvalidEvidence("artifact references must map string paths to string sha256 values")
        if not rel or "\x00" in rel or Path(rel).is_absolute() or ".." in Path(rel).parts:
            raise InvalidEvidence(f"invalid artifact reference path: {rel!r}")
        if len(digest) != 64:
            raise InvalidEvidence(f"artifact reference sha256 is malformed: {rel}")
        clean[rel] = digest
    return clean


def read_json_ref(root: Path, refs: dict[str, str], rel: str) -> tuple[str, Any | None]:
    if rel not in refs:
        return "MISSING", None
    path = checked_path(root, rel)
    if not path.is_file():
        return "MISSING", None
    if sha256_file(path) != refs[rel]:
        return "TAMPERED", None
    try:
        value = load_json(path)
        return ("OK", value) if isinstance(value, dict) else ("MALFORMED", None)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return "MALFORMED", None


def read_jsonl_ref(root: Path, refs: dict[str, str], rel: str) -> tuple[str, list[dict[str, Any]]]:
    if rel not in refs:
        return "MISSING", []
    path = checked_path(root, rel)
    if not path.is_file():
        return "MISSING", []
    if sha256_file(path) != refs[rel]:
        return "TAMPERED", []
    rows: list[dict[str, Any]] = []
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                row = json.loads(line)
                if not isinstance(row, dict):
                    return "MALFORMED", []
                rows.append(row)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return "MALFORMED", []
    return "OK", rows


def read_text_ref(root: Path, refs: dict[str, str], rel: str) -> tuple[str, str]:
    if rel not in refs:
        return "MISSING", ""
    path = checked_path(root, rel)
    if not path.is_file():
        return "MISSING", ""
    if sha256_file(path) != refs[rel]:
        return "TAMPERED", ""
    try:
        return "OK", path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return "MALFORMED", ""


def parse_stdout_json(record: Any) -> Any | None:
    if not isinstance(record, dict) or not isinstance(record.get("stdout"), str) or record.get("returncode") != 0 or record.get("status") != "OBSERVED":
        return None
    try:
        return json.loads(record["stdout"])
    except json.JSONDecodeError:
        return None


def is_positive_int(value: Any) -> bool:
    return type(value) is int and value > 0


def is_finite_seconds(value: Any, maximum: float = 3.0) -> bool:
    return type(value) in {int, float} and math.isfinite(float(value)) and 0 <= float(value) <= maximum


def port_tuple(value: Any, ip: str, port: int | None = None) -> bool:
    return isinstance(value, list) and len(value) == 2 and value[0] == ip and type(value[1]) is int and 1 <= value[1] <= 65535 and (port is None or value[1] == port)


def is_sha256_hex(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(char in "0123456789abcdef" for char in value)


def normalize_iptables(text: str) -> list[str]:
    return [line for line in text.splitlines() if line and not line.startswith("#")]


def valid_iptables_save(text: Any) -> bool:
    if not isinstance(text, str):
        return False
    rows = normalize_iptables(text)
    table = None
    completed = 0
    for row in rows:
        if row.startswith("*") and table is None:
            table = row[1:]
            if table not in {"nat", "filter", "mangle", "raw", "security"}:
                return False
        elif row == "COMMIT" and table is not None:
            table = None
            completed += 1
        elif table is None or not row.startswith((":", "-A ")):
            return False
    return completed > 0 and table is None


def valid_preserved_observation(value: dict[str, Any]) -> bool:
    processes = value.get("processes")
    mountinfo = value.get("mountinfo")
    if not isinstance(processes, list) or not isinstance(mountinfo, str) or not mountinfo.strip() or not valid_iptables_save(value.get("iptables")):
        return False
    for process in processes:
        if not isinstance(process, dict) or not is_positive_int(process.get("pid")) or not isinstance(process.get("start_ticks"), str) or not process["start_ticks"].isdigit() or int(process["start_ticks"]) <= 0 or not isinstance(process.get("exe"), str) or not process["exe"].startswith("/") or not is_sha256_hex(process.get("exe_sha256")) or not isinstance(process.get("configs"), list):
            return False
        if any(not isinstance(config, dict) or not isinstance(config.get("path"), str) or not config["path"].startswith("/") or not is_sha256_hex(config.get("sha256")) for config in process["configs"]):
            return False
    return True


def network_pair_rel(prefix: str, src: str, dst: str) -> str:
    return f"{prefix}/{src}/logs/pair-{src}-{dst}.json"


def add_network_problem(problems: list[dict[str, str]], status: str, detail: str) -> None:
    problems.append({"status": status, "detail": detail})


def network_status_from(problems: list[dict[str, str]]) -> str:
    if any(problem["status"] == "FAIL" for problem in problems):
        return "FAIL"
    if problems:
        return "BLOCKED"
    return "PASS"


def require_json_ref(root: Path, refs: dict[str, str], rel: str, problems: list[dict[str, str]]) -> dict[str, Any] | None:
    status, value = read_json_ref(root, refs, rel)
    if status == "OK" and isinstance(value, dict):
        return value
    add_network_problem(problems, "BLOCKED" if status == "MISSING" else "FAIL", f"{rel}: {status}")
    return None


def require_text_ref(root: Path, refs: dict[str, str], rel: str, problems: list[dict[str, str]]) -> str | None:
    status, value = read_text_ref(root, refs, rel)
    if status == "OK":
        return value
    add_network_problem(problems, "BLOCKED" if status == "MISSING" else "FAIL", f"{rel}: {status}")
    return None


def shell_command(row: dict[str, Any], expected_vm: str, expected_rc: int) -> str | None:
    argv = row.get("argv")
    if not isinstance(argv, list) or row.get("returncode") != expected_rc:
        return None
    expected_prefix = ["limactl", "shell", "--workdir", "/home/lzc.guest", expected_vm, "--", "sudo", "bash", "-lc"]
    if len(argv) != len(expected_prefix) + 1 or argv[:len(expected_prefix)] != expected_prefix:
        return None
    shell = argv[-1]
    if not isinstance(shell, str) or shell.lstrip().startswith("echo "):
        return None
    return shell


def shell_tokens(shell: str) -> list[str] | None:
    try:
        return shlex.split(shell)
    except ValueError:
        return None


def exact_client_command(row: dict[str, Any], src: str, dst: str, output: str, expected_rc: int, *, require_negatives: bool, wrong_client: bool = False) -> bool:
    shell = shell_command(row, NETWORK_NODES[src]["vm"], expected_rc)
    if shell is None:
        return False
    tokens = shell_tokens(shell)
    if tokens is None:
        return False
    expected = ["python3", "/var/lib/afs-acceptance/network-v67-r2/env_network.py", "client"]
    expected_values = {
        "--source-ip": NETWORK_NODES[src]["ip"],
        "--target-ip": NETWORK_NODES[dst]["ip"],
        "--port": "19566",
        "--tls-port": "19567",
        "--ca": "/var/lib/afs-acceptance/network-v67-r2/tls/ca.pem",
        "--client-cert": f"/var/lib/afs-acceptance/network-v67-r2/tls/{src}.pem",
        "--client-key": f"/var/lib/afs-acceptance/network-v67-r2/tls/{src}.key",
        "--server-hostname": NETWORK_NODES[dst]["dns"],
        "--timeout": "2",
        "--check": "tls" if wrong_client else "all",
    }
    for flag, value in expected_values.items():
        expected.extend([flag, value])
    if require_negatives:
        expected.extend(["--untrusted-ca", "--bad-ca", "/var/lib/afs-acceptance/network-v67-r2/tls/untrusted.pem", "--wrong-hostname", "--missing-client-cert"])
    if wrong_client:
        expected.extend(["--untrusted-client-cert", "--bad-client-cert", "/var/lib/afs-acceptance/network-v67-r2/tls/rogue.pem", "--bad-client-key", "/var/lib/afs-acceptance/network-v67-r2/tls/rogue.key"])
    expected.extend([">", f"/var/lib/afs-acceptance/network-v67-r2/{output}", "2>", f"/var/lib/afs-acceptance/network-v67-r2/{output.removesuffix('.json')}.stderr"])
    # This predicate supports the frozen collector's literal invocation, with
    # no duplicate options, shell suffix, or result-rewriting command.
    return tokens == expected


def network_check_positive(result: dict[str, Any], src: str, dst: str, problems: list[dict[str, str]], rel: str, *, require_negatives: bool = True) -> bool:
    src_ip, dst_ip = NETWORK_NODES[src]["ip"], NETWORK_NODES[dst]["ip"]
    if result.get("status") != "PASS" or result.get("source_bind") != src_ip or result.get("target") != dst_ip:
        add_network_problem(problems, "FAIL", f"{rel}: source/target/status mismatch")
        return False
    if type(result.get("token_bytes")) is not int or result.get("token_bytes") != 32 or not is_sha256_hex(result.get("token_sha256")) or not is_finite_seconds(result.get("timeout_seconds")) or result["timeout_seconds"] <= 0:
        add_network_problem(problems, "FAIL", f"{rel}: invalid nonce")
        return False
    checks = result.get("checks")
    if not isinstance(checks, dict):
        add_network_problem(problems, "FAIL", f"{rel}: missing checks")
        return False
    ok = True
    tcp = checks.get("tcp")
    if not isinstance(tcp, dict) or tcp.get("status") != "PASS" or tcp.get("bytes") != 32 or not port_tuple(tcp.get("local"), src_ip) or not port_tuple(tcp.get("peer"), dst_ip, 19566) or not is_finite_seconds(tcp.get("elapsed_seconds")):
        add_network_problem(problems, "FAIL", f"{rel}: invalid TCP exchange")
        ok = False
    udp = checks.get("udp")
    if not isinstance(udp, dict) or udp.get("status") != "PASS" or udp.get("bytes") != 32 or not port_tuple(udp.get("local"), src_ip) or not port_tuple(udp.get("sender"), dst_ip, 19566) or not is_finite_seconds(udp.get("elapsed_seconds")):
        add_network_problem(problems, "FAIL", f"{rel}: invalid UDP exchange")
        ok = False
    tls = checks.get("tls")
    names = tls.get("server_cert_names") if isinstance(tls, dict) else None
    if not isinstance(tls, dict) or tls.get("status") != "PASS" or tls.get("bytes") != 32 or not port_tuple(tls.get("local"), src_ip) or not port_tuple(tls.get("peer"), dst_ip, 19567) or tls.get("tls_version") not in {"TLSv1.2", "TLSv1.3"} or not is_finite_seconds(tls.get("elapsed_seconds")) or not isinstance(names, list) or dst_ip not in names or NETWORK_NODES[dst]["dns"] not in names:
        add_network_problem(problems, "FAIL", f"{rel}: invalid mTLS exchange")
        ok = False
    if require_negatives:
        expected_negatives = {
            "tls_untrusted_ca": ("untrusted_ca", "19"),
            "tls_wrong_hostname": ("wrong_hostname", "62"),
            "tls_missing_client_cert": ("missing_client_cert", None),
        }
        for key, (reason, verify_code) in expected_negatives.items():
            negative = checks.get(key)
            observed = negative.get("observed") if isinstance(negative, dict) else None
            detail = observed.get("detail", "") if isinstance(observed, dict) else ""
            if not isinstance(negative, dict) or negative.get("status") != "PASS" or negative.get("negative") != reason or not isinstance(observed, dict) or observed.get("status") != "FAIL" or observed.get("reason") != reason:
                add_network_problem(problems, "FAIL", f"{rel}: invalid {key}")
                ok = False
            if verify_code is not None and isinstance(observed, dict) and observed.get("verify_code") != verify_code:
                add_network_problem(problems, "FAIL", f"{rel}: invalid {key} verify code")
                ok = False
            if key == "tls_missing_client_cert" and "certificate required" not in str(detail).lower():
                add_network_problem(problems, "FAIL", f"{rel}: missing-client negative lacks certificate-required alert")
                ok = False
    return ok


def network_check_fault_result(result: dict[str, Any], expected_status: str, problems: list[dict[str, str]], rel: str) -> None:
    if expected_status == "PASS":
        network_check_positive(result, "a", "b", problems, rel, require_negatives=False)
        return
    if result.get("status") != "FAIL" or result.get("source_bind") != NETWORK_NODES["a"]["ip"] or result.get("target") != NETWORK_NODES["b"]["ip"]:
        add_network_problem(problems, "FAIL", f"{rel}: fault result source/target/status mismatch")
        return
    if type(result.get("token_bytes")) is not int or result.get("token_bytes") != 32 or not is_sha256_hex(result.get("token_sha256")) or not is_finite_seconds(result.get("timeout_seconds")) or result["timeout_seconds"] <= 0:
        add_network_problem(problems, "FAIL", f"{rel}: invalid fault nonce or timeout")
    checks = result.get("checks")
    if not isinstance(checks, dict):
        add_network_problem(problems, "FAIL", f"{rel}: missing fault checks")
        return
    for key in ("tcp", "udp", "tls"):
        item = checks.get(key)
        if not isinstance(item, dict) or item.get("status") != "FAIL" or item.get("reason") != "timeout" or not is_finite_seconds(item.get("elapsed_seconds"), 3.0):
            add_network_problem(problems, "FAIL", f"{rel}: invalid bounded {key} fault")


def evaluate_network(bundle: dict[str, Any], artifact_root: Path, refs: dict[str, str]) -> dict[str, Any]:
    try:
        return _evaluate_network(bundle, artifact_root, refs)
    except (InvalidEvidence, OSError) as exc:
        return check("network-tls-fault-recovery", "BLOCKED", str(exc))
    except (TypeError, ValueError, AttributeError, KeyError, OverflowError) as exc:
        return check("network-tls-fault-recovery", "FAIL", f"malformed network observation: {exc}")


def _evaluate_network(bundle: dict[str, Any], artifact_root: Path, refs: dict[str, str]) -> dict[str, Any]:
    problems: list[dict[str, str]] = []
    evidence: dict[str, Any] = {"scope": "network/TLS and directed fault preparation predicate only"}
    network = bundle.get("network")
    if not isinstance(network, dict):
        return check("network-tls-fault-recovery", "BLOCKED", "network evidence bundle is missing")
    prefix = network.get("prefix")
    if prefix != "network":
        return check("network-tls-fault-recovery", "BLOCKED", "network.prefix must be 'network'", {"prefix": prefix})
    for field in ("probe_source", "commands", "fault_source"):
        rel = network.get(field)
        if not isinstance(rel, str):
            return check("network-tls-fault-recovery", "BLOCKED", f"network.{field} is missing")
        try:
            checked_path(artifact_root, rel)
        except InvalidEvidence as exc:
            return check("network-tls-fault-recovery", "BLOCKED", str(exc))
        if rel not in refs:
            add_network_problem(problems, "BLOCKED", f"{rel}: MISSING")
    probe_path = checked_path(artifact_root, network["probe_source"])
    if not probe_path.is_file():
        add_network_problem(problems, "BLOCKED", "network probe source file is missing")
    else:
        probe_sha = sha256_file(probe_path)
        if network["probe_source"] in refs and probe_sha != refs[network["probe_source"]]:
            add_network_problem(problems, "FAIL", "network probe source reference is tampered")
        if probe_sha != EXPECTED_NETWORK_PROBE_SHA:
            add_network_problem(problems, "FAIL", "network probe source is not the frozen observed probe")
    if network["fault_source"] in refs:
        fault_path = checked_path(artifact_root, network["fault_source"])
        if not fault_path.is_file():
            add_network_problem(problems, "BLOCKED", "fault source file is missing")
        else:
            fault_sha = sha256_file(fault_path)
            if fault_sha != refs[network["fault_source"]]:
                add_network_problem(problems, "FAIL", "fault source reference is tampered")
            if fault_sha != EXPECTED_NETWORK_FAULT_SOURCE_SHA:
                add_network_problem(problems, "FAIL", "fault source is not the supported directed-fault recipe")
    command_status, commands = read_jsonl_ref(artifact_root, refs, network["commands"])
    if command_status != "OK" or not commands:
        add_network_problem(problems, "BLOCKED" if command_status == "MISSING" else "FAIL", f"{network['commands']}: {command_status or 'EMPTY'}")
    commands_available = command_status == "OK" and bool(commands)
    commands_malformed = any(not isinstance(row.get("argv"), list) or any(not isinstance(part, str) for part in row.get("argv", [])) or not is_finite_seconds(row.get("time_unix_ms"), 10**13) for row in commands)
    if commands_malformed:
        add_network_problem(problems, "FAIL", "command transcript has malformed argv or timestamp")
    if commands_available and not commands_malformed and any(commands[i]["time_unix_ms"] > commands[i + 1]["time_unix_ms"] for i in range(len(commands) - 1)):
        add_network_problem(problems, "FAIL", "command transcript timestamps are not ordered")

    ready: dict[str, dict[str, Any]] = {}
    boots: set[str] = set()
    machines: set[str] = set()
    for name, node in NETWORK_NODES.items():
        rel = f"{prefix}/{name}/ready.json"
        item = require_json_ref(artifact_root, refs, rel, problems)
        if item is None:
            continue
        ready[name] = item
        script_rel = f"{prefix}/{name}/env_network.py"
        script_path = checked_path(artifact_root, script_rel)
        if script_rel not in refs or not script_path.is_file():
            add_network_problem(problems, "BLOCKED", f"{script_rel}: MISSING")
        elif sha256_file(script_path) != refs[script_rel] or sha256_file(script_path) != EXPECTED_NETWORK_PROBE_SHA:
            add_network_problem(problems, "FAIL", f"{script_rel}: unexpected source")
        probe_input = require_text_ref(artifact_root, refs, f"{prefix}/{name}/logs/probe-input.sha256", problems)
        if probe_input is not None and EXPECTED_NETWORK_PROBE_SHA not in probe_input:
            add_network_problem(problems, "FAIL", f"{name}: probe input SHA mismatch")
        boot, machine = item.get("boot_id"), item.get("machine_id")
        if item.get("status") != "READY" or item.get("hostname") != node["hostname"] or item.get("source_ip") != node["ip"] or not is_positive_int(item.get("pid")) or not is_positive_int(item.get("start_ticks")) or not isinstance(boot, str) or not boot or not isinstance(machine, str) or not machine:
            add_network_problem(problems, "FAIL", f"{rel}: malformed ready identity")
        if item.get("script_sha256") != EXPECTED_NETWORK_PROBE_SHA:
            add_network_problem(problems, "FAIL", f"{rel}: script SHA mismatch")
        for key, port in (("tcp", 19566), ("udp", 19566), ("tls", 19567)):
            value = item.get(key)
            if not isinstance(value, dict) or value.get("bind_ip") != node["ip"] or value.get("port") != port:
                add_network_problem(problems, "FAIL", f"{rel}: invalid {key} listener")
        if not isinstance(item.get("tls"), dict) or item["tls"].get("mtls") is not True:
            add_network_problem(problems, "FAIL", f"{rel}: mTLS disabled")
        if isinstance(boot, str):
            boots.add(boot)
        if isinstance(machine, str):
            machines.add(machine)
    if len(ready) == len(NETWORK_NODES) and (len(boots) != len(NETWORK_NODES) or len(machines) != len(NETWORK_NODES)):
        add_network_problem(problems, "FAIL", "guest boot and machine identities must be nonempty and unique")

    nonces: set[str] = set()
    pair_missing = False
    for src in NETWORK_NODES:
        for dst in NETWORK_NODES:
            if src == dst:
                continue
            rel = network_pair_rel(prefix, src, dst)
            item = require_json_ref(artifact_root, refs, rel, problems)
            if item is None:
                pair_missing = True
            elif network_check_positive(item, src, dst, problems, rel):
                nonce = item["token_sha256"]
                if nonce in nonces:
                    add_network_problem(problems, "FAIL", f"{rel}: duplicate nonce")
                nonces.add(nonce)
    if not pair_missing and len(nonces) != 12:
        add_network_problem(problems, "FAIL", "expected twelve unique pair nonces")

    wrong = require_json_ref(artifact_root, refs, f"{prefix}/a/logs/wrong-client.json", problems)
    if wrong is not None:
        checks = wrong.get("checks")
        negative = checks.get("tls_untrusted_client_cert") if isinstance(checks, dict) else None
        observed = negative.get("observed") if isinstance(negative, dict) else None
        tls = checks.get("tls") if isinstance(checks, dict) else None
        names = tls.get("server_cert_names") if isinstance(tls, dict) else None
        if wrong.get("status") != "PASS" or wrong.get("source_bind") != NETWORK_NODES["a"]["ip"] or wrong.get("target") != NETWORK_NODES["b"]["ip"] or type(wrong.get("token_bytes")) is not int or wrong.get("token_bytes") != 32 or not is_sha256_hex(wrong.get("token_sha256")):
            add_network_problem(problems, "FAIL", "wrong-client positive identity is malformed")
        if not isinstance(tls, dict) or tls.get("status") != "PASS" or tls.get("bytes") != 32 or not port_tuple(tls.get("local"), NETWORK_NODES["a"]["ip"]) or not port_tuple(tls.get("peer"), NETWORK_NODES["b"]["ip"], 19567) or tls.get("tls_version") not in {"TLSv1.2", "TLSv1.3"} or not is_finite_seconds(tls.get("elapsed_seconds")) or not isinstance(names, list) or NETWORK_NODES["b"]["ip"] not in names or NETWORK_NODES["b"]["dns"] not in names:
            add_network_problem(problems, "FAIL", "wrong-client mTLS positive exchange is malformed")
        if not isinstance(negative, dict) or negative.get("status") != "PASS" or negative.get("negative") != "untrusted_client_cert" or not isinstance(observed, dict) or observed.get("status") != "FAIL" or observed.get("reason") != "untrusted_ca" or "unknown ca" not in str(observed.get("detail", "")).lower():
            add_network_problem(problems, "FAIL", "wrong-client rejection is not unknown-CA mTLS failure")

    for rel, expected in (("fault-before.json", "PASS"), ("fault-injected.json", "FAIL"), ("fault-restored.json", "PASS")):
        item = require_json_ref(artifact_root, refs, f"{prefix}/a/logs/{rel}", problems)
        if item is not None:
            network_check_fault_result(item, expected, problems, f"{prefix}/a/logs/{rel}")

    hit = require_text_ref(artifact_root, refs, f"{prefix}/b/logs/iptables-hit.txt", problems)
    if hit is not None:
        tagged = [line for line in hit.splitlines() if "DROP" in line and "afs-env-v67-only" in line]
        pattern = r"\s*([1-9][0-9]*)\s+([1-9][0-9]*)\s+DROP\s+(6|17)\s+--\s+\*\s+\*\s+192\.168\.109\.12\s+192\.168\.109\.13\s+(multiport dports 19566,19567|udp dpt:19566)\s+/\* afs-env-v67-only \*/\s*"
        matches = [re.fullmatch(pattern, line) for line in tagged]
        scopes = {(match.group(3), match.group(4)) for match in matches if match is not None}
        if len(tagged) != 2 or any(match is None for match in matches) or scopes != {("6", "multiport dports 19566,19567"), ("17", "udp dpt:19566")}:
            add_network_problem(problems, "FAIL", "directed DROP counters are not exact and nonzero")
    before_rules = require_text_ref(artifact_root, refs, f"{prefix}/b/logs/iptables-before.txt", problems)
    final_rules = require_text_ref(artifact_root, refs, f"{prefix}/b/logs/iptables-final-restored.txt", problems)
    if before_rules is not None and final_rules is not None:
        if not valid_iptables_save(before_rules) or not valid_iptables_save(final_rules) or normalize_iptables(before_rules) != normalize_iptables(final_rules):
            add_network_problem(problems, "FAIL", "iptables final rules differ from original rules")

    for name, node in NETWORK_NODES.items():
        before = require_json_ref(artifact_root, refs, f"{prefix}/{name}/logs/preserved-before.json", problems)
        after = require_json_ref(artifact_root, refs, f"{prefix}/{name}/logs/preserved-after.json", problems)
        if before is not None and after is not None:
            if not valid_preserved_observation(before) or not valid_preserved_observation(after):
                add_network_problem(problems, "FAIL", f"{name}: preserved observations lack typed processes/mounts/rules")
            for key in ("processes", "mountinfo", "iptables"):
                left = normalize_iptables(before.get(key, "")) if key == "iptables" and isinstance(before.get(key), str) else before.get(key)
                right = normalize_iptables(after.get(key, "")) if key == "iptables" and isinstance(after.get(key), str) else after.get(key)
                if left != right:
                    add_network_problem(problems, "FAIL", f"{name}: preserved {key} changed")
            if before.get("hostname") != node["hostname"] or after.get("hostname") != node["hostname"] or (name in ready and (before.get("boot_id") != ready[name].get("boot_id") or after.get("boot_id") != ready[name].get("boot_id") or before.get("machine_id") != ready[name].get("machine_id") or after.get("machine_id") != ready[name].get("machine_id"))):
                add_network_problem(problems, "FAIL", f"{name}: preserved identity mismatch")
        live = require_json_ref(artifact_root, refs, f"{prefix}/{name}/logs/server-live-after.json", problems)
        stopped = require_json_ref(artifact_root, refs, f"{prefix}/{name}/logs/server-stopped.json", problems)
        if live is not None and name in ready:
            if live.get("status") != "LIVE" or live.get("pid") != ready[name].get("pid") or live.get("start_ticks") != ready[name].get("start_ticks") or live.get("boot_id") != ready[name].get("boot_id") or live.get("script_sha256") != EXPECTED_NETWORK_PROBE_SHA:
                add_network_problem(problems, "FAIL", f"{name}: live server identity mismatch")
        if stopped is not None and name in ready:
            if stopped.get("status") != "STOPPED" or stopped.get("pid") != ready[name].get("pid") or stopped.get("start_ticks") != ready[name].get("start_ticks"):
                add_network_problem(problems, "FAIL", f"{name}: server stop identity mismatch")
        listeners = require_text_ref(artifact_root, refs, f"{prefix}/{name}/logs/listeners-after.txt", problems)
        if listeners is not None and (":19566" in listeners or ":19567" in listeners):
            add_network_problem(problems, "FAIL", f"{name}: probe listeners still present after cleanup")

    if commands_available and not commands_malformed:
        if not any(exact_client_command(row, "a", "b", "logs/wrong-client.json", 0, require_negatives=False, wrong_client=True) for row in commands):
            add_network_problem(problems, "FAIL", "missing exact untrusted-client command transcript")
        for src in NETWORK_NODES:
            for dst in NETWORK_NODES:
                if src == dst:
                    continue
                output = f"logs/pair-{src}-{dst}.json"
                if not any(exact_client_command(row, src, dst, output, 0, require_negatives=True) for row in commands):
                    add_network_problem(problems, "FAIL", f"missing exact command transcript for {src}->{dst}")
        required_fault = [
            (lambda row: exact_client_command(row, "a", "b", "logs/fault-before.json", 0, require_negatives=False), "fault-before"),
            (lambda row: shell_command(row, NETWORK_NODES["b"]["vm"], 0) == "bash /tmp/afs-v67-fault.sh install", "fault-install"),
            (lambda row: exact_client_command(row, "a", "b", "logs/fault-injected.json", 1, require_negatives=False), "fault-injected"),
            (lambda row: shell_command(row, NETWORK_NODES["b"]["vm"], 0) == "bash /tmp/afs-v67-fault.sh inspect; bash /tmp/afs-v67-fault.sh restore", "fault-restore"),
            (lambda row: exact_client_command(row, "a", "b", "logs/fault-restored.json", 0, require_negatives=False), "fault-restored"),
        ]
        pos = -1
        for predicate, name in required_fault:
            matches = [i for i, row in enumerate(commands) if i > pos and predicate(row)]
            if not matches:
                add_network_problem(problems, "FAIL", f"missing ordered command transcript step: {name}")
                break
            pos = matches[0]

    evidence["problems"] = problems[:20]
    evidence["pair_nonces"] = len(nonces)
    status = network_status_from(problems)
    detail = "hash-bound raw network/TLS exchanges and directed fault restoration validated" if status == "PASS" else "network evidence is incomplete or inconsistent"
    return check("network-tls-fault-recovery", status, detail, evidence)


def memtotal_bytes(inventory: dict[str, Any]) -> int | None:
    meminfo = inventory.get("meminfo")
    if not isinstance(meminfo, str):
        return None
    for line in meminfo.splitlines():
        if line.startswith("MemTotal:"):
            parts = line.split()
            if len(parts) >= 2 and parts[1].isdigit():
                return int(parts[1]) * 1024
    return None


def os_is_ubuntu_2404(inventory: dict[str, Any]) -> bool:
    os_release = inventory.get("os_release")
    if not isinstance(os_release, str):
        return False
    fields = dict(line.split("=", 1) for line in os_release.splitlines() if "=" in line and not line.startswith("#"))
    return fields.get("ID", "").strip('"') == "ubuntu" and fields.get("VERSION_ID", "").strip('"') == "24.04"


def address_mtu(inventory: dict[str, Any], expected_ip: str) -> tuple[bool, Any]:
    data = parse_stdout_json(inventory.get("addresses"))
    if not isinstance(data, list):
        return False, "missing ip address JSON"
    for iface in data:
        if isinstance(iface, dict) and iface.get("ifname") == "eth0":
            addresses = iface.get("addr_info")
            if not isinstance(addresses, list):
                return False, "missing addr_info array"
            ips = [a.get("local") for a in addresses if isinstance(a, dict) and a.get("family") == "inet"]
            return expected_ip in ips and iface.get("mtu") == 1500, {"ips": ips, "mtu": iface.get("mtu")}
    return False, "missing eth0"


def flatten_blocks(devices: Any) -> list[dict[str, Any]]:
    flat: list[dict[str, Any]] = []
    if not isinstance(devices, list):
        return flat
    for item in devices:
        if isinstance(item, dict):
            flat.append(item)
            flat.extend(flatten_blocks(item.get("children")))
    return flat


def has_ext4_mount(inventory: dict[str, Any], prefix: str, min_size: int) -> tuple[bool, Any]:
    data = parse_stdout_json(inventory.get("block_layout"))
    devices = flatten_blocks(data.get("blockdevices") if isinstance(data, dict) else None)
    rows = []
    for device in devices:
        mountpoints = device.get("mountpoints")
        mounts = [m for m in mountpoints if isinstance(m, str)] if isinstance(mountpoints, list) else []
        if device.get("fstype") == "ext4" and prefix in mounts:
            rows.append({"name": device.get("name"), "size": device.get("size"), "mountpoints": mounts})
    return any(isinstance(row.get("size"), int) and row["size"] >= min_size for row in rows), rows


def df_available(inventory: dict[str, Any], target: str) -> tuple[int | None, dict[str, Any]]:
    record = inventory.get("disk_space")
    stdout = record.get("stdout", "") if isinstance(record, dict) else ""
    if not isinstance(stdout, str):
        return None, {"error": "disk_space stdout must be text"}
    rows = []
    for line in stdout.splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 7:
            row = {"source": parts[0], "fstype": parts[1], "available": parts[4], "target": parts[6]}
            rows.append(row)
            if parts[6] == target and parts[1] == "ext4" and parts[4].isdigit():
                return int(parts[4]), {"row": row, "df_status": record.get("status"), "returncode": record.get("returncode")}
    return None, {"rows": rows, "df_status": record.get("status") if isinstance(record, dict) else None}


def image_digest_ok(row: dict[str, Any], lock: dict) -> bool:
    images = row.get("config", {}).get("images") if isinstance(row.get("config"), dict) else None
    expected = lock.get("image") if isinstance(lock.get("image"), dict) else {}
    locations = [expected.get("url")]
    if isinstance(expected.get("local_cache"), str):
        locations.append("file://" + expected["local_cache"])
    return isinstance(images, list) and any(isinstance(img, dict) and img.get("arch") == "aarch64" and img.get("location") in [p for p in locations if isinstance(p, str)] and img.get("digest") == f"sha256:{EXPECTED_IMAGE_SHA}" for img in images)


def check_contract(lock: dict, bundle: dict, root: Path, checks: list[dict[str, Any]]) -> None:
    expected = lock.get("contract", {}).get("sha256") if isinstance(lock.get("contract"), dict) else None
    contract = bundle.get("contract")
    if not isinstance(expected, str) or len(expected) != 64:
        checks.append(check("acceptance-contract-sha256", "BLOCKED", "lock.contract.sha256 is missing"))
        return
    if not isinstance(contract, dict) or not isinstance(contract.get("path"), str) or not isinstance(contract.get("sha256"), str):
        checks.append(check("acceptance-contract-sha256", "BLOCKED", "bundle.contract {path, sha256} is missing"))
        return
    path = checked_path(root, contract["path"])
    if not path.is_file():
        checks.append(check("acceptance-contract-sha256", "BLOCKED", "bundle contract file is missing"))
        return
    actual = sha256_file(path)
    checks.append(check("acceptance-contract-sha256", "PASS" if actual == contract["sha256"] == expected else "FAIL", "actual acceptance contract SHA-256 is bound", {"expected": expected, "bundle": contract["sha256"], "actual": actual}))


def _evaluate_environment(lock: dict, bundle: dict, artifact_root: Path) -> dict:
    checks: list[dict[str, Any]] = []
    limitations: list[str] = []
    try:
        refs = refs_from_bundle(bundle)
    except InvalidEvidence as exc:
        return {"schema_version": 1, "status": "BLOCKED", "checks": [check("bundle-shape", "BLOCKED", str(exc))], "limitations": [str(exc)], "summary": {"pass": 0, "blocked": 1, "fail": 0}, "notes": ["Invalid evidence shape; no ENV qualification made."]}
    check_contract(lock, bundle, artifact_root, checks)

    host_status, host = read_json_ref(artifact_root, refs, "host.json")
    if host_status == "OK" and isinstance(host, dict):
        if not isinstance(host.get("arch"), str) or any(type(host.get(field)) is not int for field in ("cpu_count", "ram_bytes", "available_bytes")):
            raise InvalidEvidence("host arch/cpu/ram/available fields are missing or malformed")
        host_ok = host.get("arch") in {"aarch64", "arm64"} and host.get("cpu_count", 0) >= 10 and host.get("ram_bytes", 0) >= 32 * GIB
        avail_ok = host.get("available_bytes", 0) >= 40 * GIB
        initial = host.get("initial_available_bytes")
        checks.append(check("host-actual-observed", "PASS" if host_ok and avail_ok else "FAIL", "hash-bound host arch/cpu/ram/current reserve observed", host))
        checks.append(check("host-initial-reserve", "PASS" if isinstance(initial, int) and initial >= 100 * GIB else "BLOCKED", "initial host reserve must be at least 100 GiB; absent value is BLOCKED", {"initial_available_bytes": initial}))
    else:
        checks.append(check("host-actual-observed", "BLOCKED" if host_status == "MISSING" else "FAIL", "hash-bound host.json observation is required", {"status": host_status}))

    lima_status, lima_rows = read_jsonl_ref(artifact_root, refs, "lima-after.jsonl")
    checks.append(check("lima-after-jsonl", "PASS" if lima_status == "OK" else ("BLOCKED" if lima_status == "MISSING" else "FAIL"), "hash-bound Lima topology observation", {"status": lima_status}))
    lima = {row.get("name"): row for row in lima_rows if isinstance(row.get("name"), str)}

    for name, expected in EXPECTED_VMS.items():
        row = lima.get(name, {})
        if row and (any(type(row.get(field)) is not int for field in ("cpus", "memory", "disk")) or not isinstance(row.get("additionalDisks"), list)):
            raise InvalidEvidence(f"{name} Lima resource/disk fields are malformed")
        inv_status, inv = read_json_ref(artifact_root, refs, expected["inventory"])
        inventory = inv if inv_status == "OK" and isinstance(inv, dict) else {}
        config_ok = row.get("status") == "Running" and row.get("hostname") == f"lima-{name}" and row.get("arch") == "aarch64" and row.get("cpus") == expected["cpus"] and row.get("memory") == expected["memory"] and row.get("disk") == expected["disk"]
        volume_ok = any(isinstance(d, dict) and d.get("name") == expected["volume"] and d.get("format") is True and d.get("fsType") == "ext4" for d in row.get("additionalDisks", []))
        checks.append(check(f"{name}-lima-config", "BLOCKED" if not row else ("PASS" if config_ok and volume_ok and image_digest_ok(row, lock) else "FAIL"), "Lima running topology, hostname and configured image match lock", {"status": row.get("status"), "hostname": row.get("hostname"), "arch": row.get("arch"), "cpus": row.get("cpus"), "memory": row.get("memory"), "disk": row.get("disk"), "images": row.get("config", {}).get("images") if isinstance(row.get("config"), dict) else None, "configured_image_ok": image_digest_ok(row, lock)}))

        mem = memtotal_bytes(inventory)
        missing = inv_status == "MISSING" or not inventory or mem is None or any(inventory.get(field) is None for field in ("hostname", "architecture", "cpu_count", "kernel", "os_release"))
        mismatch = bool(inventory) and (inventory.get("hostname") != f"lima-{name}" or inventory.get("architecture") != "aarch64" or inventory.get("cpu_count") != expected["cpus"] or inventory.get("kernel") != EXPECTED_KERNEL or not os_is_ubuntu_2404(inventory))
        mem_ok = mem is not None and mem >= int(expected["memory"] * 0.90)
        status = "FAIL" if inv_status in {"TAMPERED", "MALFORMED"} else ("BLOCKED" if missing else ("FAIL" if mismatch or not mem_ok else "PASS"))
        checks.append(check(f"{name}-guest-identity", status, "guest hostname, arch, CPU, RAM, Ubuntu 24.04 and kernel observed", {"inventory_status": inv_status, "hostname": inventory.get("hostname"), "architecture": inventory.get("architecture"), "cpu_count": inventory.get("cpu_count"), "memtotal_bytes": mem, "kernel": inventory.get("kernel"), "ubuntu_2404": os_is_ubuntu_2404(inventory)}))

        ip_ok, ip_ev = address_mtu(inventory, expected["ip"])
        checks.append(check(f"{name}-ip-mtu", "PASS" if ip_ok else "BLOCKED", "fixed IPv4 and MTU 1500 observed", ip_ev))
        mount_prefix = f"/mnt/lima-{expected['volume']}"
        ext4_ok, ext4_rows = has_ext4_mount(inventory, mount_prefix, expected["volume_gib"] * GIB - 32 * 1024 * 1024)
        block_observed = isinstance(parse_stdout_json(inventory.get("block_layout")), dict)
        checks.append(check(f"{name}-guest-ext4-volume", "BLOCKED" if not block_observed else ("PASS" if ext4_ok else "FAIL"), "dedicated guest ext4 volume observed", ext4_rows))
        avail, avail_ev = df_available(inventory, mount_prefix)
        if name == "afs-accept-a":
            checks.append(check(f"{name}-data-reserve", "PASS" if avail is not None and avail >= 4 * GIB else "BLOCKED", "data volume has at least 4 GiB free; historical FUSE df errors do not hide valid ext4 row", avail_ev))
        elif name != "afs-accept-ctl":
            checks.append(check(f"{name}-data-reserve-observed", "PASS" if avail is not None and avail >= 4 * GIB else "BLOCKED", "data volume reserve observed", avail_ev))
        swap_off = isinstance(inventory.get("swap"), str) and "SwapTotal:" not in inventory["swap"] and len(inventory["swap"].strip().splitlines()) <= 1
        checks.append(check(f"{name}-swap-off", "PASS" if swap_off else "BLOCKED", "swap is absent for performance preparation", inventory.get("swap")))
        checks.append(check(f"{name}-fuse-present", "PASS" if inventory.get("fuse_present") is True else "BLOCKED", "FUSE device/module presence observed", inventory.get("fuse_present")))
        rdma_records = [inventory.get(k) for k in ("rdma_device", "rdma_links")]
        rdma_valid = all(isinstance(record, dict) and isinstance(record.get("stdout"), str) and record.get("status") == "OBSERVED" and record.get("returncode") == 0 for record in rdma_records)
        rdma_text = " ".join(record["stdout"] for record in rdma_records) if rdma_valid else ""
        checks.append(check(f"{name}-rxe-device-observed", "PASS" if all(token in rdma_text for token in ("rxe0", "ACTIVE", "RoCE v2")) else "BLOCKED", "RXE device metadata observed; not cross-VM verbs proof"))

    checks.append(check("topology-total-quota", "PASS" if sum((lima.get(n, {}).get("cpus") or 0) for n in EXPECTED_VMS) == 8 and sum((lima.get(n, {}).get("memory") or 0) for n in EXPECTED_VMS) == 22 * GIB else "FAIL", "fixed topology totals are 8 vCPU and 22 GiB"))
    for name, detail in DEFERRED.items():
        if name == "network-tls-fault-recovery":
            network_check = evaluate_network(bundle, artifact_root, refs)
            checks.append(network_check)
            if network_check["status"] != "PASS":
                limitations.append(detail)
            continue
        checks.append(check(name, "BLOCKED", detail))
        limitations.append(detail)
    status = worst_status(checks)
    return {"schema_version": 1, "status": status, "checks": checks, "limitations": limitations, "summary": {"pass": sum(c["status"] == "PASS" for c in checks), "blocked": sum(c["status"] == "BLOCKED" for c in checks), "fail": sum(c["status"] == "FAIL" for c in checks)}, "notes": ["Bounded preparation evaluation only.", "Generic PASS/text receipts do not satisfy semantic ENV-01 predicates.", "Do not mark ENV driver READY or acceptance.lock.json FROZEN from this report."]}


def evaluate_environment(lock: dict, bundle: dict, artifact_root: Path) -> dict:
    try:
        if not isinstance(lock, dict) or not isinstance(bundle, dict):
            raise InvalidEvidence("lock and bundle must be objects")
        return _evaluate_environment(lock, bundle, artifact_root)
    except (InvalidEvidence, OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        return {"schema_version": 1, "status": "BLOCKED", "checks": [check("invalid-environment-evidence", "BLOCKED", str(exc))], "limitations": ["Invalid evidence prevents evaluation; no ENV qualification."], "summary": {"pass": 0, "blocked": 1, "fail": 0}, "notes": []}


def qualification_errors(lock: dict, lock_path: Path) -> list[str]:
    if not isinstance(lock, dict):
        return ["environment lock must be an object"]
    evidence = lock.get("environment_evidence")
    if not isinstance(evidence, dict):
        return ["lock.environment_evidence is missing"]
    rel, expected_sha = evidence.get("path"), evidence.get("sha256")
    if not isinstance(rel, str) or not isinstance(expected_sha, str):
        return ["lock.environment_evidence requires path and sha256"]
    try:
        bundle_path = checked_path(lock_path.parent.resolve(), rel)
        if not bundle_path.is_file():
            return ["environment evidence bundle is missing"]
        actual_sha = sha256_file(bundle_path)
        if actual_sha != expected_sha:
            return [f"environment evidence bundle sha256 mismatch: {actual_sha}"]
        bundle = load_json(bundle_path)
    except InvalidEvidence as exc:
        return [str(exc)]
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        return [f"environment evidence bundle is malformed: {exc}"]
    if not isinstance(bundle, dict):
        return ["environment evidence bundle is malformed: root must be object"]
    report = evaluate_environment(lock, bundle, bundle_path.parent)
    return [f"{item['status']} {item['name']}: {item['detail']}" for item in report.get("checks", []) if item.get("status") != "PASS"]


def is_linux_arm64() -> bool:
    return sys.platform.startswith("linux") and platform.machine().lower() in {"aarch64", "arm64"}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Evaluate AFS environment preparation evidence")
    parser.add_argument("--lock", required=True)
    parser.add_argument("--bundle", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--contract", help="acceptance.md path; CLI hashes it and records bundle.contract")
    args = parser.parse_args(argv)
    lock_path, bundle_path, output_path = Path(args.lock).resolve(), Path(args.bundle).resolve(), Path(args.output).resolve()
    lock = load_json(lock_path)
    bundle = load_json(bundle_path)
    if not isinstance(lock, dict) or not isinstance(bundle, dict):
        raise SystemExit("lock and bundle must be JSON objects")
    if args.contract:
        cpath = Path(args.contract).resolve()
        try:
            rel = str(cpath.relative_to(bundle_path.parent))
        except ValueError:
            rel = cpath.name
        bundle = dict(bundle)
        bundle["contract"] = {"path": rel, "sha256": sha256_file(cpath)}
    report = evaluate_environment(lock, bundle, bundle_path.parent)
    if not is_linux_arm64():
        report["checks"].append(check("cli-linux-arm64-guard", "BLOCKED", "environment preparation CLI must run on Linux ARM64"))
        report["status"] = worst_status(report["checks"])
        report["summary"] = {"pass": sum(c["status"] == "PASS" for c in report["checks"]), "blocked": sum(c["status"] == "BLOCKED" for c in report["checks"]), "fail": sum(c["status"] == "FAIL" for c in report["checks"])}
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0 if report["status"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())

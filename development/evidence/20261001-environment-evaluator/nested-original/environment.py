#!/usr/bin/env python3
"""Conservative AFS environment preparation evaluator."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import sys
from pathlib import Path
from typing import Any

STATUS_ORDER = {"PASS": 0, "BLOCKED": 1, "FAIL": 2}
EXPECTED_KERNEL = "6.8.0-142-generic"
EXPECTED_IMAGE_SHA = "1ea801e659d2f5035ac294e0faab0aac9b6ba66753df933ba5c7beab0c689bd0"
GIB = 1024**3
EXPECTED_VMS = {
    "afs-accept-ctl": {"cpus": 2, "memory": 4 * GIB, "disk": 24 * GIB, "volume": "afsctlstate", "volume_gib": 8, "ip": "192.168.109.11", "inventory": "inventory-ctl.json"},
    "afs-accept-a": {"cpus": 2, "memory": 6 * GIB, "disk": 24 * GIB, "volume": "afsadata", "volume_gib": 32, "ip": "192.168.109.12", "inventory": "inventory-a.json"},
    "afs-accept-b": {"cpus": 2, "memory": 6 * GIB, "disk": 24 * GIB, "volume": "afsbdata", "volume_gib": 32, "ip": "192.168.109.13", "inventory": "inventory-b.json"},
    "afs-accept-c": {"cpus": 2, "memory": 6 * GIB, "disk": 24 * GIB, "volume": "afscdata", "volume_gib": 32, "ip": "192.168.109.14", "inventory": "inventory-c-rxe.json"},
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
    if rel.startswith("/") or ".." in Path(rel).parts:
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
        checked_path(Path("/tmp/root"), rel)
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
        return "OK", load_json(path)
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


def parse_stdout_json(record: Any) -> Any | None:
    if not isinstance(record, dict) or not isinstance(record.get("stdout"), str):
        return None
    try:
        return json.loads(record["stdout"])
    except json.JSONDecodeError:
        return None


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
    return isinstance(os_release, str) and "ID=ubuntu" in os_release and 'VERSION_ID="24.04"' in os_release


def address_mtu(inventory: dict[str, Any], expected_ip: str) -> tuple[bool, Any]:
    data = parse_stdout_json(inventory.get("addresses"))
    if not isinstance(data, list):
        return False, "missing ip address JSON"
    for iface in data:
        if isinstance(iface, dict) and iface.get("ifname") == "eth0":
            ips = [a.get("local") for a in iface.get("addr_info", []) if isinstance(a, dict) and a.get("family") == "inet"]
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
        mounts = [m for m in device.get("mountpoints", []) if isinstance(m, str)]
        if device.get("fstype") == "ext4" and any(m.startswith(prefix) for m in mounts):
            rows.append({"name": device.get("name"), "size": device.get("size"), "mountpoints": mounts})
    return any(isinstance(row.get("size"), int) and row["size"] >= min_size for row in rows), rows


def df_available(inventory: dict[str, Any], target: str) -> tuple[int | None, dict[str, Any]]:
    record = inventory.get("disk_space")
    stdout = record.get("stdout", "") if isinstance(record, dict) else ""
    rows = []
    for line in stdout.splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 7:
            row = {"source": parts[0], "fstype": parts[1], "available": parts[4], "target": parts[6]}
            rows.append(row)
            if parts[6] == target and parts[4].isdigit():
                return int(parts[4]), {"row": row, "df_status": record.get("status"), "returncode": record.get("returncode")}
    return None, {"rows": rows, "df_status": record.get("status") if isinstance(record, dict) else None}


def image_digest_ok(row: dict[str, Any]) -> bool:
    images = row.get("config", {}).get("images") if isinstance(row.get("config"), dict) else None
    if not isinstance(images, list):
        images = row.get("images")
    return isinstance(images, list) and any(isinstance(img, dict) and img.get("digest") == f"sha256:{EXPECTED_IMAGE_SHA}" for img in images)


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


def evaluate_environment(lock: dict, bundle: dict, artifact_root: Path) -> dict:
    checks: list[dict[str, Any]] = []
    limitations: list[str] = []
    try:
        refs = refs_from_bundle(bundle)
    except InvalidEvidence as exc:
        return {"schema_version": 1, "status": "BLOCKED", "checks": [check("bundle-shape", "BLOCKED", str(exc))], "limitations": [str(exc)], "summary": {"pass": 0, "blocked": 1, "fail": 0}, "notes": ["Invalid evidence shape; no ENV qualification made."]}
    check_contract(lock, bundle, artifact_root, checks)

    host_status, host = read_json_ref(artifact_root, refs, "host.json")
    if host_status == "OK" and isinstance(host, dict):
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
        inv_status, inv = read_json_ref(artifact_root, refs, expected["inventory"])
        inventory = inv if inv_status == "OK" and isinstance(inv, dict) else {}
        config_ok = row.get("status") == "Running" and row.get("hostname") == f"lima-{name}" and row.get("arch") == "aarch64" and row.get("cpus") == expected["cpus"] and row.get("memory") == expected["memory"] and row.get("disk") == expected["disk"]
        volume_ok = any(isinstance(d, dict) and d.get("name") == expected["volume"] and d.get("format") is True and d.get("fsType") == "ext4" for d in row.get("additionalDisks", []))
        checks.append(check(f"{name}-lima-config", "PASS" if config_ok and volume_ok and image_digest_ok(row) else "FAIL", "Lima running topology, hostname and actual image digest match contract", {"status": row.get("status"), "hostname": row.get("hostname"), "arch": row.get("arch"), "cpus": row.get("cpus"), "memory": row.get("memory"), "disk": row.get("disk"), "actual_image_ok": image_digest_ok(row)}))

        mem = memtotal_bytes(inventory)
        missing = inv_status == "MISSING" or not inventory or mem is None
        mismatch = bool(inventory) and (inventory.get("hostname") != f"lima-{name}" or inventory.get("architecture") != "aarch64" or inventory.get("cpu_count") != expected["cpus"] or inventory.get("kernel") != EXPECTED_KERNEL or not os_is_ubuntu_2404(inventory))
        mem_ok = mem is not None and mem >= int(expected["memory"] * 0.90)
        status = "BLOCKED" if missing else ("FAIL" if mismatch or not mem_ok else "PASS")
        checks.append(check(f"{name}-guest-identity", status, "guest hostname, arch, CPU, RAM, Ubuntu 24.04 and kernel observed", {"inventory_status": inv_status, "hostname": inventory.get("hostname"), "architecture": inventory.get("architecture"), "cpu_count": inventory.get("cpu_count"), "memtotal_bytes": mem, "kernel": inventory.get("kernel"), "ubuntu_2404": os_is_ubuntu_2404(inventory)}))

        ip_ok, ip_ev = address_mtu(inventory, expected["ip"])
        checks.append(check(f"{name}-ip-mtu", "PASS" if ip_ok else "BLOCKED", "fixed IPv4 and MTU 1500 observed", ip_ev))
        mount_prefix = f"/mnt/lima-{expected['volume']}"
        ext4_ok, ext4_rows = has_ext4_mount(inventory, mount_prefix, expected["volume_gib"] * GIB - 32 * 1024 * 1024)
        checks.append(check(f"{name}-guest-ext4-volume", "PASS" if ext4_ok else "FAIL", "dedicated guest ext4 volume observed", ext4_rows))
        avail, avail_ev = df_available(inventory, mount_prefix)
        if name == "afs-accept-a":
            checks.append(check(f"{name}-data-reserve", "PASS" if avail is not None and avail >= 4 * GIB else "BLOCKED", "data volume has at least 4 GiB free; historical FUSE df errors do not hide valid ext4 row", avail_ev))
        elif name != "afs-accept-ctl":
            checks.append(check(f"{name}-data-reserve-observed", "PASS" if avail is not None and avail >= 4 * GIB else "BLOCKED", "data volume reserve observed", avail_ev))
        swap_off = isinstance(inventory.get("swap"), str) and "SwapTotal:" not in inventory["swap"] and len(inventory["swap"].strip().splitlines()) <= 1
        checks.append(check(f"{name}-swap-off", "PASS" if swap_off else "BLOCKED", "swap is absent for performance preparation", inventory.get("swap")))
        checks.append(check(f"{name}-fuse-present", "PASS" if inventory.get("fuse_present") is True else "BLOCKED", "FUSE device/module presence observed", inventory.get("fuse_present")))
        rdma_text = " ".join(str((inventory.get(k) or {}).get("stdout", "")) for k in ("rdma_device", "rdma_links"))
        checks.append(check(f"{name}-rxe-device-observed", "PASS" if all(token in rdma_text for token in ("rxe0", "ACTIVE", "RoCE v2")) else "BLOCKED", "RXE device metadata observed; not cross-VM verbs proof"))

    checks.append(check("topology-total-quota", "PASS" if sum((lima.get(n, {}).get("cpus") or 0) for n in EXPECTED_VMS) == 8 and sum((lima.get(n, {}).get("memory") or 0) for n in EXPECTED_VMS) == 22 * GIB else "FAIL", "fixed topology totals are 8 vCPU and 22 GiB"))
    for name, detail in DEFERRED.items():
        checks.append(check(name, "BLOCKED", detail))
        limitations.append(detail)
    status = worst_status(checks)
    return {"schema_version": 1, "status": status, "checks": checks, "limitations": limitations, "summary": {"pass": sum(c["status"] == "PASS" for c in checks), "blocked": sum(c["status"] == "BLOCKED" for c in checks), "fail": sum(c["status"] == "FAIL" for c in checks)}, "notes": ["Bounded preparation evaluation only.", "Generic PASS/text receipts do not satisfy semantic ENV-01 predicates.", "Do not mark ENV driver READY or acceptance.lock.json FROZEN from this report."]}


def qualification_errors(lock: dict, lock_path: Path) -> list[str]:
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

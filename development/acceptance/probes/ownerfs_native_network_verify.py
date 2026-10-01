#!/usr/bin/env python3
"""Offline verification of frozen cross-VM evidence, without tar extraction.

Checksums and identity are checked separately from case assertions. A successful
check proves this experiment's bounded scope, not architecture/performance PASS.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tarfile
import tomllib


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    directory = args.directory
    result = json.loads((directory / "result.json").read_text())
    assert result["cases_ok"] and not result.get("phase_pass")
    recovery = directory / "archive-recovery.json"
    recovery = json.loads(recovery.read_text()) if recovery.exists() else {}
    summary = {"run_id": result["run_id"], "constructor": result["expected_constructor"],
               "architecture_phase_pass": False, "roles": {}}
    boot_ids = set()
    node_ids = {}
    for role in ("ctl", "a", "b"):
        archive_name = recovery.get(role, {}).get("archive", role + "-raw.tar.gz")
        expected = recovery.get(role, {}).get("sha256", result["raw_sha256"].get(role))
        archive_bytes = (directory / archive_name).read_bytes()
        assert sha(archive_bytes) == expected
        with tarfile.open(directory / archive_name) as archive:
            members = {member.name.removeprefix("./"): member for member in archive.getmembers()}

            def read(name):
                member = members[name]
                assert member.isfile() and member.size < 16 * 1024 * 1024
                return archive.extractfile(member).read()

            if "evidence-files.sha256" in members:
                for line in read("evidence-files.sha256").decode().splitlines():
                    checksum, name = line.split("  ", 1)
                    assert sha(read(name)) == checksum, name
            cfg = tomllib.loads(read("config.toml").decode())
            record = json.loads(read("node-identity.json"))
            expected_binary = result["input_sha256"]["afs-meta" if role == "ctl" else "node-tests"]
            assert record == result["processes"][role]
            assert record["sha256"] == expected_binary
            assert cfg["id"].startswith("native-" + role + "-")
            assert cfg["fs"] == "ownerfs" and cfg["tls_server_name"] == "afs-cluster"
            assert cfg["tls_ca_certificate"] and cfg["tls_identity_certificate"] and cfg["tls_identity_private_key"]
            node_ids[role] = cfg["id"]
            boot_ids.add(record["boot_id"])
            assert json.loads(read("node-exit.json"))["exit"] == 0
            assert read("parent-mountinfo-before.txt") == read("parent-mountinfo-after.txt")
            metrics = read("metrics.txt").decode()
            counts = {}
            for line in metrics.splitlines():
                match = re.fullmatch(r'afs_ownerfiles_rpc_duration_seconds_count\{method="(?:OwnerFiles\.)?(\w+)",side="(client|server)"\} ([0-9.]+)', line)
                if match:
                    counts[match[1].lower().replace("_", "") + "-" + match[2]] = float(match[3])
            if role != "ctl":
                driver = json.loads(read("control/driver.json"))
                assert driver["pid"] == record["pid"] and driver["namespace"] == record["namespace"]
                assert cfg["data_mode"] == "grpc"
                assert cfg["meta_endpoint"] == "https://10.77.30.11:18500"
                assert cfg["advertise_endpoint"].startswith("https://" + result["hosts"][role] + ":")
                for actor_key, actor in result["actors"].items():
                    if not actor_key.startswith(role + "-"):
                        continue
                    name = actor_key.split("-", 1)[1]
                    assert json.loads(read(f"actor-{name}/ready.json")) == actor
                    assert actor["process"]["namespace"] == record["namespace"]
                    assert actor["process"]["boot_id"] == record["boot_id"]
                    assert json.loads(read(f"actor-{name}/actor-exit.json"))["exit"] == 0
            summary["roles"][role] = {"archive_sha256": expected, "process": record["pid"],
                                       "rpc_counts": counts}
            if "owned-tcp.json" in members:
                connections = json.loads(read("owned-tcp.json"))
                summary["roles"][role]["owned_tcp"] = connections
                if role == "b":
                    # 10.77.30.12:18502 in /proc/net/tcp little-endian notation.
                    assert any(line.split()[2] == "0C1E4D0A:4846" and line.split()[3] == "01"
                               for line in connections["tcp"]), "no owned B-to-A P2P socket"
    assert len(boot_ids) == 3, "three actual VM boot identities required"
    a = summary["roles"]["a"]["rpc_counts"]
    b = summary["roles"]["b"]["rpc_counts"]
    assert a.get("open-server", 0) > 0 and b.get("open-client", 0) > 0
    if result["expected_constructor"] == "native":
        assert result["passed"]
        active = result["activated"]
        assert active["state"] == "NativeActive"
        assert active["identity"]["home_node_id"] == node_ids["a"]
        assert active["source"] == result["actors"]["a-native"]["root"]
        assert active["covered_target"] == result["actors"]["a-oldfuse"]["root"]
        assert active["observed"]["source"] == active["source"]
        assert active["observed"]["covered_target"] == active["covered_target"]
        assert active["observed"]["namespace"] == active["identity"]["namespace"]
        assert a.get("read-server", 0) >= 2 and b.get("read-client", 0) >= 2
        assert a.get("write-server", 0) >= 2 and b.get("write-client", 0) >= 2
        summary["metrics_limit"] = "small-file read may use Open prefetch; Read RPC counts are not read syscall counts"
        detach = next(row["detach"] for row in result["cleanup"] if "detach" in row)
        assert detach["state"] == "Detached" and detach["observed"] is None
        reads = []
        missing = []
        extended = result.get("case_profile") == "a2-close-reopen-unlink-recreate"
        for line in (directory / "transcript.jsonl").read_text().splitlines():
            row = json.loads(line)
            if not row.get("input"):
                continue
            command = json.loads(row["input"])
            reply = json.loads(row["stdout"])
            assert row["exit"] == 0 and command["id"] == reply["id"]
            ip = row["command"][-2].removeprefix("lzc@")
            role = next(role for role, host in result["hosts"].items() if host == ip)
            if command.get("handle") == "missing":
                assert extended and command["operation"] == "open" and command["name"] == "identity"
                assert not reply["ok"] and reply["errno"] == 2
                missing.append((role, command["actor"]))
                continue
            assert reply["ok"]
            if command["operation"] == "read":
                reads.append((role, command["actor"], command["handle"], reply["result"]))
        expected_reads = [("b", "remote", "fresh", "original-A-data")]
        if extended:
            expected_reads += [("b", "remote", "warmed", "original-A-data")]
        for text in ("same-length-ABC", "short", "", "longer-native-value-after-empty"):
            expected_reads += [("b", "remote", "fresh", text), ("a", "oldfuse", "fresh", text)]
        if extended:
            expected_reads += [("b", "remote", "fresh", "longer-native-value-after-empty")]
        expected_reads += [("a", "native", "fresh", "remote-to-native"),
            ("a", "oldfuse", "old", "original-object"),
            ("a", "oldfuse", "fresh", "replacement-object"),
            ("b", "remote", "old", "original-object"),
            ("b", "remote", "fresh", "replacement-object"),
            ("a", "oldfuse", "old", "old-object-through-P2P"),
            ("a", "native", "fresh", "replacement-object"),
            ("a", "oldfuse", "old", "old-object-through-P2P"),
            ("b", "remote", "old", "old-object-through-P2P")]
        if extended:
            expected_reads += [("b", "remote", "fresh", "recreated-object"),
                ("a", "oldfuse", "fresh", "recreated-object"),
                ("a", "oldfuse", "old", "old-object-through-P2P"),
                ("b", "remote", "old", "old-object-through-P2P"),
                ("a", "oldfuse", "old", "old-still-isolated"),
                ("a", "native", "fresh", "recreated-object")]
            assert missing == [("a", "native"), ("a", "oldfuse"), ("b", "remote")]
            summary["fresh_missing_errno_verified"] = "ENOENT on native/local FUSE/remote FUSE"
        assert reads == expected_reads, reads
        summary["actual_read_assertions"] = len(reads)
    else:
        assert not result["negative_admission"]["ok"]
        assert "PermissionDenied" in result["negative_admission"]["error"]
        assert all(row.get("stop", {}).get("exit", 0) == 0 for row in result["cleanup"])
        # An archive collection failure remains failed in the original runner;
        # recovery proves artifacts only, not a retroactive full PASS.
        summary["original_runner_passed"] = result["passed"]
        summary["archive_recovery"] = bool(recovery)
    summary["checks_ok"] = True
    (directory / "verified.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()

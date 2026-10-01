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
    append_profile = result.get("case_profile") == "a3-single-write-append"
    assert not result.get("phase_pass")
    if append_profile:
        assert "append_semantics_ok" in result and result["cases_ok"] == result["append_semantics_ok"]
        assert not result.get("error")
        assert not any(any(key.endswith("error") for key in row) for row in result["cleanup"])
    else:
        assert result["cases_ok"]
    recovery = directory / "archive-recovery.json"
    recovery = json.loads(recovery.read_text()) if recovery.exists() else {}
    summary = {"run_id": result["run_id"], "constructor": result["expected_constructor"],
               "architecture_phase_pass": False, "roles": {}}
    boot_ids = set()
    node_ids = {}
    append_contents = {}
    actor_replies = {}
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
                if append_profile and role == "a":
                    for name in ("native-control", "remote-append"):
                        append_contents[name] = read(f"actor-native/content-{name}.bin")
                    if "sequential_append" in result:
                        assert read("actor-native/content-sequential-append.bin") == b"NNNNNNNNRRRR"
                for name in members:
                    match = re.fullmatch(r"actor-([a-zA-Z0-9_-]+)/reply-(c[0-9]+)\.json", name)
                    if match:
                        actor_replies[(role, match[1], match[2])] = json.loads(read(name))
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
        assert result["passed"] == result["cases_ok"]
        active = result["activated"]
        assert active["state"] == "NativeActive"
        assert active["identity"]["home_node_id"] == node_ids["a"]
        assert active["source"] == result["actors"]["a-native"]["root"]
        assert active["covered_target"] == result["actors"]["a-oldfuse"]["root"]
        assert active["observed"]["source"] == active["source"]
        assert active["observed"]["covered_target"] == active["covered_target"]
        assert active["observed"]["namespace"] == active["identity"]["namespace"]
        flock_profile = result.get("case_profile") == "a3-flock-object"
        if flock_profile:
            # Locks use authenticated NodeControl.OwnerSetLock, not the
            # OwnerFiles duration histogram. Reciprocal actual syscall
            # conflicts below qualify arbitration; no invented metric count.
            summary["lock_route_limit"] = "NodeControl locks have no OwnerFiles duration count; actual syscall conflicts and authenticated peer socket are checked"
            assert result["actors"]["a-nativepeer"]["root"] == active["source"]
        elif append_profile:
            assert a.get("write-server", 0) >= 2 and b.get("write-client", 0) >= 2
            assert result["actors"]["a-nativepeer"]["root"] == active["source"]
        else:
            assert a.get("read-server", 0) >= 2 and b.get("read-client", 0) >= 2
            assert a.get("write-server", 0) >= 2 and b.get("write-client", 0) >= 2
        summary["metrics_limit"] = "small-file read may use Open prefetch; Read RPC counts are not read syscall counts"
        detach = next(row["detach"] for row in result["cleanup"] if "detach" in row)
        assert detach["state"] == "Detached" and detach["observed"] is None
        reads = []
        missing = []
        flocks = []
        append_replies = {}
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
            if "actor" in command and "ok" in reply:
                assert actor_replies[(role, command["actor"], command["id"])] == reply
            if append_profile and ("submitted" in reply or "operation" not in command):
                if "submitted" in reply:
                    assert reply["submitted"] and command["operation"] == "append-series"
                elif "first_write" in reply:
                    assert reply["first_write"]
                else:
                    assert reply["ok"]
                    append_replies[reply["id"]] = reply
                continue
            if command["operation"] == "flock":
                assert flock_profile
                errno = 0 if reply["ok"] else reply["errno"]
                assert errno in (0, 11)
                flocks.append((role, command["actor"], command["handle"], command["mode"], errno))
                continue
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
        if append_profile:
            assert not flocks and not missing
            expected_reads = [("b", "remote", "fresh", "original-A-data")]
            outcomes = {}
            for name, content in append_contents.items():
                lane = result["append_lanes"][name]
                assert len(content) == 2609152 and content.count(b"N") == 512000
                assert content.count(b"R") == 2097152
                assert sha(content) == lane["content"]["sha256"]
                first, last = content.index(b"R"), content.rindex(b"R")
                assert first > 0 and last < len(content) - 1, "append program did not bracket the large writer"
                contiguous = content[first:last + 1] == b"R" * 2097152
                assert append_replies[lane["small"]["id"]] == lane["small"]
                assert append_replies[lane["large"]["id"]] == lane["large"]
                assert lane["small"]["result"]["writes"] == 500
                assert lane["small"]["result"]["bytes_each"] == 1024
                assert lane["large"]["result"]["writes"] == 1
                assert lane["large"]["result"]["bytes_each"] == 2097152
                position_ok = lane["large"]["result"]["positions"] == [last + 1]
                assert contiguous == lane["one_contiguous_append"]
                assert position_ok == lane["write_position_matches_end"]
                outcomes[name] = {"contiguous": contiguous, "position_matches_end": position_ok,
                                  "actual_position": lane["large"]["result"]["positions"][0],
                                  "last_large_byte_end": last + 1}
            assert outcomes["native-control"]["contiguous"] and outcomes["native-control"]["position_matches_end"]
            remote = outcomes["remote-append"]
            semantic_ok = remote["contiguous"] and remote["position_matches_end"]
            if "sequential_append" in result:
                sequential = result["sequential_append"]
                assert sequential["native"]["positions"] == [8]
                position_ok = sequential["remote"]["positions"] == [12]
                assert sequential["position_ok"] == position_ok
                semantic_ok &= position_ok
                summary["sequential_append_outcome"] = {"actual_position": sequential["remote"]["positions"][0],
                                                         "required_position": 12, "position_ok": position_ok}
            assert result["append_semantics_ok"] == semantic_ok
            summary["append_outcomes"] = outcomes
            summary["semantic_case_passed"] = result["append_semantics_ok"]
            summary["append_limit"] = "one bounded interleaving; forensic checks_ok does not turn a semantic failure into PASS"
        elif flock_profile:
            expected_reads = [("b", "remote", "fresh", "original-A-data"),
                              ("b", "remote", "old", "original-object"),
                              ("a", "native", "fresh", "replacement-object")]
            expected_flocks = [
                ("a", "native", "old", "EX", 0),
                ("a", "nativepeer", "old", "EX", 11),
                ("b", "remote", "old", "EX", 11),
                ("a", "oldfuse", "old", "EX", 11),
                ("a", "native", "old", "UN", 0),
                ("b", "remote", "old", "EX", 0),
                ("a", "native", "old", "EX", 11),
                ("a", "oldfuse", "old", "EX", 11),
                ("b", "remote", "old", "UN", 0),
                ("a", "native", "old", "SH", 0),
                ("b", "remote", "old", "SH", 0),
                ("a", "nativepeer", "old", "EX", 11),
                ("b", "remote", "old", "UN", 0),
                ("a", "native", "old", "UN", 0),
                ("b", "remote", "old", "EX", 0),
                ("a", "native", "new", "EX", 0),
                ("a", "native", "old", "EX", 11),
                ("b", "remote", "old", "UN", 0),
                ("a", "native", "old", "EX", 0),
                ("b", "remote", "old", "EX", 11),
                ("a", "native", "old", "UN", 0),
                ("a", "nativepeer", "new", "EX", 0),
                ("a", "nativepeer", "new", "UN", 0),
            ]
            assert flocks == expected_flocks, flocks
            summary["flock_scope_verified"] = "nonblocking native/local FUSE/remote P2P; shared/exclusive; object replacement; native close"
            summary["flock_limit"] = "not POSIX process locks, blocking waiter recovery, or remote final-close drain"
            summary["actual_flock_assertions"] = len(flocks)
        else:
            assert not flocks
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

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


def verify_bulk(performance, trace):
    assert performance["workload"] == "bulk"
    matrix = performance["case_matrix"]
    shared = performance.get("slice") == "shared-file-attribution"
    assert len(matrix) == (4 if shared else 30)
    large, hot = 8 * 1024 ** 3, 512 * 1024 ** 2
    expected_cases = set()
    for concurrency in (1, 8):
        for barrier in ("close", "fdatasync", "fsync"):
            expected_cases.add(("seq-write", large, 1024 ** 2, concurrency, barrier, large, "guest-cold", "seq"))
        for cache in ("guest-cold", "repeat"):
            expected_cases.add(("seq-read", large, 1024 ** 2, concurrency, "close", large, cache, "seq"))
        for block in (4096, 65536):
            expected_cases.add(("random-read", large, block, concurrency, "close", hot, "guest-cold", "seq"))
            expected_cases.add(("random-read", hot, block, concurrency, "close", hot, "hot", "hot"))
            for barrier in ("close", "fdatasync", "fsync"):
                expected_cases.add(("random-write", hot, block, concurrency, barrier, hot, "hot", "write"))
    fields = ("operation", "file_bytes", "block_bytes", "concurrency", "barrier", "io_bytes", "cache", "dataset")
    if shared:
        expected_cases = {
            ("seq-write", large, 1024 ** 2, 1, "fsync", large, "guest-cold", "seq"),
            ("random-read", large, 4096, 1, "close", hot, "guest-cold", "seq"),
            ("random-read", hot, 65536, 1, "close", hot, "hot", "hot"),
            ("random-write", hot, 65536, 1, "close", hot, "hot", "write")}
    assert {tuple(case[key] for key in fields) for case in matrix} == expected_cases
    assert performance["warmup_rounds"] == 1 and performance["measured_rounds"] == 5
    assert set(performance["containers"]) == {"ext4", "native"}
    expected = {(case, lane, round_number) for case in range(len(matrix))
                for lane in ("ext4", "native") for round_number in range(6)}
    actual = set()
    io_replies = [row for row in trace if "/io" in row["argv"]]
    file_objects = {}
    for sample in performance["samples"]:
        key = sample["case"], sample["lane"], sample["round"]
        assert key not in actual
        actual.add(key)
        case = matrix[sample["case"]]
        measurement = sample["result"]
        if shared:
            object_key = sample["case"], sample["round"]
            if object_key in file_objects:
                assert file_objects[object_key] == measurement["file_object"]
            file_objects[object_key] = measurement["file_object"]
            assert measurement["file_object"]["device"] == performance["source"]["device"]
        assert sample["warmup"] == (sample["round"] == 0)
        assert all(measurement[key] == case[key] for key in fields[:6])
        assert measurement["content_ok"] and measurement["seed"] == 257
        assert measurement["cache_requested"] == case["cache"]
        assert measurement["cache_prepare_attempts"] in (1, 2, 3) if case["cache"] == "guest-cold" else measurement["cache_prepare_attempts"] == 0
        assert measurement["operations"] == case["io_bytes"] // case["block_bytes"]
        assert measurement["wall_ns"] > 0 and measurement["client_cpu_ns"] >= 0
        assert 0 <= measurement["barrier_ns"] <= measurement["wall_ns"]
        assert 0 < measurement["p50_ns"] <= measurement["p95_ns"] <= measurement["p99_ns"]
        assert 0 <= measurement["pattern_byte"] <= 255
        if case["cache"] == "guest-cold": assert measurement["resident_before_bytes"] == 0
        if case["cache"] == "hot": assert measurement["resident_before_bytes"] == case["file_bytes"]
        assert 0 <= measurement["resident_after_bytes"] <= case["file_bytes"]
        identifier = performance["containers"][sample["lane"]]["id"]
        argv = [identifier, "/io", sample["path"], case["operation"], str(case["file_bytes"]),
            str(case["block_bytes"]), str(case["concurrency"]), case["barrier"], str(case["io_bytes"]),
            str(measurement["pattern_byte"]), "existing", case["cache"]]
        matching = [row for row in io_replies if row["argv"][-len(argv):] == argv and json.loads(row["stdout"]) == measurement]
        assert len(matching) == 1
    assert actual == expected
    assert len(performance["workload_cleanup"]) == (3 if shared else 6)
    assert all(row["unlinked"] and row["file_bytes"] in (large, hot) for row in performance["workload_cleanup"])
    assert len(performance["dataset_setup"]) == (3 if shared else 6)
    assert all(row["result"]["content_ok"] and row["result"]["barrier"] == "fsync" for row in performance["dataset_setup"])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    directory = args.directory
    result = json.loads((directory / "result.json").read_text())
    append_profile = result.get("case_profile") == "a3-single-write-append"
    mixed_profile = result.get("case_profile") == "a4-mixed-constructor-cache"
    bulk_profile = result.get("case_profile") == "p23-container-native-ext4"
    container_profile = result.get("case_profile") in ("p1-container-native-ext4", "p23-container-native-ext4")
    performance_profile = result.get("case_profile") in ("p1-local-native-ext4", "p1-container-native-ext4", "p23-container-native-ext4")
    performance_field = "container_performance" if container_profile else "metadata_performance"
    performance_recovery = performance_profile and not container_profile and bool(result.get("error"))
    assert not result.get("phase_pass")
    if append_profile or mixed_profile:
        semantic_field = "append_semantics_ok" if append_profile else "mixed_semantics_ok"
        assert semantic_field in result and result["cases_ok"] == result[semantic_field]
        assert not result.get("error")
        assert not any(any(key.endswith("error") for key in row) for row in result["cleanup"])
    elif performance_recovery:
        assert not result["passed"] and "FileNotFoundError" in result["error"]
        assert "metadata_performance" in result
    else:
        assert result["cases_ok"]
    recovery = directory / "archive-recovery.json"
    recovery = json.loads(recovery.read_text()) if recovery.exists() else {}
    summary = {"run_id": result["run_id"], "constructor": result["expected_constructor"],
               "architecture_phase_pass": False, "roles": {}}
    if performance_recovery:
        summary["original_runner_passed"] = False
        summary["performance_recovery_limit"] = "completed timing payload only; original Actor-deadline/cleanup failure is retained"
    boot_ids = set()
    node_ids = {}
    append_contents = {}
    actor_replies = {}
    container_trace = []
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
                if "node_constructors" in result:
                    selected = result["node_constructors"][role]
                    assert driver["constructor"] == selected
                    assert json.loads(read("role.json"))["constructor"] == selected
                    summary["roles"].setdefault(role, {})["constructor"] = selected
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
                    actor_exit = json.loads(read(f"actor-{name}/actor-exit.json"))["exit"]
                    if performance_recovery:
                        assert actor_exit == 1
                        assert b"TimeoutError: actor lifetime exceeded" in read(f"actor-{name}/actor.log")
                    else:
                        assert actor_exit == 0
                if append_profile and role == "a":
                    for name in ("native-control", "remote-append"):
                        append_contents[name] = read(f"actor-native/content-{name}.bin")
                    if "sequential_append" in result:
                        assert read("actor-native/content-sequential-append.bin") == b"NNNNNNNNRRRR"
                if performance_profile and role == "a":
                    assert json.loads(read("container-performance.json" if container_profile else "metadata-performance.json")) == result[performance_field]
                    assert sha(read("benchmark")) == result["input_sha256"]["benchmark"]
                    if container_profile:
                        performance = result[performance_field]
                        for name in ("container-probe", "ownerfs_native_container.py") + (("io", "ownerfs_native_container_io.py") if bulk_profile else ()):
                            assert sha(read(name)) == result["input_sha256"][name]
                        for name, file in performance["rootfs_files"].items():
                            assert sha(read("container-rootfs" + name)) == file["sha256"]
                        for name in ("benchmark", "container-probe") + (("io",) if bulk_profile else ()):
                            assert performance["rootfs_files"]["/" + name]["sha256"] == result["input_sha256"][name]
                        for lane, container in performance["containers"].items():
                            config = read("oci-" + lane + "/config.json")
                            assert sha(config) == container["config_sha256"]
                            spec = json.loads(config)
                            assert spec["root"]["readonly"] and spec["root"]["path"].endswith("/container-rootfs")
                            assert spec["process"]["noNewPrivileges"]
                            assert spec["process"]["args"] == ["/container-probe", "--idle"]
                            assert spec["process"]["user"] == {"uid": 0, "gid": 0}
                            assert spec["process"]["capabilities"] == {key: [] for key in ("bounding", "effective", "inheritable", "permitted", "ambient")}
                            assert {entry["type"] for entry in spec["linux"]["namespaces"]} == {"mount", "pid", "network", "ipc", "uts", "cgroup"}
                            assert all("path" not in entry for entry in spec["linux"]["namespaces"])
                            assert spec["linux"]["rootfsPropagation"] == "private"
                            assert len(spec["mounts"]) == 2
                            assert spec["mounts"][0] == dict(destination="/proc", type="proc", source="proc", options=["nosuid", "noexec", "nodev"])
                            assert spec["mounts"][1] == dict(destination="/ownerfs/agent1", type="bind", source=container["source_path"], options=["bind", "rw", "rprivate", "nosuid", "nodev"])
                            process = container["process"]
                            assert process["sha256"] == result["input_sha256"]["container-probe"]
                            assert process["boot_id"] == record["boot_id"]
                            assert process["namespace"] != record["namespace"]
                            assert container["namespaces"]["mnt"] == process["namespace"]
                            assert container["state"]["pid"] == process["pid"] and container["state"]["status"] == "running"
                            isolation = container["isolation"]
                            assert isolation["workspace"] == container["source"]
                            assert isolation["parent"]["entries"] == ["agent1"] and isolation["parent"]["write_errno"] == 30
                            assert len(isolation["host_paths"]) == 8 and len(isolation["symlink_paths"]) == 3
                            assert all(row["errno"] in (2, 13, 20) for row in isolation["host_paths"] + isolation["symlink_paths"])
                            assert all("Cap" + key + ":\t0000000000000000" in isolation["status"] for key in ("Inh", "Prm", "Eff", "Bnd", "Amb"))
                            assert "NoNewPrivs:\t1" in isolation["status"]
                            workspace_mounts = [line for line in isolation["mountinfo"].splitlines() if line.split()[4] == "/ownerfs/agent1"]
                            assert len(workspace_mounts) == 1
                            if lane != "moosefs":
                                assert container["source"] == result["activated"]["source"]
                                assert isolation["fstype_magic"] == 0xef53 and " - ext4 " in workspace_mounts[0]
                                assert " - fuse" not in isolation["mountinfo"]
                            if lane == "native":
                                assert container["source_path"] == cfg["ownerfs_mount"] + "/agent1"
                            if lane == "ext4":
                                assert container["source_path"].startswith(cfg["data_dir"] + "/")
                        assert {row["lane"] for row in performance["cleanup"]} == set(performance["containers"])
                        assert all(row.get("deleted") and row["state"]["status"] == "stopped" for row in performance["cleanup"])
                        container_trace = [json.loads(line) for line in read("container-transcript.jsonl").decode().splitlines()]
                        assert all(row["exit"] == 0 for row in container_trace)
                        if bulk_profile:
                            assert performance["host_namespace_source_access"]["errno"] == 2
                            assert performance["host_namespace_source_access"]["namespace"] != record["namespace"]
                            lifecycle = performance["container_lifecycle"]
                            assert json.loads(read("control/reply-container-final-detach.json")) == lifecycle["original_export_detach"]
                            if performance.get("manager_detached"):
                                assert lifecycle["original_export_detach"]["ok"]
                                assert lifecycle["original_export_detach"]["result"]["state"] == "Detached"
                                after = lifecycle["after_original_detach"]
                                assert after["workspace"] == result["activated"]["source"] and after["fstype_magic"] == 0xef53
                                assert any("/container-probe" in row["argv"] and json.loads(row["stdout"]) == after for row in container_trace)
                for name in members:
                    match = re.fullmatch(r"actor-([a-zA-Z0-9_-]+)/reply-(c[0-9]+)\.json", name)
                    if match:
                        actor_replies[(role, match[1], match[2])] = json.loads(read(name))
            summary["roles"].setdefault(role, {}).update({"archive_sha256": expected, "process": record["pid"],
                                                       "rpc_counts": counts})
            if "owned-tcp.json" in members:
                connections = json.loads(read("owned-tcp.json"))
                summary["roles"][role]["owned_tcp"] = connections
                if role == "b" and not performance_profile:
                    # 10.77.30.12:18502 in /proc/net/tcp little-endian notation.
                    assert any(line.split()[2] == "0C1E4D0A:4846" and line.split()[3] == "01"
                               for line in connections["tcp"]), "no owned B-to-A P2P socket"
    assert len(boot_ids) == 3, "three actual VM boot identities required"
    a = summary["roles"]["a"]["rpc_counts"]
    b = summary["roles"]["b"]["rpc_counts"]
    assert a.get("open-server", 0) > 0 and b.get("open-client", 0) > 0
    if result["expected_constructor"] == "native":
        if not performance_recovery:
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
        elif mixed_profile:
            assert result["node_constructors"] == {"a": "native-eligible", "b": "ordinary"}
        elif performance_profile:
            assert result[performance_field]["source"] == active["source"]
            assert result[performance_field]["node"] == result["processes"]["a"]
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
        observed_reads = []
        extended = result.get("case_profile") == "a2-close-reopen-unlink-recreate"
        for line in (directory / "transcript.jsonl").read_text().splitlines():
            row = json.loads(line)
            if not row.get("input"):
                continue
            command = json.loads(row["input"])
            if performance_recovery and row["exit"]:
                assert command["operation"] in ("close", "quit")
                assert command["actor"] in ("oldfuse", "remote", "native")
                assert "FileNotFoundError" in row["stderr"]
                continue
            reply = json.loads(row["stdout"])
            if performance_profile and row["command"][-1].endswith(" container-performance" if container_profile else " metadata-performance"):
                assert row["exit"] == 0
                assert reply == result[performance_field]
                assert command["source"] == active["source"]
                continue
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
            if command["operation"] == "observe-read":
                assert mixed_profile and role == "b" and command["actor"] == "remote"
                observed_reads.append((command["name"], reply["result"]))
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
        if performance_profile:
            assert not flocks and not missing
            expected_reads = [("b", "remote", "fresh", "original-A-data")]
            performance = result[performance_field]
            assert performance["warmup_rounds"] == 1
            slice_name = performance.get("slice", "full")
            assert slice_name in ("full", "absolute-c1-repair", "shared-file-attribution")
            forms = ("absolute", "relative") if slice_name == "full" else ("absolute",)
            concurrencies = (1, 8) if slice_name == "full" else (1,)
            measured_rounds = 2 if slice_name == "absolute-c1-repair" else 5
            assert performance["measured_rounds"] == measured_rounds
            samples = performance["samples"]
            lanes = {sample["lane"] for sample in samples}
            assert lanes in ({"ext4", "native"}, {"ext4", "native", "moosefs"})
            expected = {(form, concurrency, round_number, lane)
                        for form in forms for concurrency in concurrencies
                        for round_number in range(1 + measured_rounds) for lane in lanes}
            actual = set()
            for sample in (() if bulk_profile else samples):
                measurement = sample["result"]
                key = (measurement["path_form"], measurement["concurrency"], sample["round"], sample["lane"])
                assert key not in actual
                actual.add(key)
                assert sample["warmup"] == (sample["round"] == 0)
                assert measurement["files"] == 10000 and measurement["file_bytes"] == 4096
                assert [phase["name"] for phase in measurement["phases"]] == [
                    "create_write_close", "stat", "read_close", "readdir", "rename", "unlink"]
                for phase in measurement["phases"]:
                    assert phase["wall_ns"] > 0 and phase["client_cpu_ns"] >= 0
                    assert phase["operations"] == (1 if phase["name"] == "readdir" else 10000)
                    if phase["name"] != "readdir":
                        assert 0 < phase["p50_ns"] <= phase["p95_ns"] <= phase["p99_ns"]
            if bulk_profile:
                verify_bulk(performance, container_trace)
                expected = set()
                summary["container_bulk_scope_verified"] = "P2/P3 local buffered OCI IO; guest residency checked; exact bytes/content/endpoint; no remote/MooseFS or host-cold verdict"
            assert actual == expected
            if container_profile and not bulk_profile:
                timed = [row for row in container_trace if "/benchmark" in row["argv"]]
                probes = [row for row in container_trace if "/container-probe" in row["argv"]]
                assert len(timed) == len(samples) and len(probes) == len(lanes)
                for sample, row in zip(samples, timed):
                    container = performance["containers"][sample["lane"]]
                    assert row["argv"][-5:] == [container["id"], "/benchmark", sample["path"], sample["result"]["path_form"], str(sample["result"]["concurrency"])]
                    assert json.loads(row["stdout"]) == sample["result"]
                    assert sample["path"].startswith("/ownerfs/agent1/perf-")
                for row in probes:
                    identifier = row["argv"][row["argv"].index("/container-probe") - 1]
                    container = next(record for record in performance["containers"].values() if record["id"] == identifier)
                    assert json.loads(row["stdout"]) == container["isolation"]
                summary["container_scope_verified"] = "actual OCI namespaces; original native source; bounded host-data isolation; P1 absolute/relative paired timings"
                summary["container_limit"] = "not production READY, Docker/Podman integration, hostile-container security qualification, lifecycle/fencing or bulk IO"
            summary["performance_samples_verified"] = len(samples)
            summary["performance_limit"] = (
                "local buffered container IO diagnostic only; no remote/MooseFS IO, host-cold, crash-durability or stage acceptance"
                if bulk_profile else
                "P1 local visibility diagnostic only; timing/identity checks do not qualify durability, MooseFS configuration history, remote/bulk workloads or stage acceptance")
        elif mixed_profile:
            assert not flocks and not missing
            expected_reads = [("b", "remote", "fresh", "original-A-data")]
            observations = result["mixed_observations"]
            assert observed_reads == [("data", observations["warm_data"]),
                                     ("data", observations["changed_data"]),
                                     ("identity", observations["warm_identity"]),
                                     ("identity", observations["replaced_identity"])]
            assert observations["warm_data"]["view"]["data"] == "original-A-data"
            assert observations["warm_identity"]["view"]["data"] == "original-object"
            window = observations["replaced_identity"]["finished_ns"] - observations["warm_identity"]["started_ns"]
            assert observations["replacement_window_ns"] == window
            assert observations["within_entry_ttl"] == (window < 1_000_000_000)
            changed = observations["changed_data"]["view"]
            replaced = observations["replaced_identity"]["view"]
            semantic_ok = (changed.get("ok") and changed.get("data") == "NEW-DATA" and changed.get("size") == 8
                           and replaced.get("ok") and replaced.get("data") == "replacement-object" and replaced.get("size") == 18)
            assert result["mixed_semantics_ok"] == bool(semantic_ok)
            summary["mixed_cache_outcome"] = {"changed_data": changed, "replaced_identity": replaced,
                                             "replacement_window_ns": window, "within_entry_ttl": window < 1_000_000_000}
            summary["semantic_case_passed"] = bool(semantic_ok)
            summary["mixed_cache_limit"] = "bounded data/replacement slice; mode admission/negotiation and full namespace semantics not qualified"
        elif append_profile:
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

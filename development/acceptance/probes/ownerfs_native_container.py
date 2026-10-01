#!/usr/bin/env python3
"""Finite OCI-container bind/isolation/P1 probe, not production Agent wiring.

The runtime resolves the original ready export in the verified Node mount
namespace. Each container has its own mount/PID/network/IPC/UTS namespaces,
readonly minimal rootfs, no capabilities and no host-root/proc bind.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time


def run(base, command, guest):
    cfg = guest.role_config(base)
    assert cfg["role"] == "a" and cfg["constructor"] == "native-eligible"
    for name in ("benchmark", "container-probe", "ownerfs_native_container.py"):
        assert guest.digest(base / name) == cfg["inputs"][name]
    bulk = command.get("workload") == "bulk"
    if bulk:
        for name in ("io", "ownerfs_native_container_io.py"):
            assert guest.digest(base / name) == cfg["inputs"][name]
    runtime = Path(shutil.which("runc"))
    version = subprocess.check_output([str(runtime), "--version"], text=True)
    current = guest.node(base)
    expected = command["source"]
    backing = None
    for directory, _, _ in os.walk(base / "data"):
        stat = os.stat(directory)
        if dict(device=stat.st_dev, inode=stat.st_ino) == expected:
            assert backing is None
            backing = Path(directory)
    assert backing is not None
    native = base / "mount/agent1"
    def source_stat(path):
        return json.loads(guest.in_namespace(base, ["/usr/bin/python3", "-c",
            "import os,json,sys; s=os.stat(sys.argv[1]); print(json.dumps(dict(device=s.st_dev,inode=s.st_ino)))", str(path)]).stdout)
    assert source_stat(native) == expected
    try:
        os.stat(native)
    except FileNotFoundError:
        host_source_access = dict(errno=2, namespace=os.readlink("/proc/self/ns/mnt"))
    else:
        raise AssertionError("private export unexpectedly visible in host namespace")
    # Known outside data really exist before the container is started.
    parent_marker = base / "mount/agent2/outside-sibling-marker"
    guest.in_namespace(base, ["/usr/bin/python3", "-c",
        "from pathlib import Path; import sys; p=Path(sys.argv[1]); p.mkdir(); (p/'outside-sibling-marker').write_text('host-sibling-secret')",
        str(parent_marker.parent)])
    host_marker = base / "outside-parent-marker"
    host_marker.write_text("host-parent-secret")
    backing_marker = backing.parent / "outside-backing-marker"
    backing_marker.write_text("backing-parent-secret")
    assert guest.in_namespace(base, ["cat", str(parent_marker)]).stdout == "host-sibling-secret"
    lanes = {"ext4": backing, "native": native}
    if command.get("moosefs_mount") and not bulk:
        moosefs = Path(command["moosefs_mount"])
        assert str(moosefs).startswith("/mnt/afsdata/ownerfs-native-moosefs/") and moosefs.name == "mount"
        mounted = json.loads(guest.in_namespace(base, ["findmnt", "-T", str(moosefs), "--json"]).stdout)["filesystems"][0]
        assert mounted["source"] == "mfs#10.77.30.11:19421" and mounted["target"] == str(moosefs)
        assert mounted["fstype"] in ("fuse", "fuse.mfs")
        mfs_source = moosefs / (base.name + "-container")
        mfs_source.mkdir()
        (mfs_source / "data").write_text("original-A-data")
        lanes["moosefs"] = mfs_source
    rootfs = base / "container-rootfs"
    rootfs.mkdir()
    for name in ("proc", "ownerfs/agent1", "bin"):
        (rootfs / name).mkdir(parents=True, exist_ok=True)
    manifest = {}
    def copy(source, destination):
        target = rootfs / destination.lstrip("/")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        target.chmod(0o755)
        manifest[destination] = dict(host_source=str(source), sha256=guest.digest(target))
    for executable, destination in ((base / "benchmark", "/benchmark"),
                                     (base / "container-probe", "/container-probe")):
        copy(executable, destination)
        dependencies = subprocess.check_output(["ldd", str(executable)], text=True)
        for library in re.findall(r"(/[^\s]+)", dependencies):
            copy(Path(library), library)
    if bulk:
        copy(base / "io", "/io")
    result = dict(source=expected, node=current, runtime=dict(path=str(runtime), version=version,
        sha256=guest.digest(runtime)), rootfs_files=manifest, visible_workspace="/ownerfs/agent1",
        host_namespace_source_access=host_source_access,
        warmup_rounds=1, measured_rounds=5, containers={}, samples=[], cleanup=[],
        scope="actual OCI containers; P1 warm visibility endpoint; not production Agent readiness or full performance acceptance")
    guest.save(base / "container-performance.json", result)
    state_root = base / "oci-state"
    state_root.mkdir()
    trace = open(base / "container-transcript.jsonl", "w", buffering=1)
    active = []
    def runtime_command(arguments, allow_error=False):
        argv = [str(runtime), "--root", str(state_root), "--log", str(base / "runc.log"),
                "--log-format", "json", *arguments]
        if arguments[0] == "run":
            # A detached init inherits stdout/stderr. Pipes would keep
            # communicate() waiting for the entire container lifetime even
            # after runc itself exits. Keep durable files for this boundary.
            stdout_path = base / (arguments[-1] + "-init.stdout")
            stderr_path = base / (arguments[-1] + "-init.stderr")
            with open(stdout_path, "w") as output, open(stderr_path, "w") as errors:
                started = subprocess.run(["nsenter", "--target", str(guest.node(base)["pid"]),
                    "--mount", "--", *argv], stdin=subprocess.DEVNULL, stdout=output, stderr=errors,
                    text=True, timeout=30)
            completed = subprocess.CompletedProcess(argv, started.returncode,
                stdout_path.read_text(), stderr_path.read_text())
        else:
            completed = guest.in_namespace(base, argv) if not allow_error else subprocess.run(
                ["nsenter", "--target", str(guest.node(base)["pid"]), "--mount", "--", *argv],
                capture_output=True, text=True)
        trace.write(json.dumps(dict(argv=argv, exit=completed.returncode, stdout=completed.stdout, stderr=completed.stderr)) + "\n")
        assert allow_error or completed.returncode == 0
        return completed
    def check(lane):
        record = result["containers"][lane]
        observed = json.loads(runtime_command(["state", record["id"]]).stdout)
        assert observed["status"] == "running" and observed["pid"] == record["process"]["pid"]
        guest.verify(record["process"])
    try:
        for lane, source in lanes.items():
            bundle = base / ("oci-" + lane)
            bundle.mkdir()
            identifier = base.name + "-" + lane
            spec = dict(ociVersion="1.0.2", root=dict(path=str(rootfs), readonly=True),
                hostname="dms-bind-probe", process=dict(terminal=False, user=dict(uid=0, gid=0),
                    args=["/container-probe", "--idle"], cwd="/", env=["PATH=/bin"], noNewPrivileges=True,
                    capabilities={key: [] for key in ("bounding", "effective", "inheritable", "permitted", "ambient")}),
                mounts=[dict(destination="/proc", type="proc", source="proc", options=["nosuid", "noexec", "nodev"]),
                        dict(destination="/ownerfs/agent1", type="bind", source=str(source),
                             options=["bind", "rw", "rprivate", "nosuid", "nodev"])],
                linux=dict(rootfsPropagation="private", cgroupsPath="/dms-native-bind/" + base.name + "/" + lane,
                           namespaces=[dict(type=kind) for kind in ("mount", "pid", "network", "ipc", "uts", "cgroup")]))
            guest.save(bundle / "config.json", spec)
            runtime_command(["run", "--detach", "--bundle", str(bundle), identifier])
            active.append((lane, identifier))
            state = json.loads(runtime_command(["state", identifier]).stdout)
            process = guest.identity(state["pid"])
            assert state["status"] == "running" and process["namespace"] != current["namespace"]
            namespace_paths = {kind: os.readlink(f"/proc/{state['pid']}/ns/{kind}")
                               for kind in ("mnt", "pid", "net", "ipc", "uts", "cgroup")}
            assert all(namespace_paths[kind] != os.readlink(f"/proc/{current['pid']}/ns/{kind}")
                       for kind in namespace_paths)
            expected_lane = source_stat(source)
            result["containers"][lane] = dict(id=identifier, process=process, state=state,
                namespaces=namespace_paths, source_path=str(source), source=expected_lane,
                config_sha256=guest.digest(bundle / "config.json"))
            check(lane)
            probe = json.loads(runtime_command(["exec", identifier, "/container-probe",
                str(expected_lane["device"]), str(expected_lane["inode"]), str(current["pid"]),
                str(host_marker), str(parent_marker), str(backing_marker), str(base)]).stdout)
            assert probe["workspace"] == expected_lane
            assert all(row["errno"] in (2, 13, 20) for row in probe["host_paths"] + probe["symlink_paths"])
            workspace_mounts = [line for line in probe["mountinfo"].splitlines() if line.split()[4] == "/ownerfs/agent1"]
            assert len(workspace_mounts) == 1
            assert " - ext4 " in workspace_mounts[0] if lane != "moosefs" else " - fuse " in workspace_mounts[0]
            if lane != "moosefs":
                assert probe["fstype_magic"] == 0xef53
                assert not any(" - fuse" in line for line in probe["mountinfo"].splitlines())
            assert all("Cap" + key + ":\t0000000000000000" in probe["status"] for key in ("Inh", "Prm", "Eff", "Bnd", "Amb"))
            assert "NoNewPrivs:\t1" in probe["status"]
            assert (backing / "container-own-marker").read_text() == "container-owned-data" if lane != "moosefs" else (source / "container-own-marker").read_text() == "container-owned-data"
            result["containers"][lane]["isolation"] = probe
            guest.save(base / "container-performance.json", result)
        if bulk:
            from ownerfs_native_container_io import run_suite
            run_suite(base, result, lanes, runtime_command, check, guest, command.get("slice", "full"))
        for form in (() if bulk else ("absolute", "relative")):
            for concurrency in (1, 8):
                for round_number in range(6):
                    order = tuple(lanes) if round_number % 2 == 0 else tuple(reversed(lanes))
                    for lane in order:
                        check(lane)
                        # Identical visible path depth/length between lanes.
                        suffix = {"ext4": "ext4", "native": "bind", "moosefs": "mfs0"}[lane]
                        path = f"/ownerfs/agent1/perf-{form}-{concurrency}-{round_number:02}-{suffix}"
                        before = dict(node_stat=Path(f"/proc/{current['pid']}/stat").read_text(),
                                      diskstats=Path("/proc/diskstats").read_text(), meminfo=Path("/proc/meminfo").read_text())
                        completed = runtime_command(["exec", result["containers"][lane]["id"],
                            "/benchmark", path, form, str(concurrency)])
                        result["samples"].append(dict(lane=lane, round=round_number, warmup=round_number == 0,
                            path=path, result=json.loads(completed.stdout), resources_before=before,
                            resources_after=dict(node_stat=Path(f"/proc/{current['pid']}/stat").read_text(),
                                diskstats=Path("/proc/diskstats").read_text(), meminfo=Path("/proc/meminfo").read_text())))
                        guest.save(base / "container-performance.json", result)
        if bulk:
            # Independent namespace bind references can outlive the original
            # export. Observe this only after every performance task closes.
            check("native")
            detached = guest.driver_command(base, dict(id="container-final-detach", operation="detach"))
            result["container_lifecycle"] = dict(original_export_detach=detached)
            if detached["ok"] and detached["result"]["state"] == "Detached":
                result["manager_detached"] = True
                check("native")
                identifier = result["containers"]["native"]["id"]
                after = json.loads(runtime_command(["exec", identifier, "/container-probe",
                    str(expected["device"]), str(expected["inode"]), str(current["pid"]),
                    str(host_marker), str(parent_marker), str(backing_marker), str(base)]).stdout)
                assert after["workspace"] == expected and after["fstype_magic"] == 0xef53
                result["container_lifecycle"]["after_original_detach"] = after
                result["container_lifecycle"]["limit"] = "original normal umount is not container drain/fencing; managed container must stop/unmount before backing reuse"
            guest.save(base / "container-performance.json", result)
        result["cases_ok"] = True
    finally:
        for lane, identifier in reversed(active):
            try:
                check(lane)
                runtime_command(["kill", identifier, "TERM"])
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    state = json.loads(runtime_command(["state", identifier]).stdout)
                    if state["status"] == "stopped": break
                    time.sleep(.05)
                assert state["status"] == "stopped"
                runtime_command(["delete", identifier])
                result["cleanup"].append(dict(lane=lane, state=state, deleted=True))
            except Exception as error:
                result["cleanup"].append(dict(lane=lane, error=repr(error)))
        guest.save(base / "container-performance.json", result)
        trace.close()
    assert all(row.get("deleted") for row in result["cleanup"])
    return result

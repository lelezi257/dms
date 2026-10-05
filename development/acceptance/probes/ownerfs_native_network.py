#!/usr/bin/env python3
"""Finite cross-VM architecture experiments using the test-only Node driver.

WSL builds, Ubuntu VMs execute. The controller can invoke Windows OpenSSH with
explicit paths; no WSL key permission workaround or shared backing shortcuts.
--expect ordinary records the required negative admission control. --expect
native records A2 retained-object/close-to-open or A3 nonblocking flock evidence,
selected by --case; neither profile is a performance or whole-stage PASS.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import uuid


def digest(path):
    with open(path, "rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ssh", default="ssh")
    parser.add_argument("--scp", default="scp")
    parser.add_argument("--key", required=True)
    parser.add_argument("--known-hosts", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--test-bin", type=Path, required=True)
    parser.add_argument("--meta-bin", type=Path, required=True)
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--expect", choices=("ordinary", "native"), required=True)
    parser.add_argument("--case", choices=("a2", "flock", "append", "mixed", "metadata-perf", "container-perf", "container-bulk", "closeout-architecture", "closeout-performance"), default="a2")
    parser.add_argument("--benchmark-bin", type=Path)
    parser.add_argument("--container-probe-bin", type=Path)
    parser.add_argument("--io-bin", type=Path)
    parser.add_argument("--moosefs-mount", type=Path)
    parser.add_argument("--performance-slice", choices=("full", "absolute-c1-repair", "shared-file-attribution"), default="full")
    args = parser.parse_args()
    container_case = args.case in ("container-perf", "container-bulk")
    container_inputs = ("container-probe", "ownerfs_native_container.py") + (("io", "ownerfs_native_container_io.py") if args.case == "container-bulk" else ())
    assert args.case == "a2" or args.expect == "native"
    if args.case in ("metadata-perf", "container-perf", "container-bulk", "closeout-performance"):
        assert args.benchmark_bin is not None
    if container_case:
        assert args.container_probe_bin is not None
    if args.case == "container-bulk":
        assert args.io_bin is not None
    if args.performance_slice == "shared-file-attribution":
        assert args.case == "container-bulk"
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S")
    nonce = uuid.uuid4().hex[:8]
    run = f"network-probe-{stamp}-{nonce}"
    out = args.output / run
    out.mkdir(parents=True)
    original_sha256 = {"node-tests": digest(args.test_bin), "afs-meta": digest(args.meta_bin)}
    # Only DWARF removal for transfer. The executed digest and original input
    # digest are separate evidence; this does not turn a debug build into release.
    with tempfile.TemporaryDirectory(prefix="native-network-binaries-") as temporary:
        for name, original in (("node-tests", args.test_bin), ("afs-meta", args.meta_bin)):
            stripped = Path(temporary) / name
            shutil.copyfile(original, stripped)
            subprocess.run(["strip", "--strip-debug", str(stripped)], check=True)
            shutil.copyfile(stripped, out / name)
    shutil.copyfile(Path(__file__).with_name("ownerfs_native_network_guest.py"), out / "guest.py")
    shutil.copyfile(__file__, out / "controller.py")
    if args.benchmark_bin:
        shutil.copyfile(args.benchmark_bin, out / "benchmark")
    if container_case:
        shutil.copyfile(args.container_probe_bin, out / "container-probe")
        shutil.copyfile(Path(__file__).with_name("ownerfs_native_container.py"), out / "ownerfs_native_container.py")
    if args.case == "container-bulk":
        shutil.copyfile(args.io_bin, out / "io")
        shutil.copyfile(Path(__file__).with_name("ownerfs_native_container_io.py"), out / "ownerfs_native_container_io.py")
    closeout = args.case in ("closeout-architecture", "closeout-performance")
    performance_closeout = args.case == "closeout-performance"
    if closeout:
        shutil.copyfile(Path(__file__).with_name("ownerfs_native_closeout_guest.py"), out / "ownerfs_native_closeout_guest.py")
    if performance_closeout:
        for name in ("ownerfs_native_closeout_performance.py", "ownerfs_native_closeout_controller.py"):
            shutil.copyfile(Path(__file__).with_name(name),out/name)
        shutil.copyfile(args.io_bin,out/"io-closeout")
        shutil.copyfile(args.container_probe_bin,out/"container-probe")
    shutil.copytree(args.snapshot, out / "source-inputs")
    options = ["-i", args.key, "-o", "BatchMode=yes", "-o", "ConnectTimeout=10",
               "-o", "UserKnownHostsFile=" + args.known_hosts]
    roles = {"ctl": "10.77.30.11", "a": "10.77.30.12", "b": "10.77.30.13"}
    bases = {role: ("/mnt/afsstate" if role == "ctl" else "/mnt/afsdata")
             + "/ownerfs-native-network/" + run for role in roles}
    trace = open(out / "transcript.jsonl", "w", buffering=1)
    active = []
    actors = []
    export = False
    sequence = 0
    result = {"run_id": run, "expected_constructor": args.expect,
              "case_profile": {"a2": "a2-close-reopen-unlink-recreate", "flock": "a3-flock-object",
                               "append": "a3-single-write-append", "mixed": "a4-mixed-constructor-cache", "metadata-perf": "p1-local-native-ext4", "container-perf": "p1-container-native-ext4", "container-bulk": "p23-container-native-ext4", "closeout-architecture": "architecture-closeout", "closeout-performance": "performance-closeout"}[args.case],
              "scope": "actual Node bootstrap/Meta/TLS/P2P; test-only mount driver; bounded architecture cases, not performance",
              "input_sha256": {name: digest(out / name) for name in
                               ("node-tests", "afs-meta", "guest.py", "controller.py")},
              "snapshot": args.snapshot.name, "hosts": roles, "phase_pass": False}
    result["unstripped_binary_sha256"] = original_sha256
    result["node_constructors"] = {role: ("ordinary" if args.expect == "ordinary" or
                                         (args.case == "mixed" and role == "b") else "native-eligible")
                                   for role in ("a", "b")}
    result["binary_transform"] = "strip --strip-debug; architecture-only debug builds"
    if args.case in ("metadata-perf", "container-perf", "container-bulk", "closeout-performance"):
        result["scope"] = "optimized test-driver candidate: P1 local paired visibility workloads; not full performance acceptance"
        result["binary_transform"] = "strip --strip-debug; release profile caller must supply frozen optimized inputs"
        result["input_sha256"]["benchmark"] = digest(out / "benchmark")
    if performance_closeout:
        result["scope"] = "optimized actual five-lane OCI diagnostics; profile/counts define workload; not formal acceptance"
    if container_case:
        result["input_sha256"].update({name: digest(out / name) for name in container_inputs})
        result["scope"] = "actual OCI containers; native source and host-data isolation; local paired diagnostic"
    if closeout:
        result["input_sha256"]["ownerfs_native_closeout_guest.py"] = digest(out / "ownerfs_native_closeout_guest.py")
    if performance_closeout:
        for name in ("ownerfs_native_closeout_performance.py", "ownerfs_native_closeout_controller.py", "io-closeout", "container-probe"):
            result["input_sha256"][name]=digest(out/name)
    (out / "inputs.json").write_text(json.dumps(result, indent=2) + "\n")

    def execute(command, *, data=None, timeout=45):
        completed = subprocess.run(command, input=data, capture_output=True, text=True, timeout=timeout)
        trace.write(json.dumps({"command": command, "input": data, "exit": completed.returncode,
                               "stdout": completed.stdout, "stderr": completed.stderr}) + "\n")
        if completed.returncode:
            raise RuntimeError(f"command failed {completed.returncode}: {completed.stderr[-1200:]}")
        return completed.stdout

    def ssh(role, command, data=None, timeout=45):
        return execute([args.ssh, *options, "lzc@" + roles[role], command], data=data, timeout=timeout)

    def local_path(path):
        if args.scp.lower().endswith(".exe"):
            match = re.fullmatch(r"/mnt/([a-z])/(.*)", str(path.resolve()))
            assert match, "Windows scp staging must be on an explicit /mnt/<drive>/ path"
            return match[1].upper() + ":/" + match[2]
        return str(path)

    def guest(role, operation, extra=None, command=None):
        remote = "sudo -n python3 " + shlex.quote(bases[role] + "/guest.py") + " " + \
                 shlex.quote(bases[role]) + " " + shlex.quote(operation)
        if extra is not None:
            remote += " " + shlex.quote(extra)
        output = ssh(role, remote, None if command is None else json.dumps(command),
                     timeout=1800 if operation == "closeout-performance" else ((7200 if args.case == "container-bulk" else 1200) if operation in ("metadata-performance", "container-performance") else 45))
        return json.loads(output)

    def request(role, operation, **fields):
        nonlocal sequence
        sequence += 1
        command = {"id": f"c{sequence}", "operation": operation, **fields}
        reply = guest(role, "actor-command" if "actor" in fields else "driver", command=command)
        assert reply["ok"], reply
        return reply["result"]

    def start_actor(role, name):
        value = guest(role, "actor-start", name)
        actors.append((role, name))
        result.setdefault("actors", {})[role + "-" + name] = value
        return value

    def fresh_read(role, actor, name, expected):
        request(role, "open", actor=actor, name=name, handle="fresh")
        try:
            assert request(role, "read", actor=actor, handle="fresh") == expected
        finally:
            request(role, "close", actor=actor, handle="fresh")

    def expect_missing(role, actor, name):
        nonlocal sequence
        sequence += 1
        reply = guest(role, "actor-command", command={"id": f"c{sequence}",
                      "operation": "open", "actor": actor, "name": name, "handle": "missing"})
        assert not reply["ok"] and reply["errno"] == 2, reply
        result.setdefault("fresh_missing", []).append({"role": role, "actor": actor, "reply": reply})

    def flock(role, actor, handle, mode, errno):
        nonlocal sequence
        sequence += 1
        reply = guest(role, "actor-command", command={"id": f"c{sequence}",
                      "operation": "flock", "actor": actor, "handle": handle, "mode": mode})
        if errno:
            assert not reply["ok"] and reply["errno"] == errno, reply
        else:
            assert reply["ok"], reply
        result.setdefault("flock_results", []).append({"role": role, "actor": actor,
                      "handle": handle, "mode": mode, "errno": errno, "reply": reply})

    def submit(role, actor, **fields):
        nonlocal sequence
        sequence += 1
        command = {"id": f"c{sequence}", "actor": actor, **fields}
        reply = guest(role, "actor-submit", command=command)
        assert reply == {"id": command["id"], "submitted": True}
        return {"id": command["id"], "actor": actor}

    def append_lane(name, large_role, large_actor):
        request("a", "write", actor="native", name=name, data="")
        for role, actor in (("a", "native"), (large_role, large_actor)):
            request(role, "open", actor=actor, name=name, handle="append", flags=["O_RDWR", "O_APPEND"])
        small = submit("a", "native", operation="append-series", handle="append",
                       count=500, repeat=1024, byte="N", delay=.002)
        assert guest("a", "actor-progress", command=small)["first_write"]
        large = submit(large_role, large_actor, operation="append-series", handle="append",
                       count=1, repeat=2 * 1024 * 1024, byte="R")
        small_reply = guest("a", "actor-result", command=small)
        large_reply = guest(large_role, "actor-result", command=large)
        assert small_reply["ok"] and large_reply["ok"], (small_reply, large_reply)
        for role, actor in (("a", "native"), (large_role, large_actor)):
            request(role, "close", actor=actor, handle="append")
        content = request("a", "inspect-append", actor="native", name=name)
        assert content["size"] == 500 * 1024 + 2 * 1024 * 1024, content
        assert sum(length for byte, _, length in content["segments"] if byte == "N") == 500 * 1024
        assert sum(length for byte, _, length in content["segments"] if byte == "R") == 2 * 1024 * 1024
        regions = [segment for segment in content["segments"] if segment[0] == "R"]
        large_end = max(offset + length for _, offset, length in regions)
        lane = {"large_role": large_role, "large_actor": large_actor, "content": content,
                "small": small_reply, "large": large_reply,
                "one_contiguous_append": len(regions) == 1,
                "write_position_matches_end": large_reply["result"]["positions"] == [large_end]}
        result.setdefault("append_lanes", {})[name] = lane
        return lane["one_contiguous_append"] and lane["write_position_matches_end"]

    try:
        for role in roles:
            base = bases[role]
            tls = ("/mnt/afsstate" if role == "ctl" else "/mnt/afsdata") + "/env-rebuild/tls"
            cert = "meta" if role == "ctl" else "node-" + role
            port = {"ctl": 18500, "a": 18502, "b": 18504}[role]
            cfg = dict(role=role, ports=[port, port + 1], tls=tls, cert=cert,
                       inputs={name: result["input_sha256"][name] for name in
                               ("guest.py", "afs-meta" if role == "ctl" else "node-tests")})
            if role != "ctl":
                cfg["constructor"] = result["node_constructors"][role]
                if args.benchmark_bin:
                    cfg["inputs"]["benchmark"] = result["input_sha256"]["benchmark"]
                if container_case:
                    cfg["inputs"].update({name: result["input_sha256"][name] for name in container_inputs})
            if closeout and role != "ctl":
                cfg["inputs"]["ownerfs_native_closeout_guest.py"] = result["input_sha256"]["ownerfs_native_closeout_guest.py"]
            if performance_closeout and role != "ctl":
                for name in ("ownerfs_native_closeout_performance.py", "io-closeout", "container-probe"):
                    cfg["inputs"][name]=result["input_sha256"][name]
            directory = out / role
            directory.mkdir()
            (directory / "role.json").write_text(json.dumps(cfg) + "\n")
            fields = dict(id=f"native-{role}-{nonce}", fs="ownerfs", data_dir=base + "/data",
                          grpc_listen=f"0.0.0.0:{port}", rest_listen=f"0.0.0.0:{port + 1}",
                          tls_ca_certificate=tls + "/ca.pem", tls_identity_certificate=tls + f"/{cert}.pem",
                          tls_identity_private_key=tls + f"/{cert}-key.pem", tls_server_name="afs-cluster",
                          log_level="info", trace_enabled=False)
            if role == "ctl":
                fields["meta_store"] = "local-file"
            else:
                fields.update(meta_endpoint="https://10.77.30.11:18500",
                              advertise_endpoint=f"https://{roles[role]}:{port}",
                              uds_path=base + "/node.sock", ownerfs_mount=base + "/mount", data_mode="grpc")
            toml = "".join(key + " = " + json.dumps(value) + "\n" for key, value in fields.items())
            toml += "trusted_node_certs = { " + ", ".join(
                json.dumps(f"native-{node}-{nonce}") + " = " + json.dumps(tls + f"/node-{node}.pem")
                for node in ("a", "b")) + " }\n"
            (directory / "config.toml").write_text(toml)
            ssh(role, "test ! -e " + shlex.quote(base) + " && sudo -n install -d -m 0700 -o lzc -g lzc "
                + shlex.quote(base))
            for source in (out / "guest.py", out / ("afs-meta" if role == "ctl" else "node-tests"),
                           directory / "config.toml", directory / "role.json"):
                execute([args.scp, *options, local_path(source), "lzc@" + roles[role] + ":" + base + "/" + source.name],
                        timeout=90)
            if role != "ctl" and args.benchmark_bin:
                execute([args.scp, *options, local_path(out / "benchmark"), "lzc@" + roles[role] + ":" + base + "/benchmark"])
                ssh(role, "chmod 0755 " + shlex.quote(base + "/benchmark"))
            if role != "ctl" and container_case:
                for name in container_inputs:
                    execute([args.scp, *options, local_path(out / name), "lzc@" + roles[role] + ":" + base + "/" + name])
                ssh(role, "chmod 0755 " + shlex.quote(base + "/container-probe"))
                if args.case == "container-bulk":
                    ssh(role, "chmod 0755 " + shlex.quote(base + "/io"))
            ssh(role, "chmod 0755 " + shlex.quote(base + "/" + ("afs-meta" if role == "ctl" else "node-tests")))
            if closeout and role != "ctl":
                execute([args.scp, *options, local_path(out / "ownerfs_native_closeout_guest.py"), "lzc@" + roles[role] + ":" + base + "/ownerfs_native_closeout_guest.py"])
            if performance_closeout and role != "ctl":
                for name in ("ownerfs_native_closeout_performance.py", "io-closeout", "container-probe"):
                    execute([args.scp,*options,local_path(out/name),"lzc@"+roles[role]+":"+base+"/"+name])
                ssh(role,"chmod 0755 "+shlex.quote(base+"/io-closeout")+" "+shlex.quote(base+"/container-probe"))
            guest(role, "launch")
            active.append(role)
            until = time.monotonic() + 35
            while True:
                try:
                    result.setdefault("processes", {})[role] = guest(role, "ready")
                    break
                except RuntimeError:
                    if time.monotonic() >= until:
                        raise
                    time.sleep(.3)
            print(f"{run}: {role} actual process ready", flush=True)
        guest("a", "seed")
        start_actor("a", "oldfuse")
        start_actor("b", "remote")
        fresh_read("b", "remote", "data", "original-A-data")
        request("a", "open", actor="oldfuse", name="identity", handle="old", flags="O_RDWR")
        request("b", "open", actor="remote", name="identity", handle="old", flags="O_RDWR")
        if args.expect == "native" and args.case == "a2":
            request("b", "open", actor="remote", name="data", handle="warmed")
            assert request("b", "read", actor="remote", handle="warmed") == "original-A-data"
        if args.expect == "ordinary":
            sequence += 1
            reply = guest("a", "driver", command={"id": f"c{sequence}", "operation": "prepare", "name": "agent1"})
            assert not reply["ok"] and "Operation not permitted" in reply["error"], reply
            result["negative_admission"] = reply
            result["mechanism_result"] = "ordinary constructor denied native export; authenticated remote read succeeded"
        else:
            result["prepared"] = request("a", "prepare", name="agent1")
            export = True
            if args.case == "closeout-architecture":
                result['inflight_trace_start'] = guest('a','closeout',command=dict(action='trace-start'))
                inflight = submit('b','remote',operation='write',name='data',data='transition-data',truncate=False)['id']
                deadline=time.monotonic()+5
                while True:
                    physical=guest('a','closeout',command=dict(action='physical-check',source=result['prepared']['source'],data='transition-data'))
                    if physical['matches']: break
                    assert time.monotonic()<deadline, physical
                    time.sleep(.05)
                before=guest('b','closeout',command=dict(action='pending',actor='remote',id=inflight))
                assert not before['reply_exists'], before
                result['inflight_before_mount']=before
            result["activated"] = request("a", "activate")
            if args.case == "closeout-architecture":
                after=guest('b','closeout',command=dict(action='pending',actor='remote',id=inflight))
                assert not after['reply_exists'], after
                result['inflight_after_mount']=after
                result['inflight_trace_stop']=guest('a','closeout',command=dict(action='trace-stop'))
                assert result['inflight_trace_stop']['delayed']
                reply=guest('b','actor-result',command=dict(actor='remote',id=inflight))
                assert reply['ok'] and reply['result']==15, reply
                result['inflight_reply']=reply

            assert result["activated"]["state"] == "NativeActive"
            native = start_actor("a", "native")
            assert native["root"] == result["activated"]["source"], (native, result["activated"])
            assert native["root"]["device"] != result["actors"]["a-oldfuse"]["root"]["device"]
            if args.case in ("metadata-perf", "container-perf", "container-bulk", "closeout-performance"):
                # P1 has no retained-Actor semantics. Release these short-lived
                # bootstrap controls before a long benchmark, retaining their
                # ready/exit evidence. Each timed process enters the verified
                # Node namespace directly and closes its own descriptors.
                for role, actor in (("a", "oldfuse"), ("b", "remote")):
                    request(role, "close", actor=actor, handle="old")
                result["bootstrap_actor_exits"] = {}
                for role, actor in reversed(actors):
                    request(role, "quit", actor=actor)
                    exited = guest(role, "actor-wait", actor)
                    assert exited["exit"] == 0
                    result["bootstrap_actor_exits"][role + "-" + actor] = exited
                actors.clear()
                if performance_closeout:
                    from ownerfs_native_closeout_controller import run
                    result['closeout_performance']=run(out,result,guest,args)
                else:
                    result["container_performance" if container_case else "metadata_performance"] = guest("a", "container-performance" if container_case else "metadata-performance", command={
                    "source": result["activated"]["source"],
                    "workload": "bulk" if args.case == "container-bulk" else "metadata",
                    "moosefs_mount":str(args.moosefs_mount) if args.moosefs_mount else None,
                    "slice":args.performance_slice})
                result["mechanism_result"] = "local paired timing; remote and MooseFS strong durability not qualified by this profile"
                if container_case and result["container_performance"].get("manager_detached"):
                    export = False
            elif closeout:
                fresh_read('a','native','data','transition-data')
                fresh_read('a','oldfuse','data','transition-data')
                fresh_read('b','remote','data','transition-data')
                result['syscall_matrix']={role:guest(role,'closeout',command=dict(action='syscalls',source=result['activated']['source'])) for role in ('a','b')}
                result['remote_watch_ready']=guest('b','closeout',command=dict(action='watch-start'))
                guest('a','closeout',command=dict(action='watch-mutate'))
                result['remote_watch']=guest('b','closeout',command=dict(action='watch-finish'))
                result['mechanism_result']='actual Home write reply delayed across manager activation; old/native/remote reopen same data; mapping/watch evidence retains failures'
            elif args.case == "a2":
                for text in ("same-length-ABC", "short", "", "longer-native-value-after-empty"):
                    request("a", "write", actor="native", name="data", data=text)
                    fresh_read("b", "remote", "data", text)
                    fresh_read("a", "oldfuse", "data", text)
                # No live-refresh/snapshot assertion while this reader is open.
                request("b", "close", actor="remote", handle="warmed")
                fresh_read("b", "remote", "data", "longer-native-value-after-empty")
                request("b", "write", actor="remote", name="data", data="remote-to-native")
                fresh_read("a", "native", "data", "remote-to-native")
                request("a", "write", actor="native", name="replacement", data="replacement-object")
                request("a", "rename", actor="native", name="replacement", target="identity")
                for role, actor in (("a", "oldfuse"), ("b", "remote")):
                    assert request(role, "read", actor=actor, handle="old") == "original-object"
                    fresh_read(role, actor, "identity", "replacement-object")
                request("b", "write", actor="remote", handle="old", data="old-object-through-P2P")
                assert request("a", "read", actor="oldfuse", handle="old") == "old-object-through-P2P"
                fresh_read("a", "native", "identity", "replacement-object")
                request("a", "unlink", actor="native", name="identity")
                for role, actor in (("a", "oldfuse"), ("b", "remote")):
                    assert request(role, "read", actor=actor, handle="old") == "old-object-through-P2P"
                for role, actor in (("a", "native"), ("a", "oldfuse"), ("b", "remote")):
                    expect_missing(role, actor, "identity")
                request("a", "write", actor="native", name="identity", data="recreated-object")
                fresh_read("b", "remote", "identity", "recreated-object")
                fresh_read("a", "oldfuse", "identity", "recreated-object")
                for role, actor in (("a", "oldfuse"), ("b", "remote")):
                    assert request(role, "read", actor=actor, handle="old") == "old-object-through-P2P"
                request("b", "write", actor="remote", handle="old", data="old-still-isolated")
                assert request("a", "read", actor="oldfuse", handle="old") == "old-still-isolated"
                fresh_read("a", "native", "identity", "recreated-object")
                result["mechanism_result"] = "A2 retained objects and close-to-open candidate evidence"
            elif args.case == "flock":
                start_actor("a", "nativepeer")
                request("a", "open", actor="native", name="identity", handle="old", flags="O_RDWR")
                request("a", "open", actor="nativepeer", name="identity", handle="old", flags="O_RDWR")
                # Four distinct open descriptions, same Home object. Every
                # attempted acquisition is nonblocking and records actual errno.
                flock("a", "native", "old", "EX", 0)
                flock("a", "nativepeer", "old", "EX", 11)
                flock("b", "remote", "old", "EX", 11)
                flock("a", "oldfuse", "old", "EX", 11)
                flock("a", "native", "old", "UN", 0)
                flock("b", "remote", "old", "EX", 0)
                flock("a", "native", "old", "EX", 11)
                flock("a", "oldfuse", "old", "EX", 11)
                flock("b", "remote", "old", "UN", 0)
                flock("a", "native", "old", "SH", 0)
                flock("b", "remote", "old", "SH", 0)
                flock("a", "nativepeer", "old", "EX", 11)
                flock("b", "remote", "old", "UN", 0)
                flock("a", "native", "old", "UN", 0)
                flock("b", "remote", "old", "EX", 0)
                request("a", "write", actor="native", name="replacement", data="replacement-object")
                request("a", "rename", actor="native", name="replacement", target="identity")
                request("a", "open", actor="native", name="identity", handle="new", flags="O_RDWR")
                flock("a", "native", "new", "EX", 0)
                flock("a", "native", "old", "EX", 11)
                assert request("b", "read", actor="remote", handle="old") == "original-object"
                fresh_read("a", "native", "identity", "replacement-object")
                flock("b", "remote", "old", "UN", 0)
                flock("a", "native", "old", "EX", 0)
                flock("b", "remote", "old", "EX", 11)
                flock("a", "native", "old", "UN", 0)
                # Native final close must release the replacement's OFD lock.
                request("a", "close", actor="native", handle="new")
                request("a", "open", actor="nativepeer", name="identity", handle="new", flags="O_RDWR")
                flock("a", "nativepeer", "new", "EX", 0)
                flock("a", "nativepeer", "new", "UN", 0)
                for actor, handle in (("native", "old"), ("nativepeer", "old"), ("nativepeer", "new")):
                    request("a", "close", actor=actor, handle=handle)
                result["mechanism_result"] = "actual native/local FUSE/remote P2P nonblocking flock and retained-object isolation"
            elif args.case == "append":
                start_actor("a", "nativepeer")
                assert append_lane("native-control", "a", "nativepeer"), "native control violates assumed contract"
                result["append_semantics_ok"] = append_lane("remote-append", "b", "remote")
                # Controlled follow-up: cursor divergence without chunking or
                # concurrent in-flight writes. Hand-derived final offset is12.
                request("a", "write", actor="native", name="sequential-append", data="NNNN")
                request("b", "open", actor="remote", name="sequential-append", handle="append",
                        flags=["O_RDWR", "O_APPEND"])
                request("a", "open", actor="native", name="sequential-append", handle="append",
                        flags=["O_RDWR", "O_APPEND"])
                local = request("a", "append-series", actor="native", handle="append", count=1, repeat=4, byte="N")
                distant = request("b", "append-series", actor="remote", handle="append", count=1, repeat=4, byte="R")
                assert local["positions"] == [8]
                for role, actor in (("a", "native"), ("b", "remote")):
                    request(role, "close", actor=actor, handle="append")
                content = request("a", "inspect-append", actor="native", name="sequential-append")
                assert content["segments"] == [["N", 0, 8], ["R", 8, 4]]
                result["sequential_append"] = {"native": local, "remote": distant, "content": content,
                                               "position_ok": distant["positions"] == [12]}
                result["append_semantics_ok"] &= result["sequential_append"]["position_ok"]
                result["mechanism_result"] = "single 2MiB append versus native 1KiB appends, native/native control and actual remote chain"
            else:
                # Observe ordinary B behavior without turning semantic errno
                # failures into harness exceptions or silent admission waivers.
                warm_data = request("b", "observe-read", actor="remote", name="data")
                assert warm_data["view"]["ok"] and warm_data["view"]["data"] == "original-A-data"
                request("a", "write", actor="native", name="data", data="NEW-DATA")
                changed_data = request("b", "observe-read", actor="remote", name="data")
                warm_identity = request("b", "observe-read", actor="remote", name="identity")
                assert warm_identity["view"]["ok"] and warm_identity["view"]["data"] == "original-object"
                request("a", "atomic-replace", actor="native", name="identity", data="replacement-object")
                replaced_identity = request("b", "observe-read", actor="remote", name="identity")
                # Bounds on one VM clock: even the earliest lookup in the
                # warm call is less than one second old at the later call.
                window = replaced_identity["finished_ns"] - warm_identity["started_ns"]
                result["mixed_observations"] = {"warm_data": warm_data, "changed_data": changed_data,
                    "warm_identity": warm_identity, "replaced_identity": replaced_identity,
                    "replacement_window_ns": window, "within_entry_ttl": window < 1_000_000_000}
                result["mixed_semantics_ok"] = (changed_data["view"].get("data") == "NEW-DATA" and
                    changed_data["view"].get("size") == 8 and
                    replaced_identity["view"].get("data") == "replacement-object" and
                    replaced_identity["view"].get("size") == 18)
                result["mechanism_result"] = "native-eligible Home with ordinary remote; close/reopen data and cached-name replacement"
        if args.case not in ("metadata-perf", "container-perf", "container-bulk", "closeout-performance"):
            for role, actor in (("a", "oldfuse"), ("b", "remote")):
                request(role, "close", actor=actor, handle="old")
        result["cases_ok"] = result.get("append_semantics_ok", True) and result.get("mixed_semantics_ok", True)
    except Exception as error:
        result["error"] = repr(error)
    finally:
        cleanup = []
        if result.get("container_performance", {}).get("manager_detached"):
            cleanup.append({"role": "a", "detach": result["container_performance"]["container_lifecycle"]["original_export_detach"]["result"],
                            "scope": "post-timing container namespace lifecycle probe"})
        for role in active:
            try:
                cleanup.append({"role": role, "capture": guest(role, "capture")})
            except Exception as error:
                cleanup.append({"role": role, "capture_error": repr(error)})
        for role, actor in reversed(actors):
            try:
                # If a case failed with live handles, explicit quit refuses;
                # preserve that diagnostic, then stop only this exact actor.
                request(role, "quit", actor=actor)
                exit_record = guest(role, "actor-wait", actor)
                assert exit_record["exit"] == 0, exit_record
                cleanup.append({"role": role, "actor": actor, "actor_exit": exit_record})
            except Exception as error:
                cleanup.append({"role": role, "actor": actor, "error": repr(error)})
                try:
                    cleanup.append({"role": role, "actor": actor,
                                    "stopped_actor": guest(role, "actor-stop", actor)})
                except Exception as stop_error:
                    cleanup.append({"role": role, "actor": actor, "actor_stop_error": repr(stop_error)})
        if export:
            try:
                cleanup.append({"role": "a", "detach": request("a", "detach")})
                export = False
            except Exception as error:
                cleanup.append({"role": "a", "detach_error": repr(error)})
        for role in reversed(active):
            try:
                cleanup.append({"role": role, "stop": guest(role, "stop")})
            except Exception as error:
                cleanup.append({"role": role, "stop_error": repr(error)})
            try:
                frozen = guest(role, "archive")
                archive = out / (role + "-raw.tar.gz")
                execute([args.scp, *options, "lzc@" + roles[role] + ":" + frozen["path"],
                         local_path(archive)], timeout=90)
                result.setdefault("raw_sha256", {})[role] = digest(archive)
                assert result["raw_sha256"][role] == frozen["sha256"]
            except Exception as error:
                cleanup.append({"role": role, "archive_error": repr(error)})
        result["cleanup"] = cleanup
        result["passed"] = bool(result.get("cases_ok")) and not any(
            any(key.endswith("error") for key in row) or row.get("stop", {}).get("exit", 0) != 0
            for row in cleanup)
        (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        trace.close()
        print(json.dumps({"evidence": str(out), "passed": result["passed"], "error": result.get("error")}), flush=True)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

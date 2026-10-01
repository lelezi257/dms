#!/usr/bin/env python3
"""Finite cross-VM A2 experiment around the current test-only Node driver.

WSL builds, Ubuntu VMs execute. The controller can invoke Windows OpenSSH with
explicit paths; no WSL key permission workaround or shared backing shortcuts.
--expect ordinary records the required negative admission control. --expect
native records retained-object/close-to-open evidence, never a performance PASS.
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
    args = parser.parse_args()
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
              "case_profile": "a2-close-reopen-unlink-recreate",
              "scope": "actual Node bootstrap/Meta/TLS/P2P; test-only mount driver; A2 not performance",
              "input_sha256": {name: digest(out / name) for name in
                               ("node-tests", "afs-meta", "guest.py", "controller.py")},
              "snapshot": args.snapshot.name, "hosts": roles, "phase_pass": False}
    result["unstripped_binary_sha256"] = original_sha256
    result["binary_transform"] = "strip --strip-debug; architecture-only debug builds"
    (out / "inputs.json").write_text(json.dumps(result, indent=2) + "\n")

    def execute(command, *, data=None, timeout=45):
        completed = subprocess.run(command, input=data, capture_output=True, text=True, timeout=timeout)
        trace.write(json.dumps({"command": command, "input": data, "exit": completed.returncode,
                               "stdout": completed.stdout, "stderr": completed.stderr}) + "\n")
        if completed.returncode:
            raise RuntimeError(f"command failed {completed.returncode}: {completed.stderr[-1200:]}")
        return completed.stdout

    def ssh(role, command, data=None):
        return execute([args.ssh, *options, "lzc@" + roles[role], command], data=data)

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
        output = ssh(role, remote, None if command is None else json.dumps(command))
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

    try:
        for role in roles:
            base = bases[role]
            tls = ("/mnt/afsstate" if role == "ctl" else "/mnt/afsdata") + "/env-rebuild/tls"
            cert = "meta" if role == "ctl" else "node-" + role
            port = {"ctl": 18500, "a": 18502, "b": 18504}[role]
            cfg = dict(role=role, ports=[port, port + 1], tls=tls, cert=cert,
                       inputs={name: result["input_sha256"][name] for name in
                               ("guest.py", "afs-meta" if role == "ctl" else "node-tests")})
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
            ssh(role, "chmod 0755 " + shlex.quote(base + "/" + ("afs-meta" if role == "ctl" else "node-tests")))
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
        if args.expect == "native":
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
            result["activated"] = request("a", "activate")
            assert result["activated"]["state"] == "NativeActive"
            native = start_actor("a", "native")
            assert native["root"] == result["activated"]["source"], (native, result["activated"])
            assert native["root"]["device"] != result["actors"]["a-oldfuse"]["root"]["device"]
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
        for role, actor in (("a", "oldfuse"), ("b", "remote")):
            request(role, "close", actor=actor, handle="old")
        result["cases_ok"] = True
    except Exception as error:
        result["error"] = repr(error)
    finally:
        cleanup = []
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

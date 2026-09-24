#!/usr/bin/env python3
"""Run an installed dms-home package on three Linux VMs (A, B, center C)."""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
import platform
import shlex
import subprocess
import time
from pathlib import Path


def shell(role: str, script: str, hosts: dict[str, str], *, timeout: int = 30, check: bool = True) -> str:
    if role == "A":
        command = ["bash", "-lc", script]
    else:
        command = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", hosts[role], "bash", "-lc", shlex.quote(script)]
    result = subprocess.run(command, text=True, capture_output=True, timeout=timeout, check=False)
    if check and result.returncode:
        raise RuntimeError(f"{role} command failed ({result.returncode}): {script}\n{result.stdout[-1000:]}{result.stderr[-1000:]}")
    return result.stdout


def wait(predicate, description: str, timeout: float = 25.0) -> None:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            if predicate():
                return
        except Exception as error:  # readiness may fail while processes start
            last_error = str(error)
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {description}: {last_error}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=("nfs", "p2p"), required=True)
    parser.add_argument("--host-a", default="192.168.104.12")
    parser.add_argument("--host-b", default="192.168.104.13")
    parser.add_argument("--host-c", default="192.168.104.11")
    parser.add_argument("--ssh-user", default="lzc")
    parser.add_argument("--package", type=Path, required=True, help="same installed package path on A/B/C")
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("all filesystem acceptance must run on Linux")
    if not args.run_id.replace("-", "").isalnum():
        parser.error("run-id must use letters, numbers, and hyphens")
    if args.output.exists():
        parser.error("refusing to overwrite output")
    hosts = {"A": f"{args.ssh_user}@{args.host_a}", "B": f"{args.ssh_user}@{args.host_b}", "C": f"{args.ssh_user}@{args.host_c}"}
    addresses = {"A": args.host_a, "B": args.host_b, "C": args.host_c}
    root = f"/home/lzc.guest/dms-home-three-{args.run_id}"
    package = shlex.quote(str(args.package))
    binary = f"{package}/bin/dms-home"
    center = f"{args.host_c}:31991"
    management = f"{args.host_c}:31992"
    p2p = {"A": f"{args.host_a}:31993", "B": f"{args.host_b}:31993"}
    token = "three-vm-preview-test-token"
    steps: list[dict] = []
    started_processes: list[tuple[str, str]] = []
    start = time.time()
    status = "FAIL"
    failure = None

    def record(name: str, action):
        before = time.monotonic()
        try:
            detail = action()
            steps.append({"name": name, "status": "PASS", "ms": round((time.monotonic() - before) * 1000), "detail": detail})
            return detail
        except Exception as error:
            steps.append({"name": name, "status": "FAIL", "ms": round((time.monotonic() - before) * 1000), "error": str(error)})
            raise

    def start_process(role: str, name: str, command: str):
        script = f"nohup {command} >{root}/{name}.log 2>&1 </dev/null & echo $! >{root}/{name}.pid"
        shell(role, script, hosts)
        started_processes.append((role, name))

    def read_remote() -> str:
        return shell("B", f"python3 -c 'from pathlib import Path; print(Path(\"{root}/mnt/job-42/note\").read_bytes().decode())'", hosts).strip()

    try:
        def preflight():
            for role in ("A", "B", "C"):
                shell(role, f"test -x {binary} && cd {package} && sha256sum -c SHA256SUMS >/dev/null && mkdir -p {root}", hosts)
            for role in ("A", "B"):
                shell(role, f"mkdir -p {root}/data {root}/peers {root}/mnt", hosts)
            return {"hosts": addresses, "package": str(args.package)}
        record("installed package and three hosts", preflight)

        def launch():
            start_process("C", "center", f"env DMS_HOME_TOKEN={token} {binary} center {center} {management} {root}/center.state")
            wait(lambda: "ready" in shell("A", f"python3 -c 'import socket; socket.create_connection((\"{args.host_c}\",31991),1).close(); print(\"ready\")'", hosts), "center TCP")
            if args.backend == "nfs":
                for role in ("A", "B"):
                    shell(role, f"sudo -n {package}/scripts/setup-nfs.sh {root}/data 192.168.104.0/24 >/dev/null", hosts, timeout=60)
            for role in ("A", "B"):
                start_process(role, f"node-{role}",
                    f"env DMS_HOME_TOKEN={token} {binary} node {role} {center} {addresses[role]}:/ {p2p[role]} {root}/data {root}/peers {root}/mnt {args.backend}")
            for role in ("A", "B"):
                wait(lambda role=role: shell(role, f"mountpoint -q {root}/mnt && echo yes", hosts, check=False).strip() == "yes", f"{role} FUSE mount")
            if args.backend == "nfs":
                for role, peer in (("A", "B"), ("B", "A")):
                    wait(lambda role=role, peer=peer: shell(role, f"mountpoint -q {root}/peers/{peer} && echo yes", hosts, check=False).strip() == "yes", f"{role} NFS mount of {peer}", 40)
            return {"backend": args.backend}
        record("center, FUSE and peer backend ready", launch)

        def namespace_and_bytes():
            shell("A", f"mkdir {root}/mnt/job-42 && python3 -c 'from pathlib import Path; Path(\"{root}/mnt/job-42/note\").write_bytes(b\"first\")'", hosts)
            first = read_remote()
            if first != "first":
                raise RuntimeError(f"first remote read: {first!r}")
            shell("A", f"python3 -c 'from pathlib import Path; Path(\"{root}/mnt/job-42/note\").write_bytes(b\"second\")'", hosts)
            second = read_remote()
            if second != "second":
                raise RuntimeError(f"stale remote reopen: {second!r}")
            shell("B", f"mkdir {root}/mnt/job-42/sub && touch {root}/mnt/job-42/sub/from-b && mv {root}/mnt/job-42/sub/from-b {root}/mnt/job-42/sub/renamed", hosts)
            shell("A", f"test -f {root}/mnt/job-42/sub/renamed", hosts)
            return {"home": "A", "remote": "B", "after_close_reopen": second, "remote_mutation": "visible on A"}
        record("close-to-open and remote namespace", namespace_and_bytes)

        def single_root_owner_under_concurrent_create():
            creation = (
                "python3 -c 'import os; "
                f"p=\"{root}/mnt/job-race\"; "
                "\ntry: os.mkdir(p); print(\"created\")"
                "\nexcept FileExistsError: print(\"exists\")'"
            )
            with ThreadPoolExecutor(max_workers=2) as pool:
                attempts = {
                    role: pool.submit(shell, role, creation, hosts)
                    for role in ("A", "B")
                }
                outcomes = {role: future.result().strip() for role, future in attempts.items()}
            response = shell("A", f"python3 -c 'import urllib.request; opener=urllib.request.build_opener(urllib.request.ProxyHandler({{}})); print(opener.open(\"http://{management}/v1/roots/job-race\", timeout=3).read().decode())'", hosts)
            info = json.loads(response)
            owner = info.get("owner")
            physical = {role: shell(role, f"test -d {root}/data/job-race && echo yes || echo no", hosts).strip() for role in ("A", "B")}
            if owner not in ("A", "B") or info.get("status") != "active" or physical[owner] != "yes" or physical["B" if owner == "A" else "A"] != "no":
                raise RuntimeError(f"root authority split: {outcomes=}, {info=}, {physical=}")
            return {"attempts": outcomes, "owner": owner, "physical": physical}
        record("concurrent root create keeps one authority", single_root_owner_under_concurrent_create)

        def old_remote_fd_keeps_file_identity():
            shell("A", f"python3 -c 'from pathlib import Path; Path(\"{root}/mnt/job-42/identity-x\").write_bytes(b\"old-file\")'", hosts)
            holder = f"""
import json, os, pathlib, time
base = pathlib.Path({root!r})
fd = os.open(base / 'mnt/job-42/identity-x', os.O_RDWR)
(base / 'identity-ready').touch()
deadline = time.monotonic() + 20
while not (base / 'identity-go').exists():
    if time.monotonic() > deadline:
        raise RuntimeError('timed out waiting for rename')
    time.sleep(0.01)
before = os.fstat(fd).st_size
os.ftruncate(fd, 0)
os.pwrite(fd, b'old-only', 0)
os.fchmod(fd, 0o600)
os.fsync(fd)
after = os.pread(fd, 8, 0)
new_fd = os.open(base / 'mnt/job-42/identity-x', os.O_RDONLY)
new_bytes = os.read(new_fd, 8)
new_ino = os.fstat(new_fd).st_ino
os.close(new_fd)
os.chmod(base / 'mnt/job-42/identity-x', 0o640)
old_ino = os.fstat(fd).st_ino
os.close(fd)
print(json.dumps({{'before_size': before, 'old_fd_bytes': after.decode(), 'new_bytes': new_bytes.decode(), 'separate_inodes': old_ino != new_ino}}))
"""
            with ThreadPoolExecutor(max_workers=1) as pool:
                pending = pool.submit(shell, "B", "python3 -c " + shlex.quote(holder), hosts, timeout=30)
                wait(lambda: shell("B", f"test -f {root}/identity-ready && echo yes", hosts, check=False).strip() == "yes", "B old fd open")
                shell("A", f"python3 -c 'import os; from pathlib import Path; p=Path(\"{root}/mnt/job-42\"); os.rename(p/\"identity-x\", p/\"identity-y\"); (p/\"identity-x\").write_bytes(b\"new-file\")'", hosts)
                shell("B", f"touch {root}/identity-go", hosts)
                held = json.loads(pending.result())
            names = json.loads(shell("A", f"python3 -c 'import json,stat; from pathlib import Path; p=Path(\"{root}/mnt/job-42\"); print(json.dumps({{\"new\":(p/\"identity-x\").read_bytes().decode(),\"old\":(p/\"identity-y\").read_bytes().decode(),\"new_mode\":stat.S_IMODE((p/\"identity-x\").stat().st_mode),\"old_mode\":stat.S_IMODE((p/\"identity-y\").stat().st_mode)}}))'", hosts))
            if held != {"before_size": 8, "old_fd_bytes": "old-only", "new_bytes": "new-file", "separate_inodes": True} or names != {"new": "new-file", "old": "old-only", "new_mode": 0o640, "old_mode": 0o600}:
                raise RuntimeError(f"old fd followed recreated name: {held=}, {names=}")
            return {"old_fd": held, "home_files": names}
        record("remote old fd follows file identity after rename", old_remote_fd_keeps_file_identity)

        def unlinked_remote_fd_keeps_identity():
            shell("A", f"python3 -c 'from pathlib import Path; Path(\"{root}/mnt/job-42/unlinked-x\").write_bytes(b\"old-unlinked\")'", hosts)
            holder = f"""
import json, os, pathlib, time
base = pathlib.Path({root!r})
path = base / 'mnt/job-42/unlinked-x'
fd = os.open(path, os.O_RDWR)
(base / 'unlinked-ready').touch()
deadline = time.monotonic() + 20
while not (base / 'unlinked-go').exists():
    if time.monotonic() > deadline:
        raise RuntimeError('timed out waiting for unlink')
    time.sleep(0.01)
os.ftruncate(fd, 0)
os.pwrite(fd, b'old-fd', 0)
os.fsync(fd)
old_bytes = os.pread(fd, 6, 0)
old_nlink = os.fstat(fd).st_nlink
os.close(fd)
new_bytes = path.read_bytes()
print(json.dumps({{'old_fd_bytes': old_bytes.decode(), 'old_nlink': old_nlink, 'replacement': new_bytes.decode()}}))
"""
            with ThreadPoolExecutor(max_workers=1) as pool:
                pending = pool.submit(shell, "B", "python3 -c " + shlex.quote(holder), hosts, timeout=30)
                wait(lambda: shell("B", f"test -f {root}/unlinked-ready && echo yes", hosts, check=False).strip() == "yes", "B unlinked fd open")
                shell("A", f"python3 -c 'from pathlib import Path; p=Path(\"{root}/mnt/job-42/unlinked-x\"); p.unlink(); p.write_bytes(b\"replacement\")'", hosts)
                shell("B", f"touch {root}/unlinked-go", hosts)
                held = json.loads(pending.result())
            if held != {"old_fd_bytes": "old-fd", "old_nlink": 0, "replacement": "replacement"}:
                raise RuntimeError(f"unlinked fd switched to replacement: {held=}")
            return held
        record("remote unlinked fd survives name reuse", unlinked_remote_fd_keeps_identity)

        def alternating_sizes():
            note = f"{root}/mnt/job-42/note"
            for writer, reader, value in (
                ("A", "B", "a"),
                ("B", "A", "remote-expanded-content"),
                ("A", "B", "short"),
                ("B", "A", "remote-content-with-an-even-longer-length"),
            ):
                shell(writer, f"python3 -c 'from pathlib import Path; Path(\"{note}\").write_bytes(bytes.fromhex(\"{value.encode().hex()}\"))'", hosts)
                observed = shell(reader, f"python3 -c 'from pathlib import Path; p=Path(\"{note}\"); print(p.stat().st_size, p.read_bytes().decode())'", hosts).strip()
                if observed != f"{len(value)} {value}":
                    raise RuntimeError(f"{writer} to {reader}: expected {value!r}, got {observed!r}")
            shell("A", f"python3 -c 'from pathlib import Path; Path(\"{note}\").write_bytes(b\"second\")'", hosts)
            if read_remote() != "second":
                raise RuntimeError("failed to restore note after alternating writes")
            return {"alternations": 4, "checked": "stat size and reopened bytes on other node"}
        record("alternating cross-node length and reopen", alternating_sizes)

        def management_query():
            response = shell("A", f"python3 -c 'import urllib.request; opener=urllib.request.build_opener(urllib.request.ProxyHandler({{}})); print(opener.open(\"http://{management}/v1/roots/job-42\", timeout=3).read().decode())'", hosts)
            info = json.loads(response)
            if info.get("owner") != "A" or info.get("status") != "active":
                raise RuntimeError(f"wrong management location: {info}")
            return info
        record("scheduler location API", management_query)

        def restart_center():
            shell("C", f"kill $(cat {root}/center.pid)", hosts)
            wait(lambda: shell("C", f"kill -0 $(cat {root}/center.pid) 2>/dev/null || echo stopped", hosts, check=False).strip() == "stopped", "old center exit")
            start_process("C", "center-restarted", f"env DMS_HOME_TOKEN={token} {binary} center {center} {management} {root}/center.state")
            wait(lambda: management_query().get("owner") == "A", "durable center recovery")
            if read_remote() != "second":
                raise RuntimeError("remote bytes changed after center restart")
            return {"owner": "A", "after_restart": "second"}
        record("center restart and persistent ownership", restart_center)

        def restart_home_process():
            shell("A", f"kill $(cat {root}/node-A.pid); fusermount3 -uz {root}/mnt 2>/dev/null || true", hosts)
            wait(lambda: shell("A", f"kill -0 $(cat {root}/node-A.pid) 2>/dev/null || echo stopped", hosts, check=False).strip() == "stopped", "old home process exit")
            start_process("A", "node-A-restarted", f"env DMS_HOME_TOKEN={token} {binary} node A {center} {args.host_a}:/ {p2p['A']} {root}/data {root}/peers {root}/mnt {args.backend}")
            wait(lambda: shell("A", f"mountpoint -q {root}/mnt && echo yes", hosts, check=False).strip() == "yes", "home FUSE remount")
            wait(lambda: read_remote() == "second", "remote reopen after home restart")
            shell("A", f"test -f {root}/mnt/job-42/note", hosts)
            return {"persistent_bytes": "second"}
        record("home process restart from ordinary files", restart_home_process)

        status = "PASS"
    except Exception as error:
        failure = str(error)
    finally:
        logs = {}
        for role, name in started_processes:
            logs[f"{role}/{name}"] = shell(role, f"tail -n 50 {root}/{name}.log 2>/dev/null", hosts, check=False)[-4000:]
        for role in ("A", "B"):
            shell(role, f"fusermount3 -uz {root}/mnt >/dev/null 2>&1 || true; "
                        f"sudo -n umount -lf {root}/peers/A {root}/peers/B >/dev/null 2>&1 || true", hosts, check=False)
        for role, name in reversed(started_processes):
            shell(role, f"test -f {root}/{name}.pid && kill $(cat {root}/{name}.pid) >/dev/null 2>&1 || true", hosts, check=False)
        if args.backend == "nfs":
            for role in ("A", "B"):
                shell(role, "sudo -n rm -f /etc/exports.d/dms-home-preview.exports; sudo -n exportfs -ra", hosts, check=False)
        result = {"schema": "dms.home-three-vm-accept.v1", "status": status, "backend": args.backend,
                  "duration_ms": round((time.time() - start) * 1000), "failure": failure, "steps": steps,
                  "addresses": addresses, "runtime": root, "logs": logs}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps({"status": status, "backend": args.backend, "failure": failure, "output": str(args.output)}))
    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

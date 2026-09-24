#!/usr/bin/env python3
"""Exercise durable root transition cuts from an installed package on Linux VMs."""

from __future__ import annotations

import argparse
import json
import platform
import shlex
import time
from pathlib import Path

from accept_three_vm import shell, wait


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--host-a", default="192.168.104.12")
    parser.add_argument("--host-c", default="192.168.104.11")
    parser.add_argument("--ssh-user", default="lzc")
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("fault cuts must run on Linux")
    if not args.run_id.replace("-", "").isalnum() or args.output.exists():
        parser.error("run ID invalid or output already exists")

    hosts = {"A": f"{args.ssh_user}@{args.host_a}", "C": f"{args.ssh_user}@{args.host_c}"}
    runtime = f"/home/lzc.guest/dms-home-fault-{args.run_id}"
    binary = f"{shlex.quote(str(args.package))}/bin/dms-home"
    center = f"{args.host_c}:32991"
    management = f"{args.host_c}:32992"
    p2p = f"{args.host_a}:32993"
    token = "three-vm-fault-test-token"
    steps: list[dict] = []
    started: list[tuple[str, str]] = []
    status = "FAIL"
    failure = None

    def record(name, action):
        before = time.monotonic()
        try:
            detail = action()
            steps.append({"name": name, "status": "PASS", "ms": round((time.monotonic() - before) * 1000), "detail": detail})
            return detail
        except Exception as error:
            steps.append({"name": name, "status": "FAIL", "ms": round((time.monotonic() - before) * 1000), "error": str(error)})
            raise

    def start(role, name, command):
        shell(role, f"nohup {command} >{runtime}/{name}.log 2>&1 </dev/null & echo $! >{runtime}/{name}.pid", hosts)
        started.append((role, name))

    def stop(role, name):
        shell(role, f"kill -9 $(cat {runtime}/{name}.pid) 2>/dev/null || true", hosts)
        if role == "A":
            shell("A", f"fusermount3 -uz {runtime}/mnt 2>/dev/null || true", hosts)
            wait(lambda: shell("A", f"mountpoint -q {runtime}/mnt && echo yes || echo no", hosts).strip() == "no", "FUSE unmount")

    def rpc(command):
        program = (
            "import socket\n"
            f"s=socket.create_connection(({args.host_c!r},32991),3)\n"
            f"s.sendall(({('AUTH ' + token + ' ' + command)!r}+'\\n').encode())\n"
            "s.shutdown(socket.SHUT_WR)\n"
            "print(s.makefile().read().strip())\n"
        )
        return shell("A", "python3 -c " + shlex.quote(program), hosts).strip()

    def expect(command, expected):
        got = rpc(command)
        if got != expected:
            raise RuntimeError(f"{command}: expected {expected!r}, got {got!r}")
        return got

    def start_center(name):
        start("C", name, f"env DMS_HOME_TOKEN={token} {binary} center {center} {management} {runtime}/center.state")
        wait(lambda: rpc("ROOTS") is not None, "center ready")

    def start_node(name):
        start("A", name, f"env DMS_HOME_TOKEN={token} {binary} node A {center} {args.host_a}:/ {p2p} {runtime}/data {runtime}/peers {runtime}/mnt p2p")
        wait(lambda: shell("A", f"mountpoint -q {runtime}/mnt && echo yes", hosts, check=False).strip() == "yes", "Home FUSE mount")

    try:
        def launch():
            for role in ("A", "C"):
                shell(role, f"test -x {binary} && cd {shlex.quote(str(args.package))} && sha256sum -c SHA256SUMS >/dev/null && mkdir -p {runtime}", hosts)
            shell("A", f"mkdir -p {runtime}/data {runtime}/peers {runtime}/mnt", hosts)
            start_center("center-1")
            start_node("node-1")
            return {"package": str(args.package), "runtime": runtime}
        record("installed package ready", launch)

        def stage():
            expect("RESERVE pending-absent A", "OK")
            expect("RESERVE pending-present A", "OK")
            shell("A", f"mkdir -m 700 {runtime}/data/pending-present && sync {runtime}/data", hosts)
            for root in ("deleting-present", "deleting-absent", "tombstone-present", "active-intact"):
                shell("A", f"mkdir {runtime}/mnt/{root}", hosts)
            for root in ("deleting-present", "deleting-absent", "tombstone-present"):
                expect(f"DELETE_PREPARE {root} A", "OK")
            shell("A", f"rmdir {runtime}/data/deleting-absent && sync {runtime}/data", hosts)
            expect("DELETE_COMMIT tombstone-present A", "OK")
            return {"pending": 2, "deleting": 2, "tombstone": 1, "active": 1}
        record("stage six persisted transition cuts", stage)

        def restart_and_reconcile():
            stop("A", "node-1")
            stop("C", "center-1")
            start_center("center-2")
            expect("GET pending-absent", "PENDING A")
            expect("GET pending-present", "PENDING A")
            expect("GET deleting-present", "DELETING A")
            start_node("node-2")
            for command, expected in (
                ("GET pending-absent", "MISSING"),
                ("GET pending-present", "A"),
                ("GET deleting-present", "TOMBSTONE A"),
                ("GET deleting-absent", "TOMBSTONE A"),
                ("GET tombstone-present", "TOMBSTONE A"),
                ("GET active-intact", "A"),
            ):
                expect(command, expected)
            mode = shell("A", f"stat -c %a {runtime}/data/pending-present", hosts).strip()
            if mode != "700":
                raise RuntimeError(f"recovery changed original mkdir mode: {mode}")
            for root in ("pending-absent", "deleting-present", "deleting-absent", "tombstone-present"):
                if shell("A", f"test -e {runtime}/data/{root} && echo yes || echo no", hosts).strip() != "no":
                    raise RuntimeError(f"{root} unexpectedly materialized")
            shell("A", f"test -d {runtime}/mnt/pending-present && test -d {runtime}/mnt/active-intact", hosts)
            return {"center_restart": "durable", "home_restart": "reconciled", "mode": mode}
        record("center and Home SIGKILL recover without split ownership", restart_and_reconcile)

        def retry_original_mkdir_mode():
            shell("A", f"mkdir -m 710 {runtime}/mnt/pending-absent", hosts)
            expect("GET pending-absent", "A")
            mode = shell("A", f"stat -c %a {runtime}/data/pending-absent", hosts).strip()
            if mode != "710":
                raise RuntimeError(f"retried mkdir lost mode 710: {mode}")
            return {"mode": mode}
        record("unmaterialized reservation retries with caller mode", retry_original_mkdir_mode)

        def active_missing_fails_closed():
            shell("A", f"mkdir {runtime}/mnt/active-missing", hosts)
            stop("A", "node-2")
            shell("A", f"rmdir {runtime}/data/active-missing && sync {runtime}/data", hosts)
            start("A", "node-negative", f"env DMS_HOME_TOKEN={token} {binary} node A {center} {args.host_a}:/ {p2p} {runtime}/data {runtime}/peers {runtime}/mnt p2p")
            wait(lambda: "Input/output error" in shell("A", f"cat {runtime}/node-negative.log", hosts, check=False), "fail-closed startup")
            if shell("A", f"mountpoint -q {runtime}/mnt && echo yes || echo no", hosts).strip() != "no":
                raise RuntimeError("Home mounted despite missing active directory")
            expect("GET active-missing", "A")
            shell("A", f"test ! -e {runtime}/data/active-missing && mkdir {runtime}/data/active-missing && sync {runtime}/data", hosts)
            start_node("node-repaired")
            return {"missing_active": "startup refused", "after_disk_repair": "recovered"}
        record("missing active directory fails closed", active_missing_fails_closed)

        status = "PASS"
    except Exception as error:
        failure = str(error)
    finally:
        logs = {f"{role}/{name}": shell(role, f"tail -n 30 {runtime}/{name}.log 2>/dev/null", hosts, check=False)[-3000:] for role, name in started}
        shell("A", f"fusermount3 -uz {runtime}/mnt >/dev/null 2>&1 || true", hosts, check=False)
        for role, name in reversed(started):
            shell(role, f"test -f {runtime}/{name}.pid && kill $(cat {runtime}/{name}.pid) >/dev/null 2>&1 || true", hosts, check=False)
        result = {"schema": "dms.home-fault-cuts.v1", "status": status, "failure": failure, "steps": steps, "runtime": runtime, "logs": logs}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps({"status": status, "failure": failure, "output": str(args.output)}))
    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

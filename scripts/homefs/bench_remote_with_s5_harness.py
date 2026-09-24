#!/usr/bin/env python3
"""Compare installed HomeFs remote W2 with stock MooseFS in the S5 Linux lab."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import platform
from pathlib import Path
import shlex
import sys


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--harness", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True, help="installed on A/B/C")
    parser.add_argument("--session-id", required=True)
    parser.add_argument("--backend", choices=("nfs", "p2p"), required=True)
    parser.add_argument("--rounds", type=int, default=6)
    parser.add_argument("--count", type=int, default=200)
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("Linux required")
    if args.rounds < 6 or args.count < 1:
        parser.error("at least six rounds and one file required")
    profile = json.loads(args.profile.read_text())
    output = Path(profile["run_root"]) / args.session_id
    sys.path.insert(0, str(args.harness.parent))
    spec = importlib.util.spec_from_file_location("s5_harness", args.harness)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    h = module.Harness(args.profile, output, args.session_id, development=False)
    frozen = h.prepare()
    binary = str(args.package / "bin/dms-home")
    setup_nfs = str(args.package / "scripts/setup-nfs.sh")
    center = f"{h.address('C')}:31991"
    management = f"{h.address('C')}:31992"
    roots = {role: Path(h.backend_root("home", role)) for role in ("A", "B")}
    rows = []
    status = "FAIL"
    error = None
    try:
        h.start_moosefs()
        h.shell("C", f"mkdir -p {shlex.quote(h.backend_root('home', 'C'))}")
        for role in ("A", "B"):
            root = roots[role]
            h.shell(role, f"mkdir -p {shlex.quote(str(root / 'data'))} {shlex.quote(str(root / 'peers'))} {shlex.quote(str(root / 'mnt'))}")
            if args.backend == "nfs":
                h.shell(role, f"sudo -n {shlex.quote(setup_nfs)} {shlex.quote(str(root / 'data'))} 192.168.104.0/24 >/dev/null", timeout=60)
        h.spawn("C", "home-center", ["env", "DMS_HOME_TOKEN=benchmark-private-token", binary, "center", center, management, h.backend_root("home", "C") + "/center.state"])
        h.shell("A", f"for i in $(seq 1 100); do {shlex.quote(binary)} roots {center} >/dev/null 2>&1 && exit 0; sleep 0.1; done; exit 1", timeout=20)
        for role in ("A", "B"):
            root = roots[role]
            h.spawn(role, f"home-node-{role.lower()}", ["env", "DMS_HOME_TOKEN=benchmark-private-token",
                    binary, "node", role, center, f"{h.address(role)}:/", f"{h.address(role)}:31993",
                    str(root / "data"), str(root / "peers"), str(root / "mnt"), args.backend])
            h.wait_mount(role, str(root / "mnt"))
        if args.backend == "nfs":
            for role, peer in (("A", "B"), ("B", "A")):
                h.wait_mount(role, str(roots[role] / "peers" / peer))

        script = h.service_path("s5_distributed_workload.py")

        def stage(role: str, action: str, backend: str, seed: int, **options):
            root = roots[role] / "mnt" if backend == "home" else Path(h.mountpoint("moosefs", role))
            command = ["python3", script, action, "--root", str(root), "--seed", str(seed), "--count", str(args.count)]
            for key, value in options.items():
                command.extend(["--" + key.replace("_", "-"), str(value)])
            return json.loads(h.run(role, command, timeout=180).stdout)

        for round_number in range(1, args.rounds + 1):
            order = ("home", "moosefs") if round_number % 2 else ("moosefs", "home")
            cases = {}
            for backend in order:
                seed = 8300 + round_number
                prepared = stage("A", "prepare", backend, seed)
                first = stage("B", "read", backend, seed, version="original", operation="first_read")
                repeat = stage("B", "read", backend, seed, version="original", operation="repeat_read")
                write = stage("B", "overwrite", backend, seed, operation="remote_overwrite")
                owner = stage("A", "read", backend, seed, version="updated", operation="owner_read_after")
                cleanup = stage("A", "cleanup", backend, seed)
                stages = {"first_read": first, "repeat_read": repeat, "remote_overwrite": write, "owner_read_after": owner}
                if not all(item["correctness"] for item in [prepared, *stages.values(), cleanup]):
                    raise RuntimeError(f"W2 correctness failed: {backend}, round {round_number}")
                cases[backend] = {
                    "total_wall_us": sum(item["phase_ledger"]["wall_us"] for item in stages.values()),
                    "stages": stages,
                }
            rows.append({"round": round_number, "order": order, "cases": cases})
            print(json.dumps({"round": round_number, "total_wall_us": {name: cases[name]["total_wall_us"] for name in order}}), flush=True)
        samples = {backend: [row["cases"][backend]["total_wall_us"] for row in rows] for backend in ("home", "moosefs")}
        p50 = {backend: module.POSIX.nearest_rank(values, 0.50) for backend, values in samples.items()}
        status = "PASS"
    except Exception as failure:
        error = str(failure)
        samples = {}
        p50 = {}
    finally:
        for role in ("A", "B"):
            root = roots[role]
            h.shell(role, f"fusermount3 -uz {shlex.quote(str(root / 'mnt'))} 2>/dev/null || true", check=False)
            if args.backend == "nfs":
                peer = "B" if role == "A" else "A"
                h.shell(role, f"sudo -n umount -lf {shlex.quote(str(root / 'peers' / peer))} 2>/dev/null || true; sudo -n rm -f /etc/exports.d/dms-home-preview.exports; sudo -n exportfs -ra", check=False)
        h.close()
        result = {
            "schema": "dms.home-preview-remote-w2.v1", "status": status, "error": error,
            "backend": args.backend, "scope": "200 x 4KiB; no drop_caches or visibility sleep; MooseFS durability ACK not proven equal",
            "session_id": args.session_id, "count": args.count, "profile_sha256": frozen["profile_sha256"],
            "harness_sha256": digest(args.harness), "script_sha256": digest(Path(__file__)),
            "binary_sha256": digest(args.package / "bin/dms-home"), "rows": rows,
            "samples_us": samples, "p50_us": p50,
            "ratio_home_to_moosefs": p50.get("home", 0) / p50["moosefs"] if p50 else None,
            "environment": frozen["environment"], "commands": h.commands,
        }
        (output / "remote-w2.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps({key: result[key] for key in ("status", "error", "backend", "p50_us", "ratio_home_to_moosefs")}))
    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

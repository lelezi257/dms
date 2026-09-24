#!/usr/bin/env python3
"""Run home-local W1 with stock MooseFS and thin FUSE in the existing S5 three-VM lab."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shlex
import sys


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--harness", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True, help="installed package on A and C")
    parser.add_argument("--thin-binary", type=Path, required=True)
    parser.add_argument("--bench", type=Path, required=True)
    parser.add_argument("--session-id", required=True)
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("Linux required")
    profile = json.loads(args.profile.read_text())
    output = Path(profile["run_root"]) / args.session_id
    sys.path.insert(0, str(args.harness.parent))
    spec = importlib.util.spec_from_file_location("s5_harness", args.harness)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    h = module.Harness(args.profile, output, args.session_id, development=False)
    frozen = h.prepare()
    thin_root = Path(h.backend_root("thin_fuse", "A"))
    thin_mount = thin_root / "mnt"
    backing = thin_root / "backing"
    native = output / "native"
    home_root = Path(h.backend_root("home", "A"))
    home_mount = home_root / "mnt"
    binary = str(args.package / "bin/dms-home")
    center = f"{h.address('C')}:31991"
    management = f"{h.address('C')}:31992"
    outcome = "ERROR"
    error = None
    try:
        h.start_moosefs()
        h.shell("A", f"mkdir -p {shlex.quote(str(backing))} {shlex.quote(str(thin_mount))} {shlex.quote(str(native))} {shlex.quote(str(home_root / 'data'))} {shlex.quote(str(home_root / 'peers'))} {shlex.quote(str(home_mount))}")
        h.shell("C", f"mkdir -p {shlex.quote(h.backend_root('home', 'C'))}")
        h.spawn("A", "thin-fuse", [str(args.thin_binary), "-f", "-s", "-o",
                 f"source={backing},cache=always,timeout=1,no_writeback,default_permissions,noatime", str(thin_mount)])
        h.wait_mount("A", str(thin_mount))
        h.spawn("C", "home-center", ["env", "DMS_HOME_TOKEN=benchmark-private-token", binary, "center", center, management, h.backend_root("home", "C") + "/center.state"])
        h.shell("A", f"for i in $(seq 1 100); do {shlex.quote(binary)} roots {center} >/dev/null 2>&1 && exit 0; sleep 0.1; done; exit 1", timeout=20)
        h.spawn("A", "home-node", ["env", "DMS_HOME_TOKEN=benchmark-private-token", binary,
                "node", "A", center, f"{h.address('A')}:/", f"{h.address('A')}:31993",
                str(home_root / "data"), str(home_root / "peers"), str(home_mount), "p2p"])
        h.wait_mount("A", str(home_mount))
        command = ["python3", str(args.bench), "--dms-home", str(home_mount), "--moosefs",
                   h.mountpoint("moosefs", "A"), "--thin-fuse", str(thin_mount), "--native",
                   str(native), "--dms-binary", binary, "--output", str(output / "bench")]
        result = h.run("A", command, timeout=1200, check=False)
        (output / "bench.log").write_text(result.stdout or "")
        outcome = "PASS" if result.returncode == 0 else "FAIL"
        if result.returncode:
            error = (result.stdout or "")[-3000:]
    except Exception as failure:
        error = str(failure)
    finally:
        h.shell("A", f"fusermount3 -uz {shlex.quote(str(home_mount))} 2>/dev/null || true; fusermount3 -uz {shlex.quote(str(thin_mount))} 2>/dev/null || true", check=False)
        h.close()
        receipt = {
            "schema": "dms.home-preview-w1-lab-wrapper.v1", "status": outcome, "error": error,
            "session_id": args.session_id, "profile_sha256": frozen["profile_sha256"],
            "harness_sha256": digest(args.harness), "thin_sha256": digest(args.thin_binary),
            "package_binary_sha256": digest(args.package / "bin/dms-home"),
            "bench_sha256": digest(args.bench), "commands": h.commands,
            "environment": frozen["environment"],
        }
        (output / "home-wrapper.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps({"status": outcome, "error": error, "output": str(output)}))
    return 0 if outcome == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

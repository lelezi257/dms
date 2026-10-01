#!/usr/bin/env python3
"""Qualify crash-mount recovery with old/fixed controllers on Linux only."""
import hashlib
import json
import pathlib
import platform
import shutil
import subprocess
import time

assert platform.system() == "Linux"
base = pathlib.Path("/home/lzc.guest/afs-build")
inputs = base / "probes/repair-v60/controller-inputs"
out = base / "probes/repair-v60/controller-qualified"
out.mkdir(parents=True, exist_ok=False)
original = base / "work/root-merged-v60-r2/scripts/deploy"
manifest = {}
checks = []
for lane in ("original", "fixed"):
    dest = out / lane
    shutil.copytree(original, dest)
    shutil.copy2(inputs / "selftest.sh", dest / "selftest.sh")
    if lane == "fixed":
        shutil.copy2(inputs / "afs-processctl", dest / "afs-processctl")
    manifest[lane] = {name: hashlib.sha256((dest / name).read_bytes()).hexdigest()
                      for name in ("afs-processctl", "selftest.sh")}
    (out / "inputs.json").write_text(json.dumps(manifest, indent=2) + "\n")
    for name in ("afs-processctl", "selftest.sh"):
        subprocess.run(["bash", "-n", str(dest / name)], check=True)
    started = time.monotonic()
    with (out / f"{lane}-selftest.log").open("w") as log:
        result = subprocess.run(["timeout", "--kill-after=3s", "240s", "bash", str(dest / "selftest.sh")],
                                stdout=log, stderr=subprocess.STDOUT)
    raw = (out / f"{lane}-selftest.log").read_text()
    check = {"lane": lane, "returncode": result.returncode,
             "elapsed_seconds": time.monotonic() - started,
             "passing_groups": sum(line.startswith("ok - ") for line in raw.splitlines()),
             "argv": ["timeout", "--kill-after=3s", "240s", "bash", str(dest / "selftest.sh")]}
    checks.append(check)
    (out / "checks.json").write_text(json.dumps(checks, indent=2) + "\n")
    print(json.dumps(check), flush=True)
    if lane == "original":
        assert result.returncode != 0 and result.returncode not in (124, 137)
        assert ("stale disconnected AFS mount is recovered" in raw
                or "stale recovery uses bounded FUSE detach" in raw)
    else:
        assert result.returncode == 0, raw[-5000:]
        assert "recovers only positively disconnected exact AFS FUSE mounts" in raw
report = {"status": "PASS", "platform": platform.platform(), "checks": checks,
          "inputs": manifest, "scope": "Linux deployment regressions; not full DEP acceptance"}
(out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report), flush=True)

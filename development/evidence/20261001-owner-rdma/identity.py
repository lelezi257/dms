#!/usr/bin/env python3
"""Bind a Linux snapshot to the retained compilation-input inventory."""
import hashlib
import json
import pathlib
import platform
import subprocess
import sys


if platform.system() != "Linux":
    raise SystemExit("identity collection runs in Linux")
root = pathlib.Path(sys.argv[1]).resolve(strict=True)
inventory = json.loads(pathlib.Path(sys.argv[2]).read_text())["files"]
files = {name: hashlib.sha256((root / name).read_bytes()).hexdigest()
         for name in sorted(inventory)}
print(json.dumps({
    "snapshot": str(root), "platform": platform.platform(),
    "file_count": len(files), "files": files,
    "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
}, indent=2))

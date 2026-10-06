#!/usr/bin/env python3
"""Bind compiler inputs and acceptance/deployment tools; documentation is independent."""
import hashlib
import json
import os
import platform
import sys
from pathlib import Path

assert platform.system() == 'Linux' and platform.machine() == 'aarch64'
root = Path(__file__).resolve().parents[3]
inputs = {}
for parent, dirs, names in os.walk(root, followlinks=False):
    dirs[:] = sorted(d for d in dirs if d not in {'.git', '.local', '.omx', '__pycache__', 'target', 'evidence', 'results', 'artifacts', 'log'})
    for name in sorted(names):
        path = Path(parent, name)
        rel = path.relative_to(root)
        compiler = path.suffix in {'.rs', '.c', '.h', '.proto', '.toml', '.lock'} and rel.parts[0] not in {'docs', 'development', 'scripts'}
        tool = rel.parts[:2] in {('development', 'acceptance'), ('scripts', 'deploy'), ('development', 'checkpoints')} and path.suffix != '.md'
        if (compiler or tool) and path.suffix not in {'.pyc', '.log', '.exit'}:
            assert path.is_file() and not path.is_symlink(), rel
            inputs[str(rel)] = hashlib.sha256(path.read_bytes()).hexdigest()
receipt = Path(sys.argv[2])
if sys.argv[1] == 'capture':
    assert not receipt.exists()
    value = {'schema': 'afs-combined-checkpoint-inputs-v1', 'platform': 'Linux/aarch64', 'files': inputs,
             'map_sha256': hashlib.sha256(json.dumps(inputs, sort_keys=True, separators=(',', ':')).encode()).hexdigest()}
    receipt.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
else:
    expected = json.loads(receipt.read_text())
    assert inputs == expected['files'], 'compiler/tool inputs changed'
print(f"PASS {len(inputs)} compiler/tool inputs")

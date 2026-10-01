#!/usr/bin/env python3
"""Run unchanged fault assertions on the fresh, qualified v60 candidate."""
import importlib.util
import json
import pathlib


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, pathlib.Path(__file__).with_name(filename))
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runtime = load("repair_runtime_candidate", "repair-runtime-v60.py").runtime
fault = load("repair_fault_base", "repair-fault-v56.py")
fault.BASELINE_OUT = runtime.OUT
fault.OUT = runtime.ROOT / "evidence/afs-delivery/repair-fault-v60"
fault.v55 = runtime
runtime.OUT = fault.OUT
fault.ORIGINAL_FILE = runtime.TEST_FILE
fault.SOURCE_LOSS_FILE = "repair-v60-source-loss.bin"
hashes = json.loads((fault.BASELINE_OUT / "staged-runtime-binary-sha256.json").read_text())
assert hashes["a"]["node"] == hashes["b"]["node"]
fault.NODE_SHA = hashes["a"]["node"]
fault.META_SHA = hashes["a"]["meta"]

if __name__ == "__main__":
    fault.main()

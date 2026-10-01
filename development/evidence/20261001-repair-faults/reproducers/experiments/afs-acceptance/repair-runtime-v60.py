#!/usr/bin/env python3
"""Allocate a fresh v60 Linux runtime using the retained v55 coordinator."""
import importlib.util
import os
import pathlib

spec = importlib.util.spec_from_file_location(
    "repair_runtime_base", pathlib.Path(__file__).with_name("repair-runtime-v55.py")
)
assert spec is not None and spec.loader is not None
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)
runtime.OUT = runtime.ROOT / "evidence/afs-delivery/repair-runtime-v60"
runtime.RUN = {
    "a": "/mnt/lima-afsadata/afs-delivery/repair-v60-a",
    "b": "/mnt/lima-afsbdata/afs-delivery/repair-v60-b",
}
runtime.PORTS = {
    "meta_grpc": 18480, "meta_rest": 18481,
    "a_grpc": 18482, "a_rest": 18483,
    "b_grpc": 18484, "b_rest": 18485,
}
runtime.TEST_FILE = "repair-v60-deterministic-1m.bin"
os.environ["AFS_REPAIR_CANDIDATE"] = "v60"

if __name__ == "__main__":
    args = runtime.parser().parse_args()
    if args.candidate != "v60":
        raise SystemExit("this wrapper is bound to candidate v60")
    args.fn(args)

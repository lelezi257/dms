"""Read-only Linux audit of this slice's exact retained observations."""
import hashlib
import json
import pathlib
import platform
import re
import sys

assert platform.system() == "Linux"
root = pathlib.Path(sys.argv[1]).resolve()
product = pathlib.Path(sys.argv[2]).resolve()
r1 = root / "rdma-admission-v74"
r2 = root / "rdma-admission-v74-r2"
original = root / "rdma-admission-v74-original"
checks = []


def check(name, condition):
    assert condition, name
    checks.append(name)


def read(path):
    return path.read_text()


def counts(path):
    values = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", read(path))
    assert values, path
    return tuple(sum(int(v[i]) for v in values) for i in range(3))


before = json.loads(read(original / "before-graft-inputs.json"))["files"]
graft = json.loads(read(original / "grafted-inputs.json"))["files"]
v1 = json.loads(read(r1 / "build-inputs.json"))["files"]
final = json.loads(read(r2 / "build-inputs.json"))["files"]
check("143 frozen inputs", len(final) == 143 and before.keys() == final.keys())
check("original graft only control tests", [p for p in before if before[p] != graft[p]] == ["src/node/rpc/control.rs"])
check("r1/r2 only control tests", [p for p in v1 if v1[p] != final[p]] == ["src/node/rpc/control.rs"])
one = pathlib.Path(json.loads(read(r1 / "build-inputs.json"))["snapshot"]) / "src/node/rpc/control.rs"
two = pathlib.Path(json.loads(read(r2 / "build-inputs.json"))["snapshot"]) / "src/node/rpc/control.rs"
marker = "#[cfg(test)]\nmod tests {"
check("r1/r2 production identical", read(one).split(marker)[0] == read(two).split(marker)[0] and read(one).count(marker) == read(two).count(marker) == 1)
check("host matches final Linux inputs", all(hashlib.sha256((product / p).read_bytes()).hexdigest() == h for p, h in final.items()))
check("protected AGENTS", final.get("AGENTS.md") is None and hashlib.sha256((product / "AGENTS.md").read_bytes()).hexdigest() == "539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f")
check("handoff unchanged", hashlib.sha256((product / "docs/handoff.md").read_bytes()).hexdigest() == "8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216")
check("original failure101", read(original / "regression/original.exit").strip() == "101" and "session_id: 65" in read(original / "regression/original.log"))
check("r1 Clippy failure retained", read(r1 / "full/gate/clippy.exit").strip() == "101" and "unused" in read(r1 / "full/gate/clippy.log"))
check("local55", sum(counts(r1 / f"local/{p}.log")[0] for p in ("control", "original", "data", "peer", "contracts")) == 55)
check("r2 control9", counts(r2 / "local/control.log") == (9, 0, 1) and read(r2 / "local/control.exit").strip() == "0")
for name in ("fmt", "clippy", "lib", "error", "contracts", "local-api", "fuse-build", "fuse", "build"):
    check("final gate " + name, read(r2 / f"full/gate/{name}.exit").strip() == "0")
for name in ("no-features", "owner-features", "dfs-features", "owner-rdma", "dfs-rdma"):
    check("feature " + name, read(r2 / f"full/features/{name}.exit").strip() == "0")
# lib.log includes a subprocess child test; use the outer summary.
check("lib410/3", re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", read(r2 / "full/gate/lib.log"))[-1] == ("410", "0", "3"))
check("contracts65", counts(r2 / "full/gate/contracts.log")[0] == 65)
check("error4", counts(r2 / "full/gate/error.log")[0] == 4)
check("localAPI9", counts(r2 / "full/gate/local-api.log")[0] == 9)
check("rootFUSE5", counts(r2 / "full/gate/fuse.log")[0] == 5)
check("final original regression", counts(r2 / "original/original.log") == (1, 0, 0) and read(r2 / "original/original.exit").strip() == "0")
for name, expected in (("dfs", 1), ("owner", 1), ("lifecycle", 5)):
    check("native " + name, counts(r2 / f"native/{name}.log") == (expected, 0, 0) and read(r2 / f"native/{name}.exit").strip() == "0")
dfs = read(r2 / "native/dfs.log")
check("DFS actual bytes/auth/retry", "replica_bytes=8388608 read_bytes=75000 grpc_payload_bytes=0 exact_retry=PASS forged_grant=DENIED" in dfs and dfs.count("AFS_RDMA_COMPLETE op=READ bytes=4194304") == 2 and "AFS_RDMA_COMPLETE op=WRITE bytes=75000" in dfs)
owner = read(r2 / "native/owner.log")
for op in ("READ", "WRITE"):
    check("Owner actual " + op, sum(map(int, re.findall(r"AFS_RDMA_COMPLETE op=" + op + r" bytes=(\d+)", owner))) == 4 * 1024 * 1024 + 17)
for line in read(r2 / "stripped-binaries.txt").splitlines():
    digest, binary = line.split("  ", 1)
    check("binary " + pathlib.Path(binary).name, hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest() == digest)
print(json.dumps({"status": "PASS", "checks": checks, "check_count": len(checks), "formal_acceptance": "NOT_RUN", "environment": "PREPARING", "limits": ["No actual posted-DMA cancellation proof", "No exceptional provider reclaim proof", "No new cross-VM deployment", "DFS Auto selection remains separate"]}, indent=2))

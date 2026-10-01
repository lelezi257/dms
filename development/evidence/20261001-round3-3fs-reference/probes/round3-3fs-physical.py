#!/usr/bin/env python3
"""Bind read-only 3FS query metadata to actual old-engine replica bytes."""
import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import struct

spec = importlib.util.spec_from_file_location("round3", pathlib.Path(__file__).with_name("round3-3fs.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


def parse_size(value):
    match = re.fullmatch(r"(\d+)(B|KB|MB|GB|K|M|G)?", str(value))
    if not match:
        raise ValueError(f"unrecognized exact Size: {value}")
    return int(match[1]) * {None: 1, "B": 1, "KB": 1024, "MB": 1024**2, "GB": 1024**3, "K": 1000, "M": 1000**2, "G": 1000**3}[match[2]]


def query():
    run = driver.root("ctl")
    stat = driver.admin(run, "physical-file-stat", ["stat", "test/qualifier32m.bin", "--display-all-chunks"])
    text = pathlib.Path(stat["stdout"]).read_text()
    inode = int(re.search(r"^Inode\s+(0x[0-9a-fA-F]+)", text, re.M).group(1), 16)
    assert re.search(r"^Length\s+33554432@0", text, re.M)
    assert re.search(r"^Layout-ChunkSize\s+524288", text, re.M)
    plan = {"inode": inode, "chunk_size": 524288, "queries": [], "roles": {r: [] for r in ("a", "b", "c")}}
    targets = {int(f"{node}01001"): role for role, node in driver.STORAGE_NODE.items()}
    for index in range(64):
        chunk_id = struct.pack(">BBQHI", 0, 0, inode, 0, index).hex()
        record = driver.admin(run, f"physical-query-{index:02}", ["query-chunk", "--chain-id", "1", "--chunk", chunk_id])
        rows = json.loads(pathlib.Path(record["stdout"]).read_text())
        assert len(rows) == 3
        seen = set()
        for row in rows:
            value = row["value"]
            target, meta = value["target"], value["meta"]["value"]
            role = targets[target["targetId"]]
            assert role not in seen
            seen.add(role)
            assert target["localState"] == "UPTODATE" and target["publicState"] == "SERVING"
            assert target["useChunkEngine"] is False
            assert meta["chunkState"] == "COMMIT" and meta["commitVer"] == meta["updateVer"]
            assert meta["size"] == 524288 and meta["innerFileId"]["chunkSize"] == 524288
            path = pathlib.Path(target["path"]) / "512KB" / f'{meta["innerFileId"]["chunkIdx"]:02X}'
            assert path.is_relative_to(driver.root(role) / "data/storage/data1")
            plan["roles"][role].append({"index": index, "chunk_id": chunk_id, "path": str(path), "offset": parse_size(meta["innerOffset"]), "size": meta["size"], "commit_version": meta["commitVer"], "update_version": meta["updateVer"]})
        assert seen == {"a", "b", "c"}
        plan["queries"].append(record)
    driver.save_json(run, "physical-query-plan", plan)
    return plan


def verify(role, plan_path):
    plan = json.loads(pathlib.Path(plan_path).read_text())
    data = driver.payload()
    receipts = []
    entries = plan["roles"][role]
    assert len(entries) == 64 and {e["index"] for e in entries} == set(range(64))
    assert len({(e["path"], e["offset"]) for e in entries}) == 64
    for entry in entries:
        path = pathlib.Path(entry["path"])
        assert path.is_relative_to(driver.root(role) / "data/storage/data1")
        fd = os.open(path, os.O_RDONLY)
        try:
            observed = os.pread(fd, entry["size"], entry["offset"])
            stat = os.fstat(fd)
        finally:
            os.close(fd)
        expected = data[entry["index"] * 524288:(entry["index"] + 1) * 524288]
        assert observed == expected, f'physical payload mismatch: {entry}'
        receipts.append(dict(entry, observed_sha256=hashlib.sha256(observed).hexdigest(), expected_sha256=hashlib.sha256(expected).hexdigest(), device=stat.st_dev, inode=stat.st_ino))
    result = {"status": "PASS", "role": role, "replica_chunks_exact": len(receipts), "bytes_exact": sum(e["size"] for e in receipts), "plan_sha256": driver.sha256_file(pathlib.Path(plan_path)), "receipts": receipts, "scope": "physical old-engine indexed bytes; not crash or hardware durability"}
    driver.save_json(driver.root(role), "physical-verify", result)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=["query", "verify"])
    parser.add_argument("--role", choices=["a", "b", "c"])
    parser.add_argument("--plan")
    args = parser.parse_args()
    driver.require_guest()
    result = query() if args.action == "query" else verify(args.role, args.plan)
    print(json.dumps(result, indent=2, sort_keys=True))

#!/usr/bin/env python3
"""Local container P2/P3 paired diagnostics. No MooseFS durability assertion."""
import json
import os
from pathlib import Path


def cases():
    result = []
    large = 8 * 1024 ** 3
    hot = 512 * 1024 ** 2
    for concurrency in (1, 8):
        for barrier in ("close", "fdatasync", "fsync"):
            result.append(dict(operation="seq-write", file_bytes=large, block_bytes=1024 ** 2,
                concurrency=concurrency, barrier=barrier, io_bytes=large, cache="guest-cold", dataset="seq"))
        for cache in ("guest-cold", "repeat"):
            result.append(dict(operation="seq-read", file_bytes=large, block_bytes=1024 ** 2,
                concurrency=concurrency, barrier="close", io_bytes=large, cache=cache, dataset="seq"))
        for block_bytes in (4096, 65536):
            for size, cache, dataset in ((large, "guest-cold", "seq"), (hot, "hot", "hot")):
                result.append(dict(operation="random-read", file_bytes=size, block_bytes=block_bytes,
                    concurrency=concurrency, barrier="close", io_bytes=hot, cache=cache, dataset=dataset))
            for barrier in ("close", "fdatasync", "fsync"):
                result.append(dict(operation="random-write", file_bytes=hot, block_bytes=block_bytes,
                    concurrency=concurrency, barrier=barrier, io_bytes=hot, cache="hot", dataset="write"))
    assert len(result) == 30
    return result


def run_suite(base, result, lanes, runtime_command, check, guest, slice_name="full"):
    assert set(lanes) == {"ext4", "native"}
    expected = result["source"]
    source = lanes["ext4"]
    shared = slice_name == "shared-file-attribution"
    assert slice_name in ("full", "shared-file-attribution")
    assert os.statvfs(source).f_bavail * os.statvfs(source).f_frsize > (11 if shared else 20) * 1024 ** 3
    result["workload"] = "bulk"
    result["slice"] = slice_name
    result["case_matrix"] = [case for index, case in enumerate(cases()) if not shared or index in (2, 5, 11, 12)]
    result["cache_limit"] = "guest drop_caches/mincore only; host Hyper-V cache is uncontrolled; 8GiB repeated read is not called fully hot"
    result["scope"] = "P2/P3 local OCI ext4/native paired buffered IO; no remote/MooseFS strong-durability qualification"
    datasets = {}
    patterns = {}
    def command(lane, dataset, operation, file_bytes, block_bytes, concurrency, barrier, io_bytes, byte, creation="existing", cache="unchecked"):
        check(lane)
        identifier = result["containers"][lane]["id"]
        path = datasets[lane, dataset]
        completed = runtime_command(["exec", identifier, "/io", path, operation, str(file_bytes),
            str(block_bytes), str(concurrency), barrier, str(io_bytes), str(byte), creation, cache])
        return json.loads(completed.stdout)
    for lane in lanes:
        suffix = "data" if shared else ("ext4" if lane == "ext4" else "bind")
        for dataset, size in (("seq", 8 * 1024 ** 3), ("hot", 512 * 1024 ** 2), ("write", 512 * 1024 ** 2)):
            datasets[lane, dataset] = f"/ownerfs/agent1/io-{dataset}-{suffix}"
            patterns[lane, dataset] = 90
            if shared and lane == "native":
                continue
            prepared = command(lane, dataset, "seq-write", size, 1024 ** 2, 1, "fsync", size, 90, "create")
            result.setdefault("dataset_setup", []).append(dict(lane=lane, dataset=dataset, path=datasets[lane, dataset], result=prepared))
            guest.save(base / "container-performance.json", result)
    for case_number, case in enumerate(result["case_matrix"]):
        for round_number in range(6):
            order = tuple(lanes) if round_number % 2 == 0 else tuple(reversed(lanes))
            for lane in order:
                # Complete preceding dirty writeback outside this task timer.
                os.sync()
                byte = patterns[lane, case["dataset"]]
                if case["operation"] == "seq-write":
                    byte = 32 + case_number * 6 + round_number + (96 if shared and lane == "native" else 0)
                if case["operation"] == "random-write":
                    command(lane, "write", "seq-write", case["file_bytes"], 1024 ** 2,
                        1, "fsync", case["file_bytes"], 0)
                    byte = 161 + round_number + (64 if shared and lane == "native" else 0)
                if case["cache"] == "guest-cold":
                    Path("/proc/sys/vm/drop_caches").write_text("3\n")
                else:
                    command(lane, case["dataset"], "seq-read", case["file_bytes"], 1024 ** 2,
                        1, "close", case["file_bytes"], 0 if case["operation"] == "random-write" else patterns[lane, case["dataset"]])
                current = guest.node(base)
                before = dict(node_stat=Path(f"/proc/{current['pid']}/stat").read_text(),
                    node_io=Path(f"/proc/{current['pid']}/io").read_text(),
                    diskstats=Path("/proc/diskstats").read_text(), meminfo=Path("/proc/meminfo").read_text())
                measurement = command(lane, case["dataset"], case["operation"], case["file_bytes"],
                    case["block_bytes"], case["concurrency"], case["barrier"], case["io_bytes"], byte, cache=case["cache"])
                if case["operation"].endswith("write"):
                    patterns[lane, case["dataset"]] = byte
                    if shared:
                        for other in lanes: patterns[other, case["dataset"]] = byte
                if case["cache"] == "guest-cold":
                    assert measurement["resident_before_bytes"] == 0
                if case["cache"] == "hot":
                    assert measurement["resident_before_bytes"] == case["file_bytes"]
                result["samples"].append(dict(case=case_number, lane=lane, round=round_number,
                    warmup=round_number == 0, path=datasets[lane, case["dataset"]], result=measurement,
                    resources_before=before, resources_after=dict(
                        node_stat=Path(f"/proc/{current['pid']}/stat").read_text(),
                        node_io=Path(f"/proc/{current['pid']}/io").read_text(),
                        diskstats=Path("/proc/diskstats").read_text(), meminfo=Path("/proc/meminfo").read_text())))
                guest.save(base / "container-performance.json", result)
    # Remove only these six freshly O_EXCL-created workload artifacts, after
    # all task FDs/mappings have closed. Keep object/size/endpoint evidence.
    stat = os.stat(source)
    assert dict(device=stat.st_dev, inode=stat.st_ino) == expected
    for (lane, dataset), path in datasets.items():
        if shared and lane == "native": continue
        physical = source / Path(path).name
        assert physical.parent == source and physical.is_file() and not physical.is_symlink()
        observed = os.stat(physical)
        assert observed.st_nlink == 1
        physical.unlink()
        result.setdefault("workload_cleanup", []).append(dict(lane=lane, dataset=dataset,
            path=path, inode=observed.st_ino, file_bytes=observed.st_size, unlinked=True))

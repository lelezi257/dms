#!/usr/bin/env python3
"""Linux-only, PID-scoped resource observation for a bounded RXE test process."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import subprocess
import time


def resources():
    result = {}
    for kind in ("qp", "cq", "mr", "pd", "ctx"):
        run = subprocess.run(
            ["rdma", "-j", "resource", "show", kind],
            capture_output=True, text=True, timeout=5, check=True,
        )
        result[kind] = json.loads(run.stdout)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if platform.system() != "Linux" or os.geteuid() != 0 or not command:
        parser.error("root Linux and a test command are required")
    output = pathlib.Path(args.output)
    output.mkdir(parents=True, exist_ok=False)
    binary = pathlib.Path(command[0]).resolve(strict=True)
    before = resources()
    tids = set()
    samples = []
    started = time.time_ns()
    with (output / "test.log").open("w") as log:
        child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        tids.add(child.pid)
        try:
            while child.poll() is None:
                if time.time_ns() - started > 180_000_000_000:
                    raise TimeoutError("bounded test exceeded 180 seconds")
                task_dir = pathlib.Path(f"/proc/{child.pid}/task")
                if task_dir.is_dir():
                    tids.update(int(path.name) for path in task_dir.iterdir())
                samples.append({"time_ns": time.time_ns(), "resources": resources()})
                time.sleep(0.02)
            code = child.wait()
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
    after = resources()
    owned = lambda rows: [row for row in rows if row.get("pid") in tids]
    peak = {
        kind: max((len(owned(sample["resources"][kind])) for sample in samples), default=0)
        for kind in before
    }
    retained = {kind: owned(rows) for kind, rows in after.items()}
    result = {
        "level": "LOCAL_REGRESSION", "command": command,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "observer_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
        "platform": platform.platform(), "device": os.environ.get("AFS_TEST_RDMA_DEVICE"),
        "pid": child.pid, "observed_task_ids": sorted(tids),
        "start_time_ns": started, "end_time_ns": time.time_ns(), "exit_code": code,
        "before": before, "samples": samples, "after": after,
        "owned_peak": peak, "owned_retained": retained,
        "status": "FAIL" if code or any(retained.values()) else
                  "PASS" if all(peak.values()) else "INCONCLUSIVE",
        "scope": "selected tests and PID-owned resources after process exit; "
                 "not posted-DMA cancellation, exceptional provider teardown, "
                 "cross-VM OwnerFs, full stage gate or formal acceptance",
    }
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in
                      ("status", "exit_code", "owned_peak", "owned_retained")}))
    return 1 if result["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())

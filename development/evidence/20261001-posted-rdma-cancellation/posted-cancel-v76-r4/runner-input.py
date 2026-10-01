#!/usr/bin/env python3
"""Linux RXE diagnostic: cancel a real posted WQE before CQ consumption.

Only the native worker stops. Other threads execute cancellation and close.
This proves accepted/unconsumed work, not that physical DMA is still pending.
No production instrumentation or retry behavior is changed.
"""

import argparse
import hashlib
import json
import os
import pathlib
import platform
import signal
import subprocess
import textwrap
import threading
import time


TEST = "posted_rdma_cancel_keeps_endpoint_until_worker_drains"


def owned_resources(directory, phase, kind):
    process = json.loads((directory / f"{phase}-process.json").read_text())
    owners = {process["pid"], *process["tids"]}
    raw = json.loads((directory / f"{phase}-{kind}.json").read_text())
    assert isinstance(raw, list) and all(isinstance(entry, dict) for entry in raw), "unsupported rdma JSON schema"
    return [entry for entry in raw if entry.get("pid") in owners]


def audit(directory):
    events = [json.loads(line) for line in (directory / "debugger.jsonl").read_text().splitlines()]
    posts = [event for event in events if event["event"] == "posted"]
    resumes = [event for event in events if event["event"] == "resumed"]
    exits = [event for event in events if event["event"] == "exit"]
    assert len(posts) == len(resumes) == len(exits) == 1
    assert posts[0]["operation"] == 1 and posts[0]["bytes"] == 4096
    assert posts[0]["data_wr_id"] == 1 and not posts[0]["poisoned"]
    assert resumes[0]["monotonic"] >= posts[0]["monotonic"]
    assert exits[0]["code"] == 0
    log = (directory / "gdb.log").read_text()
    assert "test result: ok. 1 passed; 0 failed; 0 ignored" in log
    assert "lookup=STALE client=POISONED endpoint=RETAINED storage=UNTOUCHED" in log
    assert "drain=COMPLETE content=EXACT endpoint=RELEASED replay=ABSENT" in log
    assert log.count("AFS_RDMA_COMPLETE op=READ bytes=4096") == 1
    assert "AFS_RDMA_COMPLETE op=WRITE" not in log
    identities = {}
    processes = [json.loads((directory / f"{phase}-process.json").read_text()) for phase in ["baseline", "connected", "closed-paused", "drained"]]
    assert {process["pid"] for process in processes} == {posts[0]["pid"]}
    assert posts[0]["thread"][1] in processes[2]["tids"]
    for kind, identity in [("qp", "lqpn"), ("mr", "mrn"), ("cq", "cqn"), ("pd", "pdn"), ("ctx", "ctxn")]:
        phases = {
            phase: owned_resources(directory, phase, kind)
            for phase in ["baseline", "connected", "closed-paused", "drained"]
        }
        assert phases["baseline"] == phases["drained"] == [], (kind, phases)
        connected = {entry[identity] for entry in phases["connected"]}
        paused = {entry[identity] for entry in phases["closed-paused"]}
        assert len(connected) == 2 and connected == paused, (kind, phases)
        if kind == "qp":
            assert posts[0]["qp"] in paused, "posted QP must be retained"
        devices = {entry["ifname"] for phase in ["connected", "closed-paused"] for entry in phases[phase]}
        assert len(devices) == 1, "resources must belong to one RXE device"
        identities[kind] = {"connected": sorted(connected), "closed_paused": sorted(paused), "drained": []}
    return {
        "status": "PASS", "level": "local regression", "test": TEST,
        "posted": posts[0], "resource_identities": identities,
        "formal_acceptance": "NOT_RUN", "environment": "PREPARING",
        "limits": ["Diagnostic transport, not a new OwnerFs/DFS fault deployment",
                   "Posted and unconsumed does not prove physical DMA pending",
                   "No exceptional provider-destroy or hardware failure qualification"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--source", required=True, type=pathlib.Path)
    parser.add_argument("--device", required=True)
    parser.add_argument("--evidence", required=True, type=pathlib.Path)
    args = parser.parse_args()
    assert platform.system() == "Linux", "run only in Linux"
    binary, source, directory = args.binary.resolve(), args.source.resolve(), args.evidence.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    native = source / "common/transport/native/rdma.c"
    lines = native.read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith("int afs_rdma_transfer("))
    line = next(i for i in range(start, len(lines)) if "uint64_t deadline=monotonic_ms()+timeout_ms;" in lines[i])
    assert "if (ibv_post_send(" in lines[line - 1], "checkpoint must follow successful data post"
    script = directory / "checkpoint.gdb"
    script.write_text(textwrap.dedent(f"""\
        set pagination off
        set confirm off
        set print thread-events off
        set debuginfod enabled off
        set non-stop on
        set environment AFS_TEST_RDMA_DEVICE {args.device}
        set environment AFS_TEST_RDMA_CHECKPOINT {directory}
        python
        import gdb, json, pathlib, time
        directory = pathlib.Path({str(directory)!r})
        def record(value):
            with (directory / 'debugger.jsonl').open('a') as stream:
                stream.write(json.dumps(value) + '\\n')
        def exited(event):
            record({{'event': 'exit', 'code': getattr(event, 'exit_code', None)}})
        gdb.events.exited.connect(exited)
        class Posted(gdb.Breakpoint):
            def stop(self):
                record({{'event': 'posted', 'pid': gdb.selected_inferior().pid,
                        'thread': list(gdb.selected_thread().ptid),
                        'gdb_thread': gdb.selected_thread().global_num,
                        'operation': int(gdb.parse_and_eval('operation')),
                        'bytes': int(gdb.parse_and_eval('len')),
                        'data_wr_id': int(gdb.parse_and_eval('wr.wr_id')),
                        'poisoned': bool(gdb.parse_and_eval('e->poisoned')),
                        'qp': int(gdb.parse_and_eval('e->qp->qp_num')),
                        'monotonic': time.monotonic()}})
                (directory / 'posted').write_text('WQE accepted; CQ unconsumed')
                # Return promptly: GDB must process clone/vfork/exit events
                # for the other running threads while this worker is stopped.
                return True
        checkpoint = Posted('native/rdma.c:{line + 1}')
        checkpoint.condition = 'operation == 1 && len == 4096'
        end
        run --exact {TEST} --ignored --nocapture --test-threads=1
    """))
    command = ["gdb", "-nx", "-q", "-x", str(script), str(binary)]
    (directory / "identity.json").write_text(json.dumps({
        "binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "native_source_sha256": hashlib.sha256(native.read_bytes()).hexdigest(),
        "runner_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
        "checkpoint_line": line + 1, "command": command,
    }, indent=2))
    with (directory / "gdb.log").open("w") as output:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True, start_new_session=True)
        def capture():
            for line in process.stdout:
                output.write(line)
                output.flush()
        reader = threading.Thread(target=capture)
        reader.start()
        resumed = False
        deadline = time.monotonic() + 65
        try:
            while process.poll() is None:
                if time.monotonic() >= deadline:
                    raise TimeoutError("debugger/test exceeded bounded runtime")
                event_file = directory / "debugger.jsonl"
                events = [json.loads(line) for line in event_file.read_text().splitlines()] if event_file.exists() else []
                if (directory / "resume").exists() and not resumed:
                    posted = next(event for event in events if event["event"] == "posted")
                    with event_file.open("a") as stream:
                        stream.write(json.dumps({"event": "resumed", "monotonic": time.monotonic()}) + "\n")
                    process.stdin.write(f"thread {posted['gdb_thread']}\ncontinue\n")
                    process.stdin.flush()
                    resumed = True
                if any(event["event"] == "exit" for event in events):
                    process.stdin.write("quit\n")
                    process.stdin.flush()
                    process.wait(timeout=5)
                    break
                time.sleep(0.01)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
            reader.join(timeout=5)
            (directory / "runner.exit").write_text(str(process.returncode) + "\n")
    assert process.returncode == 0, "debugger failed; inspect raw evidence"
    report = audit(directory)
    (directory / "audit.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()

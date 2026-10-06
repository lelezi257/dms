#!/usr/bin/env python3
"""Capture two independent DFS readers; elapsed time is diagnostic only."""
import json
import pathlib
import socket
import subprocess
import sys
import time

r = pathlib.Path('/var/tmp/afs-e2e-cross-20261006-r1')
node, phase, start_at = sys.argv[1], sys.argv[2], float(sys.argv[3])
prefix = pathlib.Path('/var/tmp/afs-e2e-20261006-r1/prefix') if node == 'node-a' else r / 'prefix'
delay = start_at - time.time()
if delay > 0:
    time.sleep(min(delay, 10))
output = r / f'results/{phase}'
argv = [str(prefix / 'bin/afs-selfcheck'), '--mount', str(r / 'mount/dfs'),
        '--workspace', 'g2-cross-dfs', '--phase', 'read',
        '--input', str(r / 'results/dfs-a-write/manifest.json'),
        '--output', str(output), '--deadline', '180']
started = time.time_ns()
monotonic = time.monotonic_ns()
with (r / f'results/{phase}.log').open('w') as log:
    process = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT)
    try:
        rc = process.wait(timeout=220)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        raise
ended = time.time_ns()
record = {'node': node, 'hostname': socket.gethostname(), 'harness_pid': process.pid,
          'argv': argv, 'requested_start_unix_seconds': start_at,
          'start_unix_ns': started, 'end_unix_ns': ended,
          'elapsed_ns': time.monotonic_ns() - monotonic, 'returncode': rc,
          'status': 'PASS' if rc == 0 else 'FAIL',
          'scope': 'functional independent reader; wall-clock alignment is not performance clock qualification'}
(r / f'results/{phase}-process.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record))
raise SystemExit(rc)

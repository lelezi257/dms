"""Bounded Linux namespace load, separate from release or performance proof."""
import argparse
import hashlib
import json
import os
import platform
import subprocess
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--mount', type=Path, required=True)
parser.add_argument('--run-dir', type=Path, required=True)
parser.add_argument('--node-pid', type=int, required=True)
parser.add_argument('--node-sha', required=True)
parser.add_argument('--meta-pid', type=int, required=True)
parser.add_argument('--meta-sha', required=True)
parser.add_argument('--seconds', type=float, default=180)
parser.add_argument('--width', type=int, default=256)
args = parser.parse_args()
assert platform.system() == 'Linux' and os.geteuid() == 0
assert 0 < args.seconds <= 300 and 0 < args.width <= 1024
args.run_dir.mkdir(parents=True, exist_ok=False)

def identity():
    identities = {}
    for role, pid, expected in [('node', args.node_pid, args.node_sha), ('meta', args.meta_pid, args.meta_sha)]:
        proc = Path('/proc') / str(pid)
        exe = proc / 'exe'
        digest = hashlib.sha256(exe.read_bytes()).hexdigest()
        assert digest == expected, (role, digest, expected)
        stat = (proc / 'stat').read_text()
        start = int(stat[stat.rfind(')') + 2:].split()[19])
        identities[role] = {'pid': pid, 'sha256': digest, 'start_ticks': start,
                            'exe': os.readlink(exe), 'exe_inode': exe.stat().st_ino}
    mnt = subprocess.run(['findmnt', '-T', str(args.mount), '-o', 'TARGET,SOURCE,FSTYPE', '--json'],
                         check=True, capture_output=True, text=True)
    observed = json.loads(mnt.stdout)['filesystems']
    assert len(observed) == 1 and observed[0]['target'] == str(args.mount)
    assert observed[0]['source'] == 'afs-dfs' and observed[0]['fstype'].startswith('fuse')
    return {'processes': identities, 'mount': observed,
            'boot_id': Path('/proc/sys/kernel/random/boot_id').read_text().strip()}

def save(name, data):
    (args.run_dir / name).write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')

before = identity()
save('before.json', before)
fixture = args.mount / ('namespace-load-' + args.run_dir.name)
fixture.mkdir(mode=0o755)
started = time.time()
started_mono = time.monotonic()
operations = 1
rounds = 0
error = None
with (args.run_dir / 'events.jsonl').open('w') as events:
    def emit(event):
        event.update({'unix_seconds': time.time(), 'operations': operations})
        events.write(json.dumps(event, sort_keys=True) + '\n')
        events.flush()
        print(json.dumps(event, sort_keys=True), flush=True)
    emit({'event': 'START', 'fixture': str(fixture)})
    try:
        while time.monotonic() - started_mono < args.seconds:
            for index in range(args.width):
                path = fixture / ('file-' + str(index))
                fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
                os.close(fd)
                operations += 2
            for index in range(args.width):
                path = fixture / ('file-' + str(index))
                renamed = fixture / ('moved-' + str(index))
                path.rename(renamed)
                assert renamed.stat().st_size == 0
                renamed.unlink()
                operations += 3
            rounds += 1
            emit({'event': 'ROUND', 'round': rounds})
        fixture.rmdir()
        operations += 1
    except Exception as exc:
        error = {'type': type(exc).__name__, 'message': str(exc)}
    after = identity()
    save('after.json', after)
    proof = {'status': 'PASS' if error is None and after == before and rounds > 0 else 'FAIL',
             'started_unix_seconds': started, 'finished_unix_seconds': time.time(),
             'operations': operations, 'rounds': rounds, 'process_mount_identity_stable': after == before,
             'error': error, 'fixture': str(fixture), 'fixture_kept': fixture.exists(),
             'scope': 'bounded namespace load; not release or performance qualification'}
    save('result.json', proof)
    emit({'event': 'FINISH', **proof})
raise SystemExit(0 if proof['status'] == 'PASS' else 1)

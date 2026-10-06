#!/usr/bin/env python3
"""Verify this run stopped normally without touching other services."""
import json
import pathlib
import socket
import subprocess

if not __debug__:
    raise RuntimeError('validation requires normal Python')
r = pathlib.Path('/var/tmp/afs-e2e-cross-20261006-r1')
base = r / 'results'
before = json.loads((base / 'after-dfs-meta-restart-identity.json').read_text())
checks = []

def check(name, passed, evidence):
    checks.append({'name': name, 'status': 'PASS' if passed else 'FAIL', 'evidence': evidence})
    if not passed:
        raise RuntimeError(name)

check('managed-stop-exit', (base / 'stop.exit').read_text().strip() == '0', 'stop.exit')
statuses = [json.loads(line) for line in (base / 'stopped-status.jsonl').read_text().splitlines()]
check('all-managed-services-stopped', len(statuses) == len(before['processes']) and
      all(s['state'] == 'stopped' and s['exit_code'] == '0' and not s['pid'] for s in statuses),
      'stopped-status.jsonl')
for name, process in before['processes'].items():
    stat = pathlib.Path(f"/proc/{process['pid']}/stat")
    same_process = stat.exists() and int(stat.read_text().rsplit(')', 1)[1].split()[19]) == process['starttick']
    check(name + '-original-process-gone', not same_process, process)
    # processctl retains the last PID as a diagnostic; /proc starttick is authority.
    pidfile = r / f'run/{name}.pid'
    check(name + '-pidfile-has-no-new-process', not pidfile.exists() or
          pidfile.read_text().strip() in ('', str(process['pid'])), f'run/{name}.pid')
for name in ('ownerfs', 'dfs'):
    command = ['findmnt', '-J', '--mountpoint', str(r / f'mount/{name}')]
    result = subprocess.run(command, text=True, capture_output=True)
    check(name + '-unmounted', result.returncode == 1 and not result.stdout.strip(),
          {'command': command, 'exit': result.returncode, 'stderr': result.stderr})
proof = {'status': 'PASS', 'hostname': socket.gethostname(), 'run_root': str(r), 'checks': checks}
(base / 'cleanup-proof.json').write_text(json.dumps(proof, indent=2) + '\n')
print(json.dumps({'status': 'PASS', 'checks': len(checks), 'hostname': socket.gethostname()}))

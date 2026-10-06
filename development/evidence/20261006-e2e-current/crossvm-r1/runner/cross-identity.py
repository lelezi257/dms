#!/usr/bin/env python3
"""Record live process/ELF/mount identities and explicit directory barrier."""
import hashlib
import json
import os
import pathlib
import socket
import subprocess
import sys

r = pathlib.Path('/var/tmp/afs-e2e-cross-20261006-r1')
phase = sys.argv[1]
result = {'hostname': socket.gethostname(), 'phase': phase, 'processes': {}, 'mounts': {}}
for name in ('meta', 'node'):
    pidfile = r / f'run/{name}.pid'
    if not pidfile.exists():
        continue
    pid = int(pidfile.read_text())
    with open(f'/proc/{pid}/exe', 'rb') as f:
        sha = hashlib.file_digest(f, 'sha256').hexdigest()
    stat = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    result['processes'][name] = {'pid': pid, 'starttick': int(stat[19]), 'elf_sha256': sha}
for name in ('ownerfs', 'dfs'):
    result['mounts'][name] = json.loads(subprocess.check_output(
        ['findmnt', '-J', '--mountpoint', str(r / f'mount/{name}')], text=True))
    assert result['mounts'][name]['filesystems'][0]['source'] == f'afs-{name}'
if phase == 'before-meta-restart':
    path = r / 'mount/ownerfs/g2-cross-owner'
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
        result['directory_barrier'] = {'path': str(path), 'fsync': 'PASS'}
    finally:
        os.close(fd)
(r / f'results/{phase}-identity.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result))

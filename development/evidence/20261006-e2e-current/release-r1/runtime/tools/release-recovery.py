#!/usr/bin/env python3
"""Record the release candidate's bounded recovery and cleanup proof."""
import hashlib
import json
import os
import pathlib
import subprocess
import sys

if not __debug__:
    raise RuntimeError('validation requires normal Python')
r = pathlib.Path('/var/tmp/afs-e2e-release-20261006-r1')
out = r / 'results'
read = lambda name: json.loads((out / name).read_text())
phase = sys.argv[1]
def identity():
    value = {'processes': {}, 'mounts': {}}
    for name in ('node', 'meta'):
        pid = int((r / f'run/{name}.pid').read_text())
        stat = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
        value['processes'][name] = {'pid': pid, 'starttick': int(stat[19]),
            'sha256': hashlib.sha256(pathlib.Path(f'/proc/{pid}/exe').read_bytes()).hexdigest()}
    for name in ('ownerfs', 'dfs'):
        result = subprocess.run(['findmnt', '-J', '--mountpoint', str(r/f'mount/{name}')],
                                capture_output=True, text=True, check=True)
        value['mounts'][name] = json.loads(result.stdout)
    return value
if phase == 'before':
    fd = os.open(r/'mount/ownerfs/g2-release-recovery', os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    value = identity()
    value['directory_fsync'] = 'PASS'
elif phase == 'after':
    value = identity()
    before = read('recovery-before.json')
    assert value['processes']['node'] == before['processes']['node']
    assert value['mounts'] == before['mounts']
    old, new = before['processes']['meta'], value['processes']['meta']
    assert old['pid'] != new['pid'] and old['starttick'] < new['starttick'] and old['sha256'] == new['sha256']
    written, restored = read('owner-recovery-write/manifest.json'), read('owner-recovery-read/manifest.json')
    assert written['status'] == restored['status'] == 'PASS'
    assert written['sha256'] == restored['sha256'] and written['size'] == restored['size'] == 67108864
    value.update(status='PASS', scope='Owner 64MiB orderly central local-file Meta restart', sha256=written['sha256'])
elif phase == 'stop':
    before = read('recovery-after.json')
    states = [json.loads(line) for line in (out/'stopped-status.jsonl').read_text().splitlines()]
    assert (out/'stop.exit').read_text().strip() == '0'
    assert len(states) == 2 and all(s['state']=='stopped' and s['exit_code']=='0' and not s['pid'] for s in states)
    for process in before['processes'].values():
        path = pathlib.Path(f"/proc/{process['pid']}/stat")
        assert not path.exists() or int(path.read_text().rsplit(')',1)[1].split()[19]) != process['starttick']
    for name in ('ownerfs','dfs'):
        result = subprocess.run(['findmnt','-J','--mountpoint',str(r/f'mount/{name}')],capture_output=True,text=True)
        assert result.returncode == 1 and not result.stdout.strip()
    value = {'status':'PASS','scope':'managed exit0, original processes gone, both mounts removed'}
else:
    raise RuntimeError('unknown phase')
(out/f'recovery-{phase}.json').write_text(json.dumps(value,indent=2)+'\n')
print(json.dumps({'phase':phase,'status':value.get('status','RECORDED')}))

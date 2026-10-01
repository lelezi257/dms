#!/usr/bin/env python3
"""Equivalent replay of retained inline Linux cold-capacity commands.

Reconstructed helpers, not the original execution filename. Use only on the
dedicated cohort prepared by round2-capacity.py.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import platform
import signal
import time

assert platform.system() == 'Linux' and platform.machine() == 'aarch64' and os.geteuid() == 0
spec = importlib.util.spec_from_file_location('capacity', pathlib.Path(__file__).with_name('round2-capacity.py'))
capacity = importlib.util.module_from_spec(spec); spec.loader.exec_module(capacity)
parser = argparse.ArgumentParser(); parser.add_argument('action', choices=('kill', 'read', 'cleanup')); args = parser.parse_args()
run = pathlib.Path(capacity.VOLUMES['a'])/capacity.NAME
name = {'kill': 'post-error-kill', 'read': 'cold-committed-read', 'cleanup': 'cleanup'}[args.action]
assert not (run/'evidence'/f'{name}.json').exists(), 'Never repeat a recorded fault or overwrite an attempt'

if args.action == 'kill':
    result = {'identity': capacity.identity(run, 'node'), 'volume': capacity.usage(run/'volume'),
              'same_mount_dfs_dirty_view': capacity.check_read(capacity.paths(run)['dfs'], capacity.PATCH),
              'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}
    assert result['volume']['available_bytes'] == 0
    pidfd = os.pidfd_open(result['identity']['pid'])
    try:
        assert capacity.identity(run, 'node') == result['identity']
        signal.pidfd_send_signal(pidfd, signal.SIGKILL)
    finally: os.close(pidfd)
    result['signal'] = 'SIGKILL'
elif args.action == 'read':
    result = {'identity': capacity.identity(run, 'node'), 'volume': capacity.usage(run/'volume'),
              'reads': {kind: capacity.check_read(path, capacity.BASE) for kind, path in capacity.paths(run).items()}}
    assert result['volume']['available_bytes'] == 0
else:
    assert not (run/'run/node.pid').exists(), 'Stop this cohort before inspecting its disk'
    result = {'read_only_inspection': capacity.run_cmd(['mount', '-o', 'loop,ro', str(run/'capacity.ext4'), str(run/'volume')])}
    try:
        result['physical'] = [{'path': str(path.relative_to(run/'volume')), 'size': path.stat().st_size,
                               'sha256': capacity.digest(path)} for path in sorted((run/'volume/node').rglob('*')) if path.is_file()]
        assert not (run/'volume/owned-capacity-filler').exists()
    finally: result['umount'] = capacity.run_cmd(['umount', str(run/'volume')])
    loops = json.loads(capacity.run_cmd(['losetup', '--json', '--list'])['stdout'])['loopdevices']
    result['auto_detached'] = not any(row['back-file'] == str(run/'capacity.ext4') for row in loops)
    assert result['auto_detached']
    result['host_volume_after'] = capacity.usage(capacity.VOLUMES['a'])
    assert result['host_volume_after']['available_bytes'] >= 4*1024**3
    result['image_sha256'] = capacity.digest(run/'capacity.ext4')
capacity.save(run, name, result)
print(json.dumps(result, indent=2))

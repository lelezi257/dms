#!/usr/bin/env python3
"""Physical ext4 ENOSPC development fixture; separate memory authority and state.

The bounded image resides on the agreed guest virtio/ext4 data volume. This is
supplemental development evidence, not a qualified release device matrix.
"""
import argparse
import errno
import hashlib
import json
import os
import pathlib
import platform
import subprocess
import time
import tomllib

VOLUMES = {'ctl': '/mnt/lima-afsctlstate', 'a': '/mnt/lima-afsadata'}
BASELINE = 'afs-delivery/round1-mainline-v77-archive-async'
NAME = 'afs-delivery/round2-capacity-v79'
SHA = {'node': 'd28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494',
       'meta': '64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7'}
BASE = b'capacity-v79-original\x00' * 391
BASE = (BASE * 2)[:8192]
PATCH = (b'capacity-v79-new-data\x03' * 512)[:8192]


def run_cmd(argv, timeout=60):
    result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    value = {'argv': argv, 'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}
    assert result.returncode == 0, value
    return value


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def save(run, action, value):
    destination = run/'evidence'/f'{action}.json'
    assert not destination.exists(), 'Do not overwrite an attempt'
    destination.write_text(json.dumps(value, indent=2)+'\n')


def controller(run, action, role):
    return run_cmd([str(run/'prefix/bin/afs-processctl'), '--prefix', str(run/'prefix'),
                    '--config-dir', str(run/'etc'), '--run-dir', str(run/'run'),
                    '--log-dir', str(run/'log'), action, role])


def usage(volume):
    stat = os.statvfs(volume)
    return {'free_bytes': stat.f_bfree*stat.f_frsize, 'available_bytes': stat.f_bavail*stat.f_frsize,
            'fragment_bytes': stat.f_frsize}


def identity(run, role):
    pid = int((run/'run'/f'{role}.pid').read_text().strip())
    proc = pathlib.Path('/proc')/str(pid)
    assert (proc/'exe').resolve() == (run/'prefix/bin'/f'afs-{role}').resolve()
    assert digest(proc/'exe') == SHA[role]
    stat = (proc/'stat').read_text()
    return {'pid': pid, 'start_ticks': stat[stat.rindex(')')+2:].split()[19],
            'boot_id': pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            'sha256': digest(proc/'exe'), 'exe': str((proc/'exe').resolve())}


def paths(run):
    return {'ownerfs': run/'mount/ownerfs/workspace-capacity-v79/data.bin',
            'dfs': run/'mount/dfs/data.bin'}


def io_call(label, operation):
    started = time.monotonic()
    try:
        value = operation()
        return {'operation': label, 'outcome': 'success', 'value': value,
                'elapsed_seconds': time.monotonic()-started}
    except OSError as error:
        return {'operation': label, 'outcome': 'error', 'errno': error.errno,
                'errno_name': errno.errorcode.get(error.errno), 'error': str(error),
                'elapsed_seconds': time.monotonic()-started}


def check_read(path, expected):
    fd = os.open(path, os.O_RDONLY)
    try:
        value = os.read(fd, len(expected)+1)
        assert value == expected and os.read(fd, 1) == b'', (path, len(value), hashlib.sha256(value).hexdigest())
    finally:
        os.close(fd)
    return {'bytes': len(value), 'sha256': hashlib.sha256(value).hexdigest(),
            'inode': os.stat(path).st_ino, 'length': os.stat(path).st_size}


def prepare(run, which):
    assert not run.exists()
    volume = pathlib.Path(VOLUMES[which])
    before = usage(volume)
    assert before['available_bytes'] >= 4*1024**3
    baseline = volume/BASELINE
    for role, expected in SHA.items():
        assert digest(baseline/'prefix/bin'/f'afs-{role}') == expected
    run.mkdir()
    for name in ('etc', 'run', 'log', 'state', 'mount', 'evidence'):
        (run/name).mkdir()
    (run/'prefix').symlink_to(baseline/'prefix', target_is_directory=True)
    role = 'meta' if which == 'ctl' else 'node'
    text = (baseline/'etc'/f'{role}.toml').read_text()
    cfg = tomllib.loads(text)
    for key in ('grpc_listen', 'rest_listen', 'data_dir', 'meta_endpoint', 'advertise_endpoint',
                'uds_path', 'ownerfs_mount', 'dfs_mount', 'data_mode', 'rdma_device'):
        text = '\n'.join(line for line in text.splitlines() if not line.startswith(key+' ='))+'\n'
    port = 20080 if which == 'ctl' else 20082
    ip = '192.168.109.11' if which == 'ctl' else '192.168.109.12'
    text += f'grpc_listen = "0.0.0.0:{port}"\nrest_listen = "{ip}:{port+1}"\n'
    listeners = run_cmd(['ss', '-ltnH'])['stdout']
    assert all(not row.split()[3].endswith(':'+str(p)) for row in listeners.splitlines() for p in (port, port+1))
    commands = []
    if which == 'ctl':
        text += f'data_dir = "{run}/state/meta"\n'
    else:
        image = run/'capacity.ext4'
        with image.open('xb') as stream:
            # A fixed 128MiB logical image bounds shared-volume consumption.
            # Formatting can discard backing blocks; the later filler records
            # its actual allocated blocks and real ENOSPC separately.
            stream.write(b'\x00'*(128*1024**2))
            stream.flush(); os.fsync(stream.fileno())
        commands.append(run_cmd(['mkfs.ext4', '-F', '-m', '0', str(image)]))
        (run/'volume').mkdir()
        commands.append(run_cmd(['mount', '-o', 'loop', str(image), str(run/'volume')]))
        text += f'''meta_endpoint = "https://192.168.109.11:20080"
advertise_endpoint = "https://192.168.109.12:20082"
data_dir = "{run}/volume/node"
uds_path = "{run}/run/node.sock"
ownerfs_mount = "{run}/mount/ownerfs"
dfs_mount = "{run}/mount/dfs"
data_mode = "grpc"
'''
    (run/'etc'/f'{role}.toml').write_text(text)
    assert tomllib.loads(text)['dfs_sync_required_copies'] == cfg['dfs_sync_required_copies'] == 1
    assert usage(volume)['available_bytes'] >= 4*1024**3
    return {'baseline': str(baseline), 'configuration_sha256': digest(run/'etc'/f'{role}.toml'),
            'configuration': tomllib.loads(text), 'host_volume_before': before,
            'host_volume_after': usage(volume), 'commands': commands,
            'device_matrix_qualification': False}


def main(args):
    assert platform.system() == 'Linux' and platform.machine() == 'aarch64' and os.geteuid() == 0
    run = pathlib.Path(VOLUMES[args.which])/NAME
    role = 'meta' if args.which == 'ctl' else 'node'
    result = {'action': args.action, 'which': args.which, 'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'formal_acceptance': 'NOT_RUN', 'environment': 'PREPARING'}
    if args.action == 'prepare':
        result['prepare'] = prepare(run, args.which)
    elif args.action in ('start', 'stop', 'restart'):
        result['controller'] = controller(run, args.action, role)
        if args.action != 'stop': result['identity'] = identity(run, role)
    elif args.action == 'identity':
        result['identity'] = identity(run, role)
    else:
        assert args.which == 'a'
        result['identity'] = identity(run, role)
        for kind in ('ownerfs', 'dfs'):
            mount = json.loads(run_cmd(['findmnt', '-J', '-M', str(run/'mount'/kind), '-o', 'SOURCE,FSTYPE'])['stdout'])['filesystems'][0]
            assert mount['source'] == 'afs-'+kind and mount['fstype'].startswith('fuse')
        volume = run/'volume'
        if args.action == 'seed':
            os.mkdir(run/'mount/ownerfs/workspace-capacity-v79')
            for kind, path in paths(run).items():
                fd = os.open(path, os.O_CREAT|os.O_EXCL|os.O_RDWR, 0o600)
                try:
                    assert os.write(fd, BASE) == len(BASE)
                    os.fdatasync(fd); os.fsync(fd)
                finally: os.close(fd)
                result[kind] = check_read(path, BASE)
            result['volume'] = run_cmd(['findmnt', '-J', '-T', str(volume), '-o', 'TARGET,SOURCE,FSTYPE,OPTIONS'])
        elif args.action == 'fill':
            filler = volume/'owned-capacity-filler'
            # Resume only this root-owned bounded fixture after an explicit
            # preparation failure; never truncate or replace a product file.
            if filler.exists():
                assert filler.is_file() and filler.stat().st_uid == 0
                assert filler.stat().st_size <= 128*1024**2
            result['before'] = usage(volume)
            fd = os.open(filler, os.O_CREAT|os.O_APPEND|os.O_WRONLY, 0o600)
            count = os.fstat(fd).st_size
            try:
                while True:
                    try: count += os.write(fd, b'\x5a'*1024**2)
                    except OSError as error:
                        assert error.errno == errno.ENOSPC
                        result['write_errno'] = error.errno
                        break
                result['filler_sync'] = io_call('filler-fsync', lambda: os.fsync(fd))
            finally: os.close(fd)
            result['written_bytes'] = count
            result['allocated_bytes'] = filler.stat().st_blocks*512
            result['after'] = usage(volume)
            # Restore only this disposable filler if the controlling process
            # disappears. The watchdog never touches product files or mounts.
            log = (run/'evidence/filler-watchdog.log').open('w')
            watchdog = subprocess.Popen(['bash', '-c', 'sleep 180; rm -f -- "$1"',
                                         'capacity-watchdog', str(filler)], start_new_session=True,
                                        stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
            log.close()
            result['watchdog'] = {'pid': watchdog.pid, 'ttl_seconds': 180, 'owned_filler': str(filler)}
            assert count <= 128*1024**2 and result['allocated_bytes'] > 0
            # ext4 may keep metadata/emergency blocks counted in f_bfree that
            # even this root writer cannot allocate. Actual ENOSPC plus zero
            # f_bavail proves usable-capacity exhaustion.
            assert result['after']['available_bytes'] == 0, result
        elif args.action == 'fault':
            result['before'] = usage(volume)
            assert result['before']['available_bytes'] == 0
            for kind, path in paths(run).items():
                fd = os.open(path, os.O_WRONLY)
                if kind == 'ownerfs': os.lseek(fd, len(BASE), os.SEEK_SET)
                result[kind] = [io_call('write', lambda: os.write(fd, PATCH)),
                                io_call('fdatasync', lambda: os.fdatasync(fd)),
                                io_call('fsync', lambda: os.fsync(fd)),
                                io_call('close', lambda: os.close(fd))]
                save(run, 'fault-'+kind, {'identity': result['identity'], 'operations': result[kind],
                                         'volume': usage(volume)})
                # Only explicit failures can qualify failed durability. Buffered
                # DFS write may succeed; it is never reported as durable success.
                if kind == 'ownerfs':
                    assert result[kind][0].get('errno') == errno.ENOSPC, result[kind]
                else:
                    assert result[kind][1].get('errno') == errno.ENOSPC, result[kind]
                    assert result[kind][2].get('errno') == errno.ENOSPC, result[kind]
                    assert result[kind][3].get('errno') == errno.ENOSPC, result[kind]
                # Same-mount readers intentionally see accepted DFS dirty data;
                # this is not evidence of a committed FileVersion. A separate
                # cold reload while still full proves the retained committed
                # watermark. Owner's rejected append changes no bytes.
                result[kind+'-visible-read'] = check_read(path, PATCH if kind == 'dfs' else BASE)
            result['after'] = usage(volume)
        elif args.action == 'recover':
            filler = volume/'owned-capacity-filler'
            assert filler.is_file()
            result['before'] = usage(volume)
            filler.unlink()
            result['after'] = usage(volume)
            assert result['after']['free_bytes'] > 64*1024**2
            for kind, path in paths(run).items():
                fd = os.open(path, os.O_WRONLY)
                try:
                    assert os.write(fd, PATCH) == len(PATCH)
                    os.fdatasync(fd); os.fsync(fd)
                finally: os.close(fd)
                result[kind] = check_read(path, PATCH)
        elif args.action == 'verify':
            for kind, path in paths(run).items(): result[kind] = check_read(path, PATCH)
        else: raise ValueError(args.action)
    result['status'] = 'PASS'
    save(run, args.action, result)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('which', choices=VOLUMES)
    parser.add_argument('action', choices=('prepare', 'start', 'stop', 'restart', 'identity', 'seed', 'fill', 'fault', 'recover', 'verify'))
    args = parser.parse_args()
    print(json.dumps(main(args), indent=2), flush=True)

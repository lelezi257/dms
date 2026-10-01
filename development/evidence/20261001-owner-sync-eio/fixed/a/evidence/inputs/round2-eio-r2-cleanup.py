#!/usr/bin/env python3
"""Bounded Linux product EIO fixture on an exclusively owned dm device.

Uses the archived capacity helper for identity/controller/IO recording. This
supplements REL-08; it is not the formal device or transport matrix.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import platform
import signal
import subprocess
import time
import tomllib

spec = importlib.util.spec_from_file_location('capacity', pathlib.Path(__file__).with_name('round2-capacity.py'))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
c.NAME = 'afs-delivery/round2-eio-v80-r2'
c.BASELINE = '/var/lib/afs-acceptance/candidates/eio-v80-r2'
c.SHA = json.loads(pathlib.Path('/home/lzc.guest/eio-v80-r2-binaries.json').read_text())
c.BASE = (b'eio-v80-acknowledged\x00' * 512)[:8192]
c.PATCH = (b'eio-v80-unconfirmed\x03' * 512)[:8192]
MAPPER = 'afs_round2_eio_v80_r2'
UUID = 'AFS-round2-eio-v80-r2-owned'
SECTORS = 32 * 1024**2 // 512


def paths(run):
    return {'ownerfs': run/'mount/ownerfs/workspace-eio-v80/data.bin',
            'dfs': run/'mount/dfs/data.bin'}


def device(run):
    result = json.loads((run/'evidence/prepare.json').read_text())['prepare']
    assert c.run_cmd(['dmsetup', 'info', '-c', '--noheadings', '-o', 'uuid', MAPPER])['stdout'].strip() == UUID
    return result['linear_table']


def restore(run):
    table = device(run)
    commands = [c.run_cmd(['dmsetup', 'suspend', '--noflush', MAPPER]),
                c.run_cmd(['dmsetup', 'reload', MAPPER, '--table', table]),
                c.run_cmd(['dmsetup', 'resume', MAPPER]),
                c.run_cmd(['dmsetup', 'table', MAPPER])]
    assert ' linear ' in commands[-1]['stdout']
    return commands


def prepare(run, which):
    assert not run.exists()
    volume = pathlib.Path(c.VOLUMES[which])
    before = c.usage(volume)
    assert before['available_bytes'] >= 4*1024**3
    template = volume/'afs-delivery/round2-capacity-v79'
    baseline = volume/c.BASELINE
    for role, sha in c.SHA.items():
        assert c.digest(baseline/'prefix/bin'/f'afs-{role}') == sha
    role = 'meta' if which == 'ctl' else 'node'
    text = (template/'etc'/f'{role}.toml').read_text().replace(str(template), str(run))
    text = text.replace(':20080', ':20280').replace(':20081', ':20281').replace(':20082', ':20282').replace(':20083', ':20283')
    commands = []
    ports = (20280, 20281) if which == 'ctl' else (20282, 20283)
    listeners = c.run_cmd(['ss', '-ltnH'])['stdout']
    assert all(not row.split()[3].endswith(':'+str(port)) for row in listeners.splitlines() for port in ports)
    run.mkdir()
    for directory in ('etc', 'run', 'log', 'state', 'mount', 'evidence'):
        (run/directory).mkdir()
    (run/'prefix').symlink_to(baseline/'prefix', target_is_directory=True)
    result = {'host_volume_before': before, 'commands': commands}
    if which == 'a':
        assert MAPPER not in c.run_cmd(['dmsetup', 'ls'])['stdout']
        image = run/'eio.ext4'
        with image.open('xb') as stream:
            stream.truncate(SECTORS*512)
            stream.flush(); os.fsync(stream.fileno())
        loop = c.run_cmd(['losetup', '--find', '--show', str(image)])['stdout'].strip()
        assert loop.startswith('/dev/loop')
        table = f'0 {SECTORS} linear {loop} 0'
        commands.append(c.run_cmd(['dmsetup', 'create', MAPPER, '--uuid', UUID, '--table', table]))
        commands.append(c.run_cmd(['mkfs.ext4', '-F', '-m', '0', '/dev/mapper/'+MAPPER]))
        (run/'volume').mkdir()
        commands.append(c.run_cmd(['mount', '/dev/mapper/'+MAPPER, str(run/'volume')]))
        result.update(loop=loop, mapper=MAPPER, uuid=UUID, linear_table=table,
                      image_bytes=SECTORS*512, volume=c.run_cmd(['findmnt', '-J', '-M', str(run/'volume')]))
    (run/'etc'/f'{role}.toml').write_text(text)
    result.update(configuration=tomllib.loads(text), configuration_sha256=c.digest(run/'etc'/f'{role}.toml'),
                  host_volume_after=c.usage(volume))
    assert result['host_volume_after']['available_bytes'] >= 4*1024**3
    return result


def perform(run, action, which):
    role = 'meta' if which == 'ctl' else 'node'
    if action == 'prepare': return {'prepare': prepare(run, which)}
    if action in ('start', 'stop', 'restart'):
        result = {'controller': c.controller(run, action, role)}
        if action != 'stop': result['identity'] = c.identity(run, role)
        return result
    if action == 'remount':
        assert which == 'a'
        assert not (run/'run/node.pid').exists() or not (pathlib.Path('/proc')/(run/'run/node.pid').read_text().strip()).exists()
        assert ' linear ' in c.run_cmd(['dmsetup', 'table', MAPPER])['stdout']
        return {'commands': [c.run_cmd(['umount', str(run/'volume')]),
                             c.run_cmd(['mount', '/dev/mapper/'+MAPPER, str(run/'volume')]),
                             c.run_cmd(['findmnt', '-J', '-M', str(run/'volume')])]}
    if action == 'cleanup':
        assert which == 'a'
        pidfile=run/'run/node.pid'
        assert not pidfile.exists() or not (pathlib.Path('/proc')/pidfile.read_text().strip()).exists()
        prepared = json.loads((run/'evidence/prepare.json').read_text())['prepare']
        assert c.run_cmd(['dmsetup','info','-c','--noheadings','-o','uuid',MAPPER])['stdout'].strip() == UUID
        commands = [c.run_cmd(['umount', str(run/'volume')]), c.run_cmd(['dmsetup','remove',MAPPER]),
                    c.run_cmd(['losetup','-d',prepared['loop']])]
        assert MAPPER not in c.run_cmd(['dmsetup','ls'])['stdout']
        assert str(run/'eio.ext4') not in c.run_cmd(['losetup','--list','--output','NAME,BACK-FILE'])['stdout']
        return {'commands':commands,'image_sha256':c.digest(run/'eio.ext4'),'host_volume_after':c.usage(c.VOLUMES[which])}
    result = {'identity': c.identity(run, role)}
    if action == 'identity': return result
    assert which == 'a'
    if action == 'seed':
        os.mkdir(run/'mount/ownerfs/workspace-eio-v80')
        for kind, path in paths(run).items():
            fd = os.open(path, os.O_CREAT|os.O_EXCL|os.O_RDWR, 0o600)
            try:
                assert os.write(fd, c.BASE) == len(c.BASE)
                os.fdatasync(fd); os.fsync(fd)
            finally: os.close(fd)
            result[kind] = c.check_read(path, c.BASE)
        result['volume_sync'] = c.run_cmd(['sync', '-f', str(run/'volume')])
        result['table'] = c.run_cmd(['dmsetup', 'table', MAPPER])
    elif action in ('fault', 'fault-remote'):
        table = device(run)
        since = '@'+str(int(time.time()))
        # Independently restore only the UUID-bound device if the controller
        # disappears. This run does not claim watchdog-kill qualification.
        watch = subprocess.Popen(['bash', '-c',
            'sleep 90; u=$(dmsetup info -c --noheadings -o uuid "$1"); '
            'if [ "$u" = "$2" ]; then dmsetup suspend --noflush "$1"; '
            'dmsetup reload "$1" --table "$3"; dmsetup resume "$1"; fi',
            'eio-watchdog', MAPPER, UUID, table], start_new_session=True,
            stdin=subprocess.DEVNULL, stdout=(run/'evidence'/(action+'-watchdog.log')).open('w'), stderr=subprocess.STDOUT)
        result['watchdog'] = {'pid': watch.pid, 'ttl_seconds': 90, 'uuid': UUID}
        try:
            result['activate'] = [c.run_cmd(['dmsetup', 'suspend', '--noflush', MAPPER]),
                c.run_cmd(['dmsetup', 'reload', MAPPER, '--table', f'0 {SECTORS} error']),
                c.run_cmd(['dmsetup', 'resume', MAPPER]), c.run_cmd(['dmsetup', 'table', MAPPER])]
            assert ' error' in result['activate'][-1]['stdout']
            result['activated_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
            if action == 'fault-remote':
                (run/'evidence/remote-armed').write_text(result['activated_utc']+'\n')
                deadline = time.monotonic()+40
                while not (run/'evidence/remote-done').exists():
                    assert time.monotonic() < deadline, 'remote controller did not complete'
                    time.sleep(.05)
            else:
                for kind, path in paths(run).items():
                    fd = os.open(path, os.O_WRONLY)
                    # OwnerFs uses mutable local files: an unconfirmed append may
                    # survive. Preserve the acknowledged prefix, never assume
                    # rollback of an accepted overwrite after failed fsync.
                    if kind == 'ownerfs': os.lseek(fd, len(c.BASE), os.SEEK_SET)
                    result[kind] = [c.io_call('write', lambda: os.write(fd, c.PATCH)),
                        c.io_call('fdatasync', lambda: os.fdatasync(fd)),
                        c.io_call('fsync', lambda: os.fsync(fd)),
                        c.io_call('close', lambda: os.close(fd))]
                    c.save(run, 'fault-'+kind, {'identity': result['identity'], 'operations': result[kind]})
            result['kernel'] = c.run_cmd(['journalctl', '-k', '--since', since, '--no-pager', '-o', 'short-iso'])
        finally:
            result['restore'] = restore(run)
            result['restored_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
            watch.terminate()
            result['watchdog']['normal_cleanup_exit'] = watch.wait(timeout=5)
        c.save(run, action+'-observed', result)
        for kind in (() if action == 'fault-remote' else ('ownerfs', 'dfs')):
            assert all(op['outcome'] == 'error' for op in result[kind][1:]), result[kind]
            assert result[kind][1].get('errno') == 5, result[kind]
        assert 'I/O error' in result['kernel']['stdout'] or 'Buffer I/O' in result['kernel']['stdout']
    elif action == 'kill':
        pid = result['identity']['pid']
        pidfd = os.pidfd_open(pid)
        try:
            assert c.identity(run, role) == result['identity']
            signal.pidfd_send_signal(pidfd, signal.SIGKILL)
        finally: os.close(pidfd)
        result['signal'] = 'SIGKILL'
    elif action in ('remote-seed', 'remote-recover', 'verify-remote-old', 'verify-remote-new'):
        path = run/'mount/ownerfs/workspace-eio-v80/remote.bin'
        if action in ('remote-seed','remote-recover'):
            flags = os.O_CREAT|os.O_EXCL if action=='remote-seed' else os.O_TRUNC
            fd = os.open(path, flags|os.O_RDWR, 0o600)
            try:
                value = c.BASE if action=='remote-seed' else c.PATCH
                assert os.write(fd,value)==len(value)
                os.fdatasync(fd); os.fsync(fd)
            finally: os.close(fd)
            result['ownerfs'] = c.check_read(path,value)
            if action=='remote-seed':
                size_path=path.with_name('remote-size.bin')
                size_fd=os.open(size_path,os.O_CREAT|os.O_EXCL|os.O_RDWR,0o600)
                try:
                    assert os.write(size_fd,c.BASE)==8192
                    os.fdatasync(size_fd); os.fsync(size_fd)
                finally: os.close(size_fd)
                result['resize_seed']=c.check_read(size_path,c.BASE)
            result['volume_sync'] = c.run_cmd(['sync','-f',str(run/'volume')])
        elif action=='verify-remote-old':
            fd = os.open(path,os.O_RDONLY)
            try:
                data=os.read(fd,16385)
                assert data[:8192]==c.BASE and 8192<=len(data)<=16384 and not os.read(fd,1)
            finally: os.close(fd)
            result['ownerfs']={'acknowledged_prefix_bytes':8192,'acknowledged_prefix_sha256':c.hashlib.sha256(data[:8192]).hexdigest(),'actual_length':len(data),'unconfirmed_tail_bytes':len(data)-8192}
        else: result['ownerfs']=c.check_read(path,c.PATCH)
    elif action in ('verify-old', 'verify-new', 'recover'):
        expected = c.BASE if action == 'verify-old' else c.PATCH
        for kind, path in paths(run).items():
            if action == 'recover':
                fd = os.open(path, os.O_WRONLY|os.O_TRUNC)
                try:
                    assert os.write(fd, c.PATCH) == len(c.PATCH)
                    os.fdatasync(fd); os.fsync(fd)
                finally: os.close(fd)
            if action == 'verify-old' and kind == 'ownerfs':
                fd = os.open(path, os.O_RDONLY)
                try:
                    data = os.read(fd, 2*len(c.BASE)+1)
                    assert data[:len(c.BASE)] == c.BASE
                    assert len(c.BASE) <= len(data) <= 2*len(c.BASE)
                    assert not os.read(fd, 1)
                finally: os.close(fd)
                result[kind] = {'acknowledged_prefix_bytes': len(c.BASE),
                    'acknowledged_prefix_sha256': c.hashlib.sha256(data[:len(c.BASE)]).hexdigest(),
                    'actual_length': len(data), 'unconfirmed_tail_bytes': len(data)-len(c.BASE)}
            else: result[kind] = c.check_read(path, expected)
    else: raise ValueError(action)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('which', choices=c.VOLUMES)
    parser.add_argument('action')
    parser.add_argument('--record', help='Unique evidence name for lifecycle repeats')
    args = parser.parse_args()
    assert platform.system() == 'Linux' and platform.machine() == 'aarch64' and os.geteuid() == 0
    run = pathlib.Path(c.VOLUMES[args.which])/c.NAME
    result = {'action': args.action, 'which': args.which, 'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'formal_acceptance': 'NOT_RUN', 'environment': 'PREPARING'}
    try:
        result.update(perform(run, args.action, args.which)); result['status'] = 'PASS'
    except BaseException as error:
        result.update(status='FAIL', error=repr(error)); raise
    finally:
        if run.exists(): c.save(run, args.record or args.action, result)
        print(json.dumps(result, indent=2), flush=True)

#!/usr/bin/env python3
"""Root Linux probe for isolated production OwnerFs mounts (not formal acceptance)."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import tomllib
import urllib.request

RUN = {
    'a': '/mnt/lima-afsadata/afs-delivery/owner-rdma-v72-a',
    'b': '/mnt/lima-afsbdata/afs-delivery/owner-rdma-v72-b',
}
TEMPLATE = {
    'a': '/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v51',
    'b': '/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v51',
}
SHA = {
    'node': '8105b3d66f2e93a1110b442ccbda24a2d4c2ab5f97d0bbd5d9f0c9e1b97f6238',
    'meta': '3345187a3915cbc27651f7027d4dcb093f82af5b9ca3ffc213f08cef8a30c905',
}
WORKSPACE = 'owner-rdma-v72'
SIZE = 4 * 1024 * 1024 + 17


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def scalar(text, key, value):
    line = f'{key} = {value}'
    expression = re.compile(rf'^{re.escape(key)}\s*=.*$', re.MULTILINE)
    return expression.sub(line, text) if expression.search(text) else text.rstrip() + '\n' + line + '\n'


def config(which, role, text):
    assert which in RUN and role in ('node', 'meta')
    text = text.replace(TEMPLATE[which], RUN[which])
    for old in range(18280, 18286):
        text = text.replace(str(old), str(old + 500))
    text = text.replace('memory-node-a', 'owner-rdma-node-a').replace('memory-node-b', 'owner-rdma-node-b')
    text = scalar(text, 'fs', "'ownerfs'")
    if role == 'meta':
        text = scalar(text, 'id', "'owner-rdma-meta'")
        assert tomllib.loads(text)['meta_store'] == 'memory'
    else:
        text = re.sub(r'^dfs_mount\s*=.*\n?', '', text, flags=re.MULTILINE)
        text = scalar(text, 'data_mode', "'rdma'")
        text = scalar(text, 'rdma_device', "'rxe0'")
        text = scalar(text, 'timeout_ms', '10000')
    cfg = tomllib.loads(text)
    assert cfg['data_dir'].startswith(RUN[which] + '/')
    assert set(cfg['trusted_node_certs']) == {'owner-rdma-node-a', 'owner-rdma-node-b'}
    return text


def command(args):
    return subprocess.check_output(args, text=True, timeout=20)


def rest():
    # Management REST is a separate authority check, not a file-byte path.
    root_id = 'root-' + WORKSPACE.encode().hex()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open('http://192.168.109.12:18781/v1/roots/' + root_id, timeout=10) as response:
        value = json.load(response)
    assert value['home_node_id'] == 'owner-rdma-node-a' and value['home_serving'], value
    assert value['home_grpc_addr'] == 'https://192.168.109.12:18782', value
    return value


def prepare(which):
    run = pathlib.Path(RUN[which])
    assert not run.exists(), 'never overwrite an existing runtime'
    volume = json.loads(command(['findmnt', '-J', '-T', str(run.parent), '-o', 'TARGET,SOURCE,FSTYPE']))['filesystems'][0]
    assert volume['fstype'] == 'ext4', volume
    listening = command(['ss', '-ltnH'])
    ports = range(18780, 18784) if which == 'a' else range(18784, 18786)
    for row in listening.splitlines():
        assert not any(row.split()[3].endswith(':' + str(port)) for port in ports), row
    assert 'link rxe0/1 state ACTIVE' in command(['rdma', 'link', 'show'])
    free = os.statvfs(run.parent)
    assert free.f_bavail * free.f_frsize >= 4 * 1024**3
    roles = ('meta', 'node') if which == 'a' else ('node',)
    rendered = {role: config(which, role, (pathlib.Path(TEMPLATE[which]) / 'etc' / (role + '.toml')).read_text()) for role in roles}
    for cfg in rendered.values():
        for key, value in tomllib.loads(cfg).items():
            if key.startswith('tls_') and key != 'tls_server_name':
                assert pathlib.Path(value).is_file()
    for rel in ('prefix/bin', 'etc', 'run', 'log', 'state/node', 'state/meta', 'mount-ownerfs'):
        (run / rel).mkdir(parents=True, exist_ok=True)
    for folder in (run, *[p for p in run.rglob('*') if p.is_dir()]):
        os.chown(folder, int(os.environ['SUDO_UID']), int(os.environ['SUDO_GID']))
    for role, cfg in rendered.items():
        (run / 'etc' / (role + '.toml')).write_text(cfg)
    return {'prepared': str(run), 'volume': volume, 'ports': list(ports), 'available_bytes': free.f_bavail * free.f_frsize}


def identity(which):
    run = pathlib.Path(RUN[which])
    result = {'run': str(run), 'boot_id': pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(), 'processes': {}, 'configs': {}}
    for role in (('meta', 'node') if which == 'a' else ('node',)):
        pid = int((run / 'run' / (role + '.pid')).read_text())
        proc = pathlib.Path('/proc') / str(pid)
        assert os.readlink(proc / 'exe') == str(run / 'prefix/bin' / ('afs-' + role))
        assert digest(proc / 'exe') == SHA[role]
        result['processes'][role] = {'pid': pid, 'start_ticks': (proc / 'stat').read_text().rsplit(') ', 1)[1].split()[19], 'sha256': SHA[role]}
        cfg_path = run / 'etc' / (role + '.toml')
        cfg = tomllib.loads(cfg_path.read_text())
        assert cfg['fs'] == 'ownerfs'
        if role == 'node':
            assert cfg['data_mode'] == 'rdma' and cfg['rdma_device'] == 'rxe0'
        result['configs'][role] = digest(cfg_path)
    mount = json.loads(command(['findmnt', '-J', '-M', str(run / 'mount-ownerfs'), '-o', 'TARGET,SOURCE,FSTYPE']))['filesystems'][0]
    assert mount['source'] == 'afs-ownerfs' and mount['fstype'].startswith('fuse')
    result['mount'] = mount
    result['rdma_link'] = command(['rdma', 'link', 'show'])
    return result


def payload(patched=False):
    data = bytearray(bytes(range(251)) * (SIZE // 251 + 1))[:SIZE]
    if patched:
        data[1024 * 1024 - 13:1024 * 1024 - 13 + 4096] = b'P' * 4096
    return data


def file_action(which, action):
    assert which == 'b', 'all remote file IO originates at B'
    authority = rest()
    path = pathlib.Path(RUN[which]) / 'mount-ownerfs' / WORKSPACE / 'data.bin'
    patched = action in ('patch', 'read-patched')
    data = payload(patched)
    if action == 'write':
        fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
        try:
            offset = 0
            while offset < len(data):
                n = os.write(fd, data[offset:offset + 1024 * 1024])
                assert n > 0
                offset += n
            os.fdatasync(fd)
            os.fsync(fd)
        finally:
            os.close(fd)
    elif action == 'patch':
        fd = os.open(path, os.O_RDWR)
        try:
            assert os.pwrite(fd, b'P' * 4096, 1024 * 1024 - 13) == 4096
            os.fsync(fd)
        finally:
            os.close(fd)
    elif action in ('read', 'read-patched'):
        fd = os.open(path, os.O_RDONLY)
        try:
            actual = bytearray()
            while True:
                piece = os.read(fd, 1024 * 1024)
                if not piece:
                    break
                actual.extend(piece)
            assert actual == data
            assert os.pread(fd, 1, SIZE) == b''
            assert os.fstat(fd).st_size == SIZE
        finally:
            os.close(fd)
    return {'action': action, 'bytes': SIZE, 'sha256': hashlib.sha256(data).hexdigest(), 'home': authority, 'identity': identity(which)}


def collect(which):
    observation = identity(which)
    run = pathlib.Path(RUN[which])
    pid = observation['processes']['node']['pid']
    tids = {int(p.name) for p in pathlib.Path('/proc', str(pid), 'task').iterdir()}
    observation['task_ids'] = sorted(tids)
    observation['owned_resources'] = {}
    for kind in ('qp', 'cq', 'mr', 'pd', 'ctx'):
        rows = json.loads(command(['rdma', '-j', 'resource', 'show', kind]))
        observation['owned_resources'][kind] = [row for row in rows if row.get('pid') in tids]
    port = 18783 if which == 'a' else 18785
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(f'http://127.0.0.1:{port}/metrics', timeout=10) as response:
        observation['metrics'] = response.read().decode()
    observation['node_log'] = (run / 'log/node.log').read_text()
    observation['physical_files'] = [{
        'path': str(p), 'bytes': p.stat().st_size, 'sha256': digest(p),
    } for p in (run / 'state/node/ownerfs').rglob('data.bin')]
    return observation


def stopped(which):
    before = json.loads(pathlib.Path('/tmp/owner-v72-final.json').read_text())
    run = pathlib.Path(RUN[which])
    assert before['run'] == str(run)
    assert before['boot_id'] == pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    receipts = {}
    for role, old in before['processes'].items():
        proc = pathlib.Path('/proc', str(old['pid']))
        assert not proc.exists() or (proc / 'stat').read_text().rsplit(') ', 1)[1].split()[19] != old['start_ticks']
        assert digest(run / 'prefix/bin' / ('afs-' + role)) == old['sha256'] == SHA[role]
        assert digest(run / 'etc' / (role + '.toml')) == before['configs'][role]
        lifecycle = pathlib.Path((run / 'run' / (role + '.launch')).read_text().strip())
        assert lifecycle.parent == run / 'run' and lifecycle.name.startswith(role + '.lifecycle.')
        text = (lifecycle / 'exit').read_text()
        fields = dict(line.split('=', 1) for line in text.splitlines())
        assert fields['pid'] == str(old['pid']) and fields['start_ticks'] == old['start_ticks']
        assert fields['boot_id'] == before['boot_id'] and fields['exit_code'] == '0'
        assert fields['exe'] == str(run / 'prefix/bin' / ('afs-' + role))
        assert fields['config'] == str(run / 'etc' / (role + '.toml'))
        receipts[role] = fields
    probe = subprocess.run(['findmnt', '-rn', '--mountpoint', str(run / 'mount-ownerfs')], capture_output=True, text=True, timeout=10)
    assert probe.returncode == 1 and probe.stdout == ''
    retained = {}
    for kind in ('qp', 'cq', 'mr', 'pd', 'ctx'):
        rows = json.loads(command(['rdma', '-j', 'resource', 'show', kind]))
        retained[kind] = [row for row in rows if row.get('pid') in before['task_ids']]
    assert not any(retained.values())
    return {'run': str(run), 'boot_id': before['boot_id'], 'receipts': receipts,
            'mount_absent': True, 'owned_retained': retained}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('which', choices=RUN)
    parser.add_argument('action', choices=('prepare', 'identity', 'home', 'collect', 'stopped', 'create', 'write', 'read', 'patch', 'read-patched'))
    args = parser.parse_args()
    assert platform.system() == 'Linux' and platform.machine() == 'aarch64' and os.geteuid() == 0
    if args.action == 'prepare':
        result = prepare(args.which)
    elif args.action == 'identity':
        result = identity(args.which)
    elif args.action == 'home':
        result = {'home': rest(), 'identity': identity(args.which)}
    elif args.action == 'collect':
        result = collect(args.which)
    elif args.action == 'stopped':
        result = stopped(args.which)
    elif args.action == 'create':
        assert args.which == 'a'
        (pathlib.Path(RUN['a']) / 'mount-ownerfs' / WORKSPACE).mkdir()
        result = {'home': rest(), 'identity': identity('a')}
    else:
        result = file_action(args.which, args.action)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()

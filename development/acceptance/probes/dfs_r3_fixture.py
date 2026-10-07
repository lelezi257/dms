#!/usr/bin/env python3
"""One fresh current R3 DFS fixture: config, admission and backing-volume budget only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import stat
import subprocess
import tomllib

FIXTURE = 'dfs-r3-current-20261007-r1'
VOLUMES = {'ctl': '/mnt/lima-afsctlstate', 'a': '/mnt/lima-afsadata',
           'b': '/mnt/lima-afsbdata', 'c': '/mnt/lima-afscdata'}
IPS = {role: '192.168.109.' + str(number) for role, number in zip(VOLUMES, range(11, 15))}
NODES = {role: 'dfs-' + role + '-r3' for role in ('a', 'b', 'c')}
SHA = {'meta': '4150942fd873b879ab6dc9034904116cf1670904a259d6a081f3b0c1fd1e7343',
       'node': '9478f3e89905b310689c0727ce0adf28fc11a25444c9634f82055a664069b41d'}
POLICY = {'dfs_desired_copies': 3, 'dfs_sync_required_copies': 3,
          'dfs_min_distinct_nodes': 3, 'dfs_min_distinct_failure_domains': 3,
          'dfs_local_copy': 'required'}
CEILING, FLOOR, RAM = 2**30, 2**30, 512 * 2**20


def require(ok, reason):
    if not ok:
        raise RuntimeError(reason)


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def safe(path, root, exists=True):
    path, root = Path(path), Path(root)
    require(path.is_absolute() and path.is_relative_to(root) and '..' not in path.parts, 'path escape')
    require(all(not p.is_symlink() for p in (path, *path.parents)), 'symlink path')
    require(not exists or path.exists(), 'missing path: ' + str(path))
    return path


def toml(value):
    if isinstance(value, dict):
        return '{ ' + ', '.join(k + ' = ' + json.dumps(v) for k, v in value.items()) + ' }'
    return json.dumps(value)


def render(config):
    return '\n'.join(k + ' = ' + toml(v) for k, v in config.items()) + '\n'


def validate_policy(config):
    require(all(type(config.get(k)) is type(v) and config[k] == v for k, v in POLICY.items()),
            'exact three-copy synchronous policy required')


def validate_elf(path, expected):
    require(os.access(path, os.X_OK) and digest(path) == expected, 'wrong current ELF')
    with path.open('rb') as stream:
        header = stream.read(20)
    require(header[:6] == b'\x7fELF\x02\x01' and int.from_bytes(header[18:20], 'little') == 183, 'not ARM64 ELF')


def allocated(root):
    """Never descend into the fixture mount subtree or query its FUSE statfs."""
    root, seen, amount = Path(root), set(), 0
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = [name for name in dirs if Path(directory) / name != root / 'mount']
        for path in [Path(directory), *(Path(directory) / name for name in files)]:
            safe(path.parent if path != root else root, root)
            st = path.lstat()
            alias = path in (root / 'etc/node-dfs.toml', root / 'etc/node-ownerfs.toml')
            if alias and stat.S_ISLNK(st.st_mode):
                require(st.st_uid == 0 and os.readlink(path) == 'node.toml', 'unsafe installer alias')
                target = safe(root / 'etc/node.toml', root)
                require(stat.S_ISREG(target.lstat().st_mode), 'installer alias target not regular')
            else:
                safe(path, root)
            require(st.st_dev == root.stat().st_dev, 'nested non-mount device escape')
            own_socket = path == root / 'run/node.sock' and stat.S_ISSOCK(st.st_mode) and st.st_uid == 0
            require(stat.S_ISREG(st.st_mode) or stat.S_ISDIR(st.st_mode) or own_socket
                    or (alias and stat.S_ISLNK(st.st_mode)),
                    'nonregular budget entry')
            if (st.st_dev, st.st_ino) not in seen:
                seen.add((st.st_dev, st.st_ino))
                amount += st.st_blocks * 512
        for name in dirs:
            safe(Path(directory) / name, root)
    return amount


def ram():
    meminfo = Path('/proc/meminfo').read_text()
    available = int(re.search(r'^MemAvailable:\s+(\d+) kB$', meminfo, re.M)[1]) * 1024
    require(available >= RAM, 'RAM available below512MiB')
    membership = Path('/proc/self/cgroup').read_text().strip()
    require(membership.startswith('0::/'), 'cgroup-v2 memory observation required')
    base = Path('/sys/fs/cgroup')
    current = base / membership.split('::', 1)[1].lstrip('/')
    limits = []
    for directory in (current, *current.parents):
        if directory == base.parent:
            break
        if (directory / 'memory.max').is_file():
            maximum = (directory / 'memory.max').read_text().strip()
            used = int((directory / 'memory.current').read_text())
            require(maximum == 'max' or int(maximum) - used >= RAM, 'cgroup memory headroom below512MiB')
            limits.append({'path': str(directory), 'maximum': maximum, 'current': used})
    require(bool(limits), 'missing effective cgroup memory limits')
    return {'available_bytes': available, 'minimum_bytes': RAM, 'limits': limits, 'meminfo': meminfo}


class Fixture:
    def __init__(self, role, fixture_name=FIXTURE, expected_sha=None, *, ceiling_bytes=CEILING, floor_bytes=FLOOR):
        require(fixture_name and Path(fixture_name).name == fixture_name and '..' not in Path(fixture_name).parts,
                'invalid fixture name')
        require(type(ceiling_bytes) is int and ceiling_bytes > 0 and
                type(floor_bytes) is int and floor_bytes > 0, 'positive integer capacity budget required')
        self.ceiling, self.floor = ceiling_bytes, floor_bytes
        self.role, self.volume = role, Path(VOLUMES[role])
        self.fixture_name = fixture_name
        self.root = self.volume / 'afs-delivery' / fixture_name
        self.name = 'meta' if role == 'ctl' else 'node'
        self.config = self.root / 'etc' / (self.name + '.toml')
        self.binary = self.root / 'prefix/bin' / ('afs-' + self.name)
        self.sha = {**SHA, **(expected_sha or {})}
        self.commands = []

    def command(self, argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, text=True, timeout=20)
        self.commands.append({'argv': result.args, 'rc': result.returncode,
                              'stdout': result.stdout, 'stderr': result.stderr})
        require(result.returncode == 0, 'command failed: ' + repr(self.commands[-1]))
        return result.stdout

    def guest(self):
        require(platform.system() == 'Linux' and platform.machine() == 'aarch64' and os.geteuid() == 0,
                'Linux ARM64 root required')
        require(platform.node() == 'lima-afs-accept-' + self.role, 'wrong guest role')
        require(safe(self.root, self.volume).stat().st_uid == 0, 'fixture must be root-owned')

    def ports(self):
        return (24700, 24701) if self.role == 'ctl' else (24800, 24801)

    def expected(self):
        tls = self.root / 'etc/tls'
        name = 'meta' if self.role == 'ctl' else NODES[self.role]
        result = {'id': 'meta-ctl' if self.role == 'ctl' else name, 'fs': 'dfs',
                  'experimental_native_workspace': False, 'experimental_ownerfs_workspace_bind': False,
                  'data_dir': str(self.root / 'state' / self.name),
                  'grpc_listen': '0.0.0.0:' + str(self.ports()[0]),
                  'rest_listen': '0.0.0.0:' + str(self.ports()[1]), 'log_level': 'info', 'trace_enabled': False,
                  'tls_ca_certificate': str(tls / 'ca.pem'), 'tls_identity_certificate': str(tls / (name + '.pem')),
                  'tls_identity_private_key': str(tls / (name + '-key.pem')), 'tls_server_name': 'afs-meta',
                  'trusted_node_certs': {n: str(tls / (n + '.pem')) for n in NODES.values()}, **POLICY}
        if self.role == 'ctl':
            result['meta_store'] = 'local-file'
        else:
            result.update(meta_endpoint='https://' + IPS['ctl'] + ':24700', data_mode='grpc', allow_volatile_meta=False,
                          advertise_endpoint='https://' + IPS[self.role] + ':24800',
                          uds_path=str(self.root / 'run/node.sock'), dfs_mount=str(self.root / 'mount/dfs'))
        return result

    def fresh(self):
        for directory in (self.root / 'state', self.root / 'run'):
            safe(directory, self.root, exists=False)
            require(not directory.exists() or not any(not p.is_dir() or p.is_symlink() for p in directory.rglob('*')),
                    'state/runtime already initialized; policy patch forbidden')
        for port in self.ports():
            with socket.socket() as sock:
                sock.bind(('0.0.0.0', port))
        mounts = Path('/proc/self/mountinfo').read_text().splitlines()
        require(not any(str(self.root) in row for row in mounts), 'fixture mount already active')

    def patch_text(self, text):
        require(not re.search(r'^\s*\[', text, re.M), 'flat generator config required')
        original, expected = tomllib.loads(text), self.expected()
        require(original.get('fs') == 'all' and original.get('id') == expected['id'], 'not fresh generated role')
        require(all(original.get(key, False) is False for key in ('experimental_native_workspace',
                    'experimental_ownerfs_workspace_bind')), 'workspace switch enabled in generated config')
        for key in ('grpc_listen', 'rest_listen', *(() if self.role == 'ctl' else ('meta_endpoint', 'advertise_endpoint'))):
            require(original.get(key) == expected[key], 'wrong generated endpoint: ' + key)
        r2 = {k: 2 if type(v) is int else v for k, v in POLICY.items()}
        require(all(type(original.get(k)) is type(v) and original[k] == v for k, v in r2.items()), 'not generated exact R2')
        require(set(original.get('trusted_node_certs', {})) == set(NODES.values()), 'exact three trusted nodes required')
        for node, path in original['trusted_node_certs'].items():
            require(Path(path).name == node + '.pem', 'swapped trust identity')
        for key in ('tls_ca_certificate', 'tls_identity_certificate', 'tls_identity_private_key'):
            require(Path(original.get(key, '')).name == Path(expected[key]).name, 'wrong TLS identity')
        require(original.get('meta_store') == 'local-file' if self.role == 'ctl' else
                original.get('data_mode') == 'grpc' and original.get('allow_volatile_meta') is False,
                'local-file/gRPC durable config required')
        require(not (set(original) - set(expected) - {'ownerfs_mount'}), 'unexpected generated fields')
        return render(expected)

    def patch(self):
        self.fresh()
        backup = self.config.with_name(self.name + '.original.toml')
        require(not os.path.lexists(backup), 'original backup exists; patch exclusive')
        text = safe(self.config, self.root).read_text()
        changed = self.patch_text(text)
        with safe(backup, self.root, exists=False).open('x') as stream:
            stream.write(text)
        self.config.write_text(changed)
        for relative in ('run', 'state/' + self.name, 'mount/dfs', 'logs'):
            safe(self.root / relative, self.root, exists=False).mkdir(parents=True, exist_ok=True)
        return {'status': 'PATCHED_NOT_ADMITTED', 'config_sha256': digest(self.config), 'original_sha256': digest(backup)}

    def budget(self):
        row = json.loads(self.command(['findmnt', '-J', '--mountpoint', self.volume]))['filesystems']
        require(len(row) == 1 and row[0]['target'] == str(self.volume) and row[0]['fstype'] == 'ext4', 'backing ext4 required')
        require(self.root.stat().st_dev == self.volume.stat().st_dev, 'fixture not on declared volume')
        used = allocated(self.root)
        info = os.statvfs(self.volume)
        free = info.f_bavail * info.f_frsize
        require(used <= self.ceiling and free >= self.floor and free + used >= self.floor + self.ceiling,
                'allocation/reserve limit failed')
        return {'allocated_bytes': used, 'free_bytes': free, 'volume': row[0], 'ceiling_bytes': self.ceiling,
                'floor_bytes': self.floor, 'excluded': str(self.root / 'mount'),
                'scope': 'one role only; parent must enforce its predeclared aggregate ceiling'}

    def preflight(self):
        self.fresh()
        deps = {n: shutil.which(n) for n in ('python3', 'bash', 'ldd', 'openssl', 'findmnt', 'ip',
                'ss', 'curl', 'timeout', 'flock', 'setsid', 'readlink', 'sed', 'awk', 'grep', 'stat', 'mktemp', 'nohup', 'head', 'tr')}
        require(all(deps.values()), 'missing dependency; no repair')
        cfg = tomllib.loads(safe(self.config, self.root).read_text())
        require(cfg == self.expected(), 'config differs from frozen R3 fixture')
        backup = safe(self.config.with_name(self.name + '.original.toml'), self.root)
        require(self.patch_text(backup.read_text()) == self.config.read_text(), 'original generated config mismatch')
        validate_policy(cfg)
        for key in ('data_dir', 'uds_path', 'dfs_mount', 'tls_ca_certificate', 'tls_identity_certificate', 'tls_identity_private_key'):
            if key in cfg:
                safe(cfg[key], self.root, exists=key.startswith('tls'))
        for path in cfg['trusted_node_certs'].values():
            safe(path, self.root)
        addresses = json.loads(self.command(['ip', '-j', '-4', 'addr']))
        require(any(a.get('local') == IPS[self.role] for row in addresses for a in row.get('addr_info', [])), 'wrong role IP')
        safe(self.binary, self.root)
        validate_elf(self.binary, self.sha[self.name])
        helper_sha = {}
        for name in ('afs-processctl', *(('afs-trial-config',) if self.role == 'ctl' else ())):
            path = safe(self.root / 'prefix/bin' / name, self.root)
            require(os.access(path, os.X_OK), 'helper not executable: ' + name)
            helper_sha[name] = digest(path)
        require('not found' not in self.command(['ldd', self.binary]), 'missing library')
        output = json.loads(self.command([self.binary, '--config', self.config, '--print-config']))
        for key, value in cfg.items():
            if key != 'fs':
                require(type(output.get(key)) is type(value) and output[key] == value, 'print-config mismatch: ' + key)
        require(output.get('dfs') is True and output.get('ownerfs') is False and output.get('ownerfs_mount') is None
                and output.get('native_workspace') is None and output.get('ownerfs_workspace_bind') is None, 'Owner workspace isolation failed')
        if self.role != 'ctl':
            require(stat.S_ISCHR(os.stat('/dev/fuse').st_mode) and shutil.which('fusermount3'), 'FUSE/unmount missing')
        tls = self.root / 'etc/tls'
        hashes = {'ca.pem': digest(tls / 'ca.pem')}
        for identity, host, ip in [('meta', 'afs-meta', IPS['ctl']),
                                   *((node, node, IPS[role]) for role, node in NODES.items())]:
            cert = safe(tls / (identity + '.pem'), self.root)
            for flag, value in (('-verify_hostname', host), ('-verify_ip', ip)):
                self.command(['openssl', 'verify', '-CAfile', tls / 'ca.pem', flag, value, cert])
            require(self.command(['openssl', 'x509', '-in', cert, '-noout', '-subject', '-nameopt', 'RFC2253']).strip()
                    == 'subject=CN=' + host, 'TLS subject identity mismatch')
            hashes[cert.name] = digest(cert)
        require(self.command(['openssl', 'x509', '-in', cfg['tls_identity_certificate'], '-pubkey', '-noout'])
                == self.command(['openssl', 'pkey', '-in', cfg['tls_identity_private_key'], '-pubout']), 'TLS keypair mismatch')
        return {'status': 'PASS_CONFIG_ADMISSION_ONLY', 'config_sha256': digest(self.config),
                'original_sha256': digest(backup), 'helper_sha256': helper_sha,
                'elf_sha256': self.sha[self.name], 'print_config': output, 'tls_sha256': hashes,
                'dependencies': deps, 'ram': ram(), 'budget': self.budget()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--role', required=True, choices=VOLUMES)
    parser.add_argument('--fixture-name', default=FIXTURE,
                        help='fresh fixture directory name under each role volume; historical default is unchanged')
    parser.add_argument('--meta-sha256', default=SHA['meta'],
                        help='expected afs-meta SHA256 for this fresh fixture')
    parser.add_argument('--node-sha256', default=SHA['node'],
                        help='expected afs-node SHA256 for this fresh fixture')
    parser.add_argument('--ceiling-bytes', type=int, default=CEILING,
                        help='predeclared per-role capacity ceiling; historical default remains1GiB')
    parser.add_argument('--floor-bytes', type=int, default=FLOOR,
                        help='predeclared backing free-space reserve; historical default remains1GiB')
    parser.add_argument('action', choices=('patch', 'preflight', 'budget'))
    args = parser.parse_args()
    fixture = None
    try:
        fixture = Fixture(args.role, args.fixture_name, {'meta': args.meta_sha256, 'node': args.node_sha256},
                          ceiling_bytes=args.ceiling_bytes, floor_bytes=args.floor_bytes)
        fixture.guest()
        result = getattr(fixture, args.action)()
        code = 0
    except (RuntimeError, OSError, ValueError, KeyError, subprocess.SubprocessError) as exc:
        result, code = {'status': 'BLOCKED', 'error': str(exc)}, 1
    print(json.dumps({'fixture': args.fixture_name, 'role': args.role,
                      'root': str(fixture.root) if fixture else None,
                      'commands': fixture.commands if fixture else [], **result}, indent=2))
    return code


if __name__ == '__main__':
    raise SystemExit(main())

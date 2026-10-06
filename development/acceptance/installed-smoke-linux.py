#!/usr/bin/env python3
"""Bounded installed default-OFF regression; all execution must be on Linux."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import stat
import subprocess
import time
import tomllib


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def verify_executable(running, installed):
    running_stat, installed_stat = os.stat(running), os.stat(installed)
    if ((running_stat.st_dev, running_stat.st_ino) !=
            (installed_stat.st_dev, installed_stat.st_ino)
            or Path(running).resolve() != Path(installed).resolve()):
        raise ValueError('process is not running the installed executable')
    return {'path': str(Path(running).resolve()), 'device': running_stat.st_dev,
            'inode': running_stat.st_ino}


def verify_recovery(before, after, writes, reads):
    if before['node'] != after['node'] or before['mounts'] != after['mounts']:
        raise ValueError('Node or mount changed during Meta-only restart')
    old, new = before['meta'], after['meta']
    if (old['pid'] == new['pid'] or old['starttick'] >= new['starttick']
            or old['sha256'] != new['sha256']):
        raise ValueError('Meta restart identity mismatch')
    for name in ('ownerfs', 'dfs'):
        write, read = writes[name], reads[name]
        if (write['status'] != 'PASS' or read['status'] != 'PASS'
                or write['size'] != 64 * 2**20 or read['size'] != write['size']
                or write['sha256'] != read['sha256']):
            raise ValueError(name + ' recovered data mismatch')


class Run:
    def __init__(self, args):
        self.args, self.root, self.out = args, args.root, args.out
        self.out.mkdir(parents=True, exist_ok=False)
        self.commands, self.checks = [], {}
        self.started = False

    def save(self, name, value):
        (self.out / name).write_text(json.dumps(value, indent=2) + '\n')

    def check(self, name, ok, value):
        self.checks[name] = {'status': 'PASS' if ok else 'FAIL', 'value': value}
        if not ok:
            raise ValueError(name)

    def command(self, argv, timeout=90, allowed=(0,)):
        prefix = f'{len(self.commands):03d}'
        began = time.monotonic()
        timed_out = False
        with (self.out / (prefix + '.stdout')).open('wb') as stdout, (
                self.out / (prefix + '.stderr')).open('wb') as stderr:
            try:
                result = subprocess.run([str(a) for a in argv], stdout=stdout,
                                        stderr=stderr, timeout=timeout)
                code = result.returncode
            except subprocess.TimeoutExpired:
                code, timed_out = 124, True
        record = {'argv': [str(a) for a in argv], 'exit': code,
                  'timeout': timed_out, 'elapsed_seconds': time.monotonic() - began,
                  'stdout': prefix + '.stdout', 'stderr': prefix + '.stderr'}
        self.commands.append(record)
        self.save('commands.json', self.commands)
        if code not in allowed:
            raise RuntimeError(f'command {prefix} exit {code}')
        return (self.out / (prefix + '.stdout')).read_text()

    def ctl(self, *action):
        return self.command([self.root / 'prefix/bin/afs-processctl',
            '--prefix', self.root / 'prefix', '--config-dir', self.root / 'etc',
            '--run-dir', self.root / 'run', '--log-dir', self.root / 'logs', *action])

    def preflight(self):
        self.check('Linux-root', platform.system() == 'Linux'
                   and platform.machine() == 'aarch64' and os.geteuid() == 0,
                   [platform.system(), platform.machine(), os.geteuid()])
        needed = ('bash', 'python3', 'timeout', 'openssl', 'findmnt', 'fusermount3',
                  'ldd', 'sha256sum', 'tar', 'ss', 'curl', 'flock', 'sed', 'awk',
                  'grep', 'mountpoint', 'install', 'readlink', 'setsid')
        deps = {name: shutil.which(name) for name in needed}
        self.check('dependencies', all(deps.values()), deps)
        compilers = {n: shutil.which(n) for n in ('cargo', 'rustc', 'cc', 'gcc', 'clang')}
        self.check('compiler-free', not any(compilers.values()), compilers)
        self.check('dedicated-new-root', self.root.is_absolute()
                   and self.root.parent == Path('/var/tmp') and not self.root.exists(),
                   str(self.root))
        self.check('capacity', shutil.disk_usage(self.root.parent).free >= 2**30,
                   {'free_bytes': shutil.disk_usage(self.root.parent).free,
                    'minimum_bytes': 2**30})
        self.check('fuse', stat.S_ISCHR(os.stat('/dev/fuse').st_mode), '/dev/fuse')
        fs = json.loads(self.command(['findmnt', '-J', '-T', self.root.parent]))
        self.check('ext4', fs['filesystems'][0]['fstype'] == 'ext4', fs)
        for port in (22400, 22401, 22500, 22501):
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', port))
        self.check('ports-free', True, [22400, 22401, 22500, 22501])
        self.check('package-sha256', sha(self.args.package) == self.args.package_sha256,
                   sha(self.args.package))
        listing = self.command(['tar', '-tzf', self.args.package]).splitlines()
        self.check('no-source-in-package', not any(
            '/src/' in p or p.endswith(('Cargo.toml', 'Cargo.lock', '.rs')) for p in listing), listing)
        self.save('preflight.json', self.checks)

    def install(self):
        self.root.mkdir()
        extracted = self.root / 'package'
        extracted.mkdir()
        self.command(['tar', '-xzf', self.args.package, '-C', extracted])
        packages = list(extracted.iterdir())
        self.check('one-package', len(packages) == 1 and packages[0].is_dir(),
                   [str(p) for p in packages])
        package = packages[0]
        manifest = json.loads((package / 'manifest.json').read_text())
        self.check('source-commit', manifest['source_commit'] == self.args.source_commit, manifest)
        self.save('package-manifest.json', manifest)
        self.command([package / 'install.sh', '--prefix', self.root / 'prefix',
            '--config-dir', self.root / 'etc', '--state-dir', self.root / 'state',
            '--run-dir', self.root / 'run', '--log-dir', self.root / 'logs',
            '--mount-root', self.root / 'mount'])
        for name in ('afs-meta', 'afs-node'):
            binary = self.root / 'prefix/bin' / name
            expected = getattr(self.args, name.replace('-', '_') + '_sha256')
            self.check(name + '-installed', sha(binary) == expected
                       and manifest['binaries'][name]['sha256'] == expected, sha(binary))
            libraries = self.command(['ldd', binary])
            self.check(name + '-libraries', 'not found' not in libraries, libraries)
        self.command([self.root / 'prefix/bin/afs-trial-config', 'single',
            '--backend', 'local-file', '--config-dir', self.root / 'etc',
            '--state-dir', self.root / 'state', '--run-dir', self.root / 'run',
            '--mount-root', self.root / 'mount', '--meta-grpc-port', '22400',
            '--meta-rest-port', '22401', '--node-grpc-port', '22500',
            '--node-rest-port', '22501', '--force'])
        for name in ('meta', 'node'):
            path = self.root / f'etc/{name}.toml'
            config = tomllib.loads(path.read_text())
            self.check(name + '-native-OFF', not config.get('experimental_native_workspace', False),
                       config.get('experimental_native_workspace', False))
            self.check(name + '-data-isolated', Path(config['data_dir']).is_relative_to(self.root),
                       config['data_dir'])
            if name == 'meta':
                self.check('local-file', config['meta_store'] == 'local-file', config['meta_store'])
            else:
                self.check('grpc', config['data_mode'] == 'grpc', config['data_mode'])
            self.command(['openssl', 'verify', '-CAfile', config['tls_ca_certificate'],
                          config['tls_identity_certificate']])
            shutil.copyfile(path, self.out / (name + '.toml'))

    def identity(self):
        value = {}
        for name in ('meta', 'node'):
            pid = int((self.root / f'run/{name}.pid').read_text())
            proc = Path(f'/proc/{pid}')
            ticks = int((proc / 'stat').read_text().rsplit(')', 1)[1].split()[19])
            expected = getattr(self.args, 'afs_' + name + '_sha256')
            digest = sha(proc / 'exe')
            self.check(name + '-running', digest == expected, digest)
            installed = self.root / ('prefix/bin/afs-' + name)
            executable = verify_executable(proc / 'exe', installed)
            self.check(name + '-installed-process', True, executable)
            value[name] = {'pid': pid, 'starttick': ticks, 'sha256': digest,
                           'executable': executable}
        value['mounts'] = {}
        for name in ('ownerfs', 'dfs'):
            mounted = json.loads(self.command(['findmnt', '-J', '-o',
                                              'TARGET,SOURCE,FSTYPE,OPTIONS,ID', '--mountpoint',
                                              self.root / ('mount/' + name)]))
            self.check(name + '-mount', mounted['filesystems'][0]['source'] == 'afs-' + name, mounted)
            value['mounts'][name] = mounted
        return value

    def selfcheck(self, name, phase):
        output = self.out / (name + '-' + phase + '.json')
        argv = [self.root / 'prefix/bin/afs-selfcheck', '--mount', self.root / ('mount/' + name),
                '--workspace', 'installed-off', '--size', '64MiB', '--phase', phase,
                '--case-id', 'installed-off-' + name, '--deadline', '180', '--output', output]
        if phase == 'read':
            argv += ['--input', self.out / (name + '-write.json')]
        self.command(argv, timeout=220)
        return json.loads(output.read_text())

    def run(self):
        result = {'status': 'BLOCKED', 'scope': 'current installed OFF small regression',
                  'source_commit': self.args.source_commit, 'tool_sha256': sha(__file__)}
        before, after = None, None
        try:
            self.preflight()
            self.install()
            result['status'] = 'FAIL'
            self.started = True
            self.ctl('start', 'all')
            before = self.identity()
            writes = {name: self.selfcheck(name, 'write') for name in ('ownerfs', 'dfs')}
            for name in ('ownerfs', 'dfs'):
                fd = os.open(self.root / ('mount/' + name + '/installed-off'), os.O_RDONLY | os.O_DIRECTORY)
                try:
                    os.fsync(fd)
                finally:
                    os.close(fd)
            self.save('recovery-before.json', before)
            self.ctl('restart', 'meta')
            after = self.identity()
            reads = {name: self.selfcheck(name, 'read') for name in ('ownerfs', 'dfs')}
            verify_recovery(before, after, writes, reads)
            self.save('recovery-after.json', after)
            result['recovery'] = {'status': 'PASS', 'size_each': 64 * 2**20,
                                  'sha256': {n: writes[n]['sha256'] for n in writes}}
            result['status'] = 'PASS'
        except Exception as exc:
            result['error'] = str(exc)
        finally:
            if self.started:
                try:
                    self.ctl('stop', 'all')
                    statuses = [json.loads(line) for line in self.ctl('--json', 'status', 'all').splitlines()]
                    self.check('managed-exit0', len(statuses) == 2 and all(
                        s['state'] == 'stopped' and str(s['exit_code']) == '0' and not s['pid']
                        for s in statuses), statuses)
                    for name in ('ownerfs', 'dfs'):
                        output = self.command(['findmnt', '-J', '--mountpoint',
                                               self.root / ('mount/' + name)], allowed=(1,))
                        self.check(name + '-removed', not output.strip(), output)
                    for label, recorded in (('before', before), ('after', after)):
                        for name in ('meta', 'node'):
                            if not recorded:
                                continue
                            old = recorded[name]
                            path = Path(f"/proc/{old['pid']}/stat")
                            self.check(label + '-' + name + '-original-gone', not path.exists() or int(
                                path.read_text().rsplit(')', 1)[1].split()[19]) != old['starttick'], old)
                    result['cleanup'] = 'PASS'
                except Exception as exc:
                    result.update(status='FAIL', cleanup='FAIL', cleanup_error=str(exc))
            result['checks'] = self.checks
            self.save('result.json', result)
        print(json.dumps({k: v for k, v in result.items() if k != 'checks'}))
        return 0 if result['status'] == 'PASS' else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', type=Path, required=True)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    for name in ('package-sha256', 'source-commit', 'afs-meta-sha256', 'afs-node-sha256'):
        parser.add_argument('--' + name, required=True)
    raise SystemExit(Run(parser.parse_args()).run())

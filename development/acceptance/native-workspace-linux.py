#!/usr/bin/env python3
"""Bounded real OwnerFs managed-container lifecycle, not production ON qualification."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import stat
import sys

spec = importlib.util.spec_from_file_location('installed_smoke', Path(__file__).with_name('installed-smoke-linux.py'))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def content_matches(path, size, digest):
    return path.stat().st_size == size and base.sha(path) == digest


def verify_final(observed, source, namespace):
    if (observed.get('source') != source or observed.get('namespace') != namespace
            or not isinstance(observed.get('unique_mount_id'), int)
            or observed['unique_mount_id'] <= 0
            or not {'nosuid', 'nodev'} <= set(observed.get('flags', []))):
        raise ValueError('final namespace/source/unique mount/flags mismatch')


class Run(base.Run):
    def native(self, ident, *action, error=False):
        text = self.command(['python3', self.args.controller, '--socket',
            self.root / 'control/control.sock', '--id', ident, *action],
            timeout=140, allowed=(1,) if error else (0,))
        value = json.loads(text)['response']
        self.check(ident + '-response', value.get('production_ready') is False
                   and ((value.get('status') == 'ERROR') == error), value)
        return value

    def preflight(self):
        self.check('Linux-root', platform.system() == 'Linux' and platform.machine() == 'aarch64'
                   and os.geteuid() == 0, [platform.system(), platform.machine(), os.geteuid()])
        self.check('new-owned-root', self.root.parent == Path('/opt') and not self.root.exists(), str(self.root))
        deps = {n: shutil.which(n) for n in ('bash', 'python3', 'tar', 'ldd', 'openssl', 'findmnt',
                'fusermount3', 'curl', 'flock', 'ss', 'sed', 'awk', 'mountpoint', 'setsid', 'nsenter')}
        self.check('dependencies', all(deps.values()), deps)
        self.check('fuse', stat.S_ISCHR(os.stat('/dev/fuse').st_mode), '/dev/fuse')
        fs = json.loads(self.command(['findmnt', '-J', '-T', '/opt']))
        self.check('ext4', fs['filesystems'][0]['fstype'] == 'ext4', fs)
        self.check('capacity', shutil.disk_usage('/opt').free >= 2**30, shutil.disk_usage('/opt').free)
        for port in (24400, 24401, 24500, 24501):
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', port))
        self.check('ports', True, [24400, 24401, 24500, 24501])
        self.check('package', base.sha(self.args.package) == self.args.package_sha256, base.sha(self.args.package))
        self.check('runtime', base.sha('/usr/local/sbin/runc') == self.args.runtime_sha256, base.sha('/usr/local/sbin/runc'))
        if self.args.semantics_only:
            self.check('semantics-only-has-probe', self.args.semantics_probe is not None, str(self.args.semantics_probe))
        if self.args.semantics_probe:
            self.check('semantics-tool', self.args.semantics_probe.is_file(), str(self.args.semantics_probe))
        self.command(['/usr/local/sbin/runc', '--version'])
        self.save('preflight.json', self.checks)

    def install(self):
        self.root.mkdir(mode=0o755)
        package_dir = self.root / 'package'
        package_dir.mkdir()
        self.command(['tar', '-xzf', self.args.package, '-C', package_dir])
        packages = list(package_dir.iterdir())
        self.check('one-package', len(packages) == 1, [str(p) for p in packages])
        package = packages[0]
        manifest = json.loads((package / 'manifest.json').read_text())
        self.check('source-commit', manifest['source_commit'] == self.args.source_commit, manifest)
        self.save('package-manifest.json', manifest)
        self.command([package / 'install.sh', '--prefix', self.root / 'prefix', '--config-dir', self.root / 'etc',
            '--state-dir', self.root / 'state', '--run-dir', self.root / 'run', '--log-dir', self.root / 'logs',
            '--mount-root', self.root / 'mount'])
        for n in ('meta', 'node'):
            binary = self.root / ('prefix/bin/afs-' + n)
            self.check(n + '-ELF', base.sha(binary) == getattr(self.args, 'afs_' + n + '_sha256'), base.sha(binary))
            libraries = self.command(['ldd', binary])
            self.check(n + '-libraries', 'not found' not in libraries, libraries)
        self.command([self.root / 'prefix/bin/afs-trial-config', 'single', '--backend', 'local-file',
            '--config-dir', self.root / 'etc', '--state-dir', self.root / 'state', '--run-dir', self.root / 'run',
            '--mount-root', self.root / 'mount', '--meta-grpc-port', '24400', '--meta-rest-port', '24401',
            '--node-grpc-port', '24500', '--node-rest-port', '24501', '--force'])
        inputs = json.loads(self.args.rootfs_inputs.read_text())
        rootfs = self.root / 'rootfs'
        rootfs.mkdir(mode=0o755)
        for rel, expected in inputs.items():
            src = self.args.template_rootfs / rel
            self.check('input-' + rel, src.is_file() and not src.is_symlink()
                       and base.sha(src) == expected['sha256'], expected)
            dst = rootfs / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(src, dst)
            dst.chmod(0o755)
        for name in ('workspace', 'proc'):
            (rootfs / name).mkdir(mode=0o755)
        (self.root / 'control').mkdir(mode=0o700)
        self.save('rootfs-inputs.json', inputs)
        for name in ('meta', 'node'):
            path = self.root / f'etc/{name}.toml'
            text = "\n".join(line for line in path.read_text().replace('fs = "all"', 'fs = "ownerfs"').splitlines()
                             if not line.startswith("dfs_mount =")) + "\n"
            if name == 'node':
                text = 'experimental_native_workspace = true\n' + text + '\n[native_workspace]\n' + (
                    f'control_dir = "{self.root}/control"\nruntime = "/usr/local/sbin/runc"\n'
                    f'rootfs = "{rootfs}"\nworkload_uid = 501\nworkload_gid = 501\n')
            path.write_text(text)
            (self.out / (name + '.toml')).write_text(text)
        self.save('tool-inputs.json', {str(p): base.sha(p) for p in
            (Path(__file__), Path(base.__file__), self.args.controller)})

    def identity(self):
        result = {}
        for n in ('meta', 'node'):
            pid = int((self.root / f'run/{n}.pid').read_text())
            proc = Path(f'/proc/{pid}')
            result[n] = {'pid': pid, 'starttick': int((proc / 'stat').read_text().rsplit(')', 1)[1].split()[19]),
                         'sha256': base.sha(proc / 'exe'),
                         'installed': base.verify_executable(proc / 'exe', self.root / ('prefix/bin/afs-' + n))}
            self.check(n + '-live-ELF', result[n]['sha256'] == getattr(self.args, 'afs_' + n + '_sha256'), result[n])
        mounted = json.loads(self.command(['findmnt', '-J', '-o', 'TARGET,SOURCE,FSTYPE,OPTIONS,ID',
                            '--mountpoint', self.root / 'mount/ownerfs']))
        self.check('OwnerFs-mount', mounted['filesystems'][0]['source'] == 'afs-ownerfs', mounted)
        result['mount'] = mounted
        self.save('running-identity.json', result)
        return result

    def run(self):
        result = {'status': 'BLOCKED', 'scope': 'single experimental managed OwnerFs workspace; not full G2.12/13',
                  'source_commit': self.args.source_commit, 'driver_sha256': base.sha(__file__),
                  'basic_payload_selected': not self.args.semantics_only,
                  'semantic_groups': self.args.semantics_groups if self.args.semantics_probe else []}
        process = None
        try:
            self.preflight()
            self.install()
            result['status'] = 'FAIL'
            self.started = True
            self.ctl('start', 'all')
            process = self.identity()
            self.check('control-mode', stat.S_IMODE((self.root / 'control').stat().st_mode) == 0o700
                       and stat.S_IMODE((self.root / 'control/control.sock').stat().st_mode) == 0o600, '0700/0600')
            self.check('initial-idle', self.native('initial', 'status')['state'] == 'Idle', 'Idle')
            self.native('unsafe-path', 'start', '../escape', error=True)
            self.native('absent-root', 'start', 'absent', error=True)
            workspace = self.root / 'mount/ownerfs/workspace'
            workspace.mkdir(mode=0o700)
            os.chown(workspace, 501, 501)
            seed = b'OwnerFs host-to-container sentinel\n' * 1024
            with (workspace / 'seed').open('wb') as f:
                f.write(seed)
                f.flush()
                os.fsync(f.fileno())
            seed_sha = hashlib.sha256(seed).hexdigest()
            start = self.native('first', 'start', 'workspace')
            self.check('final-verified', start.get('state') == 'FinalVerified', start)
            self.check('start-replay', self.native('first', 'start', 'workspace') == start, start)
            self.native('first', 'start', 'different', error=True)
            self.native('busy', 'start', 'workspace', error=True)
            state = json.loads(self.command(['/usr/local/sbin/runc', '--root', self.root / 'control/runtime-state',
                                           'state', start['container']]))
            pid = state['pid']
            ns = os.stat(f'/proc/{pid}/ns/mnt')
            source = os.stat(f'/proc/{pid}/root/workspace')
            probes = list((self.root / 'control').glob('command-*.stdout'))
            observed = next(json.loads(p.read_text()) for p in probes if 'unique_mount_id' in p.read_text())
            verify_final(observed, {'dev': source.st_dev, 'ino': source.st_ino}, {'dev': ns.st_dev, 'ino': ns.st_ino})
            physical = [str(p) for p in (self.root / 'state/node').rglob('*')
                        if p.is_dir() and (p.stat().st_dev, p.stat().st_ino) == (source.st_dev, source.st_ino)]
            self.check('real-storage-source', len(physical) == 1, physical)
            self.save('final-identity.json', {'container': state, 'observed': observed, 'storage_path': physical})
            if not self.args.semantics_only:
                shell = (f'/bin/busybox id; /bin/busybox grep -E "^(CapEff|NoNewPrivs):" /proc/self/status; '
                    f'test "$(/bin/busybox sha256sum /workspace/seed | /bin/busybox cut -d " " -f 1)" = {seed_sha}; '
                    '/bin/busybox dd if=/dev/zero of=/workspace/payload bs=65536 count=1024; '
                    'printf "end\\n" >> /workspace/payload; /bin/busybox sync -f /workspace/payload; '
                    '/bin/busybox mv /workspace/payload /workspace/renamed; /bin/busybox chmod 0600 /workspace/renamed; '
                    '/bin/busybox mkdir /workspace/empty; /bin/busybox rmdir /workspace/empty; '
                    '/bin/busybox sha256sum /workspace/renamed')
                response = self.native('work', 'exec', '--', '/bin/sh', '-ec', shell)
                self.check('executed', response.get('status') == 'Executed', response)
                expected = hashlib.sha256(bytes(64 * 2**20) + b'end\n').hexdigest()
                self.check('host-read-full-content', content_matches(workspace / 'renamed', 64 * 2**20 + 4, expected), expected)
                st = (workspace / 'renamed').stat()
                self.check('workload-owner-mode', (st.st_uid, st.st_gid, stat.S_IMODE(st.st_mode)) == (501, 501, 0o600), [st.st_uid, st.st_gid, oct(st.st_mode)])
            if self.args.semantics_probe:
                self.save('semantics-tool.json', {'path': str(self.args.semantics_probe),
                          'sha256': base.sha(self.args.semantics_probe)})
                native_path = f'/proc/{pid}/root/workspace'
                for label, primary in (('semantics-reference', native_path), ('semantics', workspace)):
                    self.command(['python3', self.args.semantics_probe, '--primary', primary,
                        '--secondary', native_path, '--controller', self.args.controller,
                        '--socket', self.root / 'control/control.sock', '--out', self.out / label,
                        '--groups', *self.args.semantics_groups],
                        timeout=120, allowed=(0, 1))
                    semantics = json.loads((self.out / label / 'result.json').read_text())
                    self.check(label, semantics.get('status') == 'PASS', semantics)
            self.check('stop', self.native('stop-first', 'stop').get('status') == 'Stopped', 'Stopped')
            self.check('idle-after-stop', self.native('after-stop', 'status')['state'] == 'Idle', 'Idle')
            self.check('container-pid-gone', not Path(f'/proc/{pid}').exists(), pid)
            self.check('runtime-empty', json.loads(self.command(['/usr/local/sbin/runc', '--root',
                       self.root / 'control/runtime-state', 'list', '--format', 'json'])) in (None, []), 'empty')
            if not self.args.semantics_only:
                self.check('host-reopen-after-stop', content_matches(workspace / 'renamed', 64 * 2**20 + 4, expected), expected)
                self.save('content.json', {'size': 64 * 2**20 + 4, 'sha256': expected, 'seed_sha256': seed_sha})
            result['status'] = 'PASS'
        except Exception as error:
            result['error'] = repr(error)
        finally:
            if self.started:
                try:
                    # Fail-closed stop: never force/lazy-delete an uncertain export.
                    if (self.root / 'control/control.sock').exists():
                        stopped = self.native('finally-stop', 'stop')
                        self.check('finally-stopped', stopped.get('status') == 'Stopped', stopped)
                    self.ctl('stop', 'all')
                    self.check('mount-removed', self.command(['findmnt', '-rn', '--mountpoint', self.root / 'mount/ownerfs'], allowed=(1,)) == '', 'absent')
                    if process:
                        for n in ('meta', 'node'):
                            self.check(n + '-gone', not Path(f'/proc/{process[n]["pid"]}').exists(), process[n]['pid'])
                    self.check('controller-artifacts-removed', not (self.root / 'control/control.sock').exists()
                               and not (self.root / 'control/controller.lock').exists(), 'absent')
                    result['cleanup'] = 'PASS'
                except Exception as error:
                    result['cleanup'] = 'FAIL'
                    result['cleanup_error'] = repr(error)
                    result['status'] = 'FAIL'
                for n in ('control', 'logs', 'run'):
                    src = self.root / n
                    if src.exists():
                        shutil.copytree(src, self.out / n, ignore=shutil.ignore_patterns('*.sock'), dirs_exist_ok=True)
            self.save('checks.json', self.checks)
            self.save('result.json', result)
        print(json.dumps(result, indent=2))
        return 0 if result['status'] == 'PASS' and result.get('cleanup') == 'PASS' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('root', 'out', 'package', 'controller', 'template-rootfs', 'rootfs-inputs'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('source-commit', 'package-sha256', 'runtime-sha256', 'afs-meta-sha256', 'afs-node-sha256'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--semantics-only', action='store_true',
                        help='run selected short semantics and necessary lifecycle identity only; omit unchanged 64MiB basic payload')
    parser.add_argument('--semantics-probe', type=Path, help='optional short mixed-path checks before normal stop')
    parser.add_argument('--semantics-groups', nargs='+',
                        choices=('locks', 'append', 'mmap_inotify', 'permissions_errno'),
                        default=['locks', 'append', 'mmap_inotify', 'permissions_errno'],
                        help='only run affected groups; omitted groups retain their original evidence')
    return Run(parser.parse_args()).run()


if __name__ == '__main__':
    sys.exit(main())

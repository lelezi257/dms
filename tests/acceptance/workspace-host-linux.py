#!/usr/bin/env python3
"""One installed Linux Node host bind case; no container or runtime dependency."""
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
import tomllib

HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


base = load('installed_smoke', HERE / 'installed-smoke-linux.py')
checks = load('runtime_checks', HERE / 'orderly-runtime-checks.py')
receipts = load('wait_receipts', HERE / 'probes/native_orderly_recovery.py')
counts = load('callback_counts', HERE / 'probes/fuse_callback_counts.py')
host = load('workspace_host', HERE / 'probes/workspace_host.py')


class Run(base.Run):
    def budget(self, label, initial=False):
        value = checks.allocations([self.root, self.args.transport])
        value['free_bytes'] = shutil.disk_usage(self.root.parent).free
        checks.verify_budget(value['allocated_bytes'], value['free_bytes'], initial)
        self.save('budget-' + label + '.json', value)
        self.check('budget-' + label, True, value)

    def inputs(self):
        expected = json.loads(self.args.inputs.read_text())
        actual = {name: checks.sha(HERE / name) for name in expected['tools']}
        self.check('exact-tool-inputs', actual == expected['tools'], actual)
        self.check('tool-map-sha', checks.sha(self.args.inputs) == self.args.inputs_sha256,
                   checks.sha(self.args.inputs))
        return expected

    def preflight(self):
        self.check('Linux-ARM64-root', platform.system() == 'Linux'
                   and platform.machine() == 'aarch64' and os.geteuid() == 0,
                   [platform.system(), platform.machine(), os.geteuid()])
        dependencies = {name: shutil.which(name) for name in (
            'bash', 'python3', 'openssl', 'findmnt', 'fusermount3', 'ldd', 'sha256sum',
            'tar', 'ss', 'curl', 'flock', 'sed', 'awk', 'grep', 'mountpoint',
            'install', 'readlink', 'setsid', 'timeout')}
        self.check('dependencies', all(dependencies.values()), dependencies)
        self.check('owned-fresh-root', self.root.parent == Path('/opt')
                   and not self.root.exists() and self.args.transport.parent == Path('/var/tmp')
                   and self.out.is_relative_to(self.args.transport), str(self.root))
        self.inputs_before = self.inputs()
        package = checks.package_inputs(self.args.package, vars(self.args))
        self.save('admitted-package.json', package[0])
        binaries = self.args.transport / 'admission-bin'
        binaries.mkdir()
        for role, blob in package[1].items():
            binary = binaries / ('afs-' + role)
            binary.write_bytes(blob)
            binary.chmod(0o700)
            libraries = self.command(['ldd', binary])
            self.check(role + '-admitted-libraries', 'not found' not in libraries, libraries)
        self.check('package-source-ELFs', True, package[0])
        self.check('FUSE-device', stat.S_ISCHR(os.stat('/dev/fuse').st_mode), '/dev/fuse')
        parent = json.loads(self.command(['findmnt', '-J', '-T', self.root.parent]))
        self.check('ext4', parent['filesystems'][0]['fstype'] == 'ext4', parent)
        for port in (22400, 22401, 22500, 22501):
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', port))
        self.check('ports-free', True, [22400, 22401, 22500, 22501])
        self.save('ram-admission.json', checks.ram_observation())
        self.protected = checks.inventory()
        self.save('protected-before.json', self.protected)
        self.old = {str(p): checks.sha(p) for p in self.args.protect_binary}
        self.save('protected-binaries-before.json', self.old)
        self.budget('admission', initial=True)
        self.save('admission.json', self.checks)

    def configuration(self, enabled):
        for role in ('meta', 'node'):
            path = self.root / ('etc/' + role + '.toml')
            text = path.read_text().replace('fs = "all"', 'fs = "ownerfs"')
            text = '\n'.join(line for line in text.splitlines()
                             if not line.startswith(('dfs_mount =', 'experimental_ownerfs_workspace_bind =')))
            if enabled and role == 'node':
                text += '\nexperimental_ownerfs_workspace_bind = true\n\n[ownerfs_workspace_bind]\nworkspace = "workspace"\n'
            path.write_text(text + '\n')
            parsed = tomllib.loads(path.read_text())
            self.check(role + '-host-switch-' + str(enabled),
                       bool(parsed.get('experimental_ownerfs_workspace_bind', False))
                       == (enabled and role == 'node')
                       and not parsed.get('experimental_native_workspace', False), parsed)
            shutil.copyfile(path, self.out / (role + ('-ON.toml' if enabled else '-OFF.toml')))

    def identity(self):
        boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
        value = {}
        for role in ('meta', 'node'):
            pid = int((self.root / ('run/' + role + '.pid')).read_text())
            proc = Path('/proc') / str(pid)
            ticks = int((proc / 'stat').read_text().rsplit(')', 1)[1].split()[19])
            binary = self.root / ('prefix/bin/afs-' + role)
            installed = base.verify_executable(proc / 'exe', binary)
            digest = checks.sha(proc / 'exe')
            self.check(role + '-actual-ELF', digest == getattr(self.args, 'afs_' + role + '_sha256'), digest)
            ns = os.stat(proc / 'ns/mnt')
            value[role] = dict(pid=pid, starttick=ticks, sha256=digest, installed=installed,
                               boot_id=boot, namespace=dict(dev=ns.st_dev, ino=ns.st_ino))
        value['fuse'] = json.loads(self.command(['findmnt', '-J', '-o',
            'TARGET,SOURCE,FSTYPE,OPTIONS,ID', '--mountpoint', self.root / 'mount/ownerfs']))
        self.check('actual-OwnerFs-FUSE', value['fuse']['filesystems'][0]['source'] == 'afs-ownerfs', value['fuse'])
        return value

    def stop(self, label, identity):
        self.ctl('stop', 'all')
        evidence = {}
        for role in ('meta', 'node'):
            matched = [p for p in (self.root / 'run').glob(role + '.lifecycle.*')
                       if (p / 'child').is_file()
                       and receipts.fields(p / 'child').get('pid') == str(identity[role]['pid'])]
            self.check(label + '-' + role + '-unique-receipt', len(matched) == 1, list(map(str, matched)))
            child, ready, exit_record = [receipts.fields(matched[0] / n) for n in ('child', 'ready', 'exit')]
            gone = all(not Path('/proc/' + child[k]).exists() for k in ('pid', 'supervisor_pid'))
            receipts.verify_receipt(exit_record, child, ready, identity[role], gone)
            evidence[role] = dict(child=child, ready=ready, exit=exit_record, both_gone=gone)
        for path in (self.root / 'mount/ownerfs/workspace', self.root / 'mount/ownerfs'):
            self.command(['findmnt', '-rn', '--mountpoint', path], allowed=(1,))
        self.started = False
        self.save(label + '-actual-waits.json', evidence)
        self.check(label + '-normal-closure', True, evidence)

    def binding(self, identity):
        target = self.root / 'mount/ownerfs/workspace'
        config = tomllib.loads((self.root / 'etc/node.toml').read_text())
        physical = list((Path(config['data_dir']) / 'ownerfs').glob('root-776f726b7370616365-e*'))
        self.check('unique-physical-Home-epoch', len(physical) == 1 and physical[0].is_dir()
                   and not physical[0].is_symlink() and int(physical[0].name.rsplit('-e', 1)[1]) > 0,
                   list(map(str, physical)))
        source = physical[0]
        cover = json.loads(self.command(['findmnt', '-J', '-o',
            'TARGET,SOURCE,FSTYPE,OPTIONS,ID', '--mountpoint', target]))['filesystems'][0]
        underlying = json.loads(self.command(['findmnt', '-J', '-T', source]))['filesystems'][0]
        local = os.stat('/proc/self/ns/mnt')
        namespace = dict(dev=local.st_dev, ino=local.st_ino)
        a, b = os.stat(source), os.stat(target)
        value = dict(source=str(source), target=str(target), cover=cover, underlying=underlying,
                     namespace=namespace, source_identity=[a.st_dev, a.st_ino],
                     target_identity=[b.st_dev, b.st_ino])
        self.check('real-host-bind', namespace == identity['node']['namespace']
                   and (a.st_dev, a.st_ino) == (b.st_dev, b.st_ino)
                   and cover['fstype'] == underlying['fstype'] == 'ext4'
                   and {'nosuid', 'nodev'} <= set(cover['options'].split(','))
                   and cover['id'] != identity['fuse']['filesystems'][0]['id'], value)
        return source, target, value

    def snapshot(self, name):
        raw = self.command(['curl', '--fail', '--silent', '--show-error', '--max-time', '10',
                            'http://127.0.0.1:22501/metrics'])
        parsed = counts.parse_counts(raw)
        self.save(name + '-counts.json', parsed)
        return parsed

    def payload(self, action, path, uid=501, digest=None):
        argv = ['python3', HERE / 'probes/workspace_host.py', action, path, str(uid), str(uid)]
        if digest is not None:
            argv += ['--expected-sha256', digest]
        value = json.loads(self.command(argv))
        self.check(action + '-' + str(path) + '-' + str(uid), value['status'] == 'PASS'
                   and value['uid'] == value['euid'] == value['gid'] == value['egid'] == uid
                   and value['mount_namespace'] == self.on['node']['namespace'], value)
        return value

    def execute(self):
        result = dict(status='BLOCKED', scope='installed fixed-Home host functional bind; 64KiB; no throughput',
                      source_commit=self.args.source_commit)
        current = None
        owned = False
        try:
            self.preflight()
            owned = True
            self.install()
            result['status'] = 'FAIL'
            self.configuration(False)
            self.started = True
            self.ctl('start', 'all')
            current = self.identity()
            self.save('bootstrap-identity.json', current)
            for name in ('workspace', 'control'):
                path = self.root / 'mount/ownerfs' / name
                path.mkdir(mode=0o700)
                os.chown(path, 501, 501)
                os.chmod(path, 0o700)
                fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
                try:
                    os.fsync(fd)
                finally:
                    os.close(fd)
            self.stop('bootstrap', current)
            self.configuration(True)
            current = None
            self.started = True
            self.ctl('start', 'all')
            self.on = current = self.identity()
            self.save('host-identity.json', current)
            source, target, binding = self.binding(current)
            self.save('binding-before.json', binding)
            positive_before = self.snapshot('fuse-before')
            control = self.root / 'mount/ownerfs/control/_host_case'
            self.payload('write', control)
            self.payload('read', control)
            self.payload('delete', control)
            positive_after = self.snapshot('fuse-after')
            self.check('positive-FUSE-window', True, host.verify_window(positive_before, positive_after, False))
            native_before = self.snapshot('native-before')
            case = target / '_host_case'
            written = self.payload('write', case)
            proof = source / '_host_case/proof'
            self.check('host-to-physical', checks.sha(proof) == written['sha256'], checks.sha(proof))
            changed = bytes(reversed(range(256))) * 256
            fd = os.open(proof, os.O_WRONLY | os.O_TRUNC)
            try:
                with os.fdopen(fd, 'wb', closefd=False) as stream:
                    stream.write(changed)
                    stream.flush()
                    os.fsync(fd)
            finally:
                os.close(fd)
            digest = hashlib.sha256(changed).hexdigest()
            self.payload('read', case, digest=digest)
            self.payload('deny', target / 'denied-sentinel', uid=502)
            self.check('denied-sentinel-absent', not (source / 'denied-sentinel').exists(), str(source))
            self.payload('delete', case)
            self.check('physical-delete', not proof.exists() and not proof.parent.exists(), str(proof))
            native_after = self.snapshot('native-after')
            self.check('native-selected-bypass', True, host.verify_window(native_before, native_after, True))
            self.check('same-live-incarnation', self.identity() == current, current)
            _, _, after = self.binding(current)
            self.check('same-binding', after == binding, after)
            self.save('binding-after.json', after)
            self.stop('host', current)
            self.inputs()
            self.budget('final')
            protected_after = checks.inventory()
            checks.verify_protected(self.protected, protected_after)
            self.save('protected-after.json', protected_after)
            self.check('protected-binaries-unchanged', self.old == {p: checks.sha(p) for p in self.old}, self.old)
            result['status'] = 'PASS'
        except Exception as error:
            result['error'] = str(error)
        finally:
            if self.started:
                try:
                    if current is None:
                        self.ctl('stop', 'all')
                    else:
                        self.stop('failure', current)
                except Exception as error:
                    result['cleanup_error'] = str(error)
                result['status'] = 'FAIL'
            if owned and (self.root / 'logs').exists():
                shutil.copytree(self.root / 'logs', self.out / 'service-logs', dirs_exist_ok=True)
            self.save('checks.json', self.checks)
            self.save('result.json', result)
        print(json.dumps(result))
        return 0 if result['status'] == 'PASS' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('root', 'out', 'transport', 'package', 'inputs'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('package-sha256', 'source-commit', 'afs-meta-sha256', 'afs-node-sha256', 'inputs-sha256'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--protect-binary', type=Path, action='append', default=[])
    return Run(parser.parse_args()).execute()


if __name__ == '__main__':
    raise SystemExit(main())

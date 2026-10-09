#!/usr/bin/env python3
"""Targeted managed-runc RootCommand refusal regression.

Cleanup/test plan before edits:
- Reuse native-workspace installation/rootfs/controller code for the real managed runc path.
- Reuse workspace-bind-root-command command receipt validation for the real Store -> RPC -> Node refusal path.
- Keep bootstrap OFF, then run runc/native ON with host workspace bind explicitly OFF.
- Prove only one narrow runtime contract: wrong command is ignored, matching command naturally closes Node and drains container/runtime/FUSE while preserving 4KiB Home data.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import time
import tomllib

HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


native = load('native_workspace', HERE / 'native-workspace-linux.py')
root_command = load('workspace_bind_root_command', HERE / 'workspace-bind-root-command-linux.py')
receipts = load('native_receipts', HERE / 'probes/native_orderly_recovery.py')
checks = load('runtime_checks', HERE / 'orderly-runtime-checks.py')

PROOF = bytes(range(256)) * 16
PASS_STATUS = 'PASS_LIMITED_RUNC_ROOT_COMMAND_REFUSAL'


def strip_table(text, table):
    marker = '\n[' + table + ']\n'
    if marker not in text:
        return text
    before, after = text.split(marker, 1)
    lines = after.splitlines()
    index = 0
    while index < len(lines) and not (lines[index].startswith('[') and lines[index].endswith(']')):
        index += 1
    suffix = '\n'.join(lines[index:])
    return before + ('\n' + suffix if suffix else '')


def managed_node_config(text, root, enabled, idle_command, identity_command):
    text = strip_table(text, 'native_workspace')
    text = strip_table(text, 'ownerfs_workspace_bind')
    lines = [line for line in text.splitlines() if not line.startswith(
        ('experimental_native_workspace =', 'experimental_ownerfs_workspace_bind ='))]
    lines.insert(0, 'experimental_ownerfs_workspace_bind = false')
    lines.insert(0, 'experimental_native_workspace = ' + ('true' if enabled else 'false'))
    text = '\n'.join(lines).rstrip() + '\n'
    if enabled:
        text += '\n[native_workspace]\n'
        text += f'control_dir = "{root}/control"\n'
        text += 'runtime = "/usr/local/sbin/runc"\n'
        text += f'rootfs = "{root}/rootfs"\n'
        text += 'workload_uid = 501\nworkload_gid = 501\n'
        text += 'idle_command = ' + json.dumps(idle_command) + '\n'
        text += 'identity_command = ' + json.dumps(identity_command) + '\n'
    return text


def verify_managed_command_delivery(receipt, events, health):
    proof = root_command.verify_command_delivery(receipt, events, health)
    wrong_index, matching_index = proof['positions']
    shutdown_indexes = [index for index, event in enumerate(events)
                        if event.get('msg') == 'node.shutdown_failed']
    if any(index < matching_index for index in shutdown_indexes):
        raise ValueError('wrong RootCommand caused shutdown before matching refusal')
    if wrong_index >= matching_index:
        raise ValueError('wrong RootCommand must be observed before matching refusal')
    return dict(proof, wrong_retained_until_matching=True, shutdown_indexes=shutdown_indexes)


def verify_content(path, expected):
    data = path.read_bytes()
    if data != expected:
        raise ValueError('preserved Home data mismatch')
    info = path.stat()
    return {'path': str(path), 'size': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
            'dev': info.st_dev, 'ino': info.st_ino, 'mode': info.st_mode,
            'uid': info.st_uid, 'gid': info.st_gid}


class Run(native.Run):
    def configure_managed(self, enabled):
        for role in ('meta', 'node'):
            path = self.root / f'etc/{role}.toml'
            text = path.read_text()
            if role == 'node':
                text = managed_node_config(text, self.root, enabled,
                                           self.args.idle_command, self.args.identity_command)
            else:
                text = strip_table(text, 'native_workspace')
                text = strip_table(text, 'ownerfs_workspace_bind')
                text = '\n'.join(line for line in text.splitlines() if not line.startswith(
                    ('experimental_native_workspace =', 'experimental_ownerfs_workspace_bind ='))) + '\n'
            path.write_text(text)
            parsed = tomllib.loads(path.read_text())
            native_enabled = bool(parsed.get('experimental_native_workspace', False))
            host_enabled = bool(parsed.get('experimental_ownerfs_workspace_bind', False))
            self.check(role + '-managed-runc-' + str(enabled),
                       native_enabled == (enabled and role == 'node') and not host_enabled,
                       parsed)
            shutil.copyfile(path, self.out / (role + ('-runc-ON.toml' if enabled else '-runc-OFF.toml')))

    def overlay(self, role, name, expected):
        destination = self.root / ('prefix/bin/afs-' + role)
        self.command(['install', '-m', '755', self.args.transport / name, destination])
        self.check(role + '-' + name + '-installed', native.base.sha(destination) == expected, expected)
        setattr(self.args, 'afs_' + role + '_sha256', expected)

    def admit_inputs(self):
        self.check('controller-sha', native.base.sha(self.args.controller) == self.args.controller_sha256,
                   native.base.sha(self.args.controller))
        self.check('rootfs-inputs-sha', native.base.sha(self.args.rootfs_inputs) == self.args.rootfs_inputs_sha256,
                   native.base.sha(self.args.rootfs_inputs))
        rootfs_inputs = json.loads(self.args.rootfs_inputs.read_text())
        rootfs_actual = {}
        for rel, expected in rootfs_inputs.items():
            path = self.args.template_rootfs / rel
            rootfs_actual[rel] = {'sha256': native.base.sha(path) if path.is_file() else None,
                                  'is_file': path.is_file(), 'is_symlink': path.is_symlink()}
            self.check('rootfs-input-' + rel, path.is_file() and not path.is_symlink()
                       and rootfs_actual[rel]['sha256'] == expected['sha256'], expected)
        expected = json.loads(self.args.inputs.read_text())
        actual = {}
        for name in expected['tools']:
            path = HERE / name
            actual[name] = native.base.sha(path)
        self.check('exact-tool-inputs', actual == expected['tools'], actual)
        self.check('tool-map-sha', native.base.sha(self.args.inputs) == self.args.inputs_sha256,
                   native.base.sha(self.args.inputs))
        self.save('admitted-inputs.json', {'rootfs': rootfs_actual, 'tools': actual,
                                           'controller_sha256': native.base.sha(self.args.controller)})

    def preflight(self):
        super().preflight()
        self.admit_inputs()
        self.check('transport-owned', self.args.transport.is_dir(), str(self.args.transport))
        self.protected = checks.inventory()
        self.save('protected-before.json', self.protected)
        self.old = {str(path): checks.sha(path) for path in self.args.protect_binary}
        self.save('protected-binaries-before.json', self.old)
        for name in ('new-node', 'new-meta', 'issuer'):
            path = self.args.transport / name
            expected = getattr(self.args, name.replace('-', '_') + '_sha256')
            self.check(name + '-input-sha', path.is_file() and native.base.sha(path) == expected, expected)
            self.check(name + '-libraries', 'not found' not in self.command(['ldd', path]), str(path))
        self.budget('admission')
        self.save('contract.json', dict(
            candidate=self.args.runtime_base,
            runtime_map=self.args.runtime_map_sha256,
            carrier_source_commit=self.args.source_commit,
            carrier_package='installed first, then overlaid with new node/meta/test issuer',
            bootstrap='OwnerFs local-file Home created with native/runc OFF and host bind OFF',
            runtime='managed runc/native ON, host bind OFF, official runc only',
            trigger='test issuer Store transaction -> production PollRootCommandBatch -> natural Node refusal',
            bytes=4096,
            expected_node_exit=1,
            expected_meta_exit=0,
            limitations=['not host-bind ON qualification', 'not production issuer',
                         'no durable revoke ACK', 'not performance or full POSIX']))

    def identity(self):
        value = super().identity()
        boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
        for role in ('meta', 'node'):
            value[role]['boot_id'] = boot
            proc = Path('/proc') / str(value[role]['pid'])
            ns = os.stat(proc / 'ns/mnt')
            value[role]['namespace'] = {'dev': ns.st_dev, 'ino': ns.st_ino}
        return value

    def budget(self, label):
        value = checks.allocations([self.root, self.args.transport])
        value['free_bytes'] = shutil.disk_usage(self.root.parent).free
        checks.verify_budget(value['allocated_bytes'], value['free_bytes'], False)
        self.save('budget-' + label + '.json', value)
        self.check('budget-' + label, True, value)

    def receipt(self, role, observed, negative=False):
        candidates = [p for p in (self.root / 'run').glob(role + '.lifecycle.*')
                      if (p / 'child').is_file()
                      and receipts.fields(p / 'child').get('pid') == str(observed['pid'])]
        self.check(role + '-unique-lifecycle', len(candidates) == 1, list(map(str, candidates)))
        child, ready, exit_record = [receipts.fields(candidates[0] / name)
                                     for name in ('child', 'ready', 'exit')]
        gone = all(not Path('/proc/' + child[key]).exists() for key in ('pid', 'supervisor_pid'))
        if negative:
            if exit_record.get('exit_code') != '1':
                raise ValueError('expected natural Node refusal exit1')
            receipts.verify_receipt(dict(exit_record, exit_code='0'), child, ready, observed, gone)
        else:
            receipts.verify_receipt(exit_record, child, ready, observed, gone)
        return {'path': str(candidates[0]), 'child': child, 'ready': ready,
                'exit': exit_record, 'both_gone': gone}

    def stop_all(self, label, identity):
        self.ctl('stop', 'all')
        evidence = {}
        for role in ('meta', 'node'):
            evidence[role] = self.receipt(role, identity[role])
        for path in (self.root / 'mount/ownerfs/workspace', self.root / 'mount/ownerfs'):
            self.command(['findmnt', '-rn', '--mountpoint', path], allowed=(1,))
        self.started = False
        self.save(label + '-actual-waits.json', evidence)
        self.check(label + '-normal-closure', True, evidence)

    def managed_binding(self, start):
        state = json.loads(self.command(['/usr/local/sbin/runc', '--root',
                                       self.root / 'control/runtime-state', 'state', start['container']]))
        pid = state['pid']
        ns = os.stat(f'/proc/{pid}/ns/mnt')
        workspace_stat = os.stat(f'/proc/{pid}/root/workspace')
        probes = sorted((self.root / 'control').glob('command-*.stdout'))
        observed = next(json.loads(path.read_text()) for path in probes if 'unique_mount_id' in path.read_text())
        native.verify_final(observed, {'dev': workspace_stat.st_dev, 'ino': workspace_stat.st_ino},
                            {'dev': ns.st_dev, 'ino': ns.st_ino})
        physical = [p for p in (self.root / 'state/node').rglob('*')
                    if p.is_dir() and (p.stat().st_dev, p.stat().st_ino)
                    == (workspace_stat.st_dev, workspace_stat.st_ino)]
        self.check('managed-physical-Home', len(physical) == 1, list(map(str, physical)))
        mountinfo = [line for line in Path(f'/proc/{pid}/mountinfo').read_text().splitlines()
                     if ' /workspace ' in line]
        self.check('managed-container-workspace-ext4', len(mountinfo) == 1 and ' - ext4 ' in mountinfo[0], mountinfo)
        value = {'container': state, 'observed': observed, 'storage_path': str(physical[0]),
                 'mountinfo': mountinfo[0]}
        self.save('managed-final-identity.json', value)
        return pid, physical[0], value

    def observation(self, node, child, container_pid, trigger):
        mounts = {line.split()[4]: int(line.split()[0])
                  for line in Path('/proc/self/mountinfo').read_text().splitlines()}
        listeners = set()
        for table in ('/proc/net/tcp', '/proc/net/tcp6'):
            for line in Path(table).read_text().splitlines()[1:]:
                columns = line.split()
                if columns[3] == '0A':
                    listeners.add(int(columns[1].rsplit(':', 1)[1], 16))
        return dict(elapsed_seconds=time.monotonic() - trigger,
                    node_alive=Path('/proc/' + str(node['pid'])).exists(),
                    supervisor_alive=Path('/proc/' + child['supervisor_pid']).exists(),
                    container_alive=Path('/proc/' + str(container_pid)).exists(),
                    fuse=str(self.root / 'mount/ownerfs') in mounts,
                    workspace_export=str(self.root / 'mount/ownerfs/workspace') in mounts,
                    node_listeners=bool(listeners & {24500, 24501}),
                    lifecycle_exit=(Path(child['lifecycle']) / 'exit').exists())

    def close_remaining(self, result):
        for role in ('node', 'meta'):
            try:
                self.ctl('stop', role)
            except Exception as error:
                result[role + '_cleanup_error'] = str(error)

    def execute(self):
        result = dict(status='BLOCKED', candidate=self.args.runtime_base,
                      runtime_map=self.args.runtime_map_sha256,
                      carrier_source_commit=self.args.source_commit,
                      scope='managed runc RootCommand wrong-ignore/matching-refusal; 4096B')
        owned = False
        try:
            self.preflight()
            owned = True
            self.install()
            result['status'] = 'FAIL'
            self.overlay('node', 'new-node', self.args.new_node_sha256)
            self.overlay('meta', 'new-meta', self.args.new_meta_sha256)

            self.configure_managed(False)
            self.started = True
            self.ctl('start', 'all')
            bootstrap = self.identity()
            workspace = self.root / 'mount/ownerfs/workspace'
            workspace.mkdir(mode=0o700)
            os.chown(workspace, 501, 501)
            fd = os.open(workspace, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
            self.stop_all('bootstrap', bootstrap)

            self.overlay('meta', 'issuer', self.args.issuer_sha256)
            self.configure_managed(True)
            trigger_path = self.out / 'trigger.json'
            receipt_path = self.out / 'issuer-receipt.json'
            self.started = True
            self.command(['env', 'AFS_TEST_ROOT_COMMAND_TRIGGER=' + str(trigger_path),
                'AFS_TEST_ROOT_COMMAND_RECEIPT=' + str(receipt_path),
                'AFS_TEST_ROOT_COMMAND_WORKSPACE=workspace',
                self.root / 'prefix/bin/afs-processctl', '--prefix', self.root / 'prefix',
                '--config-dir', self.root / 'etc', '--run-dir', self.root / 'run',
                '--log-dir', self.root / 'logs', 'start', 'meta'])
            self.ctl('start', 'node')
            current = self.identity()
            self.save('runc-root-command-identity.json', current)
            lifecycle = [p for p in (self.root / 'run').glob('node.lifecycle.*')
                         if (p / 'child').is_file()
                         and receipts.fields(p / 'child').get('pid') == str(current['node']['pid'])]
            self.check('node-live-lifecycle', len(lifecycle) == 1, list(map(str, lifecycle)))
            child = receipts.fields(lifecycle[0] / 'child')
            self.save('node-live-child.json', child)
            self.check('initial-idle', self.native('initial', 'status')['state'] == 'Idle', 'Idle')
            start = self.native('first', 'start', 'workspace')
            self.check('final-verified-before-command', start.get('state') == 'FinalVerified', start)
            container_pid, source, binding = self.managed_binding(start)
            proof = source / 'runc-root-command-proof'
            with proof.open('xb') as stream:
                stream.write(PROOF)
                stream.flush()
                os.fsync(stream.fileno())
            proof_before = verify_content(proof, PROOF)
            self.save('physical-before.json', proof_before)
            digest = hashlib.sha256(PROOF).hexdigest()
            response = self.native('container-read-before-command', 'exec', '--', '/bin/sh', '-ec',
                f'test "$(/bin/busybox sha256sum /workspace/runc-root-command-proof | /bin/busybox cut -d " " -f 1)" = {digest}')
            self.check('container-read-before-command', response.get('status') == 'Executed', response)
            active = self.native('active-before-command', 'status')
            self.check('active-before-command', active.get('state') == 'FinalVerified'
                       and Path(f'/proc/{container_pid}').exists(), active)
            before_health = root_command.Run.health(self, 24501)
            self.save('node-health-before-command.json', before_health)

            trigger = time.monotonic()
            checks.publish(trigger_path, dict(command_id='runc-root-command-r1',
                                              mismatch_generation=True))
            observations = []
            while time.monotonic() - trigger < 35:
                row = self.observation(current['node'], child, container_pid, trigger)
                observations.append(row)
                self.save('closure-observations.json', observations)
                if not any(row[key] for key in ('node_alive', 'supervisor_alive', 'container_alive',
                                                'fuse', 'workspace_export')):
                    break
                time.sleep(.05)
            else:
                raise ValueError('matching RootCommand refusal did not close managed runc workspace')
            self.check('normal-drain-order', observations[-1]['elapsed_seconds'] < 35
                       and not any(observations[-1][key] for key in ('node_alive', 'supervisor_alive',
                                                                      'container_alive', 'fuse', 'workspace_export'))
                       and not any(row['workspace_export'] and not row['fuse'] for row in observations),
                       observations[-1])
            self.save('node-natural-wait1.json', self.receipt('node', current['node'], negative=True))
            issuer = json.loads(receipt_path.read_text())
            events = [json.loads(line) for line in (self.root / 'logs/node.log').read_text().splitlines()
                      if line.startswith('{')]
            self.check('actual-managed-command-delivery', True,
                       verify_managed_command_delivery(issuer, events, before_health))
            self.check('container-pid-gone-after-refusal', not Path(f'/proc/{container_pid}').exists(), container_pid)
            self.check('runtime-empty-after-refusal', json.loads(self.command(['/usr/local/sbin/runc', '--root',
                       self.root / 'control/runtime-state', 'list', '--format', 'json'])) in (None, []), 'empty')
            self.check('control-artifacts-removed', not (self.root / 'control/control.sock').exists()
                       and not (self.root / 'control/controller.lock').exists(), 'absent')
            self.command(['findmnt', '-rn', '--mountpoint', self.root / 'mount/ownerfs/workspace'], allowed=(1,))
            self.command(['findmnt', '-rn', '--mountpoint', self.root / 'mount/ownerfs'], allowed=(1,))
            self.ctl('stop', 'meta')
            self.save('meta-normal-wait0.json', self.receipt('meta', current['meta']))
            self.started = False
            self.check('physical-proof-preserved', verify_content(proof, PROOF) == proof_before, proof_before)
            self.save('physical-after.json', verify_content(proof, PROOF))
            self.budget('final')
            protected_after = checks.inventory()
            checks.verify_protected(self.protected, protected_after)
            self.save('protected-after.json', protected_after)
            self.check('protected-binaries-unchanged', self.old == {p: checks.sha(p) for p in self.old}, self.old)
            result['status'] = PASS_STATUS
        except Exception as error:
            result['error'] = str(error)
        finally:
            if self.started:
                self.close_remaining(result)
                result['status'] = 'FAIL'
            if owned:
                for name in ('control', 'logs', 'run'):
                    src = self.root / name
                    if src.exists():
                        try:
                            self.archive_artifacts(name, self.out / name)
                        except Exception as error:
                            result.setdefault('archive_errors', {})[name] = repr(error)
                            result['status'] = 'FAIL'
            self.save('checks.json', self.checks)
            self.save('result.json', result)
        print(json.dumps(result, indent=2))
        return 0 if result['status'] == PASS_STATUS else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('root', 'out', 'transport', 'package', 'controller', 'template-rootfs', 'rootfs-inputs', 'inputs'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('source-commit', 'runtime-base', 'package-sha256', 'runtime-sha256', 'afs-meta-sha256', 'afs-node-sha256',
                 'new-node-sha256', 'new-meta-sha256', 'issuer-sha256', 'runtime-map-sha256',
                 'rootfs-inputs-sha256', 'controller-sha256', 'inputs-sha256'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--idle-command', nargs='+', default=['/afs-workspace-probe', 'idle'],
                        help='test-provided OCI init argv; no product default')
    parser.add_argument('--identity-command', nargs='+', default=['/afs-workspace-probe', 'identity'],
                        help='test-provided final-view JSON observer argv')
    parser.add_argument('--protect-binary', type=Path, action='append', default=[])
    # This entry selects one control case; sibling native-driver modes are not selected.
    parser.set_defaults(source_rejection_only=False, semantics_only=False, semantics_probe=None,
                        control_capacity_only=False, orderly_recovery_only=False,
                        node_shutdown_only=False, fuse_counters_only=False, fuse_counter_case='data')
    return Run(parser.parse_args()).execute()


if __name__ == '__main__':
    raise SystemExit(main())

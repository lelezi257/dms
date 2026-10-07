#!/usr/bin/env python3
"""Real same-node session replacement: heartbeat failure closes an owned host bind."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import time
import tomllib

from importlib.util import module_from_spec, spec_from_file_location

HERE = Path(__file__).resolve().parent
spec = spec_from_file_location('host_bind', HERE / 'workspace-host-linux.py')
host = module_from_spec(spec)
spec.loader.exec_module(host)


def replacement_config(text, root):
    """Keep the trusted node identity; isolate every runtime resource and disable bind."""
    text = text.split('\n[ownerfs_workspace_bind]')[0]
    updates = {
        'fs': 'dfs', 'grpc_listen': '127.0.0.1:22600',
        'rest_listen': '127.0.0.1:22601',
        'advertise_endpoint': 'https://127.0.0.1:22600',
        'data_dir': str(root / 'state'), 'uds_path': str(root / 'run/node.sock'),
    }
    for key, value in updates.items():
        text, count = re.subn(r'^' + key + r' = .*$', key + ' = ' + json.dumps(value), text, flags=re.M)
        if count != 1:
            raise ValueError('missing/duplicate replacement configuration: ' + key)
    text = '\n'.join(line for line in text.splitlines() if not line.startswith(
        ('ownerfs_mount =', 'dfs_mount =', 'experimental_ownerfs_workspace_bind =', 'experimental_native_workspace =')))
    return (text + '\ndfs_mount = ' + json.dumps(str(root / 'mount/dfs'))
            + '\nexperimental_ownerfs_workspace_bind = false\nexperimental_native_workspace = false\n')


def verify_transition(before, after):
    first, second = [value['checks']['node_registration'] for value in (before, after)]
    if (before['id'] != after['id'] or before['status'] != 'ready' or after['status'] != 'ready'
            or first['ready'] is not True or second['ready'] is not True
            or not first['session_id'] or not second['session_id']
            or first['session_id'] == second['session_id']
            or type(first['lease_epoch']) is not int or type(second['lease_epoch']) is not int
            or not 0 < first['lease_epoch'] < second['lease_epoch']):
        raise ValueError('same authenticated node must expose a fresh higher-epoch session')


def verify_authority_exit(receipt, child, ready, observed, gone):
    # Preserve the real failure result, while reusing the exact incarnation verifier.
    if receipt.get('exit_code') != '1':
        raise ValueError('expected authority error exit1; success, watchdog and signals are not proof')
    host.receipts.verify_receipt(dict(receipt, exit_code='0'), child, ready, observed, gone)


def verify_authority_log(text):
    expected = ('0x04030002 PermissionDenied: Node registration epoch changed; '
                'the running authority must stop')
    events = [json.loads(line) for line in text.splitlines() if line.startswith('{')]
    if not any(event.get('msg') == 'node.shutdown_failed' and event.get('error') == expected
               for event in events):
        raise ValueError('exact structured authority error not preserved')


def verify_replacement_exit(receipt, child, ready, observed, gone, config):
    keys = ('pid', 'exe', 'config', 'start_ticks', 'boot_id', 'lifecycle', 'supervisor_pid')
    if (any(not child.get(key) or receipt.get(key) != child[key] for key in keys)
            or receipt.get('exit_code') != '0' or not gone
            or child['pid'] != str(observed['pid'])
            or child['start_ticks'] != str(observed['starttick'])
            or child['exe'] != observed['installed']['path']
            or child['boot_id'] != observed['boot_id']
            or child['config'] != str(config)
            or ready != {'supervisor_pid': child['supervisor_pid']}):
        raise ValueError('replacement exact incarnation actual wait0 missing')


class Run(host.Run):
    def inputs(self):
        # The inherited driver resolves its own file; bind this case to this entry.
        expected = json.loads(self.args.inputs.read_text())
        actual = {name: host.checks.sha(HERE / name) for name in expected['tools']}
        self.check('exact-tool-inputs', actual == expected['tools'], actual)
        self.check('tool-map-sha', host.checks.sha(self.args.inputs) == self.args.inputs_sha256,
                   host.checks.sha(self.args.inputs))
        return expected

    def preflight(self):
        super().preflight()
        for port in (22600, 22601):
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', port))
        self.check('replacement-ports-free', True, [22600, 22601])
        self.save('contract.json', dict(candidate=self.args.source_commit, bytes=4096,
            meta='local-file', bind='host workspace default OFF; case explicit ON',
            trigger='public authenticated RegisterNode by second Node; no original signal',
            observation_budget_seconds=35, expected_original_exit=1,
            limitations=['not immediate FD revocation', 'not runtime root-command watch',
                         'not durable revoke/ACK', 'not full bind or performance acceptance']))

    def health(self, port):
        return json.loads(self.command(['curl', '--noproxy', '*', '--fail', '--silent',
            '--show-error', '--max-time', '5', f'http://127.0.0.1:{port}/health']))

    def replacement_ctl(self, *action):
        return self.command([self.root / 'prefix/bin/afs-processctl', '--prefix', self.root / 'prefix',
            '--config-dir', self.replacement / 'etc', '--run-dir', self.replacement / 'run',
            '--log-dir', self.replacement / 'logs', *action])

    def receipt(self, role, directory, observed, negative=False):
        matching = [p for p in directory.glob(role + '.lifecycle.*') if (p / 'child').is_file()
                    and host.receipts.fields(p / 'child').get('pid') == str(observed['pid'])]
        if len(matching) != 1:
            raise ValueError('missing unique captured lifecycle: ' + role)
        path = matching[0]
        child, ready, exit_record = [host.receipts.fields(path / name) for name in ('child', 'ready', 'exit')]
        gone = all(not Path('/proc/' + child[key]).exists() for key in ('pid', 'supervisor_pid'))
        if negative:
            verify_authority_exit(exit_record, child, ready, observed, gone)
        elif directory.parent == self.replacement:
            verify_replacement_exit(exit_record, child, ready, observed, gone,
                                    self.replacement / 'etc/node.toml')
        else:
            host.receipts.verify_receipt(exit_record, child, ready, observed, gone)
        return dict(path=str(path), child=child, ready=ready, exit=exit_record, both_gone=gone)

    def close_remaining(self, result):
        # A failed Node receipt must not prevent closing the still-owned Meta.
        for role in ('node', 'meta'):
            try:
                self.ctl('stop', role)
            except Exception as error:
                result[role + '_cleanup_error'] = str(error)

    def execute(self):
        result = dict(status='BLOCKED', source_commit=self.args.source_commit,
                      scope='one real heartbeat-driven authority-error bind closure; 4096B')
        self.replacement = self.root / 'replacement'
        replacement_started = False
        owned = False
        try:
            self.preflight()
            owned = True
            self.install()
            result['status'] = 'FAIL'
            self.configuration(False)
            self.started = True
            self.ctl('start', 'all')
            bootstrap = self.identity()
            workspace = self.root / 'mount/ownerfs/workspace'
            workspace.mkdir(mode=0o700)
            os.chown(workspace, 501, 501)
            descriptor = os.open(workspace, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            self.stop('bootstrap', bootstrap)
            self.configuration(True)
            self.started = True
            self.ctl('start', 'all')
            original = self.identity()
            self.save('original-identity.json', original)
            lifecycle = [p for p in (self.root / 'run').glob('node.lifecycle.*')
                         if (p / 'child').is_file()
                         and host.receipts.fields(p / 'child').get('pid') == str(original['node']['pid'])]
            self.check('original-unique-live-lifecycle', len(lifecycle) == 1, list(map(str, lifecycle)))
            original_child = host.receipts.fields(lifecycle[0] / 'child')
            self.save('original-live-child.json', original_child)
            source, target, binding = self.binding(original)
            self.save('binding-before.json', binding)
            payload = bytes(range(256)) * 16
            proof = target / 'epoch-proof'
            with proof.open('xb') as stream:
                stream.write(payload)
                stream.flush()
                os.fsync(stream.fileno())
            self.check('physical-proof', (source / proof.name).read_bytes() == payload, hashlib.sha256(payload).hexdigest())
            before = self.health(22501)
            self.save('original-health.json', before)
            for name in ('etc', 'run', 'logs', 'state', 'mount/dfs'):
                (self.replacement / name).mkdir(parents=True)
            text = replacement_config((self.root / 'etc/node.toml').read_text(), self.replacement)
            (self.replacement / 'etc/node.toml').write_text(text)
            config = tomllib.loads(text)
            old = tomllib.loads((self.root / 'etc/node.toml').read_text())
            self.check('same-trusted-identity', all(config[key] == old[key] for key in (
                'id', 'tls_ca_certificate', 'tls_identity_certificate', 'tls_identity_private_key', 'trusted_node_certs')), config['id'])
            effective = json.loads(self.command([self.root / 'prefix/bin/afs-node', '--config',
                self.replacement / 'etc/node.toml', '--print-config']))
            self.check('replacement-isolated-OFF', not effective['experimental_native_workspace']
                and not effective['experimental_ownerfs_workspace_bind']
                and not effective['ownerfs'] and effective['dfs']
                and effective['data_dir'] == str(self.replacement / 'state')
                and effective['ownerfs_mount'] is None
                and effective['dfs_mount'] == str(self.replacement / 'mount/dfs'), effective)
            trigger = time.monotonic()
            replacement_started = True
            self.replacement_ctl('start', 'node')
            after = self.health(22601)
            self.save('replacement-health.json', after)
            verify_transition(before, after)
            self.check('public-session-transition', True, dict(before=before, after=after))
            pid = int((self.replacement / 'run/node.pid').read_text())
            proc = Path('/proc') / str(pid)
            replacement_identity = dict(pid=pid,
                starttick=int((proc / 'stat').read_text().rsplit(')', 1)[1].split()[19]),
                boot_id=original['node']['boot_id'],
                installed=host.base.verify_executable(proc / 'exe', self.root / 'prefix/bin/afs-node'),
                sha256=host.checks.sha(proc / 'exe'))
            self.check('replacement-actual-ELF', replacement_identity['sha256'] == self.args.afs_node_sha256,
                       replacement_identity)
            self.save('replacement-identity.json', replacement_identity)
            # Stop the trigger process before its heartbeat can create competing renewals.
            self.replacement_ctl('stop', 'node')
            replacement_started = False
            self.save('replacement-actual-wait.json', self.receipt('node', self.replacement / 'run', replacement_identity))
            self.command(['findmnt', '-rn', '--mountpoint', self.replacement / 'mount/dfs'], allowed=(1,))
            observations = []
            original_mount = str(self.root / 'mount/ownerfs')
            bind_mount = str(target)
            while time.monotonic() - trigger < 35:
                mountinfo = Path('/proc/self/mountinfo').read_text().splitlines()
                mounted = {line.split()[4] for line in mountinfo}
                row = dict(elapsed_seconds=time.monotonic() - trigger,
                    node_alive=Path('/proc/' + str(original['node']['pid'])).exists(),
                    supervisor_alive=Path('/proc/' + original_child['supervisor_pid']).exists(),
                    fuse=original_mount in mounted, bind=bind_mount in mounted)
                observations.append(row)
                self.save('closure-observations.json', observations)
                if not any(row[key] for key in ('node_alive', 'supervisor_alive', 'fuse', 'bind')):
                    break
                time.sleep(.05)
            self.check('bounded-natural-closure', observations[-1]['elapsed_seconds'] < 35
                and not any(observations[-1][key] for key in ('node_alive', 'supervisor_alive', 'fuse', 'bind'))
                and not any(row['bind'] and not row['fuse'] for row in observations), observations[-1])
            actual = self.receipt('node', self.root / 'run', original['node'], negative=True)
            self.save('original-authority-exit.json', actual)
            logs = (self.root / 'logs/node.log').read_text()
            verify_authority_log(logs)
            self.check('authority-error-preserved', True, logs)
            self.ctl('stop', 'meta')
            self.save('meta-actual-wait.json', self.receipt('meta', self.root / 'run', original['meta']))
            self.started = False
            self.check('physical-proof-preserved', (source / proof.name).read_bytes() == payload, hashlib.sha256(payload).hexdigest())
            self.inputs()
            self.budget('final')
            protected = host.checks.inventory()
            host.checks.verify_protected(self.protected, protected)
            self.save('protected-after.json', protected)
            self.check('protected-binaries-unchanged', self.old == {p: host.checks.sha(p) for p in self.old}, self.old)
            result['status'] = 'PASS'
        except Exception as error:
            result['error'] = str(error)
        finally:
            if replacement_started:
                try:
                    self.replacement_ctl('stop', 'node')
                except Exception as error:
                    result['replacement_cleanup_error'] = str(error)
            if self.started:
                self.close_remaining(result)
            if owned:
                for directory in (self.root / 'logs', self.replacement / 'logs'):
                    if directory.exists():
                        shutil.copytree(directory, self.out / ('replacement-logs' if directory.parent == self.replacement else 'service-logs'))
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

#!/usr/bin/env python3
"""Linux host bind command receipt/refusal, using an explicitly test-only Meta issuer."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import time

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('bind_epoch', HERE / 'workspace-bind-epoch-linux.py')
epoch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(epoch)
host = epoch.host
PROOF = bytes(range(256)) * 16
REFUSAL = 'workspace bind Home admission rejected by Meta RootCommand'


def verify_command_delivery(receipt, events, health):
    original = receipt['home_grant']
    issued = receipt['issued']
    session = receipt['home_session']
    registration = health['checks']['node_registration']
    if (len(issued) != 2 or health['status'] != 'ready' or registration['ready'] is not True
            or health['id'] != original['home_node_id']
            or session['node_id'] != original['home_node_id']
            or session['session_id'] != original['home_session_id']
            or registration['session_id'] != session['session_id']
            or not 0 < session['lease_epoch'] == registration['lease_epoch']
            or any(original[key] != receipt['original_root'][key]
                   for key in ('root_id', 'root_epoch', 'home_node_id', 'home_session_id'))
            or any(row['outcome']['status'] != 'committed' for row in issued)):
        raise ValueError('two exact committed test commands/current Node epoch required')
    wrong, matching = [row['command'] for row in issued]
    if (not wrong['command_id'] or not matching['command_id']
            or wrong['command_id'] == matching['command_id']
            or not 0 < issued[0]['outcome']['revision'] < issued[1]['outcome']['revision']
            or wrong['old_access_generation'] != original['access_generation'] + 1
            or matching['old_access_generation'] != original['access_generation']
            or any(command[key] != original[key] for command in (wrong, matching)
                   for key in ('root_id', 'root_epoch', 'home_node_id', 'home_session_id'))
            or any(command['command_type'] != 'RevokeAccess' for command in (wrong, matching))):
        raise ValueError('test issuance must be exact Home tuple, wrong then matching generation')
    expected = [('ownerfs.workspace_bind_root_command_ignored', wrong),
                ('ownerfs.workspace_bind_root_command_rejected', matching)]
    positions = []
    for message, command in expected:
        found = [(index, event) for index, event in enumerate(events)
                 if event.get('msg') == message and event.get('command_id') == command['command_id']]
        if len(found) != 1:
            raise ValueError('missing/duplicate actual production command event: ' + message)
        index, event = found[0]
        row = issued[len(positions)]
        if (event['root_id'] != command['root_id']
                or event['revision'] != row['outcome']['revision']):
            raise ValueError('event does not match committed command/revision')
        positions.append(index)
    if positions != sorted(positions) or not any(
            event.get('msg') == 'node.shutdown_failed' and event.get('error', '').endswith(REFUSAL)
            for event in events):
        raise ValueError('matching refusal must be the natural Node shutdown cause')
    matching_event = events[positions[1]]
    if (matching_event['root_epoch'] != matching['root_epoch']
            or matching_event['access_generation'] != matching['old_access_generation']):
        raise ValueError('refused tuple differs from committed matching tuple')
    return dict(wrong_ignored=True, matching_rejected=True, no_ack_claim=True,
                command_ids=[wrong['command_id'], matching['command_id']], positions=positions)


class Run(epoch.Run):
    def preflight(self):
        host.Run.preflight(self)
        for name in ('new-node', 'new-meta', 'issuer'):
            path = self.args.transport / name
            expected = getattr(self.args, name.replace('-', '_') + '_sha256')
            self.check(name + '-input-sha', path.is_file() and host.checks.sha(path) == expected, expected)
            self.check(name + '-libraries', 'not found' not in self.command(['ldd', path]), str(path))
        self.save('contract.json', dict(base=self.args.runtime_base, compiler_map=self.args.runtime_map_sha256,
            carrier_source=self.args.source_commit, bytes=4096, meta='local-file real Store/RPC; test-only issuer',
            trigger='test fixture Store transaction -> real mTLS PollRootCommandBatch -> production Node',
            bind='host ON; default OFF unchanged', expected_node_exit=1, expected_meta_exit=0,
            control_error_policy='host ON stops on transport/protocol/compaction/unsupported; OFF unchanged',
            limitations=['not production admin issuer', 'no durable cursor/revoke ACK',
                         'no immediate FD revocation', 'not full bind or performance']))

    def overlay(self, role, name, expected):
        destination = self.root / ('prefix/bin/afs-' + role)
        self.command(['install', '-m', '755', self.args.transport / name, destination])
        self.check(role + '-' + name + '-installed', host.checks.sha(destination) == expected, expected)
        setattr(self.args, 'afs_' + role + '_sha256', expected)

    def execute(self):
        result = dict(status='BLOCKED', scope='production command receipt/refusal and normal host bind close',
                      runtime_base=self.args.runtime_base, compiler_map=self.args.runtime_map_sha256)
        self.replacement = self.root / 'unused-trigger-placeholder'
        owned = False
        try:
            self.preflight()
            owned = True
            self.install()  # Fixed carrier scripts/TLS/config; no current-package claim.
            result['status'] = 'FAIL'
            self.overlay('node', 'new-node', self.args.new_node_sha256)
            self.overlay('meta', 'new-meta', self.args.new_meta_sha256)
            self.configuration(False)
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
            self.stop('bootstrap', bootstrap)
            self.overlay('meta', 'issuer', self.args.issuer_sha256)
            self.configuration(True)
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
            original = self.identity()
            self.save('original-identity.json', original)
            lifecycle = [p for p in (self.root / 'run').glob('node.lifecycle.*')
                         if (p / 'child').is_file()
                         and host.receipts.fields(p / 'child').get('pid') == str(original['node']['pid'])]
            self.check('original-unique-live-lifecycle', len(lifecycle) == 1, list(map(str, lifecycle)))
            child = host.receipts.fields(lifecycle[0] / 'child')
            self.save('original-live-child.json', child)
            source, target, binding = self.binding(original)
            self.save('binding-before.json', binding)
            proof = target / 'root-command-proof'
            with proof.open('xb') as stream:
                stream.write(PROOF)
                stream.flush()
                os.fsync(stream.fileno())
            info = (source / proof.name).stat()
            physical = dict(path=str(source / proof.name), dev=info.st_dev, ino=info.st_ino,
                            bytes=info.st_size, sha256=host.checks.sha(source / proof.name))
            self.check('full-physical-proof', (source / proof.name).read_bytes() == PROOF, physical)
            self.save('physical-before.json', physical)
            before = self.health(22501)
            self.save('node-health-before.json', before)
            started = time.monotonic()
            host.checks.publish(trigger_path, dict(command_id='bind-root-command-r1', mismatch_generation=True))
            observations = []
            while time.monotonic() - started < 35:
                row = self.observation(original, child, target, started)
                observations.append(row)
                self.save('observations.json', observations)
                if not any(row[key] for key in ('node_alive', 'supervisor_alive', 'fuse', 'bind')):
                    break
                time.sleep(.02)
            else:
                raise ValueError('command refusal did not naturally close owned Node/bind/FUSE')
            self.check('normal-bind-before-FUSE-closure', not any(
                row['bind'] and not row['fuse'] for row in observations), observations[-1])
            node_wait = self.receipt('node', self.root / 'run', original['node'], True)
            self.save('node-natural-wait1.json', node_wait)
            issuer = json.loads(receipt_path.read_text())
            events = [json.loads(line) for line in (self.root / 'logs/node.log').read_text().splitlines()
                      if line.startswith('{')]
            self.check('actual-command-delivery', True, verify_command_delivery(issuer, events, before))
            self.ctl('stop', 'meta')
            self.save('meta-normal-wait0.json', self.receipt('meta', self.root / 'run', original['meta']))
            self.started = False
            info = (source / proof.name).stat()
            after = dict(path=str(source / proof.name), dev=info.st_dev, ino=info.st_ino,
                         bytes=info.st_size, sha256=host.checks.sha(source / proof.name))
            self.check('original-physical-data-preserved', after == physical and
                       (source / proof.name).read_bytes() == PROOF, after)
            self.save('physical-after.json', after)
            self.inputs()
            self.budget('final')
            protected_after = host.checks.inventory()
            host.checks.verify_protected(self.protected, protected_after)
            self.save('protected-after.json', protected_after)
            self.check('protected-binaries-unchanged', self.old == {p: host.checks.sha(p) for p in self.old}, self.old)
            result['status'] = 'PASS_LIMITED_COMMAND_RECEIPT_REFUSAL'
        except Exception as error:
            result['error'] = str(error)
        finally:
            if self.started:
                self.close_remaining(result)
                result['status'] = 'FAIL'
            if owned and (self.root / 'logs').exists():
                shutil.copytree(self.root / 'logs', self.out / 'service-logs', dirs_exist_ok=True)
            self.save('checks.json', self.checks)
            self.save('result.json', result)
        print(json.dumps(result))
        return 0 if result['status'] == 'PASS_LIMITED_COMMAND_RECEIPT_REFUSAL' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('root', 'out', 'transport', 'package', 'inputs'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('package-sha256', 'source-commit', 'afs-meta-sha256', 'afs-node-sha256',
                 'inputs-sha256', 'new-node-sha256', 'new-meta-sha256', 'issuer-sha256',
                 'runtime-base', 'runtime-map-sha256'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--protect-binary', type=Path, action='append', default=[])
    return Run(parser.parse_args()).execute()


if __name__ == '__main__':
    raise SystemExit(main())

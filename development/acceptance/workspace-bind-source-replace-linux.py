#!/usr/bin/env python3
"""Installed host bind: atomic physical Home replacement must naturally close Node."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import time

from importlib.util import module_from_spec, spec_from_file_location

HERE = Path(__file__).resolve().parent
spec = spec_from_file_location('bind_epoch', HERE / 'workspace-bind-epoch-linux.py')
epoch = module_from_spec(spec)
spec.loader.exec_module(epoch)
host = epoch.host
PROOF = bytes(range(256)) * 16
SOURCE_ERROR = '0x020b0001 FailedPrecondition: native Home source was replaced'


def directory_identity(path):
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or path.resolve() != path:
        raise ValueError('physical directory must not be a symlink')
    return dict(device=info.st_dev, inode=info.st_ino, mode=stat.S_IMODE(info.st_mode),
                uid=info.st_uid, gid=info.st_gid)


def exchange(first, second):
    """Exchange two existing owned directories without an absent source-path interval."""
    libc = ctypes.CDLL(None, use_errno=True)
    function = libc.renameat2
    function.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    function.restype = ctypes.c_int
    if function(-100, os.fsencode(first), -100, os.fsencode(second), 2) != 0:
        code = ctypes.get_errno()
        raise OSError(code, os.strerror(code))


def verify_fixture_source(root, source, expected):
    if (root.parent != Path('/opt') or not source.is_relative_to(root / 'state')
            or not re.fullmatch(r'root-776f726b7370616365-e[1-9][0-9]*', source.name)
            or directory_identity(source) != expected):
        raise ValueError('refuse source outside owned fixture or with changed identity')


def verify_preserved(source, retained, old_identity, new_identity, proof):
    if (directory_identity(retained) != old_identity
            or directory_identity(source) != new_identity or any(source.iterdir())
            or (retained / proof).is_symlink() or (retained / proof).read_bytes() != PROOF):
        raise ValueError('old data/identity or empty replacement changed')


def restore_source(source, retained, old_identity, new_identity, proof):
    # Caller first proves all owned services and mounts closed. Refuse foreign data.
    verify_preserved(source, retained, old_identity, new_identity, proof)
    exchange(source, retained)
    if directory_identity(source) != old_identity or directory_identity(retained) != new_identity:
        raise ValueError('restoration identities mismatched; keep both directories')
    retained.rmdir()  # Only the checked, still-empty new directory; never recursive deletion.
    if (source / proof).read_bytes() != PROOF:
        raise ValueError('restored proof changed')


def verify_source_error(text):
    events = [json.loads(line) for line in text.splitlines() if line.startswith('{')]
    if not any(row.get('msg') == 'node.shutdown_failed' and row.get('error') == SOURCE_ERROR
               for row in events):
        raise ValueError('exact structured source replacement error missing')


class Run(epoch.Run):
    def preflight(self):
        host.Run.preflight(self)
        first, second = [self.args.transport / name for name in ('exchange-probe-a', 'exchange-probe-b')]
        first.mkdir(); second.mkdir()
        original = directory_identity(first), directory_identity(second)
        exchange(first, second)
        self.check('ext4-atomic-exchange', (directory_identity(second), directory_identity(first)) == original,
                   original)
        first.rmdir(); second.rmdir()
        self.save('contract.json', dict(candidate=self.args.source_commit, bytes=4096,
            meta='local-file', bind='default OFF; explicit host ON',
            trigger='atomic physical Home source directory exchange; no original signal',
            observation_budget_seconds=35, expected_original_exit=1, expected_error=SOURCE_ERROR,
            limitations=['not Meta RootCommand watch/ACK', 'not immediate FD revocation',
                         'not active runc cancellation', 'not full bind or performance acceptance']))

    def execute(self):
        result = dict(status='BLOCKED', source_commit=self.args.source_commit,
                      scope='physical Home identity polling and natural host-bind closure; 4096B')
        self.replacement = self.root / 'unused-session-trigger'  # Inherited receipt helper only.
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
            paths = [p for p in (self.root / 'run').glob('node.lifecycle.*')
                     if (p / 'child').is_file()
                     and host.receipts.fields(p / 'child').get('pid') == str(original['node']['pid'])]
            self.check('original-unique-live-lifecycle', len(paths) == 1, list(map(str, paths)))
            child = host.receipts.fields(paths[0] / 'child')
            self.save('original-live-child.json', child)
            source, target, binding = self.binding(original)
            self.save('binding-before.json', binding)
            proof = 'source-identity-proof'
            with (target / proof).open('xb') as stream:
                stream.write(PROOF); stream.flush(); os.fsync(stream.fileno())
            self.check('physical-proof', (source / proof).read_bytes() == PROOF,
                       hashlib.sha256(PROOF).hexdigest())
            old = directory_identity(source)
            verify_fixture_source(self.root, source, old)
            retained = source.with_name(source.name + '-source-replacement')
            retained.mkdir(mode=old['mode'])
            os.chown(retained, old['uid'], old['gid'])
            new = directory_identity(retained)
            self.save('source-before.json', dict(source=str(source), retained=str(retained),
                old=old, new=new, proof=proof, sha256=hashlib.sha256(PROOF).hexdigest()))
            trigger = time.monotonic()
            exchange(source, retained)
            verify_preserved(source, retained, old, new, proof)
            self.save('source-replaced.json', dict(source=directory_identity(source),
                retained=directory_identity(retained), replacement_empty=True, proof_preserved=True))
            observations = []
            while time.monotonic() - trigger < 35:
                row = self.observation(original, child, target, trigger)
                observations.append(row)
                self.save('closure-observations.json', observations)
                if not any(row[key] for key in ('node_alive', 'supervisor_alive', 'fuse', 'bind')):
                    break
                time.sleep(.05)
            self.check('bounded-natural-closure', bool(observations)
                and observations[-1]['elapsed_seconds'] < 35
                and not any(observations[-1][key] for key in ('node_alive', 'supervisor_alive', 'fuse', 'bind'))
                and not any(row['bind'] and not row['fuse'] for row in observations), observations[-1])
            self.save('original-authority-exit.json', self.receipt('node', self.root / 'run', original['node'], negative=True))
            logs = (self.root / 'logs/node.log').read_text()
            verify_source_error(logs)
            self.check('source-error-preserved', True, SOURCE_ERROR)
            self.ctl('stop', 'meta')
            self.save('meta-actual-wait.json', self.receipt('meta', self.root / 'run', original['meta']))
            self.started = False
            restore_source(source, retained, old, new, proof)
            self.save('source-restored.json', dict(identity=directory_identity(source),
                sha256=host.checks.sha(source / proof), replacement_absent=not retained.exists(),
                node_and_meta_closed_before_restore=True))
            self.check('original-data-and-permissions-restored', directory_identity(source) == old
                       and (source / proof).read_bytes() == PROOF and not retained.exists(), old)
            self.inputs(); self.budget('final')
            protected = host.checks.inventory()
            host.checks.verify_protected(self.protected, protected)
            self.save('protected-after.json', protected)
            self.check('protected-binaries-unchanged', self.old == {p: host.checks.sha(p) for p in self.old}, self.old)
            result['status'] = 'PASS'
        except Exception as error:
            result['error'] = str(error)
        finally:
            if self.started:
                self.close_remaining(result)
            if owned and (self.root / 'logs').exists():
                shutil.copytree(self.root / 'logs', self.out / 'service-logs')
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

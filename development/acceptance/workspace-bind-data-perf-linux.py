#!/usr/bin/env python3
"""Current host workspace bind used by ordinary OCI: one small paired data case."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics

from importlib.util import spec_from_file_location, module_from_spec
HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = spec_from_file_location(name, path)
    module = module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


host = load('host_runtime', HERE / 'workspace-host-linux.py')
perf = load('container_perf', HERE / 'container-workspace-perf-linux.py')


def paired_analysis(rounds):
    if len(rounds) != 6 or [r['round'] for r in rounds] != list(range(6)):
        raise ValueError('exact warmup plus five rounds required')
    values = {'write': [], 'read': []}
    for index, round_value in enumerate(rounds):
        expected_order = ['experiment', 'reference'] if index % 2 == 0 else ['reference', 'experiment']
        samples = round_value['samples']
        if (round_value['measured'] is not (index > 0)
                or round_value['order'] != expected_order
                or [s['target'] for s in samples] != expected_order):
            raise ValueError('pair/order/warmup accounting mismatch')
        by_target = {s['target']: s for s in samples}
        for operation in values:
            for sample in samples:
                perf.verify_io_result(sample[operation], 'seq-' + operation,
                                      'fsync' if operation == 'write' else 'close')
            if index > 0:
                values[operation].append(by_target['reference'][operation]['wall_ns']
                                         / by_target['experiment'][operation]['wall_ns'])
    return {name: dict(paired_speed_ratios=ratios, median=statistics.median(ratios),
                      minimum=min(ratios), maximum=max(ratios), threshold=0.90,
                      status='PASS' if statistics.median(ratios) >= 0.90 else 'FAIL')
            for name, ratios in values.items()}


class Containers(perf.Driver):
    """Reuse existing ordinary OCI implementation with the current run's recorder."""
    def __init__(self, run):
        self.run = run
        self.root, self.out, self.args = run.root, run.out, run.args
        self.runtime_root = run.root / 'runc-state'
        self.check, self.command = run.check, run.command

    def runc_at(self, root, argv, out_dir, timeout=300, allowed=(0,)):
        return self.command(['/usr/local/sbin/runc', '--root', root, *argv],
                            timeout=timeout, allowed=allowed)


class Run(host.Run):
    def __init__(self, args):
        super().__init__(args)
        self.containers = Containers(self)
        self.active = []

    def preflight(self):
        super().preflight()
        self.check('official-runc', host.checks.sha('/usr/local/sbin/runc') == self.args.runtime_sha256,
                   host.checks.sha('/usr/local/sbin/runc'))
        self.check('pinned-rootfs-map', host.checks.sha(self.args.rootfs_inputs)
                   == self.inputs_before['rootfs_inputs_sha256'], host.checks.sha(self.args.rootfs_inputs))
        inputs = json.loads(self.args.rootfs_inputs.read_text())
        self.check('exact-eight-rootfs-inputs', set(inputs) == {
            'afs-workspace-probe', 'bin/busybox', 'bin/sh', 'lib/ld-linux-aarch64.so.1',
            'lib/aarch64-linux-gnu/libgcc_s.so.1', 'lib/aarch64-linux-gnu/libc.so.6', 'io', 'benchmark'}, inputs)
        for rel, expected in inputs.items():
            path = self.args.template_rootfs / rel
            st = path.lstat()
            self.check('template-' + rel, path.is_file() and not path.is_symlink()
                       and host.checks.sha(path) == expected['sha256']
                       and st.st_size == expected['bytes'] and st.st_uid == expected['uid']
                       and oct(st.st_mode & 0o777) == expected['mode'], expected)
        libraries = self.command(['ldd', self.args.io_bin])
        self.check('io-libraries', 'not found' not in libraries, libraries)
        # These are solely this invocation's pre-install temporary ELF copies.
        shutil.rmtree(self.args.transport / 'admission-bin')
        self.budget('admission-copies-closed', initial=True)

    def close_containers(self, label):
        for value in self.active:
            self.containers.stop_ordinary_container(value)
            self.check(value['id'] + '-PID-gone', not Path('/proc/' + str(value['pid'])).exists(), value['pid'])
        self.active.clear()
        self.check(label + '-runtime-empty', self.containers.runtime_empty(self.containers.runtime_root, label), [])

    def cohort(self, label, identity):
        target = self.root / 'mount/ownerfs/workspace'
        reference = self.root / ('reference-' + label)
        reference.mkdir(mode=0o700)
        os.chown(reference, 501, 501)
        rounds = []
        try:
            for name, path, fstype in [('experiment', target, 'fuse' if label == 'off' else 'ext4'),
                                       ('reference', reference, 'ext4')]:
                value = self.containers.start_ordinary_container(label + '-' + name, path, fstype)
                self.active.append(value)
            by_target = {value['id'].split('-', 1)[1]: value for value in self.active}
            for index in range(6):
                order = ['experiment', 'reference'] if index % 2 == 0 else ['reference', 'experiment']
                item = dict(round=index, measured=index > 0, order=order, samples=[],
                            resources_before=perf.snapshot_host(self.root))
                for name in order:
                    container = by_target[name]
                    path = '/workspace/io-' + label + '-' + name + '-' + str(index)
                    before = self.snapshot(label + '-' + name + '-' + str(index) + '-before')
                    sample = dict(target=name)
                    for operation, barrier, mode in [('write', 'fsync', 'create'), ('read', 'close', 'existing')]:
                        raw = self.containers.runc(['exec', container['id'], '/io', path,
                            'seq-' + operation, str(perf.DATA_BYTES), str(perf.BLOCK_BYTES), '1', barrier,
                            str(perf.DATA_BYTES), str(perf.PATTERN_BYTE), mode, 'unobserved'], self.out, timeout=180)
                        payload = json.loads(raw)
                        perf.verify_io_result(payload, 'seq-' + operation, barrier)
                        sample[operation] = payload
                        self.budget(label + '-' + name + '-' + str(index) + '-' + operation)
                    self.containers.runc(['exec', container['id'], '/bin/busybox', 'rm', path], self.out, timeout=30)
                    self.check(label + '-' + name + '-' + str(index) + '-file-absent',
                               not Path(str(target if name == 'experiment' else reference) + '/' + Path(path).name).exists(), path)
                    after = self.snapshot(label + '-' + name + '-' + str(index) + '-after')
                    observed = host.counts.delta(before, after)
                    if name == 'experiment':
                        self.check(label + '-' + str(index) + '-data-callbacks',
                                   all(observed[n] == 0 for n in ('read', 'write')) if label == 'on'
                                   else all(observed[n] > 0 for n in ('read', 'write')), observed)
                    sample['callback_delta'] = observed
                    item['samples'].append(sample)
                self.check(label + '-' + str(index) + '-same-Node-mount', self.identity() == identity, identity)
                item['resources_after'] = perf.snapshot_host(self.root)
                rounds.append(item)
                self.save(label + '-round-' + str(index) + '.json', item)
            analysis = paired_analysis(rounds)
            self.save(label + '-analysis.json', analysis)
            return analysis
        finally:
            self.close_containers(label)
            # Exact input/clone hashes remain recorded; ordinary stopped copies are disposable.
            for value in ('experiment', 'reference'):
                path = self.root / ('rootfs-' + label + '-' + value)
                if path.exists():
                    shutil.rmtree(path)

    def execute(self):
        result = dict(status='BLOCKED', source_commit=self.args.source_commit,
                      scope='G2.13 current container C1 64MiB write-then-read only; cache unobserved')
        current, owned = None, False
        try:
            self.preflight()
            owned = True
            self.install()
            result['status'] = 'FAIL'
            self.configuration(False)
            self.started = True
            self.ctl('start', 'all')
            current = self.identity()
            self.save('off-identity.json', current)
            workspace = self.root / 'mount/ownerfs/workspace'
            workspace.mkdir(mode=0o700)
            os.chown(workspace, 501, 501)
            os.chmod(workspace, 0o700)
            off = self.cohort('off', current)
            self.stop('off', current)
            self.configuration(True)
            current = None
            self.started = True
            self.ctl('start', 'all')
            current = self.identity()
            self.save('on-identity.json', current)
            _, _, binding = self.binding(current)
            self.save('binding-before.json', binding)
            on = self.cohort('on', current)
            _, _, after = self.binding(current)
            self.check('same-host-bind', after == binding, after)
            self.save('binding-after.json', after)
            self.stop('on', current)
            self.inputs()
            self.check('runtime-unchanged', host.checks.sha('/usr/local/sbin/runc') == self.args.runtime_sha256, self.args.runtime_sha256)
            self.check('template-unchanged', all(host.checks.sha(self.args.template_rootfs / rel) == value['sha256']
                       for rel, value in json.loads(self.args.rootfs_inputs.read_text()).items()), str(self.args.template_rootfs))
            self.budget('final')
            protected = host.checks.inventory()
            host.checks.verify_protected(self.protected, protected)
            self.save('protected-after.json', protected)
            self.check('protected-binaries-unchanged', self.old == {p: host.checks.sha(p) for p in self.old}, self.old)
            result.update(status='DATA_RECORDED', functional='PASS', off=off, on=on,
                          selected_performance='PASS' if all(v['status'] == 'PASS' for v in on.values()) else 'FAIL')
        except Exception as error:
            result['error'] = str(error)
        finally:
            if self.active:
                try:
                    self.close_containers('failure')
                except Exception as error:
                    result['container_cleanup_error'] = str(error)
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
        return 0 if result['status'] == 'DATA_RECORDED' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('root', 'out', 'transport', 'package', 'inputs', 'template-rootfs', 'rootfs-inputs'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('package-sha256', 'source-commit', 'afs-meta-sha256', 'afs-node-sha256',
                 'inputs-sha256', 'runtime-sha256'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--protect-binary', type=Path, action='append', default=[])
    args = parser.parse_args()
    args.io_bin = args.template_rootfs / 'io'
    args.benchmark_bin = args.template_rootfs / 'benchmark'
    return Run(args).execute()


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Ordinary OCI workspace: one bounded six-phase metadata comparison."""
import json
import os
from pathlib import Path
import shutil
import statistics
from importlib.util import spec_from_file_location, module_from_spec

HERE = Path(__file__).resolve().parent
spec = spec_from_file_location('workspace_data', HERE / 'workspace-bind-data-perf-linux.py')
data = module_from_spec(spec)
spec.loader.exec_module(data)

PHASES = ('create_write_close', 'stat', 'read_close', 'readdir', 'rename', 'unlink')
CALLBACKS = ('create', 'write', 'read', 'readdir', 'rename', 'unlink', 'mkdir', 'rmdir')


def verify_payload(value):
    data.perf.verify_metadata_result(value)
    if value.get('barrier') != 'close visibility; durability unqualified':
        raise ValueError('changed metadata barrier')
    for phase in value['phases']:
        if type(phase['wall_ns']) is not int:
            raise ValueError('metadata wall timing')
        expected = 1 if phase['name'] == 'readdir' else 1000
        if type(phase.get('operations')) is not int or phase['operations'] != expected:
            raise ValueError('metadata operation accounting')
        if type(phase.get('client_cpu_ns')) is not int or phase['client_cpu_ns'] < 0:
            raise ValueError('metadata CPU timing')
        if expected == 1000:
            percentiles = [phase.get(n) for n in ('p50_ns', 'p95_ns', 'p99_ns')]
            if any(type(n) is not int or n <= 0 for n in percentiles) or percentiles != sorted(percentiles):
                raise ValueError('metadata latency accounting')
    return {phase['name']: phase for phase in value['phases']}


def paired_analysis(rounds):
    if len(rounds) != 6 or [r['round'] for r in rounds] != list(range(6)):
        raise ValueError('exact warmup plus five rounds required')
    ratios = {name: [] for name in PHASES}
    for index, item in enumerate(rounds):
        order = ['experiment', 'reference'] if index % 2 == 0 else ['reference', 'experiment']
        if (item['measured'] is not (index > 0) or item['order'] != order
                or [s['target'] for s in item['samples']] != order):
            raise ValueError('pair/order/warmup accounting mismatch')
        values = {sample['target']: verify_payload(sample['metadata']) for sample in item['samples']}
        if index > 0:
            for name in PHASES:
                ratios[name].append(values['reference'][name]['wall_ns'] / values['experiment'][name]['wall_ns'])
    return {name: dict(paired_speed_ratios=values, median=statistics.median(values),
                      minimum=min(values), maximum=max(values), threshold=.90,
                      status='PASS' if statistics.median(values) >= .90 else 'FAIL')
            for name, values in ratios.items()}


class Run(data.Run):
    scope = 'G2.13 current container1000x4096B/C1 absolute six phases; cache/durability unqualified'

    def preflight(self):
        super().preflight()
        libraries = self.command(['ldd', self.args.benchmark_bin])
        self.check('benchmark-libraries', 'not found' not in libraries, libraries)

    def cohort(self, label, identity):
        target = self.root / 'mount/ownerfs/workspace'
        reference = self.root / ('reference-' + label)
        reference.mkdir(mode=0o700)
        os.chown(reference, 501, 501)
        rounds = []
        try:
            for name, path, fstype in [('experiment', target, 'fuse' if label == 'off' else 'ext4'),
                                       ('reference', reference, 'ext4')]:
                self.active.append(self.containers.start_ordinary_container(label + '-' + name, path, fstype))
            by_target = {value['id'].split('-', 1)[1]: value for value in self.active}
            for index in range(6):
                order = ['experiment', 'reference'] if index % 2 == 0 else ['reference', 'experiment']
                item = dict(round=index, measured=index > 0, order=order, samples=[],
                            resources_before=data.perf.snapshot_host(self.root))
                for name in order:
                    container = by_target[name]
                    path = '/workspace/meta-' + label + '-' + name + '-' + str(index)
                    before = self.snapshot(label + '-' + name + '-' + str(index) + '-before')
                    raw = self.containers.runc(['exec', container['id'], '/benchmark', path, 'absolute', '1'],
                                               self.out, timeout=180)
                    payload = json.loads(raw)
                    verify_payload(payload)
                    self.budget(label + '-' + name + '-' + str(index) + '-metadata')
                    owned = (target if name == 'experiment' else reference) / Path(path).name
                    self.check(label + '-' + name + '-' + str(index) + '-empty-directory',
                               owned.is_dir() and not any(owned.iterdir()), str(owned))
                    self.containers.runc(['exec', container['id'], '/bin/busybox', 'rmdir', path],
                                         self.out, timeout=30)
                    self.check(label + '-' + name + '-' + str(index) + '-directory-absent',
                               not owned.exists(), str(owned))
                    after = self.snapshot(label + '-' + name + '-' + str(index) + '-after')
                    delta = data.host.counts.delta(before, after)
                    if name == 'experiment':
                        self.check(label + '-' + str(index) + '-metadata-callbacks',
                                   all(delta[n] == 0 for n in CALLBACKS) if label == 'on'
                                   else all(delta[n] > 0 for n in CALLBACKS), delta)
                    item['samples'].append(dict(target=name, metadata=payload, callback_delta=delta))
                self.check(label + '-' + str(index) + '-same-Node-mount', self.identity() == identity, identity)
                item['resources_after'] = data.perf.snapshot_host(self.root)
                rounds.append(item)
                self.save(label + '-round-' + str(index) + '.json', item)
            analysis = paired_analysis(rounds)
            self.save(label + '-analysis.json', analysis)
            return analysis
        finally:
            self.close_containers(label)
            for name in ('experiment', 'reference'):
                path = self.root / ('rootfs-' + label + '-' + name)
                if path.exists():
                    shutil.rmtree(path)


if __name__ == '__main__':
    raise SystemExit(Run(data.parse_args(__doc__)).execute())

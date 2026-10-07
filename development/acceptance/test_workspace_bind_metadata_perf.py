"""Reject invalid measurement evidence and keep six phase decisions independent."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('metadata_perf', Path(__file__).with_name('workspace-bind-metadata-perf-linux.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def payload():
    phases = []
    for name in runner.PHASES:
        value = dict(name=name, wall_ns=1000, client_cpu_ns=100, operations=1 if name == 'readdir' else 1000)
        if name != 'readdir':
            value.update(p50_ns=10, p95_ns=20, p99_ns=30)
        phases.append(value)
    return dict(files=1000, file_bytes=4096, concurrency=1, path_form='absolute',
                barrier='close visibility; durability unqualified', phases=phases)


def rounds():
    values = []
    for index in range(6):
        order = ['experiment', 'reference'] if index % 2 == 0 else ['reference', 'experiment']
        values.append(dict(round=index, measured=index > 0, order=order,
                           samples=[dict(target=name, metadata=payload()) for name in order]))
    return values


class MetadataGuards(unittest.TestCase):
    def test_six_phase_decisions_warmup_and_ratio_direction(self):
        value = rounds()
        for item in value:
            sample = next(s for s in item['samples'] if s['target'] == 'experiment')
            sample['metadata']['phases'][0]['wall_ns'] = 2000
        value[0]['samples'][0]['metadata']['phases'][1]['wall_ns'] = 10**9
        result = runner.paired_analysis(value)
        self.assertEqual(set(result), set(runner.PHASES))
        self.assertEqual(result['create_write_close']['median'], .5)
        self.assertEqual(result['create_write_close']['status'], 'FAIL')
        self.assertEqual(result['stat']['median'], 1)
        self.assertEqual(result['stat']['status'], 'PASS')
        self.assertEqual(len(result['stat']['paired_speed_ratios']), 5)

    def test_invalid_pairs_do_not_produce_results(self):
        for change in ('missing', 'duplicate', 'order', 'warmup'):
            value = rounds()
            if change == 'missing':
                value.pop()
            elif change == 'duplicate':
                value[1]['samples'][0]['target'] = value[1]['samples'][1]['target']
            elif change == 'order':
                value[1]['order'].reverse()
            else:
                value[0]['measured'] = True
            with self.subTest(change=change), self.assertRaises(ValueError):
                runner.paired_analysis(value)

    def test_changed_shape_and_barrier_are_rejected(self):
        for key, changed in [('files', 999), ('file_bytes', 8192), ('concurrency', 8),
                             ('path_form', 'relative'), ('barrier', 'durable')]:
            value = payload()
            value[key] = changed
            with self.subTest(key=key), self.assertRaises(ValueError):
                runner.verify_payload(value)

    def test_operation_latency_and_timing_accounting(self):
        for key, changed in [('operations', 999), ('operations', True), ('client_cpu_ns', -1),
                             ('wall_ns', True), ('p50_ns', 0), ('p95_ns', 31)]:
            value = payload()
            value['phases'][0][key] = changed
            with self.subTest(key=key, changed=changed), self.assertRaises(ValueError):
                runner.verify_payload(value)
        value = payload()
        value['phases'][3]['operations'] = 1000
        with self.assertRaises(ValueError):
            runner.verify_payload(value)

    def test_missing_or_reordered_phase_is_rejected(self):
        for change in ('missing', 'reorder'):
            value = copy.deepcopy(payload())
            if change == 'missing':
                value['phases'].pop()
            else:
                value['phases'].reverse()
            with self.subTest(change=change), self.assertRaises(ValueError):
                runner.verify_payload(value)


class CohortWindowTests(unittest.TestCase):
    def cohort_fixture(self, root, benchmark_readdir=0):
        run = object.__new__(runner.Run)
        run.root, run.out, run.active = root, root / 'output', []
        target = root / 'mount/ownerfs/workspace'
        target.mkdir(parents=True)
        counters = dict.fromkeys(runner.CALLBACKS, 0)
        events, saved, checks, backing = [], {}, {}, {}
        identity = {'node': 'same incarnation', 'mount': 'same mount'}

        def start(name, path, fstype):
            backing[name] = path
            return {'id': name}

        def runc(argv, out, timeout):
            container, operation = argv[1], argv[2]
            owned = backing[container] / Path(argv[3] if operation == '/benchmark' else argv[4]).name
            if operation == '/benchmark':
                events.append(('benchmark', container))
                owned.mkdir()
                if container == 'on-experiment':
                    counters['readdir'] += benchmark_readdir
                return json.dumps(payload())
            self.assertEqual(argv[2:4], ['/bin/busybox', 'rmdir'])
            events.append(('rmdir', container))
            owned.rmdir()
            return ''

        def snapshot(label):
            events.append(('snapshot', label))
            return dict(counters)

        def budget(label):
            events.append(('budget', label))
            # A budget walk through the mounted workspace causes real FUSE readdir callbacks.
            counters['readdir'] += 4

        def check(label, ok, value):
            checks[label] = {'ok': ok, 'value': copy.deepcopy(value)}
            if not ok:
                raise ValueError(label)

        def close(label):
            events.append(('close', label))
            run.active.clear()

        run.containers = SimpleNamespace(start_ordinary_container=start, runc=runc)
        run.snapshot, run.budget, run.check, run.close_containers = snapshot, budget, check, close
        run.identity = lambda: identity
        run.save = lambda name, value: saved.update({name: copy.deepcopy(value)})
        return run, identity, events, saved, checks

    def test_budget_readdir_is_outside_on_window_and_all_six_pairs_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            run, identity, events, saved, checks = self.cohort_fixture(Path(directory))
            with patch.object(runner.os, 'chown'), patch.object(runner.data.perf, 'snapshot_host', return_value={}):
                try:
                    analysis = runner.Run.cohort(run, 'on', identity)
                except ValueError as error:
                    self.fail('budget observation polluted the callback window: ' + str(error))
            self.assertEqual(len([name for name in saved if name.startswith('on-round-')]), 6)
            self.assertEqual(len([event for event in events if event[0] == 'benchmark']), 12)
            self.assertEqual(len([event for event in events if event[0] == 'budget']), 12)
            self.assertTrue(all(value['status'] == 'PASS' for value in analysis.values()))
            for index in range(6):
                decision = checks[f'on-{index}-metadata-callbacks']
                self.assertTrue(decision['ok'])
                self.assertEqual(decision['value'], dict.fromkeys(runner.CALLBACKS, 0))
                self.assertEqual(len(saved[f'on-round-{index}.json']['samples']), 2)
                for name in ('experiment', 'reference'):
                    self.assertLess(events.index(('snapshot', f'on-{name}-{index}-after')),
                                    events.index(('budget', f'on-{name}-{index}-metadata')))
            self.assertEqual(events[-1], ('close', 'on'))

    def test_real_benchmark_readdir_still_rejects_on_and_closes_containers(self):
        with tempfile.TemporaryDirectory() as directory:
            run, identity, events, saved, checks = self.cohort_fixture(Path(directory), benchmark_readdir=1)
            with patch.object(runner.os, 'chown'), patch.object(runner.data.perf, 'snapshot_host', return_value={}):
                with self.assertRaisesRegex(ValueError, 'on-0-metadata-callbacks'):
                    runner.Run.cohort(run, 'on', identity)
            self.assertFalse(checks['on-0-metadata-callbacks']['ok'])
            self.assertGreaterEqual(checks['on-0-metadata-callbacks']['value']['readdir'], 1)
            self.assertIn(('benchmark', 'on-experiment'), events)
            self.assertNotIn('on-analysis.json', saved)
            self.assertEqual(events[-1], ('close', 'on'))
            self.assertEqual(run.active, [])


if __name__ == '__main__':
    unittest.main()

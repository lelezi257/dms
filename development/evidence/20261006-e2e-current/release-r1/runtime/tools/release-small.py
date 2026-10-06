#!/usr/bin/env python3
"""Reuse the frozen paired I/O implementation with identified release ELFs."""
import hashlib
import importlib.util
import json
import pathlib

if not __debug__:
    raise RuntimeError('validation requires normal Python')
r = pathlib.Path('/var/tmp/afs-e2e-release-20261006-r1')
base = r / 'tools/run-small.py'
spec = importlib.util.spec_from_file_location('frozen_small', base)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
m.ROOT = r
m.OUT = r / 'results/owner-small-release-r1'
m.TOOL = r / 'tools/posix-workload'
m.OWNER = r / 'mount/ownerfs/g2-release-owner/perf-small-release-r1'
m.EXT4 = r / 'perf-ext4-small-release-r1'
m.SHA = {'meta': '9cfdf8f01def1a45383baea834598a0a6ff572f3a2a0b4aa9af6a1523963bea9',
         'node': '926ada2800093b5b7682ed37a978549eaf2704a25b5ebfc7cc346244cbd689a7'}
m.OUT.mkdir()
m.save(m.OUT / 'frozen-plan.json', {'source_commit': 'e925c5bcf0408851ebfa08a59df29953374da9e9',
       'profile': 'release', 'candidate': 'new optimized ELF, same 154 compiler inputs',
       'bind': 'OFF', 'samples_per_target_per_case': 5, 'order': 'alternating ext4/owner by pair',
       'total_bytes': 67108864, 'block_bytes': 1048576, 'concurrency': 1,
       'ratio_gate': .9, 'noise_tolerance': 0, 'reruns': 0,
       'cache': 'buffered after preparation, no cold/hot qualification',
       'delete': 'not rerun; existing qualified report retained',
       'per_action_timeout_seconds': 120, 'external_whole_run_timeout_seconds': 300,
       'frozen_implementation_sha256': hashlib.sha256(base.read_bytes()).hexdigest(),
       'wrapper_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()})
try:
    m.preflight()
except Exception as exc:
    m.save(m.OUT / 'blocked.json', {'status': 'BLOCKED', 'stage': 'preflight', 'error': str(exc)})
    raise
results = []
for family in ('seq-read', 'seq-write'):
    try:
        result = m.pair_io(family)
        for sample in m.OUT.glob(family+'-*-run.stdout'):
            raw = json.loads(sample.read_text())
            assert raw['dir_barrier_rc'] == 0
    except Exception as exc:
        result = {'case': family, 'status': 'FAIL', 'error': str(exc)}
        m.save(m.OUT / (family+'-failure.json'), result)
    results.append(result)
    print(json.dumps({k: v for k, v in result.items() if k != 'samples'}), flush=True)
m.save(m.OUT / 'summary.json', results)
before = json.loads((m.OUT / 'preflight.json').read_text())
after = {}
for name, identity in before['identities'].items():
    pid = int((r / f'run/{name}.pid').read_text())
    after[name] = {'pid': pid, 'elf_sha256': m.digest(f'/proc/{pid}/exe')}
    assert after[name] == identity
mount = json.loads(m.cmd(['findmnt', '-J', '-T', str(m.OWNER)]))['filesystems'][0]
assert mount == before['owner_mount']
failed = any(x.get('status') == 'FAIL' or x.get('performance') == 'FAIL' for x in results)
m.save(m.OUT / 'run-status.json', {'status': 'FAIL' if failed else 'PASS',
       'identity_after': after, 'mount_after': mount, 'case_results': results})
raise SystemExit(3 if failed else 0)

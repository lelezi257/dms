#!/usr/bin/env python3
"""Verify exact two-VM core scope against raw manifests, never rerun I/O."""
import json
import pathlib
import tomllib

if not __debug__:
    raise RuntimeError('validation requires normal Python')
r = pathlib.Path('/var/tmp/afs-e2e-cross-20261006-r1')
a, b = r / 'results', r / 'peer/node-b/results'
read = lambda base, name: json.loads((base / name).read_text())
checks = []

def check(name, passed, evidence):
    checks.append({'name': name, 'status': 'PASS' if passed else 'FAIL', 'evidence': evidence})
    if not passed:
        raise RuntimeError(name)

for node, base in [('node-a', a), ('node-b', b)]:
    preflight = read(base, 'preflight.json')
    config = tomllib.loads((base / 'node.toml').read_text())
    check(node + '-preflight', preflight['status'] == 'PASS', 'preflight.json')
    check(node + '-replica-policy', all(config[k] == 2 for k in (
        'dfs_desired_copies', 'dfs_sync_required_copies', 'dfs_min_distinct_nodes',
        'dfs_min_distinct_failure_domains')), 'node.toml: R2/2/2, not R3 comparator')
    for before_name, after_name in [('before-meta-restart', 'after-meta-restart'),
                                    ('before-dfs-meta-restart', 'after-dfs-meta-restart')]:
        before = read(base, before_name + '-identity.json')
        after = read(base, after_name + '-identity.json')
        check(node + '-' + before_name + '-node-unchanged',
              before['processes']['node'] == after['processes']['node'],
              [before_name + '-identity.json', after_name + '-identity.json'])
        check(node + '-' + before_name + '-mount-unchanged', before['mounts'] == after['mounts'],
              [before_name + '-identity.json', after_name + '-identity.json'])
        if node == 'node-a':
            old, new = before['processes']['meta'], after['processes']['meta']
            check(before_name + '-meta-restarted', old['pid'] != new['pid'] and
                  old['starttick'] < new['starttick'] and old['elf_sha256'] == new['elf_sha256'],
                  [before_name + '-identity.json', after_name + '-identity.json'])
    check(node + '-owner-directory-barrier', read(base, 'before-meta-restart-identity.json')
          ['directory_barrier']['fsync'] == 'PASS', 'before-meta-restart-identity.json')

owner_write = read(a, 'owner-a-write/manifest.json')
owner_patch = read(b, 'owner-b-patch/manifest.json')
check('owner-B-read-A-write', read(b, 'owner-b-read/manifest.json')['sha256'] == owner_write['sha256'],
      ['node-a/owner-a-write', 'node-b/owner-b-read'])
check('owner-B-actually-modified', owner_patch['sha256'] != owner_write['sha256'] and
      len(owner_patch['patches']) == len(owner_write['patches']) + 1, 'owner-b-patch/manifest.json')
for base, phase in [(a, 'owner-a-read-patch'), (a, 'owner-a-after-meta'), (b, 'owner-b-after-meta')]:
    manifest = read(base, phase + '/manifest.json')
    check(phase, manifest['status'] == 'PASS' and manifest['size'] == 67108864 and
          manifest['sha256'] == owner_patch['sha256'], phase + '/manifest.json')
for phase in ('before', 'after'):
    home = read(a, 'owner-home-' + phase + '.json')
    check('owner-home-' + phase, home['home_node_id'] == 'node-a' and home['home_serving'] and
          home['root_id'] == 'root-' + b'g2-cross-owner'.hex(), 'owner-home-' + phase + '.json')
check('owner-rename', read(b, 'owner-b-rename.json')['status'] == 'PASS', 'owner-b-rename.json')
deleted = read(a, 'owner-a-rename-read-delete.json')
check('owner-renamed-content-and-delete', deleted['status'] == 'PASS' and
      deleted['renamed_sha256'] == owner_patch['sha256'] and deleted['renamed_size'] == 67108864,
      'owner-a-rename-read-delete.json')
check('owner-delete-visible-B', read(b, 'owner-b-delete-visible.json')['status'] == 'PASS',
      'owner-b-delete-visible.json')

dfs_write = read(a, 'dfs-a-write/manifest.json')
check('dfs-writer-A', dfs_write['status'] == 'PASS' and dfs_write['size'] == 67108864,
      'dfs-a-write/manifest.json')
processes = []
for base, phases in [(a, ('dfs-reader-a', 'dfs-reader-a-after-meta')),
                     (b, ('dfs-reader-b', 'dfs-reader-b-after-meta'))]:
    for phase in phases:
        manifest = read(base, phase + '/manifest.json')
        process = read(base, phase + '-process.json')
        check(phase, manifest['status'] == process['status'] == 'PASS' and process['returncode'] == 0
              and manifest['size'] == 67108864 and manifest['sha256'] == dfs_write['sha256'],
              [phase + '/manifest.json', phase + '-process.json'])
        if not phase.endswith('-after-meta'):
            processes.append(process)
check('dfs-independent-readers', len({p['hostname'] for p in processes}) == 2,
      ['dfs-reader-a-process.json', 'dfs-reader-b-process.json'])
overlap = min(p['end_unix_ns'] for p in processes) - max(p['start_unix_ns'] for p in processes)
check('dfs-observed-reader-overlap', overlap > 0,
      {'observed_wall_clock_overlap_ns': overlap, 'not_performance_clock_qualification': True})
check('dfs-directory-barrier', read(a, 'dfs-directory-before-meta.json')['status'] == 'PASS',
      'dfs-directory-before-meta.json')
summary = {'status': 'PASS', 'source_commit': 'e925c5bcf0408851ebfa08a59df29953374da9e9',
           'scope': 'Owner two-VM core and orderly central recovery; DFS R2 one writer/two Node reader views and orderly central recovery',
           'checks': checks, 'owner_sha256_after_remote_patch': owner_patch['sha256'],
           'dfs_sha256': dfs_write['sha256'], 'performance': 'NOT_QUALIFIED',
           'excluded_claims': ['full POSIX', 'Node crash recovery', 'three-sync-durable 3FS parity',
                               'MooseFS parity', 'bind ON', 'G2.27 performance release']}
(r / 'results/core-proof.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({k: v for k, v in summary.items() if k != 'checks'}))
print('checks', len(checks))

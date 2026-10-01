#!/usr/bin/env python3
"""Linux-only source/runtime batch evidence and input-identity audit."""
import hashlib
import json
import pathlib
import platform
import re
import sys

assert platform.system() == 'Linux'
root = pathlib.Path(sys.argv[1]).resolve()
frozen = json.loads((root / 'build-inputs.json').read_text())
host = {line.split(maxsplit=1)[1].strip(): line.split(maxsplit=1)[0]
        for line in (root / 'host-compile-hashes.txt').read_text().splitlines()}
assert len(host) == frozen['file_count'] == 143
assert host == frozen['files']
protected = (root / 'protected-files-sha.txt').read_text()
assert '539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f  AGENTS.md' in protected
assert '8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216  docs/handoff.md' in protected
original = root / 'original-regression/owner-metrics-v73-original'
assert (original / 'timer.exit').read_text().strip() == '101'
failure = (original / 'timer.log').read_text()
assert 'owner_rdma_missing_device_errors_are_counted_by_client_timer ... FAILED' in failure
assert 'left: 0' in failure and 'right: 1' in failure
gate = root / 'owner-metrics-v73-r2'
for name in ('fmt', 'clippy', 'lib', 'error', 'contracts', 'local-api', 'fuse-build', 'fuse', 'build'):
    assert (gate / 'full' / (name + '.exit')).read_text().strip() == '0', name
for name in ('no-features', 'owner-features', 'dfs-features', 'owner-rdma', 'dfs-rdma'):
    assert (gate / 'features' / (name + '.exit')).read_text().strip() == '0', name
assert (gate / 'native-owner.exit').read_text().strip() == '0'
results = re.compile(r'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;')
def summary(name):
    return [(int(a), int(b)) for a, b in results.findall((gate / 'full' / (name + '.log')).read_text())]
assert summary('lib')[-1] == (407, 2)
assert sum(a for a, _ in summary('contracts')) == 65
assert summary('error')[0] == (4, 0)
assert summary('local-api') == [(9, 0)]
assert summary('fuse') == [(5, 0)]
assert 'owner_rdma_missing_device_errors_are_counted_by_client_timer ... ok' in (gate / 'full/lib.log').read_text()
native = (gate / 'native-owner.log').read_text()
assert 'ownerpeerclient_rdma_large_write_fsync_cold_read_roundtrip_preserves_payload ... ok' in native
assert 'AFS_RDMA_COMPLETE op=READ bytes=' in native
assert 'AFS_RDMA_COMPLETE op=WRITE bytes=' in native
runtime = root / 'runtime'
audit = json.loads((runtime / 'audit-report.json').read_text())
assert audit['status'] == 'PASS' and audit['level'] == 'STAGE_INTEGRATION'
assert json.loads((runtime / 'build-inputs.json').read_text())['files'] == frozen['files']
for name, digest in audit['files'].items():
    assert hashlib.sha256((runtime / name).read_bytes()).hexdigest() == digest, name
assert 'Ran 10 tests' in (runtime / 'audit-tests-final.log').read_text()
assert 'Ran 7 tests' in (runtime / 'script-tests-final.log').read_text()
links = []
for page in root.rglob('*.md'):
    for target in re.findall(r'\]\(([^)]+)\)', page.read_text()):
        if '://' in target or target.startswith('#'):
            continue
        assert (page.parent / target.split('#', 1)[0]).exists(), (page, target)
        links.append((str(page.relative_to(root)), target))
files = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
         for p in sorted(root.rglob('*')) if p.is_file() and p.name != 'batch-report.json'
         and '__pycache__' not in p.parts}
assert not any(part.startswith('._') for name in files for part in pathlib.Path(name).parts)
print(json.dumps({'status': 'PASS', 'level': 'STAGE_GATE', 'source_inputs': 143,
                  'gate_counts': {'library': 407, 'contracts': 65, 'shared_errors': 4,
                                  'local_api': 9, 'privileged_fuse': 5},
                  'runtime': audit['checks'], 'links': links, 'files': files,
                  'formal': '69 NOT_RUN / ENV PREPARING',
                  'limits': 'Short healthy RXE flow. No formal POSIX, persistent-backend, '
                            'failure-lifetime, performance, 8 GiB or soak promotion.'}, indent=2))

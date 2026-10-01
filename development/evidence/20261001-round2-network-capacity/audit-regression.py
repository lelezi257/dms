#!/usr/bin/env python3
"""Linux-only semantic proof rejection tests; no product release qualification."""
import importlib.util
import json
import pathlib
import platform
import shutil
import tempfile

assert platform.system() == 'Linux'
root = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('audit', root/'audit.py')
audit = importlib.util.module_from_spec(spec); spec.loader.exec_module(audit)
audit.evaluate(root)
results = [{'name': 'valid scoped packet', 'ok': True}]
cases = [
    ('wrong authority', 'network/ctl/round2-network-v79b-ctl-identity-final.json', lambda v: v['identity'].update(pid=1)),
    ('no real DROP', 'network/b/iptables-during.json', lambda v: v.update(stdout='0 0 DROP all -- * * 0.0.0.0/0 0.0.0.0/0')),
    ('firewall not restored', 'network/b/rollback.json', lambda v: v.update(restored=False)),
    ('wrong business port', 'network/b/tcp-blocked.json', lambda v: v.update(port=19566)),
    ('no physical fault', 'capacity/a/evidence/fill.json', lambda v: v.update(write_errno=0)),
    ('sparse filler', 'capacity/a/evidence/fill.json', lambda v: v.update(allocated_bytes=0)),
    ('false DFS sync success', 'capacity/a/evidence/fault-dfs.json', lambda v: v['operations'][1].update(outcome='success')),
    ('failed write published', 'capacity/a/evidence/cold-committed-read.json', lambda v: v['reads']['dfs'].update(sha256=audit.NEW)),
    ('bad successful recovered data', 'capacity/a/evidence/verify.json', lambda v: v['dfs'].update(sha256='0'*64)),
]
for name, path, mutate in cases:
    with tempfile.TemporaryDirectory() as directory:
        copied = pathlib.Path(directory)/'packet'; shutil.copytree(root, copied)
        item = copied/path; value = json.loads(item.read_text()); mutate(value); item.write_text(json.dumps(value))
        try: audit.evaluate(copied)
        except AssertionError as error: results.append({'name': name, 'ok': True, 'rejected': str(error)})
        else: raise AssertionError('Invalid evidence accepted: '+name)
print(json.dumps({'status': 'PASS', 'results': results}, indent=2))

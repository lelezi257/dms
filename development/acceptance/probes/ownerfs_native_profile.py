#!/usr/bin/env python3
"""Replay an explicitly inventoried native foundation slice on VM ext4."""
from pathlib import Path
import hashlib, json, os, re, subprocess, sys

if len(sys.argv) != 5:
    raise SystemExit('usage: ownerfs_native_profile.py BINARY FRESH_EXT4_EVIDENCE SHA256 MANIFEST')
binary=Path(sys.argv[1]).resolve(strict=True)
evidence=Path(sys.argv[2])
expected=sys.argv[3]
manifest_path=Path(sys.argv[4]).resolve(strict=True)
probe=Path(__file__).resolve().with_name('ownerfs_native_mount.sh')
assert os.geteuid()==0 and os.environ.get('AFS_NATIVE_ELIGIBLE_CACHE')=='1'
assert re.fullmatch('[0-9a-f]{64}', expected)
assert evidence.is_absolute() and evidence.name and evidence!=Path('/') and not evidence.exists()
assert evidence.parent.resolve(strict=True).is_dir()
assert subprocess.check_output(['findmnt','-n','-o','FSTYPE','-T',str(evidence.parent)],text=True).strip()=='ext4'
with binary.open('rb') as f: assert hashlib.file_digest(f,'sha256').hexdigest()==expected
manifest=json.loads(manifest_path.read_text())
assert manifest['version']==1
required=manifest['required']
diagnostics=manifest['historical_strong_directory_diagnostics']
assert required and len(required)==len(set(required))
assert len(diagnostics)==len(set(diagnostics)) and not set(required)&set(diagnostics)
assert all(re.fullmatch('[a-z][a-z0-9_]*', name) for name in required+diagnostics)
discovery=subprocess.check_output([str(binary),'--list'],text=True)
discovered=[line.removesuffix(': test') for line in discovery.splitlines() if line.endswith(': test')]
assert set(discovered)==set(required+diagnostics), 'Unclassified/missing tests must be registered before replay'
evidence.mkdir(mode=0o700)
(evidence/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
(evidence/'discovery.txt').write_text(discovery)
(evidence/'selection.txt').write_text('\n'.join(required)+'\n')
results=[]
for name in required:
    out=evidence/name
    with (evidence/(name+'.log')).open('w') as log:
        result=subprocess.run(['bash',str(probe),str(binary),str(out),expected,name],
                              stdout=log,stderr=subprocess.STDOUT,timeout=200)
    raw_exit=int((out/'tests.exit').read_text()) if (out/'tests.exit').exists() else None
    log=(evidence/(name+'.log')).read_text()
    passed=result.returncode==0 and raw_exit==0 and '1 passed; 0 failed; 0 ignored' in log
    item=dict(test=name,probe_exit=result.returncode,test_exit=raw_exit,passed=passed)
    results.append(item)
    print(json.dumps(item),flush=True)
    # The portable probe's zero exit includes parent-mount and disposable-data checks.
    (evidence/'results.json').write_text(json.dumps(dict(scope=manifest['scope'],binary_sha256=expected,
        complete=False,results=results,diagnostics_not_run=diagnostics),indent=2)+'\n')
passed=all(r['passed'] for r in results)
(evidence/'results.json').write_text(json.dumps(dict(scope=manifest['scope'],binary_sha256=expected,
    complete=True,passed=passed,results=results,diagnostics_not_run=diagnostics),indent=2)+'\n')
raise SystemExit(0 if passed else 1)

import hashlib
import importlib.util
import json
import pathlib
import posixpath
import re
import sys

assert sys.platform.startswith('linux')
root=pathlib.Path(sys.argv[1])
base=root/'development/evidence/20261001-posted-rdma-cancellation'
inventory=set(pathlib.Path(sys.argv[2]).read_text().splitlines())
checks=[]
files={p.relative_to(base).as_posix():{'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'bytes':p.stat().st_size} for p in sorted(base.rglob('*')) if p.is_file()}
(base/'artifact-manifest.json').write_text(json.dumps({'files':files},indent=2)+'\n')
spec=importlib.util.spec_from_file_location('checker',root/'scripts/check-rdma-cancellation.py')
checker=importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
assert checker.audit(base/'posted-cancel-v76-r6/cancel','rxe0')['status']=='PASS'
checks.append('copied raw cancellation semantics')
assert hashlib.sha256((root/'scripts/check-rdma-cancellation.py').read_bytes()).hexdigest()==json.loads((base/'posted-cancel-v76-r6/cancel/identity.json').read_text())['runner_sha256']
checks.append('copied executed checker identity')
compiled=json.loads((base/'posted-cancel-v76-r2/build-inputs.json').read_text())['files']
assert hashlib.sha256((root/'tests/rdma_lifecycle.rs').read_bytes()).hexdigest()==compiled['tests/rdma_lifecycle.rs']
checks.append('copied compiled test identity')
for p,expected in [('AGENTS.md','539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f'),('docs/handoff.md','8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216')]:
    assert hashlib.sha256((root/p).read_bytes()).hexdigest()==expected
    checks.append('protected '+p)
links=0
for page in [base/'README.md',base/'REVIEW.md',base/'runner-note.md',root/'docs/status.md',root/'docs/guides/validation.md']:
    rel=page.relative_to(root).as_posix()
    for link in re.findall(r'\[[^\]]*\]\(([^)]+)\)',page.read_text()):
        if '://' in link or link.startswith('#'): continue
        target=posixpath.normpath(posixpath.join(posixpath.dirname(rel),link.split('#',1)[0]))
        assert target in inventory,(rel,link,target)
        links+=1
checks.append('all selected page local links')
report={'status':'PASS','artifact_count':len(files),'local_links':links,'checks':checks,'formal_acceptance':'NOT_RUN','environment':'PREPARING'}
(base/'publish-audit.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))

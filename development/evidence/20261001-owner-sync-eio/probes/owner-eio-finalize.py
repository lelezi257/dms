import hashlib,json,re,sys
from pathlib import Path
assert sys.platform=='linux'
s=Path(sys.argv[1]);out=Path(sys.argv[2]);out.mkdir(parents=True,exist_ok=True)
p=s/'development/evidence/20261001-owner-sync-eio'
expected=json.loads((p/'build/compile-inputs-after.json').read_text())['files']
actual={f:hashlib.sha256((s/f).read_bytes()).hexdigest() for f in expected}
assert actual==expected,'compiler input mismatch'
old=json.loads((s/'development/evidence/20261001-round1-mainline/input-audit.json').read_text())['compiler_inputs']
delta=sorted(f for f in actual if old.get(f)!=actual[f]);assert delta==['src/node/vfs/ownerfs.rs'],delta
for f,h in {'AGENTS.md':'539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f','docs/handoff.md':'8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216'}.items():
 assert hashlib.sha256((s/f).read_bytes()).hexdigest()==h,f
links=0
for md in list(p.rglob('*.md'))+[s/f for f in ['docs/status.md','docs/architecture/ownerfs.md','development/issues.md','development/plan.md','development/implementation.md','development/validation.md']]:
 for target in re.findall(r'\]\(([^)]+)\)',md.read_text()):
  if '://' in target or target.startswith('#'):continue
  dest=target.split('#')[0];assert (md.parent/dest).resolve().exists(),(md,target);links+=1
for f in p.rglob('*'):assert not f.name.startswith('._'),str(f)
for log in (p/'build/full').rglob('*.exit'):assert log.read_text().strip()=='0',str(log)
audit=json.loads((p/'audit-final.json').read_text());assert audit['status']=='PASS' and len(audit['checks'])==63 and len(audit['negative_tests'])==8
for f in (p/'probes').glob('*.py'):compile(f.read_bytes(),str(f),'exec')
manifest={str(f.relative_to(p)):hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(p.rglob('*')) if f.is_file() and f.name not in ('artifact-hashes.json','verification.json')}
(out/'artifact-hashes.json').write_text(json.dumps({'files':manifest,'file_count':len(manifest)},indent=2)+'\n')
result={'status':'PASS','compiler_inputs':len(actual),'compiler_delta':delta,'local_file_links':links,'artifact_hashes':len(manifest),'semantic_checks':63,'negative_checks':8,'protected_files':'unchanged','stage_gate':'PASS','formal_acceptance':'NOT_RUN','round2':'INCOMPLETE'}
(out/'verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))

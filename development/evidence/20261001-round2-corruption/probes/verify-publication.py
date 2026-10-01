import hashlib,json,pathlib,re,sys
assert sys.platform=='linux'
s=pathlib.Path(sys.argv[1]);p=s/'development/evidence/20261001-round2-corruption';out=pathlib.Path(sys.argv[2]);out.mkdir(exist_ok=True)
expected=json.loads((s/'development/evidence/20261001-owner-sync-eio/build/compile-inputs-after.json').read_text())['files'];assert all(hashlib.sha256((s/n).read_bytes()).hexdigest()==h for n,h in expected.items())
protected={'AGENTS.md':'539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f','docs/handoff.md':'8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216'}
for n,h in protected.items():assert hashlib.sha256((s/n).read_bytes()).hexdigest()==h
links=0
for md in list(p.rglob('*.md'))+[s/n for n in ('docs/status.md','development/plan.md','development/issues.md')]:
 for target in re.findall(r'\]\(([^)]+)\)',md.read_text()):
  if '://' in target:continue
  f,_,anchor=target.partition('#');dest=(md.parent/f).resolve() if f else md;assert dest.exists(),(md,target)
  if anchor:
   headings=re.findall(r'^#+\s+(.*)$',dest.read_text(),re.M);assert anchor in {re.sub(r'[^\w\- ]','',h.lower()).replace(' ','-') for h in headings},(md,target)
  links+=1
for f in p.rglob('*'):
 assert not f.name.startswith('._'),str(f)
 if f.suffix=='.py':compile(f.read_bytes(),str(f),'exec')
audit=json.loads((p/'audit.json').read_text());assert audit['status']=='PASS' and len(audit['checks'])==49 and len(audit['negative_tests'])==6
files={str(f.relative_to(p)):hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(p.rglob('*')) if f.is_file() and f.name not in ('verification.json','artifact-hashes.json')}
(out/'artifact-hashes.json').write_text(json.dumps({'file_count':len(files),'files':files},indent=2)+'\n')
result={'status':'PASS','artifact_hashes':len(files),'local_links_and_anchors':links,'compiler_inputs':len(expected),'production_inputs_changed':0,'protected_files':'unchanged','semantic_checks':49,'negative_checks':6,'round2':'INCOMPLETE','formal_acceptance':'NOT_RUN'}
(out/'verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))

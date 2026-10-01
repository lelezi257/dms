"""Validate this bounded diagnostic packet in Linux; never a release verdict."""
import hashlib, json, pathlib, re, sys

assert sys.platform == 'linux'
source = pathlib.Path(sys.argv[1])
packet = source / 'development/evidence/20261001-round3-resource-diagnostic'
out = pathlib.Path(sys.argv[2]); out.mkdir(exist_ok=True)
checks = []
def check(name, value):
    assert value, name
    checks.append(name)
def load(path): return json.loads(path.read_text())
expected = load(packet / 'probes/eio-v80-r2-binaries.json')
total_ranges = 0
resource_summary = {}
for role in ('ctl', 'a', 'b'):
    e = packet / role / 'evidence'; kind = 'meta' if role == 'ctl' else 'node'
    check(role+' lifecycle', all(load(e/(n+'.json'))['status']=='PASS' for n in ('prepare','start','idle','stop','cleanup')))
    check(role+' reserve', load(e/'cleanup.json')['data_available_bytes'] >= 4*1024**3)
    before = load(e/'start-idle-resources.json')['processes'][kind]
    after = load(e/'final-idle-resources.json')['processes'][kind]
    check(role+' stable executable', before['identity'] == after['identity'] and after['identity']['exe_sha256']==expected[kind])
    stopped = load(e/'post-stop-resources.json')
    check(role+' stopped verbs observed', stopped['rdma']['status']=='OBSERVED')
    for typ, record in stopped['rdma']['resources'].items():
        check(role+' no user '+typ, all(item.get('comm')=='ib_core' and 'pid' not in item for item in record['parsed']))
    resource_summary[role] = {'before':before['status_fields'], 'after':after['status_fields'], 'fd_before':before['fd_count']['count'], 'fd_after':after['fd_count']['count']}
    for f in sorted(e.glob('read-*.json')):
        result = load(f)
        check(role+' '+f.name, result['status']=='PASS' and result['formal_acceptance']=='NOT_RUN')
        total_ranges += sum(w['range_checks'] for w in result['workers'])
    for f in sorted(e.glob('write-*.json')):
        result = load(f)
        check(role+' '+f.name, result['status']=='PASS' and result['bytes']==8*1024**2 and result['sha256']=='fc18a77cc119bf7e92a7bce53167d1c80ab1ed415788ffa61306a50684e94178')
    for f in sorted(e.glob('*-resources.json')):
        result = load(f)
        check(role+' scope '+f.name, result['formal_acceptance']=='NOT_RUN' and result['environment']=='PREPARING')
check('2048 fixed-seed range checks', total_ranges==2048)
a = load(packet/'a/evidence/physical-replication.json')['chunks']
b = load(packet/'b/evidence/physical-copies.json')['chunks']
check('exact physical copies', {(c['file'],c['bytes'],c['sha256']) for c in a} == {(c['file'],c['bytes'],c['sha256']) for c in b} and len(a)==2)
for c in a:
    v=c['replication']
    check(c['file']+' health', v['health']=='Satisfied' and v['available_copies']==2 and all(t['state']=='Completed' for t in v['tasks']))
    check(c['file']+' distinct ready durable', {x['record']['location']['Node']['node_id'] for x in v['copies']}=={'round1-a','round1-b'} and all(x['available'] and x['record']['state']=='Ready' and x['record']['role']=='DurableReplica' for x in v['copies']))
metrics=(packet/'b/evidence/final-idle-metrics.txt').read_text()
check('actual RDMA replica bytes', 'afs_dfs_payload_bytes_total{direction="recv",operation="replica",transport="rdma"} 8388608' in metrics)
check('Owner RDMA write bytes', 'afs_ownerfiles_payload_bytes_total{direction="write",plane="rdma",side="client"} 8388608' in metrics)
check('zero grpc replica payload', 'afs_dfs_payload_bytes_total{direction="recv",operation="replica",transport="grpc"} 0' in metrics)
check('helper regression', (packet/'build/helper-tests-r3.exit').read_text().strip()=='0' and 'Ran 13 tests' in (packet/'build/helper-tests-r3.log').read_text())
gate=source/'development/evidence/20261001-round2-closure/build/dfs-deadline-v82-final/compile-inputs-after.json'
inputs=load(gate)['files']
check('reuse exact143 compiler inputs', len(inputs)==143 and all(hashlib.sha256((source/p).read_bytes()).hexdigest()==h for p,h in inputs.items()))
for p,h in {'AGENTS.md':'539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f','docs/handoff.md':'8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216'}.items():
    check(p+' unchanged', hashlib.sha256((source/p).read_bytes()).hexdigest()==h)
for p in packet.rglob('*'):
    check('no AppleDouble '+str(p.relative_to(packet)), not p.name.startswith('._'))
    if p.suffix=='.py': compile(p.read_bytes(),str(p),'exec')
links=0
for md in list(packet.rglob('*.md'))+[source/p for p in ('docs/status.md','development/plan.md','development/issues.md')]:
    for target in re.findall(r'\]\(([^)]+)\)',md.read_text()):
        if '://' in target: continue
        name,_,anchor=target.partition('#');dest=(md.parent/name).resolve() if name else md
        if not dest.exists() and dest in (packet/'audit.json', packet/'artifact-hashes.json'):
            continue  # First pass generates these; publication pass verifies the imported outputs.
        check('link '+target,dest.exists());links+=1
        if anchor:
            headings=re.findall(r'^#+\s+(.*)$',dest.read_text(),re.M)
            check('anchor '+target,anchor in {re.sub(r'[^\w\- ]','',h.lower()).replace(' ','-') for h in headings})
files={str(f.relative_to(packet)):hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(packet.rglob('*')) if f.is_file() and f.name not in ('audit.json','artifact-hashes.json')}
(out/'artifact-hashes.json').write_text(json.dumps({'file_count':len(files),'files':files},indent=2)+'\n')
result={'status':'PASS','scope':'bounded development diagnostic only','checks':len(checks),'ranges':total_ranges,'local_links':links,'compiler_inputs_reused':143,'artifacts':len(files),'process_resources':resource_summary,'formal_acceptance':'NOT_RUN','environment':'PREPARING'}
(out/'audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))

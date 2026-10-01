#!/usr/bin/env python3
"""Read-only independent audit; product and filesystem checks execute in Linux."""
import hashlib, json, pathlib, subprocess
ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / 'evidence/afs-delivery/repair-runtime-v55'
SOURCE = ROOT / 'source'
def read(name): return json.loads((OUT/name).read_text())
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def guest(vm, code):
    p = subprocess.run(['limactl','shell',vm,'--','sudo','python3','-c',code],text=True,capture_output=True,timeout=45)
    with (OUT/'root-audit-commands.jsonl').open('a') as f:
        f.write(json.dumps({'argv':p.args,'returncode':p.returncode,'stdout':p.stdout,'stderr':p.stderr})+'\n')
    assert p.returncode == 0, p.stderr
    return json.loads(p.stdout)
chunk = read('repair-chunk-id.json')['chunk_id']
expected_sha = '3f8a853cfd1416af3ab78fd914f7574dd86045f3f132294cf9ba47717130d3a8'
initial = read('probe-a-create-underreplicated.json')['accepted'][0]['json']
final = read('wait-repair-satisfied.json')['polls'][-1]['json']
assert initial['available_copies']==1 and initial['health']=='UnderReplicated'
assert any(t['state']=='Pending' for t in initial['tasks'])
assert final['available_copies']==2 and final['health']=='Satisfied'
assert any(t['state']=='Completed' for t in final['tasks'])
assert not initial['loss_confirmed'] and not final['loss_confirmed']
restart = read('restart-b.identity-change.json')
x,y=restart['before']['processes']['node'],restart['after']['processes']['node']
assert (x['pid'],x['start_ticks']) != (y['pid'],y['start_ticks'])
assert x['sha256']==y['sha256']
assert restart['before']['config_sha256']==restart['after']['config_sha256']
assert 'exit_code=0' in (OUT/'restart-b-stop.stdout').read_text()
for name in ['verify-b-bytes-and-fuse.json','verify-b-bytes-and-fuse.2.json','restart-b-content-verify.json']:
    obj=read(name)
    assert obj['remote_fuse_sha256']==expected_sha
    assert any(c['size']==1048576 and c['sha256']==expected_sha and c['path'].endswith(chunk) for c in obj['matched_chunk_files'])
identities={}
for which,roles in [('a',['meta','node']),('b',['node'])]:
    run=f'/mnt/lima-afs{which}data/afs-delivery/repair-v55-{which}'
    oldrun=f'/mnt/lima-afs{which}data/afs-delivery/'+('p2-memory-lane-v51' if which=='a' else 'p2-memory-peer-v51')
    expected=read('identity-a-after-verify.2.json' if which=='a' else 'identity-b-after-verify.2.json')
    old=read(f'old-v51-{which}-before.json')
    code="""
import hashlib,json,os,pathlib,subprocess,urllib.request
run=pathlib.Path(RUN)
result={'processes':{},'config_sha256':{},'old':{'processes':{},'config_sha256':{}}}
for label,base in [('new',run),('old',pathlib.Path(OLDRUN))]:
    dst=result if label=='new' else result['old']
    for role in ROLES:
        pid=int((base/'run'/(role+'.pid')).read_text()); proc=pathlib.Path('/proc')/str(pid)
        fields=(proc/'stat').read_text().rsplit(') ',1)[1].split()
        dst['processes'][role]={'pid':pid,'start_ticks':fields[19],'exe':os.readlink(proc/'exe'),'sha256':hashlib.sha256((proc/'exe').read_bytes()).hexdigest()}
        dst['config_sha256'][role]=hashlib.sha256((base/'etc'/(role+'.toml')).read_bytes()).hexdigest()
result['mounts']={}
for name in ['mount-ownerfs','mount-dfs']:
    result['mounts'][name]=json.loads(subprocess.check_output(['findmnt','-J','-M',str(run/name),'-o','TARGET,SOURCE,FSTYPE'],text=True))['filesystems'][0]
if WHICH=='a':
    result['rest']=json.load(urllib.request.urlopen('http://127.0.0.1:18381/v1/dfs/chunks/'+CHUNK+'/replication',timeout=10))
else:
    result['disk_sha256']=hashlib.sha256((run/'state/node/dfs/chunks'/CHUNK).read_bytes()).hexdigest()
    b=(run/'mount-dfs/repair-v55-deterministic-1m.bin').read_bytes()
    result['fuse_sha256']=hashlib.sha256(b).hexdigest(); result['bytes']=len(b)
print(json.dumps(result))
"""
    code='RUN='+repr(run)+'\nOLDRUN='+repr(oldrun)+'\nROLES='+repr(roles)+'\nWHICH='+repr(which)+'\nCHUNK='+repr(chunk)+'\n'+code
    live=guest('afs-accept-'+which,code)
    for role in roles:
        for key in ['pid','start_ticks','exe','sha256']:
            assert live['processes'][role][key]==expected['processes'][role][key]
    assert live['config_sha256']==expected['config_sha256']
    assert live['old']['processes']==old['processes'] and live['old']['config_sha256']==old['config_sha256']
    for name,m in live['mounts'].items():
        assert m['target']==run+'/'+name and m['source']==('afs-ownerfs' if name=='mount-ownerfs' else 'afs-dfs') and m['fstype'].startswith('fuse')
    if which=='a':
        r=live['rest']; assert r['available_copies']==2 and r['health']=='Satisfied' and not r['loss_confirmed']
        records=[c['record'] for c in r['copies'] if c['available']]
        assert len(records)==2 and len({c['location']['Node']['node_id'] for c in records})==2
        for c in records:
            assert c['chunk_id']==chunk and c['persisted_bytes']==1048576 and c['verified_digest']['algorithm']=='Blake3'
            assert bytes(c['verified_digest']['bytes']).hex()==chunk.split('-')[1]
    else:
        assert live['bytes']==1048576 and live['disk_sha256']==live['fuse_sha256']==expected_sha
    identities[which]=live
inputs=json.loads((SOURCE/'development/evidence/20261001-async-repair/source/linux/qualified-linux-clean/compile-inputs.json').read_text())
for path,digest in inputs['files'].items(): assert sha(SOURCE/path)==digest,path
assert sha(SOURCE/'docs/handoff.md')=='8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216'
report={'status':'PASS','compile_inputs':len(inputs['files']),'replica_count_before':1,'replica_count_after':2,'chunk_id':chunk,'content_sha256':expected_sha,'restart_identity_changed':True,'old_v51_identity_preserved':True,'handoff_unchanged':True,'live':identities,'limits':['Memory Meta; not Meta restart durability','No source-loss/fault/RDMA/performance or release qualification']}
p=OUT/'root-independent-audit.json'
assert not p.exists(), 'immutable report already exists'
p.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({k:v for k,v in report.items() if k!='live'}))

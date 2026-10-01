#!/usr/bin/env python3
"""Root live audit of frozen sources, physical bytes, REST and FUSE results."""
import hashlib,importlib.util,json,pathlib,urllib.parse
ROOT=pathlib.Path(__file__).resolve().parents[3];out=ROOT/'evidence/afs-delivery/corrupt-lifecycle-rxe-v62-audit';out.mkdir(exist_ok=False)
def load(name,path):
 spec=importlib.util.spec_from_file_location(name,path);assert spec and spec.loader
 m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
r=load('v62',ROOT/'experiments/afs-acceptance/repair-runtime-rxe-v62.py').runtime
f=load('base',ROOT/'experiments/afs-acceptance/repair-fault-v56.py');f.v55=r;f.OUT=out
runtime=ROOT/'evidence/afs-delivery/corrupt-lifecycle-rxe-v62-runtime-r1';report=json.loads((runtime/'report.json').read_text());assert report['status']=='PASS'
staged=json.loads((r.OUT/'staged-runtime-binary-sha256.json').read_text());f.NODE_SHA=staged['a']['node'];f.META_SHA=staged['a']['meta']
manifest=json.loads((ROOT/'evidence/afs-delivery/corrupt-lifecycle-v62-r3/compile-inputs.json').read_text())['files']
assert len(manifest)==143
for n,h in manifest.items():assert hashlib.sha256((ROOT/'source'/n).read_bytes()).hexdigest()==h,n
handoff=hashlib.sha256((ROOT/'source/docs/handoff.md').read_bytes()).hexdigest();assert handoff=='8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216'
one=json.loads((runtime/'one-bad.complete.json').read_text());bad=json.loads((runtime/'all-bad.complete.json').read_text());assert one['run_id']==bad['run_id']==report['run_id']
ids={}
for which,roles in [('a',('meta','node')),('b',('node',))]:
 ids[which]=f.live_identity(which,roles,'live-'+which+'.json');f.assert_identity_matches(ids[which],report['final_identities'][which],'final live '+which)
probe=r'''
import hashlib,json,os,pathlib,tomllib,urllib.parse,urllib.request
r=pathlib.Path(RUN);cfg=tomllib.loads((r/'etc/node.toml').read_text());assert cfg['dfs_desired_copies']==2 and cfg['dfs_sync_required_copies']==1
assert cfg['data_mode']=='rdma' and cfg['rdma_device']=='rxe0'
result={'config_replication':{'N':cfg['dfs_desired_copies'],'M':cfg['dfs_sync_required_copies']},'chunks':{}}
for chunk,name,expected in [(ONE_CHUNK,ONE_FILE,ONE_SHA),(BAD_CHUNK,BAD_FILE,None)]:
 p=r/'state/node/dfs/chunks'/chunk;entries=[json.loads(line) for line in (r/'state/node/dfs/catalog.wal').read_text().splitlines() if line.strip()];records=[record for t in entries for record in t['records'] if record['chunk']['id']==chunk]
 item={'physical_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'catalog_states':[x['state'] for x in records],'catalog_revisions':[x['catalog_revision'] for x in records]}
 try:
  fd=os.open(r/'mount-dfs'/name,os.O_RDONLY)
  try:data=os.read(fd,65536)
  finally:os.close(fd)
  item['fuse']={'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
 except OSError as e:item['fuse']={'errno':e.errno,'returned_bytes':0}
 if expected:
  assert item['physical_sha256']==expected and item['fuse']['sha256']==expected and item['fuse']['bytes']==65536 and records[-1]['state']=='Durable',item
 else:assert item['fuse']['errno']==5 and records[-1]['state']=='Quarantined',item
 result['chunks'][chunk]=item
print(json.dumps(result))
'''
proofs={}
for which in ('a','b'):
 code=f.py_assignment(RUN=r.RUN[which],ONE_CHUNK=one['chunk'],ONE_FILE=one['file'],ONE_SHA=one['expected_sha256'],BAD_CHUNK=bad['chunk'],BAD_FILE=bad['file'])+probe
 proofs[which]=json.loads(f.guest_py(which,code,guest_timeout=25));f.dump_unique('physical-fuse-'+which+'.json',proofs[which])
rest=r'''
import json,urllib.parse,urllib.request
rows={}
for chunk,available,state in [(ONE,2,'Completed'),(BAD,0,'BlockedNoSource')]:
 with urllib.request.urlopen('http://127.0.0.1:'+str(PORT)+'/v1/dfs/chunks/'+urllib.parse.quote(chunk,safe='')+'/replication',timeout=3) as response:row=json.loads(response.read())
 assert row['available_copies']==available and not row['loss_confirmed'] and any(x['state']==state for x in row['tasks']),row
 rows[chunk]=row
print(json.dumps(rows))
'''
rest_rows=json.loads(f.guest_py('a',f.py_assignment(ONE=one['chunk'],BAD=bad['chunk'],PORT=r.PORTS['meta_rest'])+rest,guest_timeout=12));f.dump_unique('live-rest.json',rest_rows)
controller_sha=hashlib.sha256((ROOT/'source/scripts/deploy/afs-processctl').read_bytes()).hexdigest();controllers={}
for which in ('a','b'):
 code=f.py_assignment(PATH=r.RUN[which]+'/prefix/bin/afs-processctl')+"import pathlib,hashlib,json;print(json.dumps({'sha256':hashlib.sha256(pathlib.Path(PATH).read_bytes()).hexdigest()}))"
 controllers[which]=json.loads(f.guest_py(which,code,guest_timeout=10));assert controllers[which]['sha256']==controller_sha
f.dump_unique('controller-identities.json',controllers)
initial=json.loads((runtime/'metrics-b-initial.json').read_text());cold=json.loads((runtime/'metrics-b-after-cold-read.json').read_text());repaired=json.loads((runtime/'metrics-a-after-auto-repair.json').read_text())
assert initial['node_pid']==cold['node_pid'] and initial['start_ticks']==cold['start_ticks']
assert initial['counters']['rdma/recv/replica']>=1048576
assert cold['counters']['rdma/send/read']-initial['counters']['rdma/send/read']>=65536
restart=json.loads((runtime/'identity-a-one-bad-restarted.json').read_text())
assert repaired['node_pid']==restart['processes']['node']['pid'] and repaired['start_ticks']==restart['processes']['node']['start_ticks']
assert repaired['counters']['rdma/recv/replica']>=65536
for snapshot in (initial,cold,repaired):
 assert snapshot['counters']['grpc/recv/replica']==snapshot['counters']['grpc/send/read']==0
 assert 'state ACTIVE' in snapshot['rdma_links']
f.dump_unique('rdma-counter-audit.json',{'status':'PASS','initial_async_replica':initial,'cold_peer_read':cold,'corrupt_target_repair':repaired,'limits':'per-incarnation product counters recorded after actual verbs completion; no hardware performance/resource-lifetime claim'})
result={'status':'PASS','level':'STAGE_GATE','run_id':report['run_id'],'source_input_count':143,'live_identities':ids,'live_physical_fuse':proofs,'handoff_sha256':handoff,'controller_sha256':controller_sha,'scope':'frozen full Linux source gate + fresh short memory/mTLS/RXE-required N2/M1 corruption/repair/EIO integration; formal matrix still NOT_RUN and ENV PREPARING'}
f.dump_unique('report.json',result);print(json.dumps({'status':'PASS','level':'STAGE_GATE','source_input_count':143,'run_id':report['run_id']}))

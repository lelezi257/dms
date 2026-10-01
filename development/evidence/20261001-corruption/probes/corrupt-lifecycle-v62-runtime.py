#!/usr/bin/env python3
"""Short real Linux FUSE corruption/repair probe, not the formal REL-09 matrix."""
import argparse,hashlib,importlib.util,json,pathlib,time,uuid
ROOT=pathlib.Path(__file__).resolve().parents[3]
def load(name,p):
 spec=importlib.util.spec_from_file_location(name,p);assert spec and spec.loader
 m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
r=load('runtime',ROOT/'experiments/afs-acceptance/repair-runtime-v62.py').runtime
f=load('fault',ROOT/'experiments/afs-acceptance/repair-fault-v56.py');f.v55=r
parser=argparse.ArgumentParser();parser.add_argument('--attempt',default='r2');args=parser.parse_args()
out=ROOT/('evidence/afs-delivery/corrupt-lifecycle-v62-runtime-'+args.attempt);out.mkdir(exist_ok=False);f.OUT=out
stage=json.loads((r.OUT/'staged-runtime-binary-sha256.json').read_text());f.NODE_SHA=stage['a']['node'];f.META_SHA=stage['a']['meta']
manifest=json.loads((ROOT/'evidence/afs-delivery/corrupt-lifecycle-v62-r3/compile-inputs.json').read_text())['files']
for n,h in manifest.items():assert hashlib.sha256((ROOT/'source'/n).read_bytes()).hexdigest()==h,n
run=str(uuid.uuid4());f.dump_unique('run-start.json',{'run_id':run,'status':'RUNNING','source_inputs':len(manifest),'level':'AFFECTED_INTEGRATION','transport':'gRPC/mTLS','backend':'memory','N':2,'M':1})
def record(which,code,label,budget=25):
 result=json.loads(f.guest_py(which,code,guest_timeout=budget));f.dump_unique(label+'.json',result);return result
inspect=r'''
import hashlib,json,pathlib
r=pathlib.Path(RUN);p=r/'state/node/dfs/chunks'/CHUNK
transactions=[json.loads(line) for line in (r/'state/node/dfs/catalog.wal').read_text().splitlines() if line.strip()]
records=[dict(record,txn_revision=t['revision']) for t in transactions for record in t['records'] if record['chunk']['id']==CHUNK]
print(json.dumps({'chunk':CHUNK,'physical_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'records':records,'physical_inode':p.stat().st_ino}))
'''
inject=r'''
import hashlib,json,os,pathlib
r=pathlib.Path(RUN);p=r/'state/node/dfs/chunks'/CHUNK;b=p.read_bytes()
assert hashlib.sha256(b).hexdigest()==SHA and len(b)==65536
folder=r/'corruption-backups';folder.mkdir(exist_ok=True);backup=folder/BACKUP
with backup.open('xb') as fh:fh.write(b);fh.flush();os.fsync(fh.fileno())
ino=p.stat().st_ino
with p.open('r+b') as fh:fh.seek(8192);fh.write(bytes([b[8192]^255]));fh.flush();os.fsync(fh.fileno())
changed=p.read_bytes();assert len(changed)==len(b) and changed!=b and p.stat().st_ino==ino
print(json.dumps({'chunk':CHUNK,'before_sha256':SHA,'after_sha256':hashlib.sha256(changed).hexdigest(),'inode':ino,'bytes':len(changed),'backup':str(backup)}))
'''
read=r'''
import hashlib,json,os,pathlib
p=pathlib.Path(RUN)/'mount-dfs'/FILE
try:
 fd=os.open(p,os.O_RDONLY)
 try:data=os.read(fd,65536)
 finally:os.close(fd)
 result={'read':'OK','bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
except OSError as e:result={'read':'ERROR','errno':e.errno,'message':str(e),'returned_bytes':0}
print(json.dumps(result))
'''
def assignment(which,created,file):return f.py_assignment(RUN=r.RUN[which],CHUNK=created['chunk']['name'],SHA=created['sha256'],FILE=file,BACKUP=run+'-'+file+'.chunk')
def create(file,seed):
 code=f.py_assignment(RUN=r.RUN['a'],FILE_NAME=file,SIZE=65536)+f.CREATE_SOURCE_FILE_CODE.replace('((i*193)+41)%251',f'((i*193)+{list(hashlib.sha256(run.encode()).digest())}[(i//251)%32]+{seed})%251')
 x=record('a',code,'create-'+file);f.wait_rest(x['chunk']['name'],'available2_completed',45,'two-copies-'+file+'.json');return x
baseline={which:f.live_identity(which,roles,'identity-'+which+'-initial.json') for which,roles in [('a',('meta','node')),('b',('node',))]}
try:
 file='one-bad-'+run+'.bin';created=create(file,117)
 # Restart only the owned Node A after physical mutation. Its new FUSE mount
 # has no cached pages/pins, so the first A read reaches the local chunk path.
 f.write_unique('stop-a-before-one-bad.stdout',f.ctl('a','stop','node',timeout=40))
 receipt=record('a',assignment('a',created,file)+inject,'one-bad-injection')
 f.write_unique('start-a-one-bad.stdout',f.ctl('a','start','node',timeout=45))
 after=f.live_identity('a',('meta','node'),'identity-a-one-bad-restarted.json')
 assert after['processes']['meta']==baseline['a']['processes']['meta']
 assert after['processes']['node']['pid']!=baseline['a']['processes']['node']['pid']
 result=record('a',assignment('a',created,file)+read,'one-bad-first-cold-fuse-read')
 assert result['read']=='OK' and result['bytes']==65536 and result['sha256']==created['sha256'],result
 # A prior Completed Meta task can still be visible before this report
 # arrives. Wait for the new physical bytes AND quarantine/replacement
 # journal first; only then accept restored Meta health.
 deadline=time.monotonic()+45;poll=0
 while True:
  physical=record('a',assignment('a',created,file)+inspect,'one-bad-physical-poll-'+str(poll));poll+=1
  states=[x['state'] for x in physical['records']]
  if physical['physical_sha256']==created['sha256'] and physical['physical_inode']!=receipt['inode'] and 'Quarantined' in states and states[-1]=='Durable':break
  assert time.monotonic()<deadline,physical
  time.sleep(1)
 f.dump_unique('one-bad-auto-repaired-physical.json',physical)
 f.wait_rest(created['chunk']['name'],'available2_completed',45,'one-bad-auto-repaired-rest.json')
 states=[x['state'] for x in physical['records']];assert 'Quarantined' in states and states[-1]=='Durable',states
 assert physical['records'][-1]['catalog_revision']>next(x['catalog_revision'] for x in physical['records'] if x['state']=='Quarantined')
 f.dump_unique('one-bad.complete.json',{'status':'PASS','run_id':run,'file':file,'chunk':created['chunk']['name'],'expected_sha256':created['sha256'],'cold_read':result,'automatic_cow_repair':True})
 # Distinct payload prevents reuse of the repaired Chunk. Corrupt both exact
 # physical copies while both owned Nodes are stopped, then use fresh mounts.
 file2='all-bad-'+run+'.bin';created2=create(file2,153)
 for which in ('a','b'):f.write_unique('stop-'+which+'-before-all-bad.stdout',f.ctl(which,'stop','node',timeout=40))
 for which in ('a','b'):record(which,assignment(which,created2,file2)+inject,'all-bad-'+which+'-injection')
 for which in ('a','b'):f.write_unique('start-'+which+'-all-bad.stdout',f.ctl(which,'start','node',timeout=45))
 failed=record('a',assignment('a',created2,file2)+read,'all-bad-first-cold-fuse-read')
 assert failed['read']=='ERROR' and failed['errno']==5 and failed['returned_bytes']==0,failed
 code=f.REST_WAIT_CODE.replace("    raise AssertionError(MODE)","    if MODE == 'all_bad': return payload.get('available_copies') == 0 and any(state == 'BlockedNoSource' for state in states)\n    raise AssertionError(MODE)")
 rest=record('a',f.py_assignment(CHUNK_ID=created2['chunk']['name'],MODE='all_bad',BOUND=30,META_REST=r.PORTS['meta_rest'])+code,'all-bad-rest-blocked',35)
 assert not rest['polls'][-1]['json']['loss_confirmed']
 for which in ('a','b'):
  state=record(which,assignment(which,created2,file2)+inspect,'all-bad-'+which+'-physical')
  assert state['records'][-1]['state']=='Quarantined'
 f.dump_unique('all-bad.complete.json',{'status':'PASS','run_id':run,'file':file2,'chunk':created2['chunk']['name'],'read':failed,'state':'BlockedNoSource','permanent_loss_not_inferred':True})
 final={which:f.live_identity(which,roles,'identity-'+which+'-final.json') for which,roles in [('a',('meta','node')),('b',('node',))]}
 assert final['a']['processes']['meta']==baseline['a']['processes']['meta']
 report={'status':'PASS','level':'AFFECTED_INTEGRATION','run_id':run,'source_inputs':len(manifest),'scenarios':['one bad copy cold FUSE fallback and automatic COW repair','all bad copies EIO/zero bytes/persisted quarantine/BlockedNoSource'],'limits':'short memory/mTLS/gRPC N2/M1 only; no durable backend, RXE, full REL-09 or long stability/performance claim','final_identities':final}
 f.dump_unique('report.json',report);print(json.dumps({'status':'PASS','run_id':run,'level':report['level']}))
except BaseException as e:
 f.dump_unique('report.json',{'status':'FAIL','level':'AFFECTED_INTEGRATION','run_id':run,'error':repr(e),'scope':'raw failures preserved; no scenario exclusion or silent restoration'})
 raise

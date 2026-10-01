#!/usr/bin/env python3
"""Linux-only affected-module validation; not a complete stage/release gate."""
import hashlib,json,os,pathlib,platform,shutil,subprocess,tarfile,time
assert platform.system()=='Linux'
b=pathlib.Path('/home/lzc.guest/afs-build'); root=b/'work/root-merged-v61-local-r1'; out=b/'probes/corrupt-reader-v61-local-r1'
out.mkdir(exist_ok=False); shutil.copytree(b/'work/root-merged-v60-r2',root,ignore=shutil.ignore_patterns('target','.git','._*'))
names=['src/node/chunk.rs','src/node/dfs_read.rs','src/node/replication.rs','src/node/rpc/data.rs']
with tarfile.open(b/'probes/corrupt-reader-v61-inputs.tar') as archive:
 assert sorted(archive.getnames())==sorted(names)
 archive.extractall(root,filter='data')
env=os.environ.copy();env.update(PATH='/home/lzc.guest/.cargo/bin:'+env['PATH'],CARGO_TARGET_DIR=str(b/'target'),CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0')
def hashes():
 inputs=json.loads((b/'probes/repair-v56/input-names.json').read_text())
 return {name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in inputs}
(out/'inputs-before-format.json').write_text(json.dumps(hashes(),indent=2)+'\n')
with (out/'format.log').open('w') as log:
 rc=subprocess.run(['timeout','60','cargo','fmt','--all'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
assert rc==0,rc
initial=hashes();(out/'compile-inputs.json').write_text(json.dumps({'file_count':len(initial),'files':initial},indent=2)+'\n')
checks=[]
def run(label,args,budget=180):
 started=time.monotonic()
 with (out/(label+'.log')).open('w') as log:
  p=subprocess.run(['timeout',str(budget)]+args,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
 row={'label':label,'argv':['timeout',str(budget)]+args,'returncode':p.returncode,'seconds':time.monotonic()-started};checks.append(row)
 (out/'checks.json').write_text(json.dumps(checks,indent=2)+'\n'); print(json.dumps(row),flush=True)
 assert hashes()==initial,'frozen source changed'
 assert p.returncode==0,row
for label,selection in [('original-warm','diagnostic_warm_corrupted_local_copy_uses_healthy_peer'),('original-cold','diagnostic_cold_corrupted_local_copy_uses_healthy_peer'),('chunk','node::chunk::'),('dfs-read','node::dfs_read::'),('replication','node::replication::'),('rpc-data','node::rpc::data::tests::'),('rpc-peer','node::rpc::peer::tests::')]:
 run(label,['cargo','test','--all-features','--lib',selection,'--','--nocapture'])
run('affected-clippy',['cargo','clippy','-p','afs','--all-features','--lib','--tests','--','-D','warnings'])
run('fmt',['cargo','fmt','--all','--','--check'],60)
report={'status':'PASS','level':'LOCAL_REGRESSION','platform':platform.platform(),'input_count':len(initial),'checks':checks,'scope':'original failures, affected chunk/read/replication and localhost peer authority/stream contracts; no complete Linux stage gate, actual RXE/FUSE corruption or formal acceptance'}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True)

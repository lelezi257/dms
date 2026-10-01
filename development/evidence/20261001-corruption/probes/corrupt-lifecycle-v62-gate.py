#!/usr/bin/env python3
"""Linux-only impact-based gates for a frozen corruption lifecycle batch."""
import argparse,hashlib,json,os,pathlib,platform,re,shutil,subprocess,tarfile,time
assert platform.system()=='Linux'
p=argparse.ArgumentParser();p.add_argument('phase',choices=['prepare','local','full']);p.add_argument('--attempt',default='r1');a=p.parse_args()
b=pathlib.Path('/home/lzc.guest/afs-build');root=b/('work/root-merged-v62-'+a.attempt);parent=b/('probes/corrupt-lifecycle-v62-'+a.attempt)
names=json.loads((b/'probes/repair-v56/input-names.json').read_text())
env=os.environ.copy();env.update(PATH='/home/lzc.guest/.cargo/bin:'+env['PATH'],CARGO_TARGET_DIR=str(b/'target'),CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0')
def hashes():return {name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in names}
if a.phase=='prepare':
 parent.mkdir(exist_ok=False);shutil.copytree(b/'work/root-merged-v60-r2',root,ignore=shutil.ignore_patterns('target','.git','._*'))
 archive_path=b/'probes/corrupt-lifecycle-v62-inputs.tar'
 with tarfile.open(archive_path) as archive:
  assert sorted(archive.getnames())==sorted(names);archive.extractall(root,filter='data')
 (parent/'inputs-before-format.json').write_text(json.dumps(hashes(),indent=2)+'\n')
 with (parent/'format.log').open('w') as log:rc=subprocess.run(['timeout','60','cargo','fmt','--all'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
 assert rc==0,rc
 initial=hashes();(parent/'compile-inputs.json').write_text(json.dumps({'file_count':len(initial),'files':initial},indent=2)+'\n')
 with tarfile.open(parent/'formatted-inputs.tar','w') as archive:
  for name in names:archive.add(root/name,arcname=name)
 print(json.dumps({'status':'PREPARED','inputs':len(initial),'root':str(root)}));raise SystemExit(0)
initial=json.loads((parent/'compile-inputs.json').read_text())['files'];assert hashes()==initial
out=parent/a.phase;out.mkdir(exist_ok=False);checks=[]
def run(label,args,budget=240):
 started=time.monotonic()
 with (out/(label+'.log')).open('w') as log:rc=subprocess.run(['timeout',str(budget)]+args,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
 row={'label':label,'argv':['timeout',str(budget)]+args,'returncode':rc,'seconds':time.monotonic()-started};checks.append(row)
 (out/'checks.json').write_text(json.dumps(checks,indent=2)+'\n');print(json.dumps(row),flush=True)
 assert hashes()==initial,'frozen source changed'
 if args[:2]==['cargo','test'] and '--no-run' not in args:
  summaries=re.findall(r'test result: ok\. (\d+) passed', (out/(label+'.log')).read_text())
  if not summaries or sum(map(int,summaries))==0:rc=1;row['semantic_error']='no selected tests executed'
 if rc!=0:
  (out/'report.json').write_text(json.dumps({'status':'FAIL','phase':a.phase,'checks':checks},indent=2)+'\n');raise SystemExit(1)
if a.phase=='local':
 for label,selection in [('original-warm','diagnostic_warm_corrupted_local_copy_uses_healthy_peer'),('original-cold','diagnostic_cold_corrupted_local_copy_uses_healthy_peer'),('original-catalog-retry','corrupt_cataloged_chunk_is_replaced_by_immutable_retry'),('chunk','node::chunk::'),('dfs-read','node::dfs_read::'),('replication','node::replication::'),('meta-repair','meta::dfs::read_recovery_tests::repair'),('meta-corruption','meta::dfs::read_recovery_tests::corruption'),('rpc-data','node::rpc::data::tests::'),('rpc-peer','node::rpc::peer::tests::')]:
  run(label,['cargo','test','--all-features','--lib',selection,'--','--nocapture'])
 run('affected-clippy',['cargo','clippy','-p','afs','--all-features','--lib','--tests','--','-D','warnings'])
 run('fmt',['cargo','fmt','--all','--','--check'],60)
 report={'status':'PASS','level':'LOCAL_REGRESSION','scope':'failure replays and affected library/Meta/RPC contracts; no full source or formal acceptance','checks':checks,'platform':platform.platform(),'input_count':len(initial)}
else:
 assert json.loads((parent/'local/report.json').read_text())['status']=='PASS'
 run('fmt',['cargo','fmt','--all','--','--check'],60)
 run('clippy',['cargo','clippy','--workspace','--all-targets','--all-features','--','-D','warnings'])
 run('library',['cargo','test','--all-features','--lib','--','--nocapture'])
 run('contracts',['cargo','test','--all-features','--test','config_contract','--test','error_contract','--test','fuse_contract','--test','meta_contract','--test','ownerfs_peer_contract','--test','rest_contract','--test','vfs_contract','--','--nocapture'])
 run('error',['cargo','test','-p','afs-error','--','--nocapture'],180)
 run('local-api',['cargo','test','--all-features','--test','local_sdk','--','--nocapture'],180)
 run('fuse-build',['cargo','test','--all-features','--test','fuse_contract','--no-run','--message-format=json'],120)
 artifacts=[]
 for line in (out/'fuse-build.log').read_text().splitlines():
  try:item=json.loads(line)
  except json.JSONDecodeError:continue
  if item.get('reason')=='compiler-artifact' and item.get('target',{}).get('name')=='fuse_contract' and item.get('executable'):artifacts.append(item['executable'])
 assert len(artifacts)==1,artifacts
 run('real-root-fuse',['sudo',artifacts[0],'--ignored','--test-threads=1','--nocapture'],120)
 for label,feature in [('feature-none',None),('feature-ownerfs','ownerfs'),('feature-dfs','dfs')]:
  args=['cargo','check','--no-default-features']
  if feature:args+=['--features',feature]
  run(label,args,120)
 run('build',['cargo','build','--all-features','--bins'],180)
 destination=b/('artifacts/v62-'+a.attempt+'-qualified');destination.mkdir(exist_ok=False);binaries={}
 for role in ['afs-node','afs-meta']:
  shutil.copy2(b/'target/debug'/role,destination/role);binaries[role]=hashlib.sha256((destination/role).read_bytes()).hexdigest()
 report={'status':'PASS','level':'SOURCE_GATE','scope':'full Linux source gate; affected real multi-VM corruption integration and formal acceptance tracked separately','checks':checks,'platform':platform.platform(),'input_count':len(initial),'binary_sha256':binaries,'artifacts':str(destination)}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True)

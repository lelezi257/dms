"""Linux semantic audit of the final representative round-2 regression."""
import copy,hashlib,importlib.util,json,pathlib,re,sys
assert sys.platform=='linux'
checks=[]
def audit(root):
 def check(name,value):
  assert value,name
  checks.append(name)
 def read(path):return json.loads((root/path).read_text())
 final=root/'build/dfs-deadline-v82-final';r3=root/'build/dfs-deadline-v82-r3'
 exits=list((final/'full').rglob('*.exit'))
 check('14 complete gate exits',len(exits)==14 and all(p.read_text().strip()=='0' for p in exits))
 for file,count in [('lib',421),('error',4),('local-api',9),('fuse',5)]:
  check(file+' tests',f'test result: ok. {count} passed; 0 failed;' in (final/f'full/gate/{file}.log').read_text())
 lib=(final/'full/gate/lib.log').read_text()
 check('seven explicit lib ignores','421 passed; 0 failed; 7 ignored;' in lib)
 for name in ['real_grpc_unknown_file_commit_ack_blocks_inode_and_replays_exact_request','repair_worker_replays_real_grpc_claim_and_report_after_lost_ack']:
  check(name,re.search(re.escape(name)+r' \.\.\. ok',lib) is not None)
 matches=re.findall(r'test result: ok\. (\d+) passed; 0 failed;', (final/'full/gate/contracts.log').read_text())
 check('65 interface contracts',sum(map(int,matches))==65)
 for p in (final/'related').glob('*.exit'):check(p.stem,p.read_text().strip()=='0')
 check('four actual RDMA related cases',len(list((final/'related').glob('*.exit')))==4)
 check('runner success',(r3/'runner.exit').read_text().strip()=='0')
 spec=importlib.util.spec_from_file_location('checker',root/'probes/check-rdma-cancellation.py');checker=importlib.util.module_from_spec(spec);spec.loader.exec_module(checker)
 v=checker.audit(r3/'cancel','rxe0','dfs-deadline')
 check('real posted deadline audit',v['status']=='PASS' and v['caller_error']['kind']=='DeadlineExceeded')
 check('explicit fixture registry close', 'close=EXPLICIT' in (r3/'cancel/gdb.log').read_text())
 for path,count in [('build/dfs-deadline-v82-r3/checker-regression.json',27),('build/dfs-deadline-v82-final/owner-checker.json',24),('build/dfs-deadline-v82-final/diagnostic-checker.json',20)]:
  value=read(path);check(path,value['status']=='PASS' and value['count']==count)
 inputs=read('build/dfs-deadline-v82-final/input-audit.json')
 check('test-only delta',inputs['status']=='PASS' and inputs['compiler_inputs']==143 and inputs['changed_inputs']==['src/node/rpc/data.rs'] and inputs['production_scope']=='cfg(test) additions/imports only')
 check('retained original compile failures',all((root/f'build/dfs-deadline-v82-r{i}/compile.exit').read_text().strip()!='0' for i in [1,2]))
 node='a3fe6573fc5f5a41c30b823855cfe2fdd1428950f878f1e29756e7bfc0d7e9d9'
 expected='05f451384fb2ef7425a61766a6a6354c2b576816f42ba7bf284c23a47b02d742'
 identities={}
 for n in ['a','b']:
  def result(name):
   v=read(f'overall/{n}/evidence/{name}.json');check(n+'/'+name,v['status']=='PASS');return v['result']
  before=result('start-r1')['identity'];after=result('restart-r1')['identity'];identities[n]=(before,after)
  check(n+' actual restart',before['pid']!=after['pid'] and before['sha256']==after['sha256']==node and before['boot_id']==after['boot_id'])
  rd=result('cold-read-r1');check(n+' cold exact EOF',rd['read']=={'outcome':'success','bytes':65536,'sha256':expected,'eof':True} and rd['identity']==after)
  phys=result('physical-final');check(n+' physical durable',phys['sha256']==expected and phys['physical_inode']>0 and phys['records'][-1]['state']=='Durable')
  for name in ['metrics-r1','metrics-final']:
   met=result(name);counter=met['payload_counters'];check(n+'/'+name+' no fallback',counter['grpc/recv/replica']==counter['grpc/send/read']==0 and 'state ACTIVE' in met['rdma_links'])
  cleanup=read(f'overall/{n}/evidence/cleanup-r2.json');check(n+' cleanup',cleanup['status']=='PASS' and cleanup['mounts_gone'] and cleanup['data_available_bytes']>=4*1024**3 and cleanup['stopped_pid']==after['pid'] and cleanup['old_cohort']['health']['status']=='ready')
 check('actual RDMA payload',read('overall/b/evidence/metrics-r1.json')['result']['payload_counters']['rdma/recv/replica']>=65536)
 for i in [1,2]:
  v=read(f'overall/consistency-r{i}/report.json');check('consistency '+str(i),v['status']=='PASS' and v['summary']['steps']==v['summary']['passed']==10 and v['summary']['failed']==0 and v['identity']['cross_mount_qualified'])
  check('Linux workers '+str(i),all(x['ok'] for x in v['steps']) and all('afs-accept-' in ' '.join(x) for x in v['worker_commands'].values()))
 rep=read('overall/ctl/evidence/replicas-r1.json')['result']['polls'][-1]['value']
 check('two available durable copies',rep['available_copies']==2 and sum(x['available'] and x['record']['role']=='DurableReplica' and x['record']['state']=='Ready' for x in rep['copies'])==2)
 check('completed repair',any(x['state']=='Completed' for x in rep['tasks']))
 meta1=read('overall/ctl/evidence/start-r1.json')['result']['identity'];meta2=read('overall/ctl/evidence/identity-final.json')['result']['identity']
 check('continuous memory authority',meta1==meta2)
 for n in ['ctl','a','b','c']:
  v=read(f'sleep-recovery/{n}/after.json');check('preserved '+n,v['status']=='PASS' and v['health']['status']=='ready' and v['data_available_bytes']>=4*1024**3)
  check('preserved files '+n,all(f['length']==4194321 and f['sha256']=='7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231' and f['eof'] for f in v['files']))
  if n=='ctl':check('old memory meta not restarted',v['pid']==1418880)
  else:check(n+' recovered incarnation',int((root/f'sleep-recovery/{n}/pid-before').read_text())!=v['pid'] and 'session' in (root/f'sleep-recovery/{n}/node-before.log').read_text())
 check('forced sleep evidence','Clamshell Sleep' in (root/'overall/host-sleep.log').read_text())
 return {'status':'PASS','semantic_checks':len(checks),'round2':'REPRESENTATIVE_SCOPE_PASS','formal_acceptance':'NOT_RUN','environment':'PREPARING','checks':checks}
if __name__=='__main__':print(json.dumps(audit(pathlib.Path(sys.argv[1])),indent=2))

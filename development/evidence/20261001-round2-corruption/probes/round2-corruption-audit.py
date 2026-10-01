import copy,hashlib,json,pathlib,sys
assert sys.platform=='linux'
p=pathlib.Path(sys.argv[1]);s=pathlib.Path(sys.argv[2]);data={str(f.relative_to(p)):json.loads(f.read_text()) for f in p.glob('*/evidence/*.json')}
sha={'node':'a3fe6573fc5f5a41c30b823855cfe2fdd1428950f878f1e29756e7bfc0d7e9d9','meta':'895f39fd660b7f7d9735eaaa4f3a082c3692a409d954c4aa8b5b0cc515a700ad'}
def audit(d):
 checks=[]
 def check(name,condition):
  assert condition,name;checks.append(name)
 def r(which,label):return d[f'{which}/evidence/{label}.json']['result']
 one=r('a','create-one');allbad=r('a','create-all');check('distinct-exclusive-chunks',one['chunk']!=allbad['chunk'])
 for which in ('ctl','a','b'):
  prep=r(which,'prepare');bind=d[f'{which}/evidence/binding-final.json'];role='meta' if which=='ctl' else 'node'
  check(which+'-candidate',bind['identity']['sha256']==sha[role] and prep['candidate_hashes']==sha)
  check(which+'-exact-config',prep['config_sha256']==bind['configuration_sha256'] and bind['argv'][1:]==['--config',bind['configuration']['data_dir'].rsplit('/state/',1)[0]+'/etc/'+role+'.toml'])
  check(which+'-ready',bind['health']['status']=='ready')
  check(which+'-policy',bind['configuration']['dfs_desired_copies']==2 and bind['configuration']['dfs_sync_required_copies']==1)
  cleanup=d[f'{which}/evidence/cleanup.json'];check(which+'-stopped',cleanup['status']=='PASS' and cleanup['mounts_gone'] and r(which,'stop-final')['controller']['exit']==0)
  check(which+'-reserve-old-cohort',cleanup['data_available_bytes']>=4*1024**3 and cleanup['old_cohort']['health']['status']=='ready')
  if role=='node':
   check(which+'-required-rxe',bind['configuration']['data_mode']=='rdma' and bind['configuration']['rdma_device']=='rxe0')
   mounts=bind['mounts'];check(which+'-independent-FUSE',len(mounts)==2 and all(json.loads(v['stdout'])['filesystems'][0]['source']=='afs-'+k.removesuffix('_mount') for k,v in mounts.items()))
  else:check('central-continuous-memory-authority',bind['configuration']['meta_store']=='memory' and r('ctl','start')['identity']==bind['identity']==r('ctl','identity-final')['identity'])
 inj=r('a','inject-one');physical=r('a','repaired-one');cold=r('a','cold-one')
 check('one-physical-injection',inj['length']==65536 and inj['before_sha256']==one['sha256'] and inj['after_sha256']!=one['sha256'])
 check('one-cold-exact',cold['read']=={'outcome':'success','bytes':65536,'sha256':one['sha256'],'eof':True} and cold['identity']==r('a','start-one-bad')['identity'] and cold['identity']['pid']!=one['identity']['pid'])
 states=[r['state'] for r in physical['records']];check('one-new-physical-inode',physical['sha256']==one['sha256'] and physical['physical_inode']!=inj['physical_inode'])
 check('one-durable-quarantine-repair',states==['Durable','Quarantined','Durable'] and physical['records'][-1]['catalog_revision']>physical['records'][1]['catalog_revision'])
 rest=r('ctl','repaired-one-rest')['polls'][-1]['value'];check('one-meta-effective-copies',rest['available_copies']==2 and rest['health']=='Satisfied' and any(t['state']=='Completed' for t in rest['tasks']))
 before=r('b','metrics-initial');after=r('b','metrics-one-read');repaired=r('a','metrics-one-repaired')
 check('one-rdma-read-payload',before['identity']==after['identity'] and after['payload_counters']['rdma/send/read']-before['payload_counters']['rdma/send/read']>=65536)
 check('one-rdma-repair-payload',repaired['identity']==r('a','start-one-bad')['identity'] and repaired['payload_counters']['rdma/recv/replica']>=65536)
 check('zero-grpc-file-payload',all(m['payload_counters']['grpc/recv/replica']==m['payload_counters']['grpc/send/read']==0 for m in (before,after,repaired)))
 for which in ('a','b'):
  inj=r(which,'inject-all');physical=r(which,'quarantined-all')
  check(which+'-all-physical-corruption',inj['before_sha256']==allbad['sha256'] and inj['after_sha256']!=allbad['sha256'] and inj['length']==65536 and physical['sha256']==inj['after_sha256'])
  check(which+'-all-persisted-quarantine',physical['records'][-1]['state']=='Quarantined')
  for label in ('cold-all','bad-after-quarantine-restart'):
   x=r(which,label)['read'];check(which+'-'+label+'-EIO-zero-bytes',x['outcome']=='error' and x['errno']==5 and x['returned_bytes']==0)
  for label in ('healthy-after-all-bad','healthy-after-quarantine-restart'):
   x=r(which,label)['read'];check(which+'-'+label+'-exact',x=={'outcome':'success','bytes':65536,'sha256':one['sha256'],'eof':True})
  check(which+'-new-incarnation-after-quarantine',r(which,'restart-quarantined')['identity']['pid']!=r(which,'start-all-bad')['identity']['pid'])
 rest=r('ctl','blocked-all-rest-confirmed')['polls'][-1]['value'];check('all-meta-zero-BlockedNoSource',rest['available_copies']==0 and rest['health']=='BlockedNoSource' and any(t['state']=='BlockedNoSource' for t in rest['tasks']))
 check('no-permanent-loss-inferred',rest['loss_confirmed'] is False)
 early=r('ctl','blocked-all-rest')['polls'][-1]['value'];check('early-stale-healthy-not-qualified',early['available_copies']==2 and early['health']=='Satisfied')
 return checks
checks=audit(data);negative=[]
for label,path,edit in [
 ('bad-bytes-returned','a/evidence/cold-all.json',lambda x:x['result']['read'].update(outcome='success',returned_bytes=65536)),
 ('wrong-cold-checksum','a/evidence/cold-one.json',lambda x:x['result']['read'].update(sha256='wrong')),
 ('gRPC-fallback','b/evidence/metrics-one-read.json',lambda x:x['result']['payload_counters'].update({'grpc/send/read':65536})),
 ('no-physical-repair','a/evidence/repaired-one.json',lambda x:x['result'].update(physical_inode=data['a/evidence/inject-one.json']['result']['physical_inode'])),
 ('changed-Meta','ctl/evidence/identity-final.json',lambda x:x['result']['identity'].update(pid=1)),
 ('bad-copy-marked-available','ctl/evidence/blocked-all-rest-confirmed.json',lambda x:x['result']['polls'][-1]['value'].update(available_copies=1))]:
 d=copy.deepcopy(data);edit(d[path])
 try:audit(d)
 except AssertionError:negative.append({'case':label,'rejected':True})
 else:raise AssertionError('negative accepted: '+label)
inputs=json.loads((s/'development/evidence/20261001-owner-sync-eio/build/compile-inputs-after.json').read_text())['files'];assert all(hashlib.sha256((s/n).read_bytes()).hexdigest()==h for n,h in inputs.items())
log=(s/'development/evidence/20261001-owner-sync-eio/build/full/gate/lib.log').read_text()
for t in ('real_grpc_unknown_file_commit_ack_blocks_inode_and_replays_exact_request','repair_worker_replays_real_grpc_claim_and_report_after_lost_ack'):assert '::'+t+' ... ok' in log
result={'status':'PASS','level':'SCOPED_FAULT_INTEGRATION','checks':checks,'negative_tests':negative,'compiler_inputs':len(inputs),'reused_source_gate':'20261001-owner-sync-eio','current_source_exact_unknown_result_tests':'PASS in source gate; plain-loopback/memory/R1 scope','round2':'INCOMPLETE','formal_acceptance':'NOT_RUN'}
print(json.dumps(result,indent=2))

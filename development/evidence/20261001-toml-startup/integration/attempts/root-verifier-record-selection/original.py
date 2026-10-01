from pathlib import Path
import subprocess,json,hashlib
root=Path('/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store'); out=root/'evidence/afs-delivery/p2-integration-v51'; stage=json.loads((out/'stage.json').read_text()); expected={p['pid']:p for x in stage['identities'].values() for p in x['processes'].values()}; count=0
for name,steps in [('consistency',10),('ownerfs-cross',7),('dfs-cross',7)]:
 report=json.loads((out/name/'report.json').read_text()); assert report['status']=='PASS' and report['failures']==[] and report['summary']['passed']==report['summary']['steps']==steps and report['summary']['failed']==0
 assert (report.get('cross_mount_qualified') or report.get('identity',{}).get('cross_mount_qualified')) is True,name
 if name!='consistency':
  assert report['timeout_config']['min_wait_seconds']==35 and report['timeout_config']['child_timeout_seconds']==55
 def walk(obj):
  if isinstance(obj,dict):
   if 'pid' in obj and 'sha256' in obj:
    yield obj
   for v in obj.values(): yield from walk(v)
  elif isinstance(obj,list):
   for v in obj: yield from walk(v)
 for record in walk(report):
  p=expected[record['pid']]; assert record['sha256']==p['sha256']; assert str(record['start_ticks'])==p['start_ticks']; assert record.get('exe_path',record.get('exe'))==p['exe']; count+=1
fresh={}
for k,ident in stage['identities'].items():
 r=stage['runtime'][k]
 code=f'''import pathlib,json,hashlib,os,subprocess,urllib.request
r=pathlib.Path({r!r}); expected={ident!r}; result={{'processes':{{}},'old_processes':{{}}}}
assert pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()==expected['boot_id']
for group in ('processes','old_processes'):
 for role,p in expected[group].items():
  proc=pathlib.Path('/proc')/str(p['pid']); ticks=(proc/'stat').read_text().split(') ')[1].split()[19]; sha=hashlib.sha256((proc/'exe').read_bytes()).hexdigest(); exe=os.readlink(proc/'exe')
  assert ticks==p['start_ticks'] and sha==p['sha256'] and exe==p['exe'],(role,ticks,sha,exe)
  result[group][role]={{'pid':p['pid'],'start_ticks':ticks,'sha256':sha,'exe':exe}}
for role,sha in expected['config_sha256'].items(): assert hashlib.sha256((r/'etc'/(role+'.toml')).read_bytes()).hexdigest()==sha
assert hashlib.sha256((r/'prefix/bin/afs-processctl').read_bytes()).hexdigest()=={stage['controller_sha256']!r}
for kind in ('ownerfs','dfs'):
 m=json.loads(subprocess.check_output(['findmnt','-J','-M',str(r/('mount-'+kind)),'-o','TARGET,SOURCE,FSTYPE'],text=True))['filesystems'][0]; assert m['target']==str(r/('mount-'+kind)) and m['source']=='afs-'+kind and m['fstype'].startswith('fuse')
ports={([18281,18283] if k=='a' else [18285])!r}; result['health']={{str(p):json.load(urllib.request.urlopen('http://127.0.0.1:'+str(p)+'/health',timeout=2)) for p in ports}}
assert all(v['status']=='ready' for v in result['health'].values())
result['runtime_record_files']={{str(p.relative_to(r/'run')):p.read_text() for p in (r/'run').rglob('*') if p.is_file()}}
result['worker_sha256']={{n:hashlib.sha256((pathlib.Path('/var/tmp/afs-v51-probes')/n).read_bytes()).hexdigest() for n in ('consistency_cross.py','locks_cross.py','locks_smoke.py')}}
print(json.dumps(result))'''
 p=subprocess.run(['limactl','shell','afs-accept-'+k,'--','sudo','python3','-c',code],capture_output=True,text=True,timeout=45); assert p.returncode==0,(k,p.stdout,p.stderr)
 fresh[k]=json.loads(p.stdout)
 for n,sha in fresh[k]['worker_sha256'].items(): assert hashlib.sha256((root/'experiments/afs-acceptance/probes'/n).read_bytes()).hexdigest()==sha,n
inputs=json.loads((root/'source/development/evidence/20261001-shutdown-signal/source/qualified-linux/compile-inputs.json').read_text())
for name,digest in inputs['files'].items(): assert hashlib.sha256((root/'source'/name).read_bytes()).hexdigest()==digest,name
assert stage['controller_sha256']==hashlib.sha256((root/'source/scripts/deploy/afs-processctl').read_bytes()).hexdigest()
assert hashlib.sha256((root/'source/docs/handoff.md').read_bytes()).hexdigest()=='8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216'
result={'status':'PASS','captured_product_records':count,'rust_compile_inputs_matching':len(inputs['files']),'stage':stage,'fresh_guest_check':fresh,'handoff_unchanged':True,'scope':'identified memory R1/gRPC A/B development matrix only; no full release gate'}
(out/'root-verification.json').write_text(json.dumps(result,indent=2)+'\n')
print('Root verified',count,'product identities,143 inputs, both live environments and preserved v48 identities')

#!/usr/bin/env python3
"""Mac host orchestration only; real probes and product I/O run in Linux."""
import hashlib,json,pathlib,shlex,subprocess,sys,tempfile,time
ROOT=pathlib.Path(__file__).resolve().parents[2]
OUT=ROOT/'evidence/afs-delivery/p2-integration-v51'
SHA={'node':'97d735243d741ef98d917f05a965c9e02703b3b05468ba42e7b1d7c5f2c38e31','meta':'00cb99fe46f20024ab3b476978dd0e16d9e8a2c0535b95d5c8febc3aa863cf2d'}
RUN={'a':'/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v51','b':'/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v51'}
OLD={k:v.replace('v51','v48') for k,v in RUN.items()}
CTRL=ROOT/'source/scripts/deploy/afs-processctl'
def call(argv,timeout=120):
 p=subprocess.run(argv,text=True,capture_output=True,timeout=timeout)
 if p.returncode: raise RuntimeError(json.dumps({'argv':argv,'code':p.returncode,'stdout':p.stdout,'stderr':p.stderr}))
 return p.stdout
def guest(k,code,timeout=120):
 return call(['limactl','shell','afs-accept-'+k,'--','sudo','python3','-c',code],timeout)
def ctl(k,*args):
 r=RUN[k]; return call(['limactl','shell','afs-accept-'+k,'--','sudo',r+'/prefix/bin/afs-processctl','--prefix',r+'/prefix','--config-dir',r+'/etc','--run-dir',r+'/run','--log-dir',r+'/log','--timeout','20',*args],60)
def dump(name,obj): (OUT/name).write_text(json.dumps(obj,indent=2)+'\n')
IDENTITY=r'''
import pathlib,hashlib,os,json,platform,subprocess,tomllib
r=pathlib.Path(__RUN__); old=pathlib.Path(__OLD__); roles=__ROLES__; expect=__SHA__; result={'platform':platform.platform(),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'processes':{},'old_processes':{}}
def proc(base,role,expected=None):
 pid=int((base/'run'/(role+'.pid')).read_text()); p=pathlib.Path('/proc')/str(pid); stat=(p/'stat').read_text().split(') ')[1].split(); exe=os.readlink(p/'exe'); sha=hashlib.sha256((p/'exe').read_bytes()).hexdigest()
 if expected: assert sha==expected and exe==str(base/'prefix/bin'/('afs-'+role)),(role,exe,sha)
 return {'pid':pid,'start_ticks':stat[19],'state':stat[0],'exe':exe,'sha256':sha}
for role in roles:
 result['processes'][role]=proc(r,role,expect[role]); result['old_processes'][role]=proc(old,role)
 result.setdefault('config_sha256',{})[role]=hashlib.sha256((r/'etc'/(role+'.toml')).read_bytes()).hexdigest()
 for key in ('tls_ca_certificate','tls_identity_certificate','tls_identity_private_key'):
  assert pathlib.Path(tomllib.loads((r/'etc'/(role+'.toml')).read_text())[key]).exists()
result['mounts']=[]
for kind in ('ownerfs','dfs'):
 m=json.loads(subprocess.check_output(['findmnt','-J','-M',str(r/('mount-'+kind)),'-o','TARGET,SOURCE,FSTYPE'],text=True))['filesystems'][0]; assert m['target']==str(r/('mount-'+kind)) and m['source']=='afs-'+kind and m['fstype'].startswith('fuse'),m; result['mounts'].append(m)
result['data_volume']=json.loads(subprocess.check_output(['findmnt','-J','-T',str(r),' -o'.strip(),'TARGET,SOURCE,FSTYPE'],text=True))['filesystems'][0]; assert result['data_volume']['fstype']=='ext4'
print(json.dumps(result))
'''
def identities():
 return {k:json.loads(guest(k,IDENTITY.replace('__RUN__',repr(RUN[k])).replace('__OLD__',repr(OLD[k])).replace('__ROLES__',repr(['meta','node'] if k=='a' else ['node'])).replace('__SHA__',repr(SHA)))) for k in RUN}
def prepare():
 OUT.mkdir(exist_ok=False)
 # Both preflights precede mutation. Preserve old runtime/config/data.
 for k in RUN:
  code=f'''import pathlib,subprocess,json,os
r=pathlib.Path({RUN[k]!r}); assert not r.exists(),r
assert pathlib.Path({OLD[k]!r}).is_dir()
ports={list(range(18280,18284)) if k=='a' else [18284,18285]!r}
for addr in subprocess.check_output(['ss','-ltnH'],text=True).splitlines():
 assert not any(addr.split()[3].endswith(':'+str(p)) for p in ports),addr
s=os.statvfs(str(r.parent)); assert s.f_bavail*s.f_frsize>256*1024*1024
print(json.dumps({{'ports_clear':ports,'available_bytes':s.f_bavail*s.f_frsize}}))'''
  dump('preflight-'+k+'.json',json.loads(guest(k,code)))
 for k in RUN:
  roles=['meta','node'] if k=='a' else ['node']
  prep=f'''import pathlib,subprocess,hashlib,json,shutil
r=pathlib.Path({RUN[k]!r}); old=pathlib.Path({OLD[k]!r}); r.mkdir()
for rel in ('prefix/bin','etc','run','log','state/node','state/meta','mount-dfs','mount-ownerfs'): (r/rel).mkdir(parents=True,exist_ok=True)
for role in {roles!r}:
 text=(old/(role+'.toml')).read_text().replace(str(old),str(r))
 for p in range(17980,17986): text=text.replace(str(p),str(p+300))
 (r/'etc'/(role+'.toml')).write_text(text)
print('prepared '+str(r))'''
  (OUT/('prepare-'+k+'.stdout')).write_text(guest(k,prep))
  call(['limactl','copy',str(CTRL),'afs-accept-'+k+':'+RUN[k]+'/prefix/bin/afs-processctl'])
  guest(k,f'import os; os.chmod({(RUN[k]+"/prefix/bin/afs-processctl")!r},0o755)')
 # A uses verified hard links from the completed isolated v51 runtime on ext4.
 guest('a',f'''import pathlib,os,hashlib
r=pathlib.Path({RUN['a']!r}); source=pathlib.Path('/mnt/lima-afsadata/afs-delivery/shutdown-signal-v51/prefix/bin')
for role,sha in {SHA!r}.items():
 p=source/('afs-'+role); assert hashlib.sha256(p.read_bytes()).hexdigest()==sha
 os.link(p,r/'prefix/bin'/p.name)
''')
 with tempfile.TemporaryDirectory(prefix='afs-v51-b-') as d:
  binary=pathlib.Path(d)/'afs-node'; call(['limactl','copy','afs-build:/home/lzc.guest/afs-build/artifacts/v51-qualified/afs-node',str(binary)])
  assert hashlib.sha256(binary.read_bytes()).hexdigest()==SHA['node']
  call(['limactl','copy',str(binary),'afs-accept-b:'+RUN['b']+'/prefix/bin/afs-node'])
 guest('b',f'import pathlib,os,hashlib; p=pathlib.Path({(RUN["b"]+"/prefix/bin/afs-node")!r}); assert hashlib.sha256(p.read_bytes()).hexdigest()=={SHA["node"]!r}; os.chmod(p,0o755)')
 for k in RUN:
  guest(k,'import pathlib; pathlib.Path("/var/tmp/afs-v51-probes").mkdir(exist_ok=False)')
  for filename in ('consistency_cross.py','locks_cross.py','locks_smoke.py'):
   call(['limactl','copy',str(ROOT/'experiments/afs-acceptance/probes'/filename),'afs-accept-'+k+':/var/tmp/afs-v51-probes/'+filename])
 (OUT/'start-a.stdout').write_text(ctl('a','start','all')); (OUT/'start-b.stdout').write_text(ctl('b','start','node'))
 data=identities(); dump('stage-identities.json',data)
 guest('a',f'import os; os.mkdir({(RUN["a"]+"/mount-ownerfs/workspace-v51")!r})')
 dump('stage.json',{'status':'PASS','source_commit':call(['git','-C',str(ROOT/'source'),'rev-parse','HEAD']).strip(),'binaries':SHA,'controller_sha256':hashlib.sha256(CTRL.read_bytes()).hexdigest(),'runtime':RUN,'identities':data,'roles':'memory/R1/gRPC/TLS','limits':'isolated development; release lock PREPARING'})
 print('v51 A/B stage PASS',flush=True)
def probes():
 initial=identities(); dump('probe-start-identities.json',initial)
 for name in ('consistency','ownerfs','dfs'):
  args=json.loads((ROOT/'evidence/afs-delivery/p2-integration-v48'/(name+'-command.json')).read_text())
  args=[x.replace('p2-integration-v48','p2-integration-v51').replace('p2-memory-lane-v48','p2-memory-lane-v51').replace('p2-memory-peer-v48','p2-memory-peer-v51').replace('owner-memory-lane-20260930T220717Z','workspace-v51').replace('v48','v51') for x in args]
  for i,x in enumerate(args):
   if x=='--worker-a-json' or x=='--worker-b-json':
    k='a' if x=='--worker-a-json' else 'b'; args[i+1]=json.dumps(['limactl','shell','afs-accept-'+k,'--','sudo','python3','/var/tmp/afs-v51-probes/'+('consistency_cross.py' if name=='consistency' else 'locks_cross.py')])
   if x=='--worker-a-expected-process' or x=='--worker-b-expected-process':
    k='a' if x=='--worker-a-expected-process' else 'b'; role=args[i+1].split('=')[0]; p=initial[k]['processes'][role]; args[i+1]=f'{role}={p["pid"]}:{p["sha256"]}'
  dump(name+'-command.json',args)
  with (OUT/(name+'.stdout')).open('w') as out,(OUT/(name+'.stderr')).open('w') as err:
   p=subprocess.run(args,cwd=ROOT,stdout=out,stderr=err,timeout=360)
  dump(name+'-exit.json',{'returncode':p.returncode}); print(name,p.returncode,flush=True); assert p.returncode==0,name
 final=identities(); dump('probe-end-identities.json',final)
 for k in RUN:
  assert final[k]['processes']==initial[k]['processes']; assert final[k]['old_processes']==initial[k]['old_processes']
 dump('probe.json',{'status':'PASS','initial':initial,'final':final,'limits':'development matrix only; no full acceptance declaration'})
if __name__=='__main__':
 if len(sys.argv)!=2 or sys.argv[1] not in ('prepare','probes'): raise SystemExit('prepare|probes')
 (prepare if sys.argv[1]=='prepare' else probes)()

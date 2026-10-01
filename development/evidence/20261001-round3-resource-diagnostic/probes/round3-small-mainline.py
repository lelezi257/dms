"""Bounded whole-system development diagnostics; not a formal benchmark."""
import concurrent.futures,hashlib,importlib.util,json,os,pathlib,platform,random,subprocess,sys,time,tomllib,urllib.request
assert sys.platform=='linux' and platform.machine()=='aarch64' and os.geteuid()==0
spec=importlib.util.spec_from_file_location('base',pathlib.Path(__file__).with_name('round2-capacity.py'));base=importlib.util.module_from_spec(spec);spec.loader.exec_module(base)
which,action=sys.argv[1:3];assert which in ('ctl','a','b')
vol=pathlib.Path('/mnt/lima-afs'+('ctlstate' if which=='ctl' else which+'data'));run=vol/'afs-delivery/round3-small-v83'
role='meta' if which=='ctl' else 'node';ip={'ctl':11,'a':12,'b':13}[which];port={'ctl':20780,'a':20782,'b':20784}[which]
e=run/'evidence';expected=json.loads(pathlib.Path('/home/lzc.guest/eio-v80-r2-binaries.json').read_text())
base.SHA=expected
def save(label,v):
 path=e/(label+'.json');assert not path.exists();path.write_text(json.dumps(v,indent=2)+'\n');return v
def resources(label):
 argv=['python3','/home/lzc.guest/round3-resource-probe.py','--pid-file',f'{role}={run}/run/{role}.pid','--volume',str(vol),'--output',str(e/(label+'-resources.json'))]
 p=subprocess.run(argv,capture_output=True,text=True,timeout=30);assert p.returncode==0,(p.returncode,p.stdout,p.stderr)
def health():
 with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(f'http://192.168.109.{ip}:{port+1}/health',timeout=3) as f:return json.loads(f.read())
def identity():
 value=base.identity(run,role);assert value['sha256']==expected[role];return value
def metrics(label):
 with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(f'http://192.168.109.{ip}:{port+1}/metrics',timeout=3) as f:raw=f.read()
 (e/(label+'-metrics.txt')).write_bytes(raw)
def payload():
 return b''.join((hashlib.sha256(f'afs-round3-v83-block-{i}'.encode()).digest()*32768) for i in range(8))
def path(kind,name):return run/'mount'/kind/('workspace-round3-v83/'+name if kind=='ownerfs' else name)
def write(kind,name):
 data=payload();resources(kind+'-'+name+'-before');metrics(kind+'-'+name+'-before')
 started=time.monotonic_ns();fd=os.open(path(kind,name),os.O_CREAT|os.O_EXCL|os.O_RDWR,0o600)
 try:
  for off in range(0,len(data),1024**2):assert os.write(fd,data[off:off+1024**2])==1024**2
  wrote=time.monotonic_ns();os.fdatasync(fd);synced=time.monotonic_ns();os.fsync(fd);full=time.monotonic_ns()
 finally:os.close(fd)
 end=time.monotonic_ns();resources(kind+'-'+name+'-after');metrics(kind+'-'+name+'-after')
 return {'kind':kind,'file':name,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest(),'write_ms':(wrote-started)/1e6,'data_sync_ms':(synced-wrote)/1e6,'full_sync_ms':(full-synced)/1e6,'close_ms':(end-full)/1e6,'identity':identity(),'formal_acceptance':'NOT_RUN','timing':'unqualified development diagnostic'}
def read(kind,name,concurrency):
 data=payload();label=f'{kind}-{name}-read-c{concurrency}';resources(label+'-before');metrics(label+'-before')
 def worker(index):
  fd=os.open(path(kind,name),os.O_RDONLY);start=time.monotonic_ns();h=hashlib.sha256();n=0
  try:
   if concurrency==1:
    while block:=os.read(fd,1024**2):h.update(block);n+=len(block)
    assert n==len(data) and h.hexdigest()==hashlib.sha256(data).hexdigest() and os.read(fd,1)==b''
   else:
    rng=random.Random(8300+index)
    for _ in range(64):
     size=rng.choice((4096,65536));offset=rng.randrange(len(data)-size+1);block=os.pread(fd,size,offset)
     assert block==data[offset:offset+size];h.update(block);n+=len(block)
   return {'worker':index,'bytes':n,'elapsed_ms':(time.monotonic_ns()-start)/1e6,'range_checks':64 if concurrency>1 else 0,'digest':h.hexdigest()}
  finally:os.close(fd)
 start=time.monotonic_ns()
 with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:results=list(pool.map(worker,range(concurrency)))
 wall=(time.monotonic_ns()-start)/1e6;resources(label+'-after');metrics(label+'-after')
 return {'kind':kind,'file':name,'concurrency':concurrency,'workers':results,'elapsed_ms':wall,'identity':identity(),'formal_acceptance':'NOT_RUN','cache':'ordinary FUSE; cold/isolated performance not qualified'}
if action=='prepare':
 assert not run.exists() and os.statvfs(vol).f_bavail*os.statvfs(vol).f_frsize>=4*1024**3+24*1024**2
 old=vol/'afs-delivery/round2-overall-v82';text=(old/'etc'/f'{role}.toml').read_text().replace(str(old),str(run))
 for a,b in [(20580,20780),(20581,20781),(20582,20782),(20583,20783),(20584,20784),(20585,20785)]:text=text.replace(':'+str(a),':'+str(b))
 for d in ['etc','run','log','state','mount','evidence']:(run/d).mkdir(parents=True)
 (run/'prefix').symlink_to(old/'prefix',target_is_directory=True);(run/'etc'/f'{role}.toml').write_text(text)
 assert all(base.digest(run/'prefix/bin'/('afs-'+r))==h for r,h in expected.items())
 value={'configuration':tomllib.loads(text),'binaries':expected,'guest_ext4':base.run_cmd(['findmnt','-T',str(vol),'-J'])}
elif action in ('start','stop','restart'):
 value={'controller':base.controller(run,action,role)}
 if action!='stop':value.update(identity=identity(),health=health());resources(action+'-idle')
elif action=='workspace':
 assert which=='a';os.mkdir(run/'mount/ownerfs/workspace-round3-v83');value={'identity':identity()}
elif action=='write':value=write(sys.argv[3],sys.argv[4])
elif action=='read':value=read(sys.argv[3],sys.argv[4],int(sys.argv[5]))
elif action=='idle':resources('final-idle');metrics('final-idle');value={'identity':identity(),'health':health()}
elif action=='cleanup':
 pid=int((run/'run'/f'{role}.pid').read_text());assert not pathlib.Path('/proc',str(pid)).exists()
 for m in ['ownerfs','dfs']:assert subprocess.run(['findmnt','-M',str(run/'mount'/m)],stdout=subprocess.DEVNULL).returncode!=0
 free=os.statvfs(vol).f_bavail*os.statvfs(vol).f_frsize;assert free>=4*1024**3
 value={'stopped_pid':pid,'mounts_gone':True,'data_available_bytes':free}
else:raise ValueError(action)
value.update(status='PASS',utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),which=which,action=action,formal_acceptance='NOT_RUN',environment='PREPARING')
print(json.dumps(save('-'.join(sys.argv[2:]),value)),flush=True)

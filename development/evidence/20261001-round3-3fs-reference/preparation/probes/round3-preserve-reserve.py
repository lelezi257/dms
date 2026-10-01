"""Relocate explicit inactive historical inputs, preserving bytes and old paths."""
import hashlib,json,os,pathlib,platform,shutil,subprocess,sys,time
assert sys.platform=='linux' and platform.machine()=='aarch64' and os.geteuid()==0
role=sys.argv[1];assert role in ('ctl','a')
source=pathlib.Path('/mnt/lima-afsctlstate/afs-delivery/p1b/state/etcd' if role=='ctl' else '/mnt/lima-afsadata/afs-delivery/package-correction')
dest=pathlib.Path('/var/lib/afs-historical-preserved-v84')/role/source.name
record=pathlib.Path('/home/lzc.guest')/f'round3-reserve-{role}.json'
assert source.is_dir() and not source.is_symlink() and not dest.exists() and not record.exists()
def identity(pid):
    try:
        p=pathlib.Path('/proc')/str(pid);return {'pid':pid,'stat':(p/'stat').read_text(),'exe':str((p/'exe').resolve())}
    except OSError:return None
def check_inactive():
    refs=[]
    for p in pathlib.Path('/proc').iterdir():
        if not p.name.isdecimal():continue
        paths=[p/'cwd',p/'root',p/'exe']
        try:paths+=list((p/'fd').iterdir())
        except OSError:pass
        for f in paths:
            try:
                resolved=f.resolve();resolved.relative_to(source);refs.append(str(f))
            except (OSError,ValueError,RuntimeError):pass
        try:
            if str(source) in (p/'maps').read_text():refs.append(str(p/'maps'))
            if role=='ctl' and (p/'comm').read_text().strip()=='etcd':refs.append('etcd process '+p.name)
        except OSError:pass
    assert not refs,refs
def manifest(root):
    result={}
    for p in sorted(root.rglob('*')):
        stat=p.lstat();item={'mode':stat.st_mode,'uid':stat.st_uid,'gid':stat.st_gid,'bytes':stat.st_size}
        if p.is_symlink():item['link']=os.readlink(p)
        elif p.is_file():
            h=hashlib.sha256()
            with p.open('rb') as f:
                while block:=f.read(1024**2):h.update(block)
            item['sha256']=h.hexdigest()
        elif p.is_dir():item.pop('bytes')
        else:raise ValueError('unexpected special file '+str(p))
        result[str(p.relative_to(root))]=item
    return result
check_inactive();before=manifest(source)
size=sum(v.get('bytes',0) for v in before.values() if 'sha256' in v)
assert os.statvfs('/').f_bavail*os.statvfs('/').f_frsize>=size+4*1024**3
dest.parent.mkdir(parents=True,exist_ok=True)
subprocess.run(['cp','-a','--',str(source),str(dest)],check=True)
check_inactive();assert manifest(source)==before and manifest(dest)==before
old=source.with_name(source.name+'-verified-original-v84');assert not old.exists();source.rename(old)
try:source.symlink_to(dest,target_is_directory=True)
except BaseException:old.rename(source);raise
assert manifest(source)==before
shutil.rmtree(old)  # Only the verified duplicate; every original byte remains reachable.
value={'utc':time.time(),'source':str(source),'preserved_at':str(dest),'files':before,'regular_bytes':size,'source_is_symlink':source.is_symlink(),'source_resolves_to':str(source.resolve()),'volume_available_bytes':os.statvfs(source.parent).f_bavail*os.statvfs(source.parent).f_frsize,'status':'PASS','scope':'historical archive relocation; not acceptance data placement','formal_acceptance':'NOT_RUN'}
record.open('x').write(json.dumps(value,indent=2)+'\n');print(json.dumps({k:v for k,v in value.items() if k!='files'}),flush=True)

#!/usr/bin/env python3
"""Fresh Linux corruption lifecycle lane; never changes prior runtime paths."""
import importlib.util,pathlib,os
spec=importlib.util.spec_from_file_location('repair_runtime_base',pathlib.Path(__file__).with_name('repair-runtime-v55.py'));assert spec and spec.loader
runtime=importlib.util.module_from_spec(spec);spec.loader.exec_module(runtime)
runtime.OUT=runtime.ROOT/'evidence/afs-delivery/repair-runtime-v62'
runtime.RUN={'a':'/mnt/lima-afsadata/afs-delivery/corrupt-v62-a','b':'/mnt/lima-afsbdata/afs-delivery/corrupt-v62-b'}
runtime.PORTS={'meta_grpc':18580,'meta_rest':18581,'a_grpc':18582,'a_rest':18583,'b_grpc':18584,'b_rest':18585}
runtime.TEST_FILE='corrupt-v62-initial-1m.bin'
# Older lanes may have exited. Preserve their observed state, never infer
# liveness from a historical PID or require unrelated services to be restarted.
def old_identity(which, roles, name):
 code = "RUN="+repr(runtime.TEMPLATE[which])+"\nROLES="+repr(list(roles))+"\n"+r"""
import hashlib,json,os,pathlib
r=pathlib.Path(RUN);result={'runtime':RUN,'processes':{},'config_sha256':{}}
for role in ROLES:
 pid=int((r/'run'/(role+'.pid')).read_text());proc=pathlib.Path('/proc',str(pid));row={'pid':pid,'alive':proc.exists()}
 if proc.exists():
  row.update(start_ticks=(proc/'stat').read_text().rsplit(') ',1)[1].split()[19],exe=os.readlink(proc/'exe'),sha256=hashlib.sha256((proc/'exe').read_bytes()).hexdigest())
 else:row['disk_binary_sha256']=hashlib.sha256((r/'prefix/bin'/('afs-'+role)).read_bytes()).hexdigest()
 result['processes'][role]=row;result['config_sha256'][role]=hashlib.sha256((r/'etc'/(role+'.toml')).read_bytes()).hexdigest()
print(json.dumps(result))
"""
 result=runtime.json.loads(runtime.guest(which,code));runtime.dump(name,result);return result
runtime.old_identity=old_identity
os.environ['AFS_REPAIR_CANDIDATE']='v62-r3'
if __name__=='__main__':
 args=runtime.parser().parse_args();assert args.candidate=='v62-r3';args.fn(args)

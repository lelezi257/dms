#!/usr/bin/env python3
"""Isolated actual RXE repair lane; frozen v62-r3 Rust artifacts are reused."""
import importlib.util,json,pathlib,os
spec=importlib.util.spec_from_file_location('rxe_repair_v62_base',pathlib.Path(__file__).with_name('repair-runtime-v62.py'));assert spec and spec.loader
base=importlib.util.module_from_spec(spec);spec.loader.exec_module(base)
runtime=base.runtime
OLD_RUNTIME=dict(runtime.RUN);OLD_OUT=runtime.OUT
runtime.OUT=runtime.ROOT/'evidence/afs-delivery/repair-runtime-rxe-v62'
runtime.RUN={'a':'/mnt/lima-afsadata/afs-delivery/rxe-repair-v62-a','b':'/mnt/lima-afsbdata/afs-delivery/rxe-repair-v62-b'}
runtime.PORTS={'meta_grpc':18680,'meta_rest':18681,'a_grpc':18682,'a_rest':18683,'b_grpc':18684,'b_rest':18685}
runtime.TEST_FILE='rxe-repair-v62-initial-1m.bin'
render=runtime.render_config

def render_config(which,role,text):
 text=render(which,role,text)
 if role=='node':
  text=runtime.set_toml_scalar(text,'data_mode',"'rdma'")
  text=runtime.set_toml_scalar(text,'rdma_device',"'rxe0'")
 return text
runtime.render_config=render_config

def reuse_artifacts(candidate,hashes):
 assert candidate=='v62-r3'
 gate=json.loads((OLD_OUT/'gate-report.selected.json').read_text())
 assert gate['hashes']==hashes
 staged=json.loads((OLD_OUT/'staged-runtime-binary-sha256.json').read_text())
 results={}
 for which,roles in [('a',('meta','node')),('b',('node',))]:
  code="SRC="+repr(OLD_RUNTIME[which])+"\nDST="+repr(runtime.RUN[which])+"\nROLES="+repr(roles)+"\nEXPECTED="+repr(staged[which])+"\n"+r"""
import hashlib,json,os,pathlib
src=pathlib.Path(SRC);dst=pathlib.Path(DST);rows={}
for role in ROLES:
 old=src/'prefix/bin'/('afs-'+role);new=dst/'prefix/bin'/('afs-'+role)
 digest=hashlib.sha256(old.read_bytes()).hexdigest();assert digest==EXPECTED[role]
 assert not new.exists();os.link(old,new)
 assert hashlib.sha256(new.read_bytes()).hexdigest()==digest
 rows[role]=digest
print(json.dumps(rows))
"""
  results[which]=json.loads(runtime.guest(which,code))
 runtime.dump('staged-runtime-binary-sha256.json',results)
 runtime.dump('artifact-reuse.json',{'source_candidate':candidate,'original_hashes':hashes,'stripped_hashes':results,'method':'hardlinks to exact qualified stripped binaries; no binary mutation; separate config/state/mounts','full_source_gate_reused':True})
runtime.copy_artifacts=reuse_artifacts
if __name__=='__main__':
 args=runtime.parser().parse_args();assert args.candidate=='v62-r3';args.fn(args)

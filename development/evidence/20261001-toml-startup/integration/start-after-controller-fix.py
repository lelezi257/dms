import importlib.util,pathlib,json,hashlib
root=pathlib.Path('/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store')
p=root/'experiments/afs-acceptance/integration-v51.py'; spec=importlib.util.spec_from_file_location('integration_v51',p); m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
for k in m.RUN:
 # Copy only the controller in the new, non-running stage. No old runtime changes.
 m.call(['limactl','copy',str(m.CTRL),'afs-accept-'+k+':'+m.RUN[k]+'/prefix/bin/afs-processctl'])
 actual=m.guest(k,f'import pathlib,hashlib; print(hashlib.sha256(pathlib.Path({(m.RUN[k]+"/prefix/bin/afs-processctl")!r}).read_bytes()).hexdigest())').strip()
 assert actual==hashlib.sha256(m.CTRL.read_bytes()).hexdigest()
(m.OUT/'start-a.stdout').write_text(m.ctl('a','start','all'))
(m.OUT/'start-b.stdout').write_text(m.ctl('b','start','node'))
data=m.identities(); m.dump('stage-identities.json',data)
m.guest('a',f'import os; os.mkdir({(m.RUN["a"]+"/mount-ownerfs/workspace-v51")!r})')
m.dump('stage.json',{'status':'PASS','rust_source_base_commit':m.call(['git','-C',str(root/'source'),'rev-parse','HEAD']).strip(),'binaries':m.SHA,'controller_sha256':hashlib.sha256(m.CTRL.read_bytes()).hexdigest(),'runtime':m.RUN,'identities':data,'roles':'memory/R1/gRPC/TLS','limits':'isolated development; controller has reviewed narrow source change; release lock PREPARING'})
print('v51 A/B stage with fixed controller PASS',flush=True)

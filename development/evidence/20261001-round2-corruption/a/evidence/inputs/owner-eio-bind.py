#!/usr/bin/env python3
"""Capture exact process/config/mount binding for the owned EIO cohort."""
import argparse,hashlib,importlib.util,json,pathlib,platform,tomllib,urllib.request
spec=importlib.util.spec_from_file_location('capacity',pathlib.Path(__file__).with_name('round2-capacity.py'))
c=importlib.util.module_from_spec(spec);spec.loader.exec_module(c)
c.SHA=json.loads(pathlib.Path('/home/lzc.guest/eio-v80-r2-binaries.json').read_text())
if __name__=='__main__':
 assert platform.system()=='Linux' and platform.machine()=='aarch64'
 p=argparse.ArgumentParser();p.add_argument('run',type=pathlib.Path);p.add_argument('record');a=p.parse_args()
 role='meta' if (a.run/'etc/meta.toml').exists() else 'node'
 identity=c.identity(a.run,role); cfgpath=a.run/'etc'/f'{role}.toml';cfg=tomllib.loads(cfgpath.read_text())
 argv=(pathlib.Path('/proc')/str(identity['pid'])/'cmdline').read_bytes().split(b'\0')
 argv=[v.decode() for v in argv if v];assert argv[1:]==['--config',str(cfgpath)]
 opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
 health=json.loads(opener.open('http://'+cfg['rest_listen']+'/health',timeout=5).read());assert health['status']=='ready'
 if role=='meta': assert cfg['meta_store']=='memory'
 mounts={}
 for key in ('ownerfs_mount','dfs_mount'):
  if key in cfg:
   mount=c.run_cmd(['findmnt','-J','-M',cfg[key]]); info=json.loads(mount['stdout'])['filesystems'][0];assert info['fstype'].startswith('fuse');mounts[key]=mount
 out={'status':'PASS','identity':identity,'argv':argv,'configuration_sha256':c.digest(cfgpath),'configuration':cfg,'health':health,'mounts':mounts}
 c.save(a.run,a.record,out);print(json.dumps(out,indent=2))

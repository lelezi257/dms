import json,pathlib,tomllib,hashlib,subprocess,platform,time
assert platform.system()=='Linux' and platform.machine()=='aarch64'
r=pathlib.Path('/mnt/lima-afsbdata/afs-delivery/owner-eio-v80-r2-b')
assert not r.exists()
base=pathlib.Path('/mnt/lima-afsbdata/afs-delivery/round1-mainline-v77-archive-async')
prefix=pathlib.Path('/var/lib/afs-acceptance/candidates/eio-v80-r2/prefix')
sha=json.loads(pathlib.Path('/home/lzc.guest/eio-v80-r2-binaries.json').read_text())
for role,expected in sha.items(): assert hashlib.sha256((prefix/'bin'/f'afs-{role}').read_bytes()).hexdigest()==expected
text=(base/'etc/node.toml').read_text().replace(str(base/'state/node'),str(r/'state/node')).replace(str(base/'run'),str(r/'run')).replace(str(base/'mount'),str(r/'mount'))
text=text.replace(':19980',':20280').replace(':19984',':20284').replace(':19985',':20285').replace('data_mode = "auto"','data_mode = "grpc"').replace('fs = "all"','fs = "ownerfs"')
text='\n'.join(line for line in text.splitlines() if not line.startswith('rdma_device'))+'\n'
ss=subprocess.check_output(['ss','-ltnH'],text=True)
assert all(not row.split()[3].endswith(':'+str(p)) for row in ss.splitlines() for p in (20284,20285))
for d in ('etc','run','log','state','mount','evidence'): (r/d).mkdir(parents=True)
(r/'prefix').symlink_to(prefix,target_is_directory=True)
(r/'etc/node.toml').write_text(text)
(r/'evidence/prepare.json').write_text(json.dumps({'status':'PASS','configuration':tomllib.loads(text),'configuration_sha256':hashlib.sha256(text.encode()).hexdigest(),'binaries':sha,'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())},indent=2)+'\n')
print('isolated-B-ownerfs-prepared')

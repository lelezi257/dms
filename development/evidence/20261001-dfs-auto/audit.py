"""Linux-only read-only semantic audit for frozen DFS Auto slice evidence."""
import hashlib
import json
import pathlib
import platform
import re
import sys

assert platform.system() == 'Linux'
base = pathlib.Path(sys.argv[1])
product = pathlib.Path(sys.argv[2])
original = base / 'dfs-auto-v75-original-restored'
r1 = base / 'dfs-auto-v75-r1'
r2 = base / 'dfs-auto-v75-r2'
r3 = base / 'dfs-auto-v75-r3'
checks = []


def check(name, condition):
    assert condition, name
    checks.append(name)


def read(path):
    return path.read_text()


def counts(path):
    items = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored', read(path))
    assert items, path
    return tuple(sum(int(v[i]) for v in items) for i in range(3))


before = json.loads(read(original / 'before-graft-inputs.json'))['files']
graft = json.loads(read(original / 'grafted-inputs.json'))['files']
final = json.loads(read(r3 / 'build-inputs.json'))['files']
check('143 frozen inputs', len(final) == 143 and before.keys() == final.keys())
check('original graft only data tests', [p for p in before if before[p] != graft[p]] == ['src/node/rpc/data.rs'])
check('original behavior failure101', read(original / 'original.exit').strip() == '101' and 'left: 0' in read(original / 'original.log') and 'right: 8388608' in read(original / 'original.log'))
check('frozen product inputs match', all(hashlib.sha256((product / p).read_bytes()).hexdigest() == h for p,h in final.items()))
for tag in (r1, r2):
    old = json.loads(read(tag / 'build-inputs.json'))
    changed = {p for p,h in old['files'].items() if h != final[p]}
    check(tag.name + ' test-only changed input scope', changed <= {'src/node/rpc/data.rs', 'src/node/rpc/peer.rs'})
    for p in changed:
        marker = '#[cfg(test)]'
        old_text = read(pathlib.Path(old['snapshot']) / p)
        new_text = read(product / p)
        check(tag.name + ' production stable ' + p, marker in old_text and old_text.split(marker,1)[0] == new_text.split(marker,1)[0])
    check(tag.name + ' local51', sum(counts(tag / f'local/{n}.log')[0] for n in ('original','peer','control','data')) == 51)
    for n in ('original','peer','control','data'):
        check(tag.name + ' local exit ' + n, read(tag / f'local/{n}.exit').strip() == '0')
for name in ('fmt','clippy','lib','error','contracts','local-api','fuse-build','fuse','build'):
    check('final full gate '+name, read(r3 / f'full/gate/{name}.exit').strip() == '0')
for name in ('no-features','owner-features','dfs-features','owner-rdma','dfs-rdma'):
    check('feature '+name, read(r3 / f'full/features/{name}.exit').strip() == '0')
check('lib414/6', re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',read(r3/'full/gate/lib.log'))[-1] == ('414','0','6'))
for name, count in [('contracts',65),('error',4),('local-api',9),('fuse',5)]:
    check('full count '+name, counts(r3 / f'full/gate/{name}.log')[0] == count)
for name in ('auto-fallback','required-unsupported','canonical','auto-native','required-native','owner-native'):
    check('related '+name, counts(r3/f'related/{name}.log')==(1,0,0) and read(r3/f'related/{name}.exit').strip()=='0')
for name, mode in [('auto-native','Auto'),('required-native','Rdma')]:
    text=read(r3/f'related/{name}.log')
    check(name+' actual payload/retry/auth', f'mode={mode} plane=rdma replica_bytes=8388608 read_bytes=75000 other_payload_bytes=0 exact_retry=PASS forged_grant=DENIED' in text and text.count('AFS_RDMA_COMPLETE op=READ bytes=4194304')==2 and 'AFS_RDMA_COMPLETE op=WRITE bytes=75000' in text)
text=read(r3/'related/auto-fallback.log')
check('fallback only gRPC actual payload/retry/auth', 'mode=Auto plane=grpc replica_bytes=8388608 read_bytes=75000 other_payload_bytes=0 exact_retry=PASS forged_grant=DENIED' in text and 'AFS_RDMA_COMPLETE' not in text)
check('required read+write fail with zero payload', 'write=REJECTED read=REJECTED error=NODE_TRANSFER_UNSUPPORTED grpc_payload_bytes=0 rdma_payload_bytes=0' in read(r3/'related/required-unsupported.log'))
owner=read(r3/'related/owner-native.log')
for op in ('READ','WRITE'):
    check('Owner actual '+op, sum(map(int,re.findall(r'AFS_RDMA_COMPLETE op='+op+r' bytes=(\d+)',owner)))==4*1024*1024+17)
for line in read(r3/'stripped-binaries.txt').splitlines():
    digest, binary=line.split('  ',1)
    check('binary '+pathlib.Path(binary).name, hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest()==digest)
check('AGENTS unchanged',hashlib.sha256((product/'AGENTS.md').read_bytes()).hexdigest()=='539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f')
check('handoff unchanged',hashlib.sha256((product/'docs/handoff.md').read_bytes()).hexdigest()=='8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216')
print(json.dumps({'status':'PASS','check_count':len(checks),'checks':checks,'formal_acceptance':'NOT_RUN','environment':'PREPARING','limits':['No new cross-VM Auto Node deployment','No postdispatch unsupported injection','No late-window unsupported or exhausted deadline injection','No posted-DMA cancellation/exceptional resource/long-run fault qualification']},indent=2))

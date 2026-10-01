import hashlib
import json
import pathlib
import re
import sys

assert sys.platform.startswith('linux')
base=pathlib.Path(sys.argv[1])
checks=[]
def check(name,value):
    assert value,name
    checks.append(name)
def text(path): return path.read_text()
def counts(path):
    values=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',text(path))
    assert values,path
    return tuple(sum(int(row[i]) for row in values) for i in range(3))
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
r1=base/'posted-cancel-v76-r1'
r2=base/'posted-cancel-v76-r2'
r4=base/'posted-cancel-v76-r4'
r5=base/'posted-cancel-v76-r5'
r6=base/'posted-cancel-v76-r6'
old=json.loads(text(base/'v75-posted-baseline-inputs.json'))['files']
final=json.loads(text(r2/'build-inputs.json'))
host=dict(line.split('  ',1)[::-1] for line in text(r5/'host-inputs.txt').splitlines())
check('143 compiler inputs',len(final['files'])==143 and old.keys()==final['files'].keys())
check('production unchanged vs v75', {p for p,h in old.items() if final['files'][p]!=h}=={'tests/rdma_lifecycle.rs'})
check('Linux frozen inputs match host edits',all(h==host[p] for p,h in final['files'].items()))
check('snapshot bytes match compile manifest',all(sha(pathlib.Path(final['snapshot'])/p)==h for p,h in final['files'].items()))
check('final runner matches host',sha(r6/'runner-input.py')==host['scripts/check-rdma-cancellation.py'])
check('AGENTS unchanged',host['AGENTS.md']=='539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f')
check('handoff unchanged',host['docs/handoff.md']=='8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216')
check('initial compiler failure retained','error[E0716]' in text(r1/'compile.log'))
check('initial debugger failure retained','checkpoint_timeout' in text(r2/'cancel/debugger.jsonl') and 'AssertionError' in text(r2/'cancel-runner.log'))
for name in ('fmt','clippy','lib','contracts','error','local-api','fuse-build','fuse','build'):
    check('full '+name,text(r5/f'full/gate/{name}.exit').strip()=='0')
for name in ('no-features','owner-features','dfs-features','owner-rdma','dfs-rdma'):
    check('feature '+name,text(r5/f'full/features/{name}.exit').strip()=='0')
check('lib414/6', re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',text(r5/'full/gate/lib.log'))[-1]==('414','0','6'))
for name,n in [('contracts',65),('error',4),('local-api',9),('fuse',5)]:
    check('count '+name,counts(r5/f'full/gate/{name}.log')[0]==n)
check('default lifecycle1/6',counts(r4/'lifecycle-default.log')==(1,0,6))
check('five existing actual RXE lifecycle',counts(r4/'lifecycle-native.log')==(5,0,0))
check('five actual native probes',counts(r4/'native-probe.log')==(5,0,0))
check('local strict Clippy', 'Finished' in text(r4/'local-clippy.log') and 'error:' not in text(r4/'local-clippy.log'))
check('20 evidence checker checks',json.loads(text(r5/'audit-regression.json'))['count']==20)
for n in (3,4,5,6):
    run=base/f'posted-cancel-v76-r{n}'/'cancel'
    check(f'cancel r{n} semantic PASS',json.loads(text(run/'audit.json'))['status']=='PASS' and counts(run/'gdb.log')==(1,0,0))
identity=json.loads(text(r6/'cancel/identity.json'))
check('final executed runner binding',identity['runner_sha256']==sha(r6/'runner-input.py'))
check('native code identical',identity['native_source_sha256']==final['files']['common/transport/native/rdma.c'])
check('final QP exact identity audit',json.loads(text(r6/'cancel/audit.json'))['posted']['qp'] in json.loads(text(r6/'cancel/audit.json'))['resource_identities']['qp']['closed_paused'])
for line in text(r5/'binaries.txt').splitlines():
    digest,name=line.split('  ',1)
    check('artifact '+pathlib.Path(name).name,sha(pathlib.Path(name))==digest)
check('fixture binary is executed binary',identity['binary_sha256']==text(r5/'binaries.txt').splitlines()[0].split('  ',1)[0])
print(json.dumps({'status':'PASS','level':'stage gate','check_count':len(checks),'checks':checks,'formal_acceptance':'NOT_RUN','environment':'PREPARING','unique_related_tests':12,'limits':['Only diagnostic cancellation with real posted/unconsumed WQE','No physical in-flight DMA fault proof','No exceptional provider teardown proof','No new OwnerFs/DFS postdispatch fault deployment']},indent=2))

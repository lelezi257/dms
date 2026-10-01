// Host orchestration only. All Python validation executes inside Linux.
import fs from 'node:fs';
import crypto from 'node:crypto';
import {spawnSync} from 'node:child_process';
const evidence='/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store/evidence/afs-delivery/verbs-predicate-v70';
const guest='/tmp/afs-verbs-predicate-v70';
const label=process.argv[2];
const mode=process.argv[3];
if(!/^[a-z0-9-]+$/.test(label))throw Error('Invalid log identity');
const selected={
 local:'python3 -m unittest -v test_environment_verbs',
 related:'python3 -m unittest -v test_environment test_environment_network test_runner',
 compile:'python3 -m py_compile environment.py test_environment_verbs.py probes/env_verbs.py runner.py',
 consumer:`python3 ${guest}/linux/consumer_check.py`,
 preparation:`python3 environment.py --lock ${guest}/preparing.lock.json --bundle ${guest}/bundle/bundle.json --output ${guest}/linux/${label}.json`
};
if(!selected[mode])throw Error('Invalid scope');
const command=`cd ${guest}/source/development/acceptance; PYTHONDONTWRITEBYTECODE=1 timeout 90 ${selected[mode]} > ${guest}/linux/${label}.log 2>&1; validation_rc=$?; printf '%s\\n' "$validation_rc" > ${guest}/linux/${label}.exit; exit "$validation_rc"`;
const argv=['shell','--workdir','/home/lzc.guest','afs-accept-ctl','--','bash','-lc',command];
const started=new Date().toISOString();
const result=spawnSync('limactl',argv,{encoding:'utf8',timeout:110000,maxBuffer:4194304});
const row={scope:mode,label,argv:['limactl',...argv],started,ended:new Date().toISOString(),returncode:result.status,error:result.error?.message??null,stdout:result.stdout,stderr:result.stderr,orchestrator_sha256:crypto.createHash('sha256').update(fs.readFileSync(import.meta.filename)).digest('hex')};
fs.appendFileSync(evidence+'/commands.jsonl',JSON.stringify(row)+'\n');
console.log(JSON.stringify({label,scope:mode,returncode:result.status,error:row.error}));
process.exitCode=result.status===null?1:result.status;

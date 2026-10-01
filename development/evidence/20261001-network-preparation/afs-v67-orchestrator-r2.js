const fs=require('fs'),cp=require('child_process'),path=require('path');
const root='/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store';
const out=path.join(root,'evidence/afs-delivery/network-v67/attempt-2');fs.mkdirSync(out,{recursive:true});
const run='/var/lib/afs-acceptance/network-v67-r2';const names=['ctl','a','b','c'];const ips={ctl:'192.168.109.11',a:'192.168.109.12',b:'192.168.109.13',c:'192.168.109.14'};
function invoke(argv,timeout=15000){const r=cp.spawnSync(argv[0],argv.slice(1),{encoding:'utf8',timeout});fs.appendFileSync(path.join(out,'commands.jsonl'),JSON.stringify({time_unix_ms:Date.now(),argv,returncode:r.status,error:r.error?.message,stdout:r.stdout,stderr:r.stderr})+'\n');return r;}
function guest(x,code,timeout=15000){return invoke(['limactl','shell','--workdir','/home/lzc.guest','afs-accept-'+x,'--','sudo','bash','-lc',code],timeout);}
function requireOk(r){if(r.status!==0)throw Error('orchestration command failed: '+r.stderr+' '+r.stdout)}
function args(x,y,checks='all'){return `python3 ${run}/env_network.py client --source-ip ${ips[x]} --target-ip ${ips[y]} --port 19566 --tls-port 19567 --ca ${run}/tls/ca.pem --client-cert ${run}/tls/${x}.pem --client-key ${run}/tls/${x}.key --server-hostname afs-env-${y} --timeout 2 --check ${checks}`;}
try{
 for(const x of names){requireOk(invoke(['limactl','copy',path.join(root,'experiments/afs-acceptance/probes/env_network.py'),'afs-accept-'+x+':/tmp/afs-v67-env_network.py']));requireOk(guest(x,`set -e; cp /tmp/afs-v67-env_network.py ${run}/env_network.py; sha256sum ${run}/env_network.py > ${run}/logs/probe-input.sha256; nohup python3 ${run}/env_network.py server --bind-ip ${ips[x]} --port 19566 --tls-port 19567 --ca ${run}/tls/ca.pem --server-cert ${run}/tls/${x}.pem --server-key ${run}/tls/${x}.key --ready-json ${run}/ready.json > ${run}/logs/server.log 2>&1 < /dev/null & echo $! > ${run}/server.pid; for attempt in 1 2 3 4 5; do [ -s ${run}/ready.json ] && exit 0; sleep 0.2; done; exit 1`));}
 for(const x of names)for(const y of names)if(x!==y){requireOk(guest(x,`${args(x,y)} --untrusted-ca --bad-ca ${run}/tls/untrusted.pem --wrong-hostname --missing-client-cert > ${run}/logs/pair-${x}-${y}.json 2> ${run}/logs/pair-${x}-${y}.stderr`));}
 requireOk(guest('a',`${args('a','b','tls')} --untrusted-client-cert --bad-client-cert ${run}/tls/rogue.pem --bad-client-key ${run}/tls/rogue.key > ${run}/logs/wrong-client.json 2> ${run}/logs/wrong-client.stderr`));
 requireOk(guest('a',`${args('a','b')} > ${run}/logs/fault-before.json 2> ${run}/logs/fault-before.stderr`));
 requireOk(guest('b','bash /tmp/afs-v67-fault.sh install'));
 const fault=guest('a',`${args('a','b')} > ${run}/logs/fault-injected.json 2> ${run}/logs/fault-injected.stderr`);
 fs.writeFileSync(path.join(out,'fault-orchestration-exit.json'),JSON.stringify({returncode:fault.status})+'\n');
 requireOk(guest('b','bash /tmp/afs-v67-fault.sh inspect; bash /tmp/afs-v67-fault.sh restore'));
 requireOk(guest('a',`${args('a','b')} > ${run}/logs/fault-restored.json 2> ${run}/logs/fault-restored.stderr`));
 fs.writeFileSync(path.join(out,'orchestration-complete.json'),JSON.stringify({status:'COMPLETE',scope:'Only orchestration; Linux semantic validator still required'})+'\n');
}catch(e){fs.writeFileSync(path.join(out,'orchestration-failure.json'),JSON.stringify({error:String(e)})+'\n');process.exitCode=1;}
finally{const r=guest('b','bash /tmp/afs-v67-fault.sh restore');if(r.status!==0)process.exitCode=1;}

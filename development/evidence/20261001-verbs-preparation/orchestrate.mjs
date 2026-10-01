// Host metadata/orchestration only. Every probe and check runs in Linux.
import fs from 'node:fs';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';

const root='/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store';
const evidence=path.join(root,'evidence/afs-delivery/verbs-v69');
const commands=path.join(evidence,'commands.jsonl');
const probe=path.join(root,'source/development/acceptance/probes/env_verbs.py');
const vms={ctl:'192.168.109.11',a:'192.168.109.12',b:'192.168.109.13',c:'192.168.109.14'};
const out='/tmp/afs-verbs-v69';
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const hash=data=>createHash('sha256').update(data).digest('hex');
let sequence=0;
function command(node,args,stdin=null){
 const id=++sequence, argv=['shell','--workdir','/home/lzc.guest','afs-accept-'+node,'--',...args];
 const started=new Date().toISOString();
 const p=spawn('limactl',argv,{stdio:['pipe','pipe','pipe']});
 const stdout=[],stderr=[];
 p.stdout.on('data',x=>stdout.push(x));p.stderr.on('data',x=>stderr.push(x));
 p.stdin.end(stdin);
 const done=new Promise((resolve,reject)=>{
  p.once('error',reject);
  p.once('close',code=>{
   const result={id,node,argv:['limactl',...argv],started,ended:new Date().toISOString(),returncode:code,
      stdout:Buffer.concat(stdout).toString(),stderr:Buffer.concat(stderr).toString()};
   if(stdin!==null)result.stdin_sha256=hash(stdin);
   fs.appendFileSync(commands,JSON.stringify(result)+'\n');resolve(result);
  });
 });return {p,done};
}
async function checked(node,args,stdin=null){
 const r=await command(node,args,stdin).done;
 if(r.returncode!==0)throw Error(JSON.stringify(r));return r;
}
if(fs.existsSync(commands))throw Error('refusing to overwrite command evidence');
const source=fs.readFileSync(probe);
const observer=fs.readFileSync(path.join(evidence,'observe.py'));
for(const node of Object.keys(vms)) {
 await checked(node,['bash','-lc',`cat > ${out}/observe.py; sha256sum ${out}/observe.py`],observer);
 await checked(node,['sudo','python3',out+'/observe.py',out+'/protected-before.json']);
 await checked(node,['bash','-lc',`cat > ${out}/env_verbs.py; sha256sum ${out}/env_verbs.py`],source);
}

const pairs=[];
for(const client of Object.keys(vms))for(const server of Object.keys(vms))
 if(client!==server)pairs.push({client,server,size:256});
pairs.push({client:'a',server:'b',size:65535});
fs.writeFileSync(path.join(evidence,'matrix.json'),JSON.stringify({probe_sha256:hash(source),pairs},null,2)+'\n',{flag:'wx'});
for(let index=0;index<pairs.length;index++){
 const {client,server,size}=pairs[index];const label=`${client}-${server}-${size}`;
 const runId=randomUUID();const output=`${out}/pairs/${label}`;const port=19669;
 const args=role=>['python3',out+'/env_verbs.py','run','--role',role,'--bind',vms[role==='server'?server:client],
    '--peer',vms[role==='server'?client:server],'--port',String(port),'--size',String(size),
    '--count','3','--run-id',runId,'--output',output,'--timeout','25'];
 const running=command(server,args('server'));let ready=false;
 for(let attempt=0;attempt<35;attempt++){
  await sleep(150);
  const r=await command(server,['bash','-lc',`test -r ${output}/${runId}-server.raw.log && grep -Fx rdma_listen ${output}/${runId}-server.raw.log`]).done;
  if(r.returncode===0){ready=true;break;}
 }
 if(!ready){await running.done;throw Error('server not ready '+label);}
 const clientResult=await command(client,args('client')).done;
 const serverResult=await running.done;
 const pair={label,run_id:runId,client,server,size,port,count:3,client_exit:clientResult.returncode,server_exit:serverResult.returncode,
    client_report:`${output}/${runId}-client.json`,server_report:`${output}/${runId}-server.json`};
 fs.appendFileSync(path.join(evidence,'runs.jsonl'),JSON.stringify(pair)+'\n');
 console.log(`${index+1}/${pairs.length} ${label}: client=${clientResult.returncode} server=${serverResult.returncode}`);
 if(clientResult.returncode!==0||serverResult.returncode!==0)throw Error('endpoint failure '+label);
}
const negativeId=randomUUID();
const negative=await command('a',['python3',out+'/env_verbs.py','run','--role','client',
 '--bind',vms.a,'--peer',vms.b,'--port','19669','--size','256','--count','3',
 '--run-id',negativeId,'--output',out+'/negative-no-listener','--timeout','2']).done;
fs.writeFileSync(path.join(evidence,'negative.json'),JSON.stringify({run_id:negativeId,returncode:negative.returncode,
 report:`${out}/negative-no-listener/${negativeId}-client.json`,scope:'standalone absent-listener failure; not product fallback or required-mode proof'},null,2)+'\n',{flag:'wx'});
if(negative.returncode===0)throw Error('absent-listener probe falsely succeeded');
console.log('absent-listener negative returned '+negative.returncode);
for(const node of Object.keys(vms))
 await checked(node,['sudo','python3',out+'/observe.py',out+'/protected-after.json']);
console.log('matrix complete; raw logs and protected state retained in each guest');

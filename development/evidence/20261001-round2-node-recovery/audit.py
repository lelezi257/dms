#!/usr/bin/env python3
"""Audit actual retained Linux Node fault proof; no release qualification."""
import argparse
import json
import pathlib
import platform

NODE='d28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494'
META='64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7'
PAYLOAD='7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231'
PHYSICAL={'blake3-4e94e6f582581a0f3855f3ce504b153e951e65036fe9e2f010b7e25473c54f98-4194304':'a117210941a0b00dcb2d8577e680d84b6fa0eaf760d2afc654c953b9859d54fa','blake3-63c31766464b0c4931ff8b7406a2c1d8140d08b94328ccd7cf3b431d94cc690f-17':'31c8d139409af3a7ca3e686ff4409927e9f34154a1e6b24070b847a7b0f30111'}

def evaluate(root):
    checks=[]
    def need(name,value):
        checks.append({'name':name,'ok':bool(value)})
        assert value,name
    def load(p):return json.loads((root/p).read_text())
    def ready(row):return {c['record']['location']['Node']['node_id'] for c in row['copies'] if c['available']}
    before=load('ctl/identity-before-qualified.json')['identity']
    need('actual central authority',before['sha256']==META and before['role']=='meta' and bool(before['boot_id']) and bool(before['start_ticks']))
    for p in ('identity-during.json','identity-after-home-restart.json','identity-final.json','identity-complete.json'):
        need(p+' authority continuity',load('ctl/'+p)['identity']==before)
    for n in 'abc':
        i=load(n+'/identity-complete.json')['identity']
        need(n+' current executable',i['sha256']==NODE and i['role']=='node' and '/round1-mainline-v77-archive-async/' in i['exe'])
    ak=load('a/home-kill.json');ck=load('c/replica-kill.json')
    for n,k in (('a',ak),('c',ck)):
        need(n+' actual SIGKILL identity',k['dead'] and k['signal']=='SIGKILL' and k['identity']==load(n+'/identity-before.json')['identity'] and k['identity']['sha256']==NODE)
        restarted=load(n+'/identity-restarted.json')['identity']
        need(n+' original process replaced',restarted['pid']!=k['identity']['pid'] and restarted['boot_id']==k['identity']['boot_id'] and restarted['sha256']==NODE and restarted['exe']==k['identity']['exe'])
    need('B initial storage cold',load('b/physical-before.json')['chunks']==[])
    bc=load('b/identity-cold.json')['identity'];br=load('b/dfs-source-down.json')
    need('B cold process was actual reader',bc['pid']!=load('b/identity-before.json')['identity']['pid'] and br['identity']==bc)
    need('read inside source-outage window',ak['utc']<br['utc']<load('a/identity-restarted.json')['utc'])
    error=load('b/owner-home-down.json')
    need('Owner explicit unavailable error',error['outcome']=='error' and error['errno']==113 and error['bytes_before_error']==0)
    for n,p in (('b','dfs-source-down.json'),('b','owner-recovered.json'),('b','dfs-recovered.json'),('b','owner-target-down.json'),('b','dfs-target-down.json'),('c','owner-restarted.json'),('c','dfs-restarted.json')):
        r=load(n+'/'+p)
        need(n+'/'+p+' complete correct read',r['status']=='PASS' and r['outcome']=='success' and r['bytes']==4194321 and r['sha256']==PAYLOAD and r['elapsed_seconds']<60 and {m['source'] for m in r['mounts']}=={'afs-ownerfs','afs-dfs'})
    original=load('ctl/snapshot-before.json')['home']; recovered=load('ctl/snapshot-after-home-restart.json')['home']
    need('Home/session transition',recovered['home_serving'] and recovered['home_node_id']==original['home_node_id']=='round1-a' and recovered['home_session_id']!=original['home_session_id'] and recovered['home_lease_epoch']>original['home_lease_epoch'])
    repair=load('ctl/source-outage-repaired.json')
    need('expired Home explicitly unavailable',not repair['home']['home_serving'])
    for r in repair['replication']:
        need(r['chunk_id']+' replacement completed',ready(r)=={'round1-b','round1-c'} and r['health']=='Satisfied' and r['available_copies']==2 and any(t['state']=='Completed' and t['attempt']>=2 for t in r['tasks']))
    for n,p in (('a','physical-before.json'),('a','physical-restarted.json'),('b','physical-source-outage.json'),('c','physical-before.json'),('c','physical-restarted.json')):
        x=load(n+'/'+p)
        need(n+'/'+p+' physical hashes',{r['name']:r['sha256'] for r in x['chunks']}==PHYSICAL and sum(r['bytes'] for r in x['chunks'])==4194321)
        if n=='a':need(p+' Home bytes',x['home_file']['sha256']==PAYLOAD and x['home_file']['bytes']==4194321)
    target=load('ctl/target-outage-qualified.json')
    for r in target['replication']:
        need(r['chunk_id']+' target excluded',ready(r)=={'round1-a','round1-b'} and r['available_copies']==2 and r['health']=='Satisfied')
    final=load('ctl/snapshot-final.json')
    need('final Home and enough replicas',final['home']['home_serving'] and all(r['health']=='Satisfied' and r['available_copies']>=2 for r in final['replication']))
    def metric(path,*labels):
        rows=[float(s.rsplit(' ',1)[1]) for s in (root/path).read_text().splitlines() if s.startswith('afs_dfs_payload_bytes_total{') and all(t in s for t in labels)]
        need(path+' metric present '+str(labels),bool(rows));return sum(rows)
    need('cold surviving source used RXE',metric('c/metrics-source-outage.txt','operation="read"','transport="rdma"')-metric('baseline/c-metrics.txt','operation="read"','transport="rdma"')>=4194321)
    need('replacement received RXE bytes',metric('b/metrics-source-outage.txt','operation="replica"','transport="rdma"')>=4194321)
    for n in ('b','c'):need(n+' no DFS gRPC payload',metric(n+'/metrics-source-outage.txt','transport="grpc"')==0)
    for n,name in (('a','home-restart.log'),('c','replica-restart.log')):
        text=(root/n/name).read_text();need(n+' controller stale recovery','recovering disconnected dfs mount' in text and 'recovering disconnected ownerfs mount' in text and 'ready=true' in text)
    return checks

if __name__=='__main__':
    assert platform.system()=='Linux'
    p=argparse.ArgumentParser();p.add_argument('root',type=pathlib.Path);a=p.parse_args()
    checks=evaluate(a.root);print(json.dumps({'status':'PASS','level':'scoped fault integration','checks':checks,'formal_acceptance':'NOT_RUN','environment':'PREPARING','round2_complete':False},indent=2))

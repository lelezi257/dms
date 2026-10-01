#!/usr/bin/env python3
"""Collect real lease expiry and restart unchanged nodes under an awake host."""
import importlib.util,json,pathlib,time
ROOT=pathlib.Path(__file__).resolve().parents[3]
spec=importlib.util.spec_from_file_location('v60',ROOT/'experiments/afs-acceptance/repair-fault-v60.py');assert spec and spec.loader
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);f=m.fault
f.OUT=ROOT/'evidence/afs-delivery/session-expiry-v60-r2';f.v55.OUT=f.OUT
expected=json.loads((ROOT/'evidence/afs-delivery/repair-fault-v60-r2/root-independent-audit-v2.json').read_text())
observed={}
collect=r'''
import pathlib,time,json,hashlib,os
r=pathlib.Path(RUN);result={'now_unix':time.time(),'uptime':pathlib.Path('/proc/uptime').read_text(),'processes':{},'logs':{},'receipts':{}}
for role in ROLES:
 pid=int((r/'run'/(role+'.pid')).read_text());p=pathlib.Path('/proc',str(pid));item={'pid':pid,'alive':p.exists(),'binary_sha256':hashlib.sha256((r/'prefix/bin'/('afs-'+role)).read_bytes()).hexdigest(),'config_sha256':hashlib.sha256((r/'etc'/(role+'.toml')).read_bytes()).hexdigest()}
 if p.exists():item['start_ticks']=(p/'stat').read_text().rsplit(') ',1)[1].split()[19]
 result['processes'][role]=item
for p in (r/'log').glob('*'):
 if p.is_file():result['logs'][str(p)]=p.read_text(errors='replace')[-25000:]
for p in (r/'run').rglob('*'):
 if p.is_file() and p.name in ('exit','exit_code','exit.record','exit-code'):result['receipts'][str(p)]=p.read_text(errors='replace')
print(json.dumps(result))
'''
for which,roles in [('a',['meta','node']),('b',['node'])]:
 x=json.loads(f.guest_py(which,f.py_assignment(RUN=f.v55.RUN[which],ROLES=roles)+collect,guest_timeout=15));f.dump_unique('observation-'+which+'.json',x);observed[which]=x
 before=x['processes']['node'];prior=expected['live_identities'][which]
 assert not before['alive']
 assert before['binary_sha256']==f.NODE_SHA and before['config_sha256']==prior['config_sha256']['node']
 if which=='a':assert x['processes']['meta']['alive'] and x['processes']['meta']['pid']==prior['processes']['meta']['pid']
for which,roles in [('a',('meta','node')),('b',('node',))]:
 f.write_unique('restart-'+which+'.stdout',f.ctl(which,'start','node',timeout=45))
 live=f.live_identity(which,roles,'identity-'+which+'-after-restart.json');before=expected['live_identities'][which]
 assert live['config_sha256']==before['config_sha256'] and live['processes']['node']['pid']!=before['processes']['node']['pid']
 if which=='a':assert live['processes']['meta']==before['processes']['meta']
checks=[]
for index in range(8):
 row={'elapsed_seconds':index*10,'nodes':{}}
 for which,roles in [('a',('meta','node')),('b',('node',))]:
  live=f.live_identity(which,roles,f'awake-{index}-{which}.json');baseline=json.loads((f.OUT/('identity-'+which+'-after-restart.json')).read_text());f.assert_identity_matches(live,baseline,'awake unchanged '+which);row['nodes'][which]=live['processes']['node']
 checks.append(row);f.dump_unique(f'awake-sample-{index}.json',row)
 if index<7:time.sleep(10)
for which in ('a','b'):
 f.wait_rest(expected['target_outage' if which=='a' else 'source_outage']['chunk_id'],'available2_completed',15,'restored-copies-'+which+'.json')
f.dump_unique('awake-check.complete.json',{'status':'PASS','observed_seconds':70,'checks':checks,'scope':'unchanged v60 nodes alive across multiple30s lease windows with host idle-sleep prevention; not soak or proof of exclusive cause'})
print(json.dumps({'status':'PASS','observed_seconds':70}))

import pathlib,json,hashlib,os
base=pathlib.Path('/tmp/afs-network-v67-proof')
ips={'ctl':'192.168.109.11','a':'192.168.109.12','b':'192.168.109.13','c':'192.168.109.14'}
checks=[]
def check(name,ok,detail):checks.append({'name':name,'status':'PASS' if ok else 'FAIL','detail':detail})
def load(p):return json.loads(p.read_text())
def rules(text):return [line for line in text.splitlines() if not line.startswith('#')]
identities=[];tokens=[]
for x in ips:
 root=base/x;ready=load(root/'ready.json');before=load(root/'logs/preserved-before.json');after=load(root/'logs/preserved-after.json')
 identities.append(before['machine_id']);check(x+'-identity',before['hostname']==after['hostname']==ready['hostname'] and before['boot_id']==after['boot_id'] and before['machine_id']==after['machine_id'],'same guest boot and machine identity')
 check(x+'-preserved',before['processes']==after['processes'] and before['mountinfo']==after['mountinfo'] and rules(before['iptables'])==rules(after['iptables']),'AFS PID/start/exe/config, mounts and existing rules unchanged')
 check(x+'-probe-sha',ready['script_sha256']==hashlib.sha256((base/'env_network.py').read_bytes()).hexdigest(),'same frozen probe source')
 for y in ips:
  if x==y:continue
  r=load(root/f'logs/pair-{x}-{y}.json');c=r['checks'];tokens.append(r.get('token_sha256'))
  check(x+'-'+y+'-positive',r['status']=='PASS' and r['source_bind']==ips[x] and r['target']==ips[y] and all(c[k]['status']=='PASS' and c[k].get('bytes')==32 for k in ('tcp','udp','tls')),'fixed-IP TCP/UDP/mTLS exact 32-byte exchange')
  check(x+'-'+y+'-negative',all(c[k]['status']=='PASS' for k in ('tls_untrusted_ca','tls_wrong_hostname','tls_missing_client_cert')),'TLS rejection, not arbitrary socket timeout')
check('four-independent-machines',len(set(identities))==4,'four actual machine IDs')
check('fresh-pair-nonces',len(set(tokens))==12 and None not in tokens,'twelve distinct fresh nonce digests')
a=base/'a/logs';b=base/'b/logs'
wrong=load(a/'wrong-client.json');check('wrong-client-cert',wrong['status']=='PASS' and wrong['checks']['tls_untrusted_client_cert']['status']=='PASS','untrusted client rejected with authenticated server')
for name in ('fault-before','fault-restored'):
 r=load(a/(name+'.json'));check(name,r['status']=='PASS' and all(r['checks'][k]['status']=='PASS' for k in ('tcp','udp','tls')),'same-pair exact exchange')
r=load(a/'fault-injected.json');check('fault-bounded-failure',r['status']=='FAIL' and all(r['checks'][k]['status']=='FAIL' for k in ('tcp','udp','tls')),'all intended protocol checks fail under actual directed DROP')
rows=[line.split() for line in (b/'iptables-hit.txt').read_text().splitlines() if 'afs-env-v67-only' in line]
check('fault-hit',len(rows)==2 and all(int(row[0])>0 and row[2]=='DROP' for row in rows),'TCP and UDP targeted rule counters are nonzero')
check('fault-rule-restored',rules((b/'iptables-before.txt').read_text())==rules((b/'iptables-final-restored.txt').read_text()),'exact original rule set restored')
check('watchdog-complete',(b/'watchdog-complete.txt').is_file(),'independent bounded cleanup ran')
report={'scope':'Standalone network/TLS environment preparation only; not full ENV, RoCE/product RDMA, product authorization or REL10','status':'PASS' if all(c['status']=='PASS' for c in checks) else 'FAIL','checks':checks,'counts':{'pass':sum(c['status']=='PASS' for c in checks),'fail':sum(c['status']=='FAIL' for c in checks)}}
(base/'semantic-result.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));raise SystemExit(0 if report['status']=='PASS' else 1)

#!/usr/bin/env python3
"""Linux-only retained-fault-evidence negative regressions."""
import importlib.util
import json
import pathlib
import platform
import shutil
import tempfile
assert platform.system()=='Linux'
root=pathlib.Path(__file__).resolve().parent
s=importlib.util.spec_from_file_location('audit',root/'audit.py');m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
m.evaluate(root);results=[{'name':'valid scoped fault packet','ok':True}]
cases=[('false killed proof','a/home-kill.json',lambda v:v.update(dead=False)),('different Meta','ctl/identity-final.json',lambda v:v['identity'].update(pid=1)),('uncold initial source','b/physical-before.json',lambda v:v.update(chunks=[{}])),('Owner false success','b/owner-home-down.json',lambda v:v.update(outcome='success')),('bad read bytes','b/dfs-source-down.json',lambda v:v.update(sha256='0'*64)),('incomplete repair','ctl/source-outage-repaired.json',lambda v:v['replication'][0].update(tasks=[])),('wrong physical chunk','c/physical-restarted.json',lambda v:v['chunks'][0].update(sha256='0'*64))]
for name,path,mutation in cases:
    with tempfile.TemporaryDirectory() as t:
        copy=pathlib.Path(t)/'packet';shutil.copytree(root,copy);p=copy/path;v=json.loads(p.read_text());mutation(v);p.write_text(json.dumps(v))
        try:m.evaluate(copy)
        except AssertionError as e:results.append({'name':name,'ok':True,'rejected':str(e)})
        else:raise AssertionError('Invalid proof accepted: '+name)
print(json.dumps({'status':'PASS','results':results},indent=2))

import importlib.util,json,pathlib,shutil,sys,tempfile
assert sys.platform=='linux'
root=pathlib.Path(sys.argv[1]);spec=importlib.util.spec_from_file_location('closure',root/'probes/round2-closure-audit.py');module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
results=[]
def reject(name,path,mutate):
 with tempfile.TemporaryDirectory() as d:
  p=pathlib.Path(d)/'packet';shutil.copytree(root,p)
  f=p/path;v=json.loads(f.read_text());mutate(v);f.write_text(json.dumps(v))
  try:module.audit(p)
  except (AssertionError,KeyError,TypeError,ValueError):results.append(name)
  else:raise AssertionError('unexpected PASS '+name)
reject('cold read corruption','overall/a/evidence/cold-read-r1.json',lambda v:v['result']['read'].update(sha256='bad'))
reject('transport fallback','overall/b/evidence/metrics-r1.json',lambda v:v['result']['payload_counters'].update({'grpc/recv/replica':4096}))
reject('no real RDMA bytes','overall/b/evidence/metrics-r1.json',lambda v:v['result']['payload_counters'].update({'rdma/recv/replica':0}))
reject('no restart','overall/a/evidence/restart-r1.json',lambda v:v['result']['identity'].update(pid=749518))
reject('short reserve','overall/a/evidence/cleanup-r2.json',lambda v:v.update(data_available_bytes=0))
reject('failed consistency','overall/consistency-r2/report.json',lambda v:v.update(status='FAIL'))
reject('lost copies','overall/ctl/evidence/replicas-r1.json',lambda v:v['result']['polls'][-1]['value'].update(available_copies=0))
reject('compiler delta','build/dfs-deadline-v82-final/input-audit.json',lambda v:v.update(changed_inputs=['src/node/node.rs']))
reject('bad caller status','build/dfs-deadline-v82-r3/cancel/caller-error.json',lambda v:v.update(kind='Success',code=0))
print(json.dumps({'status':'PASS','count':len(results),'negative_tests':results},indent=2))

import hashlib,json,os,pathlib,platform,subprocess,time
assert platform.system()=="Linux"
base=pathlib.Path("/home/lzc.guest/afs-build")
root=base/"work/root-merged-v60-r2"
out=base/"probes/repair-v60/qualified-linux-clean-r2"
out.mkdir(parents=True,exist_ok=False)
env=os.environ.copy(); env.update(PATH="/home/lzc.guest/.cargo/bin:"+env["PATH"],CARGO_TARGET_DIR=str(base/"target"),CARGO_BUILD_JOBS="2",CARGO_INCREMENTAL="0")
names=json.loads((base/"probes/repair-v56/input-names.json").read_text())
def inputs(): return {name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in names}
initial=inputs(); (out/"compile-inputs.json").write_text(json.dumps({"file_count":len(initial),"files":initial},indent=2)+"\n")
checks=[]
def run(label,args,budget=240):
 started=time.monotonic()
 with (out/(label+".log")).open("w") as log:
  p=subprocess.run(["timeout",str(budget)]+args,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
 row={"label":label,"argv":["timeout",str(budget)]+args,"returncode":p.returncode,"elapsed_seconds":time.monotonic()-started}; checks.append(row)
 (out/"checks.json").write_text(json.dumps(checks,indent=2)+"\n")
 print(label,p.returncode,round(row["elapsed_seconds"],3),flush=True)
 assert p.returncode==0,row
 assert inputs()==initial,"source changed during gate"
run("fmt",["cargo","fmt","--all","--","--check"],60)
run("clippy",["cargo","clippy","--workspace","--all-targets","--all-features","--","-D","warnings"])
run("library",["cargo","test","--all-features","--lib","--","--nocapture"])
run("contracts",["cargo","test","--all-features","--test","config_contract","--test","error_contract","--test","fuse_contract","--test","meta_contract","--test","ownerfs_peer_contract","--test","rest_contract","--test","vfs_contract","--","--nocapture"])
run("error",["cargo","test","-p","afs-error","--","--nocapture"],180)
run("local-api",["cargo","test","--all-features","--test","local_sdk","--","--nocapture"],180)
run("fuse-build",["cargo","test","--all-features","--test","fuse_contract","--no-run","--message-format=json"],120)
art=[]
for line in (out/"fuse-build.log").read_text().splitlines():
 try: item=json.loads(line)
 except json.JSONDecodeError: continue
 if item.get("reason")=="compiler-artifact" and item.get("target",{}).get("name")=="fuse_contract" and item.get("executable"): art.append(item["executable"])
assert len(art)==1,art
run("real-root-fuse",["sudo",art[0],"--ignored","--test-threads=1","--nocapture"],120)
for label,feature in [("feature-none",None),("feature-ownerfs","ownerfs"),("feature-dfs","dfs")]:
 args=["cargo","check","--no-default-features"]
 if feature: args += ["--features",feature]
 run(label,args,120)
run("build",["cargo","build","--all-features","--bins"],180)
import shutil
artifacts=base/"artifacts/v60-qualified"; artifacts.mkdir(exist_ok=False)
binaries={}
for name in ("afs-node","afs-meta"):
 shutil.copy2(base/"target/debug"/name,artifacts/name); binaries[name]=hashlib.sha256((artifacts/name).read_bytes()).hexdigest()
report={"status":"PASS","platform":platform.platform(),"input_count":len(initial),"checks":checks,"binary_sha256":binaries,"artifacts":str(artifacts)}
(out/"report.json").write_text(json.dumps(report,indent=2)+"\n")
print(json.dumps(report),flush=True)

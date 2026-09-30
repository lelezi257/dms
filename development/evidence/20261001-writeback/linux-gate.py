#!/usr/bin/env python3
import hashlib,json,os,pathlib,platform,subprocess,time
assert platform.system()=="Linux"
root=pathlib.Path("/home/lzc.guest/afs-build/work/root-merged-v49")
out=pathlib.Path("/home/lzc.guest/afs-build/probes/writeback-v49-r2");out.mkdir(exist_ok=False)
env=os.environ.copy();env.update(PATH="/home/lzc.guest/.cargo/bin:"+env["PATH"],CARGO_TARGET_DIR="/home/lzc.guest/afs-build/target",CARGO_BUILD_JOBS="2",CARGO_INCREMENTAL="0")
manifest=json.loads(pathlib.Path("/var/tmp/afs-v49-compile-inputs.json").read_text())
inputs={n:hashlib.sha256((root/n).read_bytes()).hexdigest() for n in manifest["files"]};assert inputs==manifest["files"]
(out/"linux-source-hashes.json").write_text(json.dumps({"platform":platform.platform(),"file_count":len(inputs),"files":inputs},indent=2)+"\n")
checks=[]
def run(label,args,limit=240,fail=False):
 started=time.monotonic();command=["timeout",str(limit)]+args
 result=subprocess.run(command,cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
 (out/(label+".log")).write_text(result.stdout)
 record={"label":label,"argv":command,"returncode":result.returncode,"expected_failure":fail,"elapsed_seconds":time.monotonic()-started,"source":{"src/node/vfs/dfs.rs":hashlib.sha256((root/"src/node/vfs/dfs.rs").read_bytes()).hexdigest(),"src/node/rpc/meta.rs":hashlib.sha256((root/"src/node/rpc/meta.rs").read_bytes()).hexdigest()}}
 checks.append(record);(out/"checks.json").write_text(json.dumps(checks,indent=2)+"\n");print(label,result.returncode,flush=True)
 assert (result.returncode==101 if fail else result.returncode==0),(label,result.returncode,result.stdout[-2500:])
 return result
run("fmt",["cargo","fmt","--all","--","--check"],60)
run("target-writeback",["cargo","test","--all-features","--lib","background_writeback","--","--nocapture"],240)
run("target-rpc",["cargo","test","--all-features","--lib","grpc_dfs_meta_focused_timeouts_bound_hanging_rpc","--","--nocapture"],60)
dfs=root/"src/node/vfs/dfs.rs";meta=root/"src/node/rpc/meta.rs";final_dfs=dfs.read_text();final_meta=meta.read_text()
def span(s,marker):
 start=s.index(marker);brace=s.index("{",start);depth=1;i=brace+1
 while depth:
  depth+=(s[i]=="{")-(s[i]=="}");i+=1
 return brace,i
try:
 old=pathlib.Path("/var/tmp/afs-v49-original-dfs.rs").read_text();a,b=span(old,"pub fn writeback_pending(");x,y=span(final_dfs,"fn writeback_pending_with_budget(")
 dfs.write_text(final_dfs[:x]+old[a:b]+final_dfs[y:])
 run("original-unbounded-scan",["cargo","test","--all-features","--lib","background_writeback_rotates_bounded_scan_across_clean_and_busy_entries","--","--nocapture"],120,True)
finally:dfs.write_text(final_dfs)
try:
 changed=final_dfs
 for marker in ("            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {\n                let mut state = cell", "            if timeout.is_some_and(|timeout| timeout.is_zero()) {"):
  a=changed.index(marker);b,c=span(changed,marker);changed=changed[:a]+changed[c:]
 dfs.write_text(changed)
 run("original-send-after-expiry",["cargo","test","--all-features","--lib","background_writeback_expired_before_send_retains_exact_pending_for_replay","--","--nocapture"],120,True)
finally:dfs.write_text(final_dfs)
try:
 changed=final_meta
 for name in ("renew_write_lease_with_timeout","sync_inode_metadata_with_timeout","commit_file_version_with_timeout"):
  marker="    fn "+name+"(";a=changed.index(marker);b,c=span(changed,marker);part=changed[a:c];assert part.count(".run_with_timeout(")==1
  part=part.replace(".run_with_timeout(",".run(").replace("                timeout,\n","");changed=changed[:a]+part+changed[c:]
 meta.write_text(changed)
 run("original-fixed-rpc-timeout",["cargo","test","--all-features","--lib","grpc_dfs_meta_focused_timeouts_bound_hanging_rpc","--","--nocapture"],120,True)
finally:meta.write_text(final_meta)
run("clippy",["cargo","clippy","--workspace","--all-targets","--all-features","--","-D","warnings"],240)
run("library",["cargo","test","--all-features","--lib","--","--nocapture"],240)
run("local-api",["cargo","test","--all-features","--test","local_sdk","--","--nocapture"],120)
run("error",["cargo","test","-p","afs-error","--","--nocapture"],120)
run("contracts",["cargo","test","--all-features","--test","config_contract","--test","error_contract","--test","fuse_contract","--test","meta_contract","--test","ownerfs_peer_contract","--test","rest_contract","--test","vfs_contract","--","--nocapture"],240)
result=run("fuse-build",["cargo","test","--all-features","--test","fuse_contract","--no-run","--message-format=json"],120)
items=[]
for line in result.stdout.splitlines():
 try:item=json.loads(line)
 except ValueError:continue
 if item.get("reason")=="compiler-artifact" and item.get("target",{}).get("name")=="fuse_contract" and item.get("executable"):items.append(item["executable"])
assert len(items)==1
run("real-root-fuse",["sudo","timeout","120",items[0],"--ignored","--test-threads=1","--nocapture"],130)
for features in (None,"ownerfs","dfs"):
 cmd=["cargo","check","--no-default-features"]
 if features:cmd += ["--features",features]
 run("feature-"+(features or "none"),cmd,120)
run("build",["cargo","build","--all-features","--bins"],240)
artifacts=pathlib.Path("/home/lzc.guest/afs-build/artifacts/v49-qualified");artifacts.mkdir(exist_ok=False)
binaries={}
for name in ("afs-node","afs-meta"):
 p=artifacts/name;p.write_bytes((pathlib.Path(env["CARGO_TARGET_DIR"])/"debug"/name).read_bytes());p.chmod(0o755);binaries[name]=hashlib.sha256(p.read_bytes()).hexdigest()
assert {n:hashlib.sha256((root/n).read_bytes()).hexdigest() for n in manifest["files"]}==inputs
(out/"report.json").write_text(json.dumps({"status":"PASS","platform":platform.platform(),"input_count":len(inputs),"checks":checks,"binaries":binaries},indent=2)+"\n")
print("PASS",json.dumps(binaries),flush=True)

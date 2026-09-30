#!/usr/bin/env python3
"""Linux-only diagnostic: controller must not report failed child exit as clean."""
import argparse, hashlib, json, os, pathlib, platform, signal, subprocess, time
p = argparse.ArgumentParser()
p.add_argument("--controller", required=True)
p.add_argument("--run-dir", required=True)
a = p.parse_args()
assert platform.system() == "Linux"
root = pathlib.Path(a.run_dir); root.mkdir(parents=True, exist_ok=False)
c = root / "fixture.c"
c.write_text(r"""
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <unistd.h>
static int code;
static void stop(int sig) { (void)sig; _exit(code); }
int main(int argc, char **argv) {
  if (argc != 3) return 2;
  FILE *cfg = fopen(argv[2], "r");
  if (!cfg || fscanf(cfg, "%d", &code) != 1) return 2;
  fclose(cfg);
  signal(SIGTERM, stop);
  puts("fixture-ready"); fflush(stdout);
  for (;;) pause();
}
""")
exe=root/"fixture"
subprocess.run(["cc", "-Wall", "-Wextra", "-Werror", str(c), "-o", str(exe)], check=True)
results=[]
for exit_code in (0,1,124):
 lane=root/str(exit_code)
 for d in ("prefix/bin","etc","run","log"):(lane/d).mkdir(parents=True,exist_ok=True)
 binary=lane/"prefix/bin/afs-node";binary.write_bytes(exe.read_bytes());binary.chmod(0o755)
 cfg=lane/"etc/node.toml";cfg.write_text(str(exit_code)+"\n")
 # Direct parent obtains the real wait status for this exact fixture/config.
 direct=subprocess.Popen([str(binary),"--config",str(cfg)],stdout=subprocess.PIPE,text=True)
 assert direct.stdout.readline().strip()=="fixture-ready"
 direct.send_signal(signal.SIGTERM); direct_status=direct.wait(timeout=3)
 assert direct_status==exit_code
 args=[a.controller,"--prefix",str(lane/"prefix"),"--config-dir",str(lane/"etc"),"--run-dir",str(lane/"run"),"--log-dir",str(lane/"log"),"--timeout","5","--no-readiness"]
 start=subprocess.run(args+["start","node"],capture_output=True,text=True,timeout=8)
 assert start.returncode==0,(start.stdout,start.stderr)
 pid=int((lane/"run/node.pid").read_text())
 try:
  started=time.monotonic(); stop=subprocess.run(args+["stop","node"],capture_output=True,text=True,timeout=8)
  elapsed=time.monotonic()-started
  expected=exit_code==0
  ok=(stop.returncode==0)==expected
  results.append({"fixture_exit_code":exit_code,"direct_wait_status":direct_status,"controller_returncode":stop.returncode,"controller_stdout":stop.stdout,"controller_stderr":stop.stderr,"pid":pid,"elapsed_seconds":elapsed,"ok":ok})
 finally:
  try:os.kill(pid,signal.SIGTERM)
  except ProcessLookupError:pass
report={"status":"PASS" if all(r["ok"] for r in results) else "FAIL","scope":"controller exit-status diagnostic with native fixture; not a product shutdown acceptance","controller_sha256":hashlib.sha256(pathlib.Path(a.controller).read_bytes()).hexdigest(),"platform":platform.platform(),"checks":results}
(root/"report.json").write_text(json.dumps(report,indent=2)+"\n")
print(json.dumps(report,indent=2));raise SystemExit(0 if report["status"]=="PASS" else 1)

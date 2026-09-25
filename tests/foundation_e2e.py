#!/usr/bin/env python3
"""Linux-only real-process framework acceptance. Python is a test tool, not runtime."""
import argparse, errno, json, os, pathlib, signal, socket, subprocess, tempfile, time, urllib.request

p=argparse.ArgumentParser();p.add_argument('--bin-dir',required=True);p.add_argument('--rdma-device');p.add_argument('--skip-fuse',action='store_true');p.add_argument('--output',required=True);args=p.parse_args()
out=pathlib.Path(args.output);out.mkdir(parents=True,exist_ok=True)
procs=[];logs=[];results=[];mounts=[];collector=None
trace_port=None
root=pathlib.Path(tempfile.mkdtemp(prefix='afs-e2e-'))

def port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
def http(port_,path,method='GET'):
    with urllib.request.urlopen(urllib.request.Request(f'http://127.0.0.1:{port_}{path}',method=method),timeout=30) as r:return r.read().decode()
def check(name,condition,detail=''):
    results.append(dict(name=name,passed=bool(condition),detail=detail));assert condition,(name,detail)
def start(name,*options):
    file=open(out/f'{name}.log','w');logs.append(file)
    exe='afs-meta' if name=='meta' else 'afs-node'
    trace_args=['--trace-enabled','true','--trace-sample-ratio','1','--trace-endpoint',f'http://127.0.0.1:{trace_port}'] if trace_port else []
    proc=subprocess.Popen([str(pathlib.Path(args.bin_dir)/exe),*options,*trace_args],stdout=file,stderr=subprocess.STDOUT);procs.append(proc);return proc

def ready(proc,port_):
    end=time.monotonic()+20
    while time.monotonic()<end:
        if proc.poll() is not None:raise RuntimeError(f'process early exit {proc.returncode}; inspect {out}')
        try:return json.loads(http(port_,'/health'))
        except OSError:time.sleep(.05)
    raise RuntimeError('health timeout')
def run(*cmd):return subprocess.run(cmd,text=True,capture_output=True,timeout=30)
def example(name):
    path=pathlib.Path(args.bin_dir)/'examples'/name
    if path.exists() and os.access(path,os.X_OK):return path
    matches=sorted(path.parent.glob(f'{name}-*'),key=lambda p:p.stat().st_mtime,reverse=True)
    for candidate in matches:
        if candidate.is_file() and os.access(candidate,os.X_OK):
            return candidate
    raise FileNotFoundError(path)
try:
    trace_port=port()
    collector_log=open(out/'collector.log','w');logs.append(collector_log)
    collector=subprocess.Popen([str(example('trace_collector')),f'127.0.0.1:{trace_port}',str(out/'spans.jsonl')],stdout=collector_log,stderr=subprocess.STDOUT)
    end=time.monotonic()+10
    while True:
        try:
            with socket.create_connection(('127.0.0.1',trace_port),timeout=.1):break
        except OSError:
            if time.monotonic()>end:raise RuntimeError('collector not ready')
            time.sleep(.05)
    mg,mr,ag,ar,bg,br=[port() for _ in range(6)]
    meta=start('meta','--grpc-listen',f'127.0.0.1:{mg}','--rest-listen',f'127.0.0.1:{mr}');ready(meta,mr)
    mode='rdma' if args.rdma_device else 'grpc'
    node_options=[]
    if args.rdma_device:node_options=['--rdma-device',args.rdma_device]
    a=start('node-a','--id','a','--grpc-listen',f'127.0.0.1:{ag}','--rest-listen',f'127.0.0.1:{ar}','--data-dir',str(root/'a-data'),'--uds-path',str(root/'a.sock'),'--fs','all',*node_options);ready(a,ar)
    mount=root/'mnt';mount.mkdir()
    fuse_args=[] if args.skip_fuse else ['--mount',str(mount)]
    if fuse_args:mounts.append(mount)
    b=start('node-b','--id','b','--grpc-listen',f'127.0.0.1:{bg}','--rest-listen',f'127.0.0.1:{br}','--data-dir',str(root/'b-data'),'--uds-path',str(root/'b.sock'),'--fs','all','--meta-endpoint',f'http://127.0.0.1:{mg}','--peer-endpoint',f'http://127.0.0.1:{ag}','--data-mode',mode,*node_options,*fuse_args);ready(b,br)
    sdk=run(str(example('local_roundtrip')),str(root/'a.sock'),'sdk-eight')
    check('local SDK UDS and real SHM',sdk.returncode==0,sdk.stderr)
    sdk_result=json.loads(sdk.stdout)
    check('SDK8bytes readback',sdk_result.get('ok') is True and sdk_result.get('written')==8,sdk_result)
    check('SDK used actual local file',(root/'a-data'/'diagnostics'/'sdk-eight').read_bytes()==b'AFShello')
    check('meta REST ping','pong' in http(mr,'/v1/ping'))
    check('node REST ping','pong' in http(br,'/v1/ping'))
    result=json.loads(http(br,'/v1/diagnostics','POST'))
    check('node-meta + node-node control/data',result.get('ok') is True,result)
    check('selected data adapter',result.get('mode')==mode,result)
    check('meta metrics observed', 'afs_requests_total' in http(mr,'/metrics'))
    check('node metrics observed', 'afs_requests_total' in http(br,'/metrics'))
    if not args.skip_fuse:
        check('FUSE namespaces',sorted(os.listdir(mount))==['blobfs','ownerfs'])
        for fs in ('ownerfs','blobfs'):
            try:
                fd=os.open(mount/fs/'hello',os.O_CREAT|os.O_WRONLY,0o600);os.close(fd);ok=False
            except OSError as e:ok=e.errno==errno.ENOSYS
            check(f'FUSE {fs} dispatch Unsupported',ok)
        collision=run(str(pathlib.Path(args.bin_dir)/'afs-node'),'--id','mount-duplicate','--grpc-listen',f'127.0.0.1:{port()}','--rest-listen',f'127.0.0.1:{port()}','--uds-path',str(root/'mount-dup.sock'),'--data-dir',str(root/'mount-dup'),'--mount',str(mount))
        check('live FUSE mount collision rejected',collision.returncode!=0,collision.stderr[-500:])
        check('original mount preserved',sorted(os.listdir(mount))==['blobfs','ownerfs'])
        check('failed startup UDS cleaned',not (root/'mount-dup.sock').exists())
    bad=run(str(pathlib.Path(args.bin_dir)/'afs-node'),'--fs','typo','--print-config')
    check('bad configuration rejected',bad.returncode!=0)
    # A duplicate Node may not delete a live UDS pathname.
    dup=run(str(pathlib.Path(args.bin_dir)/'afs-node'),'--id','duplicate','--grpc-listen',f'127.0.0.1:{port()}','--rest-listen',f'127.0.0.1:{port()}','--uds-path',str(root/'a.sock'),'--data-dir',str(root/'dup'))
    check('UDS collision rejected',dup.returncode!=0,dup.stderr[-500:])
    check('original UDS preserved',(root/'a.sock').exists())
    ready(a,ar)
finally:
    for proc in reversed(procs):
        if proc.poll() is None:proc.send_signal(signal.SIGTERM)
    for proc in reversed(procs):
        try:proc.wait(timeout=15)
        except subprocess.TimeoutExpired:proc.kill();proc.wait();results.append(dict(name='bounded shutdown',passed=False))
    for mount in mounts:
        if os.path.ismount(mount):subprocess.run(['fusermount3','-u',str(mount)],capture_output=True)
    if collector:
        collector.terminate();collector.wait(timeout=5)
    for log in logs:log.close()
    if (out/'spans.jsonl').exists():
        spans=[json.loads(line) for line in (out/'spans.jsonl').read_text().splitlines()]
        by_trace={}
        for span in spans:by_trace.setdefault(span['trace_id'],[]).append(span)
        required={'node.diagnostics','meta.ping','node.control.ping','node.data.read','node.data.write'}
        linked=any(required.issubset({s['name'] for s in group}) for group in by_trace.values())
        results.append(dict(name='actual OTLP Meta/Node same trace',passed=linked,detail={'spans':len(spans)}))
    else:results.append(dict(name='actual OTLP Meta/Node same trace',passed=False))
    results.append(dict(name='services exit cleanly',passed=all(p.returncode==0 for p in procs),detail=[p.returncode for p in procs]))
    if not args.skip_fuse:
        node_log=(out/'node-b.log').read_text()
        results.append(dict(name='both FUSE backends logged',passed='ownerfs.create' in node_log and 'blobfs.create' in node_log))
    results.append(dict(name='owned UDS cleaned',passed=not (root/'a.sock').exists() and not (root/'b.sock').exists()))
    (out/'results.json').write_text(json.dumps(dict(root=str(root),results=results),indent=2))
print(json.dumps(results,indent=2));assert all(r['passed'] for r in results)

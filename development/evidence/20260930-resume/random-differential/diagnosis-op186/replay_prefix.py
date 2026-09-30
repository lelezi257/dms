import errno, hashlib, json, os, shutil, stat, sys, time
from pathlib import Path

def safe_join(root, rel):
    root_abs=root.resolve()
    p=root_abs/rel
    norm=Path(os.path.normpath(str(p)))
    if root_abs != norm and root_abs not in norm.parents:
        raise ValueError(rel)
    return norm

def prepare(root):
    root.mkdir(parents=True, exist_ok=False)
    for d in ["d0","d1","d2","d3"]:
        (root/d).mkdir()

def sha(path):
    h=hashlib.sha256()
    with open(path,"rb") as f:
        for b in iter(lambda:f.read(65536), b""):
            h.update(b)
    return h.hexdigest()

def snap(root, rel):
    p=safe_join(root, rel)
    try: st=os.lstat(p)
    except OSError as e: return {"exists":False,"errno":e.errno,"errno_name":errno.errorcode.get(e.errno)}
    r={"exists":True,"mode":stat.S_IMODE(st.st_mode),"nlink":st.st_nlink}
    if stat.S_ISDIR(st.st_mode): r.update(type="dir", entries=sorted(x.name for x in p.iterdir()))
    elif stat.S_ISLNK(st.st_mode): r.update(type="symlink", size=st.st_size, target=os.readlink(p))
    elif stat.S_ISREG(st.st_mode): r.update(type="file", size=st.st_size, sha256=sha(p))
    else: r.update(type=str(stat.S_IFMT(st.st_mode)))
    return r

def tree(root):
    entries={}
    for p in sorted(root.rglob("*"), key=lambda x:str(x.relative_to(root))):
        entries[str(p.relative_to(root))]=snap(root, str(p.relative_to(root)))
    enc=json.dumps(entries, sort_keys=True, separators=(",",":")).encode()
    return {"sha256":hashlib.sha256(enc).hexdigest(),"entry_count":len(entries),"entries":entries}

def ok(**kw):
    r={"ok":True,"errno":None}; r.update(kw); return r

def er(e): return {"ok":False,"errno":e.errno,"errno_name":errno.errorcode.get(e.errno)}

def close(fd):
    if fd is not None:
        try: os.close(fd)
        except OSError: pass

def apply(root, op):
    fd=None
    try:
        name=op["op"]
        if name=="create": fd=os.open(safe_join(root,op["path"]), os.O_CREAT|os.O_EXCL|os.O_WRONLY, int(op["mode"])); return ok()
        if name=="open_read": fd=os.open(safe_join(root,op["path"]), os.O_RDONLY); return ok()
        if name=="write": fd=os.open(safe_join(root,op["path"]), os.O_CREAT|os.O_WRONLY, 0o644); return ok(written=os.write(fd, bytes.fromhex(op["data_hex"])))
        if name=="pwrite": fd=os.open(safe_join(root,op["path"]), os.O_CREAT|os.O_RDWR, 0o644); return ok(written=os.pwrite(fd, bytes.fromhex(op["data_hex"]), int(op["offset"])))
        if name=="append": fd=os.open(safe_join(root,op["path"]), os.O_CREAT|os.O_WRONLY|os.O_APPEND, 0o644); return ok(written=os.write(fd, bytes.fromhex(op["data_hex"])))
        if name=="read": fd=os.open(safe_join(root,op["path"]), os.O_RDONLY); data=os.pread(fd, int(op["length"]), int(op["offset"])); return ok(read_len=len(data), sha256=hashlib.sha256(data).hexdigest(), eof=len(data)<int(op["length"]))
        if name=="truncate": os.truncate(safe_join(root,op["path"]), int(op["size"])); return ok()
        if name=="sparse": fd=os.open(safe_join(root,op["path"]), os.O_CREAT|os.O_RDWR, 0o644); return ok(written=os.pwrite(fd, bytes.fromhex(op["data_hex"]), int(op["offset"])))
        if name=="rename": os.replace(safe_join(root,op["src"]), safe_join(root,op["dst"])); return ok()
        if name=="hardlink": os.link(safe_join(root,op["src"]), safe_join(root,op["dst"])); return ok()
        if name=="symlink": os.symlink(op["target"], safe_join(root,op["path"])); return ok()
        if name=="unlink": os.unlink(safe_join(root,op["path"])); return ok()
        if name=="stat": return ok(snapshot=snap(root, op["path"]))
        raise RuntimeError(name)
    except OSError as e: return er(e)
    finally: close(fd)

def run(iteration):
    ops=[r["operation"] for r in json.loads(Path("trace.json").read_text())[:187]]
    ts=time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    ref=Path("/mnt/lima-afsadata/afs-delivery/evidence/p2-random-v27/diagnosis-op186/ext4-ref")/f"ref-{iteration}-{ts}"
    tgt=Path("/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v27/mount-dfs")/f"diag-op186-{iteration}-{ts}"
    for p in (ref,tgt):
        if p.exists(): shutil.rmtree(p)
        prepare(p)
    events=[]
    mismatch=None
    for i,op in enumerate(ops):
        rr=apply(ref,op); tr=apply(tgt,op)
        rt=tree(ref); tt=tree(tgt)
        ev={"index":i,"operation":op,"reference_result":rr,"target_result":tr,"reference_tree_sha256":rt["sha256"],"target_tree_sha256":tt["sha256"]}
        if i in range(180,187) or rr!=tr or rt["sha256"]!=tt["sha256"]:
            ev["reference_d0_f0"]=snap(ref,"d0/f0")
            ev["target_d0_f0"]=snap(tgt,"d0/f0")
            events.append(ev)
        if rr!=tr or rt["sha256"]!=tt["sha256"]:
            mismatch=ev; break
    out={"iteration":iteration,"reference":str(ref),"target":str(tgt),"mismatch":mismatch,"tail_events":events}
    Path(f"diagnosis-repeat-{iteration}.json").write_text(json.dumps(out,indent=2,sort_keys=True)+"\n")
    print(json.dumps({"iteration":iteration,"mismatch_index": None if mismatch is None else mismatch["index"], "target_result": None if mismatch is None else mismatch["target_result"]}, sort_keys=True))

for i in (1,2): run(i)

import errno, json, os, time
from pathlib import Path
stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
root = Path("/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v27/mount-dfs") / f"diag-expiry-{stamp}"
result = {"root": str(root)}
root.mkdir(parents=True, exist_ok=False)
path = root / "lease-file"
fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o644)
try:
    result["initial_write"] = {"written": os.write(fd, b"abcdef")}
finally:
    os.close(fd)
result["sleep_seconds"] = 35
time.sleep(35)
try:
    fd = os.open(path, os.O_RDWR)
    try:
        result["second_pwrite"] = {"ok": True, "written": os.pwrite(fd, b"Z", 1)}
    finally:
        os.close(fd)
except OSError as e:
    result["second_pwrite"] = {"ok": False, "errno": e.errno, "errno_name": errno.errorcode.get(e.errno)}
result["stat"] = {"size": path.stat().st_size}
Path("min-expiry-repro.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
print(json.dumps(result, sort_keys=True))

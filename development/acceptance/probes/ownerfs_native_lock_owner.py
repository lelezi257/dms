#!/usr/bin/env python3
"""Kernel design counterexample, not OwnerFs lock implementation acceptance."""
import ctypes
import errno
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys


class Flock(ctypes.Structure):
    _fields_ = [("kind", ctypes.c_short), ("whence", ctypes.c_short),
                ("start", ctypes.c_long), ("length", ctypes.c_long), ("pid", ctypes.c_int)]


libc = ctypes.CDLL(None, use_errno=True)
libc.fcntl.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
libc.fcntl.restype = ctypes.c_int


def lock(fd, command, kind, supplied_pid=0):
    value = Flock(kind, os.SEEK_SET, 0, 1, supplied_pid)
    ctypes.set_errno(0)
    result = libc.fcntl(fd, command, ctypes.byref(value))
    return result, ctypes.get_errno(), value.pid


if len(sys.argv) == 5 and sys.argv[1] == "--child":
    descriptor, supplied_pid = map(int, sys.argv[2:4])
    command = fcntl.F_OFD_SETLK if sys.argv[4] == "ofd" else fcntl.F_SETLK
    queried = lock(descriptor, fcntl.F_GETLK, fcntl.F_RDLCK)
    tried = lock(descriptor, command, fcntl.F_RDLCK, supplied_pid)
    print(json.dumps({"helper_pid": os.getpid(), "supplied_pid": supplied_pid,
                      "getlk": queried, "setlk": tried, "kind": sys.argv[4]}))
    raise SystemExit(0)

if len(sys.argv) != 2:
    raise SystemExit("usage: ownerfs_native_lock_owner.py FRESH_EXT4_EVIDENCE_DIRECTORY")
assert ctypes.sizeof(ctypes.c_long) == 8, "probe currently requires Linux LP64"
evidence = Path(sys.argv[1])
assert evidence.is_absolute() and not evidence.exists()
assert subprocess.check_output(["findmnt", "-n", "-o", "FSTYPE", "-T", str(evidence.parent)], text=True).strip() == "ext4"
evidence.mkdir(mode=0o700)
data = evidence / "same-backing"
data.write_bytes(b"data retained")
fd = os.open(data, os.O_RDWR | os.O_CLOEXEC)
other = os.open(data, os.O_RDWR | os.O_CLOEXEC)


def helper(mode, supplied_pid=0):
    result = subprocess.run([sys.executable, str(Path(__file__).resolve()), "--child",
                             str(fd), str(supplied_pid), mode], pass_fds=(fd,),
                            capture_output=True, text=True, timeout=10, check=True)
    return json.loads(result.stdout)


try:
    assert lock(fd, fcntl.F_SETLK, fcntl.F_WRLCK)[:2] == (0, 0)
    proxy = helper("posix", os.getpid())
    assert proxy["getlk"][2] == os.getpid()
    assert proxy["setlk"][0] == -1 and proxy["setlk"][1] in (errno.EACCES, errno.EAGAIN)
    ofd = helper("ofd")
    assert ofd["setlk"][0] == -1 and ofd["setlk"][1] in (errno.EACCES, errno.EAGAIN)
    same_process = lock(other, fcntl.F_SETLK, fcntl.F_RDLCK)
    assert same_process[:2] == (0, 0)
    reader = helper("posix")
    assert reader["setlk"][:2] == [0, 0]
    identity = os.fstat(fd)
    result = {"outcome": "observed_design_rejection", "kernel": os.uname().release,
              "parent_pid": os.getpid(), "file": {"dev": identity.st_dev, "ino": identity.st_ino},
              "proxy_with_claimed_parent_pid": proxy, "ofd_proxy": ofd,
              "same_process_posix_downgrade": same_process, "reader_after_downgrade": reader,
              "limitation": "A helper's POSIX/OFD lock owner is not the native caller's process owner, even with an inherited same-file fd or supplied l_pid. This rejects that simple proxy design; it does not prove every possible design impossible."}
    (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    assert data.read_bytes() == b"data retained"
    print(json.dumps(result, indent=2))
finally:
    os.close(other)
    os.close(fd)

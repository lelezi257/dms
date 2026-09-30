# Quickstart

Run commands from the clone root on Linux:

```sh
cd <clone-root>
cargo build --locked --workspace --bins --examples
```

## DFS R=1 FUSE Smoke

```sh
cargo build --locked -p afs --no-default-features --features dfs --bins
sudo python3 scripts/dfs/r1_e2e.py   --bin-dir target/debug   --work-dir /tmp/afs-dfs-r1
```

The script starts `afs-meta`, starts a DFS-only `afs-node`, mounts FUSE, writes a file, syncs it, closes it, reopens it and reads it back.

## Manual DFS Run

Terminal 1:

```sh
./target/debug/afs-meta   --fs dfs   --meta-store local-file   --data-dir /tmp/afs-meta   --grpc-listen 127.0.0.1:7400   --rest-listen 127.0.0.1:7401
```

Terminal 2:

```sh
mkdir -p /tmp/afs-dfs/mnt
./target/debug/afs-node   --fs dfs   --meta-endpoint http://127.0.0.1:7400   --dfs-mount /tmp/afs-dfs/mnt   --data-dir /tmp/afs-dfs/node
```

Terminal 3:

```sh
python3 - <<'PY'
from pathlib import Path
path = Path('/tmp/afs-dfs/mnt/hello.txt')
with path.open('wb') as f:
    f.write(b'hello')
    f.flush()
    import os
    os.fsync(f.fileno())
with path.open('rb') as f:
    print(f.read().decode())
PY
```

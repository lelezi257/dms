# Quickstart

## Release Package Smoke

The first-stage deployable path uses a release package that already contains Linux binaries. The target guest does not need Cargo, Git or a Rust toolchain.

Package creation runs on the Linux build VM after `afs-meta` and `afs-node` have been built:

```sh
scripts/deploy/build-package.sh \
  --bin-dir target/release \
  --output /tmp/afs-dist \
  --source-commit "$(git rev-parse HEAD)" \
  --features "ownerfs,dfs,rdma"
```

On a target Linux guest:

```sh
tar -xf /tmp/afs-dist/afs-*.tar.gz -C /tmp
sudo /tmp/afs-*/install.sh --start dfs
sudo /opt/afs/bin/afs-processctl status all
sudo /opt/afs/bin/dep02-smoke.sh --mount /mnt/afs/dfs
```

The package installs program files under `/opt/afs`, creates default config under `/etc/afs`, keeps persistent state under `/var/lib/afs`, uses `/mnt/afs` as the default mount root, and writes logs under `/var/log/afs`. Existing config and data are preserved by default. Test installs can pass `--prefix`, `--config-dir`, `--state-dir`, `--run-dir`, `--log-dir` and `--mount-root` to stay fully under an isolated directory. The generated package manifest records source commit, feature set, Rust toolchain, binary checksums and runtime dynamic library output. `dep02-smoke.sh` requires the target path itself to be an AFS FUSE mount. On DFS it writes one root-level file; on OwnerFs it first creates one workspace directory and writes inside that workspace. Cleanup is best-effort and reported explicitly because early slices may not yet implement unlink or rmdir.

This package flow is the entry point for DEP install cases. A successful package install or a process that answers `/health` is not by itself an acceptance PASS until the Linux runner verifies exact AFS mount readiness and file read/write/close/reopen behavior. Full DEP still requires the complete acceptance cases; this minimal smoke does not replace mkdir/unlink/namespace coverage.

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

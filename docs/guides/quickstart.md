# Quickstart

This quickstart is for the usable trial path described in the
[three-stage goal table](../../development/trial-release-goals.md). The
historical `g1.5` package completed the G1 scope for its stated version and
environment. A newer source checkpoint still needs its own Linux build and
targeted G2 reruns before it can claim the same or broader status; see the
[current source checkpoint](../../development/current-checkpoint.md).

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
sudo /tmp/afs-*/install.sh
sudo /opt/afs/bin/afs-trial-config single --backend local-file --force
sudo /opt/afs/bin/afs-processctl start all
sudo /opt/afs/bin/afs-processctl status all
sudo /opt/afs/bin/dep02-smoke.sh --mount /mnt/afs/dfs
sudo /opt/afs/bin/dep02-smoke.sh --mount /mnt/afs/ownerfs
sudo /opt/afs/bin/afs-selfcheck --mount /mnt/afs/dfs --workspace g1-dfs --output /tmp/afs-selfcheck-dfs --force
sudo /opt/afs/bin/afs-selfcheck --mount /mnt/afs/ownerfs --workspace g1-owner --output /tmp/afs-selfcheck-owner --force
```

The package installs program files under `/opt/afs`, creates default config under `/etc/afs`, keeps persistent state under `/var/lib/afs`, uses `/mnt/afs` as the default mount root, and writes logs under `/var/log/afs`. Existing config and data are preserved by default. Test installs can pass `--prefix`, `--config-dir`, `--state-dir`, `--run-dir`, `--log-dir` and `--mount-root` to stay fully under an isolated directory. Use `afs-trial-config single --backend memory` for a disposable demo and `--backend local-file` for persistent restart trials. `local-file` is the required G1 recovery backend; etcd and Redis are later lanes. For a Meta plus two data node R2 layout, see [Trial Package Guide](trial.md). The generated package manifest records source commit, feature set, Rust toolchain, binary checksums and runtime dynamic library output. `dep02-smoke.sh` requires the target path itself to be an AFS FUSE mount. On DFS it writes one root-level file; on OwnerFs it first creates one workspace directory and writes inside that workspace. `afs-selfcheck` is the stronger colleague-trial check: it validates the exact mount with `findmnt`, wraps the Python probe in GNU `timeout`, and by default writes and verifies a 64 MiB stream plus small file, chmod, lock and mmap operations. Cleanup is best-effort and reported explicitly because early slices may not yet implement unlink or rmdir.

This package flow is the entry point for DEP install cases. A successful package install or a process that answers `/health` is not by itself an acceptance PASS until the Linux runner verifies exact AFS mount readiness and file read/write/close/reopen behavior. G2 uses the standard suite fallback (`pjdfstest`, fixed LTP subset and short FSx) plus the core OwnerFs/DFS scenarios from the goal table. Do not wait for every long-running or large-capacity G3 case before accepting a smaller completed item.

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

# Trial Package Guide

The main25a8061 default-OFF package has a fresh compiler-free single-node
local-file installation/basic64MiB/ordered Meta recovery regression for both
OwnerFs and DFS. [Exact scope and evidence](../../development/evidence/20261007-installed-off/README.md).
It excludes experimental container helper/control tools and does not qualify
container ON, full POSIX or performance. Historical g1.5 acceptance remains intact.

This guide is for the first colleague-trial package. It uses Linux binaries
already present in the release archive. Trial machines do not need Cargo, Git or
network access to build dependencies.

The current scope is anchored by the [three-stage goal table](../../development/trial-release-goals.md).
G1 is the usable colleague-trial slice: OwnerFs and DFS can be installed and
tested, `memory` is allowed only as a disposable demo, and `local-file` Meta
provides the restart-recovery lane. The current source checkpoint is recorded in
[development/current-checkpoint.md](../../development/current-checkpoint.md).

## Supported Trial Shapes

| Shape | Meta backend | DFS policy | What it is for |
| --- | --- | --- | --- |
| Single node | `memory` | R1 | Fast disposable OwnerFs/DFS demo |
| Single node | `local-file` | R1 | Local persistent smoke and restart checks |
| Meta + two data nodes | `memory` | R2, sync 2 | Disposable cross-node routing and replica demo |
| Meta + two data nodes | `local-file` | R2, sync 2 | Persistent trial topology before G2 performance work |

`memory` is intentionally non-durable. If `afs-meta` exits or is killed, the
namespace and idempotency state are gone. Use `local-file` for any restart or
recovery trial.

## Single Node

Install the package, generate one-host config, start both processes and run both
mount smokes:

```sh
tar -xf afs-*.tar.gz -C /tmp
sudo /tmp/afs-*/install.sh
sudo /opt/afs/bin/afs-trial-config single --backend local-file --force
sudo /opt/afs/bin/afs-processctl start all
sudo /opt/afs/bin/dep02-smoke.sh --mount /mnt/afs/dfs --name dfs-local
sudo /opt/afs/bin/dep02-smoke.sh --mount /mnt/afs/ownerfs --name owner-local
sudo /opt/afs/bin/afs-selfcheck --mount /mnt/afs/dfs --workspace g1-dfs --output /tmp/afs-selfcheck-dfs --force
sudo /opt/afs/bin/afs-selfcheck --mount /mnt/afs/ownerfs --workspace g1-owner --output /tmp/afs-selfcheck-owner --force
sudo /opt/afs/bin/afs-processctl status all
```

For a disposable run, replace `--backend local-file` with `--backend memory`.
The generated memory Node config sets `allow_volatile_meta = true`; local-file
sets it to `false`. Do not use `--no-readiness` to bypass the persistence
readiness check.

`afs-selfcheck` is the package-level colleague-trial probe. By default it writes
and verifies one 64 MiB streaming file plus small create/open/read/write,
append, truncate, rename, unlink, chmod, fcntl-lock and mmap checks. It validates
that `--mount` is the exact AFS FUSE mount using `findmnt`, and it wraps the
Python probe in GNU `timeout` with a hard wall-clock limit of `--deadline + 30`
seconds. Increase `--deadline` for slow machines; the outer timeout follows it.

To test as a non-root colleague user, have an administrator create and chown one
workspace directory inside each mount, then run the selfcheck as that user. Do
not chmod the whole mount root:

```sh
sudo install -d -o "$USER" -g "$(id -gn)" /mnt/afs/dfs/g1-"$USER" /mnt/afs/ownerfs/g1-"$USER"
/opt/afs/bin/afs-selfcheck --mount /mnt/afs/dfs --workspace g1-"$USER" --output "$HOME/afs-selfcheck-dfs" --force
/opt/afs/bin/afs-selfcheck --mount /mnt/afs/ownerfs --workspace g1-"$USER" --output "$HOME/afs-selfcheck-owner" --force
```

## Two Data Nodes

On an admin machine, generate the bundle. Replace the addresses with the actual
Meta, A and B trial addresses:

```sh
/opt/afs/bin/afs-trial-config cluster \
  --backend local-file \
  --meta-host 192.168.109.11 \
  --node node-a=192.168.109.12 \
  --node node-b=192.168.109.13 \
  --output /tmp/afs-trial-r2 \
  --force
```

Copy `/tmp/afs-trial-r2/meta/etc` to the Meta host's config directory. Copy
`/tmp/afs-trial-r2/node-a/etc` and `/tmp/afs-trial-r2/node-b/etc` to the
corresponding data nodes' config directories. Keep file modes on `*-key.pem`
private.

Start Meta first, then each Node:

```sh
sudo /opt/afs/bin/afs-processctl start meta
sudo /opt/afs/bin/afs-processctl start node
sudo /opt/afs/bin/dep02-smoke.sh --mount /mnt/afs/dfs --name dfs-r2
sudo /opt/afs/bin/dep02-smoke.sh --mount /mnt/afs/ownerfs --name owner-r2
sudo /opt/afs/bin/afs-selfcheck --mount /mnt/afs/dfs --workspace g1-r2-dfs --output /tmp/afs-selfcheck-r2-dfs --force
sudo /opt/afs/bin/afs-selfcheck --mount /mnt/afs/ownerfs --workspace g1-r2-owner --output /tmp/afs-selfcheck-r2-owner --force
```

The generated Node configs listen on `0.0.0.0` and advertise their configured
node IPs. The controller still validates exact local FUSE mounts before it
reports the Node ready.

## Operations

Use the managed controller for lifecycle:

```sh
sudo /opt/afs/bin/afs-processctl status all
sudo /opt/afs/bin/afs-processctl restart node
sudo /opt/afs/bin/afs-processctl stop all
```

`stop` succeeds only when the process supervisor records the matching exit code.
Data, config and logs are preserved. `uninstall` removes program files but keeps
config, state, logs and lifecycle evidence.

## Current Limits

- The trial package is Linux ARM64 when built from the current ARM64 candidate.
- The qualified trial environment is Ubuntu 24.04 ARM64 with Linux 6.8 and
  guest ext4 storage. Remote OwnerFs shared mmap requires the kernel to
  advertise `FUSE_DIRECT_IO_ALLOW_MMAP`; the mount negotiates it while keeping
  ordinary remote reads and writes in direct-I/O mode. Older kernels are not
  qualified by this trial and may return `ENODEV` for remote shared mmap.
- RDMA is not the default trial path; generated configs use gRPC data mode.
- Memory trials require a binary that supports `allow_volatile_meta`; older
  v208 binaries report Node health as not ready against volatile Meta.
- Redis is intentionally left for a later TODO. etcd remains supported by code
  and prior evidence, but it is below `local-file` in the current trial order
  and its memory/resource work is a later topic.
- OwnerFs native bind mount is not enabled by this trial guide. Its function
  and performance gates are separate G2 items, default off, and a public
  production enable switch is not qualified in this checkpoint.
- Full 69-case acceptance, full upstream POSIX matrices, 8-hour soak and G2
  performance ratios are not claimed by this trial smoke. G2 starts with
  standard fallback suites and small core performance cases, then grows to
  larger and longer scenarios.

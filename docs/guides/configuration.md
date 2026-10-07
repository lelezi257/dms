# Configuration

Configuration is loaded in this order:

```text
default values
  -> TOML file
  -> explicit CLI flags
```

Unknown TOML fields are errors. Example files live in `examples/meta.toml` and `examples/node.toml`.

## Filesystem Backends

| Flag | Meaning |
| --- | --- |
| `--fs ownerfs` | run OwnerFs only |
| `--fs dfs` | run DistributedFs only |
| `--fs all` | run both backends with separate mounts |

OwnerFs and DFS mounts must use different mount paths.

## Meta Backends

The current trial priority is:

| Backend | Current role |
| --- | --- |
| `memory` | Disposable demo. It loses namespace and idempotency state when `afs-meta` restarts and requires explicit volatile Meta allowance in Node config. |
| `local-file` | First persistent trial backend. This is the only Meta backend required for G1 restart recovery. |
| `etcd` | Deferred G3 topic for resource, recovery and fault behavior; current investigation may allow a larger memory budget instead of blocking G1/G2. |
| `redis` | Last persistence backend and acceptable TODO until the higher-priority trial and performance items are stable. |

Do not treat a backend as accepted only because the config parser exposes its
fields. Each backend lane needs its own acceptance result for the selected
candidate.

## OwnerFs Native Bind Mount

Default is `experimental_native_workspace = false`. Ordinary OwnerFs/FUSE
remains the accepted trial path. An administrator-only experimental single
managed container adapter now exists; its runtime lifecycle and performance
are **not qualified**. See the [source slice and remaining gates](../../development/native-workspace-slice.md).

For an admitted isolated Linux lab, ON requires OwnerFs plus its FUSE mount and
this TOML section (all values are explicit; no paths are created automatically):

```toml
experimental_native_workspace = true
[native_workspace]
control_dir = "/var/lib/afs-native/control"
runtime = "/usr/bin/runc"
rootfs = "/var/lib/afs-native/rootfs"
workload_uid = 501
workload_gid = 501
```

The control directory must be root-owned mode0700. Runtime/rootfs and their
ancestors must be root-owned, without symlinks or group/other writes. Rootfs is
a small read-only fixture with the exact built `/afs-workspace-probe`, its
resolved library dependencies, empty `proc`/`workspace` directories and any
selected workload binaries. Control/rootfs/runtime must be outside OwnerFs
mount and data directories; control and rootfs must be disjoint. Stale controller
artifacts or nonempty runtime state refuse startup pending reconciliation.
An explicit `--experimental-native-workspace false` overrides TOML. This is a
startup-only switch. It does not bypass ordinary grant/permission/error gates.

Bind has separate G2 function and performance exits. Function must prove the
managed lifecycle, ordinary OFF regression and required permissions, cache,
lock, mmap, revocation/drain and restart semantics. Performance then compares
the same candidate's OFF, ON and ext4. Neither configuration acceptance nor a
source build closes either exit.

## OwnerFs workspace bind host entry

The independent experimental host entry is default OFF. Enable it only for an
administrator-controlled, fixed-Home workspace that already exists:

```toml
fs = "ownerfs"
ownerfs_mount = "/ownerfs"
experimental_ownerfs_workspace_bind = true
[ownerfs_workspace_bind]
workspace = "agent1"
```

The Node binds the authorized physical Home directory to `/ownerfs/agent1` in
its startup mount namespace, without runc. One safe first-level component is
required. Missing settings, another backend/role or both experimental modes
refuse startup. `--experimental-ownerfs-workspace-bind false` overrides TOML.
The old container switch/settings retain their parsing and private namespace.

Stop managed users before shutdown or root/epoch changes. Native operations
use Linux DAC; checking the current local grant cannot instantly revoke existing
FDs/mmap or establish secondary-clone drain. Automatic workspace creation,
multiple roots, abnormal restart reconciliation and full ON qualification are
pending. [Scope and exact evidence](../../development/ownerfs-workspace-host-entry.md)
separate the implementation, component tests and actual Node acceptance.

## Storage Policy

DFS replication policy is a filesystem initialization setting. The accepted base design does not support per-inode dynamic policy revisions.

## External Spill

External spill is optional and disabled unless configured. A spill backend becomes a read source only after external write, verification and Meta commit.

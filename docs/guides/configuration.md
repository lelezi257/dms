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

The native bind mount proposal is not a public production configuration switch
in this checkpoint. The accepted product behavior remains the FUSE OwnerFs path
unless a later candidate explicitly qualifies a default-off switch.

Native bind has two independent G2 gates: function and performance. Function
must prove default-off behavior, managed mount lifecycle, fallback to FUSE,
cache/lock/mmap/permission boundaries, restart reconciliation and safe refusal
of unsupported configurations. Performance must then compare the same candidate
with OFF, ON and ext4. Passing one gate does not pass the other.

## Storage Policy

DFS replication policy is a filesystem initialization setting. The accepted base design does not support per-inode dynamic policy revisions.

## External Spill

External spill is optional and disabled unless configured. A spill backend becomes a read source only after external write, verification and Meta commit.

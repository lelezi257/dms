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

## Storage Policy

DFS replication policy is a filesystem initialization setting. The accepted base design does not support per-inode dynamic policy revisions.

## External Spill

External spill is optional and disabled unless configured. A spill backend becomes a read source only after external write, verification and Meta commit.

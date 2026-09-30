# Operations

## Process Deployment

Release packages provide three deployment entry points:

| Command | Purpose |
| --- | --- |
| `scripts/deploy/build-package.sh` | package existing Linux release binaries and deploy tools; it does not compile |
| `install.sh` | install binaries, process controller, dependency notes and default configs while preserving existing state |
| `/opt/afs/bin/afs-processctl` | start, stop, restart, status and uninstall managed AFS services |
| `/opt/afs/bin/dep02-smoke.sh` | verify exact AFS FUSE mount plus minimal file create/write/fsync/close/reopen behavior; DFS writes at mount root, OwnerFs writes inside a newly created workspace |

Managed services are `meta`, `dfs` and `ownerfs`. `dfs` and `ownerfs` run as independent `afs-node` services with separate config files, data directories and mounts. `all` expands to all three services.

The process controller uses PID files, `/proc/<pid>/exe`, configured ports and a per-service start lock to reject duplicate starts and avoid PID reuse mistakes. Start waits for the service REST `/health` endpoint and, for Node services, verifies that the configured path itself is an AFS FUSE mount (`afs-dfs` or `afs-ownerfs`). A parent ext4 mount such as `/tmp` is not readiness. If readiness fails, the newly started process is terminated and its PID file is removed. It removes stale PID files only during explicit start. `uninstall` stops managed processes and removes runtime PID files, but program files, config and persistent data stay in place unless the operator deletes them separately.

Default paths:

| Path | Meaning |
| --- | --- |
| `/opt/afs` | installed binaries and package manifest |
| `/etc/afs` | `meta.toml`, `node-dfs.toml`, `node-ownerfs.toml` |
| `/var/lib/afs` | persistent Meta and Node data |
| `/run/afs` | PID files |
| `/var/log/afs` | process logs |
| `/mnt/afs` | default mount root used in generated configs; override with `install.sh --mount-root DIR` for isolated installs |

Backend and TLS options stay in TOML and are passed directly to `afs-meta` or `afs-node`. The templates keep `local-file`, `etcd` and Redis backend fields visible; a backend lane is usable only when the installed binary and the selected backend pass the corresponding acceptance cases.

## Mounts

Run OwnerFs and DFS as separate mounts. Each mount has its own FUSE session, inode table, handle table and cache policy.

## Meta

The first-stage deployment runs one Meta process per filesystem. Stop and confirm termination of the old process before restarting it; do not run independent Meta instances against the same filesystem backend. The installer checks its own deployment's PID and port use. Meta election, automatic failover and fencing between instances require the separate [high-availability TODO](../acceptance.md#10-第一阶段验收后-todo).

The store interface supports alternate persistence backends. An etcd cluster's own election does not elect an AFS Meta leader.

## Workspace Location

Management tools query OwnerFs Home placement through `GET /v1/roots/{root_id}` on Meta. Node identity, root epoch and catalog revision describe placement; Home availability must be reported separately. Scheduling a workload on Home enables local access, while another node forwards operations to Home. This management path does not proxy file bytes or automatically migrate the workspace.

## Node Local Storage

Local chunk directories contain staged and finalized objects. Staged objects are not readable. Startup reconciliation must verify local records before serving them.

## Spill

External spill is outside the compute-cluster storage pool. Operators should monitor external write failures, orphan temporary objects and recall latency separately from local disk pressure.

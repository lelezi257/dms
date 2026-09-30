# Operations

## Process Deployment

Release packages provide these deployment entry points:

| Command | Purpose |
| --- | --- |
| `scripts/deploy/build-package.sh` | package existing Linux release binaries and deploy tools; it does not compile |
| `install.sh` | install binaries, process controller, dependency notes and default configs while preserving existing state |
| `/opt/afs/bin/afs-processctl` | start, stop, restart, status and uninstall managed AFS services |
| `/opt/afs/bin/dep02-smoke.sh` | verify exact AFS FUSE mount plus minimal file create/write/fsync/close/reopen behavior; DFS writes at mount root, OwnerFs writes inside a newly created workspace |

Managed processes are `meta` and `node`. OwnerFs and DFS are modules of the same `afs-node` process, configured with separate FUSE mounts. The `dfs` and `ownerfs` controller names are aliases for that Node; they select the requested mount readiness checks. `all` controls Meta and Node.

The process controller binds the real product PID to its executable, config path, Linux boot ID, start ticks and a unique launch directory. `node`, `dfs` and `ownerfs` control the same Node process. Their lifecycle commands share one lock; `restart` holds it through stop and start. `all` controls Meta and Node. These locks prevent concurrent controller commands; they do not provide Meta high availability.

Start waits for REST `/health` and verifies each requested mount is an exact AFS FUSE mount (`afs-dfs` or `afs-ownerfs`). A parent ext4 mount such as `/tmp` is not readiness. Before acknowledging readiness it rechecks the original process identity. Failed readiness cleans up that exact newly started process; incomplete cleanup retains its identity. An interrupted startup with missing PID publication retains launch intent so `stop` can recover the original child or report an unknown result.

A small supervisor owns the product child's `wait` status and atomically records it in that launch directory. `stop` succeeds only for a matching exit record with code `0` (or a service that was never started). Codes such as `1`, `124` or `137` are returned as failures. Missing or mismatched records produce a nonzero unknown result. PID disappearance and a successful signal alone do not prove successful shutdown.

Completed PID, identity and exit records remain available for repeated `stop` and `status`. JSON status preserves its service, state, pid, config and log fields and adds `exit_code`; stopped failures have state `failed`, unavailable results have state `exit-unknown`. An explicit new start establishes a new launch and retires the previous completed records. Unknown launch directories remain for diagnosis. `restart` and `uninstall` stop on a failed or unknown termination result. Successful uninstall removes program files and preserves config, data, logs and termination evidence.

Default paths:

| Path | Meaning |
| --- | --- |
| `/opt/afs` | installed binaries and package manifest |
| `/etc/afs` | `meta.toml`, `node.toml` |
| `/var/lib/afs` | persistent Meta and Node data |
| `/run/afs` | PID, identity, launch and exit records |
| `/var/log/afs` | process logs |
| `/mnt/afs` | default mount root used in generated configs; override with `install.sh --mount-root DIR` for isolated installs |

Backend and TLS options stay in TOML and are passed directly to `afs-meta` or `afs-node`. The templates keep `local-file`, `etcd` and Redis backend fields visible; a backend lane is usable only when the installed binary and the selected backend pass the corresponding acceptance cases.

## Mounts

Run OwnerFs and DFS as separate mounts. Each mount has its own FUSE session, inode table, handle table and cache policy.

## Node Stop

Stop prevents new FUSE admission, waits for accepted callbacks, closes lock sessions and drains dirty DFS data before releasing process resources. An application fd cannot remain usable after its Node exits. Applications must complete their required sync barriers before requesting shutdown; a failed shutdown does not acknowledge outstanding writes.

The production Node registers a signal observer on a separate OS thread before starting the business executor. The first observed SIGTERM/SIGINT, or service failure, arms one 15-second shutdown deadline; later triggers do not extend it. The observer has its own I/O reactor so a blocked business executor cannot delay arming. The deadline covers service teardown, FUSE join, dirty drain, blocking runtime workers and observability cleanup. It is disarmed only after these resources finish teardown. A signal arriving before service listeners register may result in bounded failed startup rather than graceful drain.

A completed shutdown error returns nonzero immediately. Expiry exits with code `124` and is a forced failure, not a successful drain or cancellation of physical I/O. `services.stopped` marks service-task completion only; it does not establish that Node data and process cleanup have finished. Preserve the exit status and error records when diagnosing a stop. The isolated Linux paused-Meta proof returns error `1` approximately 5.03 seconds after direct SIGTERM; a separate blocked-executor regression verifies the forced `124` branch. These checks do not qualify every lifecycle fault or a universal signal-to-exit SLA.

The [implementation status](../status.md) records which controller and lifecycle matrices are qualified. A controller observing that a PID disappeared cannot infer that the process exited cleanly.

## Meta

The first-stage deployment runs one Meta process per filesystem. Stop and confirm termination of the old process before restarting it; do not run independent Meta instances against the same filesystem backend. The installer checks its own deployment's PID and port use. Meta election, automatic failover and fencing between instances require the separate [high-availability TODO](../acceptance.md#10-第一阶段验收后-todo).

The store interface supports alternate persistence backends. An etcd cluster's own election does not elect an AFS Meta leader.

## Workspace Location

Management tools query OwnerFs Home placement through `GET /v1/roots/{root_id}` on Meta. Node identity, root epoch and catalog revision describe placement; Home availability must be reported separately. Scheduling a workload on Home enables local access, while another node forwards operations to Home. This management path does not proxy file bytes or automatically migrate the workspace.

## Node Local Storage

Local chunk directories contain staged and finalized objects. Staged objects are not readable. Startup reconciliation must verify local records before serving them.

## Spill

External spill is outside the compute-cluster storage pool. Operators should monitor external write failures, orphan temporary objects and recall latency separately from local disk pressure.

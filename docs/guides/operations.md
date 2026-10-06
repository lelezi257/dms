# Operations

## Process Deployment

Release packages provide these deployment entry points:

| Command | Purpose |
| --- | --- |
| `scripts/deploy/build-package.sh` | package existing Linux release binaries and deploy tools; it does not compile |
| `install.sh` | install binaries, process controller, dependency notes and default configs while preserving existing state |
| `/opt/afs/bin/afs-trial-config` | generate single-node memory/local-file configs or a Meta + two data node R2 config bundle |
| `/opt/afs/bin/afs-processctl` | start, stop, restart, status and uninstall managed AFS services |
| `/opt/afs/bin/dep02-smoke.sh` | verify exact AFS FUSE mount plus minimal file create/write/fsync/close/reopen behavior; DFS writes at mount root, OwnerFs writes inside a newly created workspace |
| `/opt/afs/bin/afs-selfcheck` | run the stronger package selfcheck against an exact AFS FUSE mount; default workload is 64 MiB plus small file, chmod, fcntl-lock and mmap checks, wrapped in GNU `timeout` |

Managed processes are `meta` and `node`. OwnerFs and DFS are modules of the same `afs-node` process, configured with separate FUSE mounts. The `dfs` and `ownerfs` controller names are aliases for that Node; they select the requested mount readiness checks. `all` controls Meta and Node.

The process controller binds the real product PID to its executable, config path, Linux boot ID, start ticks and a unique launch directory. `node`, `dfs` and `ownerfs` control the same Node process. Their lifecycle commands share one lock; `restart` holds it through stop and start. `all` controls Meta and Node. These locks prevent concurrent controller commands; they do not provide Meta high availability.

Start waits for REST `/health` and verifies each requested mount is an exact AFS FUSE mount (`afs-dfs` or `afs-ownerfs`). A parent ext4 mount such as `/tmp` is not readiness. Before acknowledging readiness it rechecks the original process identity. Failed readiness cleans up that exact newly started process; incomplete cleanup retains its identity. An interrupted startup with missing PID publication retains launch intent so `stop` can recover the original child or report an unknown result.

A killed Node can leave disconnected FUSE mounts behind. Before creating mount directories, a new start checks both configured mounts, including when no PID record remains. Automatic recovery requires an exact configured AFS FUSE target/source and an explicit `ENOTCONN` result from a bounded inspection. Responsive, foreign or unconfirmed mounts are refused. Detach commands are bounded; failed recovery preserves existing identity evidence. Starting a new process does not change a prior failed exit into a successful shutdown or acknowledge unsynced data.

A small supervisor owns the product child's `wait` status and atomically records it in that launch directory. `stop` succeeds only for a matching exit record with code `0` (or a service that was never started). Codes such as `1`, `124` or `137` are returned as failures. Missing or mismatched records produce a nonzero unknown result. PID disappearance and a successful signal alone do not prove successful shutdown.

Completed PID, identity and exit records remain available for repeated `stop` and `status`. JSON status preserves its service, state, pid, config and log fields and adds `exit_code`; stopped failures have state `failed`, unavailable results have state `exit-unknown`. An explicit new start establishes a new launch and retires the previous completed records. Unknown launch directories remain for diagnosis. `restart` and `uninstall` stop on a failed or unknown termination result. Successful uninstall removes program files and preserves config, data, logs and termination evidence. Program-file removal includes `afs-meta`, `afs-node`, `afs-processctl`, `afs-trial-config`, `afs-selfcheck`, `dep02-smoke.sh`, the package manifest, dependency notes and installed guide copy.

Default paths:

| Path | Meaning |
| --- | --- |
| `/opt/afs` | installed binaries and package manifest |
| `/etc/afs` | `meta.toml`, `node.toml` |
| `/var/lib/afs` | persistent Meta and Node data |
| `/run/afs` | PID, identity, launch and exit records |
| `/var/log/afs` | process logs |
| `/mnt/afs` | default mount root used in generated configs; override with `install.sh --mount-root DIR` for isolated installs |

Backend and TLS options stay in TOML and are passed directly to `afs-meta` or `afs-node`. The templates keep `memory`, `local-file`, `etcd` and Redis backend fields visible; a backend lane is usable only when the installed binary and the selected backend pass the corresponding acceptance cases. `memory` is a disposable demo backend and loses namespace state on Meta restart; Node readiness requires explicit `allow_volatile_meta = true` for this lane. `local-file` is the persistent first trial lane and keeps volatile Meta disabled.

The current trial order is `memory` demo, then `local-file` persistent restart
recovery, then etcd as a later resource/reliability topic, then Redis as the
last backend lane. This order is operational as well as validation guidance:
do not block the usable package or OwnerFs/DFS core performance work on etcd or
Redis unless the selected task explicitly depends on that backend.

Listen addresses and mount paths are read from single-line configuration values during controller preflight and readiness. Literal strings with single quotes and basic strings with double quotes can have trailing comments; spaces, `#` and `=` inside a quoted value are preserved. A parse error or out-of-range listen port fails before creating a managed launch. The [implementation status](../status.md) records remaining configuration syntax limits.

## OwnerFs Peer Metrics

Node exposes OwnerFs peer metrics from its process-owned registry:

| Metric | Meaning |
| --- | --- |
| `afs_ownerfiles_rpc_duration_seconds` | Client method duration or Home handler duration; remote client read/write include data-plane negotiation, completion checks and transport close |
| `afs_ownerfiles_payload_bytes_total` | Successfully completed logical file bytes, labeled by `side=client\|server`, `direction=read\|write` and `plane=grpc\|rdma` |

Client read/write timings use `method=read` and `method=write`; server timings use
`method=OwnerFiles.Read` and `method=OwnerFiles.Write`. Cache-only prefetch reads
do not perform a Peer RPC and do not increment client RPC or payload metrics.
Payload counters use actual completed lengths, including short I/O; EOF adds
zero. Handshakes, control messages and invalid or failed completions add no
successful file bytes. No inode, path, handle or session identity is a metric
label.

These counters measure successful logical operations at each endpoint, not
physical wire traffic or RDMA completions. A server can finish before its reply
is lost, while a client cannot acknowledge that uncertain outcome; retries can
also create different endpoint totals. Correlate counters with errors, traces
and actual verbs completion evidence when diagnosing transport behavior.

## DFS Auto Selection

Configure `data_mode=auto` and `rdma_device` to prefer RDMA for DFS peer reads
and replica writes. With no local RDMA feature or device, startup records the
reason and selects gRPC. A configured device that fails initialization produces
an explicit startup error.

Peer fallback records a bounded transport reason. It is permitted only before
data dispatch when negotiation reports unsupported. A timeout or error after
dispatch keeps its original outcome; it does not start a second write on gRPC.
Use actual payload and verbs completion counters to confirm the selected path.

## RDMA Admission

Each server RDMA registry admits at most 64 owned endpoints or pending endpoint
allocations. Negotiation reserves a slot before opening native resources. A
closed, expired or poisoned session rejects new requests, but a worker that
already owns its endpoint keeps the slot until the last endpoint owner drops.
Close and TTL cleanup are not transfer drain barriers.

At capacity, negotiation returns `NODE_RDMA_CAPACITY`. Cleanup cannot make a
busy endpoint's slot reusable. Owner and DFS/diagnostic registries have separate
budgets; 64 is not a combined Node-wide limit. Native endpoint destruction runs
before its reservation is released. Exceptional provider teardown failures and
resource reclamation still require lifecycle qualification; admission accounting
does not establish that cancellation interrupts posted DMA.

## Mounts

Run OwnerFs and DFS as separate mounts. Each mount has its own FUSE session, inode table, handle table and cache policy.

OwnerFs native bind mount, if present in a development candidate, is a separate
default-off optimization lane. Operations must continue to document and support
the normal FUSE path as the default. A candidate may claim native-bind function
only after the managed lifecycle, fallback, cache, lock, mmap, permission and
restart-reconciliation checks pass; it may claim native-bind performance only
after OFF/ON/ext4 comparisons pass for the chosen workload.

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

## Experimental container workspace

For an admitted isolated Linux lab only, the Node-owned controller accepts
administrator commands through `scripts/ownerfs/native-workspace-control.py`:

```sh
sudo python3 scripts/ownerfs/native-workspace-control.py --socket /var/lib/afs-native/control/control.sock start WORKSPACE
sudo python3 scripts/ownerfs/native-workspace-control.py --socket /var/lib/afs-native/control/control.sock status
sudo python3 scripts/ownerfs/native-workspace-control.py --socket /var/lib/afs-native/control/control.sock exec -- /absolute/workload COMMAND_ARGUMENT
sudo python3 scripts/ownerfs/native-workspace-control.py --socket /var/lib/afs-native/control/control.sock stop
```

Create the workspace through the ordinary OwnerFs mount before `start`. The
client reports controller errors with a nonzero exit; partial/unknown cleanup
is not a successful stop. Same-ID identical workload requests replay their
recorded result; status is a fresh read and does not consume the bounded
operation ledger. Source and container identity come from Node, never a request
source path. This tool/helper is not part of the historical trial package.
[Configuration and unresolved qualification gates](configuration.md#ownerfs-native-bind-mount)
must be checked before running it. Do not erase Unknown/stale control or runtime
state to restart; preserve receipts and resolve ownership first.

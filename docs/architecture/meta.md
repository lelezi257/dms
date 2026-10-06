# Meta And Transactions

Meta is the authority for recoverable filesystem state. It does not proxy steady-state file bytes.

## Responsibilities

| Area | Examples |
| --- | --- |
| Namespace | dentries, inode identity and attributes |
| Write authority | write leases, owner epochs and fencing |
| Version authority | `FileVersion`, `LayoutRoot`, inode `head_version` CAS |
| Placement | placement snapshots, device epochs and failure domains |
| Copy catalog | durable replicas, verified cache, external committed copies and lifecycle state |
| Idempotence | exact operation result retention for retried commits |

## Commit Model

A Meta service validates a request against the committed state, prepares a candidate state and publishes it only after the persistence backend accepts it. If a backend write result is unknown, the exact request identity remains the recovery boundary.

The accepted base design allows Meta to keep whole committed snapshots and batch updates above the store interface. It does not require the underlying backend to expose native multi-record transactions in the first implementation. Record-level storage is a possible future implementation detail, not a semantic requirement.

## Statefulness

Meta service processes can be stateless with respect to local memory only when the persistent backend and fencing model hold the authoritative state. The filesystem is still stateful: namespace, file versions, copy records and idempotent results live in Meta's durable authority.

The [G1 trial](../../development/trial-release-goals.md) deploys one Meta process per filesystem and requires central local-file restart recovery; memory is a disposable demonstration. etcd and Redis implementation qualification is deferred to G3. Meta instance election and fencing between competing instances require a separate HA protocol. Inode owner lease and Node/Device epoch checks remain part of the ordinary file protocol.

The current local-file implementation uses checksummed recoverable state and an append log with bounded work/checkpointing. Snapshot/CAS validation and synchronization remain authority boundaries; startup recovery verifies retained state before serving it. Optimizing replay cost does not turn an unacknowledged backend write into success. Current capability and validation identities are recorded in [status](../status.md), not inferred from this target design.

### DFS Lease Identity And Renewal

A DFS write lease is identified by inode, owner node, owner process session and lease epoch. Its expiry can advance when the same owner opens a handle or renews authority. Different handles and the inode lock authority can therefore hold different expiry hints for the same epoch.

Meta validates the stored live identity before renewal, compares that stored lease atomically, and preserves a monotonically increasing expiry. Expiry contention retries with the same operation identity; exhausted contention reports a retryable error. An expired or reassigned stored lease cannot be renewed through an old handle. A repeated successful operation returns its recorded result.

Node distinguishes temporary renewal failure from confirmed fencing. Temporary failure retains existing locks and waiters while their authority is live. Confirmed fencing or expiry invalidates that authority; a delayed renewal response cannot restore an invalidated lock table. The same rules apply to local lock operations and Peer control requests.

## Snapshot Reads

Compound read operations can use `MetaReadView` to pin one committed state while resolving a file version, layout, inode and chunk sources. This gives a consistent read view above the store interface without requiring the backend itself to expose native multi-record read transactions.

## Compound Reads

A source query reads its file version, layout, placement, copies, node sessions and device state from one acknowledged `MetaReadView`. Advancing the live state during that query cannot mix revisions. This view is a Meta semantic guarantee: a backend that atomically persists the complete state can support it without native multi-record transactions.

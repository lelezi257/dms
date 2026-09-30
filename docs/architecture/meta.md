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

The [first-stage delivery](../acceptance.md) deploys one Meta process per filesystem. It requires durable restart and exact request replay, but does not implement Meta instance election or fencing between competing instances. High availability requires a separate leader and persistence protocol, listed in the acceptance TODO. Inode owner lease and Node/Device epoch checks remain part of the first-stage file protocol.

## Snapshot Reads

Compound read operations can use `MetaReadView` to pin one committed state while resolving a file version, layout, inode and chunk sources. This gives a consistent read view above the store interface without requiring the backend itself to expose native multi-record read transactions.

## Compound Reads

A source query reads its file version, layout, placement, copies, node sessions and device state from one acknowledged `MetaReadView`. Advancing the live state during that query cannot mix revisions. This view is a Meta semantic guarantee: a backend that atomically persists the complete state can support it without native multi-record transactions.

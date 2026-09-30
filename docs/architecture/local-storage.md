# Local Storage And COW

Local storage turns immutable chunk identities into bytes on a node disk.

![Local storage and COW](../images/cow.svg)

## Chunk Finalization

A chunk is readable only after finalization:

```text
staged bytes
  -> length and digest verification
  -> durable data barrier
  -> no-replace publish
  -> local catalog update
  -> ReplicaAck
```

Meta may record a durable copy only after the node can later find and verify it.

## Layout COW

File updates create new layout records and file versions. A small overwrite creates a new chunk for the overwritten range and reuses old chunks for unchanged ranges.

```text
old: [0, 4MiB) -> chunk-a
write 4KiB at 1MiB
new: [0,1MiB) -> chunk-a
     [1MiB,1MiB+4KiB) -> chunk-b
     [1MiB+4KiB,4MiB) -> chunk-a
```

This keeps old versions valid and lets the inode head move to a new version.

## Physical COW

Physical COW moves an existing chunk between local locations, packs or media without changing its `ChunkId`. It first writes and verifies the new location, then switches the local record. Reader pins protect old locations until in-flight reads finish.

Physical COW does not create a new `FileVersion`.

## Recovery

On restart, a node reconciles staged files, local catalog records and Meta copy records. Staged objects are not served. Catalog entries are useful only when they verify against the expected `ChunkObject`.

The persistent device ID and device epoch survive a process restart. After catalog recovery, Node registration advertises that device and its recovered catalog revision. Meta can derive fresh read authority for an existing durable copy without rewriting its original receipt. A changed device identity, older recovered catalog or invalid Node session cannot acquire that authority. This recovery uses the existing registration and read-grant interfaces; file bytes do not pass through Meta.

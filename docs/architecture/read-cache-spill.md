# Read, Cache And Spill

Fixed-version reads are the common path for ordinary committed reads, image lazy loading, checkpoint restore and dataset scans.

![Read path](../images/read-path.svg)

## Read Flow

1. Resolve the path and fix a committed `FileVersion` for the handle.
2. Convert file offsets to chunk ranges through that version's layout.
3. Select eligible sources for each chunk range.
4. Read ranges from local durable replicas, peer replicas, verified cache or external committed copies.
5. Verify identity and assemble the user response.

A reader never combines bytes from different file versions.

## Source Types

| Source | Role |
| --- | --- |
| Durable replica | Counts toward durability when Ready |
| Verified cache | Can serve reads after full chunk verification; can be evicted |
| Seed lease | Temporary advertisement for a Ready durable or cache copy |
| External committed copy | Optional spill or cold source after verified commit |

Partial range verification can protect transfer integrity. A node becomes a seed only after it has a complete chunk and verifies the full digest.

## Cache

Verified cache is useful when many consumers read the same fixed chunks. Cache does not silently become durability. Promotion to durable replica requires placement and Meta commitment.

## Spill

External spill is optional. It can provide cold capacity, archive or disaster recovery after write, verification and Meta commit.

Open question Q2: the minimum spill durability contract is not fixed. An external committed copy does not automatically reduce the configured local durable replica requirement unless the filesystem defines that stronger external durability contract explicitly.

## Authorization Boundary

Peer range reads require both peer authentication and a read authorization decision for the fixed file version, layout and chunk range. Authorization is tied to the committed version and chunk range being served, so a peer cannot use a stale grant to read a different version.

## Copy State

```text
Ready -> Deleting -> removed
  |
  v
Corrupt

LegacyStaging -> removed
```

- `Ready` durable copies can satisfy durability and can become read sources.
- `Ready` verified cache copies can serve reads and seed peers, but do not count as durable replicas.
- `Corrupt` copies fail identity verification and are removed from source selection.
- `Deleting` copies are no longer selected for new reads while in-flight reader pins drain.
- `LegacyStaging` is a decode-only compatibility state for old catalog entries; new code does not create or serve it.

## Seed And Eviction

A node can advertise a seed only for a complete `Ready` durable copy or a complete verified cache copy. Seed leases expire or are withdrawn before eviction. Eviction is legal only when Meta can still find enough durable or explicitly accepted external copies for the configured policy. Cache pressure cannot delete the only valid source for a committed chunk.

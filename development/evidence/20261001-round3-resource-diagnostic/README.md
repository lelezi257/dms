# Round3: short whole-system resource diagnostic

**Result:** healthy bounded diagnostic PASS; no performance ratio or formal acceptance qualified. Current round remains 3. Formal status remains 69 NOT_RUN / ENV PREPARING.

## Scope and identity

Frozen installed v80 binaries run in an isolated memory-Meta cohort on the locked ARM64 Linux ctl/A/B guests, guest ext4, mutual TLS, required RXE, separate OwnerFs/DFS mounts and desired2/sync1 replicas. C is not part of this short diagnostic. The [exact binary hashes](probes/eio-v80-r2-binaries.json), [A configuration](a/etc/node.toml), [B configuration](b/etc/node.toml) and [Meta configuration](ctl/etc/meta.toml) bind the result. Existing unrelated cohorts remain outside this run.

Three 8 MiB writes cover Owner Home-local, Owner remote-to-Home and DFS data/metadata sync and close. Four sequential reads check exact bytes and EOF; four eight-worker reads execute 2,048 fixed-seed 4 KiB/64 KiB range checks. Each operation captures stable PID/start-time/executable identity, CPU ticks, RSS/HWM, descriptors/threads, process IO, cgroup budgets, volume metadata, verbs resources and transport metrics before/after. The [driver](probes/round3-small-mainline.py) saves timing as an unqualified diagnostic, not a benchmark.

[Meta replication state](a/evidence/physical-replication.json) shows two 4 MiB chunks, each with two available Ready DurableReplica copies and Completed repair tasks. [B physical hashes](b/evidence/physical-copies.json) match A exactly. [B metrics](b/evidence/final-idle-metrics.txt) record 8 MiB RDMA replica receive and 8 MiB Owner RDMA write; gRPC replica payload is zero. DFS reads in this cohort use local verified copies; this run does not add a new peer-read RDMA claim.

All three processes stop with controller exit0. Cleanup verifies their PIDs absent and mounts gone. Post-stop verbs inventories show no user resources, retaining only kernel `ib_core` entries. [A cleanup](a/evidence/cleanup.json) retains more than 4 GiB available on its data volume. This is process-exit cleanup proof, not proof that pooled resources all disappear while a process remains alive.

## Resource observations and interpretation

| Observation | Measured result | Limit |
| --- | --- | --- |
| A Node RSS | 34,356 → 98,580 KiB | Short sample, no sustained leak/exhaustion verdict |
| B Node RSS | 34,304 → 104,640 KiB | Retained pools and allocator memory are not identified as leaks |
| Meta RSS | 20,400 → 23,308 KiB | Memory backend; serialized snapshot cost not measured |
| A sequential DFS read | 8 MiB returned; process `rchar` delta297,477,504 | Includes process overhead/background work; not physical disk IO |
| A concurrent DFS ranges | 17,764,352 bytes returned; `rchar` delta1,077,959,304 | Requires controlled attribution before optimization |
| A concurrent Owner ranges | Same requested bytes; `rchar` delta17,808,286 | Local path provides a useful diagnostic contrast, not a fair comparator baseline |

The sampled DFS reads have `read_bytes=0`: the underlying reads were served from cache. Current `PinnedChunkReader.read` and `open_verified` call `verified_range`, which scans/checks the whole chunk. This supports a read-amplification hypothesis, but does not attribute every sampled syscall byte to that function. Keep verification-before-exposure and corruption detection intact when evaluating an optimization. D07 tracks this work.

## Validation and evidence reuse

[Resource helper](probes/round3-resource-probe.py) reports unavailable observations explicitly; its top-level PASS means stable requested process identities only. Linux [13 helper regressions](build/helper-tests-r3.log) pass. Initial test-schema failures and a mutable-share syntax-read failure remain in r1/r2 logs; r3 runs immutable guest copies.

No product code, protocol or persistent format changes in this diagnostic. The [round2 closure](../20261001-round2-closure/README.md) complete Linux source gate is reused only after all143 compiler-input hashes match. Protected AGENTS.md and handoff hashes remain unchanged. [Packet audit](audit.json) and [artifact identities](artifact-hashes.json) bind the evidence.

## Remaining whole-system work

Qualify actual comparator mounts/configuration/durability and resource isolation before claiming performance targets. Stock MooseFS build evidence does not yet prove the required strong-durability write barrier; the historical patched 3FS one-node RXE run is not a fair three-node durable baseline. Build artifact inventory is not qualification. Persistent etcd/Redis parity/restart follows the core resource/performance development lane. Full installed fault axes, POSIX, installation matrix, 8 GiB, paired performance repetitions and soak remain release requirements.

Ordinary FUSE caching, short duration, async desired2/sync1, memory Meta and competing old cohorts prohibit cold-disk, isolated, R3-durability or comparative-performance conclusions from these timings. The short sample has no observed incorrect success, corruption, authorization/order violation or resource exhaustion; it cannot establish their absence under unrun workloads.

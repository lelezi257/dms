# Ordinary OwnerFs local small write: current data, formal comparison pending

2026-10-08 G2.10 independent slice. Product f03dc2b3 / compiler map 2b17 / ordinary 7bfc trial package unchanged. Existing Linux B ext4; local-file Meta, gRPC, workspace switches OFF. Client/Home and official MooseFS 4.59.2 master/chunk/client all on B, CREATE/KEEP one-copy policy. [Pre-run contract](contract.json), [independent Linux audit](independent-stored-audit.json), [raw commands/results/recipe index](raw-index.json).

| Classification | Evidence and scope |
| --- | --- |
| **Current functional slice PASS** | Twelve fresh 64MiB files, 1MiB blocks, C1, byte 97, one warmup + five alternating pairs. Every task: create/exclusive, 64 writes, successful fdatasync and close; parent-directory fsync outside timer, complete SHA/length/EOF/mode 0600 verification. Six local VALID Moose physical copies, Home identity verified. This is a bounded core write slice, not full POSIX or power-loss recovery. |
| **Current diagnostic data complete; formal NOT_QUALIFIED** | 320 measured operation intervals per target, all 640 retained and independently recomputed; throughput includes open/thread setup/writes/join/fdatasync/close. Per-operation latency covers pwrite + count validation, excludes the final file barrier; barrier_ns recorded separately. CLOCK_MONOTONIC, sorted[floor(N*p/100)], p95 frozen before run. No throughput-derived latency. Missing Moose strong durable-ACK and matched backing-cache qualification; neither performance ratio closes the formal item. |
| **Current tool guards PASS** | Prior unchanged Linux-tested C probe reused by identity. New recipe validator accepts one valid payload and rejects ten operation/barrier/cache/content/concurrency/timing/percentile/count mismatches in Linux. [Scoped guards](runner-guards.json); not product or full runner qualification. |
| **Current normal closure PASS** | Two AFS actual wait=0 and three Moose actual wait=0; seven child/supervisor PIDs plus runner gone; FUSE mounts/UDS closed, original process/mount inventory unchanged, frozen configs unchanged. [Independent guest check](independent-closure.json). No environment installation/reset/repair or old protected-service interruption. |
| **Historical unchanged** | e925 old ext4 performance FAIL retains original version/criterion/conclusion. G1 historical 8/8 and G2 headline counts unchanged. Formal target remains >=1.2× matched Moose throughput AND <=0.8× independently measured p95. |

| Diagnostic metric | OwnerFs | MooseFS |
| --- | ---: | ---: |
| Median throughput MiB/s | 909.993832 | 1170.143018 |
| Pooled p50 ns | 442423 | 207669 |
| Pooled p95 ns | 1184309 | 1374228 |
| Pooled p99 ns | 2079197 | 2451536 |

Diagnostic throughput ratio 0.777677; p95 ratio 0.861799. All five rates, raw intervals and file-barrier durations are retained. Before-create residency is an initialized field for an absent file, not an observed cold-cache proof; after-verification residency is measured. Backing-cache policy is unqualified.

Maximum sampled allocation 903024640B < pre-run 2GiB ceiling; minimum sampled free 24090857472B > 4GiB floor. Phase snapshots do not prove a continuous peak. Guest roots remain; text export is not full user-data restore. INFO logs total 14251B with 19 ERRO and 18 WARN, retained without suppression; [grouped messages](log-summary.json). Initial exporter refused a TLS key because its etc glob was too broad; original refusal/partial guest archive preserved. R2 exports only TOML under etc, and independently verifies every text member excludes private keys. This tooling correction did not rerun the product or change the VM.

Next: retain comparator qualification and performance tuning as later independent items; proceed to basic local deletion with correctness and comparison data, no new hard ratio. No package rebuild/republication, standard-suite rerun, third-party edit or source snapshot copy.

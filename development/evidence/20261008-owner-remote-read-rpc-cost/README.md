# G2.14 existing RPC cost diagnostic — 2026-10-08

**Diagnostic COMPLETE; limited content/bind/lifecycle PASS; no product optimization or final performance acceptance.** Retained production Node8e1216f4/Meta15648a87/probec87a5fd8, source main d79bf7c4 product mapf704143e. Reused the preceding stopped ctl/B/C local-file/bindON fixture with explicitly rechecked config/ELF/TLS/ports/mount/capacity; A and protected services untouched. No new Rust build, vendor change, TRACE, VM rebuild or environment repair. [Exact identity](identity.json).

One64MiB/C1 logical1MiB READ, raw existing Prometheus snapshots immediately before/after; no baseline/new-node performance pair. [Result](rpc-cost-result.json), [independent audit](audit.json):

| Existing method | RPC count | Payload | Sum seconds | Mean ms |
| --- | ---: | ---: | ---: | ---: |
| B/server OwnerFiles.Read | 128 | 67108864B | 0.008826002 | 0.068953 |
| C/client read | 128 | 67108864B | 0.160360264 | 1.252815 |

Home handler mean accounts for5.50% of matching client RPC mean. Difference1.183861ms includes transport/codec/scheduling and other client work; it is not pure wire time, and percentiles must not be subtracted. Existing histogram buckets are retained, no operation p95 inferred.128 RPC for64logical reads does not establish individual request lengths; unaligned-buffer/page splitting remains an inference, no probe-alignment change or throughput claim.

Decision: avoid further Home short-lock/copy micro-optimization without evidence; inspect the external request/transport path. Actual configured stream/connection windows were already2MiB, so no unsupported65KiB-window assumption. The following [Owner receive-frame experiment](../20261008-owner-remote-read-frames/README.md) separately records a real product pair.

Saved pre/post physical/remote full SHA/size/EOF and physical mincore64MiB hot, real ext4-source workspace bind identity, three actual diagnostic childwait0/PID disappearance and full prior mount/protected inventory checks PASS. [First prestart tool import failure](first-tool-failure.json) remains: caller omitted existing tools path; corrected import and resumed preflight before any service start, no missing dependency installation or product rerun.

[Archive](archive-index.json):289raw+27guest actual Linux recovery PASS,78647B, SHA92dcbd8c1caf11e654472a71a749ab895e9edf278f549e495718cc9fec7497d9. Audit generated afterward and indexed separately; immutable archive preserved. Raw metrics/commands/full logs/failures remain external, no tool/source copies or ELF in Git. [Provenance](provenance.json). G1 historical8/8/G2 original12/0/15 unchanged; no packaging or full POSIX/recovery/performance claim.

# DFS writer-node single-reader small observation

2026-10-07, independent G2.22 subitem. **Current small content/timing/lifecycle
recorded; qualified3FS parity and the complete G2.22 exit remain pending.**
G1 historical8/8 stays closed. [Plan and affected coverage](../../dfs-r3-local-read-small.md).

The current product remains [7e6e00a6](https://github.com/lelezi257/dms/commit/7e6e00a6e2d7fdf3c1d606ee743a016a419ec25d),
compiler map151a2c6d, published ordinary packagec3bb5a30,
Meta76a1e34c/Nodec47be268. [Exact candidate](expected-candidate.json),
[compiler inputs](compiler-inputs.json), [executed tool inputs](tool-inputs.json).
Only the maintained Python driver/guards changed; no Rust/vendor/C change or
product rebuild. Existing OFF standard scoped reuse and installation/recovery
evidence remain scoped to their original identities; no suite was repeated.

Four existing ARM64 Linux/ext4 VMs admitted exact tools/dependencies/config/TLS,
ports, RAM, capacity and protected inventory before start. [Admission](four-role-admission.json).
Local-file Meta/gRPC, both workspace switches OFF; desired/synchronous copies
and minimum node/configured domain identities all3, local-copy required.
These are three VM identities on one host, not three physical failure domains.

A seeded64MiB using64 distinct1MiB counter blocks, fdatasync/close and directory
fsync. This is an **unmeasured seed**, not a five-round write performance result.
[Writer](r2/writer-confirmed.json). Sixteen distinct4MiB chunks, three derived
ReadyDurable CopyRecords each, all48 actual physical byte/SHA copies independently
checked [before](r2/before-read-replica-proof.json) and
[after](r2/after-read-replica-proof.json) the selected reads. No chunk archive,
complete original ReplicaAck or crash durability claim.

A then performed one unmeasured warmup and five fresh-open C reads, with complete
SHA/size/EOF before and afterwards. [Results and raw stdout](r2/local-reader.json),
[onsite round files](r2/raw-read-rounds.json). Five measured MiB/s:
65.761196,65.624896,65.706844,65.560917,65.544123; median **65.624896MiB/s**.
C timing includes open/read/content-check/EOF/close, excluding driver SHA checks.
Cache residency and actual RPC/read location remain unobserved; being on the
writer node does not prove storage-local, hot or cold reads. No speedup against
the historical B/C run and no qualified3FS parity claim.

[Closure](r2/closure-summary.json) proves four actual service waits0, eight owned
child/supervisor PIDs gone, three normal FUSE/UDS closures, exact original full
mount inventories and eleven protected process identities unchanged.
[Final closure/budget](r2/final-closure.json) after export and verification remained596,570,112B<1GiB. Observed allocation [before](r2/before-read-budget.json)/[after](r2/after-read-budget.json)
stayed below the declared aggregate1GiB ceiling; every backing volume retained
its1GiB reserve. [Runtime status](r2/runtime-summary.json).

**Failure retained:** the first attempt started services but the writer refused
a missing results parent in prepare, before any C write or performance data.
[Original refusal](r1/writer.stdout), [runtime failure](r1/runtime-failure.json),
[four normal closures](r1/closure-summary.json). The orchestration omitted the
explicit test output directory. R2 created that root-owned parent and checked
the unchanged candidate/tools/config and protected identities before restart;
it executed one data run. No product/dependency/VM repair or score-based retry.
Host-only initial manifest-path and preflight-status assumption failures are
also retained in [r1](r1); they did not start data operations.

[Independent Linux observation verification](r2/linux-independent-verification.json) passed pinned identities, every C sample, independent median, both physical-copy proofs, budgets and actual waits. It covered76 completed R2 runtime/export command results; later verification/closure commands are separately retained in the command index.

Eleven affected Linux driver guards PASS, including rejecting missing/duplicate/
failed/mislabeled reads, corruption, false timers and post-read corruption without
a score. [Raw test output](linux-driver-guards.stderr). These are tool guards,
not full POSIX acceptance. No Rust checks were repeated for this Python-only item.

[Command index](commands-index.json) retains argv, exit codes and raw hashes;
exact inline commands/full raw receipts/observer scripts and durable metadata
stay in the hash-bound host archive, [external recovery paths](external-artifacts.json).
All full service logs are retained in [logs](logs), including failures/warnings;
[accounting](log-accounting.json) makes no zero-error-log claim. No historical
evidence link, suite conclusion or candidate was replaced.

Next G2.23 small single-writer data observation. Qualified3FS baseline, ordinary
OwnerFs failed targets, full bind mixed semantics, larger/long/complex reliability
and etcd/Redis keep their separate states and priorities. G2.27 remains open;
this small observation does not complete the overall goal.

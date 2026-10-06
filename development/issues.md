# Active issue ledger

This ledger tracks active risks against [the three-stage checklist](trial-release-goals.md). Historical defects with their own evidence remain in Git history and status pages; this file keeps only issues that shape the next execution choices.

| ID | Issue | Severity | Blocks | Current handling | Next check |
| --- | --- | --- | --- | --- | --- |
| CHECKPOINT-01 | Current source has not yet run OwnerFs current-candidate standard fallback suites | High | G2.04, Owner side of G2.06-G2.07, G2.27 | Treat old pjdfstest/LTP/FSx evidence as historical only | Run OwnerFs pjdfstest, fixed LTP subset and short FSx on the published candidate |
| CHECKPOINT-02 | Current candidate needs affected Owner/basic local-file recovery rerun | High | Owner side of G2.08, G2.27 | G1 remains complete; current candidate cannot inherit g1.5 recovery PASS | Run Owner local/remote basics and Meta local-file restart on current candidate |
| CHECKPOINT-03 | OwnerFs local performance target is unqualified | High | G2.09-G2.11 | Internal optimizations are bounded; no ext4 target claim | Freeze 64 MiB read/write/delete cases against native ext4 and run them |
| CHECKPOINT-04 | OwnerFs remote parity is unqualified | High | G2.14-G2.16 | The old remote speedup target is superseded by MooseFS parity | Freeze MooseFS parity cases and run remote read/write/delete after local basics |
| CHECKPOINT-05 | DFS current-candidate standard entry has not run | High | G2.05, DFS side of G2.06-G2.08, G2.21-G2.27 | Do not block Owner local progress on DFS standard entry | Run DFS pjdfstest, DFS-relevant LTP/FSx and DFS basic check before DFS performance claims |
| CHECKPOINT-06 | DFS one-writer/many-readers is unqualified | High | G2.21, DFS priority | Existing DFS basic and batch evidence does not prove this scenario | Run one writer plus multiple readers with 3FS comparison before broader DFS cases |
| CHECKPOINT-07 | Bind/native ON is not production-qualified | High | G2.12-G2.13 ON path | Production admission disabled; require a future explicit switch default OFF; PR43 remains partial/in-progress | Add explicit switch, OFF regression, ON lifecycle/drain/namespace semantics and paired performance evidence |
| CHECKPOINT-08 | Native semantic prerequisites remain open | High | G2.12 public ON | Known append/seek, POSIX owner locks, mixed mmap/watch, namespace/root/epoch/drain gaps | Close before claiming ON safe; do not block OFF trial or FUSE performance work |
| CHECKPOINT-09 | Comparator baselines are not fully frozen for all G2 performance | Medium | Affected performance items only | Functional and small standard work can continue | Freeze per-case baselines, resource budget, barrier semantics and noise tolerance before each comparison |
| CHECKPOINT-10 | VM disk capacity is tight for 8 GiB and long cases | Medium | G2.17-G2.20, G2.26, G3 long gates | Does not block standard or 64 MiB cases | Admit capacity per case; resize only after preserving active services/state |
| CHECKPOINT-11 | Formal 69-case manifest remains NOT_RUN/ENV PREPARING | Medium | G3/final release | Do not count it as G1/G2 small-case progress | Keep manifest/tools updated; run only when entering formal gate |
| CHECKPOINT-12 | etcd resource behavior needs separate topic handling | Medium | G3.12 | User allowed 2 GiB; do not spend current G2 on this unless correctness fails | Revisit after core Owner/DFS performance version |
| CHECKPOINT-13 | Redis backend parity is lowest priority | Medium | G3.13 | Existing implementation/local slices are not final parity | Keep as last/TODO unless a shared backend abstraction defect appears |

## Historical D-series boundary

| ID | Current boundary |
| --- | --- |
| D17 | etcd memory/resource growth is deferred to the user-approved 2 GiB topic lane; it is not a blocker for the Owner/DFS core performance version unless correctness fails. |
| D18 | Exact unknown-result replay work is closed only in its bounded evidence scope; it does not qualify full backend/network ACK-loss matrices. |
| D20 | Local Owner capacity/statvfs/resource semantics are closed in the recorded local slice; remote Home, DFS and root authority capacity semantics remain open. |

## Triage rules

- Critical correctness issues override the table: corruption, unsafe success, permission bypass, acknowledged-data loss or core recovery failure are fixed immediately.
- A blocked comparator or large-case resource issue blocks only the item that needs it.
- A failed experiment is closed when it has enough evidence and no new input; do not spend repeated cycles on unchanged residuals.
- Use memory and local-file lanes for filesystem progress. Durable backend lanes must not be used as a substitute for basic filesystem evidence.

## Publication review follow-up

| ID | Finding | Current boundary | Next action |
| --- | --- | --- | --- |
| REVIEW-01 | `DfsMeta::resolve_lock_authority` / `resolve_write_authority` have `open_write` trait defaults | Production `GrpcDfsMeta` overrides both with the dedicated resolver RPCs; current recording/capturing test adapters also override. No active production bypass was found. A future adapter could inherit the fallback unintentionally | When adding an adapter, require explicit implementations or move fallback to a named test adapter; retain resolver-versus-open regressions. This code-quality risk does not qualify new adapters |

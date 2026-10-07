# Active issue ledger

This ledger tracks active risks against [the three-stage checklist](trial-release-goals.md). Historical defects with their own evidence remain in Git history and status pages; this file keeps only issues that shape the next execution choices.

| ID | Issue | Severity | Blocks | Current handling | Next check |
| --- | --- | --- | --- | --- | --- |
| REMEDIATION-01 | Historical Python snapshot duplication inflates current tree | High | Evidence maintainability only | [R1 report](evidence/20261006-r1-repository-remediation/README.md);384historical files removed, Linux exact restore and fixture checks pass; raw failures retained | Current-tree cleanup complete; historical Git volume deferred |
| REMEDIATION-03 | OwnerFs workspace bind mount core naming/ownership is mixed with runc adapter | High | Separate naming/ownership exit; full bind function remains G2.12 | [Approved narrow plan](ownerfs-workspace-bind-remediation.md); physical Home→FUSE firstlevel exists only in adapter private namespace, not ordinaryhost | One OwnerFs/bind_mount.rs core, generic target component, keep runc lifecycle and defaultOFF/config compatibility; targeted Linux checks/review |
| REMEDIATION-02 | Vendored fuser contains private changes | High | Dependency policy; next migrated product candidate | Official0.16 complete21-path diff recorded; official0.18/master lack flock discrimination/interrupt APIs. No vendor edits made | Await lock support scope decision; do not silently change behavior or claim migration done |
| CHECKPOINT-01 | OwnerFs standards passed with version-specific scope | Medium | G2.27 packaging and affected regression only | e925 dev: pjdfstest236/8819, fixed LTP6/6 and short FSx PASS; same-source release: pjdfstest236/8819 PASS, LTP/FSx not rerun | Reuse unchanged proofs; functional gaps get targeted checks, no blanket rerun |
| CHECKPOINT-02 | Core local-file Meta recovery passed in bounded scope | Medium | Broader crash matrices / new package only | e925 dev single/two-VM Owner+DFS recovery PASS; release Owner single-VM recovery PASS; G2.08 complete | Retain binary/scope evidence; do not reopen G1 or bounded G2.08 |
| CHECKPOINT-03 | OwnerFs ordinary local read/write performance below target | High | G2.09-G2.10 | dev ratios0.5709/0.6297; same-source release0.3477/0.5879; correctness PASS, performance FAIL preserved; G2.11 deletion report done | Keep baseline data, defer targeted optimization; container workspace path first |
| CHECKPOINT-04 | OwnerFs remote parity is unqualified | High | G2.14-G2.16 | Cross-VM basic/recovery PASS is functional; MooseFS strong-durable write comparator BLOCKED separately | Other remote cases may baseline, retain data; targeted tuning follows container workspace priority |
| CHECKPOINT-05 | DFS capacity and standards qualified only for local R1 | Medium | Wider remote/replicated capacity only | map6161e25b real FD capacity and local R1 pjdfstest236/8819 plus LTP6/6 PASS, normal cleanup; e925 ENOSYS failures retained; [proof](evidence/20261007-dfs-statfs/README.md) | Local R1 gap closed; no fake aggregate capacity or unchanged suite reruns |
| CHECKPOINT-06 | DFS one-writer/many-readers functional branch passed; comparison pending | High | G2.21 performance exit | e925 dev R2/64MiB one writer/two VM readers exact content+EOF and center restart PASS; not three-sync-durable/3FS | Preserve functional proof, comparator performance remains pending; ordinary tuning deferred |
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

**当前交付小项（2026-10-07）：** 同Rust6d51aeb/map66dbbe3e/157输入，已有OFF包在Linux umask077下重打与已测原包完整字节一致（ee25d589，14,728,435B），复现PASS；复用35项安装/恢复PASS。[复现命令与Git输入](../development/evidence/20261007-off-trial-handoff/README.md)、[版本化试用清单](guides/trial-6d.md)已审阅，固定prerelease发布准备中，不关闭G2.27全性能/ON出口。远端只读准入区分A旧helper保留4GiB不足与新case未冻结合同；B新夹具待准入，不降低64MiB或清理旧环境。[实际库存](../development/evidence/20261007-owner-remote-admission/README.md)。下一独立项为Owner远端小读/删新夹具和数据，普通优化暂缓；G1/G2计数不变。

**当前新增（2026-10-07）：** 相同Rust6d51aeb/map66dbbe3e/157输入，append-only r8补齐独立结果：顺序52B双路径内容、并发128唯一完整记录PASS；第三SEEK_CUR24/38及62个并发偏移不匹配FAIL，原生对照全部PASS，正常清理/独立postcheck PASS。[原始命令、版本和证明](../development/evidence/20261007-append-diagnostic/README.md)。原r6/锁/watch失败与性能摸底保留，不升级ON/G2.12/13；默认OFF，G1历史8/8和G2计数不变。本项诊断收口，偏移一致性待修；下一独立项为当前OFF试用包安装/核心恢复回执，不继续无输入复测本缺口或标准/性能。

## 前序检查点（原版本/范围；下一动作由上述当前入口覆盖）

**当前容器性能诊断（2026-10-07）：** 相同Rust6d51aeb/map66dbbe3e/157输入，真实容器内OFF/ON与同卷ext4完成C1、64MiB同步写/读、1000×4KiB六项元数据，各1预热+5配对。内容/正常清理PASS；ON耗时中位数写1.023、读1.031、元数据1.048–1.158×ext4。仅诊断留数，缓存未观察/FUSE计数NOT_OBSERVED，锁/append/watch缺口未闭合，不升级G2.12/13或生产ON；G1/G2计数不变。[原始数据/身份/命令](../development/evidence/20261007-container-perf/README.md)。普通性能及未变标准不重复。下一独立小项append/SEEK_CUR功能缺口。

**当前容器小项（2026-10-07）：** Rust main6d51aeb/map66dbbe3e不变，实际受管单容器基础生命周期、64MiB及正常清理已通过；本轮短混合语义新增4KiB mmap双向数据及权限/错误PASS，锁冲突、append/SEEK_CUR和跨路径watch传播FAIL，[逐项原始证据](../development/evidence/20261007-managed-semantics/README.md)。未执行的混合并发append不计通过；旧锁阻塞判据单列补强。完整G2.12/生产ON/G2.13仍未通过，默认OFF，G1历史8/8和G2计数不变。下一项容器workspace小规模性能仅作诊断留数，不用数字掩盖语义缺口；未变标准及普通性能不重复。下方旧记录保留原版本。

**当前默认OFF安装回归（2026-10-07）：** main25a8061/map8ef8b788可复现包，在无编译器Linux VM通过Owner/DFS各64MiB基础校验、中心local-file Meta有序重启读回及正常退出/卸载；[证据](../development/evidence/20261007-installed-off/README.md)。是G2.08当前候选分支，不重开G1、不关闭G2.27，不代表pjdfstest/性能或容器ON。

**最新源码切片（2026-10-07）：** map8ef8b788的默认OFF实验容器接线已通过限定Linux源码/实际挂载/FUSE检查；[结果与边界](../development/evidence/20261007-native-workspace/README.md)。用户授权安装官方runc后，[独立运行时准入已通过](../development/evidence/20261007-runc-runtime/README.md)；实际OwnerFs受管功能/性能未验收，G2.12/13未完成；前一候选map6161e25b的DFS标准PASS保留原版本。

**最新候选（2026-10-07）：** map6161e25b，DFS本地R1容量/完整固定pjdfstest/固定LTP6及正常关闭通过；Owner历史证据仍按e925 ELF范围复用。[结果](../development/evidence/20261007-dfs-statfs/README.md)。G1保持8/8。

# Implementation Status

Updated 2026-10-06. [Three-stage acceptance checklist](../development/trial-release-goals.md) owns tasks and completion; [current code checkpoint](../development/current-checkpoint.md) binds this publication, validation and portable historical evidence.

| Stage | Status | Scope |
| --- | --- | --- |
| G1 colleague trial | **DONE, 8/8** | Historical g1.5: Linux build/offline install, memory demonstration, OwnerFs local/remote, DFS basic cross-node I/O, central local-file Meta restart, selfcheck and ordered lifecycle |
| G2 core performance version | **ACTIVE** | 27 independent tasks: 9 bounded outputs complete, 2 have performance failures, 2 bind tasks in progress, 14 awaiting acceptance. Current Owner standards and short Owner/DFS FSx qualified in recorded scope; local read/write below target; local R1 DFS statfs and fixed standards now pass; two-VM core recovery now passes; comparator qualification and new performance package remain open |
| G3 complex reliability/backends | **Deferred, 13 tasks** | Long-running/complex faults, expanded matrices/HA; etcd topic at 2GiB, Redis last |

## Current code capabilities

| Area | Present behavior | Acceptance boundary |
| --- | --- | --- |
| Runtime | afs-meta/afs-node, TLS gRPC, REST health, separate OwnerFs/DFS FUSE mounts | Current Linux source/tool gate is recorded with exact input hashes; current two-VM Owner/DFS core and orderly central Meta recovery pass in G2.08 scope; independent performance package delivery remains G2.27 |
| Meta | memory, local-file, etcd, Redis implementations; persistent capability is distinct from volatile state | G1 central local-file recovery is qualified on g1.5. Other backends have scoped historical tests; broad parity/faults remain G3 |
| OwnerFs | Home files, remote routing, write-authority/lease checks, error propagation, ordered namespace/index maintenance | B1–B4 internal correctness/evaluator outputs complete in limited scope; current local pjdfstest and fixed six-test LTP pass; short FSx passes. 64MiB/C1 read/write correctness passes but performance fails at 0.5709/0.6297×ext4; MooseFS parity remains unqualified |
| DFS | immutable chunks, version commits, replica policies, coherent read plans, streaming integrity and shared verification within one read batch | Batch CPU/read-amplification diagnostic complete; R2/64MiB one writer and two independent Node reader views pass, including orderly central Meta recovery; performance comparison and three-sync-durable 3FS parity remain unqualified |
| Capacity/health | Observed backend capability and readiness, Owner local filesystem capacity/error handling | Owner D20 and current local R1 DFS real fstatvfs slices only; remote/replicated aggregate capacity and wider faults remain open |
| Native Home | Private grant/identity foundation plus default-OFF administrator experimental single-container controller/source-FD export/final-clone cleanup | [Partial source slice](../development/native-workspace-slice.md); [Official runc installation/runtime-only admission PASS](../development/evidence/20261007-runc-runtime/README.md); [Actual managed single-container lifecycle PASS](../development/evidence/20261007-managed-workspace/README.md); Mixed-path mmap bytes/permissions pass; locks, append offset and cross-watch fail in the [short semantic evidence](../development/evidence/20261007-managed-semantics/README.md). Managed ON functional/performance exit, production READY/revocation/drain and restart reconciliation remain unqualified |
| Packaging | Process control, offline package generation, trial configuration and selfcheck | g1.5 retained; main25a8061 default-OFF fresh compiler-free install and bounded recovery pass. New ordinary trial package available separately; not a performance release or container ON package |
| Transport | gRPC and optional RDMA code plus scoped fault proofs | Actual RXE short results do not qualify the whole RDMA exception/resource matrix or physical NIC performance |

## Validation boundaries

Historical full pjdfstest results belong to v37/v48. Current e925c5b Owner local pjdfstest passes 236 files/8819 checks (28 upstream TODO, zero skips/unexpected failures); this is not full POSIX certification. The e925 DFS ENOSYS/zero-TAP/six-TBROK failures remain historical receipts. The new map6161e25b release candidate passes local R1 DFS pjdfstest236/8819 with28TODO and zero skips/unexpected failures, plus fixed LTP6/6 (651 unselected), with normal stop/unmount. [Current candidate evidence](../development/evidence/20261007-dfs-statfs/README.md). Owner/DFS short FSx passes seed1/1000 only. [Current raw ledger](../development/evidence/20261006-e2e-current/README.md) records the scope and failures. ext4 reference results are not AFS passes. The original full 69-case manifest remains NOT_RUN with environment PREPARING; it is the expanded final catalogue, not the G1 progress denominator.

See [checkpoint results](../development/checkpoints/20261006-current/results/README.md) for fresh combined-source checks. The historical 271-check g1.5 audit, DFS batch paired results, ext4 finite-tool positive results and complete parent-FD failed experiment are available in the repository. Other archived scopes are explicitly identified as external archives in the checkpoint page.

## Next and priority

Latest user priority: standard pjdfstest for functional completeness; container-mounted workspace access from Issue42/PR43 is the first performance lane, after necessary default-OFF bind functionality/safety. Ordinary local/remote/DFS measurements can remain baseline reports with optimizations deferred. [R1 historical evidence cleanup](../development/repository-remediation.md) passed restoration and fixture checks; R2 upstream migration remains independently BLOCKED by public lock/cancellation API gaps, with the full diff recorded. This does not block independent functional or container workspace work.

Reuse passed current standards and single-/two-VM core recovery. Preserve both dev and same-source release local performance failures. Release build, Owner standards/basic operations and single-VM orderly Meta recovery passed; ordinary read/write baselines are retained and targeted tuning is deferred. Remote comparisons require independent qualification. When entering DFS, one-writer/many-reader is highest priority. Each case freezes its own prerequisites, fairness and budget; a missing complex baseline or large-disk condition does not block an independent safe small case. Owner local target is ≥90%ext4 throughput; remote target is MooseFS parity; DFS matches 3FS under the same POSIX interface and three synchronous durable copies. Delete requires correctness and a quantitative report, without a new hard ratio.

Bind functionality and performance are separate G2.12/G2.13 exits, with an explicit experimental switch default OFF. OFF delivery proceeds independently; known ON safety gaps must be resolved before enabling ON. Complex reliability/etcd/Redis work remains later, while ordinary-use corruption, unsafe success, permissions and core recovery defects are repaired immediately.

**构建VM历史归档（2026-10-07）：** 旧evidence/artifacts/logs已流式复制到主机，9.49GiB归档、11,852条目/10,661文件内容及基本元数据一致，源端前后无变化；VM原件保留，尚未释放空间，移除决定待用户答复。当前构建缓存/基线/工具链未动，最终修复验证仍阻塞。[归档回执](../development/evidence/20261007-vm-archive/README.md)。

**Owner标准复用审计（2026-10-07）：** e925→已交付6d OFF普通操作未变，构造/statvfs等价变化及既有守卫支持历史限定复用；pjdf有e925 release，LTP6/短FSx仅dev。当前安装/套件只读身份匹配，未重跑标准、不增加PASS或计数。[审计与原proof索引](../development/evidence/20261007-owner-standard-reuse/README.md)。afs-build满盘仍停分支等待用户决定，草稿验证/新包/实际重启尚未完成。

**当前状态（2026-10-07）：** 3FS基线资格按用户决定留专题，主线容器workspace。main f09185e、产品6d/157/map66/既有ELF包的通过范围保留；新rootfs私有副本/命令续号修复在独立分支fix/native-orderly-recovery-20261007，不视为新可用候选。G1历史8/8，G2为10限定完成/2性能FAIL/2bind进行中/13待验收，bind默认OFF。

**新增/阻塞：** 工具11个独立Linux guards通过；早期Rust10+4有输出但未冻结源码SHA，不计最终版本PASS。静态审阅发现并最小修正非root测试入口，原FAIL保留。afs-build85GiB根盘实际满：误用debug与root工具链下载、后续ENOSPC，已停止构建并求助；最终续号/源码门禁/新包/重启读回均未完成。[版本、失败与证据](../development/evidence/20261007-native-orderly-recovery/README.md)。

**下一项：** 等环境处理决定后一次准入，复用release缓存、普通用户构建、仅真实test binary特权运行；新包后只做4KiB两阶段正常重启读回，不重跑标准/性能或3FS。独立文本/审查与证据收口不受该阻塞。

**历史说明：** 下方旧检查点按原版本/范围保存；其旧下一动作由上方当前入口和验收主表覆盖。当前native偏移/经典锁/watch失败与普通性能失败原始记录均保留。

# Implementation Status

Updated 2026-10-07. [Three-stage acceptance checklist](../development/trial-release-goals.md) owns tasks and completion; [current code checkpoint](../development/current-checkpoint.md) binds this publication, validation and portable historical evidence.

| Stage | Status | Scope |
| --- | --- | --- |
| G1 colleague trial | **DONE, 8/8** | Historical g1.5: Linux build/offline install, memory demonstration, OwnerFs local/remote, DFS basic cross-node I/O, central local-file Meta restart, selfcheck and ordered lifecycle |
| G2 core performance version | **ACTIVE** | 27 independent tasks: 10 bounded outputs complete, 2 have performance failures, 2 bind tasks in progress, 13 awaiting acceptance. Current Owner standards and short Owner/DFS FSx qualified in recorded scope; local read/write below target; local R1 DFS statfs and fixed standards now pass; two-VM core recovery now passes; comparator qualification and new performance package remain open |
| G3 complex reliability/backends | **Deferred, 13 tasks** | Long-running/complex faults, expanded matrices/HA; etcd topic at 2GiB, Redis last |

## Current code capabilities

| Area | Present behavior | Acceptance boundary |
| --- | --- | --- |
| Runtime | afs-meta/afs-node, TLS gRPC, REST health, separate OwnerFs/DFS FUSE mounts | Current Linux source/tool gate is recorded with exact input hashes; current two-VM Owner/DFS core and orderly central Meta recovery pass in G2.08 scope; independent performance package delivery remains G2.27 |
| Meta | memory, local-file, etcd, Redis implementations; persistent capability is distinct from volatile state | G1 central local-file recovery is qualified on g1.5. Other backends have scoped historical tests; broad parity/faults remain G3 |
| OwnerFs | Home files, remote routing, write-authority/lease checks, error propagation, ordered namespace/index maintenance | B1–B4 internal correctness/evaluator outputs complete in limited scope; e925 local pjdfstest and fixed six-test LTP pass in their recorded scope; short FSx passes. 64MiB/C1 read/write correctness passes but performance fails at 0.5709/0.6297×ext4; MooseFS read/write parity remains unqualified; current 6d small remote deletion correctness, quantified report and normal cleanup complete [G2.16](../development/evidence/20261007-owner-remote-delete-small/README.md) |
| DFS | immutable chunks, version commits, replica policies, coherent read plans, streaming integrity and shared verification within one read batch | Batch CPU/read-amplification diagnostic complete; R2/64MiB one writer and two independent Node reader views pass, including orderly central Meta recovery; performance comparison and three-sync-durable 3FS parity remain unqualified |
| Capacity/health | Observed backend capability and readiness, Owner local filesystem capacity/error handling | Owner D20 and current local R1 DFS real fstatvfs slices only; remote/replicated aggregate capacity and wider faults remain open |
| Native Home | Private grant/identity foundation plus default-OFF administrator experimental single-container controller/source-FD export/final-clone cleanup | [Partial source slice](../development/native-workspace-slice.md); [Official runc installation/runtime-only admission PASS](../development/evidence/20261007-runc-runtime/README.md); [Actual managed single-container lifecycle PASS](../development/evidence/20261007-managed-workspace/README.md); Mixed-path mmap bytes/permissions pass; locks, append offset and cross-watch fail in the [short semantic evidence](../development/evidence/20261007-managed-semantics/README.md). Managed ON functional/performance exit, production READY/revocation/drain and restart reconciliation remain unqualified |
| Packaging | Process control, offline package generation, trial configuration and selfcheck | g1.5 retained; main6d51aeb default-OFF fresh compiler-free install and bounded recovery35checks pass; [fixed trial package](https://github.com/lelezi257/dms/releases/tag/afs-trial-6d51aeb) is available. not a performance release or container ON package |
| Transport | gRPC and optional RDMA code plus scoped fault proofs | Actual RXE short results do not qualify the whole RDMA exception/resource matrix or physical NIC performance |

## Validation boundaries

Historical full pjdfstest results belong to v37/v48. Historical e925c5b Owner local pjdfstest passes 236 files/8819 checks (28 upstream TODO, zero skips/unexpected failures); this is not full POSIX certification. The e925 DFS ENOSYS/zero-TAP/six-TBROK failures remain historical receipts. The new map6161e25b release candidate passes local R1 DFS pjdfstest236/8819 with28TODO and zero skips/unexpected failures, plus fixed LTP6/6 (651 unselected), with normal stop/unmount. [Historical candidate evidence](../development/evidence/20261007-dfs-statfs/README.md); [6d scoped reuse audit](../development/evidence/20261007-standard-reuse/README.md), with no new6d full-suite run. Owner/DFS short FSx passes seed1/1000 only. [Current raw ledger](../development/evidence/20261006-e2e-current/README.md) records the scope and failures. ext4 reference results are not AFS passes. The original full 69-case manifest remains NOT_RUN with environment PREPARING; it is the expanded final catalogue, not the G1 progress denominator.

See [checkpoint results](../development/checkpoints/20261006-current/results/README.md) for fresh combined-source checks. The historical 271-check g1.5 audit, DFS batch paired results, ext4 finite-tool positive results and complete parent-FD failed experiment are available in the repository. Other archived scopes are explicitly identified as external archives in the checkpoint page.

## Next and priority

Latest user priority: standard pjdfstest for functional completeness; container-mounted workspace access from Issue42/PR43 is the first performance lane, after necessary default-OFF bind functionality/safety. Ordinary local/remote/DFS measurements can remain baseline reports with optimizations deferred. [R1 historical evidence cleanup](../development/repository-remediation.md) passed restoration and fixture checks; R2 upstream migration remains independently BLOCKED by public lock/cancellation API gaps, with the full diff recorded. This does not block independent functional or container workspace work.

Reuse passed current standards and single-/two-VM core recovery. Preserve both dev and same-source release local performance failures. Release build, Owner standards/basic operations and single-VM orderly Meta recovery passed; ordinary read/write baselines are retained and targeted tuning is deferred. Remote comparisons require independent qualification. When entering DFS, one-writer/many-reader is highest priority. Each case freezes its own prerequisites, fairness and budget; a missing complex baseline or large-disk condition does not block an independent safe small case. Owner local target is ≥90%ext4 throughput; remote target is MooseFS parity; DFS matches 3FS under the same POSIX interface and three synchronous durable copies. Delete requires correctness and a quantitative report, without a new hard ratio.

Bind functionality and performance are separate G2.12/G2.13 exits, with an explicit experimental switch default OFF. OFF delivery proceeds independently; known ON safety gaps must be resolved before enabling ON. Complex reliability/etcd/Redis work remains later, while ordinary-use corruption, unsafe success, permissions and core recovery defects are repaired immediately.

# DMS 原生 Filesystem 文件身份与生命周期代码导读

## 1. 本轮结果

M1.3 已把文件名、文件身份和数据生命周期分开：删除目录项不再等于立即删除 inode，更不等于立即释放 Block。当前实现支持 hard link、symlink、`unlink-open`、FUSE `FORGET/BATCH_FORGET`、Node 重连后的引用重报，以及 Meta 重启后可恢复的 orphan 回收。

这次没有创建第二套数据模型：

- Filesystem 拥有 dentry、inode、`link_count`、open handle 和 FUSE lookup 引用。
- DataCore 继续独占 `ObjectVersion → VersionLayout → Extent → Block`。
- NodeState 是本地 open/lookup 引用的唯一 owner；同一 inode 的多个本地引用只在第一次取得、最后一次释放时访问 Meta。
- MetaState 是共享 namespace、Node 引用租约和 durable orphan 回收的唯一 owner。
- 公开 KV SDK、WorkerService 数据接口和 write-through 写入合同没有改变。

真实单 VM 和三 VM FUSE 用例均为 PASS：两个 hard link 指向同一 inode，删除一个名字后另一个仍可读；文件最后一个名字删除后，已经打开的 fd 仍可读；symlink 返回精确 target；关闭和 `FORGET` 后 orphan 才进入持久回收；Meta 与 Node 重启后 namespace、内容和回收状态都没有倒退。

## 2. 为什么不能在 unlink 时直接删除数据

用户执行：

```text
fd = open("/work/a.txt", O_RDONLY)
unlink("/work/a.txt")
read(fd, buffer)          # 必须继续成功
close(fd)
```

`unlink` 只删除 `("/work", "a.txt") → inode 101` 这条 dentry，并把 inode 101 的 `link_count` 减到 0。`fd` 仍指向 inode 101，所以 inode、精确内容版本和底层 Block 必须继续存在。只有所有名字、open handle 和内核 lookup 引用都消失后，才允许回收。

```text
名字生命周期         dentry ------------------------------> 删除
inode 生命周期       inode 101 -----------------------------> 回收
                    link_count=1    link_count=0
open handle 生命周期      fd -----------------------> close
FUSE lookup 生命周期      nlookup --------------------------> FORGET
数据生命周期          ObjectVersion → Extent → Block -------> retirement
```

这几条生命周期不能合成一个计数：`link_count` 是持久 namespace 状态；open/lookup 引用是 Node 进程运行期状态，并通过租约报告给 Meta；Block 是否可以物理释放还受版本保留和 retirement ACK 约束。

## 3. 五个核心抽象的分工

| 抽象 | owner | 是否持久化 | 解决的问题 |
| :--- | :--- | :--- | :--- |
| Dentry | Meta Filesystem state | 是 | 父目录中的名字指向哪个 inode；hard link 是多个 dentry 指向同一 inode |
| Inode + `link_count` | Meta Filesystem state | 是 | 文件稳定身份和仍在 namespace 中的名字数量 |
| OpenHandle | Node Filesystem state | 否 | 已打开 fd 在 unlink 后仍定位到原 inode |
| Local inode reference | NodeState | 否 | 汇总本 Node 的 open handle 与 FUSE nlookup；0→1 建立 Meta 租约，1→0 停止续租，orphan 才立即 release |
| Leased inode reference | MetaState | 否，重启后由 Node 重报 | 判断 `link_count=0` 的 inode 是否仍被某个存活 Node 使用 |

内容对象不属于以上五项。普通文件和 symlink inode 的 `FileContentBinding` 精确指向 DataCore ObjectVersion；版本再通过既有 Extent 引用 Block。

## 4. unlink-open 的完整流程

```sequence
participant K as Kernel / 用户
participant F as FUSE + SharedFiles
participant N as NodeState
participant M as MetaState
participant D as DataCore
K ->> F: lookup("a.txt")
F ->> M: lookup，并在同一 actor turn 建立 entry reference
M -->> F: inode + generation + lease
F ->> N: 安装本地 nlookup=1
K ->> F: open(inode)
F ->> N: 本地引用 1→2，不访问 Meta
K ->> F: unlink("a.txt")
F ->> M: 删除 dentry，link_count 1→0，append WAL
M -->> F: namespace mutation 完成
K ->> F: read(fd)
F ->> D: 按 inode 的 exact version 读取
D -->> K: 原内容
K ->> F: release(fd)
F ->> N: 本地引用 2→1
K ->> F: FORGET(inode, 1)
F ->> N: 本地引用 1→0
N ->> M: release(inode, generation)
M ->> M: grace 后确认无 live reference
M ->> M: 一条 WAL 原子删除 inode + tombstone 内容对象
M ->> D: 既有 retention / Block retirement 后续安全释放
```

### 为什么 lookup 也要保护 inode

FUSE 在返回 lookup 结果后，内核可能暂存 inode，而不是立即 open。Node 必须把这个 `nlookup` 计入引用；内核以后通过 `FORGET` 或 `BATCH_FORGET` 归还。否则 `lookup → unlink → open by inode` 之间，Meta 可能过早回收 inode。

### 为什么 open 不一定访问 Meta

lookup 已让本地计数从 0 变成 1；随后 open 只是 1 变成 2。Node 只在本地计数 0→1 时 acquire Meta 租约；最后一次 1→0 时，普通有名文件停止 heartbeat 续租并等待租约自然到期，已经失去最后一个 dentry 的 orphan 才立即 release。这样同一 Node 上的多个 fd 和 nlookup 不会把 Meta 变成每次 open/close 的同步热点，同时 orphan 回收不必固定等待完整租约窗口。

## 5. generation 解决什么问题

同一 inode 的引用会经历多轮：

```text
generation 41: acquire ───────── release（网络延迟）
generation 42:          acquire ─────────────────── live
```

如果延迟到达的 generation 41 release 可以无条件删除记录，就会误删新的 generation 42 引用。Meta 只接受“generation 与当前记录相同”的 release；renew/re-report 只允许 generation 向前推进。generation 是一次本地引用周期的 fencing token，不是 inode ID，也不是用户可见版本号。

## 6. Node 崩溃与 Meta 重启

Node 的 open handle 无法在进程崩溃后继续使用，但 Meta 不能因为连接瞬断就立刻回收。规则是：

1. Node 正常运行时，heartbeat 批量续租当前引用。
2. Node 断线后，旧租约在 TTL 内继续保护 inode，允许进程快速重连。
3. Node 重连得到新 epoch，并在 heartbeat 上批量 re-report 当前仍存在的本地引用。
4. Meta 重启后内存租约表为空，因此先进入恢复保护窗口；窗口内不回收 orphan，给 Node re-report 留出时间。
5. 新的普通 acquire 不能重新打开 `link_count=0` 的 inode；只有已有本地引用的 renew/re-report 可以恢复保护。

这保证了短暂重启不会误回收，也避免已经完全不可达的 orphan 被任意新请求永久复活。

## 7. durable orphan 如何回收

Meta 周期任务只选择同时满足三个条件的 inode：

- `link_count == 0`；
- 所有 Node 的引用租约都已释放或过期；
- Meta 恢复保护窗口已经结束。

回收不是直接 free Block，而是追加一条 `FilesystemOrphanReaped` WAL：

```text
FilesystemOrphanReaped
├─ remove inode 101
└─ tombstone fs/content/101 的新 ObjectVersion

随后复用：
Version retention → 找出不再引用的 Block → Block retirement → 参与 Node ACK → Arena 释放
```

inode 删除和对象 tombstone 必须在同一条 WAL 中。这样 append 失败时两者都不生效；重放时也不会出现“inode 消失但内容 Current 还活着”或相反的半完成状态。

## 8. hard link 与 symlink

### 8.1 hard link

```text
link("a.txt", "b.txt")

(parent, "a.txt") ─┐
                    ├─> inode 101, link_count=2
(parent, "b.txt") ─┘        └─> FileContentBinding → exact ObjectVersion
```

hard link 不复制内容、不创建新 inode，也不移动 DataCore object。并发创建不同 hard link 由 MetaState 单 owner 串行线性化；每次成功都基于 actor turn 内的最新 `link_count` 增一。

### 8.2 symlink

symlink 有自己的 inode。它的 target bytes（例如 `../target.txt`）作为普通不可变 DataCore 内容保存，`readlink` 返回这些 bytes；解析 target 是内核/调用方行为。创建时 dentry、symlink inode、target ObjectVersion 和授权水位由一条 WAL 原子发布，不能先让名字可见再补 target。

## 9. 关键代码导读

| 入口 | 重点 |
| :--- | :--- |
| `server/src/node/filesystem/fuse.rs` | `lookup` 安装 nlookup；`forget/batch_forget` 归还引用；`open/release` 管理 fd；`link/symlink/readlink` 映射 POSIX callback |
| `server/src/node/filesystem/shared.rs` | `acquire_inode_reference/release_inode_reference` 屏蔽本地计数与 Meta 租约；文件业务只看到 inode 引用合同 |
| `server/src/node/runtime.rs` | `LocalFilesystemInodeReference`、计数、generation、lease deadline；NodeState 仍是唯一 owner |
| `server/src/meta/runtime.rs` | 引用 acquire/release/renew、generation fencing、recovery grace、`reap_filesystem_orphans` |
| `server/src/meta/filesystem/mod.rs` | dentry/inode/link_count 的权威 namespace 变更；orphan 候选只表达逻辑状态 |
| `server/src/meta/metadata_journal.rs` | symlink 原子记录与 `FilesystemOrphanReapedRecord` 领域定义 |
| `server/src/meta/local_wal_journal.rs` | 新 WAL record 的稳定编码/解码；旧 tag 保持不变 |
| `protocol/proto/dms/v1/filesystem_meta.proto` | Node↔Meta 的 link/symlink/reference RPC DTO；领域实现不依赖 protobuf |
| `scripts/performance/evaluate_native_filesystem.py` | 按 create、hot read、peer first read、overwrite 等分类合同评价端到端延迟，不再复用旧的全局 5% 门禁 |
| `scripts/performance/evaluate_fuse_request_amplification.py` | 独立评价 FUSE/DataCore/Meta/Peer 请求次数和复制阶段，避免性能波动掩盖路径放大 |

## 10. 日志与 Metrics

Node 暴露：

- `dms_node_filesystem_inode_references`：本 Node 当前被保护的不同 inode 数量。
- `dms_node_filesystem_inode_reference_transitions_total{transition}`：`acquire/retain/release_partial/release_final` 次数。

指标不带 inode/path 等高基数标签。需要定位具体对象时使用结构化日志中的 `inode` 和 `generation`。Meta 完成 durable orphan 回收时记录 `meta.filesystem.orphan.reaped`，包括 WAL sequence 和是否发布了内容 tombstone。

## 11. 验证证据

| 门禁 | 结果 |
| :--- | :--- |
| Rust Filesystem/Meta/Node 回归 | PASS；覆盖 generation fencing、重连重报、恢复 grace、WAL append failure、真实 Local WAL reopen、并发 hard link |
| Linux workspace `--all-features` | PASS |
| 严格 Clippy 与 fmt | PASS |
| 单 VM 双 Node 真实 FUSE | PASS；hardlink、symlink、unlink-open、FORGET、Meta/Node 重启 |
| 三 VM 独立部署真实 FUSE | PASS；相同语义跨 Node 成立，orphan 在恢复保护后持久回收 |
| 白盒生命周期证据 | PASS；Node 本地证明 acquire 先于 final release，Meta 日志证明 durable reap 已出现；不据此宣称跨进程全序 |
| 性能与请求放大 | PASS；性能按分类合同评价，请求放大按白盒合同评价，两者独立通过 |

机器证据位于 `evidence/2026-09-16-filesystem-identity-lifecycle/` 与
`evidence/m1/g004-performance-safe-id-20260917-r1/`。单 VM 最终结果在
`single-vm-final-2/evaluation.json`；三 VM 最终结果在
`three-vm-final-after-release-policy/evaluation.json`；当前性能合同结果在
`performance-evaluation.json`，请求放大合同结果在 `amplification-evaluation.json`。

### 为什么性能门禁拆成两个合同

旧的“所有 case 都按同一个 5% 历史阈值”会把两类问题混在一起：同步 write-through
突变路径天然会多一次发布成本，而热读、跨节点首读才是 DMS 架构优势路径；同时，RPC
次数、复制阶段和请求放大即使延迟偶然不差，也必须被白盒合同单独约束。

最终门禁采用以下规则：

- 正确性、样本数、结果结构：任意失败就失败。
- 延迟：按分类合同评价。本地热读和跨节点首读必须领先；create/overwrite 属于同步发布路径，
  要保持有界同级并说明成本来源。
- 白盒路径：按请求放大合同评价。FUSE/DataCore/Meta/Peer 请求次数和复制阶段不得超过合同上限。
- 当前证据中本地热读领先 44.5%～54.7%，跨节点首读领先 11.5%～29.7%；create
  4 KiB/64 KiB 分别慢 10.96%/4.64%，1 MiB create 快 2.97%；64 KiB 中段覆盖慢
  9.1%，均符合分类合同。请求放大合同全部 PASS。

## 12. 本轮边界与下一步

当前仍是 write-through，没有 dirty page 或后台 writeback；chmod/chown、xattr/ACL 与
集群 statfs 已由 M1.4 补齐。fallocate、fsync 合同、文件锁和 mmap 仍属于后续 Roadmap
阶段。

M1.4 已按同一不变量完成：没有绕过 Meta authority，没有复制 NodeState/MetaState，
没有创建第二套 Extent/Block 模型，也没有为了 FUSE 改坏公开 KV 接口。

# DMS 原生 Filesystem 首条共享文件主链代码导读

## 1. 本轮结果

本轮不是只增加一组 FUSE callback，而是把一个共享文件从内核请求一直贯通到现有 DataCore、Node、Meta、Journal、Watch 与恢复路径。验收场景已经真实跑通：Node A 创建并写入文件，Node B 首次读取并在本地复用数据，Node A 中段覆盖，Node B 收到失效事件后读取新版本，Meta 与 Node B 重启后仍能恢复并读取。

首版采用 **write-through**：`write/pwrite` 只有在数据候选已经准备完成、Meta 已用一条 journal 记录原子发布对象版本与 inode 绑定、需要等待的失效义务完成后才返回成功。当前没有 dirty page、后台回写线程或伪造的 write-back 状态；未来若引入 write-back，必须另行定义错误延迟、`fsync`、崩溃恢复与内存压力语义。

这条主链具备代表性，因为它同时覆盖了：

- path → inode 的 namespace 解析；
- inode、open handle 与文件属性生命周期；
- 文件字节复用既有 Version、Extent、Block 数据模型；
- 本地命中与跨 Node 首读；
- `pwrite` 中段覆盖而不整对象重写；
- DentryCache 与 BindingCache 的不同一致性边界；
- Meta 单 owner、先 journal 后 apply、Watch 失效与 ACK；
- WAL 重放后的 namespace、inode binding 与对象版本恢复；
- 真实热路径上的 typed metrics、trace 与错误日志。

## 2. 唯一状态与调用边界

```text
Kernel / FUSE
    │
    ▼
DmsFuse                         只翻译 FUSE 参数和 errno
    │
    ▼
SharedFileOperations            文件业务编排：lookup/open/read/write
    ├──────────────▶ DataCoreHandle ─▶ NodeHandle ─▶ 唯一 NodeState
    │                                  │              ├─ Arena / Block
    │                                  │              ├─ OpenHandleTable
    │                                  │              ├─ DentryCache
    │                                  │              └─ BindingCache
    │
    └──────────────▶ FilesystemMetaGrpcClient
                         │
                         ▼
                  FilesystemMetadataService
                         │
                         ▼
                    唯一 MetaState
                    ├─ FilesystemCatalog
                    ├─ 对象版本与副本位置
                    ├─ Journal / WAL / checkpoint
                    └─ Node Watch 事件
```

`DataCoreHandle` 不是第二个数据 owner。它只是 Node 进程内入口，把 Filesystem 的对象读写请求交给既有 `NodeHandle`。外部 KV 仍走 SDK → WorkerService → NodeHandle；进程内 Filesystem 走 FUSE → SharedFileOperations → DataCoreHandle → NodeHandle。两条入口最终共享同一个 NodeState、Arena、Block、版本与副本机制。

Filesystem 没有抽象公共 `Frontend`。FUSE、未来的镜像入口和 KV 各自保留自己的上层语义；真正收敛点是 NodeHandle，DataCoreHandle 只为进程内数据调用提供窄接口。

## 3. 一个文件由哪些抽象承载

用户创建 `/checkpoint` 并写入 `abcdef` 时，各层看到的内容如下：

| 层 | 抽象 | 这次操作中的值 | 价值 |
| :--- | :--- | :--- | :--- |
| Filesystem | Dentry | `(ROOT, "checkpoint") → inode 100` | 把路径名字与 inode 分开；以后 rename 不必改文件内容身份 |
| Filesystem | Inode | `inode=100, size=6, revision=2` | 承载 POSIX 属性与内容版本绑定 |
| Filesystem | FileContentBinding | `object_key="fs/content/100", exact_version=1` | inode 明确绑定一个不可变对象版本，不读取含糊的 Current |
| DataCore | VersionLayout | 逻辑长度 6，描述文件版本的字节布局 | 文件层不建立第二套布局模型 |
| DataCore | Extent | `[0,6) → Block B1[0,6)` | 表达逻辑区间到不可变数据块的映射 |
| Node | Block | `B1 = "abcdef"` | 可复制、校验、缓存和跨 Node 拉取的不可变 bytes |
| Node | Region/Allocation | B1 在 Node 本地内存中的物理位置 | 负责容量、对齐、SHM 与回收；不是文件语义 |
| Filesystem | OpenHandle | `fh 7 → inode 100 + 当前已解析 binding` | 让一次 open 后的 read/write 不重复做 path lookup |

当用户执行 `pwrite(fd, "X", 1, 2)`，新版本不是重新定义文件分片，而是继续使用 DataCore 的布局：旧 Block B1 的前后范围被复用，新 Block B2 保存补丁字节，Meta 用一次原子提交把新 VersionLayout 和 inode 的 `exact_version` 一起发布。

```text
Version 1
[0..6)  -> B1[0..6)       "abcdef"

pwrite(offset=2, bytes="X")

Version 2
[0..2)  -> B1[0..2)       "ab"
[2..3)  -> B2[0..1)       "X"
[3..6)  -> B1[3..6)       "def"

用户读取结果："abXdef"
```

## 4. 创建与 write-through 发布

```sequence
participant K as Kernel / FUSE
participant F as DmsFuse + SharedFileOperations
participant D as DataCore / NodeState
participant M as MetaState + Journal
participant W as 其他已缓存 Node
K ->> F: create("checkpoint")
F ->> M: CreateFilesystemInode(parent, name)
M ->> M: append inode-created journal，再 apply
M -->> F: inode 100 + cache grant
K ->> F: write(fh, offset=0, "abcdef")
F ->> D: prepare_range(object_key, base, patch)
D ->> D: 分配 Block，生成候选 VersionLayout
D -->> F: PreparedObjectVersion（尚未发布）
F ->> M: CommitFilesystemVersion(inode CAS + 对象候选)
M ->> M: 一条 journal 原子记录版本、binding、attrs、失效事件
M ->> W: Watch: invalidate inode 100 old generation
W ->> W: 先删除 BindingCache
W -->> M: ACK applied
M -->> F: 新 inode snapshot + exact version
F ->> D: finalize prepared blocks
F -->> K: write 成功
```

这里必须区分 `prepare` 与 `publish`：Node 可以先准备 bytes 和布局，但在 Meta 原子提交成功前，它们不能成为其他读者可见的文件版本。Meta 也不会先提交对象 Current、再用第二次请求更新 inode；对象版本、inode binding、size、mtime 和失效事件共用同一条 journal 序号。

## 5. Node B 首读与热读

```sequence
participant K as Kernel / FUSE
participant F as Node B Filesystem
participant M as Meta
participant P as Node A Peer
participant N as Node B NodeState
K ->> F: open("checkpoint")
F ->> F: DentryCache miss
F ->> M: LookupFilesystemEntry(ROOT, "checkpoint")
M -->> F: inode + exact version + 完整读取计划 + grant
F ->> F: 缓存 dentry 与 binding，创建 open handle
K ->> F: read(fh, 0, 6)
F ->> N: read_resolved(exact plan)
N ->> P: 缺失 Block 时按计划 PullBlock
P -->> N: Block bytes
N ->> N: 安装本地 Block
N -->> F: "abcdef"
F -->> K: 首读完成
K ->> F: 再次 open/read
F ->> F: DentryCache hit + BindingCache hit
F ->> N: 直接使用 exact plan 读取本地 Block
N -->> F: "abcdef"
F -->> K: 热读完成，不访问 Meta，不产生 Peer payload
```

首读慢不是因为 Filesystem 另造了一条数据通道，而是新 Node 必须取得权威 inode binding，并拉取本地尚不存在的 Block。热读则同时依赖两类缓存：

| 缓存 | key → value | 是否权威 | 失效规则 | 解决的问题 |
| :--- | :--- | :--- | :--- | :--- |
| DentryCache | lookup：`(parent inode, name) → DentrySnapshot`；readdir：`(directory inode, cursor) → DirectoryPage` | 否，只有正缓存 | Meta 在 namespace mutation 持久化后发送目录 revoke；Node 删除该目录的 lookup/page 缓存并 ACK，Meta 再返回变更请求；断流时由 lease expiry 兜底 | 重复 `open(path)` 不再逐次向 Meta lookup；`readdir` 只取当前页，不在 Node 聚合完整目录 |
| BindingCache | `inode → ResolvedInode + exact object plan + grant` | 否，受 grant 约束 | Watch 到达后先删除，再 ACK；断流由 lease 到期兜底 | 同一 inode 的 read/getattr 不再访问 Meta 或重新 ResolveObject |
| Node Block | `block_id → 本地不可变 bytes` | 数据副本 | 版本不原地修改；由既有副本与回收协议管理 | 热读不再拉 Peer payload，多入口复用同一份 bytes |

DentryCache 不保存“不存在”的结果，避免 negative cache 扩大一致性义务。目录页按 `(directory, cursor)` 独立缓存；目录 revoke 会清理该目录所有页，不会保留旧目录快照。BindingCache 保存的是 **Exact Version**，不是无条件相信 Current；这让主动失效和租约到期具有清晰边界。

## 6. pwrite、失效与重新读取

```sequence
participant A as Node A
participant M as MetaState
participant B as Node B
A ->> A: prepare_range：复用旧 Extent，新增补丁 Block
A ->> M: CommitFilesystemVersion(expected inode/object version)
M ->> M: CAS，append 一条 filesystem-version-committed journal
M ->> B: Watch invalidate inode + generation
B ->> B: evict BindingCache(inode)
B -->> M: ACK applied
M -->> A: publish 成功，新 exact version
B ->> M: 下一次 read/open miss 后重新解析 inode
M -->> B: 新 exact version + layout
B ->> A: 只拉本地缺失的新 Block
A -->> B: patch Block bytes
B -->> B: 用旧本地 Block + 新 Block 组合读取
```

写入方在成功响应中直接得到新 binding 并回填自己的缓存；其他 Node 由 Watch 失效。Meta 不把成功写入依赖于“希望事件最终能到达”：需要等待的 Node 必须先应用失效并 ACK，断连场景由 session/lease 约束收敛。

## 7. Meta 重启为什么能恢复

`FilesystemCatalog` 是唯一 MetaState 的字段，不是第二个 actor。它保存 inode、dentry 和目录 revision；对象版本与副本位置继续保存在同一个 MetaState 的既有数据结构中。

写操作遵循“先 journal，后 apply”：只有 durable append 成功，内存状态才前进。重启时 WAL 按顺序重放 `FilesystemInodeCreated` 与 `FilesystemVersionCommitted`，恢复 namespace、inode attributes、exact binding、对象版本和需要继续履行的事件状态。这样不会出现内存已经告诉客户端成功、进程崩溃后却找不到该版本的窗口。

当前可靠性边界是单 Meta 进程加本地 WAL/checkpoint，不等同于 Meta 高可用；多副本共识不在本轮范围。

## 8. protobuf 与领域代码的边界

Filesystem 使用独立的 `FilesystemMetadataService`，不会把 inode/dentry 参数混进既有 WorkerService 或对象 MetadataService。Node Watch 仍复用现有 Node→Meta 长连接，只新增 filesystem binding invalidation 事件。

进程内 Filesystem、SharedFileOperations 与 DataCore 使用原生 Rust 领域类型。generated protobuf 只允许出现在 `filesystem/wire.rs`、Meta gRPC handler 和 Node→Meta client 适配处。`ResolvedObject` 是既有对象读取计划的不透明包装，用于复用 Version/Extent/Block 计划，不重新实现布局算法，也不让 wire DTO 穿透 DataCore 接口。

## 9. 代码阅读顺序

| 顺序 | 代码 | 阅读重点 |
| :--- | :--- | :--- |
| 1 | [filesystem_meta.proto](../protocol/proto/dms/v1/filesystem_meta.proto) | 四个首版 Meta RPC：lookup、get inode、create、原子 commit |
| 2 | [model.rs](../server/src/filesystem/model.rs) | inode、dentry、content binding、cache grant 等稳定领域值 |
| 3 | [wire.rs](../server/src/filesystem/wire.rs) | 领域值与 protobuf 的唯一转换位置；不复制 Extent/Block 算法 |
| 4 | [shared.rs](../server/src/node/filesystem/shared.rs) | lookup/open/read/write 的核心文件业务编排 |
| 5 | [data_core.rs](../server/src/node/data_core.rs) | exact version 读取、候选版本 prepare、与 NodeHandle 的窄接口 |
| 6 | [runtime.rs](../server/src/node/runtime.rs) | 唯一 NodeState、Block/Arena、Peer 拉取及候选版本实现 |
| 7 | [dentry_cache.rs](../server/src/node/filesystem/dentry_cache.rs) | path→inode 正缓存的边界 |
| 8 | [binding_cache.rs](../server/src/node/filesystem/binding_cache.rs) | inode→exact plan 的 grant/lease 缓存 |
| 9 | [open_handles.rs](../server/src/node/filesystem/open_handles.rs) | FUSE file handle 到 inode/resolved binding 的本地生命周期 |
| 10 | [fuse.rs](../server/src/node/filesystem/fuse.rs) | FUSE callback 到 SharedFileOperations 的薄转换 |
| 11 | [meta filesystem catalog](../server/src/meta/filesystem/mod.rs) | 唯一 MetaState 内的 namespace 数据结构 |
| 12 | [meta filesystem handler](../server/src/meta/filesystem/service.rs) | generated service 到 MetaHandle 的转换边界 |
| 13 | [meta runtime](../server/src/meta/runtime.rs) | CAS、原子 journal 发布、事件生成、replay apply |
| 14 | [metadata_journal.rs](../server/src/meta/metadata_journal.rs) | 两类 filesystem journal record 与 checkpoint 数据 |
| 15 | [node.rs](../server/src/node.rs) | Watch 消费：先驱逐 binding，再 ACK |
| 16 | [真实验证脚本](../scripts/validation/run_filesystem_shared_file_e2e.sh) | 两 Node、FUSE、失效与重启恢复的可复现实验 |

## 10. 日志、Metrics 与 Trace

本轮只在真实边界加入观测，不让成功热路径输出日志：

- FUSE 的 lookup、getattr、open、create、read、write、close 建立 `dms.filesystem.*` span；
- Node typed metrics 记录 filesystem operation 次数/耗时、DentryCache 与 BindingCache hit/miss；
- Meta typed metrics 记录 filesystem lookup/get/create/commit、mailbox wait 与 journal append；
- 失败或异常恢复路径输出结构化 warning/error；正常缓存命中不逐次写日志；
- payload bytes 不携带 trace 字段，控制请求与本地业务 span 记录传输阶段和耗时。

## 11. 实测证据

证据目录：[2026-09-14-filesystem-shared-file](../evidence/2026-09-14-filesystem-shared-file/result.txt)。

| 项目 | 结果 | 它证明什么 |
| :--- | :--- | :--- |
| 完整主链 | PASS | A 写、B 首读/热读、pwrite 失效、新版本读取、Meta/Node 重启恢复均通过 |
| Node B 首读 | 231.954 ms | 包含首次 namespace/binding 解析、Peer Block 拉取和 FUSE 开销，不是热读基线 |
| 50 次热读中位数 | 0.868 ms | 重复 open/read/close 已走 Node 本地缓存与 Block |
| 50 次热读 p95 | 0.954 ms | 当前单 VM 真实 FUSE 热路径尾延迟 |
| Meta filesystem lookup | 2 次 | 50 次重复 open 没有变成 50 次 Meta lookup |
| Meta filesystem get inode | 1 次 | 失效/重启后才重新取得权威 binding |
| Node B DentryCache | 51 hit / 1 miss | path→inode 热缓存真实生效 |
| Node B BindingCache | 310 hit / 1 miss | inode→exact plan 热缓存真实生效 |
| Meta journal | create 1 条，version commit 2 条，均成功 | 初写和 pwrite 都走可恢复原子记录 |
| Meta 重启后恢复读取 | 4.597 s | 数据正确恢复；时延受当前 5 秒 session/heartbeat 重连节拍影响，不代表 WAL replay 本身耗时 |

原始数据：[latency.json](../evidence/2026-09-14-filesystem-shared-file/latency.json)、[recovery.json](../evidence/2026-09-14-filesystem-shared-file/recovery.json)、[meta.prom](../evidence/2026-09-14-filesystem-shared-file/meta.prom)、[node-b.prom](../evidence/2026-09-14-filesystem-shared-file/node-b.prom)。

## 12. 单 VM 手动复现

先在 Mac 上单独启动虚拟机：

```bash
limactl start dms-rc
limactl shell dms-rc
```

进入虚拟机后，再进入已经同步的源码目录并执行短命令：

```bash
cd /path/to/rust-distributed-memory-store/source
env CARGO_TARGET_DIR=/tmp/dms-filesystem-target \
  bash scripts/validation/run_filesystem_shared_file_e2e.sh
```

环境需要 Linux `/dev/fuse`、`fusermount3` 和可用的 Rust 工具链。脚本会自行启动一个 Meta、两个 Node、两个 FUSE mount，验证数据与指标后清理临时进程和挂载点。

## 13. 首版明确没有实现的范围

这是一条完整且真实的纵向主链，不是“完整 POSIX 已完成”。当前除 lookup、getattr、create、open、read、write/pwrite、release 外，已支持权威且可恢复的 mkdir、readdir、rename、unlink 和 rmdir。`readdir` 由 Meta 有序索引返回一页，Node 按页缓存，FUSE handle 只保存 cookie 到 Meta cursor 的映射。

当前仍未实现：

- link、symlink/readlink；
- truncate、`O_TRUNC`、稀疏文件完整规则；
- 文件锁、配额；属性授权、xattr、ACL 与集群 statfs 已由 M1.4 补齐；
- write-back、dirty page、flush/fsync 持久化语义；
- Meta 高可用和完整故障矩阵。

此外，中段覆盖已经使用 Extent overlay；文件向尾部扩展当前仍会物化完整对象，这是后续性能与稀疏文件设计需要处理的明确限制。

## 14. Review 检查点

- FUSE callback 是否只做参数/errno 转换，业务是否集中在 SharedFileOperations；
- DataCoreHandle 是否仍是 NodeHandle 的薄进程内入口，没有第二份状态；
- Filesystem 是否只引用一个文件对象 key，没有创建第二套 Extent/Block；
- 对象候选与 inode binding 是否由单一 Meta journal record 原子发布；
- Watch 是否先失效 BindingCache 再 ACK；
- 热 `open/read` 是否同时命中 DentryCache、BindingCache 和本地 Block；
- protobuf 是否止于 wire/client/handler 边界，没有进入 DataCore 接口；
- 日志、metrics、trace 是否只覆盖真实边界，成功热路径没有逐次日志；
- 未实现 POSIX 能力是否保持显式缺失，没有用假状态或空 handler 冒充实现。

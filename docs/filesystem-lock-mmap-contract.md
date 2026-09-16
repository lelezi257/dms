# DMS Native Filesystem 文件锁与 mmap 合同设计

> 状态（2026-09-16）：M1.6a 文件锁已经实现并通过单 VM、三 VM、阻塞中断、Meta
> 恢复重报和 Node epoch fencing 验证；M1.6b cached mmap 已完成 Rust 核心接线，并通过单 VM、
> 三 VM 真实 FUSE 验收。两个能力仍保持独立状态模型，不建立共同 manager。

## 1. 本阶段要解决什么

M1.5 已经完成空间预留、打洞和同步合同，但两个常见 POSIX 场景仍未闭环：

1. 两个 Node 上的进程同时对同一文件加锁时，当前内核只能看见本机锁，无法形成集群冲突关系。
2. 当前 FUSE regular file `open/create` 已返回 `FOPEN_KEEP_CACHE`，不再强制 direct I/O；远端变更
   通过 Watch 先失效 Node cache，再失效 Linux kernel page cache，最后才 ACK。

本阶段不能只在 `fuse.rs` 增加几个回调。文件锁需要跨 Node 的唯一权威状态、阻塞等待、故障
fencing 与重连恢复；mmap 则需要明确 Linux 页缓存、DMS immutable version、Watch 失效和
`msync/fsync` 的分工。

本阶段继续遵守四条边界：

- Meta 是共享文件锁状态的唯一 owner；Node 不建立第二个权威锁表。
- Linux 内核页缓存保存 mapped pages；DMS 不增加 `PageCache` 或 `DirtyPageManager`。
- 文件内容仍使用唯一的 `ObjectVersion → VersionLayout → Extent → Block`；不创建 mmap 专用 Block。
- M1.6a 不顺手引入 writeback；M1.6b 继续使用 M1.5 已确认的 write-through durability 合同。

## 2. 为什么拆成两个切片

| 切片 | 用户可见能力 | 核心状态 owner | 主要失败问题 |
| :--- | :--- | :--- | :--- |
| M1.6a | `F_GETLK/F_SETLK/F_SETLKW`、`flock` | Meta `FilesystemCatalog` | 冲突、等待取消、Node/Meta 故障后的锁释放与恢复 |
| M1.6b | `MAP_SHARED/MAP_PRIVATE`、`msync`、映射期间 truncate/打洞/远端写 | Linux 页缓存 + 既有 DataCore/Meta | 脏页提交、远端页失效、越 EOF、延迟错误 |

文件锁是共享协调状态；mmap 是内核缓存与文件内容版本之间的合同。两者都会经过 FUSE，但没有共同
的状态模型。只在验证用例里组合它们，代码中不为了“M1.6”造一个公共 manager。

## 3. 当前代码事实

| 当前代码 | 事实 | 缺口 |
| :--- | :--- | :--- |
| `node/filesystem/fuse.rs` | regular file `open/create` 返回 `FOPEN_KEEP_CACHE`；不启用 direct I/O 或 writeback cache | cached mmap 热读由 Linux page cache 承载 |
| `node/filesystem/open_handles.rs` | `OpenHandle` 保存内核 `lock_owner` | close/release 会按 owner 清理远端锁 |
| `node/filesystem/fuse.rs` | 已实现 `getlk/setlk` 与 interrupt | 跨 Node 冲突、等待和信号取消已进入真实 FUSE 流程 |
| `node/filesystem/kernel_cache.rs` | `KernelCacheInvalidator` 只保存/调用 `fuser::Notifier`，不保存页、版本或文件内容 | whole-inode invalidation 使用 `inval_inode(inode, 0, -1)` |
| `node.rs::consume_meta_events` | 远端 filesystem Watch 事件先失效 Node cache，再失效 kernel cache，最后 ACK；失败则断流重放 | 非 FUSE build 或未配置 mount 不阻塞 ACK |
| `meta/filesystem/locks.rs` | Meta actor 持有唯一 inode 区间锁表与有界 waiter | 锁状态不写 WAL，恢复时由 Node 重报完整镜像 |
| `meta/runtime.rs` | Node epoch、lease、心跳恢复标志与锁恢复屏障 | Meta 重启保留 session，但只在心跳要求时增加一次 reclaim RPC |

## 4. M1.6a：文件锁

### 4.1 用户看到的 E2E 行为

```text
Node A / process A                    Node B / process B
fcntl(fd, F_SETLK, WRLCK [0,4095])    │
  └─ 成功                             │
                                      ├─ fcntl(fd, F_SETLK, WRLCK [0,4095])
                                      │    └─ EAGAIN（不阻塞）
                                      └─ fcntl(fd, F_SETLKW, WRLCK [0,4095])
                                           └─ 等待
Node A 解锁 [0,4095]                  │
                                      └─ 被唤醒并取得锁
```

本机内核没有另一个 Node 的锁表，所以冲突判断必须进入 Meta。Node 只保存“本挂载当前成功持有的
owner/range”镜像，用于 close 清理、Meta 重连重报和诊断；它不是授权来源。

### 4.2 身份与范围

```rust
struct FileLockOwner {
    node_id: NodeId,
    node_epoch: NodeEpoch,
    lock_owner: u64,
}

struct FileLockRange {
    start: u64,
    end_inclusive: u64, // u64::MAX 表示直到 EOF
}

enum FileLockMode {
    Shared,
    Exclusive,
    Unlock,
}
```

这里不新生成 UUID。FUSE `lock_owner` 已表示内核锁 owner；与 `node_id + node_epoch` 组合后，
同一数值在 Node 重启后也不会误认成旧 owner。`pid` 只用于 `F_GETLK` 诊断返回，不能作为稳定身份，
因为 pid 会复用，也不能跨 Node 唯一。

### 4.3 `fcntl` 与 `flock` 怎么统一

Linux FUSE 提供 `FUSE_FLOCK_LOCKS` 能力时，内核把 `flock()` 通过 `SETLK` 交给文件系统处理；
`flock` 表达为整个文件范围的共享锁或排他锁，最后一个相关 file description 释放时，`release`
会携带对应 `lock_owner`。

首版采用内核定义的这种模拟合同：

- `fcntl` byte-range lock 与 `flock` 共用一张 inode 区间锁表。
- `flock` 是 `[0, EOF]` 的特殊范围，不建立第二张 whole-file lock 表。
- `release(Some(lock_owner))` 清理该 owner 的 whole-file 锁；显式 unlock 仍走 `setlk`。
- 因为采用 FUSE 的 flock-as-POSIX-lock 模型，两类锁会互相冲突；这是明确合同，不是假装实现了
  Linux 本地文件系统中彼此独立的两套锁族。

当前 `fuser 0.16` 虽然底层请求带 `lk_flags`，但公开 `Filesystem::setlk` 回调没有把该字段传给
业务实现；当前新版本同样没有解决这一点。如果未来必须严格区分两套锁族，只在 FUSE provider
边界维护一个最小补丁，暴露 `lk_flags`，不把该差异上传到 DataCore 或 Meta 通用接口。

### 4.4 Meta 中的状态与算法

```text
FilesystemCatalog
└─ locks[inode]
   ├─ granted: 按 start 排序的区间锁
   └─ waiters: 有界 FIFO 等待队列
```

一次 mutation 只在 Meta actor 的一个 turn 内做以下动作：

1. 规范化范围并清理相同 owner 被覆盖的旧区间。
2. 查找异 owner 的重叠冲突；共享锁之间不冲突，其余冲突。
3. 无冲突时插入、拆分或合并区间并返回成功。
4. 非阻塞请求有冲突时返回 `EAGAIN`。
5. 阻塞请求有冲突时只登记 waiter，然后释放 actor turn；不能让 Meta actor 阻塞等待。
6. unlock/owner expiry 后，在一个新 turn 中按 FIFO 检查 waiter，兼容的请求被授予并唤醒。

局部 unlock 必须能把一段区间拆成左右两段；相同 owner、相同 mode、相邻范围可以合并。`F_GETLK`
返回第一条确定冲突，而不是只返回布尔值。

### 4.5 mutation 身份、幂等与乱序响应

文件锁 mutation 的持久身份由 Node session 分配的单调 `mutation_sequence` 表示。FUSE request
`unique` 只用于把内核 `FUSE_INTERRUPT` 关联到当前等待请求；它会被内核复用，不能充当跨重试、
重连或多个 mount 共享的幂等编号。一个 Node session 内所有 `MetadataClient` clone 和 mount 共用同一
个原子序列，序列耗尽时拒绝复用。

Meta actor 为每个真正改变锁状态的操作再分配全局单调 `owner_revision`。两者解决的问题不同：

- `mutation_sequence` 判断同一请求的重试，精确 receipt 同时校验请求指纹，禁止“同 ID、不同参数”。
- `owner_revision` 表示 Meta 实际应用顺序，Node 用它拒绝网络乱序返回的旧 Set/Release 响应。

Release 使用三个有序阶段：先移除 owner 的锁和 waiter；再生成 release receipt、推进 release fence 并
分配 revision；最后才唤醒可能取得新锁的 waiter。这样被 Release 唤醒的新锁一定拥有更大的
`owner_revision`。Node 的锁镜像在同一把 mutex 下维护已授予锁、owner revision、release fence 和
在途 Set；旧响应不得删除或重新写入较新状态。session generation 变化、Meta 恢复后同 session reclaim
时会清除旧 revision 域，防止把新 Meta 从 1 重新计数的 revision 误判为陈旧。

为了避免异常客户端造成内存增长，Meta 对全局 active waiter、单 waiter 附着回复数、completed receipt、
release receipt 和 release fence 都设置有界窗口。`RecoveryPending`、队列已满等非终态结果不写入
completed receipt；精确重试在窗口内返回原结果，窗口外的旧 mutation 明确拒绝，不能伪装成成功。

### 4.6 阻塞锁为什么需要 FUSE provider 的小范围补丁

`F_SETLKW` 不能占住 FUSE 分发线程等待。`fuser` 的 reply 可以异步发送，因此 handler 可以把 reply
交给等待任务。上游 `fuser 0.16.0` 原本会对内核 `FUSE_INTERRUPT` 返回 `ENOSYS`；当前仓库已经通过
`[patch.crates-io]` 指向 `third_party/fuser`，在 provider 边界暴露 `Filesystem::interrupt` callback，
Node 能以 FUSE request identity 取消对应的 Meta waiter。

完整的 POSIX 阻塞语义要求：

- Meta waiter 与一个 FUSE request identity 关联。
- 收到 interrupt 时取消 waiter，并向内核返回 `EINTR`。
- 解锁与 interrupt 竞争时只有一个终态可以发送 reply。

这个补丁只修复 provider 缺失的信息，不发明 DMS RPC 框架，也不改变公开 SDK。真实 FUSE 用例已证明
信号中断会返回 `EINTR`、移除 waiter，且不会留下随后取得锁的幽灵请求。只有选定的上游 `fuser`
版本同时满足以下条件时才能删除 vendored patch：公开等价 interrupt callback、保留 request unique id、
允许内核生成的 interrupt 通过请求分发，并且现有 `interrupt_cancels_waiter` 回归继续通过。

### 4.7 Node 与 Meta 故障

锁是易失协调状态，不应像文件内容一样长期写入 WAL；但 Meta 重启也不能立即忘掉全部锁并授予
冲突请求。采用“lease + reclaim grace”恢复：

1. Node 在本地镜像本挂载已成功取得的锁。
2. Meta 重启后进入一个有界 recovery grace，期间不授予可能与旧 owner 冲突的新锁。
3. 存活 Node 重新建立已有 session，并批量 `ReclaimFilesystemLocks`。
4. Meta 使用新的 `node_epoch` 重装这些锁；旧 epoch 永远不能继续 mutation。
5. 全部旧 live Node 已完成 reclaim，或 grace/lease 到期后，恢复窗口结束。
6. Node 未能 reclaim 时，必须 fencing 该挂载的锁世代，后续相关调用返回 `ESTALE/EIO`；不能让应用
   继续相信已经丢失的锁。

Node 崩溃后内核挂载随进程消失，旧 epoch 的锁在 lease 到期后释放并唤醒 waiter。Meta 与 Node
之间复用已有 heartbeat/session，不新增 LockHeartbeat。

### 4.8 接口与目录

```text
server/src/
├─ filesystem/
│  └─ locks.rs                     领域值：owner/range/mode/request/result
├─ node/filesystem/
│  ├─ fuse.rs                      getlk/setlk/release、errno 与异步 reply
│  └─ lock_state.rs                本挂载已授予锁镜像、reclaim、wait cancellation
└─ meta/filesystem/
   ├─ mod.rs                       FilesystemCatalog 持有锁表
   └─ locks.rs                     区间算法、waiter、recovery grace

protocol/proto/dms/v1/
└─ filesystem_meta.proto
   ├─ TestFilesystemLock
   ├─ SetFilesystemLock
   └─ ReclaimFilesystemLocks
```

`TestFilesystemLock` 对应 `F_GETLK`，`SetFilesystemLock` 同时表达 acquire/unlock 与是否等待。
protobuf 只承载进程边界 DTO；区间算法不依赖 generated 类型。

## 5. M1.6b：mmap

### 5.1 FUSE 没有一个 `mmap()` handler

应用调用 `mmap()` 后，页缺失、脏页和回写主要由 Linux 内核页缓存处理。FUSE 用户态文件系统通常
看到的是 `read`、`write`、`flush`、`fsync`、`release`，而不是每次用户访问内存地址都收到回调。

因此 DMS 不需要新增：

- `MmapManager`
- 用户态 page fault handler
- Filesystem 私有 dirty page 队列
- mmap 专用 Version/Extent/Block

DMS 要做的是允许 cached I/O、正确处理内核发来的 read/write，并在远端版本变化时主动使内核页
缓存失效。

### 5.2 首版 I/O 模式决策

| 选项 | 首版选择 | 原因 |
| :--- | :--- | :--- |
| 所有 regular file 使用 `FOPEN_DIRECT_IO` | 删除 | direct I/O 绕过页缓存，默认不支持普通 shared mmap |
| cached write-through | 采用 | 支持全部 mmap mode；写请求仍及时进入 FUSE 与现有提交主链 |
| `FUSE_WRITEBACK_CACHE` | 不启用 | 会引入跨 callback 脏页、延迟错误和网络文件系统一致性风险，属于独立 writeback 设计 |
| `FUSE_DIRECT_IO_ALLOW_MMAP` | 不启用 | 虽允许 direct I/O mmap，但放宽 coherency，不适合作为首版分布式一致性基础 |

这里的 write-through 指 FUSE cached write-through 模式，不表示用户对映射地址每写一个字节就同步
进入 DMS。`MAP_SHARED` 页变脏后，内核在 writeback、`msync`、`fsync` 或回收时生成 FUSE WRITE；
每个 WRITE 继续走既有 `FileRange → DataCore patch Block → CommitFilesystemVersion`。

### 5.3 一个共享映射写流程

```sequence
participant A as 应用进程
participant K as Linux 页缓存
participant F as FUSE adapter
participant D as DataCore
participant M as MetaState
A ->> K: mmap(MAP_SHARED) 并修改一页
K ->> F: WRITE dirty range
F ->> D: prepare range patch
D -->> F: candidate layout
F ->> M: CommitFilesystemVersion
M -->> F: 新 exact version 与 revoke cursor
F -->> K: WRITE 成功
A ->> K: msync(MS_SYNC)
K ->> F: 必要 WRITE + fsync
F -->> A: 成功或此前延迟错误
```

Kernel page 是可变缓存；提交后的 DMS Block 仍是不可变数据。一次 4 KiB dirty page 可以形成一个
patch Block，旧 Version 的未覆盖 Extent 继续复用，不需要整文件读回。

### 5.4 远端写后如何避免旧页

Watch ACK 现在不只证明 Node `BindingCache/DentryCache` 已失效，还必须证明 Linux 页缓存失效已提交给
FUSE notifier。M1.6b 的 ACK 条件是：

```sequence
participant M as Meta Watch
participant N as Node cache
participant K as Kernel cache invalidator
M ->> N: InvalidateFilesystemBinding(inode, range, version)
N ->> N: 删除 BindingCache / 相关读计划
N ->> K: inval_inode(inode, offset, length)
K -->> N: 内核接受失效或 inode 已不存在
N -->> M: ACK cursor
```

只有 Node cache 与内核 cache 都成功失效后才能 ACK。若内核通知失败，Node 不假装已完成；它会断开
当前 Watch stream，让 Meta 之后按 cursor 重放事件。非 FUSE build 或未配置 mount 时没有内核页缓存
需要失效，Node 记录 skipped metric 后允许 ACK。

FUSE 类型不能泄漏到 DataCore。新增的真实边界是：

```rust
trait KernelCacheInvalidator {
    fn invalidate_inode(&self, inode: u64, offset: i64, length: i64)
        -> Result<(), KernelCacheError>;
}
```

它由 `fuser::Notifier` 实现，供 Watch consumer 调用。该接口只负责通知内核，不保存 page、不参与
版本解析。whole-inode 失效统一使用 `inval_inode(inode, 0, -1)`；这里的 `-1` 是 Linux/FUSE 合同中
“从 offset 到 EOF”的长度语义，比 `i64::MAX` 更准确。

### 5.5 mount 与 Watch 的启动顺序

Node 启动顺序已经调整为先创建 Watch session 与 `KernelCacheInvalidator`，再创建 FUSE mount 并安装
notifier，最后才启动 Watch consumer。这样不会出现事件已经 ACK 但 notifier 尚未安装的窗口：

1. Node 创建 `KernelCacheInvalidator`；若配置了 FUSE mount，则标记 notifier required。
2. FUSE session 创建后，把 cloneable notifier 安装到 invalidator。
3. 安装完成后才 spawn Watch consumer。
4. Watch consumer 收到远端 filesystem invalidation 后，按 Node cache → kernel cache → ACK 的顺序执行。

这只是解决组件初始化顺序，不是第二套 cache 或 actor。

### 5.6 mmap 与其它文件语义

| 场景 | 首版合同 |
| :--- | :--- |
| `MAP_SHARED` 写 + `msync(MS_SYNC)` | dirty range 经 FUSE WRITE 提交；随后 fsync 返回已确认 durability 或延迟错误 |
| `MAP_PRIVATE` 写 | 只改变进程私有 COW 页，不发布 DMS 新版本 |
| 远端覆盖映射范围 | Watch 在 ACK 前使对应 kernel pages 失效；后续 fault/read 取得新版本 |
| truncate shrink | 发布新 size 后失效尾部；映射访问新 EOF 之外由内核产生 `SIGBUS` |
| punch hole | 提交 HOLE 后失效对应页；后续读取为零 |
| unlink while mapped | 既有 open reference 保护 inode/content；最后 unmap/close 后 release，之后才允许 orphan 回收 |
| 两个 Node 无锁并发写同一范围 | 属于数据竞争；每次提交仍是完整版本，但最终顺序由 Meta 线性化，不承诺应用期望的合并结果 |

锁是 advisory。应用需要有序共享写时必须主动使用锁；mmap 不能自动把“应用没有加锁”升级为强制锁。

## 6. 修改后的源码边界

```text
server/src/
├─ filesystem/
│  ├─ model.rs
│  ├─ locks.rs                     新增：跨层锁领域值，无状态
│  └─ space_sync.rs
├─ node/filesystem/
│  ├─ fuse.rs                      FUSE 能力、锁回调、cached I/O
│  ├─ open_handles.rs              fd/lock owner/open reference
│  ├─ lock_state.rs                新增：本挂载锁镜像与 reclaim
│  ├─ kernel_cache.rs              Notifier 边界与 whole-inode invalidation，不存 page
│  └─ shared.rs                    继续编排内容 range 操作
├─ node/data_core.rs               不新增 mmap/lock 业务；继续处理内容版本
└─ meta/filesystem/
   ├─ mod.rs                       唯一 FilesystemCatalog
   └─ locks.rs                     新增：权威区间锁、waiter、recovery grace

protocol/proto/dms/v1/
└─ filesystem_meta.proto           锁 RPC；mmap 不新增 RPC
```

刻意没有 `mmap.rs`：FUSE 没有 mmap callback，增加该文件反而会暗示 DMS 自己管理虚拟内存页。

## 7. 可观测性

### 7.1 Metrics

| 指标族 | 有界 labels | 用途 |
| :--- | :--- | :--- |
| lock operations | operation=`test/acquire/release/reclaim`、mode、result | 成功、冲突、失败和恢复 |
| lock wait duration | mode、result | 阻塞锁等待分布 |
| lock state gauges | state=`granted/waiting/recovering` | Meta 当前协调压力 |
| kernel invalidation | reason=`remote_write/truncate/punch`、result | Watch 到 Linux 页缓存的收敛 |
| cached I/O bytes | direction=`read/write`、source/result | 判断 mmap/cached I/O 数据量 |

inode、path、pid、lock owner、operation id 都不能作为 label；它们只在采样 Trace 或失败日志中出现。

### 7.2 Trace 与日志

- 每个 FUSE lock 请求、Meta lock RPC、reclaim 批次和 kernel invalidation 建立 span。
- 不为每个用户态 page access 建 span；内核命中页缓存时 DMS 根本没有回调。
- 成功的每页 read/write 不输出日志。只记录 provider interrupt 分发失败、reclaim fencing、invalidator 失败、
  等待队列溢出和无法归类的协议错误。

## 8. 验收矩阵

### 8.1 M1.6a 文件锁

| Case | 必须证明 |
| :--- | :--- |
| 两 Node 重叠写锁 | 非阻塞请求 `EAGAIN`；阻塞请求在 unlock 后取得锁 |
| 共享读锁 | 多个读锁可共存；写锁被阻塞 |
| 局部 unlock | 区间正确拆分，`F_GETLK` 返回准确冲突范围 |
| `flock` + dup fd | 最后一个共享 file description 释放前锁不丢失 |
| 信号中断 `F_SETLKW` | waiter 取消且返回 `EINTR`，没有幽灵锁 |
| Node 崩溃 | lease/epoch 后锁释放，远端 waiter 唤醒 |
| Meta 重启 | recovery grace 内不错误授予冲突锁；Node reclaim 后保持原冲突关系 |

### 8.2 M1.6b mmap

| Case | 必须证明 |
| :--- | :--- |
| `MAP_SHARED` + `msync` | 另一 Node 读取到新版本，错误能从 msync/fsync 返回 |
| 远端写覆盖已映射页 | Watch ACK 后下一次访问不再读取旧 kernel page |
| `MAP_PRIVATE` 写 | 其它进程/Node 不可见，不产生 filesystem commit |
| truncate shrink | 新 EOF 外映射访问得到 `SIGBUS` |
| punch hole | 映射范围失效后读零，不重新分配零 Block |
| unlink-open-mmap | 名字消失但映射可继续使用；最后 unmap/release 后才回收 |
| Meta/Node 重启 | 不把旧 kernel page 或旧 binding 当成当前版本 |

验收入口：

- `scripts/validation/filesystem_mmap_helper.c`：只处理 C 层必须证明的事情，包括 `SIGBUS`、`msync`
  和映射页在远端覆盖后的可见性。
- `scripts/validation/filesystem_mmap_workload.py`：编排 POSIX 操作、抓取 Node/Meta metrics，输出
  `mmap-workload.json` 和 `mmap-recovery.json`。
- `scripts/validation/run_filesystem_mmap_e2e.sh`：单 VM 双 Node + 一个 Meta 的真实 FUSE 验收。
- `scripts/validation/run_filesystem_mmap_3vm.py`：A/B/C 三 VM 独立进程验收。
- `scripts/validation/evaluate_filesystem_mmap.py`：检查所有 mmap case、cached page hit 的 FUSE read
  放大、恢复证据和远端失效 ACK 顺序。远端失效不能只输出描述字符串，必须同时满足
  `writer_fsync_returned_before_mapped_visibility=true`、
  `dms_node_filesystem_kernel_invalidations_total{result="ok"}` 增量大于 0，以及
  `dms_meta_watch_events_total{event_type="filesystem_invalidation",result="delivered"}` 增量大于 0。

两个切片都必须提供单 VM 双 Node 和三 VM 独立 Meta 的真实 FUSE 证据，并重新核算 FUSE callback、
Meta RPC、Peer RPC、Watch ACK 和业务字节。cached page 命中不经过 DMS，报告中必须单独标明，不能把
“DMS 没收到 read”误写成漏采样。

当前 M1.6b 证据：

- 单 VM：`evidence/2026-09-16-filesystem-mmap-152340/result.txt` 为 `PASS`，
  cached page hit 的 `node_b_fuse_read_delta=0.0`。
- 三 VM：`evidence/2026-09-16-filesystem-mmap-three-vm-1527/result.txt` 为 `PASS`，
  远端覆盖后 `node_b_kernel_invalidation_ok_delta=1.0` 且 `node_b_fuse_read_delta=1.0`。

## 9. 实现顺序

1. M1.6a 已通过 vendored `fuser` patch 补齐 interrupt callback，并完成领域类型、Meta 锁表、RPC、Node
   镜像和真实双 Node 锁用例；后续升级 `fuser` 时必须先满足 4.6 的移除条件。
2. M1.6b 已移除 regular file direct I/O，接入 kernel invalidator 与 mmap 用例，并完成单 VM/三 VM
   验证。
3. 下一步进入 M1.7 总验收剩余 planned cases；不要再把 cached mmap 当作待接线能力。

## 10. 本轮建议确认的五项决策

1. 接受 M1.6a/M1.6b 分阶段实现，不建立共同的 `LockMmapManager`。
2. 接受 FUSE 定义的 flock-as-whole-file-range-lock 合同；暂不为严格分离两套锁族扩大 provider 补丁。
3. 接受为阻塞锁的 `EINTR` 在 FUSE provider 边界维护最小 interrupt 补丁。
4. 接受 mmap 使用 cached write-through，不启用 `FUSE_WRITEBACK_CACHE` 或 direct-I/O mmap，也不创建
   DMS 私有页缓存。
5. 接受远端 filesystem invalidation 只有在 Node cache 与 Linux kernel cache 都失效后才 ACK；Meta
   锁恢复使用既有 session/lease 加有界 reclaim grace，而不是持久化每次锁 mutation。

## 11. 官方语义依据

- [Linux FUSE I/O modes](https://kernel.org/doc/html/latest/filesystems/fuse/fuse-io.html)：direct I/O、cached write-through 与 writeback-cache 的差异。
- [Linux fcntl locking](https://www.man7.org/linux/man-pages/man2/fcntl_locking.2.html)：byte-range lock 与阻塞/非阻塞语义。
- [Linux flock](https://www.man7.org/linux/man-pages/man2/flock.2.html)：whole-file advisory lock 与 open file description 生命周期。
- [libfuse capability definitions](https://github.com/libfuse/libfuse/blob/master/include/fuse_common.h)：`FUSE_CAP_FLOCK_LOCKS` 和 direct-I/O mmap capability 的官方边界。

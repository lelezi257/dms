# DMS Native Filesystem 属性、权限与容量查询设计

> 状态（2026-09-16）：M1.4 已完整实现并通过真实双 Node FUSE E2E。
> 已完成 create uid/gid/mode、跨 Node getattr、chmod/chown/utimens、Meta 授权、
> `user.*` xattr、POSIX access/default ACL、集群 statfs、WAL/checkpoint 恢复、
> BindingCache revoke，以及 `size + attrs` 单次原子提交。

## 1. 本阶段要解决的用户问题

M1.4 不再增加新的文件身份或内容模型，而是在已经稳定的 inode 上补齐三类能力：

1. 用户修改 `/work/a.txt` 的 mode、owner 或时间后，所有 Node 必须看到同一份属性；Meta 重启后不能回退。
2. 无权访问或修改文件的进程必须得到稳定的 `EACCES/EPERM`，不能因为换了挂载 Node 而改变结果。
3. `df`、`statvfs` 等容量查询必须反映集群当前可用的真实 Host-memory 资源，不能返回某个 Node 的局部容量或硬编码数字。

用一条具体流程表示：

```text
Node A: create("/work/a.txt", 0644, uid=1000, gid=1000)
Node B: getattr("/work/a.txt")             -> 0644 / 1000 / 1000
Node A: chmod("/work/a.txt", 0600)
Node B: 下一次 getattr/read                -> 0600；旧授权已失效
Node B: uid=2000 open(O_RDONLY)             -> EACCES
Meta 重启
Node B: getattr("/work/a.txt")             -> 仍为 0600
任一 Node: statfs(mountpoint)               -> 相同的集群容量快照
```

本阶段的核心原则是：**inode 属性仍属于 Meta 的同一个权威 inode；权限判断消费这些属性；容量查询消费 Node 已有 Arena 的真实统计。** 不新增 path 属性表、第二个 Attribute owner 或第二套空间分配器。

## 2. 一层模块与状态归属

```text
Linux Kernel
  │ FUSE getattr/setattr/getxattr/setxattr/statfs
  ▼
FUSE adapter
  │ 只做 Request uid/gid/pid、TimeOrNow、xattr flags、errno 转换
  ▼
SharedFileOperations
  │ 进程内文件语义；不经过 KV SDK/WorkerService
  ▼
FilesystemMetaClient ───────────────► FilesystemMetadataService
                                           │
                                           ▼
                                      MetaState
                                      ├─ FilesystemCatalog
                                      │  ├─ inode attrs
                                      │  └─ inode xattrs / ACL
                                      ├─ operation idempotency
                                      ├─ grant generation / revoke
                                      └─ live Node ResourceSummary

NodeState
├─ BindingCache：现有 inode snapshot + exact content version
├─ ArenaManager：真实 capacity/allocated/staging/replica 统计
└─ heartbeat：把资源摘要上报 Meta
```

| 模块 | 本阶段职责 | 明确不负责 |
| :--- | :--- | :--- |
| FUSE adapter | 把内核调用转换为领域请求；启用内核 `default_permissions` 并在 FUSE INIT 协商 `FUSE_POSIX_ACL`；映射 errno | 不保存权威属性，不自行修改 mode bits |
| SharedFileOperations | 合并一次 `setattr` 的 size 与属性 patch；处理 CAS 重试和本地缓存回填 | 不复制 Meta 权限策略，不创建 AttributeCache |
| NodeState / BindingCache | 在既有 grant 内缓存 inode attrs；revoke 时先删再 ACK | 不持久化 attrs，不缓存全部 xattr |
| ArenaManager | 提供真实物理容量、已分配、staging、replica 字节 | 不推导文件逻辑 size，不计算全局容量 |
| MetaState / FilesystemCatalog | inode attrs、xattrs/ACL、inode 数量和变更原子性的唯一 owner | 不保存 payload bytes，不管理 Region/Slot |

### 2.1 为什么不新增 AttributeCache

`ResolvedInode` 已经包含 `FilesystemInodeSnapshot`，其中已有 mode、uid、gid、size 和三个时间字段。热 `getattr/read` 使用现有 BindingCache 即可；属性 mutation 增加 inode revision 和 grant generation，复用已经实现的 Watch revoke。新增一份 AttributeCache 只会产生两份失效状态，没有新的业务价值。

### 2.2 为什么 xattr 不放进每个 InodeSnapshot 响应

普通属性是每次 lookup/getattr 都要使用的小型固定字段；xattr 是按名称访问、大小可变的数据。把全部 xattr 塞进 `ResolvedInode` 会放大所有普通读的 Meta 响应。首版因此把 xattr 持久状态放在同一 `FilesystemCatalog`，但只在 get/list xattr 时按需返回；Node 首版不建立长期 xattr cache。

## 3. 属性更新与权限的 E2E 流程

### 3.1 chmod/chown/utimens

```sequence
participant U as 用户进程
participant F as FUSE adapter
participant N as SharedFileOperations
participant M as MetaState
participant B as Node B cache
U ->> F: chmod("a.txt", 0600)
F ->> N: set_attributes(inode, caller, mode=0600)
N ->> M: SetFilesystemAttributes(expected inode revision)
M ->> M: 校验 caller；生成 ctime；append 一条 WAL
M ->> B: revoke(inode, old grant generation)
B ->> B: 删除 BindingCache entry
B -->> M: ACK
M -->> N: 新 InodeSnapshot + 新 grant
N -->> F: 回填本地 cache
F -->> U: 成功
```

一次 mutation 必须同时完成：权限校验、属性 patch、inode revision、grant generation、幂等结果、失效事件和 WAL。不能先改 mode 再单独更新 ctime，也不能由 Node A 成功后异步补 Meta。

### 3.2 size 与属性同时出现的 setattr

Linux 一次 `setattr` 可能同时携带 `size`、`mode` 和 `mtime`。不能把它拆成 `truncate` 与 `chmod` 两次可见提交，否则其他 Node 可能看到中间状态。

```text
setattr(size=4096, mode=0600, mtime=NOW)
  ├─ DataCore prepare 新 VersionLayout
  └─ CommitFilesystemVersion(
       new_size=4096,
       attribute_patch={mode=0600, mtime=NOW},
       caller=...
     )

Meta 一条 WAL 原子发布：
ObjectVersion + inode exact binding + size + mode + mtime + ctime + revoke
```

因此代码骨架在既有 `FilesystemCommitVersionRequest` 上增加可选 caller 与 attribute patch，而不是新增一个“先 commit 内容、再 set attrs”的二阶段接口。

### 3.3 权限检查分两层，但规则只有一份权威输入

1. FUSE 挂载启用 `default_permissions`，Linux 内核用返回的 mode/uid/gid 做普通 open/read/write/execute 检查。这避免每次数据读写都额外访问 Meta。
2. FUSE INIT 必须协商 `FUSE_POSIX_ACL`。否则 ACL xattr 虽然可以保存，命名用户/组 ACL 却不会成为内核权限判定的一部分；内核不支持该 capability 时挂载直接失败，不能降级成“看似支持”。
2. chmod/chown/utimens/xattr 等 mutation 到达 Meta 时再次依据 caller 与权威 inode attrs 校验，防止进程内调用者或未来 Native API 绕过内核。

首版身份模型使用 `uid/gid/pid`。普通 chmod 只允许 root 或 owner；chown 只允许 root；非 root 的 chgrp 在没有可信 supplementary groups 前保守返回 `EPERM`。后续若引入可信身份服务，再扩展 group/capability，不把字符串 principal 当 POSIX 权限。

这里选择的是“内核检查普通访问、Meta 检查权威 mutation”的组合，不是两套互相独立的权限系统：

- 若所有 open/read/write 都访问 Meta，语义容易集中，但会破坏热路径 0 Meta RPC 的既有性能合同。
- 若只依赖 `default_permissions`，FUSE 路径可以工作，但进程内 Filesystem API、未来 Native API 或管理入口可能绕过内核。
- 若在每个 Node 复制一套完整权限策略，又会引入规则漂移和旧属性判断。

因此 mode/uid/gid/ACL 的权威状态与 mutation 授权只在 Meta；内核只消费已授权缓存中的属性，加速普通数据访问。`uid/gid/pid` 还不足以证明调用者属于哪些补充组，所以首版不能猜测非 root `chgrp` 权限，必须保守失败；该限制需要在补充组或 capability 身份合同接通后才能放开。

### 3.4 首版共享根目录为什么是 01777

启用 `default_permissions` 后，原来的 `0755 root:root` 根 inode 会让普通挂载用户无法在顶层创建任何文件。当前还没有一等 Workspace root，也没有可持久配置的租户 owner/mode，因此首版共享 namespace 使用与 `/tmp` 相同的 `01777` 合同：所有用户可以创建自己的顶层入口，sticky bit 阻止普通用户删除或替换其他 owner 的入口。

这只是共享根目录的过渡合同，不代表 DMS 已经提供 tenant 隔离。未来引入 Workspace/tenant root 时，owner、group 与 mode 必须成为持久配置；届时不能继续用一个全局 `01777` 根目录代替隔离、安全策略或 quota。Meta mutation authorization 仍必须保留，用于防止进程内或未来 Native API 绕过内核。

## 4. 时间语义

| 操作 | atime | mtime | ctime |
| :--- | :--- | :--- | :--- |
| create/mkdir/symlink | Meta commit time | Meta commit time | Meta commit time |
| write/truncate | 不变 | 请求指定的精确时间或 Meta commit time | Meta commit time |
| chmod/chown | 不变 | 不变 | Meta commit time |
| utimens | `OMIT/NOW/EXACT` | `OMIT/NOW/EXACT` | Meta commit time |
| link/unlink/rename | 文件本身不变 | 文件本身不变 | 受影响 inode 更新；父目录 mtime/ctime 更新 |

`NOW` 由 Meta 在权威 actor turn 内解析；`EXACT` 由请求携带纳秒值；`OMIT` 不改变字段。caller 不能直接设置 ctime。

读 atime 策略是挂载策略，不应让每次热读都同步写 Meta。M1.4 首版默认 `noatime`，但显式 `utimens` 完整生效；`relatime/strictatime` 留在同一配置入口，不在普通 read 主链偷偷增加 Meta RPC。

该默认值的取舍是：`strictatime` 会把普通 read 变成 Meta WAL、revision、revoke 与缓存回填；`relatime` 只降低频率，仍会周期性把读转换成写；`noatime` 则让普通读保持纯读路径。首版优先热路径性能，同时保留应用显式设置时间的 POSIX 能力。未来开放 `relatime/strictatime` 时必须作为明确挂载配置，并单独验收写放大，不能改变默认行为。

## 5. xattr 与 ACL

### 5.1 支持矩阵

| namespace | M1.4 目标 | 行为 |
| :--- | :--- | :--- |
| `user.*` | 支持 get/list/set/remove | 数据随 inode 在 Meta WAL/checkpoint 持久化 |
| `system.posix_acl_access` | 支持 | 校验 ACL 编码；同时原子同步 mode 权限位 |
| `system.posix_acl_default` | 仅目录支持 | 新子项继承；普通文件设置返回 `EACCES/EINVAL` |
| `security.*`、`trusted.*` | 首版不支持 | 明确 `EOPNOTSUPP`，不静默保存 |

xattr 仍属于 inode 元数据，不进入 DataCore Block。单值、单 inode 总量和条目数必须由 Meta 配置限制；超限返回 `ENOSPC/E2BIG`，不能让不受控 xattr 挤占 Meta 内存和 WAL。

### 5.2 ACL 不是第二套权限 owner

ACL 作为两个受约束的 system xattr 保存，但它会影响 mode 的 group class bits。Set ACL 必须在同一 Meta mutation 中同时更新 xattr、mode、ctime、inode revision 和 revoke。创建子项时，Meta 在同一个 create WAL 中完成 default ACL 继承，Node 不能创建后再补 ACL。

首版不建立 Node 长期 xattr cache，是因为 xattr 按名称访问、长度可变且远低于普通 getattr/read 的频率。把它塞进 BindingCache 会放大每个 inode snapshot；另建 cache 则必须再设计 revision、容量预算、revoke 和重启恢复。M1.4 先按需访问 Meta，并用低基数 Metrics 观察调用频率和延迟；只有真实 workload 证明它进入热路径后，才在不新增权威 owner 的前提下增加有界缓存。

## 6. statfs：容量从哪里来

### 6.1 真实数据源

Node 已经有唯一的 `ArenaManager` owner，Meta heartbeat 协议已经预留 `ResourceSummary`。M1.4 接通而不另造容量采集协议：

```text
ArenaManager::resource_summary()
  ├─ capacity_bytes
  ├─ allocated_bytes
  ├─ staging_bytes（需补充）
  └─ replica_bytes（需补充）
        │
        ▼
NodeHeartbeat.ResourceSummary
        │
        ▼
Meta live NodeSession resources
        │ 聚合当前 lease 内的 Node
        ▼
StatFilesystemResponse
```

`total/free/available blocks` 是当前 live Node 的物理容量汇总；它不是所有文件 size 之和。副本会真实占用多个 Node 的物理空间，因此自然计入多次。失去租约的 Node 不参与可用容量。这里的“资源报告未齐”有精确定义：某个 NodeSession 在当前 incarnation 已经被 Meta 判定为 live，但尚未发送该 incarnation 的第一份 `ResourceSummary`。此时 `statfs` 返回 `EAGAIN/Unavailable`，不能返回 0，也不能把缺失 Node 静默排除后冒充完整集群视图。

M1.4 仍是当前单副本容量语义，`f_bavail` 可以直接由 live Node 的物理可用空间汇总。M2 引入多副本策略后，用户还能写入多少逻辑数据取决于目标副本数、放置约束和故障域，必须按当时的副本策略重新计算 `f_bavail`；不能把原始物理 free 简单当作可写逻辑容量。

inode 容量来自 Meta 显式配置 `filesystem_max_inodes`；已用数量来自 durable catalog。没有配置或恢复尚未完成时不能编造 `f_files/f_ffree`。block size 固定为 4096 只是 `statfs` 的计量单位，不改变 Arena 的真实 Slot 对齐和 Region 大小。

### 6.2 为什么查询必须到 Meta

某个 Node 只知道自己的 Arena。若 Node A 直接用本地 stats 回复，两个挂载点会得到不同 `df` 结果，也无法在 Node B 故障后排除其容量。`statfs` 是低频控制查询，访问 Meta 汇总是必要成本；普通 read/write 不因此增加 RPC。

## 7. 接口骨架

以下是本阶段冻结的业务形状；protobuf 只是 Node↔Meta DTO，FUSE 与 Filesystem 领域代码不直接依赖生成类型。

### 7.1 属性 mutation

```rust
struct FilesystemCallerIdentity {
    uid: u32,
    gid: u32,
    pid: u32,
}

enum TimeUpdate {
    Omit,
    Now,
    Exact(i64),
}

struct AttributePatch {
    mode: Option<u32>,
    uid: Option<u32>,
    gid: Option<u32>,
    atime: TimeUpdate,
    mtime: TimeUpdate,
}

async fn set_attributes(
    inode: InodeId,
    caller: FilesystemCallerIdentity,
    patch: AttributePatch,
) -> Result<ResolvedInode>;
```

接口不暴露 ctime setter。Node 内部携带 expected inode revision、operation id 与 commit sequence；普通 FUSE 用户不处理 CAS。

### 7.2 xattr

```rust
async fn get_xattr(inode, name, caller) -> Result<Option<Vec<u8>>>;
async fn list_xattrs(inode, caller) -> Result<Vec<Vec<u8>>>;
async fn set_xattr(inode, name, value, mode, caller) -> Result<ResolvedInode>;
async fn remove_xattr(inode, name, caller) -> Result<ResolvedInode>;
```

`mode` 是 Upsert/CreateOnly/ReplaceOnly，对应 Linux xattr flags。Mutation 返回新的 inode snapshot，便于本地 cache 精确回填；读接口首版不承诺 Node xattr cache。

### 7.3 容量

```rust
struct FilesystemStats {
    block_size: u64,
    total_blocks: u64,
    free_blocks: u64,
    available_blocks: u64,
    total_inodes: u64,
    free_inodes: u64,
    max_name_length: u32,
    reporting_nodes: u32,
    capacity_revision: u64,
}

async fn stat_filesystem() -> Result<FilesystemStats>;
```

`reporting_nodes/capacity_revision` 供诊断和测试使用；FUSE `ReplyStatfs` 只选择 POSIX 字段返回。

## 8. 错误与 errno

| 条件 | 领域错误 | FUSE errno |
| :--- | :--- | :--- |
| caller 不是 owner/root，尝试 chmod/chown | PermissionDenied | `EPERM` |
| mode/ACL 拒绝普通访问 | PermissionDenied | `EACCES` |
| inode 不存在 | NotFound | `ENOENT` |
| xattr 不存在 | XattrNotFound | `ENODATA` |
| xattr CreateOnly 已存在 | AlreadyExists | `EEXIST` |
| xattr ReplaceOnly 不存在 | XattrNotFound | `ENODATA` |
| 不支持的 xattr namespace | Unimplemented | `EOPNOTSUPP` |
| xattr/容量超过配置 | ResourceExhausted | `ENOSPC` 或 `E2BIG` |
| inode revision 冲突 | Aborted | Node 有界重读重试；耗尽后 `EAGAIN` |
| statfs 资源报告不完整 | Unavailable | `EAGAIN` |

错误码在 Meta 根因处确定；Node 只做稳定 errno 映射，不根据 message 字符串猜测。

## 9. 可观察性与性能预算

新增指标保持低基数：

- Filesystem operation：`setattr/getxattr/setxattr/listxattr/removexattr/statfs`。
- 权限拒绝按固定 reason 枚举计数，不带 uid/path/inode 标签。
- heartbeat 资源 Gauge：总容量、可用、staging、replica；Meta 记录 reporting/live Node 数。
- Trace 只覆盖控制调用和 mutation/WAL/revoke；不把 xattr value、path 或 ACL 内容写入 span/log。

性能不变量：

1. 普通热 read/getattr 不新增 Meta RPC。
2. 单个纯属性 mutation 是 1 次 Node→Meta unary，加既有 revoke/ACK 屏障；不能拆成多个同步提交。
3. size+attrs 是一次 filesystem commit，不额外再发 SetAttributes。
4. statfs 是低频查询，不进入每次 open/read/write。
5. 成功 heartbeat 默认不产周期 Trace，只更新 Metrics 和 Meta resource snapshot。

## 10. 源码目录与修改落点

```text
protocol/proto/dms/v1/filesystem_meta.proto
  属性、xattr、statfs 的 Node↔Meta DTO 与 RPC

server/src/filesystem/model.rs
  CallerIdentity / AttributePatch / TimeUpdate / FilesystemStats 领域值

server/src/node/filesystem/fuse.rs
  FUSE callback、default_permissions、TimeOrNow/xattr flags/errno 转换
server/src/node/filesystem/shared.rs
  setattr 合并、CAS 重试、cache 回填
server/src/node/filesystem/meta_client.rs
  原生领域值与 protobuf DTO 转换
server/src/node/runtime.rs
  从唯一 Arena owner 读取 ResourceSummary；现有 BindingCache 失效

server/src/meta/filesystem/mod.rs
  inode attrs、xattr/ACL、inode count 的唯一 catalog
server/src/meta/runtime.rs
  授权、幂等、WAL、revoke、live resource 聚合
server/src/meta/metadata_journal.rs
server/src/meta/local_wal_journal.rs
  属性/xattr mutation 的稳定记录与 snapshot 恢复
```

不会新增 `attribute_service/`、`capacity_manager/` 或第二个 Filesystem actor。Meta 的业务接口仍由独立 `FilesystemMetadataService` 提供，既有对象 MetadataService 和公开 KV SDK 不变。

## 11. 实现顺序与验收

### 11.1 完整纵向切片（已完成）

```text
create(uid/gid/mode)
→ Node B getattr
→ Node A chmod/chown/utimens
→ Node B 第一次 getattr 已是新值
→ user xattr create/replace/remove 与跨 Node 可见
→ access ACL 同步 mode；default ACL 随子项 create 原子继承
→ 两个挂载得到相同的集群 statfs
→ 重启 Meta/Node B 后仍恢复
```

这条切片已经证明属性 authority、WAL、grant revoke、cache 回填和 errno 主链。真实验证入口是：

```text
scripts/validation/run_filesystem_attributes_e2e.sh
scripts/validation/run_filesystem_attributes_3vm.py --profile <三台 VM 的 JSON 配置>
```

验证会启动一个 Meta、两个 Node 和两个 FUSE mount；Node B 先缓存旧属性，Node A
再执行 chmod/utimens，随后检查 Node B 第一次 getattr 已得到新属性；接着验证
`user.*` xattr 的 flags/删除语义、命名用户 ACL/mode 联动、default ACL 继承，以及两个挂载
获得同一份 512 MiB/10000 inode 的集群容量视图。最后重启 Meta 与 Node B，确认
inode 属性、xattr、ACL 和 statfs 从 WAL 与重新上报的资源快照恢复。

chown/root/非 owner 授权、资源报告缺失返回 EAGAIN、xattr 限额，以及 `size + attrs`
单条 journal 由 Meta 状态机回归覆盖；脚本不要求
运行者拥有 root 权限，因此不会伪造一个依赖 sudo 的 chown E2E。

三 VM 入口把 Meta、Node A、Node B 分别部署到三台独立 VM，验证跨机第一次读取、
属性/xattr/ACL 可见性、两 Node 容量聚合，以及 Meta 与 Node B 重启后的恢复。它复用
同一套业务断言，但不把单机进程隔离误当成分布式部署证据。

### 11.2 已完成切片

1. 属性：create/getattr/chmod/chown/utimens、跨 Node revoke 与重启恢复。
2. `user.*` xattr：get/list/set/remove、CreateOnly/ReplaceOnly 和重启恢复。
3. POSIX ACL：access/default、mode 同步和子项继承。
4. 容量：Node heartbeat 真实资源上报、Meta 聚合和 statfs。
5. 原子性：size+attrs 同一次 setattr 只产生一次 filesystem version commit。

### 11.3 必须通过的矩阵

| 类别 | 关键用例 |
| :--- | :--- |
| 单 Node | chmod/chown/utimens、xattr flags、ACL/mode 同步、statfs |
| 三 VM（两个 Node + 独立 Meta） | 属性 mutation 返回后，另一挂载第一次读取即为新值；statfs 结果一致 |
| 故障 | WAL append 失败不 apply；Watch 断线由 lease 收敛；Meta/Node 重启恢复；容量 heartbeat 缺失不伪造 |
| 权限 | owner/非 owner/root、文件/目录 execute、unsupported xattr namespace |
| 性能 | 热 read/getattr 请求放大不变；纯属性 mutation 只有一次业务 unary；周期 heartbeat 不生成成功 Trace |

## 12. 已确认决策与当前边界

本轮 Review 已确认以下合同：

1. 普通 FUSE 访问使用 `default_permissions`，权威 mutation 仍由 Meta 基于 caller 与最新 inode attrs 授权；补充组未接通前，非 root `chgrp` 保守返回 `EPERM`。
2. 首版默认 `noatime`，显式 `utimens` 完整生效；`relatime/strictatime` 只作为未来显式配置，不进入默认热读路径。
3. xattr 首版按需访问 Meta，不建立长期 Node cache；ACL 是受约束的 system xattr，不成为第二套权限 owner。
4. `statfs` 由 Meta 聚合当前 live Node 的真实 Arena 资源，并使用配置的 inode 上限；当前 incarnation 尚未完成首份资源报告时返回 `EAGAIN`。多副本阶段必须按副本策略重新定义 `f_bavail`。
5. 纯属性 mutation 使用 `SetFilesystemAttributes`；只要一次 `setattr` 含 size/content 变化，就通过既有 `CommitFilesystemVersion.attribute_patch` 在一条 WAL 中原子发布内容版本与属性，禁止拆成两个提交。

M1.4 已完成并保持一套权威状态：xattr/ACL 与普通 inode 属性同属
`FilesystemCatalog`，容量来自 Node 唯一 `ArenaManager` owner 的 heartbeat 快照。
FUSE 只负责参数和 errno 转换；Node 业务层不保存第二份 xattr 或容量状态。

本轮真实证据分为两组：

- [单 VM、双 Node FUSE](../evidence/2026-09-16-filesystem-attributes/single-vm-final/)；
- [三 VM（两个 Node + 独立 Meta）](../evidence/2026-09-16-filesystem-attributes/three-vm-final-3/)。

`result.txt`、`attributes.json` 与 `recovery.json` 分别记录整体结论、重启前权威结果和
重启后逐字段比对结果；三 VM 证据还保存部署 profile 和各进程日志。它们共同证明当前
单副本容量语义与跨机双 Node 行为，不替代 M2 多副本阶段对 `f_bavail` 的重新定义。

# AFS 目录架构与模块职责

状态：Implementation Reference。本文定义当前源码目录、模块职责和依赖方向。产品语义见[架构原则](../PRINCIPLES.md)、[架构总览](architecture/overview.md)与[数据 Profile](architecture/profiles.md)。

`src/meta/store.rs` 定义 `MetaStore` 业务接口与 Store 提交队列；队列负责排队、短窗合并，并在后端 ACK 后发布权威状态。`src/meta/store/memory.rs`、`local_file.rs` 与 `etcd.rs` 是三个后端实现。`Meta::register_node/lookup_node` 承接节点业务，RPC 负责身份与 Proto 转换。Store 边界见[MetaStore 提交边界](plans/2026-09-27-meta-store.md)。

OwnerFs 的持久 Meta 根权威、本地普通文件、FUSE 文件操作、远端 P2P 与进程重启后重新打开恢复已经接通并完成 Linux 三节点复验。BlobFs 仍是骨架，OwnerFiles RDMA 内容路径尚未接通。Meta 根授权业务由 `meta/owner_roots.rs` 承载，持久权威由 `meta/store.rs` 与所选后端承载；`memory` 只供可丢弃的开发/测试，不提供 Meta 重启恢复。Node 文件业务由 `OwnerFsPeerExecutor` 注入 `node/rpc/data/owner.rs`。完整状态见[当前状态](current-status.md)。

以下目录树描述源码模块导航；实现能力以[当前状态](current-status.md)为准。

## 1. 完整目录

源码采用 `foo.rs + foo/` 布局：存在真实子模块时建立对应目录，小模块保留为单文件，不创建无内容的目录层级。

```text
afs/
├── Cargo.toml                       # workspace + afs 根 package
├── Cargo.lock
├── AGENTS.md
├── error-codes.toml                 # 稳定错误码目录，与 Rust 常量测试校验
├── PRINCIPLES.md
├── docs/code-layout.md              # 本目录合同
├── common/                          # 每个子目录是独立 crate
│   ├── logging/                     # 已有：进程日志设施
│   ├── metrics/                     # 已有：指标注册和导出设施
│   ├── tracing/                     # 已有：上下文传播和可选 Trace runtime
│   ├── error/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs               # code + kind + message，协议无关
│   │       ├── code.rs              # 稳定编号与目录
│   │       └── kind.rs              # 通用处理分类
│   ├── protocol/
│   │   ├── Cargo.toml
│   │   ├── build.rs                 # 生成 local/meta/node_control/node_data 协议代码
│   │   ├── proto/
│   │   │   ├── error.proto          # AFS ErrorDetail
│   │   │   ├── local_api.proto
│   │   │   ├── meta.proto          # Meta、OwnerRoots、BlobMeta 三个 service
│   │   │   ├── node_control.proto
│   │   │   └── node_data.proto     # DataTransfer、诊断与 OwnerFiles；未来 Blob 独立 service
│   │   └── src/lib.rs               # 导出生成协议代码
│   └── transport/
│       ├── Cargo.toml
│       ├── build.rs                 # 可选 RDMA native shim 构建
│       ├── native/
│       │   ├── rdma.c
│       │   └── rdma.h
│       └── src/
│           ├── lib.rs
│           ├── error.rs             # 保留已有传输配置错误
│           ├── grpc.rs
│           ├── grpc/
│           │   ├── error_status.rs  # google.rpc.Status / Any 错误编解码
│           │   ├── config.rs        # 已有：配置 Tonic builder
│           │   └── security.rs      # 已有：TLS/证书
│           ├── rdma.rs              # 可选 RDMA 单边搬运 adapter 与 native shim
│           └── shm.rs               # sealed memfd、FD passing 与有界字节拷贝
├── client/                          # 本机高性能 SDK 独立 crate
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── connection.rs            # 只连接本机 Node 的 UDS caller
│       └── buffer.rs                # SDK 侧 SHM 缓冲区使用
├── src/
│   ├── lib.rs
│   ├── error.rs                     # 进程 REST/errno 边界映射
│   ├── config.rs                    # CLI/TOML 配置与 feature/runtime 选择
│   ├── runtime.rs                   # 观测初始化、服务生命周期与关闭
│   ├── bin/
│   │   ├── afs-meta.rs              # 进程组装入口
│   │   └── afs-node.rs
│   ├── meta.rs
│   ├── meta/
│   │   ├── rpc.rs                   # Node → Meta Handler
│   │   ├── owner_roots.rs           # OwnerFs 根授权事务与状态机
│   │   ├── store.rs                 # MetaStore 接口、Store 提交队列与公共类型
│   │   ├── store/
│   │   │   ├── memory.rs            # 易失测试后端
│   │   │   ├── local_file.rs        # 本机 WAL/快照后端
│   │   │   └── etcd.rs              # 默认 etcd CAS 快照后端
│   │   └── rest.rs                  # 管理面 REST
│   ├── node.rs
│   └── node/
│       ├── fuse.rs                  # 统一 POSIX/FUSE 入口
│       ├── fuse/state.rs            # FUSE inode/打开句柄映射
│       ├── api.rs
│       ├── api/
│       │   ├── local.rs             # SDK 服务端：UDS gRPC + SHM
│       │   └── rest.rs              # runtime 等使用的 Node REST
│       ├── rpc.rs
│       ├── rpc/
│       │   ├── control.rs           # Node 控制 Handler
│       │   ├── data.rs              # 诊断读写 Handler
│       │   ├── data/owner.rs        # OwnerFiles 远端文件 Handler
│       │   ├── peer.rs              # 远端数据 caller 和双 adapter
│       │   └── meta.rs              # 调用 Meta
│       ├── vfs.rs                   # 统一接口与 namespace 分派
│       ├── vfs/
│       │   ├── types.rs             # 与 FUSE 解耦的文件属性、inode/句柄、请求上下文
│       │   ├── ownerfs.rs           # 可修改、根归属与本地性优先
│       │   ├── ownerfs/root.rs      # OwnerFs 私有根身份、授权缓存及生命周期接口
│       │   ├── ownerfs/catalog.rs   # 本机根身份记录与启动独占锁合同
│       │   ├── ownerfs/files.rs     # 本地/远端文件身份与打开句柄类型
│       │   ├── ownerfs/remote.rs    # 远端文件操作业务接口
│       │   └── blobfs.rs            # 通用分布式主干骨架；Mutable / Immutable Profiles
│       ├── storage.rs               # FileStore/FileHandle 与旧诊断对象
│       └── storage/localfs.rs       # 本地普通文件后端
└── tests/README.md                  # 未来跨模块集成/E2E 归属
```

Meta 根归属、节点管理、授权及 MetaStore 子模块按上方当前补记阅读。同一个 afs-meta gRPC server 注册 Meta、OwnerRoots、BlobMeta 三个 service；OwnerFs 权威 RPC 已接真实业务，BlobMeta 仍按后续 BlobFs 设计推进。Meta wire 合同不等于镜像版本已可用。

OwnerFs [全量实施前审视](plans/2026-09-26-ownerfs-readiness.md)确定的 `src/meta/store.rs` 核心接口已落地，并继续由 Store 统一提供条件事务、幂等结果、revision/watch 和恢复边界。`node/fuse/state.rs`（进程内 inode/句柄表）与 `node/rpc/peer/owner.rs`（OwnerFiles 客户端转换）若拆出只是已有模块的实现文件，不增加新业务层。`RootMode` 与 B 加入前 ACK 已从合同移除。

协议文件按进程/传输边界分：`meta.proto` 在 `afs.meta.v1` 下用注释分段，并保留 Meta、OwnerRoots、BlobMeta 三个独立 service；`node_control.proto` 管节点间传输控制，`node_data.proto` 管内容传输与文件命令。Rust 的生成模块分别是 `afs_protocol::meta`、`node_control`、`node_data`，业务 handler 仍按 service 分开。OwnerRoots/BlobMeta 之前尚未投入使用的 `afs.meta.owner.v1`/`afs.meta.blob.v1` 路径随合并改为 `afs.meta.v1`；后续客户端应按新路径生成，不能把这次整理当作已发布协议的兼容升级。

## 2. 五条接入关系

| 关系 | 调用方 | 接收入口 | 公共部分 |
|---|---|---|---|
| SDK → 本机 Node | client/connection、buffer | node/api/local | local_api Proto、gRPC config/security、SHM |
| Node → Meta | node/rpc/meta | meta/rpc | meta Proto、gRPC config/security |
| Node → Node 控制 | Node 调用业务 | node/rpc/control | node_control Proto、gRPC config/security |
| Node → Node 数据 | node/rpc/peer（当前仅诊断） | node/rpc/data、data/owner | node_data Proto 的独立 service；gRPC 或单边 RDMA |
| runtime → Node REST | 普通 HTTP 调用者 | node/api/rest | 按需共用观测设施 |
| 管理面 → Meta REST | 普通 HTTP 调用者 | meta/rest | 按需共用观测设施 |

表中将第三条 Node→Node 拆成控制/数据。SDK 只连接本机，远端访问由 Node 发起；SDK 使用 UDS gRPC 传控制信息，内容通过 sealed memfd + SCM_RIGHTS 交给 Node 读写，没有 gRPC 内容 fallback。没有 SHM 的使用者走 POSIX。不提供管理面 SDK。

## 3. 数据面与公共机制

```text
Node 文件业务 → 未来 Owner/Blob 专属 peer caller
              ├─ gRPC：命令与内容进入 gRPC
              └─ RDMA：gRPC 命令/结果 + 单边搬运内容
                       ↓
                对端 rpc/data/owner → OwnerFs 文件业务
```

`node_data.proto` 是 Node→Node 数据面唯一 Proto：共用 `DataTransfer` 选择内容传输，`NodeData` 只做 8 字节诊断，`OwnerFiles` 是真实远端文件/目录 service；未来 Blob 可在同文件定义自己的 service/消息。两种模式都使用 OwnerFiles gRPC 命令；gRPC inline 携带文件内容，RDMA 模式只由单边 READ/WRITE 搬运内容。CQ 成功不是文件业务成功。`afs.node.data.v1.OwnerFiles` 服务路径保持不变；本分支尚无可用文件业务客户端。OwnerFiles handler 独立放在 `rpc/data/owner.rs`，现阶段全部明确返回 `UNIMPLEMENTED`，不借诊断数据实现冒充 OwnerFs。

OwnerFs Case 2 只新增接口合同：`ownerfs/files.rs` 放同一后端的本地文件与远端 Home 令牌类型；`ownerfs/remote.rs` 接收当前根授权并定义 lookup/open/read/write/flush/fsync/release，后续应由 Node→Node 客户端适配器实现。服务端代码从诊断处理器分到 `rpc/data/owner.rs`。Meta 可同时授权 A 与 B；B 经 P2P 访问 A 的同一份普通文件，加入时不撤销 A、不做模式切换或 ACK。`root.rs` 已定义 A 首次校验 B 授权及缓存准入，远端业务尚未接通。

OwnerFs Case 3 的重启合同：`ownerfs/catalog.rs` 负责本机根身份记录与贯穿 Node 生命周期的排他锁；`root.rs` 的 `RootMeta::recover_root` 用记录中的 root epoch、`local_prepare_id` 和新进程 session 向 Meta 对账。`meta.proto` 的 `RecoverRoot` 只有在持久 Home 事实、当前注册会话和旧会话围栏满足后才能返回新授权，不能扫描目录就直接准入。Meta 与 P2P 的 RootAccess 都携带 `home_session_id`；A 重启后旧授权和旧远端句柄失效，B 必须重新取授权、lookup/open。当前只有接口，尚无实际落盘记录、OS 锁、MetaStore 状态机或自动恢复。

RDMA 建连参考 [3FS IBConnect](https://github.com/deepseek-ai/3FS/blob/22fca04564c7cc230fd8b9523b8b92864e1dad47/src/common/net/ib/IBConnect.cc#L338) 的实际通道探测：服务端先投递接收槽，双方经一次 `NegotiateData` gRPC 交换参数，客户端配置好 QP 后发送 `SEND_WITH_IMM` 探测并等待本地发送完成。服务端在首条数据请求中消费探测的接收完成，然后才允许文件访问；后续请求复用 ready 状态。没有独立 `ReadyData` RPC，也不为闲置连接启动持续 CQ 轮询。Close/TTL 删除会话登记，拒绝新的会话查找；已取得会话的在途请求仍可完成，资源随最后一个持有者释放，不宣称文件取消或 drain 屏障。探测带固定立即数、独立 WR ID，并校验 CQ 状态和操作码；超时或错误禁用会话。握手版本为 1，新旧 RDMA 握手不兼容，需同时更新两端；gRPC inline 文件协议不变。3FS 的设备发现/多网卡选路和完整消息传输框架不在本次移植范围内。

纯 gRPC 数据请求只依赖对应的 `NodeData` 或 `OwnerFiles`，不调用会话协商。Node 仍注册 `NodeControl` 的 Ping 和会话接口；`data_mode` 是出站 adapter 选择，不是关闭入站控制服务的开关。

通道在尚未执行业务的建连阶段选择，结果不明的写不跨通道重放。MR/SHM 缓冲区所有权覆盖真实操作周期，取消等待不意味着可以复用内存。独立穿刺不等于生产资源池、会话回收器或硬件吞吐结果。

| 模块 | 负责 | 不负责 |
|---|---|---|
| logging/metrics/tracing | 观测设施、上下文、进程级初始化能力 | 把具体业务事件/指标统一堆到公共库；SDK 初始化全局设施 |
| error | 协议无关 ErrorCode/ErrorKind/Error/Result 与当前错误码目录 | 旧 KV 业务错误全集；errno/HTTP 映射归进程入口，Status 编解码归 transport/grpc |
| protocol | local/meta/node_control/node_data wire 定义与生成类型 | Handler、域状态机、建连 |
| transport/grpc | config/security 与 error_status 编解码 | 统一 client、重试或 actor；调用方直接用 Tonic 建连/监听/注册 |
| transport/rdma、shm | RDMA/native shim、sealed memfd、FD passing、资源生命周期 | 文件 API、根授权、业务成功或重试政策；SHM 当前是有界字节拷贝，不宣称零拷贝 |
| meta/rpc、rest | 请求校验、认证与协议映射；RPC 注入 OwnerRootAuthority | 在协议层复制根授权事务或引入 actor 邮箱 |
| meta/owner_roots | OwnerFs 根授权事务和状态机，共用 MetaStore | Proto/HTTP 参数编解码 |
| node/api | SDK/REST 入口，共用对应后端业务 | 复制发布状态机或吞并 Meta 管理面 |
| node/vfs | 共用接入形状、namespace 分派与身份映射边界 | 强迫两种后端共用持久化、缓存或恢复模型 |
| node/storage | 两种后端共用的本地 I/O 机制 | 决定何时 sync、如何发布/恢复、强制 Blob 化 |

FUSE 节点号、进程内后端 inode/句柄、OwnerFs 可恢复文件身份、Blob 版本身份分开。`vfs::Backend` 定义 lookup、getattr/setattr、create/open/read/write/flush/fsync/release、目录读写与同步、rename/link 等入口形状；未实现方法明确返回 `UNIMPLEMENTED`。`create_file` 仍是原有基础框架诊断探针，不产生文件身份或句柄，不能当作真实 create。真实文件回调接通时由 `Backend::create` 原子返回 `CreatedFile`，随后删除探针。Node 内部文件业务 caller 将留在 rpc/peer 下，不搬进 transport；现有 DataPeerClient 只服务诊断，不可冒充 OwnerFs。storage 先是 Node 内模块，不提前拆独立通用存储 crate。

`ownerfs/root.rs` 将根位置、创建预留、激活授权明确分型，定义 OwnerFs 私有的 RootMeta/RootLifecycle 合同。当前仅实现本机已激活授权的缓存准入、RootUse 在途计数、失效封闭和身份核对；首次 mkdir 的持久准备记录、MetaStore、FUSE 接线和重启对账尚未实现。根内热路径按 RootId 查缓存，不逐文件问 Meta；这些类型不提升为 VFS/BlobFs 公共抽象。

Workspace 底层数据目录仅由 AFS Node 修改；Agent 经挂载入口访问。普通 write/flush/close 不强制 fsync，显式 fsync/fdatasync 才是本地文件耐久边界；目录项的持久性另需目录同步。`FileStore` 仅抽象本地可变文件/目录能力，Blob 的不变对象存放另立业务合同，不能把 OBS 或数据库塞成假 POSIX 文件后端。

## 4. Cargo 与 feature

根 `afs` package 包含两个 binary 和 meta/node library 模块；SDK 是独立 `afs-client` crate，不依赖根 package。common 不反向依赖 Node、Meta、SDK。

transport 默认启用 `grpc`，`shm` 用于本机 SDK，`rdma` 是可选 feature。SDK 明确启用 `grpc + shm`，不启用 RDMA；根 package 默认使用 transport 的 `grpc + shm`，显式 `--features rdma` 增加 RDMA native shim 和单边 adapter。根 package 两个 binary 共用 feature 图；未来如拆分独立制品，再验证 feature 隔离需求。

Proto 已生成 local/meta/node_control/node_data 类型和基础 service。基础框架只提供 ping、诊断和 8 字节数据面，不放假的文件业务成功路径。SDK 的独立 crate 边界已建立，但 publish=false 保留到实际交付验收。

## 5. 从入口读代码

第一版的关键模块、接口和资源生命周期已补中文注释。建议按以下顺序阅读：

1. `src/bin/afs-node.rs` / `afs-meta.rs` → `src/config.rs` → `src/runtime.rs`：CLI/TOML 合并、编译与运行开关、观测和退出。
2. `src/node.rs` → `node/fuse.rs` → `node/vfs.rs` → `vfs/ownerfs.rs` / `blobfs.rs`：同进程入口和后端分派。OwnerFs 文件业务已接通；BlobFs 仍为骨架。
3. `node/rpc/peer.rs` → `node/rpc/data.rs` → `node/storage.rs`：共同客户端 API、gRPC/RDMA adapter、共同服务端 Handler，以及诊断字节实际写入。
4. `node/rpc/control.rs` → `common/transport/src/rdma.rs` → `common/transport/native/rdma.c`：会话协商、Rust 所有权、libibverbs 单边搬运。`native/` 是 C ABI 适配，不是 Native FS 后端；只在启用 `rdma` feature 时编译链接。
5. `client/src/connection.rs` → `client/src/buffer.rs` → `node/api/local.rs` → `common/transport/src/shm.rs`：UDS 控制、memfd/FD passing 内容和取消后的资源回收。当前是有界拷贝，不是零拷贝。

诊断数据链与 FUSE 分派链分别验基础框架；不能把两条链已通过理解为完整文件业务已接通。代码中的未来职责说明与当前实现边界已分别标明。

错误合同、未知码与混合版本规则见 [error-contract.md](error-contract.md)。

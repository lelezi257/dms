# AFS 目录架构与模块职责

状态：2026-09-25，落实控制台已确认的正式基础框架。本文负责目录、模块归属和依赖方向；需求与业务语义仍见 [需求分析](requirements.html)、[详细架构](architecture.html)与[宪法](../PRINCIPLES.md)。旧设计中的 Master/Worker 角色对应 `afs-meta/afs-node`，文件后端命名为 `OwnerFs/BlobFs`。

当前实现已经是可运行基础框架：`afs-meta` 与 `afs-node` 可启动，配置、观测、Meta/Node REST、Node→Meta gRPC、Node→Node control/data、Local SDK UDS+SHM、FUSE namespace 分派和 8 字节诊断链路已经接线。它仍不是完整文件系统：OwnerFs/BlobFs 业务状态机尚未实现，FUSE create 明确返回 `ENOSYS`，不会假装文件已创建。

## 1. 完整目录

下列是本步主要源码布局，已有公共库内部实现保持原状。采用 `foo.rs + foo/`：有真实子模块才建立对应目录，小模块先保留 `foo.rs`，不创建无内容的子目录。

```text
afs/
├── Cargo.toml                       # workspace + afs 根 package
├── Cargo.lock
├── AGENTS.md
├── PRINCIPLES.md
├── docs/code-layout.md              # 本目录合同
├── common/                          # 每个子目录是独立 crate
│   ├── logging/                     # 已有：进程日志设施
│   ├── metrics/                     # 已有：指标注册和导出设施
│   ├── tracing/                     # 已有：上下文传播和可选 Trace runtime
│   ├── error/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs               # 公共错误基类与转换边界
│   ├── protocol/
│   │   ├── Cargo.toml
│   │   ├── build.rs                 # 生成 local/meta/node_control/node_data 协议代码
│   │   ├── proto/
│   │   │   ├── local_api.proto
│   │   │   ├── meta.proto
│   │   │   ├── node_control.proto
│   │   │   └── node_data.proto
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
│   ├── config.rs                    # CLI/TOML 配置与 feature/runtime 选择
│   ├── runtime.rs                   # 观测初始化、服务生命周期与关闭
│   ├── bin/
│   │   ├── afs-meta.rs              # 进程组装入口
│   │   └── afs-node.rs
│   ├── meta.rs
│   ├── meta/
│   │   ├── rpc.rs                   # Node → Meta Handler
│   │   └── rest.rs                  # 管理面 REST
│   ├── node.rs
│   └── node/
│       ├── fuse.rs                  # 统一 POSIX/FUSE 入口
│       ├── api.rs
│       ├── api/
│       │   ├── local.rs             # SDK 服务端：UDS gRPC + SHM
│       │   └── rest.rs              # runtime 等使用的 Node REST
│       ├── rpc.rs
│       ├── rpc/
│       │   ├── control.rs           # Node 控制 Handler
│       │   ├── data.rs              # 文件读写 Handler
│       │   ├── peer.rs              # 远端数据 caller 和双 adapter
│       │   └── meta.rs              # 调用 Meta
│       ├── vfs.rs                   # 统一接口与 namespace 分派
│       ├── vfs/
│       │   ├── ownerfs.rs           # 可修改、根归属与本地性优先
│       │   └── blobfs.rs            # 私有写入、显式发布、不可变多读
│       └── storage.rs               # Node 内共享的本地 I/O 机制
└── tests/README.md                  # 未来跨模块集成/E2E 归属
```

Meta 根归属、节点管理、授权及 MetaStore 子模块后续细分。当前 Meta 只提供 ping/health/metrics 等基础入口；两种后端已经具备 namespace 分派和 unsupported create 响应，inode、版本与块布局仍留给业务阶段。

## 2. 五条接入关系

| 关系 | 调用方 | 接收入口 | 公共部分 |
|---|---|---|---|
| SDK → 本机 Node | client/connection、buffer | node/api/local | local_api Proto、gRPC config/security、SHM |
| Node → Meta | node/rpc/meta | meta/rpc | meta Proto、gRPC config/security |
| Node → Node 控制 | Node 调用业务 | node/rpc/control | node_control Proto、gRPC config/security |
| Node → Node 数据 | node/rpc/peer | node/rpc/data | node_data Proto、gRPC 或单边 RDMA |
| runtime → Node REST | 普通 HTTP 调用者 | node/api/rest | 按需共用观测设施 |
| 管理面 → Meta REST | 普通 HTTP 调用者 | meta/rest | 按需共用观测设施 |

表中将第三条 Node→Node 拆成控制/数据。SDK 只连接本机，远端访问由 Node 发起；SDK 使用 UDS gRPC 传控制信息，内容通过 sealed memfd + SCM_RIGHTS 交给 Node 读写，没有 gRPC 内容 fallback。没有 SHM 的使用者走 POSIX。不提供管理面 SDK。

## 3. 数据面与公共机制

```text
Node 业务 → rpc/peer 的共同文件 API
              ├─ gRPC：命令与内容进入 gRPC
              └─ RDMA：gRPC 命令/结果 + 单边搬运内容
                       ↓
                对端 rpc/data → 同一个对应文件业务
```

`node_data.proto` 两种模式都需要：定义文件命令和结果。gRPC 模式下 8 字节诊断内容内联进入 gRPC；RDMA 模式下内容不进 Proto，文件写可由服务端 RDMA READ 客户端缓冲区，文件读可由服务端 RDMA WRITE 客户端缓冲区。CQ 成功不是文件业务成功；当前诊断写入只证明普通文件 I/O 和传输接线，不定义产品持久化策略。

RDMA 建连参考 [3FS IBConnect](https://github.com/deepseek-ai/3FS/blob/22fca04564c7cc230fd8b9523b8b92864e1dad47/src/common/net/ib/IBConnect.cc#L338) 的实际通道探测：服务端先投递接收槽，双方经一次 `NegotiateData` gRPC 交换参数，客户端配置好 QP 后发送 `SEND_WITH_IMM` 探测并等待本地发送完成。服务端在首条数据请求中消费探测的接收完成，然后才允许文件访问；后续请求复用 ready 状态。没有独立 `ReadyData` RPC，也不为闲置连接启动持续 CQ 轮询。Close/TTL 删除会话登记，拒绝新的会话查找；已取得会话的在途请求仍可完成，资源随最后一个持有者释放，不宣称文件取消或 drain 屏障。探测带固定立即数、独立 WR ID，并校验 CQ 状态和操作码；超时或错误禁用会话。握手版本为 1，新旧 RDMA 握手不兼容，需同时更新两端；gRPC inline 文件协议不变。3FS 的设备发现/多网卡选路和完整消息传输框架不在本次移植范围内。

纯 gRPC 数据请求只依赖 `NodeData`，不调用会话协商。Node 仍注册 `NodeControl` 的 Ping 和会话接口；`data_mode` 是出站 adapter 选择，不是关闭入站控制服务的开关。

通道在尚未执行业务的建连阶段选择，结果不明的写不跨通道重放。MR/SHM 缓冲区所有权覆盖真实操作周期，取消等待不意味着可以复用内存。独立穿刺不等于生产资源池、会话回收器或硬件吞吐结果。

| 模块 | 负责 | 不负责 |
|---|---|---|
| logging/metrics/tracing | 观测设施、上下文、进程级初始化能力 | 把具体业务事件/指标统一堆到公共库；SDK 初始化全局设施 |
| error | 公共 Error/ErrorKind/Result，当前 VFS 使用 | 业务错误全集、旧 DMS 编码；errno/Status/HTTP 映射归入口 |
| protocol | local/meta/node_control/node_data wire 定义与生成类型 | Handler、域状态机、建连 |
| transport/grpc | 仅 config/security | 统一 client、重试或 actor；调用方直接用 Tonic 建连/监听/注册 |
| transport/rdma、shm | RDMA/native shim、sealed memfd、FD passing、资源生命周期 | 文件 API、根授权、业务成功或重试政策；SHM 当前是有界字节拷贝，不宣称零拷贝 |
| meta/rpc、rest | 请求校验与协议映射，共用 Meta 业务 | 为不同协议复制权威状态 |
| node/api | SDK/REST 入口，共用对应后端业务 | 复制发布状态机或吞并 Meta 管理面 |
| node/vfs | 共用接入形状、namespace 分派与身份映射边界 | 强迫两种后端共用持久化、缓存或恢复模型 |
| node/storage | 两种后端共用的本地 I/O 机制 | 决定何时 sync、如何发布/恢复、强制 Blob 化 |

FUSE 节点号、持久文件身份、Blob 版本身份分开。Node 内部文件 DataClient 留在 rpc/peer，不搬进 transport。storage 先是 Node 内模块，不提前拆独立通用存储 crate。

## 4. Cargo 与 feature

根 `afs` package 包含两个 binary 和 meta/node library 模块；SDK 是独立 `afs-client` crate，不依赖根 package。common 不反向依赖 Node、Meta、SDK。

transport 默认启用 `grpc`，`shm` 用于本机 SDK，`rdma` 是可选 feature。SDK 明确启用 `grpc + shm`，不启用 RDMA；根 package 默认使用 transport 的 `grpc + shm`，显式 `--features rdma` 增加 RDMA native shim 和单边 adapter。根 package 两个 binary 共用 feature 图；未来如拆分独立制品，再验证 feature 隔离需求。

Proto 已生成 local/meta/node_control/node_data 类型和基础 service。基础框架只提供 ping、诊断和 8 字节数据面，不放假的文件业务成功路径。SDK 的独立 crate 边界已建立，但 publish=false 保留到实际交付验收。

## 5. 本步实施与验证

本步按用户已确认的控制台方案直接实施，不改文件业务语义、不移植历史产品、不提交或推送。

- [x] 建立本文和公共 error/protocol、SDK、进程/业务模块。
- [x] 接入 Cargo、Rust 模块树、Proto 生成和 binary 入口；保留既有公共实现。
- [x] 实现配置、observability、Meta/Node REST、Node→Meta、Node→Node control/data、Local SDK UDS+SHM、FUSE namespace 分派。
- [x] Linux fmt/check/feature matrix/test/Clippy/build/E2E；覆盖 8 字节诊断链路、FUSE ENOSYS、metrics、trace、配置拒绝与清理。
- [x] 同步导航与状态，保留已有未提交修改。

本步只验正式基础框架，不验真实 OwnerFs/BlobFs 文件业务、镜像发布、故障恢复或性能目标。最终结果见 [状态](status.md)。

## 6. 从入口读代码

第一版的关键模块、接口和资源生命周期已补中文注释。建议按以下顺序阅读：

1. `src/bin/afs-node.rs` / `afs-meta.rs` → `src/config.rs` → `src/runtime.rs`：CLI/TOML 合并、编译与运行开关、观测和退出。
2. `src/node.rs` → `node/fuse.rs` → `node/vfs.rs` → `vfs/ownerfs.rs` / `blobfs.rs`：同进程入口和后端分派。当前 FUSE 创建返回 ENOSYS，尚未串到真实文件业务。
3. `node/rpc/peer.rs` → `node/rpc/data.rs` → `node/storage.rs`：共同客户端 API、gRPC/RDMA adapter、共同服务端 Handler，以及诊断字节实际写入。
4. `node/rpc/control.rs` → `common/transport/src/rdma.rs` → `common/transport/native/rdma.c`：会话协商、Rust 所有权、libibverbs 单边搬运。`native/` 是 C ABI 适配，不是 Native FS 后端；只在启用 `rdma` feature 时编译链接。
5. `client/src/connection.rs` → `client/src/buffer.rs` → `node/api/local.rs` → `common/transport/src/shm.rs`：UDS 控制、memfd/FD passing 内容和取消后的资源回收。当前是有界拷贝，不是零拷贝。

诊断数据链与 FUSE 分派链分别验基础框架；不能把两条链已通过理解为完整文件业务已接通。代码中的未来职责说明与当前实现边界已分别标明。

# AFS 正式基础框架实施合同

> 实施采用已获授权的 E2E 执行；按 executing-plans 与测试先行推进，不另设用户评审门槛。

**Goal:** 把已确认的模块落实为后续业务可直接扩展的运行底座，以最小真实请求验证接线。
**Architecture:** afs-meta/afs-node；统一 VFS 下的 OwnerFs/BlobFs；本机 SDK UDS+SHM；节点控制 gRPC，内容 gRPC/单边 RDMA adapter。配置与观测由进程组装，不塞入业务。
**Tech Stack:** Rust 1.95、Tokio/Tonic/Prost、Fuser、memfd/SCM_RIGHTS、libibverbs、Axum。
**Spec:** ../code-layout.md、../../PRINCIPLES.md，以及本次控制台批准的正式框架目标。

## 全局约束

- 实施阶段保留未提交修改；2026-09-25 用户后续授权阶段收尾、提交并推送当前独立分支。仍不 merge 或 release。
- macOS 编辑，所有构建、测试与服务运行在 Linux。
- 不恢复旧 KV/NFS 后端，不把当前 Hello 验证称为完整文件系统。
- 功能开关：编译 ownerfs/blobfs 独立；运行 ownerfs/blobfs/all。请求未编译后端失败；关闭后端不注册 namespace。
- SDK 无共享内存时由调用者选择 POSIX，不内置 gRPC 内容兜底。RDMA 只在开始业务前选择，结果不明的写不重放。
- FUSE create 走真实回调/分派/业务入口后明确返回 Unsupported，不虚报创建成功；热路径不为演示而逐次远程 ping。

## 接口与分工

- 根 package 负责 config.rs、runtime.rs、meta、Node 装配、REST、diagnostic 流、公共错误/Proto 构建及所有 Cargo manifests。
- storage：`Storage::new(path) -> io::Result<Self>`；`read(&self,name,offset,length) -> Result<Vec<u8>, StorageError>`；`write(&self,name,offset,Vec<u8>) -> Result<usize, StorageError>`，async I/O 在阻塞线程，初版仅安全的单文件名诊断对象。它不决定业务发布/持久化语义。
- data/peer/control lane：Node 业务共同 DataClient read/write，gRPC/单边 RDMA；Proto node_data/node_control；公共 RDMA 设备机制。
- SDK lane：client、local_api Proto、Node local handler、公共 SHM 机制；每个共享缓冲的归属延续至真实操作结束。
- VFS lane：Vfs 与后端、FUSE 真实挂载；后端按 feature 编译及运行选择。
- Proto 位于 afs-protocol；不在 transport 内注册业务，不强加 actor。

## Review focus

1. 错误配置/未编译后端不得静默切换；启动监听失败须终止并回收已启动部分。
2. SDK/RDMA 调用者取消不等于内存可复用；明确归属、poison、关闭与过期。
3. 已占用 UDS/FUSE 路径不能删除别的进程资源；关闭只清理自己资源。
4. 无 RDMA/无 SHM/对端丢失必须有界返回；业务开始后的失败不能 fallback 重放。
5. 命令/日志/文档不冒充 root 授权、缓存一致性、发布与故障恢复已完成。

## 实施与验收步骤

- [x] 配置与启动：先以 CLI/TOML 优先级、未知字段、编译缺失为失败用例；实现 defaults < TOML < CLI，启动/退出及配置输出；复用 main 机制，排除旧业务配置。
- [x] Protocol/Meta：真实 generated service/client，Node ping Meta，REST health/ping/metrics，连接失败退出与 trace 传播。
- [x] VFS/FUSE：独立编译/运行选择；列举两个 namespace；touch 到对应后端日志后返回 ENOSYS；关闭与非法 namespace 验收。
- [x] SDK：真实两个进程的 UDS 控制 + SHM 8 字节 write/read；无内容字段；取消/边界/句柄过期/关闭测试。
- [x] Node data：相同 read/write 调用在 gRPC/RDMA 下读回8字节；真实 RXE CQ；无设备 auto 回退、强制 RDMA 报错、取消/过期/关闭测试。
- [x] E2E：Meta + 两 Node + SDK + FUSE；REST 诊断明确调用正式 APIs；日志标明路径、协议和 trace，metrics 有请求/错误结果。
- [x] 文档：同步全部现行架构/需求/原则/目录/使用指南/状态，历史报告不改写；记录独立进程 E2E 方法和边界。
- [x] Linux fmt、clippy、workspace test、feature 矩阵、独立 review 后修复，证据落在外层 evidence/，完成 goal。

## 停止条件

以上基础框架验证闭环，无未说明失败；硬件 RDMA 性能、完整 POSIX/授权/发布/生产可靠性不属于此轮。RXE 证明单边语义，不证明硬件性能。

## 验收结果

2026-09-25 完成。范围与已知边界见 [status.md](../status.md)，验收与交付见[阶段摘要](../foundation-milestone.md)。原始证据留在仓库外研究区，不随源码提交。独立复核的资源生命周期、协议长度、会话清理和 socket 权限问题已修正；后续按用户授权提交并推送独立分支，不合并 main。

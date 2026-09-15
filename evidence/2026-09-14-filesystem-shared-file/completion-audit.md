# Filesystem 共享文件纵向主链完成审计

本文件把目标中的每一项与最终源码、自动化测试和真实 Linux 验证绑定起来。证据来自分支 `arch/unified-node-runtime` 的最终工作树，不代表完整 POSIX 已完成。

| 目标 | 状态 | 主要证据 |
| :--- | :--- | :--- |
| Node A create/write | PASS | `result.txt`；`server/src/node/filesystem/shared.rs`；`server/src/node/data_core.rs` |
| Node B 首读与热读 | PASS | `latency.json`；`node-b.prom`；首读走 Peer Block，50 次热读复用 Node 本地 dentry、binding 与 Block |
| Node A pwrite 中段覆盖 | PASS | `result.txt`；DataCore 的 Extent overlay 单元测试；最终内容校验 |
| Node B Watch 失效后读取新版本 | PASS | `result.txt`；Node B 日志和失效指标；旧 exact binding 被撤销后重新解析 |
| Meta 重启恢复 | PASS | `recovery.json`；本地 WAL replay 测试；重启后的 Meta 与 Node B 能再次读取正确内容 |
| FilesystemCatalog 属于唯一 MetaState | PASS | `server/src/meta/runtime.rs`；文件 inode、dentry、binding 与现有 Meta journal/checkpoint 共用单 owner |
| 独立 filesystem protobuf、Client、Handler | PASS | `protocol/proto/dms/v1/filesystem_meta.proto`；`server/src/node/filesystem/meta_client.rs`；`server/src/meta.rs` |
| DataCore prepare + 单一 Meta 原子发布 | PASS | 候选版本先在 Node 准备；一次 `CommitFilesystemVersion` 同时发布 ObjectVersion、inode exact binding、属性和失效义务 |
| OpenHandleTable、DentryCache、BindingCache | PASS | `server/src/node/filesystem/`；Node B 指标为 dentry 51 hit / 1 miss、binding 310 hit / 1 miss |
| 日志、typed metrics、trace | PASS | 正常 cache hit 不逐次写日志；FUSE/Node/Meta 关键阶段有 span；Meta journal、文件操作、缓存和失效有固定标签指标 |
| 不改变既有 KV SDK、WorkerService 与单 owner | PASS | workspace 全特性测试；架构边界测试；`WorkerService` 没有新增文件系统方法 |

## 最终验证

```text
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --workspace --all-targets --all-features
bash scripts/validation/run_filesystem_shared_file_e2e.sh
```

- Rust workspace 全特性测试：PASS；`dms-server` 321 passed、0 failed、1 ignored（手工基数计时测试）。
- Clippy：PASS，warnings 按错误处理。
- 真实单 VM、双 Node、FUSE 主链：PASS。
- Node B 首读：231.954 ms。
- Node B 50 次热读：median 0.868 ms，p95 0.954 ms。
- Meta filesystem lookup：2 次，而非随 50 次重复 open 线性增长。
- Meta 重启后恢复读取：4.597 s；主要受当前 5 秒 session/heartbeat 重连节拍影响。

## 明确不在本切片内

完整目录语义、rename/link/unlink、权威 readdir、权限与锁、mmap/fsync、write-back、配额和多租户仍属于后续 POSIX 阶段；当前实现没有用占位接口伪装这些能力已经完成。

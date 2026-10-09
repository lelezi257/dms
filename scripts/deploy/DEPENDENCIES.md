# AFS 运行依赖

Linux 试用包的安装和运行不需要 Cargo、Git 或 Rust 工具链。

## 必要条件

- 使用与发行包一致的 Linux 架构。既有试用环境为 Ubuntu 24.04 ARM64、Linux 6.8、ext4；其他环境需要独立验证。远端 OwnerFs 的 direct-I/O shared mmap 需要内核公开 `FUSE_DIRECT_IO_ALLOW_MMAP` 能力，旧内核未获验收。
- `bash`、`coreutils`、`tar`、`sha256sum`、`sed`、`awk`、`grep`、`curl`、`python3`。
- GNU `timeout`（来自 `coreutils`）用于限制挂载检查和卸载等待时间；`afs-selfcheck` 也通过它限制 Python 文件系统探针的总运行时间。
- FUSE 挂载需要 `fuse3` 运行环境和 `/dev/fuse`。
- `iproute2` 提供的 `ss` 用于检查端口冲突。缺少它时进程控制器仍能启动，但端口检查能力较弱。
- `util-linux` 提供的 `findmnt` 用于检查精确的 AFS FUSE 挂载和运行自检。
- 使用 `afs-trial-config` 生成单节点或双节点试用 TLS 材料时需要 `openssl`。
- 按对应包内 `ldd.txt` 安装动态库。既有 ARM64 包列出过 `libibverbs.so.1`、`libnl-route-3.so.200`、`libnl-3.so.200`、`libgcc_s.so.1`、`libm.so.6`、`libc.so.6` 和目标动态加载器；以实际包为准。即使 `ldd.txt` 未列出 `libfuse3`，仍需要 `fuse3` 和 `/dev/fuse`。

## Meta 后端

- `memory` 用于可丢弃的演示和短周期开发；`afs-meta` 退出后不保留命名空间状态。
- `local-file` 用于持久化试用、重启检查和本地 smoke，是当前恢复验收主线。
- etcd 配置使用 `meta_store = "etcd"` 和 `etcd_endpoint`。
- 包含 Redis 后端的二进制使用 `meta_store = "redis"` 和 `redis_endpoint`。

存在配置字段不代表该后端已经验收。etcd、Redis 及复杂可靠性后置；每个后端都需要自己的恢复和行为对照证据。

## 按需条件

- 经身份认证的 Node-to-Meta 和 Node-to-Node 通信需要 TOML 引用的 TLS 证书文件。
- RDMA/RXE 验收需要对应工具和内核支持；试用包不安装内核模块。

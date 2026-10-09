# 阶段性集成交接

决策：冻结产品 a103，交付可编译、可安装、可运行的有限版本；未完成的功能、性能与可靠性继续列项。G1历史8/8关闭，原G2计数不变。GitCode目标仓库/分支待用户提供，尚未执行GitCode合入。

| 交付内容 | 固定身份与证据 |
| --- | --- |
| 产品源码 | a103a2f2；后续main提交仅文档/证据，165输入map dc06adcc不变 |
| Linux ARM64试用包 | [afs-bind-a103a2f](https://github.com/lelezi257/dms/releases/tag/afs-bind-a103a2f)，14,492,878B，SHA256 `260d5009f343dd52e938e5659a6e9108be7cdbeb8c77124ff1696c4fdf1cf2b5` |
| 实际构建、安装和恢复 | [Linux release构建、包两次一致、无编译器ON安装、Owner64KiB/DFS64MiB正常全停恢复及五wait0](evidence/20261009-workspace-bind-a103-trial/README.md) |
| 远端已有范围 | [Node217/Meta650真实Home bind ON与C远端statfs、UID501核心操作、pjdf smoke4/241](evidence/20261009-owner-remote-statfs/README.md)；不升级为Meta cda组合/full POSIX |
| 源码迁移核对 | [完整main096f在Linux恢复及锁定离线Cargo元数据核对](evidence/20261009-integration-closeout/README.md)；含所有历史证据、维护工具及vendor |

## 编译与使用

从完整源码仓根目录在Linux构建，Rust由`rust-toolchain.toml`固定为1.95.0；准备C链接工具链与`protoc`，Cargo需能访问或已缓存锁定依赖。既有构建证据使用以下命令，默认特性ownerfs/dfs，未开启rdma：

```sh
cargo build --release --locked --bin afs-node --bin afs-meta
```

默认产品二进制不包含测试探针。仅验收流程需要它时单独执行：

```sh
cargo build --release --locked --example afs-workspace-probe
```

使用固定发布包时，目标Linux不需要Cargo或编译器；依赖见[scripts/deploy/DEPENDENCIES.md](../scripts/deploy/DEPENDENCIES.md)。安装及明确ON步骤以[包内同版指南](../docs/guides/trial.md)为准。普通配置默认OFF；当前使用场景显式ON，真实Home底层目录bind到OwnerFs FUSE根下对应一级workspace，远端仍走FUSE/RPC。验收探针和容器rootfs不在普通包内。

## 合入范围与未完成项

集成必须携带`src/`、`common/`、`client/`、Cargo清单/锁和toolchain、`error-codes.toml`、当前`third_party/fuser/`、维护中的`tests/`和`scripts/`、文档及可追溯证据索引。按目标仓结构确定整体根目录或子目录集成后，再检查Cargo相对路径、CI入口和安装位置；本轮不预设目标布局。源码外VM镜像、运行ELF、rootfs和大型备份不合入。

完整096f当前树含20,062文件/约100.5MiB，development约95.5MiB；移植包完整保留历史记录和九个历史符号链接，不跟随链接复制外部数据。历史体积压缩/进一步搬迁另列，不能在集成时直接删除证据破坏链接。

- 新Meta cda/Node217多节点远端补验在[固定用例预算门禁](evidence/20261009-owner-remote-published-meta/README.md)启动前停止，零服务/零操作，仍待验；单节点ON安装及正常恢复已实际通过。
- 完整远端POSIX未通过；四脚本smoke仅证明其限定范围。
- 普通Owner本地/远端吞吐≥1.2×MooseFS与独立操作时延≤0.8×需同时满足，当前未最终达标；DFS同三同步持久副本3FS仍待验。bind八个历史核心≥0.90×ext4按原版本/未变路径限定复用。删除保持正确性与性能报告要求。
- 复杂native/FUSE append/偏移、锁/watch、生产issuer/durable ACK、即时native FD撤权、live-Meta-only/crash/R3扩展可靠性、大规模/长时、多Meta、etcd及Redis后置。支持范围见固定试用清单，不能把历史结果转为新候选全面通过。
- [R2官方fuser迁移](repository-remediation.md)仍未完成；当前Cargo依赖含既有私有差异的vendored0.16.0。本轮原样保留以保持已构建版本，未新增第三方修改，不能称依赖已与官方原版一致。

当前出口是有限可运行版本及可核对的集成输入。下一是确认GitCode目标后执行其布局/构建接线核对；本轮不追加性能优化，不重开阶段一，不声称整个G2/G3完成。

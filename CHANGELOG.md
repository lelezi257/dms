# 更新记录

## 0.1.0 候选版本（尚未公开发布）

首个功能验证版本，产品交付分为 SDK 和 Linux Node/Meta 组件。版本号用于本地构包验收，不表示已在公共仓库发布。

- 字节对象 SET/GET/DEL、批量操作、范围读写、Hash/两级键操作。
- 本地 UDS/共享内存、跨进程 TCP，以及跨 Node 数据读取。
- Meta 版本与位置管理、缓存失效协调、本地元数据 WAL/snapshot。
- 原生数字错误码、结构化日志、Prometheus Metrics、可选分布式 Trace。
- 用户文档、中文关键注释、开发协作规则和阶段产物 Skills。
- 候选 SDK 单包构建、服务文件白名单构包和隔离安装验收入口。
- 原生 Go 薄 SDK 的对象子集，以及 Rust/Go 共同的 Stat/Scan、稳定 mtime 与空 value；Go 候选通过独立 module 包消费，不依赖本地源码 replace。
- 独立 JuiceFS 后端适配与部署手册；保留文件系统原有目录、inode 和分块布局。带适配器的 JuiceFS 二进制需要单独构建，不包含在 Node/Meta 运行包中。
- 正常旧版本回收的 Prepare/排空/最终释放，以及 TCP、SHM、Peer 在途读取和共享写权归还保护；不使用 TTL 或连接中断冒充借用已结束。

### 使用限制

当前写入仅支持本地内存；Node 重启可能丢失 value。永久失联 Node 或未归还的共享写权仍可能阻塞回收；多 Meta 高可用、RDMA/UB、L2、设备内存和 Python/C++ SDK 尚未实现。Go SDK 目前是对象子集，不具备全部 Rust SDK 方法、TLS 和宿主观测注入。共享内存仅面向可信本地应用，不能替代租户隔离。完整说明见[能力与限制](docs/product.md)。

本版本不承诺生产性能、稳定 ABI 或跨版本滚动升级。正式发布前仍需确认托管地址、包名归属、版权授权及发布平台。

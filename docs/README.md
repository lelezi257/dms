# 文档导航

新用户按“安装 → 单 VM → 编程”阅读，不需要先理解全部内部模块。

已经拿到候选制品的用户直接读[候选包安装与独立编程](release-installation.md)；下面的源码构建入口用于从源码开发。

1. [能力与限制](product.md)：判断当前开发预览能否用于你的实验。
2. [安装与构建](installation.md)：准备 Linux 环境并生成二进制。
3. [单 VM 手动教程](local-single-vm-manual.md)：启动、SET/GET、停止与复现。
4. [Rust SDK 编程](rust-sdk.md)：返回值、批量、KKV、随机写和共享读视图。
5. [配置](configuration.md)：启动参数、配置文件、SDK 参数与优先级。
6. [观测](observability.md)：日志、Metrics、Trace 及图形界面。
7. [排障](troubleshooting.md)：从症状定位连接、内存、数据和观测问题。

开发者再读[架构](architecture.md)、[贡献指南](contributing.md)、[白盒性能基线](performance/whitebox-baseline.md)、[源码规则](../AGENTS.md)和[项目 Skills](../skills/README.md)。

已有的专门操作手册保留在本目录；以本导航及对应专题入口为当前阅读顺序，不能从某次实验成功推导出生产高可用或完整持久化能力。

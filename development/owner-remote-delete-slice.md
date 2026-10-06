# OwnerFs 远端小删除：正常收尾出口

**实际出口：** [G2.16完整实跑和正常收尾](evidence/20261007-owner-remote-delete-small/README.md)已完成；12sample/600独立路径/6wait0，G2为10限定完成。下面保留执行前范围和判据，不再重复本slice。下一[DFS同步读阶段](dfs-manyread-sync-slice.md)。

2026-10-07，G2.16独立小项。产品6d51aeb/map66dbbe3e/157输入和已有release ELF未变，Owner-only/local-file Meta/gRPC/native OFF。A为远端客户端，B为Owner Home及唯一Moose chunk节点。旧[删除数据/跨Home正确](evidence/20261007-owner-remote-small/README.md)及Moose客户端真实wait1 FAIL保持原样，不能仅靠另一个运行的成功退出改写它。

本项使用fresh `owner-remote-delete-6d-bhome-20261007-r1`夹具、独立端口及fresh输出，仅执行维护驱动`--case delete`，不重测读写或改产品。复用[后续写夹具正常卸载](evidence/20261007-owner-remote-write-small/README.md)的准入方式：固定fusermount3真实help/version、exact已验证mount/source正常`-u`、真实Moose子进程wait0，再停Home服务。

- A上每系统每轮100×4KiB，1预热5交替配对。准备/内容SHA在unlink计时外；返回值、数量、A视图消失正确，B独立挂载核对Owner600个路径不存在，操作性能逐样本/中位数/配对比完整报告。
- 固定Moose3工具、sole chunk B/一副本、mount选项及执行ELF身份可复核；缓存未观察明确披露。删除报告没有新增硬比例阈值，不把它宣称远端读写持平或strong durable write资格。
- 所有3AFS/3Moose实际正常wait0、exact mount消失、无owned进程，既有其它fixture incarnation不变。任何清理失败均保留，不计本出口完成；不用force/lazy卸载把失败改PASS。
- 测前一次性核对依赖/源码和二进制摘要、Linux ARM64/ext4、固定配置/端口/挂载/容量；预算及floor沿用已验证维护helper，每新fixture≤1GiB，不放大VM。实际环境阻塞保存原证据，停止受影响项求助。

维护fixture新增入口只跑有意义的路径/端口隔离guards，旧未改函数的有效Linux结果复用；无Rust改动则不重新构建或跑标准全集。完整证据、结果分类和Lore提交/普通push后更新G2.16；本计划本身不是PASS。

# 测试 VM 容量与空间管理

2026-10-07 当前生效环境；历史证据保留原磁盘容量、版本和判据。此次是独立环境维护，不重开 G1 历史 8/8，也不继承任何运行/性能 PASS。

## 实际环境与维护范围

| VM | 系统盘 | 数据/状态盘 | 本次结果 |
| --- | --- | --- | --- |
| afs-accept-a | 24GiB | 32GiB，/dev/vdb1 → /mnt/lima-afsadata | 用户决定暂缓；7 个历史 memory Meta 与 FDB 保持运行 |
| afs-accept-b | 24GiB | **42GiB**，/dev/vdb1 → /mnt/lima-afsbdata | 原 32GiB 原地扩容及完整数据身份核验通过 |
| afs-accept-c | 24GiB | **42GiB**，/dev/vdb1 → /mnt/lima-afscdata | 原 32GiB 原地扩容及完整数据身份核验通过 |
| afs-accept-ctl | 24GiB | 8GiB，/dev/vdb1 → /mnt/lima-afsctlstate | 用户决定暂缓；历史 memory/etcd Meta、Redis 保持运行 |
| afs-build | 未改 | 未改 | 空间充足；只用于 Linux 构建/只读备份验证 |

Lima 2.1.1/VZ、Ubuntu 24.04 ARM64/Linux 6.8.0-142/ext4、原网络和磁盘镜像保留。数据卷不是宿主共享目录。B/C 应用测试和基线服务原本均未运行，本次正常停机/启动，不启动历史实验来伪造恢复。两卷 UUID、GPT/分区 UUID、分区起点、挂载点与原内容/权限/xattrs/链接保持；B/C 系统盘未扩容。B/C 配置仅将附加盘 `format` 设为 `false`，防止标签异常时自动格式化；扩容使用 `limactl disk resize`，启动脚本完成 growpart/resize2fs。没有 mkfs、重建、更换基础镜像或改用宿主数据路径。

后续 A 数据盘 42GiB、ctl 状态盘16GiB/根盘32GiB 仅为待重新准入的建议，**不是已执行目标**；必须先解决用户要求保留的 memory 状态。完整建议增量46GiB，已执行 B/C 共20GiB。原建议64GiB在初始主机126.94GiB空闲、最大单盘备份27.33GiB与50GiB余量条件下不足，未执行。

主机空间按实际 df 检查，不按 raw 镜像逻辑大小猜测。每一批先锁定实际获准范围，以“本批新增容量全部写满 + 仍保留的备份最坏额外占用 + 临时预算 + >=50GiB余量”准入。APFS 克隆不是零预算；顺序维护也不能漏算仍保留的前一盘备份。本批最终仍预留完整20GiB、两个离线克隆约15.92GiB及1GiB临时，实际127.96GiB空闲对应约91.04GiB余量。A/ctl 将来单独重新核算，不能继承本批预算。

## 小规模 OwnerFs 的后续预算（2026-10-08 起）

此前 B 盘约19.86GiB空闲而单目录256MiB超限，是峰值漏算和过紧的测试目录预算；不是卷耗尽。旧失败不改判。后续64MiB/C1普通/绑定协同 Owner 对照，数据节点目录预算固定512MiB、ctl256MiB；B/C卷保留4GiB、ctl保留512MiB，且必须能覆盖该目录全部尚未使用的预算。候选/基线共用同一组预算、顺序运行，复用现有64MiB文件；新结果使用新run ID，不覆盖旧证据。这个规则只适用于该小规模轮廓，不替代大规模公式或自动提高历史轮次上限。

启动前使用维护中`probes/dfs_r3_fixture.py`的`budget --additional-bytes N`（Python调用`budget(additional_bytes=N)`）核对“当前实际分配＋预计峰值增长”。本轮B/C保守预留256MiB增长，包含payload备用64MiB、ELF/工具暂存64MiB、日志64MiB、状态/WAL64MiB；ctl预留128MiB及其目录余量。既有数据已计入当前du，不重复计算，但不漏算转移暂存。先检查再传输/启动，不能只检查当前占用。完整启动资格仍需核对套件、配置、二进制、身份、挂载和依赖。

准入、运行观察和退出核对必须使用同一份测前冻结参数；不能新准入512MiB、退出观察仍套用旧256MiB。不修改旧快照或已失败判据。运行中监控实际占用、日志和卷保留空间，增长接近边界时正常收尾并留证；预算预留本身不证明驱动已有独立日志轮转。案例结束后只保留必要数据，按既有归档/恢复规则回收可再生内容，不复制每轮工具或数据。512MiB/8GiB规模仍独立按下表的副本、generation和并发计算，不沿用小规模总预算。

[当前真实容量及19项Linux工具回归](evidence/20261008-owner-capacity-policy/README.md)。维护完成不代表产品或性能通过；A/ctl保留memory状态的扩容障碍仍按原决定后置。

## 按 case 计算峰值

测前写清数据 D、同时写者 W、持久副本 R、同时保留的完整 generation G、暂存/校验完整副本 S，以及测量轮数 K。保守集群数据预算为 `D × W × R × (G + S)`；R3 三节点各持有全部写者数据时，每数据节点为 `D × W × (G + S)`。另加 Meta、chunk/目录/索引开销、基线自身副本、日志和工具临时空间；以实测 du 校正，禁止把 EOF 数据量当整个卷占用。

以下只是**新增 payload 的保守预算示例**：R3、G=2（当前与一个保留 generation）、S=1（暂存/校验）。日志、Meta、既有数据和守卫余量另计。

| 单文件 D | W=1 每节点 / 三节点合计 | W=3 每节点 / 三节点合计 | 准入说明 |
| --- | --- | --- | --- |
| 64MiB | 192MiB / 576MiB | 576MiB / 1.6875GiB | 当前主线规模；沿用已固定小项门禁，并记录实际峰值 |
| 512MiB | 1.5GiB / 4.5GiB | 4.5GiB / 13.5GiB | 独立扩大项，不继承64MiB的容量 PASS |
| 8GiB | 24GiB / 72GiB | 72GiB / 216GiB | 当前 A 历史占用下不能准入；42GiB盘也不能支撑此 W=3 示例 |

K 轮若在内容/副本/结果保存、正常关闭后复用同一可再生空间，不再乘 K；若保留每轮唯一文件或旧 immutable chunks 未回收，G必须计入全部仍存活 generation，不能假定 unlink 即释放所有版本。候选与基线顺序运行并分别计算各自副本；同时保留两套则相加。8GiB 级仍至少保留 guest4GiB，不降低当前小项已固定的守卫。大规模/长时/复杂可靠性后置，不把扩容当作无限容量。

## 固定管理规则

- 已停止的历史测试只在完整归档、真实恢复核验、无进程/FD/mount引用后清理。结果、命令、版本、失败记录、校验和及恢复映射长期保留，原链接不改。磁盘镜像/大型备份/完整文件清单放源码树外；Git仅保存维护工具、紧凑记录与索引。
- 新性能 case 默认 INFO、`trace_enabled=false`；归因 TRACE 仅允许事先声明的短窗口并计入预算。每 VM 默认日志预算64MiB，单文件16MiB；轮转前归档旧片段，不能覆盖或丢弃失败记录。触及预算停止受影响小项、正常收尾并保存全部现有输出，不能静默截断或按成功处理。已有历史服务不在本批中改日志或 vacuum，旧证据不删。
- 新 case 启动前固定日志预算与监测责任人/驱动；上面是执行规则，**不是宣称所有旧服务已有轮转实现**。现有 driver 的总 case 上限继续有效，独立日志门禁未接线的 case 不能宣称该专项已经验证。
- 每轮引用同一受维护工具的 Git版本/哈希，不复制整份工具或源码快照。只保存差异、精确命令和结果；可再生数据在验证后复用，保留量必须在测前声明。
- 新性能结果标记本次扩容后的容量/UUID/版本。一次比较的候选与基线使用相同节点、接口、数据、并发、缓存、持久屏障及适用副本语义；不可比的旧数据仅作摸底，已通过且不受影响的结果直接复用，不重复测试。
- 实际扩容或恢复失败立即保存证据、停止该盘操作并求助；不重建、重新格式化或反复修补。维护 PASS 不等于产品功能、RDMA或性能 PASS。B/C 本次重启后的 `rdma link`为空，本次没有前态证明或 RDMA恢复资格；当前 gRPC 小项不因此冒称 RDMA通过。

[容量、完整身份检查、初始拒绝与备份恢复索引](evidence/20261007-vm-capacity-maintenance/README.md)。原 acceptance.lock 仍 PREPARING，保留原 SHA，不在此次维护中伪造正式环境锁。

For serial small Owner comparisons, reuse the existing payload. After complete raw-record archive recovery and actual Linux restoration/SHA checks of retained host ELF inputs, remove only the run-specific temporary transfer and staged ELF paths. Keep prefix binaries, original data/state/logs and earlier failed evidence. Record exact removed paths/bytes and post-cleanup capacity; do not leave a new pair of ELF staging copies per round. [Executed example](evidence/20261008-owner-remote-write-server-pair/staging-cleanup.json).

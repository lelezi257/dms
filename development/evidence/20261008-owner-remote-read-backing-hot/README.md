# 普通 OwnerFs 远端小读：底层热、客户端默认策略

**事实：** G2.14 的新独立 case，固定 f03/7bfc 产品；64MiB/C1/1MiB，1 预热＋5 交替对，A 读、B Home/唯一 Moose 副本、ctl local-file Meta/master。不是 G2.13 bind 测试；[原驱动范围文字勘误](preparation-errors.json)保留原始结果和执行脚本。

| 项目 | 当前版本状态与证据 |
| --- | --- |
| 内容、身份、独立计时 | **限定 PASS**：[测前合同](contract.json)、[准入](admission-summary.json)、[实际 Home/一份 VALID B 副本](topology-proof.json)。双方 byte97 全64MiB SHA一致，各320真实测量间隔，准备 fdatasync＋目录 fsync 均在读计时外。 |
| 吞吐 ≥1.2×MooseFS | **FAIL**：Owner/Moose 中位384.806472/14457.621211MiB/s，比例0.026616168。[结果](result-summary.json)、[全部640测量间隔](measured-rounds.json)。 |
| 独立时延 ≤0.8×MooseFS | **FAIL**：Owner p50/p95/p99=2694666/3152191/3316037ns，Moose=55128/111257/175928ns；预定 pooled p95 比28.332518。吞吐和时延分别判定，两项都失败。 |
| 正常退出、保护状态 | **PASS**：6实际wait0，12子进程/监督者消失，独立核对旧11保护进程及全部挂载不变；UDS/全部选定端口关闭；三角色预算/保留空间达标。实时61项＋独立保存数据98项核验。 |
| 维护工具/可恢复性 | Linux B 上22 fixture guards PASS，物理观察器及5个已有 Linux guards按不变SHA复用。146 raw文本文件/93538B压缩归档在 Linux 实际恢复，313项检查；3旧配方通过紧凑delta恢复、2新运行配方、2postcheck、fixture与guard均核对实际身份。[恢复索引](raw-evidence-index.json)。 |

**比较边界：** 普通 POSIX/FUSE/O_RDONLY pread，Owner 两开关 OFF，官方 Moose4.59.2 AUTO；相同应用数据、并发、预读和读 close 边界，各自内部默认缓存策略保留。每次样本前双方全客户端SHA，再全物理payload SHA；B 上24个物理区间前后mincore快照均64MiB热且dev/inode/size/mtime/ctime稳定。Moose本fixture的8192前缀被排除，其payload全文SHA和尺寸实际验证。客户端Owner0/Moose64MiB单独报告，[缓存证据](backing-cache-summary.json)；并非每次双方都走网络、并非端到端缓存驻留相同。结果只对测前声明的默认策略/底层热场景有效，不替代其它缓存或完整性能矩阵。旧 DIRECT/ENODEV 的零参考样本及原失败[保持原版本/结论](../20261008-owner-remote-read-current/README.md)，未重跑、未改判据。

实际操作时延围绕每个pread＋长度/内容检查，CLOCK_MONOTONIC；吞吐独立覆盖open、线程、readloop、join、close。合同预定 pooled nearest-rank `ceil(N*p/100)-1`，原C单次输出按其固定下标算法另留原值；不在结果后选分位数，不由吞吐推时延。读准备屏障、单份数据语义已对齐；这不是强写durable-ACK、断电耐久、POSIX全套、full bind或全G2验收。

**环境与保留：** 原有 Linux6.8.0-142-generic/ext4，未修内核/vendor、未改VM/产品。最终本case分配A41410560/B182951936/ctl50606080B，空闲A2101555200/B23435321344/ctl3155820544B，分别满足512MiB/1GiB/256MiB预算和1GiB/4GiB/512MiB空闲底线。日志3034B/4ERRO＋2WARN完整保留：创建时尚无注册root/路径以及ioctl提示，不称零日志错误。准备/审核/恢复工具首失败见[错误索引](preparation-errors.json)，没有第二次产品计时或环境修补。原数据仍在VM；文本归档不是数据盘备份。

**决策：** G1历史8/8关闭，G2完成总数及性能门槛不变；本小读摸底与限定比较收口，优化另留专题，不重复刷分。下一按既定顺序核对 DFS 一写多读已有 f03 证据的可复用范围，3FS正式比较资格、普通性能调优和大规模仍后置；不重复已过标准、8 bind核心性能及恢复。

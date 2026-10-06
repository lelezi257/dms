# 当前候选独立E2E证据

产品代码/ELF锚点e925c5bcf0408851ebfa08a59df29953374da9e9，local-file Meta/gRPC/FUSE，bind OFF。G1历史8/8保持完成；当前回归属于G2。

| 当前验收 | 范围/结果 | 原始证据 |
| --- | --- | --- |
| G2.04 Owner pjdfstest | PASS，236文件/8819checks、28upstream TODO、0skip/意外失败 | [proof](r2/owner-pjdfstest-r2/artifacts/std-01-pjdfstest/proof.json) |
| G2.06 Owner LTP分支 | PASS，固定6/6；651库存未选，非全集/DFS | [proof](r3/owner-ltp-r1/artifacts/std-02-ltp/proof.json) |
| G2.07 Owner FSx分支 | PASS，seed1/1000，默认262144字节上限；非DFS/长时 | [proof](r2/owner-fsx-r1/artifacts/std-03-fsx/proof.json) |
| G2.09 Owner小读 | 功能PASS，性能FAIL：0.5709×ext4，目标0.90 | [结果/判据](r4/README.md) |
| G2.10 Owner小写 | 功能PASS，性能FAIL：0.6297×ext4，目标0.90 | [结果/判据](r4/README.md) |
| G2.11 Owner小删除 | 100×4KiB，正确性及对照报告出口PASS，无新增比例门槛 | [结果/判据](r4/README.md) |

**DFS当前分支：** [原结果及范围](r5/README.md)：pjdfstest FAIL（236脚本、0 TAP断言）；LTP固定6项6TBROK，均因目标statfs ENOSYS。短FSx seed1/1000 PASS，无mismatch。两个失败是产品容量接口缺口，未改套件/判据，未修环境或重跑；后续准入补目标workspace statfs/df -PT，停止依赖套件。

18条Linux源/工具门禁已通过，产品输入未变直接[复用](../../checkpoints/20261006-current/results/README.md)。历史g1.5/v37/v48见[检查点](../../current-checkpoint.md#historical-evidence)，不能继承当前PASS。单机Owner/DFS各64MiB与中心local-file有序恢复研究区r1已有证据，此处尚未携带完整安装切片；随后[双VM原始结果](crossvm-r1/README.md)通过Owner核心/中心恢复及DFS R2一写两读/中心恢复，34项核验和正常stop/卸载证明关闭G2.08限定出口。DFS G2.21性能比较仍待验收。

便携拷贝保留结果/日志，省略六个kirk临时latest绝对路径软链；原始链和全部输出在研究区r3保留。r3容量误查虚拟root的测前错误、随后实际guest ext4准入均保留。r4原驱动exit0表示所有case已尝试，性能明确FAIL；审计复核结果，无benchmark重跑。[MooseFS持久写资格](../../acceptance/baselines/README.md)仍BLOCKED，只阻塞对应比较。

**同源码新release：** [独立结果](release-r1/README.md)Owner完整pjdfstest/64MiB基础及单机中心恢复PASS；普通小读/写功能PASS、性能FAIL0.3477/0.5879×ext4，按用户最新优先级保存摸底数据、暂缓专项优化。release未运行DFS标准/双VM/LTP/FSx，不继承dev ELF资格。

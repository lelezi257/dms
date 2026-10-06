# DFS 当前标准分支

2026-10-06，产品e925c5bcf0408851ebfa08a59df29953374da9e9，与Owner相同已核对ELF和Linux6.8/ext4 VM、独立DFS FUSE mount、R1/local-file Meta/gRPC、bind OFF；不是三副本或一写多读性能。

| 小项 | 原结果/范围 | 证据 |
| --- | --- | --- |
| G2.05 pjdfstest | FAIL/exit1；236脚本启动结束，0 TAP断言、0子测试；不能称POSIX跑完 | [proof](dfs-pjdfstest-r1/artifacts/std-01-pjdfstest/proof.json)、[stderr](dfs-pjdfstest-r1/artifacts/std-01-pjdfstest/pjdfstest.stderr.log) |
| G2.06 固定LTP DFS分支 | FAIL/exit1；6选中/6调用，6 TBROK，0 TPASS/0 TFAIL/0skip，651库存未选 | [proof](dfs-ltp-r1/artifacts/std-02-ltp/proof.json)、[accounting](dfs-ltp-r1/artifacts/std-02-ltp/accounting.json) |
| G2.07 短FSx DFS分支 | PASS/exit0；固定seed1/1000、默认262144字节文件上限，1选中/1执行，无mismatch；非900秒×3seed长时 | [proof](dfs-fsx-r1/artifacts/std-03-fsx/proof.json) |

**事实：** 两个失败套件在初始化时调用目标statfs，产品返回ENOSYS；pjdfstest的df类型识别失败，LTP的tst_fs_type报告TBROK，未到对应POSIX断言。不是缺Git/编译器/容量或VM环境失败，不重建环境。当前DFS未实现statfs，默认容量接口不支持；不修改套件、伪造容量/文件系统类型或删失败来计PASS。

**准入遗漏与修正：** [预检](dfs-standard-preflight-r1.log)已核对依赖、版本/ELF、真实挂载和底盘容量，但未测目标workspace的statfs，不能称完整框架准入。今后标准项在启动前另检查workspace `statvfs`/`df -PT`；失败时一次记产品能力缺口，停止所有依赖该能力的套件。本次LTP的依赖判断有误，六个原TBROK全部保留，不重测。FSx为独立通过项。

下一项为跨VM核心功能/中心local-file恢复；DFS一写多读功能可独立继续。statfs修复为明确的标准验收前置小项，Owner性能缺口和MooseFS对照资格各自登记，不阻塞所有工作。

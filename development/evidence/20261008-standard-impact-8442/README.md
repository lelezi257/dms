# 8442 默认 OFF：标准影响核对及 Owner pjdfstest smoke

**当前限定PASS，不是完整POSIX或性能验收。** 源码8442b55e，158输入mapd053a333；普通包SHA31ba38e0，Meta e2bbf9bc / Node2094eadc，完整身份见[恢复审计](restore-audit.json)。G1历史8/8关闭，G2总数11限定完成/1bind进行中/15未验收不变，默认OFF。新Release仍BLOCKED，按用户决定等待GitHub恢复，本项没有发布重试或Cargo。

| 证据分类 | 版本与实际范围 | 状态/链接 |
| --- | --- | --- |
| 历史实跑通过，限定复用 | Owner e925 dev/release pjdfstest236文件/8819检查/28upstream TODO；固定LTP6/657、短FSx保留原scope。DFS0891 localR1全集保留原版本 | [Owner原审计与proof](../20261007-owner-standard-reuse/README.md)、[DFS原proof](../20261007-dfs-statfs/README.md)；未称8442实跑全集 |
| 当前代码影响核对 | f03→8442七compiler路径变化，14列出操作/协议/Store/blob未改；普通RPC旧方法未变，Node新poll仅bind ON启动。RootManager共有phase/admission变化也影响OFF，不能称所有OFF路径完全不变 | [原只读报告](impact-review.json)、[Linux158输入/Git blob核验](linux-impact-audit.json) |
| 当前受影响守卫实跑 | 8442 map59 scopedRust tests，其中22 RootManager；真实ON精确Home拒绝58检查 | [已有证据直接复用](../20261008-workspace-bind-root-command/README.md)，未重复Cargo/运行 |
| 当前普通包恢复实跑 | 8442/local-file/OFF Owner+DFS各64MiB，43谓词与正常Meta-only恢复 | [已过证据直接复用](../20261008-current-trial-8442/README.md)，不是R3或性能 |
| 当前标准smoke实跑 | Linuxafs-g1-clean/ext4，新隔离目录，固定pjdf d25636a/ELF83f27ae2，维护STD-01原smoke：open/00.t,mkdir/00.t,rename/00.t,mknod/00.t | **4文件/241 TAP PASS，0unexpected/skip/TODO；236发现/232未选**，非全集。Owner local/local-file/R1/两个workspace开关OFF |
| 当前待验/后置 | 8442全pjdf/LTP/FSx、remote/ON标准、三副本及性能不可由此小项升级 | 按影响范围复用旧证据；无无条件全集继承，不因文档升级重跑全矩阵 |

**事实：** 先核对Linux/架构/依赖/无编译器、套件Git HEAD及tracked diff、包/维护工具/ELF、端口/FUSE/ext4/容量/RAM与cgroup记录；复用已通过的标准环境，未修环境。维护driver原CLI help及测前固定4文件/180s命令保留。产品只运行一次，51运行谓词及15标准driver谓词通过；Meta/Node原ELF和inode始终匹配安装路径，2实际wait0/4child+supervisor PID、两个自有FUSE及原AFS进程/mount库存闭合。最终采样分配82,530,304B<512MiB，空闲5,678,874,624B>1GiB，非连续峰值。

**日志/失败：** 原日志23,327B/Node92ERRO，无WARN/TRACE，含NotFound、权限/负路径诊断；[分类和SHA](log-summary.json)，不称零错误或完整POSIX。首恢复审计用Python模块名导入了带连字符文件，ModuleNotFoundError/退出1已保存；仅修读证工具的加载方式，R2实际恢复及重新解析原TAP通过，无产品重跑/环境修补。[精确更正](audit-correction.json)。原子报告中的created_at是子任务日期占位而非实测时刻；Linux审计recorded_at才是本项实际核验时间，原记录不改写。

**恢复/索引：** 原始一份archive在源码树外，46178B/SHA29d618deddb4c385a4f888d91e9c609119b7355ea201fed8aa9478276719d015，[66外层成员索引](raw-index.json)，内含唯一89guest文本包。Linux实际临时解包、由固定main84a78d69恢复3维护工具（安装、standard、target identity），新编排工具只留一份；独立重新解析原TAP/清单/身份/退出回执通过。[恢复审计](restore-audit.json)。首FAIL+R2修正由base+精确replacement恢复，[10成员证明archive](restoration-proof-index.json) 2179B/SHAc4c6cd6fa41d4b8cf30cbe8b20185c23fa1d79df4e171476af62756b0fea0553，实际恢复PASS。无ELF、普通包、TLS私钥、磁盘镜像、整份源码或Python重复快照入Git。

**下一独立小项：** 核对8442对已过DFS一写多读/三同步副本正常恢复的实际输入与路径影响，未变化的证据限定复用；只补确实受影响范围。普通性能点优化、3FS比较资格、大规模/复杂可靠性、etcd/Redis仍后置；新Release暂缓不阻塞独立E2E。

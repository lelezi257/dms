# DFS 标准证据复用与准入分类纠正

2026-10-07。**新增完成的是证据审计，不是标准套件重跑。** G1历史8/8、G2计数及全部原始PASS/FAIL不变。产品仍6d51aeb/157输入。

| 分类 | 版本、范围和证据 |
| --- | --- |
| 历史版本实跑PASS；当前限定复用 | 0891cbfe对应154输入/map6161e25b；afs-g1-clean、DFS/local-file/gRPC/本地R1/native OFF：pjdfstest236文件/8819检查/28上游TODO、0skip/意外失败；[原proof](../20261007-dfs-statfs/runtime-r2/std-01-pjdfstest-full/artifacts/std-01-pjdfstest/proof.json) |
| 历史版本实跑PASS；当前限定复用 | 同一旧版本、固定LTP6/657，6PASS/651未选；[原proof](../20261007-dfs-statfs/runtime-r2/std-02-ltp-smoke6/artifacts/std-02-ltp/proof.json)。不称完整LTP |
| 当前版本实跑PASS | 6d51aeb/map66dbbe3e默认OFF安装、DFS64MiB读写、中心local-file Meta有序恢复及正常关闭：[当前35checks](../20261007-installed-off-6d/results/result.json)、[独立postcheck](../20261007-installed-off-6d/postcheck.json)；不称标准套件实跑或statfs补验 |
| 当前需要独立验收 | native ON、远端/RN标准资格、集群容量和系统性能比较；本审计均不覆盖 |

## 复用依据

[输入审计](reuse-audit.json)逐文件比对固定Git提交：旧154与0891、新157与6d均零hash差异；148项相同，3新增、6修改。只保存变更列表、既有证据的路径/SHA/大小，不复制旧源码、README或proof。

DFS、FUSE、locks、localfs、chunk/replication、Meta local-file、peer RPC以及Cargo/toolchain未变。[配置/启动diff](off-startup.diff)及[真实Git命令](diff-command.json)表明新增native校验和worker在ON分支内；开关默认false，历史标准配置没有native TOML表。DFS构造未改。无条件REST state.clone保留同一Arc，当前6d OFF有序退出/挂载消失已经实际覆盖。已有[config7/Nodehealth20/FUSE8](../20261007-native-workspace/r3/contracts.log)及6d受影响源码门禁保留原范围。因此复用旧DFS本地R1标准结果，不为新ELF机械重跑全集。

**边界：** 这不是“6d实跑236/8819和LTP6”。OFF也不意味着任意malformed native TOML不解析；本复用要求该表缺省。若后续确实需要当前6d实际statfs回执，可在已有R1夹具上单列小检查；不从安装自检推导它已通过。

## 准入纠正

本轮初查误用了afs-g2-micro，并把已停止旧fixture/PID/mount不存在写成BLOCKED。正确标准VM是afs-g1-clean；旧r2的[正常closure](../20261007-dfs-statfs/runtime-r2/closure.json)已经停止进程并移除mount，缺少旧PID属于正常历史收尾。**纠正的是这次检查器分类，不修改任何旧产品失败。**

[纠偏摘要](admission-correction/admission-correction.json)和raw/下5组只读命令的20个文件核对了afs-g1-clean的Linux ARM64/ext4/容量、固定pjdfstest和LTP pin/二进制、依赖及当前6d安装ELF。没有跑标准、启动服务、安装或修环境。当前tarball别名缺失，但固定安装prefix存在；若未来需重安装，可复制已有固定包，这属于正常setup。新执行仍需fresh output/Meta/Node/挂载和活身份，不能复用历史PID。

原误判/raw及先前复制的历史快照原样保留在源码树外`evidence/afs-delivery/dfs-standard-6d-20261007-r1/admission/`。当前Git仅携带新增纠偏原始命令、摘要、复用索引和差异，不增加旧证据副本。下一项继续独立E2E；本地R1标准无需重跑。

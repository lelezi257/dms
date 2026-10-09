# G2.14 一次远端 READ CPU 诊断收口

事实：在 main5d83 / 产品53e、Meta650、Node b62 下，复用原64MiB文件，B真实底层workspace bind ON、C远端FUSE、ctl local-file；1预热在窗口外，固定16次读在单个C CPU窗口。官方C142 perf6.8.12，499Hz/DWARF8192、120s/16MiB上限，无第二次采样。

- 功能：LIMITED_PASS；17次完整内容、1024个正式pread区间、每次B远端read计数增量64MiB。真实ext4 bind源/目标身份、权限拒绝、ENOENT、EOF通过。
- 诊断：843样本，lost0；窗口15.055597499s，Node CPU1.86s；16次probe累计wall3.022950919s。窗口包含逐次校验/计数/控制间隔，12.35% CPU/window不是纯读取CPU比例，也不能推断等待耗时。
- 叶样本最高net_rx_action 98/843（11.63%）、AES-GCM解密77/843（9.13%）；未证实主导第一方热点，不据此删除TLS、权限、错误检查或改短锁。不再采样。
- 仅受采样影响的摸底：probe中位342.444316MiB/s，独立p50/p95/p99 2.387873/5.546373/6.860288ms。没有同条件Moose对照，不能拼接旧参考计算比例、声称改善或关闭G2.14。此前正式吞吐FAIL/p95PASS原判保持。
- 退出恢复：三个产品actualwait0、一个profiler actualwait0及三control ACK；原ELF/helper/run/log inode、64MiB数据SHA/mtime/ctime/权限、boot/mount/保护库存恢复通过；采样/监督PID身份已消失。

三份Linux生成的guest归档在主机保存并实际Linux解包核对文件SHA/文件模式；未声称归档恢复原inode。按归档逐文件映射仅回收本轮可恢复ELF、perf.data和临时归档，共162973518B逻辑字节，所有紧凑结果/日志/失败记录保留；原数据和历史实例不动。具体档案SHA、成员数、前置失败及日志计数见[result.json](result.json)。两次产品启动前工具失败及首归档审计失败均保存，不算环境阻塞；产品只启动一次。

原始材料保留源码树外，入口和校验见[raw-index-pointer.json](raw-index-pointer.json)：冻结合约/runner版本、Linux guards、命令stdout/stderr、1024区间、perf report/script、guest原始perf.data归档、实际wait、恢复及清理记录。没有复制Python/ELF/大档案入Git。

决策：本轮无产品源码改动、无新包，不重复采样，不扩大本地矩阵。下一回到既定DFS一写多读，先选择有既有证据支撑的一项产品成本；DFS写对照/3FS资格及A/ctl维护继续按用户决定后置。G1历史8/8、原G2 12/0/15不变；本次只关闭诊断子项。

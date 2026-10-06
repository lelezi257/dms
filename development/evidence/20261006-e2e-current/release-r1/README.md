# 同源码 release 切片：结果保留，普通性能专项暂缓

源码e925c5bcf0408851ebfa08a59df29953374da9e9，154编译输入map `b850fcf4ab8882f9428dc1094a6e52647ddea465a038666467c6f1542487a068`。与原dev同源，新release ELF独立身份。local-file Meta/gRPC/FUSE，bind OFF，单VM/R1；不继承dev二进制资格。

| 项 | 当前结果/原始证据 |
| --- | --- |
| Linux release构建 | PASS，Cargo1.95.0，`cargo build --release --locked --offline --all-features --bins -j 2`，122秒，154输入前后一致；[build-proof](results/build-proof.json)/[命令](results/build.command.json)/[capacity](results/capacity.jsonl)；这是构建PASS，不是产品验收 |
| Owner完整pjdfstest | PASS，固定d25636a，236选中/执行/完成，8819checks、28upstream TODO、0skip/0意外失败，含62UID/GID降权文件；[proof](runtime/results/owner-pjdfstest-release-r1/artifacts/std-01-pjdfstest/proof.json)；非完整POSIX/DFS |
| Owner64MiB基础/中心恢复 | PASS，新release 64MiB basic；独立workspace确认写、目录fsync，仅Meta重启后完整内容/EOF读回；[basic](runtime/results/owner-basic/manifest.json)/[恢复](runtime/results/recovery-after.json)；非双VM/Node崩溃矩阵 |
| G2.09普通小读 | 功能PASS、性能FAIL；Owner3997.12/ext411496.38MiB/s，0.3476847<0.90；[5对原数据](runtime/results/owner-small-release-r1/seq-read.json) |
| G2.10普通小写 | 功能PASS、性能FAIL；Owner1064.52/ext41810.80MiB/s，0.5878720<0.90；[5对原数据](runtime/results/owner-small-release-r1/seq-write.json) |
| 正常关闭 | PASS，Meta/Node managed exit0、原进程退出、两mount卸载；[cleanup](runtime/results/recovery-stop.json)、run/生命周期回执 |

release meta SHA `9cfdf8f01def1a45383baea834598a0a6ff572f3a2a0b4aa9af6a1523963bea9`，node SHA `926ada2800093b5b7682ed37a978549eaf2704a25b5ebfc7cc346244cbd689a7`。原dev ELFs及其PASS/FAIL均保留原范围；release DFS标准/双VM/固定LTP/FSx没有新跑，仍需要受影响回归。

**固定性能范围：** 同旧C工具SHA `a2a56e4cddecf4129f32d449562e7455a050fdd0d018f19f716ced7bd06d6054`，复用同一paired I/O实现，不改64MiB/C1/1MiBblock/5对交替顺序/seed/内容EOF校验/fdatasync/0.90判据，0噪声豁免、0重复；只改profile/ELF/专属root。delete原限定结果复用，本轮不重跑。buffered preparation，非cold/实证hot；读时间含内容验证，写计open/write/fdatasync/close，dirsync/fresh-open verify在计时外。驱动exit3明确是性能FAIL。

**准入：** 构建VM11项、运行VM测前21项+挂载后4项准入通过，套件pin/Git/prove依赖/工具及ELF SHA/动态库/TLS/端口/真实ext4容量一次核对；Owner目标workspace df/statfs开suite前验证。原缺Git和DFS statfs失败不改。本轮没有环境阻塞或扩盘/重配。

**用户最新决策：** pjdfstest优先功能完备性；性能第一优先级Issue42/PR43容器挂载workspace访问（必要功能/安全→性能，默认OFF）。其它普通性能本轮摸底结束，未达标数据保留，暂缓专项优化，不刷成绩。先高优先级仓库证据/原版fuser整改，再回此E2E顺序。G1历史8/8不重开；G2.27未交付。

可携版本省略工具ELF及本地编译产物，按SHA指向冻结工具/构建结果；只保留命令、原始输出、身份/校验和及少量独立runner。大ELF不入Git。

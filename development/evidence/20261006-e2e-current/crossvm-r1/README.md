# 当前候选双 VM 核心验收

**事实：PASS（功能/中心有序恢复限定范围）。** 源码 `e925c5bcf0408851ebfa08a59df29953374da9e9`；同一 dev/unoptimized ELF，未修改产品输入。local-file Meta/gRPC/FUSE，bind OFF。G1历史8/8不重开。本轮关闭G2.08的核心组合出口；G2.21仅功能分支通过，性能项仍待验收。

| 新通过项 | 范围及原始证据 |
| --- | --- |
| Owner 双节点读写 | A创建64MiB并sync/close，B完整读回；B修改2MiB并sync/close，A精确读回；[A初始manifest](node-a/results/owner-a-write/manifest.json)、[B修改manifest](node-b/results/owner-b-patch/manifest.json) |
| Owner 中心恢复 | Home=node-a，目录fsync，只有Meta PID/starttick改变、两Node/挂载不变；两端重开64MiB内容/EOF一致；[身份/屏障](node-a/results/before-meta-restart-identity.json)、[恢复后](node-a/results/after-meta-restart-identity.json)、[B读回](node-b/results/owner-b-after-meta/manifest.json) |
| Owner 远端rename/delete | B rename，A旧名消失/新名全内容正确，A删除并目录fsync，B两名字均消失；[A内容/删除](node-a/results/owner-a-rename-read-delete.json)、[B可见性](node-b/results/owner-b-delete-visible.json) |
| DFS 一写两读 | R2/2/2，A确认64MiB，两台VM两个独立Node读者完整读取相同manifest；观测窗口重叠约12秒。非R3/3FS性能资格；[读者A](node-a/results/dfs-reader-a-process.json)、[读者B](node-b/results/dfs-reader-b-process.json) |
| DFS 中心恢复 | 目录fsync，只有Meta重启；两个Node再次完整读回相同64MiB/EOF；[A读回](node-a/results/dfs-reader-a-after-meta/manifest.json)、[B读回](node-b/results/dfs-reader-b-after-meta/manifest.json) |
| 正常关闭 | A Meta/Node、B Node均managed stop=0；原进程/starttick消失，四挂载已卸载；[A cleanup](node-a/results/cleanup-proof.json)、[B cleanup](node-b/results/cleanup-proof.json)。run/保留processctl生命周期exit回执，PID诊断文件保留不表示进程存活 |

汇总[core-proof](node-a/results/core-proof.json)34项；关闭核验A8/B6项。审计读取原始证据，不重跑I/O。Owner修改后SHA `d6f76d374e0df094f5455a2fffe9bdc3011b00db6ba30b56132f3745acc0a66f`，DFS SHA `3f45dcf1bd0c5be241ccd846c3fc66c05fec8b7b1557bb054078c3969fea94eb`。

## 身份与准入

- afs-meta SHA256 `38f0e76a5b4c4afc3efdde3ee7bfa96b0e7e01a0403d556343ec385a86b20d45`；afs-node `04b68193d7cdd04dea8c861f07e47336be05281de11a91e9ea8f3890123d1900`。
- A/Meta：afs-g1-clean，192.168.109.3、2CPU/2GiB、Linux6.8.0-106；B：afs-build，192.168.109.1、4CPU/8GiB、Linux6.8.0-142。guest ext4；新runroot `/var/tmp/afs-e2e-cross-20261006-r1`，不覆盖原单机状态。资源差异允许此功能验收，不作公平性能对照。
- [A preflight](node-a/results/preflight.json)/[B preflight](node-b/results/preflight.json)：必要工具/ELF/动态库、FUSE、双向TCP、空闲专属端口、TLS SAN/trust、配置及底盘≥2GiB余量检查。准入本身不是产品PASS。
- A使用原已安装r1 prefix；B引用相同已核对build ELF及原打包processctl/selfcheck，不称B独立完整离线安装完成。
- [公开配置A](node-a/results/node.toml)/[B](node-b/results/node.toml)及SHA保存；TLS私钥/CA私钥未带入证据。Meta20400/20401，Node20500/20501。

## 复现范围与纠错

`afs-selfcheck --phase write|read|patch --size 64MiB --case-id g2-cross-owner-e925c5b` 在Owner工作空间g2-cross-owner执行，read/patch使用对端已确认manifest；DFS case-id `g2-cross-dfs-e925c5b`，工作空间g2-cross-dfs，write确认后两个Node read。各selfcheck deadline180/外部timeout210；managed服务操作timeout100。保留逐phase log/exit/manifest、进程命令与公开配置。[原执行核验脚本](runner/audit-cross.py)、[准入](runner/cross-preflight.py)、[身份/屏障](runner/cross-identity.py)、[DFS读者](runner/cross-reader.py)、[关闭核验](runner/cross-cleanup.py)。

第一次查询Home错把workspace名当RootId得到404，随后用生产编码`root-`+workspace UTF-8 hex查询；不计产品故障。第一次关闭核验错误要求删除PID文件；processctl刻意保留诊断/回执，改为核对原/proc starttick消失和managed exit=0。两次核验纠错没有修改产品、环境或I/O判据，没有重跑工作负载。

**边界：** 非完整POSIX、Node崩溃恢复、MooseFS持平、三同步durable/3FS持平、bind ON或G2.27性能包。DFS标准statfs ENOSYS失败及原本地性能FAIL均保留。新文档/证据目前在本地工作树，尚未新增GitHub提交。

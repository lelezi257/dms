# d47 bind ON 与远端 FUSE：当前组合限定回归

**功能：LIMITED PASS；测量：NOT RUN；正式性能：PENDING。** 本轮只关闭 G2.12/G2.14/G2.15 下当前发布组合的必要远端功能回归，不关闭普通 OwnerFs 性能或整个 G2.27；G1 历史8/8、原 G2 12/0/15 不变。

## 候选与范围

[测前合同](contract.json)：main `94193b59068183c980f865d50dad57555e871bd5` 的160编译输入与已发布 d47 包完全一致，Linux 核对 map `ec7e2de5ea5e5b83883725fe4175b819af75cd4713c25e0479bb275ff5bab300`。实际 Meta `650bd9714da0293c61b6aff80ce86aede1152a233edb1506ed2d97d5b53f3e58`、B/C Node `d14e3182260e0f7ab51c82c4f18b9e2deab6446cd2980a6787fa43c22e9361b4`；三角色实际 `fs=all`，两 Node 同时有 OwnerFs/DFS mount。本轮未进行 DFS 文件操作。

沿用原 ctl/B/C ARM64 Linux/ext4 小夹具、TLS/端口和 local-file Meta；仅正常停止的自有配置/辅助工具/ctl Meta 暂时替换，原文件按 inode 备份并在退出后恢复；没有重建 VM、扩盘、安装依赖或 Rust/vendor 修改。原64MiB性能文件不修改，不新增一份大数据。

B 的底层 `state/node/ownerfs/root-776f726b7370616365-e1` 真实 ext4 目录覆盖 FUSE 根的一级 `workspace`；源/目标 dev+inode 相同，C 的 workspace 无 bind 覆盖，实际通过远端 FUSE。24 个独立语义操作验证 bind→remote fresh-open、remote→bind 写后关闭可见、4KiB/64KiB扩缩容、创建、重命名、新旧路径、双向删除、chmod000/0600、uid502拒绝和 EEXIST/ENOENT/EACCES。写入有 fsync+close+parent-fsync，读取验证完整字节、长度和 EOF。[结果](result.json)。

## 退出、证据与空间

三个真实监督生命周期 actual wait0、六 PID 消失、自有 bind/两后端 FUSE/UDS 清理；三保护进程及完整旧 mount 库存不变。[退出](closure.json)、[原 ELF/config/helper 原 inode/权限/校验和恢复](restoration.json)、[原64MiB数据 inode/mtime/ctime/全SHA保留](payload-retained.json)。本轮无驱动失败、未重跑产品。

[Linux 独立保存数据审计](audit.json)核对实时回执；预算 ctl256MiB/B-C512MiB，采样峰值分别114,532,352/214,638,592/147,513,344B，卷保留512MiB/4GiB通过。日志保留原文件：累计27,557B/13ERRO/66WARN/no TRACE；仅当前运行时间窗6ERRO（ENOENT和不支持的security xattr）/1WARN（未知FUSE opcode52），不声称零错误或完整POSIX。

[原始档案](archive.json)663成员、226,903B，SHA `f783ca7245be10dad229e377c67932feffe0bcffe2ff6bd121ac66a9a0f44c41`；[Linux 实际解包/662文件SHA、两工具差异及已发布包Meta恢复](archive-restoration.json)通过，不是产品重跑。当前树仅保留紧凑回执、索引及两份差异：[证据索引](evidence-index.json)、[工具引用](tool-references.json)、[差异恢复映射](tool-deltas.json)。完整命令、日志、原始工具在源码树外档案；Meta不重复入档，从已发布包恢复。

[归档恢复后清理](staging-cleanup.json)仅27个本轮暂存、47,527,636逻辑字节/47,579,136分配字节（约45.4MiB），包含 ctl/build/主机三份临时Meta及小工具；原 prefix/state/logs、生命周期及历史失败保留。没有以回收或检查数增加产品验收数。

## 复用与未完成

当前包单机 ON/local-file 正常全停恢复见[独立交付](../20261009-workspace-bind-on-trial/README.md)；b80历史跨节点恢复、标准与八项 bind 性能保持原身份。本项补新Meta组合跨节点功能，不追溯修改这些结论，不证明 crash/live-Meta-only/通用撤权/复杂append锁watch、全POSIX或性能。

[历史 d14+旧Meta 读测量](../20261008-owner-remote-direct-read/README.md)和[WRITE改善](../20261008-owner-remote-write-server-pair/README.md)不重标本轮新Meta/combined成绩。普通远端吞吐>=1.2×Moose且独立预定p95<=0.8×仍 PENDING；Moose旧官方文件所有权准入及隔离部署选择未解决，对照不启动、不重复只测Owner。DFS写仍按用户暂缓。下一恢复正式Owner远端对照前解决已登记的Moose部署选择；普通本地、大规模和复杂可靠性后置。

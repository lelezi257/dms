# AFS 6d51aeb：默认 OFF 试用交付清单

这是 Linux ARM64 的限定试用候选，OwnerFs、DFS 都可安装和运行。
当前实测为 Ubuntu24.04 / Linux6.8 / guest ext4 / local-file Meta / gRPC / R1。
容器 native/bind 保持 OFF；当前普通读写性能尚未达标，完整阶段二仍进行中。
历史阶段一g1.5的8/8结论保留。

## 下载与版本核对

从 [固定试用版本](https://github.com/lelezi257/dms/releases/tag/afs-trial-6d51aeb)
下载归档、SHA256SUMS和TRIAL.md。该版本标记prerelease，不替换正式Latest。

- 包：`afs-0.1.0-g2-procfree-6d51aeb-linux-aarch64.tar.gz`，14,728,435字节。
- SHA256：`ee25d5892c4e884e67c86d4e5b9c6ab551af4c46649edab6a05da06659d1aff9`。
- 产品源码：`6d51aeb45c1ed8669d80f612b3817e6d1bdabe04`，157编译输入map66dbbe3e。
- Meta：`2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e`。
- Node：`2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645`。

在下载目录核对并解包：

```sh
sha256sum -c SHA256SUMS
tar -xzf afs-0.1.0-g2-procfree-6d51aeb-linux-aarch64.tar.gz
cd afs-0.1.0-g2-procfree-6d51aeb-linux-aarch64
sha256sum -c SHA256SUMS
```

运行机器无需Cargo、Rust/C编译器或Git。提前满足包内DEPENDENCIES.md：
FUSE3和`/dev/fuse`、bash/coreutils/Python3、findmnt/ss/curl/openssl及ldd.txt列出的共享库。
使用至少1GiB空闲的guest ext4目录；不要将试用数据放在macOS共享目录。
下面使用新目录和独立端口22400/22401/22500/22501；目录或端口已占用时改用预先核对过的空闲位置，不覆盖已有部署。

## 安装、读写和中心恢复

在解包目录执行；`trial_root`指向新试用根目录：

```sh
trial_root=/var/tmp/afs-trial-6d
sudo ./install.sh --prefix "$trial_root/prefix" --config-dir "$trial_root/etc" \
  --state-dir "$trial_root/state" --run-dir "$trial_root/run" \
  --log-dir "$trial_root/logs" --mount-root "$trial_root/mount"
sudo "$trial_root/prefix/bin/afs-trial-config" single --backend local-file \
  --config-dir "$trial_root/etc" --state-dir "$trial_root/state" \
  --run-dir "$trial_root/run" --mount-root "$trial_root/mount" \
  --meta-grpc-port 22400 --meta-rest-port 22401 --node-grpc-port 22500 --node-rest-port 22501 --force
sudo "$trial_root/prefix/bin/afs-processctl" --prefix "$trial_root/prefix" \
  --config-dir "$trial_root/etc" --run-dir "$trial_root/run" --log-dir "$trial_root/logs" start all

for fs in ownerfs dfs; do
  sudo "$trial_root/prefix/bin/afs-selfcheck" --mount "$trial_root/mount/$fs" \
    --workspace trial-6d --size 64MiB --phase write --case-id "trial-6d-$fs" \
    --deadline 180 --output "$trial_root/$fs-write.json"
  sudo sync -f "$trial_root/mount/$fs/trial-6d"
done
sudo "$trial_root/prefix/bin/afs-processctl" --prefix "$trial_root/prefix" \
  --config-dir "$trial_root/etc" --run-dir "$trial_root/run" --log-dir "$trial_root/logs" restart meta
for fs in ownerfs dfs; do
  sudo "$trial_root/prefix/bin/afs-selfcheck" --mount "$trial_root/mount/$fs" \
    --workspace trial-6d --size 64MiB --phase read --case-id "trial-6d-$fs" \
    --deadline 180 --input "$trial_root/$fs-write.json" --output "$trial_root/$fs-read.json"
done
sudo "$trial_root/prefix/bin/afs-processctl" --prefix "$trial_root/prefix" \
  --config-dir "$trial_root/etc" --run-dir "$trial_root/run" --log-dir "$trial_root/logs" stop all
```

每一步要求退出0，两个write/read结果均为PASS、64MiB且SHA一致；状态报告应为正常停止。
`stop`保留状态、配置和日志，重新启动后可继续使用；不要用强杀或lazy卸载代替正常停止。
上面的64MiB自检是安装/基础操作/有序中心恢复检查，不是完整POSIX或性能认证。
普通用户工作目录授权和双节点配置参考[通用试用指南](trial.md)。

## 已验证与未完成

- [当前独立安装/恢复](../../development/evidence/20261007-installed-off-6d/README.md)：无编译器VM，35checks、两个64MiB内容/EOF、中心有序恢复、执行文件及mount身份、正常清理PASS。
- [同包复现](../../development/evidence/20261007-off-trial-handoff/README.md)：固定Git输入和已有ELF，Linux umask077重打包与已测原包全字节一致；没有重新编译Rust或替换原包。
- 当前Owner/DFS标准历史证据各保留其原源码/ELF身份，见[验收主表](../../development/trial-release-goals.md)，不宣称当前ELF跑过标准全集。
- [容器小性能](../../development/evidence/20261007-container-perf/README.md)仅摸底；[混合append](../../development/evidence/20261007-append-diagnostic/README.md)内容通过但偏移失败，经典锁/watch也有失败。此包不含实验helper/controller，保持native OFF。
- R2官方fuser迁移仍受公开API缺口阻塞，不能将本包当成原版依赖迁移完成证明。
- 完整G2.27性能、跨节点当前候选矩阵、大规模、复杂可靠性、etcd和Redis仍在任务列表，不由本清单宣布完成。

包内原指南记录25a8061旧候选，通过结论保持原版本；本清单与当前6d51aeb回执配套使用。
反馈时附包SHA、内核/架构、执行命令、退出码、两个write/read JSON和本次logs，保留失败原文。

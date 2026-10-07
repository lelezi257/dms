# AFS 默认 OFF 试用候选 f03

Linux ARM64 候选版本为 `0.1.0-g2-main-f03dc2b`，产品源码
`f03dc2b3679c31daa51caee275fb2087413e949c`。`manifest.json` 标明版本、编译特性和
两个产品 ELF 的 SHA256；包内及下载侧 `SHA256SUMS` 分别校验文件和归档。
运行机器无需 Cargo、Rust/C 编译器或 Git。包仅含普通产品二进制、安装、配置、
生命周期、自检和说明，不含测试探针、源码、容器 rootfs 或私钥。

本指南提供复现步骤。试用包安装/恢复与标准、性能验收分别记录；完整阶段二性能仍待验。
[当前验收主表](https://github.com/lelezi257/dms/blob/main/development/trial-release-goals.md)
是状态入口。[历史7e6交付](https://github.com/lelezi257/dms/releases/tag/afs-trial-7e6e00a)
及[原指南](https://github.com/lelezi257/dms/blob/7eedd7eca3c1fb810663b3a491b35b18b9650f03/docs/guides/trial.md)
保留原包、判据和结论，不使用新候选结果追溯升级。

## 环境和校验

使用 Linux ARM64、FUSE3及 `/dev/fuse`，测试数据放本机 ext4。
依赖按 `DEPENDENCIES.md` 和 `ldd.txt` 核对；至少预留1GiB可用空间，确认四个端口和
新根目录。必要条件不足时停止受影响步骤并保留输出。

```sh
sha256sum -c SHA256SUMS
package_dir=afs-0.1.0-g2-main-f03dc2b-linux-aarch64
tar -xzf "$package_dir.tar.gz"
cd "$package_dir"
sha256sum -c SHA256SUMS
cat manifest.json
ldd bin/afs-meta
ldd bin/afs-node
```

第一条使用下载侧校验文件，第二次使用包内校验文件。共享库缺失时先停止，
不要把安装成功当作服务运行或性能验收通过。

## 单节点安装及中心正常重启恢复

根目录必须是新目录，22400/22401/22500/22501端口必须空闲。
OwnerFs与DFS使用两个独立挂载；本步骤使用local-file Meta、gRPC和DFS R1。
OwnerFs workspace bind的宿主入口与实验适配器均默认 OFF。

```sh
trial_root=/var/tmp/afs-trial-f03
test ! -e "$trial_root" || exit 1
sudo ./install.sh --prefix "$trial_root/prefix" --config-dir "$trial_root/etc" \
  --state-dir "$trial_root/state" --run-dir "$trial_root/run" \
  --log-dir "$trial_root/logs" --mount-root "$trial_root/mount"
sudo "$trial_root/prefix/bin/afs-trial-config" single --backend local-file \
  --config-dir "$trial_root/etc" --state-dir "$trial_root/state" \
  --run-dir "$trial_root/run" --mount-root "$trial_root/mount" \
  --meta-grpc-port 22400 --meta-rest-port 22401 \
  --node-grpc-port 22500 --node-rest-port 22501 --force
ctl() {
  sudo "$trial_root/prefix/bin/afs-processctl" --prefix "$trial_root/prefix" \
    --config-dir "$trial_root/etc" --run-dir "$trial_root/run" \
    --log-dir "$trial_root/logs" --timeout 30 "$@"
}
ctl start all
for fs in ownerfs dfs; do
  sudo "$trial_root/prefix/bin/afs-selfcheck" --mount "$trial_root/mount/$fs" \
    --workspace trial-f03 --size 64MiB --phase write --case-id "trial-f03-$fs" \
    --deadline 180 --output "$trial_root/$fs-write.json"
  sudo sync -f "$trial_root/mount/$fs/trial-f03"
done
ctl restart meta
for fs in ownerfs dfs; do
  sudo "$trial_root/prefix/bin/afs-selfcheck" --mount "$trial_root/mount/$fs" \
    --workspace trial-f03 --size 64MiB --phase read --case-id "trial-f03-$fs" \
    --deadline 180 --input "$trial_root/$fs-write.json" \
    --output "$trial_root/$fs-read.json"
done
ctl status all
ctl stop all
```

写入包含同步和关闭，读回核对完整内容与EOF。中心正常重启后已确认数据应可读；本步骤
不重启Node，也不证明断电、崩溃或多节点恢复。`stop`成功要求监督进程的实际退出回执；
保留配置、数据、日志、pid/identity/launch和退出记录，不靠删挂载或杀进程宣布通过。

自检覆盖小规模基本操作，是标准POSIX套件的补充。普通用户试用时由管理员仅在每个
挂载内创建并chown独立workspace，再以普通用户自检；不要开放整个挂载根。

## memory演示与多节点

一次性演示可将配置命令改为 `--backend memory`。Meta退出后namespace及幂等状态丢失；
生成器明确设置 `allow_volatile_meta=true`。需要恢复时使用local-file。
多节点使用包内 `afs-trial-config cluster`，先启动Meta再启动Node；默认R2演示不代表
三同步持久副本或3FS性能。参数与拓扑见
[操作指南](https://github.com/lelezi257/dms/blob/main/docs/guides/operations.md)。

## 当前范围和已知限制

- f03修复同父目录独立创建的时间戳竞争；同名、权限/属性变化和真实冲突仍保留错误。
- 同f03的[小规模三节点读写](https://github.com/lelezi257/dms/blob/main/development/evidence/20261007-dfs-r3-multinode-current/README.md)
  和[删除](https://github.com/lelezi257/dms/blob/main/development/evidence/20261007-dfs-delete-current/README.md)
  已有独立功能与计时数据。缓存未观察，正式3FS比较和独立时延证据仍待验。
- 普通OwnerFs读写要求同条件MooseFS吞吐≥1.2倍且独立操作时延≤0.8倍；现有历史或摸底
  结果不足以宣布新判据通过。删除只要求正确性和性能对照报告。
- workspace bind将物理Home目录覆盖OwnerFs FUSE一级目录，宿主入口独立于runc；
  两入口默认OFF，历史限定性能不自动升级新候选，完整ON/混合append、锁、watch仍待验。
- remote shared mmap需要内核协商 `FUSE_DIRECT_IO_ALLOW_MMAP`；普通远端读写使用direct I/O。
- 当前fuser保留历史私有差异，官方迁移的公开API缺口尚未解决，第三方原版整改未完成。
- gRPC为本试用默认通道；RDMA、大规模/长时、复杂可靠性、etcd和Redis按后续独立项推进。
- G1历史8/8保持关闭。新候选回归、历史标准、各性能小项按版本和范围记录。

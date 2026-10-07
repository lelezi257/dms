# AFS 默认 OFF 试用包

当前交付候选为 Linux ARM64 `0.1.0-g2-main-7e6e00a`，产品源码
`7e6e00a6e2d7fdf3c1d606ee743a016a419ec25d`。运行机器无需 Cargo、Rust/C 编译器或 Git。
包内 `manifest.json` 标明实际版本和两个产品二进制的 SHA256；`SHA256SUMS` 校验包内文件。
下载时同时保存归档校验文件和试用验收账本。历史
[6d51aeb 交付](https://github.com/lelezi257/dms/releases/tag/afs-trial-6d51aeb)
保持原版本和结论。

本包包含 OwnerFs、DFS 和普通安装、配置、生命周期、自检工具，不含测试探针、
容器 rootfs 或私钥。OwnerFs workspace bind mount 两个入口均默认 OFF。
新候选的实际结果以
[验收账本](https://github.com/lelezi257/dms/blob/main/development/evidence/20261007-current-trial-7e6/README.md)
为准；下方命令是可复现操作步骤，不代表完整 POSIX 或性能达标。

## 环境和下载校验

已验证的试用环境为 Ubuntu24.04 ARM64、Linux6.8、guest ext4。准备
`DEPENDENCIES.md` 所列 FUSE3、`/dev/fuse`、bash/coreutils、Python3、
findmnt/ss/curl/openssl 及 `ldd.txt` 共享库。预留至少1GiB可用空间，
测试数据放 guest ext4，不放 macOS 共享目录。检查将使用的四个端口和新根目录，
已有部署请继续保留。

```sh
sha256sum -c SHA256SUMS
package_dir=afs-0.1.0-g2-main-7e6e00a-linux-aarch64
tar -xzf "$package_dir.tar.gz"
cd "$package_dir"
sha256sum -c SHA256SUMS
cat manifest.json
```

第一条命令使用下载侧归档校验文件，第二次校验使用包内校验文件。
确认 `afs-meta` 和 `afs-node` 的依赖全部存在，必要环境不满足时先停止试用：

```sh
ldd bin/afs-meta
ldd bin/afs-node
```

## 单节点安装、64MiB读写和中心恢复

下面的根目录必须是新目录；固定端口22400/22401/22500/22501应事先确认空闲。
生成的 OwnerFs 与 DFS 是两个独立挂载，DFS使用R1，Meta使用local-file。

```sh
trial_root=/var/tmp/afs-trial-7e6
test ! -e "$trial_root" || exit 1
sudo ./install.sh --prefix "$trial_root/prefix" --config-dir "$trial_root/etc"   --state-dir "$trial_root/state" --run-dir "$trial_root/run"   --log-dir "$trial_root/logs" --mount-root "$trial_root/mount"
sudo "$trial_root/prefix/bin/afs-trial-config" single --backend local-file   --config-dir "$trial_root/etc" --state-dir "$trial_root/state"   --run-dir "$trial_root/run" --mount-root "$trial_root/mount"   --meta-grpc-port 22400 --meta-rest-port 22401   --node-grpc-port 22500 --node-rest-port 22501 --force
ctl() {
  sudo "$trial_root/prefix/bin/afs-processctl" --prefix "$trial_root/prefix"     --config-dir "$trial_root/etc" --run-dir "$trial_root/run"     --log-dir "$trial_root/logs" --timeout 30 "$@"
}
ctl start all
for fs in ownerfs dfs; do
  sudo "$trial_root/prefix/bin/afs-selfcheck" --mount "$trial_root/mount/$fs"     --workspace trial-7e6 --size 64MiB --phase write --case-id "trial-7e6-$fs"     --deadline 180 --output "$trial_root/$fs-write.json"
  sudo sync -f "$trial_root/mount/$fs/trial-7e6"
done
ctl restart meta
for fs in ownerfs dfs; do
  sudo "$trial_root/prefix/bin/afs-selfcheck" --mount "$trial_root/mount/$fs"     --workspace trial-7e6 --size 64MiB --phase read --case-id "trial-7e6-$fs"     --deadline 180 --input "$trial_root/$fs-write.json"     --output "$trial_root/$fs-read.json"
done
ctl status all
ctl stop all
```

读回包含完整内容和 EOF 校验。中心正常重启后应能读回已确认文件；本步骤没有重启Node，
也不证明断电/崩溃或多节点恢复。`stop` 必须取得精确监督进程的实际退出回执才成功，
保留的 pid/identity/launch 文件是可追溯记录。停止后保留配置、数据、日志和回执，
不要把删除挂载或杀掉进程作为通过依据。

自检包含小规模创建、重开、追加、截断、重命名、删除、权限、经典锁和 mmap 检查。
它是试用自检，标准 POSIX 套件和性能用例另有版本化账本。普通用户试用时先由管理员
在每个挂载内创建并 chown 单独 workspace，再以普通用户运行自检；不要开放整个挂载根。

## memory 演示及多节点

一次性演示可将生成配置命令改为 `--backend memory`。Meta退出后 namespace和幂等状态
丢失；生成器显式设置 `allow_volatile_meta=true`。需要恢复时使用local-file，
不能通过跳过 readiness 检查绕过持久性要求。

多节点使用包内 `afs-trial-config cluster` 生成 Meta及各Node独立配置/TLS，
先启动Meta再启动Node。R2的默认演示不代表三同步持久副本或3FS性能对照。
完整参数及拓扑步骤见
[操作指南](https://github.com/lelezi257/dms/blob/main/docs/guides/operations.md)。

## 当前补充验收

同一7e6/c3bb包的[DFS R3内容与Meta正常恢复](../../development/evidence/20261007-dfs-r3-current-recovery/README.md)已有新运行证据；[workspace八个小规模核心性能case](../../development/evidence/20261007-workspace-bind-data-current/README.md)达到预设>=0.90×ext4，作为限定G2.13完成。开关仍默认OFF，完整ON功能尚未资格化；普通路径性能FAIL和历史标准身份保持分列。已发布包和附带原验收快照不覆盖，最新补充以这些版本化索引为准。

## 已知边界

- 本包尚未达到完整阶段二性能目标；普通读写失败和历史性能数据保留在账本中。
- workspace bind核心将物理Home目录覆盖OwnerFs FUSE一级目录；宿主入口独立于runc。
  容器适配器只是使用场景。两入口默认OFF，ON的广义排空和混合append/锁/watch仍待验。
- Remote shared mmap需内核协商 `FUSE_DIRECT_IO_ALLOW_MMAP`，普通远端读写保持direct I/O。
  旧内核remote shared mmap不在已验范围。
- gRPC是本试用默认数据通道；RDMA、复杂故障、长时间/大规模、etcd/Redis后置。
- 当前fuser仍保留历史私有差异；官方迁移存在公开API缺口，不能宣称原版依赖整改完成。
- 历史G1/g1.5的8/8保留；新候选回归、历史标准、各性能子项均按各自版本记录。

Current7e6 ordinary-package DFS single-reader observation: [64MiB/C1 A reads, five samples, three-copy proof and normal closure](../../development/evidence/20261007-dfs-r3-local-read/README.md). This is measured data with unobserved cache/RPC locality, not a qualified3FS comparison or a five-round write result; the first test-output preparation refusal remains linked.

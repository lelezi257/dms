# 试用说明

当前阶段性试用包：[afs-bind-a103a2f](https://github.com/lelezi257/dms/releases/tag/afs-bind-a103a2f)

该包用于迁移前快照的有限试用：证明当前 OwnerFs workspace bind ON 场景和 local-file Meta 正常重启恢复可以独立安装运行。它不是完整 G2 性能版本，也不是复杂可靠性版本。

## 身份

| 项 | 值 |
| --- | --- |
| 产品源码基点 | `a103a2f2e8e2bd89e7205b32fe4a2398fe047b4d` |
| 包大小 | `14,492,878` bytes |
| 包 SHA256 | `260d5009f343dd52e938e5659a6e9108be7cdbeb8c77124ff1696c4fdf1cf2b5` |
| 固定证据 | [20261009-workspace-bind-a103-trial](https://github.com/lelezi257/dms/blob/6b75d78a1c9350553770394b13384e01a9b292c0/development/evidence/20261009-workspace-bind-a103-trial/README.md) |

证据链接指向归档前的固定提交，保留原版本、原环境和原结论。当前源码快照不会自动继承它没有覆盖的新验收。

## 已验证范围

- Linux ARM64 包两次构建字节一致，并已发布到 GitHub Release。
- 无编译器环境可实际安装运行。
- OwnerFs workspace bind ON 当前场景可用：真实 Home 底层目录 bind 到 OwnerFs FUSE 根下对应一级 workspace。
- UID501 可读写，UID502 被拒绝。
- OwnerFs 64KiB 与 DFS 64MiB 在 local-file Meta 下正常全服务停止、重启后可读回。
- 五个 `actual wait0` 生命周期检查通过。
- 普通发行配置默认 OFF；ON 场景需要显式配置。

## 与其它证据的区别

- a103 包是试用交付包身份。
- Node217/Meta650 相关远端 smoke 证据属于此前组合，仍按原 Meta/Node 身份登记。
- 新 Meta 组合没有因此自动获得完整远端标准或性能通过结论。
- G1 历史 8/8 保持关闭；该包不重开、不重标历史结论。

## 环境与校验

Linux ARM64、FUSE3与`/dev/fuse`、本地ext4，管理员具备正常FUSE及bind mount权限。
依赖按包内`DEPENDENCIES.md`/`ldd.txt`检查，至少预留1GiB空闲及512MiB本项预算；
四端口22400/22401/22500/22501、新根目录和挂载须未占用。缺依赖、身份不符或容量不足即停止该项，保留记录。
目标机器无需Git、Cargo或编译器。包不含源码、测试探针、rootfs、私钥；TLS在配置生成时本地创建。

```sh
sha256sum -c SHA256SUMS
package_dir=afs-0.1.0-g2-bind-a103a2f-linux-aarch64
tar -xzf "$package_dir.tar.gz"
cd "$package_dir"
sha256sum -c SHA256SUMS
cat manifest.json
ldd bin/afs-meta
ldd bin/afs-node
```

第一条核对下载侧校验文件；第二次核对包内文件。安装成功不等于功能或性能通过。

## 单个固定Home的明确ON配置与启动

先以默认OFF创建一级workspace，令中心记录Home；正常停止Node后启用宿主bind。
首次启动前没有已确认的Home时，不应绕过授权去直接覆盖目录。
示例使用管理员预先分配的uid/gid501，实际部署须替换为工作负载的合法身份。

```sh
trial_root=/var/tmp/afs-trial-bind-a103
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
sudo mkdir -m 0700 "$trial_root/mount/ownerfs/workspace"
sudo chown 501:501 "$trial_root/mount/ownerfs/workspace"
sudo sync -f "$trial_root/mount/ownerfs"
ctl stop node
sudo python3 - "$trial_root/etc/node.toml" <<'PYCONFIG'
from pathlib import Path
import sys,tomllib
p=Path(sys.argv[1]);s=p.read_text();c=tomllib.loads(s)
assert not c.get('experimental_native_workspace',False)
assert 'experimental_ownerfs_workspace_bind' not in c and 'ownerfs_workspace_bind' not in c
# Generated flat TOML only. Existing customized configs must be edited explicitly.
assert not any(line.lstrip().startswith('[') for line in s.splitlines())
p.write_text(s+'\nexperimental_ownerfs_workspace_bind = true\nownerfs_workspace_bind = { workspace = "workspace" }\n')
PYCONFIG
sudo "$trial_root/prefix/bin/afs-node" --config "$trial_root/etc/node.toml" --print-config
ctl start node
sudo findmnt --mountpoint "$trial_root/mount/ownerfs/workspace"
ctl status all
```

有效配置必须显示`experimental_ownerfs_workspace_bind=true`、workspace名`workspace`，
实验容器适配器`experimental_native_workspace=false`。核心只支持配置指定的一级workspace。
Home底层真实`state/node/ownerfs/root-…-e…`覆盖本Home的FUSE一级目录，覆盖挂载为ext4，
不是将FUSE目录本身bind后宣称绕过FUSE。可以按证据中的dev/inode及mountinfo方法核验，不能只检查目录存在。

应用以授权uid访问`$trial_root/mount/ownerfs/workspace`，关闭并完成所需同步后，
其他Node在自己的普通OwnerFs FUSE挂载上新打开同一文件。权限由实际uid/gid/mode决定。
现有打开FD/mmap不保证即时刷新或即时撤权；应用应使用close-to-open。

## 两节点使用与远端范围

多节点先用包内 `afs-trial-config cluster --help` 生成同一 TLS 信任集，再按主机分发配置。先启动 Meta、Home B 和远端 C；在 B 的普通 OwnerFs FUSE 根下创建 workspace 后，仅正常停止 B 的 Node，按上文启用 Home bind。C 保持两个实验开关 OFF，通过自己的 OwnerFs FUSE 访问 B 的 workspace。数据保存在各自 guest ext4；非 Home 节点不得给同一 workspace 开启 bind。

## 正常停止与恢复

停止前先让应用关闭 FD/mmap，停止写入并完成文件及父目录持久屏障，再执行 `ctl stop all`。保留状态、配置、日志和退出回执；确认实际 wait0、bind/FUSE 与 UDS 消失后，以同一配置 `ctl start all`，重新打开并核验数据。不要通过强杀、删挂载或删状态宣布正常退出。该流程只覆盖正常全停恢复，不代表在线 Meta-only 重启或崩溃恢复。

## 不代表通过

该包不代表以下事项通过：

- 完整 POSIX 标准测试；
- 完整远端标准；
- OwnerFs 普通本地/远端性能双目标；
- DFS 对 3FS 的三同步持久副本性能对照；
- 复杂可靠性、crash recovery、多 Meta、etcd 或 Redis 后端；
- 官方 `fuser` 无私有补丁迁移。

更多当前边界见 [当前计划](https://github.com/lelezi257/dms/blob/main/docs/development/plan.md)。

## 当前使用限制

- 混合 native/FUSE 同时 append 的偏移、混合经典锁和 watch 传播仍有历史失败，当前不支持。依赖这些操作的工作负载不属于本次可用范围。
- 已打开 FD/mmap 的即时刷新、即时撤权，以及描述符转移或二级 clone 不在当前保证内；停止或变更 root/epoch 前先停止受管用户并排空引用。
- 撤权 issuer/durable ACK、扩展 namespace 和拓扑后置。控制面失联或授权失败不能绕过 Home/root/epoch 核验。
- direct-I/O mmap 必须经真实内核能力协商；配置不能替代内核支持。

关闭 bind 时，先停止受管用户、关闭 FD/mmap，正常停止 Node，再把 `experimental_ownerfs_workspace_bind` 显式改为 `false` 后启动。不得直接移除挂载或跳过排空来绕过授权失败。

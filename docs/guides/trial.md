# OwnerFs workspace bind ON 试用：b80

当前 Linux ARM64 包版本 `0.1.0-g2-bind-b80dab6`，产品源码
`b80dab66e9d819cad821c9b30edbf97c795c0c3e`。`manifest.json` 固定产品版本、
编译特性及 Meta/Node ELF SHA256；本指南和打包工具的提交、SHA另行记录，必须与编译输入映射核对。
普通生成配置仍默认 OFF；本次场景按下列步骤明确打开 **OwnerFs workspace bind mount**。
容器/runc只是可选使用者，启用宿主bind不依赖容器或测试探针。

[当前验收表](https://github.com/lelezi257/dms/blob/main/development/trial-release-goals.md)和
[b80真实B-Home宿主ON/C远端FUSE及正常恢复证据](https://github.com/lelezi257/dms/blob/149e8725e4fbb420f28b50452be57fb4ac562dd6/development/evidence/20261008-workspace-bind-remote/README.md)
分别记录功能、测量、性能、交付。该证据不是本包的独立安装证明；包的验证状态以其随附证据索引为准。
[8442 OFF原指南](https://github.com/lelezi257/dms/blob/149e8725e4fbb420f28b50452be57fb4ac562dd6/docs/guides/trial.md)、
[历史f03包](https://github.com/lelezi257/dms/releases/tag/afs-trial-f03dc2b)保留原版本/范围/判据，不能作为当前ON包安装结论。

## 环境与校验

Linux ARM64、FUSE3与`/dev/fuse`、本地ext4，管理员具备正常FUSE及bind mount权限。
依赖按包内`DEPENDENCIES.md`/`ldd.txt`检查，至少预留1GiB空闲及512MiB本项预算；
四端口22400/22401/22500/22501、新根目录和挂载须未占用。缺依赖、身份不符或容量不足即停止该项，保留记录。
目标机器无需Git、Cargo或编译器。包不含源码、测试探针、rootfs、私钥；TLS在配置生成时本地创建。

```sh
sha256sum -c SHA256SUMS
package_dir=afs-0.1.0-g2-bind-b80dab6-linux-aarch64
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
trial_root=/var/tmp/afs-trial-bind-b80
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

多节点用包内`afs-trial-config cluster --help`和[部署操作说明](https://github.com/lelezi257/dms/blob/main/docs/guides/operations.md)
生成同一TLS信任集并按主机分发配置；数据路径位于各自guest ext4，不使用宿主共享目录替代数据盘。
先启动Meta，再启动Home B及远端C；在B创建workspace后，仅B正常停Node并按上述方式ON。
C保持宿主bind与实验容器开关OFF，通过C的OwnerFs FUSE访问B的workspace。
不要在非Home C给同一workspace开启bind，授权失败必须保留其真实错误。

已验证当前固定Home场景：4/64KiB双向写-close/新open的SHA/长度/EOF、native rename后的
新名字与旧ENOENT、远端删除/native ENOENT、chmod000/0600、uid502拒绝及EEXIST不破坏内容。
这是小规模功能范围，不是完整POSIX、远端性能达标或任意多workspace拓扑。

## 正常停止、排空和local-file恢复

停止前让应用关闭FD/mmap、停止写入并完成所需文件及父目录持久屏障，然后`ctl stop all`。
保留状态、配置、日志和processctl退出回执，不靠杀进程、删挂载或删状态宣布关闭。
确认监督者实际wait0、bind/FUSE与UDS消失，再用同一配置`ctl start all`，新打开确认数据。
当前证据覆盖Meta与两Node全服务正常停止后重启；不等于在线Meta-only重启或崩溃恢复。
ON控制面失联/授权失败按现有策略fail-closed，须等待引用排空；不要绕过Home/root/epoch核验。
需要关闭该功能时，先排空并正常停Node，将宿主开关显式改false后再启动。

## 支持边界与性能状态

- 当前Home/一级workspace宿主bind与普通远端FUSE协同；管理员安装，应用非特权uid。
  默认配置OFF，本文明确ON，不支持把非Home、FUSE自身或任意外部目录冒充合法源。
- 一般混合native/FUSE同时append的偏移、混合经典锁和watch传播仍有历史FAIL，当前不支持；
  依赖这些操作的工作负载不能按本次小闭环宣布可用。生产撤权issuer/durable ACK、即时既有FD撤权、扩展命名空间及拓扑后置。
- RootCommand精确接收/拒绝、源身份/epoch变更与FD/mmap排空分别保留原版本证据。
  direct-I/O mmap仍必须经过真实内核能力协商，不能通过配置冒充能力。
- 八项历史bind小核心case以>=0.90 native ext4限定复用；当前名字缓存修复不重标那些版本的性能。
  普通远端读写仍须吞吐>=1.2同条件MooseFS且独立时延<=0.8；本地同判据但后置。
  DFS一写多读在同接口/三份同步持久副本下持平3FS，当前合格对照仍待验；删除只报告正确性与性能。
- gRPC为本次通道；大规模、长时、复杂可靠性、多Meta、etcd、Redis后置。阶段一历史8/8关闭，当前回归属于阶段二。

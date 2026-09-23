# DMS Home Agent workspace 预览版安装

这是本地测试候选，不能替代已发布的 `dms-server` 包或宣称生产可用。适用 Linux、2–4 节点、受信任的私有网络；每台节点拥有自己的数据盘。FUSE、NFS 与 P2P 的可用性取决于内核、权限和网络。没有多副本、自动 home 接管或中心 HA。

## 从源码构包

在 Linux 安装 Rust 1.95、Cargo、C 编译器、`protoc`、Python 3、FUSE3，以及 NFS 模式需要的 `nfs-common` / `nfs-kernel-server`。在完整源码根执行：

```sh
python3 scripts/release/dependency_inventory.py --output artifacts/homefs-licenses
DMS_THIRD_PARTY_DIR="$PWD/artifacts/homefs-licenses" ./scripts/homefs/build-package.sh
```

脚本运行 `cargo build -p dms-home --release --locked`，随后生成 `artifacts/homefs/...tar.gz` 和同名 `.sha256`。Linux VM 与宿主机不得共用 target 目录。解包后在包目录运行 `sha256sum -c SHA256SUMS` 检查包内文件；外层 `.sha256` 校验下载/复制过程。

## 部署

在中心节点和每台数据节点解包。复制 `config/homefs.env.example` 为 `config/homefs.env`，分别设置节点 ID、中心地址、数据根目录、公用 FUSE 挂载点、NFS 和 P2P 地址及相同的私有 token；中心也必须配置此 token。中心 RPC 与 P2P 地址使用数字 IP:port，配置文件应只允许节点管理员读取。中心设置持久状态文件在本机可写磁盘。中心先执行 `scripts/run.sh center`；节点执行 `scripts/run.sh node`。用 `scripts/run.sh locate job-42` 或管理面 `GET http://CENTER_HTTP/v1/roots/job-42` 查询位置，调度器应优先把 Agent 放到返回的 home 节点。`scripts/run.sh roots` 查看全部目录。

NFS 后端：每个 home 节点先以 root 运行 `scripts/setup-nfs.sh DATA_ROOT TRUSTED_CLIENT_CIDR`，再将 `DMS_HOME_BACKEND=nfs`。节点以普通用户启动时，需要该用户能执行非交互式 `sudo -n mount -t nfs4`；也可由管理员提前挂好 `<PEER_MOUNTS>/<PEER_NODE_ID>`。导出使用 `no_root_squash`，因此 CIDR 内拥有 root 权限的客户端也能以 root 身份访问数据；只给隔离的测试节点网段，不暴露到其它机器。远端只在预挂载成功后可用。NFS 使用 `hard,actimeo=0,lookupcache=none,cto`；home 失联时已经进入 NFS 内核路径的调用可能等待到它恢复，不能把这个等待当成成功。P2P 后端设置 `DMS_HOME_BACKEND=p2p`，无需内核 NFS，但要求 P2P TCP 地址互通；超时返回失败。两模式都需要 FUSE 挂载能力。同一组节点必须一致配置后端，不能运行中切换。

首次在公共挂载点 `mkdir /mnt/dms-home/job-42` 将目录默认归属创建者节点。管理面查询可供调度器使用；首版不自动迁移 home。home VM 不可用时目录不可用，不能在其它节点强行改写 home。关闭写文件后另一节点重开须见新内容；`fsync` 的成功仅保证 home 本机存储保存边界，不保证 VM/磁盘永久丢失。跨一级目录 rename 返回 `EXDEV`。首版删除一级目录后保留 tombstone，不能再使用同名一级目录；工作区应使用唯一 ID 命名。中心 RPC 和 P2P 只应放在受信任私网，当前没有 TLS 或中心 RPC 鉴权。

本候选只有 Agent workspace 常用文件/目录操作；完整 POSIX、长期稳定运行与正式发布仍未验收。中心变更 RPC 和 P2P 连接虽有共享 token，但没有 TLS 或逐节点权限隔离，管理查询为只读开放接口，只能在受信任私网使用。源代码和安装包验证结果以同一提交的验收记录为准。

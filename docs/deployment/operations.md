# 运维说明

当前版本适合有限试用和开发验证，不适合声明完整生产可靠性。

## 启动前检查

启动服务前一次性检查：

- 二进制身份和包 SHA；
- 配置文件路径、Node/Meta ID、端口、TLS 证书和 trusted node 列表；
- `data_dir`、`run_dir`、`log_dir`、挂载目录和磁盘余量；
- FUSE 设备、`fusermount3` 或 `umount`、`findmnt`、`openssl` 等依赖；
- 旧进程、旧 PID、旧 UDS、旧挂载和上次失败残留。

不要启动后才发现缺少基础依赖。遇到环境阻塞时保存证据并停止受影响项。

## 启动

使用进程控制脚本启动：

```sh
sudo /opt/afs/bin/afs-processctl --prefix /opt/afs --config-dir /etc/afs --run-dir /run/afs --log-dir /var/log/afs start meta
sudo /opt/afs/bin/afs-processctl --prefix /opt/afs --config-dir /etc/afs --run-dir /run/afs --log-dir /var/log/afs start node
```

也可以使用：

```sh
sudo /opt/afs/bin/afs-processctl --prefix /opt/afs --config-dir /etc/afs --run-dir /run/afs --log-dir /var/log/afs start all
```

脚本会记录 PID、进程启动 tick、命令行、日志路径和 readiness。若 readiness 失败，先保留日志和 identity，不要手动覆盖旧状态。

## 状态检查

```sh
sudo /opt/afs/bin/afs-processctl --prefix /opt/afs --config-dir /etc/afs --run-dir /run/afs --log-dir /var/log/afs status all
findmnt /mnt/afs/ownerfs
findmnt /mnt/afs/dfs
ls -l /run/afs
```

对于 bind ON 场景，还要核对：

```sh
findmnt /mnt/afs/ownerfs/agent1
stat -c '%d:%i %u:%g %a %n' /mnt/afs/ownerfs/agent1
```

目标是确认 `/mnt/afs/ownerfs/agent1` 被 Home 底层真实目录覆盖，而不是把 FUSE 目录再次 bind。

## 正常停止

```sh
sudo /opt/afs/bin/afs-processctl --prefix /opt/afs --config-dir /etc/afs --run-dir /run/afs --log-dir /var/log/afs stop all
```

正常停止应满足：

- 已接受请求排空；
- 受管 runtime、Node、Meta 都返回可记录 exit status；
- FUSE mount 和 bind mount 已卸载；
- UDS、PID 和临时 claim 没有遗留；
- local-file Meta 下重启后仍能读回已确认内容。

memory Meta 的未持久化状态不能通过停机恢复；如果有未持久化 memory Meta 还在运行，不能为了环境维护直接停机。

## 恢复检查

local-file Meta 正常重启恢复的最小检查：

1. 写入并 `sync`/close 一个 OwnerFs 小文件和一个 DFS 小文件；
2. 正常停止 Meta 和 Node；
3. 启动相同配置；
4. 重新打开并校验完整内容和 EOF；
5. 记录二进制 SHA、配置 SHA、Meta 数据目录、挂载点和 wait0/退出状态。

该检查只证明当前范围的正常重启恢复，不等于 crash recovery、HA、多 Meta 或复杂可靠性通过。

## 空间与日志

测试数据、运行日志和原始证据默认放在仓库外。建议按用例分级预算：

| 规模 | 用途 | 注意事项 |
| --- | --- | --- |
| 64MiB | 小规模功能、恢复、基础读写 | 可重复使用自有数据空间 |
| 512MiB | bind/远端小规模性能 | 固定缓存、并发和屏障条件 |
| 8GiB | 大文件、基线或长时用例 | 需预估副本、并发、轮次和保留策略 |

性能测试避免持续全量 TRACE。停止的历史测试数据只有在完整归档和恢复校验后才清理。

## 安全与限制

- 试用 TLS 只适合隔离测试环境；正式部署需要独立证书生命周期。
- OwnerFs bind ON 当前是管理员控制能力，要求停机前先停止受管用户。
- 完整远端 POSIX、复杂可靠性、多 Meta、etcd/Redis 后端验收和大规模长时运行均未完成。能力范围以 [当前计划](../development/plan.md) 为准。

# M1 共享 Namespace 验证证据目录

本目录已经保存两组真实 Linux FUSE 验证结果：

- 根目录：单 VM 内运行 Meta + Node A + Node B，20 轮后重启 Meta 与 Node B；
- `three-vm-reviewed-final/`：A/B/C 三台 VM 分别运行 Node A、Node B、Meta，在最终审查修复后完成 20 轮和重启恢复。

两组都只通过普通 POSIX 文件操作覆盖
`mkdir/create/lookup/readdir/rename/unlink/rmdir`，并验证 rename 目录环与文件/目录替换类型矩阵，
不调用 DMS 内部接口：

- `namespace-workload.json`
- `namespace-recovery.json`
- `evaluation.json`
- `node-a.prom`
- `node-b.prom`
- `meta.prom`
- `meta.log`
- `node-a.log`
- `node-b.log`
- 重启后的 `meta-restarted.log` 与 `node-b-restarted.log`

根目录和 `three-vm-reviewed-final/` 内的 `evaluation.json` 均为 `status=PASS`。三 VM 的
`profile.json` 额外记录 VM 角色、私网地址、轮数与二进制 SHA256，便于追溯本次证据。

# Native bind与FUSE混合追加：当前偏移边界

**事实：** 固定6d候选的[既有r8证据](evidence/20261007-append-diagnostic/README.md)为52B顺序内容、128唯一完整并发记录PASS；第三次SEEK_CUR24而非38，62并发偏移不匹配FAIL。未重跑或修改这份验收结论；bind默认OFF。

**事实：** 第一方native export使用受管Home真实目录FD；native eligibility令TTL=0/private=false，FUSE open返回DIRECT_IO；物理打开保留O_APPEND，write_at实际向文件末尾追加，FUSE只reply写入字节数。代码入口：[export](../src/node/native_workspace.rs)、[Owner策略](../src/node/vfs/ownerfs.rs)、[FUSE适配](../src/node/fuse.rs)、[LocalFs](../src/node/storage/localfs.rs)。没有改第三方源码。

**推断：** 数据正确但FUSE描述符偏移错误符合“实际物理追加位置与内核ki_pos不同”的解释。[上游Linux6.8](https://raw.githubusercontent.com/torvalds/linux/v6.8/fs/fuse/file.c)的SEEK_CUR走generic_file_llseek，只有SEEK_DATA/HOLE进入FUSE_LSEEK；[公开ReplyWrite](https://docs.rs/fuser/0.16.0/fuser/struct.ReplyWrite.html)只能返回字节数。增加lseek回调不能直接修复已观察的SEEK_CUR。Ubuntu6.8.0-106.106补丁尚未逐行核对，不称已找到完整修复。

**决策：** 本项保留FAIL并暂缓实现，不试改TTL、伪造written、关闭O_APPEND或修改探针先SEEK_END来换通过。先推进可独立验证的经典锁小项；watch仍单列。将来[公开backing-file passthrough](https://docs.kernel.org/filesystems/fuse/fuse-passthrough.html)需要独立内核/ABI及受管FD生命周期资格化；当前ABI7.36未启用其7.40接口，不能靠私有fuser补丁宣布解决。

**受影响回归入口：** 既有driver的`--semantics-groups append --semantics-only`；第一方[append探针](acceptance/probes/native_mixed.py)，guards为`test_append_offset_status_preserves_wrong_offset_failure`和`test_concurrent_offset_correlation_checks_actual_record_end`。Rust `n2c_pf1_04_append_writable_retains_real_handle`只覆盖句柄保留/释放，不能证明内核SEEK_CUR。

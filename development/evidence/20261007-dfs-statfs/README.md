# DFS statfs功能修复与标准回归

2026-10-07。G1/g1.5保持8/8。修复DFS单节点、本地设备R1的真实容量查询，完成原被阻塞的功能项；不宣称集群容量、完整POSIX认证或性能达标。

候选基于main2b5d35c，修改chunk/replication/localfs/dfs四个Rust文件；[154编译输入](inputs.json)map SHA256为`6161e25b0c3a3e85f0e442193d2225fddbe78bedb4c9e94bd59139402646bd3b`。源码身份对应本提交中的上述输入，不包括文档。既有e925的ENOSYS/0-TAP/6TBROK[原失败](../20261006-e2e-current/r5/README.md)保持原样。

**Linux源码通过：** [回执](build/source-proof.json)：fmt；statfs8PASS/1ignored、statvfs1PASS、capacity17PASS；库553PASS/12ignored；config5/vfs1/FUSE契约4及8实际FUSE；DFS-only check、严格workspace/all-target/all-feature Clippy、release构建。输入前后不变。[Owner-only check](build/owner-only.log)exit0，保留两条未改方法的既有dead_code警告，不称所有feature零警告。[独立只读复核](review.json)仅批准local R1范围。Owner旧标准结果不继承新ELF；其容量转换未改变，新增容量/FUSE检查覆盖提取helper。

| 新release ELF | SHA256 |
| --- | --- |
| afs-meta | 9267052b8892a427b948059aa727ca432f8cdd4cd381d66be3cf84a50f94495a |
| afs-node | 01fb69b7b4d160e37a3f200b7b458bf5f6c1957890c6d791ceb7288f4dca288a |

**运行首轮r1：** [汇总](runtime-r1/runtime-summary.json)。真实DFS statvfs/df成功，但运行脚本漏建套件要求的夹具父目录，两驱动停在预检，0断言；属于运行脚本错误，不是VM故障或产品通过。[正常关闭](runtime-r1/closure.json)PASS，原记录保留。

**第二轮r2：** 仅修正夹具准备及挂载核验，ELF、套件、参数和判据不变；[整轮汇总](runtime-r2/runtime-summary.json)PASS，[关闭](runtime-r2/closure.json)PASS：正常stop、两个挂载均消失、记录的子进程均无活动身份。ARM64 Linux afs-g1-clean、local-file Meta/gRPC/R1/bind OFF、真实DFS statvfs/df成功，无VM重建/依赖修补/容量阻塞。

| 项目 | 固定范围 | 实际结果 |
| --- | --- | --- |
| [DFS pjdfstest](runtime-r2/std-01-pjdfstest-full/artifacts/std-01-pjdfstest/proof.json) | pin d25636a227606f8960e5179741d8f4ad7030ef41；full236 | 236选择/启动/完成、8819检查；28上游TODO（9 TODO not-ok），0skip/0意外失败；377秒 |
| [DFS LTP](runtime-r2/std-02-ltp-smoke6/artifacts/std-02-ltp/proof.json) | pin3a64d78f58bdceba93ed321e91215fb969a047ed；smoke6/657库存 | open01/read01/write01/stat01/chmod01/fcntl14：6选择/执行/6PASS，0TBROK/TCONF/FAIL/TIMEOUT；其余651未选；14.801秒 |

fcntl14保留原默认5000和每程序120秒判据，无max-tests、额外排除或事后过滤。两套驱动保存当前process/ELF、suite、exact mount、fixture identity及全部前置检查。运行脚本的`preflight.json`被后续挂载/夹具检查覆盖，不能把它单独当完整预检回执；以驱动identity/proof、selected-binaries-and-inventory、ltp-selected-elfs、mounted-proof和逐命令记录共同核对。原文件不修饰。

工具复用main2b5d35c的standard.py/ltp.py/target_identity.py及部署helpers，[哈希绑定](runtime-r2/build-proof.json)，不复制每轮整份源码。全部regular结果/日志保存，六个Kirk临时latest绝对软链仅记录[原目标](runtime-r2/archive-links.json)，未跟随或物化。原Linux完整tar仍在研究档案，SHA可追溯。ELF及私有TLS不入Git；SHA256SUMS覆盖除自身和本索引外全部packet文件。

**下一项：** Issue42/PR43容器workspace挂载访问的功能与性能，开关默认OFF。普通local/remote/DFS性能数据和原FAIL保留，专项优化暂缓。R2官方fuser API缺口仍独立阻塞，本修复未修改vendor；远端/多副本容量仍unsupported，不伪算集群容量，不继承新ELF的多节点恢复/FSx/Owner标准资格。

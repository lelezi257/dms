# 经典锁内核原语验证：有界通过，产品未修复

2026-10-07。固定产品6d51aeb/157编译输入不变；没有改Rust、第三方、AFS运行实例或重跑标准套件。Linux afs-build/root/aarch64/ext4、kernel6.8.0-142；不是原afs-g2-micro/6.8.0-106上的FUSE/native产品验收。

| 新增证据 | 结果与范围 |
| --- | --- |
| [6个内核原语组](r2-final-proof/report.json) | PRIMITIVE_PASS，真实进程POSIX/OFD双向冲突、非重叠、共享、部分解锁、两种关闭语义；11个child实际wait均0 |
| [原3个guard回执](vm-r2-final-logs/native-lock-kernel-logs-20261007-r2-final/tests/initial_guards.stderr) | unsupported errno不能计冲突、缺OFD ABI拒绝、旧output/sentinel保留；3/3，Linux实际rc0 |
| [3个受影响guard回执](vm-r2-final-logs/native-lock-kernel-logs-20261007-r2-final/tests/affected_guards.stderr) | 无输出child有界超时并回收；RESULT成功但exit1拒绝；GETLK精确type/range/pid；3/3，Linux实际rc0 |
| [条件闭合](completion-check.json)及[独立只读审阅](final-review.json) | 6个unique guard+6个CLI原语组；不是整模块7tests实跑、产品修复或完整POSIX |

当前工具：[维护探针](../../acceptance/probes/native_lock_kernel.py) SHA `1120a4fd48d7b255c7ee08c9f009d699c9eed25c2764f28efadf5ab3bcb414d6`；[针对性guard](../../acceptance/probes/test_native_lock_kernel.py) SHA `694bce8cdc339e2dc4581834d50569c1e546f037d1410d052114e733a3cd1752`。两份维护Python各保留一份，不标为generated；历史结果由固定hash/差异恢复。

## 身份与实际命令

[一次性前置原始记录](vm-r2-final-logs/native-lock-kernel-logs-20261007-r2-final/preflight/)核对root、arch、ext4、依赖、容量及source SHA；[CLI原命令](vm-r2-final-logs/native-lock-kernel-logs-20261007-r2-final/cli/primitive.cmd)、[stdout](vm-r2-final-logs/native-lock-kernel-logs-20261007-r2-final/cli/primitive.stdout)、[stderr](vm-r2-final-logs/native-lock-kernel-logs-20261007-r2-final/cli/primitive.stderr)、[实际rc0](vm-r2-final-logs/native-lock-kernel-logs-20261007-r2-final/cli/primitive.rc)。两个guard组的`.cmd/.stdout/.stderr/.rc`同样保留。无需安装依赖、改VM容量或启动产品。

原语报告SHA `d0179a8036963583e2b8109b021eca00bb0866765c5ddcaef26667b71215b68b`。第一方向GETLK准确记录F_WRLCK/start0/len100/pid-1；逆向start200/len50/pid等于真实READY child PID。`primitive_only=true`、`not_product_pass=true`、`transparent_classic_posix=false`。

## 旧版本与失败保留

[r1报告](r1-reference/report.json)、stdout/stderr和[原工具SHA](r1-reference/source.sha256)保持原身份。初版工具虽报告原语通过，审阅发现阻塞readline不能实现其声明的timeout、部分child退出/失败清理和GETLK范围判据不足；当前仅窄修工具并补有意义guard。r1口述4tests缺原始unit回执，不计本轮验证数。当前3未变guard因缺可携raw才补一次；不重复旧产品测试。

第一次collector误称r1报告不存在；随后确切sudo stat/cat成功，报告已保留。这是收集器分类纠正，不改历史产品结论。[原始stat](r1-report-check/stat.stdout)及rc保留；重复报告的字节恢复映射在[外部引用索引](external-references.json)，源码树外原始文件未删。

[初次反向恢复失败](recovery-check/summary.json)和原patch保留；修正relative label后的[最终Linux恢复](recovery-check-final/summary.json)两次apply rc0、两原工具hash完全相同。使用`recovery/*v2.patch`从当前维护文件恢复r1；不复制整份旧Python入Git。原始证据在源码树外`evidence/afs-delivery/native-lock-kernel-20261007-r1/`。

## 验收边界

[混合产品锁五项FAIL](../20261007-managed-semantics/README.md)未改，G1历史8/8和G2计数不变，bind默认OFF。[机制边界与收口决定](../../native-classic-lock-boundary.md)说明为何OFD不能透明替代process owner/any-close/access mode；本轮不实现削弱必要语义的桥。取消/阻塞、BSD、namespace及死锁不由本原语资格化。下一独立项为[Owner远端小删除正常收尾](../../owner-remote-delete-slice.md)，随后[DFS小同步读阶段](../../dfs-manyread-sync-slice.md)。

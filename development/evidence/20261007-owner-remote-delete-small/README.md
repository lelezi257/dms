# G2.16 OwnerFs远端小删除：独立出口完成

**当前版本实跑PASS：** 产品6d51aeb/map66dbbe3e/157输入、固定release ELF未变；Owner-only/local-file Meta/gRPC/native OFF。新独立根和端口，A远端客户端，B为Owner Home和唯一MooseFS4.59.2 chunkserver。G1历史8/8不重开；G2.16完成后G2为10限定完成/2普通性能FAIL/2bind进行中/13待验收。

| 检查 | 结果及证据 |
| --- | --- |
| 一次性前置 | Linux ARM64/ext4、依赖/工具SHA/TLS/print-config、独立端口、预算+floor；commands各role admission/preflight |
| Home/B与远端路径 | Meta home_node_id/B/epoch-session前后相同，B实际rootid-e1 backing、A无backing；[Home](home-location.json)、commands b-home-backing-before-delete-r2/a-no-home-backing-after-delete |
| 删除实跑 | 每系统100×4KiB/C1、1预热5交替配对，共12sample PASS；fdatasync准备、fresh open全内容、100 unlink、目录fsync/空目录/rmdir；[实跑](commands/a-delete-r1.stdout) |
| B独立可见性 | 停B前6轮600路径实际ENOENT，物理Home目录空；[600检查](commands/b-600-owner-paths-absent.stdout) |
| 操作量化 | Owner中位1049.602ops/s、Moose1542.929ops/s、配对吞吐比中位0.680266；[每轮](delete-quantitative-r1.json)、[出口摘要](summary.json) |
| 正常收尾 | 3AFS+3Moose真实wait0，正常卸载、mountgone/noowned；旧进程incarnation/旧mount ID/source/options不变；commands各role independent-postcheck-r2、fixture-postcheck、old-exact-mounts-postcheck |

**判据与边界：** 计时仅100 unlink，准备/fdatasync/内容校验/父目录fsync/B检查在计时外；缓存UNOBSERVED，旧无关负载保留。删除没有新增硬性能比例；完成正确性、量化和正常收尾，不称远端读写持平或强持久写资格。Moose单副本class与sole B在线拓扑有事实，未计时前逐文件查chunkcopy。量化文件在停止前记录normal_exit_postcheck=PENDING，停止后结论以summary及真实wait raw为准，原始文件不改。

**失败保留：** payload collector误将shell helper交给ldd；首次host deploy collector漏mkdir；首次backing collector误按workspace直名。原始非零command/raw全保留，仅纠正收集器；没有产品或环境失败、不重测已过case。旧[read/delete客户端wait1 FAIL](../20261007-owner-remote-small/README.md)保持原身份。

**来源：** [inputs](inputs.json)、[维护工具guards/review](../20261007-owner-delete-fixture/README.md)。只有结果、命令、配置摘要及身份索引，没有ELF、私钥、源码快照或重复维护Python。运输config.tar含TLS私钥仅留源码树外，禁止发布。下一[DFS同步读阶段](../../dfs-manyread-sync-slice.md)；普通性能调优、大规模及复杂可靠性后置。

[独立最终审阅](delete-final-review.json)批准限定G2.16出口，独立重算原始计时与真实wait/身份；审查未另跑服务或测试。

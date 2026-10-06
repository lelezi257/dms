# 当前6d OwnerFs远端小写：功能/清理通过，性能数据已保留

**事实：** `6d51aeb45c1ed8669d80f612b3817e6d1bdabe04` / compiler map `66dbbe3e…` / 157输入；固定OFF包及ELF未变。Owner-only/local-file Meta/gRPC/native OFF；新独立夹具由A远端写，B为Owner Home和唯一MooseFS 4.59.2 chunkserver。G1历史8/8不重开，G2正式计数不变。

| 独立检查 | 当前结果与证据 |
| --- | --- |
| 启动前准入 | 三节点依赖/固定ELF/TLS/配置/端口/容量PASS；只准备，不代替功能。见commands各role preflight |
| 小写数据 | 64MiB/1MiB/C1/byte97，每轮fresh exclusive文件；1预热+5交替配对=12文件。C fdatasync、父目录fsync及全SHA/EOF通过；[原始实跑](commands/a-write-r2.stdout) |
| 跨Home新鲜内容 | A关闭后B另行fresh open，全量读取6个Owner文件，每个64MiB/fullSHA正确；[B独立读回](commands/b-independent-readback.stdout) |
| Moose位置/副本 | 独立single-* create/keep class；6文件各1 VALID副本于B/.13:23043；[实际fileinfo](commands/a-postwrite-mfsfileinfo.stdout) |
| 性能摸底 | Owner中位数234.630MiB/s；Moose430.767MiB/s。每轮原始wall_ns、单独速率/配对比见[摘要](summary.json)，样本波动明显；不计正式持平 |
| 正常关闭 | A按已准入系统fusermount3 -u，实际监督wait0；B/ctl Moose TERM wait0，3AFS实际wait0。独立无owned进程/mount、旧incarnation不变；commands各role independent-postcheck-r2 |
| 容量 | B实占831,343,840B<1GiB预算，空闲>4GiB固定底线；768MiB逻辑样本保留。commands各role postwrite-capacity |
| 工具验证 | Linux唯一19guards=fixture14+write5；只窄复查修改项。初始工具由reverse patch逐字节恢复/核对SHA，见a-provenance-byte-restoration |

**计时和限制：** C wall_ns包含open/线程/pwrite/fdatasync/close；Python目录fsync、完整内容验证、B独立读回在计时外。匹配fdatasync公开API不证明Moose强持久ACK等价，B001仍BLOCKED；缓存UNOBSERVED、旧无关负载保留。此项`DATA_RECORDED`与内容/清理PASS不等于G2.15正式性能通过。不重跑Owner已过标准或之前读/删；读删旧Moose客户端wait1 FAIL原样保留。

**保留失败：** A首次准入把有效fusermount3 help rc1当失败；窄修广告-h/-V。write-r1严格类型门禁误拒绝stock `fuse`，未运行C；改为仅接受fuse/fuse.mfs且保持exact source，新output write-r2。首次ctl收尾检查器误用不存在的统一pointer；改用实际role pointer，只复查检查器，无服务重启/实测重复。全部原始command/stdout/stderr及版本SHA保留。

**工具来源：** 当前维护工具在development/acceptance，原始运行版本通过Git历史或本索引的反向patch恢复；不复制整份Python/ELF/rootfs/私钥进Git。输入与SHA见[inputs](inputs.json)、[最终工具映射](tool-inputs-final.json)。官方正常卸载语义参考[Moose集群停止](https://docs.moosefs.com/installation/cluster-starting-and-stopping/)和[libfuse手册](https://github.com/libfuse/libfuse/blob/master/doc/fusermount3.1)，实际固定版本和运行退出以本回执为准。

**下一项：** 回到容器workspace必要语义，下一独立经典锁小项；[append偏移边界](../../native-append-offset-boundary.md)保留FAIL暂缓，bind默认OFF，watch单列。普通性能调优、大规模/复杂可靠性、etcd/Redis后置，R2官方fuser迁移独立阻塞。

[独立运行证据复核](review-runtime.json)批准上述限定内容/数据/清理结论，不关闭正式性能与耐久出口。

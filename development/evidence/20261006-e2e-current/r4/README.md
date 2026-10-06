# Owner 本地独立小项结果

2026-10-06，产品候选 e925c5bcf0408851ebfa08a59df29953374da9e9。同一已发布 target/debug ELF，无产品重编译；ARM64 Linux6.8、单VM、local-file Meta/gRPC、普通FUSE、bind OFF。不是release优化构建或冷缓存/真实hot资格。

| 小项 | 功能 | 性能/出口 | 原始证据 |
| --- | --- | --- | --- |
| G2.09 64MiB本地读 | PASS：全内容/长度/EOF、64次IO、open/close成功 | FAIL：Owner 4109.26MiB/s，ext4 7197.76MiB/s，比值0.5709；目标≥0.90 | [seq-read](owner-small-r1/seq-read.json) |
| G2.10 64MiB本地写 | PASS：open/write/fdatasync/close、fresh-open全内容/EOF | FAIL：Owner 1046.20MiB/s，ext4 1661.31MiB/s，比值0.6297；目标≥0.90 | [seq-write](owner-small-r1/seq-write.json) |
| G2.11 小文件删除 | PASS：每样本100个4KiB文件，准备时文件/目录屏障，删除后目录同步且为空 | 完成既定报告出口；Owner13599.55ops/s，ext4 159641.64ops/s；比值0.0852，无新增硬比例门槛 | [delete](owner-small-r1/delete.json) |

**判据：** 每项5对交替ext4/Owner样本、C1；顺序数据1MiB块、64MiB文件、seed0x42/generation0；数据先准备后buffered测量，不宣称冷/热。读的计时包含内容验证；写计时包含open/write/fdatasync/close，之后目录fsync及fresh-open验证不在计时内。删除计时仅unlink循环，屏障/准备/最终空目录检查在计时外。两侧一致。吞吐分别取5样本中位数，再相除；预定目标0.90，无噪声豁免、无成绩重跑。

**身份/容量：** [测前完整准入](owner-small-r1/preflight.json)、[测后身份](owner-small-r1/post-identity.json)确认相同Meta/Node PID及ELF、Owner实际FUSE挂载、baseline与Node数据均guest ext4；可用空间足够，未扩盘或改变旧服务。

**审计：** [audit](owner-small-r1/audit.json)复核60次IO阶段、20个测量样本、10个删除样本的原始退出码/JSON/trace及统计。功能通过不能覆盖性能FAIL。原运行器exit0只表示所有独立case已尝试，不代表性能通过；原代码/结果原样保留。后续驱动增加优化Python拒绝、完整worker校验和性能失败exit3，[拒绝检查](owner-small-r1/optimized-rejection.log)通过；没有重新执行benchmark。

下一项Owner跨VM功能/中心恢复，remote性能逐项准入。MooseFS强持久写比较已有独立基线BLOCKED，不阻塞DFS标准或一写多读功能。先继续独立项，性能缺口保留待优化，不追单点完美。

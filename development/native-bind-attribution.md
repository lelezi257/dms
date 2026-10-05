# Native bind 有限归因（2026-10-06）

这次只确定可审查的机制和差距范围，不修改生产代码，不以反复采样直到 PASS 为目标。性能诊断完成不代表完整合同或正式验收成立。

## 远端元数据：全索引重建和查询放大

`network-probe-20261005T163740-2ed29486` 的五路径 10,000 × 4KiB 元数据实验，在 DMS remote 第一轮预热返回 `readdir: Interrupted system call`；仅前三个 lane 的预热完整采到，不能计算一组完整远端比较。保留原 FAIL、RPC 指标与正常 stop/delete 的五个容器清理记录。不是把超时负载记成缺失然后只发布成功样本。

在该**未计入正式比较的预热**中，读取实际 Home/peer RPC histogram：

| Home handler | 次数 | 累计秒 | 平均毫秒 |
| --- | ---: | ---: | ---: |
| Create | 10,000 | 1.289 | 0.129 |
| Write | 10,000 | 0.590 | 0.059 |
| Lookup | 50,421 | 94.272 | 1.870 |
| GetAttr | 85,634 | 172.645 | 2.016 |
| Flush | 10,001 | 19.015 | 1.901 |

指标是各 Node 启动以来、某一时刻的累计；采集时 benchmark 尚在执行，包括 bootstrap/祖先查询。A/B 抓取时间不同，不能把 client_sum - server_sum 当精确的逐请求网络延迟，也不能用累计值替代各 benchmark 阶段计时。

实际源码 `owner_entry_for_path` 取 metadata 后调用 `inode_for_path`。在 path/identity 已匹配、只更新 attributes/kind 的分支，仍无条件 `rebuild_identity_index()`；`update_record` 也这样做。该函数清空 identities，扫描所有 paths/inodes 并 clone RootId/identity。它在共享 state mutex 下运行：**每次命中为 O(N)，叠加零 TTL 的大量 Lookup/GetAttr，会造成随 namespace 扩大的工作量**。新对象插入已有增量 insert，属性命中的全量重建不源自 ext4 的属性系统调用要求。这是已测候选继承的 OwnerFs identity-cache 实现成本（主分支 b259c44 也可只读查到该 cached-hit 重建；未对 main 重新部署/计时）；不是 bind 的 native 数据访问开销。

为区分“扫描机制”与一次 RPC 观测，提取生产 `rebuild_identity_index` 函数体，使用最小替身类型和相同 HashMap 扫描/clone 结构做 200 次属性命中控制。WSL CPU 机制实验的每次 rebuild 平均：100 paths 7.67µs、1,000 paths 94.11µs、10,000 paths 1.131ms；attribute-only 控制平均约 8ns。完整源文件、函数 SHA、结果和 RPC 原文位于 [归因数据](acceptance/results/native-bind-index-attribution-20261006/result.json)。**这是 WSL 上的算法机制证据，不是 VM 比较样本，也不是生产修复/整体加速预测**；实际 VM handler 的增长方向与量级支持它是主要热点，未做完整 CPU 栈采样，不能声称解释了所有延迟。

两项已排除的猜测：RootGrant 校验存在缓存，不能说每次 file RPC 都重新访问 Meta；当前 tonic 默认 TCP_NODELAY 为 true，不能凭延迟猜 Nagle。重试/超时造成原 readdir EINTR 的完整因果链未进一步验证，作为真实失败保留；不把 index 成本与 EINTR 的因果关系冒充已直接追踪到。

交给接手 AI 的有界建议：核对 identity-only index 的增量维护和属性命中 fast path，保留替换/rename/hardlink/旧 fd 身份合同；复验相同 10k remote 失败和 histogram。这里不实现，也不通过增大 TTL 或弱化权限/epoch 缓存来换性能。

## 宿主绝对路径：FUSE 祖先查询

既有 E10 的 16 轮 trace 有 192 个绝对路径祖先查询、相对路径为 0；宿主 P1 native/ext4 的 c1 19.276× 与容器原生父目录 P1 接近 1 相互支持。单独 bind 文件数据原生不消除每次从 FUSE 根开始的路径查找。容器只提供 workspace 且父目录为 native，正是核心 Agent 场景的已测解决路径。保留原路径并没有假装宿主绝对路径没有成本。

## 容器本地剩余百分比差距

既有完整 30 shape 矩阵 0.834–1.135，同文件四个控制 0.990/1.035/1.076/0.983，不能说每项严格 native 门槛通过。最终 ext4 source、容器内路径、数据 inode、计时 syscall loop 已核对；排除 FUSE 数据回调和文件布局作为所有差距的唯一解释。最终挂载同源并不能证明固定 bind 增量成本为零或把所有剩余差异归因于调度噪声。

有限控制到此收口：完整配对轮次、同文件条件和环境限制公开，具体残差根因未定位。它不足以证明 DMS bug，也不足以证明调度/缓存是唯一原因。用户要求交付而非性能死磕，本分支不继续增加采样/优化来追求每项≤1.0。

## 统一较小负载的五路径对照

另行固定 `bounded-five-lane-v1`：所有 lane 相同 1,000 × 4KiB 六阶段元数据、64MiB 顺序/16MiB 随机的 8 个形状、c1/c8、close/fdatasync/fsync、guest-cold/hot；每形状 1 预热 + 5 测量轮、交替 lane 次序。原 10k/30 shape/8GiB 目标不改变，该较小负载是新的有界诊断，不能抹掉规模扩展问题。

已有 E19 本地 8GiB 大于 guest RAM 的 30 shape 数据继续保留。较小工作集对缓存更友好，不能据此推断大数据/全规模远端性能。stock MooseFS 的 userspace cache 与 B001 物理持久性未资格化；所有五路径 barrier 数值仅比较相同 API 回复的诊断成本。较小矩阵的完整对照及有限差距见 [交接报告](native-bind-closeout.md)。


## 新五路径诊断的有界收口

E21完整固定矩阵已采完。native/ext4逐轮耗时比：8项IO 0.973–1.251，四组metadata总耗时0.912/1.060/0.968/0.997；保留严格门槛未全过。下面从**原始五个正式样本**分解时间，不重新采样择优：

| 场景／lane | 总wall中位ms | barrier中位ms | 非barrier中位ms | client CPU中位ms |
| --- | ---: | ---: | ---: | ---: |
| 64MiB seq-write close c1 ext4 | 20.890 | 0.0005 | 20.889 | 19.309 |
| 同上 native | 23.104 | 0.0005 | 23.103 | 21.213 |
| 16MiB 4KiB random-write fdatasync c1 ext4 | 12.883 | 8.870 | 3.778 | 6.525 |
| 同上 native | 16.116 | 12.486 | 3.626 | 5.353 |

25.1%案例的额外时间定位在fdatasync等待窗口；非barrier和client CPU没有相同退化。不能把不同字段的中位数直接相减当成逐样本因果分摊，也不能把这次约3ms同步等待差异归因于固定bind成本。具体ext4/虚拟磁盘/调度的底层原因未确认。10.6% seq-close差异同时出现在client CPU与pwrite延迟，几乎没有barrier成本；同source/loop和native数据路径排除DMS回调，但具体CPU/内核写成本的残差仍未定位。到此有限归因收口，不追加优化或更多样本。

远端metadata DMS/MFS为8.333–13.880，已知index重建和查询放大机制保留；不宣称这些比值全由index一项造成。热4KiB随机read的remote比值763.502，实际DMS约2.024秒，MFS约0.00266秒。当前OwnerFs FUSE open使用DIRECT_IO，read走OwnerFiles RPC；MFS可能由kernel/client缓存命中，热读取接近本地内存路径。差距与“DMS每个小读经过远端协议、MFS有客户端缓存路径”的机制一致；该采样没有逐请求RPC计数／缓存层命中追踪，不能锁定MFS究竟是哪一层命中，也不能当纯网络能力比。FUSE驻留标记为unobserved，不能把guest drop_caches当MooseFS userspace cache已冷。

其余远端IO也有实际差距，完整表公开；不在本分支实现read cache、batch RPC、TTL放宽或barrier改动。下一步若用户决定继续追性能，应先选择是否保留每次读回Home的策略并匹配baseline缓存／持久性；任何语义／合同变化仍需明确同意。

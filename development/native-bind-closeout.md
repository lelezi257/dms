# OwnerFs native bind 有界穿刺与实现交接

任务出口按用户 2026-10-06 指示调整为：功能可行性结论、真实性能对照、有限归因、issue/MR 交接。明确反例和尚未证明的路线属于穿刺结果；不要求在此分支实现生产特性、消除性能差距或通过正式 release 门槛。沿用 RFC 0001、Issue #42、草稿 PR #43；不合入、不重启方案、不改通用可靠性主线。

## 架构判断

**同路径 native export 和只暴露 workspace 的容器机制可行；当前 unchanged standard FUSE/P2P + 不受代理仲裁的 native ext4 组合不能完整满足全部既有语义。** 这两个结论同时成立。核心容器创建后再启动 Agent 的正常路径避开旧 FUSE 引用并消除 FUSE 祖先查询，但不会自动解决远端 append、锁、映射、通知或最终容器 fencing。

| 场景 | 分类及决定性证据 | 实现/演进边界 |
| --- | --- | --- |
| 原路径 `/ownerfs/agent1` 新访问 | 已验证可行：E1/E2/E12/E18 同 ext4 source、原路径、最终 mount namespace；容器父目录原生且只提供这一 workspace | 生产 Node 仍 ordinary；需要发布到 runtime 的 namespace、在最终 Agent 核对身份后 READY。私有 Node namespace 的路径字符串不是宿主 daemon 的可访问挂载。 |
| mkdir 回复后到 bind 的过渡 | 已验证基础机制：E11 延迟/EPERM 后同 backing、不虚报 ready；E20 在真实认证 P2P Home pwrite 完成而远端回复尚待返回时激活 manager，之后 native/旧 FUSE/远端重开内容相同 | E20 限于固定偏移的一次 15B 写，不能推出 append/rename/所有在途操作均正确；不需要把已经进入 FUSE 的请求重新下放为 native。本次走同backing的正常过渡FUSE，未证明拦截全部LOOKUP到挂载后才回复；挂载自身的目标路径解析若也被门禁拦住可能形成循环等待，不能由本例推出这种门禁设计可用。生产 READY/ACK 组合为实现遗漏。 |
| 旧 cwd/dirfd/文件 fd | 已验证可行及已接受边界：E5 旧目录反例，E13 close/reopen、替换/unlink 后旧 fd 保留旧对象 | 已接受目录父关系不即时更新、跨视图 close-to-open。不能把旧 fd 按名称重绑，也不能据此豁免 append、锁或映射写回。 |
| ordinary/native 混合节点 | E16仅两个实际ordinary B/native-eligible Home重开控制通过；不是完整混合权限/缓存/替换矩阵 | E21远端计时peer为native-eligible构造。不能把本次数据当普通生产peer或后来main版本的部署结果；完整interop仍未验证。 |
| 本地/远端 append | 当前架构冲突：E15 单次 2MiB write 拆为三个 Home 写，被 native 追加穿插；非重叠例最终长度 12B 正确但远端 SEEK_CUR 为 8，native 控制为 12 | 标准请求没有原 userspace syscall 的整组边界/总长度，WRITE 回复没有 Home 实际结束偏移；本地 native 也不受 FUSE inode 锁或代理 mutex 约束。单纯 max_write、逐请求 mutex 或 fsync 不关闭问题。 |
| 锁 | 部分可行：E14 非阻塞 flock 与对象替换互斥；E4 否定把 PID 写进代理请求或改用 OFD 来保持 classic POSIX 同进程 owner | Home 代理 fcntl 的实际 kernel owner 属于代理，FUSE owner token/报告 PID 不会使它成为原 native 进程；阻塞取消/远端 final-close 组合仍未完整验证。 |
| mmap/基础文件元数据 | E20 native MAP_SHARED 修改、msync/fsync 和重读通过；native/旧 FUSE/remote MAP_PRIVATE 修改不写回通过；三路径 chmod/user.xattr/symlink 小机制通过。旧 FUSE 与 remote MAP_SHARED 均 ENODEV | shared 映射是当前能力缺口；当前 direct-I/O/ABI 未协商允许 mmap，不能归因于 bind 自身不支持 mmap，也不能说打开一位即可证明脏页/权限/一致性正确。完整 multi-UID、ACL/硬链接/复杂 namespace 仍未资格化。 |
| watch | E20 native watcher 正常收到 native 修改；旧 FUSE 和 remote watcher 都有各自 FUSE 正向控制，但没有收到 native 修改 | 两个 VFS inode/不同 kernel notification 域，close-to-open 不豁免事件语义。需 Home 事件观察及远端/旧 FUSE 事件桥；当前普通 notify-invalidation 不等于已证明的 inotify 事件桥。 |
| 删除/回收/切换 | E19 原 export 正常 Detached 时容器 clone 仍能读写；随后受控 runc stop/delete 可撤掉该容器持有的 namespace。E2/E7 已证明身份/busy 原语 | 实现遗漏：登记最终容器及 peer/旧引用、拒绝新使用、停止/排空实际使用者、正常卸载、再删除/复用/换 epoch。RootGrant 撤销不能回收已打开 native fd。普通 mounted root 的 rmdir/rename 可在 kernel 返回 EBUSY，无法靠 FUSE 回调透明截获；受管删除入口必须明确。完整换 Home/旧 RPC fencing 未验证；未知namespace/FD传递引用不能靠一次umount判定消失，应拒绝复用而非承诺可瞬时撤销任意native FD。 |
| 异常状态 | 同一存活 namespace 的 helper/journal 恢复原语有 E2；独立 container clone 不随原 export 消失有 E19 | Node/FUSE death、失去 namespace、boot/Home 迁移的全组合仍未验证；先观察 actual mount/引用/权威，不把日志当当前内核事实。完整自动重试/恢复由接手 AI 实现。 |

### E20 原始实验和范围

`network-probe-20261005T162710-11936566`：真实三 VM Meta/Node/TLS/P2P，沿用优化 Node/Meta。strace 注入 Home `pwrite64` 的返回延迟，在 physical 已写且远端 actor 回复文件未出现时进行 manager activate；激活后再观察回复仍缺失，停止注入，操作成功，三路径重开读到 `transition-data`。最终 Node/Meta 正常退出并归档。

原工具失败 `network-probe-20261005T162439-900ea5b1` 保留：8 秒注入在多个重试 worker 的首个 syscall 重复生效，超过 peer 响应超时，actor 等待失败。后续只修正注入的结束时机；没有修改产品重试/超时或把原 FAIL 改 PASS。

本实验 runner PASS 表示成功采集预期正向与负向事实，**不表示 shared mmap/watch 或完整架构合同 PASS**。

### 必须交给实现者的路线判断

1. **保持全部原合同：** 对 append 的 syscall 整组仲裁/正确客户端偏移、classic 同进程 POSIX owner，研究真正参与 native 和 FUSE 共同 kernel 状态的客户端/内核边界。标准 wire 原样加 Home 用户态锁不是已证明路线。任何候选先复验 E4/E15，不能直接承诺可实现；这次不选择或实现定制内核。
2. **用户模型约束：** 单一写入者、应用主动锁、限定追加尺寸、禁止混合旧 FUSE/native 锁等会改变用户可见合同；未经用户明确同意不能作为完成实现的捷径。管理先准备 workspace 再启动 Agent 已同意，但不推出其它约束自动同意。
3. **映射/通知：** 可独立探索当前 Linux 能力协商和缓存/脏页转发、Home 原生事件到客户端通知域的桥接。必须证明实际 msync/close/barrier、对象身份、事件类型/顺序/丢失与权限，不用当前 native 映射通过代替远端通过。
4. **容器生命周期：** 将最终 runtime/Agent mount namespace 及 source、Root/epoch/Home、进程身份纳入受管登记。原 export Detached 是局部事实；停止所有受管引用并核验最终挂载消失后才能复用 backing。完整可靠性流程由另一 AI 接手。

原理参考：[Linux v6.8 FUSE 文件实现](https://github.com/torvalds/linux/blob/v6.8/fs/fuse/file.c)、[FUSE UAPI](https://github.com/torvalds/linux/blob/v6.8/include/uapi/linux/fuse.h)、[FUSE I/O 模式](https://docs.kernel.org/filesystems/fuse/fuse-io.html)。目标 Ubuntu 6.8.0-142 的具体运行结果优先于上游原理推断。

## 性能证据与有限归因

既有 E17–E19 保留全部数据，见 [性能报告](native-bind-performance.md)。宿主绝对路径的数量级退化根因为 FUSE 祖先查询，E10 trace 与容器原生父目录对照相互支持。容器元数据和本地顺序/随机 IO 已接近 ext4；完整矩阵比值 0.834–1.135、同文件补充四项 0.990/1.035/1.076/0.983 不代表每项严格达标。

同文件/同可见路径已排除 source 身份和文件布局作为所有差距的唯一解释；原生容器最终是 ext4，文件数据操作不经过 DMS/FUSE 回调。剩余百分比差距不能仅凭这两点归因于固定 bind 成本或某个 DMS bug。有限控制和完整配对数据用于界定调度/缓存/运行环境残余；未知的具体原因如实保留，不进入不断新增样本或优化。

### E21：真实 VM 五路径有界诊断

`network-probe-20261005T170023-06e15509`，实际 OCI lanes：A ext4 / native bind / MooseFS local，B DMS remote / MooseFS remote；同一优化 Node/Meta、同版本 runc。所有 lane 的容器内路径相同；ext4/native/DMS remote 操作同一 backing，MooseFS 两个客户端操作同一单副本文件对象。

**固定较小负载**：1,000 × 4KiB 元数据、64MiB 顺序和16MiB随机、8 shapes；1轮预热+5轮交替采样。120个metadata任务和240个IO任务完整采集并核验内容/端点回复/源身份/缓存证据；不是原10k/30 shapes/8GiB规模验收。原10k首轮remote失败及所有准备失败保留，见有限归因。

表中是逐轮耗时比的中位数，>1较慢。元数据总秒数为六阶段wall time之和；不含runtime exec、准备和内容复验。所有正式样本均保留，没有择优。

| 元数据 | ext4 秒 | native 秒 | MFS local 秒 | DMS remote 秒 | MFS remote 秒 | native/ext4 | DMS/MFS remote |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| absolute c1 | 0.0385 | 0.0335 | 3.8614 | 40.9350 | 4.2498 | 0.912 | 9.519 |
| absolute c8 | 0.0340 | 0.0344 | 2.1474 | 31.6704 | 2.1752 | 1.060 | 13.880 |
| relative c1 | 0.0342 | 0.0334 | 3.7113 | 35.3339 | 4.0355 | 0.968 | 8.333 |
| relative c8 | 0.0381 | 0.0394 | 2.3119 | 29.0802 | 2.4479 | 0.997 | 11.880 |

| IO（API 屏障诊断） | ext4 秒 | native 秒 | MFS local 秒 | DMS remote 秒 | MFS remote 秒 | native/ext4 | DMS/MFS remote |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| seq-read 1024KiB c1 guest-cold close | 0.0287 | 0.0286 | 0.1016 | 0.1808 | 0.1021 | 0.996 | 1.706 |
| seq-read 1024KiB c8 hot close | 0.0107 | 0.0108 | 0.0114 | 0.1484 | 0.0109 | 0.991 | 14.341 |
| seq-write 1024KiB c1 guest-cold close | 0.0209 | 0.0231 | 0.0707 | 0.1951 | 0.0589 | 1.106 | 3.312 |
| seq-write 1024KiB c8 guest-cold fsync | 0.0636 | 0.0602 | 0.0885 | 0.2332 | 0.0746 | 0.996 | 3.127 |
| random-read 4KiB c1 hot close | 0.0023 | 0.0023 | 0.0029 | 2.0240 | 0.0027 | 0.987 | 763.502 |
| random-read 64KiB c8 guest-cold close | 0.0095 | 0.0092 | 0.0563 | 0.1795 | 0.0612 | 0.988 | 3.552 |
| random-write 4KiB c1 hot fdatasync | 0.0129 | 0.0161 | 0.1123 | 2.3266 | 0.4216 | 1.251 | 5.023 |
| random-write 64KiB c8 hot fsync | 0.0130 | 0.0123 | 0.0316 | 0.2431 | 0.0300 | 0.973 | 8.201 |

**可比性边界**：A/B 均2vCPU、约5.8GiB RAM、无swap；A同一ext4卷，MFS唯一chunk在A，master在ctl。协议/cache机制是被比较系统的一部分，但guest drop_caches不清空MFS用户态cache，也不能证明Hyper-V host cache为空；FUSE mincore不支持，标记unobserved，不伪造冷驻留。对native/ext4证明guest-cold=0/hot=完整工作集。较小数据不能替代E19大于guest RAM的本地矩阵。

**持久性边界**：stock MooseFS `HDD_FSYNC_BEFORE_CLOSE=0`、`CHANGELOG_SAVE_MODE=0` 的 B001物理屏障未资格化；表中的close/fdatasync/fsync仅比较同API回复成本。不能据此宣布强持久性三方/远端门槛PASS，不能因为一个系统做了更多同步就判定其纯网络/存储性能更差。

**验收与结论**：既定严格native目标仍是适用shape的paired median≤1.0（完整规则见原计划），较小诊断不修改目标。核心容器本地已有接近ext4的数量级证据；原宿主绝对路径退化、远端namespace规模成本、百分比残差、cache/barrier资格化各自保留。此次出口是结论/数据/归因/交接完备，不是架构/性能阶段PASS。

[完整摘要及各阶段数据](acceptance/results/native-bind-closeout-performance-20261006.json)、[全部原始输入/样本/失败 manifest](acceptance/results/raw/manifest.json)、[有限归因](native-bind-attribution.md)。原始result.scope沿用早期P1泛用措辞，真正profile/counts在closeout_performance；所有原始字段保留，后续工具已修正scope文案。

## 阶段出口（与本次任务出口区分）

- 功能完备性阶段：未PASS。当前组合有具体语义反例；保留原合同的客户端/内核路线尚未证明。
- 正式性能阶段：未PASS。完整规模、匹配持久性/cache及每项严格门槛尚不成立；本次较小负载只是诊断。
- 生产实现/可靠性阶段：暂缓。本次没有生产接入和通用可靠性修改；交给接手AI。
- 本次四项穿刺交付：以架构逐项结论、实际五路径数据、有限归因、可复现材料和issue/MR移交作为结束条件，不以新增测试数衡量。

## 实现交接及暂缓工作

已有 RFC/代码/实验不重做。接手 AI 先阅读本页、RFC 0001、用户案例、性能报告和原始结果；再决定如何在 main 中采用所需 bind 原语。继承分支包含较早 Meta/Root/RPC/LockError 基础，不代表这次授权继续修改这些模块。

- bind 生产集成：配置/worker、原 export 发布、最终 Agent 身份及 READY、runtime 登记与受管退出。
- 必须保留的合同 blocker：append/游标、classic POSIX owner、适用 shared mmap/watch；候选路线须用反例验收。
- 后续工程：完整 peer/句柄/映射排空、错误重试、持久化 ACK/cursor、daemon/boot/Home 迁移恢复、完整权限/并发回归。
- 本次不做：生产实现、通用可靠性修补、rebase/merge、以性能达标为前提的长期微优化、正式 ARM64/RDMA 发布验收。

读写交付前核对 PR #43 仍草稿、未合入。只读更新 origin/main 到 b259c44f82be90ae07158501295ddbc5359e7a35，10个继承基础文件与main重叠；记录见provenance JSON，不做rebase/merge。所有实现遗漏和未验证组合保持公开；本页不是功能完备性或正式发布批准。

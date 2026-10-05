# Native bind 穿刺复现与实现验收入口

先读 [交接报告](../../native-bind-closeout.md)、[有限归因](../../native-bind-attribution.md) 和 [RFC 0001](../../../docs/rfcs/0001-ownerfs-native-bind-mount.md)。这些脚本是受控调查工具，不能直接用作生产 Agent 管理面。

## 离线核验

从 [raw manifest](raw/manifest.json) 找实验包 SHA，在新的临时目录只展开外层包。不要执行 guest archive 内的 ELF，也不用展开 guest rootfs。E20 和 E21 的已完成诊断：

```sh
python3 development/acceptance/probes/ownerfs_native_closeout_verify.py /path/to/unpacked-run --self-test
```

观察器从归档读取常规文件，核对 archive SHA、逐文件 SHA、输入 SHA、实际 runtime 返回、完整固定矩阵、原生源对象、缓存证明和 stop/delete。`checks_ok=true`只表示证据可信，输出保留`semantic_acceptance=false`或`performance_acceptance=false`。E20 shared mmap/watch 的负向结果有正向控制，不能把 runner PASS 解释为功能PASS。E21明确是`bounded-five-lane-v1`而不是完整规模性能门槛。

随机写的 `content_ok` 只证明本次生成的触达偏移已写入预期内容，没有核对全部未触达区域的完整性；不能据此宣布整个文件无损。该限制不改变已采时间，完整功能验证由接手AI补齐。

原 runner FAIL 的包也有 archive/input 完整性记录，但不能送入“完整矩阵”观察器后要求PASS。10k remote原失败仍有原stderr、三条完整预热、当时RPC指标、source snapshot和五容器正常清理事实。旧 E15–E19 使用原 `ownerfs_native_network_verify.py`，原 FAIL 的限定规则不改变。

## 重跑受控 VM 实验

环境是已有三个独立Linux VM，ctl=10.77.30.11、Home A=10.77.30.12、peer B=10.77.30.13。A/B相同2vCPU/约5.8GiB RAM、ext4数据卷；sudo免交互、真实`/dev/fuse`、strace、nsenter、runc。Linux6.8.0-142，runc1.3.4。使用专用实验端口与目录，脚本只对核验过boot/start/exe/SHA/namespace的拥有进程发信号。不能与另一个实验共用这些端口。

MooseFS4.59.2-1的master在ctl、唯一chunk在A，A/B已挂载同一文件系统；需`native-bind-one`storage class的`keep_labels: *`。当前mfssetgoal/mfsgetgoal是弃用no-op，不能用于副本资格化。保持配置和class记录；stock barrier/cache限制见报告。

准备当前候选的**冻结优化版** lib-test Node和afs-meta，不用任意debug产物代替。实际lib-test仅Linux特权驱动选择native-eligible，普通生产Node仍未启用。对照157个Rust/proto/Cargo源码输入和执行文件SHA的记录见 [provenance](native-bind-closeout-provenance-20261006.json)。运行控制器需要传入当时源快照（inputs.json、tracked.patch及源码）；不要让后来变化的main源码冒充已测对象。

编译相同小型工具，五lane使用相同 ELF：

```sh
gcc -O2 -Wall -Wextra -Werror -pthread development/acceptance/probes/ownerfs_native_closeout_benchmark.c -o /tmp/benchmark-closeout
gcc -O2 -Wall -Wextra -Werror -pthread development/acceptance/probes/ownerfs_native_closeout_io.c -o /tmp/io-closeout
gcc -O2 -Wall -Wextra -Werror development/acceptance/probes/ownerfs_native_container_probe.c -o /tmp/container-probe
python3 development/acceptance/probes/ownerfs_native_network.py --help
```

架构：`--case closeout-architecture --expect native --snapshot SNAPSHOT --test-bin OPTIMIZED_LIB_TEST --meta-bin OPTIMIZED_META`，加SSH/SCP/key/known-hosts/output环境参数。故意延迟Home pwrite回调返回，只观察固定偏移15B在途请求；不能推断append原子性。若失败，先保留原包，再核对注入/进程身份，不改产品超时直到偶然通过。

性能同样参数，改`--case closeout-performance`，加`--benchmark-bin /tmp/benchmark-closeout --io-bin /tmp/io-closeout --container-probe-bin /tmp/container-probe --moosefs-mount /mnt/afsdata/ownerfs-native-moosefs/RUN/mount`。固定profile由`ownerfs_native_closeout_controller.cases()`给出，不传入旧10k ELF混用。此次规模较小只作五路径诊断；原完整8GiB/30shape本地实验继续使用`container-bulk`与原工具。

原始结果不可覆盖。控制器逐样本保存closeout-performance.json、transcript，最终归档A/B/ctl；正常停止/delete拥有的OCI实例再卸载export并停止Node/Meta。保留失败/未完成样本，禁止按PASS筛选。

## 接手后的有限验收清单

- 生产配置/prepare-export-READY：必须从最终runtime/Agent核对workspace Root/epoch/Home/source/namespace；只有原private Node路径字符串不够。
- append：先复验E15的2MiB单syscall插入和非重叠SEEK_CUR；不得以短追加/正确最终内容代替完整偏移/原子性。
- classic POSIX锁：E4同进程不同view owner；报告PID/OFD锁不能代替实际kernel owner；当前非阻塞flock通过只覆盖E14范围。
- 映射/watch：E20在旧FUSE/remote的MAP_SHARED ENODEV及native事件未送达；能力位/缓存改动后需验证实际msync、对象、权限及事件合同。
- 回收/切换：E19原export Detached后容器仍可访问；登记并排空/撤掉最终容器及peer/旧引用后才复用backing；管理入口和普通rmdir/rename的EBUSY区分清楚。
- 性能：先看已测核心本地native数据；如决定追10k远端，核对metadata命中的全索引重建，然后用相同原负载复验。不得在本分支将通用可靠性/Meta改动扩展开。

本次PR不合入，与后续main重叠。接手AI应按所需bind原语做集成，不盲目搬入整条分支；改变共享代码/协议后重新资格化受影响证明。

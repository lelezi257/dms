# DMS M1 产品级总验收合同

> 状态（2026-09-17）：M1 总验收清单已有 24 个 implemented 用例，覆盖单 VM、三 VM、
> POSIX 子集、数据完整性、故障矩阵、资源回落、白盒路径、性能与干净交付。当前是否能发布
> 不再取决于 planned 项，而取决于最终 clean release run 是否在无脏源码环境中全部 PASS。

## 1. 为什么需要这一层

前面的 M1.1～M1.5 都有各自的 E2E，但“每个阶段曾经通过”不等于最终产品通过：后续修改可能
破坏旧能力，单个脚本也不能证明不存在跳过项、资源泄漏、跨节点旧读或性能路径退化。

M1.7 因此不再增加一组零散脚本，而是建立一个总合同：

1. 每个产品声明必须映射到至少一个具名用例。
2. 每个用例必须声明拓扑、执行入口、通过条件和保护的不变量。
3. 已实现能力缺少执行脚本是合同错误；未实现能力必须显式标为 `planned`。
4. 发布验收不接受 planned、skip、fail 或脏源码目录。
5. 每项结果必须带机器可读证据，不能仅凭终端里的一句“成功”。

## 2. 三个文件分别解决什么

| 文件 | 作用 | 不能做什么 |
| :--- | :--- | :--- |
| `acceptance-manifest.json` | M1 声明、用例、拓扑、执行入口和不变量的唯一清单 | 不能把尚未实现的用例写成 implemented |
| `known-gaps.json` | 产品非目标及临时豁免 | 不能用“已知问题”长期掩盖失败；豁免必须有 Issue 和到期里程碑 |
| `result.schema.json` | 单次运行结果的稳定机器格式 | 不能替代语义评估 |

`m1_acceptance.py` 校验这三个合同，并把一次运行判为：

- `PASS`：选中的已实现用例全部通过，并且发布模式没有任何缺口。
- `INCOMPLETE`：发现模式下已有用例通过，但选中范围仍有 planned 项或有期豁免。
- `FAIL`：用例失败、无豁免跳过、缺少结果、出现计划外结果、发布源码不干净或合同损坏。

## 3. 能力矩阵

| 领域 | 产品声明 | 主要验收 |
| :--- | :--- | :--- |
| Namespace | 共享目录树、mutation 原子性、返回后可见 | M1.1 单/三 VM E2E，后续 pjdfstest/fstests 子集 |
| Size | truncate、append、range write、hole、精确版本 | M1.2 单/三 VM E2E，fio 数据校验 |
| Identity | hard link、symlink、unlink-open、FORGET、回收 | M1.3 单/三 VM E2E |
| Attributes | chmod/chown/utimens、xattr、ACL、statfs | M1.4 单/三 VM E2E |
| Space & Sync | fallocate、punch、sync flags、ENOSPC | M1.5 单/三 VM E2E |
| Locks | fcntl/flock、等待、中断、故障回收 | M1.6a 单/三 VM E2E，POSIX 子集 |
| mmap | cached mmap、msync、远端失效、越 EOF | `run_filesystem_mmap_e2e.sh` 与 `run_filesystem_mmap_3vm.py`，C helper 捕获 SIGBUS/msync；远端失效还要用 kernel invalidation 与 Meta watch metrics delta 证明 ACK 顺序 |
| Distributed | Watch、Peer pull、重启和状态转换 | `run_m1_fault_matrix.py` 组合三条真实 3VM 子路径：size 负责 commit/Peer pull/restart，lock 负责等待与 epoch fencing，mmap 负责 Watch ACK 与 mmap 恢复；`run_m1_agent_workspace.py` 覆盖小文件压力 |
| Performance | RPC、复制、资源和延迟不退化 | 白盒门禁、冻结性能基线、`run_m1_resource_soak.py` 抓取 RSS/FD/thread、Arena reservation、inode refs 与 watch lag 的前后快照 |
| Delivery | 干净环境可安装、挂载、观测、卸载 | `run_m1_clean_delivery.py` 从发布 tar 解包启动，不使用源码路径 |

完整清单以 `scripts/validation/m1/acceptance-manifest.json` 为准；本文只解释其边界。

## 4. fast 与 full

`fast` 用于开发回归，包含单 VM 核心 E2E、白盒门禁和发布静态检查。它必须足够快，适合每次较大
修改后运行，但不能代替发布验收。

`full` 用于 M1 发布候选，包含三 VM、一致性与故障、数据完整性、分类性能合同、资源回落和干净
制品安装。发布结论只能来自 `purpose=release` 的 full 结果。

```text
开发修改
  └─ fast discovery
       ├─ FAIL       → 立即修复
       └─ PASS/INCOMPLETE
            └─ full discovery
                 ├─ 找到问题并修复
                 └─ 干净提交上的 full release
                      └─ 只有 PASS 才能声明 M1 验收完成
```

## 5. 当前如何使用

先进入已经启动好的 Linux 虚拟机和源码工作目录，再执行：

```bash
python3 scripts/validation/m1/m1_acceptance.py validate

python3 scripts/validation/m1/m1_acceptance.py plan \
  --tier fast \
  --topology single-vm \
  --output evidence/m1/plan-fast-single-vm.json

python3 scripts/validation/m1/m1_acceptance.py plan \
  --tier full \
  --topology three-vm \
  --output evidence/m1/plan-full-three-vm.json
```

当前清单 24 个用例均为 `implemented`；如果计划命令退出码为 `2`，说明合同或 profile 又出现
planned/缺命令等问题，不能继续发布验收。

各执行器完成后写出 `dms.m1.acceptance-result.v1` JSON，再运行：

```bash
python3 scripts/validation/m1/m1_acceptance.py evaluate \
  evidence/m1/result-full-three-vm.json \
  --output evidence/m1/evaluation-full-three-vm.json

python3 scripts/validation/m1/m1_acceptance.py render \
  evidence/m1/evaluation-full-three-vm.json \
  evidence/m1/evaluation-full-three-vm.html
```

## 6. 用户 Review 什么

当前 Review 不需要逐项重跑产品测试，重点看四件事：

1. M1 的产品声明有没有遗漏，或者把未来能力误写成当前能力。
2. 每个声明是否至少有一个真正能证明它的用例，而不是只测 happy path。
3. planned 与 implemented 的边界是否诚实；发布模式是否确实不接受跳过。
4. 单 VM、三 VM、故障、性能、数据校验和交付是否形成闭环。

后续修改 M1 时，只能在新增能力真实落地后扩展清单，并提供实际执行器和证据；不能为了让总结果
变绿而删除声明或降低阈值。M1.6b 的 `implemented` 现在同时具备执行入口和真实
单/三 VM 证据：`evidence/2026-09-16-filesystem-mmap-152340/` 与
`evidence/2026-09-16-filesystem-mmap-three-vm-1527/` 均为 `PASS`。

M1.7 的 `agent-workspace-stress` 现在具备真实三 VM 执行入口：
`scripts/validation/run_m1_agent_workspace.py`。它按固定随机种子生成 create/read/overwrite/
pwrite/append/rename/unlink/stat/readdir 混合操作，在 A 节点执行 mutation，在 B 节点逐步校验
跨节点可见性，并用 reference model 对 A/B 最终目录树做逐文件 digest 对比。该用例还输出
`request_amplification_summary` 与分操作延迟摘要，用于后续判断小文件路径是否出现非预期 RPC 或复制
放大；它不替代 fio 大文件完整性、故障矩阵或资源回落用例。

M1.7 的 `fio-integrity` 现在具备单 VM 与三 VM 执行入口：
`scripts/validation/run_m1_fio.py`。该用例以 fio verify 覆盖 4 KiB 顺序写、1 MiB 随机写与
默认 512 MiB 顺序大对象，再通过跨挂载或跨 VM 的 SHA-256 校验确认另一 Node 看到同一权威内容；同时覆盖
truncate shrink/grow、punch hole 与 MAP_SHARED mmap 写后校验。`fio` 是这个验收项的必需工具，
缺失时结果为环境失败而不是跳过。

M1.7 的 `fault-transition-matrix` 现在具备真实三 VM 执行入口：
`scripts/validation/run_m1_fault_matrix.py`。它不是一套新的简化 workload，而是顺序运行 size、
lock、mmap 三条已存在的三 VM 真实路径，再由 `evaluate_m1_fault_matrix.py` 检查 commit 发布、
Peer pull、Watch invalidation ACK、锁等待、Meta 重启、Node epoch fencing 与 mmap 恢复证据；
任一子路径失败都会让矩阵失败。

M1.7 的 `posix-pjdfstest-supported` 与 `fstests-generic-supported` 现在具备单 VM 执行入口：
`scripts/validation/run_m1_pjdfstest.py` 和 `scripts/validation/run_m1_fstests.py`。这两个用例解决的是
“我们声称支持哪些上游 POSIX case”这个合同问题：allowlist 固定在
`scripts/validation/m1/posix/`，runner 必须在真实 DMS FUSE mount 上运行对应上游工具；缺少
pjdfstest/xfstests、allowlist 中的 case 不存在、工具依赖不满足或 case 失败，都会输出
`FAIL/preflight` 或 `FAIL`，不允许用 skip 伪装通过。它们不替代 M1.1～M1.6 的专用 E2E，因为专用
E2E 负责跨节点、Watch、Peer pull、故障恢复和白盒路径；POSIX 上游子集负责防止常规 syscall 语义回退。

当前真实 POSIX suite 证据（`dms-dev` Linux/FUSE VM，2026-09-17）：

- `pjdfstest` 固定 revision：`85a8aea9e685999ef0540392fd80535f873d7ff7`
  （origin `https://github.com/pjd/pjdfstest.git`）。证据目录：
  `/tmp/dms-m1-g004/evidence/pjdfstest-root-discovery-4`。11 个冻结 case 全部 PASS。
- `xfstests` 固定 revision：`a370dcbed43563f0462801e889e0eceb93c7cfad`
  （origin `https://github.com/kdave/xfstests.git`）。证据目录：
  `/tmp/dms-m1-g004/evidence/fstests-root-timeout-fix`。runner 使用 DMS 自己启动的 FUSE mount，
  `TEST_DEV=dms-node` 对齐 FUSE `FSName`，不配置 `SCRATCH_*`，并以 root 执行上游要求的
  `./check -fuse generic/001 generic/013 generic/075`。结果为 `PASS`，三项全部通过。

这两个结果的含义是：M1 已具备真实上游 POSIX suite 接线和可复现证据；基础 FUSE/POSIX 子集已经
通过。后续增加 allowlist 时必须先让真实 suite 通过，不能把失败 case 写成 skip。

M1.7 性能现在分成两个独立门禁：

1. `performance-regression` 使用
   `benchmarks/whitebox/native-filesystem-vs-glue-contract.json`。本地热读与跨节点首读必须领先；
   同步 write-through 的 create/overwrite 属于突变路径，按固定控制路径的绝对微秒增量评价：create
   的配对中位 p50/p95 最多增加 400/500 µs，middle overwrite 最多增加 250/300 µs。
   不用百分比把同一份固定成本在小文件上不成比例地放大。
2. `whitebox-path-gate` 使用
   `benchmarks/whitebox/fuse-request-amplification-contract.json`。它只看 FUSE/DataCore/Meta/Peer
   请求次数、字节复制阶段和正确性，不重复判断延迟。

发布性能验收固定使用六轮对称交替顺序：Native→Glue 与 Glue→Native 各三次，保证双方获得相同
次数的先跑与后跑机会。每轮语料为 220 个文件：140×4 KiB、50×64 KiB、30×1 MiB；评价器
要求每个关键 case 至少 100 个样本。时延门禁先在同一轮内计算 Native/Glue 配对差异，再以六轮
中位数判定；读路径使用配对比值并继续要求领先，mutation 使用配对绝对增量并限制固定控制成本。
聚合 p50/p95 继续作为报告数据，但不允许单轮宿主调度抖动决定发布结果。

最新同场证据位于 `evidence/m1/g004-performance-safe-id-20260917-r1`：本地热读领先
44.5%～54.7%，跨节点首读领先 11.5%～29.7%；create 4 KiB/64 KiB 分别慢 10.96%/4.64%，
1 MiB create 快 2.97%；64 KiB 中段覆盖慢 9.1%，均符合分类合同。M1.7 三 VM 总验收
`evidence/m1/g004-full-three-vm-20260917-r7` 的性能原始数据在当前合同下重新评价为 PASS。

M1.7 的 `resource-return-to-baseline` 现在具备单 VM 与三 VM 执行入口：
`scripts/validation/run_m1_resource_soak.py --topology single-vm|three-vm`。单 VM 模式会在本机
Linux 启动一个 Meta、两个 Node 和两个 FUSE mount；三 VM 模式把同一套 A 写、B 读、Meta 独立
进程的 workload 分布到三个 VM。两种模式都会产生自己的 `m1-result.json`，不能用三 VM 结果冒充
single-vm。该用例在 create/write/read/unlink/lock/mmap 混合 workload 前后读取 `/proc` 与 DMS
metrics，要求 `arena_reservations`、`filesystem_inode_references`、`watch_lag_events` 回到零，
并限制 RSS、FD、线程和 Arena allocated 的增长预算。缓存、碎片和 allocator 保留内存允许在预算内
存在，但不能单调泄漏或留下未释放的业务引用。

M1.7 的 `clean-install-single-and-three-vm` 现在具备执行入口：
`scripts/validation/run_m1_clean_delivery.py`。它先构建或接收 `dms-server-*.tar.gz`，再解压到
全新的安装目录；后续启动、health、FUSE create/read、跨 VM 读取、client metrics host、停止和卸载
都只调用包内 `scripts/cluster.sh`、`bin/dms-*` 与包内配置，不从源码 `target/`、`server/` 或
`sdk/` 取运行依赖。缺少 `/dev/fuse`、`fusermount3`、`curl`、`limactl` 或包内校验失败都会非零
退出，不按 skip 处理。已有发布包时可以传 `--archive`；否则在 Linux 源码环境中临时构建一个包。

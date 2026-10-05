# Native bind 实测数据（2026-10-01）

这些是 [Issue #42](https://github.com/lelezi257/dms/issues/42) 的诊断数据。
`original_runner_passed` / `checks_ok` 表示特定实验和证据检查的结果；
`stage_pass=false` / `architecture_phase_pass=false` 保留完整阶段未通过。
不把同路径挂载成功、容器隔离通过或部分比值小于1当成完整 native 性能成立。
结论、负面结果及有限补充的原因见[性能报告](../../native-bind-performance.md)。

| 数据 | 范围 |
| --- | --- |
| [宿主 P1](native-bind-p1-20261001.json) | 81个原始/有限补充任务，保留原runner失败及MooseFS配置无效标记；绝对路径性能失败。 |
| [容器 P1](native-bind-container-p1-20261001.json) | 72任务，ext4/native/MooseFS，绝对/相对路径，并发1/8，包含实际OCI spec/source/隔离。 |
| [容器本地 IO](native-bind-container-io-20261001.json) | 360任务、30组合；guest冷/热/repeat、顺序/随机、close/fdatasync/fsync；包含容器独立挂载的回收反例。 |
| [同文件归因补充](native-bind-container-io-shared-20261001.json) | 48任务、四个预先指定案例；两lane访问同device/inode/路径，保留完整矩阵。 |

`rows` 的耗时比是逐轮配对中位数，不是两列耗时中位数相除；
`samples` 保留预热及每个正式样本的原结果/内容检查/CPU/操作分位数。
IO的屏障是任务最后一次文件同步，不是逐块同步；计时不包括目标缓存
准备及写后内容重放。原始资源窗口包含这些额外步骤，不能直接等同于
timed IO的资源量。guest-cold只证明guest页缓存前提，不证明宿主冷缓存。
MooseFS P1是stock单副本可见性对照，强持久性资格化仍未完成。

## 可供另一台电脑离线核验的原始证据

[raw/manifest.json](raw/manifest.json)列出六个诊断实验包与逐文件SHA：
E15追加反例、E17宿主完整/补充及三个完成的容器实验。
外层包保存原始`result.json`、`transcript.jsonl`、A/B/ctl三个原始归档、
观察器输出/篡改审计及冻结源码输入。**IOv2完整矩阵与当前IOv3补充的
冻结源码分别保留**，不声称v2来自当前v3文件。
不包含Node/Meta大执行文件、私钥或大工作集；guest原始归档中保留小型
syscall ELF/容器libc及其实际哈希，供核对执行对象。

在单独的新目录展开**外层**实验包，再执行：

```sh
python3 development/acceptance/probes/ownerfs_native_network_verify.py <实验目录>
```

观察器直接读取guest归档中的常规文件，不需要展开或执行容器rootfs；
它核对实际回复、逐文件/归档SHA、原export/source/namespace/spec、最终
容器内容/隔离、完整采样矩阵、缓存前提及正常收尾。`checks_ok=true`
不改变报告里的语义/性能未通过结论。E15包仍是语义FAIL；E17完整包仍
保留Actor超时/runner FAIL，仅资格化已完成的计时诊断。首次容器观察器
FAIL、缓存前提FAIL的原始副本仍在本地证据库，具体身份/SHA见报告；
本目录没有将这些失败包装成完成实验。

本候选仍基于`78245771167643d5883491052e7cebcaba8c3be2`，实测Rust源码为
`33c9304ea7b7e948aa49ac4966cbcb893269b80b`加E16测试构造选择补丁。
最终核对157个Rust/proto/Cargo输入与优化构建一致；后续main的通用可靠性
改动没有被合入或资格化。最终runtime/Agent READY、daemon发布、任意UID
映射、完整回收、远端及MooseFS IO/持久性性能矩阵仍开放。


## 2026-10-06 有界穿刺补充

[交接报告](../../native-bind-closeout.md)汇总E20架构正负向结果和E21实际五路径数据；[有限归因](../../native-bind-attribution.md)保留源码/RPC机制、屏障窗口和未定位残差。

- [五路径摘要](native-bind-closeout-performance-20261006.json)：统一较小负载120 metadata /240 IO，保留每项paired ratio和各lane绝对耗时。不是原10k/8GiB完整规模资格化。
- [归因分解](native-bind-closeout-attribution-20261006.json)、[157源码/优化产物/main重叠核对](native-bind-closeout-provenance-20261006.json)。
- [复现与接手验收](native-bind-closeout-reproduce.md)、[可移植交付核验](native-bind-closeout-delivery-verification-20261006.json)。

raw/manifest现在共13包：原6包完全保留，新增E20原FAIL/完成诊断、三个准备／路径碰撞FAIL、原10k remote FAIL、E21完成的较小负载。成功runner仅表示诊断采集完成，完整功能和正式性能都不因此PASS。新包使用closeout_verify；原FAIL只核对归档／输入完整性，不包装成完整矩阵PASS。包内源／脚本／程序SHA绑定实际执行对象，source snapshot不冒充后来main。

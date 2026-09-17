# G006 M1.7 收口清理计划

> 状态：已完成。本文只约束 G006 收口清理，不改变 M1 文件系统架构、公开 SDK 接口或验收拓扑。

## 1. 行为锁定证据

本轮清理前已经具备以下行为锁定，不允许通过降低语义来换取绿色结果：

- M1 单 VM：`evidence/m1/g004-full-single-vm-20260917-r3`，13/13 PASS。
- M1 三 VM：`evidence/m1/g004-full-three-vm-20260917-r7`，真实用例 12/12 PASS；按当前性能合同重新评价后 performance 与 amplification 均 PASS。
- POSIX 上游子集：pjdfstest 11/11 PASS，fstests generic 3/3 PASS。
- JuiceFS+DMS Glue 与 Native 同场性能：`evidence/m1/g004-performance-safe-id-20260917-r1`。
- Go SDK 真 SHM 集成：`TestIntegrationSetFromUsesRealSharedMemory` 已覆盖 AllocateStaging → AcquireRegion/SCM_RIGHTS → mmap → Set/Get。
- Python evaluator 与 runner 单元测试已覆盖 M1 runner、性能 evaluator、请求放大 evaluator、干净交付 runner、fio/resource/agent workspace evaluator。

## 2. 清理范围

本轮只处理：

1. `docs/` 中与 M1.7 总验收、FUSE 请求放大、lock/mmap、文件身份生命周期相关的过期描述。
2. `scripts/performance/**` 和 `scripts/validation/**` 中 M1.7 验收脚本的错误可见性、重复 evaluator 和 profile/manifest 合同。
3. `private/` 与 `scripts/**/__pycache__` 本地生成物。

不处理：

- Rust 文件系统、Meta、Node 的业务重构。
- 公开 SDK API、protobuf 语义和传输协议。
- 长期 Roadmap 中 M2/M3 功能。

## 3. Smell 清单与处理顺序

| 顺序 | 问题 | 处理 |
| :--- | :--- | :--- |
| 1 | 旧 `evaluate_repeated_fuse_performance.py` 仍表达历史“多轮 5% 稳定回归”门禁，与当前“性能分类合同 + 请求放大合同”冲突 | 删除脚本与测试，文档改为双门禁 |
| 2 | `acceptance-manifest.json` 还有全局 5% 阈值和旧期望文字 | 改成分类性能合同，不再声明全局 5% |
| 3 | `single-vm.json` 默认 suite 路径带 `g004` 阶段名 | 改成阶段无关 `/tmp/dms-m1/...`，仍可通过 `--variable` 覆盖 |
| 4 | 远端日志收集失败、malformed metrics、cleanup 失败容易被静默吞掉 | 记录 warning 或失败证据；不覆盖主失败，但不能无痕消失 |
| 5 | `docs/filesystem-file-identity-lifecycle.*`、`docs/performance/fuse-request-amplification-audit.*`、`docs/m1-acceptance.*` 有旧证据/旧阈值/旧 planned 描述 | 按当前 24 项 implemented、性能/放大双门禁和最新证据刷新 |
| 6 | `private/tmp/dms-profile-validation-probe` 与 `scripts/**/__pycache__` 是本地污染 | 删除文件并确认不再出现在 `git status` |

## 4. 回归测试计划

- Python targeted：
  - `python3 -m unittest scripts.performance.test_evaluate_fuse_request_amplification`
  - `python3 -m unittest scripts.performance.test_evaluate_native_filesystem`
  - `python3 -m unittest scripts.performance.test_native_filesystem_workload`
  - `python3 -m unittest scripts.performance.test_build_juicefs_with_current_sdk`
  - `python3 -m unittest scripts.validation.m1.test_m1_runner`
  - `python3 -m unittest scripts.validation.test_run_m1_clean_delivery`
  - `python3 -m unittest scripts.validation.test_posix_suite_runner`
  - `python3 -m unittest scripts.validation.test_evaluate_m1_fio`
  - `python3 -m unittest scripts.validation.test_evaluate_m1_resource_soak`
- 文档渲染：
  - `python3 scripts/docs/render_review.py docs/m1-acceptance.md docs/m1-acceptance.html --check`
  - `python3 scripts/docs/render_review.py docs/filesystem-file-identity-lifecycle.md docs/filesystem-file-identity-lifecycle.html --check`
  - `python3 scripts/docs/render_review.py docs/filesystem-lock-mmap-contract.md docs/filesystem-lock-mmap-contract.html --check`
  - `python3 scripts/docs/render_review.py docs/performance/fuse-request-amplification-audit.md docs/performance/fuse-request-amplification-audit.html --check`
- 最终门禁：
  - `git diff --check`
  - Linux VM `cargo fmt --check`
  - Linux VM `cargo check --workspace --all-targets`
  - Linux VM `cargo clippy --workspace --all-targets -- -D warnings`
  - Linux VM `cargo test --workspace`
  - 最终 clean single-vm / three-vm release run。

## 5. Fallback-like findings

- 远端日志收集失败、cleanup 失败属于诊断/清理边界的 fail-safe fallback，可以保留“不中断主业务失败”的行为，但必须写入机器证据。
- malformed metrics 属于验收证据损坏，不能默默当成空指标；必须让 evaluator 或 runner 产出失败/警告。
- 删除旧重复 evaluator 不改变产品行为，只移除已经被新双门禁取代的历史判断路径。

## 6. 最终结果

- 独立代码 Review：APPROVE；独立架构复审：CLEAR。
- Linux workspace fmt/check/clippy/test 全部通过。
- 最终单 VM release：13/13 PASS，0 FAIL、0 SKIP、0 warning。
- 最终三 VM release：14/14 PASS，0 FAIL、0 SKIP、0 warning。
- 结果入口：`docs/reviews/g006-m1-final-acceptance.html`。

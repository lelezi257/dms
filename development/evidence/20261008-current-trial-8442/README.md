# 8442 默认 OFF 普通试用包：安装与核心恢复独立小项

**当前限定PASS，非完整POSIX/性能或G2.27全出口。** 产品源码[8442b55e](https://github.com/lelezi257/dms/commit/8442b55ec25b018aad0898bd0f8679232e393f09)，158 compiler-input map `d053a333988e9fa7fc11efc51b94a7272e841f26f3aca7351f64a015f160a16a`；[原完整输入/构建身份](../20261008-workspace-bind-root-command/compiler-inputs.json)与本项逐文件相同，复用已过Linux release ELF，不另开Cargo。打包输入固定guide-only e7bc3dce，13文件SHA与Git原blob核对，[输入映射](package-source-inputs.json)；manifest绑定产品源码，不能混淆为旧f03或仅凭版本号继承结果。

## 事实与范围

- 包 `afs-0.1.0-g2-main-8442b55-linux-aarch64.tar.gz`，14,453,702B，SHA256 `31ba38e0d6a34431492b5994c2accbc91583131dcdfab90735fe05d2af9731bd`。Linux两次由相同输入打包，归档字节一致；逐成员16文件/6目录、权限、包内SHA核对，无test probe/issuer、源码、rootfs或私钥。[复现清单](package-reproduction.json)。
- Meta ELF `e2bbf9bcfa6d99e0674f8f55455441cfad3b5f154ac9b73862f7e48fe0ffd77b`；Node ELF `2094eadc23cc8f57e165b080330686dbd8886f871ee3a9ddcaf61b09657660f0`。[候选身份](candidate.json)。
- 沿用Linux ARM64 `afs-g2-micro`/ext4、全新 `/var/tmp/afs-current-trial-8442-20261008-r1`。启动前一次性依赖、编译器缺席、端口、FUSE、ELF/共享库、RAM/cgroup、挂载及容量准入，未修环境。安装后两产品在运行时确为安装路径的inode/ELF；两workspace开关均OFF，local-file Meta、gRPC/DFS R1 generated/effective严格相符。
- 单次实际运行43必要谓词PASS。OwnerFs和DFS各64MiB，写入确认并目录屏障后仅正常停止/启动Meta，原Node/两个挂载不变，完整SHA/EOF读回。非Node重启、崩溃或断电恢复。3实际wait0/6child及supervisor PID消失，自有UDS、FUSE正常闭合。
- 1保护进程、50历史ELF inode/SHA及26原mount行不变，未停止A/ctl memory服务或重建任何VM。最终采样分配235,106,304B < 测前512MiB ceiling；free6,339,956,736B > 1GiB floor。不是连续峰值测量。
- 日志10,308B、26ERRO/12WARN完整保留、无TRACE；含user.*之外xattr安全拒绝、NotFound和未实现ioctl诊断。不能由小自检推断完整POSIX或零错误。[独立Linux保存数据审计](independent-stored-audit.json)、[日志索引](log-summary.json)。
- 15既有Linux driver回执/安装身份/config guards按SHA完全相同直接复用，非本轮重跑。[工具复用](tool-qualification-reuse.json)。旧f03安装、标准/性能和三副本结果保留原版本/范围；新包R1恢复不升级为新候选R3、全标准或性能。

## 失败与恢复证据

首独立审计误漏 `run/` 回执前缀，首恢复程序误用工具索引 `files` 而实际为 `tools`；均为保存数据工具错误，原代码/命令/退出1留档。仅修读证工具，R2独审与实际恢复PASS，产品未重跑、环境未修。[审计补记](audit-correction.json)。

原始证据留在源码树外，Git只保留此紧凑索引与结果：主archive78,659B/SHA `bb136e8fbff8d37249431670334824980ed9ec065ecf13d704da8b8e63e816a6`，100外层成员；其中唯一 `runtime-text.tar`256,000B/SHA `330e62018439048f7ac9e93101997ee1258d75afda9d85c99c08a734c364ca26`，93guest文本。[逐文件SHA/字节/路径](raw-index.json)。Linux临时目录实际解包100/93项，并由固定Git原blob恢复158compiler、13打包及2维护工具输入；首FAIL/回执保留，[恢复结果](archive-restoration.json)。后续恢复工具/首失败与回执另存2,316B证明archive，[索引](restoration-proof-index.json)。未将普通ELF、大包、私钥或整份源码快照放进Git；这不是完整VM或用户数据备份，不授权删除旧夹具。

## 交付及下一项

**固定发布BLOCKED：** `gh release create`在固定120s后超时，随后只读GET为HTTP404，无可核验的新Release；已停止发布写操作；[用户已决定](release-decision.json)等待GitHub恢复并继续独立标准核对，不重复上传。初wrapper未留TimeoutExpired的stdout/stderr，证据缺口明确，不伪称完整错误流；命令、超时身份、后续404及其他回执见[阻塞](publication-blocked.json)及[发布原始索引](publication-raw-index.json)。代码/运行证据已正常推送[main637394e9](https://github.com/lelezi257/dms/commit/637394e93808d426485fc48633f21831c6d7bb86)，Git远端HEAD核对；首次API EOF保存，独立Git读成功。本地四个交付文件已齐；新包安装/恢复仍限定PASS，不因GitHub发布阻塞否定。

固定prerelease目标为[afs-trial-8442b55](https://github.com/lelezi257/dms/releases/tag/afs-trial-8442b55)；仅在 `publication.json` 标记远端发布和每附件digest核对PASS后，才能称已交付。包、SHA256SUMS、GUIDE.md和TRIAL_MANIFEST.json为四附件，历史f03/7e6/6d资产不覆盖。

[普通使用指南](../../../docs/guides/trial.md)提供Linux安装与正常Meta恢复步骤。G1历史8/8关闭，G2完成计数不因补验增加；普通Owner Moose吞吐1.2倍/独立时延.8倍、bind native ext4、DFS三同步持久副本3FS判据不变。新包没有性能对照或操作时延，所以性能仍待验。下一独立小项是8442 default-OFF标准测试的代码路径影响映射：只复用身份/判据未变范围，仅补受影响路径，不盲跑全套；历史通过不冒称新版本实跑。普通点优化、3FS资格、生产issuer/ACK、大规模及复杂可靠性后置。

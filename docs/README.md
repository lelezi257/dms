# AFS 文档

本目录只保留当前可维护的产品文档。历史过程、逐轮证据、旧 checkpoint 和失败原始材料已移到源码仓上一级 `local-archive/`；它们可以用于追溯，不作为产品文档入口。

## 当前入口

- [当前计划](development/plan.md)：阶段、优先级、验收出口和迁移前阻塞项。
- [架构总览](architecture.md)：系统组件和数据路径。
- [OwnerFs](architecture/ownerfs.md)：workspace、Home、远端访问和 bind mount 场景。
- [测试与验收](testing/acceptance.md)：标准测试、核心功能、性能目标和后置项。
- [部署与试用](deployment/trial.md)：当前 a103 试用包、ON 配置和支持范围。
- [验证方式](testing/validation.md)：Linux 验证原则和受影响检查选择。
- [fuser 官方版迁移阻塞说明](development/fuser-official-blocker.md)：记录第三方依赖暂不能无损解耦的原因。

## 目录约定

- `architecture/`：稳定机制和设计合同。
- `development/`：当前计划、任务 ID、优先级和迁移前待办。
- `testing/`：验收标准、性能判据和测试运行方式。
- `deployment/`：构建、配置、试用包和运行说明。

文档描述当前实现和已接受目标；未完成能力只放在计划或明确的“未完成”段落，不写成既有能力。

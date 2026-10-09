# 验证方式

## 基本原则

- 文件系统、Rust 构建和性能结论只在 Linux 上确认。
- 先跑受影响检查；只有行为边界扩大时才扩大验证范围。
- 已通过且代码、环境、判据未变化的结果可以复用，但必须保留原版本身份。
- 测试工具就绪不等于产品目标完成。

## 常用检查

```sh
cargo fmt --check
cargo test --locked --features ownerfs,dfs
cargo build --release --locked --bin afs-node --bin afs-meta
```

实际选择以改动范围为准。涉及 FUSE、mount、权限、runc、workspace bind 或性能的检查需要 Linux VM、`/dev/fuse`、`fusermount3` 和对应测试环境。

## 验收工具

维护中的验收工具在 `tests/acceptance/`。运行前检查配置文件、二进制路径、挂载点、容量门禁和套件版本。原始运行输出放在仓库外归档，不提交到源码树。

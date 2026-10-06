# 容器 workspace 经典锁：本轮收口与剩余边界

2026-10-07。**事实：** 固定产品6d51aeb/157输入未改；受管容器混合路径锁仍FAIL，bind默认OFF。本轮仅核对第一方调用点与Linux物理锁原语，不实现一个削弱POSIX语义的桥，也不计G2.12/13通过。

## 已有失败与原因

[原混合语义证据](evidence/20261007-managed-semantics/README.md)保留native/native 8/8及mixed 3/8、五项失败。混合重叠独占、共享/独占和BSD独占错误成功，阻塞等待者在解锁前出现ACQUIRED。后两项已按真实有界读取修正判据；不把“子进程仍存活”当作仍在等锁。

native导出绑定真实ext4 inode；FUSE getlk/setlk则经Home使用第一方内存LockTable。这两个路径未共享物理锁权威。调用点：`src/node/fuse.rs` getlk_with_options/setlk_with_options/flush，`src/node/vfs/ownerfs.rs` OwnerLockRegistry/get_file_lock/set_file_lock，`src/node/vfs/locks.rs` LockTable。当前flush释放PosixOwner，release释放FlockOwner；这些生命周期与权限、session/epoch和取消检查不能删去。

## 原语验证的范围

第一方[维护探针](acceptance/probes/native_lock_kernel.py)在已有ARM64 Linux afs-build的ext4上用真实独立进程记录：OFD与经典POSIX锁双向冲突、非重叠、共享、部分解锁，以及两种关闭语义。版本、原始命令/返回值及结果见[证据索引](evidence/20261007-native-lock-kernel/README.md)。该机kernel6.8.0-142；它不是原容器运行机kernel6.8.0-106上的产品回归。

**事实：** 原语互操作不等于透明POSIX适配。实测native F_GETLK看到OFD冲突时`l_pid=-1`；POSIX任意同inode FD关闭释放该进程锁，OFD则最后一个引用关闭才释放。Linux的[fcntl锁手册](https://man7.org/linux/man-pages/man2/fcntl_locking.2.html)和[glibc OFD说明](https://sourceware.org/glibc/manual/2.23/html_node/Open-File-Description-Locks.html)说明这些差异及OFD不提供经典死锁检测。阻塞/取消、PID namespace、同一客户端混合路径、不同access mode均未由本探针资格化。

## 当前决策

不在守护进程共享FD上直接加fcntl：不同客户端会被混成同一个owner。独立open可隔离OFD，但仍不能透明保留native process owner和any-close；同owner只读/写句柄升级也不能靠偷偷改O_RDWR、换FD丢锁或多个相互冲突的OFD解决。原FD的procfd reopen还需保留权限检查、原inode身份及实际错误。

[libfuse 3.16.2 flush契约](https://github.com/libfuse/libfuse/blob/fuse-3.16.2/include/fuse_lowlevel.h#L602)要求每次close释放该inode/lock_owner的POSIX锁。未来适配还必须覆盖取消与grant竞争、flush/session/epoch失效后迟到grant、错误传播和native解锁唤醒；单独关闭阻塞线程的FD不能充当可靠取消。QEMU 7.2的[virtiofsd示例](https://github.com/qemu/qemu/blob/v7.2.0/tools/virtiofsd/passthrough_ll.c)使用独立OFD，但其阻塞请求分支拒绝执行，不能当作我们完整阻塞锁实现的完成证明。

本独立项到此收口：原FAIL、探针与候选身份可追溯，产品缺口保留。继续简单可独立验收的E2E项；只有出现可满足必要语义的明确方案时再修本项，不继续重复原测试或改第三方源码。append偏移、mixed watch和[R2官方fuser缺口](repository-remediation.md)独立保留，不由本原语结果解除。

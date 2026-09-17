//! Linux kernel page cache invalidation boundary for the filesystem entry.
//!
//! 这里刻意只保存 FUSE notifier 这个内核通知句柄，不保存页、版本或文件内容。
//! 文件内容授权仍由 Node 的 BindingCache/DataCore/Meta Watch 管理；本模块只是把
//! “远端版本已经发布，本 Node 必须丢弃旧页缓存”这个结果通知给 Linux kernel。

use std::sync::{Arc, Mutex};

#[cfg(all(target_os = "linux", feature = "fuse"))]
use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

use super::super::metrics::{FilesystemKernelInvalidationResult, NodeMetrics};

#[cfg(all(target_os = "linux", feature = "fuse"))]
use fuser::Notifier;

/// FUSE `notify_inval_inode` 的全 inode 范围。
///
/// Linux/FUSE 的 `(off, len)` 表达的是需要失效的数据范围；`len = -1` 是常用的
/// “从 offset 到文件末尾/整段有效范围”语义。这里失效的是 inode→版本授权，而不是
/// 精确 dirty range，所以统一从 0 到 EOF。不能用 `i64::MAX` 假装无限长度，避免未来
/// kernel 或库实现把它当作真实长度处理。
#[cfg(all(target_os = "linux", feature = "fuse"))]
const WHOLE_INODE_INVALIDATION_LEN: i64 = -1;

#[derive(Clone)]
pub(crate) struct KernelCacheInvalidator {
    inner: Arc<Mutex<KernelCacheState>>,
    metrics: NodeMetrics,
}

struct KernelCacheState {
    /// `true` 表示本进程配置了 FUSE mount，Watch ACK 前必须完成 kernel invalidation。
    ///
    /// 未配置 FUSE 或非 FUSE 构建时为 `false`，Meta Watch 仍能正常 ACK；此时没有
    /// Linux kernel page cache 需要失效。
    required: bool,
    #[cfg(all(target_os = "linux", feature = "fuse"))]
    notifier: Option<Notifier>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum KernelCacheInvalidationError {
    #[error("FUSE notifier is not installed yet")]
    NotifierNotInstalled,
    #[cfg(all(target_os = "linux", feature = "fuse"))]
    #[error("FUSE kernel invalidation failed: {0}")]
    Notify(#[from] std::io::Error),
}

impl KernelCacheInvalidator {
    #[allow(
        dead_code,
        reason = "非 FUSE 构建和测试夹具需要显式 disabled 入口；Linux FUSE 发布 target 只使用 new(required=false)"
    )]
    pub(crate) fn disabled(metrics: NodeMetrics) -> Self {
        Self::new(false, metrics)
    }

    pub(crate) fn new(required: bool, metrics: NodeMetrics) -> Self {
        Self {
            inner: Arc::new(Mutex::new(KernelCacheState {
                required,
                #[cfg(all(target_os = "linux", feature = "fuse"))]
                notifier: None,
            })),
            metrics,
        }
    }

    #[cfg(all(target_os = "linux", feature = "fuse"))]
    pub(crate) fn install(&self, notifier: Notifier) {
        self.inner
            .lock()
            .expect("kernel cache invalidator")
            .notifier = Some(notifier);
    }

    /// 失效整个 inode 的 metadata/data page cache。
    ///
    /// 当前 Meta Watch 事件表达的是 inode→exact object version 授权失效，而不是
    /// range-level dirty page 协议，所以这里对整个 inode 做 invalidate。若将来 Watch
    /// 携带 range，再只在本边界缩小范围，不能把 range 状态扩散到 FUSE callback。
    pub(crate) fn invalidate_inode(&self, inode: u64) -> Result<(), KernelCacheInvalidationError> {
        #[cfg(all(target_os = "linux", feature = "fuse"))]
        {
            let (required, notifier) = {
                let state = self.inner.lock().expect("kernel cache invalidator");
                (state.required, state.notifier.clone())
            };
            if !required {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Skipped,
                );
                return Ok(());
            }
            let Some(notifier) = notifier else {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Error,
                );
                return Err(KernelCacheInvalidationError::NotifierNotInstalled);
            };
            if let Err(error) = notifier.inval_inode(inode, 0, WHOLE_INODE_INVALIDATION_LEN) {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Error,
                );
                return Err(error.into());
            }
            self.metrics
                .record_filesystem_kernel_invalidation(FilesystemKernelInvalidationResult::Ok);
            Ok(())
        }

        #[cfg(not(all(target_os = "linux", feature = "fuse")))]
        {
            let _ = inode;
            let required = self
                .inner
                .lock()
                .expect("kernel cache invalidator")
                .required;
            if required {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Error,
                );
                Err(KernelCacheInvalidationError::NotifierNotInstalled)
            } else {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Skipped,
                );
                Ok(())
            }
        }
    }

    /// 失效目录中一个具体名字的 positive/negative dentry cache。
    ///
    /// inode invalidation 不等价于 dentry invalidation：create/unlink/rename 后，内核
    /// 可能仍然保留“该名字存在/不存在”的 lookup 结果。Meta Watch 所以携带
    /// `(parent, name)`，就是为了在 ACK 前完成这个边界。
    pub(crate) fn invalidate_entry(
        &self,
        parent: u64,
        name: &[u8],
    ) -> Result<(), KernelCacheInvalidationError> {
        #[cfg(all(target_os = "linux", feature = "fuse"))]
        {
            let (required, notifier) = {
                let state = self.inner.lock().expect("kernel cache invalidator");
                (state.required, state.notifier.clone())
            };
            if !required {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Skipped,
                );
                return Ok(());
            }
            let Some(notifier) = notifier else {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Error,
                );
                return Err(KernelCacheInvalidationError::NotifierNotInstalled);
            };
            if let Err(error) = notifier.inval_entry(parent, OsStr::from_bytes(name)) {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Error,
                );
                return Err(error.into());
            }
            self.metrics
                .record_filesystem_kernel_invalidation(FilesystemKernelInvalidationResult::Ok);
            Ok(())
        }

        #[cfg(not(all(target_os = "linux", feature = "fuse")))]
        {
            let _ = (parent, name);
            let required = self
                .inner
                .lock()
                .expect("kernel cache invalidator")
                .required;
            if required {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Error,
                );
                Err(KernelCacheInvalidationError::NotifierNotInstalled)
            } else {
                self.metrics.record_filesystem_kernel_invalidation(
                    FilesystemKernelInvalidationResult::Skipped,
                );
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use dms_metrics::{encode_text, registry};

    use super::*;

    #[test]
    fn disabled_invalidator_does_not_block_watch_ack() {
        let registry = registry();
        let metrics = NodeMetrics::register(&registry).expect("node metrics");
        let invalidator = KernelCacheInvalidator::disabled(metrics);

        invalidator
            .invalidate_inode(42)
            .expect("no FUSE mount means no kernel cache to invalidate");
        invalidator
            .invalidate_entry(1, b"file")
            .expect("no FUSE mount means no kernel dentry cache to invalidate");
    }

    #[test]
    fn required_invalidator_blocks_ack_until_notifier_is_installed() {
        let registry = registry();
        let metrics = NodeMetrics::register(&registry).expect("node metrics");
        let invalidator = KernelCacheInvalidator::new(true, metrics);

        assert!(matches!(
            invalidator.invalidate_inode(42),
            Err(KernelCacheInvalidationError::NotifierNotInstalled)
        ));
    }

    #[test]
    fn invalidation_metric_has_bounded_result_labels() {
        let registry = registry();
        let metrics = NodeMetrics::register(&registry).expect("node metrics");
        KernelCacheInvalidator::disabled(metrics.clone())
            .invalidate_inode(42)
            .expect("disabled invalidation");
        let _ = KernelCacheInvalidator::new(true, metrics).invalidate_inode(42);

        let text = encode_text(&registry).expect("encode metrics");
        assert!(
            text.contains("dms_node_filesystem_kernel_invalidations_total{result=\"skipped\"} 1")
        );
        assert!(
            text.contains("dms_node_filesystem_kernel_invalidations_total{result=\"error\"} 1")
        );
        assert!(text.contains("dms_node_filesystem_kernel_invalidations_total{result=\"ok\"} 0"));
    }

    #[cfg(all(target_os = "linux", feature = "fuse"))]
    #[test]
    fn whole_inode_invalidation_uses_fuse_eof_length() {
        assert_eq!(WHOLE_INODE_INVALIDATION_LEN, -1);
    }
}

//! 两种文件后端共用的接入接口与 namespace 分派。
//!
//! 供 FUSE 等入口使用，统一必要的操作形状及 inode/dentry/句柄映射边界。
//! FUSE 节点号、持久文件身份与后端版本身份不能混为一谈；两种 namespace 隔离。
//! 共用接口不意味着统一缓存、恢复或持久化模型，具体语义分别由后端持有。
//! 本轮只固化入口分派和占位操作，不伪造持久 inode 或完整文件对象。

#[cfg(feature = "blobfs")]
pub mod blobfs;
#[cfg(feature = "ownerfs")]
pub mod ownerfs;

use std::{fmt, sync::Arc};

use afs_error::{Error, ErrorKind, Result};
use afs_metrics::{IntCounterVec, MetricsError, Opts, Registry, register_collector};

#[cfg(feature = "blobfs")]
use self::blobfs::BlobFs;
#[cfg(feature = "ownerfs")]
use self::ownerfs::OwnerFs;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
/// 选择业务后端的入口类型，不是某个 workspace 的 RootId，也不是权限凭证。
pub enum Namespace {
    OwnerFs,
    BlobFs,
}

impl Namespace {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OwnerFs => "ownerfs",
            Self::BlobFs => "blobfs",
        }
    }

    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "ownerfs" => Ok(Self::OwnerFs),
            "blobfs" => Ok(Self::BlobFs),
            _ => Err(Error::new(
                ErrorKind::NotFound,
                format!("unknown namespace '{name}'"),
            )),
        }
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateRequest {
    pub namespace: Namespace,
    pub name: String,
}

/// FUSE 共用的最小业务接口；以一个 create 方法证明两后端能接到同形请求。
/// 后续按真实文件操作扩展，不把两后端的缓存、恢复、发布状态合并到这个 trait。
pub trait Backend: Send + Sync {
    fn namespace(&self) -> Namespace;
    fn create(&self, request: &CreateRequest) -> Result<()>;
}

#[derive(Clone)]
/// 持有运行时已启用的后端，集中做入口分派和观测。
/// `Arc<dyn Backend>` 是进程内调用，没有 FUSE→后端的本机网络 RPC。
pub struct Vfs {
    ownerfs: Option<Arc<dyn Backend>>,
    blobfs: Option<Arc<dyn Backend>>,
    metrics: VfsMetrics,
}

impl fmt::Debug for Vfs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Vfs")
            .field("namespaces", &self.namespaces())
            .finish()
    }
}

impl Vfs {
    pub fn new(ownerfs: bool, blobfs: bool, registry: Registry) -> Result<Self> {
        if !ownerfs && !blobfs {
            return Err(Error::new(
                ErrorKind::InvalidArgument,
                "at least one filesystem backend must be enabled",
            ));
        }
        let metrics = VfsMetrics::register(&registry).map_err(metrics_error)?;

        Ok(Self {
            ownerfs: build_ownerfs(ownerfs)?,
            blobfs: build_blobfs(blobfs)?,
            metrics,
        })
    }

    #[must_use]
    pub fn namespaces(&self) -> Vec<Namespace> {
        let mut namespaces = Vec::with_capacity(2);
        if self.ownerfs.is_some() {
            namespaces.push(Namespace::OwnerFs);
        }
        if self.blobfs.is_some() {
            namespaces.push(Namespace::BlobFs);
        }
        namespaces
    }

    #[must_use]
    pub fn has_namespace(&self, namespace: Namespace) -> bool {
        self.backend(namespace).is_some()
    }

    /// 验证单个目录项名字 → 找启用后端 → 调用 create → 记录结果。
    /// 当前后端有意返回 Unsupported，因此只验证接线，不创建持久文件或分配真实句柄。
    pub fn create_file(&self, namespace: Namespace, name: impl Into<String>) -> Result<()> {
        let request = CreateRequest {
            namespace,
            name: name.into(),
        };
        validate_child_name(&request.name)?;
        let backend = self.backend(namespace).ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                format!("namespace '{}' is not enabled", namespace.as_str()),
            )
        })?;

        let result = backend.create(&request);
        self.metrics.record_create(namespace, &result);
        result
    }

    fn backend(&self, namespace: Namespace) -> Option<&Arc<dyn Backend>> {
        match namespace {
            Namespace::OwnerFs => self.ownerfs.as_ref(),
            Namespace::BlobFs => self.blobfs.as_ref(),
        }
    }
}

#[cfg(feature = "ownerfs")]
fn build_ownerfs(enabled: bool) -> Result<Option<Arc<dyn Backend>>> {
    Ok(enabled.then(|| Arc::new(OwnerFs::new()) as Arc<dyn Backend>))
}

#[cfg(not(feature = "ownerfs"))]
fn build_ownerfs(enabled: bool) -> Result<Option<Arc<dyn Backend>>> {
    if enabled {
        Err(Error::new(
            ErrorKind::Unavailable,
            "ownerfs was not compiled into this binary",
        ))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "blobfs")]
fn build_blobfs(enabled: bool) -> Result<Option<Arc<dyn Backend>>> {
    Ok(enabled.then(|| Arc::new(BlobFs::new()) as Arc<dyn Backend>))
}

#[cfg(not(feature = "blobfs"))]
fn build_blobfs(enabled: bool) -> Result<Option<Arc<dyn Backend>>> {
    if enabled {
        Err(Error::new(
            ErrorKind::Unavailable,
            "blobfs was not compiled into this binary",
        ))
    } else {
        Ok(None)
    }
}

fn validate_child_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        Err(Error::new(
            ErrorKind::InvalidArgument,
            "file name must be one path component",
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone)]
struct VfsMetrics {
    backend_operations_total: IntCounterVec,
}

impl VfsMetrics {
    fn register(registry: &Registry) -> std::result::Result<Self, MetricsError> {
        registry.get_or_register(|registry| {
            let metrics = Self {
                backend_operations_total: IntCounterVec::new(
                    Opts::new(
                        "afs_vfs_backend_operations_total",
                        "VFS operations dispatched to filesystem backends.",
                    ),
                    &["namespace", "operation", "result"],
                )?,
            };
            register_collector(registry, &metrics.backend_operations_total)?;
            Ok(metrics)
        })
    }

    fn record_create(&self, namespace: Namespace, result: &Result<()>) {
        self.backend_operations_total
            .with_label_values(&[
                namespace.as_str(),
                "create",
                result_label(result.as_ref().map(|_| ())),
            ])
            .inc();
    }
}

fn result_label(result: std::result::Result<(), &Error>) -> &'static str {
    match result {
        Ok(()) => "ok",
        Err(error) => match error.kind() {
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::InvalidArgument => "invalid_argument",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Unavailable => "unavailable",
            ErrorKind::Io => "io",
            ErrorKind::Conflict => "conflict",
        },
    }
}

fn metrics_error(error: MetricsError) -> Error {
    Error::new(
        ErrorKind::Unavailable,
        format!("VFS metrics registration failed: {error}"),
    )
}

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
pub mod types;

use std::{ffi::OsStr, fmt, sync::Arc};

use afs_error::{Error, Result};
use afs_metrics::{IntCounterVec, MetricsError, Opts, Registry, register_collector};

#[cfg(feature = "blobfs")]
use self::blobfs::BlobFs;
#[cfg(feature = "ownerfs")]
use self::ownerfs::OwnerFs;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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
            _ => Err(afs_error::Error::coded(
                afs_error::NODE_VFS_NOT_FOUND,
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

/// 两种后端接收同一组文件操作；每项默认拒绝，未实现时不得报告成功。
///
/// 这里仅规定入口必须提供的参数与返回值。根授权、文件身份、缓存和持久化
/// 由具体后端决定。回调是同步的，以匹配当前 fuser 入口；后端可以用自己的
/// I/O 执行器，但不能在锁内无限等待远端或把未完成的操作假装完成。
pub trait Backend: Send + Sync {
    fn namespace(&self) -> Namespace;

    /// 基础框架的旧诊断入口：只验证 namespace 分派，永不创建文件。
    /// 真正文件业务接通 FUSE 时删除此探针，改由下方 `create` 原子返回文件及句柄。
    fn probe_create(&self, request: &CreateRequest) -> Result<()>;

    /// 仅在已选后端的父 inode 下查一个目录项。OwnerFs 应先由父 inode
    /// 找到 WorkspaceRoot，再校验其 RootGrant；不能按名字再查一次中心。
    fn lookup(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
    ) -> Result<types::Entry> {
        Err(unsupported("lookup"))
    }

    /// 传入 handle 时查询打开的旧对象；同名文件被删除重建后不能转向新文件。
    fn getattr(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
        _handle: Option<types::FileHandle>,
    ) -> Result<types::FileAttributes> {
        Err(unsupported("getattr"))
    }

    /// chmod/chown/truncate/时间更新共用一个可选字段结构；已打开 FD 的
    /// 属性更新优先按 handle 执行，不能依靠可能已变化的路径。
    fn setattr(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
        _handle: Option<types::FileHandle>,
        _change: &types::AttributeChange,
    ) -> Result<types::FileAttributes> {
        Err(unsupported("setattr"))
    }

    /// 一个 create 回调同时产生目录项与打开句柄；根首次创建涉及 Meta
    /// reserve/activate，是 OwnerFs 私有慢路径，不让 VFS 逐文件提交 Meta。
    fn create(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
        _mode: u32,
        _flags: i32,
    ) -> Result<types::CreatedFile> {
        Err(unsupported("create"))
    }

    /// 原始 Linux flags 由 FUSE 边缘验证后传入，后端仍必须执行权限/授权检查。
    /// O_TRUNC 必须在确认期望文件身份后生效。
    fn open(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
        _flags: i32,
    ) -> Result<types::FileHandle> {
        Err(unsupported("open"))
    }

    /// 返回写入调用者缓冲区的实际字节数；EOF 可返回 0，不补齐短读。
    fn read(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::FileHandle,
        _offset: u64,
        _out: &mut [u8],
    ) -> Result<usize> {
        Err(unsupported("read"))
    }

    /// 返回底层确认的实际字节数；不能把部分成功或未知结果报成整笔成功。
    /// O_APPEND 的末尾定位属于打开句柄语义，后端不能盲信传入 offset。
    fn write(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::FileHandle,
        _offset: u64,
        _data: &[u8],
    ) -> Result<usize> {
        Err(unsupported("write"))
    }

    /// 处理前序异步错误；普通 flush 不自动成为磁盘耐久边界。
    fn flush(&self, _ctx: &types::RequestContext, _handle: types::FileHandle) -> Result<()> {
        Err(unsupported("flush"))
    }

    /// 只有显式调用才同步；DataOnly/Full 分别对应 fdatasync/fsync。
    fn fsync(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::FileHandle,
        _mode: types::SyncMode,
    ) -> Result<()> {
        Err(unsupported("fsync"))
    }

    /// 释放本进程的句柄；不能以 path 重新寻找被 rename/unlink 的文件。
    fn release(&self, _ctx: &types::RequestContext, _handle: types::FileHandle) -> Result<()> {
        Err(unsupported("release"))
    }

    /// 返回独立目录句柄，支持 rename/unlink 后旧目录引用及 fsyncdir。
    fn opendir(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
    ) -> Result<types::DirectoryHandle> {
        Err(unsupported("opendir"))
    }

    /// `cookie` 是上一次 DirectoryEntry.next_cookie；0 表示从头开始。
    fn readdir(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::DirectoryHandle,
        _cookie: u64,
        _max_entries: usize,
    ) -> Result<Vec<types::DirectoryEntry>> {
        Err(unsupported("readdir"))
    }

    /// 目录项持久化与文件内容持久化是两个边界；调用后端的目录同步。
    fn fsyncdir(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::DirectoryHandle,
        _mode: types::SyncMode,
    ) -> Result<()> {
        Err(unsupported("fsyncdir"))
    }

    fn releasedir(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::DirectoryHandle,
    ) -> Result<()> {
        Err(unsupported("releasedir"))
    }

    /// 子目录继承其 WorkspaceRoot 的归属，不在 Meta 创建另一个 root。
    fn mkdir(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
        _mode: u32,
    ) -> Result<types::Entry> {
        Err(unsupported("mkdir"))
    }

    fn unlink(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
    ) -> Result<()> {
        Err(unsupported("unlink"))
    }

    fn rmdir(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
    ) -> Result<()> {
        Err(unsupported("rmdir"))
    }

    /// 同一后端内重命名；跨 workspace root 或跨 namespace 的策略由后端
    /// 明确拒绝，不能静默复制到另一个存储位置。
    fn rename(
        &self,
        _ctx: &types::RequestContext,
        _from_parent: types::BackendInode,
        _from_name: &OsStr,
        _to_parent: types::BackendInode,
        _to_name: &OsStr,
        _flags: types::RenameFlags,
    ) -> Result<()> {
        Err(unsupported("rename"))
    }

    fn symlink(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
        _target: &OsStr,
    ) -> Result<types::Entry> {
        Err(unsupported("symlink"))
    }

    fn readlink(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
    ) -> Result<std::ffi::OsString> {
        Err(unsupported("readlink"))
    }

    fn link(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
        _new_parent: types::BackendInode,
        _name: &OsStr,
    ) -> Result<types::Entry> {
        Err(unsupported("link"))
    }
}

fn unsupported(operation: &str) -> Error {
    Error::coded(
        afs_error::NODE_VFS_UNIMPLEMENTED,
        format!("VFS backend operation '{operation}' is not implemented"),
    )
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
            return Err(afs_error::Error::coded(
                afs_error::NODE_VFS_INVALID,
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

    /// 用已经完成 Meta 会话注册、本机根恢复和磁盘锁准备的 OwnerFs 实例
    /// 构造生产 VFS。保留 `new` 供现有框架测试；生产代码不能用它的
    /// 无依赖 OwnerFs 骨架来挂载业务目录。
    #[cfg(feature = "ownerfs")]
    pub fn with_ownerfs(
        ownerfs: Arc<dyn Backend>,
        blobfs: bool,
        registry: Registry,
    ) -> Result<Self> {
        if ownerfs.namespace() != Namespace::OwnerFs {
            return Err(afs_error::Error::coded(
                afs_error::NODE_VFS_INVALID,
                "OwnerFs backend has the wrong namespace",
            ));
        }
        let metrics = VfsMetrics::register(&registry).map_err(metrics_error)?;
        Ok(Self {
            ownerfs: Some(ownerfs),
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
            afs_error::Error::coded(
                afs_error::NODE_VFS_NOT_FOUND,
                format!("namespace '{}' is not enabled", namespace.as_str()),
            )
        })?;

        let result = backend.probe_create(&request);
        self.metrics.record_create(namespace, &result);
        result
    }

    /// 返回已启用后端的进程内接口。仅做 namespace 选择；调用方仍须传入
    /// 已认证的 RequestContext，OwnerFs 自己校验对应根的有效授权。
    #[must_use]
    pub fn backend(&self, namespace: Namespace) -> Option<&dyn Backend> {
        match namespace {
            Namespace::OwnerFs => self.ownerfs.as_deref(),
            Namespace::BlobFs => self.blobfs.as_deref(),
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
        Err(afs_error::Error::coded(
            afs_error::NODE_VFS_UNAVAILABLE,
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
        Err(afs_error::Error::coded(
            afs_error::NODE_VFS_UNAVAILABLE,
            "blobfs was not compiled into this binary",
        ))
    } else {
        Ok(None)
    }
}

fn validate_child_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        Err(afs_error::Error::coded(
            afs_error::NODE_VFS_INVALID,
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
            afs_error::ErrorKind::Unimplemented => "unsupported",
            afs_error::ErrorKind::InvalidArgument => "invalid_argument",
            afs_error::ErrorKind::NotFound => "not_found",
            afs_error::ErrorKind::Unavailable => "unavailable",
            _ => "error",
        },
    }
}
fn metrics_error(error: MetricsError) -> Error {
    afs_error::Error::coded(
        afs_error::METRICS_FAILED,
        format!("VFS metrics registration failed: {error}"),
    )
}

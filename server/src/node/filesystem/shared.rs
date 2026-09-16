//! 共享 POSIX 文件主链。
//!
//! 本层只表达 inode/open-handle/write-through 语义。内容布局仍由 DataCore
//! `VersionCandidate/Extent/Block` 表达，namespace 与对象 Current 只由 Meta 的一条
//! Filesystem journal 记录原子发布。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dms_error::DmsError;

use super::super::{
    data_core::{ByteRange, DataCoreHandle, ObjectKey, ObjectRead, ObjectWrite},
    metrics::{FilesystemOperation, NodeMetrics},
    runtime::{NodeHandle, WorkerError},
};
use super::dentry_cache::DentryLookup;
use super::meta_client::{CreateInodeRequest, FilesystemMetaClient, FilesystemMetaGrpcClient};
use super::open_handles::OpenHandle;
use crate::filesystem::{
    AttributePatch, CommitFileVersionRequest, CreateSymlinkRequest, DirectoryPage,
    FilesystemCaller, FilesystemStats, InodeId, InodeKind, LinkEntryRequest,
    NamespaceMutationResult, PreparedObjectVersion, ROOT_INODE, RemoveEntryRequest, RemoveKind,
    RemoveXattrRequest, RenameEntryRequest, ResolvedInode, SetAttributesRequest, SetXattrRequest,
    TimeUpdate, XattrSetMode,
};
use crate::node::metadata_client::digest;

const DIRECTORY_PAGE_LIMIT: u32 = 1_024;
const FILESYSTEM_CAS_RETRY_MIN_ATTEMPTS: usize = 3;
const FILESYSTEM_CAS_RETRY_BUDGET: Duration = Duration::from_secs(2);

/// FUSE 与未来其它本地文件入口共用的进程内文件服务。
///
/// 三个字段都是轻量句柄：`NodeHandle` 指向唯一 Node owner，`DataCoreHandle` 只是它的
/// 对象数据入口，`FilesystemMetaGrpcClient` 复用 Node→Meta 已存在的 Channel/Session。
#[derive(Clone)]
pub(crate) struct SharedFileOperations {
    node: NodeHandle,
    core: DataCoreHandle,
    metadata: FilesystemMetaGrpcClient,
    metrics: NodeMetrics,
}

/// Node 已经准备好、等待 Meta 原子发布的一次文件版本提交。
///
/// 把内容候选、inode CAS 条件和可选属性补丁放在同一个内部请求中，避免调用点
/// 漏传其中一部分，也直观对应 Meta 的单次 `CommitFilesystemVersion` 合同。
struct PreparedFileCommit {
    inode: InodeId,
    expected_inode_revision: u64,
    operation_id: Vec<u8>,
    prepared: PreparedObjectVersion,
    new_size: u64,
    caller: Option<FilesystemCaller>,
    attribute_patch: AttributePatch,
}

impl SharedFileOperations {
    pub(crate) fn new(node: NodeHandle) -> Result<Self, WorkerError> {
        let metadata = node.filesystem_metadata_client()?;
        let metrics = node.metrics();
        Ok(Self {
            core: DataCoreHandle::new(node.clone()),
            node,
            metadata,
            metrics,
        })
    }

    pub(crate) async fn lookup(
        &self,
        parent: InodeId,
        name: &[u8],
    ) -> Result<Option<ResolvedInode>, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Lookup);
        match self
            .node
            .filesystem_cached_dentry(parent, name.to_vec())
            .await?
        {
            DentryLookup::Hit(dentry) => {
                self.metrics.record_filesystem_dentry_cache_lookup(true);
                let resolved = self.resolve_inode(dentry.inode).await?;
                self.acquire_inode_reference(dentry.inode).await?;
                metric.success();
                return Ok(Some(resolved));
            }
            DentryLookup::Negative => {
                self.metrics.record_filesystem_dentry_cache_lookup(true);
                metric.success();
                return Ok(None);
            }
            DentryLookup::Unknown => {
                self.metrics.record_filesystem_dentry_cache_lookup(false);
            }
        }

        let reference_generation = self.node.filesystem_reserve_inode_reference_generation();
        let lookup = self
            .metadata
            .lookup(parent, name, reference_generation)
            .await
            .map_err(WorkerError::Stable)?;
        let Some(resolved) = lookup.resolved else {
            if let Some(grant) = lookup.directory_grant {
                self.node
                    .filesystem_cache_negative_dentry(parent, name.to_vec(), grant)
                    .await?;
            }
            metric.success();
            return Ok(None);
        };
        let result = resolved.resolved.clone();
        self.node
            .filesystem_install_resolved_dentry(resolved, false)
            .await?;
        metric.success();
        Ok(Some(result))
    }

    pub(crate) async fn create(
        &self,
        parent: InodeId,
        name: &[u8],
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<ResolvedInode, WorkerError> {
        self.create_kind(parent, name, InodeKind::RegularFile, mode, uid, gid)
            .await
    }

    pub(crate) async fn mkdir(
        &self,
        parent: InodeId,
        name: &[u8],
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<ResolvedInode, WorkerError> {
        self.create_kind(parent, name, InodeKind::Directory, mode, uid, gid)
            .await
    }

    pub(crate) async fn link(
        &self,
        source_inode: InodeId,
        target_parent: InodeId,
        target_name: &[u8],
    ) -> Result<crate::filesystem::InodeSnapshot, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Link);
        let operation_id = self.core.new_operation_id();
        let reference_generation = self.node.filesystem_reserve_inode_reference_generation();
        let response = self
            .metadata
            .link_entry(LinkEntryRequest {
                operation_digest: namespace_digest(
                    &operation_id,
                    &[target_name],
                    &[source_inode, target_parent],
                ),
                operation_id,
                commit_sequence: 0,
                source_inode,
                target_parent,
                target_name: target_name.to_vec(),
                expected_target_revision: None,
                reference_generation,
            })
            .await
            .map_err(WorkerError::Stable)?;
        let inode = response
            .inode
            .clone()
            .ok_or(WorkerError::MetadataUnavailable)?;
        self.install_entry_reference(
            inode.attributes.inode,
            response.entry_reference_generation,
            response.entry_reference_lease_millis,
        )
        .await?;
        self.apply_namespace_mutation(response, vec![(target_parent, target_name.to_vec())])
            .await?;
        metric.success();
        Ok(inode)
    }

    pub(crate) async fn symlink(
        &self,
        parent: InodeId,
        name: &[u8],
        target: &[u8],
        uid: u32,
        gid: u32,
    ) -> Result<ResolvedInode, WorkerError> {
        if target.is_empty() {
            return Err(WorkerError::InvalidArgument("symlink target is empty"));
        }
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Symlink);
        let operation_id = self.core.new_operation_id();
        let reference_generation = self.node.filesystem_reserve_inode_reference_generation();
        let prepared = self
            .core
            .prepare_sparse(
                ObjectKey::new(symlink_content_key(&operation_id))?,
                target.len() as u64,
                0,
                target.to_vec(),
                operation_id.clone(),
                None,
            )
            .await?;
        let created = self
            .metadata
            .create_symlink(CreateSymlinkRequest {
                operation_digest: namespace_digest(&operation_id, &[name, target], &[parent]),
                operation_id: operation_id.clone(),
                commit_sequence: 0,
                parent,
                name: name.to_vec(),
                uid,
                gid,
                expected_parent_revision: None,
                prepared: prepared.clone(),
                target_size: target.len() as u64,
                mtime_unix_nanos: unix_nanos(),
                reference_generation,
            })
            .await
            .map_err(WorkerError::Stable);
        match created {
            Ok(created) => {
                let version = created
                    .resolved
                    .granted
                    .inode
                    .content
                    .as_ref()
                    .map(|binding| binding.exact_version)
                    .ok_or(WorkerError::MetadataUnavailable)?;
                self.core
                    .finish_prepared(prepared, Some(version), false)
                    .await?;
                let result = created.resolved.clone();
                self.node
                    .filesystem_install_resolved_dentry(created, true)
                    .await?;
                metric.success();
                Ok(result)
            }
            Err(error) => {
                let rejected = filesystem_version_conflict(&error);
                let _ = self.core.finish_prepared(prepared, None, rejected).await;
                Err(error)
            }
        }
    }

    pub(crate) async fn readlink(&self, inode: InodeId) -> Result<Vec<u8>, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Readlink);
        let resolved = self.resolve_inode(inode).await?;
        let snapshot = &resolved.granted.inode;
        if snapshot.attributes.kind != InodeKind::SymbolicLink {
            return Err(WorkerError::InvalidArgument("inode is not a symbolic link"));
        }
        let Some(object) = resolved.object else {
            return Err(WorkerError::NotFound);
        };
        let read = self
            .core
            .read_resolved(object, ByteRange::new(0, snapshot.attributes.size)?)
            .await?
            .ok_or(WorkerError::NotFound)?;
        metric.success();
        Ok(read.bytes)
    }

    async fn create_kind(
        &self,
        parent: InodeId,
        name: &[u8],
        kind: InodeKind,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<ResolvedInode, WorkerError> {
        let operation = match kind {
            InodeKind::RegularFile => FilesystemOperation::Create,
            InodeKind::Directory => FilesystemOperation::Mkdir,
            InodeKind::SymbolicLink => FilesystemOperation::Create,
        };
        let mut metric = self.metrics.begin_filesystem_operation(operation);
        let reference_generation = self.node.filesystem_reserve_inode_reference_generation();
        let resolved = self
            .metadata
            .create_inode(CreateInodeRequest {
                operation_id: self.core.new_operation_id(),
                parent,
                name: name.to_vec(),
                kind,
                mode,
                uid,
                gid,
                reference_generation,
            })
            .await
            .map_err(WorkerError::Stable)?;
        let result = resolved.resolved.clone();
        self.node
            .filesystem_install_resolved_dentry(resolved, true)
            .await?;
        metric.success();
        Ok(result)
    }

    /// 读取一页 Meta 权威目录数据。
    ///
    /// `cursor` 是上一页最后一个名字；`expected_revision` 固定一次目录枚举看到的
    /// namespace 版本。Node 对每一页独立缓存，既不扫描 Meta 全表，也不在本地聚合完整目录。
    pub(crate) async fn read_directory_page(
        &self,
        directory: InodeId,
        cursor: Option<Vec<u8>>,
        expected_revision: Option<u64>,
    ) -> Result<DirectoryPage, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Readdir);
        if let Some(cached) = self
            .node
            .filesystem_cached_directory_page(directory, cursor.clone(), expected_revision)
            .await?
        {
            self.metrics.record_filesystem_dentry_cache_lookup(true);
            metric.success();
            return Ok(cached);
        }
        self.metrics.record_filesystem_dentry_cache_lookup(false);

        let page = self
            .metadata
            .read_directory(
                directory,
                cursor.clone(),
                DIRECTORY_PAGE_LIMIT,
                expected_revision,
            )
            .await
            .map_err(WorkerError::Stable)?;
        self.node
            .filesystem_cache_directory_page(cursor, page.clone())
            .await?;
        metric.success();
        Ok(page)
    }

    pub(crate) async fn rename(
        &self,
        source_parent: InodeId,
        source_name: &[u8],
        target_parent: InodeId,
        target_name: &[u8],
        replace_existing: bool,
    ) -> Result<(), WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Rename);
        let operation_id = self.core.new_operation_id();
        let response = self
            .metadata
            .rename_entry(RenameEntryRequest {
                operation_digest: namespace_digest(
                    &operation_id,
                    &[source_name, target_name],
                    &[source_parent, target_parent, u64::from(replace_existing)],
                ),
                operation_id,
                commit_sequence: 0,
                source_parent,
                source_name: source_name.to_vec(),
                target_parent,
                target_name: target_name.to_vec(),
                expected_source_revision: None,
                expected_target_revision: None,
                replace_existing,
            })
            .await
            .map_err(WorkerError::Stable)?;
        self.apply_namespace_mutation(
            response,
            vec![
                (source_parent, source_name.to_vec()),
                (target_parent, target_name.to_vec()),
            ],
        )
        .await?;
        metric.success();
        Ok(())
    }

    pub(crate) async fn unlink(&self, parent: InodeId, name: &[u8]) -> Result<(), WorkerError> {
        self.remove(parent, name, RemoveKind::File, FilesystemOperation::Unlink)
            .await
    }

    pub(crate) async fn rmdir(&self, parent: InodeId, name: &[u8]) -> Result<(), WorkerError> {
        self.remove(
            parent,
            name,
            RemoveKind::Directory,
            FilesystemOperation::Rmdir,
        )
        .await
    }

    async fn remove(
        &self,
        parent: InodeId,
        name: &[u8],
        kind: RemoveKind,
        operation: FilesystemOperation,
    ) -> Result<(), WorkerError> {
        let mut metric = self.metrics.begin_filesystem_operation(operation);
        let operation_id = self.core.new_operation_id();
        let response = self
            .metadata
            .remove_entry(RemoveEntryRequest {
                operation_digest: namespace_digest(
                    &operation_id,
                    &[name],
                    &[parent, remove_kind_digest(kind)],
                ),
                operation_id,
                commit_sequence: 0,
                parent,
                name: name.to_vec(),
                kind,
                expected_parent_revision: None,
            })
            .await
            .map_err(WorkerError::Stable)?;
        let orphaned_inode = response
            .inode
            .as_ref()
            .filter(|inode| inode.attributes.link_count == 0)
            .map(|inode| inode.attributes.inode);
        self.apply_namespace_mutation(response, vec![(parent, name.to_vec())])
            .await?;
        if let Some(inode) = orphaned_inode {
            self.node.filesystem_mark_inode_orphan(inode).await?;
        }
        metric.success();
        Ok(())
    }

    pub(crate) async fn open(&self, inode: InodeId, flags: i32) -> Result<OpenHandle, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Open);
        if flags & libc::O_TRUNC != 0 {
            // O_TRUNC 是 open(2) 的一部分：只有 truncate 原子发布成功，才创建本地
            // open handle。这样不会留下一个看似打开成功、内容却仍是旧版本的句柄。
            self.truncate(inode, 0).await?;
        } else {
            // open handle 是本 Node 生命周期；普通 open 只需确认 inode 存在并取得
            // 首次 binding grant。O_TRUNC 已在 truncate 内完成同一次解析，不能重复访问 Meta。
            self.resolve_inode(inode).await?;
        }
        let (opened, generation) = self
            .node
            .filesystem_open_handle_with_reference(inode, flags, None)
            .await?;
        if let Some(generation) = generation {
            let lease_millis = match self
                .metadata
                .acquire_inode_reference(inode, generation)
                .await
            {
                Ok(lease_millis) => lease_millis,
                Err(error) => {
                    // 首次 open 的 Meta 租约失败时，handle 不能泄漏。close 与本地引用
                    // 归还仍在同一 actor turn 内完成；Meta 从未接受本次引用，无需 release。
                    let _ = self
                        .node
                        .filesystem_close_handle_with_reference(opened.id)
                        .await;
                    return Err(WorkerError::Stable(error));
                }
            };
            if let Err(error) = self
                .node
                .filesystem_renew_inode_reference_leases(vec![(inode, generation)], lease_millis)
                .await
            {
                let _ = self
                    .node
                    .filesystem_close_handle_with_reference(opened.id)
                    .await;
                let _ = self
                    .metadata
                    .release_inode_reference(inode, generation)
                    .await;
                return Err(error);
            }
        }
        metric.success();
        Ok(opened)
    }

    /// 按 inode 取得当前快照。BindingCache 命中时不访问 Meta；Watch revoke 后会
    /// 自动回到 Meta 重新解析，因此 FUSE `getattr` 与 read 使用同一一致性规则。
    pub(crate) async fn get_inode(&self, inode: InodeId) -> Result<ResolvedInode, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Getattr);
        let resolved = self.resolve_inode(inode).await?;
        metric.success();
        Ok(resolved)
    }

    #[cfg(test)]
    pub(crate) async fn create_and_open(
        &self,
        name: &[u8],
        mode: u32,
        flags: i32,
    ) -> Result<OpenHandle, WorkerError> {
        let created = self.create(ROOT_INODE, name, mode, 0, 0).await?;
        self.open(created.granted.inode.attributes.inode, flags)
            .await
    }

    pub(crate) async fn close(&self, handle: u64) -> Result<(), WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Close);
        let (opened, released) = self
            .node
            .filesystem_close_handle_with_reference(handle)
            .await?
            .ok_or(WorkerError::NotFound)?;
        self.release_completed_reference(opened.inode, released)
            .await;
        metric.success();
        Ok(())
    }

    /// 按 open handle 读取。binding cache 命中时不会访问 Meta；缺少本地 Block 时仍走
    /// DataCore 的 Peer singleflight，安装后相同 Node 上其它 Client/FUSE 可复用。
    pub(crate) async fn read(
        &self,
        handle: u64,
        offset: u64,
        length: u64,
    ) -> Result<Option<ObjectRead>, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Read);
        let opened = self.opened(handle).await?;
        let resolved = self.resolve_inode(opened.inode).await?;
        let inode = &resolved.granted.inode;
        if offset >= inode.attributes.size || length == 0 {
            let result = Some(ObjectRead {
                version: inode
                    .content
                    .as_ref()
                    .map_or(0, |binding| binding.exact_version),
                logical_length: inode.attributes.size,
                bytes: Vec::new(),
            });
            metric.success();
            return Ok(result);
        }
        let read_length = length.min(inode.attributes.size - offset);
        let Some(object) = resolved.object else {
            // 尚未发布内容的空文件没有 DataCore object。
            let result = (inode.attributes.size == 0).then_some(ObjectRead {
                version: 0,
                logical_length: 0,
                bytes: Vec::new(),
            });
            metric.success();
            return Ok(result);
        };
        let result = self
            .core
            .read_resolved(object, ByteRange::new(offset, read_length)?)
            .await?;
        if let Some(read) = result.as_ref() {
            self.metrics
                .record_filesystem_io_bytes(FilesystemOperation::Read, read.bytes.len());
        }
        metric.success();
        Ok(result)
    }

    /// Write-through pwrite：DataCore prepare → Meta 单条原子发布 → DataCore finalize。
    /// Meta 成功前不会向 FUSE 返回成功，也不存在“对象 Current 已变、inode 仍指旧版”的窗口。
    pub(crate) async fn write(
        &self,
        handle: u64,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ObjectWrite, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Write);
        let retry_until = Instant::now() + FILESYSTEM_CAS_RETRY_BUDGET;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let opened = self.opened(handle).await?;
            let current = self.resolve_inode(opened.inode).await?;
            let inode = &current.granted.inode;
            let inode_revision = inode.revision;
            let inode_size = inode.attributes.size;
            if bytes.is_empty() {
                metric.success();
                return Ok(ObjectWrite {
                    version: inode
                        .content
                        .as_ref()
                        .map_or(0, |binding| binding.exact_version),
                    length: inode.attributes.size,
                });
            }
            let actual_offset = if opened.flags & libc::O_APPEND != 0 {
                // O_APPEND 的语义不是使用内核传来的 offset，而是以本次解析到的当前
                // EOF 为准。若并发写抢先提交，Meta CAS 会拒绝，下一轮重新解析 EOF。
                inode_size
            } else {
                offset
            };
            let write_end = actual_offset.checked_add(bytes.len() as u64).ok_or(
                WorkerError::InvalidArgument("file write range overflows u64"),
            )?;
            let operation_id = self.core.new_operation_id();
            let prepared = self
                .prepare_file_write(
                    opened.inode,
                    &current,
                    actual_offset,
                    bytes,
                    operation_id.clone(),
                )
                .await?;
            match self
                .commit_prepared_file_version(PreparedFileCommit {
                    inode: opened.inode,
                    expected_inode_revision: inode_revision,
                    operation_id,
                    prepared,
                    new_size: inode_size.max(write_end),
                    caller: None,
                    attribute_patch: AttributePatch::default(),
                })
                .await
            {
                Ok(committed) => {
                    let version = committed
                        .granted
                        .inode
                        .content
                        .as_ref()
                        .map(|binding| binding.exact_version)
                        .ok_or(WorkerError::MetadataUnavailable)?;
                    let result = ObjectWrite {
                        version,
                        length: committed.granted.inode.attributes.size,
                    };
                    self.metrics
                        .record_filesystem_io_bytes(FilesystemOperation::Write, bytes.len());
                    metric.success();
                    return Ok(result);
                }
                Err(error) if filesystem_version_conflict(&error) => {
                    self.invalidate_resolved_binding(&current).await?;
                    if attempts >= FILESYSTEM_CAS_RETRY_MIN_ATTEMPTS
                        && Instant::now() >= retry_until
                    {
                        return Err(error);
                    }
                    // 只有确定性 CAS 冲突才会走到这里；下一轮重新解析 authoritative
                    // inode/EOF，并为新的候选版本生成新的 operation。未知提交结果由
                    // MetadataClient 复用同一 request/operation 重试，不能在这里重选 EOF。
                    tokio::task::yield_now().await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// 调整普通文件大小。缩短走布局裁剪，扩展追加 sparse hole；两者最终都通过同一条
    /// Meta filesystem commit 原子发布 size、mtime 和 exact content binding，且不会为
    /// 空洞物化零 Block。
    pub(crate) async fn truncate(
        &self,
        inode: InodeId,
        size: u64,
    ) -> Result<ResolvedInode, WorkerError> {
        self.truncate_with_attributes(inode, size, None, AttributePatch::default())
            .await
    }

    /// 一次 FUSE setattr 同时修改 size 与其它属性时走这条路径。内容候选与最终
    /// inode 属性共用一次 Meta CAS/journal，不暴露“size 已更新但 attrs 仍旧”的中间态。
    pub(crate) async fn truncate_with_attributes(
        &self,
        inode: InodeId,
        size: u64,
        caller: Option<FilesystemCaller>,
        patch: AttributePatch,
    ) -> Result<ResolvedInode, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Truncate);
        let retry_until = Instant::now() + FILESYSTEM_CAS_RETRY_BUDGET;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let current = self.resolve_inode(inode).await?;
            let snapshot = &current.granted.inode;
            let inode_revision = snapshot.revision;
            let inode_size = snapshot.attributes.size;
            if snapshot.attributes.kind != InodeKind::RegularFile {
                return Err(WorkerError::InvalidArgument(
                    "only regular files can be truncated",
                ));
            }
            if inode_size == size && patch.is_empty() {
                metric.success();
                return Ok(current);
            }

            let operation_id = self.core.new_operation_id();
            let prepared = self
                .prepare_file_truncate(inode, &current, size, operation_id.clone())
                .await?;
            match self
                .commit_prepared_file_version(PreparedFileCommit {
                    inode,
                    expected_inode_revision: inode_revision,
                    operation_id,
                    prepared,
                    new_size: size,
                    caller,
                    attribute_patch: patch,
                })
                .await
            {
                Ok(committed) => {
                    metric.success();
                    return Ok(committed);
                }
                Err(error) if filesystem_version_conflict(&error) => {
                    self.invalidate_resolved_binding(&current).await?;
                    if attempts >= FILESYSTEM_CAS_RETRY_MIN_ATTEMPTS
                        && Instant::now() >= retry_until
                    {
                        return Err(error);
                    }
                    tokio::task::yield_now().await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// 修改不涉及文件内容的 inode 属性。
    ///
    /// Node 只负责从可信本地入口传递调用者身份、执行 revision CAS 并维护本地
    /// binding cache；最终权限判断、时间解析、ctime 生成和 journal 提交都在 Meta
    /// 的唯一 owner turn 中完成。确定性 revision 冲突会重新读取后重试，未知提交
    /// 结果则由 `MetadataClient` 使用同一个 operation id 重试。
    pub(crate) async fn set_attributes(
        &self,
        inode: InodeId,
        caller: FilesystemCaller,
        patch: AttributePatch,
    ) -> Result<ResolvedInode, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Setattr);
        let retry_until = Instant::now() + FILESYSTEM_CAS_RETRY_BUDGET;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let current = self.resolve_inode(inode).await?;
            if patch.is_empty() {
                metric.success();
                return Ok(current);
            }

            let operation_id = self.core.new_operation_id();
            let operation_digest = attribute_digest(
                &operation_id,
                inode,
                current.granted.inode.revision,
                caller,
                patch,
            );
            match self
                .metadata
                .set_attributes(SetAttributesRequest {
                    operation_id,
                    operation_digest,
                    commit_sequence: 0,
                    inode,
                    expected_inode_revision: current.granted.inode.revision,
                    caller,
                    patch,
                })
                .await
            {
                Ok(result) => {
                    self.node
                        .filesystem_cache_binding(result.resolved.clone())
                        .await?;
                    metric.success();
                    return Ok(result.resolved);
                }
                Err(error) => {
                    let error = WorkerError::Stable(error);
                    if filesystem_version_conflict(&error) {
                        self.invalidate_resolved_binding(&current).await?;
                        if attempts >= FILESYSTEM_CAS_RETRY_MIN_ATTEMPTS
                            && Instant::now() >= retry_until
                        {
                            return Err(error);
                        }
                        tokio::task::yield_now().await;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
    }

    /// 按需读取一个扩展属性。首版不在 Node 建立长期 xattr cache：属性通常很小且
    /// 修改频率低，直接由 Meta 做权限校验与 revision 读取，避免再引入一套失效协议。
    pub(crate) async fn get_xattr(
        &self,
        inode: InodeId,
        caller: FilesystemCaller,
        name: &[u8],
    ) -> Result<Option<Vec<u8>>, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Getxattr);
        let result = self
            .metadata
            .get_xattr(inode, name, caller)
            .await
            .map_err(WorkerError::Stable)?;
        metric.success();
        Ok(result)
    }

    /// 返回当前 inode 的 xattr 名字集合；wire 与 FUSE 的 NUL 编码留在各自 adapter。
    pub(crate) async fn list_xattrs(
        &self,
        inode: InodeId,
        caller: FilesystemCaller,
    ) -> Result<Vec<Vec<u8>>, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Listxattr);
        let result = self
            .metadata
            .list_xattrs(inode, caller)
            .await
            .map_err(WorkerError::Stable)?;
        metric.success();
        Ok(result)
    }

    /// 写入一个 xattr。revision 冲突与 setattr 使用同一有界 CAS 重试规则；不确定
    /// 提交由 MetadataClient 使用同一 operation id 查询/重试，不能生成第二次写。
    pub(crate) async fn set_xattr(
        &self,
        inode: InodeId,
        caller: FilesystemCaller,
        name: Vec<u8>,
        value: Vec<u8>,
        mode: XattrSetMode,
    ) -> Result<ResolvedInode, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Setxattr);
        let retry_until = Instant::now() + FILESYSTEM_CAS_RETRY_BUDGET;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let current = self.resolve_inode(inode).await?;
            let operation_id = self.core.new_operation_id();
            let operation_digest = xattr_digest(
                &operation_id,
                inode,
                current.granted.inode.revision,
                caller,
                &name,
                Some(&value),
                Some(mode),
            );
            match self
                .metadata
                .set_xattr(SetXattrRequest {
                    operation_id,
                    operation_digest,
                    inode,
                    expected_inode_revision: current.granted.inode.revision,
                    caller,
                    name: name.clone(),
                    value: value.clone(),
                    mode,
                })
                .await
            {
                Ok(result) => {
                    self.node
                        .filesystem_cache_binding(result.resolved.clone())
                        .await?;
                    metric.success();
                    return Ok(result.resolved);
                }
                Err(error) => {
                    let error = WorkerError::Stable(error);
                    if filesystem_version_conflict(&error) {
                        self.invalidate_resolved_binding(&current).await?;
                        if attempts >= FILESYSTEM_CAS_RETRY_MIN_ATTEMPTS
                            && Instant::now() >= retry_until
                        {
                            return Err(error);
                        }
                        tokio::task::yield_now().await;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
    }

    pub(crate) async fn remove_xattr(
        &self,
        inode: InodeId,
        caller: FilesystemCaller,
        name: Vec<u8>,
    ) -> Result<ResolvedInode, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Removexattr);
        let retry_until = Instant::now() + FILESYSTEM_CAS_RETRY_BUDGET;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let current = self.resolve_inode(inode).await?;
            let operation_id = self.core.new_operation_id();
            let operation_digest = xattr_digest(
                &operation_id,
                inode,
                current.granted.inode.revision,
                caller,
                &name,
                None,
                None,
            );
            match self
                .metadata
                .remove_xattr(RemoveXattrRequest {
                    operation_id,
                    operation_digest,
                    inode,
                    expected_inode_revision: current.granted.inode.revision,
                    caller,
                    name: name.clone(),
                })
                .await
            {
                Ok(result) => {
                    self.node
                        .filesystem_cache_binding(result.resolved.clone())
                        .await?;
                    metric.success();
                    return Ok(result.resolved);
                }
                Err(error) => {
                    let error = WorkerError::Stable(error);
                    if filesystem_version_conflict(&error) {
                        self.invalidate_resolved_binding(&current).await?;
                        if attempts >= FILESYSTEM_CAS_RETRY_MIN_ATTEMPTS
                            && Instant::now() >= retry_until
                        {
                            return Err(error);
                        }
                        tokio::task::yield_now().await;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
    }

    /// 返回 Meta 对当前仍有租约且已上报资源的 Node 聚合出的文件系统容量。
    pub(crate) async fn stat_filesystem(&self) -> Result<FilesystemStats, WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Statfs);
        let result = self
            .metadata
            .stat_filesystem()
            .await
            .map_err(WorkerError::Stable)?;
        metric.success();
        Ok(result)
    }

    async fn prepare_file_write(
        &self,
        inode: InodeId,
        current: &ResolvedInode,
        offset: u64,
        bytes: &[u8],
        operation_id: Vec<u8>,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        self.prepare_content_write(
            inode,
            current,
            offset,
            bytes,
            operation_id,
            InodeKind::RegularFile,
        )
        .await
    }

    async fn prepare_content_write(
        &self,
        inode: InodeId,
        current: &ResolvedInode,
        offset: u64,
        bytes: &[u8],
        operation_id: Vec<u8>,
        expected_kind: InodeKind,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let snapshot = &current.granted.inode;
        if snapshot.attributes.kind != expected_kind {
            return Err(WorkerError::InvalidArgument(
                "inode kind does not accept this write path",
            ));
        }
        let write_end =
            offset
                .checked_add(bytes.len() as u64)
                .ok_or(WorkerError::InvalidArgument(
                    "file write range overflows u64",
                ))?;
        let object_key = ObjectKey::new(content_key(inode))?;

        if snapshot.content.is_some() {
            let object = current
                .object
                .clone()
                .ok_or(WorkerError::MetadataUnavailable)?;
            // FUSE 可能把一次大 write(2) 拆成多个 callback。这里保留旧 Extent，
            // 只把本次 bytes 作为新 Block；若 offset 越过 EOF，中间空洞由
            // DataCore sparse hole 表达，不能合成一大段全零 tail Block。
            self.core
                .prepare_range(object_key, offset, bytes.to_vec(), operation_id, object)
                .await
        } else {
            // 首次写也可以从非零 offset 开始。DataCore 只提交用户数据 Block；
            // `[0, offset)` 是 sparse hole，读时补零，不消耗 Arena/网络。
            self.core
                .prepare_sparse(
                    object_key,
                    write_end,
                    offset,
                    bytes.to_vec(),
                    operation_id,
                    None,
                )
                .await
        }
    }

    pub(crate) async fn acquire_inode_reference(&self, inode: InodeId) -> Result<(), WorkerError> {
        let Some(generation) = self
            .node
            .filesystem_acquire_inode_reference_local(inode)
            .await?
        else {
            return Ok(());
        };
        let lease_millis = match self
            .metadata
            .acquire_inode_reference(inode, generation)
            .await
        {
            Ok(lease_millis) => lease_millis,
            Err(error) => {
                let _ = self
                    .node
                    .filesystem_release_inode_reference_local(inode, 1)
                    .await;
                return Err(WorkerError::Stable(error));
            }
        };
        self.node
            .filesystem_renew_inode_reference_leases(vec![(inode, generation)], lease_millis)
            .await?;
        Ok(())
    }

    async fn install_entry_reference(
        &self,
        inode: InodeId,
        generation: u64,
        lease_millis: u64,
    ) -> Result<(), WorkerError> {
        if inode == ROOT_INODE {
            return Ok(());
        }
        if generation == 0 || lease_millis == 0 {
            return Err(WorkerError::MetadataUnavailable);
        }
        self.node
            .filesystem_install_inode_reference(inode, generation, lease_millis)
            .await
    }

    pub(crate) async fn release_inode_reference(&self, inode: InodeId, count: u64) {
        let released = match self
            .node
            .filesystem_release_inode_reference_local(inode, count)
            .await
        {
            Ok(generation) => generation,
            Err(error) => {
                dms_logging::warn!(
                    "failed to release local filesystem inode reference";
                    "event" => "node.filesystem.reference.local_release_failed",
                    "inode" => inode,
                    "error" => format!("{error:?}"),
                );
                None
            }
        };
        self.release_completed_reference(inode, released).await;
    }

    async fn release_completed_reference(&self, inode: InodeId, released: Option<(u64, bool)>) {
        let Some((generation, eager_meta_release)) = released else {
            return;
        };
        // 普通有名文件关闭后不再逐次向 Meta 发送 release RPC：Node 停止在 heartbeat
        // 中续租，Meta 会在 lease 到期后清理记录。这样创建/关闭小文件不会把 release
        // 请求排在后续 create 前面。只有已失去最后一个目录项的 orphan 才同步 release，
        // 让 durable reap 无需等待完整租约窗口；请求失败时租约到期仍是安全兜底。
        if eager_meta_release
            && let Err(error) = self
                .metadata
                .release_inode_reference(inode, generation)
                .await
        {
            dms_logging::warn!(
                "failed to release Meta filesystem orphan reference";
                "event" => "node.filesystem.reference.meta_release_failed",
                "inode" => inode,
                "generation" => generation,
                "error" => error.to_string(),
            );
        }
    }

    async fn prepare_file_truncate(
        &self,
        inode: InodeId,
        current: &ResolvedInode,
        size: u64,
        operation_id: Vec<u8>,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let object_key = ObjectKey::new(content_key(inode))?;
        match &current.object {
            Some(object) => {
                self.core
                    .prepare_truncate(object_key, size, operation_id, object.clone())
                    .await
            }
            None => {
                self.core
                    .prepare_sparse(object_key, size, 0, Vec::new(), operation_id, None)
                    .await
            }
        }
    }

    async fn commit_prepared_file_version(
        &self,
        request: PreparedFileCommit,
    ) -> Result<ResolvedInode, WorkerError> {
        let PreparedFileCommit {
            inode,
            expected_inode_revision,
            operation_id,
            prepared,
            new_size,
            caller,
            attribute_patch,
        } = request;
        let commit = self
            .metadata
            .commit_file_version(CommitFileVersionRequest {
                operation_id,
                inode,
                expected_inode_revision,
                prepared: prepared.clone(),
                new_size,
                mtime_unix_nanos: unix_nanos(),
                caller,
                attribute_patch,
            })
            .await;
        match commit {
            Ok(committed) => {
                let version = committed
                    .resolved
                    .granted
                    .inode
                    .content
                    .as_ref()
                    .map(|binding| binding.exact_version)
                    .ok_or(WorkerError::MetadataUnavailable)?;
                self.core
                    .finish_prepared(prepared, Some(version), false)
                    .await?;
                self.node
                    .filesystem_cache_binding(committed.resolved.clone())
                    .await?;
                Ok(committed.resolved)
            }
            Err(error) => {
                let rejected = definitive_rejection(&error);
                self.core.finish_prepared(prepared, None, rejected).await?;
                Err(WorkerError::Stable(error))
            }
        }
    }

    async fn invalidate_resolved_binding(
        &self,
        resolved: &ResolvedInode,
    ) -> Result<(), WorkerError> {
        self.node
            .invalidate_filesystem_binding(
                resolved.granted.inode.attributes.inode,
                resolved.granted.grant.generation,
                resolved.granted.inode.revision,
            )
            .await
    }

    async fn opened(&self, handle: u64) -> Result<OpenHandle, WorkerError> {
        self.node
            .filesystem_get_handle(handle)
            .await?
            .ok_or(WorkerError::NotFound)
    }

    async fn resolve_inode(&self, inode: InodeId) -> Result<ResolvedInode, WorkerError> {
        let resolved = if let Some(cached) = self.node.filesystem_cached_binding(inode).await? {
            self.metrics.record_filesystem_binding_cache_lookup(true);
            cached
        } else {
            self.metrics.record_filesystem_binding_cache_lookup(false);
            let resolved = self
                .metadata
                .resolve_inode(inode)
                .await
                .map_err(WorkerError::Stable)?
                .ok_or(WorkerError::NotFound)?;
            self.node.filesystem_cache_binding(resolved.clone()).await?;
            resolved
        };
        // unlink 后的 open handle 只在 Node 持有的引用租约内继续可用。Watch/Meta
        // 断开超过 TTL 后，必须在本地 fence 掉旧 orphan，不能靠缓存无限续命。
        if resolved.granted.inode.attributes.link_count == 0
            && !self.node.filesystem_has_live_inode_reference(inode).await?
        {
            return Err(WorkerError::NotFound);
        }
        Ok(resolved)
    }

    async fn apply_namespace_mutation(
        &self,
        result: NamespaceMutationResult,
        removed_dentries: Vec<(InodeId, Vec<u8>)>,
    ) -> Result<(), WorkerError> {
        self.node
            .filesystem_apply_local_namespace_mutation(
                result.changed_directories,
                result.changed_inodes,
                removed_dentries,
            )
            .await
    }
}

fn content_key(inode: InodeId) -> Vec<u8> {
    format!("fs/content/{inode}").into_bytes()
}

fn symlink_content_key(operation_id: &[u8]) -> Vec<u8> {
    // inode 由 Meta 在同一条 symlink journal 记录里分配；Node 准备 target bytes 时
    // 还不知道 inode，因此使用 operation-scoped key，最终 inode binding 精确指向该
    // DataCore 版本，仍保持单一 commit authority。
    let mut key = b"fs/symlink/".to_vec();
    key.extend_from_slice(&hex_encode(operation_id));
    key
}

fn hex_encode(bytes: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = Vec::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize]);
        encoded.push(HEX[(byte & 0x0f) as usize]);
    }
    encoded
}

fn unix_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128) as i64
}

fn definitive_rejection(error: &DmsError) -> bool {
    matches!(
        error.code(),
        dms_error::META_CATALOG_INVALID_REQUEST | dms_error::META_CATALOG_VERSION_CONFLICT
    )
}

fn filesystem_version_conflict(error: &WorkerError) -> bool {
    matches!(error, WorkerError::Conflict)
        || matches!(
            error,
            WorkerError::Stable(stable)
                if stable.code() == dms_error::META_CATALOG_VERSION_CONFLICT
        )
}

fn attribute_digest(
    operation_id: &[u8],
    inode: InodeId,
    expected_revision: u64,
    caller: FilesystemCaller,
    patch: AttributePatch,
) -> Vec<u8> {
    let mut value = digest(operation_id);
    value.extend_from_slice(&inode.to_be_bytes());
    value.extend_from_slice(&expected_revision.to_be_bytes());
    value.extend_from_slice(&caller.uid.to_be_bytes());
    value.extend_from_slice(&caller.gid.to_be_bytes());
    value.extend_from_slice(&caller.pid.to_be_bytes());
    encode_optional_u32(&mut value, patch.mode);
    encode_optional_u32(&mut value, patch.uid);
    encode_optional_u32(&mut value, patch.gid);
    encode_time_update(&mut value, patch.atime);
    encode_time_update(&mut value, patch.mtime);
    value
}

fn xattr_digest(
    operation_id: &[u8],
    inode: InodeId,
    expected_revision: u64,
    caller: FilesystemCaller,
    name: &[u8],
    value: Option<&[u8]>,
    mode: Option<XattrSetMode>,
) -> Vec<u8> {
    let mut output = digest(operation_id);
    output.extend_from_slice(&inode.to_be_bytes());
    output.extend_from_slice(&expected_revision.to_be_bytes());
    output.extend_from_slice(&caller.uid.to_be_bytes());
    output.extend_from_slice(&caller.gid.to_be_bytes());
    output.extend_from_slice(&caller.pid.to_be_bytes());
    output.extend_from_slice(&(name.len() as u64).to_be_bytes());
    output.extend_from_slice(name);
    match value {
        Some(value) => {
            output.push(1);
            output.extend_from_slice(&(value.len() as u64).to_be_bytes());
            output.extend_from_slice(value);
        }
        None => output.push(0),
    }
    output.push(match mode {
        None => 0,
        Some(XattrSetMode::Upsert) => 1,
        Some(XattrSetMode::CreateOnly) => 2,
        Some(XattrSetMode::ReplaceOnly) => 3,
    });
    output
}

fn encode_optional_u32(output: &mut Vec<u8>, value: Option<u32>) {
    match value {
        Some(value) => {
            output.push(1);
            output.extend_from_slice(&value.to_be_bytes());
        }
        None => output.push(0),
    }
}

fn encode_time_update(output: &mut Vec<u8>, update: TimeUpdate) {
    match update {
        TimeUpdate::Omit => output.push(0),
        TimeUpdate::Now => output.push(1),
        TimeUpdate::Exact(value) => {
            output.push(2);
            output.extend_from_slice(&value.to_be_bytes());
        }
    }
}

fn namespace_digest(operation_id: &[u8], names: &[&[u8]], numbers: &[u64]) -> Vec<u8> {
    let mut value = digest(operation_id);
    for number in numbers {
        value.extend_from_slice(&number.to_be_bytes());
    }
    for name in names {
        value.extend_from_slice(&(name.len() as u64).to_be_bytes());
        value.extend_from_slice(name);
    }
    value
}

fn remove_kind_digest(kind: RemoveKind) -> u64 {
    match kind {
        RemoveKind::File => 1,
        RemoveKind::Directory => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::content_key;

    #[test]
    fn inode_content_key_is_stable_and_namespace_private() {
        assert_eq!(content_key(42), b"fs/content/42");
    }
}

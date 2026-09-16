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
#[cfg(test)]
use crate::filesystem::ROOT_INODE;
use crate::filesystem::{
    CommitFileVersionRequest, DirectoryPage, InodeId, InodeKind, NamespaceMutationResult,
    PreparedObjectVersion, RemoveEntryRequest, RemoveKind, RenameEntryRequest, ResolvedInode,
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

        let lookup = self
            .metadata
            .lookup(parent, name)
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
        self.node
            .filesystem_cache_positive_dentry(resolved.dentry.clone(), resolved.directory_grant)
            .await?;
        self.node
            .filesystem_cache_binding(resolved.resolved.clone())
            .await?;
        metric.success();
        Ok(Some(resolved.resolved))
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
            })
            .await
            .map_err(WorkerError::Stable)?;
        self.node
            .invalidate_filesystem_binding(
                parent,
                resolved.directory_grant.grant.generation,
                resolved.directory_grant.directory_revision,
            )
            .await?;
        self.node
            .filesystem_cache_positive_dentry(resolved.dentry.clone(), resolved.directory_grant)
            .await?;
        self.node
            .filesystem_cache_binding(resolved.resolved.clone())
            .await?;
        metric.success();
        Ok(resolved.resolved)
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
        self.apply_namespace_mutation(response).await?;
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
        self.apply_namespace_mutation(response).await?;
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
        let opened = self.node.filesystem_open_handle(inode, flags, None).await?;
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
        self.node
            .filesystem_open_handle(created.granted.inode.attributes.inode, flags, None)
            .await
    }

    pub(crate) async fn close(&self, handle: u64) -> Result<(), WorkerError> {
        let mut metric = self
            .metrics
            .begin_filesystem_operation(FilesystemOperation::Close);
        self.node
            .filesystem_close_handle(handle)
            .await?
            .map(|_| ())
            .ok_or(WorkerError::NotFound)?;
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
                .commit_prepared_file_version(
                    opened.inode,
                    inode_revision,
                    operation_id,
                    prepared,
                    inode_size.max(write_end),
                )
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
            if inode_size == size {
                metric.success();
                return Ok(current);
            }

            let operation_id = self.core.new_operation_id();
            let prepared = self
                .prepare_file_truncate(inode, &current, size, operation_id.clone())
                .await?;
            match self
                .commit_prepared_file_version(inode, inode_revision, operation_id, prepared, size)
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

    async fn prepare_file_write(
        &self,
        inode: InodeId,
        current: &ResolvedInode,
        offset: u64,
        bytes: &[u8],
        operation_id: Vec<u8>,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let snapshot = &current.granted.inode;
        if snapshot.attributes.kind != InodeKind::RegularFile {
            return Err(WorkerError::InvalidArgument(
                "only regular files can be written",
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
        inode: InodeId,
        expected_inode_revision: u64,
        operation_id: Vec<u8>,
        prepared: PreparedObjectVersion,
        new_size: u64,
    ) -> Result<ResolvedInode, WorkerError> {
        let commit = self
            .metadata
            .commit_file_version(CommitFileVersionRequest {
                operation_id,
                inode,
                expected_inode_revision,
                prepared: prepared.clone(),
                new_size,
                mtime_unix_nanos: unix_nanos(),
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
        if let Some(cached) = self.node.filesystem_cached_binding(inode).await? {
            self.metrics.record_filesystem_binding_cache_lookup(true);
            return Ok(cached);
        }
        self.metrics.record_filesystem_binding_cache_lookup(false);
        let resolved = self
            .metadata
            .resolve_inode(inode)
            .await
            .map_err(WorkerError::Stable)?
            .ok_or(WorkerError::NotFound)?;
        self.node.filesystem_cache_binding(resolved.clone()).await?;
        Ok(resolved)
    }

    async fn apply_namespace_mutation(
        &self,
        result: NamespaceMutationResult,
    ) -> Result<(), WorkerError> {
        for directory in result.changed_directories {
            self.node
                .invalidate_filesystem_binding(
                    directory.inode,
                    directory.grant_generation,
                    directory.revision,
                )
                .await?;
        }
        Ok(())
    }
}

fn content_key(inode: InodeId) -> Vec<u8> {
    format!("fs/content/{inode}").into_bytes()
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

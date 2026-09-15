//! 共享 POSIX 文件主链。
//!
//! 本层只表达 inode/open-handle/write-through 语义。内容布局仍由 DataCore
//! `VersionCandidate/Extent/Block` 表达，namespace 与对象 Current 只由 Meta 的一条
//! Filesystem journal 记录原子发布。

use std::time::{SystemTime, UNIX_EPOCH};

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
    RemoveEntryRequest, RemoveKind, RenameEntryRequest, ResolvedInode,
};
use crate::node::metadata_client::digest;

const DIRECTORY_PAGE_LIMIT: u32 = 1_024;

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
        // open handle 是本 Node 生命周期；先确认 inode 存在并取得首次 binding grant。
        self.resolve_inode(inode).await?;
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
        let opened = self.opened(handle).await?;
        let current = self.resolve_inode(opened.inode).await?;
        let inode = &current.granted.inode;
        let write_end =
            offset
                .checked_add(bytes.len() as u64)
                .ok_or(WorkerError::InvalidArgument(
                    "file write range overflows u64",
                ))?;
        let operation_id = self.core.new_operation_id();
        let object_key = ObjectKey::new(content_key(opened.inode))?;

        let prepared = if inode.content.is_some() {
            let object = current
                .object
                .clone()
                .ok_or(WorkerError::MetadataUnavailable)?;
            // FUSE 可能把一次大 write(2) 拆成多个 callback。旧实现每次扩容都读回
            // 已写前缀并完整 prepare，累计拷贝量随 callback 数近似 O(n²)。现在保留
            // 旧 Extent，只把本次 bytes 作为一个新 Block；稀疏区只物化必要的零尾部。
            let (patch_offset, patch_bytes) = if offset <= inode.attributes.size {
                (offset, bytes.to_vec())
            } else {
                let tail_len = usize::try_from(write_end - inode.attributes.size)
                    .map_err(|_| WorkerError::ResourceExhausted)?;
                let data_offset = usize::try_from(offset - inode.attributes.size)
                    .map_err(|_| WorkerError::ResourceExhausted)?;
                let mut tail = vec![0; tail_len];
                tail[data_offset..data_offset + bytes.len()].copy_from_slice(bytes);
                (inode.attributes.size, tail)
            };
            self.core
                .prepare_range(
                    object_key,
                    patch_offset,
                    patch_bytes,
                    operation_id.clone(),
                    object,
                )
                .await?
        } else {
            if offset != 0 {
                return Err(WorkerError::InvalidArgument(
                    "first write cannot create a sparse file",
                ));
            }
            self.core
                .prepare_put(object_key, bytes.to_vec(), operation_id.clone(), None)
                .await?
        };

        let commit = self
            .metadata
            .commit_file_version(CommitFileVersionRequest {
                operation_id,
                inode: opened.inode,
                expected_inode_revision: inode.revision,
                prepared: prepared.clone(),
                new_size: inode.attributes.size.max(write_end),
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
                let result = ObjectWrite {
                    version,
                    length: committed.resolved.granted.inode.attributes.size,
                };
                self.metrics
                    .record_filesystem_io_bytes(FilesystemOperation::Write, bytes.len());
                metric.success();
                Ok(result)
            }
            Err(error) => {
                let rejected = definitive_rejection(&error);
                self.core.finish_prepared(prepared, None, rejected).await?;
                Err(WorkerError::Stable(error))
            }
        }
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

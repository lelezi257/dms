//! Node Filesystem 访问 Meta 文件服务的原生 Rust 业务合同。
//!
//! 名字中的 `Client` 表示它是 Node 文件模块的出站依赖，不是公开 SDK。FUSE 只调用
//! Node 文件模块，不接触 protobuf；`FilesystemMetaGrpcClient` 负责把这些原生值类型
//! 转换成独立 filesystem meta protobuf DTO。
//!
//! M1 开始声明目录分页与 namespace mutation，因为这些操作已经进入独立的
//! FilesystemMeta protobuf service，并由 Meta owner 的 journal/apply 路径承载。

use dms_error::DmsResult;
use dms_error::{DmsError, ErrorKind};
use dms_protocol::v1 as pb;

use crate::filesystem::{
    AttributeMutationResult, AttributePatch, CommitFileVersionRequest, CommitFileVersionResult,
    CreateSymlinkRequest, DentrySnapshot, DirectoryGrant, DirectoryPage, FilesystemCaller,
    FilesystemStats, InodeId, InodeKind, LinkEntryRequest, NamespaceMutationResult,
    RemoveEntryRequest, RemoveXattrRequest, RenameEntryRequest, ResolvedInode,
    SetAttributesRequest, SetXattrRequest, TimeUpdate, attribute_patch_to_proto,
    attribute_result_from_proto, caller_to_proto, dentry_from_proto, directory_grant_from_proto,
    directory_page_from_proto, filesystem_stats_from_proto, namespace_result_from_proto,
    resolved_from_proto,
};
use crate::node::metadata_client::{MetadataClient, digest};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CreateInodeRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) parent: InodeId,
    pub(crate) name: Vec<u8>,
    pub(crate) kind: InodeKind,
    pub(crate) mode: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) reference_generation: u64,
}

/// 一次名字解析的两个不可拆结果：dentry 给出 path→inode，resolved 给出同一时刻的
/// inode→精确对象版本。Node 分别写入 DentryCache 与 BindingCache。
#[derive(Clone)]
pub(crate) struct ResolvedDentry {
    pub(crate) dentry: DentrySnapshot,
    pub(crate) resolved: ResolvedInode,
    pub(crate) directory_grant: DirectoryGrant,
    pub(crate) entry_reference_lease_millis: u64,
    pub(crate) entry_reference_generation: u64,
}

pub(crate) struct LookupDentry {
    pub(crate) resolved: Option<ResolvedDentry>,
    pub(crate) directory_grant: Option<DirectoryGrant>,
}

/// Node 文件模块唯一的 Node→Meta 业务接口。
///
/// `open` 不是远端接口：Node 在本地创建 `OpenHandle`。只有 binding cache 未命中时，
/// `resolve_inode` 才访问 Meta 并取得 `CacheGrant`。
pub(crate) trait FilesystemMetaClient: Send + Sync {
    async fn lookup(
        &self,
        parent: InodeId,
        name: &[u8],
        reference_generation: u64,
    ) -> DmsResult<LookupDentry>;

    async fn get_inode(&self, inode: InodeId) -> DmsResult<Option<ResolvedInode>>;

    async fn create_inode(&self, request: CreateInodeRequest) -> DmsResult<ResolvedDentry>;

    async fn create_symlink(&self, request: CreateSymlinkRequest) -> DmsResult<ResolvedDentry>;

    async fn read_directory(
        &self,
        directory: InodeId,
        cursor: Option<Vec<u8>>,
        limit: u32,
        expected_directory_revision: Option<u64>,
    ) -> DmsResult<DirectoryPage>;

    async fn link_entry(&self, request: LinkEntryRequest) -> DmsResult<NamespaceMutationResult>;

    async fn acquire_inode_reference(&self, inode: InodeId, generation: u64) -> DmsResult<u64>;

    async fn release_inode_reference(&self, inode: InodeId, generation: u64) -> DmsResult<()>;

    async fn rename_entry(&self, request: RenameEntryRequest)
    -> DmsResult<NamespaceMutationResult>;

    async fn remove_entry(&self, request: RemoveEntryRequest)
    -> DmsResult<NamespaceMutationResult>;

    async fn resolve_inode(&self, inode: InodeId) -> DmsResult<Option<ResolvedInode>>;

    async fn commit_file_version(
        &self,
        request: CommitFileVersionRequest,
    ) -> DmsResult<CommitFileVersionResult>;

    async fn set_attributes(
        &self,
        request: SetAttributesRequest,
    ) -> DmsResult<crate::filesystem::AttributeMutationResult>;

    async fn get_xattr(
        &self,
        inode: InodeId,
        name: &[u8],
        caller: FilesystemCaller,
    ) -> DmsResult<Option<Vec<u8>>>;

    async fn list_xattrs(
        &self,
        inode: InodeId,
        caller: FilesystemCaller,
    ) -> DmsResult<Vec<Vec<u8>>>;

    async fn set_xattr(&self, request: SetXattrRequest) -> DmsResult<AttributeMutationResult>;

    async fn remove_xattr(&self, request: RemoveXattrRequest)
    -> DmsResult<AttributeMutationResult>;

    async fn stat_filesystem(&self) -> DmsResult<FilesystemStats>;
}

/// 复用 Node 已建立的 Meta HTTP/2 Channel、Session 和 commit sequence。
///
/// 它只负责领域类型与 protobuf DTO 的转换，不创建第二条连接，也不拥有任何
/// namespace 状态。FUSE 与文件业务层因此不依赖生成代码。
#[derive(Clone)]
pub(crate) struct FilesystemMetaGrpcClient {
    metadata: MetadataClient,
}

impl FilesystemMetaGrpcClient {
    pub(crate) fn new(metadata: MetadataClient) -> Self {
        Self { metadata }
    }
}

impl FilesystemMetaClient for FilesystemMetaGrpcClient {
    async fn lookup(
        &self,
        parent: InodeId,
        name: &[u8],
        reference_generation: u64,
    ) -> DmsResult<LookupDentry> {
        let response = self
            .metadata
            .filesystem_lookup(parent, name.to_vec(), reference_generation)
            .await?;
        let directory_grant = response
            .directory_grant
            .map(directory_grant_from_proto)
            .transpose()
            .map_err(invalid_filesystem_response)?;
        let resolved = response
            .found
            .then(|| {
                let dentry = response.dentry.ok_or_else(missing_filesystem_response)?;
                let resolved = response.resolved.ok_or_else(missing_filesystem_response)?;
                let directory_grant = directory_grant.ok_or_else(missing_filesystem_response)?;
                Ok(ResolvedDentry {
                    dentry: dentry_from_proto(dentry),
                    resolved: resolved_from_proto(resolved).map_err(invalid_filesystem_response)?,
                    directory_grant,
                    entry_reference_lease_millis: response.entry_reference_lease_millis,
                    entry_reference_generation: response.entry_reference_generation,
                })
            })
            .transpose()?;
        Ok(LookupDentry {
            resolved,
            directory_grant,
        })
    }

    async fn get_inode(&self, inode: InodeId) -> DmsResult<Option<ResolvedInode>> {
        let response = self.metadata.filesystem_get_inode(inode).await?;
        response
            .found
            .then(|| {
                response
                    .resolved
                    .ok_or_else(missing_filesystem_response)
                    .and_then(|resolved| {
                        resolved_from_proto(resolved).map_err(invalid_filesystem_response)
                    })
            })
            .transpose()
    }

    async fn create_inode(&self, request: CreateInodeRequest) -> DmsResult<ResolvedDentry> {
        let mut operation_digest = digest(&request.operation_id);
        operation_digest.extend_from_slice(&request.parent.to_be_bytes());
        operation_digest.extend_from_slice(&request.name);
        operation_digest.extend_from_slice(&(request.kind.to_proto() as u32).to_be_bytes());
        operation_digest.extend_from_slice(&request.mode.to_be_bytes());
        operation_digest.extend_from_slice(&request.uid.to_be_bytes());
        operation_digest.extend_from_slice(&request.gid.to_be_bytes());
        let response = self
            .metadata
            .filesystem_create_inode(pb::FilesystemCreateInodeRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                parent: request.parent,
                name: request.name,
                kind: request.kind.to_proto(),
                mode: request.mode,
                uid: request.uid,
                gid: request.gid,
                expected_parent_revision: None,
                operation_digest,
                commit_sequence: 0,
                reference_generation: request.reference_generation,
            })
            .await?;
        let dentry = response.dentry.ok_or_else(missing_filesystem_response)?;
        let resolved = response.resolved.ok_or_else(missing_filesystem_response)?;
        Ok(ResolvedDentry {
            dentry: dentry_from_proto(dentry),
            resolved: resolved_from_proto(resolved).map_err(invalid_filesystem_response)?,
            directory_grant: response
                .directory_grant
                .ok_or_else(missing_filesystem_response)
                .and_then(|grant| {
                    directory_grant_from_proto(grant).map_err(invalid_filesystem_response)
                })?,
            entry_reference_lease_millis: response.entry_reference_lease_millis,
            entry_reference_generation: response.entry_reference_generation,
        })
    }

    async fn create_symlink(&self, request: CreateSymlinkRequest) -> DmsResult<ResolvedDentry> {
        let response = self
            .metadata
            .filesystem_create_symlink(pb::FilesystemCreateSymlinkRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                parent: request.parent,
                name: request.name,
                uid: request.uid,
                gid: request.gid,
                expected_parent_revision: request.expected_parent_revision,
                commit_sequence: request.commit_sequence,
                object_key: request.prepared.object_key,
                candidate: Some(request.prepared.candidate),
                replica_proofs: request.prepared.replica_proofs,
                new_replicas: request.prepared.new_replicas,
                target_size: request.target_size,
                mtime_unix_nanos: request.mtime_unix_nanos,
                reference_generation: request.reference_generation,
            })
            .await?;
        let dentry = response.dentry.ok_or_else(missing_filesystem_response)?;
        let resolved = response.resolved.ok_or_else(missing_filesystem_response)?;
        Ok(ResolvedDentry {
            dentry: dentry_from_proto(dentry),
            resolved: resolved_from_proto(resolved).map_err(invalid_filesystem_response)?,
            directory_grant: response
                .directory_grant
                .ok_or_else(missing_filesystem_response)
                .and_then(|grant| {
                    directory_grant_from_proto(grant).map_err(invalid_filesystem_response)
                })?,
            entry_reference_lease_millis: response.entry_reference_lease_millis,
            entry_reference_generation: response.entry_reference_generation,
        })
    }

    async fn read_directory(
        &self,
        directory: InodeId,
        cursor: Option<Vec<u8>>,
        limit: u32,
        expected_directory_revision: Option<u64>,
    ) -> DmsResult<DirectoryPage> {
        let response = self
            .metadata
            .filesystem_read_directory(
                directory,
                cursor.unwrap_or_default(),
                limit,
                expected_directory_revision,
            )
            .await?;
        directory_page_from_proto(directory, response).map_err(invalid_filesystem_response)
    }

    async fn link_entry(&self, request: LinkEntryRequest) -> DmsResult<NamespaceMutationResult> {
        let response = self
            .metadata
            .filesystem_link_entry(pb::FilesystemLinkRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                source_inode: request.source_inode,
                target_parent: request.target_parent,
                target_name: request.target_name,
                expected_target_revision: request.expected_target_revision,
                commit_sequence: request.commit_sequence,
                reference_generation: request.reference_generation,
            })
            .await?;
        namespace_result_from_proto(response).map_err(invalid_filesystem_response)
    }

    async fn acquire_inode_reference(&self, inode: InodeId, generation: u64) -> DmsResult<u64> {
        self.metadata
            .filesystem_acquire_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: None,
                inode,
                reference_generation: generation,
            })
            .await
            .map(|response| response.lease_millis)
    }

    async fn release_inode_reference(&self, inode: InodeId, generation: u64) -> DmsResult<()> {
        self.metadata
            .filesystem_release_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: None,
                inode,
                reference_generation: generation,
            })
            .await
            .map(|_| ())
    }

    async fn rename_entry(
        &self,
        request: RenameEntryRequest,
    ) -> DmsResult<NamespaceMutationResult> {
        let response = self
            .metadata
            .filesystem_rename_entry(pb::FilesystemRenameRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                source_parent: request.source_parent,
                source_name: request.source_name,
                target_parent: request.target_parent,
                target_name: request.target_name,
                expected_source_revision: request.expected_source_revision,
                expected_target_revision: request.expected_target_revision,
                commit_sequence: request.commit_sequence,
                replace_existing: request.replace_existing,
            })
            .await?;
        namespace_result_from_proto(response).map_err(invalid_filesystem_response)
    }

    async fn remove_entry(
        &self,
        request: RemoveEntryRequest,
    ) -> DmsResult<NamespaceMutationResult> {
        let response = self
            .metadata
            .filesystem_remove_entry(pb::FilesystemRemoveRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                parent: request.parent,
                name: request.name,
                kind: request.kind.to_proto(),
                expected_parent_revision: request.expected_parent_revision,
                commit_sequence: request.commit_sequence,
            })
            .await?;
        namespace_result_from_proto(response).map_err(invalid_filesystem_response)
    }

    async fn resolve_inode(&self, inode: InodeId) -> DmsResult<Option<ResolvedInode>> {
        self.get_inode(inode).await
    }

    async fn commit_file_version(
        &self,
        request: CommitFileVersionRequest,
    ) -> DmsResult<CommitFileVersionResult> {
        let mut operation_digest = digest(&request.operation_id);
        operation_digest.extend_from_slice(&request.inode.to_be_bytes());
        operation_digest.extend_from_slice(&request.expected_inode_revision.to_be_bytes());
        operation_digest.extend_from_slice(&request.prepared.candidate.digest);
        operation_digest.extend_from_slice(&request.new_size.to_be_bytes());
        operation_digest.extend_from_slice(&request.mtime_unix_nanos.to_be_bytes());
        if let Some(caller) = request.caller {
            operation_digest.extend_from_slice(&caller.uid.to_be_bytes());
            operation_digest.extend_from_slice(&caller.gid.to_be_bytes());
            operation_digest.extend_from_slice(&caller.pid.to_be_bytes());
        }
        append_attribute_patch_digest(&mut operation_digest, request.attribute_patch);
        let response = self
            .metadata
            .filesystem_commit_version(pb::FilesystemCommitVersionRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                operation_digest,
                inode: request.inode,
                expected_inode_revision: request.expected_inode_revision,
                object_key: request.prepared.object_key,
                expected_object_version: request.prepared.expected_object_version,
                candidate: Some(request.prepared.candidate),
                replica_proofs: request.prepared.replica_proofs,
                new_replicas: request.prepared.new_replicas,
                new_size: request.new_size,
                mtime_unix_nanos: request.mtime_unix_nanos,
                commit_sequence: 0,
                caller: request.caller.map(caller_to_proto),
                attribute_patch: (!request.attribute_patch.is_empty())
                    .then(|| attribute_patch_to_proto(request.attribute_patch)),
            })
            .await?;
        let resolved = response.resolved.ok_or_else(missing_filesystem_response)?;
        Ok(CommitFileVersionResult {
            resolved: resolved_from_proto(resolved).map_err(invalid_filesystem_response)?,
            invalidation_cursor: response.invalidation_cursor,
        })
    }

    async fn set_attributes(
        &self,
        request: SetAttributesRequest,
    ) -> DmsResult<crate::filesystem::AttributeMutationResult> {
        let response = self
            .metadata
            .filesystem_set_attributes(pb::FilesystemSetAttributesRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                inode: request.inode,
                expected_inode_revision: request.expected_inode_revision,
                caller: Some(caller_to_proto(request.caller)),
                patch: Some(attribute_patch_to_proto(request.patch)),
                commit_sequence: request.commit_sequence,
            })
            .await?;
        attribute_result_from_proto(response).map_err(invalid_filesystem_response)
    }

    async fn get_xattr(
        &self,
        inode: InodeId,
        name: &[u8],
        caller: FilesystemCaller,
    ) -> DmsResult<Option<Vec<u8>>> {
        let response = self
            .metadata
            .filesystem_get_xattr(inode, name.to_vec(), caller_to_proto(caller))
            .await?;
        Ok(response.found.then_some(response.value))
    }

    async fn list_xattrs(
        &self,
        inode: InodeId,
        caller: FilesystemCaller,
    ) -> DmsResult<Vec<Vec<u8>>> {
        self.metadata
            .filesystem_list_xattrs(inode, caller_to_proto(caller))
            .await
            .map(|response| response.names)
    }

    async fn set_xattr(&self, request: SetXattrRequest) -> DmsResult<AttributeMutationResult> {
        let response = self
            .metadata
            .filesystem_set_xattr(pb::FilesystemSetXattrRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                inode: request.inode,
                expected_inode_revision: request.expected_inode_revision,
                caller: Some(caller_to_proto(request.caller)),
                name: request.name,
                value: request.value,
                mode: request.mode.to_proto(),
                commit_sequence: 0,
            })
            .await?;
        attribute_result_from_proto(response).map_err(invalid_filesystem_response)
    }

    async fn remove_xattr(
        &self,
        request: RemoveXattrRequest,
    ) -> DmsResult<AttributeMutationResult> {
        let response = self
            .metadata
            .filesystem_remove_xattr(pb::FilesystemRemoveXattrRequest {
                context: None,
                session: None,
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                inode: request.inode,
                expected_inode_revision: request.expected_inode_revision,
                caller: Some(caller_to_proto(request.caller)),
                name: request.name,
                commit_sequence: 0,
            })
            .await?;
        attribute_result_from_proto(response).map_err(invalid_filesystem_response)
    }

    async fn stat_filesystem(&self) -> DmsResult<FilesystemStats> {
        self.metadata
            .filesystem_stat()
            .await
            .map(filesystem_stats_from_proto)
    }
}

fn append_attribute_patch_digest(digest: &mut Vec<u8>, patch: AttributePatch) {
    digest.extend_from_slice(&patch.mode.unwrap_or(u32::MAX).to_be_bytes());
    digest.extend_from_slice(&patch.uid.unwrap_or(u32::MAX).to_be_bytes());
    digest.extend_from_slice(&patch.gid.unwrap_or(u32::MAX).to_be_bytes());
    append_time_update_digest(digest, patch.atime);
    append_time_update_digest(digest, patch.mtime);
}

fn append_time_update_digest(digest: &mut Vec<u8>, update: TimeUpdate) {
    match update {
        TimeUpdate::Omit => digest.push(1),
        TimeUpdate::Now => digest.push(2),
        TimeUpdate::Exact(nanos) => {
            digest.push(3);
            digest.extend_from_slice(&nanos.to_be_bytes());
        }
    }
}

fn missing_filesystem_response() -> DmsError {
    DmsError::new(
        dms_error::NODE_METADATA_UNAVAILABLE,
        ErrorKind::Unavailable,
        "Meta filesystem response is missing required fields",
    )
}

fn invalid_filesystem_response(error: crate::filesystem::FileContractError) -> DmsError {
    DmsError::new(
        dms_error::NODE_METADATA_UNAVAILABLE,
        ErrorKind::Unavailable,
        format!("invalid Meta filesystem response: {error:?}"),
    )
}

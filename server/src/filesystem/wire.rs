//! Filesystem 领域值与 protobuf DTO 的唯一转换边界。
//!
//! FUSE、共享文件服务和 `DataCoreHandle` 只看到 [`ResolvedObject`] 这一不透明读取计划，
//! 不依赖 generated protobuf 类型。Node runtime 与 Meta adapter 在真正跨进程的位置才
//! 取出 wire 值；因此不会为了文件入口再复制一套 Extent/Block 数据结构或算法。

use dms_protocol::v1 as pb;

use super::model::{
    AttributeMutationResult, AttributePatch, CacheGrant, DentrySnapshot, DirectoryEntry,
    DirectoryGrant, DirectoryPage, DirectoryVersion, FileContentBinding, FileContractError,
    FileSpaceReservation, FilesystemCaller, FilesystemStats, GrantedInode, InodeAttributes,
    InodeId, InodeKind, InodeSnapshot, InodeVersion, NamespaceMutationResult, RemoveKind,
    ResolvedInode, TimeUpdate, XattrSetMode,
};
use super::{
    FileLockContractError, FileLockMode, FileLockOutcome, FileLockOwner, FileLockRange,
    FileLockRequest, GrantedFileLock,
};

impl FileLockMode {
    pub(crate) const fn to_proto(self) -> i32 {
        match self {
            Self::Shared => pb::FilesystemLockMode::Shared as i32,
            Self::Exclusive => pb::FilesystemLockMode::Exclusive as i32,
            Self::Unlock => pb::FilesystemLockMode::Unlock as i32,
        }
    }

    pub(crate) fn from_proto(value: i32) -> Result<Self, FileLockContractError> {
        match pb::FilesystemLockMode::try_from(value) {
            Ok(pb::FilesystemLockMode::Shared) => Ok(Self::Shared),
            Ok(pb::FilesystemLockMode::Exclusive) => Ok(Self::Exclusive),
            Ok(pb::FilesystemLockMode::Unlock) => Ok(Self::Unlock),
            _ => Err(FileLockContractError::InvalidMode),
        }
    }
}

pub(crate) fn lock_request_from_proto(
    request: &pb::FilesystemLockRequest,
    session: &pb::NodeSessionIdentity,
) -> Result<FileLockRequest, FileLockContractError> {
    let range = request
        .range
        .as_ref()
        .ok_or(FileLockContractError::InvalidRange)?;
    Ok(FileLockRequest {
        request_id: request.request_id,
        inode: request.inode,
        owner: FileLockOwner {
            node_id: session.node_id,
            node_epoch: session.node_epoch,
            lock_owner: request.lock_owner,
        },
        range: FileLockRange::new(range.start, range.end_inclusive)?,
        mode: FileLockMode::from_proto(request.mode)?,
        pid: request.pid,
        wait: request.wait,
    })
}

pub(crate) fn granted_lock_to_proto(
    inode: InodeId,
    lock: GrantedFileLock,
) -> pb::FilesystemGrantedLock {
    pb::FilesystemGrantedLock {
        inode,
        owner: Some(pb::FilesystemLockOwner {
            node_id: lock.owner.node_id,
            node_epoch: lock.owner.node_epoch,
            lock_owner: lock.owner.lock_owner,
        }),
        range: Some(pb::FilesystemLockRange {
            start: lock.range.start,
            end_inclusive: lock.range.end_inclusive,
        }),
        mode: lock.mode.to_proto(),
        pid: lock.pid,
    }
}

pub(crate) fn granted_lock_from_proto(
    lock: pb::FilesystemGrantedLock,
    session: &pb::NodeSessionIdentity,
) -> Result<(InodeId, GrantedFileLock), FileLockContractError> {
    let owner = lock.owner.ok_or(FileLockContractError::InvalidMode)?;
    let range = lock.range.ok_or(FileLockContractError::InvalidRange)?;
    if owner.node_id != session.node_id || owner.node_epoch != session.node_epoch {
        return Err(FileLockContractError::InvalidMode);
    }
    Ok((
        lock.inode,
        GrantedFileLock {
            owner: FileLockOwner {
                node_id: owner.node_id,
                node_epoch: owner.node_epoch,
                lock_owner: owner.lock_owner,
            },
            range: FileLockRange::new(range.start, range.end_inclusive)?,
            mode: FileLockMode::from_proto(lock.mode)?,
            pid: lock.pid,
        },
    ))
}

pub(crate) fn lock_outcome_to_proto(
    inode: InodeId,
    outcome: FileLockOutcome,
    owner_revision: u64,
) -> pb::FilesystemLockResponse {
    let (status, conflict) = match outcome {
        FileLockOutcome::Acquired => (pb::FilesystemLockStatus::Acquired, None),
        FileLockOutcome::Released => (pb::FilesystemLockStatus::Released, None),
        FileLockOutcome::Conflict(lock) => (
            pb::FilesystemLockStatus::Conflict,
            Some(granted_lock_to_proto(inode, lock)),
        ),
        FileLockOutcome::Interrupted => (pb::FilesystemLockStatus::Interrupted, None),
        FileLockOutcome::RecoveryPending => (pb::FilesystemLockStatus::RecoveryPending, None),
    };
    pb::FilesystemLockResponse {
        status: status as i32,
        conflict,
        owner_revision,
    }
}

/// Meta 已经授权的精确对象读取计划。
///
/// 内部继续复用既有对象协议，避免复制布局；不透明包装阻止 wire 类型越过
/// Filesystem/DataCore 的进程内合同。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedObject(pb::ResolveObjectResponse);

impl ResolvedObject {
    pub(crate) fn from_proto(value: pb::ResolveObjectResponse) -> Self {
        Self(value)
    }

    pub(crate) fn into_proto(self) -> pb::ResolveObjectResponse {
        self.0
    }
}

impl XattrSetMode {
    pub(crate) const fn to_proto(self) -> i32 {
        match self {
            Self::Upsert => pb::FilesystemXattrSetMode::Upsert as i32,
            Self::CreateOnly => pb::FilesystemXattrSetMode::CreateOnly as i32,
            Self::ReplaceOnly => pb::FilesystemXattrSetMode::ReplaceOnly as i32,
        }
    }

    pub(crate) fn from_proto(value: i32) -> Result<Self, FileContractError> {
        match pb::FilesystemXattrSetMode::try_from(value) {
            Ok(pb::FilesystemXattrSetMode::Upsert) => Ok(Self::Upsert),
            Ok(pb::FilesystemXattrSetMode::CreateOnly) => Ok(Self::CreateOnly),
            Ok(pb::FilesystemXattrSetMode::ReplaceOnly) => Ok(Self::ReplaceOnly),
            _ => Err(FileContractError::InvalidXattrSetMode),
        }
    }
}

pub(crate) fn filesystem_stats_from_proto(value: pb::FilesystemStatResponse) -> FilesystemStats {
    FilesystemStats {
        block_size: value.block_size,
        total_blocks: value.total_blocks,
        free_blocks: value.free_blocks,
        available_blocks: value.available_blocks,
        total_inodes: value.total_inodes,
        free_inodes: value.free_inodes,
        max_name_length: value.max_name_length,
        reporting_nodes: value.reporting_nodes,
        capacity_revision: value.capacity_revision,
    }
}

/// DataCore 已经准备完成、但尚未成为 Object Current 的对象版本候选。
///
/// 这些字段由 Node runtime 产生、由 Node→Meta adapter 序列化。文件业务不读取或
/// 重写 Extent；Meta 在同一条 journal 记录中为候选分配版本并发布 inode binding。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreparedObjectVersion {
    pub(crate) object_key: Vec<u8>,
    pub(crate) expected_object_version: Option<u64>,
    pub(crate) candidate: pb::VersionCandidate,
    pub(crate) replica_proofs: Vec<pb::ReplicaProof>,
    pub(crate) new_replicas: Vec<pb::ReplicaReport>,
}

impl InodeKind {
    pub(crate) const fn to_proto(self) -> i32 {
        match self {
            Self::RegularFile => pb::FilesystemInodeKind::RegularFile as i32,
            Self::Directory => pb::FilesystemInodeKind::Directory as i32,
            Self::SymbolicLink => pb::FilesystemInodeKind::SymbolicLink as i32,
        }
    }

    pub(crate) fn from_proto(value: i32) -> Result<Self, FileContractError> {
        match pb::FilesystemInodeKind::try_from(value) {
            Ok(pb::FilesystemInodeKind::RegularFile) => Ok(Self::RegularFile),
            Ok(pb::FilesystemInodeKind::Directory) => Ok(Self::Directory),
            Ok(pb::FilesystemInodeKind::SymbolicLink) => Ok(Self::SymbolicLink),
            _ => Err(FileContractError::InvalidInodeKind),
        }
    }
}

pub(crate) fn inode_to_proto(inode: &InodeSnapshot) -> pb::FilesystemInodeSnapshot {
    let attributes = &inode.attributes;
    pb::FilesystemInodeSnapshot {
        revision: inode.revision,
        attributes: Some(pb::FilesystemInodeAttributes {
            inode: attributes.inode,
            kind: attributes.kind.to_proto(),
            mode: attributes.mode,
            uid: attributes.uid,
            gid: attributes.gid,
            link_count: attributes.link_count,
            size: attributes.size,
            atime_unix_nanos: attributes.atime_unix_nanos,
            mtime_unix_nanos: attributes.mtime_unix_nanos,
            ctime_unix_nanos: attributes.ctime_unix_nanos,
        }),
        content: inode
            .content
            .as_ref()
            .map(|content| pb::FilesystemContentBinding {
                object_key: content.object_key.clone(),
                exact_version: content.exact_version,
            }),
        reservations: inode
            .reservations
            .iter()
            .map(|reservation| pb::FilesystemSpaceReservation {
                reservation_id: reservation.reservation_id.clone(),
                node_id: reservation.node_id,
                node_epoch: reservation.node_epoch,
                offset: reservation.offset,
                length: reservation.length,
            })
            .collect(),
    }
}

pub(crate) fn inode_from_proto(
    inode: pb::FilesystemInodeSnapshot,
) -> Result<InodeSnapshot, FileContractError> {
    let attributes = inode
        .attributes
        .ok_or(FileContractError::MissingInodeAttributes)?;
    Ok(InodeSnapshot {
        revision: inode.revision,
        attributes: InodeAttributes {
            inode: attributes.inode,
            kind: InodeKind::from_proto(attributes.kind)?,
            mode: attributes.mode,
            uid: attributes.uid,
            gid: attributes.gid,
            link_count: attributes.link_count,
            size: attributes.size,
            atime_unix_nanos: attributes.atime_unix_nanos,
            mtime_unix_nanos: attributes.mtime_unix_nanos,
            ctime_unix_nanos: attributes.ctime_unix_nanos,
        },
        content: inode.content.map(|content| FileContentBinding {
            object_key: content.object_key,
            exact_version: content.exact_version,
        }),
        reservations: inode
            .reservations
            .into_iter()
            .map(|reservation| FileSpaceReservation {
                reservation_id: reservation.reservation_id,
                node_id: reservation.node_id,
                node_epoch: reservation.node_epoch,
                offset: reservation.offset,
                length: reservation.length,
            })
            .collect(),
    })
}

pub(crate) fn caller_to_proto(caller: FilesystemCaller) -> pb::FilesystemCallerIdentity {
    pb::FilesystemCallerIdentity {
        uid: caller.uid,
        gid: caller.gid,
        pid: caller.pid,
    }
}

pub(crate) fn caller_from_proto(caller: pb::FilesystemCallerIdentity) -> FilesystemCaller {
    FilesystemCaller {
        uid: caller.uid,
        gid: caller.gid,
        pid: caller.pid,
    }
}

fn time_update_to_proto(update: TimeUpdate) -> pb::FilesystemTimeUpdate {
    let (kind, exact_unix_nanos) = match update {
        TimeUpdate::Omit => (pb::FilesystemTimeUpdateKind::Omit, 0),
        TimeUpdate::Now => (pb::FilesystemTimeUpdateKind::Now, 0),
        TimeUpdate::Exact(value) => (pb::FilesystemTimeUpdateKind::Exact, value),
    };
    pb::FilesystemTimeUpdate {
        kind: kind as i32,
        exact_unix_nanos,
    }
}

fn time_update_from_proto(
    update: Option<pb::FilesystemTimeUpdate>,
) -> Result<TimeUpdate, FileContractError> {
    let Some(update) = update else {
        return Ok(TimeUpdate::Omit);
    };
    match pb::FilesystemTimeUpdateKind::try_from(update.kind) {
        Ok(pb::FilesystemTimeUpdateKind::Omit) => Ok(TimeUpdate::Omit),
        Ok(pb::FilesystemTimeUpdateKind::Now) => Ok(TimeUpdate::Now),
        Ok(pb::FilesystemTimeUpdateKind::Exact) => Ok(TimeUpdate::Exact(update.exact_unix_nanos)),
        _ => Err(FileContractError::InvalidAttributePatch),
    }
}

pub(crate) fn attribute_patch_to_proto(patch: AttributePatch) -> pb::FilesystemAttributePatch {
    pb::FilesystemAttributePatch {
        mode: patch.mode,
        uid: patch.uid,
        gid: patch.gid,
        atime: Some(time_update_to_proto(patch.atime)),
        mtime: Some(time_update_to_proto(patch.mtime)),
    }
}

pub(crate) fn attribute_patch_from_proto(
    patch: pb::FilesystemAttributePatch,
) -> Result<AttributePatch, FileContractError> {
    Ok(AttributePatch {
        mode: patch.mode,
        uid: patch.uid,
        gid: patch.gid,
        atime: time_update_from_proto(patch.atime)?,
        mtime: time_update_from_proto(patch.mtime)?,
    })
}

pub(crate) fn attribute_result_from_proto(
    response: pb::FilesystemAttributeMutationResponse,
) -> Result<AttributeMutationResult, FileContractError> {
    Ok(AttributeMutationResult {
        resolved: resolved_from_proto(
            response
                .resolved
                .ok_or(FileContractError::MissingResolvedInode)?,
        )?,
        invalidation_cursor: response.invalidation_cursor,
        commit_index: response.commit_index,
    })
}

pub(crate) fn dentry_to_proto(dentry: &DentrySnapshot) -> pb::FilesystemDentrySnapshot {
    pb::FilesystemDentrySnapshot {
        parent: dentry.parent,
        name: dentry.name.clone(),
        inode: dentry.inode,
        directory_revision: dentry.directory_revision,
    }
}

pub(crate) fn dentry_from_proto(dentry: pb::FilesystemDentrySnapshot) -> DentrySnapshot {
    DentrySnapshot {
        parent: dentry.parent,
        name: dentry.name,
        inode: dentry.inode,
        directory_revision: dentry.directory_revision,
    }
}

pub(crate) fn directory_grant_to_proto(grant: DirectoryGrant) -> pb::FilesystemDirectoryGrant {
    pb::FilesystemDirectoryGrant {
        directory_revision: grant.directory_revision,
        cache: Some(pb::FilesystemCacheGrant {
            generation: grant.grant.generation,
            lease_millis: grant.grant.lease_millis,
        }),
    }
}

pub(crate) fn directory_grant_from_proto(
    grant: pb::FilesystemDirectoryGrant,
) -> Result<DirectoryGrant, FileContractError> {
    let cache = grant.cache.ok_or(FileContractError::MissingResolvedInode)?;
    Ok(DirectoryGrant {
        directory_revision: grant.directory_revision,
        grant: CacheGrant {
            generation: cache.generation,
            lease_millis: cache.lease_millis,
        },
    })
}

pub(crate) fn directory_entry_to_proto(entry: &DirectoryEntry) -> pb::FilesystemDirectoryEntry {
    pb::FilesystemDirectoryEntry {
        dentry: Some(dentry_to_proto(&entry.dentry)),
        attributes: Some(inode_attributes_to_proto(&entry.attributes)),
    }
}

pub(crate) fn directory_entry_from_proto(
    entry: pb::FilesystemDirectoryEntry,
) -> Result<DirectoryEntry, FileContractError> {
    Ok(DirectoryEntry {
        dentry: dentry_from_proto(
            entry
                .dentry
                .ok_or(FileContractError::MissingResolvedInode)?,
        ),
        attributes: inode_attributes_from_proto(
            entry
                .attributes
                .ok_or(FileContractError::MissingInodeAttributes)?,
        )?,
    })
}

pub(crate) fn directory_page_from_proto(
    directory: InodeId,
    page: pb::FilesystemReadDirectoryResponse,
) -> Result<DirectoryPage, FileContractError> {
    Ok(DirectoryPage {
        directory,
        parent: page.parent,
        grant: directory_grant_from_proto(
            page.directory_grant
                .ok_or(FileContractError::MissingResolvedInode)?,
        )?,
        entries: page
            .entries
            .into_iter()
            .map(directory_entry_from_proto)
            .collect::<Result<_, _>>()?,
        next_cursor: page.has_more.then_some(page.next_cursor),
    })
}

pub(crate) fn namespace_result_to_proto(
    result: &NamespaceMutationResult,
) -> pb::FilesystemNamespaceMutationResponse {
    pb::FilesystemNamespaceMutationResponse {
        dentry: result.dentry.as_ref().map(dentry_to_proto),
        inode: result.inode.as_ref().map(inode_to_proto),
        changed_directories: result
            .changed_directories
            .iter()
            .map(|directory| pb::FilesystemDirectoryVersion {
                inode: directory.inode,
                revision: directory.revision,
                grant_generation: directory.grant_generation,
            })
            .collect(),
        invalidation_cursor: result.invalidation_cursor,
        commit_index: result.commit_index,
        changed_inodes: result
            .changed_inodes
            .iter()
            .map(|inode| pb::FilesystemDirectoryVersion {
                inode: inode.inode,
                revision: inode.revision,
                grant_generation: inode.grant_generation,
            })
            .collect(),
        entry_reference_lease_millis: result.entry_reference_lease_millis,
        entry_reference_generation: result.entry_reference_generation,
        // 这是在线响应缓存提示，不属于持久化 NamespaceMutationResult。
        refreshed_directories: Vec::new(),
    }
}

pub(crate) fn namespace_result_from_proto(
    result: pb::FilesystemNamespaceMutationResponse,
) -> Result<NamespaceMutationResult, FileContractError> {
    Ok(NamespaceMutationResult {
        dentry: result.dentry.map(dentry_from_proto),
        inode: result.inode.map(inode_from_proto).transpose()?,
        changed_directories: result
            .changed_directories
            .into_iter()
            .map(|directory| DirectoryVersion {
                inode: directory.inode,
                revision: directory.revision,
                grant_generation: directory.grant_generation,
            })
            .collect(),
        changed_inodes: result
            .changed_inodes
            .into_iter()
            .map(|inode| InodeVersion {
                inode: inode.inode,
                revision: inode.revision,
                grant_generation: inode.grant_generation,
            })
            .collect(),
        invalidation_cursor: result.invalidation_cursor,
        commit_index: result.commit_index,
        entry_reference_lease_millis: result.entry_reference_lease_millis,
        entry_reference_generation: result.entry_reference_generation,
    })
}

impl RemoveKind {
    pub(crate) const fn to_proto(self) -> i32 {
        match self {
            Self::File => pb::FilesystemRemoveKind::File as i32,
            Self::Directory => pb::FilesystemRemoveKind::Directory as i32,
        }
    }
}

fn inode_attributes_to_proto(attributes: &InodeAttributes) -> pb::FilesystemInodeAttributes {
    pb::FilesystemInodeAttributes {
        inode: attributes.inode,
        kind: attributes.kind.to_proto(),
        mode: attributes.mode,
        uid: attributes.uid,
        gid: attributes.gid,
        link_count: attributes.link_count,
        size: attributes.size,
        atime_unix_nanos: attributes.atime_unix_nanos,
        mtime_unix_nanos: attributes.mtime_unix_nanos,
        ctime_unix_nanos: attributes.ctime_unix_nanos,
    }
}

fn inode_attributes_from_proto(
    attributes: pb::FilesystemInodeAttributes,
) -> Result<InodeAttributes, FileContractError> {
    Ok(InodeAttributes {
        inode: attributes.inode,
        kind: InodeKind::from_proto(attributes.kind)?,
        mode: attributes.mode,
        uid: attributes.uid,
        gid: attributes.gid,
        link_count: attributes.link_count,
        size: attributes.size,
        atime_unix_nanos: attributes.atime_unix_nanos,
        mtime_unix_nanos: attributes.mtime_unix_nanos,
        ctime_unix_nanos: attributes.ctime_unix_nanos,
    })
}

pub(crate) fn resolved_to_proto(resolved: &ResolvedInode) -> pb::FilesystemResolvedInode {
    pb::FilesystemResolvedInode {
        inode: Some(inode_to_proto(&resolved.granted.inode)),
        grant: Some(pb::FilesystemCacheGrant {
            generation: resolved.granted.grant.generation,
            lease_millis: resolved.granted.grant.lease_millis,
        }),
        object: resolved.object.clone().map(ResolvedObject::into_proto),
        access_acl: resolved.access_acl.clone(),
    }
}

pub(crate) fn resolved_from_proto(
    resolved: pb::FilesystemResolvedInode,
) -> Result<ResolvedInode, FileContractError> {
    let inode = inode_from_proto(
        resolved
            .inode
            .ok_or(FileContractError::MissingResolvedInode)?,
    )?;
    let grant = resolved
        .grant
        .ok_or(FileContractError::MissingResolvedInode)?;
    Ok(ResolvedInode {
        granted: GrantedInode {
            inode,
            grant: CacheGrant {
                generation: grant.generation,
                lease_millis: grant.lease_millis,
            },
        },
        object: resolved.object.map(ResolvedObject::from_proto),
        access_acl: resolved.access_acl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{
        CacheGrant, CommitFileVersionRequest, GrantedInode, InodeAttributes, InodeKind,
        InodeSnapshot,
    };

    #[test]
    fn one_file_commit_carries_data_and_inode_changes_together() {
        let request = CommitFileVersionRequest {
            operation_id: b"client-1:9".to_vec(),
            inode: 100,
            expected_inode_revision: 12,
            prepared: PreparedObjectVersion {
                object_key: b"fs/content/100".to_vec(),
                expected_object_version: Some(7),
                candidate: pb::VersionCandidate {
                    logical_length: 8,
                    extents: Vec::new(),
                    digest: Vec::new(),
                    kind: pb::VersionKind::Value as i32,
                },
                replica_proofs: Vec::new(),
                new_replicas: Vec::new(),
            },
            new_size: 8,
            mtime_unix_nanos: 99,
            caller: None,
            attribute_patch: AttributePatch::default(),
            reservation_additions: Vec::new(),
            reservation_reductions: Vec::new(),
        };

        // 文件层不能先提交 DataCore Current，再用另一次请求更新 inode。一个请求同时
        // 带着对象候选、inode CAS、size 和 mtime，供 Meta 写成一条 journal 记录。
        assert_eq!(request.inode, 100);
        assert_eq!(request.prepared.expected_object_version, Some(7));
        assert_eq!(request.new_size, request.prepared.candidate.logical_length);
    }

    #[test]
    fn resolved_inode_wire_keeps_access_acl_with_its_cache_grant() {
        let resolved = ResolvedInode {
            granted: GrantedInode {
                inode: InodeSnapshot {
                    revision: 3,
                    attributes: InodeAttributes {
                        inode: 9,
                        kind: InodeKind::RegularFile,
                        mode: 0o640,
                        uid: 1,
                        gid: 2,
                        link_count: 1,
                        size: 0,
                        atime_unix_nanos: 0,
                        mtime_unix_nanos: 0,
                        ctime_unix_nanos: 0,
                    },
                    content: None,
                    reservations: Vec::new(),
                },
                grant: CacheGrant {
                    generation: 4,
                    lease_millis: 5_000,
                },
            },
            object: None,
            access_acl: Some(vec![2, 0, 0, 0]),
        };

        assert_eq!(
            resolved_from_proto(resolved_to_proto(&resolved)).expect("wire round trip"),
            resolved
        );
    }
}

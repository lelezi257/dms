//! DistributedFs namespace and FileVersion authority on the shared MetaStore.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use afs_error::{Error, Result};

use crate::dfs::{
    CommitFileVersion, CommitMetadataMode, CopyState, Dentry, DentryKey, FileVersion,
    FileVersionId, InodeAttributes, InodeId, InodeKind, InodeRecord, LayoutRoot, NamespaceId,
    OperationId, PlacementRecord, SyncInodeMetadata, WriteLease,
};

use super::store::{
    MetaEntity, MetaKey, MetaRead, MetaStore, MetaTxn, OperationResult, RequestKey, RequestOutcome,
    StoreOperation, TxnCondition, TxnMutation, TxnOutcome,
};

#[derive(Clone)]
pub struct DfsService {
    store: Arc<dyn MetaStore>,
}

#[derive(Clone, Debug)]
pub struct CreateFileRequest {
    pub caller_id: String,
    pub owner_session_id: String,
    pub operation_id: OperationId,
    pub namespace_id: NamespaceId,
    pub parent_inode_id: InodeId,
    pub name: Vec<u8>,
    pub attributes: InodeAttributes,
    pub lease_seconds: u64,
}

impl DfsService {
    pub fn new(store: Arc<dyn MetaStore>) -> Self {
        Self { store }
    }

    pub async fn lookup(
        &self,
        namespace_id: NamespaceId,
        parent_inode_id: InodeId,
        name: Vec<u8>,
    ) -> Result<Option<InodeRecord>> {
        require_id(&namespace_id.0, "namespace_id")?;
        require_id(&parent_inode_id.0, "parent_inode_id")?;
        if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
            return Err(invalid("DFS name must be one non-empty path component"));
        }
        let key = DentryKey {
            namespace_id,
            parent_inode_id,
            name,
        };
        let snapshot = self.store.read(MetaRead::DfsDentry(key)).await?;
        let Some(MetaEntity::DfsDentry(dentry)) = snapshot.entity else {
            return Ok(None);
        };
        self.get_inode(dentry.inode_id).await.map(Some)
    }

    pub async fn create(&self, request: CreateFileRequest) -> Result<(InodeRecord, WriteLease)> {
        let CreateFileRequest {
            caller_id,
            owner_session_id,
            operation_id,
            namespace_id,
            parent_inode_id,
            name,
            attributes,
            lease_seconds,
        } = request;
        require_id(&caller_id, "caller_id")?;
        require_id(&owner_session_id, "owner_session_id")?;
        require_id(&operation_id.0, "operation_id")?;
        require_id(&namespace_id.0, "namespace_id")?;
        require_id(&parent_inode_id.0, "parent_inode_id")?;
        if let Some((inode, lease)) = self
            .replayed_inode_and_lease(&caller_id, &operation_id, StoreOperation::DfsCreate)
            .await?
        {
            return Ok((inode, lease));
        }
        if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
            return Err(invalid("DFS name must be one non-empty path component"));
        }
        self.ensure_parent_directory(&namespace_id, &parent_inode_id)
            .await?;
        let inode = InodeRecord {
            namespace_id: namespace_id.clone(),
            inode_id: InodeId::new(format!("inode:{}", operation_id.0)),
            kind: InodeKind::Regular,
            attributes,
            head_version: None,
            revision: 1,
        };
        let dentry = Dentry {
            key: DentryKey {
                namespace_id,
                parent_inode_id,
                name,
            },
            inode_id: inode.inode_id.clone(),
        };
        let lease = WriteLease {
            inode_id: inode.inode_id.clone(),
            owner_node_id: caller_id.clone(),
            owner_session_id,
            lease_epoch: 1,
            expires_at_unix_ms: lease_expiry(lease_seconds)?,
        };
        let request = RequestKey::new(caller_id, operation_id.0.clone());
        let outcome = RequestOutcome {
            request: request.clone(),
            operation: StoreOperation::DfsCreate,
            result: OperationResult::DfsInodeWithLease {
                inode: inode.clone(),
                lease: lease.clone(),
            },
        };
        let mut txn = MetaTxn::new(request.clone(), StoreOperation::DfsCreate);
        txn.conditions.extend([
            TxnCondition::RequestAbsent(request),
            TxnCondition::Missing(MetaKey::DfsDentry(dentry.key.clone())),
            TxnCondition::Missing(MetaKey::DfsInode(inode.inode_id.clone())),
            TxnCondition::Missing(MetaKey::DfsWriteLease(inode.inode_id.clone())),
        ]);
        txn.mutations.extend([
            TxnMutation::Put(MetaEntity::DfsDentry(dentry)),
            TxnMutation::Put(MetaEntity::DfsInode(inode.clone())),
            TxnMutation::Put(MetaEntity::DfsWriteLease(lease)),
            TxnMutation::RecordRequestOutcome(outcome),
        ]);
        inode_and_lease_outcome(
            self.store.compare_and_commit(txn).await?,
            StoreOperation::DfsCreate,
        )
    }

    async fn acquire_write_lease(
        &self,
        caller_id: String,
        owner_session_id: String,
        operation_id: OperationId,
        inode_id: InodeId,
        lease_seconds: u64,
    ) -> Result<WriteLease> {
        require_id(&caller_id, "caller_id")?;
        require_id(&owner_session_id, "owner_session_id")?;
        require_id(&operation_id.0, "operation_id")?;
        require_id(&inode_id.0, "inode_id")?;
        if let Some(lease) = self
            .replayed_lease(
                &caller_id,
                &operation_id,
                StoreOperation::DfsAcquireWriteLease,
            )
            .await?
        {
            return Ok(lease);
        }
        let inode = self.get_inode(inode_id.clone()).await?;
        if inode.kind != InodeKind::Regular {
            return Err(invalid("DFS write lease requires a regular file"));
        }
        let snapshot = self
            .store
            .read(MetaRead::DfsWriteLease(inode_id.clone()))
            .await?;
        let existing = match snapshot.entity {
            Some(MetaEntity::DfsWriteLease(lease)) => Some(lease),
            _ => None,
        };
        let now = now_unix_ms();
        let mut lease = existing.clone().unwrap_or_else(|| WriteLease {
            inode_id: inode_id.clone(),
            owner_node_id: caller_id.clone(),
            owner_session_id: owner_session_id.clone(),
            lease_epoch: 0,
            expires_at_unix_ms: 0,
        });
        let same_owner =
            lease.owner_node_id == caller_id && lease.owner_session_id == owner_session_id;
        let live = lease.expires_at_unix_ms > now;
        if live && !same_owner {
            return Err(conflict("DFS write lease is held by another live owner"));
        }
        lease.owner_node_id = caller_id.clone();
        lease.owner_session_id = owner_session_id;
        if !same_owner || !live {
            lease.lease_epoch = lease.lease_epoch.saturating_add(1);
        }
        lease.expires_at_unix_ms = lease_expiry(lease_seconds)?;

        let request = RequestKey::new(caller_id, operation_id.0.clone());
        let outcome = RequestOutcome {
            request: request.clone(),
            operation: StoreOperation::DfsAcquireWriteLease,
            result: OperationResult::DfsWriteLease(lease.clone()),
        };
        let mut txn = MetaTxn::new(request.clone(), StoreOperation::DfsAcquireWriteLease);
        txn.conditions.push(TxnCondition::RequestAbsent(request));
        match existing {
            Some(current) => {
                txn.conditions
                    .push(TxnCondition::EntityEquals(MetaEntity::DfsWriteLease(
                        current,
                    )))
            }
            None => txn
                .conditions
                .push(TxnCondition::Missing(MetaKey::DfsWriteLease(inode_id))),
        }
        txn.mutations.extend([
            TxnMutation::Put(MetaEntity::DfsWriteLease(lease)),
            TxnMutation::RecordRequestOutcome(outcome),
        ]);
        lease_outcome(
            self.store.compare_and_commit(txn).await?,
            StoreOperation::DfsAcquireWriteLease,
        )
    }

    pub async fn open_write(
        &self,
        caller_id: String,
        owner_session_id: String,
        operation_id: OperationId,
        inode_id: InodeId,
        lease_seconds: u64,
    ) -> Result<(InodeRecord, WriteLease)> {
        let inode = self.get_inode(inode_id.clone()).await?;
        let snapshot = self
            .store
            .read(MetaRead::DfsWriteLease(inode_id.clone()))
            .await?;
        if let Some(MetaEntity::DfsWriteLease(lease)) = snapshot.entity
            && lease.expires_at_unix_ms > now_unix_ms()
            && (lease.owner_node_id != caller_id || lease.owner_session_id != owner_session_id)
        {
            return Ok((inode, lease));
        }
        let lease = self
            .acquire_write_lease(
                caller_id,
                owner_session_id,
                operation_id,
                inode_id,
                lease_seconds,
            )
            .await?;
        Ok((inode, lease))
    }

    pub async fn renew_write_lease(
        &self,
        caller_id: String,
        owner_session_id: String,
        operation_id: OperationId,
        current: WriteLease,
        lease_seconds: u64,
    ) -> Result<WriteLease> {
        require_id(&caller_id, "caller_id")?;
        require_id(&owner_session_id, "owner_session_id")?;
        require_id(&operation_id.0, "operation_id")?;
        require_id(&current.inode_id.0, "current.inode_id")?;
        if current.owner_node_id != caller_id || current.owner_session_id != owner_session_id {
            return Err(conflict(
                "DFS write lease owner does not match renew caller",
            ));
        }
        if let Some(lease) = self
            .replayed_lease(
                &caller_id,
                &operation_id,
                StoreOperation::DfsRenewWriteLease,
            )
            .await?
        {
            return Ok(lease);
        }
        let mut renewed = current.clone();
        renewed.expires_at_unix_ms = lease_expiry(lease_seconds)?;
        let request = RequestKey::new(caller_id, operation_id.0.clone());
        let outcome = RequestOutcome {
            request: request.clone(),
            operation: StoreOperation::DfsRenewWriteLease,
            result: OperationResult::DfsWriteLease(renewed.clone()),
        };
        let mut txn = MetaTxn::new(request.clone(), StoreOperation::DfsRenewWriteLease);
        txn.conditions.extend([
            TxnCondition::RequestAbsent(request),
            TxnCondition::EntityEquals(MetaEntity::DfsWriteLease(current)),
        ]);
        txn.mutations.extend([
            TxnMutation::Put(MetaEntity::DfsWriteLease(renewed)),
            TxnMutation::RecordRequestOutcome(outcome),
        ]);
        lease_outcome(
            self.store.compare_and_commit(txn).await?,
            StoreOperation::DfsRenewWriteLease,
        )
    }

    pub async fn get_inode(&self, inode_id: InodeId) -> Result<InodeRecord> {
        require_id(&inode_id.0, "inode_id")?;
        if inode_id.0 == "1" {
            return Ok(root_inode(NamespaceId::new("default")));
        }
        let snapshot = self.store.read(MetaRead::DfsInode(inode_id)).await?;
        match snapshot.entity {
            Some(MetaEntity::DfsInode(inode)) => Ok(inode),
            _ => Err(Error::coded(
                afs_error::NODE_VFS_NOT_FOUND,
                "DFS inode was not found",
            )),
        }
    }

    pub async fn get_file_version(
        &self,
        version_id: FileVersionId,
    ) -> Result<(FileVersion, LayoutRoot)> {
        require_id(&version_id.0, "version_id")?;
        let version = match self
            .store
            .read(MetaRead::DfsFileVersion(version_id))
            .await?
            .entity
        {
            Some(MetaEntity::DfsFileVersion(version)) => version,
            _ => {
                return Err(Error::coded(
                    afs_error::NODE_VFS_NOT_FOUND,
                    "DFS FileVersion was not found",
                ));
            }
        };
        let layout = match self
            .store
            .read(MetaRead::DfsLayoutRoot(version.layout_root.clone()))
            .await?
            .entity
        {
            Some(MetaEntity::DfsLayoutRoot(layout)) => layout,
            _ => {
                return Err(Error::coded(
                    afs_error::NODE_VFS_NOT_FOUND,
                    "DFS LayoutRoot was not found",
                ));
            }
        };
        Ok((version, layout))
    }

    pub async fn sync_inode_metadata(
        &self,
        caller_id: String,
        sync: SyncInodeMetadata,
    ) -> Result<InodeRecord> {
        require_id(&caller_id, "caller_id")?;
        require_id(&sync.operation_id.0, "operation_id")?;
        require_id(&sync.inode_id.0, "inode_id")?;
        if let Some(inode) = self
            .replayed_inode(
                &caller_id,
                &sync.operation_id,
                StoreOperation::DfsSyncInodeMetadata,
            )
            .await?
        {
            return Ok(inode);
        }
        if sync.metadata_delta.mode != CommitMetadataMode::Full {
            return Err(invalid("DFS inode metadata sync requires full metadata"));
        }
        let current = self.get_inode(sync.inode_id.clone()).await?;
        let current_lease = self
            .validate_write_lease(&caller_id, &sync.inode_id, &sync.write_lease)
            .await?;
        if current.revision != sync.expected_inode_revision
            || current.head_version != sync.expected_head_version
        {
            return Err(conflict("DFS inode changed before metadata sync"));
        }
        let mut updated = current.clone();
        updated.revision = updated.revision.saturating_add(1);
        updated.attributes.mtime_unix_ms = sync
            .metadata_delta
            .mtime_unix_ms
            .ok_or_else(|| invalid("DFS full metadata sync requires mtime"))?;
        updated.attributes.ctime_unix_ms = sync
            .metadata_delta
            .ctime_unix_ms
            .ok_or_else(|| invalid("DFS full metadata sync requires ctime"))?;

        let request = RequestKey::new(caller_id, sync.operation_id.0.clone());
        let outcome = RequestOutcome {
            request: request.clone(),
            operation: StoreOperation::DfsSyncInodeMetadata,
            result: OperationResult::DfsInode(updated.clone()),
        };
        let mut txn = MetaTxn::new(request.clone(), StoreOperation::DfsSyncInodeMetadata);
        txn.conditions.extend([
            TxnCondition::RequestAbsent(request),
            TxnCondition::EntityEquals(MetaEntity::DfsInode(current)),
            TxnCondition::EntityEquals(MetaEntity::DfsWriteLease(current_lease)),
        ]);
        txn.mutations.extend([
            TxnMutation::Put(MetaEntity::DfsInode(updated)),
            TxnMutation::RecordRequestOutcome(outcome),
        ]);
        inode_outcome(
            self.store.compare_and_commit(txn).await?,
            StoreOperation::DfsSyncInodeMetadata,
        )
    }

    pub async fn commit_file_version(
        &self,
        caller_id: String,
        commit: CommitFileVersion,
    ) -> Result<InodeRecord> {
        require_id(&caller_id, "caller_id")?;
        require_id(&commit.operation_id.0, "operation_id")?;
        require_id(&commit.inode_id.0, "inode_id")?;
        require_id(
            &commit.write_lease.owner_node_id,
            "write_lease.owner_node_id",
        )?;
        require_id(
            &commit.write_lease.owner_session_id,
            "write_lease.owner_session_id",
        )?;
        require_id(&commit.file_version.id.0, "file_version.id")?;
        require_id(&commit.layout_root.id.0, "layout_root.id")?;
        if let Some(inode) = self
            .replayed_inode(
                &caller_id,
                &commit.operation_id,
                StoreOperation::DfsCommitFileVersion,
            )
            .await?
        {
            return Ok(inode);
        }
        let current = self.get_inode(commit.inode_id.clone()).await?;
        if commit.write_lease.inode_id != commit.inode_id
            || commit.write_lease.owner_node_id != caller_id
        {
            return Err(conflict(
                "DFS commit lease does not belong to caller and inode",
            ));
        }
        let current_lease = self
            .validate_write_lease(&caller_id, &commit.inode_id, &commit.write_lease)
            .await?;
        if current.revision != commit.expected_inode_revision
            || current.head_version != commit.expected_head_version
        {
            return Err(conflict("DFS inode head changed before commit"));
        }
        if commit.file_version.inode_id != commit.inode_id
            || commit.file_version.layout_root != commit.layout_root.id
            || commit.file_version.length != commit.layout_root.file_length
        {
            return Err(invalid(
                "FileVersion and LayoutRoot do not describe one file",
            ));
        }
        for receipt in &commit.chunk_receipts {
            if receipt.operation_id != commit.operation_id
                || receipt.chunk.id != receipt.copy.chunk_id
                || receipt.chunk.length != receipt.copy.persisted_bytes
                || receipt.chunk.content_digest != receipt.copy.verified_digest
                || receipt.copy.state != CopyState::Durable
                || receipt.copy.node_id != caller_id
            {
                return Err(invalid("ChunkReceipt does not prove the referenced copy"));
            }
        }
        let mut previous_end = 0;
        for extent in &commit.layout_root.inline_extents {
            let file_end = extent
                .file_offset
                .checked_add(extent.length)
                .ok_or_else(|| invalid("DFS extent file range overflows"))?;
            if extent.length == 0
                || extent.file_offset < previous_end
                || file_end > commit.file_version.length
            {
                return Err(invalid(
                    "LayoutRoot extents must be non-empty, ordered and non-overlapping",
                ));
            }
            let receipt = commit
                .chunk_receipts
                .iter()
                .find(|receipt| receipt.chunk.id == extent.chunk_id)
                .ok_or_else(|| {
                    invalid("LayoutRoot references a Chunk without a durable receipt")
                })?;
            let chunk_end = extent
                .chunk_offset
                .checked_add(extent.length)
                .ok_or_else(|| invalid("DFS extent chunk range overflows"))?;
            if chunk_end > receipt.chunk.length {
                return Err(invalid("DFS extent exceeds its referenced Chunk"));
            }
            previous_end = file_end;
        }
        let mut updated = current.clone();
        updated.head_version = Some(commit.file_version.id.clone());
        updated.revision = updated.revision.saturating_add(1);
        match commit.metadata_delta.mode {
            CommitMetadataMode::DataOnly => {}
            CommitMetadataMode::Full => {
                updated.attributes.mtime_unix_ms = commit
                    .metadata_delta
                    .mtime_unix_ms
                    .ok_or_else(|| invalid("DFS full commit requires mtime"))?;
                updated.attributes.ctime_unix_ms = commit
                    .metadata_delta
                    .ctime_unix_ms
                    .ok_or_else(|| invalid("DFS full commit requires ctime"))?;
            }
        }

        let request = RequestKey::new(caller_id, commit.operation_id.0.clone());
        let outcome = RequestOutcome {
            request: request.clone(),
            operation: StoreOperation::DfsCommitFileVersion,
            result: OperationResult::DfsInode(updated.clone()),
        };
        let mut txn = MetaTxn::new(request.clone(), StoreOperation::DfsCommitFileVersion);
        txn.conditions.extend([
            TxnCondition::RequestAbsent(request),
            TxnCondition::EntityEquals(MetaEntity::DfsInode(current)),
            TxnCondition::EntityEquals(MetaEntity::DfsWriteLease(current_lease)),
            TxnCondition::Missing(MetaKey::DfsFileVersion(commit.file_version.id.clone())),
            TxnCondition::Missing(MetaKey::DfsLayoutRoot(commit.layout_root.id.clone())),
        ]);
        for receipt in &commit.chunk_receipts {
            txn.mutations.push(TxnMutation::Put(MetaEntity::DfsChunk(
                receipt.chunk.clone(),
            )));
            txn.mutations
                .push(TxnMutation::Put(MetaEntity::DfsCopy(receipt.copy.clone())));
            txn.mutations
                .push(TxnMutation::Put(MetaEntity::DfsPlacement(
                    PlacementRecord {
                        chunk_id: receipt.chunk.id.clone(),
                        policy_id: "local-r1".into(),
                        replica_group_id: format!("r1:{}", receipt.copy.node_id),
                        epoch: 1,
                        copies: vec![receipt.copy.id.clone()],
                    },
                )));
        }
        txn.mutations.extend([
            TxnMutation::Put(MetaEntity::DfsLayoutRoot(commit.layout_root)),
            TxnMutation::Put(MetaEntity::DfsFileVersion(commit.file_version)),
            TxnMutation::Put(MetaEntity::DfsInode(updated)),
            TxnMutation::RecordRequestOutcome(outcome),
        ]);
        inode_outcome(
            self.store.compare_and_commit(txn).await?,
            StoreOperation::DfsCommitFileVersion,
        )
    }

    async fn validate_write_lease(
        &self,
        caller_id: &str,
        inode_id: &InodeId,
        presented: &WriteLease,
    ) -> Result<WriteLease> {
        if presented.inode_id != *inode_id || presented.owner_node_id != caller_id {
            return Err(conflict(
                "DFS write lease does not belong to caller and inode",
            ));
        }
        let lease_snapshot = self
            .store
            .read(MetaRead::DfsWriteLease(inode_id.clone()))
            .await?;
        let current = match lease_snapshot.entity {
            Some(MetaEntity::DfsWriteLease(lease)) => lease,
            _ => return Err(conflict("DFS operation requires a current write lease")),
        };
        if current.inode_id != presented.inode_id
            || current.owner_node_id != presented.owner_node_id
            || current.owner_session_id != presented.owner_session_id
            || current.lease_epoch != presented.lease_epoch
            || current.expires_at_unix_ms <= now_unix_ms()
        {
            return Err(conflict("DFS write lease changed or expired"));
        }
        Ok(current)
    }

    async fn ensure_parent_directory(
        &self,
        namespace_id: &NamespaceId,
        parent: &InodeId,
    ) -> Result<()> {
        let inode = if parent.0 == "1" {
            root_inode(namespace_id.clone())
        } else {
            self.get_inode(parent.clone()).await?
        };
        if inode.kind != InodeKind::Directory || inode.namespace_id != *namespace_id {
            return Err(invalid("DFS parent is not a directory in this namespace"));
        }
        Ok(())
    }

    async fn replayed_inode(
        &self,
        caller_id: &str,
        operation_id: &OperationId,
        operation: StoreOperation,
    ) -> Result<Option<InodeRecord>> {
        let key = RequestKey::new(caller_id, operation_id.0.clone());
        let snapshot = self.store.read(MetaRead::RequestOutcome(key)).await?;
        let Some(outcome) = snapshot.request_outcome else {
            return Ok(None);
        };
        if outcome.operation != operation {
            return Err(invalid(
                "DFS operation_id was already used for another operation",
            ));
        }
        match outcome.result {
            OperationResult::DfsInode(inode) => Ok(Some(inode)),
            _ => Err(invalid("DFS operation replay returned the wrong result")),
        }
    }

    async fn replayed_lease(
        &self,
        caller_id: &str,
        operation_id: &OperationId,
        operation: StoreOperation,
    ) -> Result<Option<WriteLease>> {
        let key = RequestKey::new(caller_id, operation_id.0.clone());
        let snapshot = self.store.read(MetaRead::RequestOutcome(key)).await?;
        let Some(outcome) = snapshot.request_outcome else {
            return Ok(None);
        };
        if outcome.operation != operation {
            return Err(invalid(
                "DFS operation_id was already used for another operation",
            ));
        }
        match outcome.result {
            OperationResult::DfsWriteLease(lease) => Ok(Some(lease)),
            _ => Err(invalid("DFS operation replay returned the wrong result")),
        }
    }

    async fn replayed_inode_and_lease(
        &self,
        caller_id: &str,
        operation_id: &OperationId,
        operation: StoreOperation,
    ) -> Result<Option<(InodeRecord, WriteLease)>> {
        let key = RequestKey::new(caller_id, operation_id.0.clone());
        let snapshot = self.store.read(MetaRead::RequestOutcome(key)).await?;
        let Some(outcome) = snapshot.request_outcome else {
            return Ok(None);
        };
        if outcome.operation != operation {
            return Err(invalid(
                "DFS operation_id was already used for another operation",
            ));
        }
        match outcome.result {
            OperationResult::DfsInodeWithLease { inode, lease } => Ok(Some((inode, lease))),
            _ => Err(invalid("DFS operation replay returned the wrong result")),
        }
    }
}

fn root_inode(namespace_id: NamespaceId) -> InodeRecord {
    InodeRecord {
        namespace_id,
        inode_id: InodeId::new("1"),
        kind: InodeKind::Directory,
        attributes: InodeAttributes {
            mode: 0o755,
            uid: 0,
            gid: 0,
            nlink: 2,
            atime_unix_ms: 0,
            mtime_unix_ms: 0,
            ctime_unix_ms: 0,
        },
        head_version: None,
        revision: 1,
    }
}

fn inode_outcome(outcome: TxnOutcome, operation: StoreOperation) -> Result<InodeRecord> {
    let stored = match outcome {
        TxnOutcome::Committed { outcome, .. }
        | TxnOutcome::ConditionFailed {
            existing_outcome: Some(outcome),
            ..
        } if outcome.operation == operation => outcome,
        TxnOutcome::ConditionFailed { .. } => {
            return Err(conflict("DFS metadata condition changed during commit"));
        }
        _ => {
            return Err(invalid(
                "DFS metadata operation returned an invalid outcome",
            ));
        }
    };
    match stored.result {
        OperationResult::DfsInode(inode) => Ok(inode),
        _ => Err(invalid("DFS metadata operation replayed the wrong result")),
    }
}

fn lease_outcome(outcome: TxnOutcome, operation: StoreOperation) -> Result<WriteLease> {
    let stored = match outcome {
        TxnOutcome::Committed { outcome, .. }
        | TxnOutcome::ConditionFailed {
            existing_outcome: Some(outcome),
            ..
        } if outcome.operation == operation => outcome,
        TxnOutcome::ConditionFailed { .. } => {
            return Err(conflict(
                "DFS metadata condition changed during lease update",
            ));
        }
        _ => {
            return Err(invalid(
                "DFS metadata operation returned an invalid outcome",
            ));
        }
    };
    match stored.result {
        OperationResult::DfsWriteLease(lease) => Ok(lease),
        _ => Err(invalid("DFS metadata operation replayed the wrong result")),
    }
}

fn inode_and_lease_outcome(
    outcome: TxnOutcome,
    operation: StoreOperation,
) -> Result<(InodeRecord, WriteLease)> {
    let stored = match outcome {
        TxnOutcome::Committed { outcome, .. }
        | TxnOutcome::ConditionFailed {
            existing_outcome: Some(outcome),
            ..
        } if outcome.operation == operation => outcome,
        TxnOutcome::ConditionFailed { .. } => {
            return Err(conflict("DFS metadata condition changed during create"));
        }
        _ => {
            return Err(invalid(
                "DFS metadata operation returned an invalid outcome",
            ));
        }
    };
    match stored.result {
        OperationResult::DfsInodeWithLease { inode, lease } => Ok((inode, lease)),
        _ => Err(invalid("DFS metadata operation replayed the wrong result")),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn lease_expiry(lease_seconds: u64) -> Result<u64> {
    if lease_seconds == 0 {
        return Err(invalid("lease_seconds must be greater than zero"));
    }
    Ok(now_unix_ms().saturating_add(lease_seconds.saturating_mul(1000)))
}

fn invalid(message: impl Into<String>) -> Error {
    Error::coded(afs_error::META_CATALOG_INVALID_REQUEST, message)
}

fn require_id(value: &str, field: &str) -> Result<()> {
    if value.is_empty() {
        Err(invalid(format!("{field} is required")))
    } else {
        Ok(())
    }
}

fn conflict(message: impl Into<String>) -> Error {
    Error::coded(afs_error::IO_UNAVAILABLE, message)
}

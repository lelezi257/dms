//! DistributedFs namespace and FileVersion authority on the shared MetaStore.

use std::sync::Arc;

use afs_error::{Error, Result};

use crate::dfs::{
    CommitFileVersion, CopyState, Dentry, DentryKey, FileVersion, FileVersionId, InodeAttributes,
    InodeId, InodeKind, InodeRecord, LayoutRoot, NamespaceId, OperationId, PlacementRecord,
};

use super::store::{
    MetaEntity, MetaKey, MetaRead, MetaStore, MetaTxn, OperationResult, RequestKey, RequestOutcome,
    StoreOperation, TxnCondition, TxnMutation, TxnOutcome,
};

#[derive(Clone)]
pub struct DfsService {
    store: Arc<dyn MetaStore>,
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

    pub async fn create(
        &self,
        caller_id: String,
        operation_id: OperationId,
        namespace_id: NamespaceId,
        parent_inode_id: InodeId,
        name: Vec<u8>,
        attributes: InodeAttributes,
    ) -> Result<InodeRecord> {
        require_id(&caller_id, "caller_id")?;
        require_id(&operation_id.0, "operation_id")?;
        require_id(&namespace_id.0, "namespace_id")?;
        require_id(&parent_inode_id.0, "parent_inode_id")?;
        if let Some(inode) = self
            .replayed_inode(&caller_id, &operation_id, StoreOperation::DfsCreate)
            .await?
        {
            return Ok(inode);
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
        let request = RequestKey::new(caller_id, operation_id.0.clone());
        let outcome = RequestOutcome {
            request: request.clone(),
            operation: StoreOperation::DfsCreate,
            result: OperationResult::DfsInode(inode.clone()),
        };
        let mut txn = MetaTxn::new(request.clone(), StoreOperation::DfsCreate);
        txn.conditions.extend([
            TxnCondition::RequestAbsent(request),
            TxnCondition::Missing(MetaKey::DfsDentry(dentry.key.clone())),
            TxnCondition::Missing(MetaKey::DfsInode(inode.inode_id.clone())),
        ]);
        txn.mutations.extend([
            TxnMutation::Put(MetaEntity::DfsDentry(dentry)),
            TxnMutation::Put(MetaEntity::DfsInode(inode.clone())),
            TxnMutation::RecordRequestOutcome(outcome),
        ]);
        inode_outcome(
            self.store.compare_and_commit(txn).await?,
            StoreOperation::DfsCreate,
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

    pub async fn commit_file_version(
        &self,
        caller_id: String,
        commit: CommitFileVersion,
    ) -> Result<InodeRecord> {
        require_id(&caller_id, "caller_id")?;
        require_id(&commit.operation_id.0, "operation_id")?;
        require_id(&commit.inode_id.0, "inode_id")?;
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
        updated.attributes.mtime_unix_ms = commit.file_version.created_at_unix_ms;
        updated.attributes.ctime_unix_ms = commit.file_version.created_at_unix_ms;

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

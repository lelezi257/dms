//! DFS replication coordination above the local immutable chunk store.
//!
//! The filesystem-wide `ReplicationConfig` is immutable after initialization.
//! Placement can still change as nodes and devices change; Nodes cache that
//! authority snapshot and derive one-operation `ReplicationPlan` values.

use std::{collections::HashSet, sync::Arc};

use afs_error::{Error, Result};

use crate::{
    dfs::{
        ChunkId, ChunkReceipt, ContentDigest, LocalCopyPolicy, PlacementSnapshot, ReplicaAck,
        ReplicaGroupId, ReplicaTarget, ReplicationConfig,
    },
    node::chunk::{ChunkStore, LocalChunkStore, StagedChunk},
};

pub trait PlacementProvider: Send + Sync {
    fn snapshot(&self) -> Result<Arc<PlacementSnapshot>>;
    fn refresh(&self, minimum_revision: u64) -> Result<Arc<PlacementSnapshot>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplicaTransferMode {
    GrpcStream,
    RdmaOneSided,
}

/// Transport adapter used only for remote replica bytes.
///
/// gRPC can implement this with request frames. RDMA implements the same
/// command with a negotiated memory descriptor and one-sided transfer; the
/// replication state machine never observes either transport's wire details.
pub trait ReplicaDataPlane: Send + Sync {
    fn mode(&self) -> ReplicaTransferMode;

    /// Checks transport/session readiness before any local or remote replica
    /// side effect. The framework adapters currently fail here for RN.
    fn prepare(&self, plan: &ReplicationPlan) -> Result<()>;

    fn put_remote_replicas(
        &self,
        plan: &ReplicationPlan,
        staged: &StagedChunk,
    ) -> Result<Vec<ReplicaAck>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplicationPlan {
    pub chunk_id: ChunkId,
    pub placement_revision: u64,
    pub placement_epoch: u64,
    pub replica_group_id: ReplicaGroupId,
    pub config: ReplicationConfig,
    pub ordered_targets: Vec<ReplicaTarget>,
}

impl ReplicationPlan {
    fn derive(staged: &StagedChunk, snapshot: &PlacementSnapshot) -> Result<Self> {
        if !snapshot.replication.is_valid() {
            return Err(invalid("Meta returned an invalid DFS replication config"));
        }
        let group = snapshot
            .replica_groups
            .first()
            .ok_or_else(|| invalid("placement snapshot contains no replica group"))?;
        if group.targets.len() < usize::from(snapshot.replication.sync_required_copies) {
            return Err(Error::coded(
                afs_error::NODE_VFS_UNIMPLEMENTED,
                "placement has fewer targets than the configured synchronous copy count",
            ));
        }
        Ok(Self {
            chunk_id: staged.chunk.id.clone(),
            placement_revision: snapshot.revision,
            placement_epoch: group.placement_epoch,
            replica_group_id: group.id.clone(),
            config: snapshot.replication.clone(),
            ordered_targets: group.targets.clone(),
        })
    }
}

pub struct ReplicationEngine {
    local_node_id: String,
    local: Arc<LocalChunkStore>,
    placement: Arc<dyn PlacementProvider>,
    data_plane: Arc<dyn ReplicaDataPlane>,
}

impl ReplicationEngine {
    pub fn new(
        local_node_id: String,
        local: Arc<LocalChunkStore>,
        placement: Arc<dyn PlacementProvider>,
        data_plane: Arc<dyn ReplicaDataPlane>,
    ) -> Self {
        Self {
            local_node_id,
            local,
            placement,
            data_plane,
        }
    }

    fn put(&self, staged: StagedChunk) -> Result<ChunkReceipt> {
        let snapshot = self.placement.snapshot()?;
        let plan = ReplicationPlan::derive(&staged, &snapshot)?;

        // RN is deliberately fail-fast in the framework stage. No local Chunk
        // is finalized until the selected transport confirms it can execute
        // the complete plan.
        self.data_plane.prepare(&plan)?;

        let mut durable_acks = Vec::new();
        if let Some(local_target) = plan
            .ordered_targets
            .iter()
            .find(|target| target.node_id == self.local_node_id)
        {
            durable_acks.push(self.local.persist(
                &staged,
                local_target,
                plan.placement_revision,
                plan.placement_epoch,
            )?);
        } else if plan.config.local_copy == LocalCopyPolicy::Required {
            return Err(invalid(
                "replication plan requires a local copy but has no local target",
            ));
        }
        durable_acks.extend(self.data_plane.put_remote_replicas(&plan, &staged)?);
        validate_acks(&plan, &staged, &durable_acks)?;
        Ok(ChunkReceipt {
            operation_id: staged.operation_id,
            chunk: staged.chunk,
            placement_revision: plan.placement_revision,
            placement_epoch: plan.placement_epoch,
            replica_group_id: plan.replica_group_id,
            durable_acks,
        })
    }
}

pub struct DfsChunkStore {
    local_node_id: String,
    local: Arc<LocalChunkStore>,
    placement: Arc<dyn PlacementProvider>,
    replication: ReplicationEngine,
}

impl DfsChunkStore {
    pub fn new(
        local_node_id: String,
        local: Arc<LocalChunkStore>,
        placement: Arc<dyn PlacementProvider>,
        data_plane: Arc<dyn ReplicaDataPlane>,
    ) -> Self {
        Self {
            local_node_id: local_node_id.clone(),
            local: local.clone(),
            placement: placement.clone(),
            replication: ReplicationEngine::new(local_node_id, local, placement, data_plane),
        }
    }

    fn put_local(&self, staged: StagedChunk, snapshot: &PlacementSnapshot) -> Result<ChunkReceipt> {
        let group = snapshot
            .replica_groups
            .first()
            .ok_or_else(|| invalid("placement snapshot contains no replica group"))?;
        let target = group
            .targets
            .iter()
            .find(|target| target.node_id == self.local_node_id)
            .ok_or_else(|| invalid("local R1 placement does not contain this node"))?;
        let ack = self
            .local
            .persist(&staged, target, snapshot.revision, group.placement_epoch)?;
        Ok(ChunkReceipt {
            operation_id: staged.operation_id,
            chunk: staged.chunk,
            placement_revision: snapshot.revision,
            placement_epoch: group.placement_epoch,
            replica_group_id: group.id.clone(),
            durable_acks: vec![ack],
        })
    }
}

impl ChunkStore for DfsChunkStore {
    fn put(&self, staged: StagedChunk) -> Result<ChunkReceipt> {
        let snapshot = self.placement.snapshot()?;
        if snapshot.replication.is_local_fast_path() {
            self.put_local(staged, &snapshot)
        } else {
            self.replication.put(staged)
        }
    }

    fn read_at(&self, chunk_id: &ChunkId, offset: u64, out: &mut [u8]) -> Result<usize> {
        self.local.read_at(chunk_id, offset, out)
    }

    fn read_all(&self, chunk_id: &ChunkId) -> Result<Vec<u8>> {
        self.local.read_all(chunk_id)
    }

    fn verify(&self, chunk_id: &ChunkId, expected: &ContentDigest) -> Result<()> {
        self.local.verify(chunk_id, expected)
    }
}

fn validate_acks(
    plan: &ReplicationPlan,
    staged: &StagedChunk,
    durable_acks: &[ReplicaAck],
) -> Result<()> {
    let mut acknowledged_targets = HashSet::new();
    for ack in durable_acks {
        if ack.operation_id != staged.operation_id
            || ack.chunk_id != staged.chunk.id
            || ack.placement_revision != plan.placement_revision
            || ack.placement_epoch != plan.placement_epoch
            || ack.persisted_bytes != staged.chunk.length
            || ack.verified_digest != staged.chunk.content_digest
        {
            return Err(invalid(
                "replica acknowledgement does not prove the staged Chunk",
            ));
        }
        let assigned = plan.ordered_targets.iter().any(|target| {
            target.node_id == ack.node_id
                && target.node_epoch == ack.node_epoch
                && target.device.device_id == ack.device_id
                && target.device.device_epoch == ack.device_epoch
                && target.device.catalog_revision == ack.catalog_revision
        });
        if !assigned {
            return Err(invalid(
                "replica acknowledgement is not part of the ReplicationPlan",
            ));
        }
        if !acknowledged_targets.insert((ack.node_id.clone(), ack.device_id.clone())) {
            return Err(invalid(
                "replica acknowledgements contain a duplicate target",
            ));
        }
    }
    if acknowledged_targets.len() < usize::from(plan.config.sync_required_copies) {
        return Err(invalid(
            "replica acknowledgements do not satisfy the synchronous copy count",
        ));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_STORAGE_INVALID, message)
}

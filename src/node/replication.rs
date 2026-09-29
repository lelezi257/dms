//! DFS replication coordination above the local immutable chunk store.
//!
//! The filesystem-wide `ReplicationConfig` is immutable after initialization.
//! Placement can still change as nodes and devices change; Nodes cache that
//! authority snapshot and derive one-operation `ReplicationPlan` values.

use std::{collections::HashSet, sync::Arc};

use afs_error::{Error, Result};

use crate::{
    dfs::{
        ChunkId, ChunkReceipt, LocalCopyPolicy, PlacementSnapshot, ReplicaAck, ReplicaGroupId,
        ReplicaTarget, ReplicationConfig,
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

    fn put_batch(&self, staged: Vec<StagedChunk>) -> Result<Vec<ChunkReceipt>> {
        if staged.is_empty() {
            return Ok(Vec::new());
        }
        let snapshot = self.placement.snapshot()?;
        let plans = staged
            .iter()
            .map(|item| ReplicationPlan::derive(item, &snapshot))
            .collect::<Result<Vec<_>>>()?;

        // RN is deliberately fail-fast in the framework stage. No local Chunk
        // is finalized until the selected transport confirms it can execute
        // the complete plan.
        for plan in &plans {
            self.data_plane.prepare(plan)?;
        }
        let first_plan = &plans[0];
        let local_target = first_plan
            .ordered_targets
            .iter()
            .find(|target| target.node_id == self.local_node_id)
            .cloned();
        if local_target.is_none() && first_plan.config.local_copy == LocalCopyPolicy::Required {
            return Err(invalid(
                "replication plan requires a local copy but has no local target",
            ));
        }
        if plans.iter().any(|plan| {
            plan.placement_revision != first_plan.placement_revision
                || plan.placement_epoch != first_plan.placement_epoch
                || plan.replica_group_id != first_plan.replica_group_id
                || plan.ordered_targets != first_plan.ordered_targets
        }) {
            return Err(invalid("one Chunk batch must use one ReplicationPlan"));
        }
        let local_acks = match local_target.as_ref() {
            Some(target) => self.local.persist_batch(
                &staged,
                target,
                first_plan.placement_revision,
                first_plan.placement_epoch,
            )?,
            None => Vec::new(),
        };
        let mut receipts = Vec::with_capacity(staged.len());
        for ((item, plan), local_ack) in staged.into_iter().zip(plans).zip(
            local_acks
                .into_iter()
                .map(Some)
                .chain(std::iter::repeat(None)),
        ) {
            let mut durable_acks = local_ack.into_iter().collect::<Vec<_>>();
            durable_acks.extend(self.data_plane.put_remote_replicas(&plan, &item)?);
            validate_acks(&plan, &item, &durable_acks)?;
            receipts.push(ChunkReceipt {
                operation_id: item.operation_id,
                chunk: item.chunk,
                placement_revision: plan.placement_revision,
                placement_epoch: plan.placement_epoch,
                replica_group_id: plan.replica_group_id,
                durable_acks,
            });
        }
        Ok(receipts)
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

    fn put_local_batch(
        &self,
        staged: Vec<StagedChunk>,
        snapshot: &PlacementSnapshot,
    ) -> Result<Vec<ChunkReceipt>> {
        let group = snapshot
            .replica_groups
            .first()
            .ok_or_else(|| invalid("placement snapshot contains no replica group"))?;
        let target = group
            .targets
            .iter()
            .find(|target| target.node_id == self.local_node_id)
            .ok_or_else(|| invalid("local R1 placement does not contain this node"))?;
        let acks =
            self.local
                .persist_batch(&staged, target, snapshot.revision, group.placement_epoch)?;
        Ok(staged
            .into_iter()
            .zip(acks)
            .map(|(item, ack)| ChunkReceipt {
                operation_id: item.operation_id,
                chunk: item.chunk,
                placement_revision: snapshot.revision,
                placement_epoch: group.placement_epoch,
                replica_group_id: group.id.clone(),
                durable_acks: vec![ack],
            })
            .collect())
    }
}

impl ChunkStore for DfsChunkStore {
    fn put_batch(&self, staged: Vec<StagedChunk>) -> Result<Vec<ChunkReceipt>> {
        let snapshot = self.placement.snapshot()?;
        if snapshot.replication.is_local_fast_path() {
            self.put_local_batch(staged, &snapshot)
        } else {
            self.replication.put_batch(staged)
        }
    }

    fn read_at(&self, chunk_id: &ChunkId, offset: u64, out: &mut [u8]) -> Result<usize> {
        self.local.read_at(chunk_id, offset, out)
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
                && ack.catalog_revision >= target.device.catalog_revision
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

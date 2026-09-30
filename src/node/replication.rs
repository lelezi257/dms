//! DFS replication coordination above the local immutable chunk store.
//!
//! The filesystem-wide `ReplicationConfig` is immutable after initialization.
//! Placement can still change as nodes and devices change; Nodes cache that
//! authority snapshot and derive one-operation `ReplicationPlan` values.

use std::{collections::HashSet, sync::Arc};

use afs_error::{Error, Result};

use crate::{
    dfs::{
        ChunkId, ChunkReceipt, PlacementSnapshot, ReplicaAck, ReplicaGroup, ReplicaGroupId,
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
    fn prepare_peer(&self, op: &ReplicaPeerOp) -> Result<()>;

    fn put_peer_replica(&self, op: &ReplicaPeerOp, staged: &StagedChunk)
    -> Result<Vec<ReplicaAck>>;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplicaPeerOp {
    pub chunk_id: ChunkId,
    pub placement_revision: u64,
    pub placement_epoch: u64,
    pub replica_group_id: ReplicaGroupId,
    pub target_index: usize,
    pub target: ReplicaTarget,
    pub chain_tail: Vec<ReplicaTarget>,
}

impl ReplicationPlan {
    fn derive(staged: &StagedChunk, snapshot: &PlacementSnapshot) -> Result<Self> {
        if !snapshot.replication.is_valid() {
            return Err(invalid("Meta returned an invalid DFS replication config"));
        }
        let group = select_replica_group(&staged.chunk.id, snapshot)
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

    fn local_target(&self, local_node_id: &str) -> Option<&ReplicaTarget> {
        self.ordered_targets
            .iter()
            .find(|target| target.node_id == local_node_id)
    }

    fn selected_peer_op(&self, local_node_id: &str) -> Option<ReplicaPeerOp> {
        if self
            .ordered_targets
            .first()
            .is_none_or(|target| target.node_id != local_node_id)
        {
            return None;
        }
        let target_index = 1;
        let target = self.ordered_targets.get(target_index)?;
        Some(ReplicaPeerOp {
            chunk_id: self.chunk_id.clone(),
            placement_revision: self.placement_revision,
            placement_epoch: self.placement_epoch,
            replica_group_id: self.replica_group_id.clone(),
            target_index,
            target: target.clone(),
            chain_tail: self.ordered_targets[target_index.saturating_add(1)..].to_vec(),
        })
    }

    fn shares_batch_group(&self, other: &Self) -> bool {
        self.placement_revision == other.placement_revision
            && self.placement_epoch == other.placement_epoch
            && self.replica_group_id == other.replica_group_id
            && self.config == other.config
            && self.ordered_targets == other.ordered_targets
    }
}

#[derive(Debug)]
struct ReplicationPlanGroup {
    plan: ReplicationPlan,
    indexes: Vec<usize>,
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
        let groups = group_plans(&plans);

        // RN is deliberately fail-fast in the framework stage. No local Chunk
        // is finalized until the selected transport confirms it can execute
        // every selected peer operation.
        for (plan, op) in plans.iter().zip(
            plans
                .iter()
                .map(|plan| plan.selected_peer_op(&self.local_node_id)),
        ) {
            require_local_chain_head(plan, &self.local_node_id)?;
            if let Some(op) = op {
                self.data_plane.prepare_peer(&op)?;
            }
        }

        let mut receipts = vec![None; staged.len()];
        for group in groups {
            let group_staged = group
                .indexes
                .iter()
                .map(|index| staged[*index].clone())
                .collect::<Vec<_>>();
            let local_acks = match group.plan.local_target(&self.local_node_id) {
                Some(target) => self.local.persist_batch(
                    &group_staged,
                    target,
                    group.plan.placement_revision,
                    group.plan.placement_epoch,
                )?,
                None => Vec::new(),
            };
            for ((index, item), local_ack) in group.indexes.iter().copied().zip(group_staged).zip(
                local_acks
                    .into_iter()
                    .map(Some)
                    .chain(std::iter::repeat(None)),
            ) {
                let plan = &plans[index];
                let mut durable_acks = local_ack.into_iter().collect::<Vec<_>>();
                if let Some(op) = plan.selected_peer_op(&self.local_node_id) {
                    durable_acks.extend(self.data_plane.put_peer_replica(&op, &item)?);
                }
                validate_acks(plan, &item, &durable_acks)?;
                receipts[index] = Some(ChunkReceipt {
                    operation_id: item.operation_id,
                    chunk: item.chunk,
                    placement_revision: plan.placement_revision,
                    placement_epoch: plan.placement_epoch,
                    replica_group_id: plan.replica_group_id.clone(),
                    durable_acks,
                });
            }
        }
        receipts
            .into_iter()
            .map(|receipt| receipt.ok_or_else(|| invalid("replication receipt was not produced")))
            .collect()
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
        if staged.is_empty() {
            return Ok(Vec::new());
        }
        let plans = staged
            .iter()
            .map(|item| ReplicationPlan::derive(item, snapshot))
            .collect::<Result<Vec<_>>>()?;
        let mut receipts = vec![None; staged.len()];
        for group in group_plans(&plans) {
            let target = group
                .plan
                .local_target(&self.local_node_id)
                .ok_or_else(|| invalid("local R1 placement does not contain this node"))?;
            let group_staged = group
                .indexes
                .iter()
                .map(|index| staged[*index].clone())
                .collect::<Vec<_>>();
            let local_acks = self.local.persist_batch(
                &group_staged,
                target,
                group.plan.placement_revision,
                group.plan.placement_epoch,
            )?;
            for ((index, item), ack) in group
                .indexes
                .iter()
                .copied()
                .zip(group_staged)
                .zip(local_acks)
            {
                receipts[index] = Some(ChunkReceipt {
                    operation_id: item.operation_id,
                    chunk: item.chunk,
                    placement_revision: group.plan.placement_revision,
                    placement_epoch: group.plan.placement_epoch,
                    replica_group_id: group.plan.replica_group_id.clone(),
                    durable_acks: vec![ack],
                });
            }
        }
        receipts
            .into_iter()
            .map(|receipt| receipt.ok_or_else(|| invalid("local receipt was not produced")))
            .collect()
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

fn group_plans(plans: &[ReplicationPlan]) -> Vec<ReplicationPlanGroup> {
    let mut groups: Vec<ReplicationPlanGroup> = Vec::new();
    for (index, plan) in plans.iter().enumerate() {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.plan.shares_batch_group(plan))
        {
            group.indexes.push(index);
            continue;
        }
        groups.push(ReplicationPlanGroup {
            plan: plan.clone(),
            indexes: vec![index],
        });
    }
    groups
}

fn require_local_chain_head(plan: &ReplicationPlan, local_node_id: &str) -> Result<()> {
    if plan
        .ordered_targets
        .first()
        .is_some_and(|target| target.node_id == local_node_id)
    {
        return Ok(());
    }
    Err(Error::coded(
        afs_error::NODE_VFS_UNIMPLEMENTED,
        "DFS RN writes must start from the local chain head",
    ))
}

fn select_replica_group<'a>(
    chunk_id: &ChunkId,
    snapshot: &'a PlacementSnapshot,
) -> Option<&'a ReplicaGroup> {
    if snapshot.replica_groups.is_empty() {
        return None;
    }
    let index = stable_group_index(chunk_id, snapshot.replica_groups.len());
    snapshot.replica_groups.get(index)
}

fn stable_group_index(chunk_id: &ChunkId, group_count: usize) -> usize {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in chunk_id.0.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash as usize) % group_count
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::dfs::{LocalCopyPolicy, OperationId, StorageDeviceDescriptor};
    use crate::node::chunk::StagedChunk;

    struct StaticPlacement {
        snapshot: Arc<PlacementSnapshot>,
    }

    impl PlacementProvider for StaticPlacement {
        fn snapshot(&self) -> Result<Arc<PlacementSnapshot>> {
            Ok(self.snapshot.clone())
        }

        fn refresh(&self, _: u64) -> Result<Arc<PlacementSnapshot>> {
            Ok(self.snapshot.clone())
        }
    }

    #[derive(Default)]
    struct FakeDataPlane {
        prepared: Mutex<Vec<(ChunkId, String)>>,
    }

    impl ReplicaDataPlane for FakeDataPlane {
        fn mode(&self) -> ReplicaTransferMode {
            ReplicaTransferMode::GrpcStream
        }

        fn prepare_peer(&self, op: &ReplicaPeerOp) -> Result<()> {
            self.prepared
                .lock()
                .unwrap()
                .push((op.chunk_id.clone(), op.target.node_id.clone()));
            Ok(())
        }

        fn put_peer_replica(
            &self,
            op: &ReplicaPeerOp,
            staged: &StagedChunk,
        ) -> Result<Vec<ReplicaAck>> {
            let mut acks = vec![ack_for(&op.target, op, staged)];
            acks.extend(
                op.chain_tail
                    .iter()
                    .map(|target| ack_for(target, op, staged)),
            );
            Ok(acks)
        }
    }

    fn ack_for(target: &ReplicaTarget, op: &ReplicaPeerOp, staged: &StagedChunk) -> ReplicaAck {
        ReplicaAck {
            operation_id: staged.operation_id.clone(),
            chunk_id: staged.chunk.id.clone(),
            placement_revision: op.placement_revision,
            placement_epoch: op.placement_epoch,
            node_id: target.node_id.clone(),
            node_epoch: target.node_epoch,
            device_id: target.device.device_id.clone(),
            device_epoch: target.device.device_epoch,
            catalog_revision: target.device.catalog_revision,
            persisted_bytes: staged.chunk.length,
            verified_digest: staged.chunk.content_digest.clone(),
        }
    }

    fn target(node_id: &str, device: StorageDeviceDescriptor) -> ReplicaTarget {
        ReplicaTarget {
            node_id: node_id.into(),
            node_epoch: 1,
            data_endpoint: format!("http://{node_id}"),
            device,
        }
    }

    fn remote_device(device_id: &str) -> StorageDeviceDescriptor {
        StorageDeviceDescriptor {
            device_id: device_id.into(),
            device_epoch: 1,
            catalog_revision: 0,
            failure_domain: device_id.into(),
        }
    }

    fn staged(payload: &str) -> StagedChunk {
        StagedChunk::new(
            OperationId::new(format!("op-{payload}")),
            payload.as_bytes().to_vec(),
        )
    }

    fn staged_for_group(snapshot: &PlacementSnapshot, group_index: usize) -> StagedChunk {
        for attempt in 0..10_000 {
            let item = staged(&format!("group-{group_index}-{attempt}"));
            if stable_group_index(&item.chunk.id, snapshot.replica_groups.len()) == group_index {
                return item;
            }
        }
        panic!("could not find a staged Chunk for replica group {group_index}");
    }

    fn config(copies: u16) -> ReplicationConfig {
        ReplicationConfig {
            desired_copies: copies,
            sync_required_copies: copies,
            min_distinct_nodes: copies,
            min_distinct_failure_domains: 1,
            local_copy: LocalCopyPolicy::Required,
        }
    }

    #[test]
    fn put_batch_groups_by_stable_chunk_plan_and_preserves_receipt_order() {
        let temp = tempfile::tempdir().unwrap();
        let local = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
        let local_device = local.device_descriptor().unwrap();
        let snapshot = Arc::new(PlacementSnapshot {
            revision: 7,
            replication: config(3),
            replica_groups: vec![
                ReplicaGroup {
                    id: ReplicaGroupId::new("group-0"),
                    placement_epoch: 70,
                    targets: vec![
                        target("node-a", local_device.clone()),
                        target("node-b", remote_device("remote-b")),
                        target("node-c", remote_device("remote-c")),
                    ],
                },
                ReplicaGroup {
                    id: ReplicaGroupId::new("group-1"),
                    placement_epoch: 71,
                    targets: vec![
                        target("node-a", local_device),
                        target("node-d", remote_device("remote-d")),
                        target("node-e", remote_device("remote-e")),
                    ],
                },
            ],
        });
        let first = staged_for_group(&snapshot, 1);
        let second = staged_for_group(&snapshot, 0);
        let data_plane = Arc::new(FakeDataPlane::default());
        let store = DfsChunkStore::new(
            "node-a".into(),
            local,
            Arc::new(StaticPlacement {
                snapshot: snapshot.clone(),
            }),
            data_plane.clone(),
        );

        let receipts = store
            .put_batch(vec![first.clone(), second.clone()])
            .unwrap();

        assert_eq!(receipts[0].chunk.id, first.chunk.id);
        assert_eq!(receipts[0].replica_group_id, ReplicaGroupId::new("group-1"));
        assert_eq!(receipts[0].placement_epoch, 71);
        assert_eq!(receipts[0].durable_acks.len(), 3);
        assert_eq!(receipts[1].chunk.id, second.chunk.id);
        assert_eq!(receipts[1].replica_group_id, ReplicaGroupId::new("group-0"));
        assert_eq!(receipts[1].placement_epoch, 70);
        assert_eq!(receipts[1].durable_acks.len(), 3);
        assert_eq!(data_plane.prepared.lock().unwrap().len(), 2);
    }

    #[test]
    fn non_head_local_rn_fails_before_prepare_or_local_persist() {
        let temp = tempfile::tempdir().unwrap();
        let local = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
        let local_device = local.device_descriptor().unwrap();
        let snapshot = Arc::new(PlacementSnapshot {
            revision: 9,
            replication: config(2),
            replica_groups: vec![ReplicaGroup {
                id: ReplicaGroupId::new("group-0"),
                placement_epoch: 90,
                targets: vec![
                    target("node-b", remote_device("remote-b")),
                    target("node-a", local_device),
                ],
            }],
        });
        let item = staged("non-head-local");
        let data_plane = Arc::new(FakeDataPlane::default());
        let store = DfsChunkStore::new(
            "node-a".into(),
            local.clone(),
            Arc::new(StaticPlacement { snapshot }),
            data_plane.clone(),
        );

        let error = store.put_batch(vec![item.clone()]).unwrap_err();

        assert_eq!(error.code(), afs_error::NODE_VFS_UNIMPLEMENTED);
        assert!(data_plane.prepared.lock().unwrap().is_empty());
        let mut out = [0; 1];
        assert!(local.read_at(&item.chunk.id, 0, &mut out).is_err());
    }
}

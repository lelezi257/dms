//! DFS replication coordination above the local immutable chunk store.
//!
//! The filesystem-wide `ReplicationConfig` is immutable after initialization.
//! Placement can still change as nodes and devices change; Nodes cache that
//! authority snapshot and derive one-operation `ReplicationPlan` values.

use std::{collections::HashSet, sync::Arc};

use afs_error::{Error, Result};

use crate::{
    dfs::{
        ChunkId, ChunkReceipt, LocalCopyPolicy, PlacementSnapshot, ReplicaAck, ReplicaGroup,
        ReplicaGroupId, ReplicaTarget, ReplicaWriteGrant, ReplicationConfig,
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

    /// Validates structural transport setup before replica side effects.
    /// A lazy channel is not a completed handshake or receiver authorization;
    /// those checks occur at the receiving RPC before persistence.
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
    pub initiator_node_id: String,
    pub initiator_node_epoch: u64,
    /// Complete authoritative chain; only its synchronous prefix is executed.
    pub ordered_targets: Vec<ReplicaTarget>,
    pub sync_target_count: usize,
    pub target_index: usize,
    pub target: ReplicaTarget,
    pub chain_tail: Vec<ReplicaTarget>,
}

impl ReplicaPeerOp {
    /// Build receiver execution only from an authenticated Meta response.
    pub fn from_grant(grant: &ReplicaWriteGrant) -> Result<Self> {
        if !grant.replication.is_valid() || grant.token.is_empty() {
            return Err(invalid("replica write grant has no valid authority"));
        }
        let plan = ReplicationPlan {
            chunk_id: grant.chunk_id.clone(),
            placement_revision: grant.placement_revision,
            placement_epoch: grant.placement_epoch,
            replica_group_id: grant.replica_group_id.clone(),
            config: grant.replication.clone(),
            ordered_targets: grant.replica_group.targets.clone(),
        };
        plan.validate_initiator(&grant.initiator_node_id)?;
        if grant.replica_group.id != grant.replica_group_id
            || grant.replica_group.placement_epoch != grant.placement_epoch
        {
            return Err(invalid("replica write grant group identity differs"));
        }
        let sync_target_count = plan.sync_target_count()?;
        let target_index = grant.target_index as usize;
        if target_index >= sync_target_count {
            return Err(invalid("receiver is not part of synchronous chain"));
        }
        let target = plan.ordered_targets[target_index].clone();
        if target.node_id != grant.requester_node_id
            || target.node_epoch != grant.requester_node_epoch
        {
            return Err(invalid("replica write grant is not for this receiver"));
        }
        let result = Self {
            chunk_id: grant.chunk_id.clone(),
            placement_revision: grant.placement_revision,
            placement_epoch: grant.placement_epoch,
            replica_group_id: grant.replica_group_id.clone(),
            initiator_node_id: grant.initiator_node_id.clone(),
            initiator_node_epoch: grant.initiator_node_epoch,
            ordered_targets: plan.ordered_targets.clone(),
            sync_target_count,
            target_index,
            target,
            chain_tail: plan.ordered_targets[target_index + 1..sync_target_count].to_vec(),
        };
        result.validate_shape()?;
        Ok(result)
    }

    pub fn validate_sender(&self, authenticated_peer: &str) -> Result<()> {
        self.validate_shape()?;
        let expected = if self.target_index == 0 {
            self.initiator_node_id.as_str()
        } else {
            self.ordered_targets[self.target_index - 1].node_id.as_str()
        };
        if authenticated_peer != expected {
            return Err(Error::coded(
                afs_error::IO_PERMISSION_DENIED,
                "replica writer is not the authorized predecessor",
            ));
        }
        Ok(())
    }

    /// Validate domain/wire structure before asking authority or creating files.
    /// Authority still has to validate every epoch, endpoint and assignment.
    pub fn validate_shape(&self) -> Result<()> {
        if self.initiator_node_id.is_empty()
            || self.initiator_node_epoch == 0
            || self.placement_revision == 0
            || self.placement_epoch == 0
            || self.sync_target_count == 0
            || self.sync_target_count > self.ordered_targets.len()
            || self.target_index >= self.sync_target_count
            || self.ordered_targets.get(self.target_index) != Some(&self.target)
            || self.chain_tail
                != self.ordered_targets[self.target_index + 1..self.sync_target_count]
        {
            return Err(invalid(
                "replica operation has an invalid chain/initiator identity",
            ));
        }
        let mut nodes = HashSet::new();
        for target in &self.ordered_targets {
            if target.node_id.is_empty()
                || target.node_epoch == 0
                || target.device.device_id.is_empty()
                || target.device.device_epoch == 0
                || target.data_endpoint.is_empty()
                || !nodes.insert(target.node_id.as_str())
            {
                return Err(invalid(
                    "replica operation has an invalid or duplicate target",
                ));
            }
        }
        Ok(())
    }

    pub fn next_hop(&self) -> Result<Option<Self>> {
        self.validate_shape()?;
        let target_index = self.target_index + 1;
        if target_index == self.sync_target_count {
            return Ok(None);
        }
        let mut next = self.clone();
        next.target_index = target_index;
        next.target = self.ordered_targets[target_index].clone();
        next.chain_tail = self.ordered_targets[target_index + 1..self.sync_target_count].to_vec();
        Ok(Some(next))
    }
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

    fn sync_target_count(&self) -> Result<usize> {
        let minimum = usize::from(
            self.config
                .sync_required_copies
                .max(self.config.min_distinct_nodes),
        );
        let mut domains = HashSet::new();
        for (index, target) in self.ordered_targets.iter().enumerate() {
            domains.insert(target.device.failure_domain.as_str());
            if index + 1 >= minimum
                && domains.len() >= usize::from(self.config.min_distinct_failure_domains)
            {
                return Ok(index + 1);
            }
        }
        Err(invalid(
            "placement cannot satisfy synchronous node/failure-domain constraints",
        ))
    }

    fn validate_initiator(&self, local_node_id: &str) -> Result<()> {
        let mut nodes = HashSet::new();
        if self
            .ordered_targets
            .iter()
            .any(|target| !nodes.insert(target.node_id.as_str()))
        {
            return Err(invalid(
                "placement contains multiple targets on the same node",
            ));
        }
        if self.config.local_copy == LocalCopyPolicy::Required
            && self
                .ordered_targets
                .first()
                .is_none_or(|target| target.node_id != local_node_id)
        {
            return Err(invalid(
                "local-required placement must start at the writer node",
            ));
        }
        self.sync_target_count()?;
        Ok(())
    }

    fn selected_peer_op(
        &self,
        local_node_id: &str,
        local_node_epoch: u64,
    ) -> Result<Option<ReplicaPeerOp>> {
        let sync_target_count = self.sync_target_count()?;
        let local_head = self
            .ordered_targets
            .first()
            .is_some_and(|target| target.node_id == local_node_id);
        let target_index = usize::from(local_head);
        if target_index >= sync_target_count {
            return Ok(None);
        }
        let target = self
            .ordered_targets
            .get(target_index)
            .ok_or_else(|| invalid("replica first hop is absent"))?;
        Ok(Some(ReplicaPeerOp {
            chunk_id: self.chunk_id.clone(),
            placement_revision: self.placement_revision,
            placement_epoch: self.placement_epoch,
            replica_group_id: self.replica_group_id.clone(),
            initiator_node_id: local_node_id.into(),
            initiator_node_epoch: local_node_epoch,
            ordered_targets: self.ordered_targets.clone(),
            sync_target_count,
            target_index,
            target: target.clone(),
            chain_tail: self.ordered_targets[target_index + 1..sync_target_count].to_vec(),
        }))
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
    local_node_epoch: u64,
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
        Self::new_with_epoch(local_node_id, 0, local, placement, data_plane)
    }

    pub fn new_with_epoch(
        local_node_id: String,
        local_node_epoch: u64,
        local: Arc<LocalChunkStore>,
        placement: Arc<dyn PlacementProvider>,
        data_plane: Arc<dyn ReplicaDataPlane>,
    ) -> Self {
        Self {
            local_node_id,
            local_node_epoch,
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

        // Check the whole batch before creating local durable side effects.
        for plan in &plans {
            plan.validate_initiator(&self.local_node_id)?;
            if let Some(op) = plan.selected_peer_op(&self.local_node_id, self.local_node_epoch)? {
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
            let local_acks = match group
                .plan
                .ordered_targets
                .first()
                .filter(|target| target.node_id == self.local_node_id)
            {
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
                if let Some(op) =
                    plan.selected_peer_op(&self.local_node_id, self.local_node_epoch)?
                {
                    durable_acks.extend(self.data_plane.put_peer_replica(&op, &item)?);
                }
                validate_acks(plan, &item, &durable_acks, &self.local_node_id)?;
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

    pub fn new_with_epoch(
        local_node_id: String,
        local_node_epoch: u64,
        local: Arc<LocalChunkStore>,
        placement: Arc<dyn PlacementProvider>,
        data_plane: Arc<dyn ReplicaDataPlane>,
    ) -> Self {
        Self {
            local_node_id: local_node_id.clone(),
            local: local.clone(),
            placement: placement.clone(),
            replication: ReplicationEngine::new_with_epoch(
                local_node_id,
                local_node_epoch,
                local,
                placement,
                data_plane,
            ),
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
        for plan in &plans {
            plan.validate_initiator(&self.local_node_id)?;
        }
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

/// Validate an ordered synchronous tail before acknowledging its predecessor.
pub(crate) fn validate_peer_acks(
    op: &ReplicaPeerOp,
    staged: &StagedChunk,
    acks: &[ReplicaAck],
) -> Result<()> {
    op.validate_shape()?;
    let targets = &op.ordered_targets[op.target_index..op.sync_target_count];
    if acks.len() != targets.len() {
        return Err(invalid("replica acknowledgement tail is incomplete"));
    }
    for (ack, target) in acks.iter().zip(targets) {
        if ack.operation_id != staged.operation_id
            || ack.chunk_id != staged.chunk.id
            || ack.placement_revision != op.placement_revision
            || ack.placement_epoch != op.placement_epoch
            || ack.node_id != target.node_id
            || ack.node_epoch != target.node_epoch
            || ack.device_id != target.device.device_id
            || ack.device_epoch != target.device.device_epoch
            || ack.catalog_revision < target.device.catalog_revision
            || ack.persisted_bytes != staged.chunk.length
            || ack.verified_digest != staged.chunk.content_digest
        {
            return Err(invalid(
                "replica acknowledgement differs from authorized immutable tail",
            ));
        }
    }
    Ok(())
}

fn validate_acks(
    plan: &ReplicationPlan,
    staged: &StagedChunk,
    durable_acks: &[ReplicaAck],
    initiator: &str,
) -> Result<()> {
    let mut acknowledged_targets = HashSet::new();
    let mut domains = HashSet::new();
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
        if !acknowledged_targets.insert(ack.node_id.clone()) {
            return Err(invalid(
                "replica acknowledgements contain a duplicate target",
            ));
        }
        let target = plan
            .ordered_targets
            .iter()
            .find(|target| target.node_id == ack.node_id)
            .expect("assigned acknowledgement checked above");
        domains.insert(target.device.failure_domain.as_str());
    }
    if acknowledged_targets.len() < usize::from(plan.config.min_distinct_nodes)
        || domains.len() < usize::from(plan.config.min_distinct_failure_domains)
        || (plan.config.local_copy == LocalCopyPolicy::Required
            && !acknowledged_targets.contains(initiator))
    {
        return Err(invalid(
            "replica acknowledgements violate node, failure-domain or local-copy constraints",
        ));
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

        assert_eq!(error.code(), afs_error::NODE_STORAGE_INVALID);
        assert!(data_plane.prepared.lock().unwrap().is_empty());
        let mut out = [0; 1];
        assert!(local.read_at(&item.chunk.id, 0, &mut out).is_err());
    }
    #[test]
    fn configurable_two_and_four_copy_chains_return_distinct_durable_acks() {
        for copies in [2_u16, 4] {
            let temp = tempfile::tempdir().unwrap();
            let local = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
            let mut targets = vec![target("node-a", local.device_descriptor().unwrap())];
            targets.extend((1..copies).map(|index| {
                target(
                    &format!("remote-{index}"),
                    remote_device(&format!("disk-{index}")),
                )
            }));
            let snapshot = Arc::new(PlacementSnapshot {
                revision: 7,
                replication: config(copies),
                replica_groups: vec![ReplicaGroup {
                    id: ReplicaGroupId::new("group"),
                    placement_epoch: 8,
                    targets,
                }],
            });
            let plane = Arc::new(FakeDataPlane::default());
            let store = DfsChunkStore::new_with_epoch(
                "node-a".into(),
                1,
                local,
                Arc::new(StaticPlacement { snapshot }),
                plane,
            );
            assert_eq!(
                store.put(staged("variable-n")).unwrap().durable_acks.len(),
                usize::from(copies)
            );
        }
    }

    #[test]
    fn preferred_remote_head_does_not_create_unassigned_local_copy() {
        let temp = tempfile::tempdir().unwrap();
        let local = Arc::new(LocalChunkStore::open(temp.path(), "writer").unwrap());
        let mut policy = config(2);
        policy.local_copy = LocalCopyPolicy::Preferred;
        let snapshot = Arc::new(PlacementSnapshot {
            revision: 7,
            replication: policy,
            replica_groups: vec![ReplicaGroup {
                id: ReplicaGroupId::new("group"),
                placement_epoch: 8,
                targets: vec![
                    target("head", remote_device("head-disk")),
                    target("tail", remote_device("tail-disk")),
                ],
            }],
        });
        let plane = Arc::new(FakeDataPlane::default());
        let store = DfsChunkStore::new_with_epoch(
            "writer".into(),
            9,
            local.clone(),
            Arc::new(StaticPlacement { snapshot }),
            plane.clone(),
        );
        let item = staged("remote-head");
        let receipt = store.put(item.clone()).unwrap();
        assert_eq!(receipt.durable_acks.len(), 2);
        assert_eq!(plane.prepared.lock().unwrap()[0].1, "head");
        assert!(local.read_at(&item.chunk.id, 0, &mut [0; 1]).is_err());
    }

    #[test]
    fn async_desired_three_sync_one_does_not_wait_for_tail() {
        let temp = tempfile::tempdir().unwrap();
        let local = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
        let mut policy = config(3);
        policy.sync_required_copies = 1;
        policy.min_distinct_nodes = 1;
        let snapshot = Arc::new(PlacementSnapshot {
            revision: 7,
            replication: policy,
            replica_groups: vec![ReplicaGroup {
                id: ReplicaGroupId::new("group"),
                placement_epoch: 8,
                targets: vec![
                    target("node-a", local.device_descriptor().unwrap()),
                    target("tail-1", remote_device("disk-1")),
                    target("tail-2", remote_device("disk-2")),
                ],
            }],
        });
        let plane = Arc::new(FakeDataPlane::default());
        let store = DfsChunkStore::new_with_epoch(
            "node-a".into(),
            1,
            local,
            Arc::new(StaticPlacement { snapshot }),
            plane.clone(),
        );
        assert_eq!(store.put(staged("async-m1")).unwrap().durable_acks.len(), 1);
        assert!(plane.prepared.lock().unwrap().is_empty());
    }

    #[test]
    fn failure_domain_constraint_extends_synchronous_prefix() {
        let mut policy = config(3);
        policy.sync_required_copies = 1;
        policy.min_distinct_nodes = 1;
        policy.min_distinct_failure_domains = 2;
        let mut targets = vec![
            target("a", remote_device("a")),
            target("b", remote_device("b")),
            target("c", remote_device("c")),
        ];
        targets[0].device.failure_domain = "zone-1".into();
        targets[1].device.failure_domain = "zone-1".into();
        targets[2].device.failure_domain = "zone-2".into();
        let snapshot = PlacementSnapshot {
            revision: 7,
            replication: policy,
            replica_groups: vec![ReplicaGroup {
                id: ReplicaGroupId::new("g"),
                placement_epoch: 8,
                targets,
            }],
        };
        let plan = ReplicationPlan::derive(&staged("domains"), &snapshot).unwrap();
        assert_eq!(plan.sync_target_count().unwrap(), 3);
    }

    #[test]
    fn duplicate_node_placement_is_rejected_before_any_chunk_side_effect() {
        let item = staged("duplicate-node");
        let snapshot = PlacementSnapshot {
            revision: 7,
            replication: config(2),
            replica_groups: vec![ReplicaGroup {
                id: ReplicaGroupId::new("g"),
                placement_epoch: 8,
                targets: vec![
                    target("a", remote_device("d1")),
                    target("a", remote_device("d2")),
                ],
            }],
        };
        let plan = ReplicationPlan::derive(&item, &snapshot).unwrap();
        assert!(plan.validate_initiator("a").is_err());
    }

    #[test]
    fn missing_or_duplicate_tail_ack_is_not_a_successful_receipt() {
        let item = staged("acks");
        let snapshot = PlacementSnapshot {
            revision: 7,
            replication: config(2),
            replica_groups: vec![ReplicaGroup {
                id: ReplicaGroupId::new("g"),
                placement_epoch: 8,
                targets: vec![
                    target("a", remote_device("d1")),
                    target("b", remote_device("d2")),
                ],
            }],
        };
        let plan = ReplicationPlan::derive(&item, &snapshot).unwrap();
        let op = plan.selected_peer_op("a", 1).unwrap().unwrap();
        let first = ack_for(&plan.ordered_targets[0], &op, &item);
        assert!(validate_acks(&plan, &item, std::slice::from_ref(&first), "a").is_err());
        assert!(validate_acks(&plan, &item, &[first.clone(), first], "a").is_err());
    }
    #[test]
    fn next_hop_preserves_initiator_and_stops_at_sync_prefix() {
        let mut policy = config(3);
        policy.sync_required_copies = 2;
        policy.min_distinct_nodes = 2;
        policy.local_copy = LocalCopyPolicy::NotRequired;
        let snapshot = PlacementSnapshot {
            revision: 7,
            replication: policy,
            replica_groups: vec![ReplicaGroup {
                id: ReplicaGroupId::new("g"),
                placement_epoch: 8,
                targets: vec![
                    target("a", remote_device("d1")),
                    target("b", remote_device("d2")),
                    target("c", remote_device("d3")),
                ],
            }],
        };
        let plan = ReplicationPlan::derive(&staged("hop"), &snapshot).unwrap();
        let first = plan.selected_peer_op("writer", 5).unwrap().unwrap();
        first.validate_shape().unwrap();
        let second = first.next_hop().unwrap().unwrap();
        assert_eq!(second.target.node_id, "b");
        assert_eq!(second.initiator_node_id, "writer");
        assert_eq!(second.initiator_node_epoch, 5);
        assert!(second.next_hop().unwrap().is_none());
        let mut malformed = first;
        malformed.chain_tail.clear();
        assert!(malformed.validate_shape().is_err());
    }
    #[test]
    fn receiver_execution_requires_meta_target_and_authenticated_predecessor() {
        let item = staged("grant");
        let grant = ReplicaWriteGrant {
            requester_node_id: "b".into(),
            requester_node_epoch: 1,
            initiator_node_id: "writer".into(),
            initiator_node_epoch: 5,
            operation_id: item.operation_id.clone(),
            chunk_id: item.chunk.id.clone(),
            chunk_length: item.chunk.length,
            content_digest: item.chunk.content_digest.clone(),
            placement_revision: 7,
            placement_epoch: 8,
            replica_group_id: ReplicaGroupId::new("g"),
            target_index: 1,
            replication: ReplicationConfig {
                local_copy: LocalCopyPolicy::NotRequired,
                ..config(2)
            },
            replica_group: ReplicaGroup {
                id: ReplicaGroupId::new("g"),
                placement_epoch: 8,
                targets: vec![
                    target("a", remote_device("a")),
                    target("b", remote_device("b")),
                ],
            },
            expires_at_unix_ms: u64::MAX,
            fence: 7,
            token: "authenticated-meta-response".into(),
        };
        let op = ReplicaPeerOp::from_grant(&grant).unwrap();
        op.validate_sender("a").unwrap();
        assert!(op.validate_sender("writer").is_err());
        assert!(op.validate_sender("unrelated").is_err());
        let mut wrong_target = grant;
        wrong_target.requester_node_id = "other".into();
        assert!(ReplicaPeerOp::from_grant(&wrong_target).is_err());
    }
    struct DurableFixturePlane {
        stores: std::collections::HashMap<String, Arc<LocalChunkStore>>,
        unavailable: Mutex<Option<String>>,
    }

    impl ReplicaDataPlane for DurableFixturePlane {
        fn mode(&self) -> ReplicaTransferMode {
            ReplicaTransferMode::GrpcStream
        }
        fn prepare_peer(&self, op: &ReplicaPeerOp) -> Result<()> {
            op.validate_shape()
        }
        fn put_peer_replica(
            &self,
            op: &ReplicaPeerOp,
            staged: &StagedChunk,
        ) -> Result<Vec<ReplicaAck>> {
            op.validate_shape()?;
            if self.unavailable.lock().unwrap().as_deref() == Some(op.target.node_id.as_str()) {
                return Err(Error::coded(
                    afs_error::NODE_TRANSFER_UNAVAILABLE,
                    "injected missing tail",
                ));
            }
            let local = self
                .stores
                .get(&op.target.node_id)
                .ok_or_else(|| invalid("fixture has no assigned target"))?;
            let mut result = vec![local.persist(
                staged,
                &op.target,
                op.placement_revision,
                op.placement_epoch,
            )?];
            if let Some(next) = op.next_hop()? {
                result.extend(self.put_peer_replica(&next, staged)?);
            }
            Ok(result)
        }
    }

    #[test]
    fn durable_middle_with_missing_tail_retries_content_and_returns_complete_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let mut stores = std::collections::HashMap::new();
        let mut targets = Vec::new();
        for node in ["a", "b", "c"] {
            let local = Arc::new(LocalChunkStore::open(temp.path().join(node), node).unwrap());
            targets.push(target(node, local.device_descriptor().unwrap()));
            stores.insert(node.to_owned(), local);
        }
        let local = stores["a"].clone();
        let snapshot = Arc::new(PlacementSnapshot {
            revision: 7,
            replication: config(3),
            replica_groups: vec![ReplicaGroup {
                id: ReplicaGroupId::new("g"),
                placement_epoch: 8,
                targets,
            }],
        });
        let plane = Arc::new(DurableFixturePlane {
            stores,
            unavailable: Mutex::new(Some("c".into())),
        });
        let store = DfsChunkStore::new_with_epoch(
            "a".into(),
            1,
            local.clone(),
            Arc::new(StaticPlacement {
                snapshot: snapshot.clone(),
            }),
            plane.clone(),
        );
        let item = staged("durable-chain");
        assert!(store.put(item.clone()).is_err());
        let mut bytes = [0; 13];
        assert_eq!(local.read_at(&item.chunk.id, 0, &mut bytes).unwrap(), 13);
        assert_eq!(
            plane.stores["b"]
                .read_at(&item.chunk.id, 0, &mut bytes)
                .unwrap(),
            13
        );
        assert!(
            plane.stores["c"]
                .read_at(&item.chunk.id, 0, &mut bytes)
                .is_err()
        );
        *plane.unavailable.lock().unwrap() = None;
        let receipt = store.put(item.clone()).unwrap();
        assert_eq!(receipt.durable_acks.len(), 3);
        for node in ["a", "b", "c"] {
            let reopened = LocalChunkStore::open(temp.path().join(node), node).unwrap();
            assert_eq!(reopened.read_at(&item.chunk.id, 0, &mut bytes).unwrap(), 13);
            assert_eq!(&bytes, b"durable-chain");
        }
    }
}

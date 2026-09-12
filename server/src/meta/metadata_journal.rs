//! dms-meta 的最小可靠存储边界。
//!
//! Meta 自己完成 CAS、幂等、事件生成和状态机推进；后端只保存已经决定的顺序记录。
//! 内存后端用于不要求重启恢复的配置，本地 WAL 后端负责持久记录与快照。
//! 两者不包含多 Meta 选主或共识协议，不能把本地持久化等同于高可用。

use dms_protocol::v1 as pb;

use super::metrics::JournalRecordMetric;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VersionCommitRecord {
    pub(crate) key: Vec<u8>,
    pub(crate) layout: pb::VersionLayout,
    pub(crate) modified_time_unix_millis: i64,
    pub(crate) new_replicas: Vec<(pb::ReplicaLocation, u64)>,
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommitSequenceRecord {
    pub(crate) node_id: u64,
    pub(crate) node_epoch: u64,
    pub(crate) commit_sequence: u64,
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) commit_index: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockRetirementParticipant {
    pub(crate) node_id: u64,
    pub(crate) node_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockRetirementRecord {
    pub(crate) retirement_id: Vec<u8>,
    pub(crate) block_ids: Vec<Vec<u8>>,
    pub(crate) participants: Vec<BlockRetirementParticipant>,
    pub(crate) prepare_stage_epoch: u64,
    pub(crate) final_stage_epoch: u64,
    pub(crate) fence_version: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SnapshotBlockRetirement {
    pub(crate) record: BlockRetirementRecord,
    pub(crate) prepared: Vec<BlockRetirementParticipant>,
    pub(crate) released: Vec<BlockRetirementParticipant>,
    pub(crate) final_sent: bool,
}

/// 已经由 Meta 状态机决定、可以按顺序重放的记录。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JournalRecord {
    /// 一个 Node incarnation 已经打开并获得新的 epoch。
    NodeSessionOpened {
        node_id: u64,
        node_epoch: u64,
        session_id: Vec<u8>,
        control_endpoint: String,
        next_session: u64,
        supports_commit_sequence: bool,
        commit_sequence_floor: u64,
    },
    /// 一个 Node 已经持有某个 Block 的物理事实。
    ReplicaAccepted {
        location: pb::ReplicaLocation,
        length: u64,
        catalog_revision: u64,
    },
    /// 一次后台 Replica report 的幂等结果。
    ReplicasReported {
        operation_id: Vec<u8>,
        accepted: Vec<(pb::ReplicaLocation, u64)>,
        rejected_block_ids: Vec<Vec<u8>>,
        catalog_revision: u64,
        desired_copies: u32,
    },
    /// 一个逻辑版本已经成为该 Key 的新 Current。
    VersionCommitted {
        key: Vec<u8>,
        layout: pb::VersionLayout,
        modified_time_unix_millis: i64,
        /// Replica facts created by the same logical commit. Keeping them in
        /// one journal record prevents recovery from observing half a write.
        new_replicas: Vec<(pb::ReplicaLocation, u64)>,
        operation_id: Vec<u8>,
        operation_digest: Vec<u8>,
        commit_sequence: Option<CommitSequenceRecord>,
    },
    /// Multiple key transitions accepted atomically by one Meta actor turn and
    /// persisted as one WAL record.
    VersionsCommitted {
        commits: Vec<VersionCommitRecord>,
        commit_sequence: Option<CommitSequenceRecord>,
    },
    /// 一个没有改变 Version 的操作结果也必须可恢复，例如重复 DEL 缺失 key。
    OperationRemembered {
        operation_id: Vec<u8>,
        operation_digest: Vec<u8>,
        result: pb::CommitVersionResponse,
        commit_sequence: Option<CommitSequenceRecord>,
    },
    /// Node 已经处理到某个 Meta event cursor。
    NodeEventAcknowledged { node_id: u64, cursor: u64 },
    /// 一个可回收 Block 批次已进入 durable Prepare 阶段。
    BlockRetirementPrepared { record: BlockRetirementRecord },
    /// Node 对 Prepare/Final 的专用排空或释放 ACK。
    BlockRetirementAcknowledged {
        retirement_id: Vec<u8>,
        participant: BlockRetirementParticipant,
        ack_kind: pb::BlockRetirementAckKind,
        stage_epoch: u64,
    },
    /// 所有参与者已 drain，Final Evict 已 durable 发布。
    BlockRetirementFinalized {
        retirement_id: Vec<u8>,
        stage_epoch: u64,
    },
    /// 所有副本 Node 已释放，Meta 才能删除 replica facts。
    BlockRetirementReleased {
        retirement_id: Vec<u8>,
        block_ids: Vec<Vec<u8>>,
        stage_epoch: u64,
    },
}

impl JournalRecord {
    /// Stable, bounded label used by Meta journal metrics.
    ///
    /// The label describes the state transition, never the user key or object,
    /// so the Prometheus series count stays bounded as data grows.
    pub(crate) const fn metric_kind(&self) -> JournalRecordMetric {
        match self {
            Self::NodeSessionOpened { .. } => JournalRecordMetric::NodeSessionOpened,
            Self::ReplicaAccepted { .. } => JournalRecordMetric::ReplicaAccepted,
            Self::ReplicasReported { .. } => JournalRecordMetric::ReplicasReported,
            Self::VersionCommitted { .. } => JournalRecordMetric::VersionCommitted,
            Self::VersionsCommitted { .. } => JournalRecordMetric::VersionsCommitted,
            Self::OperationRemembered { .. } => JournalRecordMetric::OperationRemembered,
            Self::NodeEventAcknowledged { .. } => JournalRecordMetric::NodeEventAcknowledged,
            Self::BlockRetirementPrepared { .. } => JournalRecordMetric::BlockRetirementPrepared,
            Self::BlockRetirementAcknowledged { .. } => {
                JournalRecordMetric::BlockRetirementAcknowledged
            }
            Self::BlockRetirementFinalized { .. } => JournalRecordMetric::BlockRetirementFinalized,
            Self::BlockRetirementReleased { .. } => JournalRecordMetric::BlockRetirementReleased,
        }
    }
}

/// Journal 中带单调序号的一条记录。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JournalEntry {
    pub(crate) sequence: u64,
    pub(crate) record: JournalRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SnapshotReplica {
    pub(crate) block_id: Vec<u8>,
    pub(crate) location: pb::ReplicaLocation,
    pub(crate) catalog_revision: u64,
    pub(crate) length: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SnapshotOperation {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) digest: Vec<u8>,
    pub(crate) result: pb::CommitVersionResponse,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SnapshotReplicaOperation {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) result: pb::ReportReplicasResponse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SnapshotSession {
    pub(crate) node_id: u64,
    pub(crate) session_id: Vec<u8>,
    pub(crate) node_epoch: u64,
    pub(crate) control_endpoint: String,
    pub(crate) last_acked_cursor: u64,
    pub(crate) supports_commit_sequence: bool,
    pub(crate) commit_sequence_floor: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MetaSnapshot {
    pub(crate) last_applied_index: u64,
    pub(crate) version_floor: u64,
    pub(crate) next_session: u64,
    pub(crate) node_epochs: Vec<(u64, u64)>,
    pub(crate) node_commit_sequence_floors: Vec<(u64, u64)>,
    pub(crate) sessions: Vec<SnapshotSession>,
    pub(crate) replicas: Vec<SnapshotReplica>,
    pub(crate) desired_replica_counts: Vec<(Vec<u8>, u32)>,
    pub(crate) versions: Vec<(Vec<u8>, Vec<pb::VersionLayout>)>,
    pub(crate) version_modified_times: Vec<(Vec<u8>, u64, i64)>,
    pub(crate) block_retirements: Vec<SnapshotBlockRetirement>,
    pub(crate) retired_block_fences: Vec<(Vec<u8>, u64)>,
    pub(crate) commit_sequences: Vec<CommitSequenceRecord>,
    pub(crate) operations: Vec<SnapshotOperation>,
    pub(crate) replica_operations: Vec<SnapshotReplicaOperation>,
    pub(crate) event_high_watermark: u64,
    pub(crate) events: Vec<pb::NodeEvent>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum JournalError {
    Unavailable(&'static str),
    InvalidSnapshot(&'static str),
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::InvalidSnapshot(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for JournalError {}

/// Meta 对可靠存储的最小依赖。
///
/// Meta actor 按顺序调用：先 append 成功，才 apply 到可见内存状态。
/// 内存后端无磁盘 I/O；本地 WAL 的 append 包含同步刷盘，会阻塞当前执行线程。
/// 这是当前实现的性能边界，不是异步存储承诺。后续异步化仍须保持提交顺序，
/// 不能在持久记录失败时先向用户暴露新版本。
pub(crate) trait MetadataJournal: Send {
    /// 按唯一顺序追加一条已决定记录，并返回分配的序号。
    fn append(&mut self, record: JournalRecord) -> Result<u64, JournalError>;

    /// 返回严格晚于 `sequence` 的记录，用于恢复或增量重放。
    fn load_after(&self, sequence: u64) -> Result<Vec<JournalEntry>, JournalError>;

    fn load_snapshot(&self) -> Result<Option<MetaSnapshot>, JournalError>;

    fn save_snapshot(&mut self, snapshot: MetaSnapshot) -> Result<(), JournalError>;

    fn truncate_prefix(&mut self, sequence: u64) -> Result<(), JournalError>;

    fn last_index(&self) -> u64;
}

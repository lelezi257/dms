//! dms-meta 的最小可靠存储边界。
//!
//! Meta 自己完成 CAS、幂等、事件生成和状态机推进；后端只保存已经决定的顺序记录。
//! 内存后端用于不要求重启恢复的配置，本地 WAL 后端负责持久记录与快照。
//! 两者不包含多 Meta 选主或共识协议，不能把本地持久化等同于高可用。

use dms_protocol::v1 as pb;

use crate::filesystem::{DentrySnapshot, InodeId, InodeSnapshot, NamespaceMutationResult};

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

/// 文件创建的一次完整 namespace 变更。
///
/// inode、dentry 与分配器水位必须共用一条记录，否则恢复后可能重用已经发布的 inode。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FilesystemInodeCreatedRecord {
    pub(crate) inode: InodeSnapshot,
    pub(crate) dentry: DentrySnapshot,
    pub(crate) next_inode: InodeId,
    pub(crate) grant_generation: u64,
}

/// 文件内容版本的一次原子发布。
///
/// `object` 是原有 DataCore 版本状态转换，`inode` 是同一时刻可见的精确绑定；两者
/// 只能一起被 journal 接受和恢复。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FilesystemVersionCommitRecord {
    pub(crate) object: VersionCommitRecord,
    pub(crate) inode: InodeSnapshot,
    pub(crate) revoked_grant_generation: u64,
    pub(crate) new_grant_generation: u64,
}

/// create/mkdir/rename/unlink/rmdir 的一次 Meta 权威 namespace 变更。
///
/// 这条记录保存的是 Meta actor 已经决定的最终状态 delta：哪些 inode 被更新、
/// 哪些 dentry 被新增/替换、哪些 dentry 被删除，以及 inode 分配器的新水位。
/// 重放时不重新做 lookup/CAS，避免恢复路径和在线路径得出不同结果。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FilesystemNamespaceMutationRecord {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) result: NamespaceMutationResult,
    pub(crate) upsert_inodes: Vec<InodeSnapshot>,
    pub(crate) upsert_dentries: Vec<DentrySnapshot>,
    pub(crate) remove_dentries: Vec<DentrySnapshot>,
    pub(crate) next_inode: InodeId,
}

/// 符号链接创建的单条权威记录。
///
/// symlink target 仍是 DataCore exact ObjectVersion；同时 dentry/inode/content binding
/// 必须一条 WAL 原子发布，禁止先让 path 可见再单独提交 target。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FilesystemSymlinkCreatedRecord {
    pub(crate) namespace: FilesystemNamespaceMutationRecord,
    pub(crate) version: VersionCommitRecord,
    pub(crate) new_grant_generation: u64,
}

/// 一个已经脱离 namespace 且不再被任何 Node 引用的 inode 被持久回收。
///
/// `object_tombstone` 与 inode 删除共用一条 WAL 记录：恢复时不会出现 inode 已消失但
/// DataCore Current 仍指向旧内容，或对象已删除但 inode 又被快照恢复的半完成状态。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FilesystemOrphanReapedRecord {
    pub(crate) inode: InodeId,
    pub(crate) object_tombstone: Option<VersionCommitRecord>,
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
    /// 一个 inode 与父目录名字同时成为可见状态。
    FilesystemInodeCreated {
        record: FilesystemInodeCreatedRecord,
    },
    /// 对象版本、inode 精确绑定和缓存撤销由同一记录发布。
    FilesystemVersionCommitted {
        record: FilesystemVersionCommitRecord,
        commit_sequence: Option<CommitSequenceRecord>,
    },
    /// 目录树名字空间的一次原子变更，覆盖 create/mkdir/rename/unlink/rmdir。
    FilesystemNamespaceMutated {
        record: FilesystemNamespaceMutationRecord,
        commit_sequence: Option<CommitSequenceRecord>,
    },
    /// symlink 的名字、inode binding 与 target 对象版本一次性发布。
    FilesystemSymlinkCreated {
        record: FilesystemSymlinkCreatedRecord,
        commit_sequence: Option<CommitSequenceRecord>,
    },
    /// link_count=0、引用租约已消失且恢复保护窗口已结束的 inode 被回收。
    FilesystemOrphanReaped {
        record: FilesystemOrphanReapedRecord,
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
            Self::FilesystemInodeCreated { .. } => JournalRecordMetric::FilesystemInodeCreated,
            Self::FilesystemVersionCommitted { .. } => {
                JournalRecordMetric::FilesystemVersionCommitted
            }
            Self::FilesystemNamespaceMutated { .. } => {
                JournalRecordMetric::FilesystemNamespaceMutated
            }
            Self::FilesystemSymlinkCreated { .. } => JournalRecordMetric::FilesystemSymlinkCreated,
            Self::FilesystemOrphanReaped { .. } => JournalRecordMetric::FilesystemOrphanReaped,
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SnapshotFilesystemNamespaceOperation {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) digest: Vec<u8>,
    pub(crate) result: NamespaceMutationResult,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SnapshotFilesystemVersionOperation {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) digest: Vec<u8>,
    pub(crate) response: pb::FilesystemCommitVersionResponse,
}

/// Snapshot 中尚未完成的 Filesystem 可见性屏障。
///
/// 该字段作为 snapshot 尾部扩展单独编码，避免改变旧版 operation entry 的二进制布局。
/// `None` 表示读取的是尚无此尾部的旧 snapshot；`Some(empty)` 则明确表示新版
/// snapshot 创建时没有待确认的 Filesystem 操作。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SnapshotFilesystemOperationVisibility {
    pub(crate) namespace: Vec<(Vec<u8>, u64)>,
    pub(crate) versions: Vec<(Vec<u8>, u64)>,
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
    pub(crate) filesystem_next_inode: InodeId,
    pub(crate) filesystem_inodes: Vec<InodeSnapshot>,
    pub(crate) filesystem_dentries: Vec<DentrySnapshot>,
    pub(crate) filesystem_grant_generations: Vec<(InodeId, u64)>,
    pub(crate) filesystem_namespace_operations: Vec<SnapshotFilesystemNamespaceOperation>,
    pub(crate) filesystem_version_operations: Vec<SnapshotFilesystemVersionOperation>,
    pub(crate) filesystem_operation_visibility: Option<SnapshotFilesystemOperationVisibility>,
}

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

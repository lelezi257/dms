//! dms-meta 内唯一的异步状态所有者。
//!
//! gRPC Handler 只向 [`MetaHandle`] 投递命令。Node Session、Replica、Version、
//! Current、OperationResult 和失效事件只属于 `run_meta` 这一条 actor Task，因此
//! 首版无需全局锁。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt,
    ops::Bound,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use dms_error::{DmsError, ErrorKind};
use dms_protocol::v1 as pb;
use dms_tracing::Instrument as _;
use tokio::sync::{mpsc, oneshot};

use crate::filesystem::{
    ACL_ACCESS_NAME, ACL_DEFAULT_NAME, AttributePatch, CacheGrant, DentrySnapshot, DirectoryGrant,
    FileContentBinding, FileLockOutcome, FileLockOwner, FileSpaceReservation, FilesystemCaller,
    InodeAttributes, InodeKind, InodeSnapshot, InodeVersion, NamespaceMutationResult, ROOT_INODE,
    RemoveKind, ResolvedInode, TimeUpdate, XattrSetMode, XattrUpdate, access_acl_after_chmod,
    attribute_patch_from_proto, caller_from_proto, dentry_to_proto, directory_entry_to_proto,
    directory_grant_to_proto, granted_lock_from_proto, inherit_default_acl, inode_to_proto,
    lock_outcome_to_proto, lock_request_from_proto, namespace_result_to_proto, resolved_to_proto,
    validate_acl_xattr,
};

use super::filesystem::{
    FilesystemCatalog,
    locks::{
        FileLockMutation, FilesystemLockTable, ReleaseLockOwnerResult, ReleaseLockReceipt,
        SetLockResult,
    },
};
#[cfg(test)]
use super::in_memory_journal::InMemoryJournal;
use super::metadata_journal::{
    BlockRetirementParticipant, BlockRetirementRecord, CommitSequenceRecord,
    FilesystemAttributesUpdatedRecord, FilesystemNamespaceMutationRecord,
    FilesystemOrphanReapedRecord, FilesystemSymlinkCreatedRecord, FilesystemVersionCommitRecord,
    FilesystemXattrUpdatedRecord, JournalError, JournalRecord, MetaSnapshot, MetadataJournal,
    SnapshotBlockRetirement, SnapshotFilesystemAttributeOperation,
    SnapshotFilesystemOperationVisibility, SnapshotFilesystemVersionOperation, SnapshotOperation,
    SnapshotReplica, SnapshotReplicaOperation, SnapshotSession, VersionCommitRecord,
};
use super::metrics::{
    CommitOutcome, MetaMetrics, MetaOperation, MetaStateMetricsSnapshot, RepairTransition,
    WatchDelivery, WatchEventType,
};

const META_MAILBOX_CAPACITY: usize = 256;
const DEFAULT_CHECKPOINT_RECORDS: u64 = crate::config::DEFAULT_META_CHECKPOINT_EVERY_RECORDS;
const DEFAULT_RETAINED_VERSIONS_PER_KEY: usize = 64;
const DEFAULT_OPERATION_RETENTION_RECORDS: u64 = 1024;
const DEFAULT_NODE_LEASE_TTL: Duration = Duration::from_secs(30);
const DEFAULT_REPAIR_ATTEMPT_TTL: Duration = Duration::from_secs(30);
const DEFAULT_SCAN_LIMIT: usize = 128;
const MAX_SCAN_LIMIT: usize = 1024;
const MAX_SCAN_CURSOR_LENGTH: usize = 64 * 1024;
const SCAN_CURSOR_TTL_MILLIS: i64 = 5 * 60 * 1000;
const MAX_RETIREMENT_BLOCKS_PER_BATCH: usize = 128;
const MAX_PENDING_BLOCK_RETIREMENTS: usize = 1024;
const MAX_RETIRED_BLOCK_FENCES: usize = 4096;
const GC_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const FILESYSTEM_BINDING_LEASE_MILLIS: u64 = 30_000;
/// 每个维护周期最多回收有限数量，避免大量 orphan 阻塞 Meta actor 的前台命令。
const MAX_FILESYSTEM_ORPHANS_PER_TICK: usize = 64;
const FILESYSTEM_BLOCK_SIZE: u64 = 4096;
#[cfg(test)]
const DEFAULT_FILESYSTEM_MAX_INODES: u64 = 1_000_000;
const MAX_XATTR_NAME_BYTES: usize = 255;
const MAX_XATTR_VALUE_BYTES: usize = 64 * 1024;
const MAX_XATTRS_PER_INODE: usize = 128;
const MAX_XATTR_BYTES_PER_INODE: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MetaRuntimeError {
    InvalidArgument(String),
    UnknownSession,
    NotFound,
    Conflict { expected: Option<u64>, actual: u64 },
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    DirectoryNotEmpty,
    PermissionDenied,
    XattrNotFound,
    XattrAlreadyExists,
    XattrUnsupported,
    XattrTooLarge,
    CapacityUnavailable,
    StaleRevision { expected: Option<u64>, actual: u64 },
    Unavailable,
    JournalAppendFailed(String),
    JournalCorrupt(String),
    JournalUnavailable(String),
}

impl fmt::Display for MetaRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(message) => {
                write!(formatter, "invalid metadata request: {message}")
            }
            Self::UnknownSession => formatter.write_str("unknown metadata session"),
            Self::NotFound => formatter.write_str("metadata object not found"),
            Self::Conflict { expected, actual } => {
                write!(
                    formatter,
                    "metadata version conflict: expected {expected:?}, actual {actual}"
                )
            }
            Self::AlreadyExists => formatter.write_str("filesystem entry already exists"),
            Self::NotDirectory => formatter.write_str("filesystem inode is not a directory"),
            Self::IsDirectory => formatter.write_str("filesystem inode is a directory"),
            Self::DirectoryNotEmpty => formatter.write_str("filesystem directory is not empty"),
            Self::PermissionDenied => formatter.write_str("filesystem permission denied"),
            Self::XattrNotFound => formatter.write_str("filesystem xattr not found"),
            Self::XattrAlreadyExists => formatter.write_str("filesystem xattr already exists"),
            Self::XattrUnsupported => formatter.write_str("filesystem xattr is unsupported"),
            Self::XattrTooLarge => formatter.write_str("filesystem xattr limit exceeded"),
            Self::CapacityUnavailable => {
                formatter.write_str("filesystem capacity is not fully reported")
            }
            Self::StaleRevision { expected, actual } => {
                write!(
                    formatter,
                    "stale filesystem directory revision: expected {expected:?}, actual {actual}"
                )
            }
            Self::Unavailable => formatter.write_str("metadata runtime unavailable"),
            Self::JournalAppendFailed(message) => {
                write!(formatter, "metadata journal append failed: {message}")
            }
            Self::JournalCorrupt(message) => {
                write!(formatter, "metadata journal corrupt: {message}")
            }
            Self::JournalUnavailable(message) => {
                write!(formatter, "metadata journal unavailable: {message}")
            }
        }
    }
}

impl std::error::Error for MetaRuntimeError {}

impl MetaRuntimeError {
    pub(crate) fn into_dms_error(self) -> DmsError {
        match self {
            Self::InvalidArgument(message) => DmsError::new(
                dms_error::META_CATALOG_INVALID_REQUEST,
                ErrorKind::InvalidArgument,
                message,
            ),
            Self::UnknownSession => DmsError::new(
                dms_error::META_SESSION_UNKNOWN,
                ErrorKind::FailedPrecondition,
                "unknown Node session",
            ),
            Self::NotFound => DmsError::new(
                dms_error::META_CATALOG_NOT_FOUND,
                ErrorKind::NotFound,
                "metadata object was not found",
            ),
            Self::Conflict { expected, actual } => DmsError::new(
                dms_error::META_CATALOG_VERSION_CONFLICT,
                ErrorKind::Aborted,
                format!("metadata version conflict: expected={expected:?} actual={actual}"),
            ),
            Self::AlreadyExists => DmsError::new(
                dms_error::META_FILESYSTEM_ALREADY_EXISTS,
                ErrorKind::AlreadyExists,
                "filesystem entry already exists",
            ),
            Self::NotDirectory => DmsError::new(
                dms_error::META_FILESYSTEM_NOT_DIRECTORY,
                ErrorKind::FailedPrecondition,
                "filesystem inode is not a directory",
            ),
            Self::IsDirectory => DmsError::new(
                dms_error::META_FILESYSTEM_IS_DIRECTORY,
                ErrorKind::FailedPrecondition,
                "filesystem inode is a directory",
            ),
            Self::DirectoryNotEmpty => DmsError::new(
                dms_error::META_FILESYSTEM_DIRECTORY_NOT_EMPTY,
                ErrorKind::FailedPrecondition,
                "filesystem directory is not empty",
            ),
            Self::PermissionDenied => DmsError::new(
                dms_error::META_FILESYSTEM_PERMISSION_DENIED,
                ErrorKind::PermissionDenied,
                "filesystem permission denied",
            ),
            Self::XattrNotFound => DmsError::new(
                dms_error::META_FILESYSTEM_XATTR_NOT_FOUND,
                ErrorKind::NotFound,
                "filesystem extended attribute was not found",
            ),
            Self::XattrAlreadyExists => DmsError::new(
                dms_error::META_FILESYSTEM_XATTR_ALREADY_EXISTS,
                ErrorKind::AlreadyExists,
                "filesystem extended attribute already exists",
            ),
            Self::XattrUnsupported => DmsError::new(
                dms_error::META_FILESYSTEM_XATTR_UNSUPPORTED,
                ErrorKind::Unimplemented,
                "filesystem extended attribute namespace or ACL is unsupported",
            ),
            Self::XattrTooLarge => DmsError::new(
                dms_error::META_FILESYSTEM_XATTR_TOO_LARGE,
                ErrorKind::ResourceExhausted,
                "filesystem extended attribute limit exceeded",
            ),
            Self::CapacityUnavailable => DmsError::new(
                dms_error::META_FILESYSTEM_CAPACITY_UNAVAILABLE,
                ErrorKind::Unavailable,
                "filesystem capacity is incomplete until every live Node reports Arena resources",
            ),
            Self::StaleRevision { expected, actual } => DmsError::new(
                dms_error::META_FILESYSTEM_STALE_REVISION,
                ErrorKind::Aborted,
                format!("stale filesystem revision: expected={expected:?} actual={actual}"),
            ),
            Self::Unavailable => DmsError::new(
                dms_error::META_JOURNAL_UNAVAILABLE,
                ErrorKind::Unavailable,
                "DMS Meta runtime or journal is unavailable",
            ),
            Self::JournalAppendFailed(message) => DmsError::new(
                dms_error::META_JOURNAL_APPEND_FAILED,
                ErrorKind::Unavailable,
                message,
            ),
            Self::JournalCorrupt(message) => DmsError::new(
                dms_error::META_JOURNAL_CORRUPT,
                ErrorKind::DataLoss,
                message,
            ),
            Self::JournalUnavailable(message) => DmsError::new(
                dms_error::META_JOURNAL_UNAVAILABLE,
                ErrorKind::Unavailable,
                message,
            ),
        }
    }
}

fn map_journal_error(error: JournalError) -> MetaRuntimeError {
    match error {
        JournalError::Unavailable(message) => {
            MetaRuntimeError::JournalUnavailable(message.to_string())
        }
        JournalError::InvalidSnapshot(message) => {
            MetaRuntimeError::JournalCorrupt(message.to_string())
        }
    }
}

fn map_journal_append_error(error: JournalError) -> MetaRuntimeError {
    match error {
        JournalError::Unavailable(message) => {
            MetaRuntimeError::JournalAppendFailed(message.to_string())
        }
        JournalError::InvalidSnapshot(message) => {
            MetaRuntimeError::JournalCorrupt(message.to_string())
        }
    }
}

/// Meta-owned retention rules for active catalog state.
///
/// The policy is based on authoritative commit indexes, not on HashMap length.
/// That makes the idempotency window explicit: an operation result remains
/// retryable until its commit index is more than
/// `operation_result_retention_records` behind the latest applied journal index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MetaRetentionPolicy {
    pub(crate) keep_versions_per_key: usize,
    pub(crate) operation_result_retention_records: u64,
    pub(crate) replica_operation_retention_records: u64,
    pub(crate) event_retention_requires_all_session_acks: bool,
    pub(crate) drop_unreferenced_replicas: bool,
}

impl Default for MetaRetentionPolicy {
    fn default() -> Self {
        Self {
            keep_versions_per_key: DEFAULT_RETAINED_VERSIONS_PER_KEY,
            operation_result_retention_records: DEFAULT_OPERATION_RETENTION_RECORDS,
            replica_operation_retention_records: DEFAULT_OPERATION_RETENTION_RECORDS,
            event_retention_requires_all_session_acks: true,
            drop_unreferenced_replicas: true,
        }
    }
}

/// Meta-owned checkpoint trigger policy.
///
/// The only configured field has a real execution path. Future byte/time
/// triggers must arrive together with their accounting and clock logic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MetaCheckpointPolicy {
    pub(crate) every_records: u64,
}

impl Default for MetaCheckpointPolicy {
    fn default() -> Self {
        Self {
            every_records: DEFAULT_CHECKPOINT_RECORDS,
        }
    }
}

impl MetaCheckpointPolicy {
    fn should_checkpoint(&self, sequence: u64, last_checkpoint_index: u64) -> bool {
        self.every_records > 0
            && sequence >= last_checkpoint_index
            && sequence - last_checkpoint_index >= self.every_records
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MetaStats {
    pub(crate) journal_last_index: u64,
    pub(crate) snapshot_index: u64,
    pub(crate) version_count: usize,
    pub(crate) operation_count: usize,
    pub(crate) replica_operation_count: usize,
    pub(crate) event_count: usize,
    pub(crate) event_high_watermark: u64,
    pub(crate) replica_block_count: usize,
    pub(crate) replica_location_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NodeSessionGrant {
    pub(crate) session_id: Vec<u8>,
    pub(crate) node_id: u64,
    pub(crate) node_epoch: u64,
    pub(crate) heartbeat_interval_millis: u64,
    pub(crate) lease_ttl_millis: u64,
    pub(crate) minimum_commit_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HeartbeatGrant {
    pub(crate) lease_ttl_millis: u64,
    pub(crate) accepted_node_epoch: u64,
    pub(crate) event_high_watermark: u64,
    pub(crate) filesystem_lock_reclaim_required: bool,
}

/// gRPC Handler 可 clone 的轻量提交句柄。
#[derive(Clone)]
pub(crate) struct MetaHandle {
    command_tx: mpsc::Sender<QueuedMetaCommand>,
    metrics: MetaMetrics,
}

impl MetaHandle {
    #[cfg(test)]
    pub(crate) fn spawn() -> Self {
        Self::try_spawn(Box::<InMemoryJournal>::default())
            .expect("in-memory meta journal should restore")
    }

    #[cfg(test)]
    pub(crate) fn try_spawn(journal: Box<dyn MetadataJournal>) -> Result<Self, MetaRuntimeError> {
        let registry = dms_metrics::registry();
        let metrics = MetaMetrics::register(&registry)
            .map_err(|error| MetaRuntimeError::JournalUnavailable(error.to_string()))?;
        Self::try_spawn_with_metrics(journal, metrics)
    }

    #[cfg(test)]
    pub(crate) fn try_spawn_with_metrics(
        journal: Box<dyn MetadataJournal>,
        metrics: MetaMetrics,
    ) -> Result<Self, MetaRuntimeError> {
        Self::try_spawn_with_runtime_policy(
            journal,
            MetaCheckpointPolicy::default(),
            metrics,
            false,
        )
    }

    /// 接收进程已解析的 checkpoint 配置，不让状态 owner 再读 CLI/文件。
    /// 保留策略暂时沿用内部默认值；提高快照间隔不改变 WAL 的追加可靠性。
    #[cfg(test)]
    pub(crate) fn try_spawn_with_runtime_policy(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        metrics: MetaMetrics,
        trace_periodic_operations: bool,
    ) -> Result<Self, MetaRuntimeError> {
        Self::try_spawn_with_filesystem_policy(
            journal,
            checkpoint_policy,
            metrics,
            trace_periodic_operations,
            DEFAULT_FILESYSTEM_MAX_INODES,
        )
    }

    /// 在进程组合根解析 inode 上限；Meta owner 只接收最终值，不自行读取配置来源。
    pub(crate) fn try_spawn_with_filesystem_policy(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        metrics: MetaMetrics,
        trace_periodic_operations: bool,
        filesystem_max_inodes: u64,
    ) -> Result<Self, MetaRuntimeError> {
        Self::try_spawn_with_policies_and_metrics(
            journal,
            checkpoint_policy,
            MetaRetentionPolicy::default(),
            metrics,
            trace_periodic_operations,
            filesystem_max_inodes,
        )
    }

    #[cfg(test)]
    pub(crate) fn try_spawn_with_policies(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        retention_policy: MetaRetentionPolicy,
    ) -> Result<Self, MetaRuntimeError> {
        let registry = dms_metrics::registry();
        let metrics = MetaMetrics::register(&registry)
            .map_err(|error| MetaRuntimeError::JournalUnavailable(error.to_string()))?;
        Self::try_spawn_with_policies_and_metrics(
            journal,
            checkpoint_policy,
            retention_policy,
            metrics,
            false,
            DEFAULT_FILESYSTEM_MAX_INODES,
        )
    }

    fn try_spawn_with_policies_and_metrics(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        retention_policy: MetaRetentionPolicy,
        metrics: MetaMetrics,
        trace_periodic_operations: bool,
        filesystem_max_inodes: u64,
    ) -> Result<Self, MetaRuntimeError> {
        let (command_tx, command_rx) = mpsc::channel(META_MAILBOX_CAPACITY);
        let state = MetaState::try_new_with_filesystem_limit(
            journal,
            checkpoint_policy,
            retention_policy,
            metrics.clone(),
            filesystem_max_inodes,
        )?;
        tokio::spawn(run_meta(
            command_rx,
            state,
            metrics.clone(),
            trace_periodic_operations,
        ));
        Ok(Self {
            command_tx,
            metrics,
        })
    }

    pub(crate) async fn open_node_session(
        &self,
        node_id: u64,
        control_endpoint: String,
        supports_commit_sequence: bool,
    ) -> Result<NodeSessionGrant, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::OpenNodeSession,
            MetaCommand::OpenNodeSession {
                node_id,
                control_endpoint,
                supports_commit_sequence,
                reply,
            },
            receive,
        )
        .await
    }

    pub(crate) async fn heartbeat(
        &self,
        session_id: Vec<u8>,
        node_id: u64,
        node_epoch: u64,
        event_cursor: u64,
        resources: Option<pb::ResourceSummary>,
    ) -> Result<HeartbeatGrant, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::Heartbeat,
            MetaCommand::Heartbeat {
                session_id,
                node_id,
                node_epoch,
                event_cursor,
                resources,
                reply,
            },
            receive,
        )
        .await
    }

    pub(crate) async fn resolve_object(
        &self,
        request: pb::ResolveObjectRequest,
    ) -> Result<pb::ResolveObjectResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::ResolveObject,
            MetaCommand::ResolveObject { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn resolve_objects(
        &self,
        request: pb::ResolveObjectsRequest,
    ) -> Result<pb::ResolveObjectsResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::ResolveObject,
            MetaCommand::ResolveObjects { request, reply },
            receive,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn debug_resolve_object_without_session(
        &self,
        key: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<pb::ResolveObjectResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::ResolveObject,
            MetaCommand::DebugResolveObjectWithoutSession {
                key,
                exact_version,
                reply,
            },
            receive,
        )
        .await
    }

    pub(crate) async fn report_replicas(
        &self,
        request: pb::ReportReplicasRequest,
    ) -> Result<pb::ReportReplicasResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::ReportReplicas,
            MetaCommand::ReportReplicas { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn commit_version(
        &self,
        request: pb::CommitVersionRequest,
    ) -> Result<pb::CommitVersionResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        let result = self
            .complete(
                MetaOperation::CommitVersion,
                MetaCommand::CommitVersion { request, reply },
                receive,
            )
            .await;
        let outcome = match &result {
            Ok(response) if response.changed => CommitOutcome::Committed,
            Ok(_) => CommitOutcome::Idempotent,
            Err(MetaRuntimeError::Conflict { .. }) => CommitOutcome::Conflict,
            Err(_) => CommitOutcome::Rejected,
        };
        self.metrics.record_commit(outcome);
        result
    }

    pub(crate) async fn commit_batch(
        &self,
        request: pb::CommitBatchRequest,
    ) -> Result<pb::CommitBatchResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::CommitBatch,
            MetaCommand::CommitBatch { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn stat(
        &self,
        request: pb::MetaStatRequest,
    ) -> Result<pb::MetaStatResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::Stat,
            MetaCommand::Stat { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn scan(
        &self,
        request: pb::MetaScanRequest,
    ) -> Result<pb::MetaScanResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::Scan,
            MetaCommand::Scan { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn get_operation(
        &self,
        request: pb::GetOperationRequest,
    ) -> Result<pb::GetOperationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::GetOperation,
            MetaCommand::GetOperation { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn plan_replicas(
        &self,
        request: pb::PlanReplicasRequest,
    ) -> Result<pb::PlanReplicasResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::PlanReplicas,
            MetaCommand::PlanReplicas { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn watch_node_events(
        &self,
        request: pb::WatchNodeEventsRequest,
        sender: mpsc::Sender<pb::NodeEvent>,
    ) -> Result<(), MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::WatchNodeEvents,
            MetaCommand::WatchNodeEvents {
                request,
                sender,
                reply,
            },
            receive,
        )
        .await
    }

    pub(crate) async fn acknowledge_node_event(
        &self,
        request: pb::AcknowledgeNodeEventRequest,
    ) -> Result<(), MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::AcknowledgeNodeEvent,
            MetaCommand::AcknowledgeNodeEvent { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn acknowledge_block_retirement(
        &self,
        request: pb::AcknowledgeBlockRetirementRequest,
    ) -> Result<pb::AcknowledgeBlockRetirementResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::AcknowledgeNodeEvent,
            MetaCommand::AcknowledgeBlockRetirement { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_lookup(
        &self,
        request: pb::FilesystemLookupRequest,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemLookup,
            MetaCommand::FilesystemLookup { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_get_inode(
        &self,
        request: pb::FilesystemGetInodeRequest,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemGetInode,
            MetaCommand::FilesystemGetInode { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_create_inode(
        &self,
        request: pb::FilesystemCreateInodeRequest,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemCreateInode,
            MetaCommand::FilesystemCreateInode { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_create_symlink(
        &self,
        request: pb::FilesystemCreateSymlinkRequest,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemCreateSymlink,
            MetaCommand::FilesystemCreateSymlink { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_read_directory(
        &self,
        request: pb::FilesystemReadDirectoryRequest,
    ) -> Result<pb::FilesystemReadDirectoryResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemReadDirectory,
            MetaCommand::FilesystemReadDirectory { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_link_entry(
        &self,
        request: pb::FilesystemLinkRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemLinkEntry,
            MetaCommand::FilesystemLinkEntry { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_acquire_inode_reference(
        &self,
        request: pb::FilesystemInodeReferenceRequest,
    ) -> Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemAcquireInodeReference,
            MetaCommand::FilesystemAcquireInodeReference { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_release_inode_reference(
        &self,
        request: pb::FilesystemInodeReferenceRequest,
    ) -> Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemReleaseInodeReference,
            MetaCommand::FilesystemReleaseInodeReference { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_renew_inode_references(
        &self,
        request: pb::FilesystemRenewInodeReferencesRequest,
    ) -> Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemRenewInodeReferences,
            MetaCommand::FilesystemRenewInodeReferences { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_rename_entry(
        &self,
        request: pb::FilesystemRenameRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemRenameEntry,
            MetaCommand::FilesystemRenameEntry { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_remove_entry(
        &self,
        request: pb::FilesystemRemoveRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemRemoveEntry,
            MetaCommand::FilesystemRemoveEntry { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_commit_version(
        &self,
        request: pb::FilesystemCommitVersionRequest,
    ) -> Result<pb::FilesystemCommitVersionResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemCommitVersion,
            MetaCommand::FilesystemCommitVersion { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_set_attributes(
        &self,
        request: pb::FilesystemSetAttributesRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemSetAttributes,
            MetaCommand::FilesystemSetAttributes { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_get_xattr(
        &self,
        request: pb::FilesystemGetXattrRequest,
    ) -> Result<pb::FilesystemGetXattrResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemGetXattr,
            MetaCommand::FilesystemGetXattr { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_list_xattrs(
        &self,
        request: pb::FilesystemListXattrsRequest,
    ) -> Result<pb::FilesystemListXattrsResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemListXattrs,
            MetaCommand::FilesystemListXattrs { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_set_xattr(
        &self,
        request: pb::FilesystemSetXattrRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemSetXattr,
            MetaCommand::FilesystemSetXattr { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_remove_xattr(
        &self,
        request: pb::FilesystemRemoveXattrRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemRemoveXattr,
            MetaCommand::FilesystemRemoveXattr { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_stat(
        &self,
        request: pb::FilesystemStatRequest,
    ) -> Result<pb::FilesystemStatResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::FilesystemStat,
            MetaCommand::FilesystemStat { request, reply },
            receive,
        )
        .await
    }

    pub(crate) async fn filesystem_test_lock(
        &self,
        request: pb::FilesystemLockRequest,
    ) -> Result<pb::FilesystemLockResponse, MetaRuntimeError> {
        let inode = request.inode;
        let (reply, receive) = oneshot::channel();
        let outcome = self
            .complete(
                MetaOperation::FilesystemTestLock,
                MetaCommand::FilesystemTestLock { request, reply },
                receive,
            )
            .await?;
        Ok(lock_outcome_to_proto(inode, outcome, 0))
    }

    pub(crate) async fn filesystem_set_lock(
        &self,
        request: pb::FilesystemLockRequest,
    ) -> Result<pb::FilesystemLockResponse, MetaRuntimeError> {
        let inode = request.inode;
        let (reply, receive) = oneshot::channel();
        let dispatch = self
            .complete(
                MetaOperation::FilesystemSetLock,
                MetaCommand::FilesystemSetLock { request, reply },
                receive,
            )
            .await?;
        let mutation = match dispatch {
            FileLockDispatch::Ready(mutation) => mutation,
            FileLockDispatch::Waiting(receiver) => {
                receiver.await.map_err(|_| MetaRuntimeError::Unavailable)?
            }
        };
        Ok(lock_outcome_to_proto(
            inode,
            mutation.outcome,
            mutation.owner_revision,
        ))
    }

    pub(crate) async fn filesystem_cancel_lock_wait(
        &self,
        request: pb::FilesystemCancelLockWaitRequest,
    ) -> Result<pb::FilesystemLockMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        let affected = self
            .complete(
                MetaOperation::FilesystemCancelLockWait,
                MetaCommand::FilesystemCancelLockWait { request, reply },
                receive,
            )
            .await?;
        Ok(pb::FilesystemLockMutationResponse {
            affected,
            owner_revision: 0,
        })
    }

    pub(crate) async fn filesystem_release_lock_owner(
        &self,
        request: pb::FilesystemReleaseLockOwnerRequest,
    ) -> Result<pb::FilesystemLockMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        let receipt = self
            .complete(
                MetaOperation::FilesystemReleaseLockOwner,
                MetaCommand::FilesystemReleaseLockOwner { request, reply },
                receive,
            )
            .await?;
        Ok(pb::FilesystemLockMutationResponse {
            affected: receipt.released as u64,
            owner_revision: receipt.owner_revision,
        })
    }

    pub(crate) async fn filesystem_reclaim_locks(
        &self,
        request: pb::FilesystemReclaimLocksRequest,
    ) -> Result<pb::FilesystemLockMutationResponse, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        let affected = self
            .complete(
                MetaOperation::FilesystemReclaimLocks,
                MetaCommand::FilesystemReclaimLocks { request, reply },
                receive,
            )
            .await?;
        Ok(pb::FilesystemLockMutationResponse {
            affected,
            owner_revision: 0,
        })
    }

    #[cfg(test)]
    pub(crate) async fn stats(&self) -> Result<MetaStats, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.submit(MetaCommand::Stats { reply }).await?;
        receive_reply(receive).await
    }

    async fn submit(&self, command: MetaCommand) -> Result<(), MetaRuntimeError> {
        self.metrics.mailbox_enqueued();
        self.command_tx
            .send(QueuedMetaCommand {
                name: command.metric(),
                enqueued_at: Instant::now(),
                trace_context: dms_tracing::capture_current_context(),
                command,
            })
            .await
            .map_err(|_| {
                self.metrics.mailbox_send_failed();
                MetaRuntimeError::Unavailable
            })
    }

    async fn complete<T>(
        &self,
        operation: MetaOperation,
        command: MetaCommand,
        receiver: oneshot::Receiver<Result<T, MetaRuntimeError>>,
    ) -> Result<T, MetaRuntimeError> {
        let mut guard = self.metrics.begin_operation(operation);
        self.submit(command).await?;
        let result = receive_reply(receiver).await;
        if result.is_ok() {
            guard.success();
        }
        result
    }
}

async fn receive_reply<T>(
    receiver: oneshot::Receiver<Result<T, MetaRuntimeError>>,
) -> Result<T, MetaRuntimeError> {
    receiver.await.map_err(|_| MetaRuntimeError::Unavailable)?
}

/// `F_SETLKW` 的两阶段 actor 结果。Meta actor 只登记 waiter 并立即返回 Receiver；
/// 真正等待发生在 gRPC Handler 对应的 task，绝不占住唯一状态 owner。
enum FileLockDispatch {
    Ready(FileLockMutation),
    Waiting(oneshot::Receiver<FileLockMutation>),
}

enum MetaCommand {
    OpenNodeSession {
        node_id: u64,
        control_endpoint: String,
        supports_commit_sequence: bool,
        reply: oneshot::Sender<Result<NodeSessionGrant, MetaRuntimeError>>,
    },
    Heartbeat {
        session_id: Vec<u8>,
        node_id: u64,
        node_epoch: u64,
        event_cursor: u64,
        resources: Option<pb::ResourceSummary>,
        reply: oneshot::Sender<Result<HeartbeatGrant, MetaRuntimeError>>,
    },
    ResolveObject {
        request: pb::ResolveObjectRequest,
        reply: oneshot::Sender<Result<pb::ResolveObjectResponse, MetaRuntimeError>>,
    },
    ResolveObjects {
        request: pb::ResolveObjectsRequest,
        reply: oneshot::Sender<Result<pb::ResolveObjectsResponse, MetaRuntimeError>>,
    },
    #[cfg(test)]
    DebugResolveObjectWithoutSession {
        key: Vec<u8>,
        exact_version: Option<u64>,
        reply: oneshot::Sender<Result<pb::ResolveObjectResponse, MetaRuntimeError>>,
    },
    ReportReplicas {
        request: pb::ReportReplicasRequest,
        reply: oneshot::Sender<Result<pb::ReportReplicasResponse, MetaRuntimeError>>,
    },
    CommitVersion {
        request: pb::CommitVersionRequest,
        reply: oneshot::Sender<Result<pb::CommitVersionResponse, MetaRuntimeError>>,
    },
    CommitBatch {
        request: pb::CommitBatchRequest,
        reply: oneshot::Sender<Result<pb::CommitBatchResponse, MetaRuntimeError>>,
    },
    Stat {
        request: pb::MetaStatRequest,
        reply: oneshot::Sender<Result<pb::MetaStatResponse, MetaRuntimeError>>,
    },
    Scan {
        request: pb::MetaScanRequest,
        reply: oneshot::Sender<Result<pb::MetaScanResponse, MetaRuntimeError>>,
    },
    GetOperation {
        request: pb::GetOperationRequest,
        reply: oneshot::Sender<Result<pb::GetOperationResponse, MetaRuntimeError>>,
    },
    PlanReplicas {
        request: pb::PlanReplicasRequest,
        reply: oneshot::Sender<Result<pb::PlanReplicasResponse, MetaRuntimeError>>,
    },
    WatchNodeEvents {
        request: pb::WatchNodeEventsRequest,
        sender: mpsc::Sender<pb::NodeEvent>,
        reply: oneshot::Sender<Result<(), MetaRuntimeError>>,
    },
    AcknowledgeNodeEvent {
        request: pb::AcknowledgeNodeEventRequest,
        reply: oneshot::Sender<Result<(), MetaRuntimeError>>,
    },
    AcknowledgeBlockRetirement {
        request: pb::AcknowledgeBlockRetirementRequest,
        reply: oneshot::Sender<Result<pb::AcknowledgeBlockRetirementResponse, MetaRuntimeError>>,
    },
    FilesystemLookup {
        request: pb::FilesystemLookupRequest,
        reply: oneshot::Sender<Result<pb::FilesystemResolveResponse, MetaRuntimeError>>,
    },
    FilesystemGetInode {
        request: pb::FilesystemGetInodeRequest,
        reply: oneshot::Sender<Result<pb::FilesystemResolveResponse, MetaRuntimeError>>,
    },
    FilesystemCreateInode {
        request: pb::FilesystemCreateInodeRequest,
        reply: oneshot::Sender<Result<pb::FilesystemResolveResponse, MetaRuntimeError>>,
    },
    FilesystemCreateSymlink {
        request: pb::FilesystemCreateSymlinkRequest,
        reply: oneshot::Sender<Result<pb::FilesystemResolveResponse, MetaRuntimeError>>,
    },
    FilesystemReadDirectory {
        request: pb::FilesystemReadDirectoryRequest,
        reply: oneshot::Sender<Result<pb::FilesystemReadDirectoryResponse, MetaRuntimeError>>,
    },
    FilesystemLinkEntry {
        request: pb::FilesystemLinkRequest,
        reply: oneshot::Sender<Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError>>,
    },
    FilesystemAcquireInodeReference {
        request: pb::FilesystemInodeReferenceRequest,
        reply: oneshot::Sender<Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError>>,
    },
    FilesystemRenewInodeReferences {
        request: pb::FilesystemRenewInodeReferencesRequest,
        reply: oneshot::Sender<Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError>>,
    },
    FilesystemReleaseInodeReference {
        request: pb::FilesystemInodeReferenceRequest,
        reply: oneshot::Sender<Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError>>,
    },
    FilesystemRenameEntry {
        request: pb::FilesystemRenameRequest,
        reply: oneshot::Sender<Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError>>,
    },
    FilesystemRemoveEntry {
        request: pb::FilesystemRemoveRequest,
        reply: oneshot::Sender<Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError>>,
    },
    FilesystemCommitVersion {
        request: pb::FilesystemCommitVersionRequest,
        reply: oneshot::Sender<Result<pb::FilesystemCommitVersionResponse, MetaRuntimeError>>,
    },
    FilesystemSetAttributes {
        request: pb::FilesystemSetAttributesRequest,
        reply: oneshot::Sender<Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError>>,
    },
    FilesystemGetXattr {
        request: pb::FilesystemGetXattrRequest,
        reply: oneshot::Sender<Result<pb::FilesystemGetXattrResponse, MetaRuntimeError>>,
    },
    FilesystemListXattrs {
        request: pb::FilesystemListXattrsRequest,
        reply: oneshot::Sender<Result<pb::FilesystemListXattrsResponse, MetaRuntimeError>>,
    },
    FilesystemSetXattr {
        request: pb::FilesystemSetXattrRequest,
        reply: oneshot::Sender<Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError>>,
    },
    FilesystemRemoveXattr {
        request: pb::FilesystemRemoveXattrRequest,
        reply: oneshot::Sender<Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError>>,
    },
    FilesystemStat {
        request: pb::FilesystemStatRequest,
        reply: oneshot::Sender<Result<pb::FilesystemStatResponse, MetaRuntimeError>>,
    },
    FilesystemTestLock {
        request: pb::FilesystemLockRequest,
        reply: oneshot::Sender<Result<FileLockOutcome, MetaRuntimeError>>,
    },
    FilesystemSetLock {
        request: pb::FilesystemLockRequest,
        reply: oneshot::Sender<Result<FileLockDispatch, MetaRuntimeError>>,
    },
    FilesystemCancelLockWait {
        request: pb::FilesystemCancelLockWaitRequest,
        reply: oneshot::Sender<Result<u64, MetaRuntimeError>>,
    },
    FilesystemReleaseLockOwner {
        request: pb::FilesystemReleaseLockOwnerRequest,
        reply: oneshot::Sender<Result<ReleaseLockReceipt, MetaRuntimeError>>,
    },
    FilesystemReclaimLocks {
        request: pb::FilesystemReclaimLocksRequest,
        reply: oneshot::Sender<Result<u64, MetaRuntimeError>>,
    },
    #[cfg(test)]
    Stats {
        reply: oneshot::Sender<Result<MetaStats, MetaRuntimeError>>,
    },
}

struct QueuedMetaCommand {
    name: MetaOperation,
    enqueued_at: Instant,
    /// Captured before crossing the actor mailbox; no business protobuf change.
    trace_context: dms_tracing::TraceContext,
    command: MetaCommand,
}

impl MetaCommand {
    fn metric(&self) -> MetaOperation {
        match self {
            Self::OpenNodeSession { .. } => MetaOperation::OpenNodeSession,
            Self::Heartbeat { .. } => MetaOperation::Heartbeat,
            Self::ResolveObject { .. } => MetaOperation::ResolveObject,
            Self::ResolveObjects { .. } => MetaOperation::ResolveObject,
            #[cfg(test)]
            Self::DebugResolveObjectWithoutSession { .. } => MetaOperation::ResolveObject,
            Self::ReportReplicas { .. } => MetaOperation::ReportReplicas,
            Self::CommitVersion { .. } => MetaOperation::CommitVersion,
            Self::CommitBatch { .. } => MetaOperation::CommitBatch,
            Self::Stat { .. } => MetaOperation::Stat,
            Self::Scan { .. } => MetaOperation::Scan,
            Self::GetOperation { .. } => MetaOperation::GetOperation,
            Self::PlanReplicas { .. } => MetaOperation::PlanReplicas,
            Self::WatchNodeEvents { .. } => MetaOperation::WatchNodeEvents,
            Self::AcknowledgeNodeEvent { .. } => MetaOperation::AcknowledgeNodeEvent,
            Self::AcknowledgeBlockRetirement { .. } => MetaOperation::AcknowledgeNodeEvent,
            Self::FilesystemLookup { .. } => MetaOperation::FilesystemLookup,
            Self::FilesystemGetInode { .. } => MetaOperation::FilesystemGetInode,
            Self::FilesystemCreateInode { .. } => MetaOperation::FilesystemCreateInode,
            Self::FilesystemCreateSymlink { .. } => MetaOperation::FilesystemCreateSymlink,
            Self::FilesystemReadDirectory { .. } => MetaOperation::FilesystemReadDirectory,
            Self::FilesystemLinkEntry { .. } => MetaOperation::FilesystemLinkEntry,
            Self::FilesystemAcquireInodeReference { .. } => {
                MetaOperation::FilesystemAcquireInodeReference
            }
            Self::FilesystemRenewInodeReferences { .. } => {
                MetaOperation::FilesystemRenewInodeReferences
            }
            Self::FilesystemReleaseInodeReference { .. } => {
                MetaOperation::FilesystemReleaseInodeReference
            }
            Self::FilesystemRenameEntry { .. } => MetaOperation::FilesystemRenameEntry,
            Self::FilesystemRemoveEntry { .. } => MetaOperation::FilesystemRemoveEntry,
            Self::FilesystemCommitVersion { .. } => MetaOperation::FilesystemCommitVersion,
            Self::FilesystemSetAttributes { .. } => MetaOperation::FilesystemSetAttributes,
            Self::FilesystemGetXattr { .. } => MetaOperation::FilesystemGetXattr,
            Self::FilesystemListXattrs { .. } => MetaOperation::FilesystemListXattrs,
            Self::FilesystemSetXattr { .. } => MetaOperation::FilesystemSetXattr,
            Self::FilesystemRemoveXattr { .. } => MetaOperation::FilesystemRemoveXattr,
            Self::FilesystemStat { .. } => MetaOperation::FilesystemStat,
            Self::FilesystemTestLock { .. } => MetaOperation::FilesystemTestLock,
            Self::FilesystemSetLock { .. } => MetaOperation::FilesystemSetLock,
            Self::FilesystemCancelLockWait { .. } => MetaOperation::FilesystemCancelLockWait,
            Self::FilesystemReleaseLockOwner { .. } => MetaOperation::FilesystemReleaseLockOwner,
            Self::FilesystemReclaimLocks { .. } => MetaOperation::FilesystemReclaimLocks,
            #[cfg(test)]
            Self::Stats { .. } => MetaOperation::Stats,
        }
    }
}

#[derive(Clone)]
struct NodeSession {
    session_id: Vec<u8>,
    node_epoch: u64,
    control_endpoint: String,
    last_acked_cursor: u64,
    last_heartbeat: Option<Instant>,
    resources: Option<pb::ResourceSummary>,
    supports_commit_sequence: bool,
    commit_sequence_floor: u64,
    commit_sequences: HashMap<u64, CommitSequenceRecord>,
}

struct NodeWatcher {
    sender: mpsc::Sender<pb::NodeEvent>,
    delivered_cursor: u64,
    replay_through: u64,
    next_gc_retry_at: Option<Instant>,
    last_gc_retry_cursor: u64,
    disconnected: bool,
}

#[derive(Clone)]
struct StoredReplica {
    location: pb::ReplicaLocation,
    catalog_revision: u64,
    length: u64,
}

#[derive(Clone)]
struct StoredOperation {
    digest: Vec<u8>,
    result: pb::CommitVersionResponse,
    commit_sequence: Option<CommitSequenceRecord>,
    /// Invalidation event that must be acknowledged before this operation may
    /// be returned as fully visible. `None` means the visibility barrier ended.
    visibility_cursor: Option<u64>,
}

#[derive(Clone)]
struct StoredFilesystemNamespaceOperation {
    digest: Vec<u8>,
    result: NamespaceMutationResult,
    /// Namespace 失效事件在所有受影响 Node 可见前保持 Some。
    /// 同一个 OperationId 重试时仍必须重新进入可见性屏障，不能因为 WAL 已提交就提前返回。
    visibility_cursor: Option<u64>,
}

#[derive(Clone)]
struct StoredFilesystemVersionOperation {
    digest: Vec<u8>,
    /// 原始文件 commit 响应。OperationId 重试必须返回这次提交当时的 inode 绑定，
    /// 不能重新读取 current inode，否则 op1 响应丢失、op2 成功后 op1 重试会错返 op2。
    response: pb::FilesystemCommitVersionResponse,
    /// 文件内容绑定失效事件在受影响 Node 可见前保持 Some。
    /// 精确响应与该 cursor 必须一起保存、恢复和回收。
    visibility_cursor: Option<u64>,
}

#[derive(Clone)]
struct StoredFilesystemAttributeOperation {
    digest: Vec<u8>,
    /// 精确保存首次提交产生的响应，OperationId 重试不能重新读取 current inode。
    response: pb::FilesystemAttributeMutationResponse,
    /// 属性缓存失效事件在所有受影响 Node 可见前保持 Some。
    visibility_cursor: Option<u64>,
}

struct FilesystemNamespaceDispatch<T> {
    operation_id: Vec<u8>,
    response: T,
    event_cursor: Option<u64>,
    waiting_nodes: HashSet<u64>,
    waiting_prior_lease_nodes: HashSet<u64>,
}

struct CommitDispatch {
    operation_ids: Vec<Vec<u8>>,
    response: pb::CommitVersionResponse,
    event_cursor: Option<u64>,
    /// 当前失效事件必须由这些 Node 的 Watch ACK 证明已处理。
    waiting_nodes: HashSet<u64>,
    /// Meta 恢复/Node 换代前已经发出去的本地缓存租约，需要等真实租约到期。
    /// 它和 Watch ACK 是两类独立条件：ACK 不能缩短旧 lease，lease 到期也不能代替 ACK。
    waiting_prior_lease_nodes: HashSet<u64>,
}

struct FilesystemCommitDispatch {
    operation_id: Vec<u8>,
    response: pb::FilesystemCommitVersionResponse,
    event_cursor: Option<u64>,
    waiting_nodes: HashSet<u64>,
    waiting_prior_lease_nodes: HashSet<u64>,
}

struct FilesystemAttributeDispatch {
    operation_id: Vec<u8>,
    response: pb::FilesystemAttributeMutationResponse,
    event_cursor: Option<u64>,
    waiting_nodes: HashSet<u64>,
    waiting_prior_lease_nodes: HashSet<u64>,
}

struct PendingCommit {
    operation_ids: Vec<Vec<u8>>,
    response: PendingResponse,
    /// 当前失效事件 ACK 尚未满足的 Node。
    waiting_nodes: HashSet<u64>,
    /// 恢复前旧租约尚未自然到期的 Node。
    waiting_prior_lease_nodes: HashSet<u64>,
    reply: PendingReply,
}

#[derive(Clone)]
struct PendingRepair {
    repair_id: Vec<u8>,
    target_node_id: u64,
    target_node_epoch: u64,
    event_cursor: u64,
    issued_at: Instant,
}

#[derive(Clone)]
struct PendingRetirement {
    record: BlockRetirementRecord,
    prepared: HashSet<(u64, u64)>,
    released: HashSet<(u64, u64)>,
    final_sent: bool,
}

enum PendingResponse {
    Single(pb::CommitVersionResponse),
    Batch(pb::CommitBatchResponse),
    Filesystem(Box<pb::FilesystemCommitVersionResponse>),
    FilesystemCreate(Box<pb::FilesystemResolveResponse>),
    FilesystemNamespace(Box<pb::FilesystemNamespaceMutationResponse>),
    FilesystemAttributes(Box<pb::FilesystemAttributeMutationResponse>),
}

enum PendingReply {
    Single(oneshot::Sender<Result<pb::CommitVersionResponse, MetaRuntimeError>>),
    Batch(oneshot::Sender<Result<pb::CommitBatchResponse, MetaRuntimeError>>),
    Filesystem(oneshot::Sender<Result<pb::FilesystemCommitVersionResponse, MetaRuntimeError>>),
    FilesystemCreate(oneshot::Sender<Result<pb::FilesystemResolveResponse, MetaRuntimeError>>),
    FilesystemNamespace(
        oneshot::Sender<Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError>>,
    ),
    FilesystemAttributes(
        oneshot::Sender<Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError>>,
    ),
}

struct MetaState {
    /// 退休仅以租约边界为依据；进程恢复后先保守等待一个完整旧租约窗口。
    recovery_lease_until: Instant,
    retired_sessions: HashSet<u64>,
    prior_lease_deadlines: HashMap<u64, Instant>,
    next_session: u64,
    node_epochs: HashMap<u64, u64>,
    node_commit_sequence_floors: HashMap<u64, u64>,
    sessions: HashMap<u64, NodeSession>,
    replicas: HashMap<Vec<u8>, Vec<StoredReplica>>,
    /// Explicit policy target per immutable Block. This must not be derived
    /// from historical locations: replacement replicas would otherwise ratchet
    /// a two-copy policy to three, four, ... after successive failures.
    desired_replica_counts: HashMap<Vec<u8>, u32>,
    versions: HashMap<Vec<u8>, BTreeMap<u64, pb::VersionLayout>>,
    version_floor: u64,
    version_modified_times: HashMap<(Vec<u8>, u64), i64>,
    pending_retirements: HashMap<Vec<u8>, PendingRetirement>,
    retired_block_fences: HashMap<Vec<u8>, u64>,
    /// 当前可见 VALUE key 的二进制有序索引；Scan 只走这里分页，不每页全量排序。
    live_keys: BTreeSet<Vec<u8>>,
    operations: HashMap<Vec<u8>, StoredOperation>,
    filesystem_namespace_operations: HashMap<Vec<u8>, StoredFilesystemNamespaceOperation>,
    filesystem_version_operations: HashMap<Vec<u8>, StoredFilesystemVersionOperation>,
    filesystem_attribute_operations: HashMap<Vec<u8>, StoredFilesystemAttributeOperation>,
    /// Meta 当前知道的跨 Node inode 生命周期引用。
    ///
    /// 这里不保存精确计数；Node 在本地 0→1 时建立租约，非 orphan 的 1→0 通过停止
    /// heartbeat 续租来释放，orphan 的 1→0 则立即发送 release。Meta 只需要知道某个
    /// node epoch 是否仍可能持有 open/lookup 引用，从而防止 orphan 被过早回收。
    filesystem_inode_references: HashMap<(u64, u64, u64), (u64, Instant)>,
    replica_operations: HashMap<Vec<u8>, pb::ReportReplicasResponse>,
    events: Vec<pb::NodeEvent>,
    event_high_watermark: u64,
    watchers: HashMap<u64, NodeWatcher>,
    pending_commits: BTreeMap<u64, Vec<PendingCommit>>,
    /// One in-flight repair attempt per immutable Block. The attempt binds a
    /// repair id to one target incarnation and expires independently of the
    /// durable replica policy, so a dead/stuck target cannot suppress repair
    /// forever. The state can be rebuilt from retained repair events after a
    /// snapshot restore; the replica catalog remains authoritative.
    pending_repairs: HashMap<Vec<u8>, PendingRepair>,
    filesystem: FilesystemCatalog,
    /// POSIX 锁是 Meta 权威的易失协调状态；它与 namespace 共用同一个 actor，
    /// 但不写入内容 WAL。Meta 重启时由存活 Node 在旧租约窗口内重报。
    filesystem_locks: FilesystemLockTable,
    /// 仅在 Node 资源快照实际变化时推进；statfs 使用它标识同一容量视图。
    capacity_revision: u64,
    filesystem_max_inodes: u64,
    journal: Box<dyn MetadataJournal>,
    last_applied_index: u64,
    last_checkpoint_index: u64,
    retention_boundary_dirty: bool,
    checkpoint_policy: MetaCheckpointPolicy,
    retention_policy: MetaRetentionPolicy,
    metrics: MetaMetrics,
}

impl MetaState {
    #[cfg(test)]
    fn new(journal: Box<dyn MetadataJournal>) -> Self {
        Self::with_policies(
            journal,
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy::default(),
        )
    }

    #[cfg(test)]
    fn with_policies(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        retention_policy: MetaRetentionPolicy,
    ) -> Self {
        let registry = dms_metrics::registry();
        let metrics = MetaMetrics::register(&registry).expect("test Meta metrics");
        Self::try_new(journal, checkpoint_policy, retention_policy, metrics)
            .expect("in-memory meta journal should restore")
    }

    #[cfg(test)]
    fn try_new(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        retention_policy: MetaRetentionPolicy,
        metrics: MetaMetrics,
    ) -> Result<Self, MetaRuntimeError> {
        Self::try_new_with_filesystem_limit(
            journal,
            checkpoint_policy,
            retention_policy,
            metrics,
            DEFAULT_FILESYSTEM_MAX_INODES,
        )
    }

    fn try_new_with_filesystem_limit(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        retention_policy: MetaRetentionPolicy,
        metrics: MetaMetrics,
        filesystem_max_inodes: u64,
    ) -> Result<Self, MetaRuntimeError> {
        if filesystem_max_inodes == 0 {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem max inodes must be positive".to_string(),
            ));
        }
        let recovery_started = Instant::now();
        let snapshot = journal.load_snapshot().map_err(map_journal_error)?;
        let replay_after = snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.last_applied_index);
        let entries = journal
            .load_after(replay_after)
            .map_err(map_journal_error)?;
        let mut state = Self {
            next_session: 1,
            node_epochs: HashMap::new(),
            node_commit_sequence_floors: HashMap::new(),
            sessions: HashMap::new(),
            replicas: HashMap::new(),
            desired_replica_counts: HashMap::new(),
            versions: HashMap::new(),
            version_floor: 0,
            version_modified_times: HashMap::new(),
            pending_retirements: HashMap::new(),
            retired_block_fences: HashMap::new(),
            live_keys: BTreeSet::new(),
            operations: HashMap::new(),
            filesystem_namespace_operations: HashMap::new(),
            filesystem_version_operations: HashMap::new(),
            filesystem_attribute_operations: HashMap::new(),
            filesystem_inode_references: HashMap::new(),
            replica_operations: HashMap::new(),
            events: Vec::new(),
            event_high_watermark: 0,
            watchers: HashMap::new(),
            recovery_lease_until: Instant::now() + DEFAULT_NODE_LEASE_TTL,
            retired_sessions: HashSet::new(),
            prior_lease_deadlines: HashMap::new(),
            pending_commits: BTreeMap::new(),
            pending_repairs: HashMap::new(),
            filesystem: FilesystemCatalog::default(),
            filesystem_locks: FilesystemLockTable::default(),
            capacity_revision: 0,
            filesystem_max_inodes,
            journal,
            last_applied_index: 0,
            last_checkpoint_index: replay_after,
            retention_boundary_dirty: false,
            checkpoint_policy,
            retention_policy,
            metrics: metrics.clone(),
        };
        if let Some(snapshot) = snapshot {
            state.restore_snapshot(snapshot);
        }
        // 恢复与在线提交共用 apply_record，避免形成第二套状态转换逻辑。
        let recovery_records = entries.len();
        for entry in entries {
            state.apply_record(entry.sequence, entry.record);
        }
        for (node_id, session) in &mut state.sessions {
            // WAL/snapshot 重放只能证明这个 session 曾经存在，不能证明对应
            // Node 当前仍然可达。必须等同 epoch 的 heartbeat 到达后才重新
            // 进入 live 集合，避免 Meta 重启后返回 ghost replica。
            session.last_heartbeat = None;
            // 快照仅保存当前 incarnation，旧 incarnation 的缓存租约不可从快照排除。
            // 对所有恢复节点（包括本次写入源）重建完整保守窗口，ACK 不能提前缩短它。
            if !state.prior_lease_deadlines.contains_key(node_id) {
                state
                    .prior_lease_deadlines
                    .insert(*node_id, state.recovery_lease_until);
            }
        }
        state
            .filesystem_locks
            .begin_recovery(state.recovery_lease_until, state.sessions.keys().copied());
        state.rebuild_pending_repairs();
        state.enforce_retention();
        metrics.record_recovery(recovery_records, recovery_started.elapsed());
        state.refresh_metrics();
        Ok(state)
    }

    fn verify_session(&self, session: &pb::NodeSessionIdentity) -> Result<(), MetaRuntimeError> {
        let current = self
            .sessions
            .get(&session.node_id)
            .ok_or(MetaRuntimeError::UnknownSession)?;
        if current.session_id != session.session_id || current.node_epoch != session.node_epoch {
            return Err(MetaRuntimeError::UnknownSession);
        }
        Ok(())
    }

    fn remember_prior_lease(&mut self, node_id: u64, until: Instant) {
        // prior_lease_deadlines 只表达“旧缓存租约还可能存在到什么时候”。
        // 它不是心跳活性状态：同 incarnation heartbeat 只能证明 Node 还活着，
        // 不能证明 Meta 恢复前已经发给 Client 的旧 Current cache 全部失效。
        self.prior_lease_deadlines
            .entry(node_id)
            .and_modify(|old| *old = (*old).max(until))
            .or_insert(until);
    }

    fn open_node_session(
        &mut self,
        node_id: u64,
        control_endpoint: String,
        supports_commit_sequence: bool,
    ) -> Result<NodeSessionGrant, MetaRuntimeError> {
        if node_id == 0 || control_endpoint.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "node id and control endpoint are required".to_string(),
            ));
        }
        // 同一 Node 打开新 incarnation 时，旧进程即使稍后发来 unlock 也不能影响
        // 新锁；在同一个 actor turn 内先建立恢复屏障，再按精确 epoch 清理旧锁。
        // 新 session 必须重报完整锁镜像（可以为空）后，其他 Node 才能继续取锁。
        if let Some(previous) = self.sessions.get(&node_id) {
            self.filesystem_locks
                .begin_node_recovery(node_id, Instant::now() + DEFAULT_NODE_LEASE_TTL);
            self.filesystem_locks
                .release_node_epoch(node_id, previous.node_epoch);
        }
        let node_epoch = self.node_epochs.entry(node_id).or_default();
        let next_epoch = *node_epoch + 1;
        let session_id = self.next_session.to_be_bytes().to_vec();
        let next_session = self.next_session + 1;
        let global_floor = *self.node_commit_sequence_floors.get(&node_id).unwrap_or(&0);
        let commit_sequence_floor = self.sessions.get(&node_id).map_or(global_floor, |session| {
            // 新 incarnation 不继承旧 session_id，但必须继承同一 Node 已经进入
            // Meta 的 commit_sequence 上界；否则 operation 结果裁剪前的乱序窗口
            // 会在 reopen 后被旧 Commit(new_replicas) 重放。
            let seen_max = session
                .commit_sequences
                .keys()
                .copied()
                .max()
                .unwrap_or(session.commit_sequence_floor);
            global_floor
                .max(session.commit_sequence_floor)
                .max(seen_max)
        });
        let record = JournalRecord::NodeSessionOpened {
            node_id,
            node_epoch: next_epoch,
            session_id: session_id.clone(),
            control_endpoint,
            next_session,
            supports_commit_sequence,
            commit_sequence_floor,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        let session = self
            .sessions
            .get(&node_id)
            .expect("session was just applied");
        Ok(NodeSessionGrant {
            session_id: session.session_id.clone(),
            node_id,
            node_epoch: session.node_epoch,
            heartbeat_interval_millis: 10_000,
            lease_ttl_millis: 30_000,
            minimum_commit_sequence: session.commit_sequence_floor + 1,
        })
    }

    fn heartbeat_with_resources(
        &mut self,
        session_id: Vec<u8>,
        node_id: u64,
        node_epoch: u64,
        _event_cursor: u64,
        resources: Option<pb::ResourceSummary>,
    ) -> Result<HeartbeatGrant, MetaRuntimeError> {
        self.verify_session(&pb::NodeSessionIdentity {
            session_id: session_id.clone(),
            node_id,
            node_epoch,
        })?;
        if let Some(session) = self.sessions.get_mut(&node_id) {
            session.last_heartbeat = Some(Instant::now());
            if session.resources != resources {
                session.resources = resources;
                self.capacity_revision = self.capacity_revision.saturating_add(1);
            }
        }
        self.retired_sessions.remove(&node_id);
        self.schedule_repairs();
        // 文件锁本身是易失协调状态，不写 WAL。Meta 恢复后保留原 session 语义，
        // 但通过 heartbeat 明确要求各 Node 重报完整本地锁镜像；正常运行期该位为
        // false，不增加额外 RPC。
        let filesystem_lock_reclaim_required = self
            .filesystem_locks
            .reclaim_required(node_id, Instant::now());
        Ok(HeartbeatGrant {
            lease_ttl_millis: 30_000,
            accepted_node_epoch: node_epoch,
            event_high_watermark: self.event_high_watermark,
            filesystem_lock_reclaim_required,
        })
    }

    #[cfg(test)]
    fn heartbeat(
        &mut self,
        session_id: Vec<u8>,
        node_id: u64,
        node_epoch: u64,
        event_cursor: u64,
    ) -> Result<HeartbeatGrant, MetaRuntimeError> {
        self.heartbeat_with_resources(session_id, node_id, node_epoch, event_cursor, None)
    }

    fn resolve_object(
        &self,
        request: pb::ResolveObjectRequest,
    ) -> Result<pb::ResolveObjectResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let key = request
            .key
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing key".to_string()))?;
        let versions = self
            .versions
            .get(&key.value)
            .ok_or(MetaRuntimeError::NotFound)?;
        let exact_version = match request.selector.as_ref() {
            Some(pb::resolve_object_request::Selector::ExactVersion(version)) => Some(*version),
            _ => None,
        };
        let layout = match exact_version {
            Some(version) => versions.get(&version),
            _ => versions.last_key_value().map(|(_, value)| value),
        }
        .ok_or(MetaRuntimeError::NotFound)?;
        if layout.kind == pb::VersionKind::Tombstone as i32 {
            return Err(MetaRuntimeError::NotFound);
        }
        let block_replicas = self.block_replicas_for_layout(layout);
        let current_lease = self.current_lease_grant(&request, layout);
        Ok(pb::ResolveObjectResponse {
            layout: Some(layout.clone()),
            block_replicas,
            current_lease,
        })
    }

    #[cfg(test)]
    fn debug_resolve_object_without_session(
        &self,
        key: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<pb::ResolveObjectResponse, MetaRuntimeError> {
        let versions = self.versions.get(&key).ok_or(MetaRuntimeError::NotFound)?;
        let layout = match exact_version {
            Some(version) => versions.get(&version),
            _ => versions.last_key_value().map(|(_, value)| value),
        }
        .ok_or(MetaRuntimeError::NotFound)?;
        if layout.kind == pb::VersionKind::Tombstone as i32 {
            return Err(MetaRuntimeError::NotFound);
        }
        Ok(pb::ResolveObjectResponse {
            layout: Some(layout.clone()),
            block_replicas: self.block_replicas_for_layout(layout),
            current_lease: None,
        })
    }

    fn block_replicas_for_layout(&self, layout: &pb::VersionLayout) -> Vec<pb::BlockReplicaSet> {
        layout
            .extents
            .iter()
            .filter(|extent| !is_filesystem_sparse_hole(extent))
            .map(|extent| pb::BlockReplicaSet {
                block_id: extent.block_id.clone(),
                replicas: self
                    .replicas
                    .get(&extent.block_id)
                    .map(|items| {
                        items
                            .iter()
                            .filter(|item| self.is_live_replica(item))
                            .map(|item| item.location.clone())
                            .collect()
                    })
                    .unwrap_or_default(),
                length: self
                    .replicas
                    .get(&extent.block_id)
                    .and_then(|items| items.first())
                    .map_or(0, |item| item.length),
                proofs: self
                    .replicas
                    .get(&extent.block_id)
                    .map(|items| {
                        items
                            .iter()
                            .filter(|item| self.is_live_replica(item))
                            .map(|item| pb::ReplicaProof {
                                block_id: item.location.block_id.clone(),
                                node_id: item.location.node_id,
                                node_epoch: item.location.node_epoch,
                                catalog_revision: item.catalog_revision,
                                checksum: item.location.checksum.clone(),
                                durability: item.location.durability,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect()
    }

    fn resolve_objects(
        &self,
        request: pb::ResolveObjectsRequest,
    ) -> Result<pb::ResolveObjectsResponse, MetaRuntimeError> {
        if request.requests.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "ResolveObjects requests are empty".to_string(),
            ));
        }
        let mut results = Vec::with_capacity(request.requests.len());
        for request in request.requests {
            match self.resolve_object(request) {
                Ok(response) => results.push(pb::ResolveObjectResult {
                    response: Some(response),
                }),
                Err(MetaRuntimeError::NotFound) => {
                    results.push(pb::ResolveObjectResult { response: None });
                }
                Err(error) => return Err(error),
            }
        }
        Ok(pb::ResolveObjectsResponse { results })
    }

    fn filesystem_lock_request(
        &self,
        request: &pb::FilesystemLockRequest,
    ) -> Result<crate::filesystem::FileLockRequest, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        if request.request_id == 0 || request.inode == 0 || request.lock_owner == 0 {
            return Err(MetaRuntimeError::InvalidArgument(
                "lock request id, inode and owner must be non-zero".to_string(),
            ));
        }
        lock_request_from_proto(request, session).map_err(|error| {
            MetaRuntimeError::InvalidArgument(format!("invalid filesystem lock: {error:?}"))
        })
    }

    fn filesystem_test_lock(
        &mut self,
        request: pb::FilesystemLockRequest,
    ) -> Result<FileLockOutcome, MetaRuntimeError> {
        let request = self.filesystem_lock_request(&request)?;
        Ok(self.filesystem_locks.test(request, Instant::now()))
    }

    fn filesystem_set_lock(
        &mut self,
        request: pb::FilesystemLockRequest,
    ) -> Result<FileLockDispatch, MetaRuntimeError> {
        let request = self.filesystem_lock_request(&request)?;
        let (reply, receive) = oneshot::channel();
        match self
            .filesystem_locks
            .set(request, request.wait.then_some(reply), Instant::now())
        {
            SetLockResult::Ready(outcome) => Ok(FileLockDispatch::Ready(outcome)),
            SetLockResult::Waiting => Ok(FileLockDispatch::Waiting(receive)),
            SetLockResult::QueueFull | SetLockResult::ReplyQueueFull => {
                Err(MetaRuntimeError::Unavailable)
            }
            SetLockResult::StaleRequestId => Err(MetaRuntimeError::InvalidArgument(
                "filesystem lock request_id is outside the retry window".to_string(),
            )),
            SetLockResult::RequestMismatch => Err(MetaRuntimeError::InvalidArgument(
                "filesystem lock request_id was reused with a different request".to_string(),
            )),
        }
    }

    fn filesystem_cancel_lock_wait(
        &mut self,
        request: pb::FilesystemCancelLockWaitRequest,
    ) -> Result<u64, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let owner = FileLockOwner {
            node_id: session.node_id,
            node_epoch: session.node_epoch,
            lock_owner: request.lock_owner,
        };
        Ok(u64::from(
            self.filesystem_locks.cancel(request.request_id, owner),
        ))
    }

    fn filesystem_release_lock_owner(
        &mut self,
        request: pb::FilesystemReleaseLockOwnerRequest,
    ) -> Result<ReleaseLockReceipt, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        if request.request_id == 0 || request.lock_owner == 0 {
            return Err(MetaRuntimeError::InvalidArgument(
                "lock release request id and owner must be non-zero".to_string(),
            ));
        }
        let owner = FileLockOwner {
            node_id: session.node_id,
            node_epoch: session.node_epoch,
            lock_owner: request.lock_owner,
        };
        match self
            .filesystem_locks
            .release_owner(owner, request.request_id)
        {
            ReleaseLockOwnerResult::Released(receipt) => Ok(receipt),
            ReleaseLockOwnerResult::QueueFull => Err(MetaRuntimeError::Unavailable),
            ReleaseLockOwnerResult::StaleRequestId => Err(MetaRuntimeError::InvalidArgument(
                "filesystem lock release request_id is outside the retry window".to_string(),
            )),
        }
    }

    fn filesystem_reclaim_locks(
        &mut self,
        request: pb::FilesystemReclaimLocksRequest,
    ) -> Result<u64, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let mut locks = Vec::with_capacity(request.locks.len());
        for lock in request.locks {
            locks.push(granted_lock_from_proto(lock, session).map_err(|error| {
                MetaRuntimeError::InvalidArgument(format!("invalid reclaimed lock: {error:?}"))
            })?);
        }
        let affected = locks.len() as u64;
        // 同一个 Node 的重报是一个完整快照：先丢弃该 incarnation 可能残留的旧表项，
        // 再安装全部条目，最后一次性解除恢复屏障。
        self.filesystem_locks
            .release_node_epoch(session.node_id, session.node_epoch);
        for (inode, lock) in locks {
            self.filesystem_locks.reclaim_entry(inode, lock);
        }
        self.filesystem_locks.finish_reclaim(session.node_id);
        dms_logging::debug!(
            "filesystem lock snapshot reclaimed";
            "event" => "meta.filesystem_lock.reclaimed",
            "node_id" => session.node_id,
            "node_epoch" => session.node_epoch,
            "lock_count" => affected,
        );
        Ok(affected)
    }

    fn filesystem_lookup(
        &mut self,
        request: pb::FilesystemLookupRequest,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        if request.name.is_empty() || request.name.contains(&b'/') {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem lookup requires one non-empty path component".to_string(),
            ));
        }
        let directory_grant = Some(self.filesystem_directory_grant(request.parent)?);
        let Some(dentry) = self.filesystem.lookup(request.parent, &request.name) else {
            return Ok(pb::FilesystemResolveResponse {
                found: false,
                resolved: None,
                dentry: None,
                directory_grant,
                entry_reference_lease_millis: 0,
                entry_reference_generation: 0,
            });
        };
        let resolved = self.filesystem_resolved_inode(session, dentry.inode)?;
        let mut response = pb::FilesystemResolveResponse {
            found: true,
            resolved: Some(resolved_to_proto(&resolved)),
            dentry: Some(dentry_to_proto(dentry)),
            directory_grant,
            entry_reference_lease_millis: 0,
            entry_reference_generation: 0,
        };
        self.attach_entry_reference_to_resolve_response(
            session,
            &mut response,
            request.reference_generation,
        );
        Ok(response)
    }

    fn filesystem_get_inode(
        &self,
        request: pb::FilesystemGetInodeRequest,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let Some(_) = self.filesystem.inode(request.inode) else {
            return Ok(pb::FilesystemResolveResponse {
                found: false,
                resolved: None,
                dentry: None,
                directory_grant: None,
                entry_reference_lease_millis: 0,
                entry_reference_generation: 0,
            });
        };
        let resolved = self.filesystem_resolved_inode(session, request.inode)?;
        Ok(pb::FilesystemResolveResponse {
            found: true,
            resolved: Some(resolved_to_proto(&resolved)),
            dentry: None,
            directory_grant: None,
            entry_reference_lease_millis: 0,
            entry_reference_generation: 0,
        })
    }

    fn filesystem_resolved_inode(
        &self,
        session: &pb::NodeSessionIdentity,
        inode: u64,
    ) -> Result<ResolvedInode, MetaRuntimeError> {
        let mut granted = self
            .filesystem
            .granted(inode, FILESYSTEM_BINDING_LEASE_MILLIS)
            .ok_or(MetaRuntimeError::NotFound)?;
        // local-memory reservation 只在 owner incarnation 存活时有效。解析结果不把
        // 已过期承诺交给 Node；下一次文件 mutation 会把清理后的快照写入 journal。
        granted.inode.reservations.retain(|reservation| {
            self.sessions
                .get(&reservation.node_id)
                .is_some_and(|owner| {
                    owner.node_epoch == reservation.node_epoch && self.is_live_session(owner)
                })
        });
        let object = match granted.inode.content.as_ref() {
            Some(binding) => Some(self.resolve_object(pb::ResolveObjectRequest {
                context: None,
                session: Some(session.clone()),
                key: Some(pb::Key {
                    value: binding.object_key.clone(),
                }),
                selector: Some(pb::resolve_object_request::Selector::ExactVersion(
                    binding.exact_version,
                )),
                range: None,
                cache_current: false,
            })?),
            None => None,
        };
        Ok(ResolvedInode {
            granted,
            object: object.map(crate::filesystem::ResolvedObject::from_proto),
            access_acl: self
                .filesystem
                .xattr(inode, ACL_ACCESS_NAME)
                .map(ToOwned::to_owned),
        })
    }

    fn filesystem_commit_response_for_inode(
        &self,
        inode: &InodeSnapshot,
        invalidation_cursor: u64,
        commit_index: u64,
    ) -> pb::FilesystemCommitVersionResponse {
        let object = inode.content.as_ref().and_then(|binding| {
            let layout = self
                .versions
                .get(&binding.object_key)?
                .get(&binding.exact_version)?;
            Some(pb::ResolveObjectResponse {
                layout: Some(layout.clone()),
                block_replicas: self.block_replicas_for_layout(layout),
                current_lease: None,
            })
        });
        pb::FilesystemCommitVersionResponse {
            resolved: Some(pb::FilesystemResolvedInode {
                inode: Some(inode_to_proto(inode)),
                grant: Some(pb::FilesystemCacheGrant {
                    generation: self.filesystem.grant_generation(inode.attributes.inode),
                    lease_millis: FILESYSTEM_BINDING_LEASE_MILLIS,
                }),
                object,
                access_acl: self
                    .filesystem
                    .xattr(inode.attributes.inode, ACL_ACCESS_NAME)
                    .map(ToOwned::to_owned),
            }),
            invalidation_cursor,
            commit_index,
        }
    }

    fn filesystem_attribute_response_for_inode(
        &self,
        inode: &InodeSnapshot,
        invalidation_cursor: u64,
        commit_index: u64,
    ) -> pb::FilesystemAttributeMutationResponse {
        let committed =
            self.filesystem_commit_response_for_inode(inode, invalidation_cursor, commit_index);
        pb::FilesystemAttributeMutationResponse {
            resolved: committed.resolved,
            invalidation_cursor,
            commit_index,
        }
    }

    fn validate_namespace_operation(
        &self,
        operation_id: &[u8],
        operation_digest: &[u8],
    ) -> Result<(), MetaRuntimeError> {
        if operation_id.is_empty() || operation_digest.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem namespace operation identity is required".to_string(),
            ));
        }
        Ok(())
    }

    fn previous_filesystem_namespace_operation(
        &self,
        operation_id: &[u8],
        operation_digest: &[u8],
    ) -> Result<Option<&NamespaceMutationResult>, MetaRuntimeError> {
        let Some(previous) = self.filesystem_namespace_operations.get(operation_id) else {
            return Ok(None);
        };
        if previous.digest == operation_digest {
            return Ok(Some(&previous.result));
        }
        Err(MetaRuntimeError::Conflict {
            expected: None,
            actual: previous.result.commit_index,
        })
    }

    fn filesystem_create_response_from_namespace(
        &self,
        session: &pb::NodeSessionIdentity,
        result: &NamespaceMutationResult,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let inode = result
            .inode
            .as_ref()
            .ok_or(MetaRuntimeError::NotFound)?
            .attributes
            .inode;
        let resolved = self.filesystem_resolved_inode(session, inode)?;
        Ok(pb::FilesystemResolveResponse {
            found: true,
            resolved: Some(resolved_to_proto(&resolved)),
            dentry: result.dentry.as_ref().map(dentry_to_proto),
            directory_grant: result
                .dentry
                .as_ref()
                .map(|dentry| self.filesystem_directory_grant(dentry.parent))
                .transpose()?,
            entry_reference_lease_millis: result.entry_reference_lease_millis,
            entry_reference_generation: result.entry_reference_generation,
        })
    }

    fn filesystem_directory_grant(
        &self,
        directory: u64,
    ) -> Result<pb::FilesystemDirectoryGrant, MetaRuntimeError> {
        let inode = self.filesystem_directory(directory)?;
        Ok(directory_grant_to_proto(DirectoryGrant {
            directory_revision: inode.revision,
            grant: CacheGrant {
                generation: self.filesystem.grant_generation(directory),
                lease_millis: FILESYSTEM_BINDING_LEASE_MILLIS,
            },
        }))
    }

    fn filesystem_directory(&self, inode: u64) -> Result<InodeSnapshot, MetaRuntimeError> {
        let directory = self
            .filesystem
            .inode(inode)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        if directory.attributes.kind != InodeKind::Directory {
            return Err(MetaRuntimeError::NotDirectory);
        }
        Ok(directory)
    }

    fn check_expected_revision(
        &self,
        expected: Option<u64>,
        actual: u64,
    ) -> Result<(), MetaRuntimeError> {
        if expected.is_some_and(|expected| expected != actual) {
            return Err(MetaRuntimeError::StaleRevision { expected, actual });
        }
        Ok(())
    }

    fn changed_directory(&self, inode: u64, revision: u64) -> crate::filesystem::DirectoryVersion {
        crate::filesystem::DirectoryVersion {
            inode,
            revision,
            grant_generation: self.filesystem.grant_generation(inode).saturating_add(1),
        }
    }

    fn changed_inode(&self, inode: u64, revision: u64) -> InodeVersion {
        InodeVersion {
            inode,
            revision,
            grant_generation: self.filesystem.grant_generation(inode).saturating_add(1),
        }
    }

    fn next_filesystem_invalidation_cursor(&self, changed_event_count: usize) -> u64 {
        self.event_high_watermark + changed_event_count as u64
    }

    fn attach_entry_reference_to_resolve_response(
        &mut self,
        session: &pb::NodeSessionIdentity,
        response: &mut pb::FilesystemResolveResponse,
        reference_generation: u64,
    ) {
        let Some(inode) = response
            .resolved
            .as_ref()
            .and_then(|resolved| resolved.inode.as_ref())
            .and_then(|inode| inode.attributes.as_ref())
            .map(|attributes| attributes.inode)
        else {
            return;
        };
        self.store_filesystem_inode_reference(session, inode, reference_generation, response);
    }

    fn attach_entry_reference_to_namespace_response(
        &mut self,
        session: &pb::NodeSessionIdentity,
        response: &mut pb::FilesystemNamespaceMutationResponse,
        reference_generation: u64,
    ) {
        let Some(inode) = response
            .inode
            .as_ref()
            .and_then(|inode| inode.attributes.as_ref())
            .map(|attributes| attributes.inode)
        else {
            return;
        };
        if inode == ROOT_INODE || reference_generation == 0 {
            return;
        }
        let generation =
            self.store_filesystem_reference_generation(session, inode, reference_generation);
        response.entry_reference_generation = generation;
        response.entry_reference_lease_millis = DEFAULT_NODE_LEASE_TTL.as_millis() as u64;
    }

    fn store_filesystem_inode_reference(
        &mut self,
        session: &pb::NodeSessionIdentity,
        inode: u64,
        reference_generation: u64,
        response: &mut pb::FilesystemResolveResponse,
    ) {
        if inode == ROOT_INODE || reference_generation == 0 {
            return;
        }
        let generation =
            self.store_filesystem_reference_generation(session, inode, reference_generation);
        response.entry_reference_generation = generation;
        response.entry_reference_lease_millis = DEFAULT_NODE_LEASE_TTL.as_millis() as u64;
    }

    fn store_filesystem_reference_generation(
        &mut self,
        session: &pb::NodeSessionIdentity,
        inode: u64,
        requested_generation: u64,
    ) -> u64 {
        let deadline = Instant::now() + DEFAULT_NODE_LEASE_TTL;
        let entry = self
            .filesystem_inode_references
            .entry((session.node_id, session.node_epoch, inode))
            .or_insert((requested_generation, deadline));
        if requested_generation >= entry.0 {
            *entry = (requested_generation, deadline);
        }
        entry.0
    }

    fn filesystem_create_inode(
        &mut self,
        request: pb::FilesystemCreateInodeRequest,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        self.validate_namespace_operation(&request.operation_id, &request.operation_digest)?;
        validate_path_component(&request.name, "create")?;
        if let Some(previous) = self
            .previous_filesystem_namespace_operation(
                &request.operation_id,
                &request.operation_digest,
            )?
            .cloned()
        {
            let mut response =
                self.filesystem_create_response_from_namespace(&session, &previous)?;
            self.attach_entry_reference_to_resolve_response(
                &session,
                &mut response,
                request.reference_generation,
            );
            return Ok(response);
        }

        let parent = self.filesystem_directory(request.parent)?;
        self.check_expected_revision(request.expected_parent_revision, parent.revision)?;
        if self
            .filesystem
            .lookup(request.parent, &request.name)
            .is_some()
        {
            return Err(MetaRuntimeError::AlreadyExists);
        }
        let kind = InodeKind::from_proto(request.kind)
            .map_err(|_| MetaRuntimeError::InvalidArgument("invalid inode kind".to_string()))?;
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        let sequence = self.journal.last_index() + 1;
        let (inode_id, next_inode) = self
            .filesystem
            .allocate_inode()
            .ok_or(MetaRuntimeError::Unavailable)?;
        let parent_default_acl = self
            .filesystem
            .xattr(request.parent, ACL_DEFAULT_NAME)
            .map(ToOwned::to_owned);
        let (created_mode, xattr_updates) =
            inherit_default_acl(inode_id, kind, request.mode, parent_default_acl.as_deref())
                .map_err(|_| MetaRuntimeError::XattrUnsupported)?;
        let now = current_time_unix_nanos();
        let mut parent_inode = parent.clone();
        parent_inode.revision = sequence;
        parent_inode.attributes.mtime_unix_nanos = now;
        parent_inode.attributes.ctime_unix_nanos = now;
        if kind == InodeKind::Directory {
            parent_inode.attributes.link_count =
                parent_inode.attributes.link_count.saturating_add(1);
        }
        let inode = InodeSnapshot {
            revision: sequence,
            attributes: InodeAttributes {
                inode: inode_id,
                kind,
                mode: created_mode,
                uid: request.uid,
                gid: request.gid,
                link_count: if kind == InodeKind::Directory { 2 } else { 1 },
                size: 0,
                atime_unix_nanos: now,
                mtime_unix_nanos: now,
                ctime_unix_nanos: now,
            },
            content: None,
            reservations: Vec::new(),
        };
        let dentry = DentrySnapshot {
            parent: request.parent,
            name: request.name,
            inode: inode_id,
            directory_revision: sequence,
        };
        let result = NamespaceMutationResult {
            dentry: Some(dentry.clone()),
            inode: Some(inode.clone()),
            changed_directories: vec![self.changed_directory(request.parent, sequence)],
            changed_inodes: Vec::new(),
            invalidation_cursor: self.next_filesystem_invalidation_cursor(1),
            commit_index: sequence,
            entry_reference_lease_millis: 0,
            entry_reference_generation: 0,
        };
        let record = JournalRecord::FilesystemNamespaceMutated {
            record: FilesystemNamespaceMutationRecord {
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                result: result.clone(),
                upsert_inodes: vec![parent_inode, inode.clone()],
                upsert_dentries: vec![dentry.clone()],
                remove_dentries: Vec::new(),
                next_inode,
                xattr_updates,
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        let mut response = self.filesystem_create_response_from_namespace(&session, &result)?;
        self.attach_entry_reference_to_resolve_response(
            &session,
            &mut response,
            request.reference_generation,
        );
        Ok(response)
    }

    fn filesystem_create_symlink(
        &mut self,
        request: pb::FilesystemCreateSymlinkRequest,
    ) -> Result<pb::FilesystemResolveResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        self.validate_namespace_operation(&request.operation_id, &request.operation_digest)?;
        validate_path_component(&request.name, "symlink")?;
        if request.target_size == 0 {
            return Err(MetaRuntimeError::InvalidArgument(
                "symlink target must not be empty".to_string(),
            ));
        }
        if let Some(previous) = self
            .previous_filesystem_namespace_operation(
                &request.operation_id,
                &request.operation_digest,
            )?
            .cloned()
        {
            let mut response =
                self.filesystem_create_response_from_namespace(&session, &previous)?;
            self.attach_entry_reference_to_resolve_response(
                &session,
                &mut response,
                request.reference_generation,
            );
            return Ok(response);
        }

        let parent = self.filesystem_directory(request.parent)?;
        self.check_expected_revision(request.expected_parent_revision, parent.revision)?;
        if self
            .filesystem
            .lookup(request.parent, &request.name)
            .is_some()
        {
            return Err(MetaRuntimeError::AlreadyExists);
        }
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        let candidate = request.candidate.ok_or_else(|| {
            MetaRuntimeError::InvalidArgument("missing symlink target candidate".to_string())
        })?;
        if candidate.kind != pb::VersionKind::Value as i32
            || candidate.logical_length != request.target_size
        {
            return Err(MetaRuntimeError::InvalidArgument(
                "symlink target candidate must exactly describe the target bytes".to_string(),
            ));
        }
        if self.versions.contains_key(&request.object_key) {
            return Err(MetaRuntimeError::AlreadyExists);
        }
        let referenced_blocks = validate_filesystem_candidate_layout(&candidate)?;
        for block_id in &referenced_blocks {
            self.reject_retired_block(block_id)?;
            let proof = request
                .replica_proofs
                .iter()
                .find(|proof| &proof.block_id == block_id);
            let new_replica = request
                .new_replicas
                .iter()
                .find(|report| &report.block_id == block_id);
            let known = proof.is_some_and(|proof| {
                self.replicas.get(block_id).is_some_and(|replicas| {
                    replicas.iter().any(|replica| {
                        replica.location.node_id == proof.node_id
                            && replica.location.node_epoch == proof.node_epoch
                            && replica.catalog_revision == proof.catalog_revision
                            && replica.location.checksum == proof.checksum
                    })
                })
            });
            let known_and_live = known && self.block_referenced_by_retained_versions(block_id);
            if !known_and_live && new_replica.is_none() {
                return Err(MetaRuntimeError::InvalidArgument(
                    "symlink target extent requires an existing proof or new replica report"
                        .to_string(),
                ));
            }
        }

        let endpoint = self
            .sessions
            .get(&session.node_id)
            .expect("verified session")
            .control_endpoint
            .clone();
        let new_replicas = request
            .new_replicas
            .into_iter()
            .map(|report| {
                self.reject_retired_block(&report.block_id)?;
                if report.block_id.is_empty() || report.length == 0 {
                    return Err(MetaRuntimeError::InvalidArgument(
                        "new symlink target replica requires block id and length".to_string(),
                    ));
                }
                Ok((
                    pb::ReplicaLocation {
                        block_id: report.block_id,
                        node_id: session.node_id,
                        node_epoch: session.node_epoch,
                        data_endpoint: endpoint.clone(),
                        checksum: report.checksum,
                        durability: report.durability,
                    },
                    report.length,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let sequence = self.journal.last_index() + 1;
        let (inode_id, next_inode) = self
            .filesystem
            .allocate_inode()
            .ok_or(MetaRuntimeError::Unavailable)?;
        let now = current_time_unix_nanos();
        let mut parent_inode = parent.clone();
        parent_inode.revision = sequence;
        parent_inode.attributes.mtime_unix_nanos = now;
        parent_inode.attributes.ctime_unix_nanos = now;
        let inode = InodeSnapshot {
            revision: sequence,
            attributes: InodeAttributes {
                inode: inode_id,
                kind: InodeKind::SymbolicLink,
                mode: 0o777,
                uid: request.uid,
                gid: request.gid,
                link_count: 1,
                size: request.target_size,
                atime_unix_nanos: now,
                mtime_unix_nanos: request.mtime_unix_nanos,
                ctime_unix_nanos: now,
            },
            content: Some(FileContentBinding {
                object_key: request.object_key.clone(),
                exact_version: 1,
            }),
            reservations: Vec::new(),
        };
        let dentry = DentrySnapshot {
            parent: request.parent,
            name: request.name,
            inode: inode_id,
            directory_revision: sequence,
        };
        let result = NamespaceMutationResult {
            dentry: Some(dentry.clone()),
            inode: Some(inode.clone()),
            changed_directories: vec![self.changed_directory(request.parent, sequence)],
            changed_inodes: Vec::new(),
            invalidation_cursor: self.next_filesystem_invalidation_cursor(1),
            commit_index: sequence,
            entry_reference_lease_millis: 0,
            entry_reference_generation: 0,
        };
        let layout = pb::VersionLayout {
            version: 1,
            logical_length: candidate.logical_length,
            extents: candidate.extents,
            digest: candidate.digest,
            kind: candidate.kind,
        };
        let record = JournalRecord::FilesystemSymlinkCreated {
            record: FilesystemSymlinkCreatedRecord {
                namespace: FilesystemNamespaceMutationRecord {
                    operation_id: request.operation_id.clone(),
                    operation_digest: request.operation_digest.clone(),
                    result: result.clone(),
                    upsert_inodes: vec![parent_inode, inode],
                    upsert_dentries: vec![dentry],
                    remove_dentries: Vec::new(),
                    next_inode,
                    xattr_updates: Vec::new(),
                },
                version: VersionCommitRecord {
                    key: request.object_key,
                    layout,
                    modified_time_unix_millis: current_time_unix_millis(),
                    new_replicas,
                    operation_id: request.operation_id,
                    operation_digest: request.operation_digest,
                },
                new_grant_generation: self.filesystem.grant_generation(inode_id).saturating_add(1),
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        let mut response = self.filesystem_create_response_from_namespace(&session, &result)?;
        self.attach_entry_reference_to_resolve_response(
            &session,
            &mut response,
            request.reference_generation,
        );
        Ok(response)
    }

    fn filesystem_read_directory(
        &self,
        request: pb::FilesystemReadDirectoryRequest,
    ) -> Result<pb::FilesystemReadDirectoryResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let directory = self.filesystem_directory(request.directory)?;
        self.check_expected_revision(request.expected_directory_revision, directory.revision)?;
        let limit = if request.limit == 0 {
            DEFAULT_SCAN_LIMIT
        } else {
            (request.limit as usize).min(MAX_SCAN_LIMIT)
        };
        let Some(page) = self.filesystem.read_directory(
            request.directory,
            &request.cursor,
            limit,
            FILESYSTEM_BINDING_LEASE_MILLIS,
        ) else {
            return Err(MetaRuntimeError::NotDirectory);
        };
        let has_more = page.next_cursor.is_some();
        Ok(pb::FilesystemReadDirectoryResponse {
            directory_grant: Some(directory_grant_to_proto(page.grant)),
            entries: page.entries.iter().map(directory_entry_to_proto).collect(),
            has_more,
            next_cursor: page.next_cursor.unwrap_or_default(),
            parent: page.parent,
        })
    }

    fn filesystem_link_entry(
        &mut self,
        request: pb::FilesystemLinkRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        self.validate_namespace_operation(&request.operation_id, &request.operation_digest)?;
        validate_path_component(&request.target_name, "link target")?;
        if let Some(previous) = self
            .previous_filesystem_namespace_operation(
                &request.operation_id,
                &request.operation_digest,
            )?
            .cloned()
        {
            let mut response = namespace_result_to_proto(&previous);
            self.attach_entry_reference_to_namespace_response(
                &session,
                &mut response,
                request.reference_generation,
            );
            return Ok(response);
        }
        let target_parent = self.filesystem_directory(request.target_parent)?;
        self.check_expected_revision(request.expected_target_revision, target_parent.revision)?;
        if self
            .filesystem
            .lookup(request.target_parent, &request.target_name)
            .is_some()
        {
            return Err(MetaRuntimeError::AlreadyExists);
        }
        let mut inode = self
            .filesystem
            .inode(request.source_inode)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        if inode.attributes.kind == InodeKind::Directory {
            return Err(MetaRuntimeError::IsDirectory);
        }
        if inode.attributes.link_count == 0 {
            return Err(MetaRuntimeError::NotFound);
        }
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        let sequence = self.journal.last_index() + 1;
        let now = current_time_unix_nanos();

        let mut parent_inode = target_parent.clone();
        parent_inode.revision = sequence;
        parent_inode.attributes.mtime_unix_nanos = now;
        parent_inode.attributes.ctime_unix_nanos = now;

        inode.revision = sequence;
        inode.attributes.link_count = inode.attributes.link_count.saturating_add(1);
        inode.attributes.ctime_unix_nanos = now;

        let dentry = DentrySnapshot {
            parent: request.target_parent,
            name: request.target_name,
            inode: request.source_inode,
            directory_revision: sequence,
        };
        let result = NamespaceMutationResult {
            dentry: Some(dentry.clone()),
            inode: Some(inode.clone()),
            changed_directories: vec![self.changed_directory(request.target_parent, sequence)],
            changed_inodes: vec![self.changed_inode(request.source_inode, sequence)],
            invalidation_cursor: self.next_filesystem_invalidation_cursor(2),
            commit_index: sequence,
            entry_reference_lease_millis: 0,
            entry_reference_generation: 0,
        };
        let record = JournalRecord::FilesystemNamespaceMutated {
            record: FilesystemNamespaceMutationRecord {
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                result: result.clone(),
                upsert_inodes: vec![parent_inode, inode],
                upsert_dentries: vec![dentry],
                remove_dentries: Vec::new(),
                next_inode: self.filesystem.next_inode_hint(),
                xattr_updates: Vec::new(),
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        let mut response = namespace_result_to_proto(&result);
        self.attach_entry_reference_to_namespace_response(
            &session,
            &mut response,
            request.reference_generation,
        );
        Ok(response)
    }

    fn filesystem_acquire_inode_reference(
        &mut self,
        request: pb::FilesystemInodeReferenceRequest,
    ) -> Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        if request.inode == ROOT_INODE {
            return Ok(pb::FilesystemInodeReferenceResponse {
                lease_millis: DEFAULT_NODE_LEASE_TTL.as_millis() as u64,
            });
        }
        let inode = self
            .filesystem
            .inode(request.inode)
            .ok_or(MetaRuntimeError::NotFound)?;
        if inode.attributes.link_count == 0 {
            // orphan 只能由 unlink 前已经建立的引用继续保护。Node 重连后的同一引用
            // 通过 renew/re-report 恢复，不能用新的 acquire 绕过 namespace 再次打开。
            return Err(MetaRuntimeError::NotFound);
        }
        self.store_filesystem_reference_generation(
            session,
            request.inode,
            request.reference_generation,
        );
        Ok(pb::FilesystemInodeReferenceResponse {
            lease_millis: DEFAULT_NODE_LEASE_TTL.as_millis() as u64,
        })
    }

    fn filesystem_release_inode_reference(
        &mut self,
        request: pb::FilesystemInodeReferenceRequest,
    ) -> Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let key = (session.node_id, session.node_epoch, request.inode);
        if self
            .filesystem_inode_references
            .get(&key)
            .is_some_and(|(generation, _)| *generation == request.reference_generation)
        {
            self.filesystem_inode_references.remove(&key);
        }
        Ok(pb::FilesystemInodeReferenceResponse { lease_millis: 0 })
    }

    fn filesystem_renew_inode_references(
        &mut self,
        request: pb::FilesystemRenewInodeReferencesRequest,
    ) -> Result<pb::FilesystemInodeReferenceResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let deadline = Instant::now() + DEFAULT_NODE_LEASE_TTL;
        for reference in request.references {
            if reference.inode == ROOT_INODE || reference.reference_generation == 0 {
                continue;
            }
            if self.filesystem.inode(reference.inode).is_none() {
                continue;
            }
            let key = (session.node_id, session.node_epoch, reference.inode);
            match self.filesystem_inode_references.get_mut(&key) {
                Some((generation, until)) if *generation > reference.reference_generation => {}
                Some((generation, until)) => {
                    *generation = reference.reference_generation;
                    *until = deadline;
                }
                None => {
                    // Node 重连会获得新 epoch；批量 renew 同时承担 re-report，避免
                    // 旧 epoch 过期时误回收仍由该 Node 打开的 orphan。
                    self.filesystem_inode_references
                        .insert(key, (reference.reference_generation, deadline));
                }
            }
        }
        Ok(pb::FilesystemInodeReferenceResponse {
            lease_millis: DEFAULT_NODE_LEASE_TTL.as_millis() as u64,
        })
    }

    fn filesystem_rename_entry(
        &mut self,
        request: pb::FilesystemRenameRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        self.validate_namespace_operation(&request.operation_id, &request.operation_digest)?;
        validate_path_component(&request.source_name, "rename source")?;
        validate_path_component(&request.target_name, "rename target")?;
        if let Some(previous) = self.previous_filesystem_namespace_operation(
            &request.operation_id,
            &request.operation_digest,
        )? {
            return Ok(namespace_result_to_proto(previous));
        }
        if request.source_parent == request.target_parent
            && request.source_name == request.target_name
        {
            let source_parent = self.filesystem_directory(request.source_parent)?;
            self.check_expected_revision(request.expected_source_revision, source_parent.revision)?;
            self.check_expected_revision(request.expected_target_revision, source_parent.revision)?;
            let dentry = self
                .filesystem
                .lookup(request.source_parent, &request.source_name)
                .cloned()
                .ok_or(MetaRuntimeError::NotFound)?;
            let inode = self
                .filesystem
                .inode(dentry.inode)
                .cloned()
                .ok_or(MetaRuntimeError::NotFound)?;
            let commit_sequence = self.validate_commit_sequence(
                &session,
                request.commit_sequence,
                &request.operation_id,
                &request.operation_digest,
            )?;
            let sequence = self.journal.last_index() + 1;
            let result = NamespaceMutationResult {
                dentry: Some(dentry),
                inode: Some(inode),
                changed_directories: Vec::new(),
                changed_inodes: Vec::new(),
                invalidation_cursor: 0,
                commit_index: sequence,
                entry_reference_lease_millis: 0,
                entry_reference_generation: 0,
            };
            let record = JournalRecord::FilesystemNamespaceMutated {
                record: FilesystemNamespaceMutationRecord {
                    operation_id: request.operation_id,
                    operation_digest: request.operation_digest,
                    result: result.clone(),
                    upsert_inodes: Vec::new(),
                    upsert_dentries: Vec::new(),
                    remove_dentries: Vec::new(),
                    next_inode: self.filesystem.next_inode_hint(),
                    xattr_updates: Vec::new(),
                },
                commit_sequence,
            };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
            self.maybe_checkpoint(sequence)?;
            return Ok(namespace_result_to_proto(&result));
        }
        let source_parent = self.filesystem_directory(request.source_parent)?;
        let target_parent = self.filesystem_directory(request.target_parent)?;
        self.check_expected_revision(request.expected_source_revision, source_parent.revision)?;
        self.check_expected_revision(request.expected_target_revision, target_parent.revision)?;
        let source_dentry = self
            .filesystem
            .lookup(request.source_parent, &request.source_name)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        let moved_inode = self
            .filesystem
            .inode(source_dentry.inode)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        if moved_inode.attributes.inode == ROOT_INODE {
            return Err(MetaRuntimeError::InvalidArgument(
                "cannot rename root".to_string(),
            ));
        }
        if moved_inode.attributes.kind == InodeKind::Directory
            && self
                .filesystem
                .is_directory_descendant_of(request.target_parent, moved_inode.attributes.inode)
        {
            return Err(MetaRuntimeError::InvalidArgument(
                "cannot move a directory into itself or its descendant".to_string(),
            ));
        }
        let replaced = self
            .filesystem
            .lookup(request.target_parent, &request.target_name)
            .cloned();
        if replaced.is_some() && !request.replace_existing {
            return Err(MetaRuntimeError::AlreadyExists);
        }
        let replaced_inode = match replaced.as_ref() {
            Some(dentry) => Some(
                self.filesystem
                    .inode(dentry.inode)
                    .cloned()
                    .ok_or(MetaRuntimeError::NotFound)?,
            ),
            None => None,
        };
        if replaced
            .as_ref()
            .is_some_and(|dentry| dentry.inode == source_dentry.inode)
        {
            let commit_sequence = self.validate_commit_sequence(
                &session,
                request.commit_sequence,
                &request.operation_id,
                &request.operation_digest,
            )?;
            let sequence = self.journal.last_index() + 1;
            let result = NamespaceMutationResult {
                dentry: Some(source_dentry),
                inode: Some(moved_inode),
                changed_directories: Vec::new(),
                changed_inodes: Vec::new(),
                invalidation_cursor: 0,
                commit_index: sequence,
                entry_reference_lease_millis: 0,
                entry_reference_generation: 0,
            };
            let record = JournalRecord::FilesystemNamespaceMutated {
                record: FilesystemNamespaceMutationRecord {
                    operation_id: request.operation_id,
                    operation_digest: request.operation_digest,
                    result: result.clone(),
                    upsert_inodes: Vec::new(),
                    upsert_dentries: Vec::new(),
                    remove_dentries: Vec::new(),
                    next_inode: self.filesystem.next_inode_hint(),
                    xattr_updates: Vec::new(),
                },
                commit_sequence,
            };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
            self.maybe_checkpoint(sequence)?;
            return Ok(namespace_result_to_proto(&result));
        }
        if let Some(inode) = replaced_inode.as_ref() {
            match (moved_inode.attributes.kind, inode.attributes.kind) {
                (InodeKind::Directory, InodeKind::Directory)
                    if !self.filesystem.directory_is_empty(inode.attributes.inode) =>
                {
                    return Err(MetaRuntimeError::DirectoryNotEmpty);
                }
                (InodeKind::Directory, InodeKind::Directory) => {}
                (InodeKind::Directory, _) => return Err(MetaRuntimeError::NotDirectory),
                (_, InodeKind::Directory) => return Err(MetaRuntimeError::IsDirectory),
                _ => {}
            }
        }
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        let sequence = self.journal.last_index() + 1;
        let now = current_time_unix_nanos();
        let mut upsert_inodes = Vec::new();
        let mut source_parent_inode = source_parent.clone();
        source_parent_inode.revision = sequence;
        source_parent_inode.attributes.mtime_unix_nanos = now;
        source_parent_inode.attributes.ctime_unix_nanos = now;
        if moved_inode.attributes.kind == InodeKind::Directory {
            source_parent_inode.attributes.link_count =
                source_parent_inode.attributes.link_count.saturating_sub(1);
        }
        let mut changed_directories = vec![self.changed_directory(request.source_parent, sequence)];
        if request.source_parent == request.target_parent {
            upsert_inodes.push(source_parent_inode);
        } else {
            let mut target_parent_inode = target_parent.clone();
            target_parent_inode.revision = sequence;
            target_parent_inode.attributes.mtime_unix_nanos = now;
            target_parent_inode.attributes.ctime_unix_nanos = now;
            if moved_inode.attributes.kind == InodeKind::Directory {
                target_parent_inode.attributes.link_count =
                    target_parent_inode.attributes.link_count.saturating_add(1);
            }
            if replaced_inode
                .as_ref()
                .is_some_and(|inode| inode.attributes.kind == InodeKind::Directory)
            {
                target_parent_inode.attributes.link_count =
                    target_parent_inode.attributes.link_count.saturating_sub(1);
            }
            upsert_inodes.push(source_parent_inode);
            upsert_inodes.push(target_parent_inode);
            changed_directories.push(self.changed_directory(request.target_parent, sequence));
        }
        let changed_replaced_inode = replaced_inode
            .as_ref()
            .map(|inode| self.changed_inode(inode.attributes.inode, sequence));
        if let Some(mut inode) = replaced_inode {
            inode.revision = sequence;
            inode.attributes.link_count = inode.attributes.link_count.saturating_sub(1);
            inode.attributes.ctime_unix_nanos = now;
            upsert_inodes.push(inode);
        }
        let new_dentry = DentrySnapshot {
            parent: request.target_parent,
            name: request.target_name,
            inode: source_dentry.inode,
            directory_revision: sequence,
        };
        let mut remove_dentries = vec![source_dentry];
        if let Some(replaced) = replaced {
            remove_dentries.push(replaced);
        }
        let changed_inodes = changed_replaced_inode.into_iter().collect::<Vec<_>>();
        let event_count = changed_directories.len() + changed_inodes.len();
        let result = NamespaceMutationResult {
            dentry: Some(new_dentry.clone()),
            inode: Some(moved_inode),
            changed_directories,
            changed_inodes,
            invalidation_cursor: self.next_filesystem_invalidation_cursor(event_count),
            commit_index: sequence,
            entry_reference_lease_millis: 0,
            entry_reference_generation: 0,
        };
        let record = JournalRecord::FilesystemNamespaceMutated {
            record: FilesystemNamespaceMutationRecord {
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                result: result.clone(),
                upsert_inodes,
                upsert_dentries: vec![new_dentry],
                remove_dentries,
                next_inode: self.filesystem.next_inode_hint(),
                xattr_updates: Vec::new(),
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        Ok(namespace_result_to_proto(&result))
    }

    fn filesystem_remove_entry(
        &mut self,
        request: pb::FilesystemRemoveRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        self.validate_namespace_operation(&request.operation_id, &request.operation_digest)?;
        validate_path_component(&request.name, "remove")?;
        if let Some(previous) = self.previous_filesystem_namespace_operation(
            &request.operation_id,
            &request.operation_digest,
        )? {
            return Ok(namespace_result_to_proto(previous));
        }
        let parent = self.filesystem_directory(request.parent)?;
        self.check_expected_revision(request.expected_parent_revision, parent.revision)?;
        let dentry = self
            .filesystem
            .lookup(request.parent, &request.name)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        let mut inode = self
            .filesystem
            .inode(dentry.inode)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        let kind = filesystem_remove_kind(request.kind)?;
        match (kind, inode.attributes.kind) {
            (RemoveKind::File, InodeKind::Directory) => return Err(MetaRuntimeError::IsDirectory),
            (RemoveKind::Directory, InodeKind::Directory) => {
                if !self.filesystem.directory_is_empty(inode.attributes.inode) {
                    return Err(MetaRuntimeError::DirectoryNotEmpty);
                }
            }
            (RemoveKind::Directory, _) => return Err(MetaRuntimeError::NotDirectory),
            (RemoveKind::File, _) => {}
        }
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        let sequence = self.journal.last_index() + 1;
        let now = current_time_unix_nanos();
        let mut parent_inode = parent.clone();
        parent_inode.revision = sequence;
        parent_inode.attributes.mtime_unix_nanos = now;
        parent_inode.attributes.ctime_unix_nanos = now;
        if inode.attributes.kind == InodeKind::Directory {
            parent_inode.attributes.link_count =
                parent_inode.attributes.link_count.saturating_sub(1);
        }
        inode.revision = sequence;
        inode.attributes.link_count = inode.attributes.link_count.saturating_sub(1);
        inode.attributes.ctime_unix_nanos = now;
        let result = NamespaceMutationResult {
            dentry: None,
            inode: Some(inode.clone()),
            changed_directories: vec![self.changed_directory(request.parent, sequence)],
            changed_inodes: vec![self.changed_inode(inode.attributes.inode, sequence)],
            invalidation_cursor: self.next_filesystem_invalidation_cursor(2),
            commit_index: sequence,
            entry_reference_lease_millis: 0,
            entry_reference_generation: 0,
        };
        let record = JournalRecord::FilesystemNamespaceMutated {
            record: FilesystemNamespaceMutationRecord {
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                result: result.clone(),
                upsert_inodes: vec![parent_inode, inode],
                upsert_dentries: Vec::new(),
                remove_dentries: vec![dentry],
                next_inode: self.filesystem.next_inode_hint(),
                xattr_updates: Vec::new(),
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        Ok(namespace_result_to_proto(&result))
    }

    fn current_lease_grant(
        &self,
        request: &pb::ResolveObjectRequest,
        layout: &pb::VersionLayout,
    ) -> Option<pb::CurrentLeaseGrant> {
        if !request.cache_current {
            return None;
        }
        if matches!(
            request.selector.as_ref(),
            Some(pb::resolve_object_request::Selector::ExactVersion(_))
        ) {
            return None;
        }
        let requested = request.session.as_ref()?;
        if self.retired_sessions.contains(&requested.node_id) {
            return None;
        }
        let session = self.sessions.get(&requested.node_id)?;
        if session.session_id != requested.session_id || session.node_epoch != requested.node_epoch
        {
            return None;
        }
        let deadline = session.last_heartbeat? + DEFAULT_NODE_LEASE_TTL;
        let ttl_millis = deadline
            .checked_duration_since(Instant::now())?
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        if ttl_millis == 0 {
            return None;
        }
        Some(pb::CurrentLeaseGrant {
            version: layout.version,
            // 这是 Meta 当前目录视图的水位，不是单个 Version 的 commit revision。
            // 首版 Node 用本地失效 generation 围栏；保留此水位用于诊断，不冒充版本号。
            revision: self.last_applied_index,
            lease_epoch: session.node_epoch,
            ttl_millis,
            // 首版没有 HA Meta leader 租约，不能伪造 leader epoch。
            leader_epoch: 0,
        })
    }

    fn is_live_replica(&self, replica: &StoredReplica) -> bool {
        self.sessions
            .get(&replica.location.node_id)
            .is_some_and(|session| {
                session.node_epoch == replica.location.node_epoch && self.is_live_session(session)
            })
    }

    fn is_live_session(&self, session: &NodeSession) -> bool {
        session
            .last_heartbeat
            .is_some_and(|last_seen| last_seen.elapsed() <= DEFAULT_NODE_LEASE_TTL)
    }

    fn report_replicas(
        &mut self,
        request: pb::ReportReplicasRequest,
    ) -> Result<pb::ReportReplicasResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        if !request.operation_id.is_empty()
            && let Some(previous) = self.replica_operations.get(&request.operation_id)
        {
            return Ok(previous.clone());
        }
        self.verify_repair_report(&session, &request.repair_id, &request.replicas)?;
        let endpoint = self
            .sessions
            .get(&session.node_id)
            .expect("verified session")
            .control_endpoint
            .clone();
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        let catalog_revision = self.journal.last_index() + 1;
        let operation_id = request.operation_id;
        let mut accepted_locations = Vec::new();
        for report in request.replicas {
            if report.block_id.is_empty()
                || report.length == 0
                || self.retired_block_fences.contains_key(&report.block_id)
                || !self.block_referenced_by_retained_versions(&report.block_id)
            {
                rejected.push(report.block_id);
                continue;
            }
            let location = pb::ReplicaLocation {
                block_id: report.block_id.clone(),
                node_id: session.node_id,
                node_epoch: session.node_epoch,
                data_endpoint: endpoint.clone(),
                checksum: report.checksum.clone(),
                durability: report.durability,
            };
            accepted_locations.push((location.clone(), report.length));
            accepted.push(pb::ReplicaReceipt {
                block_id: report.block_id,
                node_id: session.node_id,
                node_epoch: session.node_epoch,
                catalog_revision,
                checksum: report.checksum,
            });
        }
        let response = pb::ReportReplicasResponse {
            accepted,
            rejected_block_ids: rejected,
            catalog_watermark: catalog_revision,
        };
        let record = JournalRecord::ReplicasReported {
            operation_id,
            accepted: accepted_locations,
            rejected_block_ids: response.rejected_block_ids.clone(),
            catalog_revision,
            desired_copies: request.desired_copies.max(1),
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        Ok(response)
    }

    /// Accepts a repair report only from the target incarnation of the one
    /// currently pending attempt. Read-through/import reports carry no repair
    /// id and preserve the ordinary replica-registration path.
    fn verify_repair_report(
        &self,
        session: &pb::NodeSessionIdentity,
        repair_id: &[u8],
        reports: &[pb::ReplicaReport],
    ) -> Result<(), MetaRuntimeError> {
        if repair_id.is_empty() {
            return Ok(());
        }
        if reports.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "repair report requires at least one replica".to_string(),
            ));
        }
        let valid = reports.iter().all(|report| {
            self.pending_repairs
                .get(&report.block_id)
                .is_some_and(|pending| {
                    pending.repair_id == repair_id
                        && pending.target_node_id == session.node_id
                        && pending.target_node_epoch == session.node_epoch
                        && pending.issued_at.elapsed() <= DEFAULT_REPAIR_ATTEMPT_TTL
                })
        });
        valid.then_some(()).ok_or(MetaRuntimeError::Conflict {
            expected: None,
            actual: 0,
        })
    }

    fn commit_version(
        &mut self,
        request: pb::CommitVersionRequest,
    ) -> Result<pb::CommitVersionResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let key = request
            .key
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing key".to_string()))?
            .value
            .clone();
        if key.is_empty() || request.operation_id.is_empty() || request.operation_digest.is_empty()
        {
            return Err(MetaRuntimeError::InvalidArgument(
                "key and operation identity are required".to_string(),
            ));
        }
        if let Some(previous) = self.operations.get(&request.operation_id) {
            return if previous.digest == request.operation_digest {
                Ok(previous.result)
            } else {
                Err(MetaRuntimeError::Conflict {
                    expected: None,
                    actual: previous.result.version,
                })
            };
        }
        let commit_sequence = self.validate_commit_sequence(
            session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        let candidate = request.candidate.ok_or_else(|| {
            MetaRuntimeError::InvalidArgument("missing version candidate".to_string())
        })?;
        let current = self
            .versions
            .get(&key)
            .and_then(|versions| versions.last_key_value().map(|(_, value)| value));
        let current_version = current.map_or(0, |layout| layout.version);
        validate_condition(&request.condition, request.expected_version, current)?;

        // DEL 已缺失对象是成功但没有逻辑变化。
        if candidate.kind == pb::VersionKind::Tombstone as i32
            && current.is_none_or(|layout| layout.kind == pb::VersionKind::Tombstone as i32)
        {
            let index = self.journal.last_index() + 1;
            let result = pb::CommitVersionResponse {
                version: current_version,
                revision: index,
                commit_index: index,
                changed: false,
            };
            let record = JournalRecord::OperationRemembered {
                operation_id: request.operation_id,
                operation_digest: request.operation_digest,
                result,
                commit_sequence,
            };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
            self.maybe_checkpoint(sequence)?;
            return Ok(result);
        }

        if candidate.kind == pb::VersionKind::Value as i32 {
            for extent in &candidate.extents {
                self.reject_retired_block(&extent.block_id)?;
                let proof = request
                    .replica_proofs
                    .iter()
                    .find(|proof| proof.block_id == extent.block_id);
                let new_replica = request
                    .new_replicas
                    .iter()
                    .find(|report| report.block_id == extent.block_id);
                let known = proof.is_some_and(|proof| {
                    self.replicas.get(&extent.block_id).is_some_and(|replicas| {
                        replicas.iter().any(|replica| {
                            replica.location.node_id == proof.node_id
                                && replica.location.node_epoch == proof.node_epoch
                                && replica.catalog_revision == proof.catalog_revision
                                && replica.location.checksum == proof.checksum
                        })
                    })
                });
                let known_and_live =
                    known && self.block_referenced_by_retained_versions(&extent.block_id);
                if !known_and_live && new_replica.is_none() {
                    return Err(MetaRuntimeError::InvalidArgument(
                        "every extent requires an existing proof or new replica report".to_string(),
                    ));
                }
            }
        }

        let endpoint = self
            .sessions
            .get(&session.node_id)
            .expect("verified session")
            .control_endpoint
            .clone();
        let new_replicas = request
            .new_replicas
            .into_iter()
            .map(|report| {
                self.reject_retired_block(&report.block_id)?;
                if report.block_id.is_empty() || report.length == 0 {
                    return Err(MetaRuntimeError::InvalidArgument(
                        "new replica requires block id and length".to_string(),
                    ));
                }
                Ok((
                    pb::ReplicaLocation {
                        block_id: report.block_id,
                        node_id: session.node_id,
                        node_epoch: session.node_epoch,
                        data_endpoint: endpoint.clone(),
                        checksum: report.checksum,
                        durability: report.durability,
                    },
                    report.length,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let layout = pb::VersionLayout {
            version: self.next_object_version(current_version),
            logical_length: candidate.logical_length,
            extents: candidate.extents,
            digest: candidate.digest,
            kind: candidate.kind,
        };
        let record = JournalRecord::VersionCommitted {
            key,
            layout: layout.clone(),
            modified_time_unix_millis: current_time_unix_millis(),
            new_replicas,
            operation_id: request.operation_id,
            operation_digest: request.operation_digest,
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        Ok(pb::CommitVersionResponse {
            version: layout.version,
            revision: sequence,
            commit_index: sequence,
            changed: true,
        })
    }

    fn filesystem_commit_version(
        &mut self,
        request: pb::FilesystemCommitVersionRequest,
    ) -> Result<pb::FilesystemCommitVersionResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        if request.operation_id.is_empty()
            || request.operation_digest.is_empty()
            || request.object_key.is_empty()
        {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem commit requires object and operation identity".to_string(),
            ));
        }
        if request.object_key != filesystem_object_key(request.inode) {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem object key does not match inode".to_string(),
            ));
        }
        if let Some(previous) = self
            .filesystem_version_operations
            .get(&request.operation_id)
        {
            if previous.digest != request.operation_digest {
                return Err(MetaRuntimeError::Conflict {
                    expected: None,
                    actual: previous.response.commit_index,
                });
            }
            return Ok(previous.response.clone());
        }
        let current_inode = self
            .filesystem
            .inode(request.inode)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        if current_inode.attributes.kind != InodeKind::RegularFile {
            return Err(MetaRuntimeError::InvalidArgument(
                "only regular files have DataCore content versions".to_string(),
            ));
        }
        let attribute_patch = request
            .attribute_patch
            .map(attribute_patch_from_proto)
            .transpose()
            .map_err(|error| MetaRuntimeError::InvalidArgument(format!("{error:?}")))?
            .unwrap_or_default();
        match (
            request.caller.map(caller_from_proto),
            attribute_patch.is_empty(),
        ) {
            (Some(caller), false) => {
                authorize_attribute_patch(&caller, &current_inode.attributes, &attribute_patch)?
            }
            (None, true) => {}
            _ => {
                return Err(MetaRuntimeError::InvalidArgument(
                    "filesystem commit caller and attribute patch must appear together".to_string(),
                ));
            }
        }
        if current_inode.revision != request.expected_inode_revision {
            return Err(MetaRuntimeError::Conflict {
                expected: Some(request.expected_inode_revision),
                actual: current_inode.revision,
            });
        }
        let bound_version = current_inode
            .content
            .as_ref()
            .map(|binding| binding.exact_version);
        if bound_version != request.expected_object_version {
            return Err(MetaRuntimeError::Conflict {
                expected: request.expected_object_version,
                actual: bound_version.unwrap_or_default(),
            });
        }
        let current_object_version = self
            .versions
            .get(&request.object_key)
            .and_then(|versions| versions.last_key_value().map(|(_, layout)| layout.version));
        if current_object_version != request.expected_object_version {
            return Err(MetaRuntimeError::Conflict {
                expected: request.expected_object_version,
                actual: current_object_version.unwrap_or_default(),
            });
        }
        let candidate = request.candidate.ok_or_else(|| {
            MetaRuntimeError::InvalidArgument("missing filesystem version candidate".to_string())
        })?;
        if candidate.kind != pb::VersionKind::Value as i32
            || candidate.logical_length != request.new_size
        {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem candidate must be a value whose length equals inode size".to_string(),
            ));
        }
        let referenced_blocks = validate_filesystem_candidate_layout(&candidate)?;
        for extent in &candidate.extents {
            if is_filesystem_sparse_hole(extent) {
                continue;
            }
            self.reject_retired_block(&extent.block_id)?;
            let required_end = extent
                .block_offset
                .checked_add(
                    extent
                        .logical
                        .as_ref()
                        .expect("validated logical range")
                        .length,
                )
                .expect("validated block range");
            let known = request.replica_proofs.iter().any(|proof| {
                proof.block_id == extent.block_id
                    && proof.checksum == extent.digest
                    && self.replicas.get(&extent.block_id).is_some_and(|replicas| {
                        replicas.iter().any(|replica| {
                            replica.location.node_id == proof.node_id
                                && replica.location.node_epoch == proof.node_epoch
                                && replica.catalog_revision == proof.catalog_revision
                                && replica.location.checksum == proof.checksum
                                && replica.length >= required_end
                        })
                    })
                    && self.block_referenced_by_retained_versions(&extent.block_id)
            });
            let new_replica = request.new_replicas.iter().any(|report| {
                report.block_id == extent.block_id
                    && report.checksum == extent.digest
                    && report.length >= required_end
            });
            if !known && !new_replica {
                return Err(MetaRuntimeError::InvalidArgument(
                    "every filesystem data extent requires a matching complete replica".to_string(),
                ));
            }
        }
        let mut reported_blocks = HashSet::new();
        for report in &request.new_replicas {
            if !referenced_blocks.contains(&report.block_id) {
                return Err(MetaRuntimeError::InvalidArgument(
                    "new filesystem replica is not referenced by the candidate layout".to_string(),
                ));
            }
            if !reported_blocks.insert(report.block_id.clone()) {
                return Err(MetaRuntimeError::InvalidArgument(
                    "new filesystem replica is reported more than once".to_string(),
                ));
            }
        }
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        let endpoint = self
            .sessions
            .get(&session.node_id)
            .expect("verified session")
            .control_endpoint
            .clone();
        let new_replicas = request
            .new_replicas
            .into_iter()
            .map(|report| {
                if report.block_id.is_empty() || report.length == 0 {
                    return Err(MetaRuntimeError::InvalidArgument(
                        "new filesystem replica requires block id and length".to_string(),
                    ));
                }
                Ok((
                    pb::ReplicaLocation {
                        block_id: report.block_id,
                        node_id: session.node_id,
                        node_epoch: session.node_epoch,
                        data_endpoint: endpoint.clone(),
                        checksum: report.checksum,
                        durability: report.durability,
                    },
                    report.length,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let version = self.next_object_version(current_object_version.unwrap_or_default());
        let layout = pb::VersionLayout {
            version,
            logical_length: candidate.logical_length,
            extents: candidate.extents,
            digest: candidate.digest,
            kind: candidate.kind,
        };
        let now = current_time_unix_nanos();
        let mut inode = current_inode;
        inode.reservations.retain(|reservation| {
            self.sessions
                .get(&reservation.node_id)
                .is_some_and(|owner| {
                    owner.node_epoch == reservation.node_epoch && self.is_live_session(owner)
                })
        });
        apply_filesystem_reservation_changes(
            &mut inode.reservations,
            &session,
            &request.reservation_reductions,
            &request.reservation_additions,
        )?;
        let mut xattr_updates = Vec::new();
        if let Some(mode) = attribute_patch.mode
            && let Some(access_acl) = self.filesystem.xattr(request.inode, ACL_ACCESS_NAME)
        {
            xattr_updates.push(XattrUpdate {
                inode: request.inode,
                name: ACL_ACCESS_NAME.to_vec(),
                value: Some(
                    access_acl_after_chmod(access_acl, mode)
                        .map_err(|_| MetaRuntimeError::XattrUnsupported)?,
                ),
            });
        }
        inode.revision = self.journal.last_index() + 1;
        inode.attributes.size = request.new_size;
        inode.attributes.mtime_unix_nanos = request.mtime_unix_nanos;
        apply_attribute_patch(&mut inode.attributes, &attribute_patch, now);
        inode.attributes.ctime_unix_nanos = now;
        inode.content = Some(FileContentBinding {
            object_key: request.object_key.clone(),
            exact_version: version,
        });
        let revoked_grant_generation = self.filesystem.grant_generation(request.inode);
        let new_grant_generation = revoked_grant_generation.saturating_add(1);
        let record = JournalRecord::FilesystemVersionCommitted {
            record: FilesystemVersionCommitRecord {
                object: VersionCommitRecord {
                    key: request.object_key,
                    layout,
                    modified_time_unix_millis: request.mtime_unix_nanos / 1_000_000,
                    new_replicas,
                    operation_id: request.operation_id.clone(),
                    operation_digest: request.operation_digest.clone(),
                },
                inode,
                revoked_grant_generation,
                new_grant_generation,
                xattr_updates,
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        let response = self
            .filesystem_version_operations
            .get(&request.operation_id)
            .expect("applied filesystem commit must store its idempotency response")
            .response
            .clone();
        Ok(response)
    }

    fn filesystem_set_attributes(
        &mut self,
        request: pb::FilesystemSetAttributesRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        if request.operation_id.is_empty() || request.operation_digest.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem attribute operation identity is required".to_string(),
            ));
        }
        if let Some(previous) = self
            .filesystem_attribute_operations
            .get(&request.operation_id)
        {
            if previous.digest != request.operation_digest {
                return Err(MetaRuntimeError::Conflict {
                    expected: None,
                    actual: previous.response.commit_index,
                });
            }
            return Ok(previous.response.clone());
        }

        let caller = request.caller.map(caller_from_proto).ok_or_else(|| {
            MetaRuntimeError::InvalidArgument("missing caller identity".to_string())
        })?;
        let patch = request
            .patch
            .map(attribute_patch_from_proto)
            .transpose()
            .map_err(|error| MetaRuntimeError::InvalidArgument(format!("{error:?}")))?
            .ok_or_else(|| {
                MetaRuntimeError::InvalidArgument("missing attribute patch".to_string())
            })?;
        if patch.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem attribute patch is empty".to_string(),
            ));
        }

        let mut inode = self
            .filesystem
            .inode(request.inode)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        if inode.revision != request.expected_inode_revision {
            return Err(MetaRuntimeError::Conflict {
                expected: Some(request.expected_inode_revision),
                actual: inode.revision,
            });
        }
        authorize_attribute_patch(&caller, &inode.attributes, &patch)?;

        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        let now = current_time_unix_nanos();
        let mut xattr_updates = Vec::new();
        if let Some(mode) = patch.mode
            && let Some(access_acl) = self.filesystem.xattr(request.inode, ACL_ACCESS_NAME)
        {
            xattr_updates.push(XattrUpdate {
                inode: request.inode,
                name: ACL_ACCESS_NAME.to_vec(),
                value: Some(
                    access_acl_after_chmod(access_acl, mode)
                        .map_err(|_| MetaRuntimeError::XattrUnsupported)?,
                ),
            });
        }
        apply_attribute_patch(&mut inode.attributes, &patch, now);
        inode.attributes.ctime_unix_nanos = now;
        inode.revision = self.journal.last_index() + 1;

        let revoked_grant_generation = self.filesystem.grant_generation(request.inode);
        let record = JournalRecord::FilesystemAttributesUpdated {
            record: FilesystemAttributesUpdatedRecord {
                operation_id: request.operation_id.clone(),
                operation_digest: request.operation_digest,
                inode,
                revoked_grant_generation,
                new_grant_generation: revoked_grant_generation.saturating_add(1),
                xattr_updates,
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        Ok(self
            .filesystem_attribute_operations
            .get(&request.operation_id)
            .expect("applied attribute mutation must store its idempotency response")
            .response
            .clone())
    }

    fn filesystem_get_xattr(
        &self,
        request: pb::FilesystemGetXattrRequest,
    ) -> Result<pb::FilesystemGetXattrResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        request.caller.map(caller_from_proto).ok_or_else(|| {
            MetaRuntimeError::InvalidArgument("missing caller identity".to_string())
        })?;
        validate_xattr_name(&request.name)?;
        let inode = self
            .filesystem
            .inode(request.inode)
            .ok_or(MetaRuntimeError::NotFound)?;
        let value = self.filesystem.xattr(request.inode, &request.name);
        Ok(pb::FilesystemGetXattrResponse {
            found: value.is_some(),
            value: value.unwrap_or_default().to_vec(),
            inode_revision: inode.revision,
        })
    }

    fn filesystem_list_xattrs(
        &self,
        request: pb::FilesystemListXattrsRequest,
    ) -> Result<pb::FilesystemListXattrsResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        request.caller.map(caller_from_proto).ok_or_else(|| {
            MetaRuntimeError::InvalidArgument("missing caller identity".to_string())
        })?;
        let inode = self
            .filesystem
            .inode(request.inode)
            .ok_or(MetaRuntimeError::NotFound)?;
        Ok(pb::FilesystemListXattrsResponse {
            names: self.filesystem.list_xattrs(request.inode),
            inode_revision: inode.revision,
        })
    }

    fn filesystem_set_xattr(
        &mut self,
        request: pb::FilesystemSetXattrRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        validate_filesystem_attribute_operation(&request.operation_id, &request.operation_digest)?;
        if let Some(previous) = self
            .filesystem_attribute_operations
            .get(&request.operation_id)
        {
            if previous.digest != request.operation_digest {
                return Err(MetaRuntimeError::Conflict {
                    expected: None,
                    actual: previous.response.commit_index,
                });
            }
            return Ok(previous.response.clone());
        }

        validate_xattr_name(&request.name)?;
        if request.value.len() > MAX_XATTR_VALUE_BYTES {
            return Err(MetaRuntimeError::XattrTooLarge);
        }
        let mode = XattrSetMode::from_proto(request.mode)
            .map_err(|_| MetaRuntimeError::InvalidArgument("invalid xattr set mode".to_string()))?;
        let caller = request.caller.map(caller_from_proto).ok_or_else(|| {
            MetaRuntimeError::InvalidArgument("missing caller identity".to_string())
        })?;
        let mut inode = self
            .filesystem
            .inode(request.inode)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        if inode.revision != request.expected_inode_revision {
            return Err(MetaRuntimeError::Conflict {
                expected: Some(request.expected_inode_revision),
                actual: inode.revision,
            });
        }
        authorize_xattr_mutation(&caller, &inode.attributes)?;
        let previous = self.filesystem.xattr(request.inode, &request.name);
        match (mode, previous.is_some()) {
            (XattrSetMode::CreateOnly, true) => return Err(MetaRuntimeError::XattrAlreadyExists),
            (XattrSetMode::ReplaceOnly, false) => return Err(MetaRuntimeError::XattrNotFound),
            _ => {}
        }
        let (count, bytes) = self.filesystem.xattr_usage(request.inode);
        let prior_bytes = previous.map_or(0, |value| request.name.len() + value.len());
        let next_count = count + usize::from(previous.is_none());
        let next_bytes = bytes
            .saturating_sub(prior_bytes)
            .saturating_add(request.name.len() + request.value.len());
        if next_count > MAX_XATTRS_PER_INODE || next_bytes > MAX_XATTR_BYTES_PER_INODE {
            return Err(MetaRuntimeError::XattrTooLarge);
        }
        if let Some(mode) = validate_acl_xattr(
            &request.name,
            &request.value,
            inode.attributes.kind,
            inode.attributes.mode,
        )
        .map_err(|_| MetaRuntimeError::XattrUnsupported)?
        {
            inode.attributes.mode = mode;
        }
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        inode.revision = self.journal.last_index() + 1;
        inode.attributes.ctime_unix_nanos = current_time_unix_nanos();
        let revoked_grant_generation = self.filesystem.grant_generation(request.inode);
        let record = JournalRecord::FilesystemXattrUpdated {
            record: FilesystemXattrUpdatedRecord {
                operation_id: request.operation_id.clone(),
                operation_digest: request.operation_digest,
                inode,
                revoked_grant_generation,
                new_grant_generation: revoked_grant_generation.saturating_add(1),
                update: XattrUpdate {
                    inode: request.inode,
                    name: request.name,
                    value: Some(request.value),
                },
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        Ok(self
            .filesystem_attribute_operations
            .get(&request.operation_id)
            .expect("applied xattr mutation must store its idempotency response")
            .response
            .clone())
    }

    fn filesystem_remove_xattr(
        &mut self,
        request: pb::FilesystemRemoveXattrRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError> {
        let session = request
            .session
            .clone()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(&session)?;
        validate_filesystem_attribute_operation(&request.operation_id, &request.operation_digest)?;
        if let Some(previous) = self
            .filesystem_attribute_operations
            .get(&request.operation_id)
        {
            if previous.digest != request.operation_digest {
                return Err(MetaRuntimeError::Conflict {
                    expected: None,
                    actual: previous.response.commit_index,
                });
            }
            return Ok(previous.response.clone());
        }
        validate_xattr_name(&request.name)?;
        let caller = request.caller.map(caller_from_proto).ok_or_else(|| {
            MetaRuntimeError::InvalidArgument("missing caller identity".to_string())
        })?;
        let mut inode = self
            .filesystem
            .inode(request.inode)
            .cloned()
            .ok_or(MetaRuntimeError::NotFound)?;
        if inode.revision != request.expected_inode_revision {
            return Err(MetaRuntimeError::Conflict {
                expected: Some(request.expected_inode_revision),
                actual: inode.revision,
            });
        }
        authorize_xattr_mutation(&caller, &inode.attributes)?;
        if self
            .filesystem
            .xattr(request.inode, &request.name)
            .is_none()
        {
            return Err(MetaRuntimeError::XattrNotFound);
        }
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.operation_id,
            &request.operation_digest,
        )?;
        inode.revision = self.journal.last_index() + 1;
        inode.attributes.ctime_unix_nanos = current_time_unix_nanos();
        let revoked_grant_generation = self.filesystem.grant_generation(request.inode);
        let record = JournalRecord::FilesystemXattrUpdated {
            record: FilesystemXattrUpdatedRecord {
                operation_id: request.operation_id.clone(),
                operation_digest: request.operation_digest,
                inode,
                revoked_grant_generation,
                new_grant_generation: revoked_grant_generation.saturating_add(1),
                update: XattrUpdate {
                    inode: request.inode,
                    name: request.name,
                    value: None,
                },
            },
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        Ok(self
            .filesystem_attribute_operations
            .get(&request.operation_id)
            .expect("applied xattr removal must store its idempotency response")
            .response
            .clone())
    }

    fn filesystem_stat(
        &self,
        request: pb::FilesystemStatRequest,
    ) -> Result<pb::FilesystemStatResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let now = Instant::now();
        let live = self
            .sessions
            .iter()
            .filter(|(node_id, session)| {
                !self.retired_sessions.contains(node_id)
                    && session
                        .last_heartbeat
                        .is_some_and(|last| now.duration_since(last) <= DEFAULT_NODE_LEASE_TTL)
            })
            .map(|(_, session)| session)
            .collect::<Vec<_>>();
        if live.is_empty() || live.iter().any(|session| session.resources.is_none()) {
            return Err(MetaRuntimeError::CapacityUnavailable);
        }
        let (total_bytes, available_bytes) = live.iter().fold((0_u64, 0_u64), |sum, session| {
            let resources = session
                .resources
                .as_ref()
                .expect("checked resource summary");
            (
                sum.0.saturating_add(resources.total_host_memory_bytes),
                sum.1.saturating_add(resources.available_host_memory_bytes),
            )
        });
        let used_inodes = u64::try_from(self.filesystem.inode_count()).unwrap_or(u64::MAX);
        Ok(pb::FilesystemStatResponse {
            block_size: FILESYSTEM_BLOCK_SIZE,
            total_blocks: total_bytes / FILESYSTEM_BLOCK_SIZE,
            free_blocks: available_bytes / FILESYSTEM_BLOCK_SIZE,
            available_blocks: available_bytes / FILESYSTEM_BLOCK_SIZE,
            total_inodes: self.filesystem_max_inodes,
            free_inodes: self.filesystem_max_inodes.saturating_sub(used_inodes),
            max_name_length: MAX_XATTR_NAME_BYTES as u32,
            reporting_nodes: u32::try_from(live.len()).unwrap_or(u32::MAX),
            capacity_revision: self.capacity_revision,
        })
    }

    fn commit_batch(
        &mut self,
        request: pb::CommitBatchRequest,
    ) -> Result<pb::CommitBatchResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?
            .clone();
        self.verify_session(&session)?;
        if request.entries.is_empty() || request.batch_operation_id.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "batch entries and operation identity are required".to_string(),
            ));
        }

        let mut seen_keys = HashSet::new();
        let mut prior = Vec::with_capacity(request.entries.len());
        for entry in &request.entries {
            let key = entry.key.as_ref().ok_or_else(|| {
                MetaRuntimeError::InvalidArgument("missing batch key".to_string())
            })?;
            if key.value.is_empty() || !seen_keys.insert(key.value.clone()) {
                return Err(MetaRuntimeError::InvalidArgument(
                    "batch keys must be non-empty and unique".to_string(),
                ));
            }
            if entry.operation_id.is_empty() || entry.operation_digest.is_empty() {
                return Err(MetaRuntimeError::InvalidArgument(
                    "every batch entry requires an operation identity".to_string(),
                ));
            }
            prior.push(self.operations.get(&entry.operation_id));
        }
        let prior_count = prior.iter().filter(|item| item.is_some()).count();
        if prior_count > 0 {
            if prior_count != request.entries.len() {
                return Err(MetaRuntimeError::Conflict {
                    expected: None,
                    actual: 0,
                });
            }
            let mut results = Vec::with_capacity(request.entries.len());
            let mut commit_index = 0;
            for (entry, stored) in request.entries.iter().zip(prior) {
                let stored = stored.expect("all prior operations were checked");
                if stored.digest != entry.operation_digest {
                    return Err(MetaRuntimeError::Conflict {
                        expected: None,
                        actual: stored.result.version,
                    });
                }
                commit_index = commit_index.max(stored.result.commit_index);
                results.push(pb::BatchCommitResult {
                    key: entry.key.clone(),
                    result: Some(stored.result),
                });
            }
            return Ok(pb::CommitBatchResponse {
                results,
                commit_index,
            });
        }
        let batch_digest = batch_commit_sequence_digest(&request.entries);
        let commit_sequence = self.validate_commit_sequence(
            &session,
            request.commit_sequence,
            &request.batch_operation_id,
            &batch_digest,
        )?;

        let endpoint = self
            .sessions
            .get(&session.node_id)
            .expect("verified session")
            .control_endpoint
            .clone();
        let mut commits = Vec::with_capacity(request.entries.len());
        for entry in request.entries {
            let key = entry.key.expect("validated key").value;
            let candidate = entry.candidate.ok_or_else(|| {
                MetaRuntimeError::InvalidArgument("missing batch candidate".to_string())
            })?;
            if candidate.kind != pb::VersionKind::Value as i32 {
                return Err(MetaRuntimeError::InvalidArgument(
                    "MSET batch accepts value candidates only".to_string(),
                ));
            }
            let current = self
                .versions
                .get(&key)
                .and_then(|versions| versions.last_key_value().map(|(_, value)| value));
            validate_condition(&entry.condition, entry.expected_version, current)?;
            for extent in &candidate.extents {
                self.reject_retired_block(&extent.block_id)?;
                let known = entry.replica_proofs.iter().any(|proof| {
                    proof.block_id == extent.block_id
                        && self.replicas.get(&extent.block_id).is_some_and(|replicas| {
                            replicas.iter().any(|replica| {
                                replica.location.node_id == proof.node_id
                                    && replica.location.node_epoch == proof.node_epoch
                                    && replica.catalog_revision == proof.catalog_revision
                                    && replica.location.checksum == proof.checksum
                            })
                        })
                });
                let is_new = entry
                    .new_replicas
                    .iter()
                    .any(|report| report.block_id == extent.block_id);
                let known_and_live =
                    known && self.block_referenced_by_retained_versions(&extent.block_id);
                if !known_and_live && !is_new {
                    return Err(MetaRuntimeError::InvalidArgument(
                        "every batch extent requires a proof or new replica".to_string(),
                    ));
                }
            }
            let new_replicas = entry
                .new_replicas
                .into_iter()
                .map(|report| {
                    self.reject_retired_block(&report.block_id)?;
                    if report.block_id.is_empty() || report.length == 0 {
                        return Err(MetaRuntimeError::InvalidArgument(
                            "new replica requires block id and length".to_string(),
                        ));
                    }
                    Ok((
                        pb::ReplicaLocation {
                            block_id: report.block_id,
                            node_id: session.node_id,
                            node_epoch: session.node_epoch,
                            data_endpoint: endpoint.clone(),
                            checksum: report.checksum,
                            durability: report.durability,
                        },
                        report.length,
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let current_version = current.map_or(0, |layout| layout.version);
            let next_version = self.next_object_version(current_version);
            commits.push(VersionCommitRecord {
                key,
                modified_time_unix_millis: current_time_unix_millis(),
                layout: pb::VersionLayout {
                    version: next_version,
                    logical_length: candidate.logical_length,
                    extents: candidate.extents,
                    digest: candidate.digest,
                    kind: candidate.kind,
                },
                new_replicas,
                operation_id: entry.operation_id,
                operation_digest: entry.operation_digest,
            });
        }

        let record = JournalRecord::VersionsCommitted {
            commits: commits.clone(),
            commit_sequence,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        Ok(pb::CommitBatchResponse {
            results: commits
                .into_iter()
                .map(|commit| pb::BatchCommitResult {
                    key: Some(pb::Key { value: commit.key }),
                    result: Some(pb::CommitVersionResponse {
                        version: commit.layout.version,
                        revision: sequence,
                        commit_index: sequence,
                        changed: true,
                    }),
                })
                .collect(),
            commit_index: sequence,
        })
    }

    fn stat(&self, request: pb::MetaStatRequest) -> Result<pb::MetaStatResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let key = request
            .key
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing key".to_string()))?;
        if key.value.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "stat key must be non-empty".to_string(),
            ));
        }
        Ok(match self.current_value_layout(&key.value) {
            Some(layout) => pb::MetaStatResponse {
                found: true,
                info: Some(self.object_info(key.value.clone(), layout)),
            },
            None => pb::MetaStatResponse {
                found: false,
                info: None,
            },
        })
    }

    fn scan(&self, request: pb::MetaScanRequest) -> Result<pb::MetaScanResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let prefix = request
            .prefix
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing scan prefix".to_string()))?
            .value
            .clone();
        let options = request.options.unwrap_or_default();
        let delimiter = options.delimiter;
        if options.start_after.is_some() && !options.cursor.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "scan start_after and cursor are mutually exclusive".to_string(),
            ));
        }
        let limit = scan_limit(options.limit);
        let start_after = if options.cursor.is_empty() {
            options.start_after
        } else {
            let cursor = ScanCursor::decode(&options.cursor)?;
            if cursor.expires_at_unix_millis < current_time_unix_millis() {
                return Err(MetaRuntimeError::InvalidArgument(
                    "scan cursor expired".to_string(),
                ));
            }
            if cursor.prefix != prefix {
                return Err(MetaRuntimeError::InvalidArgument(
                    "scan cursor prefix mismatch".to_string(),
                ));
            }
            if cursor.delimiter != delimiter {
                return Err(MetaRuntimeError::InvalidArgument(
                    "scan cursor delimiter mismatch".to_string(),
                ));
            }
            Some(cursor.last_key)
        };

        let mut range_start = match &start_after {
            Some(start_after) if start_after.as_slice() >= prefix.as_slice() => {
                Bound::Excluded(start_after.clone())
            }
            _ => Bound::Included(prefix.clone()),
        };
        let mut items = Vec::new();
        let mut cursor_marker = None;
        let mut next_range_start = None;
        while items.len() < limit {
            let Some(key) = self
                .live_keys
                .range((range_start.clone(), Bound::Unbounded))
                .next()
                .cloned()
            else {
                break;
            };
            if !key.starts_with(&prefix) {
                break;
            }
            let Some(layout) = self.current_value_layout(&key) else {
                range_start = Bound::Excluded(key.clone());
                continue;
            };
            if let Some(group_prefix) = scan_group_prefix(&prefix, &delimiter, &key) {
                if start_after
                    .as_ref()
                    .is_some_and(|marker| group_prefix.as_slice() <= marker.as_slice())
                {
                    let Some(successor) = lexicographic_successor(&group_prefix) else {
                        break;
                    };
                    range_start = Bound::Included(successor);
                    continue;
                }
                cursor_marker = Some(group_prefix.clone());
                items.push(Self::prefix_object_info(group_prefix.clone()));
                next_range_start = lexicographic_successor(&group_prefix).map(Bound::Included);
                if let Some(next_start) = next_range_start.clone() {
                    range_start = next_start;
                } else {
                    break;
                }
                continue;
            }
            items.push(self.object_info(key.clone(), layout));
            cursor_marker = Some(key.clone());
            next_range_start = Some(Bound::Excluded(key.clone()));
            range_start = Bound::Excluded(key);
            if items.len() == limit {
                break;
            }
        }
        let next_cursor = if items.len() == limit {
            let last_key = cursor_marker.expect("non-empty page at limit");
            // 只看当前页后面的下一个有序 key，避免跨过前缀区间后线性扫描所有后续 key。
            let has_more = next_range_start.is_some_and(|range_start| {
                self.live_keys
                    .range((range_start, Bound::Unbounded))
                    .next()
                    .is_some_and(|key| key.starts_with(&prefix))
            });
            if has_more {
                ScanCursor {
                    prefix,
                    delimiter,
                    last_key,
                    expires_at_unix_millis: current_time_unix_millis() + SCAN_CURSOR_TTL_MILLIS,
                }
                .encode()
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        Ok(pb::MetaScanResponse { items, next_cursor })
    }

    fn reject_retired_block(&self, block_id: &[u8]) -> Result<(), MetaRuntimeError> {
        if self.retired_block_fences.contains_key(block_id) {
            return Err(MetaRuntimeError::Conflict {
                expected: None,
                actual: 0,
            });
        }
        Ok(())
    }

    fn next_object_version(&self, current_version: u64) -> u64 {
        (self.journal.last_index() + 1)
            .max(current_version + 1)
            .max(self.version_floor + 1)
    }

    fn block_referenced_by_retained_versions(&self, block_id: &[u8]) -> bool {
        self.versions
            .values()
            .flat_map(|versions| versions.values())
            .flat_map(|layout| layout.extents.iter())
            .any(|extent| extent.block_id == block_id)
    }

    fn validate_commit_sequence(
        &self,
        session: &pb::NodeSessionIdentity,
        commit_sequence: u64,
        operation_id: &[u8],
        operation_digest: &[u8],
    ) -> Result<Option<CommitSequenceRecord>, MetaRuntimeError> {
        let session_state = self
            .sessions
            .get(&session.node_id)
            .ok_or(MetaRuntimeError::UnknownSession)?;
        if !session_state.supports_commit_sequence {
            return if commit_sequence == 0 {
                Ok(None)
            } else {
                Err(MetaRuntimeError::InvalidArgument(
                    "commit sequence requires negotiated session support".to_string(),
                ))
            };
        }
        if commit_sequence == 0 {
            return Err(MetaRuntimeError::InvalidArgument(
                "commit sequence is required for this session".to_string(),
            ));
        }
        if commit_sequence <= session_state.commit_sequence_floor {
            return Err(MetaRuntimeError::Conflict {
                expected: Some(session_state.commit_sequence_floor + 1),
                actual: commit_sequence,
            });
        }
        if let Some(previous) = session_state.commit_sequences.get(&commit_sequence) {
            if previous.operation_id == operation_id
                && previous.operation_digest == operation_digest
            {
                return Err(MetaRuntimeError::Conflict {
                    expected: None,
                    actual: commit_sequence,
                });
            }
            return Err(MetaRuntimeError::Conflict {
                expected: Some(previous.commit_sequence),
                actual: commit_sequence,
            });
        }
        Ok(Some(CommitSequenceRecord {
            node_id: session.node_id,
            node_epoch: session.node_epoch,
            commit_sequence,
            operation_id: operation_id.to_vec(),
            operation_digest: operation_digest.to_vec(),
            commit_index: self.journal.last_index() + 1,
        }))
    }

    fn remember_commit_sequence(&mut self, record: CommitSequenceRecord) {
        let node_floor = self
            .node_commit_sequence_floors
            .entry(record.node_id)
            .or_default();
        if let Some(session) = self.sessions.get_mut(&record.node_id)
            && session.node_epoch == record.node_epoch
        {
            session
                .commit_sequences
                .insert(record.commit_sequence, record.clone());
            if session.commit_sequence_floor > *node_floor {
                *node_floor = session.commit_sequence_floor;
            }
        }
    }

    fn advance_commit_sequence_floor(&mut self, record: &CommitSequenceRecord) {
        let floor = self
            .node_commit_sequence_floors
            .entry(record.node_id)
            .or_default();
        *floor = (*floor).max(record.commit_sequence);
        if let Some(session) = self.sessions.get_mut(&record.node_id)
            && session.node_epoch == record.node_epoch
        {
            session.commit_sequences.remove(&record.commit_sequence);
            session.commit_sequence_floor =
                session.commit_sequence_floor.max(record.commit_sequence);
        }
    }

    fn current_value_layout(&self, key: &[u8]) -> Option<&pb::VersionLayout> {
        let layout = self
            .versions
            .get(key)?
            .last_key_value()
            .map(|(_, layout)| layout)?;
        (layout.kind == pb::VersionKind::Value as i32).then_some(layout)
    }

    fn object_info(&self, key: Vec<u8>, layout: &pb::VersionLayout) -> pb::ObjectInfo {
        pb::ObjectInfo {
            key: Some(pb::Key { value: key.clone() }),
            length: layout.logical_length,
            // 旧 WAL/snapshot 没有 mtime 辅助记录时使用 0，明确表示兼容默认值。
            modified_time_unix_millis: self
                .version_modified_times
                .get(&(key, layout.version))
                .copied()
                .unwrap_or_default(),
            version: layout.version,
            is_prefix: false,
        }
    }

    fn prefix_object_info(key: Vec<u8>) -> pb::ObjectInfo {
        pb::ObjectInfo {
            key: Some(pb::Key { value: key }),
            length: 0,
            modified_time_unix_millis: 0,
            version: 0,
            is_prefix: true,
        }
    }

    fn dispatch_commit_version(
        &mut self,
        request: pb::CommitVersionRequest,
    ) -> Result<CommitDispatch, MetaRuntimeError> {
        let source_node = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?
            .node_id;
        let operation_id = request.operation_id.clone();
        let response = self.commit_version(request)?;
        let event_cursor = self
            .operations
            .get(&operation_id)
            .and_then(|operation| operation.visibility_cursor);
        let (waiting_nodes, waiting_prior_lease_nodes) = event_cursor
            .map(|cursor| self.visibility_barrier_nodes(source_node, cursor))
            .unwrap_or_default();
        Ok(CommitDispatch {
            operation_ids: vec![operation_id],
            response,
            event_cursor,
            waiting_nodes,
            waiting_prior_lease_nodes,
        })
    }

    fn dispatch_filesystem_commit_version(
        &mut self,
        request: pb::FilesystemCommitVersionRequest,
    ) -> Result<FilesystemCommitDispatch, MetaRuntimeError> {
        let source_node = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?
            .node_id;
        let operation_id = request.operation_id.clone();
        let response = self.filesystem_commit_version(request)?;
        let event_cursor = self
            .filesystem_version_operations
            .get(&operation_id)
            .and_then(|operation| operation.visibility_cursor);
        let (waiting_nodes, waiting_prior_lease_nodes) = event_cursor
            .map(|cursor| self.visibility_barrier_nodes(source_node, cursor))
            .unwrap_or_default();
        Ok(FilesystemCommitDispatch {
            operation_id,
            response,
            event_cursor,
            waiting_nodes,
            waiting_prior_lease_nodes,
        })
    }

    fn dispatch_filesystem_set_attributes(
        &mut self,
        request: pb::FilesystemSetAttributesRequest,
    ) -> Result<FilesystemAttributeDispatch, MetaRuntimeError> {
        let source_node = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?
            .node_id;
        let operation_id = request.operation_id.clone();
        let response = self.filesystem_set_attributes(request)?;
        let event_cursor = self
            .filesystem_attribute_operations
            .get(&operation_id)
            .and_then(|operation| operation.visibility_cursor);
        let (waiting_nodes, waiting_prior_lease_nodes) = event_cursor
            .map(|cursor| self.visibility_barrier_nodes(source_node, cursor))
            .unwrap_or_default();
        Ok(FilesystemAttributeDispatch {
            operation_id,
            response,
            event_cursor,
            waiting_nodes,
            waiting_prior_lease_nodes,
        })
    }

    fn dispatch_filesystem_set_xattr(
        &mut self,
        request: pb::FilesystemSetXattrRequest,
    ) -> Result<FilesystemAttributeDispatch, MetaRuntimeError> {
        let source_node = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?
            .node_id;
        let operation_id = request.operation_id.clone();
        let response = self.filesystem_set_xattr(request)?;
        Ok(self.dispatch_filesystem_attributes(source_node, operation_id, response))
    }

    fn dispatch_filesystem_remove_xattr(
        &mut self,
        request: pb::FilesystemRemoveXattrRequest,
    ) -> Result<FilesystemAttributeDispatch, MetaRuntimeError> {
        let source_node = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?
            .node_id;
        let operation_id = request.operation_id.clone();
        let response = self.filesystem_remove_xattr(request)?;
        Ok(self.dispatch_filesystem_attributes(source_node, operation_id, response))
    }

    fn dispatch_filesystem_attributes(
        &self,
        source_node: u64,
        operation_id: Vec<u8>,
        response: pb::FilesystemAttributeMutationResponse,
    ) -> FilesystemAttributeDispatch {
        let event_cursor = self
            .filesystem_attribute_operations
            .get(&operation_id)
            .and_then(|operation| operation.visibility_cursor);
        let (waiting_nodes, waiting_prior_lease_nodes) = event_cursor
            .map(|cursor| self.visibility_barrier_nodes(source_node, cursor))
            .unwrap_or_default();
        FilesystemAttributeDispatch {
            operation_id,
            response,
            event_cursor,
            waiting_nodes,
            waiting_prior_lease_nodes,
        }
    }

    fn register_pending_commit(
        &mut self,
        dispatch: CommitDispatch,
        reply: oneshot::Sender<Result<pb::CommitVersionResponse, MetaRuntimeError>>,
    ) {
        let Some(cursor) = dispatch.event_cursor else {
            let _ = reply.send(Ok(dispatch.response));
            return;
        };
        if dispatch.waiting_nodes.is_empty() && dispatch.waiting_prior_lease_nodes.is_empty() {
            self.complete_operation_visibility(&dispatch.operation_ids);
            let _ = reply.send(Ok(dispatch.response));
            return;
        }
        self.pending_commits
            .entry(cursor)
            .or_default()
            .push(PendingCommit {
                operation_ids: dispatch.operation_ids,
                response: PendingResponse::Single(dispatch.response),
                waiting_nodes: dispatch.waiting_nodes,
                waiting_prior_lease_nodes: dispatch.waiting_prior_lease_nodes,
                reply: PendingReply::Single(reply),
            });
    }

    fn register_pending_batch(
        &mut self,
        source_node: u64,
        operation_ids: Vec<Vec<u8>>,
        response: pb::CommitBatchResponse,
        reply: oneshot::Sender<Result<pb::CommitBatchResponse, MetaRuntimeError>>,
    ) {
        let Some(cursor) = operation_ids
            .iter()
            .filter_map(|operation_id| {
                self.operations
                    .get(operation_id)
                    .and_then(|operation| operation.visibility_cursor)
            })
            .max()
        else {
            let _ = reply.send(Ok(response));
            return;
        };
        let (waiting_nodes, waiting_prior_lease_nodes) =
            self.visibility_barrier_nodes(source_node, cursor);
        if waiting_nodes.is_empty() && waiting_prior_lease_nodes.is_empty() {
            self.complete_operation_visibility(&operation_ids);
            let _ = reply.send(Ok(response));
            return;
        }
        self.pending_commits
            .entry(cursor)
            .or_default()
            .push(PendingCommit {
                operation_ids,
                response: PendingResponse::Batch(response),
                waiting_nodes,
                waiting_prior_lease_nodes,
                reply: PendingReply::Batch(reply),
            });
    }

    fn register_pending_filesystem_commit(
        &mut self,
        dispatch: FilesystemCommitDispatch,
        reply: oneshot::Sender<Result<pb::FilesystemCommitVersionResponse, MetaRuntimeError>>,
    ) {
        let Some(cursor) = dispatch.event_cursor else {
            let _ = reply.send(Ok(dispatch.response));
            return;
        };
        if dispatch.waiting_nodes.is_empty() && dispatch.waiting_prior_lease_nodes.is_empty() {
            self.complete_operation_visibility(std::slice::from_ref(&dispatch.operation_id));
            let _ = reply.send(Ok(dispatch.response));
            return;
        }
        self.pending_commits
            .entry(cursor)
            .or_default()
            .push(PendingCommit {
                operation_ids: vec![dispatch.operation_id],
                response: PendingResponse::Filesystem(Box::new(dispatch.response)),
                waiting_nodes: dispatch.waiting_nodes,
                waiting_prior_lease_nodes: dispatch.waiting_prior_lease_nodes,
                reply: PendingReply::Filesystem(reply),
            });
    }

    fn register_pending_filesystem_attributes(
        &mut self,
        dispatch: FilesystemAttributeDispatch,
        reply: oneshot::Sender<Result<pb::FilesystemAttributeMutationResponse, MetaRuntimeError>>,
    ) {
        let Some(cursor) = dispatch.event_cursor else {
            let _ = reply.send(Ok(dispatch.response));
            return;
        };
        if dispatch.waiting_nodes.is_empty() && dispatch.waiting_prior_lease_nodes.is_empty() {
            self.complete_operation_visibility(std::slice::from_ref(&dispatch.operation_id));
            let _ = reply.send(Ok(dispatch.response));
            return;
        }
        self.pending_commits
            .entry(cursor)
            .or_default()
            .push(PendingCommit {
                operation_ids: vec![dispatch.operation_id],
                response: PendingResponse::FilesystemAttributes(Box::new(dispatch.response)),
                waiting_nodes: dispatch.waiting_nodes,
                waiting_prior_lease_nodes: dispatch.waiting_prior_lease_nodes,
                reply: PendingReply::FilesystemAttributes(reply),
            });
    }

    fn dispatch_filesystem_namespace<T>(
        &self,
        source_node: u64,
        operation_id: Vec<u8>,
        response: T,
    ) -> FilesystemNamespaceDispatch<T> {
        let event_cursor = self
            .filesystem_namespace_operations
            .get(&operation_id)
            .and_then(|operation| operation.visibility_cursor);
        let (waiting_nodes, waiting_prior_lease_nodes) = event_cursor
            .map(|cursor| self.visibility_barrier_nodes(source_node, cursor))
            .unwrap_or_default();
        FilesystemNamespaceDispatch {
            operation_id,
            response,
            event_cursor,
            waiting_nodes,
            waiting_prior_lease_nodes,
        }
    }

    fn register_pending_filesystem_create(
        &mut self,
        dispatch: FilesystemNamespaceDispatch<pb::FilesystemResolveResponse>,
        reply: oneshot::Sender<Result<pb::FilesystemResolveResponse, MetaRuntimeError>>,
    ) {
        let Some(cursor) = dispatch.event_cursor else {
            let _ = reply.send(Ok(dispatch.response));
            return;
        };
        if dispatch.waiting_nodes.is_empty() && dispatch.waiting_prior_lease_nodes.is_empty() {
            self.complete_operation_visibility(std::slice::from_ref(&dispatch.operation_id));
            let _ = reply.send(Ok(dispatch.response));
            return;
        }
        self.pending_commits
            .entry(cursor)
            .or_default()
            .push(PendingCommit {
                operation_ids: vec![dispatch.operation_id],
                response: PendingResponse::FilesystemCreate(Box::new(dispatch.response)),
                waiting_nodes: dispatch.waiting_nodes,
                waiting_prior_lease_nodes: dispatch.waiting_prior_lease_nodes,
                reply: PendingReply::FilesystemCreate(reply),
            });
    }

    fn register_pending_filesystem_namespace(
        &mut self,
        dispatch: FilesystemNamespaceDispatch<pb::FilesystemNamespaceMutationResponse>,
        reply: oneshot::Sender<Result<pb::FilesystemNamespaceMutationResponse, MetaRuntimeError>>,
    ) {
        let Some(cursor) = dispatch.event_cursor else {
            let _ = reply.send(Ok(dispatch.response));
            return;
        };
        if dispatch.waiting_nodes.is_empty() && dispatch.waiting_prior_lease_nodes.is_empty() {
            self.complete_operation_visibility(std::slice::from_ref(&dispatch.operation_id));
            let _ = reply.send(Ok(dispatch.response));
            return;
        }
        self.pending_commits
            .entry(cursor)
            .or_default()
            .push(PendingCommit {
                operation_ids: vec![dispatch.operation_id],
                response: PendingResponse::FilesystemNamespace(Box::new(dispatch.response)),
                waiting_nodes: dispatch.waiting_nodes,
                waiting_prior_lease_nodes: dispatch.waiting_prior_lease_nodes,
                reply: PendingReply::FilesystemNamespace(reply),
            });
    }

    fn get_operation(
        &self,
        request: pb::GetOperationRequest,
    ) -> Result<pb::GetOperationResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let operation = self
            .operations
            .get(&request.operation_id)
            .ok_or(MetaRuntimeError::NotFound)?;
        Ok(pb::GetOperationResponse {
            state: if operation.visibility_cursor.is_some() {
                "waiting_visibility"
            } else {
                "committed"
            }
            .to_string(),
            committed: Some(operation.result),
        })
    }

    fn plan_replicas(
        &self,
        request: pb::PlanReplicasRequest,
    ) -> Result<pb::PlanReplicasResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        if request.block_id.is_empty() || request.length == 0 || request.desired_copies == 0 {
            return Err(MetaRuntimeError::InvalidArgument(
                "block id, length and desired copies are required".to_string(),
            ));
        }
        let existing = request
            .existing_node_ids
            .into_iter()
            .chain(std::iter::once(session.node_id))
            .collect::<HashSet<_>>();
        let live_existing_count = existing
            .iter()
            .filter(|node_id| {
                self.sessions
                    .get(node_id)
                    .is_some_and(|session| self.is_live_session(session))
            })
            .count();
        let target_count = (request.desired_copies as usize).saturating_sub(live_existing_count);
        let targets = self
            .sessions
            .iter()
            .filter(|(node_id, session)| {
                !existing.contains(node_id) && self.is_live_session(session)
            })
            .take(target_count)
            .map(|(node_id, session)| pb::PlacementTarget {
                node_id: *node_id,
                node_epoch: session.node_epoch,
                data_endpoint: session.control_endpoint.clone(),
                transports: vec!["grpc".to_string()],
            })
            .collect::<Vec<_>>();
        let mut plan_seed = request.block_id.clone();
        plan_seed.extend_from_slice(&self.last_applied_index.to_be_bytes());
        plan_seed.extend_from_slice(&(targets.len() as u64).to_be_bytes());
        Ok(pb::PlanReplicasResponse {
            plan_id: digest(&plan_seed),
            targets,
            expires_after_millis: 30_000,
        })
    }

    fn watch_node_events(
        &mut self,
        request: pb::WatchNodeEventsRequest,
        sender: mpsc::Sender<pb::NodeEvent>,
    ) -> Result<(), MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let authoritative_cursor = self
            .sessions
            .get(&session.node_id)
            .expect("verified session")
            .last_acked_cursor;
        // A reconnect may report a cursor behind Meta and receive a replay. It
        // may never skip ahead of Meta's authoritative ACK watermark.
        let replay_after = request.last_acked_cursor.min(authoritative_cursor);
        self.watchers.insert(
            session.node_id,
            NodeWatcher {
                sender,
                delivered_cursor: replay_after,
                replay_through: self.event_high_watermark,
                next_gc_retry_at: Some(Instant::now() + GC_RETRY_INTERVAL),
                last_gc_retry_cursor: replay_after,
                disconnected: false,
            },
        );
        // 先返回可消费的流，只填充现有容量；历史保留在同一个事件索引中。
        // 后续 ACK/维护 tick 继续投递，不复制无界 backlog，也不把满队列当作撤销缓存。
        self.pump_watchers();
        Ok(())
    }

    fn pump_watchers(&mut self) {
        let now = Instant::now();
        for (node_id, watcher) in &mut self.watchers {
            if watcher.disconnected {
                continue;
            }
            let start = self
                .events
                .partition_point(|event| event.cursor <= watcher.delivered_cursor);
            let mut delivered_fresh = false;
            for event in &self.events[start..] {
                if !event_targets_node(event, *node_id) {
                    watcher.delivered_cursor = event.cursor;
                    continue;
                }
                match watcher.sender.try_send(event.clone()) {
                    Ok(()) => {
                        watcher.delivered_cursor = event.cursor;
                        delivered_fresh = true;
                        self.metrics.record_watch_event(
                            event_metric_type(event),
                            if event.cursor <= watcher.replay_through {
                                WatchDelivery::Replayed
                            } else {
                                WatchDelivery::Delivered
                            },
                        );
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => break,
                    Err(mpsc::error::TrySendError::Closed(event)) => {
                        // 只计本次连接终止的投递失败；事件仍保留等待重放与 ACK。
                        watcher.disconnected = true;
                        self.metrics
                            .record_watch_event(event_metric_type(&event), WatchDelivery::Dropped);
                        break;
                    }
                }
            }
            if watcher.disconnected {
                continue;
            }
            if delivered_fresh {
                // Fresh watch traffic has priority, but it must not keep
                // postponing an already due GC retry forever under a hot
                // writer. Only initialize an empty timer; preserve due timers
                // so spare stream capacity can carry one GC-only retry below.
                watcher
                    .next_gc_retry_at
                    .get_or_insert(now + GC_RETRY_INTERVAL);
            }
            if watcher
                .next_gc_retry_at
                .is_some_and(|retry_at| retry_at > now)
            {
                continue;
            }
            let Some(event) = outstanding_retirement_retry_event(
                &self.events,
                &self.pending_retirements,
                *node_id,
                watcher.delivered_cursor,
                watcher.last_gc_retry_cursor,
            )
            .cloned() else {
                watcher.next_gc_retry_at = None;
                continue;
            };
            watcher.next_gc_retry_at = Some(now + GC_RETRY_INTERVAL);
            match watcher.sender.try_send(event.clone()) {
                Ok(()) => {
                    watcher.last_gc_retry_cursor = event.cursor;
                    self.metrics
                        .record_watch_event(event_metric_type(&event), WatchDelivery::Replayed);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(event)) => {
                    watcher.disconnected = true;
                    self.metrics
                        .record_watch_event(event_metric_type(&event), WatchDelivery::Dropped);
                }
            }
        }
    }

    fn acknowledge_node_event(
        &mut self,
        request: pb::AcknowledgeNodeEventRequest,
    ) -> Result<(), MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        // ACK means the event's state transition is durably applied. A failed
        // or retryable application must leave the cursor unchanged so Watch
        // reconnect can replay the same idempotent event.
        if request.result != "applied" {
            return Err(MetaRuntimeError::InvalidArgument(
                "only an applied event may advance the acknowledgement cursor".to_string(),
            ));
        }
        if request.cursor > self.event_high_watermark {
            return Err(MetaRuntimeError::InvalidArgument(
                "event acknowledgement exceeds high watermark".to_string(),
            ));
        }
        if self
            .sessions
            .get(&session.node_id)
            .expect("verified session")
            .last_acked_cursor
            >= request.cursor
        {
            return Ok(());
        }
        // 先从仍完整的事件队列中记下 Repair 身份。checkpoint/retention 可能在
        // ACK journal 落盘后裁剪已确认事件，因此不能等 `maybe_checkpoint` 之后
        // 再反查事件，否则 pending_repairs 会永久残留并阻止后续修复。
        let applied_repair = (request.result == "applied")
            .then(|| {
                self.events
                    .iter()
                    .find(|event| event.cursor == request.cursor)
                    .and_then(|event| match &event.event {
                        Some(pb::node_event::Event::RepairReplica(repair))
                            if repair
                                .target
                                .as_ref()
                                .is_some_and(|target| target.node_id == session.node_id) =>
                        {
                            Some(repair.block_id.clone())
                        }
                        _ => None,
                    })
            })
            .flatten();
        let acknowledged_cursor =
            self.cursor_before_outstanding_retirement(session.node_id, request.cursor);
        if self
            .sessions
            .get(&session.node_id)
            .expect("verified session")
            .last_acked_cursor
            >= acknowledged_cursor
        {
            return Ok(());
        }
        let record = JournalRecord::NodeEventAcknowledged {
            node_id: session.node_id,
            cursor: acknowledged_cursor,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        self.advance_pending_commits_for_ack(session.node_id, acknowledged_cursor);
        if acknowledged_cursor == request.cursor
            && let Some(block_id) = applied_repair
        {
            self.pending_repairs.remove(&block_id);
            self.metrics
                .record_repair_transition(RepairTransition::Completed);
        }
        Ok(())
    }

    /// Reconstructs ephemeral repair-attempt state from retained events after
    /// snapshot/tail replay. Events for an acknowledged or obsolete target
    /// incarnation are dropped: the durable replica policy will derive a fresh
    /// attempt on the next heartbeat if the Block is still under-replicated.
    fn rebuild_pending_repairs(&mut self) {
        self.pending_repairs.clear();
        let retained_events = std::mem::take(&mut self.events);
        for event in retained_events {
            let Some(pb::node_event::Event::RepairReplica(repair)) = &event.event else {
                self.events.push(event);
                continue;
            };
            let Some(target) = repair.target.as_ref() else {
                continue;
            };
            let keep = self.sessions.get(&target.node_id).is_some_and(|session| {
                session.node_epoch == target.node_epoch && session.last_acked_cursor < event.cursor
            });
            if !keep {
                continue;
            }
            self.pending_repairs.insert(
                repair.block_id.clone(),
                PendingRepair {
                    repair_id: repair.repair_id.clone(),
                    target_node_id: target.node_id,
                    target_node_epoch: target.node_epoch,
                    event_cursor: event.cursor,
                    issued_at: Instant::now(),
                },
            );
            self.events.push(event);
        }
    }

    fn cursor_before_outstanding_retirement(&self, node_id: u64, requested_cursor: u64) -> u64 {
        cursor_before_outstanding_retirement_in(
            &self.events,
            &self.pending_retirements,
            node_id,
            requested_cursor,
        )
    }

    fn advance_retirement_ack_cursor(
        &mut self,
        participant: &BlockRetirementParticipant,
        retirement_id: &[u8],
        ack_kind: pb::BlockRetirementAckKind,
    ) {
        let phase = match ack_kind {
            pb::BlockRetirementAckKind::Prepared => pb::BlockRetirementPhase::Prepare,
            pb::BlockRetirementAckKind::Released => pb::BlockRetirementPhase::Final,
            pb::BlockRetirementAckKind::Unspecified => return,
        };
        let Some(cursor) = self
            .events
            .iter()
            .find(|event| match &event.event {
                Some(pb::node_event::Event::EvictReplica(evict)) => {
                    evict.retirement_id == retirement_id
                        && evict.participant_node_id == participant.node_id
                        && evict.participant_node_epoch == participant.node_epoch
                        && pb::BlockRetirementPhase::try_from(evict.phase).ok() == Some(phase)
                }
                _ => false,
            })
            .map(|event| event.cursor)
        else {
            return;
        };
        let acknowledged_cursor =
            self.cursor_before_outstanding_retirement(participant.node_id, cursor);
        if let Some(session) = self.sessions.get_mut(&participant.node_id)
            && session.node_epoch == participant.node_epoch
        {
            session.last_acked_cursor = session.last_acked_cursor.max(acknowledged_cursor);
        }
    }

    /// Removes expired attempts and returns an under-replicated timed-out target
    /// for each Block. The scheduler avoids that target once, preferring a
    /// different live Node; if no alternative exists, a later heartbeat may
    /// retry the same Node with a new repair id.
    fn expire_pending_repairs(&mut self) -> HashMap<Vec<u8>, u64> {
        let mut expired_targets = HashMap::new();
        let mut discarded_cursors = HashSet::new();
        let candidates = self
            .pending_repairs
            .iter()
            .map(|(block_id, pending)| (block_id.clone(), pending.clone()))
            .collect::<Vec<_>>();
        for (block_id, pending) in candidates {
            let desired = self
                .desired_replica_counts
                .get(&block_id)
                .copied()
                .unwrap_or(1) as usize;
            let live_count = self.replicas.get(&block_id).map_or(0, |replicas| {
                replicas
                    .iter()
                    .filter(|replica| self.is_live_replica(replica))
                    .count()
            });
            let target_is_live =
                self.sessions
                    .get(&pending.target_node_id)
                    .is_some_and(|session| {
                        session.node_epoch == pending.target_node_epoch
                            && self.is_live_session(session)
                    });
            let expired =
                !target_is_live || pending.issued_at.elapsed() > DEFAULT_REPAIR_ATTEMPT_TTL;
            if !expired {
                continue;
            }
            self.pending_repairs.remove(&block_id);
            self.metrics
                .record_repair_transition(RepairTransition::Expired);
            discarded_cursors.insert(pending.event_cursor);
            if live_count < desired {
                expired_targets.insert(block_id, pending.target_node_id);
            }
        }
        if !discarded_cursors.is_empty() {
            self.events
                .retain(|event| !discarded_cursors.contains(&event.cursor));
        }
        expired_targets
    }

    /// Detects a replica-count regression after lease expiry and emits one
    /// targeted, replayable repair command. The desired count is explicit
    /// policy, never a count derived from historical locations.
    fn schedule_repairs(&mut self) {
        let expired_targets = self.expire_pending_repairs();
        let mut repairs = Vec::new();
        for (block_id, replicas) in &self.replicas {
            if self.pending_repairs.contains_key(block_id) {
                continue;
            }
            let desired = self
                .desired_replica_counts
                .get(block_id)
                .copied()
                .unwrap_or(1) as usize;
            if desired < 2 {
                continue;
            }
            let live = replicas
                .iter()
                .filter(|replica| self.is_live_replica(replica))
                .collect::<Vec<_>>();
            if live.is_empty() || live.len() >= desired {
                continue;
            }
            let occupied = live
                .iter()
                .map(|replica| replica.location.node_id)
                .collect::<HashSet<_>>();
            let recently_expired = expired_targets.get(block_id).copied();
            let Some((&target_node_id, target_session)) =
                self.sessions.iter().find(|(node_id, session)| {
                    !occupied.contains(node_id)
                        && recently_expired != Some(**node_id)
                        && self.is_live_session(session)
                })
            else {
                continue;
            };
            let source = live[0];
            repairs.push((
                block_id.clone(),
                live.iter()
                    .map(|replica| replica.location.clone())
                    .collect::<Vec<_>>(),
                pb::PlacementTarget {
                    node_id: target_node_id,
                    node_epoch: target_session.node_epoch,
                    data_endpoint: target_session.control_endpoint.clone(),
                    transports: vec!["grpc".to_string()],
                },
                source.length,
                desired as u32,
            ));
        }
        for (block_id, sources, target, expected_length, desired_copies) in repairs {
            let cursor = self.event_high_watermark + 1;
            self.event_high_watermark = cursor;
            let mut repair_seed = block_id.clone();
            repair_seed.extend_from_slice(&target.node_id.to_be_bytes());
            repair_seed.extend_from_slice(&target.node_epoch.to_be_bytes());
            repair_seed.extend_from_slice(&cursor.to_be_bytes());
            let repair_id = digest(&repair_seed);
            let repair = pb::RepairReplicaEvent {
                repair_id: repair_id.clone(),
                block_id: block_id.clone(),
                sources,
                target: Some(target.clone()),
                expected_length,
                desired_copies,
            };
            self.pending_repairs.insert(
                block_id,
                PendingRepair {
                    repair_id,
                    target_node_id: target.node_id,
                    target_node_epoch: target.node_epoch,
                    event_cursor: cursor,
                    issued_at: Instant::now(),
                },
            );
            self.metrics
                .record_repair_transition(RepairTransition::Issued);
            let event = pb::NodeEvent {
                event_id: cursor.to_be_bytes().to_vec(),
                cursor,
                event: Some(pb::node_event::Event::RepairReplica(repair)),
            };
            self.events.push(event.clone());
            self.pump_watchers();
        }
    }

    fn advance_pending_commits_for_ack(&mut self, node_id: u64, acknowledged_cursor: u64) {
        let cursors = self
            .pending_commits
            .range(..=acknowledged_cursor)
            .map(|(cursor, _)| *cursor)
            .collect::<Vec<_>>();
        for cursor in cursors {
            let Some(pending) = self.pending_commits.get_mut(&cursor) else {
                continue;
            };
            for commit in pending.iter_mut() {
                commit.waiting_nodes.remove(&node_id);
            }
            self.complete_ready_pending_commits(cursor);
        }
    }

    fn advance_pending_commits_for_prior_lease(&mut self, node_id: u64) {
        let cursors = self.pending_commits.keys().copied().collect::<Vec<_>>();
        for cursor in cursors {
            let Some(pending) = self.pending_commits.get_mut(&cursor) else {
                continue;
            };
            for commit in pending.iter_mut() {
                commit.waiting_prior_lease_nodes.remove(&node_id);
            }
            self.complete_ready_pending_commits(cursor);
        }
    }

    fn complete_ready_pending_commits(&mut self, cursor: u64) {
        let mut unresolved = Vec::new();
        for commit in self.pending_commits.remove(&cursor).unwrap_or_default() {
            if commit.waiting_nodes.is_empty() && commit.waiting_prior_lease_nodes.is_empty() {
                self.complete_operation_visibility(&commit.operation_ids);
                match (commit.reply, commit.response) {
                    (PendingReply::Single(reply), PendingResponse::Single(response)) => {
                        let _ = reply.send(Ok(response));
                    }
                    (PendingReply::Batch(reply), PendingResponse::Batch(response)) => {
                        let _ = reply.send(Ok(response));
                    }
                    (PendingReply::Filesystem(reply), PendingResponse::Filesystem(response)) => {
                        let _ = reply.send(Ok(*response));
                    }
                    (
                        PendingReply::FilesystemCreate(reply),
                        PendingResponse::FilesystemCreate(response),
                    ) => {
                        let _ = reply.send(Ok(*response));
                    }
                    (
                        PendingReply::FilesystemNamespace(reply),
                        PendingResponse::FilesystemNamespace(response),
                    ) => {
                        let _ = reply.send(Ok(*response));
                    }
                    (
                        PendingReply::FilesystemAttributes(reply),
                        PendingResponse::FilesystemAttributes(response),
                    ) => {
                        let _ = reply.send(Ok(*response));
                    }
                    _ => unreachable!("pending response/reply kinds must match"),
                }
            } else {
                unresolved.push(commit);
            }
        }
        if !unresolved.is_empty() {
            self.pending_commits.insert(cursor, unresolved);
        }
    }

    /// 投递失败不构成缓存撤销证明：断流 watcher 仍然参与 visibility barrier。
    /// source Node 在 Meta 回复后执行本机 Client barrier，因此不需要等待自己的
    /// 当前 Watch ACK；但 Meta 恢复前已经发出的旧 lease 仍要等真实到期。
    fn visibility_barrier_nodes(
        &self,
        source_node: u64,
        cursor: u64,
    ) -> (HashSet<u64>, HashSet<u64>) {
        let now = Instant::now();
        let active_nodes = self
            .sessions
            .keys()
            .filter(|node_id| !self.retired_sessions.contains(node_id));
        let waiting_nodes = active_nodes
            .clone()
            .filter(|node_id| **node_id != source_node)
            .filter(|node_id| {
                self.sessions
                    .get(node_id)
                    .is_some_and(|session| session.last_acked_cursor < cursor)
            })
            .copied()
            .collect();
        let waiting_prior_lease_nodes = active_nodes
            .filter(|node_id| {
                self.prior_lease_deadlines
                    .get(node_id)
                    .is_some_and(|until| *until > now)
            })
            .copied()
            .collect();
        (waiting_nodes, waiting_prior_lease_nodes)
    }

    fn complete_operation_visibility(&mut self, operation_ids: &[Vec<u8>]) {
        for operation_id in operation_ids {
            if let Some(operation) = self.operations.get_mut(operation_id) {
                operation.visibility_cursor = None;
            }
            if let Some(operation) = self.filesystem_namespace_operations.get_mut(operation_id) {
                operation.visibility_cursor = None;
            }
            if let Some(operation) = self.filesystem_version_operations.get_mut(operation_id) {
                operation.visibility_cursor = None;
            }
            if let Some(operation) = self.filesystem_attribute_operations.get_mut(operation_id) {
                operation.visibility_cursor = None;
            }
        }
    }

    fn apply_record(&mut self, sequence: u64, record: JournalRecord) {
        self.last_applied_index = self.last_applied_index.max(sequence);
        match record {
            JournalRecord::NodeSessionOpened {
                node_id,
                node_epoch,
                session_id,
                control_endpoint,
                next_session,
                supports_commit_sequence,
                commit_sequence_floor,
            } => {
                if let Some(previous) = self.sessions.get(&node_id) {
                    let until = previous
                        .last_heartbeat
                        .map_or(self.recovery_lease_until, |started| {
                            started + DEFAULT_NODE_LEASE_TTL
                        });
                    self.remember_prior_lease(node_id, until);
                }
                self.retired_sessions.remove(&node_id);
                self.node_epochs.insert(node_id, node_epoch);
                self.next_session = self.next_session.max(next_session);
                self.sessions.insert(
                    node_id,
                    NodeSession {
                        session_id,
                        node_epoch,
                        control_endpoint,
                        last_acked_cursor: self
                            .sessions
                            .get(&node_id)
                            .map_or(0, |session| session.last_acked_cursor),
                        last_heartbeat: Some(Instant::now()),
                        resources: None,
                        supports_commit_sequence,
                        commit_sequence_floor,
                        commit_sequences: HashMap::new(),
                    },
                );
                self.node_commit_sequence_floors
                    .insert(node_id, commit_sequence_floor);
            }
            JournalRecord::ReplicaAccepted {
                location,
                length,
                catalog_revision,
            } => {
                let replicas = self.replicas.entry(location.block_id.clone()).or_default();
                replicas.retain(|item| item.location.node_id != location.node_id);
                replicas.push(StoredReplica {
                    location,
                    catalog_revision,
                    length,
                });
            }
            JournalRecord::ReplicasReported {
                operation_id,
                accepted,
                rejected_block_ids,
                catalog_revision,
                desired_copies,
            } => {
                let mut receipts = Vec::new();
                for (location, length) in accepted {
                    self.desired_replica_counts
                        .entry(location.block_id.clone())
                        .and_modify(|current| *current = (*current).max(desired_copies.max(1)))
                        .or_insert(desired_copies.max(1));
                    let replicas = self.replicas.entry(location.block_id.clone()).or_default();
                    replicas.retain(|item| item.location.node_id != location.node_id);
                    replicas.push(StoredReplica {
                        location: location.clone(),
                        catalog_revision,
                        length,
                    });
                    receipts.push(pb::ReplicaReceipt {
                        block_id: location.block_id,
                        node_id: location.node_id,
                        node_epoch: location.node_epoch,
                        catalog_revision,
                        checksum: location.checksum,
                    });
                }
                if !operation_id.is_empty() {
                    self.replica_operations.insert(
                        operation_id,
                        pb::ReportReplicasResponse {
                            accepted: receipts,
                            rejected_block_ids,
                            catalog_watermark: catalog_revision,
                        },
                    );
                }
            }
            JournalRecord::VersionCommitted {
                key,
                layout,
                modified_time_unix_millis,
                new_replicas,
                operation_id,
                operation_digest,
                commit_sequence,
            } => {
                self.apply_version_commit(
                    sequence,
                    VersionCommitRecord {
                        key,
                        layout,
                        modified_time_unix_millis,
                        new_replicas,
                        operation_id,
                        operation_digest,
                    },
                    commit_sequence.as_ref(),
                );
            }
            JournalRecord::VersionsCommitted {
                commits,
                commit_sequence,
            } => {
                for commit in commits {
                    self.apply_version_commit(sequence, commit, commit_sequence.as_ref());
                }
            }
            JournalRecord::OperationRemembered {
                operation_id,
                operation_digest,
                result,
                commit_sequence,
            } => {
                if let Some(record) = commit_sequence.clone() {
                    self.remember_commit_sequence(record);
                }
                self.operations.insert(
                    operation_id,
                    StoredOperation {
                        digest: operation_digest,
                        result,
                        commit_sequence,
                        visibility_cursor: None,
                    },
                );
            }
            JournalRecord::NodeEventAcknowledged { node_id, cursor } => {
                let acknowledged_cursor =
                    self.cursor_before_outstanding_retirement(node_id, cursor);
                if let Some(session) = self.sessions.get_mut(&node_id) {
                    session.last_acked_cursor = session.last_acked_cursor.max(acknowledged_cursor);
                }
            }
            JournalRecord::BlockRetirementPrepared { record } => {
                self.apply_retirement_prepared(record);
            }
            JournalRecord::BlockRetirementAcknowledged {
                retirement_id,
                participant,
                ack_kind,
                stage_epoch,
            } => {
                self.apply_retirement_ack(
                    &retirement_id,
                    participant.clone(),
                    ack_kind,
                    stage_epoch,
                );
                self.advance_retirement_ack_cursor(&participant, &retirement_id, ack_kind);
            }
            JournalRecord::BlockRetirementFinalized {
                retirement_id,
                stage_epoch,
            } => {
                self.apply_retirement_finalized(&retirement_id, stage_epoch);
            }
            JournalRecord::BlockRetirementReleased {
                retirement_id,
                block_ids,
                stage_epoch,
            } => {
                self.apply_retirement_released(&retirement_id, &block_ids, stage_epoch);
            }
            JournalRecord::FilesystemInodeCreated { record } => {
                self.filesystem.apply_inode_created(
                    record.inode,
                    record.dentry,
                    record.next_inode,
                    record.grant_generation,
                );
            }
            JournalRecord::FilesystemVersionCommitted {
                record,
                commit_sequence,
            } => {
                let source_node_id = commit_sequence
                    .as_ref()
                    .map(|record| record.node_id)
                    .or_else(|| {
                        record
                            .object
                            .new_replicas
                            .first()
                            .map(|(location, _)| location.node_id)
                    })
                    .unwrap_or_default();
                let operation_id = record.object.operation_id.clone();
                let operation_digest = record.object.operation_digest.clone();
                let inode_id = record.inode.attributes.inode;
                let inode_revision = record.inode.revision;
                let committed_inode = record.inode.clone();
                let revoked_generation = record.revoked_grant_generation;
                let xattr_updates = record.xattr_updates.clone();
                self.apply_version_commit(sequence, record.object, commit_sequence.as_ref());
                self.filesystem
                    .apply_inode_version(record.inode, record.new_grant_generation);
                self.filesystem.apply_xattr_updates(xattr_updates);

                // FileContentBinding 是 inode cache 的授权对象，不能只依赖对象 Current
                // 失效。单独事件仍来自同一 journal 记录，因此没有第二个发布点。
                let cursor = self.event_high_watermark + 1;
                self.event_high_watermark = cursor;
                self.events.push(pb::NodeEvent {
                    event_id: cursor.to_be_bytes().to_vec(),
                    cursor,
                    event: Some(pb::node_event::Event::InvalidateFilesystemBinding(
                        pb::InvalidateFilesystemBindingEvent {
                            inode: inode_id,
                            through_generation: revoked_generation,
                            minimum_inode_revision: inode_revision,
                            source_node_id,
                            invalidated_dentries: Vec::new(),
                        },
                    )),
                });
                if let Some(operation) = self.operations.get_mut(&operation_id) {
                    operation.visibility_cursor = Some(cursor);
                }
                let response =
                    self.filesystem_commit_response_for_inode(&committed_inode, cursor, sequence);
                self.filesystem_version_operations.insert(
                    operation_id,
                    StoredFilesystemVersionOperation {
                        digest: operation_digest,
                        response,
                        visibility_cursor: Some(cursor),
                    },
                );
                self.pump_watchers();
            }
            JournalRecord::FilesystemAttributesUpdated {
                record,
                commit_sequence,
            } => {
                let source_node_id = commit_sequence
                    .as_ref()
                    .map(|record| record.node_id)
                    .unwrap_or_default();
                if let Some(record) = commit_sequence {
                    self.remember_commit_sequence(record);
                }
                let operation_id = record.operation_id;
                let operation_digest = record.operation_digest;
                let inode_id = record.inode.attributes.inode;
                let inode_revision = record.inode.revision;
                let committed_inode = record.inode.clone();
                let xattr_updates = record.xattr_updates.clone();
                self.filesystem
                    .apply_inode_version(record.inode, record.new_grant_generation);
                self.filesystem.apply_xattr_updates(xattr_updates);

                let cursor = self.event_high_watermark + 1;
                self.event_high_watermark = cursor;
                self.events.push(pb::NodeEvent {
                    event_id: cursor.to_be_bytes().to_vec(),
                    cursor,
                    event: Some(pb::node_event::Event::InvalidateFilesystemBinding(
                        pb::InvalidateFilesystemBindingEvent {
                            inode: inode_id,
                            through_generation: record.revoked_grant_generation,
                            minimum_inode_revision: inode_revision,
                            source_node_id,
                            invalidated_dentries: Vec::new(),
                        },
                    )),
                });
                let response = self.filesystem_attribute_response_for_inode(
                    &committed_inode,
                    cursor,
                    sequence,
                );
                self.filesystem_attribute_operations.insert(
                    operation_id,
                    StoredFilesystemAttributeOperation {
                        digest: operation_digest,
                        response,
                        visibility_cursor: Some(cursor),
                    },
                );
                self.pump_watchers();
            }
            JournalRecord::FilesystemXattrUpdated {
                record,
                commit_sequence,
            } => {
                let source_node_id = commit_sequence
                    .as_ref()
                    .map(|record| record.node_id)
                    .unwrap_or_default();
                if let Some(record) = commit_sequence {
                    self.remember_commit_sequence(record);
                }
                let operation_id = record.operation_id;
                let operation_digest = record.operation_digest;
                let inode_id = record.inode.attributes.inode;
                let inode_revision = record.inode.revision;
                let committed_inode = record.inode.clone();
                self.filesystem
                    .apply_inode_version(record.inode, record.new_grant_generation);
                self.filesystem.apply_xattr_updates([record.update]);

                let cursor = self.event_high_watermark + 1;
                self.event_high_watermark = cursor;
                self.events.push(pb::NodeEvent {
                    event_id: cursor.to_be_bytes().to_vec(),
                    cursor,
                    event: Some(pb::node_event::Event::InvalidateFilesystemBinding(
                        pb::InvalidateFilesystemBindingEvent {
                            inode: inode_id,
                            through_generation: record.revoked_grant_generation,
                            minimum_inode_revision: inode_revision,
                            source_node_id,
                            invalidated_dentries: Vec::new(),
                        },
                    )),
                });
                let response = self.filesystem_attribute_response_for_inode(
                    &committed_inode,
                    cursor,
                    sequence,
                );
                self.filesystem_attribute_operations.insert(
                    operation_id,
                    StoredFilesystemAttributeOperation {
                        digest: operation_digest,
                        response,
                        visibility_cursor: Some(cursor),
                    },
                );
                self.pump_watchers();
            }
            JournalRecord::FilesystemSymlinkCreated {
                record,
                commit_sequence,
            } => {
                let source_node_id = commit_sequence
                    .as_ref()
                    .map(|record| record.node_id)
                    .or_else(|| {
                        record
                            .version
                            .new_replicas
                            .first()
                            .map(|(location, _)| location.node_id)
                    })
                    .unwrap_or_default();
                if let Some(record) = commit_sequence {
                    self.remember_commit_sequence(record);
                }

                // symlink target 复用 DataCore exact ObjectVersion，但它与
                // inode/dentry/content binding 共用同一条 journal 记录。这里仅更新
                // 对象版本目录，不写通用 KV operation result，避免形成第二个提交权威。
                for (location, length) in record.version.new_replicas {
                    self.desired_replica_counts
                        .entry(location.block_id.clone())
                        .or_insert(1);
                    let replicas = self.replicas.entry(location.block_id.clone()).or_default();
                    replicas.retain(|item| item.location.node_id != location.node_id);
                    replicas.push(StoredReplica {
                        location,
                        catalog_revision: sequence,
                        length,
                    });
                }
                self.versions
                    .entry(record.version.key.clone())
                    .or_default()
                    .insert(record.version.layout.version, record.version.layout.clone());
                self.version_modified_times.insert(
                    (record.version.key.clone(), record.version.layout.version),
                    record.version.modified_time_unix_millis,
                );
                self.live_keys.insert(record.version.key);

                let invalidated_dentries = filesystem_dentry_invalidations(
                    &record.namespace.upsert_dentries,
                    &record.namespace.remove_dentries,
                );
                let xattr_updates = record.namespace.xattr_updates.clone();
                self.filesystem.apply_namespace_mutation(
                    record.namespace.upsert_inodes,
                    record.namespace.upsert_dentries,
                    record.namespace.remove_dentries,
                    record.namespace.result.changed_directories.clone(),
                    record.namespace.result.changed_inodes.clone(),
                    record.namespace.next_inode,
                );
                self.filesystem.apply_xattr_updates(xattr_updates);
                let mut emitted_event = false;
                for directory in &record.namespace.result.changed_directories {
                    let cursor = self.event_high_watermark + 1;
                    self.event_high_watermark = cursor;
                    emitted_event = true;
                    self.events.push(pb::NodeEvent {
                        event_id: cursor.to_be_bytes().to_vec(),
                        cursor,
                        event: Some(pb::node_event::Event::InvalidateFilesystemBinding(
                            pb::InvalidateFilesystemBindingEvent {
                                inode: directory.inode,
                                through_generation: directory.grant_generation.saturating_sub(1),
                                minimum_inode_revision: directory.revision,
                                source_node_id,
                                invalidated_dentries: invalidated_dentries
                                    .get(&directory.inode)
                                    .cloned()
                                    .unwrap_or_default(),
                            },
                        )),
                    });
                }
                for inode in &record.namespace.result.changed_inodes {
                    let cursor = self.event_high_watermark + 1;
                    self.event_high_watermark = cursor;
                    emitted_event = true;
                    self.events.push(pb::NodeEvent {
                        event_id: cursor.to_be_bytes().to_vec(),
                        cursor,
                        event: Some(pb::node_event::Event::InvalidateFilesystemBinding(
                            pb::InvalidateFilesystemBindingEvent {
                                inode: inode.inode,
                                through_generation: inode.grant_generation.saturating_sub(1),
                                minimum_inode_revision: inode.revision,
                                source_node_id,
                                invalidated_dentries: Vec::new(),
                            },
                        )),
                    });
                }
                self.filesystem_namespace_operations.insert(
                    record.namespace.operation_id,
                    StoredFilesystemNamespaceOperation {
                        digest: record.namespace.operation_digest,
                        visibility_cursor: emitted_event
                            .then_some(record.namespace.result.invalidation_cursor),
                        result: record.namespace.result,
                    },
                );
                if emitted_event {
                    self.pump_watchers();
                }
            }
            JournalRecord::FilesystemNamespaceMutated {
                record,
                commit_sequence,
            } => {
                let source_node_id = commit_sequence
                    .as_ref()
                    .map(|record| record.node_id)
                    .unwrap_or_default();
                if let Some(record) = commit_sequence {
                    self.remember_commit_sequence(record);
                }
                let invalidated_dentries = filesystem_dentry_invalidations(
                    &record.upsert_dentries,
                    &record.remove_dentries,
                );
                let xattr_updates = record.xattr_updates.clone();
                self.filesystem.apply_namespace_mutation(
                    record.upsert_inodes,
                    record.upsert_dentries,
                    record.remove_dentries,
                    record.result.changed_directories.clone(),
                    record.result.changed_inodes.clone(),
                    record.next_inode,
                );
                self.filesystem.apply_xattr_updates(xattr_updates);
                let mut emitted_event = false;
                for directory in &record.result.changed_directories {
                    let cursor = self.event_high_watermark + 1;
                    self.event_high_watermark = cursor;
                    emitted_event = true;
                    self.events.push(pb::NodeEvent {
                        event_id: cursor.to_be_bytes().to_vec(),
                        cursor,
                        event: Some(pb::node_event::Event::InvalidateFilesystemBinding(
                            pb::InvalidateFilesystemBindingEvent {
                                inode: directory.inode,
                                through_generation: directory.grant_generation.saturating_sub(1),
                                minimum_inode_revision: directory.revision,
                                source_node_id,
                                invalidated_dentries: invalidated_dentries
                                    .get(&directory.inode)
                                    .cloned()
                                    .unwrap_or_default(),
                            },
                        )),
                    });
                }
                for inode in &record.result.changed_inodes {
                    let cursor = self.event_high_watermark + 1;
                    self.event_high_watermark = cursor;
                    emitted_event = true;
                    self.events.push(pb::NodeEvent {
                        event_id: cursor.to_be_bytes().to_vec(),
                        cursor,
                        event: Some(pb::node_event::Event::InvalidateFilesystemBinding(
                            pb::InvalidateFilesystemBindingEvent {
                                inode: inode.inode,
                                through_generation: inode.grant_generation.saturating_sub(1),
                                minimum_inode_revision: inode.revision,
                                source_node_id,
                                invalidated_dentries: Vec::new(),
                            },
                        )),
                    });
                }
                self.filesystem_namespace_operations.insert(
                    record.operation_id,
                    StoredFilesystemNamespaceOperation {
                        digest: record.operation_digest,
                        visibility_cursor: emitted_event
                            .then_some(record.result.invalidation_cursor),
                        result: record.result,
                    },
                );
                if emitted_event {
                    self.pump_watchers();
                }
            }
            JournalRecord::FilesystemOrphanReaped { record } => {
                if let Some(tombstone) = record.object_tombstone {
                    self.apply_version_commit(sequence, tombstone, None);
                }
                self.filesystem.apply_orphan_reaped(record.inode);
                self.filesystem_inode_references
                    .retain(|(_, _, inode), _| *inode != record.inode);
            }
        }
    }

    fn apply_version_commit(
        &mut self,
        sequence: u64,
        commit: VersionCommitRecord,
        commit_sequence: Option<&CommitSequenceRecord>,
    ) {
        // commit sequence 记录真正发起提交的 Node，应优先作为来源；旧 WAL 没有
        // sequence 时才从新副本推断。两者都缺失则保留 0 并继续广播，以兼容
        // 历史记录并优先保证失效正确性。
        let source_node_id = commit_sequence
            .map(|record| record.node_id)
            .or_else(|| {
                commit
                    .new_replicas
                    .first()
                    .map(|(location, _)| location.node_id)
            })
            .unwrap_or_default();
        for (location, length) in commit.new_replicas {
            self.desired_replica_counts
                .entry(location.block_id.clone())
                .or_insert(1);
            let replicas = self.replicas.entry(location.block_id.clone()).or_default();
            replicas.retain(|item| item.location.node_id != location.node_id);
            replicas.push(StoredReplica {
                location,
                catalog_revision: sequence,
                length,
            });
        }
        let old_version = self
            .versions
            .get(&commit.key)
            .and_then(|versions| versions.last_key_value().map(|(_, item)| item.version))
            .unwrap_or(0);
        self.versions
            .entry(commit.key.clone())
            .or_default()
            .insert(commit.layout.version, commit.layout.clone());
        self.version_modified_times.insert(
            (commit.key.clone(), commit.layout.version),
            commit.modified_time_unix_millis,
        );
        if commit.layout.kind == pb::VersionKind::Value as i32 {
            self.live_keys.insert(commit.key.clone());
        } else if commit.layout.kind == pb::VersionKind::Tombstone as i32 {
            self.live_keys.remove(&commit.key);
        }
        // cursor 是 Meta watch/ACK 的全局时序号。即使首次发布不需要失效事件，
        // 也保留一个 cursor gap，避免旧 Journal 回放时历史 ACK 误确认后续事件。
        let cursor = self.event_high_watermark + 1;
        self.event_high_watermark = cursor;
        let visibility_cursor = if old_version == 0 {
            // 从未存在过的 key 没有旧 Current 可撤销。当前系统也没有负缓存；
            // 读 miss 不会被缓存成“未来仍不存在”。因此首次发布可以直接完成，
            // 不需要把未读过该 key 的 Node 拉进前台 ACK 屏障。上面的 cursor gap
            // 只用于兼容恢复，不会投递给 watcher。
            None
        } else {
            self.events.push(pb::NodeEvent {
                event_id: cursor.to_be_bytes().to_vec(),
                cursor,
                event: Some(pb::node_event::Event::InvalidateCurrent(
                    pb::InvalidateCurrentEvent {
                        key: Some(pb::Key { value: commit.key }),
                        old_version,
                        transition_id: sequence.to_be_bytes().to_vec(),
                        lease_epoch: 0,
                        revision: sequence,
                        minimum_version: commit.layout.version,
                        source_node_id,
                    },
                )),
            });
            Some(cursor)
        };
        let commit_sequence = commit_sequence.cloned();
        if let Some(record) = commit_sequence.clone() {
            self.remember_commit_sequence(record);
        }
        self.operations.insert(
            commit.operation_id,
            StoredOperation {
                digest: commit.operation_digest,
                result: pb::CommitVersionResponse {
                    version: commit.layout.version,
                    revision: sequence,
                    commit_index: sequence,
                    changed: true,
                },
                commit_sequence,
                visibility_cursor,
            },
        );
        if visibility_cursor.is_some() {
            self.pump_watchers();
        }
    }

    fn maybe_checkpoint(&mut self, sequence: u64) -> Result<(), MetaRuntimeError> {
        if !self
            .checkpoint_policy
            .should_checkpoint(sequence, self.last_checkpoint_index)
        {
            return Ok(());
        }
        let mut checkpoint_metric = self.metrics.begin_checkpoint();
        self.enforce_retention();
        let checkpoint_index = self.last_applied_index;
        let snapshot = self.snapshot();
        self.journal
            .save_snapshot(snapshot)
            .map_err(map_journal_error)?;
        self.journal
            .truncate_prefix(checkpoint_index)
            .map_err(map_journal_error)?;
        self.last_checkpoint_index = checkpoint_index;
        self.retention_boundary_dirty = false;
        checkpoint_metric.success();
        Ok(())
    }

    fn checkpoint_retention_boundary(&mut self) -> Result<(), MetaRuntimeError> {
        if !self.retention_boundary_dirty {
            return Ok(());
        }
        let checkpoint_index = self.last_applied_index;
        let snapshot = self.snapshot();
        self.journal
            .save_snapshot(snapshot)
            .map_err(map_journal_error)?;
        self.journal
            .truncate_prefix(checkpoint_index)
            .map_err(map_journal_error)?;
        self.last_checkpoint_index = checkpoint_index;
        self.retention_boundary_dirty = false;
        Ok(())
    }

    /// Appends one authoritative state transition and records the storage
    /// boundary exactly once. Callers still apply the returned sequence only
    /// after this method succeeds, preserving journal-before-apply semantics.
    fn append_record(&mut self, record: JournalRecord) -> Result<u64, MetaRuntimeError> {
        let record_type = record.metric_kind();
        let span = dms_tracing::tracing::info_span!(
            "dms.meta.journal_append",
            otel.kind = "internal",
            record_type = record_type.label(),
            result = dms_tracing::tracing::field::Empty,
        );
        // append is synchronous, so an entered guard is safe here; unlike an
        // async guard it can never be held across an `.await` suspension.
        let _entered = span.enter();
        let mut metric = self.metrics.begin_journal_append(record_type);
        let result = self
            .journal
            .append(record)
            .map_err(map_journal_append_error);
        if result.is_ok() {
            metric.success();
        }
        span.record("result", if result.is_ok() { "ok" } else { "error" });
        result
    }

    fn refresh_metrics(&self) {
        let now = Instant::now();
        let live_sessions = self
            .sessions
            .values()
            .filter(|session| {
                session.last_heartbeat.is_some_and(|heartbeat| {
                    now.duration_since(heartbeat) <= DEFAULT_NODE_LEASE_TTL
                })
            })
            .count();
        let (mut healthy, mut under_replicated, mut unavailable) = (0_usize, 0_usize, 0_usize);
        for (block_id, desired) in &self.desired_replica_counts {
            // Replica catalog 保留历史事实用于恢复/修复，但健康指标只统计
            // 当前 lease 内可达的 Node incarnation，避免失联副本误报健康。
            let available = self.replicas.get(block_id).map_or(0, |replicas| {
                replicas
                    .iter()
                    .filter(|replica| self.is_live_replica(replica))
                    .count()
            });
            if available == 0 {
                unavailable += 1;
            } else if available < *desired as usize {
                under_replicated += 1;
            } else {
                healthy += 1;
            }
        }
        let oldest_repair_age_seconds = self
            .pending_repairs
            .values()
            .map(|repair| repair.issued_at.elapsed().as_secs_f64())
            .fold(0.0, f64::max);
        // event_high_watermark 允许存在 cursor 空洞：例如首次发布新 key
        // 会消耗 cursor 保持旧 Journal ACK 兼容，但不会生成 NodeEvent。
        // lag 只统计该 Node 真正需要处理、且位于其 ACK 游标之后的保留事件。
        // 例如写入 Node 不需要处理自己产生的 Current 失效，不能把这些事件
        // 误报成 backlog；它以后 ACK 更大的目标事件时仍会自然跨过这些游标。
        let max_lag = self
            .sessions
            .iter()
            .filter(|(node_id, _)| !self.retired_sessions.contains(node_id))
            .map(|(node_id, session)| {
                self.events
                    .iter()
                    .filter(|event| {
                        event.cursor > session.last_acked_cursor
                            && event_targets_node(event, *node_id)
                    })
                    .count() as u64
            })
            .max()
            .unwrap_or(0);
        self.metrics.set_state(MetaStateMetricsSnapshot {
            keys: self.versions.len(),
            versions: self.versions.values().map(BTreeMap::len).sum(),
            replicas: self.live_replica_location_count(),
            operations: self.operations.len(),
            sessions: self.sessions.len(),
            events: self.events.len(),
            watch_streams: self
                .watchers
                .values()
                .filter(|watcher| !watcher.sender.is_closed())
                .count(),
            watch_lag_events: max_lag,
            live_sessions,
            expired_sessions: self.sessions.len().saturating_sub(live_sessions),
            healthy_blocks: healthy,
            under_replicated_blocks: under_replicated,
            unavailable_blocks: unavailable,
            repairs_pending: self.pending_repairs.len(),
            oldest_repair_age_seconds,
        });
    }

    fn retire_expired_sessions(&mut self) -> Result<(), MetaRuntimeError> {
        let now = Instant::now();
        let prior_expired = self
            .prior_lease_deadlines
            .iter()
            .filter_map(|(node, until)| (*until <= now).then_some(*node))
            .collect::<Vec<_>>();
        for node in prior_expired {
            self.prior_lease_deadlines.remove(&node);
            self.advance_pending_commits_for_prior_lease(node);
        }
        let expired = self
            .sessions
            .iter()
            .filter_map(|(node, session)| {
                let until = session
                    .last_heartbeat
                    .map_or(self.recovery_lease_until, |started| {
                        started + DEFAULT_NODE_LEASE_TTL
                    });
                (!self.retired_sessions.contains(node) && until <= now).then_some(*node)
            })
            .collect::<Vec<_>>();
        for node_id in expired {
            // Client 的租约从不超过 Node 的 Meta 租约，因此到此时已无有效旧 Current。
            // 但 GC retirement duty 只能由专用 ACK 完成；过期/断连不能被当作
            // Drain/Release 证明，因此普通水位必须停在最早欠账之前。
            let cursor =
                self.cursor_before_outstanding_retirement(node_id, self.event_high_watermark);
            let record = JournalRecord::NodeEventAcknowledged { node_id, cursor };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
            self.retired_sessions.insert(node_id);
            self.watchers.remove(&node_id);
            if let Some(session) = self.sessions.get(&node_id) {
                self.filesystem_locks
                    .release_node_epoch(node_id, session.node_epoch);
            }
            self.advance_pending_commits_for_ack(node_id, cursor);
        }
        self.retain_events();
        Ok(())
    }

    /// 持久回收已经脱离 namespace 且不再受 Node 引用租约保护的 inode。
    ///
    /// 三个条件必须同时满足：`link_count == 0`、所有 inode 引用租约已过期、Meta
    /// 恢复保护窗口已结束。删除内容对象时生成普通 DataCore tombstone，后续版本保留与
    /// BlockRetirement 继续走既有链路；这里绝不直接释放 Block。
    fn reap_filesystem_orphans(&mut self) -> Result<usize, MetaRuntimeError> {
        let now = Instant::now();
        self.filesystem_inode_references
            .retain(|_, (_, deadline)| *deadline > now);
        if now < self.recovery_lease_until {
            return Ok(0);
        }

        let candidates = self
            .filesystem
            .orphan_candidates()
            .into_iter()
            .filter(|inode| {
                !self
                    .filesystem_inode_references
                    .keys()
                    .any(|(_, _, referenced_inode)| *referenced_inode == inode.attributes.inode)
            })
            .take(MAX_FILESYSTEM_ORPHANS_PER_TICK)
            .collect::<Vec<_>>();
        let mut reaped = 0;
        let mut last_sequence = None;
        for inode in candidates {
            let object_tombstone = inode.content.as_ref().map(|binding| {
                let current_version = self
                    .versions
                    .get(&binding.object_key)
                    .and_then(|versions| {
                        versions.last_key_value().map(|(_, layout)| layout.version)
                    })
                    .unwrap_or(binding.exact_version);
                let next_version = current_version.saturating_add(1);
                let mut operation_id = b"filesystem-orphan-reap\0".to_vec();
                operation_id.extend_from_slice(&inode.attributes.inode.to_be_bytes());
                operation_id.extend_from_slice(&next_version.to_be_bytes());
                VersionCommitRecord {
                    key: binding.object_key.clone(),
                    layout: pb::VersionLayout {
                        version: next_version,
                        logical_length: 0,
                        extents: Vec::new(),
                        digest: Vec::new(),
                        kind: pb::VersionKind::Tombstone as i32,
                    },
                    modified_time_unix_millis: current_time_unix_millis(),
                    new_replicas: Vec::new(),
                    operation_digest: digest(&operation_id),
                    operation_id,
                }
            });
            let record = JournalRecord::FilesystemOrphanReaped {
                record: FilesystemOrphanReapedRecord {
                    inode: inode.attributes.inode,
                    object_tombstone,
                },
            };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
            dms_logging::info!(
                "reaped durable filesystem orphan";
                "event" => "meta.filesystem.orphan.reaped",
                "inode" => inode.attributes.inode,
                "journal_sequence" => sequence,
                "content_tombstoned" => inode.content.is_some(),
            );
            last_sequence = Some(sequence);
            reaped += 1;
        }
        if let Some(sequence) = last_sequence {
            self.maybe_checkpoint(sequence)?;
        }
        Ok(reaped)
    }

    fn enforce_retention(&mut self) {
        let pruned_versions = self.retain_versions();
        let advanced_commit_sequence_floor = self.retain_operations();
        if pruned_versions || advanced_commit_sequence_floor {
            self.retention_boundary_dirty = true;
        }
        self.retain_events();
        if let Err(error) = self.checkpoint_retention_boundary() {
            dms_logging::warn!(
                "retention checkpoint failed; physical block retirement paused";
                "error" => format!("{error:?}")
            );
            return;
        }
        self.retain_replicas_referenced_by_versions();
        self.advance_retirements();
        self.retain_retired_block_fences();
    }

    fn retain_versions(&mut self) -> bool {
        let keep = self.retention_policy.keep_versions_per_key.max(1);
        let current_index = self.last_applied_index;
        let tombstone_window = self
            .retention_policy
            .operation_result_retention_records
            .max(1);
        let mut version_floor = self.version_floor;
        let mut purge_keys = Vec::new();
        let mut changed = false;
        for (key, versions) in self.versions.iter_mut() {
            if versions
                .last_key_value()
                .is_some_and(|(_, layout)| layout.kind == pb::VersionKind::Tombstone as i32)
            {
                // 删除后的 key 只保留最后 tombstone 作为逻辑 Current/version fence。
                // 旧 VALUE 版本的 Exact selector 受保留策略约束，不是永久 pin；
                // 已经开始的读/显式 View 由 Node drain ACK 保护。
                while versions.len() > 1 {
                    let Some(oldest) = versions.first_key_value().map(|(version, _)| *version)
                    else {
                        break;
                    };
                    versions.remove(&oldest);
                    version_floor = version_floor.max(oldest);
                    changed = true;
                }
                if let Some((&tombstone_version, _)) = versions.last_key_value()
                    && current_index.saturating_sub(tombstone_version) > tombstone_window
                {
                    // tombstone 也只是有限历史 fence。全局 version_floor 单调保存
                    // 已裁掉的最高版本，key churn 下 Meta 不需要永久每 key 墓碑。
                    version_floor = version_floor.max(tombstone_version);
                    purge_keys.push(key.clone());
                    changed = true;
                }
                continue;
            }
            while versions.len() > keep {
                let Some(oldest) = versions.first_key_value().map(|(version, _)| *version) else {
                    break;
                };
                versions.remove(&oldest);
                version_floor = version_floor.max(oldest);
                changed = true;
            }
        }
        self.version_floor = version_floor;
        for key in purge_keys {
            self.versions.remove(&key);
            self.live_keys.remove(&key);
        }
        let retained = self
            .versions
            .iter()
            .flat_map(|(key, versions)| {
                versions
                    .keys()
                    .map(|version| (key.clone(), *version))
                    .collect::<Vec<_>>()
            })
            .collect::<HashSet<_>>();
        self.version_modified_times
            .retain(|identity, _| retained.contains(identity));
        changed
    }

    fn retain_operations(&mut self) -> bool {
        let current_index = self.last_applied_index;
        let operation_window = self.retention_policy.operation_result_retention_records;
        let mut expired_commit_sequences = Vec::new();
        self.operations.retain(|_, operation| {
            let retain = operation.visibility_cursor.is_some()
                || current_index.saturating_sub(operation.result.commit_index) <= operation_window;
            if !retain && let Some(record) = &operation.commit_sequence {
                expired_commit_sequences.push(record.clone());
            }
            retain
        });
        let advanced_commit_sequence_floor = !expired_commit_sequences.is_empty();
        for record in expired_commit_sequences {
            self.advance_commit_sequence_floor(&record);
        }

        let filesystem_operations_before = self.filesystem_version_operations.len();
        self.filesystem_version_operations.retain(|_, operation| {
            operation.visibility_cursor.is_some()
                || current_index.saturating_sub(operation.response.commit_index) <= operation_window
        });
        let pruned_filesystem_operations =
            self.filesystem_version_operations.len() != filesystem_operations_before;

        let namespace_operations_before = self.filesystem_namespace_operations.len();
        self.filesystem_namespace_operations.retain(|_, operation| {
            operation.visibility_cursor.is_some()
                || current_index.saturating_sub(operation.result.commit_index) <= operation_window
        });
        let pruned_namespace_operations =
            self.filesystem_namespace_operations.len() != namespace_operations_before;

        let attribute_operations_before = self.filesystem_attribute_operations.len();
        self.filesystem_attribute_operations.retain(|_, operation| {
            operation.visibility_cursor.is_some()
                || current_index.saturating_sub(operation.response.commit_index) <= operation_window
        });
        let pruned_attribute_operations =
            self.filesystem_attribute_operations.len() != attribute_operations_before;

        let replica_window = self.retention_policy.replica_operation_retention_records;
        self.replica_operations.retain(|_, result| {
            current_index.saturating_sub(result.catalog_watermark) <= replica_window
        });
        advanced_commit_sequence_floor
            || pruned_filesystem_operations
            || pruned_namespace_operations
            || pruned_attribute_operations
    }

    fn retain_events(&mut self) {
        if !self
            .retention_policy
            .event_retention_requires_all_session_acks
        {
            return;
        }
        let sessions = &self.sessions;
        let retired_sessions = &self.retired_sessions;
        let pending_retirements = self.pending_retirements.clone();
        self.events.retain(|event| {
            sessions.iter().any(|(node_id, session)| {
                !retired_sessions.contains(node_id)
                    && event_targets_node(event, *node_id)
                    && event.cursor > session.last_acked_cursor
            }) || retirement_event_outstanding_in(&pending_retirements, event)
        });
    }

    fn retain_replicas_referenced_by_versions(&mut self) {
        if !self.retention_policy.drop_unreferenced_replicas {
            return;
        }
        if self.sessions.iter().any(|(node_id, session)| {
            !self.retired_sessions.contains(node_id) && !session.supports_commit_sequence
        }) {
            dms_logging::warn!(
                "block retirement paused until all live sessions support commit sequence"
            );
            return;
        }
        if self.pending_retirements.len() >= MAX_PENDING_BLOCK_RETIREMENTS {
            dms_logging::warn!(
                "block retirement backlog full";
                "pending" => self.pending_retirements.len(),
                "limit" => MAX_PENDING_BLOCK_RETIREMENTS
            );
            return;
        }
        let referenced_blocks = self
            .versions
            .values()
            .flat_map(|versions| versions.values())
            .flat_map(|layout| layout.extents.iter())
            .map(|extent| extent.block_id.clone())
            .collect::<HashSet<_>>();
        let already_pending = self
            .pending_retirements
            .values()
            .flat_map(|retirement| retirement.record.block_ids.iter().cloned())
            .collect::<HashSet<_>>();
        let candidates = self
            .replicas
            .keys()
            .filter(|block_id| {
                !referenced_blocks.contains(*block_id)
                    && !already_pending.contains(*block_id)
                    && !self.retired_block_fences.contains_key(*block_id)
            })
            .take(MAX_RETIREMENT_BLOCKS_PER_BATCH)
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return;
        }
        if let Err(error) = self.prepare_block_retirement(candidates) {
            dms_logging::warn!("block retirement prepare failed"; "error" => format!("{error:?}"));
        }
    }

    fn retain_retired_block_fences(&mut self) {
        if self.retired_block_fences.len() <= MAX_RETIRED_BLOCK_FENCES {
            return;
        }
        let mut fences = self
            .retired_block_fences
            .iter()
            .map(|(block_id, fence_version)| (block_id.clone(), *fence_version))
            .collect::<Vec<_>>();
        fences.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.as_slice().cmp(right.0.as_slice()))
        });
        let retained = fences
            .into_iter()
            .take(MAX_RETIRED_BLOCK_FENCES)
            .map(|(block_id, _)| block_id)
            .collect::<HashSet<_>>();
        // 退休 fence 是迟到 report/commit 的有限窗口，不是无界 retiredBlockSet。
        // 超出窗口的更旧内容 ID 不再占用内存；操作幂等仍由 operation retention 控制。
        self.retired_block_fences
            .retain(|block_id, _| retained.contains(block_id));
    }

    fn prepare_block_retirement(
        &mut self,
        block_ids: Vec<Vec<u8>>,
    ) -> Result<(), MetaRuntimeError> {
        let participants = self.retirement_participants(&block_ids);
        let stage_epoch = self.journal.last_index() + 1;
        let record = BlockRetirementRecord {
            retirement_id: retirement_id(stage_epoch, &block_ids),
            block_ids,
            participants,
            prepare_stage_epoch: stage_epoch,
            final_stage_epoch: 0,
            fence_version: stage_epoch,
        };
        let journal = JournalRecord::BlockRetirementPrepared { record };
        let sequence = self.append_record(journal.clone())?;
        self.apply_record(sequence, journal);
        Ok(())
    }

    fn retirement_participants(&self, block_ids: &[Vec<u8>]) -> Vec<BlockRetirementParticipant> {
        let mut participants = self
            .sessions
            .iter()
            // 已注册 session 即使租约过期/retired，也可能在过期前或过期后
            // Resolve 过旧 layout 并持有 Node 本地 read scope。TTL 不能证明
            // drain；阶段 1 选择保守等待专用 GC ACK。
            .map(|(node_id, session)| (*node_id, session.node_epoch))
            .collect::<BTreeSet<_>>();
        for block_id in block_ids {
            if let Some(replicas) = self.replicas.get(block_id) {
                for replica in replicas {
                    participants.insert((replica.location.node_id, replica.location.node_epoch));
                }
            }
        }
        participants
            .into_iter()
            .map(|(node_id, node_epoch)| BlockRetirementParticipant {
                node_id,
                node_epoch,
            })
            .collect()
    }

    fn apply_retirement_prepared(&mut self, record: BlockRetirementRecord) {
        self.emit_retirement_events(
            &record,
            pb::BlockRetirementPhase::Prepare,
            record.prepare_stage_epoch,
        );
        self.pending_retirements.insert(
            record.retirement_id.clone(),
            PendingRetirement {
                record,
                prepared: HashSet::new(),
                released: HashSet::new(),
                final_sent: false,
            },
        );
        self.pump_watchers();
    }

    fn apply_retirement_ack(
        &mut self,
        retirement_id: &[u8],
        participant: BlockRetirementParticipant,
        ack_kind: pb::BlockRetirementAckKind,
        stage_epoch: u64,
    ) {
        let Some(retirement) = self.pending_retirements.get_mut(retirement_id) else {
            return;
        };
        if !retirement.record.participants.contains(&participant) {
            return;
        }
        match ack_kind {
            pb::BlockRetirementAckKind::Prepared
                if stage_epoch == retirement.record.prepare_stage_epoch =>
            {
                retirement
                    .prepared
                    .insert((participant.node_id, participant.node_epoch));
            }
            pb::BlockRetirementAckKind::Released
                if retirement.final_sent && stage_epoch == retirement.record.final_stage_epoch =>
            {
                retirement
                    .released
                    .insert((participant.node_id, participant.node_epoch));
            }
            _ => {}
        }
    }

    fn apply_retirement_finalized(&mut self, retirement_id: &[u8], stage_epoch: u64) {
        let Some(mut record) = self
            .pending_retirements
            .get(retirement_id)
            .map(|pending| pending.record.clone())
        else {
            return;
        };
        record.final_stage_epoch = stage_epoch;
        self.emit_retirement_events(&record, pb::BlockRetirementPhase::Final, stage_epoch);
        if let Some(retirement) = self.pending_retirements.get_mut(retirement_id) {
            retirement.record.final_stage_epoch = stage_epoch;
            retirement.final_sent = true;
        }
        self.pump_watchers();
    }

    fn apply_retirement_released(
        &mut self,
        retirement_id: &[u8],
        block_ids: &[Vec<u8>],
        stage_epoch: u64,
    ) {
        for block_id in block_ids {
            self.replicas.remove(block_id);
            self.desired_replica_counts.remove(block_id);
            self.pending_repairs.remove(block_id);
            self.retired_block_fences
                .entry(block_id.clone())
                .or_insert(stage_epoch);
        }
        self.pending_retirements.remove(retirement_id);
        self.retain_retired_block_fences();
    }

    fn emit_retirement_events(
        &mut self,
        record: &BlockRetirementRecord,
        phase: pb::BlockRetirementPhase,
        stage_epoch: u64,
    ) {
        for participant in &record.participants {
            let cursor = self.event_high_watermark + 1;
            self.event_high_watermark = cursor;
            self.events.push(pb::NodeEvent {
                event_id: event_id_with_suffix(stage_epoch, participant.node_id),
                cursor,
                event: Some(pb::node_event::Event::EvictReplica(pb::EvictReplicaEvent {
                    block_id: record.block_ids.first().cloned().unwrap_or_default(),
                    block_ids: record.block_ids.clone(),
                    retirement_id: record.retirement_id.clone(),
                    phase: phase as i32,
                    participant_node_id: participant.node_id,
                    participant_node_epoch: participant.node_epoch,
                    stage_epoch,
                    fence_version: record.fence_version,
                })),
            });
        }
    }

    fn acknowledge_block_retirement(
        &mut self,
        request: pb::AcknowledgeBlockRetirementRequest,
    ) -> Result<pb::AcknowledgeBlockRetirementResponse, MetaRuntimeError> {
        let session = request
            .session
            .as_ref()
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("missing node session".to_string()))?;
        self.verify_session(session)?;
        let ack_kind = pb::BlockRetirementAckKind::try_from(request.ack_kind).map_err(|_| {
            MetaRuntimeError::InvalidArgument("invalid retirement ack kind".to_string())
        })?;
        if ack_kind == pb::BlockRetirementAckKind::Unspecified || request.retirement_id.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "retirement ack requires id and kind".to_string(),
            ));
        }
        let Some(retirement) = self.pending_retirements.get(&request.retirement_id) else {
            if !request.block_ids.is_empty()
                && request
                    .block_ids
                    .iter()
                    .all(|block_id| self.retired_block_fences.contains_key(block_id))
            {
                return Ok(pb::AcknowledgeBlockRetirementResponse { accepted: true });
            }
            return Err(MetaRuntimeError::NotFound);
        };
        if request.block_ids != retirement.record.block_ids {
            return Err(MetaRuntimeError::InvalidArgument(
                "retirement ack block set mismatch".to_string(),
            ));
        }
        let participant = BlockRetirementParticipant {
            node_id: session.node_id,
            node_epoch: session.node_epoch,
        };
        if !retirement.record.participants.contains(&participant) {
            return Err(MetaRuntimeError::Conflict {
                expected: None,
                actual: session.node_epoch,
            });
        }
        let expected_stage = match ack_kind {
            pb::BlockRetirementAckKind::Prepared => retirement.record.prepare_stage_epoch,
            pb::BlockRetirementAckKind::Released => retirement.record.final_stage_epoch,
            pb::BlockRetirementAckKind::Unspecified => unreachable!(),
        };
        if request.stage_epoch != expected_stage || expected_stage == 0 {
            return Err(MetaRuntimeError::Conflict {
                expected: Some(expected_stage),
                actual: request.stage_epoch,
            });
        }
        let record = JournalRecord::BlockRetirementAcknowledged {
            retirement_id: request.retirement_id.clone(),
            participant,
            ack_kind,
            stage_epoch: request.stage_epoch,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.advance_retirement_after_ack(&request.retirement_id)?;
        let retirement_released = !self
            .pending_retirements
            .contains_key(&request.retirement_id);
        let checkpoint = self.maybe_checkpoint(self.last_applied_index);
        if retirement_released {
            // Release ACK 是对外的“Meta 已忘记 replica facts”边界；
            // 同步刷新，避免真实 F4 policy 立即采样到上一轮陈旧 gauge。
            self.refresh_metrics();
        }
        checkpoint?;
        Ok(pb::AcknowledgeBlockRetirementResponse { accepted: true })
    }

    fn advance_retirement_after_ack(
        &mut self,
        retirement_id: &[u8],
    ) -> Result<(), MetaRuntimeError> {
        let Some(retirement) = self.pending_retirements.get(retirement_id).cloned() else {
            return Ok(());
        };
        let participant_count = retirement.record.participants.len();
        if !retirement.final_sent && retirement.prepared.len() == participant_count {
            let stage_epoch = self.journal.last_index() + 1;
            let record = JournalRecord::BlockRetirementFinalized {
                retirement_id: retirement_id.to_vec(),
                stage_epoch,
            };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
        }
        let Some(retirement) = self.pending_retirements.get(retirement_id).cloned() else {
            return Ok(());
        };
        if retirement.final_sent && retirement.released.len() == participant_count {
            let record = JournalRecord::BlockRetirementReleased {
                retirement_id: retirement_id.to_vec(),
                block_ids: retirement.record.block_ids.clone(),
                stage_epoch: self.journal.last_index() + 1,
            };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
        }
        Ok(())
    }

    fn advance_retirements(&mut self) {
        let ids = self.pending_retirements.keys().cloned().collect::<Vec<_>>();
        for retirement_id in ids {
            if let Err(error) = self.advance_retirement_after_ack(&retirement_id) {
                dms_logging::warn!(
                    "block retirement advance failed";
                    "error" => format!("{error:?}")
                );
            }
        }
    }

    #[cfg(test)]
    fn stats(&self) -> MetaStats {
        MetaStats {
            journal_last_index: self.journal.last_index(),
            snapshot_index: self.last_checkpoint_index,
            version_count: self.versions.values().map(BTreeMap::len).sum(),
            operation_count: self.operations.len(),
            replica_operation_count: self.replica_operations.len(),
            event_count: self.events.len(),
            event_high_watermark: self.event_high_watermark,
            replica_block_count: self.live_replica_block_count(),
            replica_location_count: self.live_replica_location_count(),
        }
    }

    #[cfg(test)]
    fn live_replica_block_count(&self) -> usize {
        self.replicas
            .values()
            .filter(|replicas| replicas.iter().any(|replica| self.is_live_replica(replica)))
            .count()
    }

    fn live_replica_location_count(&self) -> usize {
        self.replicas
            .values()
            .map(|replicas| {
                replicas
                    .iter()
                    .filter(|replica| self.is_live_replica(replica))
                    .count()
            })
            .sum()
    }

    fn snapshot(&self) -> MetaSnapshot {
        let replicas = self
            .replicas
            .iter()
            .flat_map(|(block_id, replicas)| {
                replicas.iter().map(|replica| SnapshotReplica {
                    block_id: block_id.clone(),
                    location: replica.location.clone(),
                    catalog_revision: replica.catalog_revision,
                    length: replica.length,
                })
            })
            .collect();
        let versions = self
            .versions
            .iter()
            .map(|(key, versions)| {
                (
                    key.clone(),
                    versions
                        .values()
                        .cloned()
                        .collect::<Vec<pb::VersionLayout>>(),
                )
            })
            .collect();
        let operations = self
            .operations
            .iter()
            .map(|(operation_id, operation)| SnapshotOperation {
                operation_id: operation_id.clone(),
                digest: operation.digest.clone(),
                result: operation.result,
            })
            .collect();
        let commit_sequences = self
            .sessions
            .values()
            .flat_map(|session| session.commit_sequences.values().cloned())
            .collect();
        let (
            filesystem_next_inode,
            filesystem_inodes,
            filesystem_dentries,
            filesystem_grant_generations,
            filesystem_xattrs,
        ) = self.filesystem.snapshot();
        MetaSnapshot {
            last_applied_index: self.last_applied_index,
            version_floor: self.version_floor,
            next_session: self.next_session,
            node_epochs: self
                .node_epochs
                .iter()
                .map(|(node_id, epoch)| (*node_id, *epoch))
                .collect(),
            node_commit_sequence_floors: self
                .node_commit_sequence_floors
                .iter()
                .map(|(node_id, floor)| (*node_id, *floor))
                .collect(),
            sessions: self
                .sessions
                .iter()
                .map(|(node_id, session)| SnapshotSession {
                    node_id: *node_id,
                    session_id: session.session_id.clone(),
                    node_epoch: session.node_epoch,
                    control_endpoint: session.control_endpoint.clone(),
                    last_acked_cursor: session.last_acked_cursor,
                    supports_commit_sequence: session.supports_commit_sequence,
                    commit_sequence_floor: session.commit_sequence_floor,
                })
                .collect(),
            replicas,
            desired_replica_counts: self
                .desired_replica_counts
                .iter()
                .map(|(block_id, copies)| (block_id.clone(), *copies))
                .collect(),
            versions,
            version_modified_times: self
                .version_modified_times
                .iter()
                .map(|((key, version), modified_time)| (key.clone(), *version, *modified_time))
                .collect(),
            block_retirements: self
                .pending_retirements
                .values()
                .map(|retirement| SnapshotBlockRetirement {
                    record: retirement.record.clone(),
                    prepared: retirement
                        .prepared
                        .iter()
                        .map(|(node_id, node_epoch)| BlockRetirementParticipant {
                            node_id: *node_id,
                            node_epoch: *node_epoch,
                        })
                        .collect(),
                    released: retirement
                        .released
                        .iter()
                        .map(|(node_id, node_epoch)| BlockRetirementParticipant {
                            node_id: *node_id,
                            node_epoch: *node_epoch,
                        })
                        .collect(),
                    final_sent: retirement.final_sent,
                })
                .collect(),
            retired_block_fences: self
                .retired_block_fences
                .iter()
                .map(|(block_id, fence_version)| (block_id.clone(), *fence_version))
                .collect(),
            commit_sequences,
            operations,
            replica_operations: self
                .replica_operations
                .iter()
                .map(|(operation_id, result)| SnapshotReplicaOperation {
                    operation_id: operation_id.clone(),
                    result: result.clone(),
                })
                .collect(),
            event_high_watermark: self.event_high_watermark,
            events: self.events.clone(),
            filesystem_next_inode,
            filesystem_inodes,
            filesystem_dentries,
            filesystem_grant_generations,
            filesystem_xattrs,
            filesystem_namespace_operations: self
                .filesystem_namespace_operations
                .iter()
                .map(|(operation_id, operation)| {
                    super::metadata_journal::SnapshotFilesystemNamespaceOperation {
                        operation_id: operation_id.clone(),
                        digest: operation.digest.clone(),
                        result: operation.result.clone(),
                    }
                })
                .collect(),
            filesystem_version_operations: self
                .filesystem_version_operations
                .iter()
                .map(
                    |(operation_id, operation)| SnapshotFilesystemVersionOperation {
                        operation_id: operation_id.clone(),
                        digest: operation.digest.clone(),
                        response: operation.response.clone(),
                    },
                )
                .collect(),
            filesystem_attribute_operations: self
                .filesystem_attribute_operations
                .iter()
                .map(
                    |(operation_id, operation)| SnapshotFilesystemAttributeOperation {
                        operation_id: operation_id.clone(),
                        digest: operation.digest.clone(),
                        response: operation.response.clone(),
                    },
                )
                .collect(),
            filesystem_operation_visibility: Some(SnapshotFilesystemOperationVisibility {
                namespace: self
                    .filesystem_namespace_operations
                    .iter()
                    .filter_map(|(operation_id, operation)| {
                        operation
                            .visibility_cursor
                            .map(|cursor| (operation_id.clone(), cursor))
                    })
                    .collect(),
                versions: self
                    .filesystem_version_operations
                    .iter()
                    .filter_map(|(operation_id, operation)| {
                        operation
                            .visibility_cursor
                            .map(|cursor| (operation_id.clone(), cursor))
                    })
                    .collect(),
                attributes: self
                    .filesystem_attribute_operations
                    .iter()
                    .filter_map(|(operation_id, operation)| {
                        operation
                            .visibility_cursor
                            .map(|cursor| (operation_id.clone(), cursor))
                    })
                    .collect(),
            }),
        }
    }

    fn restore_snapshot(&mut self, snapshot: MetaSnapshot) {
        let filesystem_operation_visibility = snapshot.filesystem_operation_visibility.clone();
        self.filesystem = FilesystemCatalog::restored(
            snapshot.filesystem_next_inode,
            snapshot.filesystem_inodes.clone(),
            snapshot.filesystem_dentries.clone(),
            snapshot.filesystem_grant_generations.clone(),
            snapshot.filesystem_xattrs.clone(),
        );
        self.last_applied_index = snapshot.last_applied_index;
        self.version_floor = snapshot.version_floor;
        self.next_session = snapshot.next_session;
        self.node_epochs = snapshot.node_epochs.into_iter().collect();
        self.node_commit_sequence_floors =
            snapshot.node_commit_sequence_floors.into_iter().collect();
        self.sessions = snapshot
            .sessions
            .into_iter()
            .map(|session| {
                (
                    session.node_id,
                    NodeSession {
                        session_id: session.session_id,
                        node_epoch: session.node_epoch,
                        control_endpoint: session.control_endpoint,
                        last_acked_cursor: session.last_acked_cursor,
                        last_heartbeat: None,
                        resources: None,
                        supports_commit_sequence: session.supports_commit_sequence,
                        commit_sequence_floor: session.commit_sequence_floor,
                        commit_sequences: HashMap::new(),
                    },
                )
            })
            .collect();
        for (node_id, session) in &self.sessions {
            self.node_commit_sequence_floors
                .entry(*node_id)
                .and_modify(|floor| *floor = (*floor).max(session.commit_sequence_floor))
                .or_insert(session.commit_sequence_floor);
        }
        let restored_nodes = self.sessions.keys().copied().collect::<Vec<_>>();
        for node_id in restored_nodes {
            self.remember_prior_lease(node_id, self.recovery_lease_until);
        }
        for replica in snapshot.replicas {
            self.replicas
                .entry(replica.block_id)
                .or_default()
                .push(StoredReplica {
                    location: replica.location,
                    catalog_revision: replica.catalog_revision,
                    length: replica.length,
                });
        }
        self.desired_replica_counts = snapshot.desired_replica_counts.into_iter().collect();
        for (key, layouts) in snapshot.versions {
            let by_version = layouts
                .into_iter()
                .map(|layout| (layout.version, layout))
                .collect::<BTreeMap<_, _>>();
            if by_version
                .last_key_value()
                .is_some_and(|(_, layout)| layout.kind == pb::VersionKind::Value as i32)
            {
                self.live_keys.insert(key.clone());
            }
            self.versions.insert(key, by_version);
        }
        self.version_modified_times = snapshot
            .version_modified_times
            .into_iter()
            .map(|(key, version, modified_time)| ((key, version), modified_time))
            .collect();
        self.version_floor = self.version_floor.max(
            self.versions
                .values()
                .flat_map(|versions| versions.keys())
                .copied()
                .max()
                .unwrap_or(0),
        );
        self.pending_retirements = snapshot
            .block_retirements
            .into_iter()
            .map(|retirement| {
                (
                    retirement.record.retirement_id.clone(),
                    PendingRetirement {
                        record: retirement.record,
                        prepared: retirement
                            .prepared
                            .into_iter()
                            .map(|participant| (participant.node_id, participant.node_epoch))
                            .collect(),
                        released: retirement
                            .released
                            .into_iter()
                            .map(|participant| (participant.node_id, participant.node_epoch))
                            .collect(),
                        final_sent: retirement.final_sent,
                    },
                )
            })
            .collect();
        self.retired_block_fences = snapshot.retired_block_fences.into_iter().collect();
        for record in snapshot.commit_sequences {
            self.remember_commit_sequence(record);
        }
        let commit_sequence_by_operation = self
            .sessions
            .values()
            .flat_map(|session| session.commit_sequences.values())
            .map(|record| {
                (
                    (record.operation_id.clone(), record.operation_digest.clone()),
                    record.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        self.operations = snapshot
            .operations
            .into_iter()
            .map(|operation| {
                let commit_sequence = commit_sequence_by_operation
                    .get(&(operation.operation_id.clone(), operation.digest.clone()))
                    .cloned();
                (
                    operation.operation_id,
                    StoredOperation {
                        digest: operation.digest,
                        result: operation.result,
                        commit_sequence,
                        visibility_cursor: None,
                    },
                )
            })
            .collect();
        self.replica_operations = snapshot
            .replica_operations
            .into_iter()
            .map(|operation| (operation.operation_id, operation.result))
            .collect();
        self.filesystem_namespace_operations = snapshot
            .filesystem_namespace_operations
            .into_iter()
            .map(|operation| {
                (
                    operation.operation_id,
                    StoredFilesystemNamespaceOperation {
                        digest: operation.digest,
                        visibility_cursor: None,
                        result: operation.result,
                    },
                )
            })
            .collect();
        self.filesystem_version_operations = snapshot
            .filesystem_version_operations
            .into_iter()
            .map(|operation| {
                (
                    operation.operation_id,
                    StoredFilesystemVersionOperation {
                        digest: operation.digest,
                        response: operation.response,
                        visibility_cursor: None,
                    },
                )
            })
            .collect();
        self.filesystem_attribute_operations = snapshot
            .filesystem_attribute_operations
            .into_iter()
            .map(|operation| {
                (
                    operation.operation_id,
                    StoredFilesystemAttributeOperation {
                        digest: operation.digest,
                        response: operation.response,
                        visibility_cursor: None,
                    },
                )
            })
            .collect();
        self.event_high_watermark = snapshot.event_high_watermark;
        self.events = snapshot.events;
        self.event_high_watermark = self.event_high_watermark.max(
            self.events
                .iter()
                .map(|event| event.cursor)
                .max()
                .unwrap_or(0),
        );
        // Pending replies are process-local, but a retained invalidation event
        // proves that the durable operation is not yet globally visible. A
        // retried OperationId therefore re-enters the same ACK barrier after
        // recovery instead of returning early.
        let visibility_by_commit = self
            .events
            .iter()
            .filter_map(|event| match &event.event {
                Some(pb::node_event::Event::InvalidateCurrent(invalidation)) => {
                    let bytes: [u8; 8] = invalidation.transition_id.as_slice().try_into().ok()?;
                    Some((u64::from_be_bytes(bytes), event.cursor))
                }
                _ => None,
            })
            .fold(
                HashMap::<u64, u64>::new(),
                |mut cursors, (index, cursor)| {
                    cursors
                        .entry(index)
                        .and_modify(|current| *current = (*current).max(cursor))
                        .or_insert(cursor);
                    cursors
                },
            );
        for operation in self.operations.values_mut() {
            operation.visibility_cursor = visibility_by_commit
                .get(&operation.result.commit_index)
                .copied();
        }
        let retained_event_cursors = self
            .events
            .iter()
            .map(|event| event.cursor)
            .collect::<HashSet<_>>();
        if let Some(visibility) = filesystem_operation_visibility {
            // 新 snapshot 明确记录未完成的屏障。空列表表示 ACK 已完成，不能再根据
            // 仍在保留窗口内的旧事件把该操作误判成 pending。
            for (operation_id, cursor) in visibility.namespace {
                if let Some(operation) = self.filesystem_namespace_operations.get_mut(&operation_id)
                {
                    operation.visibility_cursor = Some(cursor);
                }
            }
            for (operation_id, cursor) in visibility.versions {
                if let Some(operation) = self.filesystem_version_operations.get_mut(&operation_id) {
                    operation.visibility_cursor = Some(cursor);
                }
            }
            for (operation_id, cursor) in visibility.attributes {
                if let Some(operation) = self.filesystem_attribute_operations.get_mut(&operation_id)
                {
                    operation.visibility_cursor = Some(cursor);
                }
            }
        } else {
            // 兼容旧 snapshot：旧格式没有显式 pending 状态，只能由仍保留的事件保守恢复。
            for operation in self.filesystem_namespace_operations.values_mut() {
                let count = operation.result.changed_directories.len() as u64;
                let last = operation.result.invalidation_cursor;
                let first = last.saturating_sub(count.saturating_sub(1));
                operation.visibility_cursor = (count != 0
                    && (first..=last).any(|cursor| retained_event_cursors.contains(&cursor)))
                .then_some(last);
            }
            for operation in self.filesystem_version_operations.values_mut() {
                let cursor = operation.response.invalidation_cursor;
                operation.visibility_cursor =
                    retained_event_cursors.contains(&cursor).then_some(cursor);
            }
            for operation in self.filesystem_attribute_operations.values_mut() {
                let cursor = operation.response.invalidation_cursor;
                operation.visibility_cursor =
                    retained_event_cursors.contains(&cursor).then_some(cursor);
            }
        }
    }
}

fn validate_condition(
    condition: &str,
    expected_version: Option<u64>,
    current: Option<&pb::VersionLayout>,
) -> Result<(), MetaRuntimeError> {
    let actual = current.map_or(0, |layout| layout.version);
    let exists = current.is_some_and(|layout| layout.kind == pb::VersionKind::Value as i32);
    let accepted = match condition {
        "" | "any" => true,
        "if-absent" => !exists,
        "if-present" => exists,
        value if value.starts_with("if-version:") => value
            .strip_prefix("if-version:")
            .and_then(|version| version.parse::<u64>().ok())
            .is_some_and(|version| version == actual),
        _ => expected_version.is_some_and(|version| version == actual),
    };
    accepted.then_some(()).ok_or(MetaRuntimeError::Conflict {
        expected: expected_version,
        actual,
    })
}

fn validate_path_component(name: &[u8], operation: &str) -> Result<(), MetaRuntimeError> {
    if name.is_empty() || name.contains(&b'/') {
        return Err(MetaRuntimeError::InvalidArgument(format!(
            "filesystem {operation} requires one non-empty path component"
        )));
    }
    Ok(())
}

fn filesystem_remove_kind(kind: i32) -> Result<RemoveKind, MetaRuntimeError> {
    match pb::FilesystemRemoveKind::try_from(kind) {
        Ok(pb::FilesystemRemoveKind::File) => Ok(RemoveKind::File),
        Ok(pb::FilesystemRemoveKind::Directory) => Ok(RemoveKind::Directory),
        _ => Err(MetaRuntimeError::InvalidArgument(
            "invalid filesystem remove kind".to_string(),
        )),
    }
}

fn scan_limit(limit: u32) -> usize {
    match limit {
        0 => DEFAULT_SCAN_LIMIT,
        value => (value as usize).min(MAX_SCAN_LIMIT),
    }
}

fn batch_commit_sequence_digest(entries: &[pb::BatchCommitEntry]) -> Vec<u8> {
    fn put_bytes(out: &mut Vec<u8>, value: &[u8]) {
        out.extend_from_slice(&(value.len() as u64).to_be_bytes());
        out.extend_from_slice(value);
    }

    fn put_optional_u64(out: &mut Vec<u8>, value: Option<u64>) {
        match value {
            Some(value) => {
                out.push(1);
                out.extend_from_slice(&value.to_be_bytes());
            }
            None => out.push(0),
        }
    }

    let mut digest = b"dms-meta-batch-commit-sequence-v1".to_vec();
    digest.extend_from_slice(&(entries.len() as u64).to_be_bytes());
    for entry in entries {
        if let Some(key) = &entry.key {
            put_bytes(&mut digest, &key.value);
        } else {
            put_bytes(&mut digest, &[]);
        }
        put_bytes(&mut digest, entry.condition.as_bytes());
        put_optional_u64(&mut digest, entry.expected_version);
        put_bytes(&mut digest, &entry.operation_id);
        put_bytes(&mut digest, &entry.operation_digest);
        if let Some(candidate) = &entry.candidate {
            digest.extend_from_slice(&(candidate.kind as u64).to_be_bytes());
            digest.extend_from_slice(&candidate.logical_length.to_be_bytes());
            put_bytes(&mut digest, &candidate.digest);
            digest.extend_from_slice(&(candidate.extents.len() as u64).to_be_bytes());
            for extent in &candidate.extents {
                if let Some(range) = &extent.logical {
                    digest.push(1);
                    digest.extend_from_slice(&range.offset.to_be_bytes());
                    digest.extend_from_slice(&range.length.to_be_bytes());
                } else {
                    digest.push(0);
                }
                put_bytes(&mut digest, &extent.block_id);
                digest.extend_from_slice(&extent.block_offset.to_be_bytes());
                put_bytes(&mut digest, &extent.digest);
            }
        } else {
            digest.extend_from_slice(&0_u64.to_be_bytes());
            digest.extend_from_slice(&0_u64.to_be_bytes());
            put_bytes(&mut digest, &[]);
            digest.extend_from_slice(&0_u64.to_be_bytes());
        }
        digest.extend_from_slice(&(entry.replica_proofs.len() as u64).to_be_bytes());
        for proof in &entry.replica_proofs {
            put_bytes(&mut digest, &proof.block_id);
            digest.extend_from_slice(&proof.node_id.to_be_bytes());
            digest.extend_from_slice(&proof.node_epoch.to_be_bytes());
            digest.extend_from_slice(&proof.catalog_revision.to_be_bytes());
            put_bytes(&mut digest, &proof.checksum);
            digest.extend_from_slice(&(proof.durability as u64).to_be_bytes());
        }
        digest.extend_from_slice(&(entry.new_replicas.len() as u64).to_be_bytes());
        for replica in &entry.new_replicas {
            put_bytes(&mut digest, &replica.block_id);
            digest.extend_from_slice(&replica.length.to_be_bytes());
            put_bytes(&mut digest, &replica.checksum);
            digest.extend_from_slice(&(replica.durability as u64).to_be_bytes());
        }
    }
    digest
}

fn current_time_unix_millis() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn current_time_unix_nanos() -> i64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    i64::try_from(nanos).unwrap_or(i64::MAX)
}

/// 在 Meta 单一 owner turn 内更新 inode 的 reservation 关联。
///
/// reduction 必须完整命中当前 session 所拥有的范围，否则 Node 的 Arena 账本与
/// journal 会分叉。范围拆分只改变容量关联，不创建 DATA/HOLE Extent。
fn apply_filesystem_reservation_changes(
    reservations: &mut Vec<FileSpaceReservation>,
    session: &pb::NodeSessionIdentity,
    reductions: &[pb::FilesystemReservationRange],
    additions: &[pb::FilesystemReservationRange],
) -> Result<(), MetaRuntimeError> {
    for reduction in reductions {
        validate_reservation_range(reduction)?;
        let reduction_end = reduction
            .offset
            .checked_add(reduction.length)
            .expect("validated reservation range");
        let mut covered = 0_u64;
        let mut next = Vec::with_capacity(reservations.len() + 1);
        for reservation in reservations.drain(..) {
            let end = reservation
                .offset
                .checked_add(reservation.length)
                .ok_or_else(|| {
                    MetaRuntimeError::InvalidArgument(
                        "stored filesystem reservation range overflows".to_string(),
                    )
                })?;
            let same_owner = reservation.reservation_id == reduction.reservation_id
                && reservation.node_id == session.node_id
                && reservation.node_epoch == session.node_epoch;
            let overlap_start = reservation.offset.max(reduction.offset);
            let overlap_end = end.min(reduction_end);
            if !same_owner || overlap_start >= overlap_end {
                next.push(reservation);
                continue;
            }
            covered = covered.saturating_add(overlap_end - overlap_start);
            if reservation.offset < overlap_start {
                next.push(FileSpaceReservation {
                    length: overlap_start - reservation.offset,
                    ..reservation.clone()
                });
            }
            if overlap_end < end {
                next.push(FileSpaceReservation {
                    offset: overlap_end,
                    length: end - overlap_end,
                    ..reservation
                });
            }
        }
        *reservations = next;
        if covered != reduction.length {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem reservation reduction is not fully owned by this node session"
                    .to_string(),
            ));
        }
    }

    for addition in additions {
        validate_reservation_range(addition)?;
        let end = addition
            .offset
            .checked_add(addition.length)
            .expect("validated reservation range");
        if reservations.iter().any(|reservation| {
            let existing_end = reservation.offset.saturating_add(reservation.length);
            reservation.offset < end && addition.offset < existing_end
        }) {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem reservations must not overlap".to_string(),
            ));
        }
        reservations.push(FileSpaceReservation {
            reservation_id: addition.reservation_id.clone(),
            node_id: session.node_id,
            node_epoch: session.node_epoch,
            offset: addition.offset,
            length: addition.length,
        });
    }
    reservations.sort_by_key(|reservation| reservation.offset);
    Ok(())
}

fn validate_reservation_range(
    range: &pb::FilesystemReservationRange,
) -> Result<(), MetaRuntimeError> {
    if range.reservation_id.is_empty()
        || range.length == 0
        || range.offset.checked_add(range.length).is_none()
    {
        return Err(MetaRuntimeError::InvalidArgument(
            "filesystem reservation requires identity and a non-empty valid range".to_string(),
        ));
    }
    Ok(())
}

/// Meta 是 POSIX 属性修改的最终授权点。Node 只转发由内核提供的 caller，不能
/// 因为本地缓存命中而绕过此检查。首版没有 supplementary groups，因此普通用户
/// 只能把 gid 保持为自己当前的主组；更完整的 group/capability 语义以后扩展 caller。
fn authorize_attribute_patch(
    caller: &FilesystemCaller,
    current: &InodeAttributes,
    patch: &AttributePatch,
) -> Result<(), MetaRuntimeError> {
    if caller.uid == 0 {
        return Ok(());
    }
    let is_owner = caller.uid == current.uid;
    if patch.mode.is_some() && !is_owner {
        return Err(MetaRuntimeError::PermissionDenied);
    }
    if patch.uid.is_some_and(|uid| uid != current.uid) {
        return Err(MetaRuntimeError::PermissionDenied);
    }
    if patch
        .gid
        .is_some_and(|gid| !is_owner || (gid != current.gid && gid != caller.gid))
    {
        return Err(MetaRuntimeError::PermissionDenied);
    }
    if (!matches!(patch.atime, TimeUpdate::Omit) || !matches!(patch.mtime, TimeUpdate::Omit))
        && !is_owner
    {
        return Err(MetaRuntimeError::PermissionDenied);
    }
    Ok(())
}

fn authorize_xattr_mutation(
    caller: &FilesystemCaller,
    current: &InodeAttributes,
) -> Result<(), MetaRuntimeError> {
    if caller.uid == 0 || caller.uid == current.uid {
        Ok(())
    } else {
        Err(MetaRuntimeError::PermissionDenied)
    }
}

fn validate_filesystem_attribute_operation(
    operation_id: &[u8],
    operation_digest: &[u8],
) -> Result<(), MetaRuntimeError> {
    if operation_id.is_empty() || operation_digest.is_empty() {
        return Err(MetaRuntimeError::InvalidArgument(
            "filesystem attribute operation identity is required".to_string(),
        ));
    }
    Ok(())
}

/// 首版只开放用户自定义属性与 Linux POSIX ACL 的两个标准名称。
/// security/trusted namespace 需要能力模型，不能在尚未定义授权时静默接受。
fn validate_xattr_name(name: &[u8]) -> Result<(), MetaRuntimeError> {
    if name.is_empty() || name.len() > MAX_XATTR_NAME_BYTES || name.contains(&0) {
        return Err(MetaRuntimeError::XattrUnsupported);
    }
    if name == ACL_ACCESS_NAME
        || name == ACL_DEFAULT_NAME
        || name
            .strip_prefix(b"user.")
            .is_some_and(|suffix| !suffix.is_empty())
    {
        Ok(())
    } else {
        Err(MetaRuntimeError::XattrUnsupported)
    }
}

fn apply_attribute_patch(attributes: &mut InodeAttributes, patch: &AttributePatch, now: i64) {
    if let Some(mode) = patch.mode {
        // 文件类型由 inode kind 决定，chmod 只能修改 POSIX permission/special bits。
        attributes.mode = mode & 0o7777;
    }
    if let Some(uid) = patch.uid {
        attributes.uid = uid;
    }
    if let Some(gid) = patch.gid {
        attributes.gid = gid;
    }
    apply_time_update(&mut attributes.atime_unix_nanos, patch.atime, now);
    apply_time_update(&mut attributes.mtime_unix_nanos, patch.mtime, now);
}

fn apply_time_update(target: &mut i64, update: TimeUpdate, now: i64) {
    match update {
        TimeUpdate::Omit => {}
        TimeUpdate::Now => *target = now,
        TimeUpdate::Exact(value) => *target = value,
    }
}

fn filesystem_object_key(inode: u64) -> Vec<u8> {
    format!("fs/content/{inode}").into_bytes()
}

/// Meta 是文件大小与 VersionLayout 的最终提交权威，不能信任 Node 提交的派生布局。
/// 这里验证完整覆盖、稀疏洞约束和规范摘要；失败时整条 inode/object 原子提交不落 WAL。
fn validate_filesystem_candidate_layout(
    candidate: &pb::VersionCandidate,
) -> Result<HashSet<Vec<u8>>, MetaRuntimeError> {
    let mut cursor = 0_u64;
    let mut referenced_blocks = HashSet::new();
    for extent in &candidate.extents {
        let logical = extent.logical.as_ref().ok_or_else(|| {
            MetaRuntimeError::InvalidArgument(
                "filesystem extent requires a logical range".to_string(),
            )
        })?;
        if logical.length == 0 {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem extent length must be positive".to_string(),
            ));
        }
        if logical.offset != cursor {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem extents must be ordered and contiguous".to_string(),
            ));
        }
        cursor = cursor.checked_add(logical.length).ok_or_else(|| {
            MetaRuntimeError::InvalidArgument(
                "filesystem logical extent range overflows u64".to_string(),
            )
        })?;
        if cursor > candidate.logical_length {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem extent exceeds the logical file size".to_string(),
            ));
        }

        if is_filesystem_sparse_hole(extent) {
            if extent.block_offset != 0 || !extent.block_id.is_empty() || !extent.digest.is_empty()
            {
                return Err(MetaRuntimeError::InvalidArgument(
                    "filesystem hole extent must not reference a block".to_string(),
                ));
            }
        } else if is_filesystem_data_extent(extent) {
            if extent.block_id.is_empty() || extent.digest.is_empty() {
                return Err(MetaRuntimeError::InvalidArgument(
                    "filesystem data extent requires block identity and digest".to_string(),
                ));
            }
            extent
                .block_offset
                .checked_add(logical.length)
                .ok_or_else(|| {
                    MetaRuntimeError::InvalidArgument(
                        "filesystem block extent range overflows u64".to_string(),
                    )
                })?;
            referenced_blocks.insert(extent.block_id.clone());
        } else {
            return Err(MetaRuntimeError::InvalidArgument(
                "filesystem extent kind is not supported".to_string(),
            ));
        }
    }
    if cursor != candidate.logical_length {
        return Err(MetaRuntimeError::InvalidArgument(
            "filesystem extents must cover the complete logical file".to_string(),
        ));
    }
    if candidate.digest != filesystem_layout_digest(candidate.logical_length, &candidate.extents) {
        return Err(MetaRuntimeError::InvalidArgument(
            "filesystem layout digest does not match its extents".to_string(),
        ));
    }
    Ok(referenced_blocks)
}

/// VersionLayout 的跨进程规范摘要。`Unspecified` 仅作为旧 DATA 编码兼容。
fn filesystem_layout_digest(logical_length: u64, extents: &[pb::ExtentRecord]) -> Vec<u8> {
    let mut bytes = logical_length.to_be_bytes().to_vec();
    for extent in extents {
        if let Some(logical) = &extent.logical {
            bytes.extend_from_slice(&logical.offset.to_be_bytes());
            bytes.extend_from_slice(&logical.length.to_be_bytes());
        }
        bytes.push(if is_filesystem_sparse_hole(extent) {
            2_u8
        } else {
            1_u8
        });
        bytes.extend_from_slice(&(extent.block_id.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&extent.block_id);
        bytes.extend_from_slice(&extent.block_offset.to_be_bytes());
        bytes.extend_from_slice(&extent.digest);
    }
    digest(&bytes)
}

fn is_filesystem_sparse_hole(extent: &pb::ExtentRecord) -> bool {
    extent.kind == pb::ExtentKind::Hole as i32
}

fn is_filesystem_data_extent(extent: &pb::ExtentRecord) -> bool {
    extent.kind == pb::ExtentKind::Unspecified as i32 || extent.kind == pb::ExtentKind::Data as i32
}

fn retirement_id(stage_epoch: u64, block_ids: &[Vec<u8>]) -> Vec<u8> {
    let mut id = b"retire:".to_vec();
    id.extend_from_slice(&stage_epoch.to_be_bytes());
    for block_id in block_ids {
        id.extend_from_slice(&(block_id.len() as u64).to_be_bytes());
        id.extend_from_slice(block_id);
    }
    id
}

fn event_id_with_suffix(stage_epoch: u64, node_id: u64) -> Vec<u8> {
    let mut id = stage_epoch.to_be_bytes().to_vec();
    id.extend_from_slice(&node_id.to_be_bytes());
    id
}

/// 把一次 namespace journal delta 转成 kernel dentry 精确失效集。
///
/// create/link 只有 upsert，unlink 只有 remove，rename/replace 可能同时包含
/// 两者。同一 `(parent, name)` 只通知一次；这些数据从已持久的
/// journal record 推导，恢复重放和在线 apply 因此会生成相同事件。
fn filesystem_dentry_invalidations(
    upsert_dentries: &[crate::filesystem::DentrySnapshot],
    remove_dentries: &[crate::filesystem::DentrySnapshot],
) -> HashMap<u64, Vec<pb::FilesystemDentryInvalidation>> {
    let mut names_by_parent = HashMap::<u64, BTreeSet<Vec<u8>>>::new();
    for dentry in upsert_dentries.iter().chain(remove_dentries) {
        names_by_parent
            .entry(dentry.parent)
            .or_default()
            .insert(dentry.name.clone());
    }
    names_by_parent
        .into_iter()
        .map(|(parent, names)| {
            let entries = names
                .into_iter()
                .map(|name| pb::FilesystemDentryInvalidation { parent, name })
                .collect();
            (parent, entries)
        })
        .collect()
}

struct ScanCursor {
    prefix: Vec<u8>,
    delimiter: Vec<u8>,
    last_key: Vec<u8>,
    expires_at_unix_millis: i64,
}

impl ScanCursor {
    fn encode(&self) -> String {
        if self.delimiter.is_empty() {
            format!(
                "v1:{}:{}:{}",
                self.expires_at_unix_millis,
                hex_encode(&self.prefix),
                hex_encode(&self.last_key)
            )
        } else {
            format!(
                "v2:{}:{}:{}:{}",
                self.expires_at_unix_millis,
                hex_encode(&self.prefix),
                hex_encode(&self.delimiter),
                hex_encode(&self.last_key)
            )
        }
    }

    fn decode(value: &str) -> Result<Self, MetaRuntimeError> {
        if value.len() > MAX_SCAN_CURSOR_LENGTH {
            return Err(MetaRuntimeError::InvalidArgument(
                "invalid scan cursor".to_string(),
            ));
        }
        let mut parts = value.split(':');
        let version = parts.next();
        let expires_at_unix_millis = parts
            .next()
            .and_then(|part| part.parse::<i64>().ok())
            .ok_or_else(|| MetaRuntimeError::InvalidArgument("invalid scan cursor".to_string()))?;
        let prefix =
            parts.next().map(hex_decode).transpose()?.ok_or_else(|| {
                MetaRuntimeError::InvalidArgument("invalid scan cursor".to_string())
            })?;
        let (delimiter, last_key) = match version {
            Some("v1") => {
                let last_key = parts.next().map(hex_decode).transpose()?.ok_or_else(|| {
                    MetaRuntimeError::InvalidArgument("invalid scan cursor".to_string())
                })?;
                (Vec::new(), last_key)
            }
            Some("v2") => {
                let delimiter = parts.next().map(hex_decode).transpose()?.ok_or_else(|| {
                    MetaRuntimeError::InvalidArgument("invalid scan cursor".to_string())
                })?;
                let last_key = parts.next().map(hex_decode).transpose()?.ok_or_else(|| {
                    MetaRuntimeError::InvalidArgument("invalid scan cursor".to_string())
                })?;
                if delimiter.is_empty() {
                    return Err(MetaRuntimeError::InvalidArgument(
                        "invalid scan cursor".to_string(),
                    ));
                }
                (delimiter, last_key)
            }
            _ => {
                return Err(MetaRuntimeError::InvalidArgument(
                    "invalid scan cursor".to_string(),
                ));
            }
        };
        if parts.next().is_some() || last_key.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "invalid scan cursor".to_string(),
            ));
        }
        if !last_key.starts_with(&prefix) {
            return Err(MetaRuntimeError::InvalidArgument(
                "invalid scan cursor".to_string(),
            ));
        }
        Ok(Self {
            prefix,
            delimiter,
            last_key,
            expires_at_unix_millis,
        })
    }
}

fn scan_group_prefix(prefix: &[u8], delimiter: &[u8], key: &[u8]) -> Option<Vec<u8>> {
    if delimiter.is_empty() {
        return None;
    }
    let suffix = key.strip_prefix(prefix)?;
    let delimiter_index = find_subslice(suffix, delimiter)?;
    let group_end = prefix
        .len()
        .checked_add(delimiter_index)?
        .checked_add(delimiter.len())?;
    Some(key[..group_end].to_vec())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn lexicographic_successor(value: &[u8]) -> Option<Vec<u8>> {
    let mut successor = value.to_vec();
    while let Some(last) = successor.last_mut() {
        if *last == u8::MAX {
            successor.pop();
        } else {
            *last = last.saturating_add(1);
            return Some(successor);
        }
    }
    None
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(value: &str) -> Result<Vec<u8>, MetaRuntimeError> {
    if !value.len().is_multiple_of(2) {
        return Err(MetaRuntimeError::InvalidArgument(
            "invalid scan cursor".to_string(),
        ));
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let high = hex_nibble(chunk[0])?;
        let low = hex_nibble(chunk[1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, MetaRuntimeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(MetaRuntimeError::InvalidArgument(
            "invalid scan cursor".to_string(),
        )),
    }
}

fn event_targets_node(event: &pb::NodeEvent, node_id: u64) -> bool {
    match &event.event {
        Some(pb::node_event::Event::InvalidateCurrent(invalidate)) => {
            // source_node_id=0 来自旧协议或旧 WAL，必须继续广播，不能因无法识别
            // 写入方而漏掉远端缓存失效。
            invalidate.source_node_id == 0 || invalidate.source_node_id != node_id
        }
        Some(pb::node_event::Event::RepairReplica(repair)) => repair
            .target
            .as_ref()
            .is_some_and(|target| target.node_id == node_id),
        Some(pb::node_event::Event::EvictReplica(evict)) if evict.participant_node_id != 0 => {
            evict.participant_node_id == node_id
        }
        Some(pb::node_event::Event::InvalidateFilesystemBinding(invalidate)) => {
            invalidate.source_node_id == 0 || invalidate.source_node_id != node_id
        }
        _ => true,
    }
}

fn retirement_event_outstanding_in(
    pending_retirements: &HashMap<Vec<u8>, PendingRetirement>,
    event: &pb::NodeEvent,
) -> bool {
    let Some(pb::node_event::Event::EvictReplica(evict)) = &event.event else {
        return false;
    };
    let Some(retirement) = pending_retirements.get(&evict.retirement_id) else {
        return false;
    };
    let participant = (evict.participant_node_id, evict.participant_node_epoch);
    match pb::BlockRetirementPhase::try_from(evict.phase) {
        Ok(pb::BlockRetirementPhase::Prepare) => !retirement.prepared.contains(&participant),
        Ok(pb::BlockRetirementPhase::Final) => !retirement.released.contains(&participant),
        _ => false,
    }
}

fn cursor_before_outstanding_retirement_in(
    events: &[pb::NodeEvent],
    pending_retirements: &HashMap<Vec<u8>, PendingRetirement>,
    node_id: u64,
    requested_cursor: u64,
) -> u64 {
    events
        .iter()
        .filter(|event| event.cursor <= requested_cursor)
        .find(|event| {
            if !event_targets_node(event, node_id) {
                return false;
            }
            retirement_event_outstanding_in(pending_retirements, event)
        })
        .map_or(requested_cursor, |event| event.cursor.saturating_sub(1))
}

fn outstanding_retirement_retry_event<'a>(
    events: &'a [pb::NodeEvent],
    pending_retirements: &HashMap<Vec<u8>, PendingRetirement>,
    node_id: u64,
    delivered_cursor: u64,
    last_gc_retry_cursor: u64,
) -> Option<&'a pb::NodeEvent> {
    let eligible = |event: &&pb::NodeEvent| {
        event.cursor <= delivered_cursor
            && event_targets_node(event, node_id)
            && retirement_event_outstanding_in(pending_retirements, event)
    };
    events
        .iter()
        .filter(|event| event.cursor > last_gc_retry_cursor)
        .find(eligible)
        .or_else(|| events.iter().find(eligible))
}

fn event_metric_type(event: &pb::NodeEvent) -> WatchEventType {
    match &event.event {
        Some(pb::node_event::Event::InvalidateCurrent(_)) => WatchEventType::Invalidation,
        Some(pb::node_event::Event::RepairReplica(_)) => WatchEventType::Repair,
        Some(pb::node_event::Event::EvictReplica(_)) => WatchEventType::Eviction,
        Some(pb::node_event::Event::FenceNode(_)) => WatchEventType::Fence,
        Some(pb::node_event::Event::Gap(_)) => WatchEventType::Gap,
        Some(pb::node_event::Event::InvalidateFilesystemBinding(_)) => {
            WatchEventType::FilesystemInvalidation
        }
        None => WatchEventType::Empty,
    }
}

fn digest(bytes: &[u8]) -> Vec<u8> {
    dms_transport::checksum::stable_digest_bytes(bytes).to_vec()
}

async fn run_meta(
    mut command_rx: mpsc::Receiver<QueuedMetaCommand>,
    mut state: MetaState,
    metrics: MetaMetrics,
    trace_periodic_operations: bool,
) {
    // 复杂健康快照最多每秒刷新一次；业务命令不再为指标扫描整个 catalog。
    // Watch 使用独立 10ms 有界投递窗口，读方无需 ACK 才能释放网络队列容量。
    let mut metrics_tick = tokio::time::interval(Duration::from_secs(1));
    let mut watch_tick = tokio::time::interval(Duration::from_millis(10));
    loop {
        let queued = tokio::select! {
            queued = command_rx.recv() => { let Some(queued) = queued else { break; }; queued }
            _ = metrics_tick.tick() => {
                if let Err(error) = state.retire_expired_sessions() {
                    dms_logging::warn!("session retirement journal failed"; "error" => format!("{error:?}"));
                }
                if let Err(error) = state.reap_filesystem_orphans() {
                    dms_logging::warn!("filesystem orphan reaping failed"; "error" => format!("{error:?}"));
                }
                state.enforce_retention();
                state.refresh_metrics(); continue;
            }
            _ = watch_tick.tick() => { state.pump_watchers(); continue; }
        };
        metrics.record_mailbox_receive(queued.name, queued.enqueued_at);
        let span = if !dms_tracing::tracing::enabled!(dms_tracing::tracing::Level::DEBUG)
            || (queued.name.is_periodic() && !trace_periodic_operations)
        {
            dms_tracing::tracing::Span::none()
        } else {
            let operation_name = format!("dms.meta.{}", queued.name.label());
            dms_tracing::tracing::debug_span!(
                "dms.meta.command",
                otel.name = operation_name.as_str(),
                otel.kind = "internal",
                command = ?queued.name,
            )
        };
        dms_tracing::set_parent(&span, &queued.trace_context);
        let command_name = queued.name;
        let command = queued.command;
        async {
            match command {
                MetaCommand::OpenNodeSession {
                    node_id,
                    control_endpoint,
                    supports_commit_sequence,
                    reply,
                } => {
                    let _ = reply.send(state.open_node_session(
                        node_id,
                        control_endpoint,
                        supports_commit_sequence,
                    ));
                }
                MetaCommand::Heartbeat {
                    session_id,
                    node_id,
                    node_epoch,
                    event_cursor,
                    resources,
                    reply,
                } => {
                    let _ = reply.send(state.heartbeat_with_resources(
                        session_id,
                        node_id,
                        node_epoch,
                        event_cursor,
                        resources,
                    ));
                }
                MetaCommand::ResolveObject { request, reply } => {
                    let _ = reply.send(state.resolve_object(request));
                }
                MetaCommand::ResolveObjects { request, reply } => {
                    let _ = reply.send(state.resolve_objects(request));
                }
                #[cfg(test)]
                MetaCommand::DebugResolveObjectWithoutSession {
                    key,
                    exact_version,
                    reply,
                } => {
                    let _ =
                        reply.send(state.debug_resolve_object_without_session(key, exact_version));
                }
                MetaCommand::ReportReplicas { request, reply } => {
                    let _ = reply.send(state.report_replicas(request));
                }
                MetaCommand::CommitVersion { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    match state.dispatch_commit_version(request) {
                        Ok(dispatch) => state.register_pending_commit(dispatch, reply),
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::CommitBatch { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    let source_node = request
                        .session
                        .as_ref()
                        .map_or(0, |session| session.node_id);
                    let operation_ids = request
                        .entries
                        .iter()
                        .map(|entry| entry.operation_id.clone())
                        .collect::<Vec<_>>();
                    match state.commit_batch(request) {
                        Ok(response) => state.register_pending_batch(
                            source_node,
                            operation_ids,
                            response,
                            reply,
                        ),
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::Stat { request, reply } => {
                    let _ = reply.send(state.stat(request));
                }
                MetaCommand::Scan { request, reply } => {
                    let _ = reply.send(state.scan(request));
                }
                MetaCommand::GetOperation { request, reply } => {
                    let _ = reply.send(state.get_operation(request));
                }
                MetaCommand::PlanReplicas { request, reply } => {
                    let _ = reply.send(state.plan_replicas(request));
                }
                MetaCommand::WatchNodeEvents {
                    request,
                    sender,
                    reply,
                } => {
                    let _ = reply.send(state.watch_node_events(request, sender));
                }
                MetaCommand::AcknowledgeNodeEvent { request, reply } => {
                    let _ = reply.send(state.acknowledge_node_event(request));
                }
                MetaCommand::AcknowledgeBlockRetirement { request, reply } => {
                    let _ = reply.send(state.acknowledge_block_retirement(request));
                }
                MetaCommand::FilesystemLookup { request, reply } => {
                    let _ = reply.send(state.filesystem_lookup(request));
                }
                MetaCommand::FilesystemGetInode { request, reply } => {
                    let _ = reply.send(state.filesystem_get_inode(request));
                }
                MetaCommand::FilesystemCreateInode { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    let source_node = request
                        .session
                        .as_ref()
                        .map_or(0, |session| session.node_id);
                    let operation_id = request.operation_id.clone();
                    match state.filesystem_create_inode(request) {
                        Ok(response) => {
                            let dispatch = state.dispatch_filesystem_namespace(
                                source_node,
                                operation_id,
                                response,
                            );
                            state.register_pending_filesystem_create(dispatch, reply);
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemCreateSymlink { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    let source_node = request
                        .session
                        .as_ref()
                        .map_or(0, |session| session.node_id);
                    let operation_id = request.operation_id.clone();
                    match state.filesystem_create_symlink(request) {
                        Ok(response) => {
                            let dispatch = state.dispatch_filesystem_namespace(
                                source_node,
                                operation_id,
                                response,
                            );
                            state.register_pending_filesystem_create(dispatch, reply);
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemReadDirectory { request, reply } => {
                    let _ = reply.send(state.filesystem_read_directory(request));
                }
                MetaCommand::FilesystemLinkEntry { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    let source_node = request
                        .session
                        .as_ref()
                        .map_or(0, |session| session.node_id);
                    let operation_id = request.operation_id.clone();
                    match state.filesystem_link_entry(request) {
                        Ok(response) => {
                            let dispatch = state.dispatch_filesystem_namespace(
                                source_node,
                                operation_id,
                                response,
                            );
                            state.register_pending_filesystem_namespace(dispatch, reply);
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemAcquireInodeReference { request, reply } => {
                    let _ = reply.send(state.filesystem_acquire_inode_reference(request));
                }
                MetaCommand::FilesystemRenewInodeReferences { request, reply } => {
                    let _ = reply.send(state.filesystem_renew_inode_references(request));
                }
                MetaCommand::FilesystemReleaseInodeReference { request, reply } => {
                    let _ = reply.send(state.filesystem_release_inode_reference(request));
                }
                MetaCommand::FilesystemRenameEntry { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    let source_node = request
                        .session
                        .as_ref()
                        .map_or(0, |session| session.node_id);
                    let operation_id = request.operation_id.clone();
                    match state.filesystem_rename_entry(request) {
                        Ok(response) => {
                            let dispatch = state.dispatch_filesystem_namespace(
                                source_node,
                                operation_id,
                                response,
                            );
                            state.register_pending_filesystem_namespace(dispatch, reply);
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemRemoveEntry { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    let source_node = request
                        .session
                        .as_ref()
                        .map_or(0, |session| session.node_id);
                    let operation_id = request.operation_id.clone();
                    match state.filesystem_remove_entry(request) {
                        Ok(response) => {
                            let dispatch = state.dispatch_filesystem_namespace(
                                source_node,
                                operation_id,
                                response,
                            );
                            state.register_pending_filesystem_namespace(dispatch, reply);
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemCommitVersion { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    match state.dispatch_filesystem_commit_version(request) {
                        Ok(dispatch) => state.register_pending_filesystem_commit(dispatch, reply),
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemSetAttributes { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    match state.dispatch_filesystem_set_attributes(request) {
                        Ok(dispatch) => {
                            state.register_pending_filesystem_attributes(dispatch, reply)
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemGetXattr { request, reply } => {
                    let _ = reply.send(state.filesystem_get_xattr(request));
                }
                MetaCommand::FilesystemListXattrs { request, reply } => {
                    let _ = reply.send(state.filesystem_list_xattrs(request));
                }
                MetaCommand::FilesystemSetXattr { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    match state.dispatch_filesystem_set_xattr(request) {
                        Ok(dispatch) => {
                            state.register_pending_filesystem_attributes(dispatch, reply)
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemRemoveXattr { request, reply } => {
                    if state.pending_commits.values().map(Vec::len).sum::<usize>()
                        >= META_MAILBOX_CAPACITY
                    {
                        let _ = reply.send(Err(MetaRuntimeError::Unavailable));
                        return;
                    }
                    match state.dispatch_filesystem_remove_xattr(request) {
                        Ok(dispatch) => {
                            state.register_pending_filesystem_attributes(dispatch, reply)
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                MetaCommand::FilesystemStat { request, reply } => {
                    let _ = reply.send(state.filesystem_stat(request));
                }
                MetaCommand::FilesystemTestLock { request, reply } => {
                    let _ = reply.send(state.filesystem_test_lock(request));
                }
                MetaCommand::FilesystemSetLock { request, reply } => {
                    let _ = reply.send(state.filesystem_set_lock(request));
                }
                MetaCommand::FilesystemCancelLockWait { request, reply } => {
                    let _ = reply.send(state.filesystem_cancel_lock_wait(request));
                }
                MetaCommand::FilesystemReleaseLockOwner { request, reply } => {
                    let _ = reply.send(state.filesystem_release_lock_owner(request));
                }
                MetaCommand::FilesystemReclaimLocks { request, reply } => {
                    let _ = reply.send(state.filesystem_reclaim_locks(request));
                }
                #[cfg(test)]
                MetaCommand::Stats { reply } => {
                    let _ = reply.send(Ok(state.stats()));
                }
            }
            dms_logging::debug!(
                "meta command completed";
                "event" => "meta.command.completed",
                "command" => format!("{command_name:?}"),
            );
        }
        .instrument(span)
        .await;
    }
}

#[cfg(test)]
pub(crate) fn failing_checkpoint_handle_for_test() -> MetaHandle {
    use super::metadata_journal::JournalEntry;
    #[derive(Default)]
    struct FailingCheckpoint(InMemoryJournal);
    impl MetadataJournal for FailingCheckpoint {
        fn append(&mut self, record: JournalRecord) -> Result<u64, JournalError> {
            self.0.append(record)
        }
        fn load_after(&self, sequence: u64) -> Result<Vec<JournalEntry>, JournalError> {
            self.0.load_after(sequence)
        }
        fn load_snapshot(&self) -> Result<Option<MetaSnapshot>, JournalError> {
            self.0.load_snapshot()
        }
        fn save_snapshot(&mut self, snapshot: MetaSnapshot) -> Result<(), JournalError> {
            if !snapshot.versions.is_empty() {
                return Err(JournalError::Unavailable(
                    "injected checkpoint failure after apply",
                ));
            }
            self.0.save_snapshot(snapshot)
        }
        fn truncate_prefix(&mut self, sequence: u64) -> Result<(), JournalError> {
            self.0.truncate_prefix(sequence)
        }
        fn last_index(&self) -> u64 {
            self.0.last_index()
        }
    }
    MetaHandle::try_spawn_with_policies(
        Box::<FailingCheckpoint>::default(),
        MetaCheckpointPolicy { every_records: 1 },
        MetaRetentionPolicy::default(),
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    use super::super::local_wal_journal::LocalWalJournal;
    use super::super::metadata_journal::JournalEntry;
    use super::*;

    #[derive(Clone, Default)]
    struct SharedFailingSnapshotJournal {
        inner: Arc<Mutex<InMemoryJournal>>,
        fail_snapshot: bool,
    }

    impl MetadataJournal for SharedFailingSnapshotJournal {
        fn append(&mut self, record: JournalRecord) -> Result<u64, JournalError> {
            self.inner.lock().expect("journal lock").append(record)
        }

        fn load_after(&self, sequence: u64) -> Result<Vec<JournalEntry>, JournalError> {
            self.inner
                .lock()
                .expect("journal lock")
                .load_after(sequence)
        }

        fn load_snapshot(&self) -> Result<Option<MetaSnapshot>, JournalError> {
            self.inner.lock().expect("journal lock").load_snapshot()
        }

        fn save_snapshot(&mut self, snapshot: MetaSnapshot) -> Result<(), JournalError> {
            if self.fail_snapshot && snapshot.last_applied_index > 0 {
                return Err(JournalError::Unavailable("injected snapshot failure"));
            }
            self.inner
                .lock()
                .expect("journal lock")
                .save_snapshot(snapshot)
        }

        fn truncate_prefix(&mut self, sequence: u64) -> Result<(), JournalError> {
            self.inner
                .lock()
                .expect("journal lock")
                .truncate_prefix(sequence)
        }

        fn last_index(&self) -> u64 {
            self.inner.lock().expect("journal lock").last_index()
        }
    }

    #[derive(Clone, Default)]
    struct SharedFailingAppendJournal {
        inner: Arc<Mutex<InMemoryJournal>>,
        fail_append: Arc<AtomicBool>,
    }

    impl MetadataJournal for SharedFailingAppendJournal {
        fn append(&mut self, record: JournalRecord) -> Result<u64, JournalError> {
            if self.fail_append.load(Ordering::Relaxed) {
                return Err(JournalError::Unavailable("injected append failure"));
            }
            self.inner.lock().expect("journal lock").append(record)
        }

        fn load_after(&self, sequence: u64) -> Result<Vec<JournalEntry>, JournalError> {
            self.inner
                .lock()
                .expect("journal lock")
                .load_after(sequence)
        }

        fn load_snapshot(&self) -> Result<Option<MetaSnapshot>, JournalError> {
            self.inner.lock().expect("journal lock").load_snapshot()
        }

        fn save_snapshot(&mut self, snapshot: MetaSnapshot) -> Result<(), JournalError> {
            self.inner
                .lock()
                .expect("journal lock")
                .save_snapshot(snapshot)
        }

        fn truncate_prefix(&mut self, sequence: u64) -> Result<(), JournalError> {
            self.inner
                .lock()
                .expect("journal lock")
                .truncate_prefix(sequence)
        }

        fn last_index(&self) -> u64 {
            self.inner.lock().expect("journal lock").last_index()
        }
    }

    #[test]
    fn filesystem_append_failure_keeps_object_and_inode_unpublished() {
        let journal = SharedFailingAppendJournal::default();
        let fail_append = Arc::clone(&journal.fail_append);
        let mut state = MetaState::new(Box::new(journal));
        let session = test_session(&mut state, 41);
        let inode = create_test_file(&mut state, session.clone(), b"atomic.bin");
        let key = filesystem_object_key(inode.attributes.inode);
        let before_index = state.journal.last_index();

        fail_append.store(true, Ordering::Relaxed);
        let result = state.filesystem_commit_version(filesystem_commit_request_for(
            session,
            &inode,
            b"atomic-block-v1",
            b"atomic-op-v1",
        ));

        assert!(matches!(
            result,
            Err(MetaRuntimeError::JournalAppendFailed(_))
        ));
        assert_eq!(state.journal.last_index(), before_index);
        assert!(!state.versions.contains_key(&key));
        let after = state
            .filesystem
            .inode(inode.attributes.inode)
            .expect("inode remains visible");
        assert_eq!(after.revision, inode.revision);
        assert!(after.content.is_none());
    }

    #[test]
    fn filesystem_namespace_append_failure_keeps_directory_unchanged() {
        let journal = SharedFailingAppendJournal::default();
        let fail_append = Arc::clone(&journal.fail_append);
        let mut state = MetaState::new(Box::new(journal));
        let session = test_session(&mut state, 40);
        let before_index = state.journal.last_index();

        fail_append.store(true, Ordering::Relaxed);
        let result = state.filesystem_create_inode(filesystem_create_request_for(
            session,
            ROOT_INODE,
            b"must-not-appear",
            pb::FilesystemInodeKind::RegularFile,
            b"namespace-append-failure",
        ));

        assert!(matches!(
            result,
            Err(MetaRuntimeError::JournalAppendFailed(_))
        ));
        assert_eq!(state.journal.last_index(), before_index);
        assert!(
            state
                .filesystem
                .lookup(ROOT_INODE, b"must-not-appear")
                .is_none(),
            "WAL 没有落盘时，内存目录树也不能提前发布"
        );
    }

    #[test]
    fn filesystem_commit_replays_object_and_inode_together() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 42);
        let inode = create_test_file(&mut state, session.clone(), b"replay.bin");
        let mut request =
            filesystem_commit_request_for(session, &inode, b"replay-block-v1", b"replay-op-v1");
        request
            .reservation_additions
            .push(pb::FilesystemReservationRange {
                reservation_id: b"replay-reservation-v1".to_vec(),
                offset: 4,
                length: 128,
            });
        let committed = state
            .filesystem_commit_version(request)
            .expect("filesystem commit");
        let resolved = committed.resolved.expect("resolved commit result");
        let committed_inode = resolved.inode.expect("committed inode");
        let binding = committed_inode.content.expect("content binding");

        let restored = MetaState::new(state.journal);
        let restored_inode = restored
            .filesystem
            .inode(inode.attributes.inode)
            .expect("restored inode");
        assert_eq!(restored_inode.revision, committed_inode.revision);
        assert_eq!(
            restored_inode.content.as_ref().unwrap().exact_version,
            binding.exact_version
        );
        assert_eq!(
            restored
                .versions
                .get(&binding.object_key)
                .and_then(|versions| versions.get(&binding.exact_version))
                .map(|layout| layout.logical_length),
            Some(4)
        );
        assert_eq!(restored_inode.reservations.len(), 1);
        assert_eq!(
            restored_inode.reservations[0],
            FileSpaceReservation {
                reservation_id: b"replay-reservation-v1".to_vec(),
                node_id: 42,
                node_epoch: 1,
                offset: 4,
                length: 128,
            }
        );
    }

    #[test]
    fn filesystem_reservation_reduction_splits_only_the_owned_range() {
        let session = pb::NodeSessionIdentity {
            session_id: b"session-7".to_vec(),
            node_id: 42,
            node_epoch: 3,
        };
        let mut reservations = vec![FileSpaceReservation {
            reservation_id: b"reservation".to_vec(),
            node_id: 42,
            node_epoch: 3,
            offset: 0,
            length: 100,
        }];

        apply_filesystem_reservation_changes(
            &mut reservations,
            &session,
            &[pb::FilesystemReservationRange {
                reservation_id: b"reservation".to_vec(),
                offset: 40,
                length: 20,
            }],
            &[],
        )
        .expect("reduce middle of reservation");

        assert_eq!(
            reservations,
            vec![
                FileSpaceReservation {
                    reservation_id: b"reservation".to_vec(),
                    node_id: 42,
                    node_epoch: 3,
                    offset: 0,
                    length: 40,
                },
                FileSpaceReservation {
                    reservation_id: b"reservation".to_vec(),
                    node_id: 42,
                    node_epoch: 3,
                    offset: 60,
                    length: 40,
                },
            ]
        );

        let stale_session = pb::NodeSessionIdentity {
            node_epoch: 4,
            ..session
        };
        assert!(matches!(
            apply_filesystem_reservation_changes(
                &mut reservations,
                &stale_session,
                &[pb::FilesystemReservationRange {
                    reservation_id: b"reservation".to_vec(),
                    offset: 0,
                    length: 1,
                }],
                &[],
            ),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));
    }

    #[test]
    fn filesystem_commit_idempotency_returns_original_result_after_later_commit() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 46);
        let inode = create_test_file(&mut state, session.clone(), b"idempotent.bin");
        let first_request = filesystem_commit_request_for(
            session.clone(),
            &inode,
            b"idempotent-block-v1",
            b"fs-op-v1",
        );
        let first_response = state
            .filesystem_commit_version(first_request.clone())
            .expect("first filesystem commit");
        let first_index = first_response.commit_index;
        let first_inode = crate::filesystem::inode_from_proto(
            first_response
                .resolved
                .as_ref()
                .expect("first resolved")
                .inode
                .clone()
                .expect("first inode"),
        )
        .expect("first inode proto");

        let second_response = state
            .filesystem_commit_version(filesystem_commit_request_for(
                session,
                &first_inode,
                b"idempotent-block-v2",
                b"fs-op-v2",
            ))
            .expect("second filesystem commit");
        assert_ne!(second_response.commit_index, first_index);

        let retried_first = state
            .filesystem_commit_version(first_request)
            .expect("idempotent retry after a later commit");
        assert_eq!(
            retried_first.commit_index, first_index,
            "同一 operation 重试必须返回原始提交结果，不能读取当前 inode 后错返后续提交"
        );
        assert_eq!(retried_first, first_response);
    }

    #[test]
    fn filesystem_visibility_and_exact_response_restore_as_one_lifecycle() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 47);
        let reader = test_session(&mut state, 48);
        let inode = create_test_file(&mut state, writer.clone(), b"visibility.bin");
        let request = filesystem_commit_request_for(
            writer,
            &inode,
            b"visibility-block-v1",
            b"visibility-op-v1",
        );
        let first = state
            .dispatch_filesystem_commit_version(request.clone())
            .expect("first filesystem dispatch");
        let cursor = first.event_cursor.expect("filesystem invalidation cursor");
        assert!(first.waiting_nodes.contains(&reader.node_id));

        let mut restored = MetaState::new(Box::<InMemoryJournal>::default());
        restored.restore_snapshot(state.snapshot());
        let retry = restored
            .dispatch_filesystem_commit_version(request.clone())
            .expect("retry after pending snapshot restore");
        assert_eq!(retry.response, first.response);
        assert_eq!(retry.event_cursor, Some(cursor));
        assert!(retry.waiting_nodes.contains(&reader.node_id));

        restored.complete_operation_visibility(std::slice::from_ref(&request.operation_id));
        let mut restored_after_ack = MetaState::new(Box::<InMemoryJournal>::default());
        restored_after_ack.restore_snapshot(restored.snapshot());
        let completed_retry = restored_after_ack
            .dispatch_filesystem_commit_version(request)
            .expect("retry after completed snapshot restore");
        assert_eq!(completed_retry.response, first.response);
        assert_eq!(completed_retry.event_cursor, None);
        assert!(completed_retry.waiting_nodes.is_empty());
        assert!(completed_retry.waiting_prior_lease_nodes.is_empty());
    }

    #[test]
    fn filesystem_commit_rejects_malformed_or_unowned_layout() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 49);
        let inode = create_test_file(&mut state, session.clone(), b"validate-layout.bin");

        let mut gap =
            filesystem_commit_request_for(session.clone(), &inode, b"gap-block", b"gap-operation");
        let gap_candidate = gap.candidate.as_mut().expect("candidate");
        gap_candidate.extents[0]
            .logical
            .as_mut()
            .expect("range")
            .offset = 1;
        gap_candidate.digest =
            filesystem_layout_digest(gap_candidate.logical_length, &gap_candidate.extents);
        assert!(matches!(
            state.filesystem_commit_version(gap),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));

        let mut forged_digest = filesystem_commit_request_for(
            session.clone(),
            &inode,
            b"digest-block",
            b"digest-operation",
        );
        forged_digest.candidate.as_mut().expect("candidate").digest = b"forged".to_vec();
        assert!(matches!(
            state.filesystem_commit_version(forged_digest),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));

        let mut zero_length = filesystem_commit_request_for(
            session.clone(),
            &inode,
            b"zero-block",
            b"zero-operation",
        );
        let zero_candidate = zero_length.candidate.as_mut().expect("candidate");
        zero_candidate.extents[0]
            .logical
            .as_mut()
            .expect("range")
            .length = 0;
        zero_candidate.digest =
            filesystem_layout_digest(zero_candidate.logical_length, &zero_candidate.extents);
        assert!(matches!(
            state.filesystem_commit_version(zero_length),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));

        let mut extra_replica = filesystem_commit_request_for(
            session,
            &inode,
            b"owned-block",
            b"extra-replica-operation",
        );
        extra_replica.new_replicas.push(pb::ReplicaReport {
            block_id: b"not-in-layout".to_vec(),
            length: 4,
            checksum: b"digest".to_vec(),
            durability: pb::DurabilityPolicy::LocalMemory as i32,
        });
        assert!(matches!(
            state.filesystem_commit_version(extra_replica),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));
    }

    #[test]
    fn filesystem_commit_survives_real_local_wal_reopen() {
        let directory = std::env::temp_dir().join(format!(
            "dms-filesystem-meta-restart-{}",
            uuid::Uuid::new_v4()
        ));
        let (inode_id, exact_version, inode_revision) = {
            let journal = LocalWalJournal::open(&directory).expect("open local WAL");
            let mut state = MetaState::new(Box::new(journal));
            let session = test_session(&mut state, 44);
            let inode = create_test_file(&mut state, session.clone(), b"restart.bin");
            let committed = state
                .filesystem_commit_version(filesystem_commit_request_for(
                    session,
                    &inode,
                    b"restart-block-v1",
                    b"restart-op-v1",
                ))
                .expect("commit before restart");
            let committed = committed.resolved.expect("resolved commit");
            let inode = committed.inode.expect("committed inode");
            (
                inode.attributes.as_ref().expect("attributes").inode,
                inode
                    .content
                    .as_ref()
                    .expect("content binding")
                    .exact_version,
                inode.revision,
            )
        };

        let journal = LocalWalJournal::open(&directory).expect("reopen local WAL");
        let restored = MetaState::new(Box::new(journal));
        let inode = restored
            .filesystem
            .inode(inode_id)
            .expect("inode restored after process-style reopen");
        assert_eq!(inode.revision, inode_revision);
        let binding = inode.content.as_ref().expect("binding restored");
        assert_eq!(binding.exact_version, exact_version);
        assert!(
            restored
                .versions
                .get(&binding.object_key)
                .is_some_and(|versions| versions.contains_key(&exact_version)),
            "the same WAL record must restore both inode binding and object version"
        );
        fs::remove_dir_all(directory).expect("remove test WAL");
    }

    #[test]
    fn filesystem_commit_applies_size_and_attributes_in_one_record() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 69);
        let inode = create_test_file(&mut state, session.clone(), b"combined-setattr.txt");
        let before = state.journal.last_index();
        let mut request = filesystem_commit_request_for(
            session,
            &inode,
            b"combined-setattr-block",
            b"combined-setattr-operation",
        );
        request.caller = Some(pb::FilesystemCallerIdentity {
            uid: 1000,
            gid: 1000,
            pid: 41,
        });
        request.attribute_patch = Some(pb::FilesystemAttributePatch {
            mode: Some(0o640),
            uid: None,
            gid: None,
            atime: Some(pb::FilesystemTimeUpdate {
                kind: pb::FilesystemTimeUpdateKind::Exact as i32,
                exact_unix_nanos: 111,
            }),
            mtime: Some(pb::FilesystemTimeUpdate {
                kind: pb::FilesystemTimeUpdateKind::Exact as i32,
                exact_unix_nanos: 222,
            }),
        });

        let response = state
            .filesystem_commit_version(request)
            .expect("combined content and attribute commit");
        assert_eq!(response.commit_index, before + 1);
        let inode = response
            .resolved
            .and_then(|resolved| resolved.inode)
            .expect("resolved inode");
        let attrs = inode.attributes.expect("combined attributes");
        assert_eq!(attrs.size, 4);
        assert_eq!(attrs.mode, 0o640);
        assert_eq!(attrs.atime_unix_nanos, 111);
        assert_eq!(attrs.mtime_unix_nanos, 222);
        assert!(inode.content.is_some());
    }

    #[test]
    fn filesystem_attributes_enforce_owner_permissions_and_are_idempotent() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 70);
        let inode = create_test_file(&mut state, session.clone(), b"attributes.txt");

        let owner_request = filesystem_attribute_request_for(
            session.clone(),
            &inode,
            b"owner-attributes",
            pb::FilesystemCallerIdentity {
                uid: 1000,
                gid: 1000,
                pid: 42,
            },
            pb::FilesystemAttributePatch {
                mode: Some(0o600),
                uid: None,
                gid: None,
                atime: Some(pb::FilesystemTimeUpdate {
                    kind: pb::FilesystemTimeUpdateKind::Exact as i32,
                    exact_unix_nanos: 11,
                }),
                mtime: Some(pb::FilesystemTimeUpdate {
                    kind: pb::FilesystemTimeUpdateKind::Exact as i32,
                    exact_unix_nanos: 22,
                }),
            },
        );
        let first = state
            .filesystem_set_attributes(owner_request.clone())
            .expect("owner chmod and utimens");
        let retry = state
            .filesystem_set_attributes(owner_request)
            .expect("same attribute operation retries idempotently");
        assert_eq!(first.commit_index, retry.commit_index);
        let attrs = first
            .resolved
            .as_ref()
            .and_then(|resolved| resolved.inode.as_ref())
            .and_then(|inode| inode.attributes.as_ref())
            .expect("updated attributes");
        assert_eq!(attrs.mode, 0o600);
        assert_eq!(attrs.atime_unix_nanos, 11);
        assert_eq!(attrs.mtime_unix_nanos, 22);
        assert!(attrs.ctime_unix_nanos > 0);

        let committed = crate::filesystem::inode_from_proto(
            first
                .resolved
                .expect("updated resolution")
                .inode
                .expect("updated inode"),
        )
        .expect("valid updated inode");
        let denied = state.filesystem_set_attributes(filesystem_attribute_request_for(
            session.clone(),
            &committed,
            b"foreign-chmod",
            pb::FilesystemCallerIdentity {
                uid: 2000,
                gid: 2000,
                pid: 43,
            },
            pb::FilesystemAttributePatch {
                mode: Some(0o777),
                uid: None,
                gid: None,
                atime: None,
                mtime: None,
            },
        ));
        assert!(matches!(denied, Err(MetaRuntimeError::PermissionDenied)));
        assert_eq!(
            state
                .filesystem
                .inode(committed.attributes.inode)
                .expect("inode remains")
                .attributes
                .mode,
            0o600,
            "rejected mutation must not change authoritative state"
        );

        let root = state
            .filesystem_set_attributes(filesystem_attribute_request_for(
                session,
                &committed,
                b"root-chown",
                pb::FilesystemCallerIdentity {
                    uid: 0,
                    gid: 0,
                    pid: 1,
                },
                pb::FilesystemAttributePatch {
                    mode: None,
                    uid: Some(2000),
                    gid: Some(3000),
                    atime: None,
                    mtime: None,
                },
            ))
            .expect("root chown");
        let attrs = root
            .resolved
            .and_then(|resolved| resolved.inode)
            .and_then(|inode| inode.attributes)
            .expect("chowned attributes");
        assert_eq!((attrs.uid, attrs.gid), (2000, 3000));
    }

    #[test]
    fn filesystem_attributes_survive_real_local_wal_reopen() {
        let directory = std::env::temp_dir().join(format!(
            "dms-filesystem-attributes-restart-{}",
            uuid::Uuid::new_v4()
        ));
        let (inode_id, committed_revision, operation_id) = {
            let journal = LocalWalJournal::open(&directory).expect("open local WAL");
            let mut state = MetaState::new(Box::new(journal));
            let session = test_session(&mut state, 71);
            let inode = create_test_file(&mut state, session.clone(), b"restart-attrs.txt");
            let request = filesystem_attribute_request_for(
                session,
                &inode,
                b"restart-attributes",
                pb::FilesystemCallerIdentity {
                    uid: 1000,
                    gid: 1000,
                    pid: 44,
                },
                pb::FilesystemAttributePatch {
                    mode: Some(0o640),
                    uid: None,
                    gid: None,
                    atime: None,
                    mtime: Some(pb::FilesystemTimeUpdate {
                        kind: pb::FilesystemTimeUpdateKind::Exact as i32,
                        exact_unix_nanos: 123_456,
                    }),
                },
            );
            let operation_id = request.operation_id.clone();
            let response = state
                .filesystem_set_attributes(request)
                .expect("set attrs before restart");
            let inode = response.resolved.expect("resolution").inode.expect("inode");
            (
                inode.attributes.expect("attrs").inode,
                inode.revision,
                operation_id,
            )
        };

        let journal = LocalWalJournal::open(&directory).expect("reopen local WAL");
        let restored = MetaState::new(Box::new(journal));
        let inode = restored
            .filesystem
            .inode(inode_id)
            .expect("attribute inode restored after process-style reopen");
        assert_eq!(inode.revision, committed_revision);
        assert_eq!(inode.attributes.mode, 0o640);
        assert_eq!(inode.attributes.mtime_unix_nanos, 123_456);
        assert!(
            restored
                .filesystem_attribute_operations
                .contains_key(&operation_id),
            "attribute idempotency result must survive WAL replay"
        );
        fs::remove_dir_all(directory).expect("remove test WAL");
    }

    #[test]
    fn filesystem_xattr_modes_acl_and_removal_share_inode_revision() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 72);
        let inode = create_test_file(&mut state, session.clone(), b"xattr.txt");
        let caller = pb::FilesystemCallerIdentity {
            uid: 1000,
            gid: 1000,
            pid: 45,
        };

        let set_request = filesystem_set_xattr_request_for(
            session.clone(),
            &inode,
            b"set-user-xattr",
            caller,
            b"user.build",
            b"ready",
            pb::FilesystemXattrSetMode::CreateOnly,
        );
        let first = state
            .filesystem_set_xattr(set_request.clone())
            .expect("create xattr");
        let retried = state
            .filesystem_set_xattr(set_request)
            .expect("xattr retry is idempotent");
        assert_eq!(first.commit_index, retried.commit_index);
        let current = crate::filesystem::inode_from_proto(
            first
                .resolved
                .as_ref()
                .expect("resolved")
                .inode
                .clone()
                .expect("inode"),
        )
        .expect("valid inode");
        assert!(current.revision > inode.revision);
        assert_eq!(
            state
                .filesystem
                .xattr(inode.attributes.inode, b"user.build"),
            Some(b"ready".as_slice())
        );
        assert!(matches!(
            state.filesystem_set_xattr(filesystem_set_xattr_request_for(
                session.clone(),
                &current,
                b"duplicate-create-xattr",
                caller,
                b"user.build",
                b"again",
                pb::FilesystemXattrSetMode::CreateOnly,
            )),
            Err(MetaRuntimeError::XattrAlreadyExists)
        ));

        let acl = test_acl(0o7, 0o5, 0o1);
        let acl_response = state
            .filesystem_set_xattr(filesystem_set_xattr_request_for(
                session.clone(),
                &current,
                b"set-access-acl",
                caller,
                ACL_ACCESS_NAME,
                &acl,
                pb::FilesystemXattrSetMode::Upsert,
            ))
            .expect("set access ACL");
        let acl_inode = crate::filesystem::inode_from_proto(
            acl_response
                .resolved
                .expect("ACL resolved")
                .inode
                .expect("ACL inode"),
        )
        .expect("valid ACL inode");
        assert_eq!(acl_inode.attributes.mode & 0o777, 0o751);

        let chmod = state
            .filesystem_set_attributes(filesystem_attribute_request_for(
                session.clone(),
                &acl_inode,
                b"chmod-with-acl",
                caller,
                pb::FilesystemAttributePatch {
                    mode: Some(0o640),
                    uid: None,
                    gid: None,
                    atime: None,
                    mtime: None,
                },
            ))
            .expect("chmod updates access ACL");
        let chmod_inode = crate::filesystem::inode_from_proto(
            chmod
                .resolved
                .expect("chmod resolved")
                .inode
                .expect("chmod inode"),
        )
        .expect("valid chmod inode");
        let stored_acl = state
            .filesystem
            .xattr(inode.attributes.inode, ACL_ACCESS_NAME)
            .expect("access ACL remains stored");
        assert_eq!(
            validate_acl_xattr(
                ACL_ACCESS_NAME,
                stored_acl,
                InodeKind::RegularFile,
                chmod_inode.attributes.mode,
            ),
            Ok(Some(0o640))
        );

        state
            .filesystem_remove_xattr(filesystem_remove_xattr_request_for(
                session,
                &chmod_inode,
                b"remove-user-xattr",
                caller,
                b"user.build",
            ))
            .expect("remove xattr");
        assert!(
            state
                .filesystem
                .xattr(inode.attributes.inode, b"user.build")
                .is_none()
        );
    }

    #[test]
    fn filesystem_default_acl_is_inherited_by_new_children() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 73);
        let directory = state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"acl-parent",
                pb::FilesystemInodeKind::Directory,
                b"create-acl-parent",
            ))
            .expect("create parent")
            .resolved
            .and_then(|resolved| resolved.inode)
            .and_then(|inode| crate::filesystem::inode_from_proto(inode).ok())
            .expect("parent inode");
        let default_acl = test_acl(0o7, 0o7, 0o0);
        let updated = state
            .filesystem_set_xattr(filesystem_set_xattr_request_for(
                session.clone(),
                &directory,
                b"set-default-acl",
                pb::FilesystemCallerIdentity {
                    uid: 1000,
                    gid: 1000,
                    pid: 46,
                },
                ACL_DEFAULT_NAME,
                &default_acl,
                pb::FilesystemXattrSetMode::Upsert,
            ))
            .expect("set directory default ACL");
        let directory = crate::filesystem::inode_from_proto(
            updated
                .resolved
                .expect("updated parent")
                .inode
                .expect("parent inode"),
        )
        .expect("valid parent");

        let mut child_request = filesystem_create_request_for(
            session,
            directory.attributes.inode,
            b"child",
            pb::FilesystemInodeKind::RegularFile,
            b"create-acl-child",
        );
        child_request.mode = 0o640;
        let child = state
            .filesystem_create_inode(child_request)
            .expect("create child with inherited ACL")
            .resolved
            .and_then(|resolved| resolved.inode)
            .and_then(|inode| crate::filesystem::inode_from_proto(inode).ok())
            .expect("child inode");
        assert_eq!(child.attributes.mode & 0o777, 0o640);
        assert!(
            state
                .filesystem
                .xattr(child.attributes.inode, ACL_ACCESS_NAME)
                .is_some(),
            "default ACL must become the child access ACL in the same namespace commit"
        );
    }

    #[test]
    fn filesystem_stat_requires_and_aggregates_live_node_resource_reports() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let first = test_session(&mut state, 74);
        let request = pb::FilesystemStatRequest {
            context: None,
            session: Some(first.clone()),
        };
        assert!(matches!(
            state.filesystem_stat(request.clone()),
            Err(MetaRuntimeError::CapacityUnavailable)
        ));

        state
            .heartbeat_with_resources(
                first.session_id.clone(),
                first.node_id,
                first.node_epoch,
                0,
                Some(pb::ResourceSummary {
                    total_host_memory_bytes: 16 * 1024,
                    available_host_memory_bytes: 8 * 1024,
                    staged_bytes: 1024,
                    replica_bytes: 7 * 1024,
                }),
            )
            .expect("heartbeat with resources");
        let stats = state.filesystem_stat(request).expect("statfs");
        assert_eq!(stats.block_size, 4096);
        assert_eq!(stats.total_blocks, 4);
        assert_eq!(stats.free_blocks, 2);
        assert_eq!(stats.available_blocks, 2);
        assert_eq!(stats.reporting_nodes, 1);
        assert_eq!(stats.total_inodes, DEFAULT_FILESYSTEM_MAX_INODES);
        assert_eq!(stats.free_inodes, DEFAULT_FILESYSTEM_MAX_INODES - 1);
        assert_eq!(stats.capacity_revision, 1);
    }

    #[test]
    fn filesystem_commit_rejects_stale_inode_revision() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 43);
        let inode = create_test_file(&mut state, session.clone(), b"cas.bin");
        state
            .filesystem_commit_version(filesystem_commit_request_for(
                session.clone(),
                &inode,
                b"cas-block-v1",
                b"cas-op-v1",
            ))
            .expect("first commit");

        let stale = state.filesystem_commit_version(filesystem_commit_request_for(
            session,
            &inode,
            b"cas-block-v2",
            b"cas-op-v2",
        ));
        assert!(matches!(stale, Err(MetaRuntimeError::Conflict { .. })));
    }

    #[test]
    fn filesystem_namespace_create_readdir_rename_remove_replays() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 45);
        let directory = state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"dir",
                pb::FilesystemInodeKind::Directory,
                b"mkdir-dir",
            ))
            .expect("mkdir")
            .resolved
            .expect("directory resolution")
            .inode
            .expect("directory inode");
        let directory_inode = directory
            .attributes
            .as_ref()
            .expect("directory attrs")
            .inode;
        let file = state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                directory_inode,
                b"file.txt",
                pb::FilesystemInodeKind::RegularFile,
                b"create-nested-file",
            ))
            .expect("create nested file")
            .resolved
            .expect("file resolution")
            .inode
            .expect("file inode");
        let file_inode = file.attributes.as_ref().expect("file attrs").inode;

        let listing = state
            .filesystem_read_directory(pb::FilesystemReadDirectoryRequest {
                context: None,
                session: Some(session.clone()),
                directory: directory_inode,
                cursor: Vec::new(),
                limit: 16,
                expected_directory_revision: None,
            })
            .expect("readdir");
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(
            listing.entries[0]
                .dentry
                .as_ref()
                .expect("listed dentry")
                .name,
            b"file.txt"
        );

        let rename = state
            .filesystem_rename_entry(pb::FilesystemRenameRequest {
                context: None,
                session: Some(session.clone()),
                operation_id: b"rename-file".to_vec(),
                operation_digest: b"rename-file-digest".to_vec(),
                source_parent: directory_inode,
                source_name: b"file.txt".to_vec(),
                target_parent: ROOT_INODE,
                target_name: b"renamed.txt".to_vec(),
                expected_source_revision: None,
                expected_target_revision: None,
                commit_sequence: next_test_commit_sequence(),
                replace_existing: false,
            })
            .expect("rename");
        assert_eq!(rename.changed_directories.len(), 2);
        assert_eq!(
            state.events.len(),
            4,
            "mkdir/create only invalidate the changed parent; rename invalidates both changed parents"
        );
        assert!(
            state
                .filesystem
                .lookup(directory_inode, b"file.txt")
                .is_none()
        );
        assert!(
            state
                .filesystem
                .lookup(ROOT_INODE, b"renamed.txt")
                .is_some()
        );

        let removed = state
            .filesystem_remove_entry(pb::FilesystemRemoveRequest {
                context: None,
                session: Some(session.clone()),
                operation_id: b"remove-file".to_vec(),
                operation_digest: b"remove-file-digest".to_vec(),
                parent: ROOT_INODE,
                name: b"renamed.txt".to_vec(),
                kind: pb::FilesystemRemoveKind::File as i32,
                expected_parent_revision: None,
                commit_sequence: next_test_commit_sequence(),
            })
            .expect("unlink");
        assert_eq!(
            removed
                .inode
                .expect("removed inode")
                .attributes
                .unwrap()
                .link_count,
            0
        );
        assert!(
            state
                .filesystem
                .lookup(ROOT_INODE, b"renamed.txt")
                .is_none()
        );
        assert_eq!(
            state
                .filesystem
                .inode(file_inode)
                .expect("orphan inode retained")
                .attributes
                .link_count,
            0,
            "unlink/rmdir only detaches namespace; block/inode GC is a later policy"
        );

        let restored = MetaState::new(state.journal);
        assert!(
            restored
                .filesystem
                .lookup(ROOT_INODE, b"renamed.txt")
                .is_none()
        );
        assert!(
            restored
                .filesystem
                .lookup(directory_inode, b"file.txt")
                .is_none()
        );
        assert_eq!(
            restored
                .filesystem
                .inode(file_inode)
                .expect("orphan replayed")
                .attributes
                .link_count,
            0
        );
        assert_eq!(
            restored.filesystem_namespace_operations.len(),
            4,
            "namespace operation idempotency table must survive replay"
        );
    }

    #[test]
    fn filesystem_namespace_operation_is_idempotent_by_digest() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 46);
        let mut request = filesystem_create_request_for(
            session,
            ROOT_INODE,
            b"idem",
            pb::FilesystemInodeKind::RegularFile,
            b"idem-create",
        );
        let first = state
            .filesystem_create_inode(request.clone())
            .expect("first create")
            .resolved
            .expect("first resolution")
            .inode
            .expect("first inode")
            .attributes
            .expect("first attrs")
            .inode;
        let retry = state
            .filesystem_create_inode(request.clone())
            .expect("same id and digest retry")
            .resolved
            .expect("retry resolution")
            .inode
            .expect("retry inode")
            .attributes
            .expect("retry attrs")
            .inode;
        assert_eq!(first, retry);

        request.operation_digest = b"same-id-different-digest".to_vec();
        let conflict = state.filesystem_create_inode(request);
        assert!(matches!(conflict, Err(MetaRuntimeError::Conflict { .. })));
    }

    #[test]
    fn filesystem_symlink_rejects_same_operation_id_with_different_target_digest() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 60);
        let mut request = filesystem_symlink_request_for(
            session,
            b"symlink-idempotency",
            b"symlink-idempotency-digest-a",
        );

        state
            .filesystem_create_symlink(request.clone())
            .expect("first symlink create");
        request.operation_digest = b"symlink-idempotency-digest-b".to_vec();

        assert!(matches!(
            state.filesystem_create_symlink(request),
            Err(MetaRuntimeError::Conflict { .. })
        ));
    }

    #[test]
    fn filesystem_hardlink_unlink_one_retains_inode_and_other_name() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 61);
        let file = create_test_file(&mut state, session.clone(), b"hardlink-source");
        let inode = file.attributes.inode;

        state
            .filesystem_link_entry(pb::FilesystemLinkRequest {
                context: None,
                session: Some(session.clone()),
                operation_id: b"link-hardlink-source".to_vec(),
                operation_digest: b"link-hardlink-source-digest".to_vec(),
                source_inode: inode,
                target_parent: ROOT_INODE,
                target_name: b"hardlink-alias".to_vec(),
                expected_target_revision: None,
                commit_sequence: next_test_commit_sequence(),
                reference_generation: 1,
            })
            .expect("create hardlink");
        assert_eq!(
            state
                .filesystem
                .inode(inode)
                .expect("linked inode")
                .attributes
                .link_count,
            2
        );

        state
            .filesystem_remove_entry(pb::FilesystemRemoveRequest {
                context: None,
                session: Some(session),
                operation_id: b"unlink-hardlink-source".to_vec(),
                operation_digest: b"unlink-hardlink-source-digest".to_vec(),
                parent: ROOT_INODE,
                name: b"hardlink-source".to_vec(),
                kind: pb::FilesystemRemoveKind::File as i32,
                expected_parent_revision: None,
                commit_sequence: next_test_commit_sequence(),
            })
            .expect("unlink one dentry");

        assert!(
            state
                .filesystem
                .lookup(ROOT_INODE, b"hardlink-source")
                .is_none()
        );
        assert_eq!(
            state
                .filesystem
                .lookup(ROOT_INODE, b"hardlink-alias")
                .expect("remaining alias")
                .inode,
            inode
        );
        assert_eq!(
            state
                .filesystem
                .inode(inode)
                .expect("inode retained")
                .attributes
                .link_count,
            1
        );

        let restored = MetaState::new(state.journal);
        assert_eq!(
            restored
                .filesystem
                .lookup(ROOT_INODE, b"hardlink-alias")
                .expect("alias replayed")
                .inode,
            inode
        );
        assert_eq!(
            restored
                .filesystem
                .inode(inode)
                .expect("link count replayed")
                .attributes
                .link_count,
            1
        );
    }

    #[test]
    fn filesystem_rename_same_inode_replace_is_journaled_and_idempotent() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 62);
        let file = create_test_file(&mut state, session.clone(), b"same-inode-source");
        let inode = file.attributes.inode;
        state
            .filesystem_link_entry(pb::FilesystemLinkRequest {
                context: None,
                session: Some(session.clone()),
                operation_id: b"link-same-inode".to_vec(),
                operation_digest: b"link-same-inode-digest".to_vec(),
                source_inode: inode,
                target_parent: ROOT_INODE,
                target_name: b"same-inode-target".to_vec(),
                expected_target_revision: None,
                commit_sequence: next_test_commit_sequence(),
                reference_generation: 1,
            })
            .expect("create alias before no-op rename");

        let request = pb::FilesystemRenameRequest {
            context: None,
            session: Some(session),
            operation_id: b"rename-same-inode".to_vec(),
            operation_digest: b"rename-same-inode-digest".to_vec(),
            source_parent: ROOT_INODE,
            source_name: b"same-inode-source".to_vec(),
            target_parent: ROOT_INODE,
            target_name: b"same-inode-target".to_vec(),
            expected_source_revision: None,
            expected_target_revision: None,
            commit_sequence: next_test_commit_sequence(),
            replace_existing: true,
        };
        let first = state
            .filesystem_rename_entry(request.clone())
            .expect("same inode rename succeeds as no-op");
        let retry = state
            .filesystem_rename_entry(request)
            .expect("same operation retry is idempotent");
        assert_eq!(first.commit_index, retry.commit_index);
        assert!(
            state
                .filesystem_namespace_operations
                .contains_key(b"rename-same-inode".as_slice()),
            "successful no-op still needs durable operation identity"
        );
        assert_eq!(
            state
                .filesystem
                .inode(inode)
                .expect("inode still linked twice")
                .attributes
                .link_count,
            2
        );
    }

    #[test]
    fn filesystem_reference_generation_rejects_delayed_release() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 63);
        let file = create_test_file(&mut state, session.clone(), b"ref-generation");
        let inode = file.attributes.inode;

        state
            .filesystem_acquire_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: Some(session.clone()),
                inode,
                reference_generation: 1,
            })
            .expect("first acquire");
        state
            .filesystem_acquire_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: Some(session.clone()),
                inode,
                reference_generation: 2,
            })
            .expect("newer acquire");
        state
            .filesystem_release_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: Some(session.clone()),
                inode,
                reference_generation: 1,
            })
            .expect("delayed old release is ignored");

        let key = (session.node_id, session.node_epoch, inode);
        assert_eq!(
            state
                .filesystem_inode_references
                .get(&key)
                .map(|(generation, _)| *generation),
            Some(2)
        );
    }

    #[test]
    fn filesystem_orphan_rejects_new_acquire_but_accepts_reconnect_rereport() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let first_session = test_session(&mut state, 67);
        let file = create_test_file(&mut state, first_session.clone(), b"rereport-orphan");
        let inode = file.attributes.inode;
        unlink_test_file(
            &mut state,
            first_session.clone(),
            b"rereport-orphan",
            b"unlink-rereport-orphan",
        );

        assert!(matches!(
            state.filesystem_acquire_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: Some(first_session.clone()),
                inode,
                reference_generation: 2,
            }),
            Err(MetaRuntimeError::NotFound)
        ));

        // 同一 Node 进程与 Meta 重连后获得新 epoch。它仍持有的本地 open handle
        // 通过 heartbeat 批量 renew 重新上报，而不是发起新的 acquire。
        let restarted_session = test_session(&mut state, first_session.node_id);
        assert_ne!(restarted_session.node_epoch, first_session.node_epoch);
        state
            .filesystem_renew_inode_references(pb::FilesystemRenewInodeReferencesRequest {
                context: None,
                session: Some(restarted_session.clone()),
                references: vec![pb::FilesystemInodeReferenceLease {
                    inode,
                    reference_generation: 1,
                }],
            })
            .expect("reconnected Node re-reports its live reference");

        // 模拟旧 epoch 的租约已经过期；新 epoch 的 re-report 仍阻止回收。
        for ((node_id, epoch, referenced_inode), (_, deadline)) in
            &mut state.filesystem_inode_references
        {
            if *node_id == first_session.node_id
                && *epoch == first_session.node_epoch
                && *referenced_inode == inode
            {
                *deadline = Instant::now() - Duration::from_secs(1);
            }
        }
        state.recovery_lease_until = Instant::now();
        assert_eq!(state.reap_filesystem_orphans().expect("reap tick"), 0);
        assert!(state.filesystem.inode(inode).is_some());

        state
            .filesystem_release_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: Some(restarted_session),
                inode,
                reference_generation: 1,
            })
            .expect("release re-reported reference");
        assert_eq!(state.reap_filesystem_orphans().expect("final reap"), 1);
        assert!(state.filesystem.inode(inode).is_none());
    }

    #[test]
    fn filesystem_orphan_reaper_waits_for_reference_and_recovery_grace() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 64);
        let file = create_test_file(&mut state, session.clone(), b"leased-orphan");
        let inode = file.attributes.inode;
        unlink_test_file(
            &mut state,
            session.clone(),
            b"leased-orphan",
            b"unlink-leased",
        );

        // create 返回 entry 时已在同一个 Meta actor turn 建立 generation=1 引用。
        state.recovery_lease_until = Instant::now();
        assert_eq!(state.reap_filesystem_orphans().expect("reap tick"), 0);
        assert!(state.filesystem.inode(inode).is_some());

        state
            .filesystem_release_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: Some(session),
                inode,
                reference_generation: 1,
            })
            .expect("release entry reference");
        state.recovery_lease_until = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            state
                .reap_filesystem_orphans()
                .expect("grace-protected tick"),
            0
        );
        assert!(state.filesystem.inode(inode).is_some());

        state.recovery_lease_until = Instant::now();
        assert_eq!(state.reap_filesystem_orphans().expect("reap orphan"), 1);
        assert!(state.filesystem.inode(inode).is_none());
    }

    #[test]
    fn filesystem_orphan_reaper_tombstones_content_and_replays_atomically() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 65);
        let file = create_test_file(&mut state, session.clone(), b"content-orphan");
        let inode = file.attributes.inode;
        state
            .filesystem_commit_version(filesystem_commit_request_for(
                session.clone(),
                &file,
                b"content-orphan-block",
                b"commit-content-orphan",
            ))
            .expect("commit file content");
        let key = filesystem_object_key(inode);
        unlink_test_file(
            &mut state,
            session.clone(),
            b"content-orphan",
            b"unlink-content-orphan",
        );
        state
            .filesystem_release_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: Some(session),
                inode,
                reference_generation: 1,
            })
            .expect("release entry reference");
        state.recovery_lease_until = Instant::now();

        assert_eq!(state.reap_filesystem_orphans().expect("reap orphan"), 1);
        assert!(state.filesystem.inode(inode).is_none());
        assert_eq!(
            state
                .versions
                .get(&key)
                .and_then(|versions| versions.last_key_value())
                .map(|(_, layout)| layout.kind),
            Some(pb::VersionKind::Tombstone as i32)
        );

        let restored = MetaState::new(state.journal);
        assert!(restored.filesystem.inode(inode).is_none());
        assert_eq!(
            restored
                .versions
                .get(&key)
                .and_then(|versions| versions.last_key_value())
                .map(|(_, layout)| layout.kind),
            Some(pb::VersionKind::Tombstone as i32),
            "WAL replay must never restore only one side of inode/object deletion"
        );
    }

    #[test]
    fn filesystem_orphan_reaper_append_failure_keeps_inode_and_object_current() {
        let journal = SharedFailingAppendJournal::default();
        let fail_append = Arc::clone(&journal.fail_append);
        let mut state = MetaState::new(Box::new(journal));
        let session = test_session(&mut state, 66);
        let file = create_test_file(&mut state, session.clone(), b"failed-reap");
        let inode = file.attributes.inode;
        state
            .filesystem_commit_version(filesystem_commit_request_for(
                session.clone(),
                &file,
                b"failed-reap-block",
                b"commit-failed-reap",
            ))
            .expect("commit file content");
        let key = filesystem_object_key(inode);
        unlink_test_file(
            &mut state,
            session.clone(),
            b"failed-reap",
            b"unlink-failed-reap",
        );
        state
            .filesystem_release_inode_reference(pb::FilesystemInodeReferenceRequest {
                context: None,
                session: Some(session),
                inode,
                reference_generation: 1,
            })
            .expect("release entry reference");
        state.recovery_lease_until = Instant::now();
        fail_append.store(true, Ordering::Relaxed);

        assert!(matches!(
            state.reap_filesystem_orphans(),
            Err(MetaRuntimeError::JournalAppendFailed(_))
        ));
        assert_eq!(
            state
                .filesystem
                .inode(inode)
                .expect("append failure retains orphan")
                .attributes
                .link_count,
            0
        );
        assert_eq!(
            state
                .versions
                .get(&key)
                .and_then(|versions| versions.last_key_value())
                .map(|(_, layout)| layout.kind),
            Some(pb::VersionKind::Value as i32)
        );
    }

    #[test]
    fn filesystem_orphan_reap_survives_real_local_wal_reopen() {
        let directory = std::env::temp_dir().join(format!(
            "dms-filesystem-orphan-restart-{}",
            uuid::Uuid::new_v4()
        ));
        let (inode, key) = {
            let journal = LocalWalJournal::open(&directory).expect("open local WAL");
            let mut state = MetaState::new(Box::new(journal));
            let session = test_session(&mut state, 68);
            let file = create_test_file(&mut state, session.clone(), b"wal-orphan");
            let inode = file.attributes.inode;
            state
                .filesystem_commit_version(filesystem_commit_request_for(
                    session.clone(),
                    &file,
                    b"wal-orphan-block",
                    b"commit-wal-orphan",
                ))
                .expect("commit file content");
            let key = filesystem_object_key(inode);
            unlink_test_file(
                &mut state,
                session.clone(),
                b"wal-orphan",
                b"unlink-wal-orphan",
            );
            state
                .filesystem_release_inode_reference(pb::FilesystemInodeReferenceRequest {
                    context: None,
                    session: Some(session),
                    inode,
                    reference_generation: 1,
                })
                .expect("release entry reference");
            state.recovery_lease_until = Instant::now();
            assert_eq!(state.reap_filesystem_orphans().expect("reap orphan"), 1);
            (inode, key)
        };

        let journal = LocalWalJournal::open(&directory).expect("reopen local WAL");
        let restored = MetaState::new(Box::new(journal));
        assert!(restored.filesystem.inode(inode).is_none());
        assert_eq!(
            restored
                .versions
                .get(&key)
                .and_then(|versions| versions.last_key_value())
                .map(|(_, layout)| layout.kind),
            Some(pb::VersionKind::Tombstone as i32)
        );
        fs::remove_dir_all(directory).expect("remove test WAL");
    }

    #[tokio::test]
    async fn filesystem_namespace_reply_waits_for_remote_directory_revoke_ack() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 45);
        let reader = test_session(&mut state, 46);
        let (sender, mut events) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("reader watch");

        let request = filesystem_create_request_for(
            writer.clone(),
            ROOT_INODE,
            b"visible-after-ack",
            pb::FilesystemInodeKind::RegularFile,
            b"namespace-visibility-create",
        );
        let operation_id = request.operation_id.clone();
        let response = state
            .filesystem_create_inode(request)
            .expect("durable namespace create");
        let dispatch =
            state.dispatch_filesystem_namespace(writer.node_id, operation_id.clone(), response);
        assert_eq!(dispatch.waiting_nodes, HashSet::from([reader.node_id]));
        let (reply, mut completion) = oneshot::channel();
        state.register_pending_filesystem_create(dispatch, reply);
        assert!(matches!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        let event = events.try_recv().expect("directory revoke event");
        state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("reader ack");
        assert!(completion.await.expect("reply channel").is_ok());
        assert_eq!(
            state
                .filesystem_namespace_operations
                .get(&operation_id)
                .expect("stored operation")
                .visibility_cursor,
            None
        );
    }

    #[tokio::test]
    async fn filesystem_namespace_disconnected_watch_waits_for_lease_expiry() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 47);
        let reader = test_session(&mut state, 48);
        let (sender, receiver) = mpsc::channel(1);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("reader watch");
        drop(receiver);

        let request = filesystem_create_request_for(
            writer.clone(),
            ROOT_INODE,
            b"visible-after-expiry",
            pb::FilesystemInodeKind::RegularFile,
            b"namespace-disconnected-watch",
        );
        let operation_id = request.operation_id.clone();
        let response = state
            .filesystem_create_inode(request)
            .expect("durable namespace create");
        let dispatch = state.dispatch_filesystem_namespace(writer.node_id, operation_id, response);
        assert_eq!(dispatch.waiting_nodes, HashSet::from([reader.node_id]));
        let (reply, mut completion) = oneshot::channel();
        state.register_pending_filesystem_create(dispatch, reply);
        assert!(matches!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        expire_session(&mut state, reader.node_id);
        state
            .retire_expired_sessions()
            .expect("expire disconnected reader");
        assert!(completion.await.expect("reply channel").is_ok());
    }

    #[tokio::test]
    async fn filesystem_namespace_retry_after_snapshot_keeps_visibility_barrier() {
        let mut before_restart = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut before_restart, 55);
        let reader = test_session(&mut before_restart, 56);
        let request = filesystem_create_request_for(
            writer.clone(),
            ROOT_INODE,
            b"retry-after-restart",
            pb::FilesystemInodeKind::RegularFile,
            b"namespace-restart-create",
        );
        let operation_id = request.operation_id.clone();
        before_restart
            .filesystem_create_inode(request.clone())
            .expect("durable create before restart");
        let snapshot = before_restart.snapshot();

        let mut restored = MetaState::new(Box::<InMemoryJournal>::default());
        restored.restore_snapshot(snapshot);
        restored
            .heartbeat(
                writer.session_id.clone(),
                writer.node_id,
                writer.node_epoch,
                0,
            )
            .expect("writer heartbeat");
        restored
            .heartbeat(
                reader.session_id.clone(),
                reader.node_id,
                reader.node_epoch,
                0,
            )
            .expect("reader heartbeat");
        let (sender, mut events) = mpsc::channel(4);
        restored
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("reader watch reconnect");

        let response = restored
            .filesystem_create_inode(request)
            .expect("idempotent retry");
        let dispatch =
            restored.dispatch_filesystem_namespace(writer.node_id, operation_id, response);
        assert_eq!(dispatch.waiting_nodes, HashSet::from([reader.node_id]));
        assert_eq!(
            dispatch.waiting_prior_lease_nodes,
            HashSet::from([writer.node_id, reader.node_id])
        );
        let (reply, mut completion) = oneshot::channel();
        restored.register_pending_filesystem_create(dispatch, reply);

        let event = events.try_recv().expect("replayed directory revoke");
        restored
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("reader ack");
        assert!(matches!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        for deadline in restored.prior_lease_deadlines.values_mut() {
            *deadline = Instant::now();
        }
        restored
            .retire_expired_sessions()
            .expect("expire prior leases");
        assert!(completion.await.expect("reply channel").is_ok());
    }

    #[test]
    fn filesystem_checkpoint_plus_wal_tail_replays_one_namespace() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 57);
        state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"before-checkpoint",
                pb::FilesystemInodeKind::RegularFile,
                b"before-checkpoint-create",
            ))
            .expect("create before checkpoint");
        let snapshot = state.snapshot();
        state
            .journal
            .save_snapshot(snapshot)
            .expect("save namespace checkpoint");
        state
            .filesystem_create_inode(filesystem_create_request_for(
                session,
                ROOT_INODE,
                b"after-checkpoint",
                pb::FilesystemInodeKind::RegularFile,
                b"after-checkpoint-create",
            ))
            .expect("append WAL tail");

        let restored = MetaState::new(state.journal);
        assert!(
            restored
                .filesystem
                .lookup(ROOT_INODE, b"before-checkpoint")
                .is_some()
        );
        assert!(
            restored
                .filesystem
                .lookup(ROOT_INODE, b"after-checkpoint")
                .is_some()
        );
    }

    #[tokio::test]
    async fn concurrent_namespace_create_has_one_atomic_winner_without_client_cas() {
        let handle = MetaHandle::spawn();
        let grant = handle
            .open_node_session(47, "http://127.0.0.1:19047".into(), true)
            .await
            .expect("node session");
        let session = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        };
        let first = filesystem_create_request_for(
            session.clone(),
            ROOT_INODE,
            b"same-name",
            pb::FilesystemInodeKind::RegularFile,
            b"concurrent-create-a",
        );
        let second = filesystem_create_request_for(
            session,
            ROOT_INODE,
            b"same-name",
            pb::FilesystemInodeKind::RegularFile,
            b"concurrent-create-b",
        );

        let (a, b) = tokio::join!(
            handle.filesystem_create_inode(first),
            handle.filesystem_create_inode(second)
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let error = a.err().or_else(|| b.err()).expect("one loser");
        assert!(matches!(error, MetaRuntimeError::AlreadyExists));
    }

    #[tokio::test]
    async fn concurrent_hardlinks_preserve_every_link_count_increment() {
        let handle = MetaHandle::spawn();
        let grant = handle
            .open_node_session(68, "http://127.0.0.1:19068".into(), true)
            .await
            .expect("node session");
        let session = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        };
        let created = handle
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"concurrent-link-source",
                pb::FilesystemInodeKind::RegularFile,
                b"concurrent-link-create",
            ))
            .await
            .expect("create source");
        let inode = created
            .resolved
            .expect("source resolution")
            .inode
            .expect("source inode")
            .attributes
            .expect("source attrs")
            .inode;
        let request = |index: u8| pb::FilesystemLinkRequest {
            context: None,
            session: Some(session.clone()),
            operation_id: format!("concurrent-link-{index}").into_bytes(),
            operation_digest: format!("concurrent-link-{index}-digest").into_bytes(),
            source_inode: inode,
            target_parent: ROOT_INODE,
            target_name: format!("concurrent-link-alias-{index}").into_bytes(),
            expected_target_revision: None,
            commit_sequence: next_test_commit_sequence(),
            reference_generation: u64::from(index) + 2,
        };

        let (a, b, c, d) = tokio::join!(
            handle.filesystem_link_entry(request(0)),
            handle.filesystem_link_entry(request(1)),
            handle.filesystem_link_entry(request(2)),
            handle.filesystem_link_entry(request(3)),
        );
        for result in [a, b, c, d] {
            result.expect("unique concurrent hardlink");
        }

        let resolved = handle
            .filesystem_get_inode(pb::FilesystemGetInodeRequest {
                context: None,
                session: Some(session),
                inode,
            })
            .await
            .expect("resolve linked inode")
            .resolved
            .expect("linked resolution")
            .inode
            .expect("linked inode")
            .attributes
            .expect("linked attrs");
        assert_eq!(resolved.link_count, 5);
    }

    #[test]
    fn filesystem_remove_rejects_non_empty_directory() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 47);
        let directory = state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"parent",
                pb::FilesystemInodeKind::Directory,
                b"mkdir-parent",
            ))
            .expect("mkdir")
            .resolved
            .expect("directory resolution")
            .inode
            .expect("directory inode");
        let directory_inode = directory.attributes.as_ref().expect("attrs").inode;
        state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                directory_inode,
                b"child",
                pb::FilesystemInodeKind::RegularFile,
                b"create-child",
            ))
            .expect("create child");

        let result = state.filesystem_remove_entry(pb::FilesystemRemoveRequest {
            context: None,
            session: Some(session),
            operation_id: b"rmdir-parent".to_vec(),
            operation_digest: b"rmdir-parent-digest".to_vec(),
            parent: ROOT_INODE,
            name: b"parent".to_vec(),
            kind: pb::FilesystemRemoveKind::Directory as i32,
            expected_parent_revision: None,
            commit_sequence: next_test_commit_sequence(),
        });
        assert!(matches!(result, Err(MetaRuntimeError::DirectoryNotEmpty)));
    }

    #[test]
    fn filesystem_rename_rejects_directory_cycles_before_journal_append() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 48);
        let parent = state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"parent",
                pb::FilesystemInodeKind::Directory,
                b"mkdir-cycle-parent",
            ))
            .expect("create parent")
            .resolved
            .expect("parent resolution")
            .inode
            .expect("parent inode")
            .attributes
            .expect("parent attrs")
            .inode;
        let child = state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                parent,
                b"child",
                pb::FilesystemInodeKind::Directory,
                b"mkdir-cycle-child",
            ))
            .expect("create child")
            .resolved
            .expect("child resolution")
            .inode
            .expect("child inode")
            .attributes
            .expect("child attrs")
            .inode;
        let journal_index = state.journal.last_index();

        let result = state.filesystem_rename_entry(pb::FilesystemRenameRequest {
            context: None,
            session: Some(session),
            operation_id: b"rename-cycle".to_vec(),
            operation_digest: b"rename-cycle-digest".to_vec(),
            source_parent: ROOT_INODE,
            source_name: b"parent".to_vec(),
            target_parent: child,
            target_name: b"parent".to_vec(),
            expected_source_revision: None,
            expected_target_revision: None,
            commit_sequence: next_test_commit_sequence(),
            replace_existing: false,
        });

        assert!(matches!(result, Err(MetaRuntimeError::InvalidArgument(_))));
        assert_eq!(state.journal.last_index(), journal_index);
        assert!(state.filesystem.lookup(ROOT_INODE, b"parent").is_some());
        assert!(state.filesystem.lookup(parent, b"child").is_some());
        let restored = MetaState::new(state.journal);
        assert!(restored.filesystem.lookup(ROOT_INODE, b"parent").is_some());
        assert!(restored.filesystem.lookup(parent, b"child").is_some());
    }

    #[test]
    fn filesystem_rename_enforces_replace_type_matrix_and_replays_directory_replace() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 49);
        state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"source-file",
                pb::FilesystemInodeKind::RegularFile,
                b"create-source-file",
            ))
            .expect("create source file");
        state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"target-dir",
                pb::FilesystemInodeKind::Directory,
                b"create-target-dir",
            ))
            .expect("create target dir");
        let file_over_directory = state.filesystem_rename_entry(pb::FilesystemRenameRequest {
            context: None,
            session: Some(session.clone()),
            operation_id: b"file-over-dir".to_vec(),
            operation_digest: b"file-over-dir-digest".to_vec(),
            source_parent: ROOT_INODE,
            source_name: b"source-file".to_vec(),
            target_parent: ROOT_INODE,
            target_name: b"target-dir".to_vec(),
            expected_source_revision: None,
            expected_target_revision: None,
            commit_sequence: next_test_commit_sequence(),
            replace_existing: true,
        });
        assert!(matches!(
            file_over_directory,
            Err(MetaRuntimeError::IsDirectory)
        ));

        state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"source-dir",
                pb::FilesystemInodeKind::Directory,
                b"create-source-dir",
            ))
            .expect("create source dir");
        state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"target-file",
                pb::FilesystemInodeKind::RegularFile,
                b"create-target-file",
            ))
            .expect("create target file");
        let directory_over_file = state.filesystem_rename_entry(pb::FilesystemRenameRequest {
            context: None,
            session: Some(session.clone()),
            operation_id: b"dir-over-file".to_vec(),
            operation_digest: b"dir-over-file-digest".to_vec(),
            source_parent: ROOT_INODE,
            source_name: b"source-dir".to_vec(),
            target_parent: ROOT_INODE,
            target_name: b"target-file".to_vec(),
            expected_source_revision: None,
            expected_target_revision: None,
            commit_sequence: next_test_commit_sequence(),
            replace_existing: true,
        });
        assert!(matches!(
            directory_over_file,
            Err(MetaRuntimeError::NotDirectory)
        ));

        state
            .filesystem_create_inode(filesystem_create_request_for(
                session.clone(),
                ROOT_INODE,
                b"empty-replacement",
                pb::FilesystemInodeKind::Directory,
                b"create-empty-replacement",
            ))
            .expect("create empty replacement directory");
        state
            .filesystem_rename_entry(pb::FilesystemRenameRequest {
                context: None,
                session: Some(session),
                operation_id: b"dir-over-empty-dir".to_vec(),
                operation_digest: b"dir-over-empty-dir-digest".to_vec(),
                source_parent: ROOT_INODE,
                source_name: b"source-dir".to_vec(),
                target_parent: ROOT_INODE,
                target_name: b"empty-replacement".to_vec(),
                expected_source_revision: None,
                expected_target_revision: None,
                commit_sequence: next_test_commit_sequence(),
                replace_existing: true,
            })
            .expect("directory may replace an empty directory");

        assert!(state.filesystem.lookup(ROOT_INODE, b"source-dir").is_none());
        assert!(
            state
                .filesystem
                .lookup(ROOT_INODE, b"empty-replacement")
                .is_some()
        );
        let restored = MetaState::new(state.journal);
        assert!(
            restored
                .filesystem
                .lookup(ROOT_INODE, b"source-dir")
                .is_none()
        );
        assert!(
            restored
                .filesystem
                .lookup(ROOT_INODE, b"empty-replacement")
                .is_some()
        );
    }

    #[test]
    fn default_checkpoint_batches_records_without_losing_replayable_state() {
        let policy = MetaCheckpointPolicy::default();
        assert_eq!(policy.every_records, 4096);
        assert!(!policy.should_checkpoint(4095, 0));
        assert!(policy.should_checkpoint(4096, 0));
        assert!(!policy.should_checkpoint(8191, 4096));

        // 真实提交超过旧阈值 4：不再反复生成全量快照，但恢复资料不能消失。
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        for index in 0..16 {
            state
                .commit_version(value_commit_request_for(
                    session.clone(),
                    format!("key-{index}").into_bytes(),
                    format!("block-{index}").into_bytes(),
                    format!("op-{index}").into_bytes(),
                    format!("digest-{index}").into_bytes(),
                ))
                .unwrap();
        }
        assert!(state.journal.load_snapshot().unwrap().is_none());
        assert_eq!(state.journal.load_after(0).unwrap().len(), 17);
        let restored = MetaState::new(state.journal);
        assert_eq!(restored.last_applied_index, 17);
        for index in 0..16 {
            assert!(
                restored
                    .versions
                    .contains_key(format!("key-{index}").as_bytes())
            );
            assert!(
                restored
                    .operations
                    .contains_key(format!("op-{index}").as_bytes())
            );
        }
    }

    #[test]
    #[ignore = "manual cardinality timing; run release with --ignored --nocapture"]
    fn meta_metrics_cardinality_growth_curve() {
        for count in [100_usize, 1_000, 10_000] {
            let registry = dms_metrics::registry();
            let metrics = MetaMetrics::register(&registry).unwrap();
            let mut state = MetaState::try_new(
                Box::<InMemoryJournal>::default(),
                MetaCheckpointPolicy { every_records: 0 },
                MetaRetentionPolicy::default(),
                metrics,
            )
            .unwrap();
            let session = test_session(&mut state, 7);
            // 建表不计时，使用真实提交/解析路径；此实验只隔离状态基数成本，不代表网络吞吐。
            for index in 0..count {
                let id = index.to_le_bytes().to_vec();
                state
                    .commit_version(value_commit_request_for(
                        session.clone(),
                        id.clone(),
                        id.clone(),
                        id.clone(),
                        id,
                    ))
                    .unwrap();
            }
            let request = pb::ResolveObjectRequest {
                context: None,
                session: Some(session),
                key: Some(pb::Key {
                    value: (count - 1).to_le_bytes().to_vec(),
                }),
                selector: None,
                range: None,
                cache_current: false,
            };
            let started = Instant::now();
            for _ in 0..10_000 {
                std::hint::black_box(state.resolve_object(request.clone()).unwrap());
            }
            let resolve_ns = started.elapsed().as_nanos() / 10_000;
            let started = Instant::now();
            for _ in 0..100 {
                state.refresh_metrics();
            }
            let refresh_ns = started.elapsed().as_nanos() / 100;
            let text = dms_metrics::encode_text(&registry).unwrap();
            for kind in ["keys", "versions", "replicas"] {
                assert!(
                    text.contains(&format!(
                        "dms_meta_state_items{{type=\"{kind}\"}} {count}\n"
                    )),
                    "incorrect {kind} cardinality: {text}"
                );
            }
            // 恢复检查不计入上面的耗时；防止只降低刷新频率，却遗漏启动后的正确计数。
            let mut journal = InMemoryJournal::default();
            journal.save_snapshot(state.snapshot()).unwrap();
            let restored_registry = dms_metrics::registry();
            let _restored = MetaState::try_new(
                Box::new(journal),
                MetaCheckpointPolicy { every_records: 0 },
                MetaRetentionPolicy::default(),
                MetaMetrics::register(&restored_registry).unwrap(),
            )
            .unwrap();
            let restored_text = dms_metrics::encode_text(&restored_registry).unwrap();
            for kind in ["keys", "versions", "replicas"] {
                assert!(
                    restored_text.contains(&format!(
                        "dms_meta_state_items{{type=\"{kind}\"}} {count}\n"
                    )),
                    "incorrect restored {kind} cardinality"
                );
            }
            println!(
                "meta_cardinality count={count} resolve_ns_per_op={resolve_ns} metrics_refresh_ns_per_snapshot={refresh_ns}"
            );
        }
    }

    #[test]
    fn restored_source_cannot_skip_pre_snapshot_incarnation_lease() {
        let mut original = MetaState::new(Box::<InMemoryJournal>::default());
        test_session(&mut original, 7);
        let source = test_session(&mut original, 7);
        seed_existing_key(&mut original, source.clone(), b"checkpoint/latest");
        assert!(original.prior_lease_deadlines.contains_key(&7));
        let mut journal = InMemoryJournal::default();
        journal.save_snapshot(original.snapshot()).unwrap();
        let mut restored = MetaState::new(Box::new(journal));
        let dispatch = restored
            .dispatch_commit_version(value_commit_request(source, b"restored-source".to_vec()))
            .unwrap();
        assert!(
            !dispatch.waiting_nodes.contains(&7),
            "source 只跳过本次 Watch ACK，不能把旧 lease 混进 ACK 集合"
        );
        assert!(
            dispatch.waiting_prior_lease_nodes.contains(&7),
            "source exclusion cannot erase recovered old cache leases"
        );
        let (reply, mut completion) = oneshot::channel();
        restored.register_pending_commit(dispatch, reply);
        assert!(matches!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        restored.recovery_lease_until = Instant::now();
        for until in restored.prior_lease_deadlines.values_mut() {
            *until = Instant::now();
        }
        restored.retire_expired_sessions().unwrap();
        assert!(completion.try_recv().unwrap().is_ok());
    }

    #[test]
    fn same_incarnation_heartbeat_after_restore_waits_for_prior_lease_not_source_ack() {
        let mut original = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut original, 7);
        seed_existing_key(&mut original, writer.clone(), b"checkpoint/latest");

        let mut restored = MetaState::new(original.journal);
        assert!(restored.prior_lease_deadlines.contains_key(&writer.node_id));
        restored
            .heartbeat(
                writer.session_id.clone(),
                writer.node_id,
                writer.node_epoch,
                0,
            )
            .expect("same incarnation heartbeat");
        assert!(
            restored.prior_lease_deadlines.contains_key(&writer.node_id),
            "同 incarnation heartbeat 只证明 Node 活着，不能证明恢复前 Client cache 租约已失效"
        );

        let dispatch = restored
            .dispatch_commit_version(value_commit_request(
                writer.clone(),
                b"same-incarnation-after-restore".to_vec(),
            ))
            .unwrap();
        assert!(dispatch.waiting_nodes.is_empty());
        assert_eq!(
            dispatch.waiting_prior_lease_nodes,
            HashSet::from([writer.node_id])
        );
        let expected_version = dispatch.response.version;
        let (reply, mut completion) = oneshot::channel();
        restored.register_pending_commit(dispatch, reply);
        assert!(matches!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        for until in restored.prior_lease_deadlines.values_mut() {
            *until = Instant::now();
        }
        restored.retire_expired_sessions().unwrap();
        assert_eq!(
            completion.try_recv().unwrap().unwrap().version,
            expected_version
        );
    }

    #[tokio::test]
    async fn remote_ack_before_prior_lease_expiry_does_not_complete_commit() {
        let mut original = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut original, 7);
        let reader = test_session(&mut original, 8);
        seed_existing_key(&mut original, writer.clone(), b"checkpoint/latest");

        let mut restored = MetaState::new(original.journal);
        let dispatch = restored
            .dispatch_commit_version(value_commit_request(
                writer,
                b"remote-ack-before-prior-lease".to_vec(),
            ))
            .unwrap();
        assert_eq!(dispatch.waiting_nodes, HashSet::from([reader.node_id]));
        assert_eq!(dispatch.waiting_prior_lease_nodes, HashSet::from([7, 8]));
        let expected_version = dispatch.response.version;
        let event = restored.events.last().expect("invalidation").clone();
        let (reply, mut completion) = oneshot::channel();
        restored.register_pending_commit(dispatch, reply);

        restored
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("remote ack");
        assert!(
            matches!(
                completion.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "远端 ACK 只满足 Watch 义务，不能提前跳过恢复前旧 lease"
        );

        for until in restored.prior_lease_deadlines.values_mut() {
            *until = Instant::now();
        }
        restored.retire_expired_sessions().unwrap();
        assert_eq!(completion.await.unwrap().unwrap().version, expected_version);
    }

    #[tokio::test]
    async fn prior_lease_expiry_before_remote_ack_does_not_complete_commit() {
        let mut original = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut original, 7);
        let reader = test_session(&mut original, 8);
        seed_existing_key(&mut original, writer.clone(), b"checkpoint/latest");

        let mut restored = MetaState::new(original.journal);
        let dispatch = restored
            .dispatch_commit_version(value_commit_request(
                writer,
                b"prior-lease-before-remote-ack".to_vec(),
            ))
            .unwrap();
        assert_eq!(dispatch.waiting_nodes, HashSet::from([reader.node_id]));
        assert_eq!(dispatch.waiting_prior_lease_nodes, HashSet::from([7, 8]));
        let expected_version = dispatch.response.version;
        let event = restored.events.last().expect("invalidation").clone();
        let (reply, mut completion) = oneshot::channel();
        restored.register_pending_commit(dispatch, reply);

        for until in restored.prior_lease_deadlines.values_mut() {
            *until = Instant::now();
        }
        restored.retire_expired_sessions().unwrap();
        assert!(
            matches!(
                completion.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "旧 lease 到期只满足恢复 fence，不能代替远端 Watch ACK"
        );

        restored
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("remote ack");
        assert_eq!(completion.await.unwrap().unwrap().version, expected_version);
    }

    #[tokio::test]
    async fn expired_node_leases_release_pending_commit_and_event_retention() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"checkpoint/latest");
        let (sender, receiver) = mpsc::channel(1);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .unwrap();
        drop(receiver);
        let dispatch = state
            .dispatch_commit_version(value_commit_request(writer, b"lease-retire".to_vec()))
            .unwrap();
        let (reply, mut response) = oneshot::channel();
        state.register_pending_commit(dispatch, reply);
        state.retire_expired_sessions().unwrap();
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        for session in state.sessions.values_mut() {
            session.last_heartbeat =
                Some(Instant::now() - DEFAULT_NODE_LEASE_TTL - Duration::from_secs(1));
        }
        let expected_version = state
            .operations
            .get(b"lease-retire".as_slice())
            .expect("pending operation")
            .result
            .version;
        state.retire_expired_sessions().unwrap();
        assert_eq!(response.await.unwrap().unwrap().version, expected_version);
        assert!(state.events.is_empty());
        assert!(state.watchers.is_empty());
    }

    #[test]
    fn new_incarnation_ack_cannot_retire_old_incarnation_cache_lease() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        test_session(&mut state, 8);
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"checkpoint/latest");
        let dispatch = state
            .dispatch_commit_version(value_commit_request(writer, b"incarnation-lease".to_vec()))
            .unwrap();
        let (reply, mut completion) = oneshot::channel();
        state.register_pending_commit(dispatch, reply);
        let event = state.events.last().unwrap().clone();
        state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader.clone()),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".into(),
                detail: None,
            })
            .unwrap();
        assert!(matches!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        state.prior_lease_deadlines.insert(8, Instant::now());
        state.retire_expired_sessions().unwrap();
        assert!(completion.try_recv().unwrap().is_ok());
    }

    #[tokio::test]
    async fn same_key_concurrent_cas_has_exactly_one_winner() {
        let handle = MetaHandle::spawn();
        let grant = handle
            .open_node_session(7, "http://127.0.0.1:19007".into(), true)
            .await
            .unwrap();
        let session = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: 7,
            node_epoch: grant.node_epoch,
        };
        let mut first = value_commit_request(session.clone(), b"cas-first".to_vec());
        first.condition = "if-absent".into();
        let mut second = value_commit_request(session, b"cas-second".to_vec());
        second.condition = "if-absent".into();
        let (a, b) = tokio::join!(handle.commit_version(first), handle.commit_version(second));
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let error = if let Err(error) = a {
            error
        } else {
            b.unwrap_err()
        };
        assert!(matches!(error, MetaRuntimeError::Conflict { .. }));
    }

    #[test]
    fn first_publish_new_key_skips_invalidation_and_ack_barrier() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        let (sender, mut receiver) = mpsc::channel(1);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("reader watch");

        let dispatch = state
            .dispatch_commit_version(value_commit_request_for(
                writer,
                b"new/key".to_vec(),
                b"new-key-block".to_vec(),
                b"new-key-op".to_vec(),
                b"new-key-digest".to_vec(),
            ))
            .expect("new key commit");

        assert_eq!(
            dispatch.event_cursor, None,
            "从未存在的 key 没有旧 Current cache，不需要发布失效事件"
        );
        assert!(
            dispatch.waiting_nodes.is_empty(),
            "首次发布不能把没有旧缓存的 Node 拉进前台 ACK 屏障"
        );
        assert!(state.events.is_empty());
        assert_eq!(
            state.event_high_watermark, 1,
            "首次发布保留 cursor gap，避免旧 ACK 与后续事件 cursor 碰撞"
        );
        assert!(
            receiver.try_recv().is_err(),
            "watcher 不应收到新 key 失效事件"
        );

        let (reply, mut completion) = oneshot::channel();
        let expected_response = dispatch.response;
        state.register_pending_commit(dispatch, reply);
        assert_eq!(
            completion
                .try_recv()
                .expect("new key reply should complete immediately")
                .expect("new key commit result"),
            expected_response
        );
    }

    #[test]
    fn missing_resolve_does_not_create_negative_cache_obligation() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);

        assert!(matches!(
            state.resolve_object(resolve_current_request(
                reader,
                b"miss-then-create".to_vec(),
                true,
            )),
            Err(MetaRuntimeError::NotFound)
        ));

        let dispatch = state
            .dispatch_commit_version(value_commit_request_for(
                writer,
                b"miss-then-create".to_vec(),
                b"miss-then-create-block".to_vec(),
                b"miss-then-create-op".to_vec(),
                b"miss-then-create-digest".to_vec(),
            ))
            .expect("first publish after miss");
        assert_eq!(
            dispatch.event_cursor, None,
            "Resolve miss 当前不落负缓存，因此首次发布不需要撤销 reader 的旧视图"
        );
        assert!(dispatch.waiting_nodes.is_empty());
        assert!(state.events.is_empty());
        assert_eq!(state.event_high_watermark, 1);
    }

    #[test]
    fn watch_lag_metrics_ignore_cursor_gaps_without_events() {
        let registry = dms_metrics::registry();
        let mut state = MetaState::try_new(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 0 },
            MetaRetentionPolicy::default(),
            MetaMetrics::register(&registry).expect("metrics"),
        )
        .expect("meta state");
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);

        state
            .dispatch_commit_version(value_commit_request_for(
                writer,
                b"new-key-only".to_vec(),
                b"new-key-only-block".to_vec(),
                b"new-key-only-op".to_vec(),
                b"new-key-only-digest".to_vec(),
            ))
            .expect("first publish");
        assert_eq!(state.event_high_watermark, 1);
        assert!(state.events.is_empty());

        state.refresh_metrics();
        let text = dms_metrics::encode_text(&registry).expect("metrics text");
        assert!(
            text.contains("dms_meta_watch_lag_events 0\n"),
            "reader {:?} has not ACKed cursor gap, but no real event is pending: {text}",
            reader.session_id,
        );
    }

    #[test]
    fn mixed_batch_waits_for_existing_key_but_not_new_key_in_any_order() {
        run_mixed_batch_waits_for_existing_key_but_not_new_key(false);
        run_mixed_batch_waits_for_existing_key_but_not_new_key(true);
    }

    fn run_mixed_batch_waits_for_existing_key_but_not_new_key(new_key_first: bool) {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"old/key");
        let (sender, mut receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("reader watch");

        let old_entry = batch_entry(b"old/key", b"old-key-block-v2", b"mixed-op-old", "any");
        let new_entry = batch_entry(b"new/key", b"new-key-block-v1", b"mixed-op-new", "any");
        let request = batch_request(
            writer.clone(),
            if new_key_first {
                b"mixed-batch-op-new-first".to_vec()
            } else {
                b"mixed-batch-op-old-first".to_vec()
            },
            if new_key_first {
                vec![new_entry, old_entry]
            } else {
                vec![old_entry, new_entry]
            },
        );
        let operation_ids = request
            .entries
            .iter()
            .map(|entry| entry.operation_id.clone())
            .collect::<Vec<_>>();
        let response = state.commit_batch(request).expect("mixed batch");
        let response_commit_index = response.commit_index;

        assert_eq!(state.events.len(), 1, "只有旧 key 覆盖写需要失效事件");
        let old_visibility = state.operations[b"mixed-op-old".as_slice()].visibility_cursor;
        assert_eq!(
            old_visibility,
            Some(if new_key_first { 3 } else { 2 }),
            "batch 内首次发布新 key 只留下 cursor gap；旧 key 的真实事件位置随顺序变化"
        );
        assert_eq!(
            state.operations[b"mixed-op-new".as_slice()].visibility_cursor,
            None
        );

        let (reply, mut completion) = oneshot::channel();
        state.register_pending_batch(writer.node_id, operation_ids, response, reply);
        assert!(matches!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        let event = receiver.try_recv().expect("old key invalidation");
        state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader.clone()),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("reader acknowledgement");
        assert_eq!(
            completion
                .try_recv()
                .expect("mixed batch should complete after old key ACK")
                .expect("mixed batch result")
                .commit_index,
            response_commit_index
        );
    }

    #[test]
    fn recreate_after_delete_still_invalidates_previous_current() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        let (sender, mut receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("reader watch");
        let seed = seed_existing_key(&mut state, writer.clone(), b"delete/recreate");

        let deleted = state
            .commit_version(tombstone_request_for(
                writer.clone(),
                b"delete/recreate".to_vec(),
                b"delete-recreate-del-op".to_vec(),
                b"delete-recreate-del-digest".to_vec(),
            ))
            .expect("delete existing key");
        assert_eq!(state.event_high_watermark, 2);
        assert_eq!(receiver.try_recv().expect("delete invalidation").cursor, 2);

        let dispatch = state
            .dispatch_commit_version(value_commit_request_for(
                writer,
                b"delete/recreate".to_vec(),
                b"delete-recreate-block".to_vec(),
                b"delete-recreate-put-op".to_vec(),
                b"delete-recreate-put-digest".to_vec(),
            ))
            .expect("recreate deleted key");
        assert_eq!(dispatch.event_cursor, Some(3));
        assert_eq!(dispatch.waiting_nodes, HashSet::from([reader.node_id]));
        assert_eq!(
            state.event_high_watermark, 3,
            "删除墓碑之后重建仍有旧版本历史，不能按首次发布跳过失效"
        );
        let event = receiver.try_recv().expect("recreate invalidation");
        assert_eq!(event.cursor, 3);
        let invalidate = invalidate_event(&event);
        assert_eq!(invalidate.old_version, deleted.version);
        assert_eq!(invalidate.old_version, seed.version + 1);
        assert_eq!(invalidate.minimum_version, dispatch.response.version);
    }

    #[test]
    fn journal_replay_preserves_cursor_gap_event_and_ack() {
        let mut before_restart = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut before_restart, 7);
        let reader = test_session(&mut before_restart, 8);
        let _first = before_restart
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"journal/gap".to_vec(),
                b"journal-gap-block-v1".to_vec(),
                b"journal-gap-op-v1".to_vec(),
                b"journal-gap-digest-v1".to_vec(),
            ))
            .expect("first publish");
        assert_eq!(before_restart.event_high_watermark, 1);
        assert!(before_restart.events.is_empty());

        let dispatch = before_restart
            .dispatch_commit_version(value_commit_request_for(
                writer,
                b"journal/gap".to_vec(),
                b"journal-gap-block-v2".to_vec(),
                b"journal-gap-op-v2".to_vec(),
                b"journal-gap-digest-v2".to_vec(),
            ))
            .expect("overwrite");
        assert_eq!(dispatch.event_cursor, Some(2));
        assert_eq!(dispatch.waiting_nodes, HashSet::from([reader.node_id]));
        let event = before_restart
            .events
            .last()
            .expect("overwrite event")
            .clone();
        before_restart
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader.clone()),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("ack overwrite event");

        let mut restored = MetaState::new(before_restart.journal);
        assert_eq!(restored.event_high_watermark, 2);
        assert_eq!(
            restored
                .sessions
                .get(&8)
                .expect("reader session")
                .last_acked_cursor,
            2,
            "旧 Journal 没有事件决策字段；cursor gap 必须让历史 ACK 仍指向同一个真实事件"
        );
        let (sender, mut receiver) = mpsc::channel(1);
        restored
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader),
                    last_acked_cursor: 2,
                },
                sender,
            )
            .expect("reader watch after restore");
        assert!(
            receiver.try_recv().is_err(),
            "已 ACK cursor 2 的 reader 重连时不应重放覆盖事件；事件物理保留只服务其它未 ACK 会话"
        );
        assert_eq!(
            restored
                .versions
                .get(b"journal/gap".as_slice())
                .and_then(|versions| versions.last_key_value())
                .map(|(_, layout)| layout.version),
            Some(dispatch.response.version)
        );
    }

    #[test]
    fn snapshot_gap_and_tail_overwrite_replay_real_event() {
        let mut before_restart = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut before_restart, 7);
        let reader = test_session(&mut before_restart, 8);
        let first_publish = before_restart
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"snapshot/gap".to_vec(),
                b"snapshot-gap-block-v1".to_vec(),
                b"snapshot-gap-op-v1".to_vec(),
                b"snapshot-gap-digest-v1".to_vec(),
            ))
            .expect("first publish");
        assert_eq!(before_restart.event_high_watermark, 1);
        let snapshot = before_restart.snapshot();
        before_restart
            .journal
            .save_snapshot(snapshot)
            .expect("save gap snapshot");

        let writer_node_id = writer.node_id;
        let dispatch = before_restart
            .dispatch_commit_version(value_commit_request_for(
                writer,
                b"snapshot/gap".to_vec(),
                b"snapshot-gap-block-v2".to_vec(),
                b"snapshot-gap-op-v2".to_vec(),
                b"snapshot-gap-digest-v2".to_vec(),
            ))
            .expect("tail overwrite");

        let mut restored = MetaState::new(before_restart.journal);
        assert_eq!(restored.event_high_watermark, 2);
        assert_eq!(restored.events.len(), 1);
        let event = restored.events.pop().expect("tail event");
        assert_eq!(event.cursor, 2);
        let invalidate = invalidate_event(&event);
        assert_eq!(invalidate.key.as_ref().expect("key").value, b"snapshot/gap");
        assert_eq!(invalidate.old_version, first_publish.version);
        assert_eq!(invalidate.minimum_version, dispatch.response.version);
        let (waiting_nodes, waiting_prior_lease_nodes) =
            restored.visibility_barrier_nodes(writer_node_id, event.cursor);
        assert_eq!(waiting_nodes, HashSet::from([reader.node_id]));
        assert_eq!(
            waiting_prior_lease_nodes,
            HashSet::from([writer_node_id, reader.node_id]),
            "恢复后真实事件仍可作为前台可见性屏障使用；source 不等自己的 ACK，但恢复前 lease fence 仍需保留"
        );
    }

    #[test]
    fn watch_full_or_disconnected_keeps_visibility_obligation() {
        for disconnected in [false, true] {
            let mut state = MetaState::new(Box::<InMemoryJournal>::default());
            let writer = test_session(&mut state, 7);
            let reader = test_session(&mut state, 8);
            seed_existing_key(&mut state, writer.clone(), b"checkpoint/latest");
            let (sender, receiver) = mpsc::channel(1);
            state
                .watch_node_events(
                    pb::WatchNodeEventsRequest {
                        context: None,
                        session: Some(reader),
                        last_acked_cursor: 0,
                    },
                    sender,
                )
                .unwrap();
            if disconnected {
                drop(receiver);
            }
            for index in 0..2 {
                let dispatch = state
                    .dispatch_commit_version(value_commit_request_for(
                        writer.clone(),
                        b"checkpoint/latest".to_vec(),
                        vec![index, b'v'],
                        vec![index, b'o'],
                        vec![index, b'd'],
                    ))
                    .unwrap();
                assert!(
                    dispatch.waiting_nodes.contains(&8),
                    "delivery failure is not a cache revocation proof"
                );
            }
        }
    }

    #[tokio::test]
    async fn watch_replay_larger_than_channel_capacity_progresses() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        // Reader 已经持有有效 session，代表它可能缓存旧版本；断线期间产生的
        // invalidation 才是重连后必须重放的事件。写完后才加入的新 Node 没有旧缓存，
        // 不应依赖历史 invalidation，这也避免让事件保留策略影响本测试。
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"watch/replay");
        for index in 1..=5 {
            let dispatch = state
                .dispatch_commit_version(value_commit_request_for(
                    writer.clone(),
                    b"watch/replay".to_vec(),
                    vec![index],
                    vec![index],
                    vec![index],
                ))
                .unwrap();
            assert!(dispatch.waiting_nodes.contains(&reader.node_id));
        }
        let (sender, mut receiver) = mpsc::channel(1);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("replay registration must not synchronously fill entire channel");
        for cursor in 2..=6 {
            let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(event.cursor, cursor);
            // channel 容量只有 1；消费一个事件后显式推进一次投递，验证 backlog
            // 可以在有界队列上逐步清空，而不是依赖 actor 定时器的调度快慢。
            state.pump_watchers();
        }
    }

    #[test]
    fn meta_runtime_errors_map_to_precise_public_codes() {
        assert_eq!(
            MetaRuntimeError::InvalidArgument("missing key".to_string())
                .into_dms_error()
                .code(),
            dms_error::META_CATALOG_INVALID_REQUEST
        );
        assert_eq!(
            MetaRuntimeError::NotFound.into_dms_error().code(),
            dms_error::META_CATALOG_NOT_FOUND
        );
        assert_eq!(
            MetaRuntimeError::Conflict {
                expected: Some(1),
                actual: 2,
            }
            .into_dms_error()
            .code(),
            dms_error::META_CATALOG_VERSION_CONFLICT
        );
        assert_eq!(
            MetaRuntimeError::UnknownSession.into_dms_error().code(),
            dms_error::META_SESSION_UNKNOWN
        );
    }

    #[test]
    fn journal_errors_keep_append_and_recovery_boundaries_distinct() {
        assert_eq!(
            map_journal_append_error(JournalError::Unavailable("disk full"))
                .into_dms_error()
                .code(),
            dms_error::META_JOURNAL_APPEND_FAILED
        );
        assert_eq!(
            map_journal_error(JournalError::Unavailable("backend down"))
                .into_dms_error()
                .code(),
            dms_error::META_JOURNAL_UNAVAILABLE
        );
        assert_eq!(
            map_journal_error(JournalError::InvalidSnapshot("bad frame"))
                .into_dms_error()
                .code(),
            dms_error::META_JOURNAL_CORRUPT
        );
    }

    #[test]
    fn journal_replay_reconstructs_current_and_operation_result() {
        let mut journal = InMemoryJournal::default();
        journal
            .append(JournalRecord::VersionCommitted {
                key: b"model/latest".to_vec(),
                layout: pb::VersionLayout {
                    version: 1,
                    logical_length: 0,
                    extents: Vec::new(),
                    digest: Vec::new(),
                    kind: pb::VersionKind::Tombstone as i32,
                },
                modified_time_unix_millis: 0,
                new_replicas: Vec::new(),
                operation_id: b"op-1".to_vec(),
                operation_digest: b"digest-1".to_vec(),
                commit_sequence: None,
            })
            .expect("append");
        let state = MetaState::new(Box::new(journal));
        assert_eq!(state.versions[b"model/latest".as_slice()].len(), 1);
        assert_eq!(state.operations[b"op-1".as_slice()].result.version, 1);
    }

    #[test]
    fn snapshot_and_tail_replay_reconstruct_current_and_operation_results() {
        let checkpoint_every_records = 4;
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy {
                every_records: checkpoint_every_records,
            },
            MetaRetentionPolicy::default(),
        );
        let grant = state
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .expect("open node session");
        let session = Some(pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        });
        for index in 1..=checkpoint_every_records {
            let block_id = format!("b-{index}").into_bytes();
            let digest = vec![index as u8];
            state
                .commit_version(pb::CommitVersionRequest {
                    context: None,
                    session: session.clone(),
                    key: Some(pb::Key {
                        value: format!("k-{index}").into_bytes(),
                    }),
                    candidate: Some(pb::VersionCandidate {
                        kind: pb::VersionKind::Value as i32,
                        logical_length: 1,
                        extents: vec![pb::ExtentRecord {
                            logical: Some(pb::ByteRange {
                                offset: 0,
                                length: 1,
                            }),
                            block_id: block_id.clone(),
                            block_offset: 0,
                            digest: digest.clone(),
                            kind: pb::ExtentKind::Data as i32,
                        }],
                        digest: digest.clone(),
                    }),
                    condition: "any".to_string(),
                    expected_version: None,
                    operation_id: format!("op-{index}").into_bytes(),
                    operation_digest: format!("digest-{index}").into_bytes(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                    required_memory_copies: 0,
                    replica_proofs: Vec::new(),
                    new_replicas: vec![pb::ReplicaReport {
                        block_id,
                        length: 1,
                        checksum: digest,
                        durability: pb::DurabilityPolicy::LocalMemory as i32,
                    }],
                    commit_sequence: next_test_commit_sequence(),
                })
                .expect("commit");
        }
        assert_eq!(
            state.journal.load_after(0).expect("compacted tail").len(),
            1
        );
        let snapshot = state
            .journal
            .load_snapshot()
            .expect("snapshot")
            .expect("saved");
        assert_eq!(snapshot.last_applied_index, checkpoint_every_records);

        let restored = MetaState::with_policies(
            state.journal,
            MetaCheckpointPolicy {
                every_records: checkpoint_every_records,
            },
            MetaRetentionPolicy::default(),
        );
        assert!(restored.operations.contains_key(b"op-4".as_slice()));
        assert!(restored.versions.contains_key(b"k-4".as_slice()));
        // Session open is also journaled now. The checkpoint captured index 4,
        // then the fourth commit remained in the tail and replay advanced to 5.
        assert_eq!(restored.last_applied_index, checkpoint_every_records + 1);
    }

    #[test]
    fn same_operation_id_returns_the_original_result() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let grant = state
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .expect("open node session");
        let request = pb::CommitVersionRequest {
            context: None,
            session: Some(pb::NodeSessionIdentity {
                session_id: grant.session_id,
                node_id: grant.node_id,
                node_epoch: grant.node_epoch,
            }),
            key: Some(pb::Key {
                value: b"checkpoint/deleted".to_vec(),
            }),
            candidate: Some(pb::VersionCandidate {
                kind: pb::VersionKind::Tombstone as i32,
                logical_length: 0,
                extents: Vec::new(),
                digest: Vec::new(),
            }),
            condition: "any".to_string(),
            expected_version: None,
            operation_id: b"session-7/delete-1".to_vec(),
            operation_digest: b"same-request".to_vec(),
            durability: pb::DurabilityPolicy::LocalMemory as i32,
            required_memory_copies: 0,
            replica_proofs: Vec::new(),
            new_replicas: Vec::new(),
            commit_sequence: next_test_commit_sequence(),
        };

        let first = state.commit_version(request.clone()).expect("first DEL");
        let retry = state.commit_version(request).expect("retry DEL");

        assert_eq!(first, retry);
        assert!(!first.changed);
    }

    #[test]
    fn reopen_session_returns_commit_sequence_above_seen_window() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let first_grant = state
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .expect("first session");
        let first_session = pb::NodeSessionIdentity {
            session_id: first_grant.session_id,
            node_id: first_grant.node_id,
            node_epoch: first_grant.node_epoch,
        };
        let request = value_commit_request_for(
            first_session,
            b"commit-seq/reopen".to_vec(),
            b"commit-seq/reopen-block".to_vec(),
            b"commit-seq/reopen-op".to_vec(),
            b"commit-seq/reopen-digest".to_vec(),
        );
        let seen_sequence = request.commit_sequence;
        state.commit_version(request).expect("first commit");

        let reopened = state
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .expect("reopen session");
        assert!(
            reopened.minimum_commit_sequence > seen_sequence,
            "new incarnation must not let an already seen logical commit sequence replay"
        );
        let reopened_session = pb::NodeSessionIdentity {
            session_id: reopened.session_id,
            node_id: reopened.node_id,
            node_epoch: reopened.node_epoch,
        };
        let mut stale = value_commit_request_for(
            reopened_session,
            b"commit-seq/reopen-stale".to_vec(),
            b"commit-seq/reopen-stale-block".to_vec(),
            b"commit-seq/reopen-stale-op".to_vec(),
            b"commit-seq/reopen-stale-digest".to_vec(),
        );
        stale.commit_sequence = seen_sequence;
        assert!(matches!(
            state.commit_version(stale),
            Err(MetaRuntimeError::Conflict { .. })
        ));
    }

    #[test]
    fn commit_sequence_allows_bounded_out_of_order_before_reopen() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let grant = state
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .expect("session");
        let session = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        };
        let mut second = value_commit_request_for(
            session.clone(),
            b"commit-seq/out-of-order-2".to_vec(),
            b"commit-seq/out-of-order-block-2".to_vec(),
            b"commit-seq/out-of-order-op-2".to_vec(),
            b"commit-seq/out-of-order-digest-2".to_vec(),
        );
        second.commit_sequence = 2;
        state
            .commit_version(second)
            .expect("seq 2 may arrive first");

        let mut first = value_commit_request_for(
            session.clone(),
            b"commit-seq/out-of-order-1".to_vec(),
            b"commit-seq/out-of-order-block-1".to_vec(),
            b"commit-seq/out-of-order-op-1".to_vec(),
            b"commit-seq/out-of-order-digest-1".to_vec(),
        );
        first.commit_sequence = 1;
        state
            .commit_version(first)
            .expect("seq 1 remains valid inside active replay window");

        let reopened = state
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .expect("reopen session");
        assert_eq!(
            reopened.minimum_commit_sequence, 3,
            "reopen folds active seen max into the new incarnation floor"
        );
        let reopened_session = pb::NodeSessionIdentity {
            session_id: reopened.session_id,
            node_id: reopened.node_id,
            node_epoch: reopened.node_epoch,
        };
        let mut old = value_commit_request_for(
            reopened_session,
            b"commit-seq/out-of-order-old".to_vec(),
            b"commit-seq/out-of-order-old-block".to_vec(),
            b"commit-seq/out-of-order-old-op".to_vec(),
            b"commit-seq/out-of-order-old-digest".to_vec(),
        );
        old.commit_sequence = 1;
        assert!(matches!(
            state.commit_version(old),
            Err(MetaRuntimeError::Conflict { .. })
        ));
    }

    #[test]
    fn retention_keeps_replicas_referenced_by_retained_versions_only() {
        let policy = MetaRetentionPolicy {
            keep_versions_per_key: 2,
            operation_result_retention_records: 100,
            replica_operation_retention_records: 100,
            event_retention_requires_all_session_acks: true,
            drop_unreferenced_replicas: true,
        };
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 1 },
            policy,
        );
        let session = test_session(&mut state, 7);

        for version in 1..=3 {
            state
                .commit_version(value_commit_request_for(
                    session.clone(),
                    b"checkpoint/latest".to_vec(),
                    format!("block-{version}").into_bytes(),
                    format!("op-{version}").into_bytes(),
                    format!("digest-{version}").into_bytes(),
                ))
                .expect("commit");
        }

        let versions = state
            .versions
            .get(b"checkpoint/latest".as_slice())
            .expect("retained versions");
        assert_eq!(versions.len(), 2);
        assert!(
            state.replicas.contains_key(b"block-1".as_slice()),
            "未完成两阶段 retirement 前不能先删除 Meta replica facts"
        );
        assert_eq!(state.pending_retirements.len(), 1);
        assert!(state.replicas.contains_key(b"block-2".as_slice()));
        assert!(state.replicas.contains_key(b"block-3".as_slice()));
    }

    #[test]
    fn deleted_key_trims_old_value_without_waiting_for_sixty_four_overwrites() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let session = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"retire/deleted".to_vec(),
                b"retire/deleted-block".to_vec(),
                b"retire/deleted-op-put".to_vec(),
                b"retire/deleted-digest-put".to_vec(),
            ))
            .expect("put");
        state
            .commit_version(tombstone_request_for(
                session,
                b"retire/deleted".to_vec(),
                b"retire/deleted-op-del".to_vec(),
                b"retire/deleted-digest-del".to_vec(),
            ))
            .expect("delete");

        state.enforce_retention();
        let versions = state
            .versions
            .get(b"retire/deleted".as_slice())
            .expect("tombstone current");
        assert_eq!(versions.len(), 1);
        assert_eq!(
            versions.last_key_value().expect("last tombstone").1.kind,
            pb::VersionKind::Tombstone as i32
        );
        assert_eq!(state.pending_retirements.len(), 1);
        assert!(
            state
                .replicas
                .contains_key(b"retire/deleted-block".as_slice())
        );
    }

    #[test]
    fn default_retention_enables_retirement_for_deleted_values() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"retire/default-delete".to_vec(),
                b"retire/default-delete-block".to_vec(),
                b"retire/default-delete-op-put".to_vec(),
                b"retire/default-delete-digest-put".to_vec(),
            ))
            .expect("put");
        state
            .commit_version(tombstone_request_for(
                session,
                b"retire/default-delete".to_vec(),
                b"retire/default-delete-op-del".to_vec(),
                b"retire/default-delete-digest-del".to_vec(),
            ))
            .expect("delete");

        state.enforce_retention();
        assert_eq!(
            state
                .versions
                .get(b"retire/default-delete".as_slice())
                .expect("retained tombstone")
                .len(),
            1
        );
        let retirement = state
            .pending_retirements
            .values()
            .next()
            .expect("default policy schedules retirement");
        assert_eq!(
            retirement.record.block_ids,
            vec![b"retire/default-delete-block".to_vec()]
        );
    }

    #[test]
    fn tombstone_is_finitely_retained_and_recreate_version_uses_global_floor() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                operation_result_retention_records: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let session = test_session(&mut state, 7);
        let put = state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"churn/recreate".to_vec(),
                b"churn/recreate-old".to_vec(),
                b"churn/recreate-put".to_vec(),
                b"churn/recreate-put-digest".to_vec(),
            ))
            .expect("put");
        let delete = state
            .commit_version(tombstone_request_for(
                session.clone(),
                b"churn/recreate".to_vec(),
                b"churn/recreate-del".to_vec(),
                b"churn/recreate-del-digest".to_vec(),
            ))
            .expect("delete");
        state.complete_operation_visibility(&[
            b"churn/recreate-put".to_vec(),
            b"churn/recreate-del".to_vec(),
        ]);
        state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"churn/advance".to_vec(),
                b"churn/advance-block".to_vec(),
                b"churn/advance-put".to_vec(),
                b"churn/advance-digest".to_vec(),
            ))
            .expect("advance retention clock");
        state.complete_operation_visibility(&[b"churn/advance-put".to_vec()]);
        state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"churn/advance-2".to_vec(),
                b"churn/advance-block-2".to_vec(),
                b"churn/advance-put-2".to_vec(),
                b"churn/advance-digest-2".to_vec(),
            ))
            .expect("advance retention clock past tombstone window");
        state.complete_operation_visibility(&[b"churn/advance-put-2".to_vec()]);
        state.enforce_retention();
        assert!(
            !state.versions.contains_key(b"churn/recreate".as_slice()),
            "有限窗口后不永久保留每 key tombstone"
        );
        assert!(state.version_floor >= delete.version);
        assert!(delete.version > put.version);

        let recreated = state
            .commit_version(value_commit_request_for(
                session,
                b"churn/recreate".to_vec(),
                b"churn/recreate-new".to_vec(),
                b"churn/recreate-new-op".to_vec(),
                b"churn/recreate-new-digest".to_vec(),
            ))
            .expect("recreate after tombstone purge");
        assert!(recreated.version > delete.version);
    }

    #[test]
    fn block_retirement_requires_prepare_and_release_acks_before_meta_forgets_replicas() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/key".to_vec(),
                b"retire/block-old".to_vec(),
                b"retire/op-old".to_vec(),
                b"retire/digest-old".to_vec(),
            ))
            .expect("old version");
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/key".to_vec(),
                b"retire/block-new".to_vec(),
                b"retire/op-new".to_vec(),
                b"retire/digest-new".to_vec(),
            ))
            .expect("new version");

        state.enforce_retention();
        assert!(state.replicas.contains_key(b"retire/block-old".as_slice()));
        let retirement = state
            .pending_retirements
            .values()
            .next()
            .expect("pending retirement")
            .clone();
        assert_eq!(
            retirement.record.block_ids,
            vec![b"retire/block-old".to_vec()]
        );
        assert_eq!(retirement.record.participants.len(), 2);
        assert!(state.events.iter().any(|event| matches!(
            &event.event,
            Some(pb::node_event::Event::EvictReplica(evict))
                if evict.phase == pb::BlockRetirementPhase::Prepare as i32
                    && evict.participant_node_id == reader.node_id
        )));

        state
            .acknowledge_block_retirement(retirement_ack_request(
                writer.clone(),
                &retirement.record,
                pb::BlockRetirementAckKind::Prepared,
                retirement.record.prepare_stage_epoch,
            ))
            .expect("writer prepared");
        assert!(
            !state
                .pending_retirements
                .get(&retirement.record.retirement_id)
                .expect("still pending")
                .final_sent
        );
        state
            .acknowledge_block_retirement(retirement_ack_request(
                reader.clone(),
                &retirement.record,
                pb::BlockRetirementAckKind::Prepared,
                retirement.record.prepare_stage_epoch,
            ))
            .expect("reader prepared");
        let finalized = state
            .pending_retirements
            .get(&retirement.record.retirement_id)
            .expect("final pending")
            .clone();
        assert!(finalized.final_sent);
        assert!(state.replicas.contains_key(b"retire/block-old".as_slice()));

        for session in [writer, reader] {
            state
                .acknowledge_block_retirement(retirement_ack_request(
                    session,
                    &finalized.record,
                    pb::BlockRetirementAckKind::Released,
                    finalized.record.final_stage_epoch,
                ))
                .expect("released");
        }
        assert!(!state.replicas.contains_key(b"retire/block-old".as_slice()));
        assert!(
            state
                .retired_block_fences
                .contains_key(b"retire/block-old".as_slice())
        );
    }

    #[test]
    fn release_ack_refreshes_retired_replica_metrics_immediately() {
        let registry = dms_metrics::registry();
        let metrics = MetaMetrics::register(&registry).expect("metrics");
        let mut state = MetaState::try_new(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy::default(),
            metrics,
        )
        .expect("state");
        let session = test_session(&mut state, 7);

        state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"retire/metrics".to_vec(),
                b"retire/metrics-block".to_vec(),
                b"retire/metrics-op-put".to_vec(),
                b"retire/metrics-digest-put".to_vec(),
            ))
            .expect("put");
        state
            .commit_version(tombstone_request_for(
                session.clone(),
                b"retire/metrics".to_vec(),
                b"retire/metrics-op-del".to_vec(),
                b"retire/metrics-digest-del".to_vec(),
            ))
            .expect("delete");
        state.enforce_retention();
        state.refresh_metrics();
        let before = dms_metrics::encode_text(&registry).expect("encode before");
        assert!(
            before.contains("dms_meta_state_items{type=\"replicas\"} 1\n"),
            "test setup must expose one retained replica before Release ACK: {before}"
        );
        assert!(
            before.contains("dms_meta_blocks{state=\"healthy\"} 1\n"),
            "test setup must expose one healthy retained block before Release ACK: {before}"
        );

        let retirement = state
            .pending_retirements
            .values()
            .next()
            .expect("pending retirement")
            .clone();
        state
            .acknowledge_block_retirement(retirement_ack_request(
                session.clone(),
                &retirement.record,
                pb::BlockRetirementAckKind::Prepared,
                retirement.record.prepare_stage_epoch,
            ))
            .expect("prepared");
        let finalized = state
            .pending_retirements
            .get(&retirement.record.retirement_id)
            .expect("final pending")
            .clone();
        state
            .acknowledge_block_retirement(retirement_ack_request(
                session,
                &finalized.record,
                pb::BlockRetirementAckKind::Released,
                finalized.record.final_stage_epoch,
            ))
            .expect("released");

        assert!(
            !state
                .replicas
                .contains_key(b"retire/metrics-block".as_slice())
        );
        let after = dms_metrics::encode_text(&registry).expect("encode after");
        assert!(
            after.contains("dms_meta_state_items{type=\"replicas\"} 0\n"),
            "Release ACK must synchronously publish retired replica count: {after}"
        );
        assert!(
            after.contains("dms_meta_blocks{state=\"healthy\"} 0\n"),
            "Release ACK must synchronously publish retired block health: {after}"
        );
    }

    #[test]
    fn retired_non_replica_reader_still_participates_in_block_retirement() {
        for expire_before_resolve in [false, true] {
            let mut state = MetaState::with_policies(
                Box::<InMemoryJournal>::default(),
                MetaCheckpointPolicy::default(),
                MetaRetentionPolicy {
                    keep_versions_per_key: 1,
                    drop_unreferenced_replicas: true,
                    ..MetaRetentionPolicy::default()
                },
            );
            let writer = test_session(&mut state, 7);
            let reader = test_session(&mut state, 8);
            let old = state
                .commit_version(value_commit_request_for(
                    writer.clone(),
                    b"retire/non-replica-reader".to_vec(),
                    b"retire/non-replica-reader-old".to_vec(),
                    b"retire/non-replica-reader-op-old".to_vec(),
                    b"retire/non-replica-reader-digest-old".to_vec(),
                ))
                .expect("old value");
            if expire_before_resolve {
                expire_session(&mut state, reader.node_id);
                state.retire_expired_sessions().expect("retire reader");
                assert!(state.retired_sessions.contains(&reader.node_id));
            }

            let resolved = state
                .resolve_object(pb::ResolveObjectRequest {
                    context: None,
                    session: Some(reader.clone()),
                    key: Some(pb::Key {
                        value: b"retire/non-replica-reader".to_vec(),
                    }),
                    selector: Some(pb::resolve_object_request::Selector::ExactVersion(
                        old.version,
                    )),
                    range: None,
                    cache_current: false,
                })
                .expect("reader may hold an exact read layout without owning a replica");
            assert_eq!(
                resolved.layout.expect("layout").version,
                old.version,
                "reader holds the old version layout in scenario expire_before_resolve={expire_before_resolve}"
            );
            assert!(
                resolved.block_replicas.iter().all(|block| {
                    block
                        .replicas
                        .iter()
                        .all(|replica| replica.node_id != reader.node_id)
                }),
                "reader is a layout holder, not a replica owner"
            );
            if !expire_before_resolve {
                expire_session(&mut state, reader.node_id);
                state.retire_expired_sessions().expect("retire reader");
                assert!(state.retired_sessions.contains(&reader.node_id));
            }

            state
                .commit_version(value_commit_request_for(
                    writer.clone(),
                    b"retire/non-replica-reader".to_vec(),
                    b"retire/non-replica-reader-new".to_vec(),
                    b"retire/non-replica-reader-op-new".to_vec(),
                    b"retire/non-replica-reader-digest-new".to_vec(),
                ))
                .expect("new value");
            state.enforce_retention();
            let retirement = state
                .pending_retirements
                .values()
                .next()
                .expect("pending retirement")
                .clone();
            assert!(
                retirement
                    .record
                    .participants
                    .iter()
                    .any(|participant| participant.node_id == reader.node_id
                        && participant.node_epoch == reader.node_epoch),
                "expired/retired layout holder must participate in scenario expire_before_resolve={expire_before_resolve}"
            );

            state
                .acknowledge_block_retirement(retirement_ack_request(
                    writer.clone(),
                    &retirement.record,
                    pb::BlockRetirementAckKind::Prepared,
                    retirement.record.prepare_stage_epoch,
                ))
                .expect("writer prepared");
            let pending = state
                .pending_retirements
                .get(&retirement.record.retirement_id)
                .expect("still pending without reader drain")
                .clone();
            assert!(
                !pending.final_sent,
                "must not Final before retired reader sends Prepare ACK in scenario expire_before_resolve={expire_before_resolve}"
            );
            assert!(
                state
                    .replicas
                    .contains_key(b"retire/non-replica-reader-old".as_slice())
            );

            state
                .acknowledge_block_retirement(retirement_ack_request(
                    reader.clone(),
                    &pending.record,
                    pb::BlockRetirementAckKind::Prepared,
                    pending.record.prepare_stage_epoch,
                ))
                .expect("reader prepared");
            let finalized = state
                .pending_retirements
                .get(&pending.record.retirement_id)
                .expect("final pending")
                .clone();
            assert!(finalized.final_sent);
            for participant in [writer.clone(), reader.clone()] {
                state
                    .acknowledge_block_retirement(retirement_ack_request(
                        participant,
                        &finalized.record,
                        pb::BlockRetirementAckKind::Released,
                        finalized.record.final_stage_epoch,
                    ))
                    .expect("released");
            }
            assert!(
                state
                    .retired_block_fences
                    .contains_key(b"retire/non-replica-reader-old".as_slice())
            );
            let late_report = state
                .report_replicas(pb::ReportReplicasRequest {
                    context: None,
                    session: Some(reader),
                    replicas: vec![pb::ReplicaReport {
                        block_id: b"retire/non-replica-reader-old".to_vec(),
                        length: 4,
                        checksum: b"digest".to_vec(),
                        durability: pb::DurabilityPolicy::LocalMemory as i32,
                    }],
                    desired_copies: 1,
                    repair_id: Vec::new(),
                    operation_id: format!(
                        "retire/non-replica-reader-late-report-{expire_before_resolve}"
                    )
                    .into_bytes(),
                })
                .expect("late report rejected by fence, not transport/session");
            assert!(late_report.accepted.is_empty());
            assert_eq!(
                late_report.rejected_block_ids,
                vec![b"retire/non-replica-reader-old".to_vec()]
            );
        }
    }

    #[test]
    fn block_retirement_survives_snapshot_and_rejects_late_resurrection() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/replay".to_vec(),
                b"retire/replay-old".to_vec(),
                b"retire/replay-op-old".to_vec(),
                b"retire/replay-digest-old".to_vec(),
            ))
            .expect("old version");
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/replay".to_vec(),
                b"retire/replay-new".to_vec(),
                b"retire/replay-op-new".to_vec(),
                b"retire/replay-digest-new".to_vec(),
            ))
            .expect("new version");
        state.enforce_retention();
        let retirement = state
            .pending_retirements
            .values()
            .next()
            .expect("pending")
            .record
            .clone();

        let mut journal = InMemoryJournal::default();
        journal.save_snapshot(state.snapshot()).expect("snapshot");
        let mut restored = MetaState::with_policies(
            Box::new(journal),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        restored
            .acknowledge_block_retirement(retirement_ack_request(
                writer.clone(),
                &retirement,
                pb::BlockRetirementAckKind::Prepared,
                retirement.prepare_stage_epoch,
            ))
            .expect("prepared after restore");
        let finalized = restored
            .pending_retirements
            .get(&retirement.retirement_id)
            .expect("finalized")
            .record
            .clone();
        restored
            .acknowledge_block_retirement(retirement_ack_request(
                writer.clone(),
                &finalized,
                pb::BlockRetirementAckKind::Released,
                finalized.final_stage_epoch,
            ))
            .expect("released after restore");
        assert!(
            restored
                .retired_block_fences
                .contains_key(b"retire/replay-old".as_slice())
        );
        assert!(
            restored
                .acknowledge_block_retirement(retirement_ack_request(
                    writer.clone(),
                    &finalized,
                    pb::BlockRetirementAckKind::Released,
                    finalized.final_stage_epoch,
                ))
                .expect("released ack retry is idempotent")
                .accepted
        );

        let late_report = restored
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(writer.clone()),
                replicas: vec![pb::ReplicaReport {
                    block_id: b"retire/replay-old".to_vec(),
                    length: 1,
                    checksum: b"late".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                }],
                operation_id: b"late-report".to_vec(),
                desired_copies: 1,
                repair_id: Vec::new(),
            })
            .expect("late report rejected in response");
        assert_eq!(late_report.accepted.len(), 0);
        assert_eq!(
            late_report.rejected_block_ids,
            vec![b"retire/replay-old".to_vec()]
        );
        assert!(matches!(
            restored.commit_version(value_commit_request_for(
                writer,
                b"retire/replay2".to_vec(),
                b"retire/replay-old".to_vec(),
                b"retire/replay2-op".to_vec(),
                b"retire/replay2-digest".to_vec(),
            )),
            Err(MetaRuntimeError::Conflict { .. })
        ));
    }

    #[test]
    fn retirement_watch_ack_without_gc_ack_replays_after_snapshot_restore() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/replay-duty".to_vec(),
                b"retire/replay-duty-old".to_vec(),
                b"retire/replay-duty-op-old".to_vec(),
                b"retire/replay-duty-digest-old".to_vec(),
            ))
            .expect("old version");
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/replay-duty".to_vec(),
                b"retire/replay-duty-new".to_vec(),
                b"retire/replay-duty-op-new".to_vec(),
                b"retire/replay-duty-digest-new".to_vec(),
            ))
            .expect("new version");
        state.enforce_retention();

        let event = state
            .events
            .iter()
            .find(|event| {
                matches!(
                    &event.event,
                    Some(pb::node_event::Event::EvictReplica(evict))
                        if evict.phase == pb::BlockRetirementPhase::Prepare as i32
                            && evict.participant_node_id == writer.node_id
                )
            })
            .expect("prepare duty event")
            .clone();
        let (live_sender, mut live_receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer.clone()),
                    last_acked_cursor: 0,
                },
                live_sender,
            )
            .expect("healthy watch stream");
        assert!(
            std::iter::from_fn(|| live_receiver.try_recv().ok())
                .any(|received| received.cursor == event.cursor),
            "first stream pass delivers retirement duty"
        );
        state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(writer.clone()),
                event_id: event.event_id.clone(),
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("ordinary watch ack");
        let session = state.sessions.get(&writer.node_id).expect("session");
        assert!(
            session.last_acked_cursor < event.cursor,
            "ordinary Watch ACK must not prove retirement drain/release duty"
        );

        state.retain_events();
        assert!(
            state.events.iter().any(|item| item.cursor == event.cursor),
            "outstanding retirement duty cannot be pruned by ordinary cursor ACK"
        );
        state
            .watchers
            .get_mut(&writer.node_id)
            .expect("watcher")
            .next_gc_retry_at = Some(Instant::now() - GC_RETRY_INTERVAL);
        state.pump_watchers();
        assert!(
            std::iter::from_fn(|| live_receiver.try_recv().ok())
                .any(|received| received.cursor == event.cursor),
            "same healthy stream redelivers outstanding duty after bounded retry interval"
        );

        let mut journal = InMemoryJournal::default();
        journal.save_snapshot(state.snapshot()).expect("snapshot");
        let mut restored = MetaState::with_policies(
            Box::new(journal),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let (sender, mut receiver) = mpsc::channel(1);
        restored
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer),
                    last_acked_cursor: event.cursor,
                },
                sender,
            )
            .expect("watch reconnect");
        let replayed = receiver.try_recv().expect("replayed retirement duty");
        assert_eq!(replayed.cursor, event.cursor);
    }

    #[test]
    fn capacity_one_gc_retry_does_not_starve_fresh_invalidation_or_flood_ticks() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/fairness".to_vec(),
                b"retire/fairness-old".to_vec(),
                b"retire/fairness-op-old".to_vec(),
                b"retire/fairness-digest-old".to_vec(),
            ))
            .expect("old version");
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/fairness".to_vec(),
                b"retire/fairness-new".to_vec(),
                b"retire/fairness-op-new".to_vec(),
                b"retire/fairness-digest-new".to_vec(),
            ))
            .expect("new version");
        state.enforce_retention();
        let gc_cursor = state
            .events
            .iter()
            .find(|event| {
                matches!(
                    &event.event,
                    Some(pb::node_event::Event::EvictReplica(evict))
                        if evict.phase == pb::BlockRetirementPhase::Prepare as i32
                            && evict.participant_node_id == writer.node_id
                )
            })
            .expect("prepare duty")
            .cursor;

        let (sender, mut receiver) = mpsc::channel(1);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer.clone()),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("watch");
        loop {
            let event = receiver.try_recv().expect("event before gc duty");
            if event.cursor == gc_cursor {
                break;
            }
            state.pump_watchers();
        }

        state.event_high_watermark += 1;
        let fresh_cursor = state.event_high_watermark;
        state.events.push(pb::NodeEvent {
            event_id: b"fresh-after-gc-debt".to_vec(),
            cursor: fresh_cursor,
            event: Some(pb::node_event::Event::InvalidateCurrent(
                pb::InvalidateCurrentEvent {
                    key: Some(pb::Key {
                        value: b"retire/fairness-fresh".to_vec(),
                    }),
                    old_version: 1,
                    transition_id: b"fresh-transition".to_vec(),
                    lease_epoch: 0,
                    revision: fresh_cursor,
                    minimum_version: fresh_cursor,
                    source_node_id: 0,
                },
            )),
        });

        state.pump_watchers();
        let fresh = receiver
            .try_recv()
            .expect("fresh invalidation must not starve behind GC retry");
        assert_eq!(fresh.cursor, fresh_cursor);
        assert!(matches!(
            fresh.event,
            Some(pb::node_event::Event::InvalidateCurrent(_))
        ));

        for _ in 0..5 {
            state.pump_watchers();
        }
        assert!(
            receiver.try_recv().is_err(),
            "GC retry is rate limited and does not resend every pump tick"
        );
    }

    #[test]
    fn due_gc_retry_survives_continuous_fresh_traffic_when_capacity_is_available() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/fresh-spare".to_vec(),
                b"retire/fresh-spare-old".to_vec(),
                b"retire/fresh-spare-op-old".to_vec(),
                b"retire/fresh-spare-digest-old".to_vec(),
            ))
            .expect("old version");
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/fresh-spare".to_vec(),
                b"retire/fresh-spare-new".to_vec(),
                b"retire/fresh-spare-op-new".to_vec(),
                b"retire/fresh-spare-digest-new".to_vec(),
            ))
            .expect("new version");
        state.enforce_retention();
        let gc_cursor = state
            .events
            .iter()
            .find(|event| {
                matches!(
                    &event.event,
                    Some(pb::node_event::Event::EvictReplica(evict))
                        if evict.phase == pb::BlockRetirementPhase::Prepare as i32
                            && evict.participant_node_id == writer.node_id
                )
            })
            .expect("prepare duty")
            .cursor;

        let (sender, mut receiver) = mpsc::channel(8);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer.clone()),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("watch");
        while receiver.try_recv().is_ok() {}

        state
            .watchers
            .get_mut(&writer.node_id)
            .expect("watcher")
            .next_gc_retry_at = Some(Instant::now() - GC_RETRY_INTERVAL);
        state.event_high_watermark += 1;
        let fresh_cursor = state.event_high_watermark;
        state.events.push(pb::NodeEvent {
            event_id: b"fresh-spare-after-due-gc".to_vec(),
            cursor: fresh_cursor,
            event: Some(pb::node_event::Event::InvalidateCurrent(
                pb::InvalidateCurrentEvent {
                    key: Some(pb::Key {
                        value: b"retire/fresh-spare-later".to_vec(),
                    }),
                    old_version: 1,
                    transition_id: b"fresh-spare-transition".to_vec(),
                    lease_epoch: 0,
                    revision: fresh_cursor,
                    minimum_version: fresh_cursor,
                    source_node_id: 0,
                },
            )),
        });

        state.pump_watchers();
        let first = receiver.try_recv().expect("fresh event");
        let second = receiver.try_recv().expect("due gc retry");
        assert_eq!(first.cursor, fresh_cursor, "fresh remains first");
        assert_eq!(
            second.cursor, gc_cursor,
            "due GC retry is not postponed by fresh traffic when capacity remains"
        );
        assert!(
            receiver.try_recv().is_err(),
            "one pump sends at most one GC-only retry"
        );
    }

    #[test]
    fn gc_retry_rotates_across_multiple_outstanding_duties() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        for key in [b"retire/rotate-a".as_slice(), b"retire/rotate-b".as_slice()] {
            state
                .commit_version(value_commit_request_for(
                    writer.clone(),
                    key.to_vec(),
                    [key, b"-old"].concat(),
                    [key, b"-op-old"].concat(),
                    [key, b"-digest-old"].concat(),
                ))
                .expect("old value");
            state
                .commit_version(value_commit_request_for(
                    writer.clone(),
                    key.to_vec(),
                    [key, b"-new"].concat(),
                    [key, b"-op-new"].concat(),
                    [key, b"-digest-new"].concat(),
                ))
                .expect("new value");
            state.enforce_retention();
        }
        let mut gc_cursors = state
            .events
            .iter()
            .filter(|event| {
                matches!(
                    &event.event,
                    Some(pb::node_event::Event::EvictReplica(evict))
                        if evict.phase == pb::BlockRetirementPhase::Prepare as i32
                            && evict.participant_node_id == writer.node_id
                )
            })
            .map(|event| event.cursor)
            .collect::<Vec<_>>();
        gc_cursors.sort_unstable();
        assert_eq!(gc_cursors.len(), 2);

        let (sender, mut receiver) = mpsc::channel(16);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer.clone()),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("watch");
        while receiver.try_recv().is_ok() {}

        for expected in &gc_cursors {
            state
                .watchers
                .get_mut(&writer.node_id)
                .expect("watcher")
                .next_gc_retry_at = Some(Instant::now() - GC_RETRY_INTERVAL);
            state.pump_watchers();
            let retried = receiver.try_recv().expect("rotated gc retry");
            assert_eq!(retried.cursor, *expected);
        }
    }

    #[test]
    fn retention_snapshot_failure_pauses_physical_retirement_decision() {
        let journal = SharedFailingSnapshotJournal {
            fail_snapshot: true,
            ..SharedFailingSnapshotJournal::default()
        };
        let shared = journal.inner.clone();
        let mut state = MetaState::with_policies(
            Box::new(journal),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/snapshot-fail".to_vec(),
                b"retire/snapshot-fail-old".to_vec(),
                b"retire/snapshot-fail-op-old".to_vec(),
                b"retire/snapshot-fail-digest-old".to_vec(),
            ))
            .expect("old version");
        state
            .commit_version(value_commit_request_for(
                writer,
                b"retire/snapshot-fail".to_vec(),
                b"retire/snapshot-fail-new".to_vec(),
                b"retire/snapshot-fail-op-new".to_vec(),
                b"retire/snapshot-fail-digest-new".to_vec(),
            ))
            .expect("new version");

        state.enforce_retention();
        assert!(
            state.pending_retirements.is_empty(),
            "snapshot failure must pause Prepared WAL emission"
        );
        assert!(
            state
                .replicas
                .contains_key(b"retire/snapshot-fail-old".as_slice()),
            "without a durable prune boundary Meta cannot retire the old block"
        );
        assert!(
            state
                .journal
                .load_after(0)
                .expect("journal replay")
                .iter()
                .all(|entry| !matches!(
                    entry.record,
                    JournalRecord::BlockRetirementPrepared { .. }
                )),
            "crash replay must not infer an all-gone retirement from volatile pruning"
        );

        let restored = MetaState::with_policies(
            Box::new(SharedFailingSnapshotJournal {
                inner: shared,
                fail_snapshot: true,
            }),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        assert!(
            restored
                .replicas
                .contains_key(b"retire/snapshot-fail-old".as_slice()),
            "replay after the failed boundary still references data until a later durable pass"
        );
        assert!(restored.pending_retirements.is_empty());
    }

    #[test]
    fn tightened_retention_after_snapshot_persists_prune_before_prepare() {
        let mut baseline = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 64,
                drop_unreferenced_replicas: false,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut baseline, 7);
        let old = baseline
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/tighten".to_vec(),
                b"retire/tighten-old".to_vec(),
                b"retire/tighten-op-old".to_vec(),
                b"retire/tighten-digest-old".to_vec(),
            ))
            .expect("old value");
        baseline
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/tighten".to_vec(),
                b"retire/tighten-new".to_vec(),
                b"retire/tighten-op-new".to_vec(),
                b"retire/tighten-digest-new".to_vec(),
            ))
            .expect("new value");
        baseline
            .journal
            .save_snapshot(baseline.snapshot())
            .expect("snapshot with old policy");
        baseline
            .journal
            .truncate_prefix(baseline.last_applied_index)
            .expect("truncate at snapshot");

        let tightened = MetaState::with_policies(
            baseline.journal,
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        assert_eq!(
            tightened
                .journal
                .load_snapshot()
                .expect("tightened snapshot")
                .expect("snapshot exists")
                .versions
                .into_iter()
                .find(|(key, _)| key == b"retire/tighten")
                .expect("key snapshot")
                .1
                .len(),
            1,
            "same-index retention prune must be snapshotted before Prepared can be durable"
        );
        assert_eq!(tightened.pending_retirements.len(), 1);

        let widened = MetaState::with_policies(
            tightened.journal,
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 64,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        assert!(matches!(
            widened.resolve_object(pb::ResolveObjectRequest {
                context: None,
                session: Some(writer),
                key: Some(pb::Key {
                    value: b"retire/tighten".to_vec(),
                }),
                selector: Some(pb::resolve_object_request::Selector::ExactVersion(
                    old.version,
                )),
                range: None,
                cache_current: false,
            }),
            Err(MetaRuntimeError::NotFound)
        ));
        assert_eq!(widened.pending_retirements.len(), 1);
    }

    #[test]
    fn later_retirement_ack_cannot_advance_cursor_past_earlier_gc_debt() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        for key in [b"retire/order-a".as_slice(), b"retire/order-b".as_slice()] {
            state
                .commit_version(value_commit_request_for(
                    writer.clone(),
                    key.to_vec(),
                    [key, b"-old"].concat(),
                    [key, b"-op-old"].concat(),
                    [key, b"-digest-old"].concat(),
                ))
                .expect("old value");
            state
                .commit_version(value_commit_request_for(
                    writer.clone(),
                    key.to_vec(),
                    [key, b"-new"].concat(),
                    [key, b"-op-new"].concat(),
                    [key, b"-digest-new"].concat(),
                ))
                .expect("new value");
            state.enforce_retention();
        }

        let retirements = state
            .pending_retirements
            .values()
            .map(|pending| pending.record.clone())
            .collect::<Vec<_>>();
        assert_eq!(retirements.len(), 2);
        let mut duties = retirements
            .iter()
            .map(|record| {
                let cursor = state
                    .events
                    .iter()
                    .find(|event| {
                        matches!(
                            &event.event,
                            Some(pb::node_event::Event::EvictReplica(evict))
                                if evict.retirement_id == record.retirement_id
                                    && evict.participant_node_id == writer.node_id
                                    && evict.phase == pb::BlockRetirementPhase::Prepare as i32
                        )
                    })
                    .expect("prepare event")
                    .cursor;
                (cursor, record.clone())
            })
            .collect::<Vec<_>>();
        duties.sort_by_key(|(cursor, _)| *cursor);
        let first_cursor = duties[0].0;
        let second = duties[1].1.clone();

        state
            .acknowledge_block_retirement(retirement_ack_request(
                writer.clone(),
                &second,
                pb::BlockRetirementAckKind::Prepared,
                second.prepare_stage_epoch,
            ))
            .expect("later prepare ack");
        assert!(
            state
                .sessions
                .get(&writer.node_id)
                .expect("session")
                .last_acked_cursor
                < first_cursor,
            "later dedicated ACK cannot skip an earlier outstanding retirement duty"
        );
        assert!(
            state
                .events
                .iter()
                .any(|event| event.cursor == first_cursor),
            "earlier GC duty remains replayable"
        );
    }

    #[test]
    fn expired_session_does_not_ack_past_outstanding_retirement_duty() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/expire-duty".to_vec(),
                b"retire/expire-duty-old".to_vec(),
                b"retire/expire-duty-op-old".to_vec(),
                b"retire/expire-duty-digest-old".to_vec(),
            ))
            .expect("old value");
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/expire-duty".to_vec(),
                b"retire/expire-duty-new".to_vec(),
                b"retire/expire-duty-op-new".to_vec(),
                b"retire/expire-duty-digest-new".to_vec(),
            ))
            .expect("new value");
        state.enforce_retention();
        let event = state
            .events
            .iter()
            .find(|event| {
                matches!(
                    &event.event,
                    Some(pb::node_event::Event::EvictReplica(evict))
                        if evict.phase == pb::BlockRetirementPhase::Prepare as i32
                            && evict.participant_node_id == writer.node_id
                )
            })
            .expect("prepare duty")
            .clone();
        state
            .sessions
            .get_mut(&writer.node_id)
            .expect("session")
            .last_heartbeat =
            Some(Instant::now() - DEFAULT_NODE_LEASE_TTL - Duration::from_secs(1));

        state.retire_expired_sessions().expect("retire expired");
        assert!(state.retired_sessions.contains(&writer.node_id));
        assert!(
            state
                .sessions
                .get(&writer.node_id)
                .expect("session retained for replay")
                .last_acked_cursor
                < event.cursor,
            "lease expiry cannot prove GC drain/release"
        );
        assert!(
            state.events.iter().any(|item| item.cursor == event.cursor),
            "expired session GC duty remains durable/replayable"
        );

        let mut journal = InMemoryJournal::default();
        journal.save_snapshot(state.snapshot()).expect("snapshot");
        let mut restored = MetaState::with_policies(
            Box::new(journal),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let (sender, mut receiver) = mpsc::channel(1);
        restored
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer),
                    last_acked_cursor: event.cursor,
                },
                sender,
            )
            .expect("watch after restore");
        assert_eq!(
            receiver.try_recv().expect("replayed after restore").cursor,
            event.cursor
        );
    }

    #[test]
    fn journal_index_versions_never_reuse_or_rewind_old_snapshot_versions() {
        let mut journal = InMemoryJournal::default();
        journal
            .save_snapshot(MetaSnapshot {
                last_applied_index: 10,
                version_floor: 0,
                next_session: 1,
                node_epochs: vec![(7, 1)],
                node_commit_sequence_floors: Vec::new(),
                sessions: vec![SnapshotSession {
                    node_id: 7,
                    session_id: b"manual-session".to_vec(),
                    node_epoch: 1,
                    control_endpoint: "http://127.0.0.1:19007".to_string(),
                    last_acked_cursor: 0,
                    supports_commit_sequence: true,
                    commit_sequence_floor: 0,
                }],
                replicas: Vec::new(),
                desired_replica_counts: Vec::new(),
                versions: vec![(
                    b"compat/high".to_vec(),
                    vec![pb::VersionLayout {
                        version: 100,
                        logical_length: 0,
                        extents: Vec::new(),
                        digest: b"old".to_vec(),
                        kind: pb::VersionKind::Tombstone as i32,
                    }],
                )],
                version_modified_times: Vec::new(),
                block_retirements: Vec::new(),
                retired_block_fences: Vec::new(),
                commit_sequences: Vec::new(),
                operations: Vec::new(),
                replica_operations: Vec::new(),
                event_high_watermark: 0,
                events: Vec::new(),
                filesystem_next_inode: 2,
                filesystem_inodes: Vec::new(),
                filesystem_dentries: Vec::new(),
                filesystem_grant_generations: Vec::new(),
                filesystem_xattrs: Vec::new(),
                filesystem_namespace_operations: Vec::new(),
                filesystem_version_operations: Vec::new(),
                filesystem_attribute_operations: Vec::new(),
                filesystem_operation_visibility: Some(SnapshotFilesystemOperationVisibility {
                    namespace: Vec::new(),
                    versions: Vec::new(),
                    attributes: Vec::new(),
                }),
            })
            .expect("manual snapshot");
        let mut restored = MetaState::new(Box::new(journal));
        let session = pb::NodeSessionIdentity {
            session_id: b"manual-session".to_vec(),
            node_id: 7,
            node_epoch: 1,
        };
        let response = restored
            .commit_version(value_commit_request_for(
                session,
                b"compat/high".to_vec(),
                b"compat/high-block".to_vec(),
                b"compat/high-op".to_vec(),
                b"compat/high-digest".to_vec(),
            ))
            .expect("commit after high old version");
        assert_eq!(response.version, 101);
        assert!(response.commit_index > 10);
    }

    #[test]
    fn expired_operation_retry_cannot_resurrect_retired_block() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy {
                keep_versions_per_key: 1,
                operation_result_retention_records: 0,
                drop_unreferenced_replicas: true,
                ..MetaRetentionPolicy::default()
            },
        );
        let writer = test_session(&mut state, 7);
        let old_request = value_commit_request_for(
            writer.clone(),
            b"retire/expired-op".to_vec(),
            b"retire/expired-op-old".to_vec(),
            b"retire/expired-op-old-id".to_vec(),
            b"retire/expired-op-old-digest".to_vec(),
        );
        state
            .commit_version(old_request.clone())
            .expect("old value");
        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"retire/expired-op".to_vec(),
                b"retire/expired-op-new".to_vec(),
                b"retire/expired-op-new-id".to_vec(),
                b"retire/expired-op-new-digest".to_vec(),
            ))
            .expect("new value");
        state.enforce_retention();
        let retirement = state
            .pending_retirements
            .values()
            .next()
            .expect("pending retirement")
            .record
            .clone();
        state
            .acknowledge_block_retirement(retirement_ack_request(
                writer.clone(),
                &retirement,
                pb::BlockRetirementAckKind::Prepared,
                retirement.prepare_stage_epoch,
            ))
            .expect("prepared");
        let finalized = state
            .pending_retirements
            .get(&retirement.retirement_id)
            .expect("finalized")
            .record
            .clone();
        state
            .acknowledge_block_retirement(retirement_ack_request(
                writer,
                &finalized,
                pb::BlockRetirementAckKind::Released,
                finalized.final_stage_epoch,
            ))
            .expect("released");
        state.enforce_retention();
        assert!(
            !state
                .operations
                .contains_key(b"retire/expired-op-old-id".as_slice())
        );
        assert!(
            state
                .retired_block_fences
                .contains_key(b"retire/expired-op-old".as_slice())
        );
        for index in 0..MAX_RETIRED_BLOCK_FENCES {
            state.retired_block_fences.insert(
                format!("retire/expired-op-newer-fence-{index:05}").into_bytes(),
                finalized.final_stage_epoch + index as u64 + 2,
            );
        }
        state.retain_retired_block_fences();
        assert!(
            !state
                .retired_block_fences
                .contains_key(b"retire/expired-op-old".as_slice()),
            "bounded retired fence is intentionally evicted in this regression"
        );
        assert!(matches!(
            state.commit_version(old_request),
            Err(MetaRuntimeError::Conflict { .. })
        ));
    }

    #[test]
    fn retired_block_fence_window_is_bounded() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        for index in 0..(MAX_RETIRED_BLOCK_FENCES + 2) {
            state.retired_block_fences.insert(
                format!("retired/fence-{index:05}").into_bytes(),
                index as u64,
            );
        }
        state.retain_retired_block_fences();
        assert_eq!(state.retired_block_fences.len(), MAX_RETIRED_BLOCK_FENCES);
        assert!(
            !state
                .retired_block_fences
                .contains_key(b"retired/fence-00000".as_slice())
        );
        assert!(
            state.retired_block_fences.contains_key(
                format!("retired/fence-{:05}", MAX_RETIRED_BLOCK_FENCES + 1).as_bytes()
            )
        );
    }

    #[test]
    fn admission_rejects_retired_block_after_fence_window_eviction() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let stale_block = b"retired/fence-evicted-block".to_vec();
        state.retired_block_fences.insert(stale_block.clone(), 1);
        for index in 0..MAX_RETIRED_BLOCK_FENCES {
            state.retired_block_fences.insert(
                format!("retired/newer-fence-{index:05}").into_bytes(),
                (index + 2) as u64,
            );
        }
        state.retain_retired_block_fences();
        assert!(
            !state.retired_block_fences.contains_key(&stale_block),
            "test setup must evict the old bounded fence"
        );

        let late_report = state
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(writer.clone()),
                replicas: vec![pb::ReplicaReport {
                    block_id: stale_block.clone(),
                    length: 4,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                }],
                operation_id: b"retired/fence-evicted-report".to_vec(),
                desired_copies: 1,
                repair_id: Vec::new(),
            })
            .expect("late report is a rejected response");
        assert!(late_report.accepted.is_empty());
        assert_eq!(late_report.rejected_block_ids, vec![stale_block.clone()]);

        state.replicas.insert(
            stale_block.clone(),
            vec![StoredReplica {
                location: pb::ReplicaLocation {
                    block_id: stale_block.clone(),
                    node_id: writer.node_id,
                    node_epoch: writer.node_epoch,
                    data_endpoint: "http://127.0.0.1:19007".to_string(),
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                },
                catalog_revision: 9,
                length: 4,
            }],
        );
        assert!(matches!(
            state.commit_version(pb::CommitVersionRequest {
                context: None,
                session: Some(writer),
                key: Some(pb::Key {
                    value: b"retired/fence-evicted-key".to_vec(),
                }),
                candidate: Some(pb::VersionCandidate {
                    kind: pb::VersionKind::Value as i32,
                    logical_length: 4,
                    extents: vec![pb::ExtentRecord {
                        logical: Some(pb::ByteRange {
                            offset: 0,
                            length: 4,
                        }),
                        block_id: stale_block.clone(),
                        block_offset: 0,
                        digest: b"digest".to_vec(),
                        kind: pb::ExtentKind::Data as i32,
                    }],
                    digest: b"layout".to_vec(),
                }),
                condition: "any".to_string(),
                expected_version: None,
                operation_id: b"retired/fence-evicted-commit".to_vec(),
                operation_digest: b"retired/fence-evicted-digest".to_vec(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
                required_memory_copies: 1,
                replica_proofs: vec![pb::ReplicaProof {
                    block_id: stale_block,
                    node_id: 7,
                    node_epoch: 1,
                    catalog_revision: 9,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                }],
                new_replicas: Vec::new(),
                commit_sequence: next_test_commit_sequence(),
            }),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));
    }

    #[test]
    fn event_gc_waits_until_every_target_session_acknowledges_cursor() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 1 },
            MetaRetentionPolicy::default(),
        );
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"checkpoint/latest");
        let (writer_sender, mut writer_receiver) = mpsc::channel(1);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer.clone()),
                    last_acked_cursor: 0,
                },
                writer_sender,
            )
            .expect("writer watch");

        state
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"checkpoint/latest".to_vec(),
                b"block-event".to_vec(),
                b"op-event".to_vec(),
                b"digest-event".to_vec(),
            ))
            .expect("commit");
        assert_eq!(state.events.len(), 1);

        let event = state.events[0].clone();
        assert_eq!(invalidate_event(&event).source_node_id, writer.node_id);
        assert!(
            writer_receiver.try_recv().is_err(),
            "source Node already updates its local Current on commit and must not receive its own invalidation"
        );
        assert_eq!(state.events.len(), 1, "target reader has not ACKed yet");

        state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("reader ack");
        assert!(
            state.events.is_empty(),
            "event can be removed after every target session ACKs it"
        );
        assert_eq!(state.event_high_watermark, 2);
    }

    #[test]
    fn retry_result_does_not_advance_event_cursor_or_drop_replay() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"checkpoint/latest");
        state
            .commit_version(value_commit_request_for(
                writer,
                b"checkpoint/latest".to_vec(),
                b"block-event-retry".to_vec(),
                b"op-event-retry".to_vec(),
                b"digest-event-retry".to_vec(),
            ))
            .expect("commit");
        let event = state.events[0].clone();

        let error = state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader.clone()),
                event_id: event.event_id.clone(),
                cursor: event.cursor,
                result: "retry".to_string(),
                detail: Some("temporary peer failure".to_string()),
            })
            .expect_err("retry is not an acknowledgement");
        assert!(matches!(error, MetaRuntimeError::InvalidArgument(_)));
        assert_eq!(
            state
                .sessions
                .get(&reader.node_id)
                .expect("session")
                .last_acked_cursor,
            0
        );

        let (sender, mut receiver) = mpsc::channel(1);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .expect("reconnect watch");
        assert_eq!(
            receiver.try_recv().expect("replayed event").cursor,
            event.cursor
        );
    }

    #[test]
    fn operation_retention_uses_explicit_commit_index_window() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 1 },
            MetaRetentionPolicy {
                keep_versions_per_key: 64,
                operation_result_retention_records: 1,
                replica_operation_retention_records: 1,
                event_retention_requires_all_session_acks: true,
                drop_unreferenced_replicas: false,
            },
        );
        let session = test_session(&mut state, 7);

        for index in 1..=3 {
            let dispatch = state
                .dispatch_commit_version(value_commit_request_for(
                    session.clone(),
                    format!("k-{index}").into_bytes(),
                    format!("b-{index}").into_bytes(),
                    format!("op-{index}").into_bytes(),
                    format!("digest-{index}").into_bytes(),
                ))
                .expect("commit");
            assert!(dispatch.waiting_nodes.is_empty());
            state.complete_operation_visibility(&dispatch.operation_ids);
        }

        assert!(!state.operations.contains_key(b"op-1".as_slice()));
        assert!(state.operations.contains_key(b"op-2".as_slice()));
        assert!(state.operations.contains_key(b"op-3".as_slice()));
    }

    #[test]
    fn filesystem_operation_retention_uses_explicit_commit_index_window() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 1 },
            MetaRetentionPolicy {
                keep_versions_per_key: 64,
                operation_result_retention_records: 1,
                replica_operation_retention_records: 1,
                event_retention_requires_all_session_acks: true,
                drop_unreferenced_replicas: false,
            },
        );
        let session = test_session(&mut state, 8);
        let mut inode = create_test_file(&mut state, session.clone(), b"retained-operations.bin");

        for index in 1..=3 {
            let operation_id = format!("fs-op-{index}");
            let block_id = format!("fs-block-{index}");
            let response = state
                .filesystem_commit_version(filesystem_commit_request_for(
                    session.clone(),
                    &inode,
                    block_id.as_bytes(),
                    operation_id.as_bytes(),
                ))
                .expect("filesystem commit");
            state.complete_operation_visibility(&[operation_id.as_bytes().to_vec()]);
            inode = crate::filesystem::inode_from_proto(
                response
                    .resolved
                    .expect("resolved filesystem commit")
                    .inode
                    .expect("committed inode"),
            )
            .expect("valid committed inode");
        }

        assert!(
            !state
                .filesystem_version_operations
                .contains_key(b"fs-op-1".as_slice())
        );
        assert!(
            state
                .filesystem_version_operations
                .contains_key(b"fs-op-2".as_slice())
        );
        assert!(
            state
                .filesystem_version_operations
                .contains_key(b"fs-op-3".as_slice())
        );
    }

    #[test]
    fn filesystem_operation_retention_never_prunes_pending_visibility() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 0 },
            MetaRetentionPolicy {
                keep_versions_per_key: 64,
                operation_result_retention_records: 0,
                replica_operation_retention_records: 0,
                event_retention_requires_all_session_acks: true,
                drop_unreferenced_replicas: false,
            },
        );
        let writer = test_session(&mut state, 50);
        let _reader = test_session(&mut state, 51);
        let inode = create_test_file(&mut state, writer.clone(), b"pending-retention.bin");
        let operation_id = b"pending-retention-operation".to_vec();
        let dispatch = state
            .dispatch_filesystem_commit_version(filesystem_commit_request_for(
                writer,
                &inode,
                b"pending-retention-block",
                &operation_id,
            ))
            .expect("filesystem commit");
        assert!(dispatch.event_cursor.is_some());

        state.last_applied_index = dispatch.response.commit_index.saturating_add(10);
        state.retain_operations();
        assert!(
            state
                .filesystem_version_operations
                .contains_key(&operation_id)
        );

        state.complete_operation_visibility(std::slice::from_ref(&operation_id));
        state.retain_operations();
        assert!(
            !state
                .filesystem_version_operations
                .contains_key(&operation_id)
        );
    }

    #[test]
    fn filesystem_namespace_operation_retention_uses_explicit_commit_index_window() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 1 },
            MetaRetentionPolicy {
                keep_versions_per_key: 64,
                operation_result_retention_records: 1,
                replica_operation_retention_records: 1,
                event_retention_requires_all_session_acks: true,
                drop_unreferenced_replicas: false,
            },
        );
        let session = test_session(&mut state, 9);

        for index in 1..=3 {
            let operation_id = format!("namespace-op-{index}");
            state
                .filesystem_create_inode(filesystem_create_request_for(
                    session.clone(),
                    ROOT_INODE,
                    format!("file-{index}").as_bytes(),
                    pb::FilesystemInodeKind::RegularFile,
                    operation_id.as_bytes(),
                ))
                .expect("filesystem namespace operation");
            state.complete_operation_visibility(&[operation_id.into_bytes()]);
        }

        assert!(
            !state
                .filesystem_namespace_operations
                .contains_key(b"namespace-op-1".as_slice())
        );
        assert!(
            state
                .filesystem_namespace_operations
                .contains_key(b"namespace-op-2".as_slice())
        );
        assert!(
            state
                .filesystem_namespace_operations
                .contains_key(b"namespace-op-3".as_slice())
        );
    }

    #[tokio::test]
    async fn stats_report_journal_snapshot_and_catalog_retention_counts() {
        let handle = MetaHandle::try_spawn_with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 1 },
            MetaRetentionPolicy::default(),
        )
        .expect("spawn meta");
        let grant = handle
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .await
            .expect("session");
        let session = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        };
        handle
            .commit_version(value_commit_request_for(
                session,
                b"checkpoint/latest".to_vec(),
                b"block-stats".to_vec(),
                b"op-stats".to_vec(),
                b"digest-stats".to_vec(),
            ))
            .await
            .expect("commit");

        let stats = handle.stats().await.expect("stats");
        assert!(stats.journal_last_index >= 2);
        assert_eq!(stats.snapshot_index, stats.journal_last_index);
        assert_eq!(stats.version_count, 1);
        assert_eq!(stats.operation_count, 1);
        assert_eq!(stats.replica_block_count, 1);
        assert_eq!(stats.replica_location_count, 1);
        assert_eq!(
            stats.event_high_watermark, 1,
            "首次发布新 key 不产生失效事件，但仍消耗 cursor 以兼容旧 Journal ACK"
        );
    }

    #[test]
    fn unacknowledged_watch_event_is_replayed_after_reconnect() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let grant = state
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .expect("open node session");
        let writer = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        };
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"checkpoint/latest");
        let (first_sender, mut first_receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                first_sender,
            )
            .expect("register first watch");

        state
            .commit_version(pb::CommitVersionRequest {
                context: None,
                session: Some(writer),
                key: Some(pb::Key {
                    value: b"checkpoint/latest".to_vec(),
                }),
                candidate: Some(pb::VersionCandidate {
                    kind: pb::VersionKind::Value as i32,
                    logical_length: 4,
                    extents: vec![pb::ExtentRecord {
                        logical: Some(pb::ByteRange {
                            offset: 0,
                            length: 4,
                        }),
                        block_id: b"block-1".to_vec(),
                        block_offset: 0,
                        digest: b"digest".to_vec(),
                        kind: pb::ExtentKind::Data as i32,
                    }],
                    digest: b"layout".to_vec(),
                }),
                condition: "any".to_string(),
                expected_version: None,
                operation_id: b"client/op-1".to_vec(),
                operation_digest: b"request-1".to_vec(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
                required_memory_copies: 1,
                replica_proofs: Vec::new(),
                new_replicas: vec![pb::ReplicaReport {
                    block_id: b"block-1".to_vec(),
                    length: 4,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                }],
                commit_sequence: next_test_commit_sequence(),
            })
            .expect("commit value");
        let first = first_receiver.try_recv().expect("live watch event");
        assert_eq!(first.cursor, 2);
        drop(first_receiver);

        let (reconnect_sender, mut reconnect_receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader),
                    last_acked_cursor: 0,
                },
                reconnect_sender,
            )
            .expect("reconnect watch");
        let replay = reconnect_receiver.try_recv().expect("replayed event");
        assert_eq!(replay.cursor, first.cursor);
        assert_eq!(replay.event_id, first.event_id);
    }

    #[tokio::test]
    async fn commit_reply_waits_for_remote_node_watch_ack() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = state
            .open_node_session(7, "http://127.0.0.1:19200".to_string(), true)
            .expect("writer session");
        let reader = state
            .open_node_session(8, "http://127.0.0.1:19201".to_string(), true)
            .expect("reader session");
        let reader_session = pb::NodeSessionIdentity {
            session_id: reader.session_id,
            node_id: reader.node_id,
            node_epoch: reader.node_epoch,
        };
        let writer_session = pb::NodeSessionIdentity {
            session_id: writer.session_id,
            node_id: writer.node_id,
            node_epoch: writer.node_epoch,
        };
        seed_existing_key(&mut state, writer_session.clone(), b"checkpoint/latest");
        let (writer_event_sender, mut writer_event_receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer_session.clone()),
                    last_acked_cursor: 0,
                },
                writer_event_sender,
            )
            .expect("writer watch");
        let (event_sender, mut event_receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader_session.clone()),
                    last_acked_cursor: 0,
                },
                event_sender,
            )
            .expect("reader watch");

        let dispatch = state
            .dispatch_commit_version(value_commit_request(
                writer_session,
                b"client/op-remote-barrier".to_vec(),
            ))
            .expect("dispatch commit");
        assert_eq!(dispatch.waiting_nodes, HashSet::from([reader.node_id]));
        let expected_version = dispatch.response.version;
        let (reply, mut completion) = oneshot::channel();
        state.register_pending_commit(dispatch, reply);
        assert!(matches!(
            completion.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        assert!(
            writer_event_receiver.try_recv().is_err(),
            "writer does not need a Watch round trip for its own committed Current"
        );

        let event = event_receiver.try_recv().expect("remote invalidation");
        assert_eq!(invalidate_event(&event).source_node_id, writer.node_id);
        state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader_session),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("remote acknowledgement");
        let response = completion
            .await
            .expect("commit reply sender")
            .expect("commit response");
        assert_eq!(response.version, expected_version);
    }

    #[tokio::test]
    async fn retry_with_same_operation_id_cannot_bypass_visibility_ack() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"checkpoint/latest");
        let (event_sender, mut event_receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                event_sender,
            )
            .expect("reader watch");

        let request = value_commit_request(writer, b"client/op-retry-barrier".to_vec());
        let first = state
            .dispatch_commit_version(request.clone())
            .expect("first dispatch");
        let expected_version = first.response.version;
        let (first_reply, mut first_completion) = oneshot::channel();
        state.register_pending_commit(first, first_reply);

        let retry = state
            .dispatch_commit_version(request)
            .expect("idempotent retry dispatch");
        assert_eq!(retry.event_cursor, Some(2));
        assert_eq!(retry.waiting_nodes, HashSet::from([reader.node_id]));
        let (retry_reply, mut retry_completion) = oneshot::channel();
        state.register_pending_commit(retry, retry_reply);
        assert!(first_completion.try_recv().is_err());
        assert!(retry_completion.try_recv().is_err());

        let event = event_receiver.try_recv().expect("one invalidation event");
        assert!(
            event_receiver.try_recv().is_err(),
            "retry must not emit another event"
        );
        state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader),
                event_id: event.event_id,
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("reader acknowledgement");

        assert_eq!(
            first_completion
                .await
                .expect("first reply")
                .expect("first result")
                .version,
            expected_version
        );
        assert_eq!(
            retry_completion
                .await
                .expect("retry reply")
                .expect("retry result")
                .version,
            expected_version
        );
    }

    #[tokio::test]
    async fn snapshot_restore_preserves_the_visibility_barrier_for_operation_retry() {
        let mut before_restart = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut before_restart, 7);
        let reader = test_session(&mut before_restart, 8);
        seed_existing_key(&mut before_restart, writer.clone(), b"checkpoint/latest");
        let (original_sender, mut original_receiver) = mpsc::channel(4);
        before_restart
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                original_sender,
            )
            .expect("reader watch");

        let request =
            value_commit_request(writer.clone(), b"client/op-recovery-visibility".to_vec());
        let first = before_restart
            .dispatch_commit_version(request.clone())
            .expect("commit before restart");
        let expected_version = first.response.version;
        assert_eq!(first.event_cursor, Some(2));
        assert!(original_receiver.try_recv().is_ok());

        // A process restart drops pending oneshot replies and active Watch
        // senders, but snapshot state plus the retained invalidation event must
        // keep the durable operation in waiting_visibility.
        let snapshot = before_restart.snapshot();
        let mut restored = MetaState::new(Box::<InMemoryJournal>::default());
        restored.restore_snapshot(snapshot);
        restored
            .heartbeat(
                writer.session_id.clone(),
                writer.node_id,
                writer.node_epoch,
                0,
            )
            .expect("writer heartbeat after restore");
        restored
            .heartbeat(
                reader.session_id.clone(),
                reader.node_id,
                reader.node_epoch,
                0,
            )
            .expect("reader heartbeat after restore");
        let (replay_sender, mut replay_receiver) = mpsc::channel(4);
        restored
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(reader.clone()),
                    last_acked_cursor: 0,
                },
                replay_sender,
            )
            .expect("reader reconnect watch");

        let retry = restored
            .dispatch_commit_version(request)
            .expect("idempotent retry after restore");
        assert_eq!(retry.event_cursor, Some(2));
        assert_eq!(retry.waiting_nodes, HashSet::from([reader.node_id]));
        assert_eq!(
            retry.waiting_prior_lease_nodes,
            HashSet::from([writer.node_id, reader.node_id])
        );
        let (retry_reply, mut retry_completion) = oneshot::channel();
        restored.register_pending_commit(retry, retry_reply);
        assert!(matches!(
            retry_completion.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        let replay = replay_receiver.try_recv().expect("replayed invalidation");
        restored
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(reader),
                event_id: replay.event_id,
                cursor: replay.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("replayed event acknowledgement");
        assert!(
            matches!(
                retry_completion.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "恢复后的 Watch ACK 不能替代恢复前旧 lease fence"
        );
        for until in restored.prior_lease_deadlines.values_mut() {
            *until = Instant::now();
        }
        restored
            .retire_expired_sessions()
            .expect("expire restored prior leases");
        assert_eq!(
            retry_completion
                .await
                .expect("retry reply")
                .expect("retry result")
                .version,
            expected_version
        );
    }

    #[test]
    fn resolve_filters_stale_replica_after_node_reopen() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let first = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                first,
                b"checkpoint/latest".to_vec(),
                b"block-before-restart".to_vec(),
                b"op-before-restart".to_vec(),
                b"digest-before-restart".to_vec(),
            ))
            .expect("commit first incarnation");

        let second = test_session(&mut state, 7);
        assert_eq!(second.node_epoch, 2);
        let resolved = state
            .resolve_object(pb::ResolveObjectRequest {
                context: None,
                session: Some(second),
                key: Some(pb::Key {
                    value: b"checkpoint/latest".to_vec(),
                }),
                selector: None,
                range: None,
                cache_current: false,
            })
            .expect("resolve current layout");

        assert!(resolved.layout.is_some());
        assert_eq!(resolved.block_replicas.len(), 1);
        assert!(
            resolved.block_replicas[0].replicas.is_empty(),
            "a replica registered by the previous node_epoch must not survive node restart"
        );
    }

    #[test]
    fn resolve_current_cache_grants_lease_to_live_session() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        let committed = state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"checkpoint/latest".to_vec(),
                b"block-current-lease".to_vec(),
                b"op-current-lease".to_vec(),
                b"digest-current-lease".to_vec(),
            ))
            .expect("commit current value");

        let resolved = state
            .resolve_object(resolve_current_request(
                session.clone(),
                b"checkpoint/latest".to_vec(),
                true,
            ))
            .expect("resolve current");
        let lease = resolved
            .current_lease
            .expect("live current cache request should receive a lease");
        assert_eq!(lease.version, committed.version);
        assert_eq!(lease.revision, state.last_applied_index);
        assert_eq!(lease.lease_epoch, session.node_epoch);
        assert_eq!(lease.leader_epoch, 0);
        assert!(lease.ttl_millis > 0);
        assert!(lease.ttl_millis <= DEFAULT_NODE_LEASE_TTL.as_millis() as u64);
    }

    #[test]
    fn resolve_current_lease_requires_current_selector_and_cache_request() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        let committed = state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"checkpoint/latest".to_vec(),
                b"block-current-lease-mode".to_vec(),
                b"op-current-lease-mode".to_vec(),
                b"digest-current-lease-mode".to_vec(),
            ))
            .expect("commit current value");

        let no_cache = state
            .resolve_object(resolve_current_request(
                session.clone(),
                b"checkpoint/latest".to_vec(),
                false,
            ))
            .expect("resolve without cache request");
        assert!(no_cache.current_lease.is_none());

        let exact = state
            .resolve_object(pb::ResolveObjectRequest {
                context: None,
                session: Some(session),
                key: Some(pb::Key {
                    value: b"checkpoint/latest".to_vec(),
                }),
                selector: Some(pb::resolve_object_request::Selector::ExactVersion(
                    committed.version,
                )),
                range: None,
                cache_current: true,
            })
            .expect("resolve exact version");
        assert!(exact.current_lease.is_none());
    }

    #[test]
    fn resolve_current_lease_requires_live_non_retired_session() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 7);
        let expired = test_session(&mut state, 8);
        let retired = test_session(&mut state, 9);
        state
            .commit_version(value_commit_request_for(
                writer,
                b"checkpoint/latest".to_vec(),
                b"block-current-lease-live".to_vec(),
                b"op-current-lease-live".to_vec(),
                b"digest-current-lease-live".to_vec(),
            ))
            .expect("commit current value");

        expire_session(&mut state, expired.node_id);
        let expired_resolve = state
            .resolve_object(resolve_current_request(
                expired,
                b"checkpoint/latest".to_vec(),
                true,
            ))
            .expect("expired requester may resolve layout but cannot cache current");
        assert!(expired_resolve.current_lease.is_none());

        state.retired_sessions.insert(retired.node_id);
        let retired_resolve = state
            .resolve_object(resolve_current_request(
                retired,
                b"checkpoint/latest".to_vec(),
                true,
            ))
            .expect("retired requester may resolve layout but cannot cache current");
        assert!(retired_resolve.current_lease.is_none());
    }

    #[test]
    fn resolve_current_lease_rejects_reopened_old_epoch_and_grants_new_epoch() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let old = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                old.clone(),
                b"checkpoint/latest".to_vec(),
                b"block-before-reopen-lease".to_vec(),
                b"op-before-reopen-lease".to_vec(),
                b"digest-before-reopen-lease".to_vec(),
            ))
            .expect("commit first incarnation");

        let current = test_session(&mut state, 7);
        assert_eq!(current.node_epoch, old.node_epoch + 1);
        assert!(matches!(
            state.resolve_object(resolve_current_request(
                old,
                b"checkpoint/latest".to_vec(),
                true,
            )),
            Err(MetaRuntimeError::UnknownSession)
        ));

        let current_resolve = state
            .resolve_object(resolve_current_request(
                current,
                b"checkpoint/latest".to_vec(),
                true,
            ))
            .expect("current incarnation may resolve");
        assert!(current_resolve.current_lease.is_some());
    }

    #[test]
    fn plan_replicas_excludes_source_session_and_existing_replicas() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let source = test_session(&mut state, 1);
        let existing = test_session(&mut state, 2);
        let candidate = test_session(&mut state, 3);

        let plan = state
            .plan_replicas(pb::PlanReplicasRequest {
                context: None,
                session: Some(source),
                block_id: b"block-to-copy".to_vec(),
                length: 10,
                durability: pb::DurabilityPolicy::LocalMemory as i32,
                desired_copies: 3,
                existing_node_ids: vec![existing.node_id],
                required_transports: vec!["grpc".to_string()],
            })
            .expect("plan replicas");

        assert!(!plan.plan_id.is_empty());
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].node_id, candidate.node_id);
        assert_eq!(plan.targets[0].transports, vec!["grpc"]);
    }

    #[test]
    fn plan_replicas_treats_desired_copies_as_total_copy_count() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let source = test_session(&mut state, 1);
        let candidate = test_session(&mut state, 2);

        let plan = state
            .plan_replicas(pb::PlanReplicasRequest {
                context: None,
                session: Some(source.clone()),
                block_id: b"block-one-to-two".to_vec(),
                length: 10,
                durability: pb::DurabilityPolicy::LocalMemory as i32,
                desired_copies: 2,
                existing_node_ids: Vec::new(),
                required_transports: vec!["grpc".to_string()],
            })
            .expect("plan 1 -> 2 copies");

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].node_id, candidate.node_id);

        let already_enough = state
            .plan_replicas(pb::PlanReplicasRequest {
                context: None,
                session: Some(source),
                block_id: b"block-enough".to_vec(),
                length: 10,
                durability: pb::DurabilityPolicy::LocalMemory as i32,
                desired_copies: 2,
                existing_node_ids: vec![candidate.node_id],
                required_transports: vec!["grpc".to_string()],
            })
            .expect("existing live copies already satisfy policy");

        assert!(already_enough.targets.is_empty());
    }

    #[test]
    fn lease_expiry_filters_resolve_and_placement_candidates() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let source = test_session(&mut state, 1);
        let expired = test_session(&mut state, 2);
        let live = test_session(&mut state, 3);

        state
            .commit_version(value_commit_request_for(
                expired.clone(),
                b"checkpoint/latest".to_vec(),
                b"block-expired".to_vec(),
                b"op-expired".to_vec(),
                b"digest-expired".to_vec(),
            ))
            .expect("commit on soon-expired node");
        expire_session(&mut state, expired.node_id);

        let resolved = state
            .resolve_object(pb::ResolveObjectRequest {
                context: None,
                session: Some(source.clone()),
                key: Some(pb::Key {
                    value: b"checkpoint/latest".to_vec(),
                }),
                selector: None,
                range: None,
                cache_current: false,
            })
            .expect("resolve layout");
        assert!(resolved.block_replicas[0].replicas.is_empty());

        let plan = state
            .plan_replicas(pb::PlanReplicasRequest {
                context: None,
                session: Some(source),
                block_id: b"block-expired".to_vec(),
                length: 4,
                durability: pb::DurabilityPolicy::LocalMemory as i32,
                desired_copies: 2,
                existing_node_ids: vec![expired.node_id],
                required_transports: vec!["grpc".to_string()],
            })
            .expect("plan avoids expired target");

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].node_id, live.node_id);
    }

    #[test]
    fn replica_metrics_count_live_replicas_only() {
        let registry = dms_metrics::registry();
        let metrics = MetaMetrics::register(&registry).expect("metrics");
        let mut state = MetaState::try_new(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy::default(),
            MetaRetentionPolicy::default(),
            metrics,
        )
        .expect("state");
        let owner = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                owner.clone(),
                b"live-metric-key".to_vec(),
                b"live-metric-block".to_vec(),
                b"live-metric-op".to_vec(),
                b"live-metric-digest".to_vec(),
            ))
            .expect("commit");

        state.refresh_metrics();
        let before = state.stats();
        assert_eq!(before.replica_block_count, 1);
        assert_eq!(before.replica_location_count, 1);
        let before_text = dms_metrics::encode_text(&registry).expect("encode before");
        assert!(
            before_text.contains("dms_meta_state_items{type=\"replicas\"} 1\n"),
            "live replica must be counted before lease expiry: {before_text}"
        );
        assert!(
            before_text.contains("dms_meta_blocks{state=\"healthy\"} 1\n"),
            "live replica must satisfy the desired copy count: {before_text}"
        );

        expire_session(&mut state, owner.node_id);
        state.refresh_metrics();
        let after = state.stats();
        assert_eq!(
            after.replica_block_count, 0,
            "stored catalog facts remain, but no block has a live replica"
        );
        assert_eq!(after.replica_location_count, 0);
        let after_text = dms_metrics::encode_text(&registry).expect("encode after");
        assert!(
            after_text.contains("dms_meta_state_items{type=\"replicas\"} 0\n"),
            "Prometheus replica gauge must use live locations only: {after_text}"
        );
        assert!(
            after_text.contains("dms_meta_blocks{state=\"healthy\"} 0\n"),
            "expired replica must not remain healthy: {after_text}"
        );
        assert!(
            after_text.contains("dms_meta_blocks{state=\"unavailable\"} 1\n"),
            "desired block with zero live replicas must be unavailable: {after_text}"
        );
    }

    #[test]
    fn replica_report_records_the_verified_session_node_epoch() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 6);
        let session = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                writer,
                b"session-owned-key".to_vec(),
                b"session-owned-block".to_vec(),
                b"session-owned-commit".to_vec(),
                b"session-owned-digest".to_vec(),
            ))
            .expect("publish referenced block");
        let response = state
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(session.clone()),
                replicas: vec![pb::ReplicaReport {
                    block_id: b"session-owned-block".to_vec(),
                    length: 4,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                }],
                operation_id: b"session-owned-report".to_vec(),
                desired_copies: 1,
                repair_id: Vec::new(),
            })
            .expect("valid replica report");

        assert_eq!(response.accepted.len(), 1);
        assert!(response.rejected_block_ids.is_empty());
        let location = state
            .replicas
            .get(b"session-owned-block".as_slice())
            .and_then(|replicas| {
                replicas
                    .iter()
                    .find(|replica| replica.location.node_id == session.node_id)
            })
            .expect("reported replica location");
        assert_eq!(location.location.node_id, session.node_id);
        assert_eq!(location.location.node_epoch, session.node_epoch);
    }

    #[test]
    fn replica_report_batch_publishes_multiple_blocks_with_one_journal_record() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut state, 6);
        let reader = test_session(&mut state, 7);
        for index in 0..2 {
            state
                .commit_version(value_commit_request_for(
                    writer.clone(),
                    format!("batch-key-{index}").into_bytes(),
                    format!("batch-block-{index}").into_bytes(),
                    format!("batch-commit-{index}").into_bytes(),
                    format!("batch-digest-{index}").into_bytes(),
                ))
                .expect("publish referenced block");
        }
        let before_report = state.journal.last_index();

        let response = state
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(reader.clone()),
                replicas: (0..2)
                    .map(|index| pb::ReplicaReport {
                        block_id: format!("batch-block-{index}").into_bytes(),
                        length: 4,
                        checksum: format!("batch-digest-{index}").into_bytes(),
                        durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
                    })
                    .collect(),
                operation_id: b"replica-report-batch".to_vec(),
                desired_copies: 2,
                repair_id: Vec::new(),
            })
            .expect("batch report");

        assert_eq!(response.accepted.len(), 2);
        assert!(response.rejected_block_ids.is_empty());
        assert_eq!(
            state.journal.last_index(),
            before_report + 1,
            "one batch must append one atomic journal record"
        );
        for index in 0..2 {
            let block_id = format!("batch-block-{index}").into_bytes();
            assert!(
                state
                    .replicas
                    .get(block_id.as_slice())
                    .is_some_and(|replicas| replicas
                        .iter()
                        .any(|replica| replica.location.node_id == reader.node_id))
            );
        }
    }

    #[test]
    fn lease_expiry_schedules_one_targeted_repair_for_established_replica_count() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let first = test_session(&mut state, 1);
        let second = test_session(&mut state, 2);
        let target = test_session(&mut state, 3);
        state
            .commit_version(value_commit_request_for(
                first.clone(),
                b"checkpoint/latest".to_vec(),
                b"repair-block".to_vec(),
                b"repair-op-1".to_vec(),
                b"repair-digest-1".to_vec(),
            ))
            .expect("first replica");
        state
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(second),
                replicas: vec![pb::ReplicaReport {
                    block_id: b"repair-block".to_vec(),
                    length: 4,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
                }],
                operation_id: b"repair-op-2".to_vec(),
                desired_copies: 2,
                repair_id: Vec::new(),
            })
            .expect("second replica");
        expire_session(&mut state, first.node_id);
        state
            .heartbeat(
                target.session_id.clone(),
                target.node_id,
                target.node_epoch,
                0,
            )
            .expect("heartbeat schedules repair");

        let repairs = state
            .events
            .iter()
            .filter_map(|event| match &event.event {
                Some(pb::node_event::Event::RepairReplica(repair)) => Some(repair),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0].target.as_ref().expect("target").node_id, 3);
        assert_eq!(repairs[0].sources.len(), 1);
        assert_eq!(repairs[0].sources[0].node_id, 2);
        assert_eq!(repairs[0].expected_length, 4);
        assert_eq!(repairs[0].desired_copies, 2);

        state
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(target.clone()),
                replicas: vec![pb::ReplicaReport {
                    block_id: b"repair-block".to_vec(),
                    length: 4,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
                }],
                operation_id: b"repair-op-target".to_vec(),
                desired_copies: repairs[0].desired_copies,
                repair_id: repairs[0].repair_id.clone(),
            })
            .expect("replacement report");
        assert_eq!(
            state.desired_replica_counts.get(b"repair-block".as_slice()),
            Some(&2),
            "replacement location must not ratchet the policy to three"
        );

        state.schedule_repairs();
        assert_eq!(
            state
                .events
                .iter()
                .filter(|event| matches!(
                    event.event,
                    Some(pb::node_event::Event::RepairReplica(_))
                ))
                .count(),
            1,
            "pending repair suppresses duplicates"
        );

        let read_repair = |receiver: &mut mpsc::Receiver<pb::NodeEvent>| {
            std::iter::from_fn(|| receiver.try_recv().ok())
                .find(|event| matches!(event.event, Some(pb::node_event::Event::RepairReplica(_))))
                .expect("repair event")
        };
        let (first_sender, mut first_receiver) = mpsc::channel(8);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(target.clone()),
                    last_acked_cursor: 0,
                },
                first_sender,
            )
            .expect("first repair watch");
        let first_repair = read_repair(&mut first_receiver);
        drop(first_receiver);

        let (retry_sender, mut retry_receiver) = mpsc::channel(8);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(target),
                    last_acked_cursor: 0,
                },
                retry_sender,
            )
            .expect("repair reconnect");
        let replayed_repair = read_repair(&mut retry_receiver);
        assert_eq!(replayed_repair.cursor, first_repair.cursor);
        assert_eq!(replayed_repair.event_id, first_repair.event_id);
    }

    #[test]
    fn dead_repair_target_is_reassigned_and_its_late_report_is_rejected() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let first = test_session(&mut state, 1);
        let second = test_session(&mut state, 2);
        let third = test_session(&mut state, 3);
        let fourth = test_session(&mut state, 4);
        state
            .commit_version(value_commit_request_for(
                first.clone(),
                b"checkpoint/retarget".to_vec(),
                b"repair-retarget-block".to_vec(),
                b"repair-retarget-op-1".to_vec(),
                b"repair-retarget-digest-1".to_vec(),
            ))
            .expect("first replica");
        state
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(second),
                replicas: vec![pb::ReplicaReport {
                    block_id: b"repair-retarget-block".to_vec(),
                    length: 4,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
                }],
                operation_id: b"repair-retarget-op-2".to_vec(),
                desired_copies: 2,
                repair_id: Vec::new(),
            })
            .expect("second replica");

        expire_session(&mut state, first.node_id);
        state.schedule_repairs();
        let first_attempt = state
            .events
            .iter()
            .find_map(|event| match &event.event {
                Some(pb::node_event::Event::RepairReplica(repair)) => Some(repair.clone()),
                _ => None,
            })
            .expect("first repair attempt");
        let first_target = first_attempt.target.clone().expect("first target");
        let stale_target_session = if first_target.node_id == third.node_id {
            third.clone()
        } else {
            fourth.clone()
        };
        expire_session(&mut state, first_target.node_id);

        state.schedule_repairs();
        let replacement = state
            .events
            .iter()
            .find_map(|event| match &event.event {
                Some(pb::node_event::Event::RepairReplica(repair)) => Some(repair.clone()),
                _ => None,
            })
            .expect("replacement repair attempt");
        let replacement_target = replacement.target.clone().expect("replacement target");
        assert_ne!(replacement.repair_id, first_attempt.repair_id);
        assert_ne!(replacement_target.node_id, first_target.node_id);

        let stale_report = state.report_replicas(pb::ReportReplicasRequest {
            context: None,
            session: Some(stale_target_session),
            replicas: vec![pb::ReplicaReport {
                block_id: b"repair-retarget-block".to_vec(),
                length: 4,
                checksum: b"digest".to_vec(),
                durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
            }],
            operation_id: b"late-repair-report".to_vec(),
            desired_copies: 2,
            repair_id: first_attempt.repair_id,
        });
        assert!(matches!(
            stale_report,
            Err(MetaRuntimeError::Conflict { .. })
        ));

        let replacement_session = if replacement_target.node_id == third.node_id {
            third
        } else {
            fourth
        };
        state
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(replacement_session),
                replicas: vec![pb::ReplicaReport {
                    block_id: b"repair-retarget-block".to_vec(),
                    length: 4,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
                }],
                operation_id: b"replacement-repair-report".to_vec(),
                desired_copies: 2,
                repair_id: replacement.repair_id,
            })
            .expect("current target report is accepted");
    }

    #[test]
    fn repair_can_reuse_a_restarted_node_with_a_new_epoch() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let first = test_session(&mut state, 1);
        let second = test_session(&mut state, 2);
        state
            .commit_version(value_commit_request_for(
                first.clone(),
                b"checkpoint/latest".to_vec(),
                b"repair-restart-block".to_vec(),
                b"repair-restart-op-1".to_vec(),
                b"repair-restart-digest-1".to_vec(),
            ))
            .expect("first replica");
        state
            .report_replicas(pb::ReportReplicasRequest {
                context: None,
                session: Some(second),
                replicas: vec![pb::ReplicaReport {
                    block_id: b"repair-restart-block".to_vec(),
                    length: 4,
                    checksum: b"digest".to_vec(),
                    durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
                }],
                operation_id: b"repair-restart-op-2".to_vec(),
                desired_copies: 2,
                repair_id: Vec::new(),
            })
            .expect("second replica");
        expire_session(&mut state, first.node_id);

        let restarted = test_session(&mut state, first.node_id);
        assert!(restarted.node_epoch > first.node_epoch);
        state
            .heartbeat(
                restarted.session_id,
                restarted.node_id,
                restarted.node_epoch,
                0,
            )
            .expect("heartbeat schedules repair to restarted node");

        let repair = state
            .events
            .iter()
            .find_map(|event| match &event.event {
                Some(pb::node_event::Event::RepairReplica(repair)) => Some(repair),
                _ => None,
            })
            .expect("repair event");
        let target = repair.target.as_ref().expect("repair target");
        assert_eq!(target.node_id, first.node_id);
        assert_eq!(target.node_epoch, restarted.node_epoch);
        assert_eq!(repair.desired_copies, 2);
    }

    #[test]
    fn commit_batch_is_all_or_nothing_and_retry_is_idempotent() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        let seed = state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"batch/a".to_vec(),
                b"batch/a-v1".to_vec(),
                b"batch/seed".to_vec(),
                b"batch/seed-digest".to_vec(),
            ))
            .expect("seed");

        let invalid = batch_request(
            session.clone(),
            b"batch-1".to_vec(),
            vec![
                batch_entry(b"batch/a", b"batch/a-v2", b"batch/op-a", "if-version:99"),
                batch_entry(b"batch/b", b"batch/b-v1", b"batch/op-b", "any"),
            ],
        );
        assert!(matches!(
            state.commit_batch(invalid),
            Err(MetaRuntimeError::Conflict { .. })
        ));
        assert_eq!(state.versions[b"batch/a".as_slice()].len(), 1);
        assert!(!state.versions.contains_key(b"batch/b".as_slice()));

        let valid = batch_request(
            session,
            b"batch-2".to_vec(),
            vec![
                batch_entry(
                    b"batch/a",
                    b"batch/a-v2",
                    b"batch/op-a2",
                    &format!("if-version:{}", seed.version),
                ),
                batch_entry(b"batch/b", b"batch/b-v1", b"batch/op-b2", "any"),
            ],
        );
        let first = state.commit_batch(valid.clone()).expect("batch commit");
        let retry = state.commit_batch(valid).expect("batch retry");
        assert_eq!(first, retry);
        assert_eq!(first.results.len(), 2);
        assert!(first.results.iter().all(|result| {
            result.result.as_ref().expect("result").commit_index == first.commit_index
        }));
        assert_eq!(state.versions[b"batch/a".as_slice()].len(), 2);
        assert_eq!(state.versions[b"batch/b".as_slice()].len(), 1);
    }

    #[test]
    fn stat_reports_logical_current_metadata_without_replica_liveness_probe() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        let committed = state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"stat/live".to_vec(),
                b"stat/block".to_vec(),
                b"stat/op".to_vec(),
                b"stat/digest".to_vec(),
            ))
            .expect("commit value");
        let first = state
            .stat(meta_stat_request(session.clone(), b"stat/live"))
            .expect("stat")
            .info
            .expect("info");
        assert_eq!(first.key.as_ref().unwrap().value, b"stat/live");
        assert_eq!(first.length, 4);
        assert_eq!(first.version, committed.version);
        assert!(first.modified_time_unix_millis > 0);

        expire_session(&mut state, 7);
        let offline = state
            .stat(meta_stat_request(session.clone(), b"stat/live"))
            .expect("stat does not prove replica reachability")
            .info
            .expect("info");
        assert_eq!(offline, first);

        state
            .commit_version(tombstone_request_for(
                session.clone(),
                b"stat/live".to_vec(),
                b"stat/delete".to_vec(),
                b"stat/delete-digest".to_vec(),
            ))
            .expect("delete");
        let missing = state
            .stat(meta_stat_request(session, b"stat/live"))
            .expect("tombstone is logical miss");
        assert!(!missing.found);
        assert!(missing.info.is_none());
    }

    #[test]
    fn idempotent_retry_and_snapshot_restore_preserve_modified_time() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        let request = value_commit_request_for(
            session.clone(),
            b"mtime/stable".to_vec(),
            b"mtime/block".to_vec(),
            b"mtime/op".to_vec(),
            b"mtime/digest".to_vec(),
        );
        state.commit_version(request.clone()).expect("commit");
        let first = state
            .stat(meta_stat_request(session.clone(), b"mtime/stable"))
            .unwrap()
            .info
            .unwrap()
            .modified_time_unix_millis;
        std::thread::sleep(Duration::from_millis(2));
        state.commit_version(request).expect("idempotent retry");
        let retry = state
            .stat(meta_stat_request(session.clone(), b"mtime/stable"))
            .unwrap()
            .info
            .unwrap()
            .modified_time_unix_millis;
        assert_eq!(retry, first);

        let mut journal = InMemoryJournal::default();
        journal.save_snapshot(state.snapshot()).unwrap();
        let restored = MetaState::new(Box::new(journal));
        let restored_mtime = restored
            .stat(meta_stat_request(session, b"mtime/stable"))
            .unwrap()
            .info
            .unwrap()
            .modified_time_unix_millis;
        assert_eq!(restored_mtime, first);
    }

    #[test]
    fn scan_uses_binary_prefix_order_with_bounded_cursor_pages() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        for (index, key) in [
            b"scan/\x00".as_slice(),
            b"scan/a".as_slice(),
            b"scan/a/child".as_slice(),
            b"scan/b".as_slice(),
            b"scan/\xff".as_slice(),
            b"other/z".as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            state
                .commit_version(value_commit_request_for(
                    session.clone(),
                    key.to_vec(),
                    [b"scan-block/".as_slice(), key].concat(),
                    format!("scan-op-{index}").into_bytes(),
                    format!("scan-digest-{index}").into_bytes(),
                ))
                .expect("seed key");
        }
        state
            .commit_version(tombstone_request_for(
                session.clone(),
                b"scan/a/child".to_vec(),
                b"scan/delete".to_vec(),
                b"scan/delete-digest".to_vec(),
            ))
            .expect("delete");

        let marker_before_prefix = state
            .scan(meta_scan_request(
                session.clone(),
                b"scan/",
                Some(b"other/z".to_vec()),
                "",
                2,
            ))
            .expect("marker before prefix");
        let keys = marker_before_prefix
            .items
            .iter()
            .map(|item| item.key.as_ref().unwrap().value.as_slice())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![b"scan/\x00".as_slice(), b"scan/a".as_slice()]);

        let first = state
            .scan(meta_scan_request(
                session.clone(),
                b"scan/",
                Some(b"scan/\x00".to_vec()),
                "",
                2,
            ))
            .expect("first page");
        let keys = first
            .items
            .iter()
            .map(|item| item.key.as_ref().unwrap().value.as_slice())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![b"scan/a".as_slice(), b"scan/b".as_slice()]);
        assert!(!first.next_cursor.is_empty());

        let second = state
            .scan(meta_scan_request(
                session.clone(),
                b"scan/",
                None,
                &first.next_cursor,
                2,
            ))
            .expect("second page");
        let keys = second
            .items
            .iter()
            .map(|item| item.key.as_ref().unwrap().value.as_slice())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![b"scan/\xff".as_slice()]);
        assert!(second.next_cursor.is_empty());

        let marker_after_prefix = state
            .scan(meta_scan_request(
                session.clone(),
                b"scan/",
                Some(b"scan0".to_vec()),
                "",
                2,
            ))
            .expect("marker after prefix");
        assert!(marker_after_prefix.items.is_empty());
        assert!(marker_after_prefix.next_cursor.is_empty());

        let mismatched_cursor_last_key = ScanCursor {
            prefix: b"scan/".to_vec(),
            delimiter: Vec::new(),
            last_key: b"wrong/key".to_vec(),
            expires_at_unix_millis: current_time_unix_millis() + SCAN_CURSOR_TTL_MILLIS,
        }
        .encode();
        assert!(matches!(
            state.scan(meta_scan_request(
                session.clone(),
                b"scan/",
                None,
                &mismatched_cursor_last_key,
                2,
            )),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));

        let oversized_cursor = "x".repeat(MAX_SCAN_CURSOR_LENGTH + 1);
        assert!(matches!(
            state.scan(meta_scan_request(
                session.clone(),
                b"scan/",
                None,
                &oversized_cursor,
                2,
            )),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));

        assert!(matches!(
            state.scan(meta_scan_request(
                session.clone(),
                b"scan/",
                Some(b"scan/a".to_vec()),
                &first.next_cursor,
                2,
            )),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));
        assert!(matches!(
            state.scan(meta_scan_request(
                session,
                b"wrong/",
                None,
                &first.next_cursor,
                2
            )),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));
    }

    #[test]
    fn scan_groups_delimiter_prefixes_without_repeating_limit_one_pages() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        for (index, key) in [
            b"p/a".as_slice(),
            b"p/dir/".as_slice(),
            b"p/dir/child".as_slice(),
            b"p/dir/grand/leaf".as_slice(),
            b"p/dir2/file".as_slice(),
            b"p/z".as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            state
                .commit_version(value_commit_request_for(
                    session.clone(),
                    key.to_vec(),
                    [b"scan-delim-block/".as_slice(), key].concat(),
                    format!("scan-delim-op-{index}").into_bytes(),
                    format!("scan-delim-digest-{index}").into_bytes(),
                ))
                .expect("seed delimiter key");
        }

        let mut cursor = String::new();
        let mut seen = Vec::new();
        loop {
            let page = state
                .scan(meta_scan_request_with_delimiter(
                    session.clone(),
                    b"p/",
                    None,
                    &cursor,
                    b"/",
                    1,
                ))
                .expect("delimiter page");
            assert_eq!(page.items.len(), 1);
            let item = &page.items[0];
            seen.push((
                item.key.as_ref().expect("key").value.clone(),
                item.is_prefix,
            ));
            if page.next_cursor.is_empty() {
                break;
            }
            cursor = page.next_cursor;
        }

        assert_eq!(
            seen,
            vec![
                (b"p/a".to_vec(), false),
                (b"p/dir/".to_vec(), true),
                (b"p/dir2/".to_vec(), true),
                (b"p/z".to_vec(), false),
            ]
        );

        let nested = state
            .scan(meta_scan_request_with_delimiter(
                session, b"p/dir/", None, "", b"/", 10,
            ))
            .expect("nested delimiter page");
        let nested_items = nested
            .items
            .iter()
            .map(|item| {
                (
                    item.key.as_ref().expect("key").value.clone(),
                    item.is_prefix,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            nested_items,
            vec![
                (b"p/dir/".to_vec(), false),
                (b"p/dir/child".to_vec(), false),
                (b"p/dir/grand/".to_vec(), true),
            ]
        );
        assert!(nested.next_cursor.is_empty());
    }

    #[test]
    fn scan_delimiter_cursor_binds_semantics_and_skips_marker_inside_group() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        for (index, key) in [
            b"p/dir/a".as_slice(),
            b"p/dir/b".as_slice(),
            b"p/next".as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            state
                .commit_version(value_commit_request_for(
                    session.clone(),
                    key.to_vec(),
                    [b"scan-marker-block/".as_slice(), key].concat(),
                    format!("scan-marker-op-{index}").into_bytes(),
                    format!("scan-marker-digest-{index}").into_bytes(),
                ))
                .expect("seed marker key");
        }

        let marker_inside_group = state
            .scan(meta_scan_request_with_delimiter(
                session.clone(),
                b"p/",
                Some(b"p/dir/a".to_vec()),
                "",
                b"/",
                10,
            ))
            .expect("marker inside group");
        let keys = marker_inside_group
            .items
            .iter()
            .map(|item| item.key.as_ref().expect("key").value.as_slice())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![b"p/next".as_slice()]);

        let first = state
            .scan(meta_scan_request_with_delimiter(
                session.clone(),
                b"p/",
                None,
                "",
                b"/",
                1,
            ))
            .expect("first grouped page");
        assert_eq!(
            first.items[0].key.as_ref().expect("key").value,
            b"p/dir/".to_vec()
        );
        assert!(first.items[0].is_prefix);
        assert!(!first.next_cursor.is_empty());

        assert!(matches!(
            state.scan(meta_scan_request_with_delimiter(
                session.clone(),
                b"p/",
                None,
                &first.next_cursor,
                b"::",
                1,
            )),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));
        assert!(matches!(
            state.scan(meta_scan_request_with_delimiter(
                session,
                b"wrong/",
                None,
                &first.next_cursor,
                b"/",
                1,
            )),
            Err(MetaRuntimeError::InvalidArgument(_))
        ));
    }

    #[test]
    fn scan_groups_multibyte_delimiter_and_all_ff_successor_boundary() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        for (index, key) in [
            b"m/a::x".as_slice(),
            b"m/a::y".as_slice(),
            b"m/b".as_slice(),
            b"\xff/a".as_slice(),
            b"\xff/b".as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            state
                .commit_version(value_commit_request_for(
                    session.clone(),
                    key.to_vec(),
                    [b"scan-binary-block/".as_slice(), key].concat(),
                    format!("scan-binary-op-{index}").into_bytes(),
                    format!("scan-binary-digest-{index}").into_bytes(),
                ))
                .expect("seed binary key");
        }

        let multibyte = state
            .scan(meta_scan_request_with_delimiter(
                session.clone(),
                b"m/",
                None,
                "",
                b"::",
                10,
            ))
            .expect("multibyte delimiter");
        let multibyte_items = multibyte
            .items
            .iter()
            .map(|item| {
                (
                    item.key.as_ref().expect("key").value.clone(),
                    item.is_prefix,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            multibyte_items,
            vec![(b"m/a::".to_vec(), true), (b"m/b".to_vec(), false)]
        );
        assert!(multibyte.next_cursor.is_empty());

        let all_ff = state
            .scan(meta_scan_request_with_delimiter(
                session,
                b"",
                Some(vec![0xfe]),
                "",
                b"\xff",
                1,
            ))
            .expect("all-ff delimiter group");
        assert_eq!(all_ff.items.len(), 1);
        assert_eq!(
            all_ff.items[0].key.as_ref().expect("key").value,
            b"\xff".to_vec()
        );
        assert!(all_ff.items[0].is_prefix);
        assert!(all_ff.next_cursor.is_empty());
    }

    #[test]
    fn restored_sessions_start_suspect_until_same_epoch_heartbeat() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        state
            .commit_version(value_commit_request_for(
                session.clone(),
                b"checkpoint/latest".to_vec(),
                b"block-before-meta-restart".to_vec(),
                b"op-before-meta-restart".to_vec(),
                b"digest-before-meta-restart".to_vec(),
            ))
            .expect("commit");

        let mut restored = MetaState::new(state.journal);
        let suspect_resolve = restored
            .resolve_object(pb::ResolveObjectRequest {
                context: None,
                session: Some(session.clone()),
                key: Some(pb::Key {
                    value: b"checkpoint/latest".to_vec(),
                }),
                selector: None,
                range: None,
                cache_current: false,
            })
            .expect("resolve layout while session is suspect");
        assert!(suspect_resolve.block_replicas[0].replicas.is_empty());

        let recovery_heartbeat = restored
            .heartbeat(
                session.session_id.clone(),
                session.node_id,
                session.node_epoch,
                0,
            )
            .expect("same epoch heartbeat revives session");
        assert!(
            recovery_heartbeat.filesystem_lock_reclaim_required,
            "Meta 恢复后必须要求仍使用原 session 的 Node 重报完整锁镜像"
        );
        restored
            .filesystem_reclaim_locks(pb::FilesystemReclaimLocksRequest {
                context: None,
                session: Some(session.clone()),
                locks: Vec::new(),
            })
            .expect("empty lock snapshot finishes recovery");
        assert!(
            !restored
                .heartbeat(
                    session.session_id.clone(),
                    session.node_id,
                    session.node_epoch,
                    0,
                )
                .expect("heartbeat after reclaim")
                .filesystem_lock_reclaim_required,
            "同一轮完整快照只能被要求一次"
        );
        let live_resolve = restored
            .resolve_object(pb::ResolveObjectRequest {
                context: None,
                session: Some(session),
                key: Some(pb::Key {
                    value: b"checkpoint/latest".to_vec(),
                }),
                selector: None,
                range: None,
                cache_current: false,
            })
            .expect("resolve after heartbeat");
        assert_eq!(live_resolve.block_replicas[0].replicas.len(), 1);
    }

    fn value_commit_request(
        session: pb::NodeSessionIdentity,
        operation_id: Vec<u8>,
    ) -> pb::CommitVersionRequest {
        value_commit_request_for(
            session,
            b"checkpoint/latest".to_vec(),
            b"block-remote".to_vec(),
            operation_id,
            b"request-remote".to_vec(),
        )
    }

    fn batch_request(
        session: pb::NodeSessionIdentity,
        batch_operation_id: Vec<u8>,
        entries: Vec<pb::BatchCommitEntry>,
    ) -> pb::CommitBatchRequest {
        pb::CommitBatchRequest {
            context: None,
            session: Some(session),
            entries,
            batch_operation_id,
            commit_sequence: next_test_commit_sequence(),
        }
    }

    fn batch_entry(
        key: &[u8],
        block_id: &[u8],
        operation_id: &[u8],
        condition: &str,
    ) -> pb::BatchCommitEntry {
        pb::BatchCommitEntry {
            key: Some(pb::Key {
                value: key.to_vec(),
            }),
            candidate: Some(pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length: 4,
                extents: vec![pb::ExtentRecord {
                    logical: Some(pb::ByteRange {
                        offset: 0,
                        length: 4,
                    }),
                    block_id: block_id.to_vec(),
                    block_offset: 0,
                    digest: b"digest".to_vec(),
                    kind: pb::ExtentKind::Data as i32,
                }],
                digest: b"layout".to_vec(),
            }),
            condition: condition.to_string(),
            expected_version: None,
            operation_id: operation_id.to_vec(),
            operation_digest: [operation_id, b"-digest"].concat(),
            replica_proofs: Vec::new(),
            new_replicas: vec![pb::ReplicaReport {
                block_id: block_id.to_vec(),
                length: 4,
                checksum: b"digest".to_vec(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            }],
        }
    }

    fn create_test_file(
        state: &mut MetaState,
        session: pb::NodeSessionIdentity,
        name: &[u8],
    ) -> InodeSnapshot {
        let response = state
            .filesystem_create_inode(filesystem_create_request_for(
                session,
                crate::filesystem::ROOT_INODE,
                name,
                pb::FilesystemInodeKind::RegularFile,
                &[b"create/".as_slice(), name].concat(),
            ))
            .expect("create test file");
        crate::filesystem::inode_from_proto(
            response
                .resolved
                .expect("created inode resolution")
                .inode
                .expect("created inode"),
        )
        .expect("valid created inode")
    }

    fn unlink_test_file(
        state: &mut MetaState,
        session: pb::NodeSessionIdentity,
        name: &[u8],
        operation_id: &[u8],
    ) {
        state
            .filesystem_remove_entry(pb::FilesystemRemoveRequest {
                context: None,
                session: Some(session),
                operation_id: operation_id.to_vec(),
                operation_digest: [operation_id, b"-digest"].concat(),
                parent: ROOT_INODE,
                name: name.to_vec(),
                kind: pb::FilesystemRemoveKind::File as i32,
                expected_parent_revision: None,
                commit_sequence: next_test_commit_sequence(),
            })
            .expect("unlink test file");
    }

    fn filesystem_create_request_for(
        session: pb::NodeSessionIdentity,
        parent: u64,
        name: &[u8],
        kind: pb::FilesystemInodeKind,
        operation_id: &[u8],
    ) -> pb::FilesystemCreateInodeRequest {
        pb::FilesystemCreateInodeRequest {
            context: None,
            session: Some(session),
            operation_id: operation_id.to_vec(),
            parent,
            name: name.to_vec(),
            kind: kind as i32,
            mode: if kind == pb::FilesystemInodeKind::Directory {
                0o755
            } else {
                0o644
            },
            uid: 1000,
            gid: 1000,
            expected_parent_revision: None,
            operation_digest: [operation_id, b"-digest"].concat(),
            commit_sequence: next_test_commit_sequence(),
            reference_generation: 1,
        }
    }

    fn filesystem_attribute_request_for(
        session: pb::NodeSessionIdentity,
        inode: &InodeSnapshot,
        operation_id: &[u8],
        caller: pb::FilesystemCallerIdentity,
        patch: pb::FilesystemAttributePatch,
    ) -> pb::FilesystemSetAttributesRequest {
        pb::FilesystemSetAttributesRequest {
            context: None,
            session: Some(session),
            operation_id: operation_id.to_vec(),
            operation_digest: [operation_id, b"-digest"].concat(),
            inode: inode.attributes.inode,
            expected_inode_revision: inode.revision,
            caller: Some(caller),
            patch: Some(patch),
            commit_sequence: next_test_commit_sequence(),
        }
    }

    fn filesystem_set_xattr_request_for(
        session: pb::NodeSessionIdentity,
        inode: &InodeSnapshot,
        operation_id: &[u8],
        caller: pb::FilesystemCallerIdentity,
        name: &[u8],
        value: &[u8],
        mode: pb::FilesystemXattrSetMode,
    ) -> pb::FilesystemSetXattrRequest {
        pb::FilesystemSetXattrRequest {
            context: None,
            session: Some(session),
            operation_id: operation_id.to_vec(),
            operation_digest: [operation_id, b"-digest"].concat(),
            inode: inode.attributes.inode,
            expected_inode_revision: inode.revision,
            caller: Some(caller),
            name: name.to_vec(),
            value: value.to_vec(),
            mode: mode as i32,
            commit_sequence: next_test_commit_sequence(),
        }
    }

    fn filesystem_remove_xattr_request_for(
        session: pb::NodeSessionIdentity,
        inode: &InodeSnapshot,
        operation_id: &[u8],
        caller: pb::FilesystemCallerIdentity,
        name: &[u8],
    ) -> pb::FilesystemRemoveXattrRequest {
        pb::FilesystemRemoveXattrRequest {
            context: None,
            session: Some(session),
            operation_id: operation_id.to_vec(),
            operation_digest: [operation_id, b"-digest"].concat(),
            inode: inode.attributes.inode,
            expected_inode_revision: inode.revision,
            caller: Some(caller),
            name: name.to_vec(),
            commit_sequence: next_test_commit_sequence(),
        }
    }

    fn test_acl(user: u16, group: u16, other: u16) -> Vec<u8> {
        let mut value = 2_u32.to_le_bytes().to_vec();
        for (tag, permission) in [(0x01_u16, user), (0x04_u16, group), (0x20_u16, other)] {
            value.extend_from_slice(&tag.to_le_bytes());
            value.extend_from_slice(&permission.to_le_bytes());
            value.extend_from_slice(&u32::MAX.to_le_bytes());
        }
        value
    }

    fn filesystem_symlink_request_for(
        session: pb::NodeSessionIdentity,
        operation_id: &[u8],
        operation_digest: &[u8],
    ) -> pb::FilesystemCreateSymlinkRequest {
        let block_id = [operation_id, b"-block"].concat();
        let extents = vec![pb::ExtentRecord {
            logical: Some(pb::ByteRange {
                offset: 0,
                length: 4,
            }),
            block_id: block_id.clone(),
            block_offset: 0,
            digest: b"target-digest".to_vec(),
            kind: pb::ExtentKind::Data as i32,
        }];
        pb::FilesystemCreateSymlinkRequest {
            context: None,
            session: Some(session),
            operation_id: operation_id.to_vec(),
            operation_digest: operation_digest.to_vec(),
            parent: ROOT_INODE,
            name: b"symlink-idempotency".to_vec(),
            uid: 1000,
            gid: 1000,
            expected_parent_revision: None,
            commit_sequence: next_test_commit_sequence(),
            object_key: [b"fs/symlink/test/".as_slice(), operation_id].concat(),
            candidate: Some(pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length: 4,
                digest: filesystem_layout_digest(4, &extents),
                extents,
            }),
            replica_proofs: Vec::new(),
            new_replicas: vec![pb::ReplicaReport {
                block_id,
                length: 4,
                checksum: b"target-digest".to_vec(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            }],
            target_size: 4,
            mtime_unix_nanos: 1,
            reference_generation: 1,
        }
    }

    fn filesystem_commit_request_for(
        session: pb::NodeSessionIdentity,
        inode: &InodeSnapshot,
        block_id: &[u8],
        operation_id: &[u8],
    ) -> pb::FilesystemCommitVersionRequest {
        let logical_length = 4;
        let extents = vec![pb::ExtentRecord {
            logical: Some(pb::ByteRange {
                offset: 0,
                length: logical_length,
            }),
            block_id: block_id.to_vec(),
            block_offset: 0,
            digest: b"digest".to_vec(),
            kind: pb::ExtentKind::Data as i32,
        }];
        pb::FilesystemCommitVersionRequest {
            context: None,
            session: Some(session),
            operation_id: operation_id.to_vec(),
            operation_digest: [operation_id, b"-digest"].concat(),
            inode: inode.attributes.inode,
            expected_inode_revision: inode.revision,
            object_key: filesystem_object_key(inode.attributes.inode),
            expected_object_version: inode.content.as_ref().map(|binding| binding.exact_version),
            candidate: Some(pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length,
                digest: filesystem_layout_digest(logical_length, &extents),
                extents,
            }),
            replica_proofs: Vec::new(),
            new_replicas: vec![pb::ReplicaReport {
                block_id: block_id.to_vec(),
                length: 4,
                checksum: b"digest".to_vec(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            }],
            new_size: 4,
            mtime_unix_nanos: 1,
            commit_sequence: next_test_commit_sequence(),
            caller: None,
            attribute_patch: None,
            reservation_additions: Vec::new(),
            reservation_reductions: Vec::new(),
        }
    }

    fn value_commit_request_for(
        session: pb::NodeSessionIdentity,
        key: Vec<u8>,
        block_id: Vec<u8>,
        operation_id: Vec<u8>,
        operation_digest: Vec<u8>,
    ) -> pb::CommitVersionRequest {
        pb::CommitVersionRequest {
            context: None,
            session: Some(session),
            key: Some(pb::Key { value: key }),
            candidate: Some(pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length: 4,
                extents: vec![pb::ExtentRecord {
                    logical: Some(pb::ByteRange {
                        offset: 0,
                        length: 4,
                    }),
                    block_id: block_id.clone(),
                    block_offset: 0,
                    digest: b"digest".to_vec(),
                    kind: pb::ExtentKind::Data as i32,
                }],
                digest: b"layout".to_vec(),
            }),
            condition: "any".to_string(),
            expected_version: None,
            operation_id,
            operation_digest,
            durability: pb::DurabilityPolicy::LocalMemory as i32,
            required_memory_copies: 1,
            replica_proofs: Vec::new(),
            new_replicas: vec![pb::ReplicaReport {
                block_id,
                length: 4,
                checksum: b"digest".to_vec(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            }],
            commit_sequence: next_test_commit_sequence(),
        }
    }

    fn tombstone_request_for(
        session: pb::NodeSessionIdentity,
        key: Vec<u8>,
        operation_id: Vec<u8>,
        operation_digest: Vec<u8>,
    ) -> pb::CommitVersionRequest {
        pb::CommitVersionRequest {
            context: None,
            session: Some(session),
            key: Some(pb::Key { value: key }),
            candidate: Some(pb::VersionCandidate {
                kind: pb::VersionKind::Tombstone as i32,
                logical_length: 0,
                extents: Vec::new(),
                digest: Vec::new(),
            }),
            condition: "any".to_string(),
            expected_version: None,
            operation_id,
            operation_digest,
            durability: pb::DurabilityPolicy::LocalMemory as i32,
            required_memory_copies: 0,
            replica_proofs: Vec::new(),
            new_replicas: Vec::new(),
            commit_sequence: next_test_commit_sequence(),
        }
    }

    fn meta_stat_request(session: pb::NodeSessionIdentity, key: &[u8]) -> pb::MetaStatRequest {
        pb::MetaStatRequest {
            context: None,
            session: Some(session),
            key: Some(pb::Key {
                value: key.to_vec(),
            }),
        }
    }

    fn meta_scan_request(
        session: pb::NodeSessionIdentity,
        prefix: &[u8],
        start_after: Option<Vec<u8>>,
        cursor: &str,
        limit: u32,
    ) -> pb::MetaScanRequest {
        pb::MetaScanRequest {
            context: None,
            session: Some(session),
            prefix: Some(pb::Key {
                value: prefix.to_vec(),
            }),
            options: Some(pb::ObjectScanOptions {
                limit,
                start_after,
                cursor: cursor.to_string(),
                delimiter: Vec::new(),
            }),
        }
    }

    fn meta_scan_request_with_delimiter(
        session: pb::NodeSessionIdentity,
        prefix: &[u8],
        start_after: Option<Vec<u8>>,
        cursor: &str,
        delimiter: &[u8],
        limit: u32,
    ) -> pb::MetaScanRequest {
        pb::MetaScanRequest {
            context: None,
            session: Some(session),
            prefix: Some(pb::Key {
                value: prefix.to_vec(),
            }),
            options: Some(pb::ObjectScanOptions {
                limit,
                start_after,
                cursor: cursor.to_string(),
                delimiter: delimiter.to_vec(),
            }),
        }
    }

    fn retirement_ack_request(
        session: pb::NodeSessionIdentity,
        record: &BlockRetirementRecord,
        ack_kind: pb::BlockRetirementAckKind,
        stage_epoch: u64,
    ) -> pb::AcknowledgeBlockRetirementRequest {
        pb::AcknowledgeBlockRetirementRequest {
            context: None,
            session: Some(session),
            retirement_id: record.retirement_id.clone(),
            ack_kind: ack_kind as i32,
            stage_epoch,
            block_ids: record.block_ids.clone(),
            detail: None,
        }
    }

    fn lock_request_for(
        session: pb::NodeSessionIdentity,
        request_id: u64,
        inode: u64,
        lock_owner: u64,
        mode: pb::FilesystemLockMode,
        wait: bool,
    ) -> pb::FilesystemLockRequest {
        lock_request_range_for(
            session,
            request_id,
            inode,
            lock_owner,
            (0, u64::MAX),
            mode,
            wait,
        )
    }

    fn lock_request_range_for(
        session: pb::NodeSessionIdentity,
        request_id: u64,
        inode: u64,
        lock_owner: u64,
        range: (u64, u64),
        mode: pb::FilesystemLockMode,
        wait: bool,
    ) -> pb::FilesystemLockRequest {
        pb::FilesystemLockRequest {
            context: None,
            session: Some(session),
            request_id,
            inode,
            lock_owner,
            range: Some(pb::FilesystemLockRange {
                start: range.0,
                end_inclusive: range.1,
            }),
            mode: mode as i32,
            pid: lock_owner as u32,
            wait,
        }
    }

    fn release_lock_owner_request(
        session: pb::NodeSessionIdentity,
        request_id: u64,
        lock_owner: u64,
    ) -> pb::FilesystemReleaseLockOwnerRequest {
        pb::FilesystemReleaseLockOwnerRequest {
            context: None,
            session: Some(session),
            lock_owner,
            request_id,
        }
    }

    #[tokio::test]
    async fn filesystem_lock_actor_serializes_cross_node_wait_and_wakeup() {
        let handle = MetaHandle::spawn();
        let first_grant = handle
            .open_node_session(101, "http://127.0.0.1:19101".to_string(), true)
            .await
            .expect("first node session");
        let second_grant = handle
            .open_node_session(102, "http://127.0.0.1:19102".to_string(), true)
            .await
            .expect("second node session");
        let first = pb::NodeSessionIdentity {
            session_id: first_grant.session_id,
            node_id: first_grant.node_id,
            node_epoch: first_grant.node_epoch,
        };
        let second = pb::NodeSessionIdentity {
            session_id: second_grant.session_id,
            node_id: second_grant.node_id,
            node_epoch: second_grant.node_epoch,
        };

        let acquired = handle
            .filesystem_set_lock(lock_request_for(
                first.clone(),
                1,
                77,
                11,
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await
            .expect("first lock");
        assert_eq!(acquired.status, pb::FilesystemLockStatus::Acquired as i32);

        let waiting_handle = handle.clone();
        let waiting = tokio::spawn(async move {
            waiting_handle
                .filesystem_set_lock(lock_request_for(
                    second,
                    2,
                    77,
                    22,
                    pb::FilesystemLockMode::Exclusive,
                    true,
                ))
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "conflicting F_SETLKW must wait");

        let released = handle
            .filesystem_set_lock(lock_request_for(
                first,
                3,
                77,
                11,
                pb::FilesystemLockMode::Unlock,
                false,
            ))
            .await
            .expect("unlock");
        assert_eq!(released.status, pb::FilesystemLockStatus::Released as i32);
        let acquired = waiting
            .await
            .expect("wait task")
            .expect("waiting lock result");
        assert_eq!(acquired.status, pb::FilesystemLockStatus::Acquired as i32);
    }

    #[tokio::test]
    async fn filesystem_lock_release_fence_orders_late_set_by_owner_at_actor_boundary() {
        let handle = MetaHandle::spawn();
        let grant = handle
            .open_node_session(131, "http://127.0.0.1:19131".to_string(), true)
            .await
            .expect("node session");
        let session = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        };

        handle
            .filesystem_release_lock_owner(release_lock_owner_request(session.clone(), 10, 7))
            .await
            .expect("release owner");

        let late_same_owner = handle
            .filesystem_set_lock(lock_request_for(
                session.clone(),
                9,
                77,
                7,
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await;
        assert!(
            matches!(late_same_owner, Err(MetaRuntimeError::InvalidArgument(message)) if message.contains("outside the retry window")),
            "same owner set with seq <= release fence must not recreate a ghost lock"
        );

        let independent_owner = handle
            .filesystem_set_lock(lock_request_for(
                session,
                5,
                77,
                8,
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await
            .expect("different owner is not fenced by owner release");
        assert_eq!(
            independent_owner.status,
            pb::FilesystemLockStatus::Acquired as i32
        );
    }

    #[tokio::test]
    async fn filesystem_lock_session_reopen_fences_until_complete_reclaim() {
        let handle = MetaHandle::spawn();
        let old_grant = handle
            .open_node_session(111, "http://127.0.0.1:19111".to_string(), true)
            .await
            .expect("old node session");
        let peer_grant = handle
            .open_node_session(112, "http://127.0.0.1:19112".to_string(), true)
            .await
            .expect("peer session");
        let old = pb::NodeSessionIdentity {
            session_id: old_grant.session_id,
            node_id: old_grant.node_id,
            node_epoch: old_grant.node_epoch,
        };
        let peer = pb::NodeSessionIdentity {
            session_id: peer_grant.session_id,
            node_id: peer_grant.node_id,
            node_epoch: peer_grant.node_epoch,
        };
        handle
            .filesystem_set_lock(lock_request_for(
                old,
                1,
                88,
                33,
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await
            .expect("old lock");

        let new_grant = handle
            .open_node_session(111, "http://127.0.0.1:19111".to_string(), true)
            .await
            .expect("new node session");
        let new_session = pb::NodeSessionIdentity {
            session_id: new_grant.session_id,
            node_id: new_grant.node_id,
            node_epoch: new_grant.node_epoch,
        };
        let fenced = handle
            .filesystem_test_lock(lock_request_for(
                peer.clone(),
                2,
                88,
                44,
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await
            .expect("fenced peer test");
        assert_eq!(
            fenced.status,
            pb::FilesystemLockStatus::RecoveryPending as i32
        );

        handle
            .filesystem_reclaim_locks(pb::FilesystemReclaimLocksRequest {
                context: None,
                session: Some(new_session.clone()),
                locks: vec![pb::FilesystemGrantedLock {
                    inode: 88,
                    owner: Some(pb::FilesystemLockOwner {
                        node_id: new_session.node_id,
                        node_epoch: new_session.node_epoch,
                        lock_owner: 33,
                    }),
                    range: Some(pb::FilesystemLockRange {
                        start: 0,
                        end_inclusive: u64::MAX,
                    }),
                    mode: pb::FilesystemLockMode::Exclusive as i32,
                    pid: 33,
                }],
            })
            .await
            .expect("reclaim lock snapshot");
        let conflict = handle
            .filesystem_test_lock(lock_request_for(
                peer,
                3,
                88,
                44,
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await
            .expect("peer conflict after reclaim");
        assert_eq!(conflict.status, pb::FilesystemLockStatus::Conflict as i32);
    }

    #[tokio::test]
    async fn filesystem_lock_request_id_retry_is_idempotent_at_actor_boundary() {
        let handle = MetaHandle::spawn();
        let owner_grant = handle
            .open_node_session(121, "http://127.0.0.1:19121".to_string(), true)
            .await
            .expect("owner session");
        let peer_grant = handle
            .open_node_session(122, "http://127.0.0.1:19122".to_string(), true)
            .await
            .expect("peer session");
        let owner = pb::NodeSessionIdentity {
            session_id: owner_grant.session_id,
            node_id: owner_grant.node_id,
            node_epoch: owner_grant.node_epoch,
        };
        let peer = pb::NodeSessionIdentity {
            session_id: peer_grant.session_id,
            node_id: peer_grant.node_id,
            node_epoch: peer_grant.node_epoch,
        };

        let first = handle
            .filesystem_set_lock(lock_request_range_for(
                owner.clone(),
                700,
                99,
                1,
                (0, 9),
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await
            .expect("first acquire");
        assert_eq!(first.status, pb::FilesystemLockStatus::Acquired as i32);

        let retry = handle
            .filesystem_set_lock(lock_request_range_for(
                owner.clone(),
                700,
                99,
                1,
                (0, 9),
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await
            .expect("idempotent retry");
        assert_eq!(retry.status, pb::FilesystemLockStatus::Acquired as i32);

        let mismatched_retry = handle
            .filesystem_set_lock(lock_request_range_for(
                owner,
                700,
                99,
                1,
                (10, 19),
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await;
        assert!(
            matches!(mismatched_retry, Err(MetaRuntimeError::InvalidArgument(message)) if message.contains("different request")),
            "same request_id with a different payload must be rejected"
        );

        let peer_probe = handle
            .filesystem_test_lock(lock_request_range_for(
                peer,
                701,
                99,
                2,
                (10, 19),
                pb::FilesystemLockMode::Exclusive,
                false,
            ))
            .await
            .expect("peer probe");
        assert_eq!(
            peer_probe.status,
            pb::FilesystemLockStatus::Acquired as i32,
            "同一 request_id 的重试只能查询第一次锁结果，不能留下第二个 range 的 ghost lock"
        );
    }

    fn seed_existing_key(
        state: &mut MetaState,
        session: pb::NodeSessionIdentity,
        key: &[u8],
    ) -> pb::CommitVersionResponse {
        let before_cursor = state.event_high_watermark;
        let before_events = state.events.len();
        let sequence = state.journal.last_index() + 1;
        let mut unique = key.to_vec();
        unique.extend_from_slice(&sequence.to_be_bytes());
        let response = state
            .commit_version(value_commit_request_for(
                session,
                key.to_vec(),
                [b"seed-block/".as_slice(), unique.as_slice()].concat(),
                [b"seed-op/".as_slice(), unique.as_slice()].concat(),
                [b"seed-digest/".as_slice(), unique.as_slice()].concat(),
            ))
            .expect("seed existing key");
        assert_eq!(
            state.event_high_watermark,
            before_cursor + 1,
            "测试种子是首次发布：不产生事件，但需要保留 cursor gap 兼容旧 Journal"
        );
        assert_eq!(
            state.events.len(),
            before_events,
            "测试种子只建立旧版本，不应该污染待 ACK 事件队列"
        );
        response
    }

    fn test_session(state: &mut MetaState, node_id: u64) -> pb::NodeSessionIdentity {
        let grant = state
            .open_node_session(
                node_id,
                format!("http://127.0.0.1:{}", 19000 + node_id),
                true,
            )
            .expect("open session");
        pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        }
    }

    fn next_test_commit_sequence() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    fn resolve_current_request(
        session: pb::NodeSessionIdentity,
        key: Vec<u8>,
        cache_current: bool,
    ) -> pb::ResolveObjectRequest {
        pb::ResolveObjectRequest {
            context: None,
            session: Some(session),
            key: Some(pb::Key { value: key }),
            selector: Some(pb::resolve_object_request::Selector::Current(true)),
            range: None,
            cache_current,
        }
    }

    fn expire_session(state: &mut MetaState, node_id: u64) {
        state
            .sessions
            .get_mut(&node_id)
            .expect("test session")
            .last_heartbeat =
            Some(Instant::now() - DEFAULT_NODE_LEASE_TTL - Duration::from_secs(1));
    }

    fn invalidate_event(event: &pb::NodeEvent) -> &pb::InvalidateCurrentEvent {
        match event.event.as_ref().expect("node event payload") {
            pb::node_event::Event::InvalidateCurrent(invalidation) => invalidation,
            other => panic!("expected invalidation event, got {other:?}"),
        }
    }

    #[test]
    fn namespace_dentry_invalidations_are_exact_and_deduplicated() {
        let source = DentrySnapshot {
            parent: 1,
            name: b"old".to_vec(),
            inode: 10,
            directory_revision: 7,
        };
        let target = DentrySnapshot {
            parent: 2,
            name: b"new".to_vec(),
            inode: 10,
            directory_revision: 7,
        };
        let replaced = DentrySnapshot {
            parent: 2,
            name: b"new".to_vec(),
            inode: 11,
            directory_revision: 6,
        };

        let invalidations =
            filesystem_dentry_invalidations(std::slice::from_ref(&target), &[source, replaced]);
        assert_eq!(
            invalidations.get(&1).expect("source parent")[0].name,
            b"old"
        );
        assert_eq!(
            invalidations.get(&2).expect("target parent").len(),
            1,
            "rename-over target appears in upsert and remove but needs one kernel notification"
        );
        assert_eq!(
            invalidations.get(&2).expect("target parent")[0].name,
            b"new"
        );
    }
}

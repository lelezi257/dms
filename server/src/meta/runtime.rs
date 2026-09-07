//! dms-meta 内唯一的异步状态所有者。
//!
//! gRPC Handler 只向 [`MetaHandle`] 投递命令。Node Session、Replica、Version、
//! Current、OperationResult 和失效事件只属于 `run_meta` 这一条 actor Task，因此
//! 首版无需全局锁。

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    time::{Duration, Instant},
};

use dms_error::{DmsError, ErrorKind};
use dms_protocol::v1 as pb;
use dms_tracing::Instrument as _;
use tokio::sync::{mpsc, oneshot};

#[cfg(test)]
use super::in_memory_journal::InMemoryJournal;
use super::metadata_journal::{
    JournalError, JournalRecord, MetaSnapshot, MetadataJournal, SnapshotOperation, SnapshotReplica,
    SnapshotReplicaOperation, SnapshotSession, VersionCommitRecord,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MetaRuntimeError {
    InvalidArgument(String),
    UnknownSession,
    NotFound,
    Conflict { expected: Option<u64>, actual: u64 },
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
            drop_unreferenced_replicas: false,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HeartbeatGrant {
    pub(crate) lease_ttl_millis: u64,
    pub(crate) accepted_node_epoch: u64,
    pub(crate) event_high_watermark: u64,
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
    pub(crate) fn try_spawn_with_runtime_policy(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        metrics: MetaMetrics,
        trace_periodic_operations: bool,
    ) -> Result<Self, MetaRuntimeError> {
        Self::try_spawn_with_policies_and_metrics(
            journal,
            checkpoint_policy,
            MetaRetentionPolicy::default(),
            metrics,
            trace_periodic_operations,
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
        )
    }

    fn try_spawn_with_policies_and_metrics(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        retention_policy: MetaRetentionPolicy,
        metrics: MetaMetrics,
        trace_periodic_operations: bool,
    ) -> Result<Self, MetaRuntimeError> {
        let (command_tx, command_rx) = mpsc::channel(META_MAILBOX_CAPACITY);
        let state = MetaState::try_new(
            journal,
            checkpoint_policy,
            retention_policy,
            metrics.clone(),
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
    ) -> Result<NodeSessionGrant, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::OpenNodeSession,
            MetaCommand::OpenNodeSession {
                node_id,
                control_endpoint,
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
    ) -> Result<HeartbeatGrant, MetaRuntimeError> {
        let (reply, receive) = oneshot::channel();
        self.complete(
            MetaOperation::Heartbeat,
            MetaCommand::Heartbeat {
                session_id,
                node_id,
                node_epoch,
                event_cursor,
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

enum MetaCommand {
    OpenNodeSession {
        node_id: u64,
        control_endpoint: String,
        reply: oneshot::Sender<Result<NodeSessionGrant, MetaRuntimeError>>,
    },
    Heartbeat {
        session_id: Vec<u8>,
        node_id: u64,
        node_epoch: u64,
        event_cursor: u64,
        reply: oneshot::Sender<Result<HeartbeatGrant, MetaRuntimeError>>,
    },
    ResolveObject {
        request: pb::ResolveObjectRequest,
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
            Self::ReportReplicas { .. } => MetaOperation::ReportReplicas,
            Self::CommitVersion { .. } => MetaOperation::CommitVersion,
            Self::CommitBatch { .. } => MetaOperation::CommitBatch,
            Self::GetOperation { .. } => MetaOperation::GetOperation,
            Self::PlanReplicas { .. } => MetaOperation::PlanReplicas,
            Self::WatchNodeEvents { .. } => MetaOperation::WatchNodeEvents,
            Self::AcknowledgeNodeEvent { .. } => MetaOperation::AcknowledgeNodeEvent,
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
}

struct NodeWatcher {
    sender: mpsc::Sender<pb::NodeEvent>,
    delivered_cursor: u64,
    replay_through: u64,
    disconnected: bool,
}

#[derive(Clone)]
struct StoredReplica {
    location: pb::ReplicaLocation,
    catalog_revision: u64,
    #[allow(dead_code)]
    length: u64,
}

#[derive(Clone)]
struct StoredOperation {
    digest: Vec<u8>,
    result: pb::CommitVersionResponse,
    /// Invalidation event that must be acknowledged before this operation may
    /// be returned as fully visible. `None` means the visibility barrier ended.
    visibility_cursor: Option<u64>,
}

struct CommitDispatch {
    operation_ids: Vec<Vec<u8>>,
    response: pb::CommitVersionResponse,
    event_cursor: Option<u64>,
    waiting_nodes: HashSet<u64>,
}

struct PendingCommit {
    operation_ids: Vec<Vec<u8>>,
    response: PendingResponse,
    waiting_nodes: HashSet<u64>,
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

enum PendingResponse {
    Single(pb::CommitVersionResponse),
    Batch(pb::CommitBatchResponse),
}

enum PendingReply {
    Single(oneshot::Sender<Result<pb::CommitVersionResponse, MetaRuntimeError>>),
    Batch(oneshot::Sender<Result<pb::CommitBatchResponse, MetaRuntimeError>>),
}

struct MetaState {
    /// 退休仅以租约边界为依据；进程恢复后先保守等待一个完整旧租约窗口。
    recovery_lease_until: Instant,
    retired_sessions: HashSet<u64>,
    prior_lease_deadlines: HashMap<u64, Instant>,
    next_session: u64,
    node_epochs: HashMap<u64, u64>,
    sessions: HashMap<u64, NodeSession>,
    replicas: HashMap<Vec<u8>, Vec<StoredReplica>>,
    /// Explicit policy target per immutable Block. This must not be derived
    /// from historical locations: replacement replicas would otherwise ratchet
    /// a two-copy policy to three, four, ... after successive failures.
    desired_replica_counts: HashMap<Vec<u8>, u32>,
    versions: HashMap<Vec<u8>, BTreeMap<u64, pb::VersionLayout>>,
    operations: HashMap<Vec<u8>, StoredOperation>,
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
    journal: Box<dyn MetadataJournal>,
    last_applied_index: u64,
    last_checkpoint_index: u64,
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

    fn try_new(
        journal: Box<dyn MetadataJournal>,
        checkpoint_policy: MetaCheckpointPolicy,
        retention_policy: MetaRetentionPolicy,
        metrics: MetaMetrics,
    ) -> Result<Self, MetaRuntimeError> {
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
            sessions: HashMap::new(),
            replicas: HashMap::new(),
            desired_replica_counts: HashMap::new(),
            versions: HashMap::new(),
            operations: HashMap::new(),
            replica_operations: HashMap::new(),
            events: Vec::new(),
            event_high_watermark: 0,
            watchers: HashMap::new(),
            recovery_lease_until: Instant::now() + DEFAULT_NODE_LEASE_TTL,
            retired_sessions: HashSet::new(),
            prior_lease_deadlines: HashMap::new(),
            pending_commits: BTreeMap::new(),
            pending_repairs: HashMap::new(),
            journal,
            last_applied_index: 0,
            last_checkpoint_index: replay_after,
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
            state
                .prior_lease_deadlines
                .insert(*node_id, state.recovery_lease_until);
        }
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

    fn open_node_session(
        &mut self,
        node_id: u64,
        control_endpoint: String,
    ) -> Result<NodeSessionGrant, MetaRuntimeError> {
        if node_id == 0 || control_endpoint.is_empty() {
            return Err(MetaRuntimeError::InvalidArgument(
                "node id and control endpoint are required".to_string(),
            ));
        }
        let node_epoch = self.node_epochs.entry(node_id).or_default();
        let next_epoch = *node_epoch + 1;
        let session_id = self.next_session.to_be_bytes().to_vec();
        let next_session = self.next_session + 1;
        let record = JournalRecord::NodeSessionOpened {
            node_id,
            node_epoch: next_epoch,
            session_id: session_id.clone(),
            control_endpoint,
            next_session,
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
        })
    }

    fn heartbeat(
        &mut self,
        session_id: Vec<u8>,
        node_id: u64,
        node_epoch: u64,
        _event_cursor: u64,
    ) -> Result<HeartbeatGrant, MetaRuntimeError> {
        self.verify_session(&pb::NodeSessionIdentity {
            session_id,
            node_id,
            node_epoch,
        })?;
        if let Some(session) = self.sessions.get_mut(&node_id) {
            session.last_heartbeat = Some(Instant::now());
        }
        self.retired_sessions.remove(&node_id);
        self.schedule_repairs();
        Ok(HeartbeatGrant {
            lease_ttl_millis: 30_000,
            accepted_node_epoch: node_epoch,
            event_high_watermark: self.event_high_watermark,
        })
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
        let block_replicas = layout
            .extents
            .iter()
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
            .collect();
        let current_lease = self.current_lease_grant(&request, layout);
        Ok(pb::ResolveObjectResponse {
            layout: Some(layout.clone()),
            block_replicas,
            current_lease,
        })
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
            if report.block_id.is_empty() || report.length == 0 {
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
            };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
            self.maybe_checkpoint(sequence)?;
            return Ok(result);
        }

        if candidate.kind == pb::VersionKind::Value as i32 {
            for extent in &candidate.extents {
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
                if !known && new_replica.is_none() {
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
            version: current_version + 1,
            logical_length: candidate.logical_length,
            extents: candidate.extents,
            digest: candidate.digest,
            kind: candidate.kind,
        };
        let record = JournalRecord::VersionCommitted {
            key,
            layout: layout.clone(),
            new_replicas,
            operation_id: request.operation_id,
            operation_digest: request.operation_digest,
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
                if !known && !is_new {
                    return Err(MetaRuntimeError::InvalidArgument(
                        "every batch extent requires a proof or new replica".to_string(),
                    ));
                }
            }
            let new_replicas = entry
                .new_replicas
                .into_iter()
                .map(|report| {
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
            let next_version = current.map_or(1, |layout| layout.version + 1);
            commits.push(VersionCommitRecord {
                key,
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
        let waiting_nodes = event_cursor
            .map(|cursor| self.waiting_visibility_nodes(source_node, cursor))
            .unwrap_or_default();
        Ok(CommitDispatch {
            operation_ids: vec![operation_id],
            response,
            event_cursor,
            waiting_nodes,
        })
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
        if dispatch.waiting_nodes.is_empty() {
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
        let waiting_nodes = self.waiting_visibility_nodes(source_node, cursor);
        if waiting_nodes.is_empty() {
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
                reply: PendingReply::Batch(reply),
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
                disconnected: false,
            },
        );
        // 先返回可消费的流，只填充现有容量；历史保留在同一个事件索引中。
        // 后续 ACK/维护 tick 继续投递，不复制无界 backlog，也不把满队列当作撤销缓存。
        self.pump_watchers();
        Ok(())
    }

    fn pump_watchers(&mut self) {
        for (node_id, watcher) in &mut self.watchers {
            if watcher.disconnected {
                continue;
            }
            let start = self
                .events
                .partition_point(|event| event.cursor <= watcher.delivered_cursor);
            for event in &self.events[start..] {
                if !event_targets_node(event, *node_id) {
                    watcher.delivered_cursor = event.cursor;
                    continue;
                }
                match watcher.sender.try_send(event.clone()) {
                    Ok(()) => {
                        watcher.delivered_cursor = event.cursor;
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
        let record = JournalRecord::NodeEventAcknowledged {
            node_id: session.node_id,
            cursor: request.cursor,
        };
        let sequence = self.append_record(record.clone())?;
        self.apply_record(sequence, record);
        self.maybe_checkpoint(sequence)?;
        self.advance_pending_commits(session.node_id, request.cursor);
        if let Some(block_id) = applied_repair {
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

    fn advance_pending_commits(&mut self, node_id: u64, acknowledged_cursor: u64) {
        if self
            .prior_lease_deadlines
            .get(&node_id)
            .is_some_and(|until| *until > Instant::now())
        {
            return;
        }
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
            let mut unresolved = Vec::new();
            for commit in self.pending_commits.remove(&cursor).unwrap_or_default() {
                if commit.waiting_nodes.is_empty() {
                    self.complete_operation_visibility(&commit.operation_ids);
                    match (commit.reply, commit.response) {
                        (PendingReply::Single(reply), PendingResponse::Single(response)) => {
                            let _ = reply.send(Ok(response));
                        }
                        (PendingReply::Batch(reply), PendingResponse::Batch(response)) => {
                            let _ = reply.send(Ok(response));
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
    }

    /// 投递失败不构成缓存撤销证明：断流 watcher 仍然参与 visibility barrier。
    /// source Node 在 Meta 回复后执行本机 Client barrier，因此不能在此等待自己。
    fn waiting_visibility_nodes(&self, source_node: u64, cursor: u64) -> HashSet<u64> {
        self.sessions
            .keys()
            .filter(|node_id| !self.retired_sessions.contains(node_id))
            .filter(|node_id| {
                **node_id != source_node
                    || self
                        .prior_lease_deadlines
                        .get(node_id)
                        .is_some_and(|until| *until > Instant::now())
            })
            .filter(|node_id| {
                self.sessions.get(node_id).is_some_and(|session| {
                    session.last_acked_cursor < cursor
                        || self
                            .prior_lease_deadlines
                            .get(node_id)
                            .is_some_and(|until| *until > Instant::now())
                })
            })
            .copied()
            .collect()
    }

    fn complete_operation_visibility(&mut self, operation_ids: &[Vec<u8>]) {
        for operation_id in operation_ids {
            if let Some(operation) = self.operations.get_mut(operation_id) {
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
            } => {
                if let Some(previous) = self.sessions.get(&node_id) {
                    let until = previous
                        .last_heartbeat
                        .map_or(self.recovery_lease_until, |started| {
                            started + DEFAULT_NODE_LEASE_TTL
                        });
                    self.prior_lease_deadlines
                        .entry(node_id)
                        .and_modify(|old| *old = (*old).max(until))
                        .or_insert(until);
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
                    },
                );
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
                new_replicas,
                operation_id,
                operation_digest,
            } => {
                self.apply_version_commit(
                    sequence,
                    VersionCommitRecord {
                        key,
                        layout,
                        new_replicas,
                        operation_id,
                        operation_digest,
                    },
                );
            }
            JournalRecord::VersionsCommitted { commits } => {
                for commit in commits {
                    self.apply_version_commit(sequence, commit);
                }
            }
            JournalRecord::OperationRemembered {
                operation_id,
                operation_digest,
                result,
            } => {
                self.operations.insert(
                    operation_id,
                    StoredOperation {
                        digest: operation_digest,
                        result,
                        visibility_cursor: None,
                    },
                );
            }
            JournalRecord::NodeEventAcknowledged { node_id, cursor } => {
                if let Some(session) = self.sessions.get_mut(&node_id) {
                    session.last_acked_cursor = session.last_acked_cursor.max(cursor);
                }
            }
        }
    }

    fn apply_version_commit(&mut self, sequence: u64, commit: VersionCommitRecord) {
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
                    },
                )),
            });
            Some(cursor)
        };
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
        let snapshot = self.snapshot();
        self.journal
            .save_snapshot(snapshot)
            .map_err(map_journal_error)?;
        self.journal
            .truncate_prefix(sequence)
            .map_err(map_journal_error)?;
        self.last_checkpoint_index = sequence;
        checkpoint_metric.success();
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
            let available = self
                .replicas
                .get(block_id)
                .map_or(0, |replicas| replicas.len());
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
        // lag 只统计真实保留的事件，否则新 key 热写会被误报为 watch 积压。
        //
        // events 按 cursor 单调追加。最大 lag 一定来自 ACK 最落后的会话，
        // 因此只需 O(sessions) 找最小 ACK，再 O(log events) 二分事件起点。
        let max_lag = self
            .sessions
            .values()
            .map(|session| session.last_acked_cursor)
            .min()
            .map(|min_acked_cursor| {
                let first_pending = self
                    .events
                    .partition_point(|event| event.cursor <= min_acked_cursor);
                (self.events.len() - first_pending) as u64
            })
            .unwrap_or(0);
        self.metrics.set_state(MetaStateMetricsSnapshot {
            keys: self.versions.len(),
            versions: self.versions.values().map(BTreeMap::len).sum(),
            replicas: self.replicas.values().map(Vec::len).sum(),
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
            let cursor = self
                .sessions
                .get(&node)
                .map_or(0, |session| session.last_acked_cursor);
            self.advance_pending_commits(node, cursor);
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
            // 记录退休水位后才解除等待；重放不会把已退休会话重新钉住旧历史。
            let cursor = self.event_high_watermark;
            let record = JournalRecord::NodeEventAcknowledged { node_id, cursor };
            let sequence = self.append_record(record.clone())?;
            self.apply_record(sequence, record);
            self.retired_sessions.insert(node_id);
            self.watchers.remove(&node_id);
            self.advance_pending_commits(node_id, cursor);
        }
        self.retain_events();
        Ok(())
    }

    fn enforce_retention(&mut self) {
        self.retain_versions();
        self.retain_operations();
        self.retain_events();
        self.retain_replicas_referenced_by_versions();
    }

    fn retain_versions(&mut self) {
        let keep = self.retention_policy.keep_versions_per_key.max(1);
        for versions in self.versions.values_mut() {
            while versions.len() > keep {
                let Some(oldest) = versions.first_key_value().map(|(version, _)| *version) else {
                    break;
                };
                versions.remove(&oldest);
            }
        }
    }

    fn retain_operations(&mut self) {
        let current_index = self.last_applied_index;
        let operation_window = self.retention_policy.operation_result_retention_records;
        self.operations.retain(|_, operation| {
            operation.visibility_cursor.is_some()
                || current_index.saturating_sub(operation.result.commit_index) <= operation_window
        });

        let replica_window = self.retention_policy.replica_operation_retention_records;
        self.replica_operations.retain(|_, result| {
            current_index.saturating_sub(result.catalog_watermark) <= replica_window
        });
    }

    fn retain_events(&mut self) {
        if !self
            .retention_policy
            .event_retention_requires_all_session_acks
        {
            return;
        }
        let acked_by_all_sessions = self
            .sessions
            .iter()
            .filter(|(node_id, _)| !self.retired_sessions.contains(node_id))
            .map(|(_, session)| session.last_acked_cursor)
            .min()
            .unwrap_or(self.event_high_watermark);
        self.events
            .retain(|event| event.cursor > acked_by_all_sessions);
    }

    fn retain_replicas_referenced_by_versions(&mut self) {
        if !self.retention_policy.drop_unreferenced_replicas {
            return;
        }
        let referenced_blocks = self
            .versions
            .values()
            .flat_map(|versions| versions.values())
            .flat_map(|layout| layout.extents.iter())
            .map(|extent| extent.block_id.clone())
            .collect::<HashSet<_>>();
        self.replicas
            .retain(|block_id, _| referenced_blocks.contains(block_id));
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
            replica_block_count: self.replicas.len(),
            replica_location_count: self.replicas.values().map(Vec::len).sum(),
        }
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
        MetaSnapshot {
            last_applied_index: self.last_applied_index,
            next_session: self.next_session,
            node_epochs: self
                .node_epochs
                .iter()
                .map(|(node_id, epoch)| (*node_id, *epoch))
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
                })
                .collect(),
            replicas,
            desired_replica_counts: self
                .desired_replica_counts
                .iter()
                .map(|(block_id, copies)| (block_id.clone(), *copies))
                .collect(),
            versions,
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
        }
    }

    fn restore_snapshot(&mut self, snapshot: MetaSnapshot) {
        self.last_applied_index = snapshot.last_applied_index;
        self.next_session = snapshot.next_session;
        self.node_epochs = snapshot.node_epochs.into_iter().collect();
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
                    },
                )
            })
            .collect();
        for node_id in self.sessions.keys() {
            self.prior_lease_deadlines
                .insert(*node_id, self.recovery_lease_until);
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
            self.versions.insert(
                key,
                layouts
                    .into_iter()
                    .map(|layout| (layout.version, layout))
                    .collect(),
            );
        }
        self.operations = snapshot
            .operations
            .into_iter()
            .map(|operation| {
                (
                    operation.operation_id,
                    StoredOperation {
                        digest: operation.digest,
                        result: operation.result,
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

fn event_targets_node(event: &pb::NodeEvent, node_id: u64) -> bool {
    match &event.event {
        Some(pb::node_event::Event::RepairReplica(repair)) => repair
            .target
            .as_ref()
            .is_some_and(|target| target.node_id == node_id),
        _ => true,
    }
}

fn event_metric_type(event: &pb::NodeEvent) -> WatchEventType {
    match &event.event {
        Some(pb::node_event::Event::InvalidateCurrent(_)) => WatchEventType::Invalidation,
        Some(pb::node_event::Event::RepairReplica(_)) => WatchEventType::Repair,
        Some(pb::node_event::Event::EvictReplica(_)) => WatchEventType::Eviction,
        Some(pb::node_event::Event::FenceNode(_)) => WatchEventType::Fence,
        Some(pb::node_event::Event::Gap(_)) => WatchEventType::Gap,
        None => WatchEventType::Empty,
    }
}

fn digest(bytes: &[u8]) -> Vec<u8> {
    dms_transport::checksum::fnv1a_bytes(bytes).to_vec()
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
                    reply,
                } => {
                    let _ = reply.send(state.open_node_session(node_id, control_endpoint));
                }
                MetaCommand::Heartbeat {
                    session_id,
                    node_id,
                    node_epoch,
                    event_cursor,
                    reply,
                } => {
                    let _ =
                        reply.send(state.heartbeat(session_id, node_id, node_epoch, event_cursor));
                }
                MetaCommand::ResolveObject { request, reply } => {
                    let _ = reply.send(state.resolve_object(request));
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
    use super::*;

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
            dispatch.waiting_nodes.contains(&7),
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
        state.retire_expired_sessions().unwrap();
        assert_eq!(response.await.unwrap().unwrap().version, 2);
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
            .open_node_session(7, "http://127.0.0.1:19007".into())
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
        state.register_pending_commit(dispatch, reply);
        assert_eq!(
            completion
                .try_recv()
                .expect("new key reply should complete immediately")
                .expect("new key commit result")
                .version,
            1
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
        seed_existing_key(&mut state, writer.clone(), b"delete/recreate");

        state
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
        assert_eq!(invalidate.old_version, 2);
        assert_eq!(invalidate.minimum_version, 3);
    }

    #[test]
    fn journal_replay_preserves_cursor_gap_event_and_ack() {
        let mut before_restart = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut before_restart, 7);
        let reader = test_session(&mut before_restart, 8);
        before_restart
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
            Some(2)
        );
    }

    #[test]
    fn snapshot_gap_and_tail_overwrite_replay_real_event() {
        let mut before_restart = MetaState::new(Box::<InMemoryJournal>::default());
        let writer = test_session(&mut before_restart, 7);
        let reader = test_session(&mut before_restart, 8);
        before_restart
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
        before_restart
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
        assert_eq!(invalidate.old_version, 1);
        assert_eq!(invalidate.minimum_version, 2);
        assert_eq!(
            restored.waiting_visibility_nodes(writer_node_id, event.cursor),
            HashSet::from([writer_node_id, reader.node_id]),
            "恢复后真实事件仍可作为前台可见性屏障使用；重启恢复的 source incarnation 也需等待 lease fence"
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
        let handle = MetaHandle::spawn();
        let grant = handle
            .open_node_session(7, "http://127.0.0.1:19200".into())
            .await
            .unwrap();
        let writer = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: 7,
            node_epoch: grant.node_epoch,
        };
        handle
            .commit_version(value_commit_request_for(
                writer.clone(),
                b"watch/replay".to_vec(),
                b"watch/replay-seed-block".to_vec(),
                b"watch/replay-seed-op".to_vec(),
                b"watch/replay-seed-digest".to_vec(),
            ))
            .await
            .unwrap();
        for index in 1..=5 {
            handle
                .commit_version(value_commit_request_for(
                    writer.clone(),
                    b"watch/replay".to_vec(),
                    vec![index],
                    vec![index],
                    vec![index],
                ))
                .await
                .unwrap();
        }
        let (sender, mut receiver) = mpsc::channel(1);
        handle
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(writer),
                    last_acked_cursor: 0,
                },
                sender,
            )
            .await
            .expect("replay registration must not synchronously fill entire channel");
        for cursor in 2..=6 {
            let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(event.cursor, cursor);
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
                new_replicas: Vec::new(),
                operation_id: b"op-1".to_vec(),
                operation_digest: b"digest-1".to_vec(),
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
            .open_node_session(7, "http://127.0.0.1:19200".to_string())
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
            .open_node_session(7, "http://127.0.0.1:19200".to_string())
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
        };

        let first = state.commit_version(request.clone()).expect("first DEL");
        let retry = state.commit_version(request).expect("retry DEL");

        assert_eq!(first, retry);
        assert!(!first.changed);
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
        assert_eq!(versions.keys().copied().collect::<Vec<_>>(), vec![2, 3]);
        assert!(!state.replicas.contains_key(b"block-1".as_slice()));
        assert!(state.replicas.contains_key(b"block-2".as_slice()));
        assert!(state.replicas.contains_key(b"block-3".as_slice()));
    }

    #[test]
    fn event_gc_waits_until_every_session_acknowledges_cursor() {
        let mut state = MetaState::with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 1 },
            MetaRetentionPolicy::default(),
        );
        let writer = test_session(&mut state, 7);
        let reader = test_session(&mut state, 8);
        seed_existing_key(&mut state, writer.clone(), b"checkpoint/latest");

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
        state
            .acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: None,
                session: Some(writer),
                event_id: event.event_id.clone(),
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            })
            .expect("writer ack");
        assert_eq!(state.events.len(), 1, "reader has not ACKed yet");

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
            "event can be removed after all sessions ACK it"
        );
        assert_eq!(state.event_high_watermark, 2);
    }

    #[test]
    fn retry_result_does_not_advance_event_cursor_or_drop_replay() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
        seed_existing_key(&mut state, session.clone(), b"checkpoint/latest");
        state
            .commit_version(value_commit_request_for(
                session.clone(),
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
                session: Some(session.clone()),
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
                .get(&session.node_id)
                .expect("session")
                .last_acked_cursor,
            0
        );

        let (sender, mut receiver) = mpsc::channel(1);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(session),
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

    #[tokio::test]
    async fn stats_report_journal_snapshot_and_catalog_retention_counts() {
        let handle = MetaHandle::try_spawn_with_policies(
            Box::<InMemoryJournal>::default(),
            MetaCheckpointPolicy { every_records: 1 },
            MetaRetentionPolicy::default(),
        )
        .expect("spawn meta");
        let grant = handle
            .open_node_session(7, "http://127.0.0.1:19200".to_string())
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
            .open_node_session(7, "http://127.0.0.1:19200".to_string())
            .expect("open node session");
        let session = pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        };
        seed_existing_key(&mut state, session.clone(), b"checkpoint/latest");
        let (first_sender, mut first_receiver) = mpsc::channel(4);
        state
            .watch_node_events(
                pb::WatchNodeEventsRequest {
                    context: None,
                    session: Some(session.clone()),
                    last_acked_cursor: 0,
                },
                first_sender,
            )
            .expect("register first watch");

        state
            .commit_version(pb::CommitVersionRequest {
                context: None,
                session: Some(session.clone()),
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
                    session: Some(session),
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
            .open_node_session(7, "http://127.0.0.1:19200".to_string())
            .expect("writer session");
        let reader = state
            .open_node_session(8, "http://127.0.0.1:19201".to_string())
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
        let (reply, mut completion) = oneshot::channel();
        state.register_pending_commit(dispatch, reply);
        assert!(matches!(
            completion.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        let event = event_receiver.try_recv().expect("remote invalidation");
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
        assert_eq!(response.version, 2);
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
            2
        );
        assert_eq!(
            retry_completion
                .await
                .expect("retry reply")
                .expect("retry result")
                .version,
            2
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
        assert_eq!(
            retry.waiting_nodes,
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
        assert!(matches!(
            retry_completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        restored
            .sessions
            .get_mut(&writer.node_id)
            .unwrap()
            .last_acked_cursor = replay.cursor;
        for until in restored.prior_lease_deadlines.values_mut() {
            *until = Instant::now();
        }
        restored.retire_expired_sessions().unwrap();
        assert_eq!(
            retry_completion
                .await
                .expect("retry reply")
                .expect("retry result")
                .version,
            2
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
        state
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
        assert_eq!(lease.version, 1);
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
        state
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
                selector: Some(pb::resolve_object_request::Selector::ExactVersion(1)),
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
    fn replica_report_records_the_verified_session_node_epoch() {
        let mut state = MetaState::new(Box::<InMemoryJournal>::default());
        let session = test_session(&mut state, 7);
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
            .and_then(|replicas| replicas.first())
            .expect("reported replica location");
        assert_eq!(location.location.node_id, session.node_id);
        assert_eq!(location.location.node_epoch, session.node_epoch);
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
        state
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
                batch_entry(b"batch/a", b"batch/a-v2", b"batch/op-a2", "if-version:1"),
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

        restored
            .heartbeat(
                session.session_id.clone(),
                session.node_id,
                session.node_epoch,
                0,
            )
            .expect("same epoch heartbeat revives session");
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
        }
    }

    fn seed_existing_key(state: &mut MetaState, session: pb::NodeSessionIdentity, key: &[u8]) {
        let before_cursor = state.event_high_watermark;
        let before_events = state.events.len();
        let sequence = state.journal.last_index() + 1;
        let mut unique = key.to_vec();
        unique.extend_from_slice(&sequence.to_be_bytes());
        state
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
    }

    fn test_session(state: &mut MetaState, node_id: u64) -> pb::NodeSessionIdentity {
        let grant = state
            .open_node_session(node_id, format!("http://127.0.0.1:{}", 19000 + node_id))
            .expect("open session");
        pb::NodeSessionIdentity {
            session_id: grant.session_id,
            node_id: grant.node_id,
            node_epoch: grant.node_epoch,
        }
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
}

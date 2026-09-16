//! Node-owned typed metrics.
//!
//! This module defines business meaning. `dms-metrics` only provides common
//! Prometheus mechanics, so Node allocator and replica labels cannot leak into
//! Client or Meta code.

use std::time::Instant;

use dms_metrics::{
    Gauge, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    MetricsError, Opts, Registry, latency_buckets, register_collector,
};

#[derive(Clone)]
pub(crate) struct NodeMetrics {
    mailbox_depth: IntGauge,
    mailbox_wait_duration_seconds: HistogramVec,
    sessions: IntGauge,
    session_expirations_total: IntCounterVec,
    arena_capacity_bytes: IntGauge,
    arena_allocated_bytes: IntGauge,
    arena_quarantined_bytes: IntGauge,
    arena_logical_bytes: IntGauge,
    arena_free_bytes: IntGauge,
    arena_fragmentation_ratio: Gauge,
    arena_allocations_total: IntCounterVec,
    arena_allocation_duration_seconds: Histogram,
    staging_allocations: IntGauge,
    staging_oldest_age_seconds: Gauge,
    staging_reclaimed_total: IntCounterVec,
    regions: IntGauge,
    region_expanded_bytes_total: IntCounter,
    shm_fd_grants_total: IntCounterVec,
    replica_operations_total: IntCounterVec,
    replica_bytes_total: IntCounterVec,
    replica_transfer_duration_seconds: HistogramVec,
    replica_checksum_failures_total: IntCounterVec,
    peer_import_inflight: IntGauge,
    peer_import_reserved_bytes: IntGauge,
    peer_import_failure_fences: IntGauge,
    download_tickets: IntGauge,
    peer_pull_fault_injections_total: IntCounter,
    peer_source_failovers_total: IntCounter,
    current_cache_lookups_total: IntCounterVec,
    current_cache_resets_total: IntCounterVec,
    current_cache_charged_bytes: IntGauge,
    filesystem_operations_total: IntCounterVec,
    filesystem_io_bytes_total: IntCounterVec,
    filesystem_operation_duration_seconds: HistogramVec,
    fuse_callbacks_total: IntCounterVec,
    fuse_callback_bytes_total: IntCounterVec,
    data_core_operations_total: IntCounterVec,
    data_core_bytes_total: IntCounterVec,
    filesystem_dentry_cache_lookups_total: IntCounterVec,
    filesystem_binding_cache_lookups_total: IntCounterVec,
}

/// Node 进程内 Filesystem API 的固定操作集合。
///
/// 只允许低基数操作名进入 label；inode、路径和 handle 不进入 Metrics。
#[derive(Clone, Copy)]
pub(crate) enum FilesystemOperation {
    Lookup,
    Getattr,
    Create,
    Mkdir,
    Readdir,
    Rename,
    Unlink,
    Rmdir,
    Open,
    Read,
    Write,
    Truncate,
    Close,
}

impl FilesystemOperation {
    const LIVE: &'static [Self] = &[
        Self::Lookup,
        Self::Getattr,
        Self::Create,
        Self::Mkdir,
        Self::Readdir,
        Self::Rename,
        Self::Unlink,
        Self::Rmdir,
        Self::Open,
        Self::Read,
        Self::Write,
        Self::Truncate,
        Self::Close,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Lookup => "lookup",
            Self::Getattr => "getattr",
            Self::Create => "create",
            Self::Mkdir => "mkdir",
            Self::Readdir => "readdir",
            Self::Rename => "rename",
            Self::Unlink => "unlink",
            Self::Rmdir => "rmdir",
            Self::Open => "open",
            Self::Read => "read",
            Self::Write => "write",
            Self::Truncate => "truncate",
            Self::Close => "close",
        }
    }
}

/// Linux FUSE 内核 callback 的固定集合。
///
/// 它与 `FilesystemOperation` 刻意分开：一次 `create` callback 会调用文件层的
/// create 和 open 两个操作。两组计数并列后，才看得出放大来自内核边界还是业务层。
#[derive(Clone, Copy)]
pub(crate) enum FuseCallback {
    Lookup,
    Getattr,
    Readdir,
    Mkdir,
    Rename,
    Unlink,
    Rmdir,
    Open,
    Create,
    Setattr,
    Read,
    Write,
    Flush,
    Fsync,
    Release,
}

impl FuseCallback {
    const LIVE: &'static [Self] = &[
        Self::Lookup,
        Self::Getattr,
        Self::Readdir,
        Self::Mkdir,
        Self::Rename,
        Self::Unlink,
        Self::Rmdir,
        Self::Open,
        Self::Create,
        Self::Setattr,
        Self::Read,
        Self::Write,
        Self::Flush,
        Self::Fsync,
        Self::Release,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Lookup => "lookup",
            Self::Getattr => "getattr",
            Self::Readdir => "readdir",
            Self::Mkdir => "mkdir",
            Self::Rename => "rename",
            Self::Unlink => "unlink",
            Self::Rmdir => "rmdir",
            Self::Open => "open",
            Self::Create => "create",
            Self::Setattr => "setattr",
            Self::Read => "read",
            Self::Write => "write",
            Self::Flush => "flush",
            Self::Fsync => "fsync",
            Self::Release => "release",
        }
    }
}

/// 文件入口真正使用到的 DataCore 操作。
///
/// 只记录次数和 payload 字节，不记录时延；端到端与文件层时延已经有现成指标，
/// 这里的唯一目的，是把“一个 POSIX 操作放大成几次 DataCore 操作”机器化。
#[derive(Clone, Copy)]
pub(crate) enum DataCoreOperation {
    ReadResolved,
    PrepareRange,
    PrepareSparse,
    PrepareTruncate,
    FinishPrepared,
}

impl DataCoreOperation {
    const LIVE: &'static [Self] = &[
        Self::ReadResolved,
        Self::PrepareRange,
        Self::PrepareSparse,
        Self::PrepareTruncate,
        Self::FinishPrepared,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::ReadResolved => "read_resolved",
            Self::PrepareRange => "prepare_range",
            Self::PrepareSparse => "prepare_sparse",
            Self::PrepareTruncate => "prepare_truncate",
            Self::FinishPrepared => "finish_prepared",
        }
    }
}

/// Node mailbox 中可出现的命令。枚举把 label 限制在代码审查可见的固定集合内。
#[derive(Clone, Copy, Debug)]
pub(crate) enum NodeMailboxCommand {
    OpenSession,
    AttachSession,
    Heartbeat,
    CloseSession,
    ValidateSession,
    Acknowledge,
    AllocateStaging,
    AcquireRegion,
    DeleteStaging,
    ConsumeStaging,
    Upload,
    Set,
    MSet,
    SetInline,
    SetRange,
    Delete,
    GetCached,
    GetResolved,
    MaterializeResolved,
    ImportPeerBlock,
    Download,
    PeerProbe,
    PeerPullBlock,
    PrepareReplica,
    ActivateReplica,
    AbortReplica,
    DiscardReplicaAttempt,
    ReplicaStatus,
    InvalidateCurrent,
    WaitInvalidation,
    ApplyConfigChange,
    #[cfg(test)]
    DebugCommitForPeerTest,
    #[cfg(test)]
    DebugStagingTtl,
}

impl NodeMailboxCommand {
    const LIVE: &'static [Self] = &[
        Self::OpenSession,
        Self::AttachSession,
        Self::Heartbeat,
        Self::CloseSession,
        Self::ValidateSession,
        Self::Acknowledge,
        Self::AllocateStaging,
        Self::AcquireRegion,
        Self::DeleteStaging,
        Self::ConsumeStaging,
        Self::Upload,
        Self::Set,
        Self::MSet,
        Self::SetInline,
        Self::SetRange,
        Self::Delete,
        Self::GetCached,
        Self::GetResolved,
        Self::MaterializeResolved,
        Self::ImportPeerBlock,
        Self::Download,
        Self::PeerProbe,
        Self::PeerPullBlock,
        Self::PrepareReplica,
        Self::ActivateReplica,
        Self::AbortReplica,
        Self::DiscardReplicaAttempt,
        Self::ReplicaStatus,
        Self::InvalidateCurrent,
        Self::WaitInvalidation,
        Self::ApplyConfigChange,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::OpenSession => "open_session",
            Self::AttachSession => "attach_session",
            Self::Heartbeat => "heartbeat",
            Self::CloseSession => "close_session",
            Self::ValidateSession => "validate_session",
            Self::Acknowledge => "acknowledge",
            Self::AllocateStaging => "allocate_staging",
            Self::AcquireRegion => "acquire_region",
            Self::DeleteStaging => "delete_staging",
            Self::ConsumeStaging => "consume_staging",
            Self::Upload => "upload",
            Self::Set => "set",
            Self::MSet => "mset",
            Self::SetInline => "set_inline",
            Self::SetRange => "set_range",
            Self::Delete => "delete",
            Self::GetCached => "get_cached",
            Self::GetResolved => "get_resolved",
            Self::MaterializeResolved => "materialize_resolved",
            Self::ImportPeerBlock => "import_peer_block",
            Self::Download => "download",
            Self::PeerProbe => "peer_probe",
            Self::PeerPullBlock => "peer_pull_block",
            Self::PrepareReplica => "prepare_replica",
            Self::ActivateReplica => "activate_replica",
            Self::AbortReplica => "abort_replica",
            Self::DiscardReplicaAttempt => "discard_replica_attempt",
            Self::ReplicaStatus => "replica_status",
            Self::InvalidateCurrent => "invalidate_current",
            Self::WaitInvalidation => "wait_invalidation",
            Self::ApplyConfigChange => "apply_config_change",
            #[cfg(test)]
            Self::DebugCommitForPeerTest => "debug_commit_for_peer_test",
            #[cfg(test)]
            Self::DebugStagingTtl => "debug_staging_ttl",
        }
    }

    /// Periodic liveness traffic is operationally useful as a Metric but is
    /// too noisy to export as a successful Trace by default.
    pub(crate) const fn is_periodic(self) -> bool {
        matches!(self, Self::Heartbeat)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SessionExpiration {
    Closed,
}

impl SessionExpiration {
    const fn label(self) -> &'static str {
        match self {
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CurrentCacheResetReason {
    WatchDisconnected,
    WatchReconnected,
}

impl CurrentCacheResetReason {
    const fn label(self) -> &'static str {
        match self {
            Self::WatchDisconnected => "watch_disconnected",
            Self::WatchReconnected => "watch_reconnected",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum StagingReclaimReason {
    Cancel,
    Expired,
    SessionClosed,
}

impl StagingReclaimReason {
    const fn label(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Expired => "expired",
            Self::SessionClosed => "session_closed",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ShmFdGrantResult {
    Issued,
    Claimed,
    Error,
}

impl ShmFdGrantResult {
    const fn label(self) -> &'static str {
        match self {
            Self::Issued => "issued",
            Self::Claimed => "claimed",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ReplicaOperation {
    Probe,
    Prepare,
    Pull,
    Activate,
    Abort,
    Status,
}

impl ReplicaOperation {
    const LIVE: &'static [Self] = &[
        Self::Probe,
        Self::Prepare,
        Self::Pull,
        Self::Activate,
        Self::Abort,
        Self::Status,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Prepare => "prepare",
            Self::Pull => "pull",
            Self::Activate => "activate",
            Self::Abort => "abort",
            Self::Status => "status",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ReplicaDirection {
    Send,
    Receive,
}

impl ReplicaDirection {
    const fn label(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Receive => "receive",
        }
    }
}

/// 一次状态采样需要同时更新的一组 Arena Gauge。
pub(crate) struct ArenaMetricsSnapshot {
    pub(crate) allocated_bytes: u64,
    pub(crate) quarantined_bytes: u64,
    pub(crate) logical_bytes: u64,
    pub(crate) free_bytes: u64,
    pub(crate) fragmentation_ratio: f64,
    pub(crate) staging_allocations: usize,
    pub(crate) regions: usize,
    pub(crate) oldest_staging_age_seconds: f64,
}

/// Peer 导入相关 owner 状态的一次采样。
///
/// 这些值属于 Node 内部可靠性控制面，不按 key/block 打 label，避免把对象数量
/// 放大成 Prometheus 时间序列数量。后续 runtime 接线时只需要在 owner 状态变化
/// 后调用一次 `set_peer_import_state`。
#[allow(dead_code)]
pub(crate) struct PeerImportMetricsSnapshot {
    pub(crate) inflight: usize,
    pub(crate) reserved_bytes: u64,
    pub(crate) failure_fences: usize,
    pub(crate) download_tickets: usize,
}

impl NodeMetrics {
    pub(crate) fn register(registry: &Registry) -> Result<Self, MetricsError> {
        let metrics = Self {
            mailbox_depth: IntGauge::new(
                "dms_node_mailbox_depth",
                "Commands queued for the Node state owner.",
            )?,
            mailbox_wait_duration_seconds: histogram_vec(
                "dms_node_mailbox_wait_duration_seconds",
                "Time a command waits before the Node state owner receives it.",
                &["command"],
            )?,
            sessions: IntGauge::new(
                "dms_node_sessions",
                "Active Client sessions owned by this Node.",
            )?,
            session_expirations_total: counter_vec(
                "dms_node_session_expirations_total",
                "Sessions reclaimed by close or timeout reason.",
                &["reason"],
            )?,
            arena_capacity_bytes: IntGauge::new(
                "dms_node_arena_capacity_bytes",
                "Configured host-memory Arena capacity.",
            )?,
            arena_allocated_bytes: IntGauge::new(
                "dms_node_arena_allocated_bytes",
                "Aligned bytes held by live and quarantined allocations.",
            )?,
            arena_quarantined_bytes: IntGauge::new(
                "dms_node_arena_quarantined_bytes",
                "Exported retired allocation bytes withheld from reuse until Node exit.",
            )?,
            arena_logical_bytes: IntGauge::new(
                "dms_node_arena_logical_bytes",
                "Logical payload bytes held by staging and blocks.",
            )?,
            arena_free_bytes: IntGauge::new(
                "dms_node_arena_free_bytes",
                "Arena capacity still available for allocation.",
            )?,
            arena_fragmentation_ratio: Gauge::new(
                "dms_node_arena_fragmentation_ratio",
                "External free-slot fragmentation ratio.",
            )?,
            arena_allocations_total: counter_vec(
                "dms_node_arena_allocations_total",
                "Arena allocation attempts by result.",
                &["result"],
            )?,
            arena_allocation_duration_seconds: Histogram::with_opts(
                HistogramOpts::new(
                    "dms_node_arena_allocation_duration_seconds",
                    "Arena slot allocation latency.",
                )
                .buckets(latency_buckets()),
            )?,
            staging_allocations: IntGauge::new(
                "dms_node_staging_allocations",
                "Uncommitted staging allocations.",
            )?,
            staging_oldest_age_seconds: Gauge::new(
                "dms_node_staging_oldest_age_seconds",
                "Age of the oldest staging allocation.",
            )?,
            staging_reclaimed_total: counter_vec(
                "dms_node_staging_reclaimed_total",
                "Staging allocations reclaimed by reason.",
                &["reason"],
            )?,
            regions: IntGauge::new(
                "dms_node_regions",
                "Physical Arena Regions currently owned by the Node.",
            )?,
            region_expanded_bytes_total: IntCounter::new(
                "dms_node_region_expanded_bytes_total",
                "Cumulative bytes added by Region expansion.",
            )?,
            shm_fd_grants_total: counter_vec(
                "dms_node_shm_fd_grants_total",
                "SCM_RIGHTS Region grants by result.",
                &["result"],
            )?,
            replica_operations_total: counter_vec(
                "dms_node_replica_operations_total",
                "Replica operations by stage and result.",
                &["operation", "result"],
            )?,
            replica_bytes_total: counter_vec(
                "dms_node_replica_bytes_total",
                "Node-to-Node replica payload bytes.",
                &["direction"],
            )?,
            replica_transfer_duration_seconds: histogram_vec(
                "dms_node_replica_transfer_duration_seconds",
                "Node-to-Node replica transfer latency.",
                &["operation", "provider"],
            )?,
            replica_checksum_failures_total: counter_vec(
                "dms_node_replica_checksum_failures_total",
                "Replica payload checksum failures.",
                &["provider"],
            )?,
            peer_import_inflight: IntGauge::new(
                "dms_node_peer_import_inflight",
                "Peer Block import attempts currently owned by this Node.",
            )?,
            peer_import_reserved_bytes: IntGauge::new(
                "dms_node_peer_import_reserved_bytes",
                "Expected bytes reserved by in-flight peer imports.",
            )?,
            peer_import_failure_fences: IntGauge::new(
                "dms_node_peer_import_failure_fences",
                "Recent peer import failure fences retained to wake delayed readers safely.",
            )?,
            download_tickets: IntGauge::new(
                "dms_node_download_tickets",
                "One-shot payload download tickets currently held by this Node.",
            )?,
            peer_pull_fault_injections_total: IntCounter::new(
                "dms_node_peer_pull_fault_injections_total",
                "Explicit reliability-test peer pull gates that were triggered.",
            )?,
            peer_source_failovers_total: IntCounter::new(
                "dms_node_peer_source_failovers_total",
                "Peer Block pulls that switched to another replica source within one request.",
            )?,
            current_cache_lookups_total: counter_vec(
                "dms_node_current_cache_lookups_total",
                "Node Current layout cache lookups by result; this cache stores key layout, not value bytes.",
                &["result"],
            )?,
            current_cache_resets_total: counter_vec(
                "dms_node_current_cache_resets_total",
                "Low-frequency Current layout cache resets caused by metadata watch state changes.",
                &["reason"],
            )?,
            current_cache_charged_bytes: IntGauge::new(
                "dms_node_current_cache_charged_bytes",
                "Current layout cache budget charge. This is an accounting value, not process RSS.",
            )?,
            filesystem_operations_total: counter_vec(
                "dms_node_filesystem_operations_total",
                "In-process filesystem operations by operation and result.",
                &["operation", "result"],
            )?,
            filesystem_io_bytes_total: counter_vec(
                "dms_node_filesystem_io_bytes_total",
                "Bytes returned or accepted by successful FUSE read and write callbacks.",
                &["operation"],
            )?,
            filesystem_operation_duration_seconds: histogram_vec(
                "dms_node_filesystem_operation_duration_seconds",
                "End-to-end latency of in-process filesystem operations.",
                &["operation"],
            )?,
            fuse_callbacks_total: counter_vec(
                "dms_node_fuse_callbacks_total",
                "Linux FUSE callbacks received from the kernel by operation.",
                &["operation"],
            )?,
            fuse_callback_bytes_total: counter_vec(
                "dms_node_fuse_callback_bytes_total",
                "Payload bytes received or returned at successful FUSE read/write callback boundaries.",
                &["operation"],
            )?,
            data_core_operations_total: counter_vec(
                "dms_node_data_core_operations_total",
                "DataCore operations invoked by in-process filesystem and image entries.",
                &["operation"],
            )?,
            data_core_bytes_total: counter_vec(
                "dms_node_data_core_bytes_total",
                "Payload bytes handled by measured DataCore read/prepare operations.",
                &["operation"],
            )?,
            filesystem_dentry_cache_lookups_total: counter_vec(
                "dms_node_filesystem_dentry_cache_lookups_total",
                "Filesystem positive dentry hint cache lookups by result.",
                &["result"],
            )?,
            filesystem_binding_cache_lookups_total: counter_vec(
                "dms_node_filesystem_binding_cache_lookups_total",
                "Filesystem inode binding cache lookups by result.",
                &["result"],
            )?,
        };
        metrics.register_all(registry)?;
        metrics.initialize_bounded_series();
        Ok(metrics)
    }

    fn initialize_bounded_series(&self) {
        for command in NodeMailboxCommand::LIVE {
            self.mailbox_wait_duration_seconds
                .with_label_values(&[command.label()]);
        }
        for reason in ["closed", "expired"] {
            self.session_expirations_total.with_label_values(&[reason]);
        }
        for result in ["ok", "capacity", "error"] {
            self.arena_allocations_total.with_label_values(&[result]);
        }
        for reason in ["cancel", "expired", "session_closed", "commit_failed"] {
            self.staging_reclaimed_total.with_label_values(&[reason]);
        }
        for result in ["issued", "claimed", "error"] {
            self.shm_fd_grants_total.with_label_values(&[result]);
        }
        for operation in ReplicaOperation::LIVE {
            self.replica_transfer_duration_seconds
                .with_label_values(&[operation.label(), "grpc"]);
            for result in ["ok", "error"] {
                self.replica_operations_total
                    .with_label_values(&[operation.label(), result]);
            }
        }
        for direction in ["send", "receive"] {
            self.replica_bytes_total.with_label_values(&[direction]);
        }
        self.replica_checksum_failures_total
            .with_label_values(&["grpc"]);
        for result in ["hit", "miss"] {
            self.current_cache_lookups_total
                .with_label_values(&[result]);
        }
        for reason in ["watch_disconnected", "watch_reconnected"] {
            self.current_cache_resets_total.with_label_values(&[reason]);
        }
        for operation in FilesystemOperation::LIVE {
            self.filesystem_operation_duration_seconds
                .with_label_values(&[operation.label()]);
            for result in ["ok", "error"] {
                self.filesystem_operations_total
                    .with_label_values(&[operation.label(), result]);
            }
        }
        for operation in [FilesystemOperation::Read, FilesystemOperation::Write] {
            self.filesystem_io_bytes_total
                .with_label_values(&[operation.label()]);
        }
        for callback in FuseCallback::LIVE {
            self.fuse_callbacks_total
                .with_label_values(&[callback.label()]);
        }
        for callback in [FuseCallback::Read, FuseCallback::Write] {
            self.fuse_callback_bytes_total
                .with_label_values(&[callback.label()]);
        }
        for operation in DataCoreOperation::LIVE {
            self.data_core_operations_total
                .with_label_values(&[operation.label()]);
            self.data_core_bytes_total
                .with_label_values(&[operation.label()]);
        }
        for result in ["hit", "miss"] {
            self.filesystem_dentry_cache_lookups_total
                .with_label_values(&[result]);
            self.filesystem_binding_cache_lookups_total
                .with_label_values(&[result]);
        }
    }

    fn register_all(&self, registry: &Registry) -> Result<(), MetricsError> {
        register_collector(registry, &self.mailbox_depth)?;
        register_collector(registry, &self.mailbox_wait_duration_seconds)?;
        register_collector(registry, &self.sessions)?;
        register_collector(registry, &self.session_expirations_total)?;
        register_collector(registry, &self.arena_capacity_bytes)?;
        register_collector(registry, &self.arena_allocated_bytes)?;
        register_collector(registry, &self.arena_quarantined_bytes)?;
        register_collector(registry, &self.arena_logical_bytes)?;
        register_collector(registry, &self.arena_free_bytes)?;
        register_collector(registry, &self.arena_fragmentation_ratio)?;
        register_collector(registry, &self.arena_allocations_total)?;
        register_collector(registry, &self.arena_allocation_duration_seconds)?;
        register_collector(registry, &self.staging_allocations)?;
        register_collector(registry, &self.staging_oldest_age_seconds)?;
        register_collector(registry, &self.staging_reclaimed_total)?;
        register_collector(registry, &self.regions)?;
        register_collector(registry, &self.region_expanded_bytes_total)?;
        register_collector(registry, &self.shm_fd_grants_total)?;
        register_collector(registry, &self.replica_operations_total)?;
        register_collector(registry, &self.replica_bytes_total)?;
        register_collector(registry, &self.replica_transfer_duration_seconds)?;
        register_collector(registry, &self.replica_checksum_failures_total)?;
        register_collector(registry, &self.peer_import_inflight)?;
        register_collector(registry, &self.peer_import_reserved_bytes)?;
        register_collector(registry, &self.peer_import_failure_fences)?;
        register_collector(registry, &self.download_tickets)?;
        register_collector(registry, &self.peer_pull_fault_injections_total)?;
        register_collector(registry, &self.peer_source_failovers_total)?;
        register_collector(registry, &self.current_cache_lookups_total)?;
        register_collector(registry, &self.current_cache_resets_total)?;
        register_collector(registry, &self.current_cache_charged_bytes)?;
        register_collector(registry, &self.filesystem_operations_total)?;
        register_collector(registry, &self.filesystem_io_bytes_total)?;
        register_collector(registry, &self.filesystem_operation_duration_seconds)?;
        register_collector(registry, &self.fuse_callbacks_total)?;
        register_collector(registry, &self.fuse_callback_bytes_total)?;
        register_collector(registry, &self.data_core_operations_total)?;
        register_collector(registry, &self.data_core_bytes_total)?;
        register_collector(registry, &self.filesystem_dentry_cache_lookups_total)?;
        register_collector(registry, &self.filesystem_binding_cache_lookups_total)?;
        Ok(())
    }

    pub(crate) fn mailbox_enqueued(&self) {
        self.mailbox_depth.inc();
    }

    pub(crate) fn mailbox_send_failed(&self) {
        self.mailbox_depth.dec();
    }

    pub(crate) fn record_mailbox_receive(&self, command: NodeMailboxCommand, enqueued_at: Instant) {
        self.mailbox_depth.dec();
        self.mailbox_wait_duration_seconds
            .with_label_values(&[command.label()])
            .observe(enqueued_at.elapsed().as_secs_f64());
    }

    pub(crate) fn set_sessions(&self, sessions: usize) {
        self.sessions.set(to_i64(sessions as u64));
    }

    pub(crate) fn record_session_expiration(&self, reason: SessionExpiration) {
        self.session_expirations_total
            .with_label_values(&[reason.label()])
            .inc();
    }

    pub(crate) fn set_arena_capacity(&self, bytes: u64) {
        self.arena_capacity_bytes.set(to_i64(bytes));
    }

    pub(crate) fn begin_arena_allocation(&self) -> ArenaAllocationGuard {
        ArenaAllocationGuard {
            metrics: self.clone(),
            started: Instant::now(),
            result: "error",
        }
    }

    pub(crate) fn set_arena_state(&self, snapshot: ArenaMetricsSnapshot) {
        self.arena_allocated_bytes
            .set(to_i64(snapshot.allocated_bytes));
        self.arena_quarantined_bytes
            .set(to_i64(snapshot.quarantined_bytes));
        self.arena_logical_bytes.set(to_i64(snapshot.logical_bytes));
        self.arena_free_bytes.set(to_i64(snapshot.free_bytes));
        self.arena_fragmentation_ratio
            .set(snapshot.fragmentation_ratio);
        self.staging_allocations
            .set(to_i64(snapshot.staging_allocations as u64));
        self.regions.set(to_i64(snapshot.regions as u64));
        self.staging_oldest_age_seconds
            .set(snapshot.oldest_staging_age_seconds);
    }

    pub(crate) fn record_staging_reclaimed(&self, reason: StagingReclaimReason) {
        self.staging_reclaimed_total
            .with_label_values(&[reason.label()])
            .inc();
    }

    pub(crate) fn record_region_expanded(&self, bytes: u64) {
        self.region_expanded_bytes_total.inc_by(bytes);
    }

    pub(crate) fn record_shm_fd_grant(&self, result: ShmFdGrantResult) {
        self.shm_fd_grants_total
            .with_label_values(&[result.label()])
            .inc();
    }

    pub(crate) fn begin_replica_operation(
        &self,
        operation: ReplicaOperation,
    ) -> ReplicaOperationGuard {
        ReplicaOperationGuard {
            metrics: self.clone(),
            operation,
            started: Instant::now(),
            result: "error",
            payload: None,
        }
    }

    pub(crate) fn record_replica_checksum_failure(&self) {
        self.replica_checksum_failures_total
            .with_label_values(&["grpc"])
            .inc();
    }

    /// 更新 Peer 导入控制面的容量/清理状态。
    ///
    /// R6 故障验证需要看见“拉取中、失败 fence、下载票据”是否最终归零；这些
    /// Gauge 后续由 Node owner 在状态变化点统一维护，避免测试脚本扫描内部结构。
    #[allow(dead_code)]
    pub(crate) fn set_peer_import_state(&self, snapshot: PeerImportMetricsSnapshot) {
        self.peer_import_inflight
            .set(to_i64(snapshot.inflight as u64));
        self.peer_import_reserved_bytes
            .set(to_i64(snapshot.reserved_bytes));
        self.peer_import_failure_fences
            .set(to_i64(snapshot.failure_fences as u64));
        self.download_tickets
            .set(to_i64(snapshot.download_tickets as u64));
    }

    /// 只在显式可靠性测试 gate 命中时增加；生产默认没有 gate，因此热路径为 0。
    #[cfg_attr(not(feature = "reliability-faults"), allow(dead_code))]
    pub(crate) fn record_peer_pull_fault_injection(&self) {
        self.peer_pull_fault_injections_total.inc();
    }

    /// 一次 Peer 拉取在当前候选不可用后，切换到下一候选时记录一次。
    ///
    /// 计数器不带 endpoint、node_id 或 block_id 标签，避免节点数和对象数进入
    /// Prometheus 时序维度；具体来源只在故障测试收据和诊断日志中出现。
    pub(crate) fn record_peer_source_failover(&self) {
        self.peer_source_failovers_total.inc();
    }

    /// 记录 Node 本地 Current 解析缓存命中结果。
    ///
    /// 缓存保存有效 VersionLayout 和同次解析的位置提示，不保存 value bytes。
    /// hit 表示版本授权命中，不保证 payload 已在本地；若旧位置失效，仍可能
    /// 按固定版本访问 Meta 刷新位置，因此不能把 hit 直接等同于省掉的 RPC 数。
    pub(crate) fn record_current_cache_lookup(&self, hit: bool) {
        let result = if hit { "hit" } else { "miss" };
        self.current_cache_lookups_total
            .with_label_values(&[result])
            .inc();
    }

    /// 记录因 Meta watch 状态变化而撤销本地 Current cache/grant 的次数。
    ///
    /// 这个计数只在 watch 断线或重连时增加，不在 GET/SET 热路径上运行；用于
    /// 验证“断线期间不继续信任旧 Current，重连 replay 后再恢复”的可靠性语义。
    pub(crate) fn record_current_cache_reset(&self, reason: CurrentCacheResetReason) {
        self.current_cache_resets_total
            .with_label_values(&[reason.label()])
            .inc();
    }

    /// 更新 Node Current 布局缓存预算占用。
    ///
    /// charge 来自 key、layout、extent、位置/证明和固定条目开销估算，不等于进程 RSS。
    pub(crate) fn set_current_cache_charge(&self, bytes: u64) {
        self.current_cache_charged_bytes.set(to_i64(bytes));
    }

    pub(crate) fn begin_filesystem_operation(
        &self,
        operation: FilesystemOperation,
    ) -> FilesystemOperationGuard {
        FilesystemOperationGuard {
            metrics: self.clone(),
            operation,
            started: Instant::now(),
            result: "error",
        }
    }

    pub(crate) fn record_filesystem_binding_cache_lookup(&self, hit: bool) {
        self.filesystem_binding_cache_lookups_total
            .with_label_values(&[if hit { "hit" } else { "miss" }])
            .inc();
    }

    /// 记录一次成功 FUSE read/write callback 实际交付的字节数。
    ///
    /// 调用次数已经由 `filesystem_operations_total` 记录；两者组合后可以判断一次
    /// 用户级 1 MiB I/O 被内核拆成多少 callback，以及每次 callback 的平均大小。
    pub(crate) fn record_filesystem_io_bytes(&self, operation: FilesystemOperation, bytes: usize) {
        debug_assert!(matches!(
            operation,
            FilesystemOperation::Read | FilesystemOperation::Write
        ));
        self.filesystem_io_bytes_total
            .with_label_values(&[operation.label()])
            .inc_by(bytes as u64);
    }

    pub(crate) fn record_filesystem_dentry_cache_lookup(&self, hit: bool) {
        self.filesystem_dentry_cache_lookups_total
            .with_label_values(&[if hit { "hit" } else { "miss" }])
            .inc();
    }

    pub(crate) fn record_fuse_callback(&self, callback: FuseCallback) {
        self.fuse_callbacks_total
            .with_label_values(&[callback.label()])
            .inc();
    }

    pub(crate) fn record_fuse_callback_bytes(&self, callback: FuseCallback, bytes: usize) {
        debug_assert!(matches!(callback, FuseCallback::Read | FuseCallback::Write));
        self.fuse_callback_bytes_total
            .with_label_values(&[callback.label()])
            .inc_by(bytes as u64);
    }

    pub(crate) fn record_data_core_operation(&self, operation: DataCoreOperation) {
        self.data_core_operations_total
            .with_label_values(&[operation.label()])
            .inc();
    }

    pub(crate) fn record_data_core_bytes(&self, operation: DataCoreOperation, bytes: usize) {
        self.data_core_bytes_total
            .with_label_values(&[operation.label()])
            .inc_by(bytes as u64);
    }
}

pub(crate) struct FilesystemOperationGuard {
    metrics: NodeMetrics,
    operation: FilesystemOperation,
    started: Instant,
    result: &'static str,
}

impl FilesystemOperationGuard {
    pub(crate) fn success(&mut self) {
        self.result = "ok";
    }
}

impl Drop for FilesystemOperationGuard {
    fn drop(&mut self) {
        self.metrics
            .filesystem_operations_total
            .with_label_values(&[self.operation.label(), self.result])
            .inc();
        self.metrics
            .filesystem_operation_duration_seconds
            .with_label_values(&[self.operation.label()])
            .observe(self.started.elapsed().as_secs_f64());
    }
}

pub(crate) struct ArenaAllocationGuard {
    metrics: NodeMetrics,
    started: Instant,
    result: &'static str,
}

impl ArenaAllocationGuard {
    pub(crate) fn success(&mut self) {
        self.result = "ok";
    }

    pub(crate) fn capacity_exhausted(&mut self) {
        self.result = "capacity";
    }
}

impl Drop for ArenaAllocationGuard {
    fn drop(&mut self) {
        self.metrics
            .arena_allocations_total
            .with_label_values(&[self.result])
            .inc();
        self.metrics
            .arena_allocation_duration_seconds
            .observe(self.started.elapsed().as_secs_f64());
    }
}

pub(crate) struct ReplicaOperationGuard {
    metrics: NodeMetrics,
    operation: ReplicaOperation,
    started: Instant,
    result: &'static str,
    payload: Option<(ReplicaDirection, usize)>,
}

impl ReplicaOperationGuard {
    pub(crate) fn success(&mut self) {
        self.result = "ok";
    }

    pub(crate) fn success_with_payload(&mut self, direction: ReplicaDirection, bytes: usize) {
        self.result = "ok";
        self.payload = Some((direction, bytes));
    }
}

impl Drop for ReplicaOperationGuard {
    fn drop(&mut self) {
        self.metrics
            .replica_operations_total
            .with_label_values(&[self.operation.label(), self.result])
            .inc();
        self.metrics
            .replica_transfer_duration_seconds
            .with_label_values(&[self.operation.label(), "grpc"])
            .observe(self.started.elapsed().as_secs_f64());
        if let Some((direction, bytes)) = self.payload {
            self.metrics
                .replica_bytes_total
                .with_label_values(&[direction.label()])
                .inc_by(bytes as u64);
        }
    }
}

fn to_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn counter_vec(name: &str, help: &str, labels: &[&str]) -> Result<IntCounterVec, MetricsError> {
    Ok(IntCounterVec::new(Opts::new(name, help), labels)?)
}

fn histogram_vec(name: &str, help: &str, labels: &[&str]) -> Result<HistogramVec, MetricsError> {
    Ok(HistogramVec::new(
        HistogramOpts::new(name, help).buckets(latency_buckets()),
        labels,
    )?)
}

#[cfg(test)]
mod tests {
    use dms_metrics::{encode_text, registry};

    use super::*;

    #[test]
    fn registers_exactly_the_live_node_contracts() {
        let registry = registry();
        let metrics = NodeMetrics::register(&registry).expect("node metrics");
        metrics.set_arena_capacity(1024);
        metrics.record_current_cache_lookup(true);
        metrics.record_current_cache_lookup(false);
        metrics.record_current_cache_reset(CurrentCacheResetReason::WatchDisconnected);
        metrics.record_current_cache_reset(CurrentCacheResetReason::WatchReconnected);
        metrics.set_current_cache_charge(128);
        metrics.set_peer_import_state(PeerImportMetricsSnapshot {
            inflight: 2,
            reserved_bytes: 4096,
            failure_fences: 1,
            download_tickets: 3,
        });
        metrics.record_peer_pull_fault_injection();
        metrics.record_peer_source_failover();
        let mut filesystem = metrics.begin_filesystem_operation(FilesystemOperation::Read);
        filesystem.success();
        drop(filesystem);
        metrics.record_filesystem_dentry_cache_lookup(true);
        metrics.record_filesystem_dentry_cache_lookup(false);
        metrics.record_filesystem_binding_cache_lookup(true);
        metrics.record_filesystem_binding_cache_lookup(false);
        let mut guard = metrics.begin_replica_operation(ReplicaOperation::Pull);
        guard.success_with_payload(ReplicaDirection::Send, 3);
        drop(guard);
        let text = encode_text(&registry).expect("encode");
        assert!(text.contains("dms_node_arena_capacity_bytes 1024"));
        assert!(text.contains("dms_node_replica_operations_total"));
        assert!(text.contains("dms_node_current_cache_lookups_total{result=\"hit\"} 1"));
        assert!(text.contains("dms_node_current_cache_lookups_total{result=\"miss\"} 1"));
        assert!(
            text.contains("dms_node_current_cache_resets_total{reason=\"watch_disconnected\"} 1")
        );
        assert!(
            text.contains("dms_node_current_cache_resets_total{reason=\"watch_reconnected\"} 1")
        );
        assert!(text.contains("dms_node_current_cache_charged_bytes 128"));
        assert!(text.contains("dms_node_peer_import_inflight 2"));
        assert!(text.contains("dms_node_peer_import_reserved_bytes 4096"));
        assert!(text.contains("dms_node_peer_import_failure_fences 1"));
        assert!(text.contains("dms_node_download_tickets 3"));
        assert!(text.contains("dms_node_peer_pull_fault_injections_total 1"));
        assert!(text.contains("dms_node_peer_source_failovers_total 1"));
        assert!(
            text.contains(
                "dms_node_filesystem_operations_total{operation=\"read\",result=\"ok\"} 1"
            )
        );
        assert!(text.contains("dms_node_filesystem_dentry_cache_lookups_total{result=\"hit\"} 1"));
        assert!(text.contains("dms_node_filesystem_dentry_cache_lookups_total{result=\"miss\"} 1"));
        assert!(text.contains("dms_node_filesystem_binding_cache_lookups_total{result=\"hit\"} 1"));
        assert!(
            text.contains("dms_node_filesystem_binding_cache_lookups_total{result=\"miss\"} 1")
        );
        assert!(!text.contains("dms_node_views"));
    }
}

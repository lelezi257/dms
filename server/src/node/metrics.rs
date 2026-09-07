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
    current_cache_lookups_total: IntCounterVec,
    current_cache_charged_bytes: IntGauge,
}

/// Node mailbox 中可出现的命令。枚举把 label 限制在代码审查可见的固定集合内。
#[derive(Clone, Copy, Debug)]
pub(crate) enum NodeMailboxCommand {
    OpenSession,
    AttachSession,
    Heartbeat,
    CloseSession,
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
            current_cache_lookups_total: counter_vec(
                "dms_node_current_cache_lookups_total",
                "Node Current layout cache lookups by result; this cache stores key layout, not value bytes.",
                &["result"],
            )?,
            current_cache_charged_bytes: IntGauge::new(
                "dms_node_current_cache_charged_bytes",
                "Current layout cache budget charge. This is an accounting value, not process RSS.",
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
        register_collector(registry, &self.current_cache_lookups_total)?;
        register_collector(registry, &self.current_cache_charged_bytes)?;
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

    /// 更新 Node Current 布局缓存预算占用。
    ///
    /// charge 来自 key、layout、extent、位置/证明和固定条目开销估算，不等于进程 RSS。
    pub(crate) fn set_current_cache_charge(&self, bytes: u64) {
        self.current_cache_charged_bytes.set(to_i64(bytes));
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
        metrics.set_current_cache_charge(128);
        let mut guard = metrics.begin_replica_operation(ReplicaOperation::Pull);
        guard.success_with_payload(ReplicaDirection::Send, 3);
        drop(guard);
        let text = encode_text(&registry).expect("encode");
        assert!(text.contains("dms_node_arena_capacity_bytes 1024"));
        assert!(text.contains("dms_node_replica_operations_total"));
        assert!(text.contains("dms_node_current_cache_lookups_total{result=\"hit\"} 1"));
        assert!(text.contains("dms_node_current_cache_lookups_total{result=\"miss\"} 1"));
        assert!(text.contains("dms_node_current_cache_charged_bytes 128"));
        assert!(!text.contains("dms_node_views"));
    }
}

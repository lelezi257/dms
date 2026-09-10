//! Meta-owned typed metrics for authority, journal, recovery and repair state.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dms_metrics::{
    ExemplarRecorder, Gauge, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec,
    IntGauge, IntGaugeVec, MetricExemplar, MetricsError, Opts, Registry, latency_buckets,
    register_collector,
};

#[derive(Clone)]
pub(crate) struct MetaMetrics {
    operations_total: IntCounterVec,
    operation_duration_seconds: HistogramVec,
    operation_duration_exemplars: ExemplarRecorder,
    mailbox_depth: IntGauge,
    mailbox_wait_duration_seconds: HistogramVec,
    commits_total: IntCounterVec,
    state_items: IntGaugeVec,
    journal_appends_total: IntCounterVec,
    journal_append_duration_seconds: HistogramVec,
    journal_append_exemplars: ExemplarRecorder,
    checkpoint_total: IntCounterVec,
    checkpoint_duration_seconds: Histogram,
    checkpoint_last_success_timestamp_seconds: Gauge,
    recovery_duration_seconds: Histogram,
    recovery_records_total: IntCounter,
    watch_streams: IntGauge,
    watch_events_total: IntCounterVec,
    watch_lag_events: IntGauge,
    node_sessions: IntGaugeVec,
    blocks: IntGaugeVec,
    repairs_pending: IntGauge,
    repair_attempts_total: IntCounterVec,
    repair_oldest_age_seconds: Gauge,
}

/// Meta 对外操作，同时也是 mailbox 等待时间的有界分类。
#[derive(Clone, Copy, Debug)]
pub(crate) enum MetaOperation {
    OpenNodeSession,
    Heartbeat,
    ResolveObject,
    ReportReplicas,
    CommitVersion,
    CommitBatch,
    Stat,
    Scan,
    GetOperation,
    PlanReplicas,
    WatchNodeEvents,
    AcknowledgeNodeEvent,
    #[cfg(test)]
    Stats,
}

impl MetaOperation {
    const LIVE: &'static [Self] = &[
        Self::OpenNodeSession,
        Self::Heartbeat,
        Self::ResolveObject,
        Self::ReportReplicas,
        Self::CommitVersion,
        Self::CommitBatch,
        Self::Stat,
        Self::Scan,
        Self::GetOperation,
        Self::PlanReplicas,
        Self::WatchNodeEvents,
        Self::AcknowledgeNodeEvent,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::OpenNodeSession => "open_node_session",
            Self::Heartbeat => "heartbeat",
            Self::ResolveObject => "resolve_object",
            Self::ReportReplicas => "report_replicas",
            Self::CommitVersion => "commit_version",
            Self::CommitBatch => "commit_batch",
            Self::Stat => "stat",
            Self::Scan => "scan",
            Self::GetOperation => "get_operation",
            Self::PlanReplicas => "plan_replicas",
            Self::WatchNodeEvents => "watch_node_events",
            Self::AcknowledgeNodeEvent => "acknowledge_node_event",
            #[cfg(test)]
            Self::Stats => "stats",
        }
    }

    /// Heartbeat is high-frequency liveness traffic. Metrics keep every call;
    /// successful command spans are opt-in to avoid polluting Tempo.
    pub(crate) const fn is_periodic(self) -> bool {
        matches!(self, Self::Heartbeat)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CommitOutcome {
    Committed,
    Idempotent,
    Conflict,
    Rejected,
}

impl CommitOutcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Idempotent => "idempotent",
            Self::Conflict => "conflict",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum JournalRecordMetric {
    NodeSessionOpened,
    ReplicaAccepted,
    ReplicasReported,
    VersionCommitted,
    VersionsCommitted,
    OperationRemembered,
    NodeEventAcknowledged,
    BlockRetirementPrepared,
    BlockRetirementAcknowledged,
    BlockRetirementFinalized,
    BlockRetirementReleased,
}

impl JournalRecordMetric {
    const LIVE: &'static [Self] = &[
        Self::NodeSessionOpened,
        Self::ReplicaAccepted,
        Self::ReplicasReported,
        Self::VersionCommitted,
        Self::VersionsCommitted,
        Self::OperationRemembered,
        Self::NodeEventAcknowledged,
        Self::BlockRetirementPrepared,
        Self::BlockRetirementAcknowledged,
        Self::BlockRetirementFinalized,
        Self::BlockRetirementReleased,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::NodeSessionOpened => "node_session_opened",
            Self::ReplicaAccepted => "replica_accepted",
            Self::ReplicasReported => "replicas_reported",
            Self::VersionCommitted => "version_committed",
            Self::VersionsCommitted => "versions_committed",
            Self::OperationRemembered => "operation_remembered",
            Self::NodeEventAcknowledged => "node_event_acknowledged",
            Self::BlockRetirementPrepared => "block_retirement_prepared",
            Self::BlockRetirementAcknowledged => "block_retirement_acknowledged",
            Self::BlockRetirementFinalized => "block_retirement_finalized",
            Self::BlockRetirementReleased => "block_retirement_released",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum WatchEventType {
    Invalidation,
    Repair,
    Eviction,
    Fence,
    Gap,
    Empty,
}

impl WatchEventType {
    const LIVE: &'static [Self] = &[
        Self::Invalidation,
        Self::Repair,
        Self::Eviction,
        Self::Fence,
        Self::Gap,
        Self::Empty,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Invalidation => "invalidation",
            Self::Repair => "repair",
            Self::Eviction => "eviction",
            Self::Fence => "fence",
            Self::Gap => "gap",
            Self::Empty => "empty",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum WatchDelivery {
    Delivered,
    Dropped,
    Replayed,
}

impl WatchDelivery {
    const fn label(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Dropped => "dropped",
            Self::Replayed => "replayed",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum RepairTransition {
    Issued,
    Completed,
    Expired,
}

impl RepairTransition {
    const fn label(self) -> &'static str {
        match self {
            Self::Issued => "issued",
            Self::Completed => "completed",
            Self::Expired => "expired",
        }
    }
}

/// 一次刷新 Meta 权威状态需要共同更新的 Gauge 快照。
pub(crate) struct MetaStateMetricsSnapshot {
    pub(crate) keys: usize,
    pub(crate) versions: usize,
    pub(crate) replicas: usize,
    pub(crate) operations: usize,
    pub(crate) sessions: usize,
    pub(crate) events: usize,
    pub(crate) watch_streams: usize,
    pub(crate) watch_lag_events: u64,
    pub(crate) live_sessions: usize,
    pub(crate) expired_sessions: usize,
    pub(crate) healthy_blocks: usize,
    pub(crate) under_replicated_blocks: usize,
    pub(crate) unavailable_blocks: usize,
    pub(crate) repairs_pending: usize,
    pub(crate) oldest_repair_age_seconds: f64,
}

impl MetaMetrics {
    pub(crate) fn register(registry: &Registry) -> Result<Self, MetricsError> {
        let buckets = latency_buckets();
        let metrics = Self {
            operations_total: counter_vec(
                "dms_meta_operations_total",
                "Completed Meta service operations.",
                &["operation", "result"],
            )?,
            operation_duration_seconds: histogram_vec(
                "dms_meta_operation_duration_seconds",
                "Meta service operation latency.",
                &["operation"],
            )?,
            operation_duration_exemplars: registry.exemplar_recorder(
                "dms_meta_operation_duration_seconds",
                &["operation"],
                &buckets,
            ),
            mailbox_depth: IntGauge::new(
                "dms_meta_mailbox_depth",
                "Commands queued for the Meta state owner.",
            )?,
            mailbox_wait_duration_seconds: histogram_vec(
                "dms_meta_mailbox_wait_duration_seconds",
                "Time a command waits before Meta receives it.",
                &["command"],
            )?,
            commits_total: counter_vec(
                "dms_meta_commits_total",
                "Version commits by authoritative outcome.",
                &["result"],
            )?,
            state_items: IntGaugeVec::new(
                Opts::new(
                    "dms_meta_state_items",
                    "Authoritative in-memory state cardinality.",
                ),
                &["type"],
            )?,
            journal_appends_total: counter_vec(
                "dms_meta_journal_appends_total",
                "Journal appends by record type and result.",
                &["record_type", "result"],
            )?,
            journal_append_duration_seconds: histogram_vec(
                "dms_meta_journal_append_duration_seconds",
                "Journal append latency.",
                &["record_type"],
            )?,
            journal_append_exemplars: registry.exemplar_recorder(
                "dms_meta_journal_append_duration_seconds",
                &["record_type"],
                &buckets,
            ),
            checkpoint_total: counter_vec(
                "dms_meta_checkpoint_total",
                "Checkpoint attempts by result.",
                &["result"],
            )?,
            checkpoint_duration_seconds: Histogram::with_opts(
                HistogramOpts::new(
                    "dms_meta_checkpoint_duration_seconds",
                    "Snapshot and journal truncation latency.",
                )
                .buckets(latency_buckets()),
            )?,
            checkpoint_last_success_timestamp_seconds: Gauge::new(
                "dms_meta_checkpoint_last_success_timestamp_seconds",
                "Unix timestamp of the last successful checkpoint.",
            )?,
            recovery_duration_seconds: Histogram::with_opts(
                HistogramOpts::new(
                    "dms_meta_recovery_duration_seconds",
                    "Meta snapshot load and journal replay latency.",
                )
                .buckets(latency_buckets()),
            )?,
            recovery_records_total: IntCounter::new(
                "dms_meta_recovery_records_total",
                "Journal records replayed during recovery.",
            )?,
            watch_streams: IntGauge::new(
                "dms_meta_watch_streams",
                "Active Node event watch streams.",
            )?,
            watch_events_total: counter_vec(
                "dms_meta_watch_events_total",
                "Node watch event deliveries by type and result.",
                &["event_type", "result"],
            )?,
            watch_lag_events: IntGauge::new(
                "dms_meta_watch_lag_events",
                "Maximum Node event cursor lag.",
            )?,
            node_sessions: IntGaugeVec::new(
                Opts::new("dms_meta_node_sessions", "Node sessions by liveness state."),
                &["state"],
            )?,
            blocks: IntGaugeVec::new(
                Opts::new("dms_meta_blocks", "Blocks by availability state."),
                &["state"],
            )?,
            repairs_pending: IntGauge::new(
                "dms_meta_repairs_pending",
                "Repair attempts not yet completed.",
            )?,
            repair_attempts_total: counter_vec(
                "dms_meta_repair_attempts_total",
                "Repair state transitions by result.",
                &["result"],
            )?,
            repair_oldest_age_seconds: Gauge::new(
                "dms_meta_repair_oldest_age_seconds",
                "Age of the oldest pending repair.",
            )?,
        };
        metrics.register_all(registry)?;
        metrics.initialize_bounded_series();
        Ok(metrics)
    }

    fn initialize_bounded_series(&self) {
        for operation in MetaOperation::LIVE {
            self.operation_duration_seconds
                .with_label_values(&[operation.label()]);
            for result in ["ok", "error"] {
                self.operations_total
                    .with_label_values(&[operation.label(), result]);
            }
        }
        for command in MetaOperation::LIVE {
            self.mailbox_wait_duration_seconds
                .with_label_values(&[command.label()]);
        }
        for result in ["committed", "idempotent", "conflict", "rejected"] {
            self.commits_total.with_label_values(&[result]);
        }
        for item_type in [
            "keys",
            "versions",
            "replicas",
            "operations",
            "sessions",
            "events",
        ] {
            self.state_items.with_label_values(&[item_type]).set(0);
        }
        for record_type in JournalRecordMetric::LIVE {
            self.journal_append_duration_seconds
                .with_label_values(&[record_type.label()]);
            for result in ["ok", "error"] {
                self.journal_appends_total
                    .with_label_values(&[record_type.label(), result]);
            }
        }
        for result in ["ok", "error"] {
            self.checkpoint_total.with_label_values(&[result]);
        }
        for event_type in WatchEventType::LIVE {
            for result in ["delivered", "dropped", "replayed"] {
                self.watch_events_total
                    .with_label_values(&[event_type.label(), result]);
            }
        }
        for state in ["live", "expired"] {
            self.node_sessions.with_label_values(&[state]).set(0);
        }
        for state in ["healthy", "under_replicated", "unavailable"] {
            self.blocks.with_label_values(&[state]).set(0);
        }
        for result in ["issued", "completed", "expired", "retargeted", "failed"] {
            self.repair_attempts_total.with_label_values(&[result]);
        }
    }

    fn register_all(&self, registry: &Registry) -> Result<(), MetricsError> {
        register_collector(registry, &self.operations_total)?;
        register_collector(registry, &self.operation_duration_seconds)?;
        register_collector(registry, &self.mailbox_depth)?;
        register_collector(registry, &self.mailbox_wait_duration_seconds)?;
        register_collector(registry, &self.commits_total)?;
        register_collector(registry, &self.state_items)?;
        register_collector(registry, &self.journal_appends_total)?;
        register_collector(registry, &self.journal_append_duration_seconds)?;
        register_collector(registry, &self.checkpoint_total)?;
        register_collector(registry, &self.checkpoint_duration_seconds)?;
        register_collector(registry, &self.checkpoint_last_success_timestamp_seconds)?;
        register_collector(registry, &self.recovery_duration_seconds)?;
        register_collector(registry, &self.recovery_records_total)?;
        register_collector(registry, &self.watch_streams)?;
        register_collector(registry, &self.watch_events_total)?;
        register_collector(registry, &self.watch_lag_events)?;
        register_collector(registry, &self.node_sessions)?;
        register_collector(registry, &self.blocks)?;
        register_collector(registry, &self.repairs_pending)?;
        register_collector(registry, &self.repair_attempts_total)?;
        register_collector(registry, &self.repair_oldest_age_seconds)?;
        Ok(())
    }

    pub(crate) fn mailbox_enqueued(&self) {
        self.mailbox_depth.inc();
    }

    pub(crate) fn mailbox_send_failed(&self) {
        self.mailbox_depth.dec();
    }

    pub(crate) fn record_mailbox_receive(&self, operation: MetaOperation, enqueued_at: Instant) {
        self.mailbox_depth.dec();
        self.mailbox_wait_duration_seconds
            .with_label_values(&[operation.label()])
            .observe(enqueued_at.elapsed().as_secs_f64());
    }

    pub(crate) fn begin_operation(&self, operation: MetaOperation) -> MetaOperationGuard {
        MetaOperationGuard {
            metrics: self.clone(),
            operation,
            started: Instant::now(),
            result: "error",
            exemplar: dms_tracing::current_exemplar(),
        }
    }

    pub(crate) fn record_commit(&self, outcome: CommitOutcome) {
        self.commits_total
            .with_label_values(&[outcome.label()])
            .inc();
    }

    pub(crate) fn begin_checkpoint(&self) -> CheckpointGuard {
        CheckpointGuard {
            metrics: self.clone(),
            started: Instant::now(),
            succeeded: false,
        }
    }

    pub(crate) fn begin_journal_append(
        &self,
        record_type: JournalRecordMetric,
    ) -> JournalAppendGuard {
        JournalAppendGuard {
            metrics: self.clone(),
            record_type,
            started: Instant::now(),
            succeeded: false,
            exemplar: dms_tracing::current_exemplar(),
        }
    }

    pub(crate) fn record_recovery(&self, records: usize, elapsed: Duration) {
        self.recovery_records_total.inc_by(records as u64);
        self.recovery_duration_seconds
            .observe(elapsed.as_secs_f64());
    }

    pub(crate) fn record_watch_event(&self, event: WatchEventType, result: WatchDelivery) {
        self.watch_events_total
            .with_label_values(&[event.label(), result.label()])
            .inc();
    }

    pub(crate) fn record_repair_transition(&self, transition: RepairTransition) {
        self.repair_attempts_total
            .with_label_values(&[transition.label()])
            .inc();
    }

    pub(crate) fn set_state(&self, snapshot: MetaStateMetricsSnapshot) {
        for (item_type, value) in [
            ("keys", snapshot.keys),
            ("versions", snapshot.versions),
            ("replicas", snapshot.replicas),
            ("operations", snapshot.operations),
            ("sessions", snapshot.sessions),
            ("events", snapshot.events),
        ] {
            self.state_items
                .with_label_values(&[item_type])
                .set(to_i64(value as u64));
        }
        self.watch_streams
            .set(to_i64(snapshot.watch_streams as u64));
        self.watch_lag_events.set(to_i64(snapshot.watch_lag_events));
        self.node_sessions
            .with_label_values(&["live"])
            .set(to_i64(snapshot.live_sessions as u64));
        self.node_sessions
            .with_label_values(&["expired"])
            .set(to_i64(snapshot.expired_sessions as u64));
        self.blocks
            .with_label_values(&["healthy"])
            .set(to_i64(snapshot.healthy_blocks as u64));
        self.blocks
            .with_label_values(&["under_replicated"])
            .set(to_i64(snapshot.under_replicated_blocks as u64));
        self.blocks
            .with_label_values(&["unavailable"])
            .set(to_i64(snapshot.unavailable_blocks as u64));
        self.repairs_pending
            .set(to_i64(snapshot.repairs_pending as u64));
        self.repair_oldest_age_seconds
            .set(snapshot.oldest_repair_age_seconds);
    }
}

pub(crate) struct MetaOperationGuard {
    metrics: MetaMetrics,
    operation: MetaOperation,
    started: Instant,
    result: &'static str,
    exemplar: Option<MetricExemplar>,
}

impl MetaOperationGuard {
    pub(crate) fn success(&mut self) {
        self.result = "ok";
    }
}

impl Drop for MetaOperationGuard {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_secs_f64();
        self.metrics
            .operations_total
            .with_label_values(&[self.operation.label(), self.result])
            .inc();
        self.metrics
            .operation_duration_seconds
            .with_label_values(&[self.operation.label()])
            .observe(elapsed);
        self.metrics.operation_duration_exemplars.record(
            &[self.operation.label()],
            elapsed,
            self.exemplar,
        );
    }
}

pub(crate) struct CheckpointGuard {
    metrics: MetaMetrics,
    started: Instant,
    succeeded: bool,
}

impl CheckpointGuard {
    pub(crate) fn success(&mut self) {
        self.succeeded = true;
    }
}

impl Drop for CheckpointGuard {
    fn drop(&mut self) {
        let result = if self.succeeded { "ok" } else { "error" };
        self.metrics
            .checkpoint_total
            .with_label_values(&[result])
            .inc();
        self.metrics
            .checkpoint_duration_seconds
            .observe(self.started.elapsed().as_secs_f64());
        if self.succeeded {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            self.metrics
                .checkpoint_last_success_timestamp_seconds
                .set(timestamp);
        }
    }
}

pub(crate) struct JournalAppendGuard {
    metrics: MetaMetrics,
    record_type: JournalRecordMetric,
    started: Instant,
    succeeded: bool,
    exemplar: Option<MetricExemplar>,
}

impl JournalAppendGuard {
    pub(crate) fn success(&mut self) {
        self.succeeded = true;
    }
}

impl Drop for JournalAppendGuard {
    fn drop(&mut self) {
        let result = if self.succeeded { "ok" } else { "error" };
        let elapsed = self.started.elapsed().as_secs_f64();
        self.metrics
            .journal_appends_total
            .with_label_values(&[self.record_type.label(), result])
            .inc();
        self.metrics
            .journal_append_duration_seconds
            .with_label_values(&[self.record_type.label()])
            .observe(elapsed);
        self.metrics.journal_append_exemplars.record(
            &[self.record_type.label()],
            elapsed,
            self.exemplar,
        );
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
    use super::*;
    use dms_metrics::{encode_text, registry};

    #[test]
    fn registers_the_twenty_one_meta_contracts() {
        let registry = registry();
        let metrics = MetaMetrics::register(&registry).expect("meta metrics");
        let mut guard = metrics.begin_operation(MetaOperation::ResolveObject);
        guard.success();
        drop(guard);
        let text = encode_text(&registry).expect("encode");
        assert!(text.contains("dms_meta_operations_total"));
        assert!(text.contains("dms_meta_repair_oldest_age_seconds"));
    }
}

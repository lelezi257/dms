//! Typed Prometheus handles owned by the Rust SDK.
//!
//! Metric names and labels are fixed here, while applications own the Registry
//! and decide how it is exported. No key, field, object, node ID, or operation
//! ID is ever used as a label.

use std::time::Instant;

use dms_metrics::{
    Histogram, HistogramVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, MetricsError, Opts,
    Registry, latency_buckets, register_collector,
};

#[derive(Clone)]
pub(crate) struct ClientMetrics {
    operations_total: IntCounterVec,
    operation_duration_seconds: HistogramVec,
    inflight_operations: IntGaugeVec,
    cache_lookups_total: IntCounterVec,
    cache_invalidations_total: IntCounterVec,
    node_connection_up: IntGauge,
    node_session_events_total: IntCounterVec,
    payload_transfers_total: IntCounterVec,
    payload_bytes_total: IntCounterVec,
    payload_transfer_duration_seconds: HistogramVec,
    region_mapping_lookups_total: IntCounterVec,
    region_mappings: IntGauge,
    operation_duration_exemplars: dms_metrics::ExemplarRecorder,
    payload_transfer_exemplars: dms_metrics::ExemplarRecorder,
}

/// Public SDK operations are a closed metric vocabulary, not caller-provided
/// Prometheus labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClientOperation {
    Set,
    Get,
    Delete,
    SetRange,
    MSet,
    MGet,
    HSet,
    HGet,
    HMGet,
    HDelete,
    HScan,
    HGetAll,
    HWriteAt,
    AllocateWrite,
    CommitShared,
    GetView,
}

impl ClientOperation {
    const ALL: [Self; 16] = [
        Self::Set,
        Self::Get,
        Self::Delete,
        Self::SetRange,
        Self::MSet,
        Self::MGet,
        Self::HSet,
        Self::HGet,
        Self::HMGet,
        Self::HDelete,
        Self::HScan,
        Self::HGetAll,
        Self::HWriteAt,
        Self::AllocateWrite,
        Self::CommitShared,
        Self::GetView,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Get => "get",
            Self::Delete => "del",
            Self::SetRange => "set_range",
            Self::MSet => "mset",
            Self::MGet => "mget",
            Self::HSet => "hset",
            Self::HGet => "hget",
            Self::HMGet => "hmget",
            Self::HDelete => "hdel",
            Self::HScan => "hscan",
            Self::HGetAll => "hgetall",
            Self::HWriteAt => "hwrite_at",
            Self::AllocateWrite => "allocate_write",
            Self::CommitShared => "commit_shared",
            Self::GetView => "get_view",
        }
    }

    /// Creates a stable public-operation span without recording keys or values.
    /// Static names keep Tempo searches useful while the closed enum prevents
    /// caller-provided high-cardinality span names.
    pub(crate) fn span(self) -> dms_tracing::tracing::Span {
        use dms_tracing::tracing::{field, info_span};
        // SDK 不安装宿主 Subscriber。无 Subscriber 时当前最大级别为 OFF，
        // 先返回无元数据 Span，避免 tracing/log 将禁用 Span 字段写入业务日志。
        // 宿主稍后初始化或使用线程局部 Subscriber 时，级别提示会同步更新；
        // 具体名称/字段过滤仍由原始调用点决定，不使用虚构 metadata 预过滤。
        if dms_tracing::tracing::level_filters::LevelFilter::current()
            < dms_tracing::tracing::Level::INFO
        {
            return dms_tracing::tracing::Span::none();
        }
        match self {
            Self::Set => info_span!(
                "dms.client.set",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::Get => info_span!(
                "dms.client.get",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::Delete => info_span!(
                "dms.client.del",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::SetRange => info_span!(
                "dms.client.set_range",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::MSet => info_span!(
                "dms.client.mset",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::MGet => info_span!(
                "dms.client.mget",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::HSet => info_span!(
                "dms.client.hset",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::HGet => info_span!(
                "dms.client.hget",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::HMGet => info_span!(
                "dms.client.hmget",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::HDelete => info_span!(
                "dms.client.hdel",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::HScan => info_span!(
                "dms.client.hscan",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::HGetAll => info_span!(
                "dms.client.hgetall",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::HWriteAt => info_span!(
                "dms.client.hwrite_at",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::AllocateWrite => info_span!(
                "dms.client.allocate_write",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::CommitShared => info_span!(
                "dms.client.commit_shared",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
            Self::GetView => info_span!(
                "dms.client.get_view",
                otel.kind = "client",
                result = field::Empty,
                error.code = field::Empty,
                error.kind = field::Empty
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CacheLookup {
    Hit,
    Miss,
    Stale,
}

impl CacheLookup {
    const ALL: [Self; 3] = [Self::Hit, Self::Miss, Self::Stale];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Stale => "stale",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CacheInvalidation {
    Evicted,
    Ignored,
}

impl CacheInvalidation {
    const ALL: [Self; 2] = [Self::Evicted, Self::Ignored];

    const fn label(self) -> &'static str {
        match self {
            Self::Evicted => "evicted",
            Self::Ignored => "ignored",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeSessionEvent {
    Connected,
    Disconnected,
}

impl NodeSessionEvent {
    const ALL: [Self; 2] = [Self::Connected, Self::Disconnected];

    const fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransferDirection {
    Read,
    Write,
}

impl TransferDirection {
    const ALL: [Self; 2] = [Self::Read, Self::Write];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransferProvider {
    Shm,
    Grpc,
    Rdma,
    Ub,
    Unknown,
}

impl TransferProvider {
    const LIVE: [Self; 2] = [Self::Shm, Self::Grpc];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Shm => "shm",
            Self::Grpc => "grpc",
            Self::Rdma => "rdma",
            Self::Ub => "ub",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegionMappingLookup {
    Hit,
    Miss,
}

impl RegionMappingLookup {
    const ALL: [Self; 2] = [Self::Hit, Self::Miss];

    const fn label(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
        }
    }
}

impl ClientMetrics {
    pub(crate) fn register(registry: &Registry) -> Result<Self, MetricsError> {
        let metrics = Self {
            operations_total: IntCounterVec::new(
                Opts::new(
                    "dms_client_operations_total",
                    "Completed public SDK operations.",
                ),
                &["operation", "result"],
            )?,
            operation_duration_seconds: HistogramVec::new(
                dms_metrics::HistogramOpts::new(
                    "dms_client_operation_duration_seconds",
                    "End-to-end public SDK operation latency in seconds.",
                )
                .buckets(latency_buckets()),
                &["operation"],
            )?,
            inflight_operations: IntGaugeVec::new(
                Opts::new(
                    "dms_client_inflight_operations",
                    "Public SDK operations currently executing.",
                ),
                &["operation"],
            )?,
            cache_lookups_total: IntCounterVec::new(
                Opts::new(
                    "dms_client_cache_lookups_total",
                    "Current-value cache lookups.",
                ),
                &["result"],
            )?,
            cache_invalidations_total: IntCounterVec::new(
                Opts::new(
                    "dms_client_cache_invalidations_total",
                    "Current-value invalidation and local capacity eviction events.",
                ),
                &["result"],
            )?,
            node_connection_up: IntGauge::new(
                "dms_client_node_connection_up",
                "Number of usable Node sessions sharing this host registry.",
            )?,
            node_session_events_total: IntCounterVec::new(
                Opts::new(
                    "dms_client_node_session_events_total",
                    "Lifecycle events of the active Node session stream.",
                ),
                &["event"],
            )?,
            payload_transfers_total: IntCounterVec::new(
                Opts::new(
                    "dms_client_payload_transfers_total",
                    "Completed payload provider attempts.",
                ),
                &["direction", "provider", "result"],
            )?,
            payload_bytes_total: IntCounterVec::new(
                Opts::new(
                    "dms_client_payload_bytes_total",
                    "Payload bytes transferred successfully.",
                ),
                &["direction", "provider"],
            )?,
            payload_transfer_duration_seconds: HistogramVec::new(
                dms_metrics::HistogramOpts::new(
                    "dms_client_payload_transfer_duration_seconds",
                    "Payload-provider transfer latency in seconds.",
                )
                .buckets(latency_buckets()),
                &["direction", "provider"],
            )?,
            region_mapping_lookups_total: IntCounterVec::new(
                Opts::new(
                    "dms_client_region_mapping_lookups_total",
                    "Region mapping cache lookups.",
                ),
                &["result"],
            )?,
            region_mappings: IntGauge::new(
                "dms_client_region_mappings",
                "Regions currently mmap-ed by this SDK process.",
            )?,
            operation_duration_exemplars: registry.exemplar_recorder(
                "dms_client_operation_duration_seconds",
                &["operation"],
                &latency_buckets(),
            ),
            payload_transfer_exemplars: registry.exemplar_recorder(
                "dms_client_payload_transfer_duration_seconds",
                &["direction", "provider"],
                &latency_buckets(),
            ),
        };
        register_collector(registry, &metrics.operations_total)?;
        register_collector(registry, &metrics.operation_duration_seconds)?;
        register_collector(registry, &metrics.inflight_operations)?;
        register_collector(registry, &metrics.cache_lookups_total)?;
        register_collector(registry, &metrics.cache_invalidations_total)?;
        register_collector(registry, &metrics.node_connection_up)?;
        register_collector(registry, &metrics.node_session_events_total)?;
        register_collector(registry, &metrics.payload_transfers_total)?;
        register_collector(registry, &metrics.payload_bytes_total)?;
        register_collector(registry, &metrics.payload_transfer_duration_seconds)?;
        register_collector(registry, &metrics.region_mapping_lookups_total)?;
        register_collector(registry, &metrics.region_mappings)?;
        metrics.initialize_bounded_series();
        Ok(metrics)
    }

    /// Materializes the finite label vocabulary at zero so a fresh process
    /// exposes the complete contract before the first request arrives.
    fn initialize_bounded_series(&self) {
        for operation in ClientOperation::ALL {
            let operation = operation.label();
            self.inflight_operations
                .with_label_values(&[operation])
                .set(0);
            self.operation_duration_seconds
                .with_label_values(&[operation]);
            for result in ["ok", "error"] {
                self.operations_total
                    .with_label_values(&[operation, result]);
            }
        }
        for result in CacheLookup::ALL {
            self.cache_lookups_total
                .with_label_values(&[result.label()]);
        }
        for result in CacheInvalidation::ALL {
            self.cache_invalidations_total
                .with_label_values(&[result.label()]);
        }
        for event in NodeSessionEvent::ALL {
            self.node_session_events_total
                .with_label_values(&[event.label()]);
        }
        for direction in TransferDirection::ALL {
            for provider in TransferProvider::LIVE {
                self.payload_bytes_total
                    .with_label_values(&[direction.label(), provider.label()]);
                self.payload_transfer_duration_seconds
                    .with_label_values(&[direction.label(), provider.label()]);
                for result in ["ok", "error"] {
                    self.payload_transfers_total.with_label_values(&[
                        direction.label(),
                        provider.label(),
                        result,
                    ]);
                }
            }
        }
        for result in RegionMappingLookup::ALL {
            self.region_mapping_lookups_total
                .with_label_values(&[result.label()]);
        }
    }

    /// Starts one public SDK operation and owns its inflight/count/latency trio.
    pub(crate) fn begin_operation(&self, operation: ClientOperation) -> OperationGuard {
        let inflight = self
            .inflight_operations
            .with_label_values(&[operation.label()]);
        inflight.inc();
        OperationGuard {
            inflight,
            operations: self.operations_total.clone(),
            duration: self
                .operation_duration_seconds
                .with_label_values(&[operation.label()]),
            exemplars: self.operation_duration_exemplars.clone(),
            operation,
            started: Instant::now(),
            result: "error",
            exemplar: dms_tracing::current_exemplar(),
        }
    }

    pub(crate) fn record_cache_lookup(&self, result: CacheLookup) {
        self.cache_lookups_total
            .with_label_values(&[result.label()])
            .inc();
    }

    pub(crate) fn record_cache_invalidation(&self, result: CacheInvalidation) {
        self.cache_invalidations_total
            .with_label_values(&[result.label()])
            .inc();
    }

    pub(crate) fn record_cache_capacity_evictions(&self, count: usize) {
        self.cache_invalidations_total
            .with_label_values(&[CacheInvalidation::Evicted.label()])
            .inc_by(count as u64);
    }

    pub(crate) fn node_connection_guard(&self) -> NodeConnectionGuard {
        self.node_connection_up.inc();
        NodeConnectionGuard(self.node_connection_up.clone())
    }

    pub(crate) fn record_node_session_event(&self, event: NodeSessionEvent) {
        self.node_session_events_total
            .with_label_values(&[event.label()])
            .inc();
    }

    /// Starts one provider attempt. The guard records one result and latency;
    /// successful completion additionally records the transferred byte count.
    pub(crate) fn begin_transfer(
        &self,
        direction: TransferDirection,
        provider: TransferProvider,
    ) -> TransferGuard {
        TransferGuard {
            transfers: self.payload_transfers_total.clone(),
            duration: self
                .payload_transfer_duration_seconds
                .with_label_values(&[direction.label(), provider.label()]),
            payload_bytes: self
                .payload_bytes_total
                .with_label_values(&[direction.label(), provider.label()]),
            exemplars: self.payload_transfer_exemplars.clone(),
            direction,
            provider,
            started: Instant::now(),
            bytes: None,
            exemplar: dms_tracing::current_exemplar(),
        }
    }

    pub(crate) fn record_region_mapping_lookup(&self, result: RegionMappingLookup) {
        self.region_mapping_lookups_total
            .with_label_values(&[result.label()])
            .inc();
    }

    pub(crate) fn region_mapping_guard(&self) -> RegionMappingGuard {
        self.region_mappings.inc();
        RegionMappingGuard(self.region_mappings.clone())
    }
}

/// 由真正持有 mmap 的对象拥有，缓存淘汰但 View 尚存时不能提前扣减。
#[derive(Debug)]
pub(crate) struct RegionMappingGuard(IntGauge);

impl Drop for RegionMappingGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

/// 共享 Registry 下统计存活 Session 数；取消任务也通过 Drop 关闭计数。
pub(crate) struct NodeConnectionGuard(IntGauge);

impl Drop for NodeConnectionGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

pub(crate) struct OperationGuard {
    inflight: IntGauge,
    operations: IntCounterVec,
    duration: Histogram,
    exemplars: dms_metrics::ExemplarRecorder,
    operation: ClientOperation,
    started: Instant,
    result: &'static str,
    exemplar: Option<dms_metrics::MetricExemplar>,
}

impl OperationGuard {
    pub(crate) fn success(&mut self) {
        self.result = "ok";
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_secs_f64();
        self.inflight.dec();
        self.operations
            .with_label_values(&[self.operation.label(), self.result])
            .inc();
        self.duration.observe(elapsed);
        self.exemplars
            .record(&[self.operation.label()], elapsed, self.exemplar);
    }
}

pub(crate) struct TransferGuard {
    transfers: IntCounterVec,
    duration: Histogram,
    payload_bytes: IntCounter,
    exemplars: dms_metrics::ExemplarRecorder,
    direction: TransferDirection,
    provider: TransferProvider,
    started: Instant,
    bytes: Option<usize>,
    exemplar: Option<dms_metrics::MetricExemplar>,
}

impl TransferGuard {
    pub(crate) fn success(&mut self, bytes: usize) {
        self.bytes = Some(bytes);
    }
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        let direction = self.direction.label();
        let provider = self.provider.label();
        let result = if self.bytes.is_some() { "ok" } else { "error" };
        self.transfers
            .with_label_values(&[direction, provider, result])
            .inc();
        let elapsed = self.started.elapsed().as_secs_f64();
        self.duration.observe(elapsed);
        self.exemplars
            .record(&[direction, provider], elapsed, self.exemplar);
        if let Some(bytes) = self.bytes {
            self.payload_bytes.inc_by(bytes as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use dms_metrics::{encode_text, registry};

    use super::*;

    struct HostLogger(std::sync::Mutex<Vec<String>>);

    static HOST_LOGGER: HostLogger = HostLogger(std::sync::Mutex::new(Vec::new()));

    impl log::Log for HostLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Info
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                self.0.lock().unwrap().push(record.args().to_string());
            }
        }

        fn flush(&self) {}
    }

    struct HostSubscriber(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl dms_tracing::tracing::Subscriber for HostSubscriber {
        fn enabled(&self, metadata: &dms_tracing::tracing::Metadata<'_>) -> bool {
            metadata.is_span()
                && (metadata.name().starts_with("dms.client.")
                    || metadata.name().starts_with("dms.payload."))
        }
        fn new_span(
            &self,
            _: &dms_tracing::tracing::span::Attributes<'_>,
        ) -> dms_tracing::tracing::span::Id {
            dms_tracing::tracing::span::Id::from_u64(
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u64 + 1,
            )
        }
        fn record(
            &self,
            _: &dms_tracing::tracing::span::Id,
            _: &dms_tracing::tracing::span::Record<'_>,
        ) {
        }
        fn record_follows_from(
            &self,
            _: &dms_tracing::tracing::span::Id,
            _: &dms_tracing::tracing::span::Id,
        ) {
        }
        fn event(&self, _: &dms_tracing::tracing::Event<'_>) {}
        fn enter(&self, _: &dms_tracing::tracing::span::Id) {}
        fn exit(&self, _: &dms_tracing::tracing::span::Id) {}
    }

    fn exercise_sdk_span_sites() {
        for operation in ClientOperation::ALL {
            let span = operation.span();
            dms_tracing::record_ok(&span);
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let channel =
                tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
            let engine =
                crate::internal::transfer_engine::TransferEngine::new(channel, 1, None, None, None);
            assert!(engine.upload(1, Default::default(), b"x").await.is_err());
            assert!(engine.download(1, Default::default()).await.is_err());
        });
    }

    // 模拟只安装 env_logger 类标准日志后端的宿主；每次重新启动测试进程，
    // 避免其它用例注册过 Subscriber 后把隐式 fallback 永久关闭而掩盖缺陷。
    #[test]
    fn sdk_spans_do_not_fall_back_to_host_logs() {
        if std::env::var_os("DMS_SDK_LOG_ONLY_PROBE").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "metrics::tests::sdk_spans_do_not_fall_back_to_host_logs",
                    "--nocapture",
                ])
                .env("DMS_SDK_LOG_ONLY_PROBE", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        log::set_logger(&HOST_LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Info);
        exercise_sdk_span_sites();
        log::info!("host_business_log_sentinel");
        let records = HOST_LOGGER.0.lock().unwrap();
        assert!(
            records
                .iter()
                .any(|record| record == "host_business_log_sentinel")
        );
        assert!(
            !records
                .iter()
                .any(|record| record.contains("dms.client.") || record.contains("dms.payload.")),
            "{records:?}"
        );
        drop(records);

        // SDK 使用之后宿主仍可安装自己的 Subscriber，且每个原始调用点继续
        // 遵循名称过滤。避免通过默认禁用 Subscriber 或泛化 enabled! 吞掉宿主配置。
        let spans = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        dms_tracing::tracing::subscriber::set_global_default(HostSubscriber(spans.clone()))
            .unwrap();
        exercise_sdk_span_sites();
        assert_eq!(
            spans.load(std::sync::atomic::Ordering::Relaxed),
            ClientOperation::ALL.len() + 2
        );
    }

    #[test]
    fn lifetime_gauges_close_each_shared_owner() {
        let registry = registry();
        let metrics = ClientMetrics::register(&registry).unwrap();
        let first = metrics.node_connection_guard();
        let second = metrics.node_connection_guard();
        assert_eq!(metrics.node_connection_up.get(), 2);
        drop(first);
        assert_eq!(metrics.node_connection_up.get(), 1);
        drop(second);
        assert_eq!(metrics.node_connection_up.get(), 0);
        let mapping = std::sync::Arc::new(metrics.region_mapping_guard());
        let retained_view = mapping.clone();
        drop(mapping);
        assert_eq!(metrics.region_mappings.get(), 1);
        drop(retained_view);
        assert_eq!(metrics.region_mappings.get(), 0);
        metrics.record_cache_capacity_evictions(3);
        assert_eq!(
            metrics
                .cache_invalidations_total
                .with_label_values(&["evicted"])
                .get(),
            3
        );
    }

    #[test]
    fn registers_the_twelve_client_contracts() {
        let registry = registry();
        let metrics = ClientMetrics::register(&registry).expect("client metrics");
        let mut guard = metrics.begin_operation(ClientOperation::Get);
        guard.success();
        drop(guard);
        let mut transfer = metrics.begin_transfer(TransferDirection::Read, TransferProvider::Grpc);
        transfer.success(3);
        drop(transfer);
        metrics.record_cache_invalidation(CacheInvalidation::Evicted);
        metrics.record_node_session_event(NodeSessionEvent::Connected);
        let text = encode_text(&registry).expect("encode");
        assert!(text.contains("dms_client_operations_total"));
        assert!(text.contains("dms_client_payload_bytes_total"));
        assert!(text.contains("dms_client_cache_invalidations_total{result=\"evicted\"} 1"));
        assert!(text.contains("dms_client_node_session_events_total{event=\"connected\"} 1"));
        assert!(!text.contains("dms_client_provider_fallbacks_total"));
    }
}

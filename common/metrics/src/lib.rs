//! Shared Prometheus mechanics for DMS processes and SDK hosts.
//!
//! This crate owns only cross-cutting mechanics and cross-process metric
//! contracts. Client, Node and Meta business metrics stay in their own modules.
//! A [`Registry`] is always created by the embedding process; this crate never
//! installs a global registry or opens a network listener.

#![forbid(unsafe_code)]

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt,
    hash::{Hash, Hasher},
    sync::OnceLock,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use dms_error::DmsError;
use prometheus::{Encoder, TextEncoder};
pub use prometheus::{
    Gauge, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts,
};

/// Process-owned collector registry plus a small OpenMetrics exemplar sidecar.
///
/// `prometheus` 0.14 owns counters/histograms. The sidecar stores at most one
/// sampled trace per bounded histogram bucket and augments exposition only;
/// it is never consulted by a business operation.
#[derive(Clone)]
pub struct Registry {
    inner: prometheus::Registry,
    exemplars: Arc<Mutex<ExemplarStore>>,
    bundles: Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: prometheus::Registry::new(),
            exemplars: Arc::new(Mutex::new(HashMap::new())),
            bundles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register(
        &self,
        collector: Box<dyn prometheus::core::Collector>,
    ) -> Result<(), prometheus::Error> {
        self.inner.register(collector)
    }

    /// 注册并复用宿主 Registry 内的一组句柄；独立 Registry 之间不共享状态。
    ///
    /// 初始化只发生一次并与其它注册者串行；factory 不得递归调用本方法。
    /// factory 应先构造 Collector 再注册，失败可能留下已注册的部分 Collector。
    pub fn get_or_register<T>(
        &self,
        factory: impl FnOnce(&Self) -> Result<T, MetricsError>,
    ) -> Result<T, MetricsError>
    where
        T: Clone + Send + Sync + 'static,
    {
        let mut bundles = self
            .bundles
            .lock()
            .map_err(|_| MetricsError::BundlePoisoned)?;
        if let Some(bundle) = bundles.get(&TypeId::of::<T>()) {
            return Ok(bundle
                .downcast_ref::<T>()
                .expect("bundle type identity")
                .clone());
        }
        let bundle = factory(self)?;
        bundles.insert(TypeId::of::<T>(), Box::new(bundle.clone()));
        Ok(bundle)
    }

    #[must_use]
    pub fn gather(&self) -> Vec<prometheus::proto::MetricFamily> {
        self.inner.gather()
    }

    #[must_use]
    pub fn exemplar_recorder(
        &self,
        metric: &'static str,
        labels: &'static [&'static str],
        buckets: &[f64],
    ) -> ExemplarRecorder {
        let mut label_order = (0..labels.len()).collect::<Vec<_>>();
        label_order.sort_unstable_by_key(|index| labels[*index]);
        ExemplarRecorder {
            metric,
            labels,
            buckets: Arc::from(buckets),
            label_order: Arc::from(label_order),
            store: Arc::clone(&self.exemplars),
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Registry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Registry").finish_non_exhaustive()
    }
}

pub const OPENMETRICS_CONTENT_TYPE: &str =
    "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Trace identity attached to one selected histogram observation.
///
/// It is deliberately a fixed 16-byte value. `trace_id` is never a metric
/// label; hexadecimal encoding is deferred until OpenMetrics exposition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricExemplar {
    trace_id: [u8; 16],
}

type ExemplarProvider = fn() -> Option<MetricExemplar>;
static EXEMPLAR_PROVIDER: OnceLock<ExemplarProvider> = OnceLock::new();

/// Installs the trace bridge without making Metrics depend on one tracing SDK.
/// Repeated installation is intentionally harmless for SDK/server composition.
pub fn install_exemplar_provider(provider: ExemplarProvider) {
    let _ = EXEMPLAR_PROVIDER.set(provider);
}

fn capture_exemplar() -> Option<MetricExemplar> {
    EXEMPLAR_PROVIDER.get().and_then(|provider| provider())
}

/// Typed handle used beside one existing `HistogramVec`.
///
/// Labels must be the same bounded labels used by that histogram. The trace ID
/// is attached to a bucket sample, never promoted to a Prometheus label.
#[derive(Clone)]
pub struct ExemplarRecorder {
    metric: &'static str,
    labels: &'static [&'static str],
    // 配置只读，Guard 克隆不再为未采样请求重复分配 bucket 数组。
    buckets: Arc<[f64]>,
    label_order: Arc<[usize]>,
    store: Arc<Mutex<ExemplarStore>>,
}

// 以稳定身份散列定位候选，再比较完整标签消解碰撞。热身后更新已有
// series 不复制标签、不排序、不编码 Trace ID；仅首次采样建立有界系列。
type ExemplarStore = HashMap<u64, Vec<(ExemplarSeries, ExemplarValue)>>;

impl ExemplarRecorder {
    pub fn record(&self, labels: &[&str], value: f64, exemplar: Option<MetricExemplar>) {
        let Some(exemplar) = exemplar else {
            return;
        };
        if labels.len() != self.labels.len() {
            return;
        }
        let upper_bound = self
            .buckets
            .iter()
            .copied()
            .find(|bound| value <= *bound)
            .unwrap_or(f64::INFINITY);
        let ordered_labels = || {
            self.label_order
                .iter()
                .map(|index| (self.labels[*index], labels[*index]))
        };
        let mut identity = std::collections::hash_map::DefaultHasher::new();
        self.metric.hash(&mut identity);
        upper_bound.to_bits().hash(&mut identity);
        for label in ordered_labels() {
            label.hash(&mut identity);
        }
        let observed_at_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as u64);
        let value = ExemplarValue {
            trace_id: exemplar.trace_id(),
            observed_value: value,
            observed_at_millis,
        };
        if let Ok(mut store) = self.store.lock() {
            let entries = store.entry(identity.finish()).or_default();
            if let Some((_, previous)) = entries.iter_mut().find(|(series, _)| {
                series.metric == self.metric
                    && series.upper_bound_bits == upper_bound.to_bits()
                    && series.labels.len() == self.labels.len()
                    && series.labels.iter().zip(ordered_labels()).all(
                        |((name, value), (expected_name, expected_value))| {
                            name == expected_name && value == expected_value
                        },
                    )
            }) {
                *previous = value;
            } else {
                entries.push((
                    ExemplarSeries {
                        metric: self.metric,
                        labels: ordered_labels()
                            .map(|(name, value)| (name.to_owned(), value.to_owned()))
                            .collect(),
                        upper_bound_bits: upper_bound.to_bits(),
                    },
                    value,
                ));
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ExemplarSeries {
    metric: &'static str,
    labels: Vec<(String, String)>,
    upper_bound_bits: u64,
}

#[derive(Clone, Debug)]
struct ExemplarValue {
    trace_id: [u8; 16],
    observed_value: f64,
    observed_at_millis: u64,
}

impl MetricExemplar {
    #[must_use]
    pub const fn new(trace_id: [u8; 16]) -> Self {
        Self { trace_id }
    }

    #[must_use]
    pub const fn trace_id(self) -> [u8; 16] {
        self.trace_id
    }
}

/// Registration or text-encoding failure at process composition time.
#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("metrics bundle initialization lock poisoned")]
    BundlePoisoned,
    #[error("Prometheus collector registration failed: {0}")]
    Prometheus(#[from] prometheus::Error),
    #[error("Prometheus text encoding produced invalid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// Creates one explicit process-owned registry.
#[must_use]
pub fn registry() -> Registry {
    Registry::new()
}

/// Encodes one OpenMetrics snapshot, including sampled histogram exemplars.
pub fn encode_text(registry: &Registry) -> Result<String, MetricsError> {
    let mut buffer = Vec::new();
    TextEncoder::new().encode(&registry.gather(), &mut buffer)?;
    let prometheus = String::from_utf8(buffer)?;
    let exemplars = registry.exemplars.lock().map_or_else(
        |_| HashMap::new(),
        |store| store.values().flatten().cloned().collect(),
    );
    let mut output = String::with_capacity(prometheus.len() + 128);
    for line in prometheus.lines() {
        output.push_str(line);
        if let Some(exemplar) = exemplar_for_line(line, &exemplars) {
            output.push_str(" # {trace_id=\"");
            output.push_str(&hex_trace_id(exemplar.trace_id));
            output.push_str("\"} ");
            output.push_str(&exemplar.observed_value.to_string());
            output.push(' ');
            output.push_str(&exemplar.observed_at_millis.to_string());
        }
        output.push('\n');
    }
    output.push_str("# EOF\n");
    Ok(output)
}

fn exemplar_for_line<'a>(
    line: &str,
    exemplars: &'a HashMap<ExemplarSeries, ExemplarValue>,
) -> Option<&'a ExemplarValue> {
    let (metric_and_labels, _) = line.split_once(' ')?;
    let (metric, raw_labels) = metric_and_labels.split_once('{')?;
    let metric = metric.strip_suffix("_bucket")?;
    let raw_labels = raw_labels.strip_suffix('}')?;
    let mut labels = Vec::new();
    let mut upper_bound = None;
    for pair in raw_labels.split(',') {
        let (name, value) = pair.split_once('=')?;
        let value = value.trim_matches('"');
        if name == "le" {
            upper_bound = Some(if value == "+Inf" {
                f64::INFINITY
            } else {
                value.parse::<f64>().ok()?
            });
        } else {
            labels.push((name.to_string(), value.to_string()));
        }
    }
    labels.sort_unstable();
    exemplars.get(&ExemplarSeries {
        metric: exemplars.keys().find(|key| key.metric == metric)?.metric,
        labels,
        upper_bound_bits: upper_bound?.to_bits(),
    })
}

fn hex_trace_id(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(32);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

/// A bounded gRPC service/method label pair.
///
/// Business code selects one of these constants instead of spelling Prometheus
/// labels itself. This keeps the metric vocabulary finite and makes a proto
/// method rename a compile-time change rather than a silent new time series.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RpcCall {
    service: &'static str,
    method: &'static str,
}

impl RpcCall {
    const fn new(service: &'static str, method: &'static str) -> Self {
        Self { service, method }
    }

    pub const WORKER_OPEN_SESSION: Self = Self::new("WorkerService", "OpenSession");
    pub const WORKER_SESSION: Self = Self::new("WorkerService", "Session");
    pub const WORKER_HEARTBEAT: Self = Self::new("WorkerService", "Heartbeat");
    pub const WORKER_ALLOCATE_STAGING: Self = Self::new("WorkerService", "AllocateStaging");
    pub const WORKER_DELETE_STAGING: Self = Self::new("WorkerService", "DeleteStaging");
    pub const WORKER_SET_INLINE: Self = Self::new("WorkerService", "SetInline");
    pub const WORKER_SET: Self = Self::new("WorkerService", "Set");
    pub const WORKER_DELETE: Self = Self::new("WorkerService", "Delete");
    pub const WORKER_GET: Self = Self::new("WorkerService", "Get");
    pub const WORKER_STAT: Self = Self::new("WorkerService", "Stat");
    pub const WORKER_SCAN: Self = Self::new("WorkerService", "Scan");
    pub const WORKER_MSET: Self = Self::new("WorkerService", "MSet");
    pub const WORKER_MGET: Self = Self::new("WorkerService", "MGet");
    pub const WORKER_SET_RANGE: Self = Self::new("WorkerService", "SetRange");
    pub const WORKER_HSET: Self = Self::new("WorkerService", "HSet");
    pub const WORKER_HGET: Self = Self::new("WorkerService", "HGet");
    pub const WORKER_HMGET: Self = Self::new("WorkerService", "HMGet");
    pub const WORKER_HGET_ALL: Self = Self::new("WorkerService", "HGetAll");
    pub const WORKER_HDELETE: Self = Self::new("WorkerService", "HDelete");
    pub const WORKER_HSCAN: Self = Self::new("WorkerService", "HScan");
    pub const WORKER_HWRITE_AT: Self = Self::new("WorkerService", "HWriteAt");
    pub const WORKER_ACQUIRE_REGION: Self = Self::new("WorkerService", "AcquireRegion");
    pub const PAYLOAD_UPLOAD: Self = Self::new("WorkerPayloadService", "Upload");
    pub const PAYLOAD_DOWNLOAD: Self = Self::new("WorkerPayloadService", "Download");
    pub const PEER_PROBE: Self = Self::new("PeerService", "Probe");
    pub const PEER_PULL_BLOCK: Self = Self::new("PeerService", "PullBlock");
    pub const PEER_PREPARE_REPLICA: Self = Self::new("PeerService", "PrepareReplica");
    pub const PEER_ACTIVATE_REPLICA: Self = Self::new("PeerService", "ActivateReplica");
    pub const PEER_ABORT_REPLICA: Self = Self::new("PeerService", "AbortReplica");
    pub const PEER_GET_REPLICA_STATUS: Self = Self::new("PeerService", "GetReplicaStatus");
    pub const META_OPEN_NODE_SESSION: Self = Self::new("MetadataService", "OpenNodeSession");
    pub const META_HEARTBEAT: Self = Self::new("MetadataService", "Heartbeat");
    pub const META_RESOLVE_OBJECT: Self = Self::new("MetadataService", "ResolveObject");
    pub const META_RESOLVE_OBJECTS: Self = Self::new("MetadataService", "ResolveObjects");
    pub const META_REPORT_REPLICAS: Self = Self::new("MetadataService", "ReportReplicas");
    pub const META_COMMIT_VERSION: Self = Self::new("MetadataService", "CommitVersion");
    pub const META_COMMIT_BATCH: Self = Self::new("MetadataService", "CommitBatch");
    pub const META_STAT: Self = Self::new("MetadataService", "Stat");
    pub const META_SCAN: Self = Self::new("MetadataService", "Scan");
    pub const META_GET_OPERATION: Self = Self::new("MetadataService", "GetOperation");
    pub const META_PLAN_REPLICAS: Self = Self::new("MetadataService", "PlanReplicas");
    pub const META_WATCH_NODE_EVENTS: Self = Self::new("MetadataService", "WatchNodeEvents");
    pub const META_ACKNOWLEDGE_NODE_EVENT: Self =
        Self::new("MetadataService", "AcknowledgeNodeEvent");
    pub const META_ACKNOWLEDGE_BLOCK_RETIREMENT: Self =
        Self::new("MetadataService", "AcknowledgeBlockRetirement");
}

/// Five transport-boundary collectors shared by every gRPC relationship.
///
/// `Registry` owns the scrape side of each collector. `RpcMetrics` is the
/// typed write handle retained by the caller/server. The Prometheus collector
/// clones share one underlying metric state; registering does not duplicate
/// counters.
#[derive(Clone)]
pub struct RpcMetrics {
    client_requests_total: IntCounterVec,
    client_duration_seconds: HistogramVec,
    server_requests_total: IntCounterVec,
    server_duration_seconds: HistogramVec,
    server_inflight_requests: IntGaugeVec,
    client_duration_exemplars: ExemplarRecorder,
    server_duration_exemplars: ExemplarRecorder,
}

impl RpcMetrics {
    pub fn register(registry: &Registry) -> Result<Self, MetricsError> {
        let metrics = Self {
            client_requests_total: IntCounterVec::new(
                Opts::new(
                    "dms_rpc_client_requests_total",
                    "Completed outbound gRPC calls.",
                ),
                &["service", "method", "result"],
            )?,
            client_duration_seconds: HistogramVec::new(
                HistogramOpts::new(
                    "dms_rpc_client_duration_seconds",
                    "Outbound gRPC call latency in seconds.",
                )
                .buckets(latency_buckets()),
                &["service", "method"],
            )?,
            server_requests_total: IntCounterVec::new(
                Opts::new(
                    "dms_rpc_server_requests_total",
                    "Completed inbound gRPC calls.",
                ),
                &["service", "method", "result"],
            )?,
            server_duration_seconds: HistogramVec::new(
                HistogramOpts::new(
                    "dms_rpc_server_duration_seconds",
                    "Inbound gRPC handler latency in seconds.",
                )
                .buckets(latency_buckets()),
                &["service", "method"],
            )?,
            server_inflight_requests: IntGaugeVec::new(
                Opts::new(
                    "dms_rpc_server_inflight_requests",
                    "Inbound gRPC calls currently executing.",
                ),
                &["service", "method"],
            )?,
            client_duration_exemplars: registry.exemplar_recorder(
                "dms_rpc_client_duration_seconds",
                &["service", "method"],
                &latency_buckets(),
            ),
            server_duration_exemplars: registry.exemplar_recorder(
                "dms_rpc_server_duration_seconds",
                &["service", "method"],
                &latency_buckets(),
            ),
        };
        register_collector(registry, &metrics.client_requests_total)?;
        register_collector(registry, &metrics.client_duration_seconds)?;
        register_collector(registry, &metrics.server_requests_total)?;
        register_collector(registry, &metrics.server_duration_seconds)?;
        register_collector(registry, &metrics.server_inflight_requests)?;
        Ok(metrics)
    }

    /// Begins one outbound gRPC call.
    ///
    /// The returned guard defaults to `error`. A normal success must be marked
    /// explicitly; every return path then records count and latency in `Drop`.
    #[must_use]
    pub fn begin_client_call(&self, call: RpcCall) -> RpcClientCallGuard {
        RpcClientCallGuard {
            requests: self.client_requests_total.clone(),
            duration: self
                .client_duration_seconds
                .with_label_values(&[call.service, call.method]),
            exemplars: self.client_duration_exemplars.clone(),
            call,
            started: std::time::Instant::now(),
            result: "error",
            exemplar: capture_exemplar(),
        }
    }

    /// Begins one inbound gRPC handler invocation.
    ///
    /// In addition to count and latency, the server guard owns the inflight
    /// `+1/-1` pair so early `?` returns cannot leak the gauge.
    #[must_use]
    pub fn begin_server_call(&self, call: RpcCall) -> RpcServerCallGuard {
        let inflight = self
            .server_inflight_requests
            .with_label_values(&[call.service, call.method]);
        inflight.inc();
        RpcServerCallGuard {
            inflight,
            requests: self.server_requests_total.clone(),
            duration: self
                .server_duration_seconds
                .with_label_values(&[call.service, call.method]),
            exemplars: self.server_duration_exemplars.clone(),
            call,
            started: std::time::Instant::now(),
            result: "error",
            exemplar: capture_exemplar(),
        }
    }
}

/// Outbound call lifecycle. It records one completed result and one latency
/// sample when ordinary Rust control flow drops it.
pub struct RpcClientCallGuard {
    requests: IntCounterVec,
    duration: Histogram,
    exemplars: ExemplarRecorder,
    call: RpcCall,
    started: std::time::Instant,
    result: &'static str,
    exemplar: Option<MetricExemplar>,
}

impl RpcClientCallGuard {
    pub fn success(&mut self) {
        self.result = "ok";
    }

    pub fn not_found(&mut self) {
        self.result = "not_found";
    }
}

impl Drop for RpcClientCallGuard {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_secs_f64();
        self.requests
            .with_label_values(&[self.call.service, self.call.method, self.result])
            .inc();
        self.duration.observe(elapsed);
        self.exemplars.record(
            &[self.call.service, self.call.method],
            elapsed,
            self.exemplar,
        );
    }
}

/// Inbound handler lifecycle. The pessimistic default result makes every early
/// return observable, while `Drop` always closes the inflight pair.
pub struct RpcServerCallGuard {
    inflight: IntGauge,
    requests: IntCounterVec,
    duration: Histogram,
    exemplars: ExemplarRecorder,
    call: RpcCall,
    started: std::time::Instant,
    result: &'static str,
    exemplar: Option<MetricExemplar>,
}

impl RpcServerCallGuard {
    pub fn success(&mut self) {
        self.result = "ok";
    }

    pub fn not_found(&mut self) {
        self.result = "not_found";
    }
}

impl Drop for RpcServerCallGuard {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_secs_f64();
        self.inflight.dec();
        self.requests
            .with_label_values(&[self.call.service, self.call.method, self.result])
            .inc();
        self.duration.observe(elapsed);
        self.exemplars.record(
            &[self.call.service, self.call.method],
            elapsed,
            self.exemplar,
        );
    }
}

/// Stable terminal DMS errors. Relay boundaries must not count the same error again.
#[derive(Clone)]
pub struct ErrorMetrics {
    errors_total: IntCounterVec,
}

/// Self-observability for the process-owned trace exporter.
#[derive(Clone)]
pub struct TraceRuntimeMetrics {
    export_batches_total: IntCounterVec,
    exported_spans_total: IntCounter,
    dropped_spans_total: IntCounterVec,
    export_duration_seconds: Histogram,
    export_queue_depth: IntGauge,
    last_success_timestamp_seconds: Gauge,
}

impl TraceRuntimeMetrics {
    pub fn register(registry: &Registry) -> Result<Self, MetricsError> {
        let metrics = Self {
            export_batches_total: IntCounterVec::new(
                Opts::new(
                    "dms_trace_export_batches_total",
                    "Completed OTLP export batches.",
                ),
                &["result"],
            )?,
            exported_spans_total: IntCounter::new(
                "dms_trace_exported_spans_total",
                "Spans successfully exported through OTLP.",
            )?,
            dropped_spans_total: IntCounterVec::new(
                Opts::new(
                    "dms_trace_dropped_spans_total",
                    "Spans dropped before OTLP export.",
                ),
                &["reason"],
            )?,
            export_duration_seconds: Histogram::with_opts(
                HistogramOpts::new(
                    "dms_trace_export_duration_seconds",
                    "OTLP batch export latency in seconds.",
                )
                .buckets(latency_buckets()),
            )?,
            export_queue_depth: IntGauge::new(
                "dms_trace_export_queue_depth",
                "Sampled spans waiting in the process export queue.",
            )?,
            last_success_timestamp_seconds: Gauge::new(
                "dms_trace_last_success_timestamp_seconds",
                "Unix timestamp of the most recent successful OTLP export.",
            )?,
        };
        register_collector(registry, &metrics.export_batches_total)?;
        register_collector(registry, &metrics.exported_spans_total)?;
        register_collector(registry, &metrics.dropped_spans_total)?;
        register_collector(registry, &metrics.export_duration_seconds)?;
        register_collector(registry, &metrics.export_queue_depth)?;
        register_collector(registry, &metrics.last_success_timestamp_seconds)?;
        // A CounterVec does not expose a metric family until at least one label
        // set has been materialized. Freeze the bounded reason vocabulary here
        // so a healthy process and a queue-full process publish the same
        // contract (the value remains zero until a real drop occurs).
        metrics
            .dropped_spans_total
            .with_label_values(&["queue_full"])
            .inc_by(0);
        Ok(metrics)
    }

    pub fn record_export_batch(&self, spans: usize, elapsed: std::time::Duration, success: bool) {
        self.export_batches_total
            .with_label_values(&[if success { "ok" } else { "error" }])
            .inc();
        self.export_duration_seconds.observe(elapsed.as_secs_f64());
        if success {
            self.exported_spans_total.inc_by(spans as u64);
            if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                self.last_success_timestamp_seconds.set(now.as_secs_f64());
            }
        }
    }

    pub fn record_dropped_span(&self, reason: &'static str) {
        self.dropped_spans_total.with_label_values(&[reason]).inc();
    }

    pub fn set_queue_depth(&self, depth: usize) {
        self.export_queue_depth.set(depth as i64);
    }
}

/// Component that is allowed to count a stable terminal error for the first time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorComponent {
    Client,
    Node,
    Meta,
}

impl ErrorComponent {
    const fn label(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Node => "node",
            Self::Meta => "meta",
        }
    }
}

impl ErrorMetrics {
    pub fn register(registry: &Registry) -> Result<Self, MetricsError> {
        let errors_total = IntCounterVec::new(
            Opts::new(
                "dms_errors_total",
                "Terminal DMS subsystem errors by stable code.",
            ),
            &["component", "subsystem", "error_code"],
        )?;
        register_collector(registry, &errors_total)?;
        Ok(Self { errors_total })
    }

    pub fn record(&self, error: &DmsError) {
        let raw = error.code().raw();
        let component = match raw >> 24 {
            1 => "client",
            2 => "node",
            3 => "meta",
            _ => "unknown",
        };
        let subsystem = format!("{:02x}", (raw >> 16) & 0xff);
        let code = format!("0x{raw:08x}");
        self.errors_total
            .with_label_values(&[component, &subsystem, &code])
            .inc();
    }

    /// Counts an error only where its stable component code was first created.
    /// Relay layers call this method with their own component and therefore do
    /// not double-count an unchanged Node/Meta error in the SDK registry.
    pub fn record_if_component(&self, expected_component: ErrorComponent, error: &DmsError) {
        let component = match error.code().raw() >> 24 {
            1 => "client",
            2 => "node",
            3 => "meta",
            _ => "unknown",
        };
        if component == expected_component.label() {
            self.record(error);
        }
    }
}

/// Registers a cloneable collector in an explicit process-owned registry.
///
/// Business crates use this helper so they own metric meaning and labels while
/// this common crate owns the Prometheus registration mechanics.
pub fn register_collector<C>(registry: &Registry, collector: &C) -> Result<(), MetricsError>
where
    C: prometheus::core::Collector + Clone + 'static,
{
    registry.register(Box::new(collector.clone()))?;
    Ok(())
}

/// Latency buckets shared by RPC and business-operation histograms.
#[must_use]
pub fn latency_buckets() -> Vec<f64> {
    vec![
        0.000_1, 0.000_25, 0.000_5, 0.001, 0.002_5, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
        2.5, 5.0,
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn sampled_updates_reuse_bound_series_and_binary_identity() {
        let registry = super::registry();
        let recorder = registry.exemplar_recorder("test", &["z", "a"], &[1.0]);
        recorder.record(
            &["last", "first"],
            0.1,
            Some(super::MetricExemplar::new([1; 16])),
        );
        let original_labels = {
            let store = registry.exemplars.lock().unwrap();
            let (series, _) = &store.values().next().unwrap()[0];
            assert_eq!(series.labels[0].0, "a");
            series.labels.as_ptr()
        };
        for _ in 0..1000 {
            recorder.record(
                &["last", "first"],
                0.2,
                Some(super::MetricExemplar::new([2; 16])),
            );
        }
        let store = registry.exemplars.lock().unwrap();
        assert_eq!(store.len(), 1);
        let entries = store.values().next().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0.labels.as_ptr(), original_labels);
        assert_eq!(entries[0].1.trace_id, [2; 16]);
        assert_eq!(entries[0].1.observed_value, 0.2);
    }

    #[test]
    fn registry_bundles_are_shared_across_clones_and_reconnects() {
        let registry = super::registry();
        let mut threads = Vec::new();
        for _ in 0..8 {
            let registry = registry.clone();
            threads.push(std::thread::spawn(move || {
                registry
                    .get_or_register::<super::RpcMetrics>(super::RpcMetrics::register)
                    .unwrap()
                    .begin_client_call(super::RpcCall::WORKER_GET)
                    .success();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let metrics = registry
            .get_or_register::<super::RpcMetrics>(|_| panic!("already registered"))
            .unwrap();
        assert_eq!(
            metrics
                .client_requests_total
                .with_label_values(&["WorkerService", "Get", "ok"])
                .get(),
            8
        );
        let independent = super::registry();
        let metrics = independent
            .get_or_register(super::RpcMetrics::register)
            .unwrap();
        assert_eq!(
            metrics
                .client_requests_total
                .with_label_values(&["WorkerService", "Get", "ok"])
                .get(),
            0
        );
    }

    #[test]
    fn recorder_clone_shares_immutable_buckets() {
        let registry = super::registry();
        let recorder = registry.exemplar_recorder("test", &[], &[0.1, 1.0]);
        let cloned = recorder.clone();
        assert_eq!(recorder.buckets.as_ptr(), cloned.buckets.as_ptr());
        recorder.record(&[], 0.1, None);
        assert!(registry.exemplars.lock().unwrap().is_empty());
    }
    use dms_error::{DmsError, ErrorKind, NODE_ARENA_CAPACITY_EXHAUSTED};

    use super::*;

    #[test]
    fn explicit_registry_exports_rpc_and_error_contracts() {
        let registry = registry();
        let rpc = RpcMetrics::register(&registry).expect("rpc metrics");
        let errors = ErrorMetrics::register(&registry).expect("error metrics");
        let mut client_call = rpc.begin_client_call(RpcCall::WORKER_GET);
        client_call.success();
        drop(client_call);
        let mut server_call = rpc.begin_server_call(RpcCall::WORKER_GET);
        server_call.success();
        drop(server_call);
        errors.record(&DmsError::new(
            NODE_ARENA_CAPACITY_EXHAUSTED,
            ErrorKind::ResourceExhausted,
            "full",
        ));

        let text = encode_text(&registry).expect("encode");
        assert!(text.contains("dms_rpc_client_requests_total"));
        assert!(text.contains(
            "dms_rpc_server_inflight_requests{method=\"Get\",service=\"WorkerService\"} 0"
        ));
        assert!(text.contains("dms_errors_total"));
        assert!(!text.contains("full"));
    }

    #[test]
    fn openmetrics_exposition_attaches_trace_id_without_creating_a_label_series() {
        let registry = registry();
        let histogram = HistogramVec::new(
            HistogramOpts::new("dms_test_latency_seconds", "test latency").buckets(vec![0.1, 1.0]),
            &["operation"],
        )
        .expect("histogram");
        register_collector(&registry, &histogram).expect("register");
        let recorder =
            registry.exemplar_recorder("dms_test_latency_seconds", &["operation"], &[0.1, 1.0]);
        histogram.with_label_values(&["get"]).observe(0.25);
        recorder.record(&["get"], 0.25, Some(MetricExemplar::new([0xab; 16])));

        let text = encode_text(&registry).expect("encode");
        assert!(text.contains("# {trace_id=\"abababababababababababababababab\"} 0.25"));
        assert!(!text.contains("trace_id=\"abababababababababababababababab\",operation"));
        assert!(text.ends_with("# EOF\n"));
    }

    #[test]
    fn trace_runtime_contract_exposes_drop_family_before_the_first_drop() {
        let registry = registry();
        let _metrics = TraceRuntimeMetrics::register(&registry).expect("trace runtime metrics");

        let text = encode_text(&registry).expect("encode");
        assert!(text.contains("dms_trace_dropped_spans_total{reason=\"queue_full\"} 0"));
    }
}

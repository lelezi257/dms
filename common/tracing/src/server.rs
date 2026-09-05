use std::{
    future::Future,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};

use opentelemetry::propagation::{Extractor, TextMapPropagator};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tonic::codegen::http::{HeaderMap, Request};
use tower::{Layer, Service};
use tracing::Instrument as _;

use crate::{TraceContext, set_parent};

/// Tower layer installed once around a Tonic server.
///
/// Every generated service receives the same W3C parent extraction and one
/// transport span. Business handlers remain focused on DTO/domain conversion.
#[derive(Clone, Copy, Debug, Default)]
pub struct GrpcServerTraceLayer {
    periodic_operations: bool,
}

impl GrpcServerTraceLayer {
    #[must_use]
    pub const fn new(periodic_operations: bool) -> Self {
        Self {
            periodic_operations,
        }
    }
}

impl<S> Layer<S> for GrpcServerTraceLayer {
    type Service = GrpcServerTraceService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcServerTraceService {
            inner,
            periodic_operations: self.periodic_operations,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GrpcServerTraceService<S> {
    inner: S,
    periodic_operations: bool,
}

impl<S, B> Service<Request<B>> for GrpcServerTraceService<S>
where
    S: Service<Request<B>> + Send + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, context: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        let method = request.uri().path();
        if !self.periodic_operations && is_periodic_method(method) {
            return Box::pin(self.inner.call(request));
        }
        let operation_name = grpc_operation_name(method);
        let span = tracing::info_span!(
            "dms.grpc.server",
            // `operation_name` is already a bounded `&'static str`, so the
            // OpenTelemetry layer records it through the string-value visitor.
            otel.name = operation_name,
            otel.kind = "server",
            rpc.system = "grpc",
            rpc.method = %method,
            transport.result = tracing::field::Empty,
        );
        // 无 Span 时无需解析远端父上下文。enabled 判断交给当前 Subscriber，
        // 不引入会屏蔽 SDK 宿主 Subscriber 的进程全局开关。
        if span.is_disabled() {
            return Box::pin(self.inner.call(request));
        }
        let parent = TraceContext::from_remote(
            TraceContextPropagator::new().extract(&HeaderExtractor(request.headers())),
        );
        set_parent(&span, &parent);
        let future = self.inner.call(request);
        let result_span = span.clone();
        Box::pin(
            async move {
                let result = future.await;
                // At this generic Tower boundary an HTTP/2 response can still
                // carry a business-level gRPC Status. Therefore this field is
                // deliberately named `transport.result`; handlers and typed
                // operation metrics own the business result.
                result_span.record(
                    "transport.result",
                    if result.is_ok() { "ok" } else { "error" },
                );
                result
            }
            .instrument(span),
        )
    }
}

fn is_periodic_method(method: &str) -> bool {
    matches!(
        method,
        "/dms.v1.MetadataService/Heartbeat" | "/dms.v1.WorkerService/Heartbeat"
    )
}

/// Maps generated gRPC route names to bounded, human-readable Tempo names.
/// Unknown routes retain a generic name instead of turning arbitrary URI text
/// into an unbounded tracing dimension.
fn grpc_operation_name(method: &str) -> &'static str {
    match method {
        "/dms.v1.WorkerService/OpenSession" => "dms.grpc.worker.open_session",
        "/dms.v1.WorkerService/Session" => "dms.grpc.worker.session",
        "/dms.v1.WorkerService/Heartbeat" => "dms.grpc.worker.heartbeat",
        "/dms.v1.WorkerService/AllocateStaging" => "dms.grpc.worker.allocate_staging",
        "/dms.v1.WorkerService/AcquireRegion" => "dms.grpc.worker.acquire_region",
        "/dms.v1.WorkerService/DeleteStaging" => "dms.grpc.worker.delete_staging",
        "/dms.v1.WorkerService/SetInline" => "dms.grpc.worker.set_inline",
        "/dms.v1.WorkerService/Set" => "dms.grpc.worker.set",
        "/dms.v1.WorkerService/Delete" => "dms.grpc.worker.delete",
        "/dms.v1.WorkerService/Get" => "dms.grpc.worker.get",
        "/dms.v1.WorkerService/MSet" => "dms.grpc.worker.mset",
        "/dms.v1.WorkerService/MGet" => "dms.grpc.worker.mget",
        "/dms.v1.WorkerService/SetRange" => "dms.grpc.worker.set_range",
        "/dms.v1.WorkerService/HSet" => "dms.grpc.worker.hset",
        "/dms.v1.WorkerService/HGet" => "dms.grpc.worker.hget",
        "/dms.v1.WorkerService/HMGet" => "dms.grpc.worker.hmget",
        "/dms.v1.WorkerService/HGetAll" => "dms.grpc.worker.hget_all",
        "/dms.v1.WorkerService/HDelete" => "dms.grpc.worker.hdelete",
        "/dms.v1.WorkerService/HScan" => "dms.grpc.worker.hscan",
        "/dms.v1.WorkerService/HWriteAt" => "dms.grpc.worker.hwrite_at",
        "/dms.v1.WorkerPayloadService/Upload" => "dms.grpc.payload.upload",
        "/dms.v1.WorkerPayloadService/Download" => "dms.grpc.payload.download",
        "/dms.v1.PeerService/Probe" => "dms.grpc.peer.probe",
        "/dms.v1.PeerService/PullBlock" => "dms.grpc.peer.pull_block",
        "/dms.v1.PeerService/PrepareReplica" => "dms.grpc.peer.prepare_replica",
        "/dms.v1.PeerService/ActivateReplica" => "dms.grpc.peer.activate_replica",
        "/dms.v1.PeerService/AbortReplica" => "dms.grpc.peer.abort_replica",
        "/dms.v1.PeerService/GetReplicaStatus" => "dms.grpc.peer.get_replica_status",
        "/dms.v1.MetadataService/OpenNodeSession" => "dms.grpc.meta.open_node_session",
        "/dms.v1.MetadataService/Heartbeat" => "dms.grpc.meta.heartbeat",
        "/dms.v1.MetadataService/ResolveObject" => "dms.grpc.meta.resolve_object",
        "/dms.v1.MetadataService/ReportReplicas" => "dms.grpc.meta.report_replicas",
        "/dms.v1.MetadataService/CommitVersion" => "dms.grpc.meta.commit_version",
        "/dms.v1.MetadataService/CommitBatch" => "dms.grpc.meta.commit_batch",
        "/dms.v1.MetadataService/GetOperation" => "dms.grpc.meta.get_operation",
        "/dms.v1.MetadataService/PlanReplicas" => "dms.grpc.meta.plan_replicas",
        "/dms.v1.MetadataService/WatchNodeEvents" => "dms.grpc.meta.watch_node_events",
        "/dms.v1.MetadataService/AcknowledgeNodeEvent" => "dms.grpc.meta.acknowledge_node_event",
        _ => "dms.grpc.server",
    }
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key)?.to_str().ok()
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|key| key.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_routes_have_operation_aware_names() {
        assert_eq!(
            grpc_operation_name("/dms.v1.WorkerService/Get"),
            "dms.grpc.worker.get"
        );
        assert_eq!(
            grpc_operation_name("/dms.v1.MetadataService/ResolveObject"),
            "dms.grpc.meta.resolve_object"
        );
        assert_eq!(
            grpc_operation_name("/dms.v1.WorkerPayloadService/Download"),
            "dms.grpc.payload.download"
        );
        assert_eq!(grpc_operation_name("/unknown"), "dms.grpc.server");
    }

    #[test]
    fn only_lifecycle_heartbeats_are_periodic() {
        assert!(is_periodic_method("/dms.v1.MetadataService/Heartbeat"));
        assert!(is_periodic_method("/dms.v1.WorkerService/Heartbeat"));
        assert!(!is_periodic_method("/dms.v1.WorkerService/Get"));
    }
}

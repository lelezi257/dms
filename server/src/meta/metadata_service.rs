//! Node→Meta 的 generated gRPC Service Handler。
//!
//! Handler 已覆盖 Session、heartbeat、resolve、commit、operation result、
//! Watch 与 ACK；只做 wire/domain 转换和 mailbox 投递，权威状态仍唯一属于
//! [`MetaHandle`] 后的 Meta actor。

use std::pin::Pin;

use dms_protocol::v1 as pb;
use dms_transport::dms_error_to_status;
use pb::metadata_service_server::MetadataService;
use tokio::sync::mpsc;
use tokio_stream::{Stream, wrappers::ReceiverStream};
use tonic::{Request, Response, Status};

use super::runtime::{MetaHandle, MetaRuntimeError};

#[derive(Clone)]
pub(crate) struct MetadataServiceHandler {
    // Handle 是向 Meta owner 投递命令的轻量 Sender。
    meta: MetaHandle,
    rpc_metrics: dms_metrics::RpcMetrics,
    error_metrics: dms_metrics::ErrorMetrics,
}

impl MetadataServiceHandler {
    #[cfg(test)]
    pub(crate) fn new(meta: MetaHandle) -> Self {
        let registry = dms_metrics::registry();
        let rpc_metrics = dms_metrics::RpcMetrics::register(&registry)
            .expect("test Meta RPC metrics registration");
        let error_metrics = dms_metrics::ErrorMetrics::register(&registry)
            .expect("test Meta error metrics registration");
        Self::with_metrics(meta, rpc_metrics, error_metrics)
    }

    pub(crate) fn with_metrics(
        meta: MetaHandle,
        rpc_metrics: dms_metrics::RpcMetrics,
        error_metrics: dms_metrics::ErrorMetrics,
    ) -> Self {
        Self {
            meta,
            rpc_metrics,
            error_metrics,
        }
    }

    fn map_meta_error(&self, error: MetaRuntimeError) -> Status {
        let error = error.into_dms_error();
        self.error_metrics
            .record_if_component(dms_metrics::ErrorComponent::Meta, &error);
        dms_error_to_status(error)
    }
}

#[tonic::async_trait]
impl MetadataService for MetadataServiceHandler {
    type WatchNodeEventsStream =
        Pin<Box<dyn Stream<Item = Result<pb::NodeEvent, Status>> + Send + 'static>>;

    async fn open_node_session(
        &self,
        request: Request<pb::OpenNodeSessionRequest>,
    ) -> Result<Response<pb::OpenNodeSessionResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_OPEN_NODE_SESSION);
        let request = request.into_inner();
        // proto3 message 字段可能未出现，因此先验证 registration 存在。
        let registration = request.registration.ok_or_else(|| {
            self.map_meta_error(MetaRuntimeError::InvalidArgument(
                "missing node registration".to_string(),
            ))
        })?;
        // Handler 不直接改 Session Map，只异步提交给 Meta owner。
        let grant = self
            .meta
            .open_node_session(registration.node_id, registration.control_endpoint)
            .await
            .map_err(|error| self.map_meta_error(error))?;
        // 将领域 Grant 编码为 generated response DTO。
        rpc.success();
        Ok(Response::new(pb::OpenNodeSessionResponse {
            session: Some(pb::NodeSessionIdentity {
                session_id: grant.session_id,
                node_id: grant.node_id,
                node_epoch: grant.node_epoch,
            }),
            heartbeat_interval_millis: grant.heartbeat_interval_millis,
            lease_ttl_millis: grant.lease_ttl_millis,
        }))
    }

    async fn heartbeat(
        &self,
        request: Request<pb::NodeHeartbeatRequest>,
    ) -> Result<Response<pb::NodeHeartbeatResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_HEARTBEAT);
        let request = request.into_inner();
        // session identity 是 heartbeat 的 fencing 依据，缺失时立即拒绝。
        let session = request.session.ok_or_else(|| {
            self.map_meta_error(MetaRuntimeError::InvalidArgument(
                "missing node session".to_string(),
            ))
        })?;
        let grant = self
            .meta
            .heartbeat(
                session.session_id,
                session.node_id,
                session.node_epoch,
                request.event_cursor,
            )
            .await
            .map_err(|error| self.map_meta_error(error))?;
        rpc.success();
        Ok(Response::new(pb::NodeHeartbeatResponse {
            lease_ttl_millis: grant.lease_ttl_millis,
            accepted_node_epoch: grant.accepted_node_epoch,
            event_high_watermark: grant.event_high_watermark,
        }))
    }

    async fn resolve_object(
        &self,
        request: Request<pb::ResolveObjectRequest>,
    ) -> Result<Response<pb::ResolveObjectResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_RESOLVE_OBJECT);
        let response = self
            .meta
            .resolve_object(request.into_inner())
            .await
            .map_err(|error| self.map_meta_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn report_replicas(
        &self,
        request: Request<pb::ReportReplicasRequest>,
    ) -> Result<Response<pb::ReportReplicasResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_REPORT_REPLICAS);
        let response = self
            .meta
            .report_replicas(request.into_inner())
            .await
            .map_err(|error| self.map_meta_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn commit_version(
        &self,
        request: Request<pb::CommitVersionRequest>,
    ) -> Result<Response<pb::CommitVersionResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_COMMIT_VERSION);
        let response = self
            .meta
            .commit_version(request.into_inner())
            .await
            .map_err(|error| self.map_meta_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn commit_batch(
        &self,
        request: Request<pb::CommitBatchRequest>,
    ) -> Result<Response<pb::CommitBatchResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_COMMIT_BATCH);
        let response = self
            .meta
            .commit_batch(request.into_inner())
            .await
            .map_err(|error| self.map_meta_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn get_operation(
        &self,
        request: Request<pb::GetOperationRequest>,
    ) -> Result<Response<pb::GetOperationResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_GET_OPERATION);
        let response = self
            .meta
            .get_operation(request.into_inner())
            .await
            .map_err(|error| self.map_meta_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn plan_replicas(
        &self,
        request: Request<pb::PlanReplicasRequest>,
    ) -> Result<Response<pb::PlanReplicasResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_PLAN_REPLICAS);
        let response = self
            .meta
            .plan_replicas(request.into_inner())
            .await
            .map_err(|error| self.map_meta_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn watch_node_events(
        &self,
        request: Request<pb::WatchNodeEventsRequest>,
    ) -> Result<Response<Self::WatchNodeEventsStream>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_WATCH_NODE_EVENTS);
        let (event_sender, mut event_receiver) = mpsc::channel(128);
        self.meta
            .watch_node_events(request.into_inner(), event_sender)
            .await
            .map_err(|error| self.map_meta_error(error))?;
        let (wire_sender, wire_receiver) = mpsc::channel(128);
        tokio::spawn(async move {
            while let Some(event) = event_receiver.recv().await {
                if wire_sender.send(Ok(event)).await.is_err() {
                    break;
                }
            }
        });
        rpc.success();
        Ok(Response::new(Box::pin(ReceiverStream::new(wire_receiver))))
    }

    async fn acknowledge_node_event(
        &self,
        request: Request<pb::AcknowledgeNodeEventRequest>,
    ) -> Result<Response<pb::AcknowledgeNodeEventResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::META_ACKNOWLEDGE_NODE_EVENT);
        self.meta
            .acknowledge_node_event(request.into_inner())
            .await
            .map_err(|error| self.map_meta_error(error))?;
        rpc.success();
        Ok(Response::new(pb::AcknowledgeNodeEventResponse {}))
    }
}

#[cfg(test)]
mod tests {
    use dms_protocol::v1::{
        NodeHeartbeatRequest, NodeRegistration, OpenNodeSessionRequest, RequestContext,
        ResourceSummary, metadata_service_client::MetadataServiceClient,
        metadata_service_server::MetadataServiceServer,
    };
    use dms_transport::{GrpcConfig, SecurityManager, TlsConfig};
    use tokio::{net::TcpListener, sync::oneshot};
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Endpoint;

    use super::MetadataServiceHandler;
    use crate::meta::runtime::MetaHandle;

    #[tokio::test]
    async fn node_to_meta_session_crosses_grpc_mailbox_and_oneshot() {
        // 这个测试覆盖真实 TCP：generated Client→Tonic→Handler→mailbox→MetaState→oneshot。
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind meta");
        let address = listener.local_addr().expect("meta address");
        let handler = MetadataServiceHandler::new(MetaHandle::spawn());
        let security = SecurityManager::new(TlsConfig::Disabled).expect("security");
        let server = GrpcConfig::default().configure_server(tonic::transport::Server::builder());
        let mut server = security.configure_server(server).expect("server security");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        // Server 在后台 Task 运行，shutdown_rx 用于测试收尾。
        let server_task = tokio::spawn(async move {
            server
                .add_service(MetadataServiceServer::new(handler))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("meta server");
        });

        let endpoint = Endpoint::from_shared(format!("http://{address}")).expect("meta endpoint");
        let endpoint = GrpcConfig::default().configure_client(endpoint);
        let endpoint = security
            .configure_client(endpoint)
            .expect("client security");
        let channel = endpoint.connect().await.expect("connect meta");
        let mut client = MetadataServiceClient::new(channel);
        // context 证明公共请求字段也能跨 wire；R1 owner 尚未消费它。
        let context = RequestContext {
            node_id: 7,
            principal: "node-7".to_string(),
            timeout_millis: 1_000,
        };
        // 第一次 RPC 创建 Node Session，并得到 epoch。
        let opened = client
            .open_node_session(OpenNodeSessionRequest {
                context: Some(context.clone()),
                registration: Some(NodeRegistration {
                    node_id: 7,
                    control_endpoint: "http://127.0.0.1:19207".to_string(),
                    transport_capabilities: vec!["grpc".to_string()],
                    total_host_memory_bytes: 1024,
                    failure_domain: "test".to_string(),
                }),
            })
            .await
            .expect("open node session")
            .into_inner();
        let session = opened.session.expect("session identity");
        // 第二次 RPC 带回同一 session/epoch，验证 Meta 能识别当前 Node incarnation。
        let heartbeat = client
            .heartbeat(NodeHeartbeatRequest {
                context: Some(context),
                session: Some(session.clone()),
                resources: Some(ResourceSummary {
                    total_host_memory_bytes: 1024,
                    available_host_memory_bytes: 768,
                    staged_bytes: 0,
                    replica_bytes: 256,
                }),
                catalog_watermark: 0,
                event_cursor: 3,
            })
            .await
            .expect("heartbeat")
            .into_inner();

        assert_eq!(session.node_id, 7);
        assert_eq!(heartbeat.accepted_node_epoch, session.node_epoch);
        // Node 上报 cursor=3 不能伪造 Meta 事件；当前尚无提交，因此权威高水位仍是 0。
        assert_eq!(heartbeat.event_high_watermark, 0);
        let _ = shutdown_tx.send(());
        server_task.await.expect("join meta server");
    }
}

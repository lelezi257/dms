//! Node→Node 的 generated gRPC Service Handler。
//!
//! 本层只负责 protobuf DTO、gRPC Status 与领域参数之间的转换。Peer 请求与
//! Client 请求共享同一个 [`NodeHandle`]、mailbox 和 `NodeState`，因此不会形成
//! 第二份 Arena/Object/Replica 状态。

use dms_protocol::v1 as pb;
use dms_transport::dms_error_to_status;
use pb::peer_service_server::PeerService;
use tonic::{Request, Response, Status};

use super::metrics::{NodeMetrics, ReplicaDirection, ReplicaOperation};
use super::runtime::{NodeHandle, ReplicaPrepareSpec, ReplicaStateView, WorkerError};

/// generated `PeerService` 的 Node 侧实现。
#[derive(Clone)]
pub(crate) struct PeerServiceHandler {
    // clone 只增加同一个 Node mailbox 的 Sender，不复制 NodeState。
    node: NodeHandle,
    metrics: NodeMetrics,
    rpc_metrics: dms_metrics::RpcMetrics,
}

impl PeerServiceHandler {
    #[cfg(test)]
    pub(crate) fn new(node: NodeHandle) -> Self {
        let registry = dms_metrics::registry();
        let rpc_metrics = dms_metrics::RpcMetrics::register(&registry)
            .expect("test Peer RPC metrics registration");
        Self::with_metrics(node, rpc_metrics)
    }

    pub(crate) fn with_metrics(node: NodeHandle, rpc_metrics: dms_metrics::RpcMetrics) -> Self {
        let metrics = node.metrics();
        Self {
            node,
            metrics,
            rpc_metrics,
        }
    }
}

#[tonic::async_trait]
impl PeerService for PeerServiceHandler {
    async fn probe(
        &self,
        request: Request<pb::PeerProbeRequest>,
    ) -> Result<Response<pb::PeerProbeResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::PEER_PROBE);
        let mut metric = self
            .metrics
            .begin_replica_operation(ReplicaOperation::Probe);
        // 消费 Tonic wrapper，取得 generated DTO 的所有权。
        let request = request.into_inner();
        // Probe 与 Client set/get 一样进入唯一 Node owner；Handler 不持有业务状态。
        let result = self
            .node
            .probe(request.source_node_id, request.nonce)
            .await
            .map_err(map_node_error)?;
        metric.success();
        rpc.success();
        Ok(Response::new(pb::PeerProbeResponse {
            serving_node_id: result.serving_node_id,
            nonce: result.nonce,
        }))
    }

    async fn pull_block(
        &self,
        request: Request<pb::PeerPullBlockRequest>,
    ) -> Result<Response<pb::PeerPullBlockResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::PEER_PULL_BLOCK);
        let mut metric = self.metrics.begin_replica_operation(ReplicaOperation::Pull);
        let request = request.into_inner();
        let range = match (request.offset, request.length) {
            (Some(offset), Some(length)) => Some((offset, length)),
            (None, None) => None,
            _ => {
                return Err(node_invalid_argument(
                    "offset and length must appear together",
                ));
            }
        };
        let result = self
            .node
            .pull_block(request.source_node_id, request.block_id, range)
            .await
            .map_err(map_node_error)?;
        if request
            .expected_length
            .is_some_and(|expected| expected != result.length)
            || (!request.expected_checksum.is_empty()
                && request.expected_checksum != result.checksum)
        {
            self.metrics.record_replica_checksum_failure();
            return Err(dms_error_to_status(dms_error::DmsError::new(
                dms_error::NODE_TRANSFER_CORRUPT_DATA,
                dms_error::ErrorKind::DataLoss,
                "peer block does not match expected length/checksum",
            )));
        }
        metric.success_with_payload(ReplicaDirection::Send, result.payload.len());
        rpc.success();
        Ok(Response::new(pb::PeerPullBlockResponse {
            serving_node_id: result.serving_node_id,
            block_id: result.block_id,
            payload: result.payload,
            checksum: result.checksum,
            length: result.length,
        }))
    }

    async fn prepare_replica(
        &self,
        request: Request<pb::PeerPrepareReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::PEER_PREPARE_REPLICA);
        let mut metric = self
            .metrics
            .begin_replica_operation(ReplicaOperation::Prepare);
        let request = request.into_inner();
        let result = self
            .node
            .prepare_replica(ReplicaPrepareSpec {
                source_node_id: request.source_node_id,
                source_endpoint: request.source_endpoint,
                plan_id: request.plan_id,
                block_id: request.block_id,
                expected_length: request.expected_length,
                expected_checksum: request.expected_checksum,
            })
            .await
            .map_err(map_node_error)?;
        metric.success();
        rpc.success();
        Ok(Response::new(encode_replica_status(result)))
    }

    async fn activate_replica(
        &self,
        request: Request<pb::PeerActivateReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::PEER_ACTIVATE_REPLICA);
        let mut metric = self
            .metrics
            .begin_replica_operation(ReplicaOperation::Activate);
        let result = self
            .node
            .activate_replica(request.into_inner().plan_id)
            .await
            .map_err(map_node_error)?;
        metric.success();
        rpc.success();
        Ok(Response::new(encode_replica_status(result)))
    }

    async fn abort_replica(
        &self,
        request: Request<pb::PeerAbortReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::PEER_ABORT_REPLICA);
        let mut metric = self
            .metrics
            .begin_replica_operation(ReplicaOperation::Abort);
        let result = self
            .node
            .abort_replica(request.into_inner().plan_id)
            .await
            .map_err(map_node_error)?;
        metric.success();
        rpc.success();
        Ok(Response::new(encode_replica_status(result)))
    }

    async fn get_replica_status(
        &self,
        request: Request<pb::PeerReplicaStatusRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::PEER_GET_REPLICA_STATUS);
        let mut metric = self
            .metrics
            .begin_replica_operation(ReplicaOperation::Status);
        let result = self
            .node
            .replica_status(request.into_inner().plan_id)
            .await
            .map_err(map_node_error)?;
        metric.success();
        rpc.success();
        Ok(Response::new(encode_replica_status(result)))
    }
}

fn encode_replica_status(status: ReplicaStateView) -> pb::PeerReplicaStatusResponse {
    pb::PeerReplicaStatusResponse {
        plan_id: status.plan_id,
        block_id: status.block_id,
        status: status.status.to_string(),
        length: status.length,
        checksum: status.checksum,
    }
}

fn map_node_error(error: WorkerError) -> Status {
    dms_error_to_status(super::runtime::worker_error_to_dms(error))
}

fn node_invalid_argument(message: impl Into<String>) -> Status {
    dms_error_to_status(dms_error::DmsError::new(
        dms_error::NODE_WORKER_INVALID_REQUEST,
        dms_error::ErrorKind::InvalidArgument,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use dms_protocol::v1::{
        PeerActivateReplicaRequest, PeerPrepareReplicaRequest, PeerProbeRequest,
        PeerPullBlockRequest, metadata_service_server::MetadataServiceServer,
        peer_service_client::PeerServiceClient, peer_service_server::PeerServiceServer,
    };
    use dms_transport::{GrpcConfig, SecurityManager, TlsConfig};
    use tokio::{net::TcpListener, sync::oneshot};
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Endpoint;

    use super::PeerServiceHandler;
    use crate::meta::{metadata_service::MetadataServiceHandler, runtime::MetaHandle};
    use crate::node::metadata_client::MetadataClient;
    use crate::node::runtime::NodeHandle;

    #[tokio::test]
    async fn node_to_node_probe_crosses_grpc_and_shared_node_owner() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind peer");
        let address = listener.local_addr().expect("peer address");
        let node = NodeHandle::spawn_without_metadata("node-b".to_string());
        let peer = PeerServiceHandler::new(node);
        let security = SecurityManager::new(TlsConfig::Disabled).expect("security");
        let server = GrpcConfig::default().configure_server(tonic::transport::Server::builder());
        let mut server = security.configure_server(server).expect("server security");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            server
                .add_service(PeerServiceServer::new(peer))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("peer server");
        });

        let endpoint = Endpoint::from_shared(format!("http://{address}")).expect("peer endpoint");
        let endpoint = GrpcConfig::default().configure_client(endpoint);
        let endpoint = security
            .configure_client(endpoint)
            .expect("client security");
        let mut client = PeerServiceClient::connect(endpoint)
            .await
            .expect("connect peer");
        let response = client
            .probe(PeerProbeRequest {
                source_node_id: "node-a".to_string(),
                nonce: b"probe-1".to_vec(),
            })
            .await
            .expect("probe")
            .into_inner();

        assert_eq!(response.serving_node_id, "node-b");
        assert_eq!(response.nonce, b"probe-1");
        let _ = shutdown_tx.send(());
        server_task.await.expect("join peer server");
    }

    #[tokio::test]
    async fn node_to_node_pull_block_reads_shared_arena() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind peer");
        let address = listener.local_addr().expect("peer address");
        let node = NodeHandle::spawn_without_metadata("node-b".to_string());
        let session_id = node.open_session(false).await.expect("session");
        let allocation = node
            .allocate_staging(session_id, 10)
            .await
            .expect("allocate");
        let receipt = node
            .upload(allocation.transfer_id, b"peer-bytes".to_vec())
            .await
            .expect("upload");
        let block_id = b"block-from-peer".to_vec();
        node.debug_commit_for_peer_test(
            session_id,
            allocation.staging_id,
            receipt,
            block_id.clone(),
        )
        .await
        .expect("debug commit");

        let peer = PeerServiceHandler::new(node);
        let security = SecurityManager::new(TlsConfig::Disabled).expect("security");
        let server = GrpcConfig::default().configure_server(tonic::transport::Server::builder());
        let mut server = security.configure_server(server).expect("server security");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            server
                .add_service(PeerServiceServer::new(peer))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("peer server");
        });

        let endpoint = Endpoint::from_shared(format!("http://{address}")).expect("peer endpoint");
        let endpoint = GrpcConfig::default().configure_client(endpoint);
        let endpoint = security
            .configure_client(endpoint)
            .expect("client security");
        let mut client = PeerServiceClient::connect(endpoint)
            .await
            .expect("connect peer");
        let response = client
            .pull_block(PeerPullBlockRequest {
                source_node_id: "node-a".to_string(),
                block_id,
                offset: None,
                length: None,
                expected_length: Some(10),
                expected_checksum: Vec::new(),
            })
            .await
            .expect("pull")
            .into_inner();

        assert_eq!(response.serving_node_id, "node-b");
        assert_eq!(response.payload, b"peer-bytes");
        let _ = shutdown_tx.send(());
        server_task.await.expect("join peer server");
    }

    #[tokio::test]
    async fn reader_node_pulls_missing_block_from_writer_node() {
        let meta_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind meta");
        let meta_endpoint = format!("http://{}", meta_listener.local_addr().expect("meta addr"));
        let meta_handler = MetadataServiceHandler::new(MetaHandle::spawn());
        let meta_task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(MetadataServiceServer::new(meta_handler))
                .serve_with_incoming(TcpListenerStream::new(meta_listener))
                .await
                .expect("meta server");
        });

        let peer_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind writer peer");
        let writer_endpoint = format!("http://{}", peer_listener.local_addr().expect("peer addr"));
        let writer_metadata =
            MetadataClient::connect(&meta_endpoint, 1, writer_endpoint.clone(), None)
                .await
                .expect("writer meta");
        let writer = NodeHandle::spawn(
            "writer-node".to_string(),
            writer_metadata,
            256 * 1024 * 1024,
            std::time::Duration::from_secs(30),
            None,
        );
        let peer = PeerServiceHandler::new(writer.clone());
        let peer_task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(PeerServiceServer::new(peer))
                .serve_with_incoming(TcpListenerStream::new(peer_listener))
                .await
                .expect("peer server");
        });

        let writer_session = writer.open_session(false).await.expect("writer session");
        writer
            .set_inline(
                writer_session,
                b"shared/key".to_vec(),
                b"from-writer-node".to_vec(),
                [b"client-uuid-0001".as_slice(), &1_u64.to_be_bytes()].concat(),
                "any".to_string(),
            )
            .await
            .expect("writer set");

        let reader_metadata =
            MetadataClient::connect(&meta_endpoint, 2, "http://127.0.0.1:0".to_string(), None)
                .await
                .expect("reader meta");
        let reader = NodeHandle::spawn(
            "reader-node".to_string(),
            reader_metadata,
            256 * 1024 * 1024,
            std::time::Duration::from_secs(30),
            None,
        );
        let reader_session = reader.open_session(false).await.expect("reader session");
        let ticket = reader
            .get(reader_session, b"shared/key".to_vec(), None, None)
            .await
            .expect("reader get");
        let segment = ticket.segments.into_iter().next().expect("one segment");
        let payload = match segment.target {
            crate::node::runtime::ReadTarget::Grpc { transfer_id } => {
                reader.download(transfer_id).await.expect("download")
            }
            crate::node::runtime::ReadTarget::Shm(_) => {
                panic!("peer test should use gRPC read target")
            }
        };

        assert_eq!(payload, b"from-writer-node");
        meta_task.abort();
        peer_task.abort();
    }

    #[tokio::test]
    async fn prepared_replica_is_not_readable_until_activation() {
        let source_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind source");
        let source_endpoint = format!(
            "http://{}",
            source_listener.local_addr().expect("source addr")
        );
        let source = NodeHandle::spawn_without_metadata("source-node".to_string());
        let source_session = source.open_session(false).await.expect("source session");
        let source_allocation = source
            .allocate_staging(source_session, 12)
            .await
            .expect("source allocate");
        let source_receipt = source
            .upload(source_allocation.transfer_id, b"replica-data".to_vec())
            .await
            .expect("source upload");
        let block_id = b"replica-block".to_vec();
        source
            .debug_commit_for_peer_test(
                source_session,
                source_allocation.staging_id,
                source_receipt.clone(),
                block_id.clone(),
            )
            .await
            .expect("source commit");
        let source_peer = PeerServiceHandler::new(source);
        let source_task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(PeerServiceServer::new(source_peer))
                .serve_with_incoming(TcpListenerStream::new(source_listener))
                .await
                .expect("source peer server");
        });

        let target_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind target");
        let target_endpoint = format!(
            "http://{}",
            target_listener.local_addr().expect("target addr")
        );
        let target = NodeHandle::spawn_without_metadata("target-node".to_string());
        let target_peer = PeerServiceHandler::new(target.clone());
        let target_task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(PeerServiceServer::new(target_peer))
                .serve_with_incoming(TcpListenerStream::new(target_listener))
                .await
                .expect("target peer server");
        });

        let mut client = PeerServiceClient::connect(target_endpoint)
            .await
            .expect("connect target");
        let plan_id = b"plan-1".to_vec();
        let prepared = client
            .prepare_replica(PeerPrepareReplicaRequest {
                source_node_id: "target-node".to_string(),
                source_endpoint,
                plan_id: plan_id.clone(),
                block_id: block_id.clone(),
                expected_length: 12,
                expected_checksum: source_receipt.digest.clone(),
            })
            .await
            .expect("prepare")
            .into_inner();
        assert_eq!(prepared.status, "prepared");
        assert!(matches!(
            target
                .pull_block("source-node".to_string(), block_id.clone(), None)
                .await,
            Err(crate::node::runtime::WorkerError::NotFound)
        ));

        let activated = client
            .activate_replica(PeerActivateReplicaRequest { plan_id })
            .await
            .expect("activate")
            .into_inner();
        assert_eq!(activated.status, "active");
        let pulled = target
            .pull_block("source-node".to_string(), block_id, None)
            .await
            .expect("pull activated");
        assert_eq!(pulled.payload, b"replica-data");

        source_task.abort();
        target_task.abort();
    }
}

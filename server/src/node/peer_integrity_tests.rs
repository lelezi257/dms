use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use dms_protocol::v1 as pb;
use pb::{
    metadata_service_server::{MetadataService, MetadataServiceServer},
    peer_service_server::{PeerService, PeerServiceServer},
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tokio_stream::{Stream, wrappers::TcpListenerStream};
use tonic::{Request, Response, Status};

use super::*;
use crate::meta::{metadata_service::MetadataServiceHandler, runtime::MetaHandle};

#[test]
fn prepare_replica_rejects_corrupt_bytes_with_valid_checksum_without_prepared_state() {
    let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
    let plan_id = b"corrupt-prepare-plan".to_vec();
    let block_id = b"corrupt-prepare-block".to_vec();
    let bytes = b"replica-owner-verified".to_vec();
    let checksum = digest(&bytes);
    let mut corrupt = bytes.clone();
    corrupt[0] ^= 0x01;

    let result = state.prepare_replica(
        plan_id.clone(),
        block_id.clone(),
        corrupt,
        checksum,
        bytes.len() as u64,
    );

    assert!(matches!(result, Err(WorkerError::Conflict)));
    assert!(!state.prepared_replicas.contains_key(&plan_id));
    assert!(state.arena.read_bytes(&block_id).is_none());
}

#[test]
fn prepare_replica_preserves_empty_checksum_compatibility_for_owner_import() {
    let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
    let plan_id = b"empty-checksum-plan".to_vec();
    let block_id = b"empty-checksum-block".to_vec();
    let bytes = b"legacy-peer-bytes".to_vec();

    let prepared = state
        .prepare_replica(
            plan_id.clone(),
            block_id.clone(),
            bytes.clone(),
            Vec::new(),
            bytes.len() as u64,
        )
        .expect("empty checksum remains legacy-compatible");

    assert_eq!(prepared.status, "prepared");
    assert_eq!(prepared.checksum, Vec::<u8>::new());
    assert!(matches!(
        state.prepared_replicas.get(&plan_id),
        Some(ReplicaTransferState::Prepared(replica))
            if replica.block_id == block_id && replica.bytes == bytes
    ));
}

#[tokio::test]
async fn import_rejects_corrupt_payload_when_peer_echoes_expected_checksum() {
    let meta = CountingMetaServer::start().await;
    let original = b"authoritative peer bytes".to_vec();
    let expected_length = original.len() as u64;
    let checksum = digest(&original);
    let block_id = b"corrupt-small-block".to_vec();
    let peer = FakePeer::new(block_id.clone(), original)
        .corrupt_payload()
        .override_checksum(checksum.clone());
    let peer = FakePeerServer::start(peer).await;
    let registry = dms_metrics::registry();
    let node = spawn_node_with_registry(&meta.endpoint, "import-target", 41, &registry).await;

    let result = node
        .import_and_report_peer_block(
            node.metadata.as_ref().expect("node metadata"),
            b"integrity-test/",
            PeerPullSpec {
                endpoint: peer.endpoint.clone(),
                block_id: block_id.clone(),
                expected_checksum: checksum,
                expected_length,
            },
        )
        .await;

    assert!(matches!(result, Err(WorkerError::Conflict)));
    assert!(matches!(
        node.pull_block("verifier".into(), block_id, None).await,
        Err(WorkerError::NotFound)
    ));
    assert_eq!(meta.report_count(), 0);
    let metrics = dms_metrics::encode_text(&registry).expect("node metrics");
    assert_eq!(
        counter_value(
            &metrics,
            "dms_node_replica_operations_total",
            &[("operation", "pull"), ("result", "ok")]
        ),
        0.0
    );
    assert_eq!(
        counter_value(
            &metrics,
            "dms_node_replica_operations_total",
            &[("operation", "pull"), ("result", "error")]
        ),
        1.0
    );
    assert_eq!(
        counter_value(
            &metrics,
            "dms_node_replica_bytes_total",
            &[("direction", "receive")]
        ),
        0.0
    );
    assert_eq!(
        counter_value(
            &metrics,
            "dms_node_replica_checksum_failures_total",
            &[("provider", "grpc")]
        ),
        1.0
    );

    peer.stop().await;
    meta.stop().await;
}

#[tokio::test]
async fn import_rejects_corrupt_payload_when_only_peer_checksum_is_present() {
    let meta = CountingMetaServer::start().await;
    let original = b"peer supplied checksum bytes".to_vec();
    let expected_length = original.len() as u64;
    let checksum = digest(&original);
    let block_id = b"peer-checksum-block".to_vec();
    let peer = FakePeer::new(block_id.clone(), original)
        .corrupt_payload()
        .override_checksum(checksum);
    let peer = FakePeerServer::start(peer).await;
    let registry = dms_metrics::registry();
    let node = spawn_node_with_registry(
        &meta.endpoint,
        "import-target-empty-expected",
        42,
        &registry,
    )
    .await;

    let result = node
        .import_and_report_peer_block(
            node.metadata.as_ref().expect("node metadata"),
            b"integrity-test/",
            PeerPullSpec {
                endpoint: peer.endpoint.clone(),
                block_id: block_id.clone(),
                expected_checksum: Vec::new(),
                expected_length,
            },
        )
        .await;

    assert!(matches!(result, Err(WorkerError::Conflict)));
    assert!(matches!(
        node.pull_block("verifier".into(), block_id, None).await,
        Err(WorkerError::NotFound)
    ));
    assert_eq!(meta.report_count(), 0);
    let metrics = dms_metrics::encode_text(&registry).expect("node metrics");
    assert_eq!(
        counter_value(
            &metrics,
            "dms_node_replica_checksum_failures_total",
            &[("provider", "grpc")]
        ),
        1.0
    );

    peer.stop().await;
    meta.stop().await;
}

#[tokio::test]
async fn import_and_report_peer_block_preserves_double_empty_checksum_compatibility() {
    let meta = CountingMetaServer::start().await;
    let original = b"legacy unchecked peer payload".to_vec();
    let block_id = b"double-empty-checksum-block".to_vec();
    let peer = FakePeer::new(block_id.clone(), original.clone())
        .corrupt_payload()
        .override_checksum(Vec::new());
    let peer = FakePeerServer::start(peer).await;
    let registry = dms_metrics::registry();
    let node =
        spawn_node_with_registry(&meta.endpoint, "import-target-double-empty", 43, &registry).await;

    node.import_and_report_peer_block(
        node.metadata.as_ref().expect("node metadata"),
        b"integrity-test/",
        PeerPullSpec {
            endpoint: peer.endpoint.clone(),
            block_id: block_id.clone(),
            expected_checksum: Vec::new(),
            expected_length: original.len() as u64,
        },
    )
    .await
    .expect("double-empty checksum keeps the pre-existing compatibility path");

    assert_eq!(meta.report_count(), 1);

    peer.stop().await;
    meta.stop().await;
}

#[tokio::test]
async fn prepare_replica_reassembles_out_of_order_segments_and_validates_tail() {
    let payload = deterministic_bytes((PEER_PULL_SEGMENT_BYTES * 2 + 17) as usize);
    let checksum = digest(&payload);
    let block_id = b"segmented-tail-block".to_vec();
    let peer = FakePeer::new(block_id.clone(), payload.clone()).delay_first_segment();
    let peer = FakePeerServer::start(peer).await;
    let target = NodeHandle::spawn_without_metadata("segmented-target".to_string());
    let plan_id = b"segmented-tail-plan".to_vec();

    let prepared = target
        .prepare_replica(ReplicaPrepareSpec {
            source_node_id: "segmented-source".to_string(),
            source_endpoint: peer.endpoint.clone(),
            plan_id: plan_id.clone(),
            block_id: block_id.clone(),
            expected_length: payload.len() as u64,
            expected_checksum: checksum,
        })
        .await
        .expect("prepare segmented replica");

    assert_eq!(prepared.status, "prepared");
    assert_eq!(
        peer.requested_segments(),
        vec![
            (0, PEER_PULL_SEGMENT_BYTES),
            (PEER_PULL_SEGMENT_BYTES, PEER_PULL_SEGMENT_BYTES),
            (PEER_PULL_SEGMENT_BYTES * 2, 17),
        ]
    );
    assert!(matches!(
        target
            .pull_block("verifier".into(), block_id.clone(), None)
            .await,
        Err(WorkerError::NotFound)
    ));
    target
        .activate_replica(plan_id)
        .await
        .expect("activate prepared segmented replica");
    let pulled = target
        .pull_block("verifier".into(), block_id, None)
        .await
        .expect("pull activated segmented replica");
    assert_eq!(pulled.payload, payload);

    peer.stop().await;
}

struct CountingMetaServer {
    endpoint: String,
    report_count: Arc<AtomicUsize>,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl CountingMetaServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind counting Meta");
        let endpoint = format!("http://{}", listener.local_addr().expect("Meta address"));
        let report_count = Arc::new(AtomicUsize::new(0));
        let service = CountingMetaService {
            inner: MetadataServiceHandler::new(MetaHandle::spawn()),
            report_count: report_count.clone(),
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(MetadataServiceServer::new(service))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve counting Meta");
        });
        Self {
            endpoint,
            report_count,
            shutdown: Some(shutdown_tx),
            task,
        }
    }

    fn report_count(&self) -> usize {
        self.report_count.load(Ordering::SeqCst)
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
        let _ = self.task.await;
    }
}

#[derive(Clone)]
struct CountingMetaService {
    inner: MetadataServiceHandler,
    report_count: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl MetadataService for CountingMetaService {
    type WatchNodeEventsStream = Pin<Box<dyn Stream<Item = Result<pb::NodeEvent, Status>> + Send>>;

    async fn open_node_session(
        &self,
        request: Request<pb::OpenNodeSessionRequest>,
    ) -> Result<Response<pb::OpenNodeSessionResponse>, Status> {
        self.inner.open_node_session(request).await
    }

    async fn heartbeat(
        &self,
        request: Request<pb::NodeHeartbeatRequest>,
    ) -> Result<Response<pb::NodeHeartbeatResponse>, Status> {
        self.inner.heartbeat(request).await
    }

    async fn resolve_object(
        &self,
        request: Request<pb::ResolveObjectRequest>,
    ) -> Result<Response<pb::ResolveObjectResponse>, Status> {
        self.inner.resolve_object(request).await
    }

    async fn report_replicas(
        &self,
        request: Request<pb::ReportReplicasRequest>,
    ) -> Result<Response<pb::ReportReplicasResponse>, Status> {
        self.report_count.fetch_add(1, Ordering::SeqCst);
        self.inner.report_replicas(request).await
    }

    async fn commit_version(
        &self,
        request: Request<pb::CommitVersionRequest>,
    ) -> Result<Response<pb::CommitVersionResponse>, Status> {
        self.inner.commit_version(request).await
    }

    async fn commit_batch(
        &self,
        request: Request<pb::CommitBatchRequest>,
    ) -> Result<Response<pb::CommitBatchResponse>, Status> {
        self.inner.commit_batch(request).await
    }

    async fn get_operation(
        &self,
        request: Request<pb::GetOperationRequest>,
    ) -> Result<Response<pb::GetOperationResponse>, Status> {
        self.inner.get_operation(request).await
    }

    async fn plan_replicas(
        &self,
        request: Request<pb::PlanReplicasRequest>,
    ) -> Result<Response<pb::PlanReplicasResponse>, Status> {
        self.inner.plan_replicas(request).await
    }

    async fn acknowledge_node_event(
        &self,
        request: Request<pb::AcknowledgeNodeEventRequest>,
    ) -> Result<Response<pb::AcknowledgeNodeEventResponse>, Status> {
        self.inner.acknowledge_node_event(request).await
    }

    async fn watch_node_events(
        &self,
        request: Request<pb::WatchNodeEventsRequest>,
    ) -> Result<Response<Self::WatchNodeEventsStream>, Status> {
        self.inner.watch_node_events(request).await
    }
}

#[derive(Clone)]
struct FakePeer {
    block_id: Vec<u8>,
    payload: Arc<Vec<u8>>,
    checksum_override: Arc<Mutex<Option<Vec<u8>>>>,
    corrupt_payload: bool,
    delay_first_segment: bool,
    requests: Arc<Mutex<BTreeMap<u64, u64>>>,
}

impl FakePeer {
    fn new(block_id: Vec<u8>, payload: Vec<u8>) -> Self {
        Self {
            block_id,
            payload: Arc::new(payload),
            checksum_override: Arc::new(Mutex::new(None)),
            corrupt_payload: false,
            delay_first_segment: false,
            requests: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn corrupt_payload(mut self) -> Self {
        self.corrupt_payload = true;
        self
    }

    fn override_checksum(self, checksum: Vec<u8>) -> Self {
        *self
            .checksum_override
            .lock()
            .expect("checksum override lock") = Some(checksum);
        self
    }

    fn delay_first_segment(mut self) -> Self {
        self.delay_first_segment = true;
        self
    }

    fn requested_segments(&self) -> Vec<(u64, u64)> {
        self.requests
            .lock()
            .expect("requests lock")
            .iter()
            .map(|(offset, length)| (*offset, *length))
            .collect()
    }
}

#[tonic::async_trait]
impl PeerService for FakePeer {
    async fn probe(
        &self,
        request: Request<pb::PeerProbeRequest>,
    ) -> Result<Response<pb::PeerProbeResponse>, Status> {
        Ok(Response::new(pb::PeerProbeResponse {
            serving_node_id: request.into_inner().source_node_id,
            nonce: b"fake-peer".to_vec(),
        }))
    }

    async fn pull_block(
        &self,
        request: Request<pb::PeerPullBlockRequest>,
    ) -> Result<Response<pb::PeerPullBlockResponse>, Status> {
        let request = request.into_inner();
        if request.block_id != self.block_id {
            return Err(Status::not_found("unknown fake block"));
        }
        let offset = request.offset.unwrap_or(0);
        let length = request.length.unwrap_or(self.payload.len() as u64);
        self.requests
            .lock()
            .expect("requests lock")
            .insert(offset, length);
        if self.delay_first_segment && offset == 0 && request.length.is_some() {
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        }
        let start = usize::try_from(offset).map_err(|_| Status::out_of_range("offset"))?;
        let end = usize::try_from(offset + length).map_err(|_| Status::out_of_range("length"))?;
        if end > self.payload.len() {
            return Err(Status::out_of_range("segment exceeds payload"));
        }
        let mut payload = self.payload[start..end].to_vec();
        if self.corrupt_payload && !payload.is_empty() {
            payload[0] ^= 0x01;
        }
        let checksum = self
            .checksum_override
            .lock()
            .expect("checksum override lock")
            .clone()
            .unwrap_or_else(|| digest(&payload));
        Ok(Response::new(pb::PeerPullBlockResponse {
            serving_node_id: "fake-peer".to_string(),
            block_id: request.block_id,
            length: request.expected_length.unwrap_or(self.payload.len() as u64),
            checksum,
            payload,
        }))
    }

    async fn prepare_replica(
        &self,
        _request: Request<pb::PeerPrepareReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        Err(Status::unimplemented("fake peer only supports PullBlock"))
    }

    async fn activate_replica(
        &self,
        _request: Request<pb::PeerActivateReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        Err(Status::unimplemented("fake peer only supports PullBlock"))
    }

    async fn abort_replica(
        &self,
        _request: Request<pb::PeerAbortReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        Err(Status::unimplemented("fake peer only supports PullBlock"))
    }

    async fn get_replica_status(
        &self,
        _request: Request<pb::PeerReplicaStatusRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        Err(Status::unimplemented("fake peer only supports PullBlock"))
    }
}

struct FakePeerServer {
    endpoint: String,
    peer: FakePeer,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl FakePeerServer {
    async fn start(peer: FakePeer) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake peer");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("fake peer address")
        );
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_peer = peer.clone();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(PeerServiceServer::new(server_peer))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve fake peer");
        });
        Self {
            endpoint,
            peer,
            shutdown: Some(shutdown_tx),
            task,
        }
    }

    fn requested_segments(&self) -> Vec<(u64, u64)> {
        self.peer.requested_segments()
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
        let _ = self.task.await;
    }
}

async fn spawn_node_with_registry(
    meta_endpoint: &str,
    node_name: &str,
    node_id: u64,
    registry: &dms_metrics::Registry,
) -> NodeHandle {
    let metadata = MetadataClient::connect(
        meta_endpoint,
        node_id,
        format!("http://127.0.0.1:{}", 18_000 + node_id),
        None,
    )
    .await
    .expect("connect node to Meta");
    NodeHandle::spawn_with_metrics(
        node_name.to_string(),
        metadata,
        NodeTaskConfig {
            arena_capacity_bytes: 256 * 1024 * 1024,
            region_size_bytes: crate::config::DEFAULT_REGION_SIZE_BYTES,
            staging_ttl: Duration::from_secs(30),
            client_cache_lease_ttl: CLIENT_CACHE_LEASE_TTL,
            node_current_cache_bytes: crate::config::DEFAULT_NODE_CURRENT_CACHE_BYTES,
            node_current_cache_ttl: Duration::from_millis(
                crate::config::DEFAULT_NODE_CURRENT_CACHE_TTL_MILLIS,
            ),
            shared_fd_broker: None,
            log_level: dms_logging::LevelController::new(slog::Level::Info),
            trace_periodic_operations: false,
        },
        NodeMetrics::register(registry).expect("Node metrics"),
        dms_metrics::RpcMetrics::register(registry).expect("RPC metrics"),
    )
}

fn counter_value(text: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    text.lines()
        .find_map(|line| {
            if !line.starts_with(name) {
                return None;
            }
            if labels
                .iter()
                .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
            {
                return line
                    .split_whitespace()
                    .last()
                    .and_then(|value| value.parse().ok());
            }
            None
        })
        .unwrap_or_else(|| panic!("missing metric {name} with labels {labels:?}\n{text}"))
}

fn deterministic_bytes(length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| (index.wrapping_mul(31) % 251) as u8)
        .collect()
}

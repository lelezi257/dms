//! Node 侧 Current layout cache 的黑盒回归。
//!
//! 这些测试走真实 Tonic Meta 服务、真实 `MetadataClient`，并复用进程组合根中的
//! Meta watch / heartbeat 任务。这样覆盖的是 Node 与 Meta 的进程合同，而不是
//! `NodeState` 私有函数的局部行为。

use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use dms_client::{ClientOptions, DmsClient};
use dms_error::{self, DmsError, ErrorKind};
use dms_protocol::v1 as pb;
use dms_transport::dms_error_to_status;
use pb::{
    filesystem_metadata_service_server::{
        FilesystemMetadataService, FilesystemMetadataServiceServer,
    },
    metadata_service_server::{MetadataService, MetadataServiceServer},
    peer_service_server::{PeerService, PeerServiceServer},
    worker_payload_service_server::WorkerPayloadServiceServer,
    worker_service_server::WorkerServiceServer,
};
use tokio::{
    net::TcpListener,
    sync::{Notify, mpsc, oneshot},
    task::JoinHandle,
};
use tokio_stream::{Stream, wrappers::TcpListenerStream};
use tonic::{Request, Response, Status};

use super::{
    arena_manager::HostReceipt,
    consume_meta_events,
    data_core::{ByteRange, DataCoreHandle, ObjectKey, ReadOptions, VersionSelector},
    filesystem::{FileOperations, SharedFileOperations},
    image::ImageReader,
    metadata_client::MetadataClient,
    peer_service::PeerServiceHandler,
    runtime::{NodeEvent, NodeHandle, ReadTicket, SetRangeInput, WorkerError},
    send_meta_heartbeats, version_layout,
    worker_service::WorkerServiceHandler,
};
use crate::meta::{
    filesystem::service::FilesystemMetadataServiceHandler,
    metadata_service::MetadataServiceHandler, runtime::MetaHandle,
};

#[derive(Clone)]
struct CountingMetaService {
    inner: MetadataServiceHandler,
    commit_requests: Arc<Mutex<Vec<Vec<u8>>>>,
    resolve_count: Arc<AtomicUsize>,
    stat_count: Arc<AtomicUsize>,
    resolve_queries: Arc<Mutex<Vec<Option<u64>>>>,
    stale_location_once: Arc<AtomicBool>,
    resolve_delay: Arc<Mutex<Option<ResolveDelay>>>,
    watch_count: Arc<AtomicUsize>,
    reject_watch: Arc<AtomicBool>,
    reject_reports: Arc<AtomicBool>,
    report_count: Arc<AtomicUsize>,
    watch_drop: Arc<WatchDropControl>,
    ack_state: Arc<AckState>,
}

#[derive(Clone)]
struct CountingFilesystemMetaService {
    inner: FilesystemMetadataServiceHandler,
    commit_requests: Arc<Mutex<Vec<Vec<u8>>>>,
    drop_next_commit_response: Arc<AtomicBool>,
}

#[derive(Clone)]
struct ResolveDelay {
    key: Vec<u8>,
    fired: Arc<AtomicBool>,
    captured: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    release: Arc<Notify>,
}

struct ResolveDelayHandle {
    captured: oneshot::Receiver<()>,
    release: Arc<Notify>,
}

struct WatchDropControl {
    requested: AtomicBool,
    captured: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    waker: Mutex<Option<Waker>>,
}

struct WatchDropStream {
    inner: Pin<Box<dyn Stream<Item = Result<pb::NodeEvent, Status>> + Send>>,
    watch_drop: Arc<WatchDropControl>,
    ack_state: Arc<AckState>,
    node_id: u64,
}

struct AckState {
    delivered: Mutex<HashMap<Vec<u8>, DeliveredInvalidation>>,
    acknowledged: Mutex<Vec<DeliveredInvalidation>>,
    notify: Notify,
}

#[derive(Clone)]
struct DeliveredInvalidation {
    node_id: u64,
    key: Vec<u8>,
    minimum_version: u64,
    cursor: u64,
}

impl Stream for WatchDropStream {
    type Item = Result<pb::NodeEvent, Status>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.watch_drop.requested.swap(false, Ordering::AcqRel) {
            // 返回 None 等价于本次 server-stream 正常结束；Node 真实
            // consume_meta_events 会关闭新 cache lease，然后按 cursor 重连。
            if let Some(captured) = self
                .watch_drop
                .captured
                .lock()
                .expect("watch drop capture")
                .take()
            {
                let _ = captured.send(());
            }
            return Poll::Ready(None);
        }
        *self.watch_drop.waker.lock().expect("watch drop waker") = Some(context.waker().clone());
        let poll = self.inner.as_mut().poll_next(context);
        if let Poll::Ready(Some(Ok(event))) = &poll {
            self.ack_state.record_delivered(self.node_id, event.clone());
        }
        poll
    }
}

impl AckState {
    fn new() -> Self {
        Self {
            delivered: Mutex::new(HashMap::new()),
            acknowledged: Mutex::new(Vec::new()),
            notify: Notify::new(),
        }
    }

    fn record_delivered(&self, node_id: u64, event: pb::NodeEvent) {
        let Some(pb::node_event::Event::InvalidateCurrent(invalidation)) = event.event else {
            return;
        };
        let Some(key) = invalidation.key else {
            return;
        };
        self.delivered.lock().expect("delivered events").insert(
            event.event_id,
            DeliveredInvalidation {
                node_id,
                key: key.value,
                minimum_version: invalidation.minimum_version,
                cursor: event.cursor,
            },
        );
    }

    fn record_acknowledged(&self, event_id: &[u8], cursor: u64) {
        let delivered = self
            .delivered
            .lock()
            .expect("delivered events")
            .get(event_id)
            .filter(|event| event.cursor == cursor)
            .cloned();
        if let Some(event) = delivered {
            self.acknowledged
                .lock()
                .expect("acknowledged events")
                .push(event);
            self.notify.notify_waiters();
        }
    }

    fn has_acknowledged(&self, node_id: u64, key: &[u8], minimum_version: u64) -> bool {
        self.acknowledged
            .lock()
            .expect("acknowledged events")
            .iter()
            .any(|event| {
                event.node_id == node_id
                    && event.key == key
                    && event.minimum_version >= minimum_version
            })
    }
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
        let requested_key = request
            .get_ref()
            .key
            .as_ref()
            .map(|key| key.value.clone())
            .unwrap_or_default();
        self.resolve_count.fetch_add(1, Ordering::Relaxed);
        let exact = match request.get_ref().selector.as_ref() {
            Some(pb::resolve_object_request::Selector::ExactVersion(version)) => Some(*version),
            _ => None,
        };
        self.resolve_queries.lock().unwrap().push(exact);
        let mut response = self.inner.resolve_object(request).await;
        if self.stale_location_once.swap(false, Ordering::AcqRel)
            && let Ok(response) = &mut response
        {
            // 只损坏首个（原始）Block 的位置，patch 仍可正常首读。
            // Exact 刷新返回真实 Meta 位置，验证客户端不会改查另一个 Current。
            let resolved = response.get_mut();
            let first_block = resolved.layout.as_ref().unwrap().extents[0]
                .block_id
                .clone();
            let set = resolved
                .block_replicas
                .iter_mut()
                .find(|set| set.block_id == first_block)
                .unwrap();
            for replica in &mut set.replicas {
                replica.data_endpoint = "http://127.0.0.1:1".to_owned();
            }
        }
        let delay = self
            .resolve_delay
            .lock()
            .expect("resolve delay lock")
            .clone();
        if let Some(delay) = delay
            && delay.key == requested_key
            && !delay.fired.swap(true, Ordering::AcqRel)
        {
            // 这里故意在 Meta 已解析出旧 Current 后暂停响应，用来复现
            // “迟到的旧 resolve 响应不能重新污染 Node Current cache”的竞态。
            if let Some(captured) = delay.captured.lock().expect("capture lock").take() {
                let _ = captured.send(());
            }
            delay.release.notified().await;
        }
        response
    }

    async fn resolve_objects(
        &self,
        request: Request<pb::ResolveObjectsRequest>,
    ) -> Result<Response<pb::ResolveObjectsResponse>, Status> {
        let mut results = Vec::with_capacity(request.get_ref().requests.len());
        for request in request.into_inner().requests {
            match self.resolve_object(Request::new(request)).await {
                Ok(response) => results.push(pb::ResolveObjectResult {
                    response: Some(response.into_inner()),
                }),
                Err(status) if status.code() == tonic::Code::NotFound => {
                    results.push(pb::ResolveObjectResult { response: None });
                }
                Err(status) => return Err(status),
            }
        }
        Ok(Response::new(pb::ResolveObjectsResponse { results }))
    }

    async fn report_replicas(
        &self,
        request: Request<pb::ReportReplicasRequest>,
    ) -> Result<Response<pb::ReportReplicasResponse>, Status> {
        self.report_count.fetch_add(1, Ordering::AcqRel);
        if self.reject_reports.load(Ordering::Acquire) {
            return Err(dms_error_to_status(DmsError::new(
                dms_error::META_JOURNAL_UNAVAILABLE,
                ErrorKind::Unavailable,
                "test forced report failure",
            )));
        }
        self.inner.report_replicas(request).await
    }

    async fn commit_version(
        &self,
        request: Request<pb::CommitVersionRequest>,
    ) -> Result<Response<pb::CommitVersionResponse>, Status> {
        self.commit_requests
            .lock()
            .unwrap()
            .push(request.get_ref().operation_id.clone());
        self.inner.commit_version(request).await
    }

    async fn commit_batch(
        &self,
        request: Request<pb::CommitBatchRequest>,
    ) -> Result<Response<pb::CommitBatchResponse>, Status> {
        self.inner.commit_batch(request).await
    }

    async fn stat(
        &self,
        request: Request<pb::MetaStatRequest>,
    ) -> Result<Response<pb::MetaStatResponse>, Status> {
        self.stat_count.fetch_add(1, Ordering::Relaxed);
        self.inner.stat(request).await
    }

    async fn scan(
        &self,
        request: Request<pb::MetaScanRequest>,
    ) -> Result<Response<pb::MetaScanResponse>, Status> {
        self.inner.scan(request).await
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

    async fn watch_node_events(
        &self,
        request: Request<pb::WatchNodeEventsRequest>,
    ) -> Result<Response<Self::WatchNodeEventsStream>, Status> {
        // 只操控读节点的 Watch；写节点必须保持连通，不能共享单个测试 Waker。
        if request
            .get_ref()
            .session
            .as_ref()
            .map(|session| session.node_id)
            != Some(1)
        {
            return self.inner.watch_node_events(request).await;
        }
        self.watch_count.fetch_add(1, Ordering::AcqRel);
        if self.reject_watch.load(Ordering::Acquire) {
            return Err(Status::unavailable("test forced watch reconnect failure"));
        }
        let stream = self.inner.watch_node_events(request).await?.into_inner();
        Ok(Response::new(Box::pin(WatchDropStream {
            inner: stream,
            watch_drop: self.watch_drop.clone(),
            ack_state: self.ack_state.clone(),
            node_id: 1,
        })))
    }

    async fn acknowledge_node_event(
        &self,
        request: Request<pb::AcknowledgeNodeEventRequest>,
    ) -> Result<Response<pb::AcknowledgeNodeEventResponse>, Status> {
        let request = request.into_inner();
        let event_id = request.event_id.clone();
        let cursor = request.cursor;
        let response = self
            .inner
            .acknowledge_node_event(Request::new(request))
            .await?;
        self.ack_state.record_acknowledged(&event_id, cursor);
        Ok(response)
    }

    async fn acknowledge_block_retirement(
        &self,
        request: Request<pb::AcknowledgeBlockRetirementRequest>,
    ) -> Result<Response<pb::AcknowledgeBlockRetirementResponse>, Status> {
        self.inner.acknowledge_block_retirement(request).await
    }
}

#[tonic::async_trait]
impl FilesystemMetadataService for CountingFilesystemMetaService {
    async fn lookup_filesystem_entry(
        &self,
        request: Request<pb::FilesystemLookupRequest>,
    ) -> Result<Response<pb::FilesystemResolveResponse>, Status> {
        self.inner.lookup_filesystem_entry(request).await
    }

    async fn get_filesystem_inode(
        &self,
        request: Request<pb::FilesystemGetInodeRequest>,
    ) -> Result<Response<pb::FilesystemResolveResponse>, Status> {
        self.inner.get_filesystem_inode(request).await
    }

    async fn create_filesystem_inode(
        &self,
        request: Request<pb::FilesystemCreateInodeRequest>,
    ) -> Result<Response<pb::FilesystemResolveResponse>, Status> {
        self.inner.create_filesystem_inode(request).await
    }

    async fn create_filesystem_symlink(
        &self,
        request: Request<pb::FilesystemCreateSymlinkRequest>,
    ) -> Result<Response<pb::FilesystemResolveResponse>, Status> {
        self.inner.create_filesystem_symlink(request).await
    }

    async fn read_filesystem_directory(
        &self,
        request: Request<pb::FilesystemReadDirectoryRequest>,
    ) -> Result<Response<pb::FilesystemReadDirectoryResponse>, Status> {
        self.inner.read_filesystem_directory(request).await
    }

    async fn link_filesystem_entry(
        &self,
        request: Request<pb::FilesystemLinkRequest>,
    ) -> Result<Response<pb::FilesystemNamespaceMutationResponse>, Status> {
        self.inner.link_filesystem_entry(request).await
    }

    async fn acquire_filesystem_inode_reference(
        &self,
        request: Request<pb::FilesystemInodeReferenceRequest>,
    ) -> Result<Response<pb::FilesystemInodeReferenceResponse>, Status> {
        self.inner.acquire_filesystem_inode_reference(request).await
    }

    async fn release_filesystem_inode_reference(
        &self,
        request: Request<pb::FilesystemInodeReferenceRequest>,
    ) -> Result<Response<pb::FilesystemInodeReferenceResponse>, Status> {
        self.inner.release_filesystem_inode_reference(request).await
    }

    async fn renew_filesystem_inode_references(
        &self,
        request: Request<pb::FilesystemRenewInodeReferencesRequest>,
    ) -> Result<Response<pb::FilesystemInodeReferenceResponse>, Status> {
        self.inner.renew_filesystem_inode_references(request).await
    }

    async fn rename_filesystem_entry(
        &self,
        request: Request<pb::FilesystemRenameRequest>,
    ) -> Result<Response<pb::FilesystemNamespaceMutationResponse>, Status> {
        self.inner.rename_filesystem_entry(request).await
    }

    async fn remove_filesystem_entry(
        &self,
        request: Request<pb::FilesystemRemoveRequest>,
    ) -> Result<Response<pb::FilesystemNamespaceMutationResponse>, Status> {
        self.inner.remove_filesystem_entry(request).await
    }

    async fn commit_filesystem_version(
        &self,
        request: Request<pb::FilesystemCommitVersionRequest>,
    ) -> Result<Response<pb::FilesystemCommitVersionResponse>, Status> {
        self.commit_requests
            .lock()
            .expect("filesystem commit requests")
            .push(request.get_ref().operation_id.clone());
        let response = self.inner.commit_filesystem_version(request).await?;
        if self.drop_next_commit_response.swap(false, Ordering::AcqRel) {
            // 模拟“Meta 已提交成功，但响应在回到 Node 前丢失”。这类未知结果
            // 必须复用同一个 operation/request 重试，不能重新选择 O_APPEND 的 EOF。
            return Err(Status::unavailable(
                "test forced filesystem commit response loss",
            ));
        }
        Ok(response)
    }
}

struct CountingMetaServer {
    handle: MetaHandle,
    commit_requests: Arc<Mutex<Vec<Vec<u8>>>>,
    filesystem_commit_requests: Arc<Mutex<Vec<Vec<u8>>>>,
    drop_next_filesystem_commit_response: Arc<AtomicBool>,
    handler: MetadataServiceHandler,
    endpoint: String,
    resolve_count: Arc<AtomicUsize>,
    stat_count: Arc<AtomicUsize>,
    resolve_queries: Arc<Mutex<Vec<Option<u64>>>>,
    stale_location_once: Arc<AtomicBool>,
    resolve_delay: Arc<Mutex<Option<ResolveDelay>>>,
    watch_count: Arc<AtomicUsize>,
    reject_watch: Arc<AtomicBool>,
    reject_reports: Arc<AtomicBool>,
    report_count: Arc<AtomicUsize>,
    watch_drop: Arc<WatchDropControl>,
    ack_state: Arc<AckState>,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl CountingMetaServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind counting Meta");
        let endpoint = format!("http://{}", listener.local_addr().expect("Meta address"));
        let resolve_count = Arc::new(AtomicUsize::new(0));
        let stat_count = Arc::new(AtomicUsize::new(0));
        let resolve_queries = Arc::new(Mutex::new(Vec::new()));
        let stale_location_once = Arc::new(AtomicBool::new(false));
        let resolve_delay = Arc::new(Mutex::new(None));
        let watch_count = Arc::new(AtomicUsize::new(0));
        let reject_watch = Arc::new(AtomicBool::new(false));
        let reject_reports = Arc::new(AtomicBool::new(false));
        let report_count = Arc::new(AtomicUsize::new(0));
        let watch_drop = Arc::new(WatchDropControl {
            requested: AtomicBool::new(false),
            captured: Arc::new(Mutex::new(None)),
            waker: Mutex::new(None),
        });
        let ack_state = Arc::new(AckState::new());
        let handle = MetaHandle::spawn();
        let commit_requests = Arc::new(Mutex::new(Vec::new()));
        let filesystem_commit_requests = Arc::new(Mutex::new(Vec::new()));
        let drop_next_filesystem_commit_response = Arc::new(AtomicBool::new(false));
        let handler = MetadataServiceHandler::new(handle.clone());
        let registry = dms_metrics::registry();
        let filesystem_handler = CountingFilesystemMetaService {
            inner: FilesystemMetadataServiceHandler::new(
                handle.clone(),
                dms_metrics::RpcMetrics::register(&registry).expect("filesystem RPC metrics"),
                dms_metrics::ErrorMetrics::register(&registry).expect("filesystem error metrics"),
            ),
            commit_requests: filesystem_commit_requests.clone(),
            drop_next_commit_response: drop_next_filesystem_commit_response.clone(),
        };
        let service = CountingMetaService {
            inner: handler.clone(),
            commit_requests: commit_requests.clone(),
            resolve_count: resolve_count.clone(),
            stat_count: stat_count.clone(),
            resolve_queries: resolve_queries.clone(),
            stale_location_once: stale_location_once.clone(),
            resolve_delay: resolve_delay.clone(),
            watch_count: watch_count.clone(),
            reject_watch: reject_watch.clone(),
            reject_reports: reject_reports.clone(),
            report_count: report_count.clone(),
            watch_drop: watch_drop.clone(),
            ack_state: ack_state.clone(),
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(MetadataServiceServer::new(service))
                .add_service(FilesystemMetadataServiceServer::new(filesystem_handler))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve counting Meta");
        });
        Self {
            handle,
            commit_requests,
            filesystem_commit_requests,
            drop_next_filesystem_commit_response,
            handler,
            endpoint,
            resolve_count,
            stat_count,
            resolve_queries,
            stale_location_once,
            resolve_delay,
            watch_count,
            reject_watch,
            reject_reports,
            report_count,
            watch_drop,
            ack_state,
            shutdown: Some(shutdown_tx),
            task,
        }
    }

    fn reset_resolve_count(&self) {
        self.resolve_count.store(0, Ordering::Relaxed);
    }

    fn reset_stat_count(&self) {
        self.stat_count.store(0, Ordering::Relaxed);
    }

    fn reset_meta_read_counts(&self) {
        self.reset_resolve_count();
        self.reset_stat_count();
    }

    fn drop_next_commit_response(&self) {
        self.handler.drop_next_commit_response_for_test();
    }

    fn drop_next_filesystem_commit_response(&self) {
        self.drop_next_filesystem_commit_response
            .store(true, Ordering::Release);
    }

    fn resolve_count(&self) -> usize {
        self.resolve_count.load(Ordering::Relaxed)
    }

    fn stat_count(&self) -> usize {
        self.stat_count.load(Ordering::Relaxed)
    }

    fn delay_next_resolve_for_key(&self, key: &[u8]) -> ResolveDelayHandle {
        let (captured_tx, captured_rx) = oneshot::channel();
        let release = Arc::new(Notify::new());
        *self.resolve_delay.lock().expect("resolve delay lock") = Some(ResolveDelay {
            key: key.to_vec(),
            fired: Arc::new(AtomicBool::new(false)),
            captured: Arc::new(Mutex::new(Some(captured_tx))),
            release: release.clone(),
        });
        ResolveDelayHandle {
            captured: captured_rx,
            release,
        }
    }

    fn watch_count(&self) -> usize {
        self.watch_count.load(Ordering::Acquire)
    }

    fn set_watch_rejected(&self, rejected: bool) {
        self.reject_watch.store(rejected, Ordering::Release);
    }

    fn set_report_rejected(&self, rejected: bool) {
        self.reject_reports.store(rejected, Ordering::Release);
    }

    fn reset_report_count(&self) {
        self.report_count.store(0, Ordering::Release);
    }

    async fn wait_for_report_count_at_least(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.report_count.load(Ordering::Acquire) < expected {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("background replica report did not make progress");
    }

    fn drop_current_watch(&self) -> oneshot::Receiver<()> {
        let (captured_tx, captured_rx) = oneshot::channel();
        *self.watch_drop.captured.lock().expect("watch drop capture") = Some(captured_tx);
        self.watch_drop.requested.store(true, Ordering::Release);
        if let Some(waker) = self
            .watch_drop
            .waker
            .lock()
            .expect("watch drop waker")
            .take()
        {
            waker.wake();
        }
        captured_rx
    }

    async fn wait_for_invalidation_ack(&self, node_id: u64, key: &[u8], minimum_version: u64) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self
                .ack_state
                .has_acknowledged(node_id, key, minimum_version)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Meta did not observe invalidation ACK for node={node_id} key={} min_version={minimum_version}",
                String::from_utf8_lossy(key)
            );
            tokio::select! {
                _ = self.ack_state.notify.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
    }
}

impl Drop for CountingMetaServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

struct TestNode {
    node: NodeHandle,
    _watch_task: JoinHandle<()>,
    _heartbeat_task: JoinHandle<()>,
    _peer_task: Option<JoinHandle<()>>,
    peer_pull_count: Option<Arc<AtomicUsize>>,
    peer_pull_delay_ms: Arc<AtomicU64>,
    peer_max_active: Arc<AtomicUsize>,
}

impl TestNode {
    async fn start(meta_endpoint: &str, node_id: u64) -> Self {
        Self::start_inner(meta_endpoint, node_id, None).await
    }

    async fn start_with_peer_server(meta_endpoint: &str, node_id: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind peer server");
        let endpoint = format!("http://{}", listener.local_addr().expect("peer addr"));
        Self::start_inner(meta_endpoint, node_id, Some((endpoint, listener))).await
    }

    async fn start_inner(
        meta_endpoint: &str,
        node_id: u64,
        peer_listener: Option<(String, TcpListener)>,
    ) -> Self {
        let advertised_endpoint = peer_listener
            .as_ref()
            .map(|(endpoint, _)| endpoint.clone())
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", 19_200 + node_id));
        let metadata = MetadataClient::connect(meta_endpoint, node_id, advertised_endpoint, None)
            .await
            .expect("connect Meta");
        let registry = dms_metrics::registry();
        let node = NodeHandle::spawn_with_metrics(
            format!("test-node-{node_id}"),
            metadata.clone(),
            super::runtime::NodeTaskConfig {
                arena_capacity_bytes: 256 * 1024 * 1024,
                region_size_bytes: crate::config::DEFAULT_REGION_SIZE_BYTES,
                staging_ttl: Duration::from_secs(30),
                client_cache_lease_ttl: Duration::from_millis(
                    crate::config::DEFAULT_CLIENT_CACHE_LEASE_TTL_MILLIS,
                ),
                node_current_cache_bytes: crate::config::DEFAULT_NODE_CURRENT_CACHE_BYTES,
                // 故意长于迟到响应的测试期限，确保安全性不是靠缓存自然过期。
                node_current_cache_ttl: Duration::from_secs(30),
                shared_fd_broker: None,
                log_level: dms_logging::LevelController::new(slog::Level::Info),
                trace_periodic_operations: false,
            },
            super::metrics::NodeMetrics::register(&registry).expect("Node metrics"),
            dms_metrics::RpcMetrics::register(&registry).expect("RPC metrics"),
        );
        let initial_watch = metadata.watch_events(0).await.expect("open Meta watch");
        let lease_started = Instant::now();
        let lease_ttl = metadata.heartbeat(0).await.expect("Meta heartbeat");
        node.metadata_lease(Some(lease_started + Duration::from_millis(lease_ttl)), None)
            .await
            .expect("install Meta lease");
        let acked_cursor = Arc::new(AtomicU64::new(0));
        let watch_task = tokio::spawn(consume_meta_events(
            metadata.clone(),
            node.clone(),
            node_id,
            Some(initial_watch),
            acked_cursor.clone(),
        ));
        let heartbeat_task =
            tokio::spawn(send_meta_heartbeats(metadata, node.clone(), acked_cursor));
        let peer_pull_count = peer_listener
            .as_ref()
            .map(|_| Arc::new(AtomicUsize::new(0)));
        let peer_pull_delay_ms = Arc::new(AtomicU64::new(0));
        let peer_max_active = Arc::new(AtomicUsize::new(0));
        let max_active = peer_max_active.clone();
        let peer_task = peer_listener.map(|(_, listener)| {
            let handler = PeerServiceHandler::new(node.clone());
            let peer_pull_count = peer_pull_count.as_ref().expect("peer pull counter").clone();
            let delay_ms = peer_pull_delay_ms.clone();
            tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(PeerServiceServer::new(CountingPeerService {
                        inner: handler,
                        pull_count: peer_pull_count,
                        delay_ms,
                        active: Arc::new(AtomicUsize::new(0)),
                        max_active,
                    }))
                    .serve_with_incoming(TcpListenerStream::new(listener))
                    .await
                    .expect("serve peer test server");
            })
        });
        wait_for_metadata_watch(&node).await;
        Self {
            node,
            _watch_task: watch_task,
            _heartbeat_task: heartbeat_task,
            _peer_task: peer_task,
            peer_pull_count,
            peer_pull_delay_ms,
            peer_max_active,
        }
    }

    async fn open_cached_session(&self) -> (u64, mpsc::Receiver<NodeEvent>) {
        let session_id = self.node.open_session(false).await.expect("open session");
        let (event_tx, event_rx) = mpsc::channel(16);
        self.node
            .attach_session(session_id, event_tx)
            .await
            .expect("attach session");
        wait_for_cache_lease(&self.node, session_id).await;
        (session_id, event_rx)
    }

    async fn open_write_session(&self) -> u64 {
        self.node
            .open_session(false)
            .await
            .expect("open writer session")
    }

    async fn set_inline(&self, session_id: u64, key: &[u8], value: &[u8], sequence: u64) -> u64 {
        self.node
            .set_inline(
                session_id,
                key.to_vec(),
                value.to_vec(),
                operation_id(0x51, sequence),
                "any".to_string(),
            )
            .await
            .expect("set inline")
            .version
    }

    async fn read_inline(&self, session_id: u64, key: &[u8]) -> ReadTicket {
        self.node
            .get_with_inline_limit(session_id, key.to_vec(), None, None, 64 * 1024)
            .await
            .expect("get inline")
    }

    async fn read_inline_range(
        &self,
        session_id: u64,
        key: &[u8],
        range: (u64, u64),
    ) -> ReadTicket {
        self.node
            .get_with_inline_limit(session_id, key.to_vec(), None, Some(range), 64 * 1024)
            .await
            .expect("get inline range")
    }

    fn reset_peer_pull_count(&self) {
        if let Some(count) = &self.peer_pull_count {
            count.store(0, Ordering::Relaxed);
        }
    }

    fn peer_pull_count(&self) -> usize {
        self.peer_pull_count
            .as_ref()
            .map(|count| count.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
}

#[derive(Clone)]
struct CountingPeerService {
    inner: PeerServiceHandler,
    pull_count: Arc<AtomicUsize>,
    delay_ms: Arc<AtomicU64>,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

// 测量 handler 的真实重叠区间；取消 Future 时也要扣减，不能只在成功出口计数。
struct ActivePeerCall(Arc<AtomicUsize>);
impl Drop for ActivePeerCall {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[tonic::async_trait]
impl PeerService for CountingPeerService {
    async fn probe(
        &self,
        request: Request<pb::PeerProbeRequest>,
    ) -> Result<Response<pb::PeerProbeResponse>, Status> {
        self.inner.probe(request).await
    }

    async fn pull_block(
        &self,
        request: Request<pb::PeerPullBlockRequest>,
    ) -> Result<Response<pb::PeerPullBlockResponse>, Status> {
        self.pull_count.fetch_add(1, Ordering::Relaxed);
        let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
        self.max_active.fetch_max(active, Ordering::AcqRel);
        let _active = ActivePeerCall(self.active.clone());
        // 测试专用的慢 Peer：扩大两个已到达请求的重叠窗口，不改变业务内容。
        let delay_ms = self.delay_ms.load(Ordering::Relaxed);
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        self.inner.pull_block(request).await
    }

    async fn prepare_replica(
        &self,
        request: Request<pb::PeerPrepareReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        self.inner.prepare_replica(request).await
    }

    async fn activate_replica(
        &self,
        request: Request<pb::PeerActivateReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        self.inner.activate_replica(request).await
    }

    async fn abort_replica(
        &self,
        request: Request<pb::PeerAbortReplicaRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        self.inner.abort_replica(request).await
    }

    async fn get_replica_status(
        &self,
        request: Request<pb::PeerReplicaStatusRequest>,
    ) -> Result<Response<pb::PeerReplicaStatusResponse>, Status> {
        self.inner.get_replica_status(request).await
    }
}

impl Drop for TestNode {
    fn drop(&mut self) {
        self._watch_task.abort();
        self._heartbeat_task.abort();
        if let Some(peer_task) = &self._peer_task {
            peer_task.abort();
        }
    }
}

/// 在现有 TestNode 上暴露真实 SDK 入口；计数使用产品 RPC metrics，避免另写一套
/// Worker fake 或把“第二个 Client 读成功”误当成“请求确实到了 Node”的证据。
struct TestWorkerServer {
    endpoint: String,
    registry: dms_metrics::Registry,
    task: JoinHandle<()>,
}

impl TestWorkerServer {
    async fn start(node: &TestNode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind Worker server");
        let endpoint = format!("http://{}", listener.local_addr().expect("Worker address"));
        let registry = dms_metrics::registry();
        let handler = WorkerServiceHandler::with_metrics(
            node.node.clone(),
            false,
            dms_metrics::RpcMetrics::register(&registry).expect("Worker RPC metrics"),
            dms_metrics::ErrorMetrics::register(&registry).expect("Worker error metrics"),
        );
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(WorkerServiceServer::new(handler.clone()))
                .add_service(WorkerPayloadServiceServer::new(handler))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .expect("serve Worker test server");
        });
        Self {
            endpoint,
            registry,
            task,
        }
    }
}

impl Drop for TestWorkerServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn completed_worker_calls(registry: &dms_metrics::Registry, method: &str) -> u64 {
    let text = dms_metrics::encode_text(registry).expect("encode Worker metrics");
    text.lines()
        .filter(|line| {
            line.starts_with("dms_rpc_server_requests_total{")
                && line.contains("service=\"WorkerService\"")
                && line.contains(&format!("method=\"{method}\""))
        })
        .map(|line| {
            line.rsplit_once(' ')
                .expect("metric value")
                .1
                .parse::<u64>()
                .expect("integer counter")
        })
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn thin_clients_share_node_blocks_without_sdk_value_cache() {
    let meta = CountingMetaServer::start().await;
    let node_a = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let node_b = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let worker_a = TestWorkerServer::start(&node_a).await;
    let worker_b = TestWorkerServer::start(&node_b).await;
    let endpoint_a = worker_a.endpoint.clone();
    let endpoint_b = worker_b.endpoint.clone();
    let worker_metrics = worker_b.registry.clone();
    let peer_pulls = node_a.peer_pull_count.as_ref().unwrap().clone();

    // DmsClient 是同步 API，必须在 blocking 线程里创建/使用/释放自己的 Runtime。
    // 两个独立 reader 分别使用默认配置和旧非零预算，均不能重新开启 SDK value cache。
    tokio::task::spawn_blocking(move || {
        let writer = DmsClient::connect(&endpoint_a, ClientOptions::default()).expect("writer");
        let mut readers = Vec::new();
        for old_budget in [None, Some(64 * 1024 * 1024)] {
            let registry = dms_metrics::registry();
            let client = DmsClient::connect(
                &endpoint_b,
                ClientOptions {
                    current_cache_bytes: old_budget,
                    metrics_registry: Some(registry.clone()),
                    heartbeat_interval: Some(Duration::from_millis(1)),
                    ..ClientOptions::default()
                },
            )
            .expect("reader");
            readers.push((client, registry));
        }
        let key = "thin-client/shared-peer-block";
        writer.set(key, b"value-v1").expect("publish v1 on A");
        let pulls_before = peer_pulls.load(Ordering::Relaxed);
        let gets_before = completed_worker_calls(&worker_metrics, "Get");

        assert_eq!(
            readers[0].0.get(key).expect("first reader"),
            Some(b"value-v1".to_vec())
        );
        let pulls_after_first = peer_pulls.load(Ordering::Relaxed);
        assert_eq!(
            pulls_after_first,
            pulls_before + 1,
            "B 首读从 A 拉取一个 Block"
        );

        // 同一 Client 重读和第二个 Client 首读都继续经过 B；value 已在 B，不能再拉 A。
        for _ in 0..2 {
            for (reader, _) in &readers {
                assert_eq!(
                    reader.get(key).expect("Node cache read"),
                    Some(b"value-v1".to_vec())
                );
            }
        }
        assert_eq!(
            completed_worker_calls(&worker_metrics, "Get"),
            gets_before + 5
        );
        assert_eq!(peer_pulls.load(Ordering::Relaxed), pulls_after_first);

        // 不轮询、不 sleep：同步写成功后，两 reader 的第一次读取都必须看到新值。
        writer.set(key, b"value-v2").expect("publish v2 on A");
        for (reader, _) in &readers {
            assert_eq!(
                reader.get(key).expect("first read after SET"),
                Some(b"value-v2".to_vec())
            );
        }
        assert_eq!(peer_pulls.load(Ordering::Relaxed), pulls_after_first + 1);
        writer.del(key).expect("delete on A");
        for (reader, registry) in &readers {
            assert!(reader.get(key).expect("first read after DEL").is_none());
            let metrics = dms_metrics::encode_text(registry).expect("SDK metrics");
            assert!(
                !metrics.contains("dms_client_cache_"),
                "旧 value cache 指标不应再注册: {metrics}"
            );
        }
        assert_eq!(
            completed_worker_calls(&worker_metrics, "Get"),
            gets_before + 9
        );
        assert_eq!(
            completed_worker_calls(&worker_metrics, "Heartbeat"),
            0,
            "生命周期使用 Session stream，不应调用旧 value-cache unary Heartbeat"
        );
    })
    .await
    .expect("join SDK clients");
}

fn is_not_found(error: &WorkerError) -> bool {
    matches!(error, WorkerError::NotFound)
        || matches!(error, WorkerError::Stable(stable) if stable.kind() == ErrorKind::NotFound)
}

async fn wait_for_metadata_watch(node: &NodeHandle) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let session = node.open_session(false).await.expect("probe session");
        let (event_tx, _event_rx) = mpsc::channel(1);
        node.attach_session(session, event_tx)
            .await
            .expect("attach probe session");
        if node.renew_cache_lease(session, None).await.expect("renew") > 0 {
            node.close_session(session).await.expect("close probe");
            return;
        }
        node.close_session(session).await.expect("close probe");
        assert!(
            Instant::now() < deadline,
            "Meta watch did not become cache-lease eligible"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_cache_lease(node: &NodeHandle, session_id: u64) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if node
            .renew_cache_lease(session_id, None)
            .await
            .expect("renew cache lease")
            > 0
        {
            return;
        }
        assert!(Instant::now() < deadline, "cache lease was not granted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn open_session_without_cache_lease(node: &NodeHandle) -> u64 {
    let session_id = node
        .open_session(false)
        .await
        .expect("open uncached session");
    let (event_tx, _event_rx) = mpsc::channel(1);
    node.attach_session(session_id, event_tx)
        .await
        .expect("attach uncached session");
    assert_eq!(
        node.renew_cache_lease(session_id, None)
            .await
            .expect("renew uncached session"),
        0,
        "Meta watch 断开期间，新 session 不能获得 Current cache lease"
    );
    session_id
}

async fn wait_for_watch_count(meta: &CountingMetaServer, minimum: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if meta.watch_count() >= minimum {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Meta watch did not reconnect to expected generation"
        );
        tokio::task::yield_now().await;
    }
}

fn operation_id(prefix: u8, sequence: u64) -> Vec<u8> {
    let mut id = vec![prefix; 16];
    id.extend_from_slice(&sequence.to_be_bytes());
    id
}

async fn resolve_filesystem_content_layout(
    meta: &CountingMetaServer,
    inode: u64,
    version: u64,
) -> pb::ResolveObjectResponse {
    meta.handle
        .debug_resolve_object_without_session(
            format!("fs/content/{inode}").into_bytes(),
            Some(version),
        )
        .await
        .expect("resolve filesystem content layout")
}

async fn acknowledge_next_invalidation(
    node: &NodeHandle,
    session_id: u64,
    events: &mut mpsc::Receiver<NodeEvent>,
    expected_key: &[u8],
    expected_minimum_version: u64,
) {
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .expect("wait invalidation")
        .expect("invalidation event");
    let NodeEvent::InvalidateCurrent {
        event_sequence,
        key,
        minimum_version,
        ..
    } = event;
    assert_eq!(key, expected_key);
    assert!(
        minimum_version >= expected_minimum_version,
        "invalidation must cover at least the version that replaced the cached Current layout"
    );
    node.acknowledge(session_id, event_sequence)
        .await
        .expect("ack invalidation");
}

async fn set_range_with_node(
    node: NodeHandle,
    session_id: u64,
    key: Vec<u8>,
    offset: u64,
    value: Vec<u8>,
    expected_version: u64,
    sequence: u64,
) -> u64 {
    let allocation = node
        .allocate_staging(session_id, value.len() as u64)
        .await
        .expect("allocate range patch");
    let receipt = node
        .upload(allocation.transfer_id, value)
        .await
        .expect("upload range patch");
    node.set_range(SetRangeInput {
        session_id,
        key,
        offset,
        staging_id: allocation.staging_id,
        receipt,
        operation_id: operation_id(0x52, sequence),
        expected_version: Some(expected_version),
    })
    .await
    .expect("set range")
    .version
}

async fn stage_value(node: &NodeHandle, session_id: u64, value: Vec<u8>) -> (u64, HostReceipt) {
    let allocation = node
        .allocate_staging(session_id, value.len() as u64)
        .await
        .expect("allocate mset value");
    let receipt = node
        .upload(allocation.transfer_id, value)
        .await
        .expect("upload mset value");
    (allocation.staging_id, receipt)
}

async fn mset_with_node(
    node: NodeHandle,
    session_id: u64,
    entries: Vec<(Vec<u8>, Vec<u8>)>,
    sequence: u64,
) -> Vec<super::runtime::KeySetOutcome> {
    let mut staged = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let (staging_id, receipt) = stage_value(&node, session_id, value).await;
        staged.push((key, staging_id, receipt));
    }
    node.mset(session_id, staged, operation_id(0x56, sequence))
        .await
        .expect("mset")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_commit_primes_node_layout_cache_for_current_get() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let writer = node.open_write_session().await;
    let key = b"node-cache/read-through";
    node.set_inline(writer, key, b"v1", 1).await;
    let (session, _events) = node.open_cached_session().await;

    meta.reset_resolve_count();
    let first = node.read_inline(session, key).await;
    assert_eq!(first.inline_value.as_deref(), Some(b"v1".as_slice()));
    let second = node.read_inline(session, key).await;
    assert_eq!(second.inline_value.as_deref(), Some(b"v1".as_slice()));

    assert_eq!(
        meta.resolve_count(),
        0,
        "本 Node 刚完成的提交应直接回填 Current layout cache，后续 GET 不应再次访问 Meta"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_core_file_and_image_paths_share_one_object_state() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let core = DataCoreHandle::new(node.node.clone());
    let fs = FileOperations::new(core.clone());
    let image = ImageReader::new(core.clone());

    core.put(
        ObjectKey::new(b"fs:/shared/object".to_vec()).expect("valid object key"),
        b"abcdef".to_vec(),
    )
    .await
    .expect("in-process object put through DataCore");
    let range = fs
        .read("/shared/object", 1, 3)
        .await
        .expect("FS read through DataCore")
        .expect("object exists");
    assert_eq!(range.bytes, b"bcd");

    let layer = image
        .open_layer(b"fs:/shared/object".to_vec())
        .await
        .expect("Image open through DataCore")
        .expect("layer exists");
    let mut output = [0_u8; 2];
    let read = image
        .read_exact_at(&layer, 2, &mut output)
        .await
        .expect("Image read_exact_at")
        .expect("layer still exists");
    assert_eq!(read, 2);
    assert_eq!(&output, b"cd");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_core_write_is_not_limited_by_grpc_single_message_size() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let core = DataCoreHandle::new(node.node.clone());
    let key = ObjectKey::new(b"core/larger-than-grpc-message".to_vec()).unwrap();
    let bytes = vec![0x5a; (super::runtime::GRPC_PAYLOAD_SAFE_BYTES + 1) as usize];

    core.put(key.clone(), bytes.clone())
        .await
        .expect("进程内 DataCore 写入只受 Arena 容量约束，不受 gRPC 单消息限制");
    let read = core
        .read(key, ReadOptions::default())
        .await
        .expect("read DataCore object")
        .expect("object exists");
    assert_eq!(read.bytes, bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filesystem_random_write_uses_core_range_semantics() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let fs = FileOperations::new(DataCoreHandle::new(node.node.clone()));

    fs.create("/cases/random-write")
        .await
        .expect("create empty file");
    let initial = fs
        .write("/cases/random-write", 0, b"abcdef")
        .await
        .expect("write initial value");
    let patched = fs
        .write_range("/cases/random-write", 2, b"ZZ")
        .await
        .expect("range patch");
    assert!(
        patched.version > initial.version,
        "range write must publish a new immutable object version"
    );
    let inspector = MetadataClient::connect(
        &meta.endpoint,
        91,
        "http://127.0.0.1:20991".to_string(),
        None,
    )
    .await
    .expect("connect Meta inspector");
    let layout = inspector
        .resolve(b"fs:/cases/random-write".to_vec(), Some(patched.version))
        .await
        .expect("resolve patched layout")
        .layout
        .expect("patched layout");
    assert_eq!(
        layout.extents.len(),
        3,
        "DataCore range write must commit an Extent overlay, not materialize and replace the full object"
    );

    let read = fs
        .read("/cases/random-write", 0, 64)
        .await
        .expect("read patched file")
        .expect("file exists");
    assert_eq!(read.bytes, b"abZZef");
    assert_eq!(read.logical_length, 6);

    fs.delete("/cases/random-write")
        .await
        .expect("delete file object");
    assert!(
        fs.stat("/cases/random-write")
            .await
            .expect("stat deleted file")
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filesystem_create_is_if_absent_and_preserves_existing_object() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let core = DataCoreHandle::new(node.node.clone());
    let key = ObjectKey::new(b"fs:/cases/existing".to_vec()).unwrap();
    core.put(key.clone(), b"keep-me".to_vec())
        .await
        .expect("seed existing object");

    let fs = FileOperations::new(core.clone());
    assert!(
        fs.create("/cases/existing")
            .await
            .is_err_and(|error| error.is_version_conflict())
    );
    let read = core
        .read(key, ReadOptions::default())
        .await
        .expect("read existing object")
        .expect("object remains present");
    assert_eq!(read.bytes, b"keep-me");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_core_exact_full_read_retries_with_resolved_version_length() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let core = DataCoreHandle::new(node.node.clone());
    let key = ObjectKey::new(b"core/exact-longer-than-current".to_vec()).unwrap();
    let first = core
        .put(key.clone(), b"long-version".to_vec())
        .await
        .expect("put long first version");
    core.put(key.clone(), b"x".to_vec())
        .await
        .expect("replace with shorter current version");

    // 全量读最初按 Current 的 1 byte 预分配，但 Exact 目标需要 12 bytes。
    // Node 返回目标版本的 required length，DataCore 固定该版本扩容后重试；
    // 这不能被误报为 ENOSPC。
    let read = core
        .read(
            key,
            ReadOptions {
                version: VersionSelector::Exact(first.version),
                range: None,
                clamp_range: false,
            },
        )
        .await
        .expect("read exact old version")
        .expect("exact version exists");
    assert_eq!(read.version, first.version);
    assert_eq!(read.bytes, b"long-version");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_filesystem_extensions_preserve_both_writes() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let fs = FileOperations::new(DataCoreHandle::new(node.node.clone()));
    fs.create("/cases/concurrent-extend")
        .await
        .expect("create file");
    fs.write("/cases/concurrent-extend", 0, b"ab")
        .await
        .expect("seed file");

    let left = fs.write("/cases/concurrent-extend", 2, b"X");
    let right = fs.write("/cases/concurrent-extend", 3, b"Y");
    let (left, right) = tokio::join!(left, right);
    left.expect("first extension eventually commits");
    right.expect("second extension retries on version conflict");

    let read = fs
        .read("/cases/concurrent-extend", 0, 16)
        .await
        .expect("read file")
        .expect("file exists");
    assert_eq!(read.bytes, b"abXY");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filesystem_truncate_shrinks_and_zero_extends() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let fs = FileOperations::new(DataCoreHandle::new(node.node.clone()));
    fs.create("/cases/truncate").await.expect("create file");
    fs.write("/cases/truncate", 0, b"abcdef")
        .await
        .expect("write file");

    fs.truncate("/cases/truncate", 3)
        .await
        .expect("shrink file");
    let shrunk = fs.read("/cases/truncate", 0, 16).await.unwrap().unwrap();
    assert_eq!(shrunk.bytes, b"abc");

    fs.truncate("/cases/truncate", 5)
        .await
        .expect("extend file with zeroes");
    let extended = fs.read("/cases/truncate", 0, 16).await.unwrap().unwrap();
    assert_eq!(extended.bytes, b"abc\0\0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn image_second_lazy_range_read_reuses_node_peer_cache() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"image/lazy-layer";
    writer_node
        .set_inline(writer, key, b"0123456789abcdef", 710)
        .await;

    let image = ImageReader::new(DataCoreHandle::new(reader_node.node.clone()));
    let layer = image
        .open_layer(key.to_vec())
        .await
        .expect("open remote image layer")
        .expect("layer exists");
    assert_eq!(ImageReader::layer_length(&layer), 16);

    writer_node.reset_peer_pull_count();
    meta.reset_resolve_count();
    let first = image
        .read_at(&layer, 4, 4)
        .await
        .expect("first lazy range read")
        .expect("range exists");
    assert_eq!(first.bytes, b"4567");
    assert_eq!(
        meta.resolve_count(),
        1,
        "固定版本第一次 lazy read 仍需向 Meta resolve 该版本的布局"
    );
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "第一次读取远端 layer range 需要一次 peer payload"
    );

    writer_node.reset_peer_pull_count();
    meta.reset_resolve_count();
    let second = image
        .read_at(&layer, 4, 4)
        .await
        .expect("second lazy range read")
        .expect("range exists");
    assert_eq!(second.bytes, b"4567");
    assert_eq!(
        meta.resolve_count(),
        1,
        "当前尚未新增 Exact layout cache；第二次固定版本读取仍会 resolve，后续若要优化应复用 NodeState 内同一份解析缓存"
    );
    assert_eq!(
        writer_node.peer_pull_count(),
        0,
        "同一固定版本的相同 range 第二次应复用 Node 已安装 Block"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_core_current_cache_is_shared_with_filesystem_without_meta_or_peer_on_second_read() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"fs:/cache/shared-current";
    writer_node
        .set_inline(writer, key, b"0123456789abcdef", 711)
        .await;

    let core = DataCoreHandle::new(reader_node.node.clone());
    writer_node.reset_peer_pull_count();
    meta.reset_meta_read_counts();
    let first = core
        .read(
            ObjectKey::new(key.to_vec()).expect("object key"),
            ReadOptions::current_range(ByteRange::new(4, 4).expect("range")),
        )
        .await
        .expect("first DataCore current read")
        .expect("object exists");
    assert_eq!(first.bytes, b"4567");
    assert_eq!(
        meta.resolve_count(),
        1,
        "首次 DataCore Current miss 必须向 Meta resolve，并把同一 Node CurrentCache 安全回填"
    );
    assert_eq!(
        meta.stat_count(),
        0,
        "带 range 的 DataCore read 不应为了预分配返回 buffer 额外 stat Meta"
    );
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "首次跨节点 Current 读缺本地 Block，需要一次 peer payload"
    );

    let fs = FileOperations::new(core);
    writer_node.reset_peer_pull_count();
    meta.reset_meta_read_counts();
    let second = fs
        .read("/cache/shared-current", 4, 4)
        .await
        .expect("second Filesystem current read")
        .expect("object exists");
    assert_eq!(second.bytes, b"4567");
    assert_eq!(
        meta.resolve_count(),
        0,
        "Filesystem 与 DataCore 共享同一 NodeState CurrentCache；第二次 Current 热读不再访问 Meta"
    );
    assert_eq!(
        meta.stat_count(),
        0,
        "Filesystem range read 的完整热路径不应再为了 capacity 访问 Meta stat"
    );
    assert_eq!(
        writer_node.peer_pull_count(),
        0,
        "首次 DataCore 读已把远端 Block 安装到本 Node；Filesystem 热读不再走 peer payload"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_filesystem_write_peer_read_revoke_and_patch_are_one_version_chain() {
    let meta = CountingMetaServer::start().await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer_fs = SharedFileOperations::new(writer_node.node.clone()).expect("writer FS");
    let reader_fs = SharedFileOperations::new(reader_node.node.clone()).expect("reader FS");

    let writer = writer_fs
        .create_and_open(b"shared.txt", 0o644, 2)
        .await
        .expect("create and open on Node A");
    let first_version = writer_fs
        .write(writer.id, 0, b"abcdef")
        .await
        .expect("initial write-through commit");
    assert_eq!(first_version.length, 6);

    let looked_up = reader_fs
        .lookup(crate::filesystem::ROOT_INODE, b"shared.txt")
        .await
        .expect("lookup from Node B")
        .expect("shared file exists");
    let reader = reader_fs
        .open(looked_up.granted.inode.attributes.inode, 0)
        .await
        .expect("open on Node B");

    writer_node.reset_peer_pull_count();
    let cold = reader_fs
        .read(reader.id, 0, 6)
        .await
        .expect("Node B first read")
        .expect("file content");
    assert_eq!(cold.bytes, b"abcdef");
    assert_eq!(writer_node.peer_pull_count(), 1, "首读只拉一次 payload");

    let hot = reader_fs
        .read(reader.id, 0, 6)
        .await
        .expect("Node B hot read")
        .expect("file content");
    assert_eq!(hot.bytes, b"abcdef");
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "binding 与 Block 均命中时不再访问 Peer payload"
    );

    let patched = writer_fs
        .write(writer.id, 2, b"ZZ")
        .await
        .expect("Node A range write waits for Node B revoke ACK");
    assert!(patched.version > first_version.version);

    let refreshed = reader_fs
        .read(reader.id, 0, 6)
        .await
        .expect("Node B read after revoke")
        .expect("file content");
    assert_eq!(refreshed.bytes, b"abZZef");
    assert_eq!(
        writer_node.peer_pull_count(),
        2,
        "旧 base Block 已在 Node B；新版本只需拉取一个 patch Block"
    );

    writer_fs.close(writer.id).await.expect("close writer");
    reader_fs.close(reader.id).await.expect("close reader");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_filesystem_sparse_pwrite_and_truncate_do_not_publish_zero_blocks() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let fs = SharedFileOperations::new(node.node.clone()).expect("shared FS");
    let opened = fs
        .create_and_open(b"sparse.bin", 0o644, libc::O_RDWR)
        .await
        .expect("create sparse file");

    let written = fs
        .write(opened.id, 5, b"xy")
        .await
        .expect("pwrite beyond EOF");
    assert_eq!(written.length, 7);
    let pwrite_read = fs
        .read(opened.id, 0, 7)
        .await
        .expect("read sparse pwrite")
        .expect("sparse content exists");
    assert_eq!(pwrite_read.bytes, b"\0\0\0\0\0xy");

    let pwrite_layout = resolve_filesystem_content_layout(&meta, opened.inode, written.version)
        .await
        .layout
        .expect("pwrite layout");
    assert_eq!(
        pwrite_layout.extents.len(),
        2,
        "越过 EOF 的 pwrite 只能产生 hole + 用户数据 Block，不能分配零 Block"
    );
    assert!(
        version_layout::is_hole(&pwrite_layout.extents[0]),
        "第一个 Extent 必须是逻辑零洞"
    );
    assert!(
        !version_layout::is_hole(&pwrite_layout.extents[1]),
        "第二个 Extent 才是用户写入的数据 Block"
    );
    let pwrite_resolved =
        resolve_filesystem_content_layout(&meta, opened.inode, written.version).await;
    assert_eq!(
        pwrite_resolved.block_replicas.len(),
        1,
        "Meta resolve 不应为 sparse hole 返回空 block replica 集"
    );

    let grown = fs.truncate(opened.inode, 10).await.expect("truncate grow");
    let grow_version = grown
        .granted
        .inode
        .content
        .as_ref()
        .expect("grown content binding")
        .exact_version;
    let grow_read = fs
        .read(opened.id, 0, 10)
        .await
        .expect("read grown sparse file")
        .expect("grown content exists");
    assert_eq!(grow_read.bytes, b"\0\0\0\0\0xy\0\0\0");
    let grow_resolved = resolve_filesystem_content_layout(&meta, opened.inode, grow_version).await;
    let grow_layout = grow_resolved.layout.as_ref().expect("grow layout");
    assert_eq!(
        grow_resolved.block_replicas.len(),
        1,
        "truncate grow 只追加逻辑 hole，不新增零 Block replica"
    );
    assert!(
        grow_layout.extents.iter().any(version_layout::is_hole),
        "扩容后的布局必须显式保留逻辑零洞"
    );

    let shrunk = fs.truncate(opened.inode, 4).await.expect("truncate shrink");
    let shrink_version = shrunk
        .granted
        .inode
        .content
        .as_ref()
        .expect("shrunk content binding")
        .exact_version;
    let shrink_read = fs
        .read(opened.id, 0, 4)
        .await
        .expect("read shrunk sparse file")
        .expect("shrunk content exists");
    assert_eq!(shrink_read.bytes, b"\0\0\0\0");
    let shrink_resolved =
        resolve_filesystem_content_layout(&meta, opened.inode, shrink_version).await;
    let shrink_layout = shrink_resolved.layout.as_ref().expect("shrink layout");
    assert_eq!(
        shrink_layout.extents.len(),
        1,
        "shrink 只裁剪仍可见的 Extent 前缀，不复制已被截掉的数据 Block"
    );
    assert!(
        version_layout::is_hole(&shrink_layout.extents[0]),
        "截断到原 hole 内时，新版本只剩逻辑零洞"
    );
    assert!(
        shrink_resolved.block_replicas.is_empty(),
        "只包含 hole 的文件版本不需要任何 Block replica"
    );

    fs.close(opened.id).await.expect("close sparse file");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_filesystem_open_trunc_publishes_empty_version_before_handle_returns() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let fs = SharedFileOperations::new(node.node.clone()).expect("shared FS");
    let opened = fs
        .create_and_open(b"open-trunc.txt", 0o644, libc::O_RDWR)
        .await
        .expect("create file");
    let initial = fs
        .write(opened.id, 0, b"abcdef")
        .await
        .expect("initial content");
    assert_eq!(initial.length, 6);
    fs.close(opened.id).await.expect("close initial handle");

    let trunc_handle = fs
        .open(opened.inode, libc::O_RDWR | libc::O_TRUNC)
        .await
        .expect("open with O_TRUNC");
    let inode = fs
        .get_inode(opened.inode)
        .await
        .expect("get inode after O_TRUNC");
    assert_eq!(
        inode.granted.inode.attributes.size, 0,
        "O_TRUNC open 返回前必须已经原子发布 size=0"
    );
    let version = inode
        .granted
        .inode
        .content
        .as_ref()
        .expect("empty content binding")
        .exact_version;
    assert!(
        version > initial.version,
        "O_TRUNC 必须发布一个新的空文件版本，而不是只改本地 handle"
    );
    let empty = fs
        .read(trunc_handle.id, 0, 64)
        .await
        .expect("read after O_TRUNC")
        .expect("empty file read returns metadata");
    assert!(empty.bytes.is_empty());
    assert_eq!(empty.logical_length, 0);
    let resolved = resolve_filesystem_content_layout(&meta, opened.inode, version).await;
    let layout = resolved.layout.expect("empty version layout");
    assert!(layout.extents.is_empty());
    assert!(resolved.block_replicas.is_empty());

    fs.close(trunc_handle.id)
        .await
        .expect("close O_TRUNC handle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_filesystem_append_uses_authoritative_eof_and_retries_on_conflict() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let fs = SharedFileOperations::new(node.node.clone()).expect("shared FS");
    let base = fs
        .create_and_open(b"append.txt", 0o644, libc::O_RDWR)
        .await
        .expect("create append file");
    fs.write(base.id, 0, b"base").await.expect("base content");
    let appender = fs
        .open(base.inode, libc::O_WRONLY | libc::O_APPEND)
        .await
        .expect("open appender");

    let appended = fs
        .write(appender.id, 0, b"-tail")
        .await
        .expect("append ignores caller offset");
    assert_eq!(appended.length, 9);
    let content = fs
        .read(base.id, 0, 32)
        .await
        .expect("read append content")
        .expect("append content exists");
    assert_eq!(
        content.bytes, b"base-tail",
        "O_APPEND 不能信任 caller offset=0，必须以权威 EOF 追加"
    );

    let first = fs.clone();
    let second = fs.clone();
    let first_handle = appender.id;
    let second_handle = appender.id;
    let (first, second) = tokio::join!(
        async move { first.write(first_handle, 0, b"-A").await },
        async move { second.write(second_handle, 0, b"-B").await },
    );
    first.expect("first concurrent append should eventually commit");
    second.expect("second concurrent append should retry after CAS conflict and commit");
    let final_read = fs
        .read(base.id, 0, 64)
        .await
        .expect("read final append content")
        .expect("final content exists");
    assert!(
        final_read.bytes == b"base-tail-A-B" || final_read.bytes == b"base-tail-B-A",
        "并发 O_APPEND 必须形成两个完整追加段，不能互相覆盖或使用旧 offset"
    );

    fs.close(appender.id).await.expect("close appender");
    fs.close(base.id).await.expect("close base handle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_filesystem_append_survives_sixteen_concurrent_writers() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let fs = SharedFileOperations::new(node.node.clone()).expect("shared FS");
    let base = fs
        .create_and_open(b"append-16.txt", 0o644, libc::O_RDWR)
        .await
        .expect("create append file");
    fs.write(base.id, 0, b"base").await.expect("base content");
    let appender = fs
        .open(base.inode, libc::O_WRONLY | libc::O_APPEND)
        .await
        .expect("open shared appender");

    let mut tasks = Vec::new();
    for index in 0..16 {
        let writer = fs.clone();
        let handle = appender.id;
        let chunk = format!("-{index:02}").into_bytes();
        tasks.push(tokio::spawn(async move {
            writer.write(handle, 0, &chunk).await.map(|_| chunk)
        }));
    }

    let mut expected_chunks = Vec::new();
    for task in tasks {
        let chunk = task.await.expect("append writer task should finish");
        expected_chunks.push(chunk.expect("append writer should retry conflicts and commit"));
    }

    let final_read = fs
        .read(base.id, 0, 128)
        .await
        .expect("read final append content")
        .expect("final content exists");
    assert_eq!(final_read.bytes.len(), b"base".len() + 16 * 3);
    assert!(
        final_read.bytes.starts_with(b"base"),
        "O_APPEND 并发写只能追加，不能覆盖已有内容"
    );
    for chunk in expected_chunks {
        let matches = final_read
            .bytes
            .windows(chunk.len())
            .filter(|window| *window == chunk.as_slice())
            .count();
        assert_eq!(matches, 1, "每个并发 writer 的追加片段必须恰好发布一次");
    }

    fs.close(appender.id).await.expect("close appender");
    fs.close(base.id).await.expect("close base handle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_filesystem_append_reuses_operation_when_commit_result_is_unknown() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let fs = SharedFileOperations::new(node.node.clone()).expect("shared FS");
    let base = fs
        .create_and_open(b"append-unknown-result.txt", 0o644, libc::O_RDWR)
        .await
        .expect("create append file");
    fs.write(base.id, 0, b"base").await.expect("base content");
    let appender = fs
        .open(base.inode, libc::O_WRONLY | libc::O_APPEND)
        .await
        .expect("open appender");

    meta.filesystem_commit_requests
        .lock()
        .expect("filesystem commit requests")
        .clear();
    meta.drop_next_filesystem_commit_response();
    let appended = fs
        .write(appender.id, 0, b"-tail")
        .await
        .expect("append retries unknown commit result");
    assert_eq!(appended.length, 9);

    let content = fs
        .read(base.id, 0, 32)
        .await
        .expect("read append content")
        .expect("append content exists");
    assert_eq!(
        content.bytes, b"base-tail",
        "未知提交结果只能复用同一个 operation 重试，不能重新选 EOF 后重复追加"
    );

    {
        let requests = meta
            .filesystem_commit_requests
            .lock()
            .expect("filesystem commit requests");
        assert!(requests.len() >= 2, "测试必须真实触发提交后响应丢失和重试");
        assert!(
            requests.windows(2).all(|pair| pair[0] == pair[1]),
            "未知提交结果重试必须复用同一个 operation id"
        );
    }

    fs.close(appender.id).await.expect("close appender");
    fs.close(base.id).await.expect("close base handle");
}

#[test]
fn data_core_and_process_local_subsystems_do_not_depend_on_wire_or_sdk_crates() {
    let files = [
        ("data_core.rs", include_str!("data_core.rs")),
        ("filesystem/mod.rs", include_str!("filesystem/mod.rs")),
        ("image/mod.rs", include_str!("image/mod.rs")),
    ];
    for (name, source) in files {
        for forbidden in [
            "use dms_client",
            "use tonic",
            "use dms_protocol",
            "dms_protocol::",
            "pb::",
        ] {
            assert!(
                !source.contains(forbidden),
                "{name} must stay a process-local object/subsystem module and not depend on {forbidden}"
            );
        }
    }
}

#[test]
fn worker_and_process_local_object_boundaries_remain_siblings() {
    let data_core = include_str!("data_core.rs");
    let worker_service = include_str!("worker_service.rs");

    assert!(
        !data_core.contains("session_id:") && !data_core.contains("from_session("),
        "DataCoreHandle 只服务进程内子系统，不能重新承接 Worker session"
    );
    assert!(
        !worker_service.contains("use super::data_core")
            && !worker_service.contains("DataCoreHandle::"),
        "WorkerService 应直接适配 NodeHandle，不能借道进程内 DataCoreHandle"
    );
}

#[test]
fn data_core_owned_read_does_not_bounce_through_read_into() {
    let source = include_str!("data_core.rs");
    let read_start = source
        .find("    pub(crate) async fn read(")
        .expect("DataCoreHandle::read exists");
    let read_into_start = source
        .find("    pub(crate) async fn read_into(")
        .expect("DataCoreHandle::read_into exists");
    let read_body = &source[read_start..read_into_start];
    assert!(
        !read_body.contains("self.read_into("),
        "owned read must submit one owned Vec to Node owner directly instead of allocating A, delegating to read_into, allocating B, then copying B back into A"
    );
    assert!(
        read_body.contains(".data_core_read_into("),
        "owned read should still reuse the same Node owner materialization path as read_into"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_core_read_into_uses_caller_buffer_contract() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let core = DataCoreHandle::new(node.node.clone());
    core.put(
        ObjectKey::new(b"core/read-into".to_vec()).unwrap(),
        b"payload".to_vec(),
    )
    .await
    .expect("put object");

    let mut target = [0_u8; 4];
    let meta = core
        .read_into(
            ObjectKey::new(b"core/read-into".to_vec()).unwrap(),
            ReadOptions::current_range(ByteRange::new(3, 4).unwrap()),
            &mut target,
        )
        .await
        .expect("read into caller buffer")
        .expect("object exists");
    assert_eq!(meta.bytes_read, 4);
    assert_eq!(&target, b"load");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_range_invalidates_node_layout_cache_before_next_current_read() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let (reader, mut reader_events) = node.open_cached_session().await;
    let writer = node.open_write_session().await;
    let key = b"node-cache/set-range";
    let v1 = node.set_inline(writer, key, b"abcd", 10).await;
    let first = node.read_inline(reader, key).await;
    assert_eq!(first.inline_value.as_deref(), Some(b"abcd".as_slice()));

    meta.reset_resolve_count();
    let write = tokio::spawn(set_range_with_node(
        node.node.clone(),
        writer,
        key.to_vec(),
        1,
        b"Z".to_vec(),
        v1,
        11,
    ));
    acknowledge_next_invalidation(&node.node, reader, &mut reader_events, key, v1 + 1).await;
    let v2 = write.await.expect("join set range");
    assert!(v2 > v1);
    // 同一 Node 内的 Client barrier 已在上面确认。写入 Node 会在提交路径直接
    // 更新本机 Current，不再等待一条发给自己的 Meta Watch 事件。

    // SET_RANGE 写路径本身需要向 Meta resolve base layout；这里重新计数，
    // 只验证后续 Current 读是否“第一次 resolve、第二次命中 Node cache”。
    meta.reset_resolve_count();
    let second = node.read_inline(reader, key).await;
    assert_eq!(second.inline_value.as_deref(), Some(b"aZcd".as_slice()));
    let third = node.read_inline(reader, key).await;
    assert_eq!(third.inline_value.as_deref(), Some(b"aZcd".as_slice()));
    assert_eq!(
        meta.resolve_count(),
        1,
        "SET_RANGE 后第一次读必须重新 resolve，新版本随后再次进入 Node cache"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_reads_skip_current_cache_but_range_current_reads_reuse_it() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let writer = node.open_write_session().await;
    let key = b"node-cache/exact-range";
    let version = node.set_inline(writer, key, b"abcdef", 12).await;
    let (reader, _events) = node.open_cached_session().await;

    meta.reset_resolve_count();
    for _ in 0..2 {
        let exact = node
            .node
            .get_with_inline_limit(reader, key.to_vec(), Some(version), None, 64 * 1024)
            .await
            .expect("exact read");
        assert_eq!(exact.version, version);
        assert_eq!(exact.inline_value.as_deref(), Some(b"abcdef".as_slice()));
    }
    assert_eq!(
        meta.resolve_count(),
        2,
        "ExactVersion 读不代表 Current，可绕过 Current layout cache"
    );

    meta.reset_resolve_count();
    let range_a = node.read_inline_range(reader, key, (1, 3)).await;
    assert_eq!(range_a.version, version);
    assert_eq!(range_a.inline_value.as_deref(), Some(b"bcd".as_slice()));
    let range_b = node.read_inline_range(reader, key, (1, 3)).await;
    assert_eq!(range_b.version, version);
    assert_eq!(range_b.inline_value.as_deref(), Some(b"bcd".as_slice()));
    assert_eq!(
        meta.resolve_count(),
        0,
        "ExactVersion 读不污染写提交回填的 Current cache，Current range read 可直接复用"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_range_hit_only_requires_blocks_that_intersect_requested_range() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"node-cache/range-required-blocks";

    let v1 = writer_node.set_inline(writer, key, b"abcdefgh", 50).await;
    let v2 = set_range_with_node(
        writer_node.node.clone(),
        writer,
        key.to_vec(),
        4,
        b"Z".to_vec(),
        v1,
        51,
    )
    .await;
    assert_eq!(v2, v1 + 1);
    let (reader, _events) = reader_node.open_cached_session().await;

    writer_node.reset_peer_pull_count();
    meta.reset_resolve_count();
    let first_patch_read = reader_node.read_inline_range(reader, key, (4, 1)).await;
    assert_eq!(first_patch_read.version, v2);
    assert_eq!(
        first_patch_read.inline_value.as_deref(),
        Some(b"Z".as_slice())
    );
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "第一次 range 读只需拉取覆盖本次 range 的 patch Block"
    );
    assert_eq!(
        meta.resolve_count(),
        1,
        "第一次 range 读需要一次 Current resolve 来获得版本和位置"
    );

    writer_node.reset_peer_pull_count();
    meta.reset_resolve_count();
    let cached_patch_read = reader_node.read_inline_range(reader, key, (4, 1)).await;
    assert_eq!(cached_patch_read.version, v2);
    assert_eq!(
        cached_patch_read.inline_value.as_deref(),
        Some(b"Z".as_slice())
    );
    assert_eq!(
        writer_node.peer_pull_count(),
        0,
        "range 命中本地 patch Block 后，不应因为同版本旧前缀 Block 缺失而拉取 peer"
    );
    assert_eq!(
        meta.resolve_count(),
        0,
        "range 只依赖 patch Block；旧前缀 Block 缺失不应迫使同一有效 Current 再次 resolve"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_current_locations_pull_missing_peer_blocks_without_current_resolve() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"node-cache/cached-peer-location";

    let v1 = writer_node.set_inline(writer, key, b"abcdefgh", 60).await;
    let v2 = set_range_with_node(
        writer_node.node.clone(),
        writer,
        key.to_vec(),
        4,
        b"Z".to_vec(),
        v1,
        61,
    )
    .await;
    assert_eq!(v2, v1 + 1);
    let (reader, _events) = reader_node.open_cached_session().await;

    let patch = reader_node.read_inline_range(reader, key, (4, 1)).await;
    assert_eq!(patch.version, v2);
    assert_eq!(patch.inline_value.as_deref(), Some(b"Z".as_slice()));

    writer_node.reset_peer_pull_count();
    meta.reset_resolve_count();
    let full = reader_node.read_inline(reader, key).await;
    assert_eq!(full.version, v2);
    assert_eq!(full.inline_value.as_deref(), Some(b"abcdZfgh".as_slice()));
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "缓存位置提示可以直接补齐缺失 base Block，且只补本次缺失 Block"
    );
    assert_eq!(
        meta.resolve_count(),
        0,
        "缓存里已有同一 Current 版本的位置提示；补齐缺失旧 Block 不应重复 Resolve Current"
    );

    writer_node.reset_peer_pull_count();
    meta.reset_resolve_count();
    let local = reader_node.read_inline(reader, key).await;
    assert_eq!(local.version, v2);
    assert_eq!(local.inline_value.as_deref(), Some(b"abcdZfgh".as_slice()));
    assert_eq!(
        writer_node.peer_pull_count(),
        0,
        "缺失 Block 补齐后，本地命中不再访问 peer"
    );
    assert_eq!(
        meta.resolve_count(),
        0,
        "Block 补齐后，本地命中仍保持 0 Meta Resolve"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_location_read_retries_replica_report_outside_foreground_get() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"node-cache/report-failure";

    let v1 = writer_node.set_inline(writer, key, b"abcdefgh", 70).await;
    let v2 = set_range_with_node(
        writer_node.node.clone(),
        writer,
        key.to_vec(),
        4,
        b"Z".to_vec(),
        v1,
        71,
    )
    .await;
    let (reader, _events) = reader_node.open_cached_session().await;

    let patch = reader_node.read_inline_range(reader, key, (4, 1)).await;
    assert_eq!(patch.version, v2);
    assert_eq!(patch.inline_value.as_deref(), Some(b"Z".as_slice()));

    writer_node.reset_peer_pull_count();
    meta.reset_resolve_count();
    meta.reset_report_count();
    meta.set_report_rejected(true);
    let result = reader_node
        .node
        .get_with_inline_limit(reader, key.to_vec(), None, None, 64 * 1024)
        .await
        .expect("validated and installed bytes must not wait for replica discovery");
    assert_eq!(result.inline_value.as_deref(), Some(b"abcdZfgh".as_slice()));
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "前台只拉取一次缺失的 base Block"
    );
    assert_eq!(
        meta.resolve_count(),
        0,
        "缓存位置路径不应先回 Meta 重新解析 Current"
    );
    meta.wait_for_report_count_at_least(1).await;
    meta.set_report_rejected(false);
    meta.wait_for_report_count_at_least(2).await;

    writer_node.reset_peer_pull_count();
    let local = reader_node.read_inline(reader, key).await;
    assert_eq!(local.inline_value.as_deref(), Some(b"abcdZfgh".as_slice()));
    assert_eq!(
        writer_node.peer_pull_count(),
        0,
        "安装后的 Block 可直接复用"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_cold_half_reads_share_one_peer_import() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"peer-flight/two-half-reads";
    let value = (0..32768)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let version = writer_node.set_inline(writer, key, &value, 701).await;
    let reader = reader_node.open_write_session().await;
    writer_node.peer_pull_delay_ms.store(100, Ordering::Relaxed);
    writer_node.reset_peer_pull_count();

    // 两次独立 Exact GET，各读同一个不可变 Block 的一半；不能只在单个
    // layout 内去重，也不能把两个用户的读票据/生命周期合成一个。
    let read = |offset| {
        reader_node.node.get_with_inline_limit(
            reader,
            key.to_vec(),
            Some(version),
            Some((offset, 16384)),
            65536,
        )
    };
    let (left, right) = tokio::join!(read(0), read(16384));
    assert_eq!(left.unwrap().inline_value.as_deref(), Some(&value[..16384]));
    assert_eq!(
        right.unwrap().inline_value.as_deref(),
        Some(&value[16384..])
    );
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "同 Block 并发冷读应只搬一次 bytes"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_filesystem_and_image_cold_reads_share_one_peer_import() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"fs:/shared/block-flight";
    let value = (0..32768)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    writer_node.set_inline(writer, key, &value, 712).await;

    let core = DataCoreHandle::new(reader_node.node.clone());
    let fs = FileOperations::new(core.clone());
    let image = ImageReader::new(core);
    let layer = image
        .open_layer(key.to_vec())
        .await
        .expect("open image layer")
        .expect("remote layer exists");

    writer_node.peer_pull_delay_ms.store(100, Ordering::Relaxed);
    writer_node.reset_peer_pull_count();
    meta.reset_meta_read_counts();

    // 文件与镜像两个子系统同时读同一个远端 Block 的不同 range。它们分别走
    // DataCore Current/Exact 入口，但缺块安装都回到同一个 NodeState::peer_imports，
    // 因而只能有一个真正的 Node→Node payload pull。
    let fs_read = fs.read("/shared/block-flight", 0, 16_384);
    let image_read = image.read_at(&layer, 16_384, 16_384);
    let (fs_result, image_result) = tokio::join!(fs_read, image_read);
    let fs_result = fs_result
        .expect("filesystem read")
        .expect("filesystem object exists");
    let image_result = image_result
        .expect("image read")
        .expect("image object exists");
    assert_eq!(fs_result.bytes, value[..16_384]);
    assert_eq!(image_result.bytes, value[16_384..]);
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "跨子系统的同 Block 并发冷读必须复用 NodeState.peer_imports singleflight"
    );
}

/// 证据测试用系统 sha256sum 校验真实返回值，不把长度相同当成数据完整。
#[cfg(target_os = "linux")]
fn proof_sha256(bytes: &[u8]) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mechanism_proof_different_blocks_transfer_concurrently() {
    let meta = CountingMetaServer::start().await;
    let reader = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let ws = writer.open_write_session().await;
    let rs = reader.open_write_session().await;
    let values = [vec![17; 32768], vec![93; 32768]];
    let keys = [b"proof/block-a".as_slice(), b"proof/block-b".as_slice()];
    let a = writer.set_inline(ws, keys[0], &values[0], 901).await;
    let b = writer.set_inline(ws, keys[1], &values[1], 902).await;
    writer.peer_pull_delay_ms.store(150, Ordering::Release);
    let (left, right) = tokio::join!(
        reader
            .node
            .get_with_inline_limit(rs, keys[0].to_vec(), Some(a), None, 65536),
        reader
            .node
            .get_with_inline_limit(rs, keys[1].to_vec(), Some(b), None, 65536),
    );
    let returned = [
        left.unwrap().inline_value.unwrap(),
        right.unwrap().inline_value.unwrap(),
    ];
    let hashes = returned.each_ref().map(|value| proof_sha256(value));
    let expected = values.each_ref().map(|value| proof_sha256(value));
    assert_eq!(hashes, expected);
    let max_active = writer.peer_max_active.load(Ordering::Acquire);
    assert!(max_active >= 2, "不同 Block 不能被单 flight 队列串行化");
    assert_eq!(writer.peer_pull_count(), keys.len());
    println!(
        "DMS_MECHANISM_PROOF {{\"mechanism\":\"different_blocks\",\"requested_blocks\":{},\"max_simultaneous_transfers\":{},\"pull_count\":{},\"all_sha_match\":{},\"sha256\":[\"{}\",\"{}\"],\"measurement\":\"real-peer-grpc-handler-overlap\"}}",
        keys.len(),
        max_active,
        writer.peer_pull_count(),
        hashes == expected,
        hashes[0],
        hashes[1]
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mechanism_proof_lost_commit_reply_retry_keeps_one_logical_version() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let session = node.open_write_session().await;
    let before = meta.handle.stats().await.unwrap();
    let key = b"proof/retry";
    let value = b"value-after-lost-commit-response";
    let identity = operation_id(0x51, 903);
    meta.drop_next_commit_response();
    let result = node
        .node
        .set_inline(
            session,
            key.to_vec(),
            value.to_vec(),
            identity.clone(),
            "any".into(),
        )
        .await
        .unwrap();
    let returned = node
        .node
        .get_with_inline_limit(session, key.to_vec(), Some(result.version), None, 65536)
        .await
        .unwrap()
        .inline_value
        .unwrap();
    let expected_sha = proof_sha256(value);
    let returned_sha = proof_sha256(&returned);
    assert_eq!(expected_sha, returned_sha);
    let after = meta.handle.stats().await.unwrap();
    let logical_commits = after.version_count - before.version_count;
    assert_eq!(logical_commits, 1);
    assert_eq!(after.operation_count - before.operation_count, 1);
    let requests = meta.commit_requests.lock().unwrap();
    assert!(requests.len() >= 2);
    let preserved = requests.windows(2).all(|pair| pair[0] == pair[1]);
    assert!(preserved);
    println!(
        "DMS_MECHANISM_PROOF {{\"mechanism\":\"retry\",\"attempts\":{},\"operation_id_preserved\":{},\"logical_commits\":{},\"returned_sha_match\":{},\"sha256\":\"{}\",\"versions_before\":{},\"versions_after\":{},\"measurement\":\"real-meta-grpc-post-commit-response-loss\"}}",
        requests.len(),
        preserved,
        logical_commits,
        expected_sha == returned_sha,
        returned_sha,
        before.version_count,
        after.version_count
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_exact_and_materialized_reads_share_one_peer_import() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"peer-flight/exact-and-materialized";
    let value = vec![37; 32768];
    let version = writer_node.set_inline(writer, key, &value, 702).await;
    let reader = reader_node.open_write_session().await;
    writer_node.peer_pull_delay_ms.store(100, Ordering::Relaxed);
    let (range, materialized) = tokio::join!(
        reader_node.node.get_with_inline_limit(
            reader,
            key.to_vec(),
            Some(version),
            Some((0, 16384)),
            65536
        ),
        reader_node
            .node
            .get_materialized(reader, key.to_vec(), Some(version)),
    );
    assert_eq!(
        range.unwrap().inline_value.as_deref(),
        Some(&value[..16384])
    );
    assert_eq!(materialized.unwrap(), (version, value));
    assert_eq!(writer_node.peer_pull_count(), 1);
    assert_eq!(reader_node.node.debug_peer_imports().await, (0, 0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_cached_location_and_exact_reads_share_one_peer_import() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"peer-flight/cached-location-and-exact";
    let v1 = writer_node.set_inline(writer, key, b"abcdefgh", 706).await;
    let v2 = set_range_with_node(
        writer_node.node.clone(),
        writer,
        key.to_vec(),
        4,
        b"Z".to_vec(),
        v1,
        707,
    )
    .await;
    let (reader, _events) = reader_node.open_cached_session().await;
    // 只读取 patch，缓存 Current 布局及远端位置，但 base Block 仍未下载。
    let patch = reader_node.read_inline_range(reader, key, (4, 1)).await;
    assert_eq!(patch.inline_value.as_deref(), Some(b"Z".as_slice()));
    writer_node.reset_peer_pull_count();
    meta.reset_resolve_count();
    writer_node.peer_pull_delay_ms.store(100, Ordering::Relaxed);
    let (cached, exact) = tokio::join!(
        reader_node
            .node
            .get_with_inline_limit(reader, key.to_vec(), None, Some((0, 4)), 65536),
        reader_node
            .node
            .get_with_inline_limit(reader, key.to_vec(), Some(v2), Some((5, 3)), 65536),
    );
    assert_eq!(
        cached.unwrap().inline_value.as_deref(),
        Some(b"abcd".as_slice())
    );
    assert_eq!(
        exact.unwrap().inline_value.as_deref(),
        Some(b"fgh".as_slice())
    );
    assert_eq!(writer_node.peer_pull_count(), 1);
    assert_eq!(meta.resolve_count(), 1, "只有 Exact 入口需要一次权威解析");
    assert_eq!(reader_node.node.debug_peer_imports().await, (0, 0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_import_shares_one_pull_while_report_retries_in_background() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"peer-flight/report-rejected";
    let version = writer_node.set_inline(writer, key, b"abcdefgh", 703).await;
    let reader = reader_node.open_write_session().await;
    writer_node.peer_pull_delay_ms.store(100, Ordering::Relaxed);
    meta.reset_report_count();
    meta.set_report_rejected(true);
    let read = || {
        reader_node
            .node
            .get_with_inline_limit(reader, key.to_vec(), Some(version), None, 65536)
    };
    let (first, second) = tokio::join!(read(), read());
    assert_eq!(
        first.unwrap().inline_value.as_deref(),
        Some(b"abcdefgh".as_slice())
    );
    assert_eq!(
        second.unwrap().inline_value.as_deref(),
        Some(b"abcdefgh".as_slice())
    );
    assert_eq!(writer_node.peer_pull_count(), 1);
    assert_eq!(reader_node.node.debug_peer_imports().await, (0, 0, 0));
    meta.wait_for_report_count_at_least(1).await;
    meta.set_report_rejected(false);
    meta.wait_for_report_count_at_least(2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_import_leader_does_not_cancel_other_readers() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"peer-flight/cancel-leader";
    let version = writer_node.set_inline(writer, key, b"abcdefgh", 704).await;
    let reader = reader_node.open_write_session().await;
    writer_node.peer_pull_delay_ms.store(150, Ordering::Relaxed);
    let node = reader_node.node.clone();
    let leader = tokio::spawn(async move {
        node.get_with_inline_limit(reader, key.to_vec(), Some(version), None, 65536)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while writer_node.peer_pull_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("leader reached peer");
    let node = reader_node.node.clone();
    let follower = tokio::spawn(async move {
        node.get_with_inline_limit(reader, key.to_vec(), Some(version), Some((4, 4)), 65536)
            .await
    });
    leader.abort();
    assert!(leader.await.unwrap_err().is_cancelled());
    let result = follower.await.unwrap().unwrap();
    assert_eq!(result.inline_value.as_deref(), Some(b"efgh".as_slice()));
    assert_eq!(writer_node.peer_pull_count(), 1);
    assert_eq!(reader_node.node.debug_peer_imports().await, (0, 0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_all_import_waiters_leave_no_inflight_state() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"peer-flight/cancel-all";
    let version = writer_node.set_inline(writer, key, b"abcdefgh", 705).await;
    let reader = reader_node.open_write_session().await;
    writer_node.peer_pull_delay_ms.store(100, Ordering::Relaxed);
    let node = reader_node.node.clone();
    let request = tokio::spawn(async move {
        node.get_with_inline_limit(reader, key.to_vec(), Some(version), None, 65536)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while writer_node.peer_pull_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        while reader_node.node.debug_peer_imports().await != (0, 0, 0) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("detached bounded import finished and released scope pin");
    assert_eq!(writer_node.peer_pull_count(), 1);
    let result = reader_node
        .node
        .get_with_inline_limit(reader, key.to_vec(), Some(version), None, 65536)
        .await
        .unwrap();
    assert_eq!(result.inline_value.as_deref(), Some(b"abcdefgh".as_slice()));
    assert_eq!(
        writer_node.peer_pull_count(),
        1,
        "已安装数据仍可复用，无独立客户端cache"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_stale_location_refreshes_exact_version_once() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"node-cache/stale-location";
    let v1 = writer_node.set_inline(writer, key, b"abcdefgh", 80).await;
    let v2 = set_range_with_node(
        writer_node.node.clone(),
        writer,
        key.to_vec(),
        4,
        b"Z".to_vec(),
        v1,
        81,
    )
    .await;
    let (reader, _events) = reader_node.open_cached_session().await;
    meta.stale_location_once.store(true, Ordering::Release);
    assert_eq!(
        reader_node
            .read_inline_range(reader, key, (4, 1))
            .await
            .version,
        v2
    );

    meta.reset_resolve_count();
    meta.resolve_queries.lock().unwrap().clear();
    writer_node.reset_peer_pull_count();
    let result = reader_node.read_inline(reader, key).await;
    assert_eq!(result.version, v2);
    assert_eq!(result.inline_value.as_deref(), Some(b"abcdZfgh".as_slice()));
    assert_eq!(meta.resolve_count(), 1);
    assert_eq!(*meta.resolve_queries.lock().unwrap(), vec![Some(v2)]);
    assert_eq!(writer_node.peer_pull_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_node_authority_rechecks_meta_even_with_all_bytes_local() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"node-cache/expired-authority";
    writer_node
        .set_inline(writer, key, b"local-after-first-read", 90)
        .await;
    let (reader, _events) = reader_node.open_cached_session().await;
    reader_node.read_inline(reader, key).await;
    // 模拟本地已知的授权截止，不依赖 wall-clock sleep 的竞争窗口。
    reader_node._heartbeat_task.abort();
    reader_node
        .node
        .metadata_lease(Some(Instant::now() - Duration::from_secs(1)), None)
        .await
        .unwrap();
    meta.reset_resolve_count();
    writer_node.reset_peer_pull_count();
    let result = reader_node.read_inline(reader, key).await;
    assert_eq!(
        result.inline_value.as_deref(),
        Some(b"local-after-first-read".as_slice())
    );
    assert_eq!(meta.resolve_count(), 1, "本地 bytes 不能替代版本授权");
    assert_eq!(
        writer_node.peer_pull_count(),
        0,
        "只重新解析，不重复拉取已有 bytes"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mset_invalidates_cached_local_current_key() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let writer = node.open_write_session().await;
    let key = b"node-cache/mset-a";
    let other_key = b"node-cache/mset-b";
    let v1 = node.set_inline(writer, key, b"old-a", 13).await;
    let (reader, mut reader_events) = node.open_cached_session().await;
    let first = node.read_inline(reader, key).await;
    assert_eq!(first.version, v1);
    assert_eq!(first.inline_value.as_deref(), Some(b"old-a".as_slice()));

    let write = tokio::spawn(mset_with_node(
        node.node.clone(),
        writer,
        vec![
            (key.to_vec(), b"new-a".to_vec()),
            (other_key.to_vec(), b"new-b".to_vec()),
        ],
        14,
    ));
    acknowledge_next_invalidation(&node.node, reader, &mut reader_events, key, v1 + 1).await;
    let versions = write.await.expect("join mset");
    assert_eq!(versions.len(), 2);
    let v2 = versions[0].version;
    assert!(v2 > v1);
    // 另一个 key 是本批新建对象，不需要失效已有 Current cache，也不会等待前台 ACK。

    meta.reset_resolve_count();
    let second = node.read_inline(reader, key).await;
    assert_eq!(second.version, v2);
    assert_eq!(second.inline_value.as_deref(), Some(b"new-a".as_slice()));
    let third = node.read_inline(reader, key).await;
    assert_eq!(third.version, v2);
    assert_eq!(third.inline_value.as_deref(), Some(b"new-a".as_slice()));
    assert_eq!(
        meta.resolve_count(),
        1,
        "MSET 改到已缓存 key 时必须失效；新 Current 第一次读后才能再次进入 Node cache"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_invalidates_cached_current_and_following_get_observes_not_found() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let (reader, mut reader_events) = node.open_cached_session().await;
    let writer = node.open_write_session().await;
    let key = b"node-cache/delete";
    let v1 = node.set_inline(writer, key, b"live", 20).await;
    let first = node.read_inline(reader, key).await;
    assert_eq!(first.inline_value.as_deref(), Some(b"live".as_slice()));

    let delete_node = node.node.clone();
    let delete_key = key.to_vec();
    let delete = tokio::spawn(async move {
        delete_node
            .delete(writer, delete_key, operation_id(0x53, 21))
            .await
    });
    acknowledge_next_invalidation(&node.node, reader, &mut reader_events, key, v1 + 1).await;
    let deleted = delete.await.expect("join delete").expect("delete");
    assert!(deleted.deleted);
    assert!(deleted.version > v1);

    let after_delete = node
        .node
        .get_with_inline_limit(reader, key.to_vec(), None, None, 64 * 1024)
        .await;
    assert!(
        after_delete.as_ref().is_err_and(is_not_found),
        "DELETE 后不能从 Node Current cache 读到旧值: {after_delete:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_remote_resolve_cannot_refill_stale_current_cache() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let (reader, mut reader_events) = reader_node.open_cached_session().await;
    let writer = writer_node.open_write_session().await;
    let key = b"node-cache/two-node-stale-resolve";

    let v1 = writer_node.set_inline(writer, key, b"remote-v1", 30).await;
    let delay = meta.delay_next_resolve_for_key(key);
    let read_node = reader_node.node.clone();
    let read_key = key.to_vec();
    let stale_read_started = Instant::now();
    let stale_read = tokio::spawn(async move {
        read_node
            .get_with_inline_limit(reader, read_key, None, None, 64 * 1024)
            .await
    });
    delay
        .captured
        .await
        .expect("reader resolve should be paused after v1 resolution");

    let v2_writer = writer_node.node.clone();
    let write_key = key.to_vec();
    let write_v2 = tokio::spawn(async move {
        v2_writer
            .set_inline(
                writer,
                write_key,
                b"remote-v2".to_vec(),
                operation_id(0x54, 31),
                "any".to_string(),
            )
            .await
    });
    acknowledge_next_invalidation(&reader_node.node, reader, &mut reader_events, key, v1 + 1).await;
    let v2 = write_v2
        .await
        .expect("join remote v2 write")
        .expect("remote v2 write")
        .version;
    assert!(v2 > v1);
    meta.wait_for_invalidation_ack(1, key, v2).await;

    assert!(
        stale_read_started.elapsed() < Duration::from_secs(10),
        "迟到 resolve 必须在 30s cache lease TTL 内释放，避免测试靠租约自然过期误过"
    );
    delay.release.notify_waiters();
    let stale = stale_read
        .await
        .expect("join stale resolve read")
        .expect("stale resolve read may still return the older linearization point");
    assert_eq!(stale.version, v1);
    assert_eq!(stale.inline_value.as_deref(), Some(b"remote-v1".as_slice()));

    meta.reset_resolve_count();
    let fresh = reader_node.read_inline(reader, key).await;
    assert_eq!(fresh.version, v2);
    assert_eq!(fresh.inline_value.as_deref(), Some(b"remote-v2".as_slice()));
    let cached = reader_node.read_inline(reader, key).await;
    assert_eq!(cached.version, v2);
    assert_eq!(
        cached.inline_value.as_deref(),
        Some(b"remote-v2".as_slice())
    );
    assert_eq!(
        meta.resolve_count(),
        1,
        "迟到 v1 resolve 的 cache_refill token 必须被失效栅栏拒绝，后续读重新 resolve v2 后才能进 cache"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_watch_reconnect_restores_current_cache_after_stream_drop() {
    let meta = CountingMetaServer::start().await;
    let reader_node = TestNode::start_with_peer_server(&meta.endpoint, 1).await;
    let writer_node = TestNode::start_with_peer_server(&meta.endpoint, 2).await;
    let writer = writer_node.open_write_session().await;
    let key = b"node-cache/watch-reconnect";
    let v1 = writer_node
        .set_inline(writer, key, b"before-drop", 40)
        .await;
    let (reader, mut reader_events) = reader_node.open_cached_session().await;
    let first = reader_node.read_inline(reader, key).await;
    assert_eq!(
        first.inline_value.as_deref(),
        Some(b"before-drop".as_slice())
    );

    let first_watch_generation = meta.watch_count();
    meta.set_watch_rejected(true);
    let watch_dropped = meta.drop_current_watch();
    tokio::time::timeout(Duration::from_secs(3), watch_dropped)
        .await
        .expect("watch stream should be forced to close")
        .expect("watch drop notification");
    wait_for_watch_count(&meta, first_watch_generation + 1).await;

    let uncached_reader = open_session_without_cache_lease(&reader_node.node).await;
    meta.reset_resolve_count();
    let uncached_a = reader_node.read_inline(uncached_reader, key).await;
    assert_eq!(uncached_a.version, v1);
    assert_eq!(
        uncached_a.inline_value.as_deref(),
        Some(b"before-drop".as_slice())
    );
    let uncached_b = reader_node.read_inline(uncached_reader, key).await;
    assert_eq!(uncached_b.version, v1);
    assert_eq!(
        uncached_b.inline_value.as_deref(),
        Some(b"before-drop".as_slice())
    );
    assert_eq!(
        meta.resolve_count(),
        2,
        "Meta watch 断开且重连被拒绝时，Node 不应继续回填 Current cache"
    );
    reader_node
        .node
        .close_session(uncached_reader)
        .await
        .expect("close uncached session");

    meta.set_watch_rejected(false);
    wait_for_watch_count(&meta, first_watch_generation + 2).await;
    let write_node = writer_node.node.clone();
    let write_key = key.to_vec();
    let write_v2 = tokio::spawn(async move {
        write_node
            .set_inline(
                writer,
                write_key,
                b"after-reconnect".to_vec(),
                operation_id(0x55, 41),
                "any".to_string(),
            )
            .await
    });
    acknowledge_next_invalidation(&reader_node.node, reader, &mut reader_events, key, v1 + 1).await;
    let v2 = write_v2
        .await
        .expect("join v2 write after watch reconnect")
        .expect("v2 write after watch reconnect")
        .version;
    assert!(v2 > v1);
    meta.wait_for_invalidation_ack(1, key, v2).await;

    meta.reset_resolve_count();
    let fresh = reader_node.read_inline(reader, key).await;
    assert_eq!(fresh.version, v2);
    assert_eq!(
        fresh.inline_value.as_deref(),
        Some(b"after-reconnect".as_slice())
    );
    let cached = reader_node.read_inline(reader, key).await;
    assert_eq!(cached.version, v2);
    assert_eq!(
        cached.inline_value.as_deref(),
        Some(b"after-reconnect".as_slice())
    );
    assert_eq!(
        meta.resolve_count(),
        1,
        "watch 断开后必须先关闭新租约；重连并处理 replay event 后，新 Current 才能再次进入 Node cache"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_service_handler_reuses_node_current_layout_cache() {
    let meta = CountingMetaServer::start().await;
    let node = TestNode::start(&meta.endpoint, 1).await;
    let handler = WorkerServiceHandler::new(node.node.clone(), false);

    let open = <WorkerServiceHandler as pb::worker_service_server::WorkerService>::open_session(
        &handler,
        Request::new(pb::OpenSessionRequest {
            min_version: 1,
            max_version: 1,
            shared_memory: false,
            zero_copy_read: false,
            zero_copy_write: false,
            supports_write_lease_release: false,
        }),
    )
    .await
    .expect("open session")
    .into_inner();
    let (events_tx, _events_rx) = mpsc::channel(16);
    node.node
        .attach_session(open.session_id, events_tx)
        .await
        .expect("attach session");
    wait_for_cache_lease(&node.node, open.session_id).await;

    let key = pb::Key {
        value: b"node-cache/worker-service".to_vec(),
    };
    <WorkerServiceHandler as pb::worker_service_server::WorkerService>::set_inline(
        &handler,
        Request::new(pb::SetInlineRequest {
            session_id: open.session_id,
            key: Some(key.clone()),
            value: b"from-worker".to_vec(),
            operation_id: Some(pb::OperationId {
                client_instance_id: vec![0x54; 16],
                sequence: 30,
            }),
            condition: "any".to_string(),
            durability: "local-memory".to_string(),
        }),
    )
    .await
    .expect("set inline");

    meta.reset_resolve_count();
    for _ in 0..2 {
        let response = <WorkerServiceHandler as pb::worker_service_server::WorkerService>::get(
            &handler,
            Request::new(pb::GetRequest {
                session_id: open.session_id,
                key: Some(key.clone()),
                exact_version: None,
                range: None,
                clamp_range: false,
                max_inline_bytes: 64 * 1024,
                read_request_id: 0,
            }),
        )
        .await
        .expect("get")
        .into_inner();
        assert_eq!(
            response.inline_value.as_deref(),
            Some(b"from-worker".as_slice())
        );
    }
    assert_eq!(
        meta.resolve_count(),
        0,
        "WorkerService 写成功后应回填 Node Current layout cache"
    );
}

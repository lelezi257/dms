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

use dms_error::ErrorKind;
use dms_protocol::v1 as pb;
use pb::{
    metadata_service_server::{MetadataService, MetadataServiceServer},
    peer_service_server::PeerServiceServer,
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
    metadata_client::MetadataClient,
    peer_service::PeerServiceHandler,
    runtime::{NodeEvent, NodeHandle, ReadTicket, SetRangeInput, WorkerError},
    send_meta_heartbeats,
    worker_service::WorkerServiceHandler,
};
use crate::meta::{metadata_service::MetadataServiceHandler, runtime::MetaHandle};

#[derive(Clone)]
struct CountingMetaService {
    inner: MetadataServiceHandler,
    resolve_count: Arc<AtomicUsize>,
    resolve_delay: Arc<Mutex<Option<ResolveDelay>>>,
    watch_count: Arc<AtomicUsize>,
    reject_watch: Arc<AtomicBool>,
    watch_drop: Arc<WatchDropControl>,
    ack_state: Arc<AckState>,
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
        let response = self.inner.resolve_object(request).await;
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

    async fn report_replicas(
        &self,
        request: Request<pb::ReportReplicasRequest>,
    ) -> Result<Response<pb::ReportReplicasResponse>, Status> {
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
}

struct CountingMetaServer {
    endpoint: String,
    resolve_count: Arc<AtomicUsize>,
    resolve_delay: Arc<Mutex<Option<ResolveDelay>>>,
    watch_count: Arc<AtomicUsize>,
    reject_watch: Arc<AtomicBool>,
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
        let resolve_delay = Arc::new(Mutex::new(None));
        let watch_count = Arc::new(AtomicUsize::new(0));
        let reject_watch = Arc::new(AtomicBool::new(false));
        let watch_drop = Arc::new(WatchDropControl {
            requested: AtomicBool::new(false),
            captured: Arc::new(Mutex::new(None)),
            waker: Mutex::new(None),
        });
        let ack_state = Arc::new(AckState::new());
        let service = CountingMetaService {
            inner: MetadataServiceHandler::new(MetaHandle::spawn()),
            resolve_count: resolve_count.clone(),
            resolve_delay: resolve_delay.clone(),
            watch_count: watch_count.clone(),
            reject_watch: reject_watch.clone(),
            watch_drop: watch_drop.clone(),
            ack_state: ack_state.clone(),
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
            resolve_count,
            resolve_delay,
            watch_count,
            reject_watch,
            watch_drop,
            ack_state,
            shutdown: Some(shutdown_tx),
            task,
        }
    }

    fn reset_resolve_count(&self) {
        self.resolve_count.store(0, Ordering::Relaxed);
    }

    fn resolve_count(&self) -> usize {
        self.resolve_count.load(Ordering::Relaxed)
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
            Some(initial_watch),
            acked_cursor.clone(),
        ));
        let heartbeat_task =
            tokio::spawn(send_meta_heartbeats(metadata, node.clone(), acked_cursor));
        let peer_task = peer_listener.map(|(_, listener)| {
            let handler = PeerServiceHandler::new(node.clone());
            tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(PeerServiceServer::new(handler))
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
    assert_eq!(minimum_version, expected_minimum_version);
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
async fn current_get_resolves_meta_once_then_reuses_node_layout_cache() {
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
        1,
        "首次 Current GET 可读取 Meta，第二次同 key 应命中 Node Current layout cache"
    );
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
    assert_eq!(v2, v1 + 1);
    meta.wait_for_invalidation_ack(1, key, v2).await;

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
        1,
        "Current range read 仍可复用同一份 Current layout cache"
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
    assert_eq!(versions[0].version, v1 + 1);
    meta.wait_for_invalidation_ack(1, key, v1 + 1).await;
    // 另一个 key 是本批新建对象，不需要失效已有 Current cache，也不会等待前台 ACK。

    meta.reset_resolve_count();
    let second = node.read_inline(reader, key).await;
    assert_eq!(second.version, v1 + 1);
    assert_eq!(second.inline_value.as_deref(), Some(b"new-a".as_slice()));
    let third = node.read_inline(reader, key).await;
    assert_eq!(third.version, v1 + 1);
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
    meta.wait_for_invalidation_ack(1, key, v1 + 1).await;

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
    assert_eq!(v2, v1 + 1);
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
    assert_eq!(v2, v1 + 1);
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
async fn worker_service_handler_keeps_the_node_cache_below_sdk_cache() {
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
                max_inline_bytes: 64 * 1024,
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
    assert_eq!(meta.resolve_count(), 1);
}

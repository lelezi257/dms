//! dms-node 到单一 dms-meta 的 gRPC 客户端。
//!
//! 这里负责 protobuf DTO 和连接细节；Node actor 只调用 `resolve/commit/watch/ack`
//! 这些当前真实业务动作。`Channel` 可安全 clone，并共享底层 HTTP/2 连接。

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use dms_error::{DmsError, ErrorKind};
use dms_protocol::v1 as pb;
use dms_transport::{GrpcConfig, SecurityManager, TlsConfig, status_to_dms_error_with};
use pb::filesystem_metadata_service_client::FilesystemMetadataServiceClient as GrpcFilesystemMetadataClient;
use pb::metadata_service_client::MetadataServiceClient as GrpcMetadataClient;
use tokio::sync::{Mutex, MutexGuard, RwLock, mpsc, oneshot};
use tonic::{Response, Status, transport::Endpoint};

use crate::filesystem::{
    FileLockMode, FileLockOwner, FileLockRange, GrantedFileLock, granted_lock_to_proto,
};

const METADATA_RESOLVE_BATCH_MAX: usize = 64;
const METADATA_COMMIT_MAX_ATTEMPTS: usize = 3;
// 普通控制请求需要尽快发现 Meta 断线，尤其是 Watch ACK：若 ACK 长时间挂起，
// Node 就无法及时重建 Watch 并从已确认 cursor 继续补发事件。
const METADATA_CONTROL_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
// Meta 重启后必须等恢复前发出的最长 Node lease 自然到期，才能确认旧缓存不再
// 可见。Meta 的首版 lease 是 30 秒，因此 Commit 不能沿用普通控制请求的 5 秒
// 截止时间；否则安全屏障尚未满足，提交者会先把一次仍可能成功的写判成超时。
// 这里多留 5 秒给调度与响应传输，成功路径不会因此增加延迟。
const METADATA_COMMIT_REQUEST_TIMEOUT: Duration = Duration::from_secs(35);

struct ResolveJob {
    key: Vec<u8>,
    exact_version: Option<u64>,
    reply: oneshot::Sender<Result<pb::ResolveObjectResponse, DmsError>>,
}

/// 当前 Node 已被 Meta 接纳的文件锁镜像。只保存重报需要的领域字段；Node epoch
/// 始终取重连后的当前 session，避免把旧 incarnation 身份重放给 Meta。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LocalHeldFileLock {
    inode: u64,
    lock_owner: u64,
    start: u64,
    end_inclusive: u64,
    mode: i32,
    pid: u32,
}

#[derive(Default)]
struct LocalFilesystemLockMirror {
    locks: Vec<LocalHeldFileLock>,
    release_fences: HashMap<u64, u64>,
    inflight_sets: HashMap<u64, HashSet<u64>>,
    owner_revisions: HashMap<u64, u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalReleaseMirrorUpdate {
    Applied,
    Obsolete,
    SessionStale,
}

impl LocalHeldFileLock {
    fn to_reclaim_proto(
        self,
        session: &pb::NodeSessionIdentity,
    ) -> Result<pb::FilesystemGrantedLock, DmsError> {
        let range = FileLockRange::new(self.start, self.end_inclusive).map_err(|error| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::Internal,
                format!("invalid local filesystem lock mirror range: {error:?}"),
            )
        })?;
        let mode = FileLockMode::from_proto(self.mode).map_err(|error| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::Internal,
                format!("invalid local filesystem lock mirror mode: {error:?}"),
            )
        })?;
        Ok(granted_lock_to_proto(
            self.inode,
            GrantedFileLock {
                owner: FileLockOwner {
                    node_id: session.node_id,
                    node_epoch: session.node_epoch,
                    lock_owner: self.lock_owner,
                },
                range,
                mode,
                pid: self.pid,
            },
        ))
    }
}

/// 解析批处理任务只持有发 RPC 所需的共享状态，不持有发送端，避免后台任务和
/// 自己消费的 channel 形成生命周期环。它不是新的业务模块。
struct ResolveBatchRpc {
    client: GrpcMetadataClient<dms_tracing::TracedChannel>,
    filesystem_client: GrpcFilesystemMetadataClient<dms_tracing::TracedChannel>,
    session_state: Arc<RwLock<SessionState>>,
    session_reopen_gate: Arc<Mutex<()>>,
    next_commit_sequence: Arc<AtomicU64>,
    node_id: u64,
    data_endpoint: String,
    rpc_metrics: Option<dms_metrics::RpcMetrics>,
    filesystem_locks: Arc<Mutex<LocalFilesystemLockMirror>>,
    // 只有启用了原生 filesystem/FUSE 的 Node 才依赖 FilesystemMetadataService。
    // 纯 KV Node 必须保持原来的 MetadataService 合同，不能因为文件锁恢复在连接
    // 阶段额外调用一个并不存在的 gRPC Service。
    filesystem_enabled: bool,
}

pub(crate) struct BatchValueCommit {
    pub(crate) key: Vec<u8>,
    pub(crate) block_id: Vec<u8>,
    pub(crate) length: u64,
    pub(crate) checksum: Vec<u8>,
    pub(crate) operation_id: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionSnapshot {
    identity: pb::NodeSessionIdentity,
    generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionState {
    identity: pb::NodeSessionIdentity,
    generation: u64,
}

impl SessionState {
    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            identity: self.identity.clone(),
            generation: self.generation,
        }
    }
}

#[derive(Clone)]
pub(crate) struct MetadataClient {
    client: GrpcMetadataClient<dms_tracing::TracedChannel>,
    filesystem_client: GrpcFilesystemMetadataClient<dms_tracing::TracedChannel>,
    // identity 与 generation 必须来自同一个线性一致快照。reopen/reclaim 一旦
    // 确认开始就先推进 generation，再异步执行 open/reclaim/publish；这样旧 RPC
    // 的成功响应即使在新 session 发布前返回，也无法写入本地 mirror。
    session_state: Arc<RwLock<SessionState>>,
    // 每个逻辑提交仅分配一次，克隆连接与 RPC 重试共享计数器。它不是用户
    // operation_id 的替代；仅让 Meta 在历史幂等结果裁剪后拒绝旧 wire 请求。
    next_commit_sequence: Arc<AtomicU64>,
    // 文件锁 mutation sequence 属于 Node session，而不是某个 FUSE mount 或
    // SharedFileOperations 实例。Meta 的 floor/retry 合同要求同一个 node_epoch
    // 内全局严格单调；因此所有本地文件入口都必须共享这一份计数器。
    next_filesystem_lock_mutation_sequence: Arc<AtomicU64>,
    // Meta 使用每个 Node 单调递增的 commit_sequence 拒绝旧请求。HTTP/2 允许
    // 并发 RPC 乱序抵达，因此必须在 Node 出口保持“分配序号 + 完成提交”的顺序；
    // 否则较大的序号先提交后，仍在途的较小序号会被误判为重放。锁只覆盖
    // commit_version/commit_batch，不串行化 resolve、watch、heartbeat 或数据面。
    commit_gate: Arc<Mutex<()>>,
    // 所有会替换 Meta session 的路径共享同一把锁。恢复事务必须完整执行
    // open -> reclaim 文件锁快照 -> publish session；否则并发重连或后台
    // Resolve 重连可能只发布新 epoch，导致旧 epoch 下的锁无法再正常 unlock。
    session_reopen_gate: Arc<Mutex<()>>,
    node_id: u64,
    data_endpoint: String,
    rpc_metrics: Option<dms_metrics::RpcMetrics>,
    resolve_tx: mpsc::Sender<ResolveJob>,
    filesystem_locks: Arc<Mutex<LocalFilesystemLockMirror>>,
    // 只有启用了原生 filesystem/FUSE 的 Node 才依赖 FilesystemMetadataService。
    // 纯 KV Node 必须保持原来的 MetadataService 合同，不能因为文件锁恢复在连接
    // 阶段额外调用一个并不存在的 gRPC Service。
    filesystem_enabled: bool,
}

struct FilesystemLockSetInflightGuard {
    client: Option<MetadataClient>,
    lock_owner: u64,
    request_id: u64,
}

impl FilesystemLockSetInflightGuard {
    async fn finish(mut self) {
        if let Some(client) = self.client.take() {
            client
                .finish_filesystem_lock_set_inflight(self.lock_owner, self.request_id)
                .await;
        }
    }
}

impl Drop for FilesystemLockSetInflightGuard {
    fn drop(&mut self) {
        let Some(client) = self.client.take() else {
            return;
        };
        let lock_owner = self.lock_owner;
        let request_id = self.request_id;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                client
                    .finish_filesystem_lock_set_inflight(lock_owner, request_id)
                    .await;
            });
        }
    }
}

/// 当前 Node 在 Meta 中登记的副本身份。
///
/// 这是 Node 内部把“刚提交成功的本地 Block”写入 Current 布局缓存时所需的
/// 最小信息，不是新的协议或公开抽象。Node session 重建后 epoch 会改变，因此
/// 必须从当前 session 读取，不能只保存启动时的值。
pub(crate) struct LocalReplicaIdentity {
    pub(crate) node_id: u64,
    pub(crate) node_epoch: u64,
    pub(crate) data_endpoint: String,
}

#[derive(Clone, Copy)]
enum MetadataRpcDeadline {
    /// 普通控制面 RPC 需要快速发现 Meta 断线。
    Control,
    /// 等待型文件锁 RPC 的等待时间由内核 cancel 决定，不套 5 秒控制面超时。
    Operation,
}

impl MetadataClient {
    pub(crate) fn node_id(&self) -> u64 {
        self.node_id
    }
    /// 建立到 Meta 的 HTTP/2 连接，并立即注册当前 Node incarnation。
    pub(crate) async fn connect(
        endpoint: &str,
        node_id: u64,
        data_endpoint: String,
        rpc_metrics: Option<dms_metrics::RpcMetrics>,
        filesystem_enabled: bool,
    ) -> Result<Self, DmsError> {
        let endpoint = Endpoint::from_shared(endpoint.to_string()).map_err(|error| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::InvalidArgument,
                format!("invalid Meta endpoint: {error}"),
            )
        })?;
        let grpc_config = GrpcConfig {
            // Channel 的外层 timeout 取最长业务请求预算；普通控制 RPC 再由
            // `observe_control_rpc` 收紧到 5 秒。这样所有 generated clients 仍
            // 共享同一条 HTTP/2 连接，不为两类超时复制 Channel/连接池。
            request_timeout: METADATA_COMMIT_REQUEST_TIMEOUT,
            ..GrpcConfig::default()
        };
        let endpoint = grpc_config.configure_client(endpoint);
        let security = SecurityManager::new(TlsConfig::Disabled).map_err(|error| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::Unavailable,
                format!("invalid Meta transport security config: {error}"),
            )
        })?;
        let endpoint = security.configure_client(endpoint).map_err(|error| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::Unavailable,
                format!("failed to configure Meta transport: {error}"),
            )
        })?;
        let channel = endpoint.connect().await.map_err(|error| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::Unavailable,
                format!("failed to connect Meta endpoint: {error}"),
            )
        })?;
        let mut client = metadata_client(channel.clone());
        let (session, minimum_sequence) =
            Self::open_session(&mut client, node_id, &data_endpoint, rpc_metrics.as_ref()).await?;
        let client = metadata_client(channel.clone());
        let filesystem_client = filesystem_metadata_client(channel);
        let session_state = Arc::new(RwLock::new(SessionState {
            identity: session,
            generation: 1,
        }));
        let session_reopen_gate = Arc::new(Mutex::new(()));
        let next_commit_sequence = Arc::new(AtomicU64::new(minimum_sequence.max(1)));
        let next_filesystem_lock_mutation_sequence = Arc::new(AtomicU64::new(1));
        let filesystem_locks = Arc::new(Mutex::new(LocalFilesystemLockMirror::default()));
        let (resolve_tx, resolve_rx) = mpsc::channel(METADATA_RESOLVE_BATCH_MAX * 4);
        tokio::spawn(run_resolve_batcher(
            ResolveBatchRpc {
                client: client.clone(),
                filesystem_client: filesystem_client.clone(),
                session_state: Arc::clone(&session_state),
                session_reopen_gate: Arc::clone(&session_reopen_gate),
                next_commit_sequence: Arc::clone(&next_commit_sequence),
                node_id,
                data_endpoint: data_endpoint.clone(),
                rpc_metrics: rpc_metrics.clone(),
                filesystem_locks: Arc::clone(&filesystem_locks),
                filesystem_enabled,
            },
            resolve_rx,
        ));
        let result = Self {
            client,
            filesystem_client,
            session_state,
            next_commit_sequence,
            next_filesystem_lock_mutation_sequence,
            commit_gate: Arc::new(Mutex::new(())),
            session_reopen_gate,
            node_id,
            data_endpoint,
            rpc_metrics,
            resolve_tx,
            filesystem_locks,
            filesystem_enabled,
        };
        // OpenNodeSession 对同 node_id 的旧 incarnation 建立锁恢复屏障。新进程的
        // 本地镜像为空也必须显式重报这个完整空快照，才能立即释放旧锁并解除屏障。
        if filesystem_enabled {
            let opened_session = result.current_session().await;
            result
                .reclaim_filesystem_locks_for_session(&opened_session)
                .await?;
        }
        Ok(result)
    }

    async fn open_session(
        client: &mut GrpcMetadataClient<dms_tracing::TracedChannel>,
        node_id: u64,
        data_endpoint: &str,
        rpc_metrics: Option<&dms_metrics::RpcMetrics>,
    ) -> Result<(pb::NodeSessionIdentity, u64), DmsError> {
        let result = observe_control_rpc(
            rpc_metrics,
            dms_metrics::RpcCall::META_OPEN_NODE_SESSION,
            client.open_node_session(pb::OpenNodeSessionRequest {
                supports_commit_sequence: true,
                context: Some(context(node_id)),
                registration: Some(pb::NodeRegistration {
                    node_id,
                    control_endpoint: data_endpoint.to_string(),
                    transport_capabilities: vec!["grpc".to_string()],
                    total_host_memory_bytes: 0,
                    failure_domain: "local".to_string(),
                }),
            }),
        )
        .await;
        let opened = result.map_err(map_status)?.into_inner();
        let identity = opened.session.ok_or_else(|| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::Unavailable,
                "Meta OpenNodeSession response is missing session identity",
            )
        })?;
        Ok((identity, opened.minimum_commit_sequence))
    }

    async fn current_session(&self) -> pb::NodeSessionIdentity {
        self.session_state.read().await.identity.clone()
    }

    pub(crate) fn allocate_filesystem_lock_mutation_sequence(&self) -> Result<u64, DmsError> {
        self.next_filesystem_lock_mutation_sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1).filter(|next| *next != 0)
            })
            .map_err(|_| {
                DmsError::new(
                    dms_error::NODE_METADATA_UNAVAILABLE,
                    ErrorKind::ResourceExhausted,
                    "filesystem lock mutation sequence exhausted; refusing sequence reuse",
                )
            })
    }

    async fn current_session_snapshot(&self) -> SessionSnapshot {
        self.session_state.read().await.snapshot()
    }

    /// Current Node incarnation assigned by Meta. This is the sole fencing
    /// identity for in-memory replicas; there is no duplicate storage epoch.
    pub(crate) async fn node_epoch(&self) -> u64 {
        self.current_session().await.node_epoch
    }

    pub(crate) async fn local_replica_identity(&self) -> LocalReplicaIdentity {
        let session = self.current_session().await;
        LocalReplicaIdentity {
            node_id: session.node_id,
            node_epoch: session.node_epoch,
            data_endpoint: self.data_endpoint.clone(),
        }
    }

    /// 返回本次续租的有效毫秒数；调用方以发请求前的 Instant 计算保守截止时间，
    /// 不能以收到响应的时间再加 TTL，否则网络延迟会让 Node 比 Meta 更晚过期。
    pub(crate) async fn heartbeat(
        &self,
        event_cursor: u64,
        resources: pb::ResourceSummary,
    ) -> Result<u64, DmsError> {
        let mut snapshot = self.current_session_snapshot().await;
        let request = pb::NodeHeartbeatRequest {
            context: Some(context(self.node_id)),
            session: Some(snapshot.identity.clone()),
            resources: Some(resources),
            catalog_watermark: 0,
            event_cursor,
        };
        let mut client = self.client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_HEARTBEAT,
            client.heartbeat(request.clone()),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            snapshot = self.reopen_session_if_current(&snapshot).await?;
            let mut client = self.client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_HEARTBEAT,
                client.heartbeat(pb::NodeHeartbeatRequest {
                    session: Some(snapshot.identity.clone()),
                    ..request
                }),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        let response = result?;
        if self.filesystem_enabled && response.filesystem_lock_reclaim_required {
            // Meta 的 session 可以跨重启继续使用，但易失锁表不能。reclaim 同样
            // 推进本地 generation：在完整快照被 Meta 接收前，迟到的旧 SetLock
            // 成功响应不能污染 mirror。
            self.reclaim_filesystem_locks_if_current(&snapshot).await?;
        }
        Ok(response.lease_ttl_millis)
    }

    async fn reopen_session(&self) -> Result<pb::NodeSessionIdentity, DmsError> {
        let observed = self.current_session_snapshot().await;
        self.reopen_session_if_current(&observed)
            .await
            .map(|snapshot| snapshot.identity)
    }

    async fn reopen_session_if_current(
        &self,
        observed: &SessionSnapshot,
    ) -> Result<SessionSnapshot, DmsError> {
        let _reopen_guard = self.session_reopen_gate.lock().await;
        let next_generation = {
            let mut state = self.session_state.write().await;
            let current = state.snapshot();
            if current != *observed {
                return Ok(current);
            }
            state.generation += 1;
            state.generation
        };
        let mut client = self.client();
        let (session, minimum_sequence) = Self::open_session(
            &mut client,
            self.node_id,
            &self.data_endpoint,
            self.rpc_metrics.as_ref(),
        )
        .await?;
        // 不重置计数器；并发克隆也不能把已发出的 sequence 再次分配出去。
        self.next_commit_sequence
            .fetch_max(minimum_sequence.max(1), Ordering::Relaxed);
        dms_logging::info!(
            "Meta session reopened";
            "event" => "node.meta_session.reopened",
            "node_id" => self.node_id,
            "node_epoch" => session.node_epoch,
        );
        // 先用新 session 把完整本地锁镜像交给 Meta，再向其它并发调用发布它。
        // 如果重报失败，后续心跳仍持有旧 session 并会再次进入 reopen，而不会让
        // 一个“有效但尚未解除锁恢复屏障”的 session 永久留在客户端。
        if self.filesystem_enabled {
            self.reclaim_filesystem_locks_for_session(&session).await?;
        }
        *self.session_state.write().await = SessionState {
            identity: session.clone(),
            generation: next_generation,
        };
        Ok(SessionSnapshot {
            identity: session,
            generation: next_generation,
        })
    }

    async fn reclaim_filesystem_locks_if_current(
        &self,
        observed: &SessionSnapshot,
    ) -> Result<SessionSnapshot, DmsError> {
        let _reopen_guard = self.session_reopen_gate.lock().await;
        let fenced = {
            let mut state = self.session_state.write().await;
            let current = state.snapshot();
            if current != *observed {
                return Ok(current);
            }
            state.generation += 1;
            state.snapshot()
        };
        self.reclaim_filesystem_locks_for_session(&fenced.identity)
            .await?;
        Ok(fenced)
    }

    async fn reclaim_filesystem_locks_for_session(
        &self,
        session: &pb::NodeSessionIdentity,
    ) -> Result<(), DmsError> {
        reclaim_filesystem_locks_snapshot(
            self.filesystem_client(),
            &self.filesystem_locks,
            session,
            self.node_id,
            self.rpc_metrics.as_ref(),
        )
        .await
    }

    pub(crate) async fn resolve(
        &self,
        key: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<pb::ResolveObjectResponse, DmsError> {
        let (reply, receive) = oneshot::channel();
        self.resolve_tx
            .send(ResolveJob {
                key,
                exact_version,
                reply,
            })
            .await
            .map_err(|_| metadata_unavailable("Meta resolve batcher stopped"))?;
        receive
            .await
            .map_err(|_| metadata_unavailable("Meta resolve batcher dropped its reply"))?
    }

    pub(crate) async fn filesystem_lookup(
        &self,
        parent: u64,
        name: Vec<u8>,
        reference_generation: u64,
    ) -> Result<pb::FilesystemResolveResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::FilesystemLookupRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
            parent,
            name: name.clone(),
            reference_generation,
        };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_LOOKUP,
            client.lookup_filesystem_entry(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_LOOKUP,
                client.lookup_filesystem_entry(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_get_inode(
        &self,
        inode: u64,
    ) -> Result<pb::FilesystemResolveResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::FilesystemGetInodeRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
            inode,
        };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_GET_INODE,
            client.get_filesystem_inode(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_GET_INODE,
                client.get_filesystem_inode(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_create_inode(
        &self,
        mut request: pb::FilesystemCreateInodeRequest,
    ) -> Result<pb::FilesystemResolveResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_CREATE_INODE,
                client.create_filesystem_inode(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem create retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_create_symlink(
        &self,
        mut request: pb::FilesystemCreateSymlinkRequest,
    ) -> Result<pb::FilesystemResolveResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_CREATE_SYMLINK,
                client.create_filesystem_symlink(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem symlink create retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_read_directory(
        &self,
        directory: u64,
        cursor: Vec<u8>,
        limit: u32,
        expected_directory_revision: Option<u64>,
    ) -> Result<pb::FilesystemReadDirectoryResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::FilesystemReadDirectoryRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
            directory,
            cursor: cursor.clone(),
            limit,
            expected_directory_revision,
        };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_READ_DIRECTORY,
            client.read_filesystem_directory(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_READ_DIRECTORY,
                client.read_filesystem_directory(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_link_entry(
        &self,
        mut request: pb::FilesystemLinkRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_LINK,
                client.link_filesystem_entry(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem link retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_acquire_inode_reference(
        &self,
        mut request: pb::FilesystemInodeReferenceRequest,
    ) -> Result<pb::FilesystemInodeReferenceResponse, DmsError> {
        request.context = Some(context(self.node_id));
        let mut session = self.current_session().await;
        let make_request = |mut request: pb::FilesystemInodeReferenceRequest,
                            session: pb::NodeSessionIdentity| {
            request.session = Some(session);
            request
        };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_ACQUIRE_INODE_REFERENCE,
            client
                .acquire_filesystem_inode_reference(make_request(request.clone(), session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_ACQUIRE_INODE_REFERENCE,
                client.acquire_filesystem_inode_reference(make_request(request, session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_release_inode_reference(
        &self,
        mut request: pb::FilesystemInodeReferenceRequest,
    ) -> Result<pb::FilesystemInodeReferenceResponse, DmsError> {
        request.context = Some(context(self.node_id));
        let mut session = self.current_session().await;
        let make_request = |mut request: pb::FilesystemInodeReferenceRequest,
                            session: pb::NodeSessionIdentity| {
            request.session = Some(session);
            request
        };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_RELEASE_INODE_REFERENCE,
            client
                .release_filesystem_inode_reference(make_request(request.clone(), session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_RELEASE_INODE_REFERENCE,
                client.release_filesystem_inode_reference(make_request(request, session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_renew_inode_references(
        &self,
        references: Vec<pb::FilesystemInodeReferenceLease>,
    ) -> Result<pb::FilesystemInodeReferenceResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request =
            |session: pb::NodeSessionIdentity| pb::FilesystemRenewInodeReferencesRequest {
                context: Some(context(self.node_id)),
                session: Some(session),
                references: references.clone(),
            };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_RENEW_INODE_REFERENCES,
            client.renew_filesystem_inode_references(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_RENEW_INODE_REFERENCES,
                client.renew_filesystem_inode_references(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_rename_entry(
        &self,
        mut request: pb::FilesystemRenameRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_RENAME,
                client.rename_filesystem_entry(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem rename retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_remove_entry(
        &self,
        mut request: pb::FilesystemRemoveRequest,
    ) -> Result<pb::FilesystemNamespaceMutationResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_REMOVE,
                client.remove_filesystem_entry(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem remove retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_commit_version(
        &self,
        mut request: pb::FilesystemCommitVersionRequest,
    ) -> Result<pb::FilesystemCommitVersionResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_COMMIT_VERSION,
                client.commit_filesystem_version(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem commit retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_set_attributes(
        &self,
        mut request: pb::FilesystemSetAttributesRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_SET_ATTRIBUTES,
                client.set_filesystem_attributes(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem attribute retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_get_xattr(
        &self,
        inode: u64,
        name: Vec<u8>,
        caller: pb::FilesystemCallerIdentity,
    ) -> Result<pb::FilesystemGetXattrResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::FilesystemGetXattrRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
            inode,
            name: name.clone(),
            caller: Some(caller),
        };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_GET_XATTR,
            client.get_filesystem_xattr(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_GET_XATTR,
                client.get_filesystem_xattr(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_list_xattrs(
        &self,
        inode: u64,
        caller: pb::FilesystemCallerIdentity,
    ) -> Result<pb::FilesystemListXattrsResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::FilesystemListXattrsRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
            inode,
            caller: Some(caller),
        };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_LIST_XATTRS,
            client.list_filesystem_xattrs(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_LIST_XATTRS,
                client.list_filesystem_xattrs(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_set_xattr(
        &self,
        mut request: pb::FilesystemSetXattrRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_SET_XATTR,
                client.set_filesystem_xattr(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem setxattr retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_remove_xattr(
        &self,
        mut request: pb::FilesystemRemoveXattrRequest,
    ) -> Result<pb::FilesystemAttributeMutationResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        request.context = Some(context(self.node_id));
        request.commit_sequence = commit_sequence;
        let mut session = self.current_session().await;
        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            request.session = Some(session.clone());
            let mut client = self.filesystem_client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_REMOVE_XATTR,
                client.remove_filesystem_xattr(request.clone()),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem removexattr retry loop exhausted",
        ))
    }

    pub(crate) async fn filesystem_stat(&self) -> Result<pb::FilesystemStatResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::FilesystemStatRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
        };
        let mut client = self.filesystem_client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::FILESYSTEM_STAT,
            client.stat_filesystem(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.filesystem_client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::FILESYSTEM_STAT,
                client.stat_filesystem(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn filesystem_test_lock(
        &self,
        request: pb::FilesystemLockRequest,
    ) -> Result<pb::FilesystemLockResponse, DmsError> {
        self.filesystem_rpc_with_session_reopen(
            dms_metrics::RpcCall::FILESYSTEM_TEST_LOCK,
            MetadataRpcDeadline::Control,
            |session| {
                let mut request = request.clone();
                request.context = Some(context(self.node_id));
                request.session = Some(session);
                request
            },
            |mut client, request| async move { client.test_filesystem_lock(request).await },
        )
        .await
    }

    pub(crate) async fn filesystem_set_lock(
        &self,
        request: pb::FilesystemLockRequest,
    ) -> Result<pb::FilesystemLockResponse, DmsError> {
        let mirror_request = request.clone();
        let deadline = if request.wait {
            MetadataRpcDeadline::Operation
        } else {
            MetadataRpcDeadline::Control
        };
        let inflight_guard = self
            .begin_filesystem_lock_set_inflight(request.lock_owner, request.request_id)
            .await;
        // SetLock 会在 Meta 上产生易失锁状态。若旧 epoch 的成功响应和本地 reopen
        // 竞态到达，不能把旧锁写入新 session mirror，也不能向 FUSE 返回“当前
        // incarnation 已持锁”的假成功；同一 request_id 必须在当前 session 下重试。
        let result = async {
            for _attempt in 0..METADATA_COMMIT_MAX_ATTEMPTS {
                let (used_session, response) = self
                    .filesystem_rpc_with_session_reopen_and_session(
                        dms_metrics::RpcCall::FILESYSTEM_SET_LOCK,
                        deadline,
                        |session| {
                            let mut request = request.clone();
                            request.context = Some(context(self.node_id));
                            request.session = Some(session);
                            request
                        },
                        |mut client, request| async move {
                            client.set_filesystem_lock(request).await
                        },
                    )
                    .await?;
                let status = filesystem_lock_status(&response)?;
                if !filesystem_lock_status_requires_mirror(status) {
                    return Ok(response);
                }
                if self
                    .update_filesystem_lock_mirror_if_session_current(
                        &mirror_request,
                        &response,
                        &used_session,
                    )
                    .await
                {
                    return Ok(response);
                }
            }
            Err(metadata_unavailable(
                "Meta filesystem lock session changed too often while retrying",
            ))
        }
        .await;
        inflight_guard.finish().await;
        result
    }

    async fn filesystem_rpc_with_session_reopen<Request, ResponseBody, BuildRequest, Send, Fut>(
        &self,
        call: dms_metrics::RpcCall,
        deadline: MetadataRpcDeadline,
        build_request: BuildRequest,
        send: Send,
    ) -> Result<ResponseBody, DmsError>
    where
        BuildRequest: FnMut(pb::NodeSessionIdentity) -> Request,
        Send: FnMut(GrpcFilesystemMetadataClient<dms_tracing::TracedChannel>, Request) -> Fut,
        Fut: Future<Output = Result<Response<ResponseBody>, Status>>,
    {
        self.filesystem_rpc_with_session_reopen_and_session(call, deadline, build_request, send)
            .await
            .map(|(_, response)| response)
    }

    async fn filesystem_rpc_with_session_reopen_and_session<
        Request,
        ResponseBody,
        BuildRequest,
        Send,
        Fut,
    >(
        &self,
        call: dms_metrics::RpcCall,
        deadline: MetadataRpcDeadline,
        mut build_request: BuildRequest,
        mut send: Send,
    ) -> Result<(SessionSnapshot, ResponseBody), DmsError>
    where
        BuildRequest: FnMut(pb::NodeSessionIdentity) -> Request,
        Send: FnMut(GrpcFilesystemMetadataClient<dms_tracing::TracedChannel>, Request) -> Fut,
        Fut: Future<Output = Result<Response<ResponseBody>, Status>>,
    {
        let mut session = self.current_session_snapshot().await;
        let mut result = self
            .filesystem_rpc_once(
                call,
                deadline,
                build_request(session.identity.clone()),
                &mut send,
            )
            .await;
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session_if_current(&session).await?;
            result = self
                .filesystem_rpc_once(
                    call,
                    deadline,
                    build_request(session.identity.clone()),
                    &mut send,
                )
                .await;
        }
        result.map(|response| (session, response))
    }

    pub(crate) async fn filesystem_cancel_lock_wait(
        &self,
        request_id: u64,
        lock_owner: u64,
    ) -> Result<u64, DmsError> {
        self.filesystem_rpc_with_session_reopen(
            dms_metrics::RpcCall::FILESYSTEM_CANCEL_LOCK_WAIT,
            MetadataRpcDeadline::Control,
            |session| pb::FilesystemCancelLockWaitRequest {
                context: Some(context(self.node_id)),
                session: Some(session),
                request_id,
                lock_owner,
            },
            |mut client, request| async move { client.cancel_filesystem_lock_wait(request).await },
        )
        .await
        .map(|response| response.affected)
    }

    pub(crate) async fn filesystem_release_lock_owner(
        &self,
        request_id: u64,
        lock_owner: u64,
    ) -> Result<u64, DmsError> {
        for _attempt in 0..METADATA_COMMIT_MAX_ATTEMPTS {
            let reopen_after_error = {
                let _reopen_guard = self.session_reopen_gate.lock().await;
                let used_session = self.current_session_snapshot().await;
                let response = self
                    .filesystem_rpc_once(
                        dms_metrics::RpcCall::FILESYSTEM_RELEASE_LOCK_OWNER,
                        MetadataRpcDeadline::Control,
                        pb::FilesystemReleaseLockOwnerRequest {
                            context: Some(context(self.node_id)),
                            session: Some(used_session.identity.clone()),
                            lock_owner,
                            request_id,
                        },
                        &mut |mut client, request| async move {
                            client.release_filesystem_lock_owner(request).await
                        },
                    )
                    .await;
                match response {
                    Ok(response) => {
                        let mirror_update = self
                            .remove_filesystem_lock_owner_mirror_if_session_current(
                                request_id,
                                lock_owner,
                                response.owner_revision,
                                &used_session,
                            )
                            .await;
                        match mirror_update {
                            LocalReleaseMirrorUpdate::Applied
                            | LocalReleaseMirrorUpdate::Obsolete => return Ok(response.affected),
                            LocalReleaseMirrorUpdate::SessionStale => None,
                        }
                    }
                    Err(error) if is_reopenable_session_error(&error) => Some(used_session),
                    Err(error) => return Err(error),
                }
            };
            if let Some(observed) = reopen_after_error {
                self.reopen_session_if_current(&observed).await?;
            }
        }
        Err(metadata_unavailable(
            "Meta filesystem lock owner release raced with session change too often",
        ))
    }

    async fn filesystem_rpc_once<Request, ResponseBody, Send, Fut>(
        &self,
        call: dms_metrics::RpcCall,
        deadline: MetadataRpcDeadline,
        request: Request,
        send: &mut Send,
    ) -> Result<ResponseBody, DmsError>
    where
        Send: FnMut(GrpcFilesystemMetadataClient<dms_tracing::TracedChannel>, Request) -> Fut,
        Fut: Future<Output = Result<Response<ResponseBody>, Status>>,
    {
        let client = self.filesystem_client();
        // 等待型锁请求不能继承普通控制面的 5 秒超时；其它锁 RPC 必须快速发现
        // session 失效并走统一 reopen 路径。这里仅封装“同一请求重发一次”的模板，
        // 不改变各 RPC 的 method、指标标签或错误映射边界。
        let response = match deadline {
            MetadataRpcDeadline::Control => {
                observe_control_rpc(self.rpc_metrics.as_ref(), call, send(client, request)).await
            }
            MetadataRpcDeadline::Operation => {
                observe_rpc(self.rpc_metrics.as_ref(), call, send(client, request)).await
            }
        };
        response.map(|value| value.into_inner()).map_err(map_status)
    }

    async fn update_filesystem_lock_mirror_if_session_current(
        &self,
        request: &pb::FilesystemLockRequest,
        response: &pb::FilesystemLockResponse,
        used_session: &SessionSnapshot,
    ) -> bool {
        let Ok(status) = filesystem_lock_status(response) else {
            return false;
        };
        debug_assert!(filesystem_lock_status_requires_mirror(status));
        let Some(range) = request.range.as_ref() else {
            return false;
        };
        let mut mirror = self.filesystem_locks.lock().await;
        // 顺序必须是 mirror mutex -> session read。reopen 发布新 session 前会先
        // 按同一个 mirror mutex 重报快照；这样旧 epoch 的成功响应要么进入本次
        // reclaim 快照，要么在新 session 发布后被丢弃，不会跨 incarnation 污染 mirror。
        let current = self.current_session_snapshot().await;
        if current != *used_session {
            return false;
        }
        if mirror
            .release_fences
            .get(&request.lock_owner)
            .is_some_and(|fence| request.request_id <= *fence)
        {
            return false;
        }
        if response.owner_revision == 0 {
            return false;
        }
        if mirror
            .owner_revisions
            .get(&request.lock_owner)
            .is_some_and(|revision| response.owner_revision < *revision)
        {
            return false;
        }
        mirror
            .owner_revisions
            .insert(request.lock_owner, response.owner_revision);
        replace_local_file_lock_range(
            &mut mirror.locks,
            request.inode,
            request.lock_owner,
            range.start,
            range.end_inclusive,
            (status == pb::FilesystemLockStatus::Acquired).then_some((request.mode, request.pid)),
        );
        true
    }

    async fn begin_filesystem_lock_set_inflight(
        &self,
        lock_owner: u64,
        request_id: u64,
    ) -> FilesystemLockSetInflightGuard {
        let mut mirror = self.filesystem_locks.lock().await;
        mirror
            .inflight_sets
            .entry(lock_owner)
            .or_default()
            .insert(request_id);
        FilesystemLockSetInflightGuard {
            client: Some(self.clone()),
            lock_owner,
            request_id,
        }
    }

    #[cfg(test)]
    async fn register_filesystem_lock_set_inflight(&self, lock_owner: u64, request_id: u64) {
        let guard = self
            .begin_filesystem_lock_set_inflight(lock_owner, request_id)
            .await;
        std::mem::forget(guard);
    }

    async fn finish_filesystem_lock_set_inflight(&self, lock_owner: u64, request_id: u64) {
        let mut mirror = self.filesystem_locks.lock().await;
        if let Some(inflight) = mirror.inflight_sets.get_mut(&lock_owner) {
            inflight.remove(&request_id);
            if inflight.is_empty() {
                mirror.inflight_sets.remove(&lock_owner);
            }
        }
        prune_local_release_fence_if_unblocked(&mut mirror, lock_owner);
    }

    async fn remove_filesystem_lock_owner_mirror_if_session_current(
        &self,
        request_id: u64,
        lock_owner: u64,
        owner_revision: u64,
        used_session: &SessionSnapshot,
    ) -> LocalReleaseMirrorUpdate {
        let mut mirror = self.filesystem_locks.lock().await;
        let current = self.current_session_snapshot().await;
        if current != *used_session {
            return LocalReleaseMirrorUpdate::SessionStale;
        }
        if owner_revision == 0 {
            return LocalReleaseMirrorUpdate::SessionStale;
        }
        let current_owner_revision = mirror
            .owner_revisions
            .get(&lock_owner)
            .copied()
            .unwrap_or(0);
        if owner_revision < current_owner_revision {
            return LocalReleaseMirrorUpdate::Obsolete;
        }
        if owner_revision == current_owner_revision {
            // This is an exact retry or a duplicate response for a release already
            // observed by the local mirror. Keep it idempotent: applying deletion a
            // second time would let an old response mutate state after the current
            // owner revision has already been established locally.
            return LocalReleaseMirrorUpdate::Obsolete;
        }
        mirror
            .release_fences
            .entry(lock_owner)
            .and_modify(|fence| *fence = (*fence).max(request_id))
            .or_insert(request_id);
        mirror
            .owner_revisions
            .entry(lock_owner)
            .and_modify(|revision| *revision = (*revision).max(owner_revision))
            .or_insert(owner_revision);
        mirror.locks.retain(|lock| lock.lock_owner != lock_owner);
        prune_local_release_fence_if_unblocked(&mut mirror, lock_owner);
        LocalReleaseMirrorUpdate::Applied
    }

    pub(crate) async fn stat(&self, key: Vec<u8>) -> Result<pb::MetaStatResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::MetaStatRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
            key: Some(pb::Key { value: key.clone() }),
        };
        let mut client = self.client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_STAT,
            client.stat(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_STAT,
                client.stat(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn scan(
        &self,
        prefix: Vec<u8>,
        options: Option<pb::ObjectScanOptions>,
    ) -> Result<pb::MetaScanResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::MetaScanRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
            prefix: Some(pb::Key {
                value: prefix.clone(),
            }),
            options: options.clone(),
        };
        let mut client = self.client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_SCAN,
            client.scan(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_SCAN,
                client.scan(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn commit_value(
        &self,
        key: Vec<u8>,
        block_id: Vec<u8>,
        length: u64,
        checksum: Vec<u8>,
        operation_id: Vec<u8>,
        condition: String,
    ) -> Result<pb::CommitVersionResponse, DmsError> {
        if length == 0 {
            return self
                .commit(
                    key,
                    operation_id,
                    pb::VersionCandidate {
                        kind: pb::VersionKind::Value as i32,
                        logical_length: 0,
                        extents: Vec::new(),
                        digest: digest(&[]),
                    },
                    Vec::new(),
                    Vec::new(),
                    condition,
                )
                .await;
        }
        self.commit(
            key,
            operation_id,
            pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length: length,
                extents: vec![pb::ExtentRecord {
                    logical: Some(pb::ByteRange { offset: 0, length }),
                    block_id: block_id.clone(),
                    block_offset: 0,
                    digest: checksum.clone(),
                    kind: pb::ExtentKind::Data as i32,
                }],
                digest: checksum.clone(),
            },
            Vec::new(),
            vec![pb::ReplicaReport {
                block_id,
                length,
                checksum,
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            }],
            condition,
        )
        .await
    }

    /// Publishes an arbitrary immutable Extent layout. SET_RANGE, Hash and
    /// later file semantics reuse this path instead of materializing a flat
    /// value merely to fit the single-block SET helper.
    pub(crate) async fn commit_layout(
        &self,
        key: Vec<u8>,
        operation_id: Vec<u8>,
        candidate: pb::VersionCandidate,
        replica_proofs: Vec<pb::ReplicaProof>,
        new_replicas: Vec<pb::ReplicaReport>,
        condition: String,
    ) -> Result<pb::CommitVersionResponse, DmsError> {
        self.commit(
            key,
            operation_id,
            candidate,
            replica_proofs,
            new_replicas,
            condition,
        )
        .await
    }

    pub(crate) async fn commit_batch_values(
        &self,
        values: Vec<BatchValueCommit>,
        batch_operation_id: Vec<u8>,
    ) -> Result<pb::CommitBatchResponse, DmsError> {
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| {
            let entries = values
                .iter()
                .map(|value| {
                    let candidate = pb::VersionCandidate {
                        kind: pb::VersionKind::Value as i32,
                        logical_length: value.length,
                        extents: vec![pb::ExtentRecord {
                            logical: Some(pb::ByteRange {
                                offset: 0,
                                length: value.length,
                            }),
                            block_id: value.block_id.clone(),
                            block_offset: 0,
                            digest: value.checksum.clone(),
                            kind: pb::ExtentKind::Data as i32,
                        }],
                        digest: value.checksum.clone(),
                    };
                    let mut operation_digest = digest(&value.key);
                    operation_digest.extend_from_slice(&candidate.digest);
                    operation_digest.push(candidate.kind as u8);
                    pb::BatchCommitEntry {
                        key: Some(pb::Key {
                            value: value.key.clone(),
                        }),
                        candidate: Some(candidate),
                        condition: "any".to_string(),
                        expected_version: None,
                        operation_id: value.operation_id.clone(),
                        operation_digest,
                        replica_proofs: Vec::new(),
                        new_replicas: vec![pb::ReplicaReport {
                            block_id: value.block_id.clone(),
                            length: value.length,
                            checksum: value.checksum.clone(),
                            durability: pb::DurabilityPolicy::LocalMemory as i32,
                        }],
                    }
                })
                .collect();
            pb::CommitBatchRequest {
                commit_sequence,
                context: Some(context(self.node_id)),
                session: Some(session),
                entries,
                batch_operation_id: batch_operation_id.clone(),
            }
        };
        let mut client = self.client();
        let mut result = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_COMMIT_BATCH,
            client.commit_batch(make_request(session.clone())),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client();
            result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_COMMIT_BATCH,
                client.commit_batch(make_request(session)),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    /// Registers a readable replica after peer pull/repair has completed.
    /// This is deliberately separate from normal SET, whose first replica and
    /// Version are committed atomically in `CommitVersion.new_replicas`.
    pub(crate) async fn report_replica(
        &self,
        block_id: Vec<u8>,
        length: u64,
        checksum: Vec<u8>,
        operation_id: Vec<u8>,
        desired_copies: u32,
        repair_id: Vec<u8>,
    ) -> Result<(), DmsError> {
        self.report_replicas(
            vec![pb::ReplicaReport {
                block_id,
                length,
                checksum,
                durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
            }],
            operation_id,
            desired_copies,
            repair_id,
        )
        .await
    }

    /// 一次登记多个已经落到本 Node 的不可变 Block。
    ///
    /// `ReportReplicasRequest` 的 wire 合同原本就允许 `repeated replicas`。普通
    /// Peer 首读由后台 reporter 把队列里已经就绪的 Block 合成一批，避免每个
    /// 小 Block 单独做一次 Meta RPC；repair 仍可通过 `report_replica` 发送单项。
    pub(crate) async fn report_replicas(
        &self,
        replicas: Vec<pb::ReplicaReport>,
        operation_id: Vec<u8>,
        desired_copies: u32,
        repair_id: Vec<u8>,
    ) -> Result<(), DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::ReportReplicasRequest {
            context: Some(context(self.node_id)),
            session: Some(session.clone()),
            replicas: replicas.clone(),
            operation_id: operation_id.clone(),
            desired_copies: desired_copies.max(1),
            repair_id: repair_id.clone(),
        };
        let mut client = self.client();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_REPORT_REPLICAS,
            client.report_replicas(make_request(session.clone())),
        )
        .await
        .map(|_| ())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_REPORT_REPLICAS,
                client.report_replicas(make_request(session)),
            )
            .await
            .map(|_| ())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn commit_delete(
        &self,
        key: Vec<u8>,
        operation_id: Vec<u8>,
    ) -> Result<pb::CommitVersionResponse, DmsError> {
        self.commit(
            key,
            operation_id,
            pb::VersionCandidate {
                kind: pb::VersionKind::Tombstone as i32,
                logical_length: 0,
                extents: Vec::new(),
                digest: Vec::new(),
            },
            Vec::new(),
            Vec::new(),
            "any".to_string(),
        )
        .await
    }

    async fn commit(
        &self,
        key: Vec<u8>,
        operation_id: Vec<u8>,
        candidate: pb::VersionCandidate,
        replica_proofs: Vec<pb::ReplicaProof>,
        new_replicas: Vec<pb::ReplicaReport>,
        condition: String,
    ) -> Result<pb::CommitVersionResponse, DmsError> {
        // digest 同时覆盖操作身份与候选内容；相同 operation_id 携带不同内容会冲突。
        let mut operation_digest = digest(&key);
        operation_digest.extend_from_slice(&candidate.digest);
        operation_digest.push(candidate.kind as u8);
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        let mut session = self.current_session().await;

        for attempt in 1..=METADATA_COMMIT_MAX_ATTEMPTS {
            let mut client = self.client();
            let result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_COMMIT_VERSION,
                client.commit_version(pb::CommitVersionRequest {
                    commit_sequence,
                    context: Some(context(self.node_id)),
                    session: Some(session.clone()),
                    key: Some(pb::Key { value: key.clone() }),
                    candidate: Some(candidate.clone()),
                    condition: condition.clone(),
                    expected_version: None,
                    operation_id: operation_id.clone(),
                    operation_digest: operation_digest.clone(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                    required_memory_copies: 1,
                    replica_proofs: replica_proofs.clone(),
                    new_replicas: new_replicas.clone(),
                }),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);

            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_reopenable_session_error(&error) =>
                {
                    session = self.reopen_session().await?;
                }
                Err(error)
                    if attempt < METADATA_COMMIT_MAX_ATTEMPTS
                        && is_uncertain_commit_result_error(&error) =>
                {
                    // Unavailable/DeadlineExceeded 表示 Node 不知道 Meta 是否已经在
                    // journal/apply 后丢了回复。重发必须复用完全相同的 request；
                    // Meta 通过 operation_id/digest 和 commit_sequence 做幂等收敛。
                    dms_logging::warn!(
                        "Meta commit response is uncertain; retrying same operation";
                        "event" => "node.meta_commit.uncertain_retry",
                        "node_id" => self.node_id,
                        "attempt" => attempt,
                        "max_attempts" => METADATA_COMMIT_MAX_ATTEMPTS,
                        "error_code" => error.code().raw(),
                        "error_kind" => format!("{:?}", error.kind()),
                    );
                }
                Err(error) => return Err(error),
            }
        }

        Err(metadata_unavailable(
            "Meta commit retry loop exhausted without a terminal result",
        ))
    }

    async fn begin_commit(&self) -> Result<(MutexGuard<'_, ()>, u64), DmsError> {
        // 先取得门闩、再分配序号。若反过来，等待锁的两个调用仍可能把较大
        // sequence 先发出去，无法保证 Meta 看到的顺序。
        let guard = self.commit_gate.lock().await;
        let sequence = allocate_commit_sequence(&self.next_commit_sequence)?;
        Ok((guard, sequence))
    }

    pub(crate) async fn watch_events(
        &self,
        last_acked_cursor: u64,
    ) -> Result<tonic::Streaming<pb::NodeEvent>, DmsError> {
        let mut session = self.current_session().await;
        let mut client = self.client.clone();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_WATCH_NODE_EVENTS,
            client.watch_node_events(pb::WatchNodeEventsRequest {
                context: Some(context(self.node_id)),
                session: Some(session.clone()),
                last_acked_cursor,
            }),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client.clone();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_WATCH_NODE_EVENTS,
                client.watch_node_events(pb::WatchNodeEventsRequest {
                    context: Some(context(self.node_id)),
                    session: Some(session),
                    last_acked_cursor,
                }),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn acknowledge_event(&self, event: &pb::NodeEvent) -> Result<(), DmsError> {
        let mut session = self.current_session().await;
        let mut client = self.client.clone();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_ACKNOWLEDGE_NODE_EVENT,
            client.acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                context: Some(context(self.node_id)),
                session: Some(session.clone()),
                event_id: event.event_id.clone(),
                cursor: event.cursor,
                result: "applied".to_string(),
                detail: None,
            }),
        )
        .await
        .map(|_| ())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client.clone();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_ACKNOWLEDGE_NODE_EVENT,
                client.acknowledge_node_event(pb::AcknowledgeNodeEventRequest {
                    context: Some(context(self.node_id)),
                    session: Some(session),
                    event_id: event.event_id.clone(),
                    cursor: event.cursor,
                    result: "applied".to_string(),
                    detail: None,
                }),
            )
            .await
            .map(|_| ())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn acknowledge_block_retirement(
        &self,
        event: &pb::EvictReplicaEvent,
        ack_kind: pb::BlockRetirementAckKind,
        detail: Option<String>,
    ) -> Result<(), DmsError> {
        let mut session = self.current_session().await;
        let make_request =
            |session: pb::NodeSessionIdentity| pb::AcknowledgeBlockRetirementRequest {
                context: Some(context(self.node_id)),
                session: Some(session),
                retirement_id: event.retirement_id.clone(),
                ack_kind: ack_kind as i32,
                stage_epoch: event.stage_epoch,
                block_ids: retirement_block_ids(event),
                detail: detail.clone(),
            };
        let mut client = self.client.clone();
        let mut result = observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_ACKNOWLEDGE_BLOCK_RETIREMENT,
            client.acknowledge_block_retirement(make_request(session.clone())),
        )
        .await
        .and_then(|response| {
            response
                .into_inner()
                .accepted
                .then_some(())
                .ok_or_else(|| Status::failed_precondition("retirement ack rejected"))
        })
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client.clone();
            result = observe_control_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_ACKNOWLEDGE_BLOCK_RETIREMENT,
                client.acknowledge_block_retirement(make_request(session)),
            )
            .await
            .and_then(|response| {
                response
                    .into_inner()
                    .accepted
                    .then_some(())
                    .ok_or_else(|| Status::failed_precondition("retirement ack rejected"))
            })
            .map_err(map_status);
        }
        result
    }

    fn client(&self) -> GrpcMetadataClient<dms_tracing::TracedChannel> {
        self.client.clone()
    }

    fn filesystem_client(&self) -> GrpcFilesystemMetadataClient<dms_tracing::TracedChannel> {
        self.filesystem_client.clone()
    }
}

async fn run_resolve_batcher(mut rpc: ResolveBatchRpc, mut receive: mpsc::Receiver<ResolveJob>) {
    while let Some(first) = receive.recv().await {
        let jobs = collect_ready_resolve_jobs(first, &mut receive);
        let result = rpc.resolve(&jobs).await;
        deliver_resolve_results(jobs, result);
    }
}

/// 为一次 Resolve RPC 收集“调用时已经就绪”的请求。
///
/// 这个函数刻意不是 `async`，也没有定时器：第一项到达后立即形成批次，只用
/// `try_recv` 吸收同一调度波次中已经排队的请求。这样并发小对象能共享控制 RPC，
/// 单个请求却不会为了凑批增加固定延迟。
fn collect_ready_resolve_jobs(
    first: ResolveJob,
    receive: &mut mpsc::Receiver<ResolveJob>,
) -> Vec<ResolveJob> {
    let mut jobs = Vec::with_capacity(METADATA_RESOLVE_BATCH_MAX);
    jobs.push(first);
    while jobs.len() < METADATA_RESOLVE_BATCH_MAX {
        match receive.try_recv() {
            Ok(job) => jobs.push(job),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }
    jobs
}

impl ResolveBatchRpc {
    async fn current_session_snapshot(&self) -> SessionSnapshot {
        self.session_state.read().await.snapshot()
    }

    async fn resolve(
        &mut self,
        jobs: &[ResolveJob],
    ) -> Result<Vec<pb::ResolveObjectResult>, DmsError> {
        let mut session = self.current_session_snapshot().await;
        let mut result = self.resolve_once(jobs, session.identity.clone()).await;
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session_if_current(&session).await?;
            result = self.resolve_once(jobs, session.identity).await;
        }
        result
    }

    async fn resolve_once(
        &mut self,
        jobs: &[ResolveJob],
        session: pb::NodeSessionIdentity,
    ) -> Result<Vec<pb::ResolveObjectResult>, DmsError> {
        let request = pb::ResolveObjectsRequest {
            requests: jobs
                .iter()
                .map(|job| resolve_request(self.node_id, session.clone(), job))
                .collect(),
        };
        observe_control_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_RESOLVE_OBJECTS,
            self.client.resolve_objects(request),
        )
        .await
        .map(|response| response.into_inner().results)
        .map_err(map_status)
    }

    async fn reopen_session_if_current(
        &mut self,
        observed: &SessionSnapshot,
    ) -> Result<SessionSnapshot, DmsError> {
        let _reopen_guard = self.session_reopen_gate.lock().await;
        let next_generation = {
            let mut state = self.session_state.write().await;
            let current = state.snapshot();
            if current != *observed {
                return Ok(current);
            }
            state.generation += 1;
            state.generation
        };
        let (session, minimum_sequence) = MetadataClient::open_session(
            &mut self.client,
            self.node_id,
            &self.data_endpoint,
            self.rpc_metrics.as_ref(),
        )
        .await?;
        self.next_commit_sequence
            .fetch_max(minimum_sequence.max(1), Ordering::Relaxed);
        dms_logging::info!(
            "Meta session reopened by resolve batcher";
            "event" => "node.meta_session.reopened",
            "node_id" => self.node_id,
            "node_epoch" => session.node_epoch,
        );
        // Resolve 批处理任务和普通 MetadataClient 调用共享 session。只有
        // filesystem Node 才需要在发布新 epoch 前重报完整锁快照；纯 KV Node
        // 不应依赖另一个 gRPC Service。
        if self.filesystem_enabled {
            reclaim_filesystem_locks_snapshot(
                self.filesystem_client.clone(),
                &self.filesystem_locks,
                &session,
                self.node_id,
                self.rpc_metrics.as_ref(),
            )
            .await?;
        }
        *self.session_state.write().await = SessionState {
            identity: session.clone(),
            generation: next_generation,
        };
        Ok(SessionSnapshot {
            identity: session,
            generation: next_generation,
        })
    }
}

async fn reclaim_filesystem_locks_snapshot(
    mut client: GrpcFilesystemMetadataClient<dms_tracing::TracedChannel>,
    filesystem_locks: &Arc<Mutex<LocalFilesystemLockMirror>>,
    session: &pb::NodeSessionIdentity,
    node_id: u64,
    rpc_metrics: Option<&dms_metrics::RpcMetrics>,
) -> Result<(), DmsError> {
    let locks = {
        let mut mirror = filesystem_locks.lock().await;
        // Reclaim 发生在 session generation 已推进之后：旧响应会被 generation
        // fence 拒绝；如果 Meta 在 same-session restart 后 revision 从 1 重新开始，
        // 本地旧 revision 高水位也必须同时清掉，否则新 Meta incarnation 的合法锁
        // mutation 会被误判成旧响应。
        mirror.owner_revisions.clear();
        mirror
            .locks
            .iter()
            .copied()
            // Reclaim 只把本地镜像重新序列化给 Meta；真正的锁领域结构仍在
            // filesystem::wire 统一映射，避免 Node 侧再维护一份 DTO 拼装规则。
            .map(|lock| lock.to_reclaim_proto(session))
            .collect::<Result<Vec<_>, _>>()?
    };
    observe_control_rpc(
        rpc_metrics,
        dms_metrics::RpcCall::FILESYSTEM_RECLAIM_LOCKS,
        client.reclaim_filesystem_locks(pb::FilesystemReclaimLocksRequest {
            context: Some(context(node_id)),
            session: Some(session.clone()),
            locks,
        }),
    )
    .await
    .map(|_| ())
    .map_err(map_status)
}

fn resolve_request(
    node_id: u64,
    session: pb::NodeSessionIdentity,
    job: &ResolveJob,
) -> pb::ResolveObjectRequest {
    pb::ResolveObjectRequest {
        context: Some(context(node_id)),
        session: Some(session),
        key: Some(pb::Key {
            value: job.key.clone(),
        }),
        selector: Some(
            job.exact_version
                .map(pb::resolve_object_request::Selector::ExactVersion)
                .unwrap_or(pb::resolve_object_request::Selector::Current(true)),
        ),
        range: None,
        cache_current: true,
    }
}

fn deliver_resolve_results(
    jobs: Vec<ResolveJob>,
    result: Result<Vec<pb::ResolveObjectResult>, DmsError>,
) {
    match result {
        Ok(results) if results.len() == jobs.len() => {
            for (job, result) in jobs.into_iter().zip(results) {
                let result = result.response.ok_or_else(|| {
                    DmsError::new(
                        dms_error::META_CATALOG_NOT_FOUND,
                        ErrorKind::NotFound,
                        "metadata object was not found",
                    )
                });
                let _ = job.reply.send(result);
            }
        }
        Ok(results) => {
            let error = metadata_unavailable(format!(
                "ResolveObjects returned {} results for {} requests",
                results.len(),
                jobs.len()
            ));
            for job in jobs {
                let _ = job.reply.send(Err(error.clone()));
            }
        }
        Err(error) => {
            for job in jobs {
                let _ = job.reply.send(Err(error.clone()));
            }
        }
    }
}

fn metadata_unavailable(message: impl Into<String>) -> DmsError {
    DmsError::new(
        dms_error::NODE_METADATA_UNAVAILABLE,
        ErrorKind::Unavailable,
        message,
    )
}

fn filesystem_lock_status(
    response: &pb::FilesystemLockResponse,
) -> Result<pb::FilesystemLockStatus, DmsError> {
    pb::FilesystemLockStatus::try_from(response.status).map_err(|_| {
        DmsError::new(
            dms_error::NODE_METADATA_UNAVAILABLE,
            ErrorKind::Internal,
            format!(
                "Meta returned unknown filesystem lock status {}",
                response.status
            ),
        )
    })
}

fn filesystem_lock_status_requires_mirror(status: pb::FilesystemLockStatus) -> bool {
    matches!(
        status,
        pb::FilesystemLockStatus::Acquired | pb::FilesystemLockStatus::Released
    )
}

pub(crate) fn retirement_block_ids(event: &pb::EvictReplicaEvent) -> Vec<Vec<u8>> {
    if event.block_ids.is_empty() {
        (!event.block_id.is_empty())
            .then(|| event.block_id.clone())
            .into_iter()
            .collect()
    } else {
        event.block_ids.clone()
    }
}

fn allocate_commit_sequence(next: &AtomicU64) -> Result<u64, DmsError> {
    next.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_add(1)
    })
    .map_err(|_| {
        DmsError::new(
            dms_error::NODE_METADATA_UNAVAILABLE,
            ErrorKind::ResourceExhausted,
            "Node commit sequence exhausted; refusing sequence reuse",
        )
    })
}

fn metadata_client(
    channel: tonic::transport::Channel,
) -> GrpcMetadataClient<dms_tracing::TracedChannel> {
    let config = GrpcConfig::default();
    GrpcMetadataClient::new(dms_tracing::traced_channel(channel))
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes)
}

fn filesystem_metadata_client(
    channel: tonic::transport::Channel,
) -> GrpcFilesystemMetadataClient<dms_tracing::TracedChannel> {
    let config = GrpcConfig::default();
    GrpcFilesystemMetadataClient::new(dms_tracing::traced_channel(channel))
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes)
}

/// 统计一次真实的 Node→Meta 网络尝试；Session 失效后的重试会单独计数。
async fn observe_rpc<T, F>(
    metrics: Option<&dms_metrics::RpcMetrics>,
    call: dms_metrics::RpcCall,
    future: F,
) -> Result<Response<T>, Status>
where
    F: Future<Output = Result<Response<T>, Status>>,
{
    let mut guard = metrics.map(|metrics| metrics.begin_client_call(call));
    let result = future.await;
    if result.is_ok()
        && let Some(guard) = &mut guard
    {
        guard.success();
    }
    result
}

/// 执行一次需要快速故障发现的普通 Node→Meta 控制 RPC。
///
/// gRPC `Channel` 的外层超时必须允许 Commit 等待恢复租约，但 Heartbeat、Resolve、
/// Watch 建连和 ACK 不能继承 35 秒预算。这里仅包住一次 unary 请求（或 server
/// stream 返回响应头之前的 Future）；拿到 Watch stream 后，后续事件等待不受 5 秒
/// 限制。超时会作为标准 DeadlineExceeded 进入现有错误映射与指标统计。
async fn observe_control_rpc<T, F>(
    metrics: Option<&dms_metrics::RpcMetrics>,
    call: dms_metrics::RpcCall,
    future: F,
) -> Result<Response<T>, Status>
where
    F: Future<Output = Result<Response<T>, Status>>,
{
    tokio::time::timeout(
        METADATA_CONTROL_REQUEST_TIMEOUT,
        observe_rpc(metrics, call, future),
    )
    .await
    .map_err(|_| Status::deadline_exceeded("Meta control RPC deadline exceeded"))?
}

fn context(node_id: u64) -> pb::RequestContext {
    pb::RequestContext {
        node_id,
        principal: format!("node-{node_id}"),
        timeout_millis: Duration::from_secs(30).as_millis() as u64,
    }
}

fn replace_local_file_lock_range(
    locks: &mut Vec<LocalHeldFileLock>,
    inode: u64,
    lock_owner: u64,
    start: u64,
    end_inclusive: u64,
    replacement: Option<(i32, u32)>,
) {
    let mut retained = Vec::with_capacity(locks.len() + 1);
    for held in locks.drain(..) {
        if held.inode != inode
            || held.lock_owner != lock_owner
            || held.end_inclusive < start
            || end_inclusive < held.start
        {
            retained.push(held);
            continue;
        }
        if held.start < start {
            retained.push(LocalHeldFileLock {
                end_inclusive: start - 1,
                ..held
            });
        }
        if held.end_inclusive > end_inclusive && end_inclusive != u64::MAX {
            retained.push(LocalHeldFileLock {
                start: end_inclusive + 1,
                ..held
            });
        }
    }
    if let Some((mode, pid)) = replacement {
        retained.push(LocalHeldFileLock {
            inode,
            lock_owner,
            start,
            end_inclusive,
            mode,
            pid,
        });
    }
    retained.sort_by_key(|lock| (lock.inode, lock.lock_owner, lock.mode, lock.pid, lock.start));
    let mut normalized = Vec::<LocalHeldFileLock>::with_capacity(retained.len());
    for lock in retained {
        if let Some(previous) = normalized.last_mut()
            && previous.inode == lock.inode
            && previous.lock_owner == lock.lock_owner
            && previous.mode == lock.mode
            && previous.pid == lock.pid
            && (previous.end_inclusive >= lock.start
                || previous.end_inclusive.checked_add(1) == Some(lock.start))
        {
            previous.end_inclusive = previous.end_inclusive.max(lock.end_inclusive);
        } else {
            normalized.push(lock);
        }
    }
    *locks = normalized;
}

fn prune_local_release_fence_if_unblocked(mirror: &mut LocalFilesystemLockMirror, lock_owner: u64) {
    if let Some(fence) = mirror.release_fences.get(&lock_owner).copied() {
        let has_older_inflight = mirror
            .inflight_sets
            .get(&lock_owner)
            .is_some_and(|requests| requests.iter().any(|request_id| *request_id <= fence));
        if has_older_inflight {
            return;
        }
        mirror.release_fences.remove(&lock_owner);
    }
    let has_locks = mirror
        .locks
        .iter()
        .any(|lock| lock.lock_owner == lock_owner);
    let has_inflight = mirror
        .inflight_sets
        .get(&lock_owner)
        .is_some_and(|requests| !requests.is_empty());
    if !has_locks && !has_inflight && !mirror.release_fences.contains_key(&lock_owner) {
        mirror.owner_revisions.remove(&lock_owner);
    }
}

pub(crate) fn digest(bytes: &[u8]) -> Vec<u8> {
    dms_transport::checksum::stable_digest_bytes(bytes).to_vec()
}

fn is_reopenable_session_error(error: &DmsError) -> bool {
    error.code() == dms_error::META_SESSION_UNKNOWN
}

fn is_uncertain_commit_result_error(error: &DmsError) -> bool {
    matches!(
        error.kind(),
        ErrorKind::Unavailable | ErrorKind::DeadlineExceeded
    )
}

fn map_status(status: tonic::Status) -> DmsError {
    status_to_dms_error_with(status, |message| {
        DmsError::new(
            dms_error::NODE_METADATA_UNAVAILABLE,
            ErrorKind::Unavailable,
            message,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_transport::dms_error_to_status;
    use pb::filesystem_metadata_service_server::{
        FilesystemMetadataService, FilesystemMetadataServiceServer,
    };
    use std::{collections::HashSet, sync::atomic::AtomicBool, thread};
    use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
    use tokio_stream::wrappers::TcpListenerStream;

    fn resolve_job(index: u64) -> ResolveJob {
        let (reply, _receive) = oneshot::channel();
        ResolveJob {
            key: format!("key-{index}").into_bytes(),
            exact_version: None,
            reply,
        }
    }

    fn metadata_client_for_test(session: pb::NodeSessionIdentity) -> MetadataClient {
        let channel = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        MetadataClient {
            client: metadata_client(channel.clone()),
            filesystem_client: filesystem_metadata_client(channel),
            session_state: Arc::new(RwLock::new(SessionState {
                identity: session,
                generation: 1,
            })),
            next_commit_sequence: Arc::new(AtomicU64::new(1)),
            next_filesystem_lock_mutation_sequence: Arc::new(AtomicU64::new(1)),
            commit_gate: Arc::new(Mutex::new(())),
            session_reopen_gate: Arc::new(Mutex::new(())),
            node_id: 9,
            data_endpoint: "http://127.0.0.1:0".to_string(),
            rpc_metrics: None,
            resolve_tx: mpsc::channel(1).0,
            filesystem_locks: Arc::new(Mutex::new(LocalFilesystemLockMirror::default())),
            filesystem_enabled: false,
        }
    }

    #[derive(Clone, Default)]
    struct LockOrderingFilesystemService {
        state: Arc<LockOrderingState>,
    }

    #[derive(Default)]
    struct LockOrderingState {
        events: Mutex<Vec<&'static str>>,
        meta_locks: Mutex<Vec<pb::FilesystemGrantedLock>>,
        release_entered: Notify,
        release_continue: Notify,
        reclaim_entered: Notify,
        reclaim_continue: Notify,
        block_release: AtomicBool,
        block_reclaim: AtomicBool,
        reject_release_as_stale: AtomicBool,
    }

    impl LockOrderingFilesystemService {
        async fn events(&self) -> Vec<&'static str> {
            self.state.events.lock().await.clone()
        }

        async fn meta_lock_owners(&self) -> Vec<u64> {
            self.state
                .meta_locks
                .lock()
                .await
                .iter()
                .filter_map(|lock| lock.owner.as_ref().map(|owner| owner.lock_owner))
                .collect()
        }

        fn block_release(&self) {
            self.state.block_release.store(true, Ordering::Relaxed);
        }

        fn unblock_release(&self) {
            self.state.release_continue.notify_waiters();
        }

        fn block_reclaim(&self) {
            self.state.block_reclaim.store(true, Ordering::Relaxed);
        }

        fn unblock_reclaim(&self) {
            self.state.reclaim_continue.notify_waiters();
        }

        fn reject_release_as_stale(&self) {
            self.state
                .reject_release_as_stale
                .store(true, Ordering::Relaxed);
        }
    }

    fn unimplemented_filesystem_response<T>(
        name: &'static str,
    ) -> Result<tonic::Response<T>, tonic::Status> {
        Err(tonic::Status::unimplemented(name))
    }

    #[tonic::async_trait]
    impl FilesystemMetadataService for LockOrderingFilesystemService {
        async fn lookup_filesystem_entry(
            &self,
            _request: tonic::Request<pb::FilesystemLookupRequest>,
        ) -> Result<tonic::Response<pb::FilesystemResolveResponse>, tonic::Status> {
            unimplemented_filesystem_response("lookup_filesystem_entry")
        }

        async fn get_filesystem_inode(
            &self,
            _request: tonic::Request<pb::FilesystemGetInodeRequest>,
        ) -> Result<tonic::Response<pb::FilesystemResolveResponse>, tonic::Status> {
            unimplemented_filesystem_response("get_filesystem_inode")
        }

        async fn create_filesystem_inode(
            &self,
            _request: tonic::Request<pb::FilesystemCreateInodeRequest>,
        ) -> Result<tonic::Response<pb::FilesystemResolveResponse>, tonic::Status> {
            unimplemented_filesystem_response("create_filesystem_inode")
        }

        async fn create_filesystem_symlink(
            &self,
            _request: tonic::Request<pb::FilesystemCreateSymlinkRequest>,
        ) -> Result<tonic::Response<pb::FilesystemResolveResponse>, tonic::Status> {
            unimplemented_filesystem_response("create_filesystem_symlink")
        }

        async fn read_filesystem_directory(
            &self,
            _request: tonic::Request<pb::FilesystemReadDirectoryRequest>,
        ) -> Result<tonic::Response<pb::FilesystemReadDirectoryResponse>, tonic::Status> {
            unimplemented_filesystem_response("read_filesystem_directory")
        }

        async fn link_filesystem_entry(
            &self,
            _request: tonic::Request<pb::FilesystemLinkRequest>,
        ) -> Result<tonic::Response<pb::FilesystemNamespaceMutationResponse>, tonic::Status>
        {
            unimplemented_filesystem_response("link_filesystem_entry")
        }

        async fn acquire_filesystem_inode_reference(
            &self,
            _request: tonic::Request<pb::FilesystemInodeReferenceRequest>,
        ) -> Result<tonic::Response<pb::FilesystemInodeReferenceResponse>, tonic::Status> {
            unimplemented_filesystem_response("acquire_filesystem_inode_reference")
        }

        async fn release_filesystem_inode_reference(
            &self,
            _request: tonic::Request<pb::FilesystemInodeReferenceRequest>,
        ) -> Result<tonic::Response<pb::FilesystemInodeReferenceResponse>, tonic::Status> {
            unimplemented_filesystem_response("release_filesystem_inode_reference")
        }

        async fn renew_filesystem_inode_references(
            &self,
            _request: tonic::Request<pb::FilesystemRenewInodeReferencesRequest>,
        ) -> Result<tonic::Response<pb::FilesystemInodeReferenceResponse>, tonic::Status> {
            unimplemented_filesystem_response("renew_filesystem_inode_references")
        }

        async fn rename_filesystem_entry(
            &self,
            _request: tonic::Request<pb::FilesystemRenameRequest>,
        ) -> Result<tonic::Response<pb::FilesystemNamespaceMutationResponse>, tonic::Status>
        {
            unimplemented_filesystem_response("rename_filesystem_entry")
        }

        async fn remove_filesystem_entry(
            &self,
            _request: tonic::Request<pb::FilesystemRemoveRequest>,
        ) -> Result<tonic::Response<pb::FilesystemNamespaceMutationResponse>, tonic::Status>
        {
            unimplemented_filesystem_response("remove_filesystem_entry")
        }

        async fn set_filesystem_attributes(
            &self,
            _request: tonic::Request<pb::FilesystemSetAttributesRequest>,
        ) -> Result<tonic::Response<pb::FilesystemAttributeMutationResponse>, tonic::Status>
        {
            unimplemented_filesystem_response("set_filesystem_attributes")
        }

        async fn get_filesystem_xattr(
            &self,
            _request: tonic::Request<pb::FilesystemGetXattrRequest>,
        ) -> Result<tonic::Response<pb::FilesystemGetXattrResponse>, tonic::Status> {
            unimplemented_filesystem_response("get_filesystem_xattr")
        }

        async fn list_filesystem_xattrs(
            &self,
            _request: tonic::Request<pb::FilesystemListXattrsRequest>,
        ) -> Result<tonic::Response<pb::FilesystemListXattrsResponse>, tonic::Status> {
            unimplemented_filesystem_response("list_filesystem_xattrs")
        }

        async fn set_filesystem_xattr(
            &self,
            _request: tonic::Request<pb::FilesystemSetXattrRequest>,
        ) -> Result<tonic::Response<pb::FilesystemAttributeMutationResponse>, tonic::Status>
        {
            unimplemented_filesystem_response("set_filesystem_xattr")
        }

        async fn remove_filesystem_xattr(
            &self,
            _request: tonic::Request<pb::FilesystemRemoveXattrRequest>,
        ) -> Result<tonic::Response<pb::FilesystemAttributeMutationResponse>, tonic::Status>
        {
            unimplemented_filesystem_response("remove_filesystem_xattr")
        }

        async fn stat_filesystem(
            &self,
            _request: tonic::Request<pb::FilesystemStatRequest>,
        ) -> Result<tonic::Response<pb::FilesystemStatResponse>, tonic::Status> {
            unimplemented_filesystem_response("stat_filesystem")
        }

        async fn commit_filesystem_version(
            &self,
            _request: tonic::Request<pb::FilesystemCommitVersionRequest>,
        ) -> Result<tonic::Response<pb::FilesystemCommitVersionResponse>, tonic::Status> {
            unimplemented_filesystem_response("commit_filesystem_version")
        }

        async fn test_filesystem_lock(
            &self,
            _request: tonic::Request<pb::FilesystemLockRequest>,
        ) -> Result<tonic::Response<pb::FilesystemLockResponse>, tonic::Status> {
            unimplemented_filesystem_response("test_filesystem_lock")
        }

        async fn set_filesystem_lock(
            &self,
            _request: tonic::Request<pb::FilesystemLockRequest>,
        ) -> Result<tonic::Response<pb::FilesystemLockResponse>, tonic::Status> {
            unimplemented_filesystem_response("set_filesystem_lock")
        }

        async fn cancel_filesystem_lock_wait(
            &self,
            _request: tonic::Request<pb::FilesystemCancelLockWaitRequest>,
        ) -> Result<tonic::Response<pb::FilesystemLockMutationResponse>, tonic::Status> {
            unimplemented_filesystem_response("cancel_filesystem_lock_wait")
        }

        async fn release_filesystem_lock_owner(
            &self,
            request: tonic::Request<pb::FilesystemReleaseLockOwnerRequest>,
        ) -> Result<tonic::Response<pb::FilesystemLockMutationResponse>, tonic::Status> {
            self.state.events.lock().await.push("release");
            self.state.release_entered.notify_waiters();
            if self.state.block_release.load(Ordering::Relaxed) {
                self.state.release_continue.notified().await;
            }
            if self.state.reject_release_as_stale.load(Ordering::Relaxed) {
                return Err(tonic::Status::invalid_argument(
                    "filesystem lock release request_id is outside the retry window",
                ));
            }
            let request = request.into_inner();
            let mut meta_locks = self.state.meta_locks.lock().await;
            let before = meta_locks.len();
            meta_locks.retain(|lock| {
                lock.owner
                    .as_ref()
                    .is_none_or(|owner| owner.lock_owner != request.lock_owner)
            });
            Ok(tonic::Response::new(pb::FilesystemLockMutationResponse {
                affected: (before - meta_locks.len()) as u64,
                owner_revision: request.request_id,
            }))
        }

        async fn reclaim_filesystem_locks(
            &self,
            request: tonic::Request<pb::FilesystemReclaimLocksRequest>,
        ) -> Result<tonic::Response<pb::FilesystemLockMutationResponse>, tonic::Status> {
            self.state.events.lock().await.push("reclaim");
            self.state.reclaim_entered.notify_waiters();
            if self.state.block_reclaim.load(Ordering::Relaxed) {
                self.state.reclaim_continue.notified().await;
            }
            let locks = request.into_inner().locks;
            let affected = locks.len() as u64;
            *self.state.meta_locks.lock().await = locks;
            Ok(tonic::Response::new(pb::FilesystemLockMutationResponse {
                affected,
                owner_revision: 0,
            }))
        }
    }

    async fn metadata_client_with_lock_ordering_service(
        session: pb::NodeSessionIdentity,
    ) -> (
        MetadataClient,
        LockOrderingFilesystemService,
        oneshot::Sender<()>,
        JoinHandle<()>,
    ) {
        let service = LockOrderingFilesystemService::default();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind filesystem lock ordering service");
        let endpoint = format!("http://{}", listener.local_addr().expect("local addr"));
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task_service = service.clone();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(FilesystemMetadataServiceServer::new(task_service))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve filesystem lock ordering service");
        });
        let channel = Endpoint::from_shared(endpoint)
            .expect("endpoint")
            .connect()
            .await
            .expect("connect filesystem lock ordering service");
        let client = MetadataClient {
            client: metadata_client(channel.clone()),
            filesystem_client: filesystem_metadata_client(channel),
            session_state: Arc::new(RwLock::new(SessionState {
                identity: session,
                generation: 1,
            })),
            next_commit_sequence: Arc::new(AtomicU64::new(1)),
            next_filesystem_lock_mutation_sequence: Arc::new(AtomicU64::new(1)),
            commit_gate: Arc::new(Mutex::new(())),
            session_reopen_gate: Arc::new(Mutex::new(())),
            node_id: 9,
            data_endpoint: "http://127.0.0.1:0".to_string(),
            rpc_metrics: None,
            resolve_tx: mpsc::channel(1).0,
            filesystem_locks: Arc::new(Mutex::new(LocalFilesystemLockMirror::default())),
            filesystem_enabled: true,
        };
        (client, service, shutdown_tx, task)
    }

    #[test]
    fn resolve_batch_absorbs_only_ready_jobs_without_waiting() {
        let (sender, mut receiver) = mpsc::channel(METADATA_RESOLVE_BATCH_MAX + 2);
        for index in 1..=(METADATA_RESOLVE_BATCH_MAX + 2) {
            sender
                .try_send(resolve_job(index as u64))
                .expect("test resolve queue has capacity");
        }

        let batch = collect_ready_resolve_jobs(resolve_job(0), &mut receiver);

        assert_eq!(batch.len(), METADATA_RESOLVE_BATCH_MAX);
        assert_eq!(receiver.len(), 3, "overflow waits for the next RPC batch");
    }

    #[test]
    fn commit_sequence_is_shared_and_never_wraps() {
        let next = Arc::new(AtomicU64::new(1));
        let clone = next.clone();
        assert_eq!(allocate_commit_sequence(&next).unwrap(), 1);
        assert_eq!(allocate_commit_sequence(&clone).unwrap(), 2);
        next.store(u64::MAX, Ordering::Relaxed);
        assert!(allocate_commit_sequence(&next).is_err());
        assert_eq!(next.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn local_lock_reclaim_proto_uses_current_session_identity() {
        let session = pb::NodeSessionIdentity {
            session_id: b"new-session".to_vec(),
            node_id: 7,
            node_epoch: 42,
        };
        let lock = LocalHeldFileLock {
            inode: 11,
            lock_owner: 99,
            start: 4,
            end_inclusive: 8,
            mode: pb::FilesystemLockMode::Exclusive as i32,
            pid: 1234,
        };

        let dto = lock.to_reclaim_proto(&session).expect("valid lock mirror");

        let owner = dto.owner.expect("owner");
        let range = dto.range.expect("range");
        assert_eq!(dto.inode, 11);
        assert_eq!(owner.node_id, 7);
        assert_eq!(owner.node_epoch, 42);
        assert_eq!(owner.lock_owner, 99);
        assert_eq!(range.start, 4);
        assert_eq!(range.end_inclusive, 8);
        assert_eq!(dto.mode, pb::FilesystemLockMode::Exclusive as i32);
        assert_eq!(dto.pid, 1234);
    }

    #[tokio::test]
    async fn filesystem_release_before_same_session_reclaim_keeps_meta_and_mirror_empty() {
        let session = pb::NodeSessionIdentity {
            session_id: b"same-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let (client, service, shutdown, task) =
            metadata_client_with_lock_ordering_service(session.clone()).await;
        let lock = LocalHeldFileLock {
            inode: 11,
            lock_owner: 3,
            start: 0,
            end_inclusive: 9,
            mode: pb::FilesystemLockMode::Exclusive as i32,
            pid: 1234,
        };
        client.filesystem_locks.lock().await.locks.push(lock);
        service
            .state
            .meta_locks
            .lock()
            .await
            .push(lock.to_reclaim_proto(&session).unwrap());

        service.block_release();
        let release_entered = service.state.release_entered.notified();
        let release_client = client.clone();
        let release = tokio::spawn(async move {
            release_client
                .filesystem_release_lock_owner(100, 3)
                .await
                .expect("release owner")
        });
        release_entered.await;
        assert_eq!(service.events().await, vec!["release"]);

        let observed = client.current_session_snapshot().await;
        let reclaim_client = client.clone();
        let reclaim = tokio::spawn(async move {
            reclaim_client
                .reclaim_filesystem_locks_if_current(&observed)
                .await
                .expect("same-session reclaim")
        });
        assert!(
            tokio::time::timeout(
                Duration::from_millis(50),
                service.state.reclaim_entered.notified()
            )
            .await
            .is_err(),
            "reclaim RPC must wait behind in-flight release"
        );

        let reclaim_entered = service.state.reclaim_entered.notified();
        service.unblock_release();
        assert_eq!(release.await.unwrap(), 1);
        reclaim_entered.await;
        reclaim.await.unwrap();

        assert_eq!(service.events().await, vec!["release", "reclaim"]);
        assert!(service.meta_lock_owners().await.is_empty());
        assert!(client.filesystem_locks.lock().await.locks.is_empty());
        let _ = shutdown.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn stale_release_error_does_not_delete_local_lock_mirror() {
        let session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let (client, service, shutdown, task) =
            metadata_client_with_lock_ordering_service(session).await;
        client
            .filesystem_locks
            .lock()
            .await
            .locks
            .push(LocalHeldFileLock {
                inode: 11,
                lock_owner: 3,
                start: 0,
                end_inclusive: 9,
                mode: pb::FilesystemLockMode::Exclusive as i32,
                pid: 1234,
            });
        service.reject_release_as_stale();

        let result = client.filesystem_release_lock_owner(1, 3).await;

        assert!(result.is_err(), "stale release must be visible to Node");
        assert_eq!(
            client.filesystem_locks.lock().await.locks.len(),
            1,
            "Node must not delete local mirror unless Meta accepted the exact release"
        );
        let _ = shutdown.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn late_set_response_after_release_does_not_repopulate_mirror_or_reclaim() {
        let session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let (client, service, shutdown, task) =
            metadata_client_with_lock_ordering_service(session).await;
        let request = pb::FilesystemLockRequest {
            context: None,
            session: None,
            request_id: 1,
            inode: 11,
            lock_owner: 3,
            range: Some(pb::FilesystemLockRange {
                start: 0,
                end_inclusive: 9,
            }),
            mode: pb::FilesystemLockMode::Exclusive as i32,
            pid: 1234,
            wait: false,
        };
        let response = pb::FilesystemLockResponse {
            status: pb::FilesystemLockStatus::Acquired as i32,
            conflict: None,
            owner_revision: 1,
        };

        client
            .register_filesystem_lock_set_inflight(request.lock_owner, request.request_id)
            .await;
        assert_eq!(client.filesystem_release_lock_owner(2, 3).await.unwrap(), 0);

        let current = client.current_session_snapshot().await;
        assert!(
            !client
                .update_filesystem_lock_mirror_if_session_current(&request, &response, &current)
                .await,
            "set response with seq <= owner release fence must not repopulate mirror"
        );
        client
            .finish_filesystem_lock_set_inflight(request.lock_owner, request.request_id)
            .await;
        {
            let mirror = client.filesystem_locks.lock().await;
            assert!(mirror.locks.is_empty());
            assert!(mirror.inflight_sets.is_empty());
            assert!(
                mirror.release_fences.is_empty(),
                "fence is only kept while it protects an older in-flight set"
            );
        }

        let observed = client.current_session_snapshot().await;
        client
            .reclaim_filesystem_locks_if_current(&observed)
            .await
            .expect("reclaim empty mirror");
        assert!(service.meta_lock_owners().await.is_empty());
        let _ = shutdown.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn older_owner_revision_set_response_after_release_does_not_repopulate_mirror() {
        let session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let (client, _service, shutdown, task) =
            metadata_client_with_lock_ordering_service(session).await;
        let request = pb::FilesystemLockRequest {
            context: None,
            session: None,
            request_id: 3,
            inode: 11,
            lock_owner: 3,
            range: Some(pb::FilesystemLockRange {
                start: 0,
                end_inclusive: 9,
            }),
            mode: pb::FilesystemLockMode::Exclusive as i32,
            pid: 1234,
            wait: false,
        };
        // This response models Meta applying Set(seq=3) first, then Release(seq=2)
        // second, but the Set response arrives at Node last. Request-id fences alone
        // would accept seq=3; owner_revision must reject it because Release is the
        // later Meta-applied mutation.
        let stale_set_response = pb::FilesystemLockResponse {
            status: pb::FilesystemLockStatus::Acquired as i32,
            conflict: None,
            owner_revision: 1,
        };

        client
            .register_filesystem_lock_set_inflight(request.lock_owner, request.request_id)
            .await;
        assert_eq!(client.filesystem_release_lock_owner(2, 3).await.unwrap(), 0);
        let current = client.current_session_snapshot().await;
        assert!(
            !client
                .update_filesystem_lock_mirror_if_session_current(
                    &request,
                    &stale_set_response,
                    &current,
                )
                .await,
            "late Set response with older Meta owner_revision must not repopulate mirror"
        );
        client
            .finish_filesystem_lock_set_inflight(request.lock_owner, request.request_id)
            .await;
        assert!(client.filesystem_locks.lock().await.locks.is_empty());

        let observed = client.current_session_snapshot().await;
        client
            .reclaim_filesystem_locks_if_current(&observed)
            .await
            .expect("reclaim empty mirror");
        let _ = shutdown.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn reclaim_generation_clears_local_owner_revisions_for_meta_restart() {
        let session = pb::NodeSessionIdentity {
            session_id: b"same-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let (client, _service, shutdown, task) =
            metadata_client_with_lock_ordering_service(session).await;
        {
            let mut mirror = client.filesystem_locks.lock().await;
            mirror.owner_revisions.insert(3, 100);
        }

        let observed = client.current_session_snapshot().await;
        client
            .reclaim_filesystem_locks_if_current(&observed)
            .await
            .expect("same-session reclaim");

        let request = pb::FilesystemLockRequest {
            context: None,
            session: None,
            request_id: 101,
            inode: 11,
            lock_owner: 3,
            range: Some(pb::FilesystemLockRange {
                start: 0,
                end_inclusive: 9,
            }),
            mode: pb::FilesystemLockMode::Exclusive as i32,
            pid: 1234,
            wait: false,
        };
        let new_meta_response = pb::FilesystemLockResponse {
            status: pb::FilesystemLockStatus::Acquired as i32,
            conflict: None,
            owner_revision: 1,
        };
        let current = client.current_session_snapshot().await;
        assert!(
            client
                .update_filesystem_lock_mirror_if_session_current(
                    &request,
                    &new_meta_response,
                    &current,
                )
                .await,
            "same-session Meta restart must not leave old revision high-watermark behind"
        );
        assert_eq!(client.filesystem_locks.lock().await.locks.len(), 1);
        let _ = shutdown.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn local_release_fences_do_not_grow_without_protected_inflight_sets() {
        let current_session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let client = metadata_client_for_test(current_session.clone());
        let snapshot = SessionSnapshot {
            identity: current_session,
            generation: 1,
        };

        for owner in 1..=512 {
            assert_eq!(
                client
                    .remove_filesystem_lock_owner_mirror_if_session_current(
                        owner, owner, owner, &snapshot
                    )
                    .await,
                LocalReleaseMirrorUpdate::Applied
            );
        }

        let mirror = client.filesystem_locks.lock().await;
        assert!(mirror.locks.is_empty());
        assert!(mirror.inflight_sets.is_empty());
        assert!(
            mirror.release_fences.is_empty(),
            "release fences are tombstones for older in-flight sets, not historical owners"
        );
    }

    #[tokio::test]
    async fn same_session_reclaim_before_release_keeps_meta_and_mirror_empty() {
        let session = pb::NodeSessionIdentity {
            session_id: b"same-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let (client, service, shutdown, task) =
            metadata_client_with_lock_ordering_service(session).await;
        client
            .filesystem_locks
            .lock()
            .await
            .locks
            .push(LocalHeldFileLock {
                inode: 11,
                lock_owner: 3,
                start: 0,
                end_inclusive: 9,
                mode: pb::FilesystemLockMode::Exclusive as i32,
                pid: 1234,
            });

        service.block_reclaim();
        let observed = client.current_session_snapshot().await;
        let reclaim_entered = service.state.reclaim_entered.notified();
        let reclaim_client = client.clone();
        let reclaim = tokio::spawn(async move {
            reclaim_client
                .reclaim_filesystem_locks_if_current(&observed)
                .await
                .expect("same-session reclaim")
        });
        reclaim_entered.await;
        assert_eq!(service.events().await, vec!["reclaim"]);

        let release_client = client.clone();
        let release = tokio::spawn(async move {
            release_client
                .filesystem_release_lock_owner(100, 3)
                .await
                .expect("release owner")
        });
        assert!(
            tokio::time::timeout(
                Duration::from_millis(50),
                service.state.release_entered.notified()
            )
            .await
            .is_err(),
            "release RPC must wait behind in-flight reclaim"
        );

        let release_entered = service.state.release_entered.notified();
        service.unblock_reclaim();
        reclaim.await.unwrap();
        release_entered.await;
        assert_eq!(release.await.unwrap(), 1);

        assert_eq!(service.events().await, vec!["reclaim", "release"]);
        assert!(service.meta_lock_owners().await.is_empty());
        assert!(client.filesystem_locks.lock().await.locks.is_empty());
        let _ = shutdown.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn commit_sequence_is_allocated_only_after_commit_gate() {
        let client = metadata_client_for_test(pb::NodeSessionIdentity {
            session_id: b"test-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        });

        let held = client.commit_gate.lock().await;
        let waiting_client = client.clone();
        let waiting = tokio::spawn(async move {
            let (_guard, sequence) = waiting_client.begin_commit().await.unwrap();
            sequence
        });
        tokio::task::yield_now().await;
        assert_eq!(client.next_commit_sequence.load(Ordering::Relaxed), 1);

        drop(held);
        assert_eq!(waiting.await.unwrap(), 1);
        assert_eq!(client.next_commit_sequence.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn filesystem_lock_sequence_ignores_external_correlation_ids() {
        let client = metadata_client_for_test(pb::NodeSessionIdentity {
            session_id: b"test-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        });
        let _high_external_fuse_unique = u64::MAX - 10;
        let first_meta_sequence = client
            .allocate_filesystem_lock_mutation_sequence()
            .expect("first sequence");
        let _low_external_fuse_unique = 1;
        let second_meta_sequence = client
            .allocate_filesystem_lock_mutation_sequence()
            .expect("second sequence");

        assert_eq!(first_meta_sequence, 1);
        assert_eq!(second_meta_sequence, 2);
        assert!(
            second_meta_sequence > first_meta_sequence,
            "Meta request id must be allocated by Node, not copied from FUSE unique"
        );
    }

    #[tokio::test]
    async fn filesystem_lock_sequence_is_shared_across_metadata_client_clones() {
        let client = metadata_client_for_test(pb::NodeSessionIdentity {
            session_id: b"test-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        });
        let first_mount =
            super::super::filesystem::meta_client::FilesystemMetaGrpcClient::new(client.clone());
        let second_mount =
            super::super::filesystem::meta_client::FilesystemMetaGrpcClient::new(client.clone());

        assert_eq!(first_mount.allocate_lock_mutation_sequence().unwrap(), 1);
        assert_eq!(second_mount.allocate_lock_mutation_sequence().unwrap(), 2);
        assert_eq!(first_mount.allocate_lock_mutation_sequence().unwrap(), 3);
    }

    #[tokio::test]
    async fn filesystem_lock_sequence_is_concurrently_unique_and_monotonic() {
        let client = metadata_client_for_test(pb::NodeSessionIdentity {
            session_id: b"test-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        });
        let mut threads = Vec::new();
        for _ in 0..8 {
            let client = client.clone();
            threads.push(thread::spawn(move || {
                (0..128)
                    .map(|_| {
                        client
                            .allocate_filesystem_lock_mutation_sequence()
                            .expect("sequence")
                    })
                    .collect::<Vec<_>>()
            }));
        }

        let mut values = threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        let unique = values.iter().copied().collect::<HashSet<_>>();
        assert_eq!(unique.len(), values.len());
        values.sort_unstable();
        assert_eq!(values, (1..=1024).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn filesystem_lock_sequence_never_wraps_to_zero() {
        let client = metadata_client_for_test(pb::NodeSessionIdentity {
            session_id: b"test-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        });
        client
            .next_filesystem_lock_mutation_sequence
            .store(u64::MAX, Ordering::Relaxed);

        let error = client
            .allocate_filesystem_lock_mutation_sequence()
            .expect_err("u64::MAX must not be handed out");
        assert_eq!(error.kind(), ErrorKind::ResourceExhausted);
        assert_eq!(
            client
                .next_filesystem_lock_mutation_sequence
                .load(Ordering::Relaxed),
            u64::MAX
        );
    }

    #[test]
    fn structured_meta_status_preserves_original_meta_code() {
        let original = DmsError::new(
            dms_error::META_CATALOG_VERSION_CONFLICT,
            ErrorKind::Aborted,
            "stale expected version",
        );
        let decoded = map_status(dms_error_to_status(original.clone()));

        assert_eq!(decoded.code(), original.code());
        assert_eq!(decoded.kind(), original.kind());
        assert_eq!(decoded.message(), original.message());
    }

    #[test]
    fn bare_meta_status_is_classified_at_node_meta_boundary() {
        let decoded = map_status(tonic::Status::unavailable("connection refused"));

        assert_eq!(decoded.code(), dms_error::NODE_METADATA_UNAVAILABLE);
        assert_eq!(decoded.kind(), ErrorKind::Unavailable);
    }

    #[tokio::test]
    async fn write_fails_when_meta_endpoint_is_unavailable() {
        // connect_lazy 只构造 Channel，不要求此刻连通。它模拟“Node 曾经持有 Meta
        // Session，但 Meta 当前已经退出”的写路径。
        let client = metadata_client_for_test(pb::NodeSessionIdentity {
            session_id: b"stale-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        });
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            client.commit_delete(b"must-fail".to_vec(), b"op-1".to_vec()),
        )
        .await;
        assert!(
            !matches!(result, Ok(Ok(_))),
            "Meta unavailable must never acknowledge a commit"
        );
    }

    #[tokio::test]
    async fn stale_lock_success_does_not_update_current_session_mirror() {
        let current_session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 2,
        };
        let client = metadata_client_for_test(current_session.clone());
        let stale_session = pb::NodeSessionIdentity {
            session_id: b"stale-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let request = pb::FilesystemLockRequest {
            context: None,
            session: None,
            request_id: 77,
            inode: 11,
            lock_owner: 3,
            range: Some(pb::FilesystemLockRange {
                start: 0,
                end_inclusive: 9,
            }),
            mode: pb::FilesystemLockMode::Exclusive as i32,
            pid: 1234,
            wait: false,
        };
        let response = pb::FilesystemLockResponse {
            status: pb::FilesystemLockStatus::Acquired as i32,
            conflict: None,
            owner_revision: 1,
        };

        let updated = client
            .update_filesystem_lock_mirror_if_session_current(
                &request,
                &response,
                &SessionSnapshot {
                    identity: stale_session,
                    generation: 1,
                },
            )
            .await;
        assert!(!updated, "旧 epoch 的成功响应不能写入当前 mirror");
        assert!(client.filesystem_locks.lock().await.locks.is_empty());

        let updated = client
            .update_filesystem_lock_mirror_if_session_current(
                &request,
                &response,
                &SessionSnapshot {
                    identity: current_session,
                    generation: 1,
                },
            )
            .await;
        assert!(updated, "当前 epoch 的成功响应仍应维护 reclaim mirror");
        assert_eq!(
            client.filesystem_locks.lock().await.locks.as_slice(),
            &[LocalHeldFileLock {
                inode: 11,
                lock_owner: 3,
                start: 0,
                end_inclusive: 9,
                mode: pb::FilesystemLockMode::Exclusive as i32,
                pid: 1234,
            }]
        );
    }

    #[test]
    fn only_acquired_and_released_lock_statuses_require_mirror_fence() {
        assert!(filesystem_lock_status_requires_mirror(
            pb::FilesystemLockStatus::Acquired
        ));
        assert!(filesystem_lock_status_requires_mirror(
            pb::FilesystemLockStatus::Released
        ));
        assert!(!filesystem_lock_status_requires_mirror(
            pb::FilesystemLockStatus::Conflict
        ));
        assert!(!filesystem_lock_status_requires_mirror(
            pb::FilesystemLockStatus::Interrupted
        ));
        assert!(!filesystem_lock_status_requires_mirror(
            pb::FilesystemLockStatus::RecoveryPending
        ));
    }

    #[tokio::test]
    async fn session_generation_invalidates_old_lock_success_before_publish() {
        let current_session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 2,
        };
        let client = metadata_client_for_test(current_session.clone());
        let request = pb::FilesystemLockRequest {
            context: None,
            session: None,
            request_id: 88,
            inode: 11,
            lock_owner: 3,
            range: Some(pb::FilesystemLockRange {
                start: 0,
                end_inclusive: 9,
            }),
            mode: pb::FilesystemLockMode::Exclusive as i32,
            pid: 1234,
            wait: false,
        };
        let response = pb::FilesystemLockResponse {
            status: pb::FilesystemLockStatus::Acquired as i32,
            conflict: None,
            owner_revision: 1,
        };
        let before_reopen = SessionSnapshot {
            identity: current_session,
            generation: 1,
        };

        client.session_state.write().await.generation = 2;

        let updated = client
            .update_filesystem_lock_mirror_if_session_current(&request, &response, &before_reopen)
            .await;
        assert!(
            !updated,
            "reopen 开始后即使 session identity 尚未 publish，旧 generation 成功响应也不能写 mirror"
        );
        assert!(client.filesystem_locks.lock().await.locks.is_empty());
    }

    #[tokio::test]
    async fn stale_release_success_does_not_remove_current_session_mirror() {
        let current_session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 2,
        };
        let client = metadata_client_for_test(current_session.clone());
        client
            .filesystem_locks
            .lock()
            .await
            .locks
            .push(LocalHeldFileLock {
                inode: 11,
                lock_owner: 3,
                start: 0,
                end_inclusive: 9,
                mode: pb::FilesystemLockMode::Exclusive as i32,
                pid: 1234,
            });

        assert_eq!(
            client
                .remove_filesystem_lock_owner_mirror_if_session_current(
                    100,
                    3,
                    100,
                    &SessionSnapshot {
                        identity: pb::NodeSessionIdentity {
                            session_id: b"stale-session".to_vec(),
                            node_id: 9,
                            node_epoch: 1,
                        },
                        generation: 1,
                    },
                )
                .await,
            LocalReleaseMirrorUpdate::SessionStale
        );
        assert_eq!(client.filesystem_locks.lock().await.locks.len(), 1);

        assert_eq!(
            client
                .remove_filesystem_lock_owner_mirror_if_session_current(
                    100,
                    3,
                    100,
                    &SessionSnapshot {
                        identity: current_session,
                        generation: 1,
                    },
                )
                .await,
            LocalReleaseMirrorUpdate::Applied
        );
        assert!(client.filesystem_locks.lock().await.locks.is_empty());
    }

    #[tokio::test]
    async fn older_release_revision_after_newer_set_does_not_remove_mirror() {
        let current_session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let client = metadata_client_for_test(current_session.clone());
        {
            let mut mirror = client.filesystem_locks.lock().await;
            mirror.locks.push(LocalHeldFileLock {
                inode: 11,
                lock_owner: 3,
                start: 0,
                end_inclusive: 9,
                mode: pb::FilesystemLockMode::Exclusive as i32,
                pid: 1234,
            });
            mirror.owner_revisions.insert(3, 8);
        }

        let applied = client
            .remove_filesystem_lock_owner_mirror_if_session_current(
                6,
                3,
                5,
                &SessionSnapshot {
                    identity: current_session,
                    generation: 1,
                },
            )
            .await;

        assert_eq!(
            applied,
            LocalReleaseMirrorUpdate::Obsolete,
            "release response with an older Meta owner_revision must be treated as stale"
        );
        let mirror = client.filesystem_locks.lock().await;
        assert_eq!(
            mirror.locks.as_slice(),
            &[LocalHeldFileLock {
                inode: 11,
                lock_owner: 3,
                start: 0,
                end_inclusive: 9,
                mode: pb::FilesystemLockMode::Exclusive as i32,
                pid: 1234,
            }]
        );
        assert_eq!(mirror.owner_revisions.get(&3), Some(&8));
        assert!(
            mirror.release_fences.is_empty(),
            "stale release must not install a release fence"
        );
    }

    #[tokio::test]
    async fn obsolete_release_revision_returns_ok_and_preserves_newer_mirror() {
        let session = pb::NodeSessionIdentity {
            session_id: b"current-session".to_vec(),
            node_id: 9,
            node_epoch: 1,
        };
        let (client, _service, shutdown, task) =
            metadata_client_with_lock_ordering_service(session).await;
        {
            let mut mirror = client.filesystem_locks.lock().await;
            mirror.locks.push(LocalHeldFileLock {
                inode: 11,
                lock_owner: 3,
                start: 0,
                end_inclusive: 9,
                mode: pb::FilesystemLockMode::Exclusive as i32,
                pid: 1234,
            });
            mirror.owner_revisions.insert(3, 2);
        }

        let affected = client
            .filesystem_release_lock_owner(1, 3)
            .await
            .expect("obsolete release response should be safe success");

        assert_eq!(affected, 0);
        let mirror = client.filesystem_locks.lock().await;
        assert_eq!(
            mirror.locks.as_slice(),
            &[LocalHeldFileLock {
                inode: 11,
                lock_owner: 3,
                start: 0,
                end_inclusive: 9,
                mode: pb::FilesystemLockMode::Exclusive as i32,
                pid: 1234,
            }]
        );
        assert_eq!(mirror.owner_revisions.get(&3), Some(&2));
        assert!(
            mirror.release_fences.is_empty(),
            "obsolete release must not install a release fence"
        );
        let _ = shutdown.send(());
        task.await.unwrap();
    }
}

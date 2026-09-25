//! Node → Node 控制 Handler。
//!
//! 这个文件实现 node_control.proto 生成的 gRPC service，负责“建立/关闭数据
//! 传输会话”和基础 ping。它不传文件内容，不代替 Meta 提交新的访问授权，
//! 也不决定文件读写是否成功。
//!
//! RDMA 建连顺序：
//! 1. 客户端本地打开 RdmaEndpoint，拿到 client_info；
//! 2. 客户端调用 NegotiateData，把 client_info 通过 gRPC 发给服务端；
//! 3. 服务端先 prepare_probe 投递接收，再连接 client_info，返回 server_info + session_id；
//! 4. 客户端连接 server_info，通过 RDMA SEND_WITH_IMM 发探测并等待发送完成；
//! 5. 首次 data 请求在 session() 中消费探测的接收完成，然后才能访问文件或单边搬运。
//!
//! 参考 3FS 的“实际 RDMA 通道证明就绪”；这里按首次请求消费 CQ，不为闲置连接起轮询线程。
//! 握手版本不匹配直接拒绝；就绪不是收到 gRPC 协商请求就能成立的。
//!
//! TTL/close 只管理 RDMA 资源生命周期：过期或 close 后旧 session 不能继续使用。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use afs_protocol::node_control::{
    CloseDataReply, CloseDataRequest, NegotiateDataReply, NegotiateDataRequest, PingReply,
    PingRequest,
    node_control_server::{NodeControl, NodeControlServer},
};
use afs_tracing::Instrument;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

#[cfg(feature = "rdma")]
use afs_transport::rdma::{CAPACITY, INFO_BYTES, RdmaEndpoint};
#[cfg(feature = "rdma")]
use std::sync::atomic::AtomicU64;

#[cfg(feature = "rdma")]
const MAX_RDMA_SESSIONS: usize = 64;
/// RDMA SEND_WITH_IMM 探测握手版本；双方都确认后才创建/使用探测资源。
pub const RDMA_HANDSHAKE_VERSION: u32 = 1;
#[cfg(feature = "rdma")]
const PROBE_TIMEOUT_MS: u32 = 5000;
const SESSION_TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, Default)]
pub struct NodeControlConfig {
    pub rdma_device: Option<String>,
}

/// 服务端 RDMA session 表。
///
/// 它把 control proto 里的 `session_id` 映射到服务端持有的 `RdmaEndpoint`。
/// data.rs 每次 RDMA read/write 都先通过这里校验：session 存在、已 Ready、未 poisoned。
/// poisoned 表示这条 RDMA 路径结果可能不明，后续请求必须 fail closed。
#[derive(Clone, Default)]
pub struct RdmaSessionRegistry {
    inner: Arc<Mutex<HashMap<u64, Arc<RdmaSession>>>>,
    #[cfg(feature = "rdma")]
    next_id: Arc<AtomicU64>,
    device: Option<String>,
    ttl: Duration,
}

impl RdmaSessionRegistry {
    #[must_use]
    pub fn new(device: Option<String>) -> Self {
        Self::with_ttl(device, SESSION_TTL)
    }

    #[must_use]
    pub fn with_ttl(device: Option<String>, ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "rdma")]
            next_id: Arc::new(AtomicU64::new(1)),
            device,
            ttl,
        }
    }

    pub async fn session(&self, session_id: u64) -> Result<Arc<RdmaSession>, Status> {
        let session = self
            .inner
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| Status::failed_precondition("unknown RDMA session"))?;
        if session.poisoned.load(Ordering::SeqCst) {
            return Err(Status::failed_precondition("RDMA session poisoned"));
        }
        if !session.ready.load(Ordering::SeqCst) {
            #[cfg(feature = "rdma")]
            {
                // 工作任务持有 Arc 和 endpoint 锁直到 CQ 消费结束。即使 RPC future
                // 被取消，接收缓冲和队列也不会提前释放；并发首请求只消费一次探测。
                let pending = session.clone();
                tokio::task::spawn_blocking(move || {
                    let mut endpoint = pending.endpoint.blocking_lock();
                    if pending.poisoned.load(Ordering::SeqCst) {
                        return Err(Status::failed_precondition("RDMA session poisoned"));
                    }
                    if !pending.ready.load(Ordering::SeqCst) {
                        if let Err(error) = endpoint.wait_probe(PROBE_TIMEOUT_MS) {
                            pending.poisoned.store(true, Ordering::SeqCst);
                            return Err(Status::unavailable(error.to_string()));
                        }
                        pending.ready.store(true, Ordering::SeqCst);
                    }
                    Ok(())
                })
                .await
                .map_err(|error| Status::internal(error.to_string()))??;
            }
            #[cfg(not(feature = "rdma"))]
            return Err(Status::failed_precondition("RDMA session is not ready"));
        }
        *session.last_used.lock().expect("last_used lock poisoned") = Instant::now();
        Ok(session)
    }

    /// 清理超过 TTL 或已经 poisoned 的 session。
    ///
    /// 这不是业务恢复机制，只是释放传输资源并阻止旧 session 继续访问 MR/QP。
    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        self.inner.lock().await.retain(|_, session| {
            !session.poisoned.load(Ordering::SeqCst)
                && now.duration_since(*session.last_used.lock().expect("last_used lock poisoned"))
                    < self.ttl
        });
    }

    #[cfg(feature = "rdma")]
    pub async fn ensure_capacity(&self) -> Result<(), Status> {
        self.cleanup_expired().await;
        if self.inner.lock().await.len() >= MAX_RDMA_SESSIONS {
            return Err(Status::resource_exhausted("too many RDMA sessions"));
        }
        Ok(())
    }

    #[cfg(feature = "rdma")]
    async fn insert(&self, session: RdmaSession) -> Result<u64, Status> {
        self.cleanup_expired().await;
        let mut sessions = self.inner.lock().await;
        if sessions.len() >= MAX_RDMA_SESSIONS {
            return Err(Status::resource_exhausted("too many RDMA sessions"));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        sessions.insert(id, Arc::new(session));
        Ok(id)
    }

    // Close/TTL 关闭的是新请求入口，不是文件操作的取消或 drain 屏障。
    // 已取得 Arc 的请求可以完成；endpoint 最后一个持有者释放时才销毁 QP/MR。
    async fn remove(&self, session_id: u64) {
        self.inner.lock().await.remove(&session_id);
    }
}

/// 单个服务端 RDMA session 的状态。
///
/// `ready=false` 表示尚未消费真实 RDMA 探测接收完成；不是只缺一条 gRPC 确认。
/// `poisoned=true` 时表示某次 DMA/命令可能处于未知状态，必须拒绝复用。
pub struct RdmaSession {
    #[cfg(feature = "rdma")]
    pub endpoint: Arc<Mutex<RdmaEndpoint>>,
    ready: AtomicBool,
    pub poisoned: AtomicBool,
    last_used: StdMutex<Instant>,
}

impl RdmaSession {
    #[cfg(feature = "rdma")]
    fn new(endpoint: RdmaEndpoint) -> Self {
        Self {
            endpoint: Arc::new(Mutex::new(endpoint)),
            ready: AtomicBool::new(false),
            poisoned: AtomicBool::new(false),
            last_used: StdMutex::new(Instant::now()),
        }
    }
}

#[derive(Clone)]
pub struct NodeControlService {
    registry: RdmaSessionRegistry,
}

impl NodeControlService {
    #[must_use]
    pub fn new(registry: RdmaSessionRegistry) -> Self {
        Self { registry }
    }
}

pub fn make_control_server(registry: RdmaSessionRegistry) -> NodeControlServer<NodeControlService> {
    NodeControlServer::new(NodeControlService::new(registry))
}

#[tonic::async_trait]
impl NodeControl for NodeControlService {
    async fn negotiate_data(
        &self,
        request: Request<NegotiateDataRequest>,
    ) -> Result<Response<NegotiateDataReply>, Status> {
        let request = request.into_inner();
        if request.handshake_version != RDMA_HANDSHAKE_VERSION {
            return Err(Status::failed_precondition(
                "unsupported RDMA handshake version",
            ));
        }
        let Some(device) = self.registry.device.clone() else {
            return Ok(Response::new(NegotiateDataReply {
                session_id: 0,
                server_info: Vec::new(),
                capacity: 0,
                rdma_supported: false,
                handshake_version: RDMA_HANDSHAKE_VERSION,
            }));
        };
        negotiate_rdma(&self.registry, device, request).await
    }

    async fn close_data(
        &self,
        request: Request<CloseDataRequest>,
    ) -> Result<Response<CloseDataReply>, Status> {
        self.registry.remove(request.into_inner().session_id).await;
        Ok(Response::new(CloseDataReply {}))
    }

    async fn ping(&self, request: Request<PingRequest>) -> Result<Response<PingReply>, Status> {
        async move {
            let payload = request.into_inner().payload;
            let payload = if payload == "ping" {
                "pong".into()
            } else {
                payload
            };
            Ok(Response::new(PingReply { payload }))
        }
        .instrument(afs_tracing::tracing::info_span!("node.control.ping"))
        .await
    }
}

#[cfg(feature = "rdma")]
/// 处理 NegotiateData：服务端创建自己的 endpoint，并把 server_info 返回给客户端。
///
/// 注意这里仍是控制面 gRPC；真正文件内容不会出现在这个 proto 里。
async fn negotiate_rdma(
    registry: &RdmaSessionRegistry,
    device: String,
    request: NegotiateDataRequest,
) -> Result<Response<NegotiateDataReply>, Status> {
    if request.client_info.len() != INFO_BYTES {
        return Err(Status::invalid_argument("client_info must be 38 bytes"));
    }
    if request.capacity as usize > CAPACITY {
        return Err(Status::invalid_argument("client capacity exceeds 1MiB"));
    }
    registry.ensure_capacity().await?;
    let client_info = request.client_info;
    let (endpoint, info) = tokio::task::spawn_blocking(move || {
        let mut endpoint = RdmaEndpoint::open(&device).map_err(native_status)?;
        let info = endpoint.info().map_err(native_status)?;
        // 必须先投递 RECV 再向客户端公开 endpoint，避免客户端探测到达时没有接收槽。
        endpoint.prepare_probe().map_err(native_status)?;
        endpoint.connect(&client_info).map_err(native_status)?;
        Ok::<_, Status>((endpoint, info))
    })
    .await
    .map_err(|error| Status::internal(error.to_string()))??;
    let session_id = registry.insert(RdmaSession::new(endpoint)).await?;
    Ok(Response::new(NegotiateDataReply {
        session_id,
        server_info: info.to_vec(),
        capacity: CAPACITY as u32,
        rdma_supported: true,
        handshake_version: RDMA_HANDSHAKE_VERSION,
    }))
}

#[cfg(not(feature = "rdma"))]
/// 未编译 RDMA 时仍保留控制 RPC，但仅返回“不支持”，不会创建 endpoint。
async fn negotiate_rdma(
    _registry: &RdmaSessionRegistry,
    _device: String,
    _request: NegotiateDataRequest,
) -> Result<Response<NegotiateDataReply>, Status> {
    Ok(Response::new(NegotiateDataReply {
        session_id: 0,
        server_info: Vec::new(),
        capacity: 0,
        rdma_supported: false,
        handshake_version: RDMA_HANDSHAKE_VERSION,
    }))
}

#[cfg(feature = "rdma")]
fn native_status(error: afs_transport::rdma::RdmaError) -> Status {
    Status::unavailable(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_handshake_is_rejected_before_allocating_rdma_resources() {
        let service = NodeControlService::new(RdmaSessionRegistry::new(None));
        let error = service
            .negotiate_data(Request::new(NegotiateDataRequest::default()))
            .await
            .expect_err("missing probe handshake version must not negotiate");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn ping_returns_pong_for_literal_ping() {
        let service = NodeControlService::new(RdmaSessionRegistry::new(None));

        let reply = service
            .ping(Request::new(PingRequest {
                payload: "ping".into(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(reply.payload, "pong");
    }

    #[tokio::test]
    async fn unknown_and_closed_sessions_are_rejected() {
        let registry = RdmaSessionRegistry::new(None);

        assert_eq!(
            session_error_code(&registry, 99).await,
            tonic::Code::FailedPrecondition
        );
        registry.remove(99).await;
        assert_eq!(
            session_error_code(&registry, 99).await,
            tonic::Code::FailedPrecondition
        );
    }

    #[cfg(not(feature = "rdma"))]
    #[tokio::test]
    async fn cleanup_expired_removes_stale_entries_without_long_waits() {
        let registry = RdmaSessionRegistry::with_ttl(None, Duration::from_millis(1));
        registry.inner.lock().await.insert(
            1,
            Arc::new(RdmaSession {
                ready: AtomicBool::new(true),
                poisoned: AtomicBool::new(false),
                last_used: StdMutex::new(Instant::now() - Duration::from_secs(1)),
            }),
        );

        registry.cleanup_expired().await;

        assert_eq!(
            session_error_code(&registry, 1).await,
            tonic::Code::FailedPrecondition
        );
    }

    async fn session_error_code(registry: &RdmaSessionRegistry, session_id: u64) -> tonic::Code {
        match registry.session(session_id).await {
            Ok(_) => panic!("session unexpectedly exists"),
            Err(status) => status.code(),
        }
    }
}

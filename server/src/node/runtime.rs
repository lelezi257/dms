//! dms-node 内唯一的异步业务状态所有者。
//!
//! Client→Node 与 Node→Node gRPC Handler 都通过 [`NodeHandle`] 投递命令。
//! Handle 只持有 bounded `mpsc::Sender`，真正的 HashMap 只属于 `run_node`。
//! 每条 command 携带一个 `oneshot` 回信端，把结果送回原 RPC Future；等待结果时
//! 只挂起当前协程，不阻塞 Tokio 工作线程。

// Node actor 内的小型控制索引使用 HashMap；Payload bytes 只归 ArenaManager。
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use dms_error::{DmsError, ErrorKind};
use dms_logging::LevelController;
use dms_protocol::v1 as pb;
use dms_transport::{GrpcConfig, SecurityManager, TlsConfig};
// mpsc：多个 RPC Task 生产 command，一个 Node 状态 owner Task 消费。
// oneshot：一个请求对应一个、且只发送一次的结果。
use dms_tracing::Instrument as _;
use tokio::sync::{Mutex, mpsc, oneshot};
use tonic::transport::{Channel, Endpoint};

use super::arena_manager::{
    ArenaError, ArenaManager, ArenaReadTicket, HostAllocationTarget, HostReceipt, HostRegionGrant,
    HostShmDescriptor, ReleasedWriteAllocation, SharedFdBroker,
};
use super::current_cache::CurrentCache;
use super::metadata_client::{BatchValueCommit, MetadataClient, digest};
use super::metrics::{
    NodeMailboxCommand, NodeMetrics, ReplicaDirection, ReplicaOperation, SessionExpiration,
};
use crate::config::{ConfigChange, ConfigError, OnlineConfigController};

// 有界队列防止请求无限堆积；满载时 Sender::send().await 会施加背压。
const NODE_MAILBOX_CAPACITY: usize = 256;
// 测试构造器复用正式配置默认值；生产值随 NodeTaskConfig 一次性移入 owner。
#[cfg(test)]
const CLIENT_CACHE_LEASE_TTL: Duration =
    Duration::from_millis(crate::config::DEFAULT_CLIENT_CACHE_LEASE_TTL_MILLIS);
// Node 只保存“这个 session 可能缓存了哪些 Current key”的一致性兴趣，
// 不保存 Client 的数据。超过上限时退化为通配兴趣，直到租约过期。
const CLIENT_CACHE_INTEREST_KEY_LIMIT: usize = 4096;
const CLIENT_CACHE_INTEREST_BYTES_LIMIT: usize = 256 * 1024;
// Peer gRPC 默认有 4MiB 解码上限。跨 Node 大对象拉取必须拆成有界分段，
// 既避免单条消息无限放大，又保持 Node→Node 协议不变。
const PEER_PULL_SEGMENT_BYTES: u64 = 2 * 1024 * 1024;
const PEER_CHANNEL_CACHE_LIMIT: usize = 128;
const COMPLETED_RETIREMENT_CACHE_LIMIT: usize = 1024;
const RETIREMENT_PHASE_WAITER_LIMIT: usize = 16;

// 准备和完成都只访问唯一 owner；中间 Future 只拥有不可变提交资料与 Meta client。
// JoinSet 的数量上限与 mailbox 相同，饱和时拒绝新写，但 ACK/心跳仍可推进。
type WriteCompletion<T> = Box<dyn FnOnce(&mut NodeState) -> Result<T, WorkerError> + Send>;
type PreparedWrite<T> = Pin<Box<dyn Future<Output = WriteCompletion<T>> + Send>>;
type ApplyWrite = Box<dyn FnOnce(&mut NodeState) + Send>;

fn launch_write<T: Send + 'static>(
    state: &mut NodeState,
    writes: &mut tokio::task::JoinSet<ApplyWrite>,
    reply: oneshot::Sender<Result<T, WorkerError>>,
    prepare: impl FnOnce(&mut NodeState) -> Result<PreparedWrite<T>, WorkerError>,
) {
    let prepared = if writes.len() >= NODE_MAILBOX_CAPACITY {
        Err(WorkerError::ResourceExhausted)
    } else {
        prepare(state)
    };
    match prepared {
        Ok(prepared) => {
            writes.spawn(
                async move {
                    let complete = prepared.await;
                    Box::new(move |state: &mut NodeState| {
                        let _ = reply.send(complete(state));
                    }) as ApplyWrite
                }
                .instrument(dms_tracing::tracing::Span::current()),
            );
        }
        Err(error) => {
            let _ = reply.send(Err(error));
        }
    }
}

/// 单个 Client session 内共享读借用的单调序号。
///
/// `active_views` 将它关联到实际 allocation；连续归还水位推进后才能解除
/// 相应保护。`retirement_can_release` 检查仍活动的借用，阻止 GC 复用旧 bytes。
/// 不以 session 断连或后续 View 先完成代替这次借用已经归还。
type ViewEpoch = u64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NodeEvent {
    /// 通知 Client：这个 key 的 Current 缓存至少要达到 minimum_version。
    InvalidateCurrent {
        session_epoch: u64,
        event_sequence: u64,
        key: Vec<u8>,
        minimum_version: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetOutcome {
    /// 本次提交生成的新对象版本。
    pub(crate) version: u64,
    /// 新 value 的逻辑字节长度。
    pub(crate) length: u64,
    /// Local Client sessions must ACK this invalidation barrier before the
    /// synchronous Worker RPC can report success.
    barrier_id: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeleteOutcome {
    pub(crate) deleted: bool,
    pub(crate) version: u64,
    barrier_id: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadTicket {
    pub(crate) read_request_id: u64,
    /// 本次读取命中的对象版本。
    pub(crate) version: u64,
    /// 完整对象长度；range read 返回的 bytes 可能更短。
    pub(crate) logical_length: u64,
    /// 非 SHM 小对象直接随控制响应返回；None 表示沿 segments 获取 payload。
    pub(crate) inline_value: Option<Vec<u8>>,
    /// Ordered physical slices that reconstruct the requested logical range.
    /// A range overlay can therefore return old-prefix/new-patch/old-suffix
    /// without materializing another full-value buffer in dms-node.
    pub(crate) segments: Vec<ReadTicketSegment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadTicketSegment {
    pub(crate) logical_offset: u64,
    pub(crate) target: ReadTarget,
    pub(crate) payload_length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReadTarget {
    Grpc { transfer_id: u64 },
    Shm(HostShmDescriptor),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StagingAllocation {
    pub(crate) staging_id: u64,
    pub(crate) transfer_id: u64,
    pub(crate) length: u64,
    pub(crate) target: HostAllocationTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KeySetOutcome {
    pub(crate) key: Vec<u8>,
    pub(crate) version: u64,
}

#[derive(Debug)]
pub(crate) struct SetRangeInput {
    pub(crate) session_id: u64,
    pub(crate) key: Vec<u8>,
    pub(crate) offset: u64,
    pub(crate) staging_id: u64,
    pub(crate) receipt: HostReceipt,
    pub(crate) operation_id: Vec<u8>,
    pub(crate) expected_version: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MSetOutcome {
    versions: Vec<KeySetOutcome>,
    barrier_ids: Vec<u64>,
}

struct ValueCommitInput {
    cache_session_id: Option<u64>,
    key: Vec<u8>,
    block_id: Vec<u8>,
    length: u64,
    checksum: Vec<u8>,
    operation_id: Vec<u8>,
    condition: String,
}

/// Node→Node Probe 的传输无关结果。
pub(crate) struct PeerProbeResult {
    pub(crate) serving_node_id: String,
    pub(crate) nonce: Vec<u8>,
}

pub(crate) struct PeerBlockResult {
    pub(crate) serving_node_id: String,
    pub(crate) block_id: Vec<u8>,
    pub(crate) payload: Vec<u8>,
    pub(crate) checksum: Vec<u8>,
    pub(crate) length: u64,
}

#[derive(Clone, Debug)]
struct PeerPullSpec {
    endpoint: String,
    block_id: Vec<u8>,
    expected_checksum: Vec<u8>,
    expected_length: u64,
}

/// 一轮缺块导入只共享完成状态，不共享用户票据或大块 Vec。
/// Peer 位置失败可以按固定版本刷新；owner 接纳/Meta 登记失败不能当位置失败重试。
#[derive(Clone, Debug)]
struct PeerImportFailure {
    error: WorkerError,
    location_failure: bool,
}

impl PeerImportFailure {
    fn terminal(error: WorkerError) -> Self {
        Self {
            error,
            location_failure: false,
        }
    }
}

struct PeerImportFlight {
    // 仅标识本 Node 的一次内部网络任务，不替代 Meta 的幂等 operation id。
    attempt: u64,
    // 发起读取消后仍由 flight 持有此 GC pin，直到安装/Report 的任务真正结束。
    read_scope_id: u64,
    expected_length: u64,
    expected_checksum: Vec<u8>,
    task_id: Option<tokio::task::Id>,
    waiters: Vec<oneshot::Sender<Result<(), PeerImportFailure>>>,
}

/// 已产生缺块计划但尚未入队 Ensure 的同批读，也必须看见 Import/Report 的终态失败。
/// 仅保留到失败发生前的 read scopes 排空，不是跨请求的永久负缓存。
struct PeerImportFailureFence {
    cutoff_scope: u64,
    failure: PeerImportFailure,
}

type PeerImportCompletion = (Vec<u8>, u64, Result<(), PeerImportFailure>);

#[derive(Clone, Debug)]
pub(crate) struct ReplicaPrepareSpec {
    pub(crate) source_node_id: String,
    pub(crate) source_endpoint: String,
    pub(crate) plan_id: Vec<u8>,
    pub(crate) block_id: Vec<u8>,
    pub(crate) expected_length: u64,
    pub(crate) expected_checksum: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReplicaStateView {
    pub(crate) plan_id: Vec<u8>,
    pub(crate) block_id: Vec<u8>,
    pub(crate) status: &'static str,
    pub(crate) length: u64,
    pub(crate) checksum: Vec<u8>,
}

enum GetOutcome {
    Ready(ReadTicket),
    NeedsRemoteBlocks(Vec<PeerPullSpec>),
}

enum CachedReadOutcome {
    Miss,
    Ready(ReadTicket),
    NeedsExactRefresh {
        version: u64,
    },
    NeedsRemoteBlocks {
        resolved: pb::ResolveObjectResponse,
        specs: Vec<PeerPullSpec>,
    },
}

enum MaterializeOutcome {
    Ready { version: u64, bytes: Vec<u8> },
    NeedsRemoteBlocks(Vec<PeerPullSpec>),
}

#[derive(Clone, Debug)]
pub(crate) enum WorkerError {
    /// 请求内容不合法；静态字符串避免临时分配。
    InvalidArgument(&'static str),
    UnknownSession,
    UnknownStaging,
    UnknownTransfer,
    NotFound,
    Conflict,
    ResourceExhausted,
    WorkerUnavailable,
    MetadataUnavailable,
    TransferUnavailable,
    ArenaInvalidRequest,
    ArenaStaleHandle,
    ArenaShmUnavailable,
    ArenaAccessDenied,
    Stable(DmsError),
}

#[derive(Clone, Copy)]
struct DownloadTicket {
    read: ArenaReadTicket,
    session_id: u64,
    read_request_id: u64,
    expires_at: Instant,
}

#[derive(Clone, Debug)]
struct ActiveReadView {
    read_request_id: u64,
    allocation_ids: Vec<u64>,
}

struct PendingRetirement {
    block_ids: Vec<Vec<u8>>,
    cutoff_scope: u64,
    prepared: bool,
    final_requested: bool,
    released: bool,
    prepare_waiters: Vec<oneshot::Sender<Result<(), WorkerError>>>,
    final_waiters: Vec<oneshot::Sender<Result<(), WorkerError>>>,
}

struct ReadScopeGuard {
    node: NodeHandle,
    scope_id: Option<u64>,
}

struct ReadScopeLease {
    node: NodeHandle,
    scope_id: Option<u64>,
}

impl ReadScopeLease {
    fn new(node: NodeHandle, scope_id: u64) -> Self {
        Self {
            node,
            scope_id: Some(scope_id),
        }
    }

    fn into_guard(mut self) -> ReadScopeGuard {
        ReadScopeGuard {
            node: self.node.clone(),
            scope_id: self.scope_id.take(),
        }
    }
}

impl Drop for ReadScopeLease {
    fn drop(&mut self) {
        if let Some(scope_id) = self.scope_id.take() {
            let node = self.node.clone();
            // BeginReadScope 的 reply 可能已经成功写入 oneshot，但调用方 Future
            // 在 poll 出结果前被取消；lease 自身承接这段 handoff 窗口的归还义务。
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = node.finish_read_scope(scope_id).await;
                });
            } else {
                node.finish_read_scope_best_effort(scope_id);
            }
        }
    }
}

impl ReadScopeGuard {
    async fn begin(node: &NodeHandle, session_id: u64) -> Result<Self, WorkerError> {
        Ok(node.begin_read_scope(session_id).await?.into_guard())
    }

    fn id(&self) -> u64 {
        self.scope_id.expect("read scope is active")
    }

    async fn finish(mut self) -> Result<(), WorkerError> {
        let scope_id = self.scope_id.expect("read scope is active");
        self.node.finish_read_scope(scope_id).await?;
        self.scope_id.take();
        Ok(())
    }
}

impl Drop for ReadScopeGuard {
    fn drop(&mut self) {
        if let Some(scope_id) = self.scope_id.take() {
            let node = self.node.clone();
            // 取消 Future 时也必须释放 scope，否则 Prepare 会被永久卡住。
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = node.finish_read_scope(scope_id).await;
                });
            } else {
                node.finish_read_scope_best_effort(scope_id);
            }
        }
    }
}

/// Worker/Peer gRPC Handler 共用、可 clone 的 Node 任务提交句柄。
///
/// Cloning this value creates another producer for the same bounded mailbox;
/// it does not clone Node state.
#[derive(Clone)]
pub(crate) struct NodeHandle {
    // Sender 可 clone；每个 clone 都投向同一个 Receiver/NodeState。
    command_tx: mpsc::Sender<QueuedNodeCommand>,
    node_id: String,
    metadata: Option<MetadataClient>,
    fd_broker_path: Option<PathBuf>,
    metrics: NodeMetrics,
    rpc_metrics: dms_metrics::RpcMetrics,
    /// endpoint -> 已建立的 Peer gRPC Channel。
    ///
    /// Peer GET 缺块可能连续向同一个写入 Node 拉多个 Block；连接建立属于固定成本，
    /// 放在这里复用。这里缓存 Channel 而不是 generated client：generated client 是
    /// 某个 proto service 的 typed façade，按调用现场临时创建即可；Channel 才是连接资源。
    peer_channels: Arc<Mutex<HashMap<String, Channel>>>,
}

/// Resources and policies owned by the Node state task.
///
/// Grouping these values keeps the constructor boundary aligned with their
/// common lifecycle: they are all moved once into the single `run_node` owner.
pub(crate) struct NodeTaskConfig {
    pub(crate) arena_capacity_bytes: u64,
    pub(crate) region_size_bytes: u64,
    pub(crate) staging_ttl: Duration,
    pub(crate) client_cache_lease_ttl: Duration,
    pub(crate) node_current_cache_bytes: u64,
    pub(crate) node_current_cache_ttl: Duration,
    pub(crate) shared_fd_broker: Option<SharedFdBroker>,
    pub(crate) log_level: LevelController,
    /// Opt-in switch for successful Heartbeat command spans.
    pub(crate) trace_periodic_operations: bool,
}

impl NodeHandle {
    #[cfg(test)]
    pub(crate) async fn debug_peer_imports(&self) -> (usize, usize, u64) {
        let (reply, rx) = oneshot::channel();
        self.submit(NodeCommand::DebugPeerImports { reply })
            .await
            .expect("debug peer imports");
        rx.await.expect("debug peer imports reply")
    }

    pub(crate) fn metrics(&self) -> NodeMetrics {
        self.metrics.clone()
    }

    /// 启动唯一的状态 owner Task，并返回它的提交句柄。
    #[cfg(test)]
    pub(crate) fn spawn(
        node_id: String,
        metadata: MetadataClient,
        arena_capacity_bytes: u64,
        staging_ttl: Duration,
        shared_fd_broker: Option<SharedFdBroker>,
    ) -> Self {
        let registry = dms_metrics::registry();
        let metrics = NodeMetrics::register(&registry).expect("Node metrics registration");
        let rpc_metrics =
            dms_metrics::RpcMetrics::register(&registry).expect("Node RPC metrics registration");
        Self::spawn_with_metrics(
            node_id,
            metadata,
            NodeTaskConfig {
                arena_capacity_bytes,
                region_size_bytes: crate::config::DEFAULT_REGION_SIZE_BYTES,
                staging_ttl,
                client_cache_lease_ttl: CLIENT_CACHE_LEASE_TTL,
                node_current_cache_bytes: crate::config::DEFAULT_NODE_CURRENT_CACHE_BYTES,
                node_current_cache_ttl: Duration::from_millis(
                    crate::config::DEFAULT_NODE_CURRENT_CACHE_TTL_MILLIS,
                ),
                shared_fd_broker,
                log_level: LevelController::new(slog::Level::Info),
                trace_periodic_operations: false,
            },
            metrics,
            rpc_metrics,
        )
    }

    /// Production constructor using the process-owned Registry handles.
    pub(crate) fn spawn_with_metrics(
        node_id: String,
        metadata: MetadataClient,
        task_config: NodeTaskConfig,
        metrics: NodeMetrics,
        rpc_metrics: dms_metrics::RpcMetrics,
    ) -> Self {
        // channel 返回 `(Sender, Receiver)`；Receiver 只移动给 run_node。
        let (command_tx, command_rx) = mpsc::channel(NODE_MAILBOX_CAPACITY);
        let fd_broker_path = task_config
            .shared_fd_broker
            .as_ref()
            .map(|broker| broker.path.clone());
        // spawn 立即调度 async Future；这里不会等待 run_node 结束。
        tokio::spawn(run_node(
            node_id.clone(),
            Some(metadata.clone()),
            task_config,
            metrics.clone(),
            command_rx,
        ));
        Self {
            command_tx,
            node_id,
            metadata: Some(metadata),
            fd_broker_path,
            metrics,
            rpc_metrics,
            peer_channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub(crate) fn spawn_without_metadata(node_id: String) -> Self {
        let registry = dms_metrics::registry();
        let metrics = NodeMetrics::register(&registry).expect("test Node metrics");
        let rpc_metrics =
            dms_metrics::RpcMetrics::register(&registry).expect("test Node RPC metrics");
        let (command_tx, command_rx) = mpsc::channel(NODE_MAILBOX_CAPACITY);
        tokio::spawn(run_node(
            node_id.clone(),
            None,
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
                log_level: LevelController::new(slog::Level::Info),
                trace_periodic_operations: false,
            },
            metrics.clone(),
            command_rx,
        ));
        Self {
            command_tx,
            node_id,
            metadata: None,
            fd_broker_path: None,
            metrics,
            rpc_metrics,
            peer_channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // 首版没有在线管理 RPC；保留这一条真正作用到 Node owner 的预埋入口，
    // 避免未来控制面只能修改一份与业务脱节的配置副本。
    #[allow(dead_code)]
    pub(crate) async fn apply_config_change(
        &self,
        change: ConfigChange,
    ) -> Result<u64, ConfigError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::ApplyConfigChange {
            change,
            reply: reply_tx,
        })
        .await
        .map_err(|_| ConfigError::InvalidValue {
            field: "node",
            reason: "owner unavailable",
        })?;
        reply_rx.await.map_err(|_| ConfigError::InvalidValue {
            field: "node",
            reason: "owner unavailable",
        })?
    }

    #[cfg(test)]
    pub(crate) async fn debug_staging_ttl(&self) -> Result<Duration, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::DebugStagingTtl { reply: reply_tx })
            .await?;
        receive(reply_rx).await
    }

    #[cfg(test)]
    pub(crate) async fn open_session(&self, shared_memory: bool) -> Result<u64, WorkerError> {
        self.open_session_with_write_release(shared_memory, false)
            .await
    }

    pub(crate) async fn open_session_with_write_release(
        &self,
        shared_memory: bool,
        supports_write_lease_release: bool,
    ) -> Result<u64, WorkerError> {
        // oneshot 两端类型由 Command 的 reply 字段和 receive() 返回值共同推导。
        let (reply_tx, reply_rx) = oneshot::channel();
        // 只把 Sender 移进 command；Receiver 仍留在当前 RPC Task。
        self.submit(NodeCommand::OpenSession {
            shared_memory,
            supports_write_lease_release,
            reply: reply_tx,
        })
        .await?;
        // await 挂起当前 Task；Node owner 完成 reply.send 后，该 Task 被 Tokio 唤醒。
        receive(reply_rx).await
    }

    pub(crate) fn fd_broker_path(&self) -> Option<PathBuf> {
        self.fd_broker_path.clone()
    }

    pub(crate) async fn acquire_region(
        &self,
        session_id: u64,
        region_id: u64,
    ) -> Result<HostRegionGrant, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::AcquireRegion {
            session_id,
            region_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn attach_session(
        &self,
        session_id: u64,
        sender: mpsc::Sender<NodeEvent>,
    ) -> Result<(), WorkerError> {
        // sender 本身可 clone，但这里把一个 producer 的所有权交给 NodeState 保存。
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::AttachSession {
            session_id,
            sender,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    #[cfg(test)]
    pub(crate) async fn heartbeat(
        &self,
        session_id: u64,
        released_view_through: Option<u64>,
    ) -> Result<(), WorkerError> {
        // 每个公开 Handle 方法都遵循同一模式：建回信通道→投递→等待匹配结果。
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::Heartbeat {
            session_id,
            released_view_through,
            released_write_allocations: Vec::new(),
            finished_read_request_through: None,
            renew_cache: false,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await.map(|_| ())
    }

    pub(crate) async fn heartbeat_with_write_releases(
        &self,
        session_id: u64,
        released_view_through: Option<u64>,
        released_write_allocations: Vec<ReleasedWriteAllocation>,
        finished_read_request_through: Option<u64>,
    ) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::Heartbeat {
            session_id,
            released_view_through,
            released_write_allocations,
            finished_read_request_through,
            renew_cache: false,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await.map(|_| ())
    }

    #[cfg(test)]
    pub(crate) async fn renew_cache_lease(
        &self,
        session_id: u64,
        released_view_through: Option<u64>,
    ) -> Result<u64, WorkerError> {
        self.renew_cache_lease_with_write_releases(
            session_id,
            released_view_through,
            Vec::new(),
            None,
        )
        .await
    }

    pub(crate) async fn renew_cache_lease_with_write_releases(
        &self,
        session_id: u64,
        released_view_through: Option<u64>,
        released_write_allocations: Vec<ReleasedWriteAllocation>,
        finished_read_request_through: Option<u64>,
    ) -> Result<u64, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::Heartbeat {
            session_id,
            released_view_through,
            released_write_allocations,
            finished_read_request_through,
            renew_cache: true,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn metadata_lease(
        &self,
        valid_until: Option<Instant>,
        watch_connected: Option<bool>,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::MetadataLease {
            valid_until,
            watch_connected,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn close_session(&self, session_id: u64) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::CloseSession {
            session_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn prepare_block_retirement(
        &self,
        retirement_id: Vec<u8>,
        block_ids: Vec<Vec<u8>>,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::PrepareBlockRetirement {
            retirement_id,
            block_ids,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn finalize_block_retirement(
        &self,
        retirement_id: Vec<u8>,
        block_ids: Vec<Vec<u8>>,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FinalizeBlockRetirement {
            retirement_id,
            block_ids,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn validate_session(&self, session_id: u64) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::ValidateSession {
            session_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    async fn begin_read_scope(&self, session_id: u64) -> Result<ReadScopeLease, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::BeginReadScope {
            session_id,
            cleanup_node: Box::new(self.clone()),
            reply,
        })
        .await?;
        receive(receiver).await
    }

    async fn finish_read_scope(&self, scope_id: u64) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FinishReadScope { scope_id, reply })
            .await?;
        receive(receiver).await
    }

    fn finish_read_scope_best_effort(&self, scope_id: u64) {
        let (reply, _receiver) = oneshot::channel();
        let command = NodeCommand::FinishReadScope { scope_id, reply };
        let name = command.metric();
        let queued = QueuedNodeCommand {
            name,
            enqueued_at: Instant::now(),
            trace_context: dms_tracing::capture_current_context(),
            command,
        };
        if self.command_tx.try_send(queued).is_err() {
            self.metrics.mailbox_send_failed();
        }
    }

    pub(crate) async fn stat(
        &self,
        session_id: u64,
        key: Vec<u8>,
    ) -> Result<pb::MetaStatResponse, WorkerError> {
        if key.is_empty() {
            return Err(WorkerError::InvalidArgument("key must not be empty"));
        }
        let metadata = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?;
        self.validate_session(session_id).await?;
        metadata.stat(key).await.map_err(map_metadata_error)
    }

    pub(crate) async fn scan(
        &self,
        session_id: u64,
        prefix: Vec<u8>,
        options: Option<pb::ObjectScanOptions>,
    ) -> Result<pb::MetaScanResponse, WorkerError> {
        let metadata = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?;
        self.validate_session(session_id).await?;
        metadata
            .scan(prefix, options)
            .await
            .map_err(map_metadata_error)
    }

    pub(crate) async fn acknowledge(
        &self,
        session_id: u64,
        sequence: u64,
    ) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::Acknowledge {
            session_id,
            sequence,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn allocate_staging(
        &self,
        session_id: u64,
        length: u64,
    ) -> Result<StagingAllocation, WorkerError> {
        // 元组返回 `(staging_id, transfer_id)`；两者作用不同，见 NodeState。
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::AllocateStaging {
            session_id,
            length,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn delete_staging(
        &self,
        session_id: u64,
        staging_id: u64,
    ) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::DeleteStaging {
            session_id,
            staging_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    /// Consumes one validated staging allocation into Node-owned bytes.
    ///
    /// Hash/KKV commands use this after the transport has filled staging. The
    /// public protocol remains allocate/upload/HSET, while the Hash service can
    /// atomically combine several field values into one versioned layout.
    pub(crate) async fn consume_staging(
        &self,
        session_id: u64,
        staging_id: u64,
        receipt: HostReceipt,
    ) -> Result<Vec<u8>, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::ConsumeStaging {
            session_id,
            staging_id,
            receipt,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn upload(
        &self,
        transfer_id: u64,
        bytes: Vec<u8>,
    ) -> Result<HostReceipt, WorkerError> {
        // Vec<u8> 按值传入并移动到 command；这一层不会再次复制 payload。
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::Upload {
            transfer_id,
            bytes,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn set(
        &self,
        session_id: u64,
        key: Vec<u8>,
        staging_id: u64,
        receipt: HostReceipt,
        operation_id: Vec<u8>,
        condition: String,
    ) -> Result<SetOutcome, WorkerError> {
        // key 同样按所有权移动，避免在 Handler→mailbox 边界 clone。
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::Set {
            session_id,
            key,
            staging_id,
            receipt,
            operation_id,
            condition,
            reply: reply_tx,
        })
        .await?;
        let outcome = receive(reply_rx).await?;
        if let Some(barrier_id) = outcome.barrier_id {
            self.wait_invalidation(barrier_id).await?;
        }
        Ok(outcome)
    }

    pub(crate) async fn mset(
        &self,
        session_id: u64,
        entries: Vec<(Vec<u8>, u64, HostReceipt)>,
        operation_id: Vec<u8>,
    ) -> Result<Vec<KeySetOutcome>, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::MSet {
            session_id,
            entries,
            operation_id,
            reply: reply_tx,
        })
        .await?;
        let outcome = receive(reply_rx).await?;
        for barrier_id in outcome.barrier_ids {
            self.wait_invalidation(barrier_id).await?;
        }
        Ok(outcome.versions)
    }

    pub(crate) async fn set_inline(
        &self,
        session_id: u64,
        key: Vec<u8>,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        condition: String,
    ) -> Result<SetOutcome, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::SetInline {
            session_id,
            key,
            bytes,
            operation_id,
            condition,
            reply: reply_tx,
        })
        .await?;
        let outcome = receive(reply_rx).await?;
        if let Some(barrier_id) = outcome.barrier_id {
            self.wait_invalidation(barrier_id).await?;
        }
        Ok(outcome)
    }

    pub(crate) async fn delete(
        &self,
        session_id: u64,
        key: Vec<u8>,
        operation_id: Vec<u8>,
    ) -> Result<DeleteOutcome, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::Delete {
            session_id,
            key,
            operation_id,
            reply: reply_tx,
        })
        .await?;
        let outcome = receive(reply_rx).await?;
        if let Some(barrier_id) = outcome.barrier_id {
            self.wait_invalidation(barrier_id).await?;
        }
        Ok(outcome)
    }

    /// Meta 事件消费 Task 通过这个入口把权威 Current 变化交还给 Node owner。
    pub(crate) async fn invalidate_current(
        &self,
        key: Vec<u8>,
        minimum_version: u64,
    ) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::InvalidateCurrent {
            key,
            minimum_version,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    async fn wait_invalidation(&self, barrier_id: u64) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::WaitInvalidation {
            barrier_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    #[allow(dead_code)]
    pub(crate) async fn get(
        &self,
        session_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
        range: Option<(u64, u64)>,
    ) -> Result<ReadTicket, WorkerError> {
        self.get_with_inline_limit(session_id, key, exact_version, range, 0)
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn get_with_inline_limit(
        &self,
        session_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
        range: Option<(u64, u64)>,
        max_inline_bytes: u64,
    ) -> Result<ReadTicket, WorkerError> {
        self.get_with_inline_limit_for_request(
            session_id,
            key,
            exact_version,
            range,
            false,
            max_inline_bytes,
            0,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "wire GET entrypoint keeps the opt-in range policy explicit beside the existing request identity and inline budget"
    )]
    pub(crate) async fn get_with_inline_limit_for_request(
        &self,
        session_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
        read_request_id: u64,
    ) -> Result<ReadTicket, WorkerError> {
        let scope = ReadScopeGuard::begin(self, session_id).await?;
        let result = self
            .get_with_inline_limit_scoped(
                session_id,
                scope.id(),
                read_request_id,
                key,
                exact_version,
                range,
                clamp_range,
                max_inline_bytes,
            )
            .await;
        let finish = scope.finish().await;
        match (result, finish) {
            (Ok(ticket), Ok(())) => Ok(ticket),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "read path keeps session, scope, request id, range, and inline budget explicit across the existing wire contract"
    )]
    async fn get_with_inline_limit_scoped(
        &self,
        session_id: u64,
        read_scope_id: u64,
        read_request_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
    ) -> Result<ReadTicket, WorkerError> {
        let metadata = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?;
        // 命中、Client 兴趣登记和失效处理共用同一 owner 顺序。
        // Exact 不缓存；Current 缺块优先使用同一次权威解析保存的位置提示。
        // 位置提示失败时只按固定版本 Exact 刷新，不能混用更新后的 Current 布局。
        let node_epoch = metadata.node_epoch().await;
        let refill_token = if exact_version.is_none() {
            let (reply, rx) = oneshot::channel();
            self.submit(NodeCommand::GetCached {
                session_id,
                read_scope_id,
                read_request_id,
                key: key.clone(),
                node_epoch,
                range,
                clamp_range,
                max_inline_bytes,
                reply,
            })
            .await?;
            let (token, cached) = receive(rx).await?;
            match cached {
                CachedReadOutcome::Ready(ticket) => return Ok(ticket),
                CachedReadOutcome::NeedsExactRefresh { version } => {
                    return self
                        .read_exact_version_after_cached_location_failure(
                            metadata,
                            session_id,
                            read_scope_id,
                            read_request_id,
                            key.clone(),
                            version,
                            range,
                            clamp_range,
                            max_inline_bytes,
                        )
                        .await;
                }
                CachedReadOutcome::NeedsRemoteBlocks { resolved, specs } => {
                    let cached_version = resolved
                        .layout
                        .as_ref()
                        .ok_or(WorkerError::NotFound)?
                        .version;
                    for spec in specs {
                        match self.ensure_peer_block(read_scope_id, spec).await {
                            Ok(()) => {}
                            Err(failure)
                                if failure.location_failure
                                    && can_refresh_cached_location(&failure.error) =>
                            {
                                // 只捕获 Peer 拉取阶段的位置失败。安装/登记失败不属于
                                // 换地址重试，必须直接返回，不能被下面的本地读掩盖。
                                return self
                                    .read_exact_version_after_cached_location_failure(
                                        metadata,
                                        session_id,
                                        read_scope_id,
                                        read_request_id,
                                        key.clone(),
                                        cached_version,
                                        range,
                                        clamp_range,
                                        max_inline_bytes,
                                    )
                                    .await;
                            }
                            Err(failure) => return Err(failure.error),
                        }
                    }
                    return self
                        .read_resolved_with_imports(
                            session_id,
                            read_scope_id,
                            read_request_id,
                            resolved,
                            range,
                            clamp_range,
                            max_inline_bytes,
                            None,
                        )
                        .await;
                }
                CachedReadOutcome::Miss => {}
            }
            token
        } else {
            None
        };
        let requested_at = Instant::now();
        let resolved = metadata
            .resolve(key.clone(), exact_version)
            .await
            .map_err(map_metadata_error)?;
        self.read_resolved_with_imports(
            session_id,
            read_scope_id,
            read_request_id,
            resolved,
            range,
            clamp_range,
            max_inline_bytes,
            refill_token.map(|token| (token, key, requested_at, node_epoch)),
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "retry path must carry the original read identity, range, and budget without hiding contract fields"
    )]
    async fn read_exact_version_after_cached_location_failure(
        &self,
        metadata: &MetadataClient,
        session_id: u64,
        read_scope_id: u64,
        read_request_id: u64,
        key: Vec<u8>,
        version: u64,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
    ) -> Result<ReadTicket, WorkerError> {
        let resolved = metadata
            .resolve(key, Some(version))
            .await
            .map_err(map_metadata_error)?;
        self.read_resolved_with_imports(
            session_id,
            read_scope_id,
            read_request_id,
            resolved,
            range,
            clamp_range,
            max_inline_bytes,
            None,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "import loop deliberately threads independent read identity, range, budget, and cache-refill authority"
    )]
    async fn read_resolved_with_imports(
        &self,
        session_id: u64,
        read_scope_id: u64,
        read_request_id: u64,
        resolved: pb::ResolveObjectResponse,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
        cache_refill: Option<(u64, Vec<u8>, Instant, u64)>,
    ) -> Result<ReadTicket, WorkerError> {
        // Each pass imports every currently missing immutable Block outside
        // the Node actor. The second pass only builds local read tickets.
        for _ in 0..2 {
            let (reply_tx, reply_rx) = oneshot::channel();
            self.submit(NodeCommand::GetResolved {
                session_id,
                read_scope_id,
                read_request_id,
                resolved: resolved.clone(),
                range,
                clamp_range,
                max_inline_bytes,
                cache_refill: cache_refill.clone(),
                reply: reply_tx,
            })
            .await?;
            match receive(reply_rx).await? {
                GetOutcome::Ready(ticket) => return Ok(ticket),
                GetOutcome::NeedsRemoteBlocks(specs) => {
                    for spec in specs {
                        self.ensure_peer_block(read_scope_id, spec)
                            .await
                            .map_err(|failure| failure.error)?;
                    }
                }
            }
        }
        Err(WorkerError::NotFound)
    }

    /// Resolves and materializes an immutable object entirely inside dms-node.
    ///
    /// This is intentionally distinct from the Client read-ticket path: an
    /// internal service such as Hash/KKV needs bytes for read-modify-CAS even
    /// when the caller negotiated SHM. Remote blocks are still pulled outside
    /// the single Node state owner, so the actor never waits on network I/O.
    pub(crate) async fn get_materialized(
        &self,
        session_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<(u64, Vec<u8>), WorkerError> {
        let scope = ReadScopeGuard::begin(self, session_id).await?;
        let result = self
            .get_materialized_scoped(session_id, scope.id(), key, exact_version)
            .await;
        let finish = scope.finish().await;
        match (result, finish) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    async fn get_materialized_scoped(
        &self,
        session_id: u64,
        read_scope_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<(u64, Vec<u8>), WorkerError> {
        let metadata = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?;
        let resolved = metadata
            .resolve(key, exact_version)
            .await
            .map_err(map_metadata_error)?;
        for _ in 0..2 {
            let (reply_tx, reply_rx) = oneshot::channel();
            self.submit(NodeCommand::MaterializeResolved {
                session_id,
                read_scope_id,
                resolved: resolved.clone(),
                reply: reply_tx,
            })
            .await?;
            match receive(reply_rx).await? {
                MaterializeOutcome::Ready { version, bytes } => return Ok((version, bytes)),
                MaterializeOutcome::NeedsRemoteBlocks(specs) => {
                    for spec in specs {
                        self.ensure_peer_block(read_scope_id, spec)
                            .await
                            .map_err(|failure| failure.error)?;
                    }
                }
            }
        }
        Err(WorkerError::NotFound)
    }

    /// Current、Exact、内部 materialize 共用的 Node 级缺块导入入口。
    /// 请求取消只丢弃自己的等待，owner 启动的有界任务仍负责完成 Import/Report。
    async fn ensure_peer_block(
        &self,
        read_scope_id: u64,
        spec: PeerPullSpec,
    ) -> Result<(), PeerImportFailure> {
        let (reply, rx) = oneshot::channel();
        self.submit(NodeCommand::EnsurePeerBlock {
            read_scope_id,
            spec,
            node: Box::new(self.clone()),
            reply,
        })
        .await
        .map_err(PeerImportFailure::terminal)?;
        rx.await
            .map_err(|_| PeerImportFailure::terminal(WorkerError::WorkerUnavailable))?
    }

    // 完整性测试直接检验一次未合并的导入；正式读入口一律使用 ensure_peer_block。
    #[cfg(test)]
    async fn import_and_report_peer_block(
        &self,
        metadata: &MetadataClient,
        operation_namespace: &[u8],
        read_scope_id: u64,
        spec: PeerPullSpec,
    ) -> Result<(), WorkerError> {
        self.pull_and_report_peer_block(metadata, operation_namespace, read_scope_id, spec, None)
            .await
            .map_err(|failure| failure.error)
    }

    /// Installs an immutable peer Block locally, then publishes the local
    /// replica identity to Meta. Both Client reads and internal materialized
    /// reads use this exact state transition; keeping it in one place prevents
    /// the two paths from drifting on idempotency or replica policy.
    async fn pull_and_report_peer_block(
        &self,
        metadata: &MetadataClient,
        operation_namespace: &[u8],
        read_scope_id: u64,
        spec: PeerPullSpec,
        import_attempt: Option<u64>,
    ) -> Result<(), PeerImportFailure> {
        // 接收成功以“完整数据通过owner接纳”为边界；不能在网络收到响应时
        // 提前记成功，否则延后的完整checksum拒绝会被错误统计为成功传输。
        let metric = self.metrics.begin_replica_operation(ReplicaOperation::Pull);
        let payload =
            pull_block_from_peer(&self.node_id, spec, &self.rpc_metrics, &self.peer_channels)
                .await
                .map_err(|error| PeerImportFailure {
                    error,
                    location_failure: true,
                })?;
        self.install_and_report_peer_block(
            metadata,
            operation_namespace,
            read_scope_id,
            payload,
            metric,
            import_attempt,
        )
        .await
        .map_err(PeerImportFailure::terminal)
    }

    /// 缓存位置和权威位置的读共用接纳与登记逻辑；本函数的错误不能触发位置回退。
    async fn install_and_report_peer_block(
        &self,
        metadata: &MetadataClient,
        operation_namespace: &[u8],
        read_scope_id: u64,
        payload: PeerBlockResult,
        mut metric: super::metrics::ReplicaOperationGuard,
        import_attempt: Option<u64>,
    ) -> Result<(), WorkerError> {
        let received_bytes = payload.payload.len();
        let report_block_id = payload.block_id.clone();
        let report_checksum = payload.checksum.clone();
        let report_length = payload.length;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::ImportPeerBlock {
            block_id: payload.block_id,
            bytes: payload.payload,
            checksum: payload.checksum,
            length: payload.length,
            read_scope_id,
            import_attempt,
            reply: reply_tx,
        })
        .await?;
        let report_replica = receive(reply_rx).await?;
        metric.success_with_payload(ReplicaDirection::Receive, received_bytes);
        // 该耗时包括拉取、完整校验和本地安装，不包括下面的Meta位置登记。
        drop(metric);
        if !report_replica {
            return Ok(());
        }

        let mut report_operation = operation_namespace.to_vec();
        report_operation.extend_from_slice(&report_block_id);
        // Report belongs to this Node incarnation. Including node_epoch keeps
        // a restart from hitting an idempotency result created by an old Node.
        report_operation.extend_from_slice(&metadata.node_epoch().await.to_be_bytes());
        metadata
            .report_replica(
                report_block_id,
                report_length,
                report_checksum,
                digest(&report_operation),
                2,
                Vec::new(),
            )
            .await
            .map_err(map_metadata_error)?;
        Ok(())
    }

    pub(crate) async fn set_range(&self, input: SetRangeInput) -> Result<SetOutcome, WorkerError> {
        let resolved = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?
            .resolve(input.key.clone(), input.expected_version)
            .await
            .map_err(map_metadata_error)?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::SetRange {
            input,
            resolved,
            reply: reply_tx,
        })
        .await?;
        let outcome = receive(reply_rx).await?;
        if let Some(barrier_id) = outcome.barrier_id {
            self.wait_invalidation(barrier_id).await?;
        }
        Ok(outcome)
    }

    pub(crate) async fn download(&self, transfer_id: u64) -> Result<Vec<u8>, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::Download {
            transfer_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn probe(
        &self,
        source_node_id: String,
        nonce: Vec<u8>,
    ) -> Result<PeerProbeResult, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::PeerProbe {
            source_node_id,
            nonce,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    #[cfg(test)]
    pub(crate) async fn debug_commit_for_peer_test(
        &self,
        session_id: u64,
        staging_id: u64,
        receipt: HostReceipt,
        block_id: Vec<u8>,
    ) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::DebugCommitForPeerTest {
            session_id,
            staging_id,
            receipt,
            block_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn pull_block(
        &self,
        source_node_id: String,
        block_id: Vec<u8>,
        range: Option<(u64, u64)>,
    ) -> Result<PeerBlockResult, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::PeerPullBlock {
            source_node_id,
            block_id,
            range,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn prepare_replica(
        &self,
        spec: ReplicaPrepareSpec,
    ) -> Result<ReplicaStateView, WorkerError> {
        if spec.plan_id.is_empty() || spec.source_endpoint.is_empty() || spec.block_id.is_empty() {
            return Err(WorkerError::InvalidArgument(
                "plan id, source endpoint and block id are required",
            ));
        }
        let payload = pull_block_from_peer(
            &spec.source_node_id,
            PeerPullSpec {
                endpoint: spec.source_endpoint.clone(),
                block_id: spec.block_id.clone(),
                expected_checksum: spec.expected_checksum.clone(),
                expected_length: spec.expected_length,
            },
            &self.rpc_metrics,
            &self.peer_channels,
        )
        .await?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::PrepareReplica {
            plan_id: spec.plan_id,
            block_id: payload.block_id,
            bytes: payload.payload,
            checksum: payload.checksum,
            length: payload.length,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn activate_replica(
        &self,
        plan_id: Vec<u8>,
    ) -> Result<ReplicaStateView, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::ActivateReplica {
            plan_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn abort_replica(
        &self,
        plan_id: Vec<u8>,
    ) -> Result<ReplicaStateView, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::AbortReplica {
            plan_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    /// Drops an active repair attempt that Meta refused to publish.
    ///
    /// Unlike the peer-facing `abort_replica`, this internal operation is
    /// allowed to remove an active-but-unpublished Block because its caller
    /// owns the `activate -> report` ordering.
    pub(crate) async fn discard_replica_attempt(
        &self,
        plan_id: Vec<u8>,
    ) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::DiscardReplicaAttempt {
            plan_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    pub(crate) async fn replica_status(
        &self,
        plan_id: Vec<u8>,
    ) -> Result<ReplicaStateView, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::ReplicaStatus {
            plan_id,
            reply: reply_tx,
        })
        .await?;
        receive(reply_rx).await
    }

    async fn submit(&self, command: NodeCommand) -> Result<(), WorkerError> {
        // send().await 在队列有容量时立即完成；队列满时挂起当前 RPC Task。
        // Receiver 被关闭说明 Node 状态 owner 已退出，此时映射为 WorkerUnavailable。
        self.metrics.mailbox_enqueued();
        self.command_tx
            .send(QueuedNodeCommand {
                name: command.metric(),
                enqueued_at: Instant::now(),
                trace_context: dms_tracing::capture_current_context(),
                command,
            })
            .await
            .map_err(|_| {
                self.metrics.mailbox_send_failed();
                WorkerError::WorkerUnavailable
            })
    }
}

async fn receive<T>(reply_rx: oneshot::Receiver<Result<T, WorkerError>>) -> Result<T, WorkerError> {
    // 外层 Result 是 oneshot 是否还存在；内层 Result 是实际业务成功或 WorkerError。
    // 第一个 `?` 解开“通道结果”，函数返回表达式再把“业务结果”原样交给调用者。
    reply_rx.await.map_err(|_| WorkerError::WorkerUnavailable)?
}

/// Node owner 能处理的全部强类型命令。
///
/// enum 而不是 `AnyMessage + method_id`，因此每个分支的参数和返回类型都由编译器检查。
// owner 返回：在途查询的回填围栏，以及可直接返回的读取票据（未命中时为空）。
type CachedRead = (Option<u64>, CachedReadOutcome);

enum NodeCommand {
    OpenSession {
        shared_memory: bool,
        supports_write_lease_release: bool,
        reply: oneshot::Sender<Result<u64, WorkerError>>,
    },
    AttachSession {
        session_id: u64,
        /// NodeEvent 的发送端；接收端在 gRPC Session stream Task 中。
        sender: mpsc::Sender<NodeEvent>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    Heartbeat {
        session_id: u64,
        released_view_through: Option<u64>,
        released_write_allocations: Vec<ReleasedWriteAllocation>,
        finished_read_request_through: Option<u64>,
        renew_cache: bool,
        reply: oneshot::Sender<Result<u64, WorkerError>>,
    },
    MetadataLease {
        valid_until: Option<Instant>,
        watch_connected: Option<bool>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    CloseSession {
        session_id: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    ValidateSession {
        session_id: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    Acknowledge {
        session_id: u64,
        sequence: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    AllocateStaging {
        session_id: u64,
        length: u64,
        reply: oneshot::Sender<Result<StagingAllocation, WorkerError>>,
    },
    AcquireRegion {
        session_id: u64,
        region_id: u64,
        reply: oneshot::Sender<Result<HostRegionGrant, WorkerError>>,
    },
    DeleteStaging {
        session_id: u64,
        staging_id: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    ConsumeStaging {
        session_id: u64,
        staging_id: u64,
        receipt: HostReceipt,
        reply: oneshot::Sender<Result<Vec<u8>, WorkerError>>,
    },
    Upload {
        transfer_id: u64,
        /// payload 所有权随 command 一起进入状态 owner。
        bytes: Vec<u8>,
        reply: oneshot::Sender<Result<HostReceipt, WorkerError>>,
    },
    Set {
        session_id: u64,
        key: Vec<u8>,
        staging_id: u64,
        receipt: HostReceipt,
        operation_id: Vec<u8>,
        /// SDK 的写条件保持字符串 wire 表达，最终由 Meta 权威校验。
        condition: String,
        reply: oneshot::Sender<Result<SetOutcome, WorkerError>>,
    },
    MSet {
        session_id: u64,
        entries: Vec<(Vec<u8>, u64, HostReceipt)>,
        operation_id: Vec<u8>,
        reply: oneshot::Sender<Result<MSetOutcome, WorkerError>>,
    },
    SetInline {
        session_id: u64,
        key: Vec<u8>,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        condition: String,
        reply: oneshot::Sender<Result<SetOutcome, WorkerError>>,
    },
    SetRange {
        input: SetRangeInput,
        resolved: pb::ResolveObjectResponse,
        reply: oneshot::Sender<Result<SetOutcome, WorkerError>>,
    },
    Delete {
        session_id: u64,
        key: Vec<u8>,
        operation_id: Vec<u8>,
        reply: oneshot::Sender<Result<DeleteOutcome, WorkerError>>,
    },
    GetResolved {
        session_id: u64,
        read_scope_id: u64,
        read_request_id: u64,
        resolved: pb::ResolveObjectResponse,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
        cache_refill: Option<(u64, Vec<u8>, Instant, u64)>,
        reply: oneshot::Sender<Result<GetOutcome, WorkerError>>,
    },
    GetCached {
        session_id: u64,
        read_scope_id: u64,
        read_request_id: u64,
        key: Vec<u8>,
        node_epoch: u64,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
        reply: oneshot::Sender<Result<CachedRead, WorkerError>>,
    },
    MaterializeResolved {
        session_id: u64,
        read_scope_id: u64,
        resolved: pb::ResolveObjectResponse,
        reply: oneshot::Sender<Result<MaterializeOutcome, WorkerError>>,
    },
    ImportPeerBlock {
        block_id: Vec<u8>,
        bytes: Vec<u8>,
        checksum: Vec<u8>,
        length: u64,
        read_scope_id: u64,
        import_attempt: Option<u64>,
        reply: oneshot::Sender<Result<bool, WorkerError>>,
    },
    EnsurePeerBlock {
        read_scope_id: u64,
        spec: PeerPullSpec,
        node: Box<NodeHandle>,
        reply: oneshot::Sender<Result<(), PeerImportFailure>>,
    },
    Download {
        transfer_id: u64,
        reply: oneshot::Sender<Result<Vec<u8>, WorkerError>>,
    },
    PeerProbe {
        source_node_id: String,
        nonce: Vec<u8>,
        reply: oneshot::Sender<Result<PeerProbeResult, WorkerError>>,
    },
    PeerPullBlock {
        source_node_id: String,
        block_id: Vec<u8>,
        range: Option<(u64, u64)>,
        reply: oneshot::Sender<Result<PeerBlockResult, WorkerError>>,
    },
    PrepareReplica {
        plan_id: Vec<u8>,
        block_id: Vec<u8>,
        bytes: Vec<u8>,
        checksum: Vec<u8>,
        length: u64,
        reply: oneshot::Sender<Result<ReplicaStateView, WorkerError>>,
    },
    ActivateReplica {
        plan_id: Vec<u8>,
        reply: oneshot::Sender<Result<ReplicaStateView, WorkerError>>,
    },
    AbortReplica {
        plan_id: Vec<u8>,
        reply: oneshot::Sender<Result<ReplicaStateView, WorkerError>>,
    },
    DiscardReplicaAttempt {
        plan_id: Vec<u8>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    ReplicaStatus {
        plan_id: Vec<u8>,
        reply: oneshot::Sender<Result<ReplicaStateView, WorkerError>>,
    },
    #[cfg(test)]
    DebugCommitForPeerTest {
        session_id: u64,
        staging_id: u64,
        receipt: HostReceipt,
        block_id: Vec<u8>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    InvalidateCurrent {
        key: Vec<u8>,
        minimum_version: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    PrepareBlockRetirement {
        retirement_id: Vec<u8>,
        block_ids: Vec<Vec<u8>>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FinalizeBlockRetirement {
        retirement_id: Vec<u8>,
        block_ids: Vec<Vec<u8>>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    BeginReadScope {
        session_id: u64,
        cleanup_node: Box<NodeHandle>,
        reply: oneshot::Sender<Result<ReadScopeLease, WorkerError>>,
    },
    FinishReadScope {
        scope_id: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    WaitInvalidation {
        barrier_id: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    #[allow(dead_code)]
    ApplyConfigChange {
        change: ConfigChange,
        reply: oneshot::Sender<Result<u64, ConfigError>>,
    },
    #[cfg(test)]
    DebugStagingTtl {
        reply: oneshot::Sender<Result<Duration, WorkerError>>,
    },
    #[cfg(test)]
    DebugPeerImports {
        reply: oneshot::Sender<(usize, usize, u64)>,
    },
}

struct QueuedNodeCommand {
    name: NodeMailboxCommand,
    enqueued_at: Instant,
    /// Captured before crossing the actor mailbox; never placed in payload bytes.
    trace_context: dms_tracing::TraceContext,
    command: NodeCommand,
}

impl NodeCommand {
    fn metric(&self) -> NodeMailboxCommand {
        match self {
            Self::OpenSession { .. } => NodeMailboxCommand::OpenSession,
            Self::AttachSession { .. } => NodeMailboxCommand::AttachSession,
            Self::Heartbeat { .. } => NodeMailboxCommand::Heartbeat,
            Self::MetadataLease { .. } => NodeMailboxCommand::Heartbeat,
            Self::CloseSession { .. } => NodeMailboxCommand::CloseSession,
            Self::ValidateSession { .. } => NodeMailboxCommand::ValidateSession,
            Self::Acknowledge { .. } => NodeMailboxCommand::Acknowledge,
            Self::AllocateStaging { .. } => NodeMailboxCommand::AllocateStaging,
            Self::AcquireRegion { .. } => NodeMailboxCommand::AcquireRegion,
            Self::DeleteStaging { .. } => NodeMailboxCommand::DeleteStaging,
            Self::ConsumeStaging { .. } => NodeMailboxCommand::ConsumeStaging,
            Self::Upload { .. } => NodeMailboxCommand::Upload,
            Self::Set { .. } => NodeMailboxCommand::Set,
            Self::MSet { .. } => NodeMailboxCommand::MSet,
            Self::SetInline { .. } => NodeMailboxCommand::SetInline,
            Self::SetRange { .. } => NodeMailboxCommand::SetRange,
            Self::Delete { .. } => NodeMailboxCommand::Delete,
            Self::GetResolved { .. } => NodeMailboxCommand::GetResolved,
            Self::GetCached { .. } => NodeMailboxCommand::GetCached,
            Self::MaterializeResolved { .. } => NodeMailboxCommand::MaterializeResolved,
            Self::ImportPeerBlock { .. } => NodeMailboxCommand::ImportPeerBlock,
            Self::EnsurePeerBlock { .. } => NodeMailboxCommand::ImportPeerBlock,
            Self::Download { .. } => NodeMailboxCommand::Download,
            Self::PeerProbe { .. } => NodeMailboxCommand::PeerProbe,
            Self::PeerPullBlock { .. } => NodeMailboxCommand::PeerPullBlock,
            Self::PrepareReplica { .. } => NodeMailboxCommand::PrepareReplica,
            Self::ActivateReplica { .. } => NodeMailboxCommand::ActivateReplica,
            Self::AbortReplica { .. } => NodeMailboxCommand::AbortReplica,
            Self::DiscardReplicaAttempt { .. } => NodeMailboxCommand::DiscardReplicaAttempt,
            Self::ReplicaStatus { .. } => NodeMailboxCommand::ReplicaStatus,
            #[cfg(test)]
            Self::DebugCommitForPeerTest { .. } => NodeMailboxCommand::DebugCommitForPeerTest,
            Self::InvalidateCurrent { .. } => NodeMailboxCommand::InvalidateCurrent,
            Self::PrepareBlockRetirement { .. } => NodeMailboxCommand::InvalidateCurrent,
            Self::FinalizeBlockRetirement { .. } => NodeMailboxCommand::InvalidateCurrent,
            Self::BeginReadScope { .. } => NodeMailboxCommand::GetCached,
            Self::FinishReadScope { .. } => NodeMailboxCommand::GetCached,
            Self::WaitInvalidation { .. } => NodeMailboxCommand::WaitInvalidation,
            Self::ApplyConfigChange { .. } => NodeMailboxCommand::ApplyConfigChange,
            #[cfg(test)]
            Self::DebugStagingTtl { .. } => NodeMailboxCommand::DebugStagingTtl,
            #[cfg(test)]
            Self::DebugPeerImports { .. } => NodeMailboxCommand::GetCached,
        }
    }
}

/// 唯一允许修改 R1 Node 状态的 Task。
async fn run_node(
    node_id: String,
    metadata: Option<MetadataClient>,
    task_config: NodeTaskConfig,
    metrics: NodeMetrics,
    mut command_rx: mpsc::Receiver<QueuedNodeCommand>,
) {
    let trace_periodic_operations = task_config.trace_periodic_operations;
    // NodeState 是普通非线程安全结构，因为它从始至终只属于当前 Task。
    let mut state = NodeState::with_metrics(node_id, metadata, task_config, metrics.clone());
    let mut maintenance = tokio::time::interval(Duration::from_secs(1));
    let mut writes = tokio::task::JoinSet::<ApplyWrite>::new();
    // owner 保管表和任务集合；Future 只做网络等待，并通过原 Import 命令交回 bytes。
    let mut peer_imports = tokio::task::JoinSet::<PeerImportCompletion>::new();
    loop {
        // recv().await 在队列为空时挂起；maintenance tick 同样在这个唯一 owner
        // 内执行，因此回收不会引入第二个 Arena 状态入口。
        let next_cache_expiry = state.next_cache_lease_expiry();
        let queued = tokio::select! {
            completed = peer_imports.join_next_with_id(), if !peer_imports.is_empty() => {
                match completed {
                    Some(Ok((_, (block_id, attempt, result)))) => {
                        state.complete_peer_import(&block_id, attempt, result);
                    }
                    Some(Err(error)) => state.fail_peer_import_task(error.id()),
                    None => {}
                }
                continue;
            }
            completed = writes.join_next(), if !writes.is_empty() => {
                if let Some(Ok(apply)) = completed { apply(&mut state); }
                continue;
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    break;
                };
                command
            }
            _ = maintenance.tick() => {
                state.tick();
                continue;
            }
            _ = async {
                if let Some(deadline) = next_cache_expiry {
                    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                }
            }, if next_cache_expiry.is_some() => {
                state.expire_cache_leases();
                continue;
            }
        };
        metrics.record_mailbox_receive(queued.name, queued.enqueued_at);
        let span = if !dms_tracing::tracing::enabled!(dms_tracing::tracing::Level::DEBUG)
            || (queued.name.is_periodic() && !trace_periodic_operations)
        {
            dms_tracing::tracing::Span::none()
        } else {
            let operation_name = format!("dms.node.{}", queued.name.label());
            dms_tracing::tracing::debug_span!(
                "dms.node.command",
                otel.name = operation_name.as_str(),
                otel.kind = "internal",
                command = ?queued.name,
            )
        };
        dms_tracing::set_parent(&span, &queued.trace_context);
        let command_name = queued.name;
        let command = queued.command;
        // match 会消费 command，使各分支直接取得其中字段的所有权。
        async {
            match command {
                #[cfg(test)]
                NodeCommand::DebugPeerImports { reply } => {
                    let _ = reply.send((
                        state.peer_imports.len(),
                        state.peer_import_failures.len(),
                        state.peer_import_bytes,
                    ));
                }
                NodeCommand::OpenSession {
                    shared_memory,
                    supports_write_lease_release,
                    reply,
                } => {
                    // Client 可能已经取消 RPC，此时 send 返回 Err；业务已经执行，所以忽略它。
                    let _ = reply.send(Ok(state.open_session_with_write_release(
                        shared_memory,
                        supports_write_lease_release,
                    )));
                }
                NodeCommand::AttachSession {
                    session_id,
                    sender,
                    reply,
                } => {
                    let _ = reply.send(state.attach_session(session_id, sender));
                }
                NodeCommand::Heartbeat {
                    session_id,
                    released_view_through,
                    released_write_allocations,
                    finished_read_request_through,
                    renew_cache,
                    reply,
                } => {
                    let _ = reply.send(state.heartbeat_inner(
                        session_id,
                        released_view_through,
                        released_write_allocations,
                        finished_read_request_through,
                        renew_cache,
                    ));
                }
                NodeCommand::MetadataLease {
                    valid_until,
                    watch_connected,
                    reply,
                } => {
                    if let Some(until) = valid_until {
                        state.metadata_lease_until = until;
                    }
                    if let Some(connected) = watch_connected {
                        // 即便重连前后都为 true，也撤销旧响应的回填资格。
                        state.current_cache.clear();
                        state.metrics.set_current_cache_charge(0);
                        state.metadata_watch_connected = connected;
                    }
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::CloseSession { session_id, reply } => {
                    let _ = reply.send(state.close_session(session_id));
                }
                NodeCommand::ValidateSession { session_id, reply } => {
                    let _ = reply.send(state.live_session(session_id).map(|_| ()));
                }
                NodeCommand::Acknowledge {
                    session_id,
                    sequence,
                    reply,
                } => {
                    let _ = reply.send(state.acknowledge(session_id, sequence));
                }
                NodeCommand::AllocateStaging {
                    session_id,
                    length,
                    reply,
                } => {
                    let _ = reply.send(state.allocate_staging(session_id, length));
                }
                NodeCommand::AcquireRegion {
                    session_id,
                    region_id,
                    reply,
                } => {
                    let _ = reply.send(state.acquire_region(session_id, region_id));
                }
                NodeCommand::DeleteStaging {
                    session_id,
                    staging_id,
                    reply,
                } => {
                    let _ = reply.send(state.delete_staging(session_id, staging_id));
                }
                NodeCommand::ConsumeStaging {
                    session_id,
                    staging_id,
                    receipt,
                    reply,
                } => {
                    let _ = reply.send((|| {
                        let receipt = state.receipt_for_session(session_id, receipt)?;
                        state
                            .arena
                            .take_staging_bytes(session_id, staging_id, &receipt)
                            .map_err(map_arena_error)
                    })());
                }
                NodeCommand::Upload {
                    transfer_id,
                    bytes,
                    reply,
                } => {
                    let _ = reply.send(state.upload(transfer_id, bytes));
                }
                NodeCommand::Set {
                    session_id,
                    key,
                    staging_id,
                    receipt,
                    operation_id,
                    condition,
                    reply,
                } => {
                    launch_write(&mut state, &mut writes, reply, |state| {
                        state.set(
                            session_id,
                            key,
                            staging_id,
                            receipt,
                            operation_id,
                            condition,
                        )
                    });
                }
                NodeCommand::MSet {
                    session_id,
                    entries,
                    operation_id,
                    reply,
                } => {
                    launch_write(&mut state, &mut writes, reply, |state| {
                        state.mset(session_id, entries, operation_id)
                    });
                }
                NodeCommand::SetInline {
                    session_id,
                    key,
                    bytes,
                    operation_id,
                    condition,
                    reply,
                } => {
                    launch_write(&mut state, &mut writes, reply, |state| {
                        state.commit_bytes(session_id, key, bytes, operation_id, condition)
                    });
                }
                NodeCommand::SetRange {
                    input,
                    resolved,
                    reply,
                } => {
                    launch_write(&mut state, &mut writes, reply, |state| {
                        state.set_range(input, resolved)
                    });
                }
                NodeCommand::Delete {
                    session_id,
                    key,
                    operation_id,
                    reply,
                } => {
                    launch_write(&mut state, &mut writes, reply, |state| {
                        state.delete(session_id, key, operation_id)
                    });
                }
                NodeCommand::GetResolved {
                    session_id,
                    read_scope_id,
                    read_request_id,
                    resolved,
                    range,
                    clamp_range,
                    max_inline_bytes,
                    cache_refill,
                    reply,
                } => {
                    if reply.is_closed() {
                        state.finish_read_scope(read_scope_id);
                        return;
                    }
                    let result = state.get_resolved_for_request(
                        session_id,
                        read_scope_id,
                        read_request_id,
                        &resolved,
                        range,
                        clamp_range,
                        max_inline_bytes,
                    );
                    if matches!(&result, Ok(GetOutcome::Ready(_)))
                        && state.metadata_watch_connected
                        && Instant::now() < state.metadata_lease_until
                        && !state.layout_contains_retiring_block(&resolved)
                        && let Some((token, key, requested_at, node_epoch)) = cache_refill
                    {
                        state.current_cache.insert(
                            token,
                            key,
                            &resolved,
                            requested_at,
                            node_epoch,
                            Instant::now(),
                        );
                        state
                            .metrics
                            .set_current_cache_charge(state.current_cache.charged());
                    }
                    let _ = reply.send(result);
                }
                NodeCommand::GetCached {
                    session_id,
                    read_scope_id,
                    read_request_id,
                    key,
                    node_epoch,
                    range,
                    clamp_range,
                    max_inline_bytes,
                    reply,
                } => {
                    if reply.is_closed() {
                        state.finish_read_scope(read_scope_id);
                        return;
                    }
                    let result = state.get_cached(
                        session_id,
                        read_scope_id,
                        read_request_id,
                        &key,
                        node_epoch,
                        range,
                        clamp_range,
                        max_inline_bytes,
                    );
                    let _ = reply.send(result);
                }
                NodeCommand::MaterializeResolved {
                    session_id,
                    read_scope_id,
                    resolved,
                    reply,
                } => {
                    if reply.is_closed() {
                        state.finish_read_scope(read_scope_id);
                        return;
                    }
                    let _ = reply.send(state.materialize_resolved(
                        session_id,
                        read_scope_id,
                        &resolved,
                    ));
                }
                NodeCommand::EnsurePeerBlock {
                    read_scope_id,
                    spec,
                    node,
                    reply,
                } => {
                    if let Some(attempt) = state.begin_peer_import(read_scope_id, &spec, reply) {
                        let block_id = spec.block_id.clone();
                        let task_block_id = block_id.clone();
                        let task = peer_imports.spawn(
                            async move {
                                let result = match node.metadata.as_ref() {
                                    Some(metadata) => {
                                        node.pull_and_report_peer_block(
                                            metadata,
                                            b"cache-import/",
                                            read_scope_id,
                                            spec,
                                            Some(attempt),
                                        )
                                        .await
                                    }
                                    None => Err(PeerImportFailure::terminal(
                                        WorkerError::MetadataUnavailable,
                                    )),
                                };
                                (task_block_id, attempt, result)
                            }
                            .instrument(dms_tracing::tracing::Span::current()),
                        );
                        state
                            .peer_imports
                            .get_mut(&block_id)
                            .expect("new peer import")
                            .task_id = Some(task.id());
                    }
                }
                NodeCommand::ImportPeerBlock {
                    block_id,
                    bytes,
                    checksum,
                    length,
                    read_scope_id,
                    import_attempt,
                    reply,
                } => {
                    // 任务已结束/取消时，迟到的安装命令不能越过已解除的 GC pin。
                    // None 仅供直接检验原始导入的完整性测试使用。
                    if let Some(attempt) = import_attempt
                        && let Err(error) =
                            state.validate_peer_import_attempt(&block_id, read_scope_id, attempt)
                    {
                        let _ = reply.send(Err(error));
                        return;
                    }
                    let _ = reply.send(state.import_peer_block(
                        read_scope_id,
                        block_id,
                        bytes,
                        checksum,
                        length,
                    ));
                }
                NodeCommand::Download { transfer_id, reply } => {
                    let _ = reply.send(state.download(transfer_id));
                }
                NodeCommand::PeerProbe {
                    source_node_id,
                    nonce,
                    reply,
                } => {
                    let _ = reply.send(state.probe(source_node_id, nonce));
                }
                NodeCommand::PeerPullBlock {
                    source_node_id,
                    block_id,
                    range,
                    reply,
                } => {
                    let _ = reply.send(state.pull_block(source_node_id, block_id, range));
                }
                NodeCommand::PrepareReplica {
                    plan_id,
                    block_id,
                    bytes,
                    checksum,
                    length,
                    reply,
                } => {
                    let _ = reply
                        .send(state.prepare_replica(plan_id, block_id, bytes, checksum, length));
                }
                NodeCommand::ActivateReplica { plan_id, reply } => {
                    let _ = reply.send(state.activate_replica(plan_id));
                }
                NodeCommand::AbortReplica { plan_id, reply } => {
                    let _ = reply.send(state.abort_replica(plan_id));
                }
                NodeCommand::DiscardReplicaAttempt { plan_id, reply } => {
                    let _ = reply.send(state.discard_replica_attempt(plan_id));
                }
                NodeCommand::ReplicaStatus { plan_id, reply } => {
                    let _ = reply.send(state.replica_status(&plan_id));
                }
                #[cfg(test)]
                NodeCommand::DebugCommitForPeerTest {
                    session_id,
                    staging_id,
                    receipt,
                    block_id,
                    reply,
                } => {
                    let _ = reply.send(
                        state.debug_commit_for_peer_test(session_id, staging_id, receipt, block_id),
                    );
                }
                NodeCommand::InvalidateCurrent {
                    key,
                    minimum_version,
                    reply,
                } => {
                    if let Some(barrier_id) = state.broadcast_invalidation(key, minimum_version) {
                        state.attach_barrier_waiter(barrier_id, reply);
                    } else {
                        let _ = reply.send(Ok(()));
                    }
                }
                NodeCommand::PrepareBlockRetirement {
                    retirement_id,
                    block_ids,
                    reply,
                } => {
                    state.register_prepare_retirement(retirement_id, block_ids, reply);
                }
                NodeCommand::FinalizeBlockRetirement {
                    retirement_id,
                    block_ids,
                    reply,
                } => {
                    state.register_final_retirement(retirement_id, block_ids, reply);
                }
                NodeCommand::BeginReadScope {
                    session_id,
                    cleanup_node,
                    reply,
                } => {
                    state.begin_read_scope_reply(session_id, *cleanup_node, reply);
                }
                NodeCommand::FinishReadScope { scope_id, reply } => {
                    state.finish_read_scope(scope_id);
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::WaitInvalidation { barrier_id, reply } => {
                    state.attach_barrier_waiter(barrier_id, reply);
                }
                NodeCommand::ApplyConfigChange { change, reply } => {
                    let _ = reply.send(state.apply_config_change(change));
                }
                #[cfg(test)]
                NodeCommand::DebugStagingTtl { reply } => {
                    let _ = reply.send(Ok(state.arena.staging_ttl()));
                }
            }
            dms_logging::debug!(
                "node command completed";
                "event" => "node.command.completed",
                "command" => format!("{command_name:?}"),
            );
        }
        .instrument(span)
        .await;
    }
}

/// 一个 Client 与 Node 之间的逻辑会话状态。
struct Session {
    /// Client session incarnation；旧 stream/ACK 不能作用于重开的 session。
    epoch: u64,
    /// 发往此 Client 的下一个事件序号。
    next_event_sequence: u64,
    /// Session stream 建立后填入；None 表示只有 unary session 尚未接 stream。
    sender: Option<mpsc::Sender<NodeEvent>>,
    /// Client 已确认处理的最大事件序号。
    last_ack: u64,
    /// 分配给共享读 view 的下一个单调 epoch。
    next_view_epoch: ViewEpoch,
    /// Client 已通过 heartbeat 释放的连续最大 view epoch。
    /// View 只能连续归还，不能越过 Node 已授予的最大 epoch。
    released_view_through: ViewEpoch,
    /// SDK 已确认结束的连续最大内部读请求；用于回收响应丢失/取消的票据。
    finished_read_request_through: u64,
    /// 已授予但还没由 Client watermark 归还的 SHM read view。
    active_views: HashMap<ViewEpoch, ActiveReadView>,
    /// 该 session 是否协商使用本机共享内存 payload target。
    shared_memory: bool,
    /// 新 SDK 显式 opt-in 后，写 SHM allocation 才能凭 token 回到 free-list。
    write_lease_release_supported: bool,
    /// 仅 unary renewal 回复授予缓存资格；单向 stream heartbeat 不延长此边界。
    cache_until: Option<Instant>,
    /// Node 只对这些 key 的持有者发送失效并建立写入屏障。
    ///
    /// 这里记录的是“这个 session 可能缓存了该 key 的 Current 结果”，不是缓存
    /// bytes 本身；真正的数据仍在 SDK 或 SHM mapping 中。这样 disconnected
    /// reader 只会阻塞同 key 的后续写入，不会拖慢无关 key。
    cached_current_keys: HashSet<Vec<u8>>,
    /// Total retained bytes of `cached_current_keys`. The count limit prevents
    /// many keys; this byte limit prevents few but very large keys from making
    /// the consistency index unbounded.
    cached_current_key_bytes: usize,
    /// 单个 session 的 key interest 是有界的。超过上限时不继续增长 HashSet，
    /// 而是退化为“本租约内可能缓存任意 Current key”。这会临时多等一些写，
    /// 但边界明确：只持续到 cache_until。
    cache_interest_all: bool,
    disconnected: bool,
}

struct InvalidationBarrier {
    waiting: HashMap<u64, u64>,
    waiter: Option<oneshot::Sender<Result<(), WorkerError>>>,
}

struct PreparedReplica {
    block_id: Vec<u8>,
    bytes: Vec<u8>,
    checksum: Vec<u8>,
    length: u64,
    expires_at: Instant,
}

struct FinishedReplica {
    block_id: Vec<u8>,
    checksum: Vec<u8>,
    length: u64,
    status: &'static str,
    /// Whether this attempt inserted a new Arena Block rather than reusing an
    /// identical Block already held locally for another logical reference.
    owns_block: bool,
}

enum ReplicaTransferState {
    Prepared(PreparedReplica),
    Active(FinishedReplica),
    Aborted(FinishedReplica),
}

/// R1 Node 的全部可变状态；Client 与 Peer 入口共享，只有 run_node 拥有它。
struct NodeState {
    node_id: String,
    // 各类 ID 使用独立序列，避免把不同生命周期误当成同一个概念。
    next_session: u64,
    next_transfer: u64,
    next_barrier: u64,
    next_read_scope: u64,
    sessions: HashMap<u64, Session>,
    barriers: HashMap<u64, InvalidationBarrier>,
    active_read_scopes: HashSet<u64>,
    /// Block → 正在拉取/接纳/登记的一轮任务。保留 origin scope 的 GC 保护，
    /// 即使最初调用者取消也不允许 Prepare 越过仍可能安装 bytes 的任务。
    peer_imports: HashMap<Vec<u8>, PeerImportFlight>,
    peer_import_failures: HashMap<Vec<u8>, PeerImportFailureFence>,
    next_peer_import: u64,
    peer_import_bytes: u64,
    peer_import_byte_limit: u64,
    downloads: HashMap<u64, DownloadTicket>,
    pending_retirements: HashMap<Vec<u8>, PendingRetirement>,
    completed_retirements: VecDeque<Vec<u8>>,
    retiring_blocks: HashSet<Vec<u8>>,
    prepared_replicas: HashMap<Vec<u8>, ReplicaTransferState>,
    /// block_id → 本次 prepare 是否新建；拒绝重入，失败只能回收自己新建的 Block。
    pending_blocks: HashMap<Vec<u8>, bool>,
    /// 唯一 Host-memory owner：staging、transfer index 和 committed blocks。
    arena: ArenaManager,
    config: OnlineConfigController,
    metadata: Option<MetadataClient>,
    metrics: NodeMetrics,
    log_level: LevelController,
    metadata_lease_until: Instant,
    metadata_watch_connected: bool,
    client_cache_lease_ttl: Duration,
    current_cache: CurrentCache,
}

impl NodeState {
    #[cfg(test)]
    fn new(
        node_id: String,
        metadata: Option<MetadataClient>,
        arena_capacity_bytes: u64,
        staging_ttl: Duration,
        shared_fd_broker: Option<SharedFdBroker>,
    ) -> Self {
        let registry = dms_metrics::registry();
        let metrics = NodeMetrics::register(&registry).expect("test Node metrics");
        Self::with_metrics(
            node_id,
            metadata,
            NodeTaskConfig {
                arena_capacity_bytes,
                region_size_bytes: crate::config::DEFAULT_REGION_SIZE_BYTES,
                staging_ttl,
                client_cache_lease_ttl: CLIENT_CACHE_LEASE_TTL,
                node_current_cache_bytes: crate::config::DEFAULT_NODE_CURRENT_CACHE_BYTES,
                node_current_cache_ttl: Duration::from_millis(
                    crate::config::DEFAULT_NODE_CURRENT_CACHE_TTL_MILLIS,
                ),
                shared_fd_broker,
                log_level: LevelController::new(slog::Level::Info),
                trace_periodic_operations: false,
            },
            metrics,
        )
    }

    fn with_metrics(
        node_id: String,
        metadata: Option<MetadataClient>,
        task_config: NodeTaskConfig,
        metrics: NodeMetrics,
    ) -> Self {
        let mut arena = ArenaManager::with_metrics(
            task_config.arena_capacity_bytes,
            task_config.staging_ttl,
            metrics.clone(),
        );
        arena.set_region_size(task_config.region_size_bytes);
        if let Some(broker) = task_config.shared_fd_broker {
            arena.enable_shared_region(broker);
        }
        // ID 从 1 开始，让 0 可保留为“未分配/无效”哨兵值。
        Self {
            node_id,
            next_session: 1,
            next_transfer: 1,
            next_barrier: 1,
            next_read_scope: 1,
            sessions: HashMap::new(),
            barriers: HashMap::new(),
            active_read_scopes: HashSet::new(),
            peer_imports: HashMap::new(),
            peer_import_failures: HashMap::new(),
            next_peer_import: 1,
            peer_import_bytes: 0,
            peer_import_byte_limit: task_config.arena_capacity_bytes,
            downloads: HashMap::new(),
            pending_retirements: HashMap::new(),
            completed_retirements: VecDeque::new(),
            retiring_blocks: HashSet::new(),
            prepared_replicas: HashMap::new(),
            pending_blocks: HashMap::new(),
            arena,
            config: OnlineConfigController::new(task_config.staging_ttl),
            metadata,
            metrics,
            log_level: task_config.log_level,
            metadata_lease_until: Instant::now(),
            metadata_watch_connected: false,
            client_cache_lease_ttl: task_config.client_cache_lease_ttl,
            current_cache: CurrentCache::new(
                task_config.node_current_cache_bytes,
                task_config.node_current_cache_ttl,
            ),
        }
    }

    fn apply_config_change(&mut self, change: ConfigChange) -> Result<u64, ConfigError> {
        let requested_log_level = match &change {
            ConfigChange::LogLevel(level) => Some(*level),
            _ => None,
        };
        let version = self.config.apply(change)?;
        self.arena.set_staging_ttl(self.config.staging_ttl());
        if let Some(level) = requested_log_level {
            self.log_level.set(level);
        }
        Ok(version)
    }

    fn probe(
        &self,
        source_node_id: String,
        nonce: Vec<u8>,
    ) -> Result<PeerProbeResult, WorkerError> {
        if source_node_id.is_empty() {
            return Err(WorkerError::InvalidArgument("source node id is empty"));
        }
        if nonce.is_empty() {
            return Err(WorkerError::InvalidArgument("probe nonce is empty"));
        }
        Ok(PeerProbeResult {
            serving_node_id: self.node_id.clone(),
            nonce,
        })
    }

    fn pull_block(
        &self,
        source_node_id: String,
        block_id: Vec<u8>,
        range: Option<(u64, u64)>,
    ) -> Result<PeerBlockResult, WorkerError> {
        if source_node_id.is_empty() || block_id.is_empty() {
            return Err(WorkerError::InvalidArgument(
                "source node id and block id are required",
            ));
        }
        let (ticket, full_length) = self
            .arena
            .open_read(&block_id, range)
            .map_err(map_arena_error)?;
        let payload = self.arena.read_ticket(ticket).map_err(map_arena_error)?;
        let checksum = digest(&payload);
        Ok(PeerBlockResult {
            serving_node_id: self.node_id.clone(),
            block_id,
            checksum,
            length: full_length,
            payload,
        })
    }

    #[cfg(test)]
    fn debug_commit_for_peer_test(
        &mut self,
        session_id: u64,
        staging_id: u64,
        receipt: HostReceipt,
        block_id: Vec<u8>,
    ) -> Result<(), WorkerError> {
        self.arena
            .commit_staging(session_id, staging_id, &receipt, block_id)
            .map_err(map_arena_error)
    }

    #[cfg(test)]
    fn open_session(&mut self, shared_memory: bool) -> u64 {
        self.open_session_with_write_release(shared_memory, false)
    }

    fn open_session_with_write_release(
        &mut self,
        shared_memory: bool,
        supports_write_lease_release: bool,
    ) -> u64 {
        // 先取当前 ID，再推进计数器；`&mut self` 保证此过程由 owner 串行执行。
        let id = self.next_session;
        self.next_session += 1;
        self.sessions.insert(
            id,
            Session {
                epoch: 1,
                next_event_sequence: 1,
                sender: None,
                last_ack: 0,
                next_view_epoch: 1,
                released_view_through: 0,
                finished_read_request_through: 0,
                active_views: HashMap::new(),
                shared_memory,
                write_lease_release_supported: shared_memory && supports_write_lease_release,
                cache_until: None,
                cached_current_keys: HashSet::new(),
                cached_current_key_bytes: 0,
                cache_interest_all: false,
                disconnected: false,
            },
        );
        // Inline SET does not allocate staging, but its first SHM GET still
        // needs the same RegionGroup authorization as a staging-backed write.
        self.arena.register_session(id);
        self.metrics.set_sessions(self.sessions.len());
        id
    }

    fn attach_session(
        &mut self,
        session_id: u64,
        sender: mpsc::Sender<NodeEvent>,
    ) -> Result<(), WorkerError> {
        self.live_session(session_id)?;
        // `get_mut` 返回对 HashMap value 的可变借用；借用只活到本函数结束。
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(WorkerError::UnknownSession)?;
        // 把 Sender 移入 Session；以后 set() 可以向该 Client 发送事件。
        session.sender = Some(sender);
        Ok(())
    }

    #[cfg(test)]
    fn heartbeat(
        &mut self,
        session_id: u64,
        released_view_through: Option<u64>,
        renew_cache: bool,
    ) -> Result<u64, WorkerError> {
        self.heartbeat_inner(
            session_id,
            released_view_through,
            Vec::new(),
            None,
            renew_cache,
        )
    }

    fn heartbeat_inner(
        &mut self,
        session_id: u64,
        released_view_through: Option<u64>,
        released_write_allocations: Vec<ReleasedWriteAllocation>,
        finished_read_request_through: Option<u64>,
        renew_cache: bool,
    ) -> Result<u64, WorkerError> {
        let cleanup_only = released_view_through.is_some()
            || !released_write_allocations.is_empty()
            || finished_read_request_through.is_some();
        let write_lease_release_supported = {
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or(WorkerError::UnknownSession)?;
            if session.disconnected && !cleanup_only {
                return Err(WorkerError::UnknownSession);
            }
            if let Some(released) = released_view_through {
                let max_granted = session.next_view_epoch.saturating_sub(1);
                if released > max_granted {
                    return Err(WorkerError::InvalidArgument(
                        "released view watermark exceeds granted views",
                    ));
                }
                session.released_view_through = session.released_view_through.max(released);
                session
                    .active_views
                    .retain(|view_epoch, _| *view_epoch > session.released_view_through);
            }
            if let Some(finished) = finished_read_request_through {
                session.finished_read_request_through =
                    session.finished_read_request_through.max(finished);
                let finished = session.finished_read_request_through;
                // finished 只清理“该 request 已结束但响应可能丢失”的借用；
                // SDK 已交付给用户的 View 仍必须等 released_view_through。
                session
                    .active_views
                    .retain(|_, view| view.read_request_id == 0 || view.read_request_id > finished);
            }
            session.write_lease_release_supported
        };
        if let Some(finished) = finished_read_request_through {
            self.downloads.retain(|_, ticket| {
                ticket.session_id != session_id
                    || ticket.read_request_id == 0
                    || ticket.read_request_id > finished
            });
        }
        if !released_write_allocations.is_empty() && !write_lease_release_supported {
            return Err(WorkerError::InvalidArgument(
                "write lease release was not negotiated",
            ));
        }
        for released in released_write_allocations {
            self.arena
                .release_write_allocation(session_id, &released)
                .map_err(map_arena_error)?;
        }
        self.advance_retirements();
        if !renew_cache {
            return Ok(0);
        }
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(WorkerError::UnknownSession)?;
        if session.disconnected
            || !self.metadata_watch_connected
            || session.last_ack < session.next_event_sequence.saturating_sub(1)
            || session.sender.as_ref().is_none_or(mpsc::Sender::is_closed)
        {
            return Ok(0);
        }
        let now = Instant::now();
        let ttl_millis = self
            .metadata_lease_until
            .saturating_duration_since(now)
            .min(self.client_cache_lease_ttl)
            .as_millis() as u64;
        // Client 从发送请求前的时间计 TTL，服务端从处理请求时计，因此服务端等待边界
        // 不早于 Client 停止缓存命中的边界；迟到的响应不能重新延长旧资格。
        if ttl_millis > 0 {
            session.cache_until = Some(now + Duration::from_millis(ttl_millis));
        }
        Ok(ttl_millis)
    }

    fn register_cache_interest(
        &mut self,
        session_id: u64,
        key: Vec<u8>,
    ) -> Result<(), WorkerError> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .filter(|session| !session.disconnected)
            .ok_or(WorkerError::UnknownSession)?;
        if session.cache_until.is_none() || session.cache_interest_all {
            return Ok(());
        }
        if session.cached_current_keys.contains(&key) {
            return Ok(());
        }
        let projected_key_bytes = session.cached_current_key_bytes.saturating_add(key.len());
        if session.cached_current_keys.len() >= CLIENT_CACHE_INTEREST_KEY_LIMIT
            || projected_key_bytes > CLIENT_CACHE_INTEREST_BYTES_LIMIT
        {
            session.cached_current_keys.clear();
            session.cached_current_key_bytes = 0;
            session.cache_interest_all = true;
        } else {
            session.cached_current_key_bytes = projected_key_bytes;
            session.cached_current_keys.insert(key);
        }
        Ok(())
    }

    fn live_session(&self, session_id: u64) -> Result<&Session, WorkerError> {
        // 断连后留下的记录只承担旧租约屏障，不能重新创建分配或重新绑定事件流。
        self.sessions
            .get(&session_id)
            .filter(|session| !session.disconnected)
            .ok_or(WorkerError::UnknownSession)
    }

    fn close_session(&mut self, session_id: u64) -> Result<(), WorkerError> {
        // Disconnect cleanup is idempotent. Removing the Session drops its
        // event sender and reclaims every uncommitted allocation it owned.
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.disconnected = true;
            session.sender = None;
        }
        self.metrics.set_sessions(self.sessions.len());
        self.metrics
            .record_session_expiration(SessionExpiration::Closed);
        self.arena.reclaim_session(session_id);
        self.expire_cache_leases();
        Ok(())
    }

    fn acknowledge(&mut self, session_id: u64, sequence: u64) -> Result<(), WorkerError> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(WorkerError::UnknownSession)?;
        // max 使重复或乱序 ACK 保持幂等，游标绝不倒退。
        session.last_ack = session.last_ack.max(sequence);
        self.advance_barriers(session_id, sequence);
        Ok(())
    }

    fn allocate_staging(
        &mut self,
        session_id: u64,
        length: u64,
    ) -> Result<StagingAllocation, WorkerError> {
        // 空 value 暂不支持；未来若支持必须明确其对象语义。
        if length == 0 {
            return Err(WorkerError::InvalidArgument("value must not be empty"));
        }
        let shared_memory = self.live_session(session_id)?.shared_memory;
        let mut allocation = self
            .arena
            .allocate(session_id, length)
            .map_err(map_arena_error)?;
        if shared_memory
            && let Some(descriptor) = self
                .arena
                .shm_descriptor_for_staging(session_id, allocation.staging_id)
                .map_err(map_arena_error)?
        {
            allocation.target = HostAllocationTarget::Shm(descriptor);
        }
        Ok(StagingAllocation {
            staging_id: allocation.staging_id,
            transfer_id: allocation.transfer_id,
            length: allocation.length,
            target: allocation.target,
        })
    }

    fn acquire_region(
        &mut self,
        session_id: u64,
        region_id: u64,
    ) -> Result<HostRegionGrant, WorkerError> {
        if !self.live_session(session_id)?.shared_memory {
            return Err(WorkerError::UnknownSession);
        }
        self.arena
            .acquire_region(session_id, region_id)
            .map_err(map_arena_error)
    }

    fn upload(&mut self, transfer_id: u64, bytes: Vec<u8>) -> Result<HostReceipt, WorkerError> {
        self.arena
            .upload(transfer_id, &bytes)
            .map_err(map_arena_error)
    }

    fn receipt_for_session(
        &self,
        session_id: u64,
        mut receipt: HostReceipt,
    ) -> Result<HostReceipt, WorkerError> {
        let session = self.live_session(session_id)?;
        if !session.write_lease_release_supported {
            // B 批安全边界：旧 SDK/未协商 session 即便回传了字段，也不能把
            // “Set 成功”伪装成可写 mmap 已归还；Arena 会继续隔离未知写者。
            receipt.release_token.clear();
        }
        Ok(receipt)
    }

    fn delete_staging(&mut self, session_id: u64, staging_id: u64) -> Result<(), WorkerError> {
        self.live_session(session_id)?;
        // Cancellation is deliberately idempotent: an already consumed or
        // already deleted staging allocation is a successful no-op.
        self.arena.delete_staging(session_id, staging_id);
        Ok(())
    }

    fn set(
        &mut self,
        session_id: u64,
        key: Vec<u8>,
        staging_id: u64,
        receipt: HostReceipt,
        operation_id: Vec<u8>,
        condition: String,
    ) -> Result<PreparedWrite<SetOutcome>, WorkerError> {
        if key.is_empty() {
            return Err(WorkerError::InvalidArgument("key must not be empty"));
        }
        self.live_session(session_id)?;
        let receipt = self.receipt_for_session(session_id, receipt)?;
        let block_id = block_identity(&self.node_id, &operation_id);
        let owns_block = self.check_block_preparation(&block_id)?;
        self.arena
            .commit_staging(session_id, staging_id, &receipt, block_id.clone())
            .map_err(map_arena_error)?;
        self.pending_blocks.insert(block_id.clone(), owns_block);
        self.commit_block(ValueCommitInput {
            cache_session_id: Some(session_id),
            key,
            block_id,
            length: receipt.length,
            checksum: receipt.digest,
            operation_id,
            condition,
        })
    }

    fn mset(
        &mut self,
        session_id: u64,
        entries: Vec<(Vec<u8>, u64, HostReceipt)>,
        operation_id: Vec<u8>,
    ) -> Result<PreparedWrite<MSetOutcome>, WorkerError> {
        if entries.is_empty() {
            return Err(WorkerError::InvalidArgument("MSet entries are empty"));
        }
        self.live_session(session_id)?;
        let metadata = self
            .metadata
            .clone()
            .ok_or(WorkerError::MetadataUnavailable)?;
        let mut values = Vec::with_capacity(entries.len());
        let mut committed_blocks: Vec<Vec<u8>> = Vec::with_capacity(entries.len());
        for (index, (key, staging_id, receipt)) in entries.into_iter().enumerate() {
            let receipt = self.receipt_for_session(session_id, receipt)?;
            let entry_operation = derive_batch_operation_id(&operation_id, index)?;
            let block_id = block_identity(&self.node_id, &entry_operation);
            let owns_block = match self.check_block_preparation(&block_id) {
                Ok(owns) => owns,
                Err(error) => {
                    for block in &committed_blocks {
                        self.finish_block_preparation(block, true);
                    }
                    return Err(error);
                }
            };
            if let Err(error) =
                self.arena
                    .commit_staging(session_id, staging_id, &receipt, block_id.clone())
            {
                for block in &committed_blocks {
                    self.finish_block_preparation(block, true);
                }
                return Err(map_arena_error(error));
            }
            self.pending_blocks.insert(block_id.clone(), owns_block);
            committed_blocks.push(block_id.clone());
            values.push(BatchValueCommit {
                key,
                block_id,
                length: receipt.length,
                checksum: receipt.digest,
                operation_id: entry_operation,
            });
        }
        Ok(Box::pin(async move {
            let committed = metadata.commit_batch_values(values, operation_id).await;
            Box::new(move |state: &mut NodeState| {
                let rejected = committed
                    .as_ref()
                    .is_err_and(is_definitive_metadata_rejection);
                for block in &committed_blocks {
                    state.finish_block_preparation(block, rejected);
                }
                let committed = match committed {
                    Ok(committed) => committed,
                    Err(error) => {
                        return Err(map_metadata_error(error));
                    }
                };
                let mut versions = Vec::with_capacity(committed.results.len());
                let mut barrier_ids = Vec::new();
                for (block_id, item) in committed_blocks.iter().zip(committed.results) {
                    let key = item.key.ok_or(WorkerError::MetadataUnavailable)?.value;
                    let result = item.result.ok_or(WorkerError::MetadataUnavailable)?;
                    state.arena.mark_committed(block_id, result.version);
                    if let Some(barrier_id) =
                        state.broadcast_invalidation(key.clone(), result.version)
                    {
                        barrier_ids.push(barrier_id);
                    }
                    state.register_cache_interest(session_id, key.clone())?;
                    versions.push(KeySetOutcome {
                        key,
                        version: result.version,
                    });
                }
                Ok(MSetOutcome {
                    versions,
                    barrier_ids,
                })
            }) as WriteCompletion<MSetOutcome>
        }))
    }

    fn set_range(
        &mut self,
        input: SetRangeInput,
        resolved: pb::ResolveObjectResponse,
    ) -> Result<PreparedWrite<SetOutcome>, WorkerError> {
        let SetRangeInput {
            session_id,
            key,
            offset,
            staging_id,
            receipt,
            operation_id,
            expected_version: _,
        } = input;
        if key.is_empty() || operation_id.len() != 24 {
            return Err(WorkerError::InvalidArgument(
                "key and operation identity are required",
            ));
        }
        self.live_session(session_id)?;
        let metadata = self
            .metadata
            .clone()
            .ok_or(WorkerError::MetadataUnavailable)?;
        let layout = resolved.layout.as_ref().ok_or(WorkerError::NotFound)?;
        let base_version = layout.version;
        let patch_end = offset
            .checked_add(receipt.length)
            .ok_or(WorkerError::InvalidArgument("range write overflows u64"))?;
        if patch_end > layout.logical_length {
            return Err(WorkerError::InvalidArgument(
                "range write extends beyond current value",
            ));
        }

        // Seal the patch in-place. No base bytes are pulled and no full-value
        // Vec is created: the new layout overlays one immutable patch Block
        // on the immutable base extents.
        let patch_block = block_identity(&self.node_id, &operation_id);
        let receipt = self.receipt_for_session(session_id, receipt)?;
        let owns_block = self.check_block_preparation(&patch_block)?;
        let extents = super::version_layout::overlay(
            &layout.extents,
            layout.logical_length,
            offset,
            receipt.length,
            &patch_block,
            &receipt.digest,
        )?;
        let candidate_digest = super::version_layout::digest(layout.logical_length, &extents);
        self.arena
            .commit_staging(session_id, staging_id, &receipt, patch_block.clone())
            .map_err(map_arena_error)?;
        self.pending_blocks.insert(patch_block.clone(), owns_block);
        let replica_proofs = resolved
            .block_replicas
            .iter()
            .filter(|set| set.block_id != patch_block)
            .filter_map(|set| set.proofs.first().cloned())
            .collect();
        let logical_length = layout.logical_length;
        Ok(Box::pin(async move {
            let committed = metadata
                .commit_layout(
                    key.clone(),
                    operation_id,
                    pb::VersionCandidate {
                        kind: pb::VersionKind::Value as i32,
                        logical_length,
                        extents,
                        digest: candidate_digest,
                    },
                    replica_proofs,
                    vec![pb::ReplicaReport {
                        block_id: patch_block.clone(),
                        length: receipt.length,
                        checksum: receipt.digest,
                        durability: pb::DurabilityPolicy::LocalMemory as i32,
                    }],
                    format!("if-version:{base_version}"),
                )
                .await;
            Box::new(move |state: &mut NodeState| {
                state.finish_block_preparation(
                    &patch_block,
                    committed
                        .as_ref()
                        .is_err_and(is_definitive_metadata_rejection),
                );
                let committed = match committed {
                    Ok(committed) => committed,
                    Err(error) => {
                        return Err(map_metadata_error(error));
                    }
                };
                state.arena.mark_committed(&patch_block, committed.version);
                let barrier_id = state.broadcast_invalidation(key.clone(), committed.version);
                state.register_cache_interest(session_id, key)?;
                Ok(SetOutcome {
                    version: committed.version,
                    length: logical_length,
                    barrier_id,
                })
            }) as WriteCompletion<SetOutcome>
        }))
    }

    fn commit_bytes(
        &mut self,
        session_id: u64,
        key: Vec<u8>,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        condition: String,
    ) -> Result<PreparedWrite<SetOutcome>, WorkerError> {
        if key.is_empty() || operation_id.len() != 24 {
            return Err(WorkerError::InvalidArgument(
                "key and operation identity are required",
            ));
        }
        self.live_session(session_id)?;
        let block_id = block_identity(&self.node_id, &operation_id);
        if bytes.is_empty() {
            // 空 value 是真实 VALUE 版本，不是 Tombstone；不能为区分语义而分配
            // 一个假 payload Block。Meta 的 kind/length 是逻辑存在性的权威。
            return self.commit_block(ValueCommitInput {
                cache_session_id: Some(session_id),
                key,
                block_id,
                length: 0,
                checksum: digest(&[]),
                operation_id,
                condition,
            });
        }
        let owns_block = self.check_block_preparation(&block_id)?;
        let length = bytes.len() as u64;
        let checksum = digest(&bytes);
        self.arena
            .commit_inline_with_verified_digest(block_id.clone(), bytes, checksum.clone())
            .map_err(map_arena_error)?;
        self.pending_blocks.insert(block_id.clone(), owns_block);
        self.commit_block(ValueCommitInput {
            cache_session_id: Some(session_id),
            key,
            block_id,
            length,
            checksum,
            operation_id,
            condition,
        })
    }

    fn commit_block(
        &mut self,
        input: ValueCommitInput,
    ) -> Result<PreparedWrite<SetOutcome>, WorkerError> {
        let ValueCommitInput {
            cache_session_id,
            key,
            block_id,
            length,
            checksum,
            operation_id,
            condition,
        } = input;
        let metadata = self
            .metadata
            .clone()
            .ok_or(WorkerError::MetadataUnavailable)?;
        Ok(Box::pin(async move {
            let committed = metadata
                .commit_value(
                    key.clone(),
                    block_id.clone(),
                    length,
                    checksum,
                    operation_id,
                    condition,
                )
                .await;
            Box::new(move |state: &mut NodeState| {
                state.finish_block_preparation(
                    &block_id,
                    committed
                        .as_ref()
                        .is_err_and(is_definitive_metadata_rejection),
                );
                let committed = match committed {
                    Ok(committed) => committed,
                    Err(error) => {
                        return Err(map_metadata_error(error));
                    }
                };
                state.arena.mark_committed(&block_id, committed.version);
                let barrier_id = state.broadcast_invalidation(key.clone(), committed.version);
                if let Some(session_id) = cache_session_id {
                    state.register_cache_interest(session_id, key)?;
                }
                Ok(SetOutcome {
                    version: committed.version,
                    length,
                    barrier_id,
                })
            }) as WriteCompletion<SetOutcome>
        }))
    }

    fn check_block_preparation(&self, block_id: &[u8]) -> Result<bool, WorkerError> {
        if self.metadata.is_none() {
            return Err(WorkerError::MetadataUnavailable);
        }
        if self.pending_blocks.contains_key(block_id) {
            return Err(WorkerError::WorkerUnavailable);
        }
        Ok(self.arena.open_read(block_id, None).is_err())
    }

    fn finish_block_preparation(&mut self, block_id: &[u8], rejected: bool) {
        if self.pending_blocks.remove(block_id) == Some(true) && rejected {
            self.arena.retire_block(block_id);
        }
    }

    fn delete(
        &mut self,
        session_id: u64,
        key: Vec<u8>,
        operation_id: Vec<u8>,
    ) -> Result<PreparedWrite<DeleteOutcome>, WorkerError> {
        if key.is_empty() {
            return Err(WorkerError::InvalidArgument("key must not be empty"));
        }
        self.live_session(session_id)?;
        let metadata = self
            .metadata
            .clone()
            .ok_or(WorkerError::MetadataUnavailable)?;
        Ok(Box::pin(async move {
            let committed = metadata.commit_delete(key.clone(), operation_id).await;
            Box::new(move |state: &mut NodeState| {
                let committed = committed.map_err(map_metadata_error)?;
                let barrier_id = committed
                    .changed
                    .then(|| state.broadcast_invalidation(key, committed.version))
                    .flatten();
                Ok(DeleteOutcome {
                    deleted: committed.changed,
                    version: committed.version,
                    barrier_id,
                })
            }) as WriteCompletion<DeleteOutcome>
        }))
    }

    /// 有效 Current 解析可省去权威 Current 查询。
    ///
    /// 如果本次范围的 Block 已在本地，直接返回票据；如果只缺 payload bytes，
    /// 使用同一缓存项里的位置提示生成 peer 拉取计划。位置提示只随 layout 在同一
    /// 租约、Watch 和 generation 内生效，失败后上层会按固定版本 Exact 回退。
    #[expect(
        clippy::too_many_arguments,
        reason = "cached read lookup keeps session/scope/request/range/budget separate to preserve cancellation and cache contracts"
    )]
    fn get_cached(
        &mut self,
        session_id: u64,
        read_scope_id: u64,
        read_request_id: u64,
        key: &[u8],
        node_epoch: u64,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
    ) -> Result<CachedRead, WorkerError> {
        self.live_session(session_id)?;
        self.reject_finished_read_request(session_id, read_request_id)?;
        if range.is_none() {
            self.register_cache_interest(session_id, key.to_vec())?;
        }
        let now = Instant::now();
        if !self.metadata_watch_connected || now >= self.metadata_lease_until {
            self.current_cache.clear();
            self.metrics.set_current_cache_charge(0);
            self.metrics.record_current_cache_lookup(false);
            return Ok((None, CachedReadOutcome::Miss));
        }
        let token = self.current_cache.token();
        let cached = self.current_cache.get(key, node_epoch, now);
        let outcome = if let Some(cached) = cached {
            // Arc 只在 owner 内借用，按请求生成独立票据，不缓存可写内存地址。
            match self.get_resolved_parts(
                session_id,
                read_scope_id,
                read_request_id,
                cached.layout.as_ref(),
                cached.block_replicas.as_slice(),
                range,
                clamp_range,
                max_inline_bytes,
            ) {
                Ok(GetOutcome::Ready(ticket)) => CachedReadOutcome::Ready(ticket),
                Ok(GetOutcome::NeedsRemoteBlocks(specs)) => CachedReadOutcome::NeedsRemoteBlocks {
                    // 热命中本地 Block 时不复制 replica 列表；只有确实缺块、需要
                    // 脱离 owner 异步拉取 peer 时，才把同一缓存项转换回可移动的
                    // ResolveObjectResponse。
                    resolved: pb::ResolveObjectResponse {
                        layout: Some((*cached.layout).clone()),
                        block_replicas: (*cached.block_replicas).clone(),
                        current_lease: None,
                    },
                    specs,
                },
                Err(error) if can_refresh_cached_location(&error) => {
                    CachedReadOutcome::NeedsExactRefresh {
                        version: cached.layout.version,
                    }
                }
                Err(error) => return Err(error),
            }
        } else {
            CachedReadOutcome::Miss
        };
        self.metrics
            .record_current_cache_lookup(!matches!(outcome, CachedReadOutcome::Miss));
        self.metrics
            .set_current_cache_charge(self.current_cache.charged());
        Ok((token, outcome))
    }

    #[cfg(test)]
    fn get_resolved(
        &mut self,
        session_id: u64,
        resolved: &pb::ResolveObjectResponse,
        range: Option<(u64, u64)>,
        max_inline_bytes: u64,
    ) -> Result<GetOutcome, WorkerError> {
        self.get_resolved_for_request(
            session_id,
            u64::MAX,
            0,
            resolved,
            range,
            false,
            max_inline_bytes,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "resolved read planning keeps request identity, range policy, and inline budget explicit for tests and actor calls"
    )]
    fn get_resolved_for_request(
        &mut self,
        session_id: u64,
        read_scope_id: u64,
        read_request_id: u64,
        resolved: &pb::ResolveObjectResponse,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
    ) -> Result<GetOutcome, WorkerError> {
        let layout = resolved.layout.as_ref().ok_or(WorkerError::NotFound)?;
        self.get_resolved_parts(
            session_id,
            read_scope_id,
            read_request_id,
            layout,
            &resolved.block_replicas,
            range,
            clamp_range,
            max_inline_bytes,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "ticket construction needs explicit session, scope, request id, range, and inline budget for bounded read semantics"
    )]
    fn get_resolved_parts(
        &mut self,
        session_id: u64,
        read_scope_id: u64,
        read_request_id: u64,
        layout: &pb::VersionLayout,
        block_replicas: &[pb::BlockReplicaSet],
        range: Option<(u64, u64)>,
        clamp_range: bool,
        max_inline_bytes: u64,
    ) -> Result<GetOutcome, WorkerError> {
        self.live_session(session_id)?;
        self.reject_finished_read_request(session_id, read_request_id)?;
        super::version_layout::validate(layout.logical_length, &layout.extents)?;
        let requested = Self::normalize_read_range(range, layout.logical_length, clamp_range)?;
        let request_end = requested
            .0
            .checked_add(requested.1)
            .ok_or(WorkerError::InvalidArgument("read range overflows u64"))?;

        let mut missing = Vec::new();
        let mut planned = Vec::new();
        for extent in &layout.extents {
            let logical = extent.logical.as_ref().ok_or(WorkerError::InvalidArgument(
                "extent logical range is missing",
            ))?;
            let extent_end = logical
                .offset
                .checked_add(logical.length)
                .ok_or(WorkerError::InvalidArgument("extent range overflows u64"))?;
            let start = requested.0.max(logical.offset);
            let end = request_end.min(extent_end);
            if start >= end {
                continue;
            }
            if self.retirement_blocks_new_reads(&extent.block_id, read_scope_id) {
                return Err(WorkerError::NotFound);
            }
            if let Some(failure) = self.peer_import_failure(&extent.block_id, read_scope_id) {
                return Err(failure.error.clone());
            }
            let block_offset = extent
                .block_offset
                .checked_add(start - logical.offset)
                .ok_or(WorkerError::InvalidArgument("block range overflows u64"))?;
            let block_range = (block_offset, end - start);
            // Import 已安装但 Report 仍在进行时，也必须等待同一轮结果；
            // 不能因为本地暂时可见 bytes 就让并发请求掩盖登记失败。
            let local = if self.peer_imports.contains_key(&extent.block_id) {
                Err(ArenaError::UnknownBlock)
            } else {
                self.arena.open_read(&extent.block_id, Some(block_range))
            };
            match local {
                Ok((read, _)) => planned.push((start - requested.0, read)),
                Err(ArenaError::UnknownBlock) => {
                    if !missing
                        .iter()
                        .any(|item: &PeerPullSpec| item.block_id == extent.block_id)
                    {
                        missing.push(self.describe_missing_block(block_replicas, extent)?);
                    }
                }
                Err(error) => return Err(map_arena_error(error)),
            }
        }
        if !missing.is_empty() {
            return Ok(GetOutcome::NeedsRemoteBlocks(missing));
        }
        if requested.1 > 0 && planned.is_empty() {
            return Err(WorkerError::NotFound);
        }
        let version = layout.version;
        let logical_length = layout.logical_length;
        let (shared_memory, view_epoch) = {
            let session = self
                .sessions
                .get(&session_id)
                .ok_or(WorkerError::UnknownSession)?;
            (session.shared_memory, session.next_view_epoch)
        };
        if let Some(inline_value) =
            self.inline_read_value(requested.1, shared_memory, max_inline_bytes, &planned)?
        {
            return Ok(GetOutcome::Ready(ReadTicket {
                read_request_id,
                version,
                logical_length,
                inline_value: Some(inline_value),
                segments: Vec::new(),
            }));
        }
        let mut segments = Vec::with_capacity(planned.len());
        let mut view_allocations = Vec::new();
        for (logical_offset, read) in planned {
            let payload_length = read.length;
            let target = if shared_memory {
                let transfer_id = self.next_transfer;
                self.next_transfer += 1;
                match self
                    .arena
                    .shm_descriptor_for_read(session_id, read, transfer_id, view_epoch)
                    .map_err(map_arena_error)?
                {
                    Some(descriptor) => {
                        view_allocations.push(descriptor.allocation_id);
                        ReadTarget::Shm(descriptor)
                    }
                    None => self.grpc_download_target_with_id(
                        session_id,
                        read,
                        transfer_id,
                        read_request_id,
                    ),
                }
            } else {
                self.grpc_download_target(session_id, read, read_request_id)
            };
            segments.push(ReadTicketSegment {
                logical_offset,
                target,
                payload_length,
            });
        }
        // 只有真正返回 SHM 借用才消耗序号。TCP/内联、空范围，以及构造票据
        // 失败都不能制造 Client 永远收不到的 epoch 空洞。此段在唯一 owner 内。
        if segments
            .iter()
            .any(|segment| matches!(segment.target, ReadTarget::Shm(_)))
        {
            let next = view_epoch
                .checked_add(1)
                .ok_or(WorkerError::InvalidArgument("view epoch exhausted"))?;
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or(WorkerError::UnknownSession)?;
            session.active_views.insert(
                view_epoch,
                ActiveReadView {
                    read_request_id,
                    allocation_ids: view_allocations,
                },
            );
            session.next_view_epoch = next;
        }
        Ok(GetOutcome::Ready(ReadTicket {
            read_request_id,
            version,
            logical_length,
            inline_value: None,
            segments,
        }))
    }

    /// 先验证调用方请求的 u64 范围本身，再按 opt-in 策略裁剪到同一已解析版本。
    fn normalize_read_range(
        range: Option<(u64, u64)>,
        logical_length: u64,
        clamp_range: bool,
    ) -> Result<(u64, u64), WorkerError> {
        let Some((offset, length)) = range else {
            return Ok((0, logical_length));
        };
        let request_end = offset
            .checked_add(length)
            .ok_or(WorkerError::InvalidArgument("read range overflows u64"))?;
        if !clamp_range {
            if request_end > logical_length {
                return Err(WorkerError::InvalidArgument(
                    "read range extends beyond current value",
                ));
            }
            return Ok((offset, length));
        }
        if offset >= logical_length {
            return Ok((logical_length, 0));
        }
        Ok((offset, request_end.min(logical_length) - offset))
    }

    /// 只拼接本次已解析版本、已裁剪范围的读票据；不重新按 Current 找版本。
    /// 在创建下载票据前选择内联，避免制造无人消费的 DownloadTicket。
    fn inline_read_value(
        &self,
        requested_length: u64,
        shared_memory: bool,
        max_inline_bytes: u64,
        planned: &[(u64, ArenaReadTicket)],
    ) -> Result<Option<Vec<u8>>, WorkerError> {
        let inline_limit = max_inline_bytes.min(dms_protocol::MAX_INLINE_READ_BYTES);
        if shared_memory || inline_limit == 0 || requested_length > inline_limit {
            return Ok(None);
        }
        let capacity =
            usize::try_from(requested_length).map_err(|_| WorkerError::ResourceExhausted)?;
        let mut ordered = planned.to_vec();
        ordered.sort_by_key(|(logical_offset, _)| *logical_offset);
        let mut bytes = Vec::with_capacity(capacity);
        let mut cursor = 0_u64;
        for (logical_offset, read) in ordered {
            if logical_offset != cursor {
                return Err(WorkerError::InvalidArgument(
                    "layout extents contain a gap or overlap",
                ));
            }
            let part = self.arena.read_ticket(read).map_err(map_arena_error)?;
            cursor = cursor
                .checked_add(part.len() as u64)
                .filter(|next| *next <= requested_length)
                .ok_or(WorkerError::ResourceExhausted)?;
            bytes.extend_from_slice(&part);
        }
        if cursor != requested_length {
            return Err(WorkerError::InvalidArgument(
                "layout extents do not cover requested range",
            ));
        }
        Ok(Some(bytes))
    }

    fn materialize_resolved(
        &mut self,
        session_id: u64,
        read_scope_id: u64,
        resolved: &pb::ResolveObjectResponse,
    ) -> Result<MaterializeOutcome, WorkerError> {
        self.live_session(session_id)?;
        let layout = resolved.layout.as_ref().ok_or(WorkerError::NotFound)?;
        super::version_layout::validate(layout.logical_length, &layout.extents)?;
        let mut missing = Vec::new();
        let mut planned = Vec::with_capacity(layout.extents.len());
        for extent in &layout.extents {
            let logical = extent.logical.as_ref().ok_or(WorkerError::InvalidArgument(
                "extent logical range is missing",
            ))?;
            if self.retirement_blocks_new_reads(&extent.block_id, read_scope_id) {
                return Err(WorkerError::NotFound);
            }
            if let Some(failure) = self.peer_import_failure(&extent.block_id, read_scope_id) {
                return Err(failure.error.clone());
            }
            let local = if self.peer_imports.contains_key(&extent.block_id) {
                Err(ArenaError::UnknownBlock)
            } else {
                self.arena.open_read(
                    &extent.block_id,
                    Some((extent.block_offset, logical.length)),
                )
            };
            match local {
                Ok((read, _)) => planned.push((logical.offset, read)),
                Err(ArenaError::UnknownBlock) => {
                    if !missing
                        .iter()
                        .any(|item: &PeerPullSpec| item.block_id == extent.block_id)
                    {
                        missing
                            .push(self.describe_missing_block(&resolved.block_replicas, extent)?);
                    }
                }
                Err(error) => return Err(map_arena_error(error)),
            }
        }
        if !missing.is_empty() {
            return Ok(MaterializeOutcome::NeedsRemoteBlocks(missing));
        }
        planned.sort_by_key(|(offset, _)| *offset);
        let capacity =
            usize::try_from(layout.logical_length).map_err(|_| WorkerError::ResourceExhausted)?;
        let mut bytes = Vec::with_capacity(capacity);
        let mut expected_offset = 0_u64;
        for (offset, ticket) in planned {
            if offset != expected_offset {
                return Err(WorkerError::InvalidArgument(
                    "layout extents contain a gap or overlap",
                ));
            }
            let part = self.arena.read_ticket(ticket).map_err(map_arena_error)?;
            expected_offset = expected_offset
                .checked_add(part.len() as u64)
                .ok_or(WorkerError::ResourceExhausted)?;
            bytes.extend_from_slice(&part);
        }
        if expected_offset != layout.logical_length {
            return Err(WorkerError::InvalidArgument(
                "layout extents do not cover logical length",
            ));
        }
        Ok(MaterializeOutcome::Ready {
            version: layout.version,
            bytes,
        })
    }

    fn grpc_download_target(
        &mut self,
        session_id: u64,
        read: ArenaReadTicket,
        read_request_id: u64,
    ) -> ReadTarget {
        let transfer_id = self.next_transfer;
        self.next_transfer += 1;
        self.grpc_download_target_with_id(session_id, read, transfer_id, read_request_id)
    }

    fn grpc_download_target_with_id(
        &mut self,
        session_id: u64,
        read: ArenaReadTicket,
        transfer_id: u64,
        read_request_id: u64,
    ) -> ReadTarget {
        self.downloads.insert(
            transfer_id,
            DownloadTicket {
                read,
                session_id,
                read_request_id,
                expires_at: Instant::now() + Duration::from_secs(30),
            },
        );
        ReadTarget::Grpc { transfer_id }
    }

    fn describe_missing_block(
        &self,
        block_replicas: &[pb::BlockReplicaSet],
        extent: &pb::ExtentRecord,
    ) -> Result<PeerPullSpec, WorkerError> {
        let replica_set = block_replicas
            .iter()
            .find(|set| set.block_id == extent.block_id)
            .ok_or(WorkerError::NotFound)?;
        let replica = replica_set
            .replicas
            .iter()
            .find(|replica| replica.data_endpoint.starts_with("http://"))
            .ok_or(WorkerError::NotFound)?;
        Ok(PeerPullSpec {
            endpoint: replica.data_endpoint.clone(),
            block_id: extent.block_id.clone(),
            expected_checksum: extent.digest.clone(),
            expected_length: replica_set.length,
        })
    }

    /// 返回 attempt 表示需要启动新任务；None 表示已回复或加入已有任务。
    /// 此表不是另一个 value cache：只持有有界控制状态，bytes 始终归 Arena。
    fn begin_peer_import(
        &mut self,
        read_scope_id: u64,
        spec: &PeerPullSpec,
        reply: oneshot::Sender<Result<(), PeerImportFailure>>,
    ) -> Option<u64> {
        if reply.is_closed() {
            return None;
        }
        let reject = |reply: oneshot::Sender<_>, error| {
            let _ = reply.send(Err(PeerImportFailure::terminal(error)));
        };
        if !self.active_read_scopes.contains(&read_scope_id)
            || self.retirement_blocks_new_reads(&spec.block_id, read_scope_id)
        {
            reject(reply, WorkerError::NotFound);
            return None;
        }
        if let Some(failure) = self.peer_import_failure(&spec.block_id, read_scope_id) {
            let _ = reply.send(Err(failure.clone()));
            return None;
        }
        if let Some(flight) = self.peer_imports.get_mut(&spec.block_id) {
            if flight.expected_length != spec.expected_length
                || flight.expected_checksum != spec.expected_checksum
            {
                reject(reply, WorkerError::Conflict);
            } else {
                flight.waiters.retain(|waiter| !waiter.is_closed());
                if flight.waiters.len() >= NODE_MAILBOX_CAPACITY {
                    reject(reply, WorkerError::ResourceExhausted);
                } else {
                    flight.waiters.push(reply);
                }
            }
            return None;
        }
        // 两个缺块计划之间，另一轮可能已完成；在 owner 内重新检查避免重复搬运。
        if let Some((length, checksum)) = self.arena.block_length_and_digest(&spec.block_id) {
            if length == spec.expected_length
                && (spec.expected_checksum.is_empty() || checksum == spec.expected_checksum)
            {
                let _ = reply.send(Ok(()));
            } else {
                reject(reply, WorkerError::Conflict);
            }
            return None;
        }
        let charged = self.peer_import_bytes.checked_add(spec.expected_length);
        if self.peer_imports.len() + self.peer_import_failures.len() >= NODE_MAILBOX_CAPACITY
            || charged.is_none_or(|bytes| bytes > self.peer_import_byte_limit)
        {
            reject(reply, WorkerError::ResourceExhausted);
            return None;
        }
        let Some(next) = self.next_peer_import.checked_add(1) else {
            reject(reply, WorkerError::ResourceExhausted);
            return None;
        };
        let attempt = self.next_peer_import;
        self.next_peer_import = next;
        self.peer_import_bytes = charged.expect("checked peer import bytes");
        self.peer_imports.insert(
            spec.block_id.clone(),
            PeerImportFlight {
                attempt,
                read_scope_id,
                expected_length: spec.expected_length,
                expected_checksum: spec.expected_checksum.clone(),
                task_id: None,
                waiters: vec![reply],
            },
        );
        Some(attempt)
    }

    /// 安装和完成都校验批次身份，旧任务不能给新 flight 安装数据或释放它的 pin。
    fn validate_peer_import_attempt(
        &self,
        block_id: &[u8],
        read_scope_id: u64,
        attempt: u64,
    ) -> Result<(), WorkerError> {
        match self.peer_imports.get(block_id) {
            Some(flight) if flight.attempt == attempt && flight.read_scope_id == read_scope_id => {
                Ok(())
            }
            _ => Err(WorkerError::Conflict),
        }
    }

    fn complete_peer_import(
        &mut self,
        block_id: &[u8],
        attempt: u64,
        result: Result<(), PeerImportFailure>,
    ) {
        if self
            .peer_imports
            .get(block_id)
            .is_none_or(|flight| flight.attempt != attempt)
        {
            return;
        }
        let flight = self
            .peer_imports
            .remove(block_id)
            .expect("matching peer import");
        self.peer_import_bytes -= flight.expected_length;
        if let Err(failure) = &result
            && !failure.location_failure
        {
            // Report 超时不等于 Meta 未登记，不能撤销可能已可达的 bytes。
            // 但同批读不能因为 bytes 已安装就伪造成功：保留到这些 scope 排空。
            // 新 scope 不受此 fence 影响；表与在途任务共用条目上限。
            self.peer_import_failures.insert(
                block_id.to_vec(),
                PeerImportFailureFence {
                    cutoff_scope: self.next_read_scope.saturating_sub(1),
                    failure: failure.clone(),
                },
            );
        }
        for waiter in flight.waiters {
            let _ = waiter.send(result.clone());
        }
        self.advance_retirements();
    }

    fn peer_import_failure(
        &self,
        block_id: &[u8],
        read_scope_id: u64,
    ) -> Option<&PeerImportFailure> {
        self.peer_import_failures
            .get(block_id)
            .filter(|fence| read_scope_id <= fence.cutoff_scope)
            .map(|fence| &fence.failure)
    }

    fn fail_peer_import_task(&mut self, task_id: tokio::task::Id) {
        // 只在 panic/任务取消的异常路径扫描有界表；正常完成直接按 Block+attempt 查找。
        let failed = self.peer_imports.iter().find_map(|(block_id, flight)| {
            (flight.task_id == Some(task_id)).then_some((block_id.clone(), flight.attempt))
        });
        if let Some((block_id, attempt)) = failed {
            self.complete_peer_import(
                &block_id,
                attempt,
                Err(PeerImportFailure::terminal(
                    WorkerError::TransferUnavailable,
                )),
            );
        }
    }

    fn import_peer_block(
        &mut self,
        read_scope_id: u64,
        block_id: Vec<u8>,
        bytes: Vec<u8>,
        checksum: Vec<u8>,
        length: u64,
    ) -> Result<bool, WorkerError> {
        if length != bytes.len() as u64 {
            return Err(WorkerError::Conflict);
        }
        let retiring_for_old_scope = self.retiring_blocks.contains(&block_id);
        if retiring_for_old_scope && self.retirement_blocks_new_reads(&block_id, read_scope_id) {
            return Err(WorkerError::Conflict);
        }
        // 完整 Block 在唯一 owner 接纳时校验一次。接收协程只负责范围/长度/
        // 分段摘要；不能因为传输成功就跳过这里，也不能先登记副本再检查。
        let verified_digest = digest(&bytes);
        if !checksum.is_empty() && verified_digest != checksum {
            self.metrics.record_replica_checksum_failure();
            return Err(WorkerError::Conflict);
        }
        self.arena
            .commit_inline_with_verified_digest(block_id, bytes, verified_digest)
            .map_err(map_arena_error)?;
        Ok(!retiring_for_old_scope)
    }

    fn prepare_replica(
        &mut self,
        plan_id: Vec<u8>,
        block_id: Vec<u8>,
        bytes: Vec<u8>,
        checksum: Vec<u8>,
        length: u64,
    ) -> Result<ReplicaStateView, WorkerError> {
        if plan_id.is_empty() || block_id.is_empty() || length != bytes.len() as u64 {
            return Err(WorkerError::Conflict);
        }
        if !checksum.is_empty() && digest(&bytes) != checksum {
            self.metrics.record_replica_checksum_failure();
            return Err(WorkerError::Conflict);
        }
        let incoming = PreparedReplica {
            block_id,
            bytes,
            checksum,
            length,
            expires_at: Instant::now() + Duration::from_secs(30),
        };
        match self.prepared_replicas.get(&plan_id) {
            Some(ReplicaTransferState::Prepared(existing))
                if existing.block_id == incoming.block_id
                    && existing.length == incoming.length
                    && existing.checksum == incoming.checksum =>
            {
                return Ok(replica_view(
                    &plan_id,
                    &existing.block_id,
                    "prepared",
                    existing.length,
                    &existing.checksum,
                ));
            }
            Some(ReplicaTransferState::Active(existing))
                if existing.block_id == incoming.block_id
                    && existing.length == incoming.length
                    && existing.checksum == incoming.checksum =>
            {
                return Ok(replica_view(
                    &plan_id,
                    &existing.block_id,
                    existing.status,
                    existing.length,
                    &existing.checksum,
                ));
            }
            Some(_) => return Err(WorkerError::Conflict),
            None => {}
        }
        let view = replica_view(
            &plan_id,
            &incoming.block_id,
            "prepared",
            incoming.length,
            &incoming.checksum,
        );
        self.prepared_replicas
            .insert(plan_id, ReplicaTransferState::Prepared(incoming));
        Ok(view)
    }

    fn activate_replica(&mut self, plan_id: Vec<u8>) -> Result<ReplicaStateView, WorkerError> {
        let state = self
            .prepared_replicas
            .remove(&plan_id)
            .ok_or(WorkerError::NotFound)?;
        match state {
            ReplicaTransferState::Prepared(prepared) => {
                let owns_block = self.arena.read_bytes(&prepared.block_id).is_none();
                // 兼容旧 peer 未携带摘要的情况：Arena 仍必须保存真实身份。
                // 正常非空摘要在 prepare 已验证，此处不重复扫描 payload。
                let verified_digest = if prepared.checksum.is_empty() {
                    digest(&prepared.bytes)
                } else {
                    prepared.checksum.clone()
                };
                self.arena
                    .commit_inline_with_verified_digest(
                        prepared.block_id.clone(),
                        prepared.bytes,
                        verified_digest,
                    )
                    .map_err(map_arena_error)?;
                let finished = FinishedReplica {
                    block_id: prepared.block_id,
                    checksum: prepared.checksum,
                    length: prepared.length,
                    status: "active",
                    owns_block,
                };
                let view = replica_view(
                    &plan_id,
                    &finished.block_id,
                    finished.status,
                    finished.length,
                    &finished.checksum,
                );
                self.prepared_replicas
                    .insert(plan_id, ReplicaTransferState::Active(finished));
                Ok(view)
            }
            ReplicaTransferState::Active(finished) => {
                let view = replica_view(
                    &plan_id,
                    &finished.block_id,
                    finished.status,
                    finished.length,
                    &finished.checksum,
                );
                self.prepared_replicas
                    .insert(plan_id, ReplicaTransferState::Active(finished));
                Ok(view)
            }
            ReplicaTransferState::Aborted(finished) => {
                self.prepared_replicas
                    .insert(plan_id, ReplicaTransferState::Aborted(finished));
                Err(WorkerError::Conflict)
            }
        }
    }

    fn abort_replica(&mut self, plan_id: Vec<u8>) -> Result<ReplicaStateView, WorkerError> {
        let state = self
            .prepared_replicas
            .remove(&plan_id)
            .ok_or(WorkerError::NotFound)?;
        let finished = match state {
            ReplicaTransferState::Prepared(prepared) => FinishedReplica {
                block_id: prepared.block_id,
                checksum: prepared.checksum,
                length: prepared.length,
                status: "aborted",
                owns_block: false,
            },
            ReplicaTransferState::Active(finished) => {
                self.prepared_replicas
                    .insert(plan_id, ReplicaTransferState::Active(finished));
                return Err(WorkerError::Conflict);
            }
            ReplicaTransferState::Aborted(finished) => finished,
        };
        let view = replica_view(
            &plan_id,
            &finished.block_id,
            finished.status,
            finished.length,
            &finished.checksum,
        );
        self.prepared_replicas
            .insert(plan_id, ReplicaTransferState::Aborted(finished));
        Ok(view)
    }

    fn discard_replica_attempt(&mut self, plan_id: Vec<u8>) -> Result<(), WorkerError> {
        let state = self
            .prepared_replicas
            .remove(&plan_id)
            .ok_or(WorkerError::NotFound)?;
        match state {
            ReplicaTransferState::Prepared(_) | ReplicaTransferState::Aborted(_) => Ok(()),
            ReplicaTransferState::Active(finished) => {
                if finished.owns_block {
                    self.arena.retire_block(&finished.block_id);
                }
                Ok(())
            }
        }
    }

    fn replica_status(&self, plan_id: &[u8]) -> Result<ReplicaStateView, WorkerError> {
        match self
            .prepared_replicas
            .get(plan_id)
            .ok_or(WorkerError::NotFound)?
        {
            ReplicaTransferState::Prepared(replica) => Ok(replica_view(
                plan_id,
                &replica.block_id,
                "prepared",
                replica.length,
                &replica.checksum,
            )),
            ReplicaTransferState::Active(replica) | ReplicaTransferState::Aborted(replica) => {
                Ok(replica_view(
                    plan_id,
                    &replica.block_id,
                    replica.status,
                    replica.length,
                    &replica.checksum,
                ))
            }
        }
    }

    fn begin_read_scope(&mut self, session_id: u64) -> Result<u64, WorkerError> {
        self.live_session(session_id)?;
        let scope_id = self.next_read_scope;
        self.next_read_scope = self
            .next_read_scope
            .checked_add(1)
            .ok_or(WorkerError::InvalidArgument("read scope id exhausted"))?;
        self.active_read_scopes.insert(scope_id);
        Ok(scope_id)
    }

    fn begin_read_scope_reply(
        &mut self,
        session_id: u64,
        cleanup_node: NodeHandle,
        reply: oneshot::Sender<Result<ReadScopeLease, WorkerError>>,
    ) {
        match self.begin_read_scope(session_id) {
            Ok(scope_id) => {
                if reply
                    .send(Ok(ReadScopeLease::new(cleanup_node, scope_id)))
                    .is_err()
                {
                    // Caller was cancelled after scope allocation but before
                    // receiving the id; undo immediately so Prepare cannot
                    // wait forever on an unobservable scope.
                    self.finish_read_scope(scope_id);
                }
            }
            Err(error) => {
                let _ = reply.send(Err(error));
            }
        }
    }

    fn finish_read_scope(&mut self, scope_id: u64) {
        self.active_read_scopes.remove(&scope_id);
        self.advance_retirements();
    }

    #[cfg(test)]
    fn prepare_block_retirement(&mut self, block_ids: Vec<Vec<u8>>) -> Result<u64, WorkerError> {
        self.current_cache.clear();
        self.metrics.set_current_cache_charge(0);
        for block_id in block_ids {
            self.retiring_blocks.insert(block_id);
        }
        Ok(self.next_read_scope.saturating_sub(1))
    }

    fn register_prepare_retirement(
        &mut self,
        retirement_id: Vec<u8>,
        block_ids: Vec<Vec<u8>>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    ) {
        if retirement_id.is_empty() || block_ids.is_empty() {
            let _ = reply.send(Err(WorkerError::InvalidArgument(
                "retirement id and blocks are required",
            )));
            return;
        }
        if self.completed_retirement(&retirement_id) {
            let _ = reply.send(Ok(()));
            return;
        }
        // Prepare 是 Meta 对旧物理位置的 cut：先撤销本机 Current cache，并把
        // block 放入 retiring set。cutoff 前已开始的 scope 可走完；之后的新读
        // 不能再从旧 cached/resolved layout 新建借用。
        self.current_cache.clear();
        self.metrics.set_current_cache_charge(0);
        for block_id in &block_ids {
            self.retiring_blocks.insert(block_id.clone());
        }
        let entry = self
            .pending_retirements
            .entry(retirement_id)
            .or_insert_with(|| PendingRetirement {
                block_ids: block_ids.clone(),
                cutoff_scope: self.next_read_scope.saturating_sub(1),
                prepared: false,
                final_requested: false,
                released: false,
                prepare_waiters: Vec::new(),
                final_waiters: Vec::new(),
            });
        if entry.block_ids != block_ids {
            let _ = reply.send(Err(WorkerError::Conflict));
            return;
        }
        if entry.prepared {
            let _ = reply.send(Ok(()));
        } else if entry.prepare_waiters.len() >= RETIREMENT_PHASE_WAITER_LIMIT {
            let _ = reply.send(Err(WorkerError::ResourceExhausted));
        } else {
            entry.prepare_waiters.push(reply);
            self.advance_retirements();
        }
    }

    fn register_final_retirement(
        &mut self,
        retirement_id: Vec<u8>,
        block_ids: Vec<Vec<u8>>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    ) {
        if retirement_id.is_empty() || block_ids.is_empty() {
            let _ = reply.send(Err(WorkerError::InvalidArgument(
                "retirement id and blocks are required",
            )));
            return;
        }
        if self.completed_retirement(&retirement_id) {
            let _ = reply.send(Ok(()));
            return;
        }
        let entry = self
            .pending_retirements
            .entry(retirement_id)
            .or_insert_with(|| PendingRetirement {
                block_ids: block_ids.clone(),
                cutoff_scope: self.next_read_scope.saturating_sub(1),
                prepared: false,
                final_requested: false,
                released: false,
                prepare_waiters: Vec::new(),
                final_waiters: Vec::new(),
            });
        if entry.block_ids != block_ids {
            let _ = reply.send(Err(WorkerError::Conflict));
            return;
        }
        if entry.released {
            let _ = reply.send(Ok(()));
        } else if entry.final_waiters.len() >= RETIREMENT_PHASE_WAITER_LIMIT {
            let _ = reply.send(Err(WorkerError::ResourceExhausted));
        } else {
            entry.final_requested = true;
            entry.final_waiters.push(reply);
            self.advance_retirements();
        }
    }

    fn read_scopes_drained(&self, cutoff: u64) -> bool {
        !self
            .active_read_scopes
            .iter()
            .any(|scope_id| *scope_id <= cutoff)
            && !self
                .peer_imports
                .values()
                .any(|flight| flight.read_scope_id <= cutoff)
    }

    #[cfg(test)]
    fn try_finalize_block_retirement(
        &mut self,
        block_ids: Vec<Vec<u8>>,
    ) -> Result<bool, WorkerError> {
        if !self.retirement_can_release(&block_ids)? {
            return Ok(false);
        }
        for block_id in &block_ids {
            self.arena.retire_block(block_id);
            self.retiring_blocks.remove(block_id);
        }
        Ok(true)
    }

    fn retirement_can_release(&self, block_ids: &[Vec<u8>]) -> Result<bool, WorkerError> {
        let mut allocation_ids = Vec::with_capacity(block_ids.len());
        for block_id in block_ids {
            if let Some(allocation_id) = self.arena.block_allocation_id(block_id) {
                allocation_ids.push(allocation_id);
            }
        }
        if allocation_ids.is_empty() {
            return Ok(true);
        }
        for allocation_id in &allocation_ids {
            if self
                .downloads
                .values()
                .any(|ticket| ticket.read.handle.allocation_id == *allocation_id)
            {
                return Ok(false);
            }
            if self.sessions.values().any(|session| {
                session
                    .active_views
                    .values()
                    .any(|views| views.allocation_ids.contains(allocation_id))
            }) {
                return Ok(false);
            }
        }
        for block_id in block_ids {
            if self.arena.block_has_unreturned_write(block_id) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn advance_retirements(&mut self) {
        self.peer_import_failures.retain(|_, fence| {
            self.active_read_scopes
                .iter()
                .any(|scope| *scope <= fence.cutoff_scope)
        });
        let ready_prepares = self
            .pending_retirements
            .iter()
            .filter_map(|(id, pending)| {
                (!pending.prepared && self.read_scopes_drained(pending.cutoff_scope))
                    .then_some(id.clone())
            })
            .collect::<Vec<_>>();
        for id in ready_prepares {
            if let Some(pending) = self.pending_retirements.get_mut(&id) {
                pending.prepared = true;
                for waiter in pending.prepare_waiters.drain(..) {
                    let _ = waiter.send(Ok(()));
                }
            }
        }

        let ready_finals = self
            .pending_retirements
            .iter()
            .filter_map(|(id, pending)| {
                (pending.prepared
                    && pending.final_requested
                    && !pending.released
                    && self
                        .retirement_can_release(&pending.block_ids)
                        .unwrap_or(false))
                .then_some(id.clone())
            })
            .collect::<Vec<_>>();
        for id in ready_finals {
            let Some(mut pending) = self.pending_retirements.remove(&id) else {
                continue;
            };
            pending.released = true;
            for block_id in &pending.block_ids {
                self.arena.retire_block(block_id);
                self.retiring_blocks.remove(block_id);
            }
            for waiter in pending.prepare_waiters.drain(..) {
                let _ = waiter.send(Ok(()));
            }
            for waiter in pending.final_waiters.drain(..) {
                let _ = waiter.send(Ok(()));
            }
            self.remember_completed_retirement(id);
        }
    }

    fn completed_retirement(&self, retirement_id: &[u8]) -> bool {
        self.completed_retirements
            .iter()
            .any(|completed| completed.as_slice() == retirement_id)
    }

    fn remember_completed_retirement(&mut self, retirement_id: Vec<u8>) {
        if self.completed_retirement(&retirement_id) {
            return;
        }
        self.completed_retirements.push_back(retirement_id);
        while self.completed_retirements.len() > COMPLETED_RETIREMENT_CACHE_LIMIT {
            self.completed_retirements.pop_front();
        }
    }

    fn reject_finished_read_request(
        &self,
        session_id: u64,
        read_request_id: u64,
    ) -> Result<(), WorkerError> {
        if read_request_id == 0 {
            return Ok(());
        }
        let session = self.live_session(session_id)?;
        if read_request_id <= session.finished_read_request_through {
            return Err(WorkerError::InvalidArgument(
                "read request has already finished",
            ));
        }
        Ok(())
    }

    fn retirement_blocks_new_reads(&self, block_id: &[u8], read_scope_id: u64) -> bool {
        if !self.retiring_blocks.contains(block_id) {
            return false;
        }
        !self.pending_retirements.values().any(|pending| {
            pending
                .block_ids
                .iter()
                .any(|pending_block| pending_block.as_slice() == block_id)
                && read_scope_id <= pending.cutoff_scope
        })
    }

    fn layout_contains_retiring_block(&self, resolved: &pb::ResolveObjectResponse) -> bool {
        resolved.layout.as_ref().is_some_and(|layout| {
            layout
                .extents
                .iter()
                .any(|extent| self.retiring_blocks.contains(&extent.block_id))
        })
    }

    fn broadcast_invalidation(&mut self, key: Vec<u8>, minimum_version: u64) -> Option<u64> {
        // 先撤销本机布局及在途回填，再等待 Client ACK，最后才允许 ACK Meta。
        // 本机写 completion 同样走这里：Meta barrier 排除了本次 source Node。
        self.current_cache.invalidate(&key);
        self.metrics
            .set_current_cache_charge(self.current_cache.charged());
        self.expire_cache_leases();
        let mut waiting = HashMap::new();
        let mut failed_sessions = Vec::new();
        for (session_id, session) in &mut self.sessions {
            if session.cache_until.is_none()
                || (!session.cache_interest_all && !session.cached_current_keys.contains(&key))
            {
                continue;
            }
            let event_sequence = session.next_event_sequence;
            session.next_event_sequence += 1;
            // 包括已断流但租约尚未过期的 Client；不能先删等待者再宣称写成功。
            waiting.insert(*session_id, event_sequence);
            let Some(sender) = session.sender.clone() else {
                continue;
            };
            match sender.try_send(NodeEvent::InvalidateCurrent {
                session_epoch: session.epoch,
                event_sequence,
                key: key.clone(),
                minimum_version,
            }) {
                Ok(()) => {}
                Err(_) => failed_sessions.push(*session_id),
            }
        }
        // 撤销后不再授予缓存租约；保留旧租约义务直到 ACK 或真实过期。
        for session_id in failed_sessions {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                session.disconnected = true;
                session.sender = None;
            }
            self.arena.reclaim_session(session_id);
        }
        if waiting.is_empty() {
            return None;
        }
        let barrier_id = self.next_barrier;
        self.next_barrier += 1;
        self.barriers.insert(
            barrier_id,
            InvalidationBarrier {
                waiting,
                waiter: None,
            },
        );
        Some(barrier_id)
    }

    fn attach_barrier_waiter(
        &mut self,
        barrier_id: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    ) {
        let Some(barrier) = self.barriers.get_mut(&barrier_id) else {
            let _ = reply.send(Ok(()));
            return;
        };
        if barrier.waiter.is_some() {
            // A barrier has exactly one owner: either a SET handler or the
            // Meta Watch task. A second waiter is an internal protocol bug.
            let _ = reply.send(Err(WorkerError::Conflict));
        } else {
            barrier.waiter = Some(reply);
        }
    }

    fn advance_barriers(&mut self, session_id: u64, sequence: u64) {
        let completed = self
            .barriers
            .iter_mut()
            .filter_map(|(barrier_id, barrier)| {
                if barrier
                    .waiting
                    .get(&session_id)
                    .is_some_and(|required| sequence >= *required)
                {
                    barrier.waiting.remove(&session_id);
                }
                barrier.waiting.is_empty().then_some(*barrier_id)
            })
            .collect::<Vec<_>>();
        self.complete_barriers(completed);
    }

    fn release_session_from_barriers(&mut self, session_id: u64) {
        let completed = self
            .barriers
            .iter_mut()
            .filter_map(|(barrier_id, barrier)| {
                barrier.waiting.remove(&session_id);
                barrier.waiting.is_empty().then_some(*barrier_id)
            })
            .collect::<Vec<_>>();
        self.complete_barriers(completed);
    }

    fn next_cache_lease_expiry(&self) -> Option<Instant> {
        self.sessions
            .values()
            .filter_map(|session| session.cache_until)
            .min()
    }

    fn expire_cache_leases(&mut self) {
        let now = Instant::now();
        let expired = self
            .sessions
            .iter_mut()
            .filter_map(|(id, session)| {
                if session.cache_until.is_none_or(|until| until <= now) {
                    session.cache_until = None;
                    session.cached_current_keys.clear();
                    session.cached_current_key_bytes = 0;
                    session.cache_interest_all = false;
                    Some(*id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for id in expired {
            self.release_session_from_barriers(id);
            if self
                .sessions
                .get(&id)
                .is_some_and(|session| session.disconnected)
                && !self.session_has_retained_obligations(id)
            {
                self.sessions.remove(&id);
            }
        }
        self.metrics.set_sessions(self.sessions.len());
    }

    fn session_has_retained_obligations(&self, session_id: u64) -> bool {
        self.sessions
            .get(&session_id)
            .is_some_and(|session| !session.active_views.is_empty())
            || self
                .downloads
                .values()
                .any(|ticket| ticket.session_id == session_id)
            || self.arena.session_has_unreturned_write(session_id)
    }

    fn complete_barriers(&mut self, barrier_ids: Vec<u64>) {
        for barrier_id in barrier_ids {
            if let Some(mut barrier) = self.barriers.remove(&barrier_id)
                && let Some(waiter) = barrier.waiter.take()
            {
                let _ = waiter.send(Ok(()));
            }
        }
    }

    fn download(&mut self, transfer_id: u64) -> Result<Vec<u8>, WorkerError> {
        // remove 让下载票据成为一次性：成功下载后同一 transfer_id 不能重放。
        let ticket = self
            .downloads
            .remove(&transfer_id)
            .ok_or(WorkerError::UnknownTransfer)?;
        if Instant::now() > ticket.expires_at {
            return Err(WorkerError::UnknownTransfer);
        }
        let bytes = self
            .arena
            .read_ticket(ticket.read)
            .map_err(map_arena_error)?;
        self.advance_retirements();
        Ok(bytes)
    }

    fn tick(&mut self) {
        self.expire_cache_leases();
        self.arena.tick();
        let now = Instant::now();
        self.downloads.retain(|_, ticket| ticket.expires_at > now);
        self.advance_retirements();
        // Prepared bytes are not readable and must not live forever if the
        // coordinator or source disappears between prepare and activate.
        let expired_plan_ids = self
            .prepared_replicas
            .iter()
            .filter_map(|(plan_id, state)| {
                matches!(state, ReplicaTransferState::Prepared(replica) if replica.expires_at <= now)
                    .then_some(plan_id.clone())
            })
            .collect::<Vec<_>>();
        for plan_id in expired_plan_ids {
            let Some(ReplicaTransferState::Prepared(replica)) =
                self.prepared_replicas.remove(&plan_id)
            else {
                continue;
            };
            self.prepared_replicas.insert(
                plan_id,
                ReplicaTransferState::Aborted(FinishedReplica {
                    block_id: replica.block_id,
                    checksum: replica.checksum,
                    length: replica.length,
                    status: "aborted",
                    owns_block: false,
                }),
            );
        }
    }
}

fn block_identity(node_id: &str, operation_id: &[u8]) -> Vec<u8> {
    let mut input = node_id.as_bytes().to_vec();
    input.extend_from_slice(operation_id);
    digest(&input)
}

fn derive_batch_operation_id(operation_id: &[u8], index: usize) -> Result<Vec<u8>, WorkerError> {
    if operation_id.len() != 24 {
        return Err(WorkerError::InvalidArgument(
            "operation identity must contain client id and sequence",
        ));
    }
    let mut input = operation_id.to_vec();
    input.extend_from_slice(&(index as u64).to_be_bytes());
    let digest = digest(&input);
    let mut derived = Vec::with_capacity(24);
    // OperationId 的 wire 形态是 16-byte client_instance_id + 8-byte sequence。
    // 批量操作的子操作必须保留同一个 client_instance_id，避免和普通请求或其它
    // Client 实例混淆；sequence 部分由父 OperationId + batch index 派生，保证同
    // 一个 MSET/HSET 内每个 field/entry 都有稳定且不同的幂等身份。
    derived.extend_from_slice(&operation_id[..16]);
    derived.extend_from_slice(&digest[..8]);
    Ok(derived)
}

fn replica_view(
    plan_id: &[u8],
    block_id: &[u8],
    status: &'static str,
    length: u64,
    checksum: &[u8],
) -> ReplicaStateView {
    ReplicaStateView {
        plan_id: plan_id.to_vec(),
        block_id: block_id.to_vec(),
        status,
        length,
        checksum: checksum.to_vec(),
    }
}

fn map_metadata_error(error: DmsError) -> WorkerError {
    WorkerError::Stable(error)
}

fn can_refresh_cached_location(error: &WorkerError) -> bool {
    match error {
        WorkerError::NotFound | WorkerError::TransferUnavailable => true,
        WorkerError::Stable(error) => error.code() == dms_error::NODE_TRANSFER_UNAVAILABLE,
        _ => false,
    }
}

fn is_definitive_metadata_rejection(error: &DmsError) -> bool {
    // 只有提交前的校验/CAS 拒绝能证明未发布。WAL append 或 checkpoint 错误
    // 可能发生在持久化或 apply 之后，未知错误也必须保留块，等待同 operation 重试。
    matches!(
        error.code(),
        dms_error::META_CATALOG_INVALID_REQUEST | dms_error::META_CATALOG_VERSION_CONFLICT
    )
}

pub(crate) fn worker_error_to_dms(error: WorkerError) -> DmsError {
    match error {
        WorkerError::InvalidArgument(reason) => DmsError::new(
            dms_error::NODE_WORKER_INVALID_REQUEST,
            ErrorKind::InvalidArgument,
            reason,
        ),
        WorkerError::ArenaInvalidRequest => DmsError::new(
            dms_error::NODE_ARENA_INVALID_REQUEST,
            ErrorKind::InvalidArgument,
            "invalid DMS Arena request",
        ),
        WorkerError::UnknownSession => DmsError::new(
            dms_error::NODE_SESSION_UNKNOWN,
            ErrorKind::Unauthenticated,
            "unknown DMS session",
        ),
        WorkerError::UnknownStaging | WorkerError::UnknownTransfer | WorkerError::NotFound => {
            DmsError::new(
                dms_error::NODE_OBJECT_NOT_FOUND,
                ErrorKind::NotFound,
                "DMS resource not found",
            )
        }
        WorkerError::Conflict => DmsError::new(
            dms_error::NODE_VERSION_CONFLICT,
            ErrorKind::FailedPrecondition,
            "DMS staging or version conflict",
        ),
        WorkerError::ResourceExhausted => DmsError::new(
            dms_error::NODE_ARENA_CAPACITY_EXHAUSTED,
            ErrorKind::ResourceExhausted,
            "DMS Host Arena is full",
        ),
        WorkerError::WorkerUnavailable => DmsError::new(
            dms_error::NODE_WORKER_UNAVAILABLE,
            ErrorKind::Unavailable,
            "DMS Worker actor or mailbox is unavailable",
        ),
        WorkerError::MetadataUnavailable => DmsError::new(
            dms_error::NODE_METADATA_UNAVAILABLE,
            ErrorKind::Unavailable,
            "DMS metadata service is unavailable or returned an incomplete response",
        ),
        WorkerError::TransferUnavailable => DmsError::new(
            dms_error::NODE_TRANSFER_UNAVAILABLE,
            ErrorKind::Unavailable,
            "DMS transfer path is unavailable",
        ),
        WorkerError::ArenaStaleHandle => DmsError::new(
            dms_error::NODE_ARENA_STALE_HANDLE,
            ErrorKind::Unavailable,
            "stale DMS Arena region or allocation handle",
        ),
        WorkerError::ArenaShmUnavailable => DmsError::new(
            dms_error::NODE_ARENA_SHM_UNAVAILABLE,
            ErrorKind::Unavailable,
            "DMS Arena shared memory is unavailable",
        ),
        WorkerError::ArenaAccessDenied => DmsError::new(
            dms_error::NODE_ARENA_ACCESS_DENIED,
            ErrorKind::PermissionDenied,
            "DMS Arena region access denied",
        ),
        WorkerError::Stable(error) => error,
    }
}

pub(crate) fn is_not_found_result(error: &WorkerError) -> bool {
    match error {
        WorkerError::NotFound => true,
        // 结构化 Meta NotFound 代表 key/exact version 未解析到。对 GET/MGET
        // 这类“miss 是正常返回值”的 Worker API，应转成 found=false；
        // 对 HDEL/HWRITE 等变更 API，则仍由调用点继续返回错误。
        WorkerError::Stable(error) => error.kind() == ErrorKind::NotFound,
        _ => false,
    }
}

fn map_arena_error(error: ArenaError) -> WorkerError {
    match error {
        ArenaError::UnknownTransfer => WorkerError::UnknownTransfer,
        ArenaError::UnknownStaging => WorkerError::UnknownStaging,
        ArenaError::StagingNotWritable | ArenaError::ReceiptConflict => WorkerError::Conflict,
        ArenaError::StaleHandle | ArenaError::UnknownRegion => WorkerError::ArenaStaleHandle,
        ArenaError::SharedMemoryUnavailable => WorkerError::ArenaShmUnavailable,
        ArenaError::RegionAccessDenied => WorkerError::ArenaAccessDenied,
        ArenaError::RegionCreateFailed => WorkerError::Stable(DmsError::new(
            dms_error::NODE_ARENA_ALLOCATION_FAILED,
            ErrorKind::ResourceExhausted,
            "DMS Arena failed to allocate a backing region",
        )),
        ArenaError::CapacityExhausted => WorkerError::ResourceExhausted,
        ArenaError::EmptyPayload => WorkerError::ArenaInvalidRequest,
        ArenaError::UnknownBlock => WorkerError::NotFound,
        ArenaError::LengthMismatch | ArenaError::RangeOutOfBounds | ArenaError::RegionOverflow => {
            WorkerError::ArenaInvalidRequest
        }
    }
}

async fn pull_block_from_peer(
    source_node_id: &str,
    spec: PeerPullSpec,
    rpc_metrics: &dms_metrics::RpcMetrics,
    peer_channels: &Arc<Mutex<HashMap<String, Channel>>>,
) -> Result<PeerBlockResult, WorkerError> {
    if spec.expected_length <= PEER_PULL_SEGMENT_BYTES {
        return pull_peer_segment(
            source_node_id,
            &spec,
            rpc_metrics,
            peer_channels,
            None,
            None,
            true,
        )
        .await;
    }

    let capacity =
        usize::try_from(spec.expected_length).map_err(|_| WorkerError::ResourceExhausted)?;
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(capacity)
        .map_err(|_| WorkerError::ResourceExhausted)?;
    let mut serving_node_id = String::new();
    let mut offset = 0;
    let mut next_request = 0;
    let mut pulls = tokio::task::JoinSet::new();
    let mut ready = std::collections::BTreeMap::new();
    // 先建好共享连接，避免最初的并发请求各自建立一个 Channel。
    peer_channel_for(&spec.endpoint, peer_channels).await?;
    while offset < spec.expected_length {
        // 窗口同时计算“请求未完成”和“已返回但尚不能按序追加”两种占用。
        // 第一段先到可立即补第三段；第二段先到则占住窗口，不能无限积累结果。
        // 每段仍为2MiB unary，额外分段bytes最多4MiB（不包括整块聚合Vec）。
        while pulls.len() + ready.len() < 2 && next_request < spec.expected_length {
            let start = next_request;
            let length = (spec.expected_length - start).min(PEER_PULL_SEGMENT_BYTES);
            next_request += length;
            let source_node_id = source_node_id.to_owned();
            let spec = spec.clone();
            let rpc_metrics = rpc_metrics.clone();
            let peer_channels = peer_channels.clone();
            pulls.spawn(
                async move {
                    let segment = pull_peer_segment(
                        &source_node_id,
                        &spec,
                        &rpc_metrics,
                        &peer_channels,
                        Some(start),
                        Some(length),
                        false,
                    )
                    .await?;
                    Ok::<_, WorkerError>((start, segment))
                }
                .instrument(dms_tracing::tracing::Span::current()),
            );
        }
        // JoinSet 在出错或父请求取消时 drop 会取消剩余拉取任务；已经返回的
        // 分段和未发布的聚合Vec随函数释放。这里不会提前安装或登记副本。
        let (start, segment) = pulls
            .join_next()
            .await
            .ok_or(WorkerError::TransferUnavailable)?
            .map_err(|_| WorkerError::TransferUnavailable)??;
        ready.insert(start, segment);
        while let Some(segment) = ready.remove(&offset) {
            let length = (spec.expected_length - offset).min(PEER_PULL_SEGMENT_BYTES);
            if segment.length != spec.expected_length || segment.payload.len() as u64 != length {
                return Err(WorkerError::Conflict);
            }
            if serving_node_id.is_empty() {
                serving_node_id = segment.serving_node_id;
            } else if serving_node_id != segment.serving_node_id {
                return Err(WorkerError::Conflict);
            }
            payload.extend_from_slice(&segment.payload);
            offset += length;
        }
    }

    // 有权威摘要时直接带到 ImportPeerBlock/PrepareReplica：两条入口都在
    // owner 接纳前验证完整 owned bytes。这里再扫描一遍不会增加完整性保证。
    // 老调用者未提供摘要时仍计算摘要，保留原有返回值和后续校验行为。
    let checksum = if spec.expected_checksum.is_empty() {
        digest(&payload)
    } else {
        spec.expected_checksum
    };
    Ok(PeerBlockResult {
        serving_node_id,
        block_id: spec.block_id,
        payload,
        checksum,
        length: spec.expected_length,
    })
}

async fn pull_peer_segment(
    source_node_id: &str,
    spec: &PeerPullSpec,
    rpc_metrics: &dms_metrics::RpcMetrics,
    peer_channels: &Arc<Mutex<HashMap<String, Channel>>>,
    offset: Option<u64>,
    length: Option<u64>,
    validate_whole_block: bool,
) -> Result<PeerBlockResult, WorkerError> {
    let mut retry_after_cached_channel_failure = true;
    let response = loop {
        let channel = peer_channel_for(&spec.endpoint, peer_channels).await?;
        let mut client = peer_client(channel);
        let mut rpc = rpc_metrics.begin_client_call(dms_metrics::RpcCall::PEER_PULL_BLOCK);
        match client
            .pull_block(pb::PeerPullBlockRequest {
                source_node_id: source_node_id.to_string(),
                block_id: spec.block_id.clone(),
                offset,
                length,
                expected_length: Some(spec.expected_length),
                expected_checksum: if validate_whole_block {
                    spec.expected_checksum.clone()
                } else {
                    Vec::new()
                },
            })
            .await
        {
            Ok(response) => {
                rpc.success();
                break response.into_inner();
            }
            Err(status)
                if retry_after_cached_channel_failure && is_retryable_peer_status(&status) =>
            {
                peer_channels.lock().await.remove(&spec.endpoint);
                retry_after_cached_channel_failure = false;
                continue;
            }
            Err(status) => return Err(map_peer_pull_status(status)),
        }
    };
    let expected_payload_length = length.unwrap_or(spec.expected_length);
    if response.block_id != spec.block_id
        || response.length != spec.expected_length
        || response.payload.len() as u64 != expected_payload_length
        || (validate_whole_block
            && !spec.expected_checksum.is_empty()
            && response.checksum != spec.expected_checksum)
        // 整块响应的实际 bytes 留给最终 owner 校验；上面仍验证与权威摘要
        // 一致。分段响应则必须在这里校验，最终 owner 还会检查聚合整块摘要。
        || (!validate_whole_block
            && !response.checksum.is_empty()
            && digest(&response.payload) != response.checksum)
    {
        return Err(WorkerError::Conflict);
    }
    Ok(PeerBlockResult {
        serving_node_id: response.serving_node_id,
        block_id: response.block_id,
        payload: response.payload,
        checksum: response.checksum,
        length: response.length,
    })
}

async fn peer_channel_for(
    endpoint: &str,
    peer_channels: &Arc<Mutex<HashMap<String, Channel>>>,
) -> Result<Channel, WorkerError> {
    if let Some(channel) = peer_channels.lock().await.get(endpoint).cloned() {
        return Ok(channel);
    }
    let endpoint_uri = endpoint.to_string();
    let endpoint = Endpoint::from_shared(endpoint_uri.clone())
        .map_err(|_| WorkerError::TransferUnavailable)?;
    let security =
        SecurityManager::new(TlsConfig::Disabled).map_err(|_| WorkerError::TransferUnavailable)?;
    let grpc = GrpcConfig::default();
    let endpoint = security
        .configure_client(grpc.configure_client(endpoint))
        .map_err(|_| WorkerError::TransferUnavailable)?;
    let channel = endpoint
        .connect()
        .await
        .map_err(|status| transfer_unavailable(format!("peer connect failed: {status}")))?;
    let mut channels = peer_channels.lock().await;
    if let Some(existing) = channels.get(&endpoint_uri).cloned() {
        return Ok(existing);
    }
    if channels.len() >= PEER_CHANNEL_CACHE_LIMIT
        && let Some(evicted_endpoint) = channels.keys().next().cloned()
    {
        channels.remove(&evicted_endpoint);
    }
    channels.insert(endpoint_uri, channel.clone());
    Ok(channel)
}

fn peer_client(
    channel: Channel,
) -> pb::peer_service_client::PeerServiceClient<dms_tracing::TracedChannel> {
    let config = GrpcConfig::default();
    pb::peer_service_client::PeerServiceClient::new(dms_tracing::traced_channel(channel))
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes)
}

fn is_retryable_peer_status(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::Unavailable | tonic::Code::Unknown | tonic::Code::DeadlineExceeded
    )
}

fn map_peer_pull_status(status: tonic::Status) -> WorkerError {
    match status.code() {
        tonic::Code::NotFound => WorkerError::NotFound,
        tonic::Code::InvalidArgument => WorkerError::InvalidArgument("invalid peer request"),
        tonic::Code::FailedPrecondition | tonic::Code::Aborted => WorkerError::Conflict,
        _ => transfer_unavailable(format!("peer PullBlock failed: {status}")),
    }
}

fn transfer_unavailable(message: String) -> WorkerError {
    WorkerError::Stable(DmsError::new(
        dms_error::NODE_TRANSFER_UNAVAILABLE,
        ErrorKind::Unavailable,
        message,
    ))
}

#[cfg(test)]
#[path = "peer_integrity_tests.rs"]
mod peer_integrity_tests;

#[cfg(test)]
#[path = "peer_import_tests.rs"]
mod peer_import_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_import_validates_owned_bytes_before_publishing() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let bytes = b"peer-value".to_vec();
        let checksum = digest(&bytes);
        let mut corrupt = bytes.clone();
        corrupt[0] ^= 1;
        assert!(matches!(
            state.import_peer_block(
                u64::MAX,
                b"bad".to_vec(),
                corrupt,
                checksum.clone(),
                bytes.len() as u64
            ),
            Err(WorkerError::Conflict)
        ));
        assert!(state.arena.read_bytes(b"bad").is_none());
        assert!(matches!(
            state.import_peer_block(
                u64::MAX,
                b"short".to_vec(),
                bytes.clone(),
                checksum.clone(),
                1
            ),
            Err(WorkerError::Conflict)
        ));
        assert!(state.arena.read_bytes(b"short").is_none());
        state
            .import_peer_block(
                u64::MAX,
                b"ok".to_vec(),
                bytes.clone(),
                checksum,
                bytes.len() as u64,
            )
            .unwrap();
        assert_eq!(state.arena.read_bytes(b"ok"), Some(bytes));
    }

    #[test]
    fn retiring_peer_import_allows_cutoff_scope_without_republishing() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let old_scope = state.begin_read_scope(session).unwrap();
        let (prepare_tx, mut prepare_rx) = oneshot::channel();
        state.register_prepare_retirement(
            b"retire-peer-import".to_vec(),
            vec![b"block".to_vec()],
            prepare_tx,
        );
        assert!(prepare_rx.try_recv().is_err());

        let bytes = b"peer-value".to_vec();
        let checksum = digest(&bytes);
        assert!(
            !state
                .import_peer_block(
                    old_scope,
                    b"block".to_vec(),
                    bytes.clone(),
                    checksum.clone(),
                    bytes.len() as u64,
                )
                .unwrap(),
            "old read scope may complete but must not re-publish a retiring block"
        );
        assert_eq!(state.arena.read_bytes(b"block"), Some(bytes.clone()));
        let new_scope = state.begin_read_scope(session).unwrap();
        assert!(matches!(
            state.import_peer_block(
                new_scope,
                b"block".to_vec(),
                bytes,
                checksum,
                b"peer-value".len() as u64,
            ),
            Err(WorkerError::Conflict)
        ));

        state.finish_read_scope(old_scope);
        assert_eq!(prepare_rx.try_recv().unwrap().unwrap(), ());
    }

    #[tokio::test]
    async fn cancelled_begin_read_scope_reply_does_not_leak_scope() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let (reply, receiver) = oneshot::channel();
        drop(receiver);

        let node = NodeHandle::spawn_without_metadata("cleanup-node".into());
        state.begin_read_scope_reply(session, node, reply);

        assert!(state.active_read_scopes.is_empty());
    }

    #[tokio::test]
    async fn unclaimed_successful_begin_read_scope_reply_does_not_leak_scope() {
        let node = NodeHandle::spawn_without_metadata("node-a".into());
        let session = node.open_session(false).await.expect("open session");
        let (reply, receiver) = oneshot::channel();
        node.submit(NodeCommand::BeginReadScope {
            session_id: session,
            cleanup_node: Box::new(node.clone()),
            reply,
        })
        .await
        .expect("submit begin scope");
        node.validate_session(session)
            .await
            .expect("begin reply has been sent before this command");

        drop(receiver);

        let (reply, receiver) = oneshot::channel();
        node.submit(NodeCommand::PrepareBlockRetirement {
            retirement_id: b"unclaimed-scope".to_vec(),
            block_ids: vec![b"block".to_vec()],
            reply,
        })
        .await
        .expect("submit prepare");
        tokio::time::timeout(Duration::from_secs(3), receive(receiver))
            .await
            .expect("unclaimed successful begin reply must drop its lease")
            .expect("prepare after unclaimed begin reply");
    }

    #[tokio::test]
    async fn dropped_read_scope_guard_finishes_scope() {
        let node = NodeHandle::spawn_without_metadata("node-a".into());
        let session = node.open_session(false).await.expect("open session");
        let guard = ReadScopeGuard::begin(&node, session)
            .await
            .expect("begin read scope");
        let scope_id = guard.id();

        drop(guard);

        let (reply, receiver) = oneshot::channel();
        node.submit(NodeCommand::PrepareBlockRetirement {
            retirement_id: b"drop-scope".to_vec(),
            block_ids: vec![b"block".to_vec()],
            reply,
        })
        .await
        .expect("submit prepare");
        tokio::time::timeout(Duration::from_secs(3), receive(receiver))
            .await
            .unwrap_or_else(|_| panic!("dropped read scope {scope_id} did not finish"))
            .expect("prepare after dropped scope");
    }

    fn test_extent(
        logical_offset: u64,
        length: u64,
        block_id: &[u8],
        block_offset: u64,
    ) -> pb::ExtentRecord {
        pb::ExtentRecord {
            logical: Some(pb::ByteRange {
                offset: logical_offset,
                length,
            }),
            block_id: block_id.to_vec(),
            block_offset,
            digest: block_id.to_vec(),
        }
    }

    fn resolved_value(
        version: u64,
        logical_length: u64,
        extents: Vec<pb::ExtentRecord>,
    ) -> pb::ResolveObjectResponse {
        pb::ResolveObjectResponse {
            layout: Some(pb::VersionLayout {
                version,
                logical_length,
                extents,
                digest: b"test-digest".to_vec(),
                kind: pb::VersionKind::Value as i32,
            }),
            block_replicas: Vec::new(),
            current_lease: None,
        }
    }

    fn replica_set(block_id: &[u8], length: u64) -> pb::BlockReplicaSet {
        pb::BlockReplicaSet {
            block_id: block_id.to_vec(),
            replicas: vec![pb::ReplicaLocation {
                block_id: block_id.to_vec(),
                node_id: 2,
                node_epoch: 1,
                data_endpoint: "http://127.0.0.1:25299".to_string(),
                checksum: block_id.to_vec(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            }],
            length,
            proofs: Vec::new(),
        }
    }

    #[test]
    fn only_confirmed_metadata_rejections_allow_block_retirement() {
        for code in [
            dms_error::META_CATALOG_INVALID_REQUEST,
            dms_error::META_CATALOG_VERSION_CONFLICT,
        ] {
            assert!(is_definitive_metadata_rejection(&DmsError::new(
                code,
                ErrorKind::InvalidArgument,
                "rejected before commit"
            )));
        }
        for code in [
            dms_error::META_JOURNAL_APPEND_FAILED,
            dms_error::META_JOURNAL_UNAVAILABLE,
            dms_error::NODE_METADATA_UNAVAILABLE,
        ] {
            assert!(!is_definitive_metadata_rejection(&DmsError::new(
                code,
                ErrorKind::Unavailable,
                "unknown commit outcome"
            )));
        }
    }

    #[test]
    fn disconnected_session_cannot_create_new_staging_or_reattach() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state.sessions.get_mut(&session).unwrap().cache_until =
            Some(Instant::now() + Duration::from_secs(1));
        state.close_session(session).unwrap();
        assert!(matches!(
            state.allocate_staging(session, 4),
            Err(WorkerError::UnknownSession)
        ));
        let (sender, _receiver) = mpsc::channel(1);
        assert!(matches!(
            state.attach_session(session, sender),
            Err(WorkerError::UnknownSession)
        ));
        state.sessions.get_mut(&session).unwrap().cache_until = Some(Instant::now());
        state.expire_cache_leases();
        assert!(!state.sessions.contains_key(&session));
        assert!(matches!(
            state.allocate_staging(session, 4),
            Err(WorkerError::UnknownSession)
        ));
    }

    #[tokio::test]
    async fn checkpoint_failure_after_commit_preserves_published_block() {
        let meta = crate::meta::runtime::failing_checkpoint_handle_for_test();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let service = crate::meta::metadata_service::MetadataServiceHandler::new(meta);
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(pb::metadata_service_server::MetadataServiceServer::new(
                    service,
                ))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        let metadata = MetadataClient::connect(&endpoint, 7, "http://127.0.0.1:19007".into(), None)
            .await
            .unwrap();
        let mut state = NodeState::new(
            "n".into(),
            Some(metadata.clone()),
            4096,
            Duration::from_secs(30),
            None,
        );
        let session = state.open_session(false);
        let operation = vec![5; 24];
        let prepared = state
            .commit_bytes(
                session,
                b"key".to_vec(),
                b"published".to_vec(),
                operation.clone(),
                "any".into(),
            )
            .unwrap();
        let error = prepared.await(&mut state).unwrap_err();
        assert_eq!(
            worker_error_to_dms(error).code(),
            dms_error::META_JOURNAL_UNAVAILABLE
        );
        assert!(
            metadata
                .resolve(b"key".to_vec(), None)
                .await
                .unwrap()
                .layout
                .unwrap()
                .version
                > 0
        );
        assert_eq!(
            state.arena.read_bytes(&block_identity("n", &operation)),
            Some(b"published".to_vec()),
            "published metadata still references this block"
        );
        server.abort();
    }

    #[tokio::test]
    async fn rejected_operation_reuse_cannot_retire_another_write_block() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let service = crate::meta::metadata_service::MetadataServiceHandler::new(
            crate::meta::runtime::MetaHandle::spawn(),
        );
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(pb::metadata_service_server::MetadataServiceServer::new(
                    service,
                ))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        let metadata = MetadataClient::connect(&endpoint, 7, "http://127.0.0.1:19007".into(), None)
            .await
            .unwrap();
        let mut state = NodeState::new(
            "n".into(),
            Some(metadata),
            4096,
            Duration::from_secs(30),
            None,
        );
        let session = state.open_session(false);
        let operation = vec![1; 24];
        let first = state
            .commit_bytes(
                session,
                b"a".to_vec(),
                b"value".to_vec(),
                operation.clone(),
                "any".into(),
            )
            .unwrap();
        assert!(matches!(
            state.commit_bytes(
                session,
                b"b".to_vec(),
                b"value".to_vec(),
                operation.clone(),
                "any".into()
            ),
            Err(WorkerError::WorkerUnavailable)
        ));
        first.await(&mut state).unwrap();
        let second = state
            .commit_bytes(
                session,
                b"b".to_vec(),
                b"value".to_vec(),
                operation.clone(),
                "any".into(),
            )
            .unwrap();
        assert!(second.await(&mut state).is_err());
        assert_eq!(
            state
                .arena
                .read_bytes(&block_identity("n", &operation))
                .unwrap(),
            b"value"
        );
        assert!(state.pending_blocks.is_empty());
        server.abort();
    }

    #[test]
    fn disconnected_client_obligation_ends_only_at_granted_cache_deadline() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let (sender, receiver) = mpsc::channel(1);
        state.attach_session(session, sender).unwrap();
        state.metadata_watch_connected = true;
        state.metadata_lease_until = Instant::now() + Duration::from_secs(5);
        let ttl = state.heartbeat(session, None, true).unwrap();
        assert!(ttl > 0 && ttl <= 1000);
        let granted = state.sessions[&session].cache_until.unwrap();
        state.heartbeat(session, None, false).unwrap();
        assert_eq!(
            state.sessions[&session].cache_until,
            Some(granted),
            "one-way heartbeat is not a cache grant"
        );
        state
            .sessions
            .get_mut(&session)
            .unwrap()
            .cached_current_keys
            .insert(b"key".to_vec());
        drop(receiver);
        let barrier = state.broadcast_invalidation(b"key".to_vec(), 2).unwrap();
        state.close_session(session).unwrap();
        assert!(state.barriers.contains_key(&barrier));
        assert!(state.heartbeat(session, None, true).is_err());
        state.sessions.get_mut(&session).unwrap().cache_until = Some(Instant::now());
        state.expire_cache_leases();
        assert!(!state.barriers.contains_key(&barrier));
        assert!(!state.sessions.contains_key(&session));
    }

    #[test]
    fn disconnected_cached_session_does_not_block_unrelated_key() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state.sessions.get_mut(&session).unwrap().cache_until =
            Some(Instant::now() + Duration::from_secs(5));
        state
            .register_cache_interest(session, b"model/a".to_vec())
            .unwrap();
        state.close_session(session).unwrap();

        assert_eq!(state.broadcast_invalidation(b"model/b".to_vec(), 2), None);
        assert!(
            state
                .broadcast_invalidation(b"model/a".to_vec(), 2)
                .is_some()
        );
    }

    #[test]
    fn acknowledgement_does_not_clear_current_cache_interest() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let (sender, mut receiver) = mpsc::channel(1);
        state.attach_session(session, sender).unwrap();
        state.sessions.get_mut(&session).unwrap().cache_until =
            Some(Instant::now() + Duration::from_secs(5));
        state
            .register_cache_interest(session, b"model/a".to_vec())
            .unwrap();

        let barrier = state
            .broadcast_invalidation(b"model/a".to_vec(), 2)
            .unwrap();
        let event = receiver.try_recv().unwrap();
        let sequence = match event {
            NodeEvent::InvalidateCurrent { event_sequence, .. } => event_sequence,
        };
        let (waiter, mut completion) = oneshot::channel();
        state.attach_barrier_waiter(barrier, waiter);
        state.acknowledge(session, sequence).unwrap();
        completion.try_recv().unwrap().unwrap();

        assert!(
            state
                .sessions
                .get(&session)
                .unwrap()
                .cached_current_keys
                .contains(b"model/a".as_slice()),
            "ACK only completes the current barrier; the key remains protected until lease expiry"
        );
        assert!(
            state
                .broadcast_invalidation(b"model/a".to_vec(), 3)
                .is_some(),
            "a later same-key write still has to invalidate this session during the lease"
        );
    }

    #[test]
    fn cache_grants_require_watch_and_are_bounded_by_meta_lease() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let (sender, _receiver) = mpsc::channel(1);
        state.attach_session(session, sender).unwrap();
        state.metadata_lease_until = Instant::now() + Duration::from_millis(200);
        assert_eq!(state.heartbeat(session, None, true).unwrap(), 0);
        state.metadata_watch_connected = true;
        let ttl = state.heartbeat(session, None, true).unwrap();
        assert!(ttl > 0 && ttl <= 200);
        assert!(state.sessions[&session].cache_until.unwrap() <= state.metadata_lease_until);
        state.client_cache_lease_ttl = Duration::from_millis(20);
        let configured_ttl = state.heartbeat(session, None, true).unwrap();
        assert!(configured_ttl > 0 && configured_ttl <= 20);
    }

    #[test]
    fn cache_interest_overflow_degrades_to_lease_bounded_wildcard() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state.sessions.get_mut(&session).unwrap().cache_until =
            Some(Instant::now() + Duration::from_secs(5));

        for index in 0..=CLIENT_CACHE_INTEREST_KEY_LIMIT {
            state
                .register_cache_interest(session, format!("key-{index}").into_bytes())
                .unwrap();
        }

        let session_state = state.sessions.get(&session).unwrap();
        assert!(session_state.cache_interest_all);
        assert!(session_state.cached_current_keys.is_empty());
        assert_eq!(session_state.cached_current_key_bytes, 0);
        assert!(
            state
                .broadcast_invalidation(b"unseen-key".to_vec(), 2)
                .is_some()
        );

        state.sessions.get_mut(&session).unwrap().cache_until = Some(Instant::now());
        state.expire_cache_leases();
        let session_state = state.sessions.get(&session).unwrap();
        assert!(!session_state.cache_interest_all);
        assert!(session_state.cached_current_keys.is_empty());
        assert_eq!(session_state.cached_current_key_bytes, 0);
        assert_eq!(
            state.broadcast_invalidation(b"unseen-key".to_vec(), 3),
            None
        );
    }

    #[test]
    fn cache_interest_large_keys_degrade_to_lease_bounded_wildcard() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state.sessions.get_mut(&session).unwrap().cache_until =
            Some(Instant::now() + Duration::from_secs(5));

        state
            .register_cache_interest(session, vec![b'x'; CLIENT_CACHE_INTEREST_BYTES_LIMIT + 1])
            .unwrap();

        let session_state = state.sessions.get(&session).unwrap();
        assert!(session_state.cache_interest_all);
        assert!(session_state.cached_current_keys.is_empty());
        assert_eq!(session_state.cached_current_key_bytes, 0);
        assert!(
            state
                .broadcast_invalidation(b"unseen-key".to_vec(), 2)
                .is_some()
        );
    }

    #[test]
    fn expired_interest_must_register_again_after_new_cache_lease() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let (sender, mut receiver) = mpsc::channel(1);
        state.attach_session(session, sender).unwrap();
        state.metadata_watch_connected = true;
        state.metadata_lease_until = Instant::now() + Duration::from_secs(5);

        assert!(state.heartbeat(session, None, true).unwrap() > 0);
        state
            .register_cache_interest(session, b"model/a".to_vec())
            .unwrap();
        assert!(
            state
                .broadcast_invalidation(b"model/a".to_vec(), 2)
                .is_some(),
            "the first lease protects the key registered by the in-flight GET"
        );
        let event = receiver.try_recv().unwrap();
        let sequence = match event {
            NodeEvent::InvalidateCurrent { event_sequence, .. } => event_sequence,
        };
        state.acknowledge(session, sequence).unwrap();

        state.sessions.get_mut(&session).unwrap().cache_until = Some(Instant::now());
        state.expire_cache_leases();
        assert!(state.heartbeat(session, None, true).unwrap() > 0);
        assert_eq!(
            state.broadcast_invalidation(b"model/a".to_vec(), 3),
            None,
            "a renewed lease starts with an empty interest set; an old GET cannot keep Node waiting"
        );

        state
            .register_cache_interest(session, b"model/a".to_vec())
            .unwrap();
        assert!(
            state
                .broadcast_invalidation(b"model/a".to_vec(), 4)
                .is_some(),
            "a fresh GET under the new lease must register interest again"
        );
    }

    #[test]
    fn next_cache_lease_expiry_uses_earliest_live_deadline() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let first = state.open_session(false);
        let second = state.open_session(false);
        let later = Instant::now() + Duration::from_secs(5);
        let earlier = Instant::now() + Duration::from_millis(25);
        state.sessions.get_mut(&first).unwrap().cache_until = Some(later);
        state.sessions.get_mut(&second).unwrap().cache_until = Some(earlier);

        assert_eq!(state.next_cache_lease_expiry(), Some(earlier));
    }

    #[tokio::test]
    async fn owner_waits_for_same_key_lease_but_wakes_before_maintenance_tick() {
        let node = NodeHandle::spawn_without_metadata("expiry-test".into());
        let session = node.open_session(false).await.unwrap();
        let (sender, receiver) = mpsc::channel(1);
        node.attach_session(session, sender).await.unwrap();
        // 缩短上游资格以授予 200ms 本地租约；不改变正式默认配置。
        node.metadata_lease(
            Some(Instant::now() + Duration::from_millis(200)),
            Some(true),
        )
        .await
        .unwrap();
        let ttl = node.renew_cache_lease(session, None).await.unwrap();
        assert!(ttl > 100 && ttl <= 200);
        // Current 查询把兴趣登记与缓存判断合并为一次 owner 消息。
        let (reply, cache_reply) = oneshot::channel();
        node.submit(NodeCommand::GetCached {
            session_id: session,
            read_scope_id: u64::MAX,
            read_request_id: 0,
            key: b"same-key".to_vec(),
            node_epoch: 0,
            range: None,
            clamp_range: false,
            max_inline_bytes: 0,
            reply,
        })
        .await
        .unwrap();
        receive(cache_reply).await.unwrap();
        drop(receiver);
        node.close_session(session).await.unwrap();

        // 没有 ACK 的断连读者仍保护同 key，不能借优化提前放行。
        let invalidation = node.invalidate_current(b"same-key".to_vec(), 2);
        tokio::pin!(invalidation);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut invalidation)
                .await
                .is_err()
        );
        // 真正跑 owner 的 select/sleep_until；旧的 1s maintenance 实现会超时。
        tokio::time::timeout(Duration::from_millis(600), &mut invalidation)
            .await
            .expect("lease expiry must not wait for the 1s maintenance tick")
            .unwrap();
    }

    #[tokio::test]
    async fn simultaneous_node_writes_keep_heartbeat_and_invalidation_progressing() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let service = crate::meta::metadata_service::MetadataServiceHandler::new(
            crate::meta::runtime::MetaHandle::spawn(),
        );
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(pb::metadata_service_server::MetadataServiceServer::new(
                    service,
                ))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        let mut nodes = Vec::new();
        let mut streams = Vec::new();
        for id in 1..=2 {
            let metadata = MetadataClient::connect(
                &endpoint,
                id,
                format!("http://127.0.0.1:{}", 19000 + id),
                None,
            )
            .await
            .unwrap();
            streams.push((metadata.clone(), metadata.watch_events(0).await.unwrap()));
            let node = NodeHandle::spawn(
                format!("node-{id}"),
                metadata,
                1024 * 1024,
                Duration::from_secs(30),
                None,
            );
            let session = node.open_session(false).await.unwrap();
            nodes.push((node, session));
        }
        for (index, (node, session)) in nodes.iter().cloned().enumerate() {
            node.set_inline(
                session,
                vec![index as u8],
                b"old".to_vec(),
                vec![0x80 + index as u8; 24],
                "any".into(),
            )
            .await
            .unwrap();
        }
        let mut writes = Vec::new();
        for (index, (node, session)) in nodes.iter().cloned().enumerate() {
            writes.push(tokio::spawn(async move {
                node.set_inline(
                    session,
                    vec![index as u8],
                    b"value".to_vec(),
                    vec![index as u8 + 1; 24],
                    "any".into(),
                )
                .await
            }));
        }
        // 先确认双方提交都已到达 Meta，再检查 owner 未被回复等待堵住。
        let mut events = Vec::new();
        for (_, stream) in &mut streams {
            let first = tokio::time::timeout(Duration::from_secs(2), stream.message())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let second = tokio::time::timeout(Duration::from_secs(2), stream.message())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            events.push([first, second]);
        }
        for (index, (node, session)) in nodes.iter().enumerate() {
            tokio::time::timeout(Duration::from_millis(250), node.heartbeat(*session, None))
                .await
                .expect("outbound commit must not block heartbeat")
                .unwrap();
            for event in &events[index] {
                if let Some(pb::node_event::Event::InvalidateCurrent(value)) = &event.event {
                    node.invalidate_current(
                        value.key.as_ref().unwrap().value.clone(),
                        value.minimum_version,
                    )
                    .await
                    .unwrap();
                }
                streams[index].0.acknowledge_event(event).await.unwrap();
            }
        }
        for write in writes {
            tokio::time::timeout(Duration::from_secs(2), write)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        server.abort();
    }

    #[test]
    fn batch_operation_ids_keep_client_identity_and_derive_sequence() {
        let operation_id: Vec<u8> = (0u8..24u8).collect();

        let first = derive_batch_operation_id(&operation_id, 0).expect("first id");
        let second = derive_batch_operation_id(&operation_id, 1).expect("second id");

        assert_eq!(first.len(), 24);
        assert_eq!(second.len(), 24);
        assert_eq!(&first[..16], &operation_id[..16]);
        assert_eq!(&second[..16], &operation_id[..16]);
        assert_ne!(&first[16..], &second[16..]);
    }

    #[test]
    fn batch_operation_ids_reject_invalid_parent_identity() {
        let error = derive_batch_operation_id(&[1, 2, 3], 0).expect_err("invalid length");

        assert!(matches!(error, WorkerError::InvalidArgument(_)));
    }

    #[test]
    fn arena_errors_keep_precise_public_codes() {
        let cases = [
            (
                ArenaError::StaleHandle,
                dms_error::NODE_ARENA_STALE_HANDLE,
                ErrorKind::Unavailable,
            ),
            (
                ArenaError::SharedMemoryUnavailable,
                dms_error::NODE_ARENA_SHM_UNAVAILABLE,
                ErrorKind::Unavailable,
            ),
            (
                ArenaError::RegionAccessDenied,
                dms_error::NODE_ARENA_ACCESS_DENIED,
                ErrorKind::PermissionDenied,
            ),
            (
                ArenaError::UnknownRegion,
                dms_error::NODE_ARENA_STALE_HANDLE,
                ErrorKind::Unavailable,
            ),
            (
                ArenaError::EmptyPayload,
                dms_error::NODE_ARENA_INVALID_REQUEST,
                ErrorKind::InvalidArgument,
            ),
            (
                ArenaError::RegionCreateFailed,
                dms_error::NODE_ARENA_ALLOCATION_FAILED,
                ErrorKind::ResourceExhausted,
            ),
        ];

        for (arena_error, expected_code, expected_kind) in cases {
            let public = worker_error_to_dms(map_arena_error(arena_error));
            assert_eq!(public.code(), expected_code);
            assert_eq!(public.kind(), expected_kind);
            assert_ne!(public.code(), dms_error::NODE_METADATA_UNAVAILABLE);
            assert_ne!(public.code(), dms_error::NODE_VERSION_CONFLICT);
        }
    }

    #[test]
    fn worker_and_transfer_unavailable_use_separate_codes() {
        let worker = worker_error_to_dms(WorkerError::WorkerUnavailable);
        assert_eq!(worker.code(), dms_error::NODE_WORKER_UNAVAILABLE);
        assert_eq!(worker.kind(), ErrorKind::Unavailable);

        let metadata = worker_error_to_dms(WorkerError::MetadataUnavailable);
        assert_eq!(metadata.code(), dms_error::NODE_METADATA_UNAVAILABLE);
        assert_eq!(metadata.kind(), ErrorKind::Unavailable);

        let transfer = worker_error_to_dms(WorkerError::TransferUnavailable);
        assert_eq!(transfer.code(), dms_error::NODE_TRANSFER_UNAVAILABLE);
        assert_eq!(transfer.kind(), ErrorKind::Unavailable);
    }

    #[tokio::test]
    async fn mailbox_correlates_each_result_with_its_request() {
        // 如果 oneshot 关联错请求，这两个并发语义上的返回 ID 就可能串线。
        let node = NodeHandle::spawn_without_metadata("node-a".to_string());
        let first = node.open_session(false).await.expect("first session");
        let second = node.open_session(false).await.expect("second session");
        assert_eq!((first, second), (1, 2));
        node.heartbeat(second, None)
            .await
            .expect("second heartbeat");
    }

    #[tokio::test]
    async fn online_config_updates_staging_ttl_through_node_owner() {
        let node = NodeHandle::spawn_without_metadata("node-a".to_string());

        let version = node
            .apply_config_change(ConfigChange::StagingTtl(Duration::from_millis(5)))
            .await
            .expect("online ttl update");

        assert_eq!(version, 2);
        assert_eq!(
            node.debug_staging_ttl().await.expect("debug ttl"),
            Duration::from_millis(5)
        );
    }

    #[tokio::test]
    async fn invalidation_barrier_completes_only_after_every_session_ack() {
        let mut state = NodeState::new(
            "node-a".to_string(),
            None,
            256 * 1024 * 1024,
            Duration::from_secs(30),
            None,
        );
        let first = state.open_session(false);
        let second = state.open_session(false);
        for session in state.sessions.values_mut() {
            session.cache_until = Some(Instant::now() + Duration::from_secs(5));
            session
                .cached_current_keys
                .insert(b"checkpoint/latest".to_vec());
        }
        let (first_sender, mut first_events) = mpsc::channel(1);
        let (second_sender, mut second_events) = mpsc::channel(1);
        state
            .attach_session(first, first_sender)
            .expect("attach first");
        state
            .attach_session(second, second_sender)
            .expect("attach second");

        let barrier = state
            .broadcast_invalidation(b"checkpoint/latest".to_vec(), 2)
            .expect("barrier");
        let first_event = first_events.try_recv().expect("first event");
        let second_event = second_events.try_recv().expect("second event");
        let first_sequence = match first_event {
            NodeEvent::InvalidateCurrent { event_sequence, .. } => event_sequence,
        };
        let second_sequence = match second_event {
            NodeEvent::InvalidateCurrent { event_sequence, .. } => event_sequence,
        };
        let (waiter, mut completion) = oneshot::channel();
        state.attach_barrier_waiter(barrier, waiter);

        state
            .acknowledge(first, first_sequence)
            .expect("first acknowledgement");
        assert!(matches!(
            completion.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        state
            .acknowledge(second, second_sequence)
            .expect("second acknowledgement");
        completion
            .await
            .expect("barrier sender")
            .expect("barrier result");
    }

    #[test]
    fn close_session_reclaims_owned_staging() {
        let mut state = NodeState::new(
            "node-a".to_string(),
            None,
            4096,
            Duration::from_secs(30),
            None,
        );
        let session = state.open_session(false);
        let allocation = state
            .allocate_staging(session, 5)
            .expect("allocate staging");

        state.close_session(session).expect("close session");

        assert_eq!(state.arena.stats().staging_count, 0);
        assert_eq!(state.arena.stats().logical_bytes, 0);
        assert!(matches!(
            state.delete_staging(session, allocation.staging_id),
            Err(WorkerError::UnknownSession)
        ));
    }

    #[test]
    fn download_ticket_is_one_shot_handle_and_can_expire() {
        let mut state = NodeState::new(
            "node-a".to_string(),
            None,
            4096,
            Duration::from_secs(30),
            None,
        );
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block-1".to_vec(), b"abcdef".to_vec())
            .expect("commit inline");
        let (read, _) = state
            .arena
            .open_read(b"block-1", Some((1, 3)))
            .expect("open read");

        state.downloads.insert(
            1,
            DownloadTicket {
                read,
                session_id: session,
                read_request_id: 0,
                expires_at: Instant::now() + Duration::from_secs(30),
            },
        );
        assert_eq!(state.download(1).expect("download"), b"bcd");
        assert!(matches!(
            state.download(1),
            Err(WorkerError::UnknownTransfer)
        ));

        state.downloads.insert(
            2,
            DownloadTicket {
                read,
                session_id: session,
                read_request_id: 0,
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );
        assert!(matches!(
            state.download(2),
            Err(WorkerError::UnknownTransfer)
        ));
    }

    #[test]
    fn get_resolved_zero_inline_budget_keeps_legacy_download_ticket() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block-1".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(7, 3, vec![test_extent(0, 3, b"block-1", 0)]);

        let ticket = match state
            .get_resolved(session, &resolved, None, 0)
            .expect("get resolved")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("block is local"),
        };

        assert_eq!(ticket.inline_value, None);
        assert_eq!(ticket.segments.len(), 1);
        assert_eq!(state.downloads.len(), 1);
    }

    #[test]
    fn get_resolved_small_non_shm_read_returns_inline_without_download_ticket() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block-1".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(8, 3, vec![test_extent(0, 3, b"block-1", 0)]);

        let ticket = match state
            .get_resolved(session, &resolved, None, 3)
            .expect("get resolved")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("block is local"),
        };

        assert_eq!(ticket.inline_value.as_deref(), Some(b"abc".as_slice()));
        assert!(ticket.segments.is_empty());
        assert!(state.downloads.is_empty());
    }

    #[test]
    fn get_resolved_inline_budget_is_clamped_by_protocol_limit() {
        let mut state = NodeState::new(
            "node-a".into(),
            None,
            dms_protocol::MAX_INLINE_READ_BYTES + 4096,
            Duration::from_secs(30),
            None,
        );
        let session = state.open_session(false);
        let value = vec![b'x'; dms_protocol::MAX_INLINE_READ_BYTES as usize + 1];
        state
            .arena
            .commit_inline(b"large-block".to_vec(), value)
            .expect("commit block");
        let resolved = resolved_value(
            9,
            dms_protocol::MAX_INLINE_READ_BYTES + 1,
            vec![test_extent(
                0,
                dms_protocol::MAX_INLINE_READ_BYTES + 1,
                b"large-block",
                0,
            )],
        );

        let ticket = match state
            .get_resolved(session, &resolved, None, u64::MAX)
            .expect("get resolved")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("block is local"),
        };

        assert_eq!(ticket.inline_value, None);
        assert_eq!(ticket.segments.len(), 1);
        assert_eq!(state.downloads.len(), 1);
    }

    #[test]
    fn get_resolved_range_overlay_can_return_inline_from_bound_version() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"base".to_vec(), b"abcdef".to_vec())
            .expect("commit base");
        state
            .arena
            .commit_inline(b"patch".to_vec(), b"X".to_vec())
            .expect("commit patch");
        let resolved = resolved_value(
            10,
            6,
            vec![
                test_extent(0, 2, b"base", 0),
                test_extent(2, 1, b"patch", 0),
                test_extent(3, 3, b"base", 3),
            ],
        );

        let ticket = match state
            .get_resolved(session, &resolved, Some((1, 3)), 3)
            .expect("get resolved")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("blocks are local"),
        };

        assert_eq!(ticket.version, 10);
        assert_eq!(ticket.logical_length, 6);
        assert_eq!(ticket.inline_value.as_deref(), Some(b"bXd".as_slice()));
        assert!(ticket.segments.is_empty());
        assert!(state.downloads.is_empty());
    }

    #[test]
    fn strict_read_range_past_end_stays_invalid() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(12, 3, vec![test_extent(0, 3, b"block", 0)]);

        assert!(matches!(
            state.get_resolved(session, &resolved, Some((2, 2)), 64),
            Err(WorkerError::InvalidArgument(
                "read range extends beyond current value"
            ))
        ));
    }

    #[test]
    fn clamp_range_past_end_returns_empty_without_download_ticket() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(13, 3, vec![test_extent(0, 3, b"block", 0)]);

        let ticket = match state
            .get_resolved_for_request(session, u64::MAX, 1, &resolved, Some((3, 9)), true, 64)
            .expect("clamped get")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("empty range must not need peer blocks"),
        };

        assert_eq!(ticket.version, 13);
        assert_eq!(ticket.logical_length, 3);
        assert_eq!(ticket.inline_value.as_deref(), Some([].as_slice()));
        assert!(ticket.segments.is_empty());
        assert!(state.downloads.is_empty());
    }

    #[test]
    fn clamp_range_preserves_zero_length_as_empty() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(14, 3, vec![test_extent(0, 3, b"block", 0)]);

        let ticket = match state
            .get_resolved_for_request(session, u64::MAX, 2, &resolved, Some((1, 0)), true, 64)
            .expect("zero-length get")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("empty range must not need peer blocks"),
        };

        assert_eq!(ticket.inline_value.as_deref(), Some([].as_slice()));
        assert!(ticket.segments.is_empty());
        assert!(state.downloads.is_empty());
    }

    #[test]
    fn clamp_range_clips_tail_across_multiple_extents() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"left".to_vec(), b"abc".to_vec())
            .expect("commit left");
        state
            .arena
            .commit_inline(b"right".to_vec(), b"def".to_vec())
            .expect("commit right");
        let resolved = resolved_value(
            14,
            6,
            vec![
                test_extent(0, 3, b"left", 0),
                test_extent(3, 3, b"right", 0),
            ],
        );

        let ticket = match state
            .get_resolved_for_request(session, u64::MAX, 2, &resolved, Some((2, 99)), true, 64)
            .expect("clamped get")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("blocks are local"),
        };

        assert_eq!(ticket.inline_value.as_deref(), Some(b"cdef".as_slice()));
        assert!(ticket.segments.is_empty());
    }

    #[test]
    fn clamp_range_keeps_overflow_invalid_before_tail_clip() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(15, 3, vec![test_extent(0, 3, b"block", 0)]);

        assert!(matches!(
            state.get_resolved_for_request(
                session,
                u64::MAX,
                3,
                &resolved,
                Some((u64::MAX, 1)),
                true,
                64
            ),
            Err(WorkerError::InvalidArgument("read range overflows u64"))
        ));
    }

    #[test]
    fn clamp_range_still_rejects_invalid_layout_gap() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"left".to_vec(), b"ab".to_vec())
            .expect("commit left");
        state
            .arena
            .commit_inline(b"right".to_vec(), b"d".to_vec())
            .expect("commit right");
        let resolved = resolved_value(
            16,
            4,
            vec![
                test_extent(0, 2, b"left", 0),
                test_extent(3, 1, b"right", 0),
            ],
        );

        assert!(matches!(
            state.get_resolved_for_request(
                session,
                u64::MAX,
                4,
                &resolved,
                Some((1, 99)),
                true,
                64
            ),
            Err(WorkerError::InvalidArgument(
                "version layout contains a gap, overlap, or empty extent"
            ))
        ));
    }

    #[test]
    fn clamp_range_peer_fallback_uses_clipped_resolved_version() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"left".to_vec(), b"abc".to_vec())
            .expect("commit left");
        let mut resolved = resolved_value(
            17,
            6,
            vec![
                test_extent(0, 3, b"left", 0),
                test_extent(3, 3, b"remote", 0),
            ],
        );
        resolved.block_replicas = vec![replica_set(b"remote", 3)];

        let specs = match state
            .get_resolved_for_request(session, u64::MAX, 5, &resolved, Some((4, 99)), true, 64)
            .expect("clamped missing-block get")
        {
            GetOutcome::Ready(_) => panic!("remote extent is missing"),
            GetOutcome::NeedsRemoteBlocks(specs) => specs,
        };

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].block_id, b"remote".to_vec());
        assert_eq!(specs[0].expected_length, 3);
    }

    #[test]
    fn current_cache_clamped_range_reuses_layout_without_full_interest() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state.metadata_watch_connected = true;
        state.metadata_lease_until = Instant::now() + Duration::from_secs(30);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let mut resolved = resolved_value(18, 3, vec![test_extent(0, 3, b"block", 0)]);
        resolved.current_lease = Some(pb::CurrentLeaseGrant {
            version: 18,
            lease_epoch: 0,
            leader_epoch: 0,
            revision: 0,
            ttl_millis: 30_000,
        });
        let now = Instant::now();
        assert!(state.current_cache.insert(
            state.current_cache.token().expect("cache token"),
            b"key".to_vec(),
            &resolved,
            now,
            0,
            now
        ));

        let (_token, outcome) = state
            .get_cached(session, u64::MAX, 6, b"key", 0, Some((1, 99)), true, 64)
            .expect("cached read");
        let CachedReadOutcome::Ready(ticket) = outcome else {
            panic!("cached layout should satisfy clamped local range");
        };

        assert_eq!(ticket.version, 18);
        assert_eq!(ticket.inline_value.as_deref(), Some(b"bc".as_slice()));
        assert!(
            !state.sessions[&session]
                .cached_current_keys
                .contains(b"key".as_slice())
        );
    }

    #[test]
    fn get_resolved_shm_session_ignores_inline_budget() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(true);
        state
            .arena
            .commit_inline(b"block-1".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(11, 3, vec![test_extent(0, 3, b"block-1", 0)]);

        let ticket = match state
            .get_resolved(session, &resolved, None, 3)
            .expect("get resolved")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("block is local"),
        };

        assert_eq!(ticket.inline_value, None);
        assert_eq!(ticket.segments.len(), 1);
        assert_eq!(state.downloads.len(), 1);
        // 协商 SHM 不等于真的借出了共享页；Private backing 回退下载不能消耗序号。
        assert_eq!(state.sessions[&session].next_view_epoch, 1);
    }

    #[test]
    fn copied_tcp_reads_do_not_consume_shared_view_epochs() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);
        for inline_budget in [0, 3] {
            state
                .get_resolved(session, &resolved, None, inline_budget)
                .unwrap();
            assert_eq!(state.sessions[&session].next_view_epoch, 1);
        }
    }

    #[test]
    fn finished_read_request_releases_lost_tcp_ticket_without_view_epoch() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);

        assert!(matches!(
            state
                .get_resolved_for_request(session, u64::MAX, 7, &resolved, None, false, 0)
                .unwrap(),
            GetOutcome::Ready(_)
        ));
        assert_eq!(state.downloads.len(), 1);

        state
            .heartbeat_inner(session, None, Vec::new(), Some(7), false)
            .unwrap();

        assert!(state.downloads.is_empty());
        assert!(
            state
                .try_finalize_block_retirement(vec![b"block".to_vec()])
                .unwrap()
        );
    }

    #[test]
    fn cancel_first_read_request_refuses_late_ticket_creation() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);
        state
            .heartbeat_inner(session, None, Vec::new(), Some(7), false)
            .unwrap();

        assert!(matches!(
            state.get_resolved_for_request(session, u64::MAX, 7, &resolved, None, false, 0),
            Err(WorkerError::InvalidArgument(_))
        ));
        assert!(state.downloads.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finished_read_request_releases_lost_shm_view_without_epoch_watermark() {
        let path = std::env::temp_dir().join(format!("dms-lost-view-{}.sock", std::process::id()));
        let broker = SharedFdBroker::bind(path.clone()).unwrap();
        let mut state = NodeState::new(
            "node-a".into(),
            None,
            4096,
            Duration::from_secs(30),
            Some(broker),
        );
        let session = state.open_session(true);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);
        assert!(matches!(
            state
                .get_resolved_for_request(session, u64::MAX, 11, &resolved, None, false, 0)
                .unwrap(),
            GetOutcome::Ready(_)
        ));
        assert_eq!(state.sessions[&session].next_view_epoch, 2);
        assert_eq!(state.sessions[&session].released_view_through, 0);
        assert_eq!(state.sessions[&session].active_views.len(), 1);

        state
            .heartbeat_inner(session, None, Vec::new(), Some(11), false)
            .unwrap();

        assert!(state.sessions[&session].active_views.is_empty());
        assert_eq!(state.sessions[&session].released_view_through, 0);
        assert!(
            state
                .try_finalize_block_retirement(vec![b"block".to_vec()])
                .unwrap()
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn heartbeat_rejects_view_watermark_beyond_granted_epoch() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(true);

        assert!(state.heartbeat(session, Some(1), false).is_err());
        assert_eq!(state.sessions[&session].released_view_through, 0);
    }

    #[test]
    fn retirement_prepare_is_event_driven_and_blocks_new_cached_borrows() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);
        let old_scope = state.begin_read_scope(session).unwrap();
        let (prepare_tx, mut prepare_rx) = oneshot::channel();

        state.register_prepare_retirement(
            b"retire-1".to_vec(),
            vec![b"block".to_vec()],
            prepare_tx,
        );
        assert!(prepare_rx.try_recv().is_err());
        let new_scope = state.begin_read_scope(session).unwrap();
        assert!(matches!(
            state.get_resolved_for_request(session, new_scope, 1, &resolved, None, false, 0),
            Err(WorkerError::NotFound)
        ));

        state.finish_read_scope(old_scope);

        assert_eq!(prepare_rx.try_recv().unwrap().unwrap(), ());
        assert!(state.arena.block_allocation_id(b"block").is_some());
    }

    #[test]
    fn retirement_final_completes_from_release_event_without_polling() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);
        let ticket = match state
            .get_resolved_for_request(session, u64::MAX, 9, &resolved, None, false, 0)
            .unwrap()
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("local block must be ready"),
        };
        let transfer_id = match &ticket.segments[0].target {
            ReadTarget::Grpc { transfer_id } => *transfer_id,
            ReadTarget::Shm(_) => panic!("expected TCP ticket"),
        };
        let (prepare_tx, mut prepare_rx) = oneshot::channel();
        state.register_prepare_retirement(
            b"retire-1".to_vec(),
            vec![b"block".to_vec()],
            prepare_tx,
        );
        assert_eq!(prepare_rx.try_recv().unwrap().unwrap(), ());
        let (final_tx, mut final_rx) = oneshot::channel();
        state.register_final_retirement(b"retire-1".to_vec(), vec![b"block".to_vec()], final_tx);
        assert!(final_rx.try_recv().is_err());

        assert_eq!(state.download(transfer_id).unwrap(), b"abc");

        assert_eq!(final_rx.try_recv().unwrap().unwrap(), ());
        assert!(state.arena.block_allocation_id(b"block").is_none());

        let (replay_tx, mut replay_rx) = oneshot::channel();
        state.register_final_retirement(b"retire-1".to_vec(), vec![b"block".to_vec()], replay_tx);
        assert_eq!(replay_rx.try_recv().unwrap().unwrap(), ());
    }

    #[test]
    fn prepare_retirement_waits_only_cutoff_read_scopes() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let old_scope = state.begin_read_scope(session).unwrap();

        let cutoff = state
            .prepare_block_retirement(vec![b"block".to_vec()])
            .unwrap();
        let later_scope = state.begin_read_scope(session).unwrap();
        assert!(!state.read_scopes_drained(cutoff));

        state.finish_read_scope(old_scope);

        assert!(state.read_scopes_drained(cutoff));
        assert!(state.active_read_scopes.contains(&later_scope));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn final_retirement_waits_for_read_view_release() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dms-final-view-{}-{unique}.sock",
            std::process::id()
        ));
        let broker = SharedFdBroker::bind(path.clone()).unwrap();
        let mut state = NodeState::new(
            "node-a".into(),
            None,
            4096,
            Duration::from_secs(30),
            Some(broker),
        );
        let session = state.open_session(true);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);
        assert!(matches!(
            state.get_resolved(session, &resolved, None, 0).unwrap(),
            GetOutcome::Ready(_)
        ));

        assert!(
            !state
                .try_finalize_block_retirement(vec![b"block".to_vec()])
                .unwrap()
        );
        state.heartbeat(session, Some(1), false).unwrap();
        assert!(
            state
                .try_finalize_block_retirement(vec![b"block".to_vec()])
                .unwrap()
        );
        assert!(state.arena.block_allocation_id(b"block").is_none());
        let _ = std::fs::remove_file(path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn disconnected_read_view_keeps_retirement_blocked_until_release() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dms-disconnected-view-{}-{unique}.sock",
            std::process::id()
        ));
        let broker = SharedFdBroker::bind(path.clone()).unwrap();
        let mut state = NodeState::new(
            "node-a".into(),
            None,
            4096,
            Duration::from_secs(30),
            Some(broker),
        );
        let session = state.open_session(true);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);
        assert!(matches!(
            state.get_resolved(session, &resolved, None, 0).unwrap(),
            GetOutcome::Ready(_)
        ));
        let (prepare_tx, mut prepare_rx) = oneshot::channel();
        state.register_prepare_retirement(
            b"retire-disconnected".to_vec(),
            vec![b"block".to_vec()],
            prepare_tx,
        );
        assert_eq!(prepare_rx.try_recv().unwrap().unwrap(), ());

        state.close_session(session).unwrap();
        assert!(state.sessions.contains_key(&session));
        let (final_tx, mut final_rx) = oneshot::channel();
        state.register_final_retirement(
            b"retire-disconnected".to_vec(),
            vec![b"block".to_vec()],
            final_tx,
        );
        assert!(final_rx.try_recv().is_err());
        assert!(state.arena.block_allocation_id(b"block").is_some());

        state.heartbeat(session, Some(1), false).unwrap();

        assert_eq!(final_rx.try_recv().unwrap().unwrap(), ());
        assert!(state.arena.block_allocation_id(b"block").is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn final_retirement_waits_for_tcp_download_ticket() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abc".to_vec())
            .unwrap();
        let resolved = resolved_value(1, 3, vec![test_extent(0, 3, b"block", 0)]);
        let ticket = match state.get_resolved(session, &resolved, None, 0).unwrap() {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("local block must be ready"),
        };
        let transfer_id = match &ticket.segments[0].target {
            ReadTarget::Grpc { transfer_id } => *transfer_id,
            ReadTarget::Shm(_) => panic!("expected TCP ticket"),
        };

        assert!(
            !state
                .try_finalize_block_retirement(vec![b"block".to_vec()])
                .unwrap()
        );
        assert_eq!(state.download(transfer_id).unwrap(), b"abc");
        assert!(
            state
                .try_finalize_block_retirement(vec![b"block".to_vec()])
                .unwrap()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn final_retirement_waits_for_write_release_token() {
        let path =
            std::env::temp_dir().join(format!("dms-final-write-{}.sock", std::process::id()));
        let broker = SharedFdBroker::bind(path.clone()).unwrap();
        let mut state = NodeState::new(
            "node-a".into(),
            None,
            4096,
            Duration::from_secs(30),
            Some(broker),
        );
        let session = state.open_session_with_write_release(true, true);
        let allocation = state.allocate_staging(session, 1).unwrap();
        let release = match &allocation.target {
            HostAllocationTarget::Shm(descriptor) => ReleasedWriteAllocation {
                allocation_id: descriptor.allocation_id,
                release_token: descriptor.release_token.clone(),
            },
            HostAllocationTarget::Grpc => panic!("expected SHM staging"),
        };
        let receipt = state.upload(allocation.transfer_id, b"x".to_vec()).unwrap();
        state
            .arena
            .commit_staging(session, allocation.staging_id, &receipt, b"block".to_vec())
            .unwrap();

        assert!(
            !state
                .try_finalize_block_retirement(vec![b"block".to_vec()])
                .unwrap()
        );
        state
            .heartbeat_inner(session, None, vec![release], None, false)
            .unwrap();
        assert!(
            state
                .try_finalize_block_retirement(vec![b"block".to_vec()])
                .unwrap()
        );
        let _ = std::fs::remove_file(path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_successful_shared_read_tickets_consume_one_epoch() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("s59-{}-{unique}.sock", std::process::id()));
        let broker = SharedFdBroker::bind(path.clone()).unwrap();
        let mut state = NodeState::new(
            "node-a".into(),
            None,
            4096,
            Duration::from_secs(30),
            Some(broker),
        );
        let session = state.open_session(true);
        state
            .arena
            .commit_inline(b"block".to_vec(), b"abcdef".to_vec())
            .unwrap();
        // 同次读有多个 Extent，但只是一份读借用，所有段共享同一 epoch。
        let resolved = resolved_value(
            1,
            6,
            vec![
                test_extent(0, 3, b"block", 0),
                test_extent(3, 3, b"block", 3),
            ],
        );
        for expected in [1, 2] {
            let GetOutcome::Ready(ticket) =
                state.get_resolved(session, &resolved, None, 0).unwrap()
            else {
                panic!("local block must be ready")
            };
            assert_eq!(ticket.segments.len(), 2);
            for segment in ticket.segments {
                let ReadTarget::Shm(target) = segment.target else {
                    panic!("expected SHM")
                };
                assert_eq!(target.view_epoch, Some(expected));
            }
            assert_eq!(state.sessions[&session].next_view_epoch, expected + 1);
        }
        // 无返回 bytes 或校验失败的读，不会留下 Client 无从释放的序号空洞。
        state
            .get_resolved(session, &resolved, Some((6, 0)), 0)
            .unwrap();
        assert!(
            state
                .get_resolved(session, &resolved, Some((7, 1)), 0)
                .is_err()
        );
        assert_eq!(state.sessions[&session].next_view_epoch, 3);
        drop(state);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn prepared_replica_timeout_transitions_to_aborted_without_publication() {
        let mut state = NodeState::new(
            "node-a".to_string(),
            None,
            4096,
            Duration::from_secs(30),
            None,
        );
        let plan_id = b"repair-plan".to_vec();
        let bytes = b"replica-bytes".to_vec();
        let block_id = digest(&bytes);
        state
            .prepare_replica(
                plan_id.clone(),
                block_id.clone(),
                bytes.clone(),
                digest(&bytes),
                bytes.len() as u64,
            )
            .expect("prepare");
        if let Some(ReplicaTransferState::Prepared(replica)) =
            state.prepared_replicas.get_mut(&plan_id)
        {
            replica.expires_at = Instant::now() - Duration::from_millis(1);
        } else {
            panic!("prepared state missing");
        }

        state.tick();

        let status = state.replica_status(&plan_id).expect("status");
        assert_eq!(status.status, "aborted");
        assert!(matches!(
            state.arena.open_read(&block_id, None),
            Err(ArenaError::UnknownBlock)
        ));
    }

    #[test]
    fn rejected_repair_report_discards_its_active_unpublished_block() {
        let mut state = NodeState::new(
            "node-a".to_string(),
            None,
            4096,
            Duration::from_secs(30),
            None,
        );
        let plan_id = b"late-repair-plan".to_vec();
        let bytes = b"late-replica-bytes".to_vec();
        let block_id = digest(&bytes);
        state
            .prepare_replica(
                plan_id.clone(),
                block_id.clone(),
                bytes.clone(),
                digest(&bytes),
                bytes.len() as u64,
            )
            .expect("prepare");
        state.activate_replica(plan_id.clone()).expect("activate");
        assert!(state.arena.open_read(&block_id, None).is_ok());

        state
            .discard_replica_attempt(plan_id.clone())
            .expect("discard unpublished attempt");

        assert!(matches!(
            state.arena.open_read(&block_id, None),
            Err(ArenaError::UnknownBlock)
        ));
        assert!(!state.prepared_replicas.contains_key(&plan_id));
    }

    #[test]
    fn stable_meta_error_keeps_its_code_through_node_boundary() {
        let meta_error = DmsError::new(
            dms_error::META_CATALOG_VERSION_CONFLICT,
            ErrorKind::Aborted,
            "meta rejected stale object version",
        );
        let worker_error = map_metadata_error(meta_error.clone());
        let public_error = worker_error_to_dms(worker_error);

        assert_eq!(public_error.code(), meta_error.code());
        assert_eq!(public_error.kind(), meta_error.kind());
        assert_eq!(public_error.message(), meta_error.message());
    }
}

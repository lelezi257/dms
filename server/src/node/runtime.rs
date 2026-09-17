//! dms-node 内唯一的异步业务状态所有者。
//!
//! Client→Node 与 Node→Node gRPC Handler 都通过 [`NodeHandle`] 投递命令。
//! Handle 只持有 bounded `mpsc::Sender`，真正的 HashMap 只属于 `run_node`。
//! 每条 command 携带一个 `oneshot` 回信端，把结果送回原 RPC Future；等待结果时
//! 只挂起当前协程，不阻塞 Tokio 工作线程。

// Node actor 内的小型控制索引使用 HashMap；Payload bytes 只归 ArenaManager。
#[cfg(feature = "reliability-faults")]
use std::io::Write;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    hash::{Hash, Hasher},
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
    HostShmDescriptor, ReleasedWriteAllocation, ReservationConsumption, SharedFdBroker,
};
use super::current_cache::CurrentCache;
use super::filesystem::{
    binding_cache::BindingCache,
    dentry_cache::{DentryCache, DentryLookup},
    meta_client::ResolvedDentry,
    open_handles::{OpenHandle, OpenHandleTable},
};
use super::metadata_client::{BatchValueCommit, LocalReplicaIdentity, MetadataClient, digest};
use super::metrics::{
    CurrentCacheResetReason, FilesystemInodeReferenceTransition, NodeMailboxCommand, NodeMetrics,
    PeerImportMetricsSnapshot, ReplicaDirection, ReplicaOperation, SessionExpiration,
};
use super::replica_reporter::{self, ReplicaReportJob};
use crate::config::{ConfigChange, ConfigError, OnlineConfigController};
use crate::filesystem::{
    DirectoryPage, DirectoryVersion, FileHandleId, InodeId, InodeVersion, PreparedObjectVersion,
    ROOT_INODE, ResolvedInode, ResolvedObject,
};

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
// Node 是用户输入的权威边界。SDK 可以做友好预检，但所有 key/field、批量和
// gRPC payload 上限必须在 Node 再检查一次，防止其它语言 SDK 或手写 client
// 绕过限制后把内存 owner 推入不可控分配。
pub(crate) const USER_KEY_BYTES_MAX: usize = 1024;
pub(crate) const USER_FIELD_BYTES_MAX: usize = 1024;
pub(crate) const BATCH_MAX_ITEMS: usize = 1024;
pub(crate) const BATCH_MAX_PAYLOAD_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const HASH_MAX_FIELDS_PER_OPERATION: usize = 1024;
pub(crate) const HASH_MAX_ENCODED_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const HSCAN_MAX_LIMIT: u32 = 1024;
// 该值低于默认 Tonic message 上限，使 Rust SDK 和 Node handler 都能在真正
// gRPC 编解码失败前返回带 DMS 数字错误码的 ResourceExhausted。SHM 走 mmap，
// 不受这条单条 protobuf 消息预算约束。
pub(crate) const GRPC_PAYLOAD_SAFE_BYTES: u64 = 8 * 1024 * 1024;
// Peer gRPC 默认有 4MiB 解码上限。单 Block 回退与 PullBlocks 正常流都使用
// 2MiB 有界分段，避免单条消息随对象大小增长。
const PEER_PULL_SEGMENT_BYTES: u64 = 2 * 1024 * 1024;
// Filesystem 的第一次顺序读通常会继续消费同一 Exact Version。对不超过工作负载
// 上限的文件，在 offset=0 首次缺块时一次接管完整布局，避免每个 FUSE read 回调
// 都建立一条 Peer RPC。Image/普通 KV range read 不使用该策略，仍保持真正按需。
const FILESYSTEM_PEER_PREFETCH_BYTES_MAX: u64 = 512 * 1024 * 1024;
/// 目录 lookup 顺带预取只服务小文件窗口。上限足以覆盖 Agent workspace
/// 的一批相邻文件，同时避免一次 lookup 因目录中的大对象占满 Arena 导入预算。
const FILESYSTEM_DIRECTORY_PREFETCH_BYTES_MAX: u64 = 8 * 1024 * 1024;
const FILESYSTEM_DIRECTORY_PREFETCH_BLOCKS_MAX: usize = 256;
const PEER_PULL_PLAN_BLOCKS_MAX: usize = 1024;
const PEER_CHANNEL_CACHE_LIMIT: usize = 128;
const COMPLETED_RETIREMENT_CACHE_LIMIT: usize = 1024;
const RETIREMENT_PHASE_WAITER_LIMIT: usize = 16;
type FilesystemOpenReferenceResult = (OpenHandle, Option<u64>);
type FilesystemCloseReferenceResult = Option<(OpenHandle, Option<(u64, bool)>)>;
#[cfg(feature = "reliability-faults")]
const SOURCE_SELECTION_RECEIPT_ENV: &str = "DMS_RELIABILITY_SOURCE_SELECTION_RECEIPT";
#[cfg(all(feature = "reliability-faults", test))]
static SOURCE_SELECTION_RECEIPT_PATH_FOR_TEST: std::sync::Mutex<Option<PathBuf>> =
    std::sync::Mutex::new(None);

// 准备和完成都只访问唯一 owner；中间 Future 只拥有不可变提交资料与 Meta client。
// JoinSet 的数量上限与 mailbox 相同，饱和时拒绝新写，但 ACK/心跳仍可推进。
type WriteCompletion<T> = Box<dyn FnOnce(&mut NodeState) -> Result<T, WorkerError> + Send>;
type PreparedWrite<T> = Pin<Box<dyn Future<Output = WriteCompletion<T>> + Send>>;
type ApplyWrite = Box<dyn FnOnce(&mut NodeState) + Send>;

// session_id 的 0 值保留给协议哨兵；起点限制在低半区，保证一次 Node 进程内
// 有足够大的单调递增空间，不会因为启动种子靠近 u64::MAX 而很快溢出。
const SESSION_ID_START_SPACE: u64 = 1u64 << 63;
static NODE_SESSION_INCARNATION_NONCE: AtomicU64 = AtomicU64::new(1);
// reference generation 只负责为一次 Node 进程中的引用周期生成 fencing token；它不
// 读取 inode 表，也不决定引用是否存活。使用原子序列可让出站 Meta 请求直接取得 token，
// 真正的引用安装、计数和释放仍然只能进入 NodeState owner。
static FILESYSTEM_REFERENCE_GENERATION: AtomicU64 = AtomicU64::new(1);

fn node_session_start(node_id: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node_id.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    NODE_SESSION_INCARNATION_NONCE
        .fetch_add(1, Ordering::Relaxed)
        .hash(&mut hasher);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    normalize_session_start(hasher.finish())
}

fn normalize_session_start(raw: u64) -> u64 {
    // 映射到 1..=2^63，避免 0，同时远离 u64::MAX 溢出边界。
    raw % SESSION_ID_START_SPACE + 1
}

fn next_filesystem_reference_generation() -> u64 {
    loop {
        let generation = FILESYSTEM_REFERENCE_GENERATION.fetch_add(1, Ordering::Relaxed);
        if generation != 0 {
            return generation;
        }
    }
}

#[cfg(feature = "reliability-faults")]
fn record_source_selection_receipt(
    block_id: &[u8],
    source: &PeerPullSource,
) -> Result<(), WorkerError> {
    let Some(path) = source_selection_receipt_path() else {
        return Ok(());
    };

    // 这是 reliability-faults 下的私有观测证据：只记录“实际选中的远端来源”，
    // 让外部故障控制器可以严格核对 dead epoch 没有被选择。默认 feature 关闭时
    // 这段代码不会编译进产物，也不会给正常热路径增加分支或 I/O。
    let line = format!(
        "block_id={} node_id={} node_epoch={}\n",
        bytes_to_lower_hex(block_id),
        source.node_id,
        source.node_epoch
    );
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|_| WorkerError::WorkerUnavailable)
}

#[cfg(feature = "reliability-faults")]
fn source_selection_receipt_path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = SOURCE_SELECTION_RECEIPT_PATH_FOR_TEST
        .lock()
        .expect("source selection receipt path lock")
        .clone()
    {
        return Some(path);
    }

    let path = std::env::var_os(SOURCE_SELECTION_RECEIPT_ENV)?;
    if path.as_os_str().is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

#[cfg(feature = "reliability-faults")]
fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub(crate) fn validate_user_key(key: &[u8]) -> Result<(), WorkerError> {
    if key.is_empty() {
        return Err(WorkerError::InvalidArgument("key must not be empty"));
    }
    if key.len() > USER_KEY_BYTES_MAX {
        return Err(WorkerError::InvalidArgument(
            "key length exceeds 1024 bytes",
        ));
    }
    Ok(())
}

pub(crate) fn validate_user_field(field: &[u8]) -> Result<(), WorkerError> {
    if field.is_empty() {
        return Err(WorkerError::InvalidArgument("field must not be empty"));
    }
    if field.len() > USER_FIELD_BYTES_MAX {
        return Err(WorkerError::InvalidArgument(
            "field length exceeds 1024 bytes",
        ));
    }
    Ok(())
}

fn validate_operation_id(operation_id: &[u8]) -> Result<(), WorkerError> {
    if operation_id.len() != 24 {
        return Err(WorkerError::InvalidArgument(
            "operation identity is required",
        ));
    }
    Ok(())
}

pub(crate) fn validate_grpc_payload_bytes(length: u64) -> Result<(), WorkerError> {
    if length > GRPC_PAYLOAD_SAFE_BYTES {
        return Err(WorkerError::ResourceExhausted);
    }
    Ok(())
}

fn validate_batch_write(entries: &[(Vec<u8>, u64, HostReceipt)]) -> Result<(), WorkerError> {
    if entries.is_empty() {
        return Err(WorkerError::InvalidArgument("MSet entries are empty"));
    }
    if entries.len() > BATCH_MAX_ITEMS {
        return Err(WorkerError::ResourceExhausted);
    }
    let mut total = 0_u64;
    for (key, _, receipt) in entries {
        validate_user_key(key)?;
        total = total
            .checked_add(receipt.length)
            .ok_or(WorkerError::ResourceExhausted)?;
        if total > BATCH_MAX_PAYLOAD_BYTES {
            return Err(WorkerError::ResourceExhausted);
        }
    }
    Ok(())
}

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
    Zero { length: u64 },
}

#[derive(Clone, Debug)]
enum PlannedReadPart {
    Block {
        logical_offset: u64,
        read: ArenaReadTicket,
    },
    Zero {
        logical_offset: u64,
        length: u64,
    },
}

#[derive(Clone, Debug)]
enum DataCorePlannedReadPart {
    Block {
        logical_offset: u64,
        read: ArenaReadTicket,
    },
    Remote {
        logical_offset: u64,
        block_id: Vec<u8>,
        block_offset: u64,
        length: u64,
    },
    Zero {
        logical_offset: u64,
        length: u64,
    },
}

#[derive(Clone, Debug)]
struct RemoteReadPart {
    logical_offset: u64,
    block_id: Vec<u8>,
    block_offset: u64,
    length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DataCoreReadIntoResult {
    pub(crate) version: u64,
    pub(crate) logical_length: u64,
    pub(crate) bytes: Vec<u8>,
    pub(crate) bytes_read: usize,
}

/// DataCore 一次读尝试的结果。
///
/// `BufferTooSmall` 不是容量耗尽：它表示调用方在解析目标版本前只能按旧长度
/// 预分配。返回本次已经解析出的固定版本和所需长度后，DataCore 可以只重试该
/// 版本，避免 Current 在两次请求间继续变化导致读到混合快照。
pub(crate) enum DataCoreReadAttempt {
    Ready(DataCoreReadIntoResult),
    NotFound,
    BufferTooSmall { version: u64, required: usize },
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
    pub(crate) payload: prost::bytes::Bytes,
    pub(crate) checksum: Vec<u8>,
    pub(crate) length: u64,
}

#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "reliability-faults"), allow(dead_code))]
struct PeerPullSource {
    node_id: u64,
    node_epoch: u64,
    endpoint: String,
}

#[derive(Clone, Debug)]
struct PeerPullSpec {
    // Meta 已按当前租约过滤出可用位置，但“租约仍有效”不等于进程此刻一定
    // 可连接。保留同一 Block 的全部候选，让一次读能在请求内切换来源；这里
    // 不改变集群成员状态，也不会因为一次连接失败就宣告某个 Node 死亡。
    sources: Vec<PeerPullSource>,
    block_id: Vec<u8>,
    expected_checksum: Vec<u8>,
    expected_length: u64,
}

impl PeerPullSpec {
    fn single_source(
        endpoint: String,
        block_id: Vec<u8>,
        expected_checksum: Vec<u8>,
        expected_length: u64,
    ) -> Self {
        Self {
            // Repair 计划当前只携带 endpoint；0 表示该内部调用没有附带可用于
            // 诊断的 Meta Node 身份，不参与协议或来源选择。
            sources: vec![PeerPullSource {
                node_id: 0,
                node_epoch: 0,
                endpoint,
            }],
            block_id,
            expected_checksum,
            expected_length,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PeerSegmentRequest {
    offset: Option<u64>,
    length: Option<u64>,
    validate_whole_block: bool,
}

impl PeerSegmentRequest {
    const WHOLE_BLOCK: Self = Self {
        offset: None,
        length: None,
        validate_whole_block: true,
    };

    fn segment(offset: u64, length: u64) -> Self {
        Self {
            offset: Some(offset),
            length: Some(length),
            validate_whole_block: false,
        }
    }
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
    waiters: Vec<PeerImportReply>,
}

/// 已产生缺块计划但尚未入队 Ensure 的同批读，也必须看见 Import/Report 的终态失败。
/// 仅保留到失败发生前的 read scopes 排空，不是跨请求的永久负缓存。
struct PeerImportFailureFence {
    cutoff_scope: u64,
    failure: PeerImportFailure,
}

type PeerImportCompletion = (Vec<u8>, u64, Result<(), PeerImportFailure>);
/// 同一 Block 的等待方只接收已校验的 immutable bytes；`None` 表示 owner
/// 在请求入队前已经完成安装，调用方下一轮直接从 Arena 读取。
type PeerImportReply = oneshot::Sender<Result<Option<prost::bytes::Bytes>, PeerImportFailure>>;
type PeerImportRequest = (PeerPullSpec, PeerImportReply);
/// PullBlocks 流内一个 Block 的安装终态，以及它是否产生后台副本登记。
type PeerInstallOutcome = (Vec<u8>, u64, Result<(), PeerImportFailure>, bool);

#[derive(Clone, Debug)]
struct PeerImportWork {
    spec: PeerPullSpec,
    attempt: u64,
}

/// PullBlocks 接收协程已经把一个完整 Block 交给 Node owner，但 owner 还未回信。
/// 队列上限固定为 2，只允许“接收下一块”与“安装上一块”重叠，不聚合整文件。
struct PendingPeerInstall {
    block_id: Vec<u8>,
    attempt: Option<u64>,
    receiver: oneshot::Receiver<Result<bool, WorkerError>>,
    metric: super::metrics::ReplicaOperationGuard,
    received_bytes: usize,
    report_length: u64,
    report_checksum: Vec<u8>,
    report_operation: Vec<u8>,
}

/// 一次 Peer 导入在本地安装阶段所需的上下文。
///
/// 这些字段始终成组从导入任务传到 Arena 接纳与 Meta 登记阶段。把它们放在
/// 一个内部结构中，可以让函数签名表达“导入上下文”而不是暴露一串易错参数；
/// 该结构不跨进程，也不是新的公开抽象。
struct PeerImportContext<'a> {
    operation_namespace: &'a [u8],
    read_scope_id: u64,
    import_attempt: Option<u64>,
    replica_report_tx: Option<mpsc::Sender<ReplicaReportJob>>,
}

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

fn planned_read_part_offset(part: &PlannedReadPart) -> u64 {
    match part {
        PlannedReadPart::Zero { logical_offset, .. }
        | PlannedReadPart::Block { logical_offset, .. } => *logical_offset,
    }
}

fn data_core_planned_read_part_offset(part: &DataCorePlannedReadPart) -> u64 {
    match part {
        DataCorePlannedReadPart::Zero { logical_offset, .. }
        | DataCorePlannedReadPart::Block { logical_offset, .. }
        | DataCorePlannedReadPart::Remote { logical_offset, .. } => *logical_offset,
    }
}

enum DataCoreMaterializeOutcome {
    Ready(DataCoreReadIntoResult),
    BufferTooSmall {
        version: u64,
        required: usize,
    },
    NeedsRemoteBlocks {
        specs: Vec<PeerPullSpec>,
        // Filesystem 顺序首读可以把整个 Exact Version 加入同一条预取流，
        // 但本次 FUSE 回调只等待覆盖当前 range 的 Block。其余 Block 在同一
        // PullBlocks 流中继续后台接管，避免“先下载完整文件、再返回第一页”。
        required_blocks: HashSet<Vec<u8>>,
        // 首次物化已经把本地 Block 与 hole 写进 output。远端 Block 安装成功后，
        // 当前读可直接从同一份已校验 bytes 补齐这些区间，无需再次进入 owner
        // 从 Arena 读取；Arena 仍是后续请求的唯一持久内存 owner。
        remote_parts: Vec<RemoteReadPart>,
        version: u64,
        logical_length: u64,
        bytes_read: usize,
        output: Vec<u8>,
    },
    NotFound,
}

enum DataCoreCachedReadOutcome {
    Miss {
        output: Vec<u8>,
    },
    Ready(DataCoreReadIntoResult),
    BufferTooSmall {
        version: u64,
        required: usize,
    },
    NeedsRemoteBlocks {
        resolved: pb::ResolveObjectResponse,
        specs: Vec<PeerPullSpec>,
        output: Vec<u8>,
    },
    NotFound,
}

#[derive(Clone, Debug)]
pub(crate) enum WorkerError {
    /// 请求内容不合法；静态字符串避免临时分配。
    InvalidArgument(&'static str),
    UnknownSession,
    UnknownStaging,
    UnknownTransfer,
    NotFound,
    NoLiveReplica,
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

impl WorkerError {
    /// 是否为对象版本条件冲突。Meta 在提交边界返回稳定 DMS 错误码，Node 内部
    /// 也可能在更早阶段直接发现冲突；上层重试策略不应依赖错误来自哪一层。
    #[cfg(test)]
    pub(crate) fn is_version_conflict(&self) -> bool {
        matches!(self, Self::Conflict)
            || matches!(
                self,
                Self::Stable(error)
                    if error.code() == dms_error::META_CATALOG_VERSION_CONFLICT
            )
    }
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

    /// 返回复用同一 Meta HTTP/2 Channel 与 Node session 的文件元数据客户端。
    /// 这里只 clone 轻量句柄，不建立第二条连接。
    pub(crate) fn filesystem_metadata_client(
        &self,
    ) -> Result<super::filesystem::meta_client::FilesystemMetaGrpcClient, WorkerError> {
        self.metadata
            .clone()
            .map(super::filesystem::meta_client::FilesystemMetaGrpcClient::new)
            .ok_or(WorkerError::MetadataUnavailable)
    }

    pub(crate) async fn filesystem_cached_binding(
        &self,
        inode: InodeId,
    ) -> Result<Option<ResolvedInode>, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemGetBinding { inode, reply })
            .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_cache_binding(
        &self,
        resolved: ResolvedInode,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemCacheBinding { resolved, reply })
            .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_cached_dentry(
        &self,
        parent: InodeId,
        name: Vec<u8>,
    ) -> Result<DentryLookup, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemGetDentry {
            parent,
            name,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_cache_negative_dentry(
        &self,
        parent: InodeId,
        name: Vec<u8>,
        grant: crate::filesystem::DirectoryGrant,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemCacheNegativeDentry {
            parent,
            name,
            grant,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_apply_local_namespace_mutation(
        &self,
        changed_directories: Vec<DirectoryVersion>,
        changed_inodes: Vec<InodeVersion>,
        removed_dentries: Vec<(InodeId, Vec<u8>)>,
        refreshed_directories: Vec<ResolvedInode>,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemApplyLocalNamespaceMutation {
            changed_directories,
            changed_inodes,
            removed_dentries,
            refreshed_directories,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    /// 把一次 Meta 返回的完整目录项在同一个 Node owner turn 中落入本地状态。
    ///
    /// dentry、inode binding 与 entry reference 来自同一份权威响应，拆成多条
    /// mailbox 命令既没有一致性收益，还会放大 create/lookup 的本地排队开销。
    /// `apply_directory_mutation` 只在本 Node 发起 create/symlink 时为 true；普通
    /// lookup 不得把远端响应误当成本地 namespace mutation。
    pub(crate) async fn filesystem_install_resolved_dentry(
        &self,
        resolved: ResolvedDentry,
        apply_directory_mutation: bool,
    ) -> Result<(), WorkerError> {
        let inode = resolved.resolved.granted.inode.attributes.inode;
        if inode != ROOT_INODE
            && (resolved.entry_reference_generation == 0
                || resolved.entry_reference_lease_millis == 0)
        {
            return Err(WorkerError::MetadataUnavailable);
        }
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemInstallResolvedDentry {
            resolved,
            apply_directory_mutation,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_cached_directory_page(
        &self,
        directory: InodeId,
        cursor: Option<Vec<u8>>,
        expected_revision: Option<u64>,
    ) -> Result<Option<DirectoryPage>, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemGetDirectoryPage {
            directory,
            cursor,
            expected_revision,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_cache_directory_page(
        &self,
        cursor: Option<Vec<u8>>,
        page: DirectoryPage,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemCacheDirectoryPage {
            cursor,
            page,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    /// 在同一个 Node actor turn 内建立 open handle 并增加 inode 引用。
    ///
    /// handle 与引用属于同一份 NodeState；把两个动作拆成两条 mailbox 命令既会制造
    /// 不必要的排队，也会留下中间态。返回的 generation 仅在本地引用从 0→1 时存在，
    /// 调用方随后用它向 Meta 建立租约。
    pub(crate) async fn filesystem_open_handle_with_reference(
        &self,
        inode: InodeId,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> Result<FilesystemOpenReferenceResult, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemOpenHandleWithReference {
            inode,
            flags,
            lock_owner,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_get_handle(
        &self,
        handle: FileHandleId,
    ) -> Result<Option<super::filesystem::open_handles::OpenHandle>, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemGetHandle { handle, reply })
            .await?;
        receive(receiver).await
    }

    /// 在同一个 Node actor turn 内关闭 handle 并归还它持有的一份 inode 引用。
    pub(crate) async fn filesystem_close_handle_with_reference(
        &self,
        handle: FileHandleId,
    ) -> Result<FilesystemCloseReferenceResult, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemCloseHandleWithReference { handle, reply })
            .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_acquire_inode_reference_local(
        &self,
        inode: InodeId,
    ) -> Result<Option<u64>, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemAcquireInodeReference { inode, reply })
            .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_release_inode_reference_local(
        &self,
        inode: InodeId,
        count: u64,
    ) -> Result<Option<(u64, bool)>, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemReleaseInodeReference {
            inode,
            count,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    /// 标记本 Node 已知该 inode 失去最后一个目录项。
    ///
    /// 普通 close 只需停止心跳续租，不能为每个文件再向 Meta 发送 release RPC；
    /// orphan 则需要尽快通知 Meta，避免只能等待租约自然到期才回收。
    pub(crate) async fn filesystem_mark_inode_orphan(
        &self,
        inode: InodeId,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemMarkInodeOrphan { inode, reply })
            .await?;
        receive(receiver).await
    }

    pub(crate) fn filesystem_reserve_inode_reference_generation(&self) -> u64 {
        next_filesystem_reference_generation()
    }

    pub(crate) async fn filesystem_install_inode_reference(
        &self,
        inode: InodeId,
        generation: u64,
        lease_millis: u64,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemInstallInodeReference {
            inode,
            generation,
            lease_millis,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_inode_reference_snapshot(
        &self,
    ) -> Result<Vec<(InodeId, u64)>, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemInodeReferenceSnapshot { reply })
            .await?;
        receive(receiver).await
    }

    /// 从 Node 唯一 Arena owner 读取资源快照，供周期心跳上报给 Meta。
    pub(crate) async fn resource_summary(&self) -> Result<pb::ResourceSummary, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::ResourceSummary { reply }).await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_renew_inode_reference_leases(
        &self,
        references: Vec<(InodeId, u64)>,
        lease_millis: u64,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemRenewInodeReferenceLeases {
            references,
            lease_millis,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn filesystem_has_live_inode_reference(
        &self,
        inode: InodeId,
    ) -> Result<bool, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::FilesystemHasLiveInodeReference { inode, reply })
            .await?;
        receive(receiver).await
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

    async fn begin_data_core_read_scope(&self) -> Result<ReadScopeLease, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::BeginDataCoreReadScope {
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
        validate_user_key(&key)?;
        let metadata = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?;
        self.validate_session(session_id).await?;
        metadata.stat(key).await.map_err(map_metadata_error)
    }

    pub(crate) async fn data_core_stat(
        &self,
        key: Vec<u8>,
    ) -> Result<pb::MetaStatResponse, WorkerError> {
        validate_user_key(&key)?;
        let metadata = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?;
        metadata.stat(key).await.map_err(map_metadata_error)
    }

    pub(crate) async fn data_core_set_inline(
        &self,
        key: Vec<u8>,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        condition: String,
    ) -> Result<SetOutcome, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::DataCoreSetInline {
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

    /// 在已解析的精确版本上准备文件 Extent overlay；允许覆盖或从 EOF 扩容，且不会
    /// 先发布第二个 Object Current。普通 KV `SET_RANGE` 仍保持不改变 value 长度。
    pub(crate) async fn data_core_prepare_range(
        &self,
        key: Vec<u8>,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        resolved: ResolvedObject,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCorePrepareRange {
            key,
            offset,
            bytes,
            operation_id,
            resolved: resolved.into_proto(),
            reply,
        })
        .await?;
        receive(receiver).await
    }

    /// 为文件稀疏写准备候选版本。
    ///
    /// 这是文件语义专用入口，不改变普通 KV `SET_RANGE` 合同。`bytes` 只保存用户实际
    /// 写入的数据；`offset` 前后的空洞由 VersionLayout sparse hole 表达，不申请全零
    /// Block，也不会向 Meta 上报零副本。
    pub(crate) async fn data_core_prepare_sparse(
        &self,
        key: Vec<u8>,
        logical_length: u64,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        expected_version: Option<u64>,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCorePrepareSparse {
            key,
            logical_length,
            offset,
            bytes,
            operation_id,
            expected_version,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    /// 为文件 truncate 准备一个只改变 Extent 布局的候选版本。
    ///
    /// 缩短文件不产生新 Block，不应退化成“读完整文件再重新写入”；扩展文件只追加
    /// HOLE Extent，读路径本地填零，不物化全零 Block。
    pub(crate) async fn data_core_prepare_truncate(
        &self,
        key: Vec<u8>,
        new_length: u64,
        operation_id: Vec<u8>,
        resolved: ResolvedObject,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCorePrepareTruncate {
            key,
            new_length,
            operation_id,
            resolved: resolved.into_proto(),
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn data_core_prepare_punch_hole(
        &self,
        key: Vec<u8>,
        offset: u64,
        length: u64,
        operation_id: Vec<u8>,
        resolved: ResolvedObject,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCorePreparePunchHole {
            key,
            offset,
            length,
            operation_id,
            resolved: resolved.into_proto(),
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn data_core_reserve_file_space(
        &self,
        reservation_id: Vec<u8>,
        length: u64,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCoreReserveFileSpace {
            reservation_id,
            length,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn data_core_consume_file_space(
        &self,
        ranges: Vec<(Vec<u8>, u64)>,
    ) -> Result<Vec<ReservationConsumption>, WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCoreConsumeFileSpace { ranges, reply })
            .await?;
        receive(receiver).await
    }

    pub(crate) async fn data_core_restore_file_space(
        &self,
        consumptions: Vec<ReservationConsumption>,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCoreRestoreFileSpace {
            consumptions,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    pub(crate) async fn data_core_release_file_space(
        &self,
        ranges: Vec<(Vec<u8>, u64)>,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCoreReleaseFileSpace { ranges, reply })
            .await?;
        receive(receiver).await
    }

    /// 完成两阶段文件写。`version` 只在 Meta 已原子发布时存在；确定性 CAS/参数拒绝
    /// 才允许回收新块，未知提交结果必须保留，以便同 operation 重试。
    pub(crate) async fn data_core_finish_prepared(
        &self,
        prepared: PreparedObjectVersion,
        version: Option<u64>,
        rejected: bool,
    ) -> Result<(), WorkerError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::DataCoreFinishPrepared {
            prepared,
            version,
            rejected,
            reply,
        })
        .await?;
        receive(receiver).await
    }

    #[cfg(test)]
    pub(crate) async fn data_core_set_range_inline(
        &self,
        key: Vec<u8>,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        expected_version: Option<u64>,
    ) -> Result<SetOutcome, WorkerError> {
        validate_user_key(&key)?;
        let resolved = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?
            .resolve(key.clone(), expected_version)
            .await
            .map_err(map_metadata_error)?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::DataCoreSetRangeInline {
            key,
            offset,
            bytes,
            operation_id,
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

    #[cfg(test)]
    pub(crate) async fn data_core_delete(
        &self,
        key: Vec<u8>,
        operation_id: Vec<u8>,
    ) -> Result<DeleteOutcome, WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::DataCoreDelete {
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

    /// Meta Watch 在 ACK 前撤销文件绑定授权。重复事件保持幂等：条目已经不存在时也成功。
    pub(crate) async fn invalidate_filesystem_binding(
        &self,
        inode: u64,
        through_generation: u64,
        minimum_inode_revision: u64,
    ) -> Result<(), WorkerError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::InvalidateFilesystemBinding {
            inode,
            through_generation,
            minimum_inode_revision,
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

    /// 旧的直接读取测试入口；生产 Worker 使用带 request id 的完整入口。
    #[cfg(test)]
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

    /// 测试可在不构造 protobuf Handler 的情况下指定 inline 上限。
    #[cfg(test)]
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
        validate_user_key(&key)?;
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
                    match self.ensure_peer_blocks(read_scope_id, specs).await {
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
                    self.ensure_peer_blocks(read_scope_id, specs)
                        .await
                        .map_err(|failure| failure.error)?;
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
        validate_user_key(&key)?;
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

    pub(crate) async fn data_core_read_into(
        &self,
        key: Vec<u8>,
        exact_version: Option<u64>,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        output: Vec<u8>,
    ) -> Result<DataCoreReadAttempt, WorkerError> {
        validate_user_key(&key)?;
        let scope = self.begin_data_core_read_scope().await?.into_guard();
        let result = self
            .data_core_read_into_scoped(scope.id(), key, exact_version, range, clamp_range, output)
            .await;
        let finish = scope.finish().await;
        match (result, finish) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    /// 使用 Filesystem binding 中已经取得的精确读取计划，跳过对象 ResolveObject。
    pub(crate) async fn data_core_read_pre_resolved_into(
        &self,
        resolved: ResolvedObject,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        output: Vec<u8>,
    ) -> Result<DataCoreReadAttempt, WorkerError> {
        let scope = self.begin_data_core_read_scope().await?.into_guard();
        let result = self
            .data_core_read_resolved_into(
                scope.id(),
                resolved.into_proto(),
                range,
                clamp_range,
                output,
                None,
                true,
            )
            .await;
        let finish = scope.finish().await;
        match (result, finish) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    /// 为同一目录解析窗口中的多个 Exact Version 合并一次 Peer 预取。
    ///
    /// 解析计划必须回到唯一 NodeState owner，因为只有它能安全判断 Arena
    /// 已命中、正在 singleflight 导入、退役 fence 和字节预算。
    pub(crate) async fn data_core_prefetch_resolved(
        &self,
        objects: Vec<ResolvedObject>,
    ) -> Result<(), WorkerError> {
        let scope = self.begin_data_core_read_scope().await?.into_guard();
        let (reply, receiver) = oneshot::channel();
        self.submit(NodeCommand::PlanFilesystemPeerPrefetch {
            read_scope_id: scope.id(),
            resolved: objects
                .into_iter()
                .map(ResolvedObject::into_proto)
                .collect(),
            reply,
        })
        .await?;
        let specs = receive(receiver).await?;
        let result = self
            .ensure_peer_blocks(scope.id(), specs)
            .await
            .map_err(|failure| failure.error);
        let finish = scope.finish().await;
        match (result, finish) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
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
                    self.ensure_peer_blocks(read_scope_id, specs)
                        .await
                        .map_err(|failure| failure.error)?;
                }
            }
        }
        Err(WorkerError::NotFound)
    }

    async fn data_core_read_into_scoped(
        &self,
        read_scope_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        mut output: Vec<u8>,
    ) -> Result<DataCoreReadAttempt, WorkerError> {
        let metadata = self
            .metadata
            .as_ref()
            .ok_or(WorkerError::MetadataUnavailable)?;
        let node_epoch = metadata.node_epoch().await;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::DataCoreGetCached {
            read_scope_id,
            key: key.clone(),
            node_epoch,
            exact_version,
            range,
            clamp_range,
            output,
            reply: reply_tx,
        })
        .await?;
        let (refill_token, cached) = receive(reply_rx).await?;
        match cached {
            DataCoreCachedReadOutcome::Ready(result) => {
                return Ok(DataCoreReadAttempt::Ready(result));
            }
            DataCoreCachedReadOutcome::BufferTooSmall { version, required } => {
                return Ok(DataCoreReadAttempt::BufferTooSmall { version, required });
            }
            DataCoreCachedReadOutcome::NeedsRemoteBlocks {
                resolved,
                specs,
                output: returned,
            } => {
                output = returned;
                self.ensure_peer_blocks(read_scope_id, specs)
                    .await
                    .map_err(|failure| failure.error)?;
                return self
                    .data_core_read_resolved_into(
                        read_scope_id,
                        resolved,
                        range,
                        clamp_range,
                        output,
                        None,
                        false,
                    )
                    .await;
            }
            DataCoreCachedReadOutcome::NotFound => return Ok(DataCoreReadAttempt::NotFound),
            DataCoreCachedReadOutcome::Miss { output: returned } => {
                output = returned;
            }
        }
        let requested_at = Instant::now();
        let resolved = metadata
            .resolve(key.clone(), exact_version)
            .await
            .map_err(map_metadata_error)?;
        let cache_refill = exact_version
            .is_none()
            .then(|| refill_token.map(|token| (token, key, requested_at, node_epoch)))
            .flatten();
        self.data_core_read_resolved_into(
            read_scope_id,
            resolved,
            range,
            clamp_range,
            output,
            cache_refill,
            false,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "resolved-read loop deliberately carries independent read-scope, exact layout, range, caller buffer, cache-refill authority, and full-layout prefetch policy"
    )]
    async fn data_core_read_resolved_into(
        &self,
        read_scope_id: u64,
        resolved: pb::ResolveObjectResponse,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        mut output: Vec<u8>,
        cache_refill: Option<(u64, Vec<u8>, Instant, u64)>,
        prefetch_full_layout: bool,
    ) -> Result<DataCoreReadAttempt, WorkerError> {
        for _ in 0..2 {
            let (reply_tx, reply_rx) = oneshot::channel();
            self.submit(NodeCommand::DataCoreMaterializeInto {
                read_scope_id,
                resolved: resolved.clone(),
                range,
                clamp_range,
                output,
                cache_refill: cache_refill.clone(),
                prefetch_full_layout,
                reply: reply_tx,
            })
            .await?;
            match receive(reply_rx).await? {
                DataCoreMaterializeOutcome::Ready(result) => {
                    return Ok(DataCoreReadAttempt::Ready(result));
                }
                DataCoreMaterializeOutcome::BufferTooSmall { version, required } => {
                    return Ok(DataCoreReadAttempt::BufferTooSmall { version, required });
                }
                DataCoreMaterializeOutcome::NeedsRemoteBlocks {
                    specs,
                    required_blocks,
                    remote_parts,
                    version,
                    logical_length,
                    bytes_read,
                    output: returned,
                } => {
                    output = returned;
                    let imported = self
                        .ensure_peer_blocks_waiting_for(read_scope_id, specs, &required_blocks)
                        .await
                        .map_err(|failure| failure.error)?;
                    // Filesystem 的 Exact Version 已经固定，且 owner 在唤醒 waiter 前
                    // 已完成 checksum、fence 与 Arena 安装。当前回调直接用同一份
                    // immutable Bytes 补齐远端区间，避免为了读取刚安装的数据再次
                    // 排队进入 actor。若 owner 发现块在入队前已存在，会返回 None，
                    // 继续下一轮从 Arena 读取即可。
                    if prefetch_full_layout
                        && Self::fill_imported_parts(&mut output, &remote_parts, &imported)?
                    {
                        return Ok(DataCoreReadAttempt::Ready(DataCoreReadIntoResult {
                            version,
                            logical_length,
                            bytes_read,
                            bytes: output,
                        }));
                    }
                }
                DataCoreMaterializeOutcome::NotFound => {
                    return Ok(DataCoreReadAttempt::NotFound);
                }
            }
        }
        Err(WorkerError::NotFound)
    }

    fn fill_imported_parts(
        output: &mut [u8],
        parts: &[RemoteReadPart],
        imported: &HashMap<Vec<u8>, prost::bytes::Bytes>,
    ) -> Result<bool, WorkerError> {
        for part in parts {
            let Some(bytes) = imported.get(&part.block_id) else {
                return Ok(false);
            };
            let source_start =
                usize::try_from(part.block_offset).map_err(|_| WorkerError::ResourceExhausted)?;
            let length =
                usize::try_from(part.length).map_err(|_| WorkerError::ResourceExhausted)?;
            let source_end = source_start
                .checked_add(length)
                .ok_or(WorkerError::ResourceExhausted)?;
            let target_start =
                usize::try_from(part.logical_offset).map_err(|_| WorkerError::ResourceExhausted)?;
            let target_end = target_start
                .checked_add(length)
                .ok_or(WorkerError::ResourceExhausted)?;
            let source = bytes
                .get(source_start..source_end)
                .ok_or(WorkerError::Conflict)?;
            let target = output
                .get_mut(target_start..target_end)
                .ok_or(WorkerError::Conflict)?;
            target.copy_from_slice(source);
        }
        Ok(true)
    }

    /// Current、Exact、内部 materialize 共用的 Node 级缺块导入入口。
    /// 请求取消只丢弃自己的等待，owner 启动的有界任务仍负责完成 Import/Report。
    async fn ensure_peer_blocks(
        &self,
        read_scope_id: u64,
        specs: Vec<PeerPullSpec>,
    ) -> Result<(), PeerImportFailure> {
        let required_blocks = specs
            .iter()
            .map(|spec| spec.block_id.clone())
            .collect::<HashSet<_>>();
        self.ensure_peer_blocks_waiting_for(read_scope_id, specs, &required_blocks)
            .await
            .map(|_| ())
    }

    /// 把完整预取计划一次交给 owner，但只等待当前调用真正依赖的 Block。
    ///
    /// 非 required receiver 在本函数返回时被丢弃；这只表示当前调用不再等待，
    /// 不会取消 owner 已启动的 PullBlocks 任务。后续读若追上后台预取，会加入
    /// 同一个 flight 等待，避免重复建立 Peer RPC。
    async fn ensure_peer_blocks_waiting_for(
        &self,
        read_scope_id: u64,
        specs: Vec<PeerPullSpec>,
        required_blocks: &HashSet<Vec<u8>>,
    ) -> Result<HashMap<Vec<u8>, prost::bytes::Bytes>, PeerImportFailure> {
        if specs.is_empty() {
            return Ok(HashMap::new());
        }
        let mut receivers = Vec::with_capacity(specs.len());
        let requests = specs
            .into_iter()
            .map(|spec| {
                let (reply, receiver) = oneshot::channel();
                receivers.push((spec.block_id.clone(), receiver));
                (spec, reply)
            })
            .collect();
        self.submit(NodeCommand::EnsurePeerBlocks {
            read_scope_id,
            requests,
            node: Box::new(self.clone()),
        })
        .await
        .map_err(PeerImportFailure::terminal)?;
        let mut imported = HashMap::with_capacity(required_blocks.len());
        for (block_id, receiver) in receivers {
            if !required_blocks.contains(&block_id) {
                continue;
            }
            if let Some(bytes) = receiver
                .await
                .map_err(|_| PeerImportFailure::terminal(WorkerError::WorkerUnavailable))??
            {
                imported.insert(block_id, bytes);
            }
        }
        Ok(imported)
    }

    // 完整性测试直接检验一次未合并的导入；正式读入口一律使用批量入口。
    #[cfg(test)]
    async fn import_and_report_peer_block(
        &self,
        metadata: &MetadataClient,
        operation_namespace: &[u8],
        read_scope_id: u64,
        spec: PeerPullSpec,
    ) -> Result<(), WorkerError> {
        self.pull_and_report_peer_block(
            metadata,
            spec,
            PeerImportContext {
                operation_namespace,
                read_scope_id,
                import_attempt: None,
                replica_report_tx: None,
            },
        )
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
        spec: PeerPullSpec,
        context: PeerImportContext<'_>,
    ) -> Result<(), PeerImportFailure> {
        // 接收成功以“完整数据通过owner接纳”为边界；不能在网络收到响应时
        // 提前记成功，否则延后的完整checksum拒绝会被错误统计为成功传输。
        let metric = self.metrics.begin_replica_operation(ReplicaOperation::Pull);
        let payload = pull_block_from_peers(
            &self.node_id,
            spec,
            &self.rpc_metrics,
            &self.peer_channels,
            &self.metrics,
        )
        .await
        .map_err(|error| PeerImportFailure {
            error,
            location_failure: true,
        })?;
        self.install_and_report_peer_block(metadata, payload, metric, context)
            .await
            .map_err(PeerImportFailure::terminal)
    }

    /// 将同一次 Exact Version 读取计划中的缺块合并为一条 server-streaming RPC。
    ///
    /// 流仍按 Block 边界逐个校验并交回 Node owner 安装，不会在接收端聚合整个
    /// 文件。若公共来源在中途失效，仅对尚未完成的 Block 回退到原有逐块来源
    /// 切换路径；已经安装的 Block 不重复搬运。
    async fn pull_and_report_peer_blocks(
        &self,
        metadata: &MetadataClient,
        read_scope_id: u64,
        work: Vec<PeerImportWork>,
        replica_report_tx: mpsc::Sender<ReplicaReportJob>,
    ) -> Vec<PeerImportCompletion> {
        // 单个小 Block 没有“把 RPC 数与 Block 数解耦”的收益。继续复用 unary
        // PullBlock 可以省去 server-stream 建立和首帧状态机成本；多 Block（尤其
        // Filesystem 512 MiB 预取）才进入 PullBlocks。large-peer-first 的旧
        // PullBlock 计数因此仍为 0，workspace 小文件则避免为一个 Block 建流。
        if work.len() == 1 {
            let item = work.into_iter().next().expect("one peer import item");
            let block_id = item.spec.block_id.clone();
            let result = self
                .pull_and_report_peer_block(
                    metadata,
                    item.spec,
                    PeerImportContext {
                        operation_namespace: b"cache-import/",
                        read_scope_id,
                        import_attempt: Some(item.attempt),
                        replica_report_tx: Some(replica_report_tx),
                    },
                )
                .await;
            return vec![(block_id, item.attempt, result)];
        }
        let mut completed = if let Some(source) = common_peer_source(&work) {
            self.pull_peer_plan_from_source(
                metadata,
                read_scope_id,
                &work,
                &source,
                replica_report_tx.clone(),
            )
            .await
        } else {
            Vec::new()
        };
        let finished = completed
            .iter()
            .map(|(block_id, _, _)| block_id.clone())
            .collect::<HashSet<_>>();
        for item in work {
            if finished.contains(&item.spec.block_id) {
                continue;
            }
            let block_id = item.spec.block_id.clone();
            let result = self
                .pull_and_report_peer_block(
                    metadata,
                    item.spec,
                    PeerImportContext {
                        operation_namespace: b"cache-import/",
                        read_scope_id,
                        import_attempt: Some(item.attempt),
                        replica_report_tx: Some(replica_report_tx.clone()),
                    },
                )
                .await;
            completed.push((block_id, item.attempt, result));
        }
        completed
    }

    async fn pull_peer_plan_from_source(
        &self,
        metadata: &MetadataClient,
        read_scope_id: u64,
        work: &[PeerImportWork],
        source: &PeerPullSource,
        replica_report_tx: mpsc::Sender<ReplicaReportJob>,
    ) -> Vec<PeerImportCompletion> {
        let channel = match peer_channel_for(&source.endpoint, &self.peer_channels).await {
            Ok(channel) => channel,
            Err(_) => return Vec::new(),
        };
        let mut client = peer_client(channel);
        let mut rpc = self
            .rpc_metrics
            .begin_client_call(dms_metrics::RpcCall::PEER_PULL_BLOCKS);
        let request = pb::PeerPullBlocksRequest {
            source_node_id: self.node_id.clone(),
            blocks: work
                .iter()
                .map(|item| pb::PeerPullBlockSpec {
                    block_id: item.spec.block_id.clone(),
                    expected_length: item.spec.expected_length,
                    expected_checksum: item.spec.expected_checksum.clone(),
                })
                .collect(),
        };
        let mut stream = match client.pull_blocks(request).await {
            Ok(response) => response.into_inner(),
            Err(status) => {
                if is_retryable_peer_status(&status) {
                    self.peer_channels.lock().await.remove(&source.endpoint);
                }
                return Vec::new();
            }
        };

        let mut completed = Vec::with_capacity(work.len());
        let mut index = 0_usize;
        let mut payload = Vec::new();
        let mut serving_node_id = String::new();
        let mut checksum = Vec::new();
        let mut installed = Vec::with_capacity(work.len());
        let mut pending_installs = VecDeque::with_capacity(2);
        let mut pending_reports = Vec::new();
        let mut metric = self.metrics.begin_replica_operation(ReplicaOperation::Pull);
        let mut stream_complete = false;
        while index < work.len() {
            let chunk = match stream.message().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) | Err(_) => break,
            };
            let item = &work[index];
            if chunk.block_id != item.spec.block_id
                || chunk.block_length != item.spec.expected_length
                || chunk.block_offset != payload.len() as u64
                || chunk.payload.is_empty()
                || (!item.spec.expected_checksum.is_empty()
                    && chunk.checksum != item.spec.expected_checksum)
                || (!checksum.is_empty() && chunk.checksum != checksum)
            {
                break;
            }
            if payload.is_empty() {
                let Ok(capacity) = usize::try_from(item.spec.expected_length) else {
                    break;
                };
                if payload.try_reserve_exact(capacity).is_err() {
                    break;
                }
                serving_node_id = chunk.serving_node_id.clone();
                checksum = chunk.checksum.clone();
            } else if serving_node_id != chunk.serving_node_id {
                break;
            }
            // 常见 Block 小于单条 chunk 上限。Prost 的 Bytes 可以直接借用解码帧，
            // 完整单 chunk 不再先复制进临时 Vec；跨 chunk 的大 Block 才组装 Vec。
            let complete_payload =
                if payload.is_empty() && chunk.block_offset == 0 && chunk.end_of_block {
                    Some(chunk.payload)
                } else {
                    payload.extend_from_slice(&chunk.payload);
                    chunk
                        .end_of_block
                        .then(|| prost::bytes::Bytes::from(std::mem::take(&mut payload)))
                };
            let Some(complete_payload) = complete_payload else {
                continue;
            };
            if complete_payload.len() as u64 != item.spec.expected_length {
                break;
            }
            let verified = digest(&complete_payload);
            if (!item.spec.expected_checksum.is_empty() && verified != item.spec.expected_checksum)
                || (!checksum.is_empty() && verified != checksum)
            {
                self.metrics.record_replica_checksum_failure();
                break;
            }
            let block_id = item.spec.block_id.clone();
            let result = self
                .begin_peer_install(
                    PeerBlockResult {
                        serving_node_id: std::mem::take(&mut serving_node_id),
                        block_id: block_id.clone(),
                        payload: complete_payload,
                        checksum: if checksum.is_empty() {
                            verified
                        } else {
                            std::mem::take(&mut checksum)
                        },
                        length: item.spec.expected_length,
                    },
                    metric,
                    b"cache-import/",
                    read_scope_id,
                    Some(item.attempt),
                )
                .await
                .map_err(PeerImportFailure::terminal);
            index += 1;
            match result {
                Ok(pending) => pending_installs.push_back(pending),
                Err(error) => {
                    installed.push((block_id, item.attempt, Err(error), false));
                    break;
                }
            }

            // 一个 Block 在 owner 复制/校验时，继续从 HTTP/2 流接收下一个 Block。
            // 两项上限把额外内存固定为至多一个 Block，同时消除网络与 Arena 接纳
            // 完全串行造成的吞吐损失。
            if pending_installs.len() >= 2 {
                let pending = pending_installs.pop_front().expect("two pending installs");
                let failed = self
                    .finish_peer_plan_install(
                        metadata,
                        pending,
                        source,
                        &mut pending_reports,
                        &mut installed,
                    )
                    .await;
                if failed {
                    break;
                }
            }
            if index == work.len() {
                stream_complete = true;
                break;
            }
            checksum.clear();
            metric = self.metrics.begin_replica_operation(ReplicaOperation::Pull);
        }

        while let Some(pending) = pending_installs.pop_front() {
            self.finish_peer_plan_install(
                metadata,
                pending,
                source,
                &mut pending_reports,
                &mut installed,
            )
            .await;
        }
        if stream_complete && installed.iter().all(|(_, _, result, _)| result.is_ok()) {
            rpc.success();
        }
        if !pending_reports.is_empty()
            && replica_report_tx
                .send(ReplicaReportJob::combine(pending_reports))
                .await
                .is_err()
        {
            // bytes 已通过 owner 接纳，前台 reader 也可能已经返回；后台位置登记
            // 通道关闭不能再倒转已完成读取。Node 停止时 Meta 依靠 lease 过期移除位置。
            dms_logging::warn!(
                "replica report queue closed after peer plan installation";
                "event" => "node.peer.report_queue_closed",
            );
        }
        completed.extend(
            installed
                .into_iter()
                .map(|(block_id, attempt, result, _)| (block_id, attempt, result)),
        );
        completed
    }

    /// 完成一个已入队的 owner 安装，并把副本登记事实留给计划级合并。
    async fn finish_peer_plan_install(
        &self,
        metadata: &MetadataClient,
        pending: PendingPeerInstall,
        _source: &PeerPullSource,
        pending_reports: &mut Vec<ReplicaReportJob>,
        installed: &mut Vec<PeerInstallOutcome>,
    ) -> bool {
        let block_id = pending.block_id.clone();
        let attempt = pending.attempt.expect("peer plan install has an attempt");
        let result = self
            .finish_peer_install(metadata, pending)
            .await
            .map_err(PeerImportFailure::terminal);
        #[cfg(feature = "reliability-faults")]
        if result.is_ok() {
            let _ = record_source_selection_receipt(&block_id, _source);
        }
        let failed = result.is_err();
        match result {
            Ok(report) => {
                let needs_report = report.is_some();
                pending_reports.extend(report);
                installed.push((block_id, attempt, Ok(()), needs_report));
            }
            Err(error) => installed.push((block_id, attempt, Err(error), false)),
        }
        failed
    }

    /// 缓存位置和权威位置的读共用接纳与登记逻辑；本函数的错误不能触发位置回退。
    async fn install_and_report_peer_block(
        &self,
        metadata: &MetadataClient,
        payload: PeerBlockResult,
        metric: super::metrics::ReplicaOperationGuard,
        context: PeerImportContext<'_>,
    ) -> Result<(), WorkerError> {
        // `install_peer_block` 消费上下文；先保留 Sender，确保安装完成后仍能把
        // 控制面登记任务交给后台 Reporter。Sender 的 clone 只增加引用计数。
        let replica_report_tx = context.replica_report_tx.clone();
        let Some(job) = self
            .install_peer_block(metadata, payload, metric, context)
            .await?
        else {
            return Ok(());
        };
        if let Some(replica_report_tx) = replica_report_tx {
            // 正常读只等待任务进入有界队列，不等待 Meta WAL 持久化。队列满时
            // send().await 形成明确背压，避免 Meta 故障期间无限积累任务。
            replica_report_tx
                .send(job)
                .await
                .map_err(|_| WorkerError::WorkerUnavailable)
        } else {
            // 直接完整性测试仍同步执行登记，便于精确验证错误传播。
            replica_reporter::report_now(metadata, job)
                .await
                .map_err(map_metadata_error)
        }
    }

    /// 完整校验并安装一个 Block，返回需要异步登记的控制面事实。调用方可以把同一
    /// PullBlocks 计划的多个事实合并后再入队；payload bytes 从不进入 Reporter。
    async fn install_peer_block(
        &self,
        metadata: &MetadataClient,
        payload: PeerBlockResult,
        metric: super::metrics::ReplicaOperationGuard,
        context: PeerImportContext<'_>,
    ) -> Result<Option<ReplicaReportJob>, WorkerError> {
        let pending = self
            .begin_peer_install(
                payload,
                metric,
                context.operation_namespace,
                context.read_scope_id,
                context.import_attempt,
            )
            .await?;
        self.finish_peer_install(metadata, pending).await
    }

    /// 把完整 Block 投递给 Node owner，但不等待 owner 完成复制。PullBlocks 正常路径
    /// 借此用一个固定为 2 的小流水线重叠“接收下一块”和“安装上一块”。
    async fn begin_peer_install(
        &self,
        payload: PeerBlockResult,
        metric: super::metrics::ReplicaOperationGuard,
        operation_namespace: &[u8],
        read_scope_id: u64,
        import_attempt: Option<u64>,
    ) -> Result<PendingPeerInstall, WorkerError> {
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
        let mut report_operation = operation_namespace.to_vec();
        report_operation.extend_from_slice(&report_block_id);
        Ok(PendingPeerInstall {
            block_id: report_block_id,
            attempt: import_attempt,
            receiver: reply_rx,
            metric,
            received_bytes,
            report_length,
            report_checksum,
            report_operation,
        })
    }

    async fn finish_peer_install(
        &self,
        metadata: &MetadataClient,
        mut pending: PendingPeerInstall,
    ) -> Result<Option<ReplicaReportJob>, WorkerError> {
        let needs_report = receive(pending.receiver).await?;
        pending
            .metric
            .success_with_payload(ReplicaDirection::Receive, pending.received_bytes);
        // 该耗时包括拉取、完整校验和本地安装，不包括下面的 Meta 位置登记。
        drop(pending.metric);
        if !needs_report {
            return Ok(None);
        }

        // Report belongs to this Node incarnation. Including node_epoch keeps
        // a restart from hitting an idempotency result created by an old Node.
        pending
            .report_operation
            .extend_from_slice(&metadata.node_epoch().await.to_be_bytes());
        Ok(Some(ReplicaReportJob::new(
            pending.block_id,
            pending.report_length,
            pending.report_checksum,
            digest(&pending.report_operation),
        )))
    }

    pub(crate) async fn set_range(&self, input: SetRangeInput) -> Result<SetOutcome, WorkerError> {
        validate_user_key(&input.key)?;
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
        let payload = pull_block_from_peers(
            &spec.source_node_id,
            PeerPullSpec::single_source(
                spec.source_endpoint.clone(),
                spec.block_id.clone(),
                spec.expected_checksum.clone(),
                spec.expected_length,
            ),
            &self.rpc_metrics,
            &self.peer_channels,
            &self.metrics,
        )
        .await?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.submit(NodeCommand::PrepareReplica {
            plan_id: spec.plan_id,
            block_id: payload.block_id,
            // 主动复制流程当前把 prepared bytes 保存在 NodeState 的 owned Vec；
            // P4 只优化读取接管路径，这里保留明确转换，避免改变副本状态机。
            bytes: payload.payload.to_vec(),
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
type DataCoreCachedRead = (Option<u64>, DataCoreCachedReadOutcome);

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
    ResourceSummary {
        reply: oneshot::Sender<Result<pb::ResourceSummary, WorkerError>>,
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
    DataCoreSetInline {
        key: Vec<u8>,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        condition: String,
        reply: oneshot::Sender<Result<SetOutcome, WorkerError>>,
    },
    DataCorePrepareRange {
        key: Vec<u8>,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        resolved: pb::ResolveObjectResponse,
        reply: oneshot::Sender<Result<PreparedObjectVersion, WorkerError>>,
    },
    DataCorePrepareSparse {
        key: Vec<u8>,
        logical_length: u64,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        expected_version: Option<u64>,
        reply: oneshot::Sender<Result<PreparedObjectVersion, WorkerError>>,
    },
    DataCorePrepareTruncate {
        key: Vec<u8>,
        new_length: u64,
        operation_id: Vec<u8>,
        resolved: pb::ResolveObjectResponse,
        reply: oneshot::Sender<Result<PreparedObjectVersion, WorkerError>>,
    },
    DataCorePreparePunchHole {
        key: Vec<u8>,
        offset: u64,
        length: u64,
        operation_id: Vec<u8>,
        resolved: pb::ResolveObjectResponse,
        reply: oneshot::Sender<Result<PreparedObjectVersion, WorkerError>>,
    },
    DataCoreReserveFileSpace {
        reservation_id: Vec<u8>,
        length: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    DataCoreConsumeFileSpace {
        ranges: Vec<(Vec<u8>, u64)>,
        reply: oneshot::Sender<Result<Vec<ReservationConsumption>, WorkerError>>,
    },
    DataCoreRestoreFileSpace {
        consumptions: Vec<ReservationConsumption>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    DataCoreReleaseFileSpace {
        ranges: Vec<(Vec<u8>, u64)>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    DataCoreFinishPrepared {
        prepared: PreparedObjectVersion,
        version: Option<u64>,
        rejected: bool,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    #[cfg(test)]
    DataCoreSetRangeInline {
        key: Vec<u8>,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        resolved: pb::ResolveObjectResponse,
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
    #[cfg(test)]
    DataCoreDelete {
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
    DataCoreMaterializeInto {
        read_scope_id: u64,
        resolved: pb::ResolveObjectResponse,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        output: Vec<u8>,
        cache_refill: Option<(u64, Vec<u8>, Instant, u64)>,
        /// Filesystem 已持有 Exact Version，offset=0 冷读可接管完整有界布局。
        prefetch_full_layout: bool,
        reply: oneshot::Sender<Result<DataCoreMaterializeOutcome, WorkerError>>,
    },
    PlanFilesystemPeerPrefetch {
        read_scope_id: u64,
        resolved: Vec<pb::ResolveObjectResponse>,
        reply: oneshot::Sender<Result<Vec<PeerPullSpec>, WorkerError>>,
    },
    DataCoreGetCached {
        read_scope_id: u64,
        key: Vec<u8>,
        node_epoch: u64,
        exact_version: Option<u64>,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        output: Vec<u8>,
        reply: oneshot::Sender<Result<DataCoreCachedRead, WorkerError>>,
    },
    ImportPeerBlock {
        block_id: Vec<u8>,
        bytes: prost::bytes::Bytes,
        checksum: Vec<u8>,
        length: u64,
        read_scope_id: u64,
        import_attempt: Option<u64>,
        reply: oneshot::Sender<Result<bool, WorkerError>>,
    },
    EnsurePeerBlocks {
        read_scope_id: u64,
        requests: Vec<PeerImportRequest>,
        node: Box<NodeHandle>,
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
    InvalidateFilesystemBinding {
        inode: u64,
        through_generation: u64,
        minimum_inode_revision: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemGetBinding {
        inode: InodeId,
        reply: oneshot::Sender<Result<Option<ResolvedInode>, WorkerError>>,
    },
    FilesystemCacheBinding {
        resolved: ResolvedInode,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemGetDentry {
        parent: InodeId,
        name: Vec<u8>,
        reply: oneshot::Sender<Result<DentryLookup, WorkerError>>,
    },
    FilesystemCacheNegativeDentry {
        parent: InodeId,
        name: Vec<u8>,
        grant: crate::filesystem::DirectoryGrant,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemApplyLocalNamespaceMutation {
        changed_directories: Vec<DirectoryVersion>,
        changed_inodes: Vec<InodeVersion>,
        removed_dentries: Vec<(InodeId, Vec<u8>)>,
        refreshed_directories: Vec<ResolvedInode>,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemInstallResolvedDentry {
        resolved: ResolvedDentry,
        apply_directory_mutation: bool,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemGetDirectoryPage {
        directory: InodeId,
        cursor: Option<Vec<u8>>,
        expected_revision: Option<u64>,
        reply: oneshot::Sender<Result<Option<DirectoryPage>, WorkerError>>,
    },
    FilesystemCacheDirectoryPage {
        cursor: Option<Vec<u8>>,
        page: DirectoryPage,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemOpenHandleWithReference {
        inode: InodeId,
        flags: i32,
        lock_owner: Option<u64>,
        reply: oneshot::Sender<Result<FilesystemOpenReferenceResult, WorkerError>>,
    },
    FilesystemGetHandle {
        handle: FileHandleId,
        reply: oneshot::Sender<
            Result<Option<super::filesystem::open_handles::OpenHandle>, WorkerError>,
        >,
    },
    FilesystemCloseHandleWithReference {
        handle: FileHandleId,
        reply: oneshot::Sender<Result<FilesystemCloseReferenceResult, WorkerError>>,
    },
    FilesystemAcquireInodeReference {
        inode: InodeId,
        reply: oneshot::Sender<Result<Option<u64>, WorkerError>>,
    },
    FilesystemReleaseInodeReference {
        inode: InodeId,
        count: u64,
        reply: oneshot::Sender<Result<Option<(u64, bool)>, WorkerError>>,
    },
    FilesystemMarkInodeOrphan {
        inode: InodeId,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemInstallInodeReference {
        inode: InodeId,
        generation: u64,
        lease_millis: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemInodeReferenceSnapshot {
        reply: oneshot::Sender<Result<Vec<(InodeId, u64)>, WorkerError>>,
    },
    FilesystemRenewInodeReferenceLeases {
        references: Vec<(InodeId, u64)>,
        lease_millis: u64,
        reply: oneshot::Sender<Result<(), WorkerError>>,
    },
    FilesystemHasLiveInodeReference {
        inode: InodeId,
        reply: oneshot::Sender<Result<bool, WorkerError>>,
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
    BeginDataCoreReadScope {
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
            Self::ResourceSummary { .. } => NodeMailboxCommand::Heartbeat,
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
            Self::DataCoreSetInline { .. } => NodeMailboxCommand::SetInline,
            Self::DataCorePrepareRange { .. } => NodeMailboxCommand::SetRange,
            Self::DataCorePrepareSparse { .. } => NodeMailboxCommand::SetRange,
            Self::DataCorePrepareTruncate { .. } => NodeMailboxCommand::SetRange,
            Self::DataCorePreparePunchHole { .. } => NodeMailboxCommand::SetRange,
            Self::DataCoreReserveFileSpace { .. }
            | Self::DataCoreConsumeFileSpace { .. }
            | Self::DataCoreRestoreFileSpace { .. }
            | Self::DataCoreReleaseFileSpace { .. } => NodeMailboxCommand::SetRange,
            Self::DataCoreFinishPrepared { .. } => NodeMailboxCommand::SetInline,
            Self::SetRange { .. } => NodeMailboxCommand::SetRange,
            #[cfg(test)]
            Self::DataCoreSetRangeInline { .. } => NodeMailboxCommand::SetRange,
            Self::Delete { .. } => NodeMailboxCommand::Delete,
            #[cfg(test)]
            Self::DataCoreDelete { .. } => NodeMailboxCommand::Delete,
            Self::GetResolved { .. } => NodeMailboxCommand::GetResolved,
            Self::GetCached { .. } => NodeMailboxCommand::GetCached,
            Self::MaterializeResolved { .. } => NodeMailboxCommand::MaterializeResolved,
            Self::DataCoreMaterializeInto { .. } => NodeMailboxCommand::MaterializeResolved,
            Self::PlanFilesystemPeerPrefetch { .. } => NodeMailboxCommand::MaterializeResolved,
            Self::DataCoreGetCached { .. } => NodeMailboxCommand::GetCached,
            Self::ImportPeerBlock { .. } => NodeMailboxCommand::ImportPeerBlock,
            Self::EnsurePeerBlocks { .. } => NodeMailboxCommand::ImportPeerBlock,
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
            Self::InvalidateFilesystemBinding { .. } => NodeMailboxCommand::InvalidateCurrent,
            Self::FilesystemGetBinding { .. }
            | Self::FilesystemCacheBinding { .. }
            | Self::FilesystemGetDentry { .. }
            | Self::FilesystemCacheNegativeDentry { .. }
            | Self::FilesystemApplyLocalNamespaceMutation { .. }
            | Self::FilesystemInstallResolvedDentry { .. }
            | Self::FilesystemGetDirectoryPage { .. }
            | Self::FilesystemCacheDirectoryPage { .. } => NodeMailboxCommand::GetCached,
            Self::FilesystemOpenHandleWithReference { .. }
            | Self::FilesystemGetHandle { .. }
            | Self::FilesystemCloseHandleWithReference { .. }
            | Self::FilesystemAcquireInodeReference { .. }
            | Self::FilesystemReleaseInodeReference { .. }
            | Self::FilesystemMarkInodeOrphan { .. }
            | Self::FilesystemInstallInodeReference { .. }
            | Self::FilesystemInodeReferenceSnapshot { .. }
            | Self::FilesystemRenewInodeReferenceLeases { .. }
            | Self::FilesystemHasLiveInodeReference { .. } => NodeMailboxCommand::GetCached,
            Self::PrepareBlockRetirement { .. } => NodeMailboxCommand::InvalidateCurrent,
            Self::FinalizeBlockRetirement { .. } => NodeMailboxCommand::InvalidateCurrent,
            Self::BeginReadScope { .. } => NodeMailboxCommand::GetCached,
            Self::BeginDataCoreReadScope { .. } => NodeMailboxCommand::GetCached,
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
    let (replica_report_tx, replica_report_rx) = mpsc::channel(NODE_MAILBOX_CAPACITY);
    let replica_reporter = metadata
        .clone()
        .map(|metadata| tokio::spawn(replica_reporter::run(metadata, replica_report_rx)));
    // NodeState 是普通非线程安全结构，因为它从始至终只属于当前 Task。
    let mut state = NodeState::with_metrics(node_id, metadata, task_config, metrics.clone());
    let mut maintenance = tokio::time::interval(Duration::from_secs(1));
    let mut writes = tokio::task::JoinSet::<ApplyWrite>::new();
    // owner 保管表和任务集合；Future 只做网络等待，并通过原 Import 命令交回 bytes。
    let mut peer_imports = tokio::task::JoinSet::<Vec<PeerImportCompletion>>::new();
    loop {
        // recv().await 在队列为空时挂起；maintenance tick 同样在这个唯一 owner
        // 内执行，因此回收不会引入第二个 Arena 状态入口。
        let next_cache_expiry = state.next_cache_lease_expiry();
        let queued = tokio::select! {
            completed = peer_imports.join_next_with_id(), if !peer_imports.is_empty() => {
                match completed {
                    Some(Ok((_, completed))) => {
                        for (block_id, attempt, result) in completed {
                            state.complete_peer_import(
                                &block_id,
                                attempt,
                                result.map(|()| None),
                            );
                        }
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
                    let _ = reply.send(state.open_session_with_write_release(
                        shared_memory,
                        supports_write_lease_release,
                    ));
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
                        state.reset_current_cache_for_watch(connected);
                    }
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::ResourceSummary { reply } => {
                    let _ = reply.send(Ok(state.arena.resource_summary()));
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
                NodeCommand::DataCoreSetInline {
                    key,
                    bytes,
                    operation_id,
                    condition,
                    reply,
                } => {
                    launch_write(&mut state, &mut writes, reply, |state| {
                        state.commit_bytes_for_data_core(key, bytes, operation_id, condition)
                    });
                }
                NodeCommand::DataCorePrepareRange {
                    key,
                    offset,
                    bytes,
                    operation_id,
                    resolved,
                    reply,
                } => {
                    let _ = reply.send(state.prepare_range_for_filesystem(
                        key,
                        offset,
                        bytes,
                        operation_id,
                        resolved,
                    ));
                }
                NodeCommand::DataCorePrepareSparse {
                    key,
                    logical_length,
                    offset,
                    bytes,
                    operation_id,
                    expected_version,
                    reply,
                } => {
                    let _ = reply.send(state.prepare_sparse_for_filesystem(
                        key,
                        logical_length,
                        offset,
                        bytes,
                        operation_id,
                        expected_version,
                    ));
                }
                NodeCommand::DataCorePrepareTruncate {
                    key,
                    new_length,
                    operation_id,
                    resolved,
                    reply,
                } => {
                    let _ = reply.send(state.prepare_truncate_for_filesystem(
                        key,
                        new_length,
                        operation_id,
                        resolved,
                    ));
                }
                NodeCommand::DataCorePreparePunchHole {
                    key,
                    offset,
                    length,
                    operation_id,
                    resolved,
                    reply,
                } => {
                    let _ = reply.send(state.prepare_punch_hole_for_filesystem(
                        key,
                        offset,
                        length,
                        operation_id,
                        resolved,
                    ));
                }
                NodeCommand::DataCoreReserveFileSpace {
                    reservation_id,
                    length,
                    reply,
                } => {
                    let _ = reply.send(
                        state
                            .arena
                            .reserve_file_space(reservation_id, length)
                            .map_err(map_arena_error),
                    );
                }
                NodeCommand::DataCoreConsumeFileSpace { ranges, reply } => {
                    let mut consumed = Vec::with_capacity(ranges.len());
                    let mut failure = None;
                    for (reservation_id, length) in ranges {
                        match state.arena.consume_file_space(&reservation_id, length) {
                            Ok(receipt) => consumed.push(receipt),
                            Err(error) => {
                                failure = Some(map_arena_error(error));
                                break;
                            }
                        }
                    }
                    if let Some(error) = failure {
                        for receipt in &consumed {
                            let _ = state.arena.restore_file_space(receipt);
                        }
                        let _ = reply.send(Err(error));
                    } else {
                        let _ = reply.send(Ok(consumed));
                    }
                }
                NodeCommand::DataCoreRestoreFileSpace {
                    consumptions,
                    reply,
                } => {
                    let result = consumptions.iter().try_for_each(|consumption| {
                        state
                            .arena
                            .restore_file_space(consumption)
                            .map_err(map_arena_error)
                    });
                    let _ = reply.send(result);
                }
                NodeCommand::DataCoreReleaseFileSpace { ranges, reply } => {
                    let result = ranges.iter().try_for_each(|(reservation_id, length)| {
                        state
                            .arena
                            .release_file_space(reservation_id, *length)
                            .map_err(map_arena_error)
                    });
                    let _ = reply.send(result);
                }
                NodeCommand::DataCoreFinishPrepared {
                    prepared,
                    version,
                    rejected,
                    reply,
                } => {
                    let _ = reply
                        .send(state.finish_filesystem_preparation(prepared, version, rejected));
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
                #[cfg(test)]
                NodeCommand::DataCoreSetRangeInline {
                    key,
                    offset,
                    bytes,
                    operation_id,
                    resolved,
                    reply,
                } => {
                    launch_write(&mut state, &mut writes, reply, |state| {
                        state.set_range_inline_for_data_core(
                            key,
                            offset,
                            bytes,
                            operation_id,
                            resolved,
                        )
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
                #[cfg(test)]
                NodeCommand::DataCoreDelete {
                    key,
                    operation_id,
                    reply,
                } => {
                    launch_write(&mut state, &mut writes, reply, |state| {
                        state.delete_for_data_core(key, operation_id)
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
                NodeCommand::DataCoreMaterializeInto {
                    read_scope_id,
                    resolved,
                    range,
                    clamp_range,
                    output,
                    cache_refill,
                    prefetch_full_layout,
                    reply,
                } => {
                    if reply.is_closed() {
                        state.finish_read_scope(read_scope_id);
                        return;
                    }
                    let _ = reply.send(state.materialize_resolved_into_for_data_core(
                        read_scope_id,
                        &resolved,
                        range,
                        clamp_range,
                        output,
                        cache_refill,
                        prefetch_full_layout,
                    ));
                }
                NodeCommand::PlanFilesystemPeerPrefetch {
                    read_scope_id,
                    resolved,
                    reply,
                } => {
                    if reply.is_closed() {
                        state.finish_read_scope(read_scope_id);
                        return;
                    }
                    let _ = reply
                        .send(state.plan_filesystem_directory_prefetch(read_scope_id, &resolved));
                }
                NodeCommand::DataCoreGetCached {
                    read_scope_id,
                    key,
                    node_epoch,
                    exact_version,
                    range,
                    clamp_range,
                    output,
                    reply,
                } => {
                    if reply.is_closed() {
                        state.finish_read_scope(read_scope_id);
                        return;
                    }
                    let result = state.get_cached_for_data_core(
                        read_scope_id,
                        &key,
                        node_epoch,
                        exact_version,
                        range,
                        clamp_range,
                        output,
                    );
                    let _ = reply.send(result);
                }
                NodeCommand::EnsurePeerBlocks {
                    read_scope_id,
                    requests,
                    node,
                } => {
                    let mut work = Vec::new();
                    for (spec, reply) in requests {
                        if let Some(attempt) = state.begin_peer_import(read_scope_id, &spec, reply)
                        {
                            work.push(PeerImportWork { spec, attempt });
                        }
                    }
                    if !work.is_empty() {
                        let block_ids = work
                            .iter()
                            .map(|item| item.spec.block_id.clone())
                            .collect::<Vec<_>>();
                        let replica_report_tx = replica_report_tx.clone();
                        let task = peer_imports.spawn(
                            async move {
                                match node.metadata.as_ref() {
                                    Some(metadata) => {
                                        node.pull_and_report_peer_blocks(
                                            metadata,
                                            read_scope_id,
                                            work,
                                            replica_report_tx,
                                        )
                                        .await
                                    }
                                    None => work
                                        .into_iter()
                                        .map(|item| {
                                            (
                                                item.spec.block_id,
                                                item.attempt,
                                                Err(PeerImportFailure::terminal(
                                                    WorkerError::MetadataUnavailable,
                                                )),
                                            )
                                        })
                                        .collect(),
                                }
                            }
                            .instrument(dms_tracing::tracing::Span::current()),
                        );
                        for block_id in block_ids {
                            state
                                .peer_imports
                                .get_mut(&block_id)
                                .expect("new peer import")
                                .task_id = Some(task.id());
                        }
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
                    let completion_block_id = block_id.clone();
                    // Bytes::clone 只增加引用计数。Arena 仍接管一份实际拷贝；成功后
                    // 把这份已经过 owner 校验的不可变接收缓冲交给当前等待读，省去
                    // 立即从 Arena 再复制一次的往返。
                    let imported_bytes = bytes.clone();
                    let result =
                        state.import_peer_block(read_scope_id, block_id, bytes, checksum, length);
                    // 一条 PullBlocks 流可能持续搬运数百个 Block。每个 Block 一旦
                    // 通过 owner 的完整性校验并进入 Arena，就立即完成自己的 flight，
                    // 让覆盖当前 FUSE range 的读先返回；无需等待整条流结束。
                    if result.is_ok()
                        && let Some(attempt) = import_attempt
                    {
                        state.complete_peer_import(
                            &completion_block_id,
                            attempt,
                            Ok(Some(imported_bytes)),
                        );
                    }
                    let _ = reply.send(result);
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
                NodeCommand::InvalidateFilesystemBinding {
                    inode,
                    through_generation,
                    minimum_inode_revision,
                    reply,
                } => {
                    state.filesystem_bindings.revoke(inode, through_generation);
                    state.filesystem_dentries.revoke_directory(
                        inode,
                        through_generation,
                        minimum_inode_revision,
                    );
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemGetBinding { inode, reply } => {
                    let resolved = state
                        .filesystem_bindings
                        .get_authorized(inode, Instant::now());
                    let _ = reply.send(Ok(resolved));
                }
                NodeCommand::FilesystemCacheBinding { resolved, reply } => {
                    state.filesystem_bindings.insert(resolved, Instant::now());
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemGetDentry {
                    parent,
                    name,
                    reply,
                } => {
                    let dentry = state
                        .filesystem_dentries
                        .lookup(parent, &name, Instant::now());
                    let _ = reply.send(Ok(dentry));
                }
                NodeCommand::FilesystemCacheNegativeDentry {
                    parent,
                    name,
                    grant,
                    reply,
                } => {
                    state
                        .filesystem_dentries
                        .insert_negative(parent, name, grant, Instant::now());
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemApplyLocalNamespaceMutation {
                    changed_directories,
                    changed_inodes,
                    removed_dentries,
                    refreshed_directories,
                    reply,
                } => {
                    for directory in changed_directories {
                        state
                            .filesystem_bindings
                            .revoke(directory.inode, directory.grant_generation);
                        let removed_names = removed_dentries
                            .iter()
                            .filter_map(|(parent, name)| {
                                (*parent == directory.inode).then_some(name.clone())
                            })
                            .collect::<Vec<_>>();
                        state.filesystem_dentries.apply_local_mutation(
                            directory.inode,
                            directory.revision,
                            directory.grant_generation,
                            &removed_names,
                        );
                    }
                    for inode in changed_inodes {
                        state
                            .filesystem_bindings
                            .revoke(inode.inode, inode.grant_generation);
                    }
                    // Meta 已在同一权威 mutation 响应里返回更新后的父目录快照。
                    // 先撤销旧水位、再安装新 grant，避免内核随后 getattr 时为同一
                    // mutation 追加一次 GetFilesystemInode RPC。
                    for resolved in refreshed_directories {
                        state.filesystem_bindings.insert(resolved, Instant::now());
                    }
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemInstallResolvedDentry {
                    resolved,
                    apply_directory_mutation,
                    reply,
                } => {
                    let ResolvedDentry {
                        dentry,
                        resolved,
                        directory_grant,
                        refreshed_directories,
                        entry_reference_lease_millis,
                        entry_reference_generation,
                        prefetched_entries,
                    } = resolved;
                    if apply_directory_mutation {
                        state
                            .filesystem_bindings
                            .revoke(dentry.parent, directory_grant.grant.generation);
                        state.filesystem_dentries.apply_local_mutation(
                            dentry.parent,
                            directory_grant.directory_revision,
                            directory_grant.grant.generation,
                            std::slice::from_ref(&dentry.name),
                        );
                    }
                    state.filesystem_dentries.insert_positive(
                        dentry,
                        directory_grant,
                        Instant::now(),
                    );
                    let inode = resolved.granted.inode.attributes.inode;
                    state.filesystem_bindings.insert(resolved, Instant::now());
                    // 顺带条目和主条目来自同一个 Meta actor turn，共用同一份
                    // DirectoryGrant。它们同时安装 count=0 的短期 reference
                    // reservation；内核真正 lookup 时才在本地升级为 nlookup。
                    for prefetched in prefetched_entries {
                        let inode = prefetched.resolved.granted.inode.attributes.inode;
                        state.filesystem_dentries.insert_positive(
                            prefetched.dentry,
                            directory_grant,
                            Instant::now(),
                        );
                        state
                            .filesystem_bindings
                            .insert(prefetched.resolved, Instant::now());
                        state.install_filesystem_inode_reference_reservation(
                            inode,
                            prefetched.entry_reference_generation,
                            prefetched.entry_reference_lease_millis,
                        );
                    }
                    for directory in refreshed_directories {
                        state.filesystem_bindings.insert(directory, Instant::now());
                    }
                    state.install_filesystem_inode_reference(
                        inode,
                        entry_reference_generation,
                        entry_reference_lease_millis,
                    );
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemGetDirectoryPage {
                    directory,
                    cursor,
                    expected_revision,
                    reply,
                } => {
                    let page = state.filesystem_dentries.directory_page(
                        directory,
                        cursor.as_deref(),
                        expected_revision,
                        Instant::now(),
                    );
                    let _ = reply.send(Ok(page));
                }
                NodeCommand::FilesystemCacheDirectoryPage {
                    cursor,
                    page,
                    reply,
                } => {
                    state
                        .filesystem_dentries
                        .insert_directory_page(cursor, page, Instant::now());
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemOpenHandleWithReference {
                    inode,
                    flags,
                    lock_owner,
                    reply,
                } => {
                    let generation = state.acquire_filesystem_inode_reference_local(inode);
                    let handle = state.filesystem_handles.open(inode, flags, lock_owner);
                    let _ = reply.send(Ok((handle, generation)));
                }
                NodeCommand::FilesystemGetHandle { handle, reply } => {
                    let opened = state.filesystem_handles.get(handle).cloned();
                    let _ = reply.send(Ok(opened));
                }
                NodeCommand::FilesystemCloseHandleWithReference { handle, reply } => {
                    let closed = state.filesystem_handles.close(handle).map(|opened| {
                        let released =
                            state.release_filesystem_inode_reference_local(opened.inode, 1);
                        (opened, released)
                    });
                    let _ = reply.send(Ok(closed));
                }
                NodeCommand::FilesystemAcquireInodeReference { inode, reply } => {
                    let generation = state.acquire_filesystem_inode_reference_local(inode);
                    let _ = reply.send(Ok(generation));
                }
                NodeCommand::FilesystemReleaseInodeReference {
                    inode,
                    count,
                    reply,
                } => {
                    let generation = state.release_filesystem_inode_reference_local(inode, count);
                    let _ = reply.send(Ok(generation));
                }
                NodeCommand::FilesystemMarkInodeOrphan { inode, reply } => {
                    state.mark_filesystem_inode_orphan(inode);
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemInstallInodeReference {
                    inode,
                    generation,
                    lease_millis,
                    reply,
                } => {
                    state.install_filesystem_inode_reference(inode, generation, lease_millis);
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemInodeReferenceSnapshot { reply } => {
                    let _ = reply.send(Ok(state.filesystem_inode_reference_snapshot()));
                }
                NodeCommand::FilesystemRenewInodeReferenceLeases {
                    references,
                    lease_millis,
                    reply,
                } => {
                    state.renew_filesystem_inode_reference_leases(&references, lease_millis);
                    let _ = reply.send(Ok(()));
                }
                NodeCommand::FilesystemHasLiveInodeReference { inode, reply } => {
                    let _ = reply.send(Ok(state.has_live_filesystem_inode_reference(inode)));
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
                NodeCommand::BeginDataCoreReadScope {
                    cleanup_node,
                    reply,
                } => {
                    state.begin_data_core_read_scope_reply(*cleanup_node, reply);
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
    // Node owner 消失后不允许 reporter 独立存活；进程下次启动会以新 epoch
    // 重新建立可达性事实。未处理队列保持进程内有界，不伪装成持久任务。
    if let Some(replica_reporter) = replica_reporter {
        replica_reporter.abort();
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
    /// 文件 inode→Exact ObjectVersion 的本地授权索引；只由 Node owner 修改。
    filesystem_bindings: BindingCache,
    /// 文件 path component→inode 的正向提示；当前只缓存 create/lookup 成功结果。
    filesystem_dentries: DentryCache,
    /// FUSE open() 生命周期属于本 Node，不进入 Meta，也不复制 DataCore 状态。
    filesystem_handles: OpenHandleTable,
    /// 本 Node 内 open/lookup/opendir 对 inode 的本地引用计数。
    ///
    /// Meta 只需要知道“这个 node epoch 是否仍持有引用”，不保存精确 count。
    /// 因此只有本表从 0→1 时建立 Meta lease；普通 N→0 停止 heartbeat 续租即可，
    /// 已知 orphan 的 N→0 才立即发送 Meta release 以加速回收。
    filesystem_inode_references: HashMap<InodeId, LocalFilesystemInodeReference>,
}

#[derive(Clone, Debug)]
struct LocalFilesystemInodeReference {
    count: u64,
    generation: u64,
    lease_until: Instant,
    /// 该 Node 已观察到 link_count 变为 0；最后一个本地引用释放时可立即通知 Meta。
    eager_meta_release: bool,
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
        // session_id 对外仍是 u64，但每次 Node 进程启动都会选择一个新的非零起点。
        // 这样即使 Node 重启，旧 Client 持有的 session_id 也不容易误撞新 incarnation。
        let next_session = node_session_start(&node_id);
        Self {
            node_id,
            next_session,
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
            filesystem_bindings: BindingCache::default(),
            filesystem_dentries: DentryCache::default(),
            filesystem_handles: OpenHandleTable::default(),
            filesystem_inode_references: HashMap::new(),
        }
    }

    #[cfg(test)]
    fn set_next_session_start_for_test(&mut self, raw: u64) {
        self.next_session = normalize_session_start(raw);
    }

    #[cfg(test)]
    fn force_next_session_for_test(&mut self, value: u64) {
        self.next_session = value;
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

    fn acquire_filesystem_inode_reference_local(&mut self, inode: InodeId) -> Option<u64> {
        if inode == ROOT_INODE {
            return None;
        }
        let now = Instant::now();
        if let Some(reference) = self.filesystem_inode_references.get_mut(&inode) {
            if reference.count != 0 || reference.lease_until > now {
                // count=0 表示目录预取留下的短期 reservation。
                // 内核真正 lookup 时在 owner turn 内升级为活跃引用，
                // 不再单独访问 Meta。
                reference.count = reference.count.saturating_add(1);
                self.metrics.record_filesystem_inode_reference_transition(
                    FilesystemInodeReferenceTransition::Retain,
                );
                dms_logging::debug!(
                    "retained local filesystem inode reference";
                    "event" => "node.filesystem.reference.retained",
                    "inode" => inode,
                    "generation" => reference.generation,
                    "count" => reference.count,
                );
                return None;
            }
            // 未使用的 reservation 已过期；本次 lookup 必须重新建立
            // Meta 保护，不能把过期 generation 当成活跃引用。
            self.filesystem_inode_references.remove(&inode);
        }
        let generation = next_filesystem_reference_generation();
        self.filesystem_inode_references.insert(
            inode,
            LocalFilesystemInodeReference {
                count: 1,
                generation,
                lease_until: Instant::now(),
                eager_meta_release: false,
            },
        );
        self.metrics
            .set_filesystem_inode_references(self.filesystem_inode_references.len());
        self.metrics.record_filesystem_inode_reference_transition(
            FilesystemInodeReferenceTransition::Acquire,
        );
        dms_logging::debug!(
            "acquired local filesystem inode reference";
            "event" => "node.filesystem.reference.acquired",
            "inode" => inode,
            "generation" => generation,
            "count" => 1_u64,
        );
        Some(generation)
    }

    fn install_filesystem_inode_reference_reservation(
        &mut self,
        inode: InodeId,
        generation: u64,
        lease_millis: u64,
    ) {
        if inode == ROOT_INODE || generation == 0 || lease_millis == 0 {
            return;
        }
        let lease_until = Instant::now() + Duration::from_millis(lease_millis);
        match self.filesystem_inode_references.get_mut(&inode) {
            Some(reference) if reference.count != 0 => {
                // 活跃 nlookup 由 heartbeat 续租，预取不改变其计数、世代或 TTL。
            }
            Some(reference) => {
                reference.generation = generation;
                reference.lease_until = reference.lease_until.max(lease_until);
            }
            None => {
                self.filesystem_inode_references.insert(
                    inode,
                    LocalFilesystemInodeReference {
                        count: 0,
                        generation,
                        lease_until,
                        eager_meta_release: false,
                    },
                );
                self.metrics
                    .set_filesystem_inode_references(self.filesystem_inode_references.len());
            }
        }
    }

    fn install_filesystem_inode_reference(
        &mut self,
        inode: InodeId,
        generation: u64,
        lease_millis: u64,
    ) {
        if inode == ROOT_INODE || generation == 0 {
            return;
        }
        let lease_until = Instant::now() + Duration::from_millis(lease_millis);
        match self.filesystem_inode_references.get_mut(&inode) {
            Some(reference) => {
                reference.count = reference.count.saturating_add(1);
                if generation >= reference.generation {
                    reference.generation = generation;
                    reference.lease_until = lease_until;
                }
                self.metrics.record_filesystem_inode_reference_transition(
                    FilesystemInodeReferenceTransition::Retain,
                );
                dms_logging::debug!(
                    "retained installed filesystem inode reference";
                    "event" => "node.filesystem.reference.retained",
                    "inode" => inode,
                    "generation" => reference.generation,
                    "count" => reference.count,
                );
            }
            None => {
                self.filesystem_inode_references.insert(
                    inode,
                    LocalFilesystemInodeReference {
                        count: 1,
                        generation,
                        lease_until,
                        eager_meta_release: false,
                    },
                );
                self.metrics
                    .set_filesystem_inode_references(self.filesystem_inode_references.len());
                self.metrics.record_filesystem_inode_reference_transition(
                    FilesystemInodeReferenceTransition::Acquire,
                );
                dms_logging::debug!(
                    "installed local filesystem inode reference";
                    "event" => "node.filesystem.reference.acquired",
                    "inode" => inode,
                    "generation" => generation,
                    "count" => 1_u64,
                );
            }
        }
    }

    fn filesystem_inode_reference_snapshot(&self) -> Vec<(InodeId, u64)> {
        self.filesystem_inode_references
            .iter()
            // count=0 是未被内核使用的短期预留，不得通过 heartbeat
            // 把它变成长期引用。
            .filter(|(_, reference)| reference.count != 0)
            .map(|(inode, reference)| (*inode, reference.generation))
            .collect()
    }

    fn renew_filesystem_inode_reference_leases(
        &mut self,
        references: &[(InodeId, u64)],
        lease_millis: u64,
    ) {
        let lease_until = Instant::now() + Duration::from_millis(lease_millis);
        for (inode, generation) in references {
            if let Some(reference) = self.filesystem_inode_references.get_mut(inode)
                && reference.generation == *generation
            {
                reference.lease_until = lease_until;
            }
        }
    }

    fn has_live_filesystem_inode_reference(&self, inode: InodeId) -> bool {
        inode == ROOT_INODE
            || self
                .filesystem_inode_references
                .get(&inode)
                .is_some_and(|reference| reference.lease_until > Instant::now())
    }

    fn mark_filesystem_inode_orphan(&mut self, inode: InodeId) {
        if let Some(reference) = self.filesystem_inode_references.get_mut(&inode) {
            reference.eager_meta_release = true;
        }
    }

    fn release_filesystem_inode_reference_local(
        &mut self,
        inode: InodeId,
        count: u64,
    ) -> Option<(u64, bool)> {
        if inode == ROOT_INODE || count == 0 {
            return None;
        }
        let reference = self.filesystem_inode_references.get_mut(&inode)?;
        if reference.count > count {
            reference.count -= count;
            self.metrics.record_filesystem_inode_reference_transition(
                FilesystemInodeReferenceTransition::ReleasePartial,
            );
            dms_logging::debug!(
                "released part of local filesystem inode reference";
                "event" => "node.filesystem.reference.released",
                "inode" => inode,
                "generation" => reference.generation,
                "count" => reference.count,
            );
            return None;
        }
        let generation = reference.generation;
        let eager_meta_release = reference.eager_meta_release;
        self.filesystem_inode_references.remove(&inode);
        self.metrics
            .set_filesystem_inode_references(self.filesystem_inode_references.len());
        self.metrics.record_filesystem_inode_reference_transition(
            FilesystemInodeReferenceTransition::ReleaseFinal,
        );
        dms_logging::debug!(
            "released final local filesystem inode reference";
            "event" => "node.filesystem.reference.released_final",
            "inode" => inode,
            "generation" => generation,
            "count" => 0_u64,
        );
        Some((generation, eager_meta_release))
    }

    fn reset_current_cache_for_watch(&mut self, connected: bool) {
        // Meta watch 是 Current cache 的一致性前提。断线时立即撤销 Node 本地
        // Current cache，并关闭后续新 grant；重连时也要撤销断线期间可能返回的
        // 旧响应，等待 last_ack 追上 replay 后再续租。
        //
        // 注意：这里不能清已经发给 Client 且尚未过期的 cache_until/interests。
        // Node 无法同步收回 Client 侧 TTL 内的旧资格；这些旧 grant 仍需按 TTL
        // 或 ACK 屏障自然结束，才能避免写入过早放行。
        let reason = if connected {
            CurrentCacheResetReason::WatchReconnected
        } else {
            CurrentCacheResetReason::WatchDisconnected
        };
        self.current_cache.clear();
        self.filesystem_bindings.clear();
        self.filesystem_dentries.clear();
        self.metrics.set_current_cache_charge(0);
        self.metadata_watch_connected = connected;
        self.metrics.record_current_cache_reset(reason);
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
        // 完整 Block 在提交时已经计算并保存过 digest。Block 发布后不可变，
        // 因此 Peer 整块读取直接复用这份身份，不再为每次拉取扫描全部 bytes。
        // 区间读取返回的是不同的 payload，仍必须为该区间单独计算 checksum。
        let committed_checksum = range.is_none().then(|| {
            self.arena
                .block_length_and_digest(&block_id)
                .map(|(_, digest)| digest.to_vec())
                .ok_or(WorkerError::NotFound)
        });
        let (ticket, full_length) = self
            .arena
            .open_read(&block_id, range)
            .map_err(map_arena_error)?;
        let payload = self.arena.read_ticket(ticket).map_err(map_arena_error)?;
        let checksum = match committed_checksum {
            Some(checksum) => checksum?,
            None => digest(&payload),
        };
        Ok(PeerBlockResult {
            serving_node_id: self.node_id.clone(),
            block_id,
            checksum,
            length: full_length,
            payload: payload.into(),
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
            .expect("test session id should be available")
    }

    fn open_session_with_write_release(
        &mut self,
        shared_memory: bool,
        supports_write_lease_release: bool,
    ) -> Result<u64, WorkerError> {
        // 先取当前 ID，再推进计数器；`&mut self` 保证此过程由 owner 串行执行。
        // 如果理论上的 2^64 空间耗尽，返回资源耗尽，不能回绕成 0 或复用旧 session。
        let id = self.next_session_id()?;
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
        Ok(id)
    }

    fn next_session_id(&mut self) -> Result<u64, WorkerError> {
        if self.next_session == 0 {
            // 0 是协议哨兵，真实生产不会走到这里；保留修正逻辑便于抵御测试注入或坏状态。
            self.next_session = normalize_session_start(0);
        }
        let id = self.next_session;
        if id == u64::MAX {
            return Err(WorkerError::ResourceExhausted);
        }
        self.next_session = id.checked_add(1).ok_or(WorkerError::ResourceExhausted)?;
        Ok(id)
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
        let can_use_shared_memory = shared_memory && self.arena.shared_region_enabled();
        if !can_use_shared_memory {
            // TCP/gRPC payload 必须受单条 protobuf 消息预算保护；本地 SHM session
            // 返回 offset/length，真正 bytes 走 mmap，不受该上限影响。
            validate_grpc_payload_bytes(length)?;
        }
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
        validate_grpc_payload_bytes(bytes.len() as u64)?;
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
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
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
        validate_batch_write(&entries)?;
        validate_operation_id(&operation_id)?;
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
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
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
            .filter(|set| !set.block_id.is_empty())
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

    #[cfg(test)]
    fn set_range_inline_for_data_core(
        &mut self,
        key: Vec<u8>,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        resolved: pb::ResolveObjectResponse,
    ) -> Result<PreparedWrite<SetOutcome>, WorkerError> {
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
        let metadata = self
            .metadata
            .clone()
            .ok_or(WorkerError::MetadataUnavailable)?;
        let layout = resolved.layout.as_ref().ok_or(WorkerError::NotFound)?;
        let base_version = layout.version;
        let patch_length = bytes.len() as u64;
        let patch_end = offset
            .checked_add(patch_length)
            .ok_or(WorkerError::InvalidArgument("range write overflows u64"))?;
        if patch_end > layout.logical_length {
            return Err(WorkerError::InvalidArgument(
                "range write extends beyond current value",
            ));
        }

        // 进程内文件子系统已经把 patch bytes 交给 Node owner。这里直接把 patch
        // 封成新的 immutable Block，再用 Extent overlay 描述新版本；不读取、不复制
        // base value。
        let patch_block = block_identity(&self.node_id, &operation_id);
        let owns_block = self.check_block_preparation(&patch_block)?;
        let patch_digest = digest(&bytes);
        let extents = super::version_layout::overlay(
            &layout.extents,
            layout.logical_length,
            offset,
            patch_length,
            &patch_block,
            &patch_digest,
        )?;
        let candidate_digest = super::version_layout::digest(layout.logical_length, &extents);
        if patch_length > 0 {
            self.arena
                .commit_inline_with_verified_digest(
                    patch_block.clone(),
                    bytes,
                    patch_digest.clone(),
                )
                .map_err(map_arena_error)?;
            self.pending_blocks.insert(patch_block.clone(), owns_block);
        }
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
                        length: patch_length,
                        checksum: patch_digest,
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
                let barrier_id = state.broadcast_invalidation(key, committed.version);
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
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
        validate_grpc_payload_bytes(bytes.len() as u64)?;
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

    fn commit_bytes_for_data_core(
        &mut self,
        key: Vec<u8>,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        condition: String,
    ) -> Result<PreparedWrite<SetOutcome>, WorkerError> {
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
        // DataCore 是进程内入口，不经过 gRPC wire，因此不能继承 8 MiB 的单消息限制。
        // 对象是否能够接纳由 Arena/Region 容量统一判断；跨进程 Worker 请求仍在各自
        // 的 gRPC handler 边界执行 `validate_grpc_payload_bytes`。
        let block_id = block_identity(&self.node_id, &operation_id);
        if bytes.is_empty() {
            return self.commit_block(ValueCommitInput {
                cache_session_id: None,
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
            cache_session_id: None,
            key,
            block_id,
            length,
            checksum,
            operation_id,
            condition,
        })
    }

    /// 为文件覆盖或扩容准备一个 patch Block 和 Extent overlay。
    ///
    /// `offset` 可以越过当前 EOF；中间范围由 HOLE Extent 表达。base bytes 始终保持
    /// immutable，普通追加也只新增 tail Block，不能退化成“读回旧文件再完整提交”。
    fn prepare_range_for_filesystem(
        &mut self,
        key: Vec<u8>,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        resolved: pb::ResolveObjectResponse,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
        if self.metadata.is_none() {
            return Err(WorkerError::MetadataUnavailable);
        }
        let layout = resolved.layout.as_ref().ok_or(WorkerError::NotFound)?;
        let patch_length = bytes.len() as u64;
        let patch_end = offset
            .checked_add(patch_length)
            .ok_or(WorkerError::InvalidArgument("range write overflows u64"))?;
        let patch_block = block_identity(&self.node_id, &operation_id);
        let patch_digest = digest(&bytes);
        let extents = super::version_layout::overlay_file_write(
            &layout.extents,
            layout.logical_length,
            offset,
            patch_length,
            &patch_block,
            &patch_digest,
        )?;
        if patch_length > 0 {
            let owns_block = self.check_block_preparation(&patch_block)?;
            self.arena
                .commit_inline_with_verified_digest(
                    patch_block.clone(),
                    bytes,
                    patch_digest.clone(),
                )
                .map_err(map_arena_error)?;
            self.pending_blocks.insert(patch_block.clone(), owns_block);
        }
        let replica_proofs = resolved
            .block_replicas
            .iter()
            .filter(|set| set.block_id != patch_block)
            .filter_map(|set| set.proofs.first().cloned())
            .collect();
        let new_replicas = (patch_length > 0)
            .then_some(pb::ReplicaReport {
                block_id: patch_block,
                length: patch_length,
                checksum: patch_digest,
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            })
            .into_iter()
            .collect();
        Ok(PreparedObjectVersion {
            object_key: key,
            expected_object_version: Some(layout.version),
            candidate: pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length: layout.logical_length.max(patch_end),
                digest: super::version_layout::digest(
                    layout.logical_length.max(patch_end),
                    &extents,
                ),
                extents,
            },
            replica_proofs,
            new_replicas,
        })
    }

    /// 为首次稀疏写或纯扩容准备候选布局。
    ///
    /// 该入口只在文件层使用。它不会把 `[0, logical_length)` 物化为一整块 bytes：
    /// 用户实际写入的数据形成一个 Block，其余范围是 sparse hole。Meta 提交后读
    /// 路径遇到 hole 直接填零。
    fn prepare_sparse_for_filesystem(
        &mut self,
        key: Vec<u8>,
        logical_length: u64,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        expected_version: Option<u64>,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
        if self.metadata.is_none() {
            return Err(WorkerError::MetadataUnavailable);
        }
        let patch_length = bytes.len() as u64;
        let patch_end = offset
            .checked_add(patch_length)
            .ok_or(WorkerError::InvalidArgument("sparse write overflows u64"))?;
        if patch_end > logical_length {
            return Err(WorkerError::InvalidArgument(
                "sparse write extends beyond declared file length",
            ));
        }

        let mut new_replicas = Vec::new();
        let mut extents = if patch_length == 0 {
            super::version_layout::truncate_file(&[], 0, logical_length)?
        } else {
            let patch_block = block_identity(&self.node_id, &operation_id);
            let patch_digest = digest(&bytes);
            let owns_block = self.check_block_preparation(&patch_block)?;
            self.arena
                .commit_inline_with_verified_digest(
                    patch_block.clone(),
                    bytes,
                    patch_digest.clone(),
                )
                .map_err(map_arena_error)?;
            self.pending_blocks.insert(patch_block.clone(), owns_block);
            new_replicas.push(pb::ReplicaReport {
                block_id: patch_block.clone(),
                length: patch_length,
                checksum: patch_digest.clone(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            });
            super::version_layout::overlay_file_write(
                &[],
                0,
                offset,
                patch_length,
                &patch_block,
                &patch_digest,
            )?
        };
        let covered = extents
            .last()
            .and_then(|extent| extent.logical.as_ref())
            .and_then(|range| range.offset.checked_add(range.length))
            .unwrap_or_default();
        if covered < logical_length {
            extents = super::version_layout::truncate_file(&extents, covered, logical_length)?;
        }
        Ok(PreparedObjectVersion {
            object_key: key,
            expected_object_version: expected_version,
            candidate: pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length,
                digest: super::version_layout::digest(logical_length, &extents),
                extents,
            },
            replica_proofs: Vec::new(),
            new_replicas,
        })
    }

    /// 为文件缩短准备新的布局候选。
    ///
    /// 这里不接收 payload bytes，也不向 Arena 申请新 Block；它只把现有精确版本的
    /// Extent 前缀裁剪到 `new_length`。Meta 提交时仍会校验每个保留 Block 的 proof，
    /// 确认这些 bytes 已经在可达副本中存在。
    fn prepare_truncate_for_filesystem(
        &mut self,
        key: Vec<u8>,
        new_length: u64,
        operation_id: Vec<u8>,
        resolved: pb::ResolveObjectResponse,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
        if self.metadata.is_none() {
            return Err(WorkerError::MetadataUnavailable);
        }
        let layout = resolved.layout.as_ref().ok_or(WorkerError::NotFound)?;
        let extents = super::version_layout::truncate_file(
            &layout.extents,
            layout.logical_length,
            new_length,
        )?;
        let referenced_blocks = extents
            .iter()
            .filter(|extent| !super::version_layout::is_hole(extent))
            .map(|extent| extent.block_id.clone())
            .collect::<HashSet<_>>();
        let replica_proofs = resolved
            .block_replicas
            .iter()
            .filter(|set| referenced_blocks.contains(&set.block_id))
            .filter_map(|set| set.proofs.first().cloned())
            .collect();
        Ok(PreparedObjectVersion {
            object_key: key,
            expected_object_version: Some(layout.version),
            candidate: pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length: new_length,
                digest: super::version_layout::digest(new_length, &extents),
                extents,
            },
            replica_proofs,
            new_replicas: Vec::new(),
        })
    }

    /// 为文件打洞准备保持 logical length 不变的新布局。
    ///
    /// 未打洞的 Extent 继续引用原 Block；目标范围变成 HOLE，因此本阶段不触碰
    /// Arena，也不会伪造全零副本。旧 Block 何时可回收仍由既有版本生命周期决定。
    fn prepare_punch_hole_for_filesystem(
        &mut self,
        key: Vec<u8>,
        offset: u64,
        length: u64,
        operation_id: Vec<u8>,
        resolved: pb::ResolveObjectResponse,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
        if self.metadata.is_none() {
            return Err(WorkerError::MetadataUnavailable);
        }
        let layout = resolved.layout.as_ref().ok_or(WorkerError::NotFound)?;
        let extents = super::version_layout::punch_hole_file(
            &layout.extents,
            layout.logical_length,
            offset,
            length,
        )?;
        let referenced_blocks = extents
            .iter()
            .filter(|extent| !super::version_layout::is_hole(extent))
            .map(|extent| extent.block_id.clone())
            .collect::<HashSet<_>>();
        let replica_proofs = resolved
            .block_replicas
            .iter()
            .filter(|set| referenced_blocks.contains(&set.block_id))
            .filter_map(|set| set.proofs.first().cloned())
            .collect();
        Ok(PreparedObjectVersion {
            object_key: key,
            expected_object_version: Some(layout.version),
            candidate: pb::VersionCandidate {
                kind: pb::VersionKind::Value as i32,
                logical_length: layout.logical_length,
                digest: super::version_layout::digest(layout.logical_length, &extents),
                extents,
            },
            replica_proofs,
            new_replicas: Vec::new(),
        })
    }

    fn finish_filesystem_preparation(
        &mut self,
        prepared: PreparedObjectVersion,
        version: Option<u64>,
        rejected: bool,
    ) -> Result<(), WorkerError> {
        for replica in &prepared.new_replicas {
            self.finish_block_preparation(&replica.block_id, rejected);
            if let Some(version) = version {
                self.arena.mark_committed(&replica.block_id, version);
            }
        }
        if let Some(version) = version {
            // Meta 的 object + inode 提交不会通过普通 KV 写路径回填本地 Current。
            // 先撤销旧对象解析；文件热读随后使用同一提交返回的 ResolvedInode 回填。
            self.broadcast_invalidation(prepared.object_key, version);
        }
        Ok(())
    }

    #[cfg(test)]
    fn delete_for_data_core(
        &mut self,
        key: Vec<u8>,
        operation_id: Vec<u8>,
    ) -> Result<PreparedWrite<DeleteOutcome>, WorkerError> {
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
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
        // 这次提交发生前取得 cache token。若提交期间收到更高版本事件、Watch
        // 重连或租约失效，generation 会变化，完成阶段的回填会被安全拒绝。
        let cache_refill = self
            .current_cache
            .token()
            .map(|token| (token, Instant::now()));
        let cache_block_id = block_id.clone();
        let cache_checksum = checksum.clone();
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
            let local_identity = if committed.is_ok() {
                Some(metadata.local_replica_identity().await)
            } else {
                None
            };
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
                if let (Some((token_before_commit, requested_at)), Some(local_identity)) =
                    (cache_refill, local_identity)
                {
                    // 上面的本地失效恰好推进一次 generation。提交期间若还发生过
                    // 其它失效、Watch 重连或租约清理，当前 token 就不会等于
                    // `token_before_commit + 1`，此时宁可放弃回填并让下次 GET
                    // 权威解析，也不能把已经落后的版本放回 Current cache。
                    if let Some(refill_token) = token_before_commit
                        .checked_add(1)
                        .filter(|expected| state.current_cache.token() == Some(*expected))
                    {
                        state.cache_committed_value(
                            refill_token,
                            requested_at,
                            key.clone(),
                            cache_block_id,
                            length,
                            cache_checksum,
                            committed.version,
                            committed.revision,
                            local_identity,
                        );
                    }
                }
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
        validate_user_key(&key)?;
        validate_operation_id(&operation_id)?;
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
        validate_user_key(key)?;
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
        let charged_before_lookup = self.current_cache.charged();
        let cached = self.current_cache.get(key, node_epoch, now);
        if self.current_cache.charged() != charged_before_lookup {
            // get() 会顺手淘汰过期或旧 epoch 条目；Gauge 必须同步反映真实预算，
            // 否则现场会误判成“条目仍在但 key 不匹配”。
            self.metrics
                .set_current_cache_charge(self.current_cache.charged());
        }
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

    /// DataCore/进程内子系统复用 Node CurrentCache，但不创建 KV Session。
    ///
    /// 这条路径只依赖 Node 与 Meta 之间的 Watch/Lease：Watch 断开或租约过期时
    /// 直接 miss 并清理缓存；ExactVersion 只有在当前缓存版本刚好相等时机会性
    /// 复用，否则仍由上层向 Meta 做固定版本解析。这里不登记 Client cache
    /// interest，因为进程内子系统的缓存一致性由同一个 Node owner 维护。
    #[expect(
        clippy::too_many_arguments,
        reason = "DataCore cache lookup carries range/output ownership explicitly to avoid hidden session state"
    )]
    fn get_cached_for_data_core(
        &mut self,
        read_scope_id: u64,
        key: &[u8],
        node_epoch: u64,
        exact_version: Option<u64>,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        output: Vec<u8>,
    ) -> Result<DataCoreCachedRead, WorkerError> {
        validate_user_key(key)?;
        let now = Instant::now();
        if !self.metadata_watch_connected || now >= self.metadata_lease_until {
            self.current_cache.clear();
            self.metrics.set_current_cache_charge(0);
            self.metrics.record_current_cache_lookup(false);
            return Ok((None, DataCoreCachedReadOutcome::Miss { output }));
        }
        let token = self.current_cache.token();
        let charged_before_lookup = self.current_cache.charged();
        let cached = self.current_cache.get(key, node_epoch, now);
        if self.current_cache.charged() != charged_before_lookup {
            self.metrics
                .set_current_cache_charge(self.current_cache.charged());
        }
        let outcome = if let Some(cached) = cached
            .filter(|cached| exact_version.is_none_or(|version| cached.layout.version == version))
        {
            let resolved = pb::ResolveObjectResponse {
                layout: Some((*cached.layout).clone()),
                block_replicas: (*cached.block_replicas).clone(),
                current_lease: None,
            };
            match self.materialize_resolved_into_for_data_core(
                read_scope_id,
                &resolved,
                range,
                clamp_range,
                output,
                None,
                false,
            )? {
                DataCoreMaterializeOutcome::Ready(result) => {
                    DataCoreCachedReadOutcome::Ready(result)
                }
                DataCoreMaterializeOutcome::BufferTooSmall { version, required } => {
                    DataCoreCachedReadOutcome::BufferTooSmall { version, required }
                }
                DataCoreMaterializeOutcome::NeedsRemoteBlocks { specs, output, .. } => {
                    DataCoreCachedReadOutcome::NeedsRemoteBlocks {
                        resolved,
                        specs,
                        output,
                    }
                }
                DataCoreMaterializeOutcome::NotFound => DataCoreCachedReadOutcome::NotFound,
            }
        } else {
            DataCoreCachedReadOutcome::Miss { output }
        };
        self.metrics.record_current_cache_lookup(!matches!(
            outcome,
            DataCoreCachedReadOutcome::Miss { .. }
        ));
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
            if super::version_layout::is_hole(extent) {
                planned.push(PlannedReadPart::Zero {
                    logical_offset: start - requested.0,
                    length: end - start,
                });
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
                Ok((read, _)) => planned.push(PlannedReadPart::Block {
                    logical_offset: start - requested.0,
                    read,
                }),
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
        for part in planned {
            match part {
                PlannedReadPart::Zero {
                    logical_offset,
                    length,
                } => segments.push(ReadTicketSegment {
                    logical_offset,
                    target: ReadTarget::Zero { length },
                    payload_length: length,
                }),
                PlannedReadPart::Block {
                    logical_offset,
                    read,
                } => {
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
                            None => {
                                validate_grpc_payload_bytes(payload_length)?;
                                self.grpc_download_target_with_id(
                                    session_id,
                                    read,
                                    transfer_id,
                                    read_request_id,
                                )
                            }
                        }
                    } else {
                        validate_grpc_payload_bytes(payload_length)?;
                        self.grpc_download_target(session_id, read, read_request_id)
                    };
                    segments.push(ReadTicketSegment {
                        logical_offset,
                        target,
                        payload_length,
                    });
                }
            }
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
        planned: &[PlannedReadPart],
    ) -> Result<Option<Vec<u8>>, WorkerError> {
        let inline_limit = max_inline_bytes.min(dms_protocol::MAX_INLINE_READ_BYTES);
        if shared_memory || inline_limit == 0 || requested_length > inline_limit {
            return Ok(None);
        }
        let capacity =
            usize::try_from(requested_length).map_err(|_| WorkerError::ResourceExhausted)?;
        let mut ordered = planned.to_vec();
        ordered.sort_by_key(|part| match part {
            PlannedReadPart::Block { logical_offset, .. }
            | PlannedReadPart::Zero { logical_offset, .. } => *logical_offset,
        });
        let mut bytes = Vec::with_capacity(capacity);
        let mut cursor = 0_u64;
        for part in ordered {
            let (logical_offset, length) = match &part {
                PlannedReadPart::Block {
                    logical_offset,
                    read,
                } => (*logical_offset, read.length),
                PlannedReadPart::Zero {
                    logical_offset,
                    length,
                } => (*logical_offset, *length),
            };
            if logical_offset != cursor {
                return Err(WorkerError::InvalidArgument(
                    "layout extents contain a gap or overlap",
                ));
            }
            cursor = cursor
                .checked_add(length)
                .filter(|next| *next <= requested_length)
                .ok_or(WorkerError::ResourceExhausted)?;
            match part {
                PlannedReadPart::Block { read, .. } => {
                    let part = self.arena.read_ticket(read).map_err(map_arena_error)?;
                    bytes.extend_from_slice(&part);
                }
                PlannedReadPart::Zero { length, .. } => {
                    let zero_len =
                        usize::try_from(length).map_err(|_| WorkerError::ResourceExhausted)?;
                    bytes.resize(bytes.len() + zero_len, 0);
                }
            }
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
            if super::version_layout::is_hole(extent) {
                planned.push(PlannedReadPart::Zero {
                    logical_offset: logical.offset,
                    length: logical.length,
                });
                continue;
            }
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
                Ok((read, _)) => planned.push(PlannedReadPart::Block {
                    logical_offset: logical.offset,
                    read,
                }),
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
        planned.sort_by_key(planned_read_part_offset);
        let capacity =
            usize::try_from(layout.logical_length).map_err(|_| WorkerError::ResourceExhausted)?;
        let mut bytes = Vec::with_capacity(capacity);
        let mut expected_offset = 0_u64;
        for part in planned {
            let offset = planned_read_part_offset(&part);
            if offset != expected_offset {
                return Err(WorkerError::InvalidArgument(
                    "layout extents contain a gap or overlap",
                ));
            }
            match part {
                PlannedReadPart::Zero { length, .. } => {
                    let zero_len =
                        usize::try_from(length).map_err(|_| WorkerError::ResourceExhausted)?;
                    expected_offset = expected_offset
                        .checked_add(length)
                        .ok_or(WorkerError::ResourceExhausted)?;
                    bytes.resize(bytes.len() + zero_len, 0);
                }
                PlannedReadPart::Block { read, .. } => {
                    let part = self.arena.read_ticket(read).map_err(map_arena_error)?;
                    expected_offset = expected_offset
                        .checked_add(part.len() as u64)
                        .ok_or(WorkerError::ResourceExhausted)?;
                    bytes.extend_from_slice(&part);
                }
            }
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

    #[expect(
        clippy::too_many_arguments,
        reason = "owner-side materialization deliberately receives independent read-scope, exact layout, range, caller buffer, cache-refill authority, and full-layout prefetch policy"
    )]
    fn materialize_resolved_into_for_data_core(
        &mut self,
        read_scope_id: u64,
        resolved: &pb::ResolveObjectResponse,
        range: Option<(u64, u64)>,
        clamp_range: bool,
        mut output: Vec<u8>,
        cache_refill: Option<(u64, Vec<u8>, Instant, u64)>,
        prefetch_full_layout: bool,
    ) -> Result<DataCoreMaterializeOutcome, WorkerError> {
        let Some(layout) = resolved.layout.as_ref() else {
            return Ok(DataCoreMaterializeOutcome::NotFound);
        };
        super::version_layout::validate(layout.logical_length, &layout.extents)?;
        let requested = Self::normalize_read_range(range, layout.logical_length, clamp_range)?;
        let required = usize::try_from(requested.1).map_err(|_| WorkerError::ResourceExhausted)?;
        if required > output.len() {
            return Ok(DataCoreMaterializeOutcome::BufferTooSmall {
                version: layout.version,
                required,
            });
        }
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
            if super::version_layout::is_hole(extent) {
                planned.push(DataCorePlannedReadPart::Zero {
                    logical_offset: start - requested.0,
                    length: end - start,
                });
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
            let local = if self.peer_imports.contains_key(&extent.block_id) {
                Err(ArenaError::UnknownBlock)
            } else {
                self.arena.open_read(&extent.block_id, Some(block_range))
            };
            match local {
                Ok((read, _)) => planned.push(DataCorePlannedReadPart::Block {
                    logical_offset: start - requested.0,
                    read,
                }),
                Err(ArenaError::UnknownBlock) => {
                    if !missing
                        .iter()
                        .any(|item: &PeerPullSpec| item.block_id == extent.block_id)
                    {
                        missing
                            .push(self.describe_missing_block(&resolved.block_replicas, extent)?);
                    }
                    planned.push(DataCorePlannedReadPart::Remote {
                        logical_offset: start - requested.0,
                        block_id: extent.block_id.clone(),
                        block_offset,
                        length: end - start,
                    });
                }
                Err(error) => return Err(map_arena_error(error)),
            }
        }
        if requested.1 > 0 && planned.is_empty() {
            return Ok(DataCoreMaterializeOutcome::NotFound);
        }
        planned.sort_by_key(data_core_planned_read_part_offset);
        let mut cursor = 0_u64;
        let mut remote_parts = Vec::new();
        for part in planned {
            let offset = data_core_planned_read_part_offset(&part);
            if offset != cursor {
                return Err(WorkerError::InvalidArgument(
                    "layout extents contain a gap or overlap",
                ));
            }
            let start = usize::try_from(cursor).map_err(|_| WorkerError::ResourceExhausted)?;
            match part {
                DataCorePlannedReadPart::Zero { length, .. } => {
                    let length_usize =
                        usize::try_from(length).map_err(|_| WorkerError::ResourceExhausted)?;
                    let end = start
                        .checked_add(length_usize)
                        .ok_or(WorkerError::ResourceExhausted)?;
                    output[start..end].fill(0);
                    cursor = cursor
                        .checked_add(length)
                        .ok_or(WorkerError::ResourceExhausted)?;
                }
                DataCorePlannedReadPart::Block { read, .. } => {
                    let written = self
                        .arena
                        .read_ticket_into(read, &mut output[start..])
                        .map_err(map_arena_error)?;
                    cursor = cursor
                        .checked_add(written as u64)
                        .ok_or(WorkerError::ResourceExhausted)?;
                }
                DataCorePlannedReadPart::Remote {
                    logical_offset,
                    block_id,
                    block_offset,
                    length,
                } => {
                    remote_parts.push(RemoteReadPart {
                        logical_offset,
                        block_id,
                        block_offset,
                        length,
                    });
                    cursor = cursor
                        .checked_add(length)
                        .ok_or(WorkerError::ResourceExhausted)?;
                }
            }
        }
        if cursor != requested.1 {
            return Err(WorkerError::InvalidArgument(
                "layout extents do not cover requested range",
            ));
        }
        if !missing.is_empty() {
            let required_blocks = missing
                .iter()
                .map(|spec| spec.block_id.clone())
                .collect::<HashSet<_>>();
            if prefetch_full_layout {
                missing = self.expand_filesystem_peer_prefetch(
                    read_scope_id,
                    layout,
                    &resolved.block_replicas,
                    requested,
                    missing,
                );
            }
            return Ok(DataCoreMaterializeOutcome::NeedsRemoteBlocks {
                specs: missing,
                required_blocks,
                remote_parts,
                version: layout.version,
                logical_length: layout.logical_length,
                bytes_read: required,
                output,
            });
        }
        if self.metadata_watch_connected
            && Instant::now() < self.metadata_lease_until
            && !self.layout_contains_retiring_block(resolved)
            && let Some((token, key, requested_at, node_epoch)) = cache_refill
        {
            self.current_cache.insert(
                token,
                key,
                resolved,
                requested_at,
                node_epoch,
                Instant::now(),
            );
            self.metrics
                .set_current_cache_charge(self.current_cache.charged());
        }
        Ok(DataCoreMaterializeOutcome::Ready(DataCoreReadIntoResult {
            version: layout.version,
            logical_length: layout.logical_length,
            bytes: output,
            bytes_read: usize::try_from(cursor).map_err(|_| WorkerError::ResourceExhausted)?,
        }))
    }

    /// 把 Filesystem 的 offset=0 冷读从“每个 FUSE read 回调补一个 Block”提升为
    /// “一次接管这个 Exact Version 的完整有界布局”。这里只扩展接管计划，不改变
    /// 当前 read 返回的 range；后续回调直接读取 Arena 中已经校验过的 immutable Block。
    ///
    /// 预取是机会性的：布局过大、Block 过多、预算不足或任一非请求 Block 当前无法
    /// 安全解析时，退回原请求真正需要的缺块，避免让一个小 range 因预取失败而失败。
    fn expand_filesystem_peer_prefetch(
        &self,
        read_scope_id: u64,
        layout: &pb::VersionLayout,
        block_replicas: &[pb::BlockReplicaSet],
        requested: (u64, u64),
        requested_missing: Vec<PeerPullSpec>,
    ) -> Vec<PeerPullSpec> {
        if requested.0 != 0
            || requested.1 == 0
            || requested.1 >= layout.logical_length
            || layout.logical_length > FILESYSTEM_PEER_PREFETCH_BYTES_MAX
            || layout.extents.len() > PEER_PULL_PLAN_BLOCKS_MAX
        {
            return requested_missing;
        }

        let available_budget = self
            .peer_import_byte_limit
            .saturating_sub(self.peer_import_bytes);
        let mut planned_bytes = 0_u64;
        let mut seen = HashSet::new();
        let mut planned = Vec::new();
        for extent in &layout.extents {
            if super::version_layout::is_hole(extent) || !seen.insert(extent.block_id.clone()) {
                continue;
            }
            if self
                .arena
                .block_length_and_digest(&extent.block_id)
                .is_some()
            {
                continue;
            }
            if self.retirement_blocks_new_reads(&extent.block_id, read_scope_id)
                || self.peer_import_failures.contains_key(&extent.block_id)
            {
                return requested_missing;
            }
            let Ok(spec) = self.describe_missing_block(block_replicas, extent) else {
                return requested_missing;
            };
            if !self.peer_imports.contains_key(&extent.block_id) {
                let Some(next) = planned_bytes.checked_add(spec.expected_length) else {
                    return requested_missing;
                };
                planned_bytes = next;
                if planned_bytes > available_budget {
                    return requested_missing;
                }
            }
            planned.push(spec);
            if planned.len() > PEER_PULL_PLAN_BLOCKS_MAX {
                return requested_missing;
            }
        }
        if planned.is_empty() {
            requested_missing
        } else {
            planned
        }
    }

    /// 把目录 lookup 窗口里的多个文件布局收敛成一个 Peer 导入计划。
    ///
    /// 这里只返回当前确实缺失的 immutable Block；Arena 已命中、空洞和重复
    /// Block 都不进入计划。目录预取只接管“整个布局都能放进窗口”的小对象，
    /// 不能只拉一个大对象的前缀：否则后续 offset=0 的冷读会误以为首个窗口
    /// 已命中，无法把完整 Exact Version 合并为一条 PullBlocks 流。大对象由
    /// 真正 read 的顺序首读计划接管；坏的顺带条目也不能拒绝主 lookup。
    fn plan_filesystem_directory_prefetch(
        &self,
        read_scope_id: u64,
        objects: &[pb::ResolveObjectResponse],
    ) -> Result<Vec<PeerPullSpec>, WorkerError> {
        let available_budget = self
            .peer_import_byte_limit
            .saturating_sub(self.peer_import_bytes)
            .min(FILESYSTEM_DIRECTORY_PREFETCH_BYTES_MAX);
        let mut planned_bytes = 0_u64;
        let mut seen = HashSet::new();
        let mut planned = Vec::new();

        'objects: for resolved in objects {
            let Some(layout) = resolved.layout.as_ref() else {
                continue;
            };
            if super::version_layout::validate(layout.logical_length, &layout.extents).is_err() {
                continue;
            }
            // 小文件目录预取的价值是合并大量完整对象，而不是提前搬运大文件
            // 的任意前缀。超过窗口的对象整体跳过，后面的较小 sibling 仍可加入。
            if layout.logical_length > FILESYSTEM_DIRECTORY_PREFETCH_BYTES_MAX {
                continue;
            }
            for extent in &layout.extents {
                if super::version_layout::is_hole(extent) || !seen.insert(extent.block_id.clone()) {
                    continue;
                }
                if self
                    .arena
                    .block_length_and_digest(&extent.block_id)
                    .is_some()
                    || self.retirement_blocks_new_reads(&extent.block_id, read_scope_id)
                    || self.peer_import_failures.contains_key(&extent.block_id)
                {
                    continue;
                }
                let Ok(spec) = self.describe_missing_block(&resolved.block_replicas, extent) else {
                    continue;
                };
                let Some(next_bytes) = planned_bytes.checked_add(spec.expected_length) else {
                    break 'objects;
                };
                if next_bytes > available_budget
                    || planned.len() >= FILESYSTEM_DIRECTORY_PREFETCH_BLOCKS_MAX
                {
                    break 'objects;
                }
                planned_bytes = next_bytes;
                planned.push(spec);
            }
        }
        Ok(planned)
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
        self.refresh_peer_import_metrics();
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
        let sources = replica_set
            .replicas
            .iter()
            .filter(|replica| replica.data_endpoint.starts_with("http://"))
            .map(|replica| PeerPullSource {
                node_id: replica.node_id,
                node_epoch: replica.node_epoch,
                endpoint: replica.data_endpoint.clone(),
            })
            .collect::<Vec<_>>();
        if sources.is_empty() {
            return Err(WorkerError::NoLiveReplica);
        }
        Ok(PeerPullSpec {
            sources,
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
        reply: oneshot::Sender<Result<Option<prost::bytes::Bytes>, PeerImportFailure>>,
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
                // 该块在 Ensure 命令到达 owner 前已经安装。当前调用重新走一次
                // Arena 物化即可；不为这个极小竞态额外复制完整 Block。
                let _ = reply.send(Ok(None));
            } else {
                reject(reply, WorkerError::Conflict);
            }
            return None;
        }
        let charged = self.peer_import_bytes.checked_add(spec.expected_length);
        // mailbox 容量约束“排队的命令数”，不能拿来限制一条已经入队的
        // PullBlocks 计划里有多少 Block。否则 512 MiB / 1 MiB Block 的正常计划
        // 会在第 257 个 Block 被截断，重新退化成每个 FUSE read 一次 RPC。
        // flight 表使用协议计划自身的有界上限，bytes 仍由下面的独立预算约束。
        if self.peer_imports.len() + self.peer_import_failures.len() >= PEER_PULL_PLAN_BLOCKS_MAX
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
        self.refresh_peer_import_metrics();
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
        result: Result<Option<prost::bytes::Bytes>, PeerImportFailure>,
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
        // 一条 PullBlocks 任务可能同时拥有多个 flight。只在 panic/任务取消的
        // 异常路径扫描有界表；正常完成仍直接按 Block+attempt 查找。
        let failed = self
            .peer_imports
            .iter()
            .filter_map(|(block_id, flight)| {
                (flight.task_id == Some(task_id)).then_some((block_id.clone(), flight.attempt))
            })
            .collect::<Vec<_>>();
        for (block_id, attempt) in failed {
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
        bytes: impl Into<prost::bytes::Bytes>,
        checksum: Vec<u8>,
        length: u64,
    ) -> Result<bool, WorkerError> {
        let bytes = bytes.into();
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
            .commit_inline_from_slice_with_verified_digest(&block_id, &bytes, verified_digest)
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
        self.begin_read_scope_unchecked()
    }

    fn begin_read_scope_unchecked(&mut self) -> Result<u64, WorkerError> {
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

    fn begin_data_core_read_scope_reply(
        &mut self,
        cleanup_node: NodeHandle,
        reply: oneshot::Sender<Result<ReadScopeLease, WorkerError>>,
    ) {
        match self.begin_read_scope_unchecked() {
            Ok(scope_id) => {
                if reply
                    .send(Ok(ReadScopeLease::new(cleanup_node, scope_id)))
                    .is_err()
                {
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
        self.current_cache.invalidate_blocks(&block_ids);
        self.metrics
            .set_current_cache_charge(self.current_cache.charged());
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
        // Prepare 是 Meta 对旧物理位置的 cut：先撤销引用这些 Block 的 Current
        // 布局，并把 block 放入 retiring set。无关 key 的有效缓存继续保留；全局
        // generation 已由 invalidate_blocks 推进，事件前启动的迟到 resolve 仍不能
        // 回填。cutoff 前已开始的 scope 可走完，之后的新读不能再借用待回收 Block。
        self.current_cache.invalidate_blocks(&block_ids);
        self.metrics
            .set_current_cache_charge(self.current_cache.charged());
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
        self.refresh_peer_import_metrics();
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
        self.current_cache.invalidate_before(&key, minimum_version);
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

    #[expect(
        clippy::too_many_arguments,
        reason = "write-through cache refill needs the committed layout and the Meta-assigned local replica identity"
    )]
    fn cache_committed_value(
        &mut self,
        token: u64,
        requested_at: Instant,
        key: Vec<u8>,
        block_id: Vec<u8>,
        length: u64,
        checksum: Vec<u8>,
        version: u64,
        revision: u64,
        local_identity: LocalReplicaIdentity,
    ) {
        let now = Instant::now();
        if !self.metadata_watch_connected || now >= self.metadata_lease_until {
            return;
        }
        let ttl_millis = self
            .metadata_lease_until
            .saturating_duration_since(requested_at)
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        if ttl_millis == 0 {
            return;
        }
        let LocalReplicaIdentity {
            node_id,
            node_epoch,
            data_endpoint,
        } = local_identity;

        let (extents, block_replicas) = if length == 0 {
            (Vec::new(), Vec::new())
        } else {
            let location = pb::ReplicaLocation {
                block_id: block_id.clone(),
                node_id,
                node_epoch,
                data_endpoint,
                checksum: checksum.clone(),
                durability: pb::DurabilityPolicy::LocalMemory as i32,
            };
            (
                vec![pb::ExtentRecord {
                    logical: Some(pb::ByteRange { offset: 0, length }),
                    block_id: block_id.clone(),
                    block_offset: 0,
                    digest: checksum.clone(),
                    kind: pb::ExtentKind::Data as i32,
                }],
                vec![pb::BlockReplicaSet {
                    block_id: block_id.clone(),
                    replicas: vec![location.clone()],
                    length,
                    proofs: vec![pb::ReplicaProof {
                        block_id,
                        node_id: location.node_id,
                        node_epoch: location.node_epoch,
                        catalog_revision: revision,
                        checksum: checksum.clone(),
                        durability: location.durability,
                    }],
                }],
            )
        };
        let resolved = pb::ResolveObjectResponse {
            layout: Some(pb::VersionLayout {
                version,
                logical_length: length,
                extents,
                digest: checksum,
                kind: pb::VersionKind::Value as i32,
            }),
            block_replicas,
            current_lease: Some(pb::CurrentLeaseGrant {
                version,
                revision,
                lease_epoch: node_epoch,
                ttl_millis,
                leader_epoch: 0,
            }),
        };
        self.current_cache
            .insert(token, key, &resolved, requested_at, node_epoch, now);
        self.metrics
            .set_current_cache_charge(self.current_cache.charged());
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
        self.refresh_peer_import_metrics();
        if Instant::now() > ticket.expires_at {
            return Err(WorkerError::UnknownTransfer);
        }
        let bytes = self
            .arena
            .read_ticket(ticket.read)
            .map_err(map_arena_error)?;
        validate_grpc_payload_bytes(bytes.len() as u64)?;
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

    fn refresh_peer_import_metrics(&self) {
        self.metrics
            .set_peer_import_state(PeerImportMetricsSnapshot {
                inflight: self.peer_imports.len(),
                reserved_bytes: self.peer_import_bytes,
                failure_fences: self.peer_import_failures.len(),
                download_tickets: self.downloads.len(),
            });
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
        WorkerError::NoLiveReplica => DmsError::new(
            dms_error::NODE_OBJECT_UNAVAILABLE,
            ErrorKind::Unavailable,
            "DMS object version exists, but no live replica can serve the required block",
        ),
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
        ArenaError::StagingNotWritable
        | ArenaError::ReceiptConflict
        | ArenaError::ReservationConflict => WorkerError::Conflict,
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
        ArenaError::UnknownBlock | ArenaError::UnknownReservation => WorkerError::NotFound,
        ArenaError::LengthMismatch | ArenaError::RangeOutOfBounds | ArenaError::RegionOverflow => {
            WorkerError::ArenaInvalidRequest
        }
    }
}

fn common_peer_source(work: &[PeerImportWork]) -> Option<PeerPullSource> {
    let first = work.first()?;
    first.spec.sources.iter().find_map(|candidate| {
        work.iter()
            .all(|item| {
                item.spec.sources.iter().any(|source| {
                    source.node_id == candidate.node_id
                        && source.node_epoch == candidate.node_epoch
                        && source.endpoint == candidate.endpoint
                })
            })
            .then(|| candidate.clone())
    })
}

async fn pull_block_from_peers(
    source_node_id: &str,
    spec: PeerPullSpec,
    rpc_metrics: &dms_metrics::RpcMetrics,
    peer_channels: &Arc<Mutex<HashMap<String, Channel>>>,
    metrics: &NodeMetrics,
) -> Result<PeerBlockResult, WorkerError> {
    let mut last_error = WorkerError::NoLiveReplica;
    for (index, source) in spec.sources.iter().enumerate() {
        match pull_block_from_peer_source(source_node_id, &spec, source, rpc_metrics, peer_channels)
            .await
        {
            Ok(payload) => {
                #[cfg(feature = "reliability-faults")]
                record_source_selection_receipt(&spec.block_id, source)?;
                return Ok(payload);
            }
            Err(error) if index + 1 < spec.sources.len() && can_try_next_peer_source(&error) => {
                // 只记录请求内真正发生的来源切换。Node 的全局存活判断仍由
                // Meta lease/heartbeat 负责，避免瞬时网络错误污染成员状态。
                metrics.record_peer_source_failover();
                last_error = error;
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error)
}

async fn pull_block_from_peer_source(
    source_node_id: &str,
    spec: &PeerPullSpec,
    source: &PeerPullSource,
    rpc_metrics: &dms_metrics::RpcMetrics,
    peer_channels: &Arc<Mutex<HashMap<String, Channel>>>,
) -> Result<PeerBlockResult, WorkerError> {
    if spec.expected_length <= PEER_PULL_SEGMENT_BYTES {
        return pull_peer_segment(
            source_node_id,
            spec,
            source,
            rpc_metrics,
            peer_channels,
            PeerSegmentRequest::WHOLE_BLOCK,
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
    peer_channel_for(&source.endpoint, peer_channels).await?;
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
            let source = source.clone();
            let rpc_metrics = rpc_metrics.clone();
            let peer_channels = peer_channels.clone();
            pulls.spawn(
                async move {
                    let segment = pull_peer_segment(
                        &source_node_id,
                        &spec,
                        &source,
                        &rpc_metrics,
                        &peer_channels,
                        PeerSegmentRequest::segment(start, length),
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
        spec.expected_checksum.clone()
    };
    Ok(PeerBlockResult {
        serving_node_id,
        block_id: spec.block_id.clone(),
        payload: payload.into(),
        checksum,
        length: spec.expected_length,
    })
}

async fn pull_peer_segment(
    source_node_id: &str,
    spec: &PeerPullSpec,
    source: &PeerPullSource,
    rpc_metrics: &dms_metrics::RpcMetrics,
    peer_channels: &Arc<Mutex<HashMap<String, Channel>>>,
    segment: PeerSegmentRequest,
) -> Result<PeerBlockResult, WorkerError> {
    let mut retry_after_cached_channel_failure = true;
    let response = loop {
        let channel = peer_channel_for(&source.endpoint, peer_channels).await?;
        let mut client = peer_client(channel);
        let mut rpc = rpc_metrics.begin_client_call(dms_metrics::RpcCall::PEER_PULL_BLOCK);
        match client
            .pull_block(pb::PeerPullBlockRequest {
                source_node_id: source_node_id.to_string(),
                block_id: spec.block_id.clone(),
                offset: segment.offset,
                length: segment.length,
                expected_length: Some(spec.expected_length),
                expected_checksum: if segment.validate_whole_block {
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
                peer_channels.lock().await.remove(&source.endpoint);
                retry_after_cached_channel_failure = false;
                continue;
            }
            Err(status) => return Err(map_peer_pull_status(status)),
        }
    };
    let expected_payload_length = segment.length.unwrap_or(spec.expected_length);
    if response.block_id != spec.block_id
        || response.length != spec.expected_length
        || response.payload.len() as u64 != expected_payload_length
        || (segment.validate_whole_block
            && !spec.expected_checksum.is_empty()
            && response.checksum != spec.expected_checksum)
        // 整块响应的实际 bytes 留给最终 owner 校验；上面仍验证与权威摘要
        // 一致。分段响应则必须在这里校验，最终 owner 还会检查聚合整块摘要。
        || (!segment.validate_whole_block
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

fn can_try_next_peer_source(error: &WorkerError) -> bool {
    // 这些错误只说明“当前候选位置不能提供所需的不可变 Block”。其它候选
    // 仍可能有效。参数错误和本地资源不足对所有来源都相同，不能盲目换源。
    matches!(
        error,
        WorkerError::NotFound | WorkerError::TransferUnavailable | WorkerError::Conflict
    ) || matches!(
        error,
        WorkerError::Stable(stable)
            if matches!(
                stable.kind(),
                ErrorKind::Unavailable | ErrorKind::NotFound | ErrorKind::DataLoss
            )
    )
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

    #[cfg(feature = "reliability-faults")]
    struct SourceSelectionReceiptPathGuard {
        previous: Option<PathBuf>,
    }

    #[cfg(feature = "reliability-faults")]
    impl SourceSelectionReceiptPathGuard {
        fn set(path: &std::path::Path) -> Self {
            let mut guard = SOURCE_SELECTION_RECEIPT_PATH_FOR_TEST
                .lock()
                .expect("source selection receipt path lock");
            let previous = guard.replace(path.to_path_buf());
            Self { previous }
        }
    }

    #[cfg(feature = "reliability-faults")]
    impl Drop for SourceSelectionReceiptPathGuard {
        fn drop(&mut self) {
            *SOURCE_SELECTION_RECEIPT_PATH_FOR_TEST
                .lock()
                .expect("source selection receipt path lock") = self.previous.take();
        }
    }

    #[test]
    fn session_start_differs_across_injected_incarnations() {
        let mut first = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let mut second = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        first.set_next_session_start_for_test(0x1111_2222_3333_4444);
        second.set_next_session_start_for_test(0x5555_6666_7777_8888);

        let first_session = first.open_session(false);
        let second_session = second.open_session(false);

        assert_ne!(
            first_session, second_session,
            "不同 Node incarnation 的首个 session_id 不应复用"
        );
        assert_ne!(first_session, 0);
        assert_ne!(second_session, 0);
    }

    #[test]
    fn session_ids_are_monotonic_inside_one_incarnation() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        state.set_next_session_start_for_test(0x2222_3333_4444_5555);

        let first = state.open_session(false);
        let second = state.open_session(false);

        assert_eq!(second, first + 1);
    }

    #[test]
    fn session_id_start_normalizes_zero_and_overflow_boundary() {
        assert_eq!(normalize_session_start(0), 1);
        let near_overflow = normalize_session_start(u64::MAX);
        assert_ne!(near_overflow, 0);
        assert!(
            near_overflow <= SESSION_ID_START_SPACE,
            "启动起点必须远离 u64::MAX，避免新进程刚启动就接近溢出"
        );

        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        state.force_next_session_for_test(0);
        assert_eq!(state.open_session(false), 1);

        state.force_next_session_for_test(u64::MAX);
        assert!(matches!(
            state.open_session_with_write_release(false, false),
            Err(WorkerError::ResourceExhausted)
        ));
    }

    #[test]
    fn peer_import_metrics_return_to_baseline_after_state_changes() {
        let registry = dms_metrics::registry();
        let metrics = NodeMetrics::register(&registry).expect("node metrics");
        let mut state = NodeState::with_metrics(
            "node-a".into(),
            None,
            NodeTaskConfig {
                arena_capacity_bytes: 4096,
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
            metrics,
        );
        let session = state.open_session(false);
        let scope = state.begin_read_scope(session).unwrap();
        let spec = PeerPullSpec::single_source(
            "http://127.0.0.1:19999".to_string(),
            b"remote-block".to_vec(),
            digest(b"payload"),
            b"payload".len() as u64,
        );
        let (reply, _rx) = oneshot::channel();

        assert_eq!(state.begin_peer_import(scope, &spec, reply), Some(1));
        assert_peer_import_metrics(&registry, 1, b"payload".len() as u64, 0, 0);

        state.complete_peer_import(
            &spec.block_id,
            1,
            Err(PeerImportFailure::terminal(
                WorkerError::TransferUnavailable,
            )),
        );
        assert_peer_import_metrics(&registry, 0, 0, 1, 0);

        state.finish_read_scope(scope);
        assert_peer_import_metrics(&registry, 0, 0, 0, 0);

        state
            .arena
            .commit_inline(b"local-block".to_vec(), b"local".to_vec())
            .unwrap();
        let (ticket, _) = state.arena.open_read(b"local-block", None).unwrap();
        let target = state.grpc_download_target(session, ticket, 0);
        let transfer_id = match target {
            ReadTarget::Grpc { transfer_id } => transfer_id,
            ReadTarget::Shm(_) => panic!("test uses non-SHM download ticket"),
            ReadTarget::Zero { .. } => panic!("test uses non-zero download ticket"),
        };
        assert_peer_import_metrics(&registry, 0, 0, 0, 1);

        assert_eq!(state.download(transfer_id).unwrap(), b"local".to_vec());
        assert_peer_import_metrics(&registry, 0, 0, 0, 0);
    }

    #[test]
    fn filesystem_inode_reference_lifecycle_updates_state_and_metrics() {
        let registry = dms_metrics::registry();
        let metrics = NodeMetrics::register(&registry).expect("node metrics");
        let mut state = NodeState::with_metrics(
            "node-a".into(),
            None,
            NodeTaskConfig {
                arena_capacity_bytes: 4096,
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
            metrics,
        );
        let inode = 42;

        let generation = state
            .acquire_filesystem_inode_reference_local(inode)
            .expect("first local holder establishes a Meta generation");
        assert_eq!(state.acquire_filesystem_inode_reference_local(inode), None);
        assert_eq!(
            state
                .filesystem_inode_references
                .get(&inode)
                .map(|reference| (reference.count, reference.generation)),
            Some((2, generation))
        );

        assert_eq!(
            state.release_filesystem_inode_reference_local(inode, 1),
            None
        );
        assert_eq!(
            state.release_filesystem_inode_reference_local(inode, 1),
            Some((generation, false))
        );
        assert!(!state.filesystem_inode_references.contains_key(&inode));

        let orphan_generation = state
            .acquire_filesystem_inode_reference_local(inode)
            .expect("new local holder establishes a new generation");
        state.mark_filesystem_inode_orphan(inode);
        assert_eq!(
            state.release_filesystem_inode_reference_local(inode, 1),
            Some((orphan_generation, true)),
            "已知 orphan 的最后一个引用必须要求立即通知 Meta"
        );

        let text = dms_metrics::encode_text(&registry).expect("encode metrics");
        for expected in [
            "dms_node_filesystem_inode_references 0",
            "dms_node_filesystem_inode_reference_transitions_total{transition=\"acquire\"} 2",
            "dms_node_filesystem_inode_reference_transitions_total{transition=\"retain\"} 1",
            "dms_node_filesystem_inode_reference_transitions_total{transition=\"release_partial\"} 1",
            "dms_node_filesystem_inode_reference_transitions_total{transition=\"release_final\"} 2",
        ] {
            assert!(
                text.contains(expected),
                "missing metric: {expected}\n{text}"
            );
        }
    }

    #[test]
    fn filesystem_prefetch_reference_is_promoted_locally_but_not_renewed_while_unused() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let inode = 77;
        state.install_filesystem_inode_reference_reservation(inode, 19, 1_000);

        assert!(state.filesystem_inode_reference_snapshot().is_empty());
        assert_eq!(
            state.acquire_filesystem_inode_reference_local(inode),
            None,
            "live reservation must avoid a per-file Meta acquire"
        );
        assert_eq!(
            state.filesystem_inode_reference_snapshot(),
            vec![(inode, 19)]
        );
        assert_eq!(
            state.release_filesystem_inode_reference_local(inode, 1),
            Some((19, false))
        );

        state.install_filesystem_inode_reference_reservation(inode, 20, 1_000);
        state
            .filesystem_inode_references
            .get_mut(&inode)
            .expect("reservation")
            .lease_until = Instant::now() - Duration::from_millis(1);
        let replacement = state
            .acquire_filesystem_inode_reference_local(inode)
            .expect("expired reservation must reacquire Meta protection");
        assert_ne!(replacement, 20);
    }

    fn assert_peer_import_metrics(
        registry: &dms_metrics::Registry,
        inflight: usize,
        reserved_bytes: u64,
        failure_fences: usize,
        download_tickets: usize,
    ) {
        let text = dms_metrics::encode_text(registry).expect("encode metrics");
        for expected in [
            format!("dms_node_peer_import_inflight {inflight}"),
            format!("dms_node_peer_import_reserved_bytes {reserved_bytes}"),
            format!("dms_node_peer_import_failure_fences {failure_fences}"),
            format!("dms_node_download_tickets {download_tickets}"),
        ] {
            assert!(
                text.contains(&expected),
                "missing metric line `{expected}` in:\n{text}"
            );
        }
    }

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
            kind: pb::ExtentKind::Data as i32,
        }
    }

    fn test_hole_extent(logical_offset: u64, length: u64) -> pb::ExtentRecord {
        pb::ExtentRecord {
            logical: Some(pb::ByteRange {
                offset: logical_offset,
                length,
            }),
            block_id: Vec::new(),
            block_offset: 0,
            digest: Vec::new(),
            kind: pb::ExtentKind::Hole as i32,
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
    fn filesystem_offset_zero_peer_miss_expands_to_full_bounded_layout() {
        let state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let first = test_extent(0, 4, b"first", 0);
        let second = test_extent(4, 4, b"second", 0);
        let layout = pb::VersionLayout {
            version: 7,
            logical_length: 8,
            extents: vec![first.clone(), second],
            digest: b"layout".to_vec(),
            kind: pb::VersionKind::Value as i32,
        };
        let replicas = vec![replica_set(b"first", 4), replica_set(b"second", 4)];
        let requested = vec![
            state
                .describe_missing_block(&replicas, &first)
                .expect("first block has a peer"),
        ];

        let expanded =
            state.expand_filesystem_peer_prefetch(1, &layout, &replicas, (0, 4), requested.clone());
        assert_eq!(expanded.len(), 2, "首个顺序 range 接管完整 Exact Version");

        let random =
            state.expand_filesystem_peer_prefetch(1, &layout, &replicas, (4, 4), requested);
        assert_eq!(random.len(), 1, "非零 offset 仍保持真正按需读取");
    }

    #[test]
    fn directory_prefetch_skips_large_object_instead_of_pulling_its_prefix() {
        let state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let large_length = FILESYSTEM_DIRECTORY_PREFETCH_BYTES_MAX + 1;
        let mut large = resolved_value(
            7,
            large_length,
            vec![test_extent(0, large_length, b"large", 0)],
        );
        large.block_replicas = vec![replica_set(b"large", large_length)];

        let mut small = resolved_value(8, 4, vec![test_extent(0, 4, b"small", 0)]);
        small.block_replicas = vec![replica_set(b"small", 4)];

        let planned = state
            .plan_filesystem_directory_prefetch(1, &[large, small])
            .expect("directory prefetch planning succeeds");

        assert_eq!(planned.len(), 1, "大对象不应占用目录预取窗口");
        assert_eq!(planned[0].block_id, b"small");
    }

    #[test]
    fn filesystem_peer_miss_reuses_installed_bytes_without_second_materialize() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        state
            .arena
            .commit_inline(b"local".to_vec(), b"left".to_vec())
            .expect("install local prefix");
        let resolved = pb::ResolveObjectResponse {
            layout: Some(pb::VersionLayout {
                version: 7,
                logical_length: 8,
                extents: vec![
                    test_extent(0, 4, b"local", 0),
                    test_extent(4, 4, b"remote", 0),
                ],
                digest: b"layout".to_vec(),
                kind: pb::VersionKind::Value as i32,
            }),
            block_replicas: vec![replica_set(b"remote", 4)],
            current_lease: None,
        };

        let outcome = state
            .materialize_resolved_into_for_data_core(
                1,
                &resolved,
                Some((0, 8)),
                false,
                vec![0; 8],
                None,
                true,
            )
            .expect("build peer read plan");
        let DataCoreMaterializeOutcome::NeedsRemoteBlocks {
            remote_parts,
            mut output,
            bytes_read,
            ..
        } = outcome
        else {
            panic!("remote suffix must require peer import")
        };
        assert_eq!(&output[..4], b"left");
        assert_eq!(bytes_read, 8);
        assert_eq!(remote_parts.len(), 1);

        let imported = HashMap::from([(
            b"remote".to_vec(),
            prost::bytes::Bytes::from_static(b"rght"),
        )]);
        assert!(
            NodeHandle::fill_imported_parts(&mut output, &remote_parts, &imported)
                .expect("fill imported suffix")
        );
        assert_eq!(output, b"leftrght");
    }

    #[test]
    fn peer_import_plan_capacity_is_not_coupled_to_mailbox_capacity() {
        let mut state = NodeState::new("node-a".into(), None, 1024, Duration::from_secs(30), None);
        let scope = 1;
        state.active_read_scopes.insert(scope);
        let mut receivers = Vec::new();

        for index in 0..512_u64 {
            let block_id = format!("block-{index}").into_bytes();
            let spec = PeerPullSpec::single_source(
                "http://127.0.0.1:25299".to_string(),
                block_id,
                digest(&[index as u8]),
                1,
            );
            let (reply, receiver) = oneshot::channel();
            receivers.push(receiver);
            assert!(
                state.begin_peer_import(scope, &spec, reply).is_some(),
                "一个有界 PullBlocks 计划不应在 mailbox 的 256 条边界被截断"
            );
        }

        assert_eq!(state.peer_imports.len(), 512);
        assert_eq!(state.peer_import_bytes, 512);
    }

    #[test]
    fn peer_import_completion_is_published_per_block_before_plan_ends() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let scope = 1;
        state.active_read_scopes.insert(scope);

        let first_bytes = b"first".to_vec();
        let second_bytes = b"second".to_vec();
        let first = PeerPullSpec::single_source(
            "http://127.0.0.1:25299".to_string(),
            b"first-block".to_vec(),
            digest(&first_bytes),
            first_bytes.len() as u64,
        );
        let second = PeerPullSpec::single_source(
            "http://127.0.0.1:25299".to_string(),
            b"second-block".to_vec(),
            digest(&second_bytes),
            second_bytes.len() as u64,
        );
        let (first_reply, mut first_receiver) = oneshot::channel();
        let (second_reply, mut second_receiver) = oneshot::channel();
        let first_attempt = state
            .begin_peer_import(scope, &first, first_reply)
            .expect("first flight");
        let _second_attempt = state
            .begin_peer_import(scope, &second, second_reply)
            .expect("second flight");

        state
            .import_peer_block(
                scope,
                first.block_id.clone(),
                first_bytes.clone(),
                digest(&first_bytes),
                first_bytes.len() as u64,
            )
            .expect("install first block");
        state.complete_peer_import(
            &first.block_id,
            first_attempt,
            Ok(Some(prost::bytes::Bytes::from(first_bytes))),
        );

        assert!(matches!(
            first_receiver.try_recv(),
            Ok(Ok(Some(bytes))) if bytes.as_ref() == b"first"
        ));
        assert!(matches!(
            second_receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(
            state.peer_imports.contains_key(&second.block_id),
            "后续 Block 仍由同一计划继续接管"
        );
    }

    #[cfg(feature = "reliability-faults")]
    #[test]
    fn missing_block_keeps_all_http_candidates_and_receipt_records_successful_source() {
        let receipt_path = std::env::temp_dir().join(format!(
            "dms-source-selection-{}-{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let _path_guard = SourceSelectionReceiptPathGuard::set(&receipt_path);

        let state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let extent = test_extent(0, 4, b"remote-block", 0);
        let replicas = vec![pb::BlockReplicaSet {
            block_id: b"remote-block".to_vec(),
            replicas: vec![
                pb::ReplicaLocation {
                    block_id: b"remote-block".to_vec(),
                    node_id: 10,
                    node_epoch: 99,
                    data_endpoint: "uds://dead-replica".to_string(),
                    checksum: b"remote-block".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                },
                pb::ReplicaLocation {
                    block_id: b"remote-block".to_vec(),
                    node_id: 11,
                    node_epoch: 100,
                    data_endpoint: "http://127.0.0.1:25299".to_string(),
                    checksum: b"remote-block".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                },
                pb::ReplicaLocation {
                    block_id: b"remote-block".to_vec(),
                    node_id: 12,
                    node_epoch: 101,
                    data_endpoint: "http://127.0.0.1:25300".to_string(),
                    checksum: b"remote-block".to_vec(),
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                },
            ],
            length: 4,
            proofs: Vec::new(),
        }];

        let spec = state
            .describe_missing_block(&replicas, &extent)
            .expect("select remote replica");
        assert_eq!(spec.sources.len(), 2);
        assert_eq!(spec.sources[0].endpoint, "http://127.0.0.1:25299");
        assert_eq!(spec.sources[1].endpoint, "http://127.0.0.1:25300");

        // 收据只在某个候选实际返回并通过校验后记录，失败的候选不会被写成
        // “已选中来源”。网络级切换由下面的异步完整性测试覆盖。
        record_source_selection_receipt(&spec.block_id, &spec.sources[1])
            .expect("record successful source");

        let receipt = std::fs::read_to_string(&receipt_path).expect("read receipt");
        assert_eq!(
            receipt,
            "block_id=72656d6f74652d626c6f636b node_id=12 node_epoch=101\n"
        );
        assert!(
            !receipt.contains("node_id=10") && !receipt.contains("node_id=11"),
            "receipt must only contain the actually selected replica"
        );
        let _ = std::fs::remove_file(receipt_path);
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
    async fn checkpoint_failure_after_apply_is_retried_internally_without_extra_version() {
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
        let metadata =
            MetadataClient::connect(&endpoint, 7, "http://127.0.0.1:19007".into(), None, false)
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
        let first_outcome = prepared.await(&mut state)
            .expect("checkpoint failure after apply is internally retried with the same operation");
        let retry = state
            .commit_bytes(
                session,
                b"key".to_vec(),
                b"published".to_vec(),
                operation.clone(),
                "any".into(),
            )
            .unwrap();
        let retry_outcome = retry.await(&mut state)
            .expect("explicit same-operation retry returns remembered result");
        assert_eq!(retry_outcome.version, first_outcome.version);
        assert_eq!(retry_outcome.length, b"published".len() as u64);

        let current = metadata
            .resolve(b"key".to_vec(), None)
            .await
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(current.version, first_outcome.version);
        assert_eq!(current.logical_length, b"published".len() as u64);
        let missing_next = metadata
            .resolve(b"key".to_vec(), Some(first_outcome.version + 1))
            .await
            .unwrap_err();
        assert_eq!(missing_next.code(), dms_error::META_CATALOG_NOT_FOUND);
        assert_eq!(
            state.arena.read_bytes(&block_identity("n", &operation)),
            Some(b"published".to_vec()),
            "published metadata still references this block"
        );
        assert!(state.pending_blocks.is_empty());
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
        let metadata =
            MetadataClient::connect(&endpoint, 7, "http://127.0.0.1:19007".into(), None, false)
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
    fn metadata_watch_reset_clears_node_current_cache() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let token = state.current_cache.token().unwrap();
        let now = Instant::now();
        let mut resolved = resolved_value(7, 4, vec![test_extent(0, 4, b"block-a", 0)]);
        resolved.current_lease = Some(pb::CurrentLeaseGrant {
            version: 7,
            revision: 1,
            lease_epoch: 42,
            ttl_millis: 5_000,
            leader_epoch: 0,
        });

        assert!(
            state
                .current_cache
                .insert(token, b"key-a".to_vec(), &resolved, now, 42, now)
        );
        assert!(state.current_cache.charged() > 0);

        state.reset_current_cache_for_watch(false);

        assert_eq!(state.current_cache.charged(), 0);
        assert!(
            state
                .current_cache
                .get(b"key-a", 42, Instant::now())
                .is_none(),
            "Meta watch 断线后不能继续命中 Node 本地 Current cache"
        );
    }

    #[test]
    fn metadata_watch_reset_clears_filesystem_grant_caches() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let now = Instant::now();
        let grant = crate::filesystem::CacheGrant {
            generation: 3,
            lease_millis: 5_000,
        };
        let attrs = crate::filesystem::InodeAttributes {
            inode: 9,
            kind: crate::filesystem::InodeKind::RegularFile,
            mode: 0o644,
            uid: 0,
            gid: 0,
            link_count: 1,
            size: 0,
            atime_unix_nanos: 0,
            mtime_unix_nanos: 0,
            ctime_unix_nanos: 0,
        };
        state.filesystem_bindings.insert(
            crate::filesystem::ResolvedInode {
                granted: crate::filesystem::GrantedInode {
                    inode: crate::filesystem::InodeSnapshot {
                        revision: 2,
                        attributes: attrs.clone(),
                        content: None,
                        reservations: Vec::new(),
                    },
                    grant,
                },
                object: None,
                access_acl: None,
            },
            now,
        );
        state.filesystem_dentries.insert_directory_page(
            None,
            crate::filesystem::DirectoryPage {
                directory: crate::filesystem::ROOT_INODE,
                parent: crate::filesystem::ROOT_INODE,
                grant: crate::filesystem::DirectoryGrant {
                    directory_revision: 2,
                    grant,
                },
                entries: vec![crate::filesystem::DirectoryEntry {
                    dentry: crate::filesystem::DentrySnapshot {
                        parent: crate::filesystem::ROOT_INODE,
                        name: b"a.txt".to_vec(),
                        inode: 9,
                        directory_revision: 2,
                    },
                    attributes: attrs,
                }],
                next_cursor: None,
            },
            now,
        );

        assert!(state.filesystem_bindings.get_authorized(9, now).is_some());
        assert!(
            state
                .filesystem_dentries
                .directory_page(crate::filesystem::ROOT_INODE, None, Some(2), now)
                .is_some()
        );

        state.reset_current_cache_for_watch(false);

        assert!(state.filesystem_bindings.get_authorized(9, now).is_none());
        assert!(
            state
                .filesystem_dentries
                .directory_page(crate::filesystem::ROOT_INODE, None, Some(2), now)
                .is_none()
        );
    }

    #[test]
    fn metadata_watch_reconnect_requires_replayed_events_to_be_acked_before_new_grant() {
        let mut state = NodeState::new("n".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let (sender, mut receiver) = mpsc::channel(1);
        state.attach_session(session, sender).unwrap();
        state.metadata_lease_until = Instant::now() + Duration::from_secs(5);
        state.reset_current_cache_for_watch(true);
        assert!(state.heartbeat(session, None, true).unwrap() > 0);
        state
            .register_cache_interest(session, b"model/a".to_vec())
            .unwrap();

        let barrier = state
            .broadcast_invalidation(b"model/a".to_vec(), 2)
            .expect("registered cache interest must create invalidation barrier");
        let event = receiver.try_recv().unwrap();
        let sequence = match event {
            NodeEvent::InvalidateCurrent { event_sequence, .. } => event_sequence,
        };

        // 断线期间不发新 grant；重连后也必须等 Client ACK 已 replay 的事件。
        state.sessions.get_mut(&session).unwrap().cache_until = None;
        state.reset_current_cache_for_watch(false);
        assert_eq!(state.heartbeat(session, None, true).unwrap(), 0);
        state.reset_current_cache_for_watch(true);
        assert_eq!(
            state.heartbeat(session, None, true).unwrap(),
            0,
            "last_ack 未追上 replay 游标前，重连也不能恢复 cache grant"
        );

        let (waiter, mut completion) = oneshot::channel();
        state.attach_barrier_waiter(barrier, waiter);
        state.acknowledge(session, sequence).unwrap();
        completion.try_recv().unwrap().unwrap();
        assert!(
            state.heartbeat(session, None, true).unwrap() > 0,
            "Client ACK replay 后才恢复新的 cache grant"
        );
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
                false,
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
        // 每个写只向另一个 Node 投递失效；写入 Node 已在本机提交路径更新
        // Current，不再接收自己的 Watch 事件。先确认双方提交均到达 Meta，
        // 再检查 owner 未被等待远端 ACK 的 Future 堵住。
        let mut events = Vec::new();
        for (_, stream) in &mut streams {
            let event = tokio::time::timeout(Duration::from_secs(2), stream.message())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            events.push(event);
        }
        for (index, (node, session)) in nodes.iter().enumerate() {
            tokio::time::timeout(Duration::from_millis(250), node.heartbeat(*session, None))
                .await
                .expect("outbound commit must not block heartbeat")
                .unwrap();
            let event = &events[index];
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

    #[test]
    fn node_boundary_rejects_oversized_key_and_batch_before_write() {
        assert!(matches!(
            validate_user_key(&vec![b'k'; USER_KEY_BYTES_MAX + 1]),
            Err(WorkerError::InvalidArgument(_))
        ));

        let receipt = HostReceipt {
            transfer_id: 1,
            length: 1,
            digest: vec![0; 32],
            allocation_id: 1,
            release_token: Vec::new(),
        };
        let entries = (0..=BATCH_MAX_ITEMS)
            .map(|index| {
                (
                    format!("k-{index}").into_bytes(),
                    index as u64 + 1,
                    receipt.clone(),
                )
            })
            .collect::<Vec<_>>();

        assert!(matches!(
            validate_batch_write(&entries),
            Err(WorkerError::ResourceExhausted)
        ));
    }

    #[test]
    fn tcp_staging_rejects_payload_above_grpc_safe_budget_before_arena_allocation() {
        let mut state = NodeState::new(
            "node-a".to_string(),
            None,
            GRPC_PAYLOAD_SAFE_BYTES + 4096,
            Duration::from_secs(30),
            None,
        );
        let session = state.open_session(false);

        let error = state
            .allocate_staging(session, GRPC_PAYLOAD_SAFE_BYTES + 1)
            .expect_err("TCP/gRPC staging is bounded before Arena allocation");

        assert!(matches!(error, WorkerError::ResourceExhausted));
        assert_eq!(state.arena.stats().allocated_bytes, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shm_staging_can_exceed_grpc_safe_budget() {
        let path =
            std::env::temp_dir().join(format!("dms-g003-shm-budget-{}.sock", std::process::id()));
        let broker = SharedFdBroker::bind(path.clone()).expect("bind fd broker");
        let mut state = NodeState::new(
            "node-a".to_string(),
            None,
            GRPC_PAYLOAD_SAFE_BYTES + 4096,
            Duration::from_secs(30),
            Some(broker),
        );
        let session = state.open_session(true);

        let allocation = state
            .allocate_staging(session, GRPC_PAYLOAD_SAFE_BYTES + 1)
            .expect("SHM staging uses mmap bytes, not one gRPC message");

        assert!(matches!(allocation.target, HostAllocationTarget::Shm(_)));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn mailbox_correlates_each_result_with_its_request() {
        // 如果 oneshot 关联错请求，这两个并发语义上的返回 ID 就可能串线。
        let node = NodeHandle::spawn_without_metadata("node-a".to_string());
        let first = node.open_session(false).await.expect("first session");
        let second = node.open_session(false).await.expect("second session");
        assert_ne!(first, 0);
        assert_ne!(second, 0);
        assert_eq!(second, first + 1);
        node.heartbeat(second, None)
            .await
            .expect("heartbeat must use the actual second session id");
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
    fn get_resolved_inline_materializes_sparse_hole_as_zero_bytes() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block-1".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(
            9,
            6,
            vec![
                test_hole_extent(0, 2),
                test_extent(2, 3, b"block-1", 0),
                test_hole_extent(5, 1),
            ],
        );

        let ticket = match state
            .get_resolved(session, &resolved, None, 6)
            .expect("get resolved")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("block is local"),
        };

        assert_eq!(
            ticket.inline_value.as_deref(),
            Some(b"\0\0abc\0".as_slice())
        );
        assert!(ticket.segments.is_empty());
        assert!(state.downloads.is_empty());
    }

    #[test]
    fn get_resolved_non_inline_returns_zero_segments_without_download_tickets() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        state
            .arena
            .commit_inline(b"block-1".to_vec(), b"abc".to_vec())
            .expect("commit block");
        let resolved = resolved_value(
            10,
            5,
            vec![test_hole_extent(0, 2), test_extent(2, 3, b"block-1", 0)],
        );

        let ticket = match state
            .get_resolved(session, &resolved, None, 0)
            .expect("get resolved")
        {
            GetOutcome::Ready(ticket) => ticket,
            GetOutcome::NeedsRemoteBlocks(_) => panic!("block is local"),
        };

        assert_eq!(ticket.inline_value, None);
        assert_eq!(ticket.segments.len(), 2);
        assert!(matches!(
            ticket.segments[0].target,
            ReadTarget::Zero { length: 2 }
        ));
        assert!(matches!(ticket.segments[1].target, ReadTarget::Grpc { .. }));
        assert_eq!(state.downloads.len(), 1);
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
    fn resolved_version_without_live_replica_is_unavailable_not_not_found() {
        let mut state = NodeState::new("node-a".into(), None, 4096, Duration::from_secs(30), None);
        let session = state.open_session(false);
        let mut resolved = resolved_value(18, 4, vec![test_extent(0, 4, b"lost-block", 0)]);
        resolved.block_replicas = vec![pb::BlockReplicaSet {
            block_id: b"lost-block".to_vec(),
            replicas: Vec::new(),
            length: 4,
            proofs: Vec::new(),
        }];

        let error = match state.get_resolved_for_request(
            session,
            u64::MAX,
            6,
            &resolved,
            None,
            false,
            64,
        ) {
            Ok(_) => panic!("version exists but no live replica can serve it"),
            Err(error) => error,
        };
        assert!(
            !is_not_found_result(&error),
            "R5 must not be translated into found=false"
        );
        let public = worker_error_to_dms(error);
        assert_eq!(public.code(), dms_error::NODE_OBJECT_UNAVAILABLE);
        assert_eq!(public.kind(), ErrorKind::Unavailable);
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
            ReadTarget::Zero { .. } => panic!("expected data ticket"),
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
            ReadTarget::Zero { .. } => panic!("expected data ticket"),
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
        let session = state
            .open_session_with_write_release(true, true)
            .expect("open SHM session with release support");
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

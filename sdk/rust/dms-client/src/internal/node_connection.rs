//! SDK 到一个 dms-node 的连接，以及这条连接上的长生命周期 Session stream。
//!
//! `connect_channel` 是唯一判断 `unix://` 与 `http(s)://` 的位置。连接建立后，
//! `set/get` 都只看到同一种 Tonic `Channel`，因此业务逻辑不需要 transport 分支。

// Session 后台 Task 与前台读操作共享释放水位，不共享跨请求 value 缓存。
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    io::Read,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

// `as pb` 给生成代码起短别名，后续 `pb::SetRequest` 明确表示 wire DTO。
use dms_protocol::{MAX_INLINE_READ_BYTES, v1 as pb};
use hyper_util::rt::TokioIo;
use pb::worker_service_client::WorkerServiceClient;
use tokio::net::UnixStream;
// 有界 mpsc 用作 Session 上行消息队列：多个生产者，一个 stream 消费者。
use tokio::sync::mpsc;
// Tonic streaming 接口需要 `Stream`，ReceiverStream 把 mpsc::Receiver 适配成 Stream。
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    Response, Status,
    transport::{Channel, Endpoint},
};
// service_fn 把一个 async closure 变成 Tower Service，供 Tonic 自定义如何建连接。
use tower::service_fn;

use super::grpc_clients::worker_client;
use crate::types::system_time_from_unix_millis;
use crate::{
    ByteRange, ClientTlsOptions, DeleteResult, DmsError, DurabilityPolicy, GetIntoResult,
    GetOptions, GetResult, HashDeleteOptions, HashEntriesResult, HashField, HashGetOptions,
    HashMultiGetResult, HashRangeWriteOptions, HashRangeWriteResult, HashReadVersion,
    HashScanOptions, HashScanResult, HashSetResult, HashValue, HashVersion, HashWriteMode,
    HashWriteOptions, Key, KeyVersion, MSetResult, ObjectInfo, ObjectVersion, OperationId,
    RangeWriteOptions, ReadVersion, ScanCursor, ScanOptions, ScanResult, SetOptions, SetResult,
    WriteCondition,
};

use super::transfer_engine::{PayloadBuffer, ReadPayload, TransferEngine};
use crate::client::ResolvedClientOptions;
use crate::metrics::{ClientMetrics, NodeSessionEvent};
use dms_transport::{GrpcConfig, SecurityManager, TlsConfig, status_to_dms_error_with};

pub(crate) struct NodeConnection {
    // generated typed Client 是控制面代理；并发调用 clone 此轻量句柄。
    worker: WorkerServiceClient<dms_tracing::TracedChannel>,
    // Payload target 的 provider 选择只发生在 TransferEngine 内。
    transfer: TransferEngine,
    // 每次 generated gRPC Client 的真实网络尝试都通过它记录；业务操作指标另算。
    rpc_metrics: Option<dms_metrics::RpcMetrics>,
    // Node 分配的会话身份，后续所有 Worker 请求都携带它。
    session_id: u64,
    inline_threshold_bytes: usize,
    view_releases: Arc<ViewReleaseTracker>,
    read_finishes: Arc<ReadRequestFinishTracker>,
    next_read_request_id: AtomicU64,
    write_releases: WriteLeaseReleaser,
}

#[derive(Default)]
pub(crate) struct ViewReleaseTracker {
    inner: Mutex<ViewReleaseState>,
}

#[derive(Default)]
struct ViewReleaseState {
    released_through: u64,
    // 已释放但尚不能推进水位的闭区间 start..=end。长持有 View 不能让
    // 后面十万次普通 GET 留下十万个独立序号；连续完成合并为一个区间。
    pending: BTreeMap<u64, u64>,
}

impl ViewReleaseTracker {
    fn mark_released(&self, view_epoch: u64) {
        if view_epoch == 0 {
            return;
        }
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        if view_epoch <= state.released_through {
            return;
        }
        let (mut start, mut end) = (view_epoch, view_epoch);
        if let Some((&left_start, &left_end)) = state.pending.range(..=view_epoch).next_back() {
            if view_epoch <= left_end {
                return; // 同次多段响应或批读 guard 的重复释放。
            }
            if left_end.checked_add(1) == Some(view_epoch) {
                start = left_start;
                state.pending.remove(&left_start);
            }
        }
        if let Some((&right_start, &right_end)) = state.pending.range(view_epoch..).next()
            && view_epoch.checked_add(1) == Some(right_start)
        {
            end = right_end;
            state.pending.remove(&right_start);
        }
        state.pending.insert(start, end);
        if let Some((&first, &last)) = state.pending.first_key_value()
            && state.released_through.checked_add(1) == Some(first)
        {
            state.released_through = last;
            state.pending.pop_first();
        }
    }

    fn released_view_through(&self) -> Option<u64> {
        let Ok(state) = self.inner.lock() else {
            return None;
        };
        (state.released_through > 0).then_some(state.released_through)
    }

    /// 响应到达后、校验或 mmap 前接管借用；失败也不能遗失已知的 epoch。
    /// 同次读的多个 Extent 可共享一个 epoch，重复释放由 tracker 去重。
    fn protect<'a>(
        self: &Arc<Self>,
        segments: impl Iterator<Item = &'a pb::ReadSegment>,
        read: Option<ReadRequestGuard>,
    ) -> ReadProtection {
        ReadProtection {
            releases: Arc::clone(self),
            read,
            epochs: segments
                .filter_map(|segment| match segment.target.as_ref()?.target.as_ref()? {
                    pb::payload_target::Target::Shm(target) => target.view_epoch,
                    _ => None,
                })
                .collect(),
        }
    }
}

/// 一次响应里的共享读借用。普通 GET 复制完即 drop；显式 View 持有至用户 drop。
/// 只报告收到且已不再使用的 epoch；丢失响应造成的序号空洞不能猜成已释放。
struct ReadProtection {
    releases: Arc<ViewReleaseTracker>,
    read: Option<ReadRequestGuard>,
    epochs: Vec<u64>,
}

impl Drop for ReadProtection {
    fn drop(&mut self) {
        for &epoch in &self.epochs {
            self.releases.mark_released(epoch);
        }
        // 普通读在复制结束后、显式 View 在用户 drop 后，才推进“本次读请求完成”
        // 水位；这样 Node 可以回收只服务于该请求的下载票据/临时读保护。
        let _ = self.read.take();
    }
}

#[derive(Default)]
struct ReadRequestFinishTracker {
    inner: Mutex<ReadRequestFinishState>,
}

#[derive(Default)]
struct ReadRequestFinishState {
    finished_through: u64,
    pending: BTreeMap<u64, u64>,
}

impl ReadRequestFinishTracker {
    fn mark_finished(&self, read_request_id: u64) {
        if read_request_id == 0 {
            return;
        }
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        if read_request_id <= state.finished_through {
            return;
        }
        let (mut start, mut end) = (read_request_id, read_request_id);
        if let Some((&left_start, &left_end)) = state.pending.range(..=read_request_id).next_back()
        {
            if read_request_id <= left_end {
                return;
            }
            if left_end.checked_add(1) == Some(read_request_id) {
                start = left_start;
                state.pending.remove(&left_start);
            }
        }
        if let Some((&right_start, &right_end)) = state.pending.range(read_request_id..).next()
            && read_request_id.checked_add(1) == Some(right_start)
        {
            end = right_end;
            state.pending.remove(&right_start);
        }
        state.pending.insert(start, end);
        if let Some((&first, &last)) = state.pending.first_key_value()
            && state.finished_through.checked_add(1) == Some(first)
        {
            state.finished_through = last;
            state.pending.pop_first();
        }
    }

    fn finished_read_request_through(&self) -> Option<u64> {
        let Ok(state) = self.inner.lock() else {
            return None;
        };
        (state.finished_through > 0).then_some(state.finished_through)
    }
}

struct ReadRequestGuard {
    finishes: Arc<ReadRequestFinishTracker>,
    read_request_id: u64,
}

impl ReadRequestGuard {
    fn id(&self) -> u64 {
        self.read_request_id
    }
}

impl Drop for ReadRequestGuard {
    fn drop(&mut self) {
        self.finishes.mark_finished(self.read_request_id);
    }
}

/// SDK 内部持有的显式共享写 buffer。
///
/// 公开的 `SharedWriteBuffer` 只委托给它，不暴露 protobuf descriptor 或 mmap 对象。
pub(crate) struct SharedWriteInner {
    pub(crate) key: Key,
    staging_id: u64,
    operation_id: OperationId,
    options: SetOptions,
    default_durability: DurabilityPolicy,
    buffer: PayloadBuffer,
    write_release: Option<WriteLeaseGuard>,
}

impl SharedWriteInner {
    pub(crate) fn as_mut_slice(&mut self) -> Result<&mut [u8], DmsError> {
        self.buffer.as_mut_slice()
    }

    pub(crate) fn len(&self) -> Result<usize, DmsError> {
        Ok(self.buffer.as_slice()?.len())
    }
}

/// SDK 内部持有的 exact-version 共享读 view。
pub(crate) struct SharedViewInner {
    version: ObjectVersion,
    buffer: PayloadBuffer,
    _protection: ReadProtection,
}

/// SDK 内部持有的 fixed-version native reader。
///
/// 它持有 ReadProtection 直到 EOF、错误或用户提前 Drop。gRPC 分段按需下载当前
/// segment；SHM 分段按需映射后直接复制到调用方 read buffer。
pub(crate) struct ValueReaderInner {
    version: ObjectVersion,
    len: u64,
    segments: Vec<pb::ReadSegment>,
    current_segment: usize,
    offset_in_segment: usize,
    consumed: u64,
    loaded: Option<LoadedReadSegment>,
    protection: Option<ReadProtection>,
}

struct LoadedReadSegment {
    index: usize,
    payload: ReadPayload,
}

impl ValueReaderInner {
    pub(crate) fn version(&self) -> ObjectVersion {
        self.version
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    fn release_protection(&mut self) {
        let _ = self.protection.take();
    }
}

/// 长连接 Task 自己消费的两个运行参数，避免把 SDK 的整份配置带进后台任务。
struct SessionTaskOptions {
    heartbeat_interval: Duration,
    channel_capacity: usize,
    write_lease_release_supported: bool,
}

#[derive(Clone)]
struct WriteLeaseReleaser {
    session_id: u64,
    supported: bool,
    backlog: Arc<Mutex<WriteLeaseReleaseBacklog>>,
}

#[derive(Default)]
struct WriteLeaseReleaseBacklog {
    pending: VecDeque<pb::ReleasedWriteAllocation>,
    capacity: usize,
    quarantined: u64,
}

struct WriteLeaseGuard {
    releaser: WriteLeaseReleaser,
    release: Option<pb::ReleasedWriteAllocation>,
}

impl WriteLeaseReleaser {
    #[cfg(test)]
    fn disabled(session_id: u64) -> Self {
        Self {
            session_id,
            supported: false,
            backlog: Arc::new(Mutex::new(WriteLeaseReleaseBacklog::with_capacity(0))),
        }
    }

    fn new(session_id: u64, supported: bool, backlog_capacity: usize) -> Self {
        Self {
            session_id,
            supported,
            backlog: Arc::new(Mutex::new(WriteLeaseReleaseBacklog::with_capacity(
                backlog_capacity,
            ))),
        }
    }

    fn release(&self, release: Option<pb::ReleasedWriteAllocation>) {
        let Some(release) = release else {
            return;
        };
        self.release_many(vec![release]);
    }

    fn release_many(&self, releases: Vec<pb::ReleasedWriteAllocation>) {
        if releases.is_empty() || !self.supported {
            return;
        }
        // Drop 路径不能 await，也不能使用无界队列。这里先进入有界 backlog，
        // 再由 Session 心跳或 Client 正常关闭时的 final Heartbeat 负责冲刷。
        //
        // 关键不变量：这些 token 对应“已暴露给 SDK 的可写 lease”。如果通知
        // 丢失，Node 不能仅靠 TTL 把同一块内存安全复用给别的写。因此 SDK
        // 宁可把超出 backlog 的 token 标为 quarantined（后续泄漏/告警），也
        // 不声称它们已经安全归还。
        self.enqueue_backlog(releases);
    }

    fn take_pending_batch(&self, max_items: usize) -> Vec<pb::ReleasedWriteAllocation> {
        let Ok(mut state) = self.backlog.lock() else {
            return Vec::new();
        };
        let batch_len = state.pending.len().min(max_items);
        state.pending.drain(..batch_len).collect()
    }

    fn take_all_pending(&self) -> Vec<pb::ReleasedWriteAllocation> {
        self.take_pending_batch(usize::MAX)
    }

    fn requeue_front(&self, batch: Vec<pb::ReleasedWriteAllocation>) {
        let Ok(mut state) = self.backlog.lock() else {
            return;
        };
        for release in batch.into_iter().rev() {
            state.pending.push_front(release);
        }
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.backlog
            .lock()
            .map(|state| state.pending.len())
            .unwrap_or(0)
    }

    #[cfg(test)]
    fn quarantined_count(&self) -> u64 {
        self.backlog
            .lock()
            .map(|state| state.quarantined)
            .unwrap_or(0)
    }

    fn enqueue_backlog(&self, releases: Vec<pb::ReleasedWriteAllocation>) {
        let Ok(mut state) = self.backlog.lock() else {
            return;
        };
        for release in releases {
            if state.pending.len() < state.capacity {
                state.pending.push_back(release);
            } else {
                state.quarantined = state.quarantined.saturating_add(1);
                log::error!(
                    "DMS SHM write lease release backlog is full; allocation quarantined \
                     instead of relying on TTL reuse (session_id={}, quarantined={})",
                    self.session_id,
                    state.quarantined
                );
            }
        }
    }
}

impl WriteLeaseReleaseBacklog {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            ..Self::default()
        }
    }
}

impl WriteLeaseGuard {
    fn new(
        releaser: WriteLeaseReleaser,
        release: Option<pb::ReleasedWriteAllocation>,
    ) -> Option<Self> {
        release.map(|release| Self {
            releaser,
            release: Some(release),
        })
    }

    fn consume(&mut self) {
        self.release = None;
    }
}

impl Drop for WriteLeaseGuard {
    fn drop(&mut self) {
        self.releaser.release(self.release.take());
    }
}

impl SharedViewInner {
    pub(crate) fn version(&self) -> ObjectVersion {
        self.version
    }

    pub(crate) fn as_slice(&self) -> Result<&[u8], DmsError> {
        self.buffer.as_slice()
    }

    pub(crate) fn len(&self) -> Result<usize, DmsError> {
        Ok(self.buffer.as_slice()?.len())
    }
}

impl NodeConnection {
    pub(crate) async fn connect(
        options: &ResolvedClientOptions,
        metrics: Option<ClientMetrics>,
        rpc_metrics: Option<dms_metrics::RpcMetrics>,
    ) -> Result<Self, DmsError> {
        // `..Default::default()` 是 struct update 语法：只覆盖 request_timeout，
        // 其余字段从默认配置复制。
        let config = GrpcConfig {
            request_timeout: options.timeout,
            ..GrpcConfig::default()
        };
        let security = SecurityManager::new(match options.tls {
            ClientTlsOptions::Disabled => TlsConfig::Disabled,
        })
        .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?;
        // `&config`/`&security` 是只读借用；`.await?` 等待连接并向上传播错误。
        let channel = connect_channel(&options.endpoint, &config, &security).await?;
        // generated Client 是薄句柄，内部持有 clone 后的 Channel。
        let mut worker = worker_client(channel.clone(), &config);
        // 先通过 unary RPC 建立逻辑 Session，协商当前支持的能力。
        let session_result = observe_rpc(
            rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_OPEN_SESSION,
            worker.open_session(pb::OpenSessionRequest {
                min_version: 1,
                max_version: 1,
                shared_memory: options.shared_memory,
                zero_copy_read: options.shared_memory,
                zero_copy_write: options.shared_memory,
                supports_write_lease_release: true,
            }),
        )
        .await;
        let session = session_result.map_err(map_status)?.into_inner();
        let session_id = session.session_id;
        let fd_broker_path = session.shm.map(|shm| shm.fd_broker_path);
        let view_releases = Arc::new(ViewReleaseTracker::default());
        let read_finishes = Arc::new(ReadRequestFinishTracker::default());
        // 长连接负责 Session 存活与 View 释放水位；不申请 SDK value 缓存租约。
        let write_releases = start_session_task(
            worker,
            session_id,
            Arc::clone(&view_releases),
            Arc::clone(&read_finishes),
            SessionTaskOptions {
                heartbeat_interval: options.heartbeat_interval,
                channel_capacity: options.session_channel_capacity,
                write_lease_release_supported: session.write_lease_release_supported,
            },
            metrics.clone(),
            rpc_metrics.as_ref(),
        )
        .await?;
        // 返回时 generated Client 本身可以丢弃；Channel 留在 NodeConnection 中复用。
        Ok(Self {
            worker: worker_client(channel.clone(), &config),
            transfer: TransferEngine::new(
                channel,
                session_id,
                fd_broker_path,
                &config,
                metrics.clone(),
                rpc_metrics.clone(),
            ),
            rpc_metrics,
            session_id,
            inline_threshold_bytes: options.inline_threshold_bytes,
            view_releases,
            read_finishes,
            next_read_request_id: AtomicU64::new(1),
            write_releases,
        })
    }

    fn begin_read_request(&self) -> ReadRequestGuard {
        let read_request_id = self.next_read_request_id.fetch_add(1, Ordering::Relaxed);
        ReadRequestGuard {
            finishes: Arc::clone(&self.read_finishes),
            read_request_id,
        }
    }

    pub(crate) async fn set(
        &self,
        key: &Key,
        value: &[u8],
        options: SetOptions,
        default_durability: DurabilityPolicy,
        operation_id: OperationId,
    ) -> Result<SetResult, DmsError> {
        let mut worker = self.worker.clone();
        // 小对象直接内联到一个控制 RPC；阈值来自解析后的 SDK 配置。
        if value.len() <= self.inline_threshold_bytes {
            let response = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::WORKER_SET_INLINE,
                worker.set_inline(pb::SetInlineRequest {
                    session_id: self.session_id,
                    key: Some(pb::Key {
                        value: key.as_bytes().to_vec(),
                    }),
                    value: value.to_vec(),
                    operation_id: Some(encode_operation_id(operation_id)),
                    condition: encode_condition(options.condition),
                    durability: encode_durability(options.durability.unwrap_or(default_durability)),
                }),
            )
            .await
            .map_err(map_status)?
            .into_inner();
            return Ok(SetResult {
                version: ObjectVersion(response.version),
                len: response.length,
            });
        }
        // 第一步只申请 staging：Node 返回未来上传 payload 使用的 transfer_id。
        let allocation = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_ALLOCATE_STAGING,
            worker.allocate_staging(pb::AllocateStagingRequest {
                session_id: self.session_id,
                // usize 是本进程地址宽度；wire 协议固定使用 u64，所以这里显式转换。
                length: value.len() as u64,
                purpose: "full-value".to_string(),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        // protobuf message 字段可能缺失，因此 generated Rust 类型是 Option<T>。
        // `ok_or_else(...)?` 把协议缺字段转换为 DmsError。
        let target = allocation.target.ok_or_else(|| {
            DmsError::client_protocol_violation("missing payload target".to_string())
        })?;
        let upload_release = write_release_from_target(&target);

        // 第二步由统一传输边界选择 provider，业务层不识别 gRPC/SHM/RDMA/UB。
        let receipt = match self.transfer.upload(self.session_id, target, value).await {
            Ok(receipt) => receipt,
            Err(error) => {
                self.write_releases.release(upload_release);
                // Allocation succeeded but payload transfer did not. Release is
                // idempotent, and failure to release must not hide the original
                // transfer error. If release delivery is backpressured, the
                // session backlog keeps or quarantines the token rather than
                // pretending that Node can safely reuse it.
                let _ = observe_rpc(
                    self.rpc_metrics.as_ref(),
                    dms_metrics::RpcCall::WORKER_DELETE_STAGING,
                    worker.delete_staging(pb::DeleteStagingRequest {
                        session_id: self.session_id,
                        staging_id: allocation.staging_id,
                    }),
                )
                .await;
                return Err(error);
            }
        };
        let commit_release = write_release_from_receipt(&receipt);

        // 第三步 Set 只提交 key、staging_id 和上传回执；Node 原子地发布新版本。
        let response = match observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_SET,
            worker.set(pb::SetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                value: Some(pb::StagedValue {
                    staging_id: allocation.staging_id,
                    receipt: Some(receipt),
                }),
                operation_id: Some(encode_operation_id(operation_id)),
                condition: encode_condition(options.condition),
                durability: encode_durability(options.durability.unwrap_or(default_durability)),
            }),
        )
        .await
        {
            Ok(response) => response.into_inner(),
            Err(status) => {
                // Set may fail before or after consuming staging. Delete is a
                // possible unknown outcome, so never delete bytes here. Only
                // return the SHM write lease; Node decides whether the staging
                // was already consumed or should later be reclaimed.
                self.write_releases.release(commit_release);
                return Err(map_status(status));
            }
        };
        // 把 protobuf DTO 转换为 SDK 的公开领域类型，避免用户依赖生成代码。
        Ok(SetResult {
            version: ObjectVersion(response.version),
            len: response.length,
        })
    }

    pub(crate) async fn set_from<R: Read>(
        &self,
        key: &Key,
        mut src: R,
        length: u64,
        options: SetOptions,
        default_durability: DurabilityPolicy,
        operation_id: OperationId,
    ) -> Result<SetResult, DmsError> {
        let len = usize::try_from(length).map_err(|_| {
            DmsError::client_invalid_argument("SET_FROM length is too large for this platform")
        })?;
        let mut worker = self.worker.clone();
        if len <= self.inline_threshold_bytes {
            let value = read_exact_source(&mut src, len)?;
            let response = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::WORKER_SET_INLINE,
                worker.set_inline(pb::SetInlineRequest {
                    session_id: self.session_id,
                    key: Some(pb::Key {
                        value: key.as_bytes().to_vec(),
                    }),
                    value,
                    operation_id: Some(encode_operation_id(operation_id)),
                    condition: encode_condition(options.condition),
                    durability: encode_durability(options.durability.unwrap_or(default_durability)),
                }),
            )
            .await
            .map_err(map_status)?
            .into_inner();
            return Ok(SetResult {
                version: ObjectVersion(response.version),
                len: response.length,
            });
        }

        let allocation = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_ALLOCATE_STAGING,
            worker.allocate_staging(pb::AllocateStagingRequest {
                session_id: self.session_id,
                length,
                purpose: "reader-value".to_string(),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        let target = allocation.target.ok_or_else(|| {
            DmsError::client_protocol_violation("missing payload target".to_string())
        })?;
        let upload_release = write_release_from_target(&target);
        let receipt = match self.upload_reader_to_target(target, &mut src, len).await {
            Ok(receipt) => receipt,
            Err(error) => {
                self.write_releases.release(upload_release);
                let _ = observe_rpc(
                    self.rpc_metrics.as_ref(),
                    dms_metrics::RpcCall::WORKER_DELETE_STAGING,
                    worker.delete_staging(pb::DeleteStagingRequest {
                        session_id: self.session_id,
                        staging_id: allocation.staging_id,
                    }),
                )
                .await;
                return Err(error);
            }
        };
        let commit_release = write_release_from_receipt(&receipt);
        let response = match observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_SET,
            worker.set(pb::SetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                value: Some(pb::StagedValue {
                    staging_id: allocation.staging_id,
                    receipt: Some(receipt),
                }),
                operation_id: Some(encode_operation_id(operation_id)),
                condition: encode_condition(options.condition),
                durability: encode_durability(options.durability.unwrap_or(default_durability)),
            }),
        )
        .await
        {
            Ok(response) => response.into_inner(),
            Err(status) => {
                self.write_releases.release(commit_release);
                return Err(map_status(status));
            }
        };
        Ok(SetResult {
            version: ObjectVersion(response.version),
            len: response.length,
        })
    }

    pub(crate) async fn allocate_write(
        &self,
        key: Key,
        len: usize,
        options: SetOptions,
        default_durability: DurabilityPolicy,
        operation_id: OperationId,
    ) -> Result<SharedWriteInner, DmsError> {
        if len == 0 {
            return Err(DmsError::client_invalid_argument(
                "allocate_write length must be positive".to_string(),
            ));
        }
        let mut worker = self.worker.clone();
        let allocation = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_ALLOCATE_STAGING,
            worker.allocate_staging(pb::AllocateStagingRequest {
                session_id: self.session_id,
                length: len as u64,
                purpose: "shared-write".to_string(),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        let target = allocation.target.ok_or_else(|| {
            DmsError::client_protocol_violation("missing payload target".to_string())
        })?;
        let release = write_release_from_target(&target);
        let buffer = match self.transfer.map_target(self.session_id, target).await {
            Ok(buffer) => buffer,
            Err(error) => {
                self.write_releases.release(release);
                let _ = observe_rpc(
                    self.rpc_metrics.as_ref(),
                    dms_metrics::RpcCall::WORKER_DELETE_STAGING,
                    worker.delete_staging(pb::DeleteStagingRequest {
                        session_id: self.session_id,
                        staging_id: allocation.staging_id,
                    }),
                )
                .await;
                return Err(error);
            }
        };
        Ok(SharedWriteInner {
            key,
            staging_id: allocation.staging_id,
            operation_id,
            options,
            default_durability,
            buffer,
            write_release: WriteLeaseGuard::new(self.write_releases.clone(), release),
        })
    }

    pub(crate) async fn commit_shared(
        &self,
        mut write: SharedWriteInner,
    ) -> Result<SetResult, DmsError> {
        let mut worker = self.worker.clone();
        let receipt = write.buffer.receipt()?;
        let response = match observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_SET,
            worker.set(pb::SetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: write.key.as_bytes().to_vec(),
                }),
                value: Some(pb::StagedValue {
                    staging_id: write.staging_id,
                    receipt: Some(receipt),
                }),
                operation_id: Some(encode_operation_id(write.operation_id)),
                condition: encode_condition(write.options.condition),
                durability: encode_durability(
                    write.options.durability.unwrap_or(write.default_durability),
                ),
            }),
        )
        .await
        {
            Ok(response) => response.into_inner(),
            Err(status) => {
                return Err(map_status(status));
            }
        };
        if let Some(release) = &mut write.write_release {
            release.consume();
        }
        Ok(SetResult {
            version: ObjectVersion(response.version),
            len: response.length,
        })
    }

    pub(crate) async fn get_view(
        &self,
        key: &Key,
        options: GetOptions,
    ) -> Result<Option<SharedViewInner>, DmsError> {
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_GET,
            worker.get(pb::GetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                exact_version: match options.version {
                    ReadVersion::Current => None,
                    ReadVersion::Exact(version) => Some(version.0),
                },
                range: options.range.map(encode_range),
                max_inline_bytes: 0,
                read_request_id,
                clamp_range: options.clamp_range,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        self.decode_view_response(response, options.range, options.clamp_range, Some(read))
            .await
    }

    // 与普通响应解码一样：在任何可能失败的步骤前接管本次读取保护。
    async fn decode_view_response(
        &self,
        response: pb::GetResponse,
        range: Option<ByteRange>,
        clamp_range: bool,
        read: Option<ReadRequestGuard>,
    ) -> Result<Option<SharedViewInner>, DmsError> {
        // 即使不是单段、协议校验失败或 FD 获取失败，也要释放整份响应的借用。
        let protection = self.view_releases.protect(response.segments.iter(), read);
        if !response.found {
            reject_inline_on_miss(&response)?;
            return Ok(None);
        }
        reject_inline_value(&response)?;
        if response.segments.len() != 1 {
            return Err(DmsError::node_transfer_unsupported(
                "shared-memory view currently supports exactly one read segment".to_string(),
            ));
        }
        let expected_length = selected_read_length(response.logical_length, range, clamp_range)?;
        validate_read_segments(&response.segments, expected_length)?;
        let segment = response.segments.into_iter().next().ok_or_else(|| {
            DmsError::client_protocol_violation("found response has no segment".to_string())
        })?;
        let target = segment.target.ok_or_else(|| {
            DmsError::client_protocol_violation("read segment has no target".to_string())
        })?;
        let buffer = self.transfer.map_target(self.session_id, target).await?;
        if buffer.as_slice()?.len() as u64 != expected_length {
            return Err(DmsError::client_protocol_violation(
                "shared view length does not match requested range",
            ));
        }
        Ok(Some(SharedViewInner {
            version: ObjectVersion(response.version),
            buffer,
            _protection: protection,
        }))
    }

    pub(crate) async fn mset(
        &self,
        entries: &[crate::KvEntry],
        options: crate::MSetOptions,
        default_durability: DurabilityPolicy,
        operation_id: OperationId,
    ) -> Result<MSetResult, DmsError> {
        let mut worker = self.worker.clone();
        let mut staged_entries = Vec::with_capacity(entries.len());
        let mut allocated_staging = Vec::with_capacity(entries.len());
        for entry in entries {
            let (staging_id, staged) = match self
                .stage_value(&mut worker, &entry.value, "mset-entry")
                .await
            {
                Ok(staged) => staged,
                Err(error) => {
                    self.release_staged_key_value_leases(&staged_entries);
                    self.delete_staging_many(&mut worker, &allocated_staging)
                        .await;
                    return Err(error);
                }
            };
            allocated_staging.push(staging_id);
            staged_entries.push(pb::StagedKeyValue {
                key: Some(pb::Key {
                    value: entry.key.as_bytes().to_vec(),
                }),
                value: Some(staged),
            });
        }
        let commit_releases = staged_key_value_releases(&staged_entries);
        let response = match observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_MSET,
            worker.m_set(pb::MSetRequest {
                session_id: self.session_id,
                entries: staged_entries,
                operation_id: Some(encode_operation_id(operation_id)),
                durability: encode_durability(options.durability.unwrap_or(default_durability)),
            }),
        )
        .await
        {
            Ok(response) => response.into_inner(),
            Err(status) => {
                self.write_releases.release_many(commit_releases);
                return Err(map_status(status));
            }
        };
        let versions = response
            .versions
            .into_iter()
            .map(|item| {
                let key = item.key.ok_or_else(|| {
                    DmsError::client_protocol_violation("MSet version missing key".to_string())
                })?;
                Ok(KeyVersion {
                    key: Key::new(key.value)
                        .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?,
                    version: ObjectVersion(item.version),
                })
            })
            .collect::<Result<Vec<_>, DmsError>>()?;
        Ok(MSetResult { versions })
    }

    pub(crate) async fn get(
        &self,
        key: &Key,
        options: GetOptions,
    ) -> Result<Option<GetResult>, DmsError> {
        // 小 TCP 值可以内联；其它情况由相同响应带回 payload 位置。
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_GET,
            worker.get(pb::GetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                // Rust enum 强制调用方明确 Current 与 Exact；wire 用 Optional u64 表达。
                exact_version: match options.version {
                    ReadVersion::Current => None,
                    ReadVersion::Exact(version) => Some(version.0),
                },
                range: options.range.map(encode_range),
                // GET 只允许“小对象读响应”随控制 RPC 返回；较大 value 继续走
                // payload transfer。这里还受协议常量二次裁剪，避免配置误把过大
                // bytes 塞进控制响应。
                max_inline_bytes: self.inline_read_budget() as u64,
                read_request_id,
                clamp_range: options.clamp_range,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        self.decode_read_response(
            response,
            options.range,
            options.clamp_range,
            self.inline_read_budget(),
            Some(read),
        )
        .await
    }

    pub(crate) async fn get_into(
        &self,
        key: &Key,
        dst: &mut [u8],
        options: GetOptions,
    ) -> Result<Option<GetIntoResult>, DmsError> {
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_GET,
            worker.get(pb::GetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                exact_version: match options.version {
                    ReadVersion::Current => None,
                    ReadVersion::Exact(version) => Some(version.0),
                },
                range: options.range.map(encode_range),
                max_inline_bytes: self.inline_read_budget() as u64,
                read_request_id,
                clamp_range: options.clamp_range,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        self.decode_read_into_response(
            response,
            options.range,
            options.clamp_range,
            self.inline_read_budget(),
            Some(read),
            dst,
        )
        .await
    }

    pub(crate) async fn get_reader(
        &self,
        key: &Key,
        options: GetOptions,
    ) -> Result<Option<ValueReaderInner>, DmsError> {
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_GET,
            worker.get(pb::GetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                exact_version: match options.version {
                    ReadVersion::Current => None,
                    ReadVersion::Exact(version) => Some(version.0),
                },
                range: options.range.map(encode_range),
                max_inline_bytes: 0,
                read_request_id,
                clamp_range: options.clamp_range,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        self.decode_reader_response(response, options.range, options.clamp_range, Some(read))
            .await
    }

    pub(crate) async fn stat(&self, key: &Key) -> Result<Option<ObjectInfo>, DmsError> {
        let mut worker = self.worker.clone();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_STAT,
            worker.stat(pb::StatRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        if !response.found {
            return Ok(None);
        }
        let info = response.info.ok_or_else(|| {
            DmsError::client_protocol_violation("stat hit has no ObjectInfo".to_string())
        })?;
        decode_object_info(info).map(Some)
    }

    pub(crate) async fn scan(
        &self,
        prefix: &[u8],
        options: ScanOptions,
    ) -> Result<ScanResult, DmsError> {
        let mut worker = self.worker.clone();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_SCAN,
            worker.scan(pb::ScanRequest {
                session_id: self.session_id,
                prefix: prefix.to_vec(),
                options: Some(encode_scan_options(options)),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        Ok(ScanResult {
            items: response
                .items
                .into_iter()
                .map(decode_object_info)
                .collect::<Result<_, _>>()?,
            next_cursor: (!response.next_cursor.is_empty()).then_some(response.next_cursor),
        })
    }

    pub(crate) async fn mget(&self, keys: &[Key]) -> Result<Vec<Option<GetResult>>, DmsError> {
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_MGET,
            worker.m_get(pb::MGetRequest {
                session_id: self.session_id,
                keys: keys
                    .iter()
                    .map(|key| pb::Key {
                        value: key.as_bytes().to_vec(),
                    })
                    .collect(),
                read_request_id,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        self.decode_mget_response(response, Some(read)).await
    }

    async fn decode_mget_response(
        &self,
        response: pb::MGetResponse,
        read: Option<ReadRequestGuard>,
    ) -> Result<Vec<Option<GetResult>>, DmsError> {
        // 前面的 item 失败时，后面的 item 尚未解码也已由 Node 发放借用。
        // 整批保护直到处理结束，不能只在逐 item 成功后才记录释放。
        let _batch_protection = self.view_releases.protect(
            response.items.iter().flat_map(|item| item.segments.iter()),
            read,
        );
        let mut results = Vec::with_capacity(response.items.len());
        for item in response.items {
            results.push(self.decode_get_response(item).await?);
        }
        Ok(results)
    }

    fn inline_read_budget(&self) -> usize {
        self.inline_threshold_bytes
            .min(usize::try_from(MAX_INLINE_READ_BYTES).unwrap_or(usize::MAX))
    }

    pub(crate) async fn set_range(
        &self,
        key: &Key,
        offset: u64,
        data: &[u8],
        options: RangeWriteOptions,
        default_durability: DurabilityPolicy,
        operation_id: OperationId,
    ) -> Result<SetResult, DmsError> {
        let mut worker = self.worker.clone();
        let (staging_id, staged) = self.stage_value(&mut worker, data, "range-patch").await?;
        let commit_release = write_release_from_staged_value(&staged);
        let response = match observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_SET_RANGE,
            worker.set_range(pb::SetRangeRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                offset,
                value: Some(staged),
                operation_id: Some(encode_operation_id(operation_id)),
                expected_version: options.expected_version.map(|version| version.0),
                durability: encode_durability(options.durability.unwrap_or(default_durability)),
            }),
        )
        .await
        {
            Ok(response) => response.into_inner(),
            Err(status) => {
                let _ = staging_id;
                self.write_releases.release(commit_release);
                return Err(map_status(status));
            }
        };
        Ok(SetResult {
            version: ObjectVersion(response.version),
            len: response.length,
        })
    }

    pub(crate) async fn hset(
        &self,
        key: &Key,
        entries: &[(HashField, Vec<u8>)],
        options: HashWriteOptions,
        default_durability: DurabilityPolicy,
        operation_id: OperationId,
    ) -> Result<HashSetResult, DmsError> {
        let mut worker = self.worker.clone();
        let mut staged_entries = Vec::with_capacity(entries.len());
        let mut staging_ids = Vec::with_capacity(entries.len());
        for (field, bytes) in entries {
            let (staging_id, staged) =
                match self.stage_value(&mut worker, bytes, "hash-field").await {
                    Ok(staged) => staged,
                    Err(error) => {
                        self.release_staged_hash_entry_leases(&staged_entries);
                        self.delete_staging_many(&mut worker, &staging_ids).await;
                        return Err(error);
                    }
                };
            staging_ids.push(staging_id);
            staged_entries.push(pb::StagedHashEntry {
                field: Some(pb::HashField {
                    value: field.as_bytes().to_vec(),
                }),
                value: Some(staged),
            });
        }
        let commit_releases = staged_hash_entry_releases(&staged_entries);
        let response = match observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_HSET,
            worker.h_set(pb::HSetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                entries: staged_entries,
                operation_id: Some(encode_operation_id(operation_id)),
                mode: match options.mode {
                    HashWriteMode::Merge => "merge",
                    HashWriteMode::Replace => "replace",
                }
                .to_string(),
                expected_version: options.expected_version.map(|version| version.0),
                durability: encode_durability(options.durability.unwrap_or(default_durability)),
            }),
        )
        .await
        {
            Ok(response) => response.into_inner(),
            Err(status) => {
                // HSET 可能在 Node 端已消费 staging 但响应丢失；此处只归还
                // SHM 写 lease，不主动删除可能已发布的 bytes。
                self.write_releases.release_many(commit_releases);
                return Err(map_status(status));
            }
        };
        Ok(HashSetResult {
            version: HashVersion(response.hash_version),
            field_count: response.field_count,
        })
    }

    pub(crate) async fn hget(
        &self,
        key: &Key,
        field: &HashField,
        options: HashGetOptions,
    ) -> Result<Option<HashValue>, DmsError> {
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_HGET,
            worker.h_get(pb::HGetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                field: Some(pb::HashField {
                    value: field.as_bytes().to_vec(),
                }),
                exact_hash_version: encode_hash_read_version(options.version),
                read_request_id,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        drop(read);
        decode_hash_get(response)
    }

    pub(crate) async fn hmget(
        &self,
        key: &Key,
        fields: &[HashField],
        options: HashGetOptions,
    ) -> Result<HashMultiGetResult, DmsError> {
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_HMGET,
            worker.hm_get(pb::HmGetRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                fields: fields
                    .iter()
                    .map(|field| pb::HashField {
                        value: field.as_bytes().to_vec(),
                    })
                    .collect(),
                exact_hash_version: encode_hash_read_version(options.version),
                read_request_id,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        drop(read);
        Ok(HashMultiGetResult {
            version: response.hash_version.map(HashVersion),
            values: response
                .values
                .into_iter()
                .map(decode_hash_get)
                .collect::<Result<_, _>>()?,
        })
    }

    pub(crate) async fn hget_all(
        &self,
        key: &Key,
        options: HashGetOptions,
    ) -> Result<HashEntriesResult, DmsError> {
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_HGET_ALL,
            worker.h_get_all(pb::HGetAllRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                exact_hash_version: encode_hash_read_version(options.version),
                read_request_id,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        drop(read);
        Ok(HashEntriesResult {
            version: response.hash_version.map(HashVersion),
            entries: response
                .entries
                .into_iter()
                .map(decode_hash_value)
                .collect::<Result<_, _>>()?,
        })
    }

    pub(crate) async fn hdelete(
        &self,
        key: &Key,
        fields: &[HashField],
        options: HashDeleteOptions,
        default_durability: DurabilityPolicy,
        operation_id: OperationId,
    ) -> Result<HashSetResult, DmsError> {
        let mut worker = self.worker.clone();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_HDELETE,
            worker.h_delete(pb::HDeleteRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                fields: fields
                    .iter()
                    .map(|field| pb::HashField {
                        value: field.as_bytes().to_vec(),
                    })
                    .collect(),
                operation_id: Some(encode_operation_id(operation_id)),
                expected_version: options.expected_version.map(|version| version.0),
                durability: encode_durability(options.durability.unwrap_or(default_durability)),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        Ok(HashSetResult {
            version: HashVersion(response.hash_version),
            field_count: response.field_count,
        })
    }

    pub(crate) async fn hscan(
        &self,
        key: &Key,
        cursor: ScanCursor,
        options: HashScanOptions,
    ) -> Result<HashScanResult, DmsError> {
        let mut worker = self.worker.clone();
        let read = self.begin_read_request();
        let read_request_id = read.id();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_HSCAN,
            worker.h_scan(pb::HScanRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                cursor: cursor.0,
                limit: u32::try_from(options.limit)
                    .map_err(|_| DmsError::client_invalid_argument("HSCAN limit is too large"))?,
                exact_hash_version: encode_hash_read_version(options.version),
                read_request_id,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        drop(read);
        Ok(HashScanResult {
            version: response.hash_version.map(HashVersion),
            next_cursor: ScanCursor(response.next_cursor),
            entries: response
                .entries
                .into_iter()
                .map(decode_hash_value)
                .collect::<Result<_, _>>()?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn hwrite_at(
        &self,
        key: &Key,
        field: &HashField,
        offset: u64,
        data: &[u8],
        options: HashRangeWriteOptions,
        default_durability: DurabilityPolicy,
        operation_id: OperationId,
    ) -> Result<HashRangeWriteResult, DmsError> {
        let mut worker = self.worker.clone();
        let (staging_id, staged) = self
            .stage_value(&mut worker, data, "hash-range-patch")
            .await?;
        let commit_release = write_release_from_staged_value(&staged);
        let response = match observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_HWRITE_AT,
            worker.h_write_at(pb::HWriteAtRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                field: Some(pb::HashField {
                    value: field.as_bytes().to_vec(),
                }),
                offset,
                value: Some(staged),
                operation_id: Some(encode_operation_id(operation_id)),
                expected_hash_version: options.expected_version.map(|version| version.0),
                durability: encode_durability(options.durability.unwrap_or(default_durability)),
            }),
        )
        .await
        {
            Ok(response) => response.into_inner(),
            Err(status) => {
                let _ = staging_id;
                self.write_releases.release(commit_release);
                return Err(map_status(status));
            }
        };
        Ok(HashRangeWriteResult {
            hash_version: HashVersion(response.hash_version),
            value_version: ObjectVersion(response.value_version),
            len: response.length,
            field_count: response.field_count,
        })
    }

    pub(crate) async fn del(
        &self,
        key: &Key,
        operation_id: OperationId,
    ) -> Result<DeleteResult, DmsError> {
        let mut worker = self.worker.clone();
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_DELETE,
            worker.delete(pb::DeleteRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                operation_id: Some(encode_operation_id(operation_id)),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        Ok(DeleteResult {
            deleted: response.deleted,
            version: ObjectVersion(response.version),
        })
    }

    pub(crate) async fn close(&self) -> Result<(), DmsError> {
        let released_write_allocations = self.write_releases.take_all_pending();
        let Some(request) = self.shutdown_heartbeat_request(released_write_allocations.clone())
        else {
            return Ok(());
        };

        let mut worker = self.worker.clone();
        let result = tokio::time::timeout(
            Duration::from_millis(250),
            observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::WORKER_HEARTBEAT,
                worker.heartbeat(request),
            ),
        )
        .await;
        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(status)) => {
                self.write_releases
                    .requeue_front(released_write_allocations);
                Err(map_status(status))
            }
            Err(_) => {
                self.write_releases
                    .requeue_front(released_write_allocations);
                Err(DmsError::client_connection_unavailable(
                    "timed out while flushing DMS session state during shutdown",
                ))
            }
        }
    }

    fn shutdown_heartbeat_request(
        &self,
        released_write_allocations: Vec<pb::ReleasedWriteAllocation>,
    ) -> Option<pb::HeartbeatRequest> {
        let request = pb::HeartbeatRequest {
            session_id: self.session_id,
            released_view_through: self.view_releases.released_view_through(),
            released_write_allocations,
            finished_read_request_through: self.read_finishes.finished_read_request_through(),
        };
        (request.released_view_through.is_some()
            || !request.released_write_allocations.is_empty()
            || request.finished_read_request_through.is_some())
        .then_some(request)
    }

    async fn stage_value(
        &self,
        worker: &mut WorkerServiceClient<dms_tracing::TracedChannel>,
        value: &[u8],
        purpose: &str,
    ) -> Result<(u64, pb::StagedValue), DmsError> {
        let allocation = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_ALLOCATE_STAGING,
            worker.allocate_staging(pb::AllocateStagingRequest {
                session_id: self.session_id,
                length: value.len() as u64,
                purpose: purpose.to_string(),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        let target = allocation.target.ok_or_else(|| {
            DmsError::client_protocol_violation("missing payload target".to_string())
        })?;
        let upload_release = write_release_from_target(&target);
        let receipt = match self.transfer.upload(self.session_id, target, value).await {
            Ok(receipt) => receipt,
            Err(error) => {
                self.write_releases.release(upload_release);
                let _ = observe_rpc(
                    self.rpc_metrics.as_ref(),
                    dms_metrics::RpcCall::WORKER_DELETE_STAGING,
                    worker.delete_staging(pb::DeleteStagingRequest {
                        session_id: self.session_id,
                        staging_id: allocation.staging_id,
                    }),
                )
                .await;
                return Err(error);
            }
        };
        Ok((
            allocation.staging_id,
            pb::StagedValue {
                staging_id: allocation.staging_id,
                receipt: Some(receipt),
            },
        ))
    }

    /// Best-effort cleanup for a compound request that failed before the Node
    /// atomically consumed every staged value. Cleanup is intentionally kept in
    /// the connection boundary so individual SDK methods cannot forget it.
    async fn delete_staging_many(
        &self,
        worker: &mut WorkerServiceClient<dms_tracing::TracedChannel>,
        staging_ids: &[u64],
    ) {
        for &staging_id in staging_ids {
            let _ = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::WORKER_DELETE_STAGING,
                worker.delete_staging(pb::DeleteStagingRequest {
                    session_id: self.session_id,
                    staging_id,
                }),
            )
            .await;
        }
    }

    fn release_staged_key_value_leases(&self, entries: &[pb::StagedKeyValue]) {
        self.write_releases
            .release_many(staged_key_value_releases(entries));
    }

    fn release_staged_hash_entry_leases(&self, entries: &[pb::StagedHashEntry]) {
        self.write_releases
            .release_many(staged_hash_entry_releases(entries));
    }

    async fn decode_get_response(
        &self,
        response: pb::GetResponse,
    ) -> Result<Option<GetResult>, DmsError> {
        self.decode_read_response(response, None, false, 0, None)
            .await
    }

    async fn decode_read_response(
        &self,
        response: pb::GetResponse,
        range: Option<ByteRange>,
        clamp_range: bool,
        max_inline_bytes: usize,
        read: Option<ReadRequestGuard>,
    ) -> Result<Option<GetResult>, DmsError> {
        // 普通 GET 最终交付 owned Vec。复制、验证、映射失败及取消都通过 Drop
        // 归还已收到的共享读 epoch；这里不保留跨请求 Buffer/value。
        let _protection = self.view_releases.protect(response.segments.iter(), read);
        if !response.found {
            reject_inline_on_miss(&response)?;
            return Ok(None);
        }
        let expected_length = selected_read_length(response.logical_length, range, clamp_range)?;
        if let Some(bytes) = take_inline_value(
            response.inline_value,
            &response.segments,
            expected_length,
            max_inline_bytes,
        )? {
            return Ok(Some(GetResult {
                version: ObjectVersion(response.version),
                bytes,
            }));
        }
        let bytes = self
            .download_segments(response.segments, expected_length)
            .await?;
        Ok(Some(GetResult {
            version: ObjectVersion(response.version),
            bytes,
        }))
    }

    async fn download_segments(
        &self,
        mut segments: Vec<pb::ReadSegment>,
        expected_length: u64,
    ) -> Result<Vec<u8>, DmsError> {
        segments.sort_by_key(|segment| segment.logical_offset);
        validate_read_segments(&segments, expected_length)?;
        let mut bytes = Vec::new();
        let mut expected_offset = 0_u64;
        for segment in segments {
            if segment.logical_offset != expected_offset {
                return Err(DmsError::client_protocol_violation(
                    "read segments contain a gap or overlap".to_string(),
                ));
            }
            let target = segment.target.ok_or_else(|| {
                DmsError::client_protocol_violation("read segment has no target".to_string())
            })?;
            let declared_length = read_target_length(&target)?;
            let part = self.transfer.download(self.session_id, target).await?;
            if part.len() as u64 != declared_length {
                return Err(DmsError::client_protocol_violation(
                    "read payload length differs from descriptor",
                ));
            }
            expected_offset = expected_offset
                .checked_add(part.len() as u64)
                .ok_or_else(|| {
                    DmsError::client_protocol_violation("read length overflow".to_string())
                })?;
            // download 已经返回独立拥有的 Vec；首段可直接成为结果，避免
            // 再分配同样大小的 Vec 并复制一次。后续 Extent 仍按顺序追加，
            // 不借用 SHM 页，也不改变普通 GET 的所有权或版本校验。
            append_downloaded_part(&mut bytes, part);
        }
        Ok(bytes)
    }

    async fn decode_read_into_response(
        &self,
        response: pb::GetResponse,
        range: Option<ByteRange>,
        clamp_range: bool,
        max_inline_bytes: usize,
        read: Option<ReadRequestGuard>,
        dst: &mut [u8],
    ) -> Result<Option<GetIntoResult>, DmsError> {
        let _protection = self.view_releases.protect(response.segments.iter(), read);
        if !response.found {
            reject_inline_on_miss(&response)?;
            return Ok(None);
        }
        let expected_length = selected_read_length(response.logical_length, range, clamp_range)?;
        let len = usize::try_from(expected_length).map_err(|_| {
            DmsError::client_protocol_violation("read response length is too large")
        })?;
        if dst.len() < len {
            return Err(DmsError::client_invalid_argument(format!(
                "destination buffer length {} is smaller than selected DMS read length {}",
                dst.len(),
                expected_length
            )));
        }
        if let Some(bytes) = take_inline_value(
            response.inline_value,
            &response.segments,
            expected_length,
            max_inline_bytes,
        )? {
            dst[..len].copy_from_slice(&bytes);
            return Ok(Some(GetIntoResult {
                version: ObjectVersion(response.version),
                len: expected_length,
            }));
        }
        self.copy_segments_into(response.segments, expected_length, &mut dst[..len])
            .await?;
        Ok(Some(GetIntoResult {
            version: ObjectVersion(response.version),
            len: expected_length,
        }))
    }

    async fn decode_reader_response(
        &self,
        mut response: pb::GetResponse,
        range: Option<ByteRange>,
        clamp_range: bool,
        read: Option<ReadRequestGuard>,
    ) -> Result<Option<ValueReaderInner>, DmsError> {
        let protection = self.view_releases.protect(response.segments.iter(), read);
        if !response.found {
            reject_inline_on_miss(&response)?;
            return Ok(None);
        }
        reject_inline_value(&response)?;
        let expected_length = selected_read_length(response.logical_length, range, clamp_range)?;
        response
            .segments
            .sort_by_key(|segment| segment.logical_offset);
        validate_read_segments(&response.segments, expected_length)?;
        Ok(Some(ValueReaderInner {
            version: ObjectVersion(response.version),
            len: expected_length,
            segments: response.segments,
            current_segment: 0,
            offset_in_segment: 0,
            consumed: 0,
            loaded: None,
            protection: Some(protection),
        }))
    }

    async fn copy_segments_into(
        &self,
        mut segments: Vec<pb::ReadSegment>,
        expected_length: u64,
        dst: &mut [u8],
    ) -> Result<(), DmsError> {
        segments.sort_by_key(|segment| segment.logical_offset);
        validate_read_segments(&segments, expected_length)?;
        let mut output_offset = 0usize;
        for segment in segments {
            let target = segment.target.ok_or_else(|| {
                DmsError::client_protocol_violation("read segment has no target".to_string())
            })?;
            let declared_length = read_target_length(&target)?;
            let payload = self.transfer.read_payload(self.session_id, target).await?;
            if payload.len()? as u64 != declared_length {
                return Err(DmsError::client_protocol_violation(
                    "read payload length differs from descriptor",
                ));
            }
            let part_len = usize::try_from(declared_length).map_err(|_| {
                DmsError::client_protocol_violation("read segment length is too large")
            })?;
            let end = output_offset.checked_add(part_len).ok_or_else(|| {
                DmsError::client_protocol_violation("read output offset overflow".to_string())
            })?;
            payload.copy_range_into(0, &mut dst[output_offset..end])?;
            output_offset = end;
        }
        Ok(())
    }

    pub(crate) async fn read_value_reader(
        &self,
        reader: &mut ValueReaderInner,
        dst: &mut [u8],
    ) -> Result<usize, DmsError> {
        if dst.is_empty() {
            return Ok(0);
        }
        if reader.consumed == reader.len {
            reader.release_protection();
            return Ok(0);
        }
        let result = self.read_value_reader_once(reader, dst).await;
        match &result {
            Ok(_) if reader.consumed == reader.len => reader.release_protection(),
            Err(_) => reader.release_protection(),
            _ => {}
        }
        result
    }

    async fn read_value_reader_once(
        &self,
        reader: &mut ValueReaderInner,
        dst: &mut [u8],
    ) -> Result<usize, DmsError> {
        let mut written = 0usize;
        while written < dst.len()
            && reader.consumed < reader.len
            && reader.current_segment < reader.segments.len()
        {
            if reader
                .loaded
                .as_ref()
                .is_none_or(|loaded| loaded.index != reader.current_segment)
            {
                let target = reader.segments[reader.current_segment]
                    .target
                    .clone()
                    .ok_or_else(|| {
                        DmsError::client_protocol_violation(
                            "read segment has no target".to_string(),
                        )
                    })?;
                let declared_length = read_target_length(&target)?;
                let payload = self.transfer.read_payload(self.session_id, target).await?;
                if payload.len()? as u64 != declared_length {
                    return Err(DmsError::client_protocol_violation(
                        "read payload length differs from descriptor",
                    ));
                }
                reader.loaded = Some(LoadedReadSegment {
                    index: reader.current_segment,
                    payload,
                });
            }
            let loaded = reader.loaded.as_ref().ok_or_else(|| {
                DmsError::client_protocol_violation("reader segment was not loaded")
            })?;
            let available = loaded
                .payload
                .len()?
                .checked_sub(reader.offset_in_segment)
                .ok_or_else(|| {
                    DmsError::client_protocol_violation("reader offset exceeds segment length")
                })?;
            if available == 0 {
                reader.current_segment += 1;
                reader.offset_in_segment = 0;
                reader.loaded = None;
                continue;
            }
            let to_copy = available.min(dst.len() - written);
            loaded.payload.copy_range_into(
                reader.offset_in_segment,
                &mut dst[written..written + to_copy],
            )?;
            reader.offset_in_segment += to_copy;
            reader.consumed = reader.consumed.checked_add(to_copy as u64).ok_or_else(|| {
                DmsError::client_protocol_violation("reader consumed length overflow")
            })?;
            written += to_copy;
            if reader.offset_in_segment == loaded.payload.len()? {
                reader.current_segment += 1;
                reader.offset_in_segment = 0;
                reader.loaded = None;
            }
        }
        Ok(written)
    }

    async fn upload_reader_to_target<R: Read>(
        &self,
        target: pb::PayloadTarget,
        src: &mut R,
        len: usize,
    ) -> Result<pb::TransferReceipt, DmsError> {
        match target.target.as_ref() {
            Some(pb::payload_target::Target::Grpc(grpc)) if grpc.length != len as u64 => {
                return Err(DmsError::client_protocol_violation(
                    "payload target length does not match source length".to_string(),
                ));
            }
            Some(pb::payload_target::Target::Shm(shm)) if shm.length != len as u64 => {
                return Err(DmsError::client_protocol_violation(
                    "SHM target length does not match source length".to_string(),
                ));
            }
            _ => {}
        }
        match target.target.as_ref() {
            Some(pb::payload_target::Target::Grpc(_)) => {
                let value = read_exact_source(src, len)?;
                self.transfer.upload(self.session_id, target, &value).await
            }
            Some(pb::payload_target::Target::Shm(_)) => {
                let mut buffer = self.transfer.map_target(self.session_id, target).await?;
                read_exact_into(src, buffer.as_mut_slice()?)?;
                buffer.receipt()
            }
            Some(pb::payload_target::Target::Rdma(_)) => Err(DmsError::node_transfer_unsupported(
                "RDMA provider is not enabled by this SDK build".to_string(),
            )),
            Some(pb::payload_target::Target::Ub(_)) => Err(DmsError::node_transfer_unsupported(
                "UB provider is not enabled by this SDK build".to_string(),
            )),
            None => Err(DmsError::client_protocol_violation(
                "empty payload target".to_string(),
            )),
        }
    }
}

fn append_downloaded_part(bytes: &mut Vec<u8>, part: Vec<u8>) {
    // 所有尺寸都可以移动所有权；4MiB 文件块与小范围读不应落回重复复制。
    // 后续片段仍按需扩容，不能为省复制而返回共享页或预留无界内存。
    if bytes.is_empty() {
        *bytes = part;
    } else {
        bytes.extend_from_slice(&part);
    }
}

fn read_exact_source(src: &mut impl Read, len: usize) -> Result<Vec<u8>, DmsError> {
    let mut value = vec![0; len];
    read_exact_into(src, &mut value)?;
    Ok(value)
}

fn read_exact_into(src: &mut impl Read, dst: &mut [u8]) -> Result<(), DmsError> {
    src.read_exact(dst).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            DmsError::client_invalid_argument(
                "SET_FROM source ended before the declared length was read",
            )
        } else {
            DmsError::client_invalid_argument(format!("failed to read SET_FROM source: {error}"))
        }
    })
}

fn selected_read_length(
    logical_length: u64,
    range: Option<ByteRange>,
    clamp_range: bool,
) -> Result<u64, DmsError> {
    match range {
        Some(range) if range.offset.checked_add(range.len).is_none() => {
            Err(DmsError::client_protocol_violation(
                "read response logical length does not contain requested range",
            ))
        }
        Some(range) if clamp_range && range.offset >= logical_length => Ok(0),
        Some(range) if clamp_range => Ok(range.len.min(logical_length - range.offset)),
        Some(range)
            if range
                .offset
                .checked_add(range.len)
                .is_none_or(|end| end > logical_length) =>
        {
            Err(DmsError::client_protocol_violation(
                "read response logical length does not contain requested range",
            ))
        }
        Some(range) => Ok(range.len),
        None => Ok(logical_length),
    }
}

fn take_inline_value(
    inline_value: Option<Vec<u8>>,
    segments: &[pb::ReadSegment],
    expected_length: u64,
    max_inline_bytes: usize,
) -> Result<Option<Vec<u8>>, DmsError> {
    let Some(bytes) = inline_value else {
        return Ok(None);
    };
    if max_inline_bytes == 0 {
        return Err(DmsError::client_protocol_violation(
            "inline read response is not allowed for this request".to_string(),
        ));
    }
    if !segments.is_empty() {
        return Err(DmsError::client_protocol_violation(
            "inline read response must not also carry read segments".to_string(),
        ));
    }
    if bytes.len() as u64 != expected_length {
        return Err(DmsError::client_protocol_violation(
            "inline read response length does not match requested length".to_string(),
        ));
    }
    if bytes.len() > max_inline_bytes {
        return Err(DmsError::client_protocol_violation(
            "inline read response exceeds requested budget".to_string(),
        ));
    }
    Ok(Some(bytes))
}

fn reject_inline_value(response: &pb::GetResponse) -> Result<(), DmsError> {
    if response.inline_value.is_some() {
        return Err(DmsError::client_protocol_violation(
            "inline read response is not allowed for this request".to_string(),
        ));
    }
    Ok(())
}

fn reject_inline_on_miss(response: &pb::GetResponse) -> Result<(), DmsError> {
    if response.inline_value.is_some() {
        return Err(DmsError::client_protocol_violation(
            "missing read response must not carry inline bytes".to_string(),
        ));
    }
    Ok(())
}

fn read_target_length(target: &pb::PayloadTarget) -> Result<u64, DmsError> {
    match target.target.as_ref() {
        Some(pb::payload_target::Target::Shm(target)) => Ok(target.length),
        Some(pb::payload_target::Target::Grpc(target)) => Ok(target.length),
        Some(pb::payload_target::Target::Rdma(target)) => Ok(target.length),
        Some(pb::payload_target::Target::Ub(target)) => Ok(target.length),
        None => Err(DmsError::client_protocol_violation(
            "read segment has empty target",
        )),
    }
}

fn write_release_from_target(target: &pb::PayloadTarget) -> Option<pb::ReleasedWriteAllocation> {
    match target.target.as_ref() {
        Some(pb::payload_target::Target::Shm(target)) if !target.release_token.is_empty() => {
            Some(pb::ReleasedWriteAllocation {
                allocation_id: target.allocation_id,
                release_token: target.release_token.clone(),
            })
        }
        _ => None,
    }
}

fn write_release_from_receipt(
    receipt: &pb::TransferReceipt,
) -> Option<pb::ReleasedWriteAllocation> {
    if receipt.release_token.is_empty() {
        return None;
    }
    Some(pb::ReleasedWriteAllocation {
        allocation_id: receipt.target_allocation_id,
        release_token: receipt.release_token.clone(),
    })
}

fn write_release_from_staged_value(value: &pb::StagedValue) -> Option<pb::ReleasedWriteAllocation> {
    value.receipt.as_ref().and_then(write_release_from_receipt)
}

fn staged_key_value_releases(entries: &[pb::StagedKeyValue]) -> Vec<pb::ReleasedWriteAllocation> {
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .value
                .as_ref()
                .and_then(write_release_from_staged_value)
        })
        .collect()
}

fn staged_hash_entry_releases(entries: &[pb::StagedHashEntry]) -> Vec<pb::ReleasedWriteAllocation> {
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .value
                .as_ref()
                .and_then(write_release_from_staged_value)
        })
        .collect()
}

fn validate_read_segments(
    segments: &[pb::ReadSegment],
    expected_length: u64,
) -> Result<(), DmsError> {
    let mut end = 0u64;
    for segment in segments {
        if segment.logical_offset != end {
            return Err(DmsError::client_protocol_violation(
                "read segments contain a gap or overlap",
            ));
        }
        let target = segment
            .target
            .as_ref()
            .ok_or_else(|| DmsError::client_protocol_violation("read segment has no target"))?;
        end = end
            .checked_add(read_target_length(target)?)
            .ok_or_else(|| DmsError::client_protocol_violation("read length overflow"))?;
    }
    if end != expected_length {
        return Err(DmsError::client_protocol_violation(
            "read segments length differs from requested length",
        ));
    }
    Ok(())
}

/// 给 generated Tonic 调用套上统一的 outbound RPC 生命周期。
///
/// 这里统计的是一次真实网络尝试；如果上层因 Session 失效重试，重试会形成第二个
/// 样本。业务 `set/get` 指标由 `ClientMetrics` 另行统计，二者不会混为一谈。
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

/// Creates the one Tonic channel used by all generated Worker clients.
///
/// UDS/TCP diverge only here. After connection, business methods receive the
/// same `Channel` and never branch on transport locality.
async fn connect_channel(
    address: &str,
    config: &GrpcConfig,
    security: &SecurityManager,
) -> Result<Channel, DmsError> {
    // `strip_prefix` 同时完成判断和去掉 scheme；匹配成功得到 socket 路径切片。
    if let Some(path) = address.strip_prefix("unix://") {
        if path.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "empty Unix-domain socket path".to_string(),
            ));
        }
        // 转成拥有所有权的 PathBuf，使它可以安全移动进 `'static` async closure。
        let path = PathBuf::from(path);
        // gRPC/HTTP2 语义仍需要一个 URI；对于 UDS，这个 URI 只是 Tonic 的占位 authority，
        // 真正连接动作会被下面的 UnixStream connector 替换。
        let endpoint = config.configure_client(Endpoint::from_static("http://[::]:50051"));
        return endpoint
            .connect_with_connector(service_fn(move |_| {
                // closure 可能被调用多次，所以每次 clone 一份 PathBuf。
                let path = path.clone();
                // async move 把本次 path 所有权移动进 Future，避免借用悬空。
                async move { UnixStream::connect(path).await.map(TokioIo::new) }
            }))
            .await
            .map_err(|error| DmsError::client_protocol_violation(error.to_string()));
    }

    // 非 UDS 情况只接受显式 HTTP/HTTPS URI，不偷偷猜协议。
    if !address.starts_with("http://") && !address.starts_with("https://") {
        return Err(DmsError::client_invalid_argument(format!(
            "unsupported node endpoint `{address}`"
        )));
    }
    // from_shared 接受运行时字符串并校验 URI；to_string 提供其拥有的 String。
    let endpoint = Endpoint::from_shared(address.to_string())
        .map_err(|error| DmsError::client_invalid_argument(error.to_string()))?;
    let endpoint = config.configure_client(endpoint);
    let endpoint = security
        .configure_client(endpoint)
        .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?;
    // 直到这里才真正建立 TCP/TLS/HTTP2 连接。
    endpoint
        .connect()
        .await
        .map_err(|error| DmsError::client_protocol_violation(error.to_string()))
}

async fn start_session_task(
    mut worker: WorkerServiceClient<dms_tracing::TracedChannel>,
    session_id: u64,
    view_releases: Arc<ViewReleaseTracker>,
    read_finishes: Arc<ReadRequestFinishTracker>,
    options: SessionTaskOptions,
    metrics: Option<ClientMetrics>,
    rpc_metrics: Option<&dms_metrics::RpcMetrics>,
) -> Result<WriteLeaseReleaser, DmsError> {
    let (outbound_sender, outbound_receiver) = mpsc::channel(options.channel_capacity);
    let write_releases = WriteLeaseReleaser::new(
        session_id,
        options.write_lease_release_supported,
        options.channel_capacity.saturating_mul(16).max(64),
    );
    // 第一条心跳标识 session，后续心跳只维持生命周期并归还连续 View 水位。
    outbound_sender
        .send(heartbeat(
            session_id,
            view_releases.released_view_through(),
            Vec::new(),
            read_finishes.finished_read_request_through(),
        ))
        .await
        .map_err(|_| {
            DmsError::client_connection_unavailable("DMS session stream is unavailable")
        })?;
    let mut inbound = observe_rpc(
        rpc_metrics,
        dms_metrics::RpcCall::WORKER_SESSION,
        worker.session(ReceiverStream::new(outbound_receiver)),
    )
    .await
    .map_err(map_status)?
    .into_inner();
    let connection_guard = metrics.as_ref().map(ClientMetrics::node_connection_guard);
    if let Some(metrics) = &metrics {
        metrics.record_node_session_event(NodeSessionEvent::Connected);
    }

    // 不再启动 cache-only unary Heartbeat。新 SDK 不申请跨请求 value 缓存租约，
    // 也不会被列入服务端失效等待者；服务端旧 SDK 的租约义务不受影响。
    let task_write_releases = write_releases.clone();
    tokio::spawn(async move {
        let _connection_guard = connection_guard;
        let mut ticker = tokio::time::interval(options.heartbeat_interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if !flush_write_release_backlog(&outbound_sender, &task_write_releases).await {
                        break;
                    }
                    if outbound_sender
                        .send(heartbeat(
                            session_id,
                            view_releases.released_view_through(),
                            Vec::new(),
                            read_finishes.finished_read_request_through(),
                        ))
                        .await.is_err()
                    {
                        break;
                    }
                }
                message = inbound.message() => {
                    let Ok(Some(event)) = message else { break };
                    // 当前已无 SDK Current value 可失效，旧服务端仍可能发该事件。
                    // 仅确认已知事件；未知事件不能假装处理完成并推进 ACK。
                    if !matches!(event.event,
                        Some(pb::node_session_event::Event::CurrentInvalidation(_)))
                    {
                        continue;
                    }
                    let ack = pb::ClientSessionMessage {
                        session_id,
                        message: Some(pb::client_session_message::Message::EventAck(
                            pb::SessionEventAck { event_sequence: event.event_sequence },
                        )),
                    };
                    if outbound_sender.send(ack).await.is_err() { break; }
                }
            }
        }
        // 断流不是 View 已释放的证明。此处不跨过未收到/未归还的 epoch。
        log::warn!("DMS node session disconnected (session_id={session_id})");
        if let Some(metrics) = &metrics {
            metrics.record_node_session_event(NodeSessionEvent::Disconnected);
        }
    });
    Ok(write_releases)
}

fn heartbeat(
    session_id: u64,
    released_view_through: Option<u64>,
    released_write_allocations: Vec<pb::ReleasedWriteAllocation>,
    finished_read_request_through: Option<u64>,
) -> pb::ClientSessionMessage {
    // 纯构造函数：没有 I/O，只把领域参数编码成 protobuf DTO。
    pb::ClientSessionMessage {
        session_id,
        message: Some(pb::client_session_message::Message::Heartbeat(
            pb::SessionHeartbeat {
                released_view_through,
                released_write_allocations,
                finished_read_request_through,
            },
        )),
    }
}

async fn flush_write_release_backlog(
    sender: &mpsc::Sender<pb::ClientSessionMessage>,
    releaser: &WriteLeaseReleaser,
) -> bool {
    loop {
        let releases = releaser.take_pending_batch(64);
        if releases.is_empty() {
            return true;
        }
        if sender
            .send(heartbeat(releaser.session_id, None, releases.clone(), None))
            .await
            .is_err()
        {
            releaser.requeue_front(releases);
            return false;
        }
    }
}

fn encode_operation_id(operation: OperationId) -> pb::OperationId {
    pb::OperationId {
        client_instance_id: operation.client_instance_id.to_vec(),
        sequence: operation.sequence,
    }
}

fn encode_range(range: ByteRange) -> pb::ByteRange {
    pb::ByteRange {
        offset: range.offset,
        length: range.len,
    }
}

fn encode_condition(condition: WriteCondition) -> String {
    // 当前 wire schema 暂用字符串；match 穷举所有 enum 分支，新增策略时编译器会提醒。
    match condition {
        WriteCondition::Any => "any".to_string(),
        WriteCondition::IfAbsent => "if-absent".to_string(),
        WriteCondition::IfPresent => "if-present".to_string(),
        WriteCondition::IfVersion(version) => format!("if-version:{}", version.0),
    }
}

fn encode_durability(durability: DurabilityPolicy) -> String {
    match durability {
        DurabilityPolicy::LocalMemory => "local-memory".to_string(),
        DurabilityPolicy::MemoryCopies(copies) => format!("memory-copies:{copies}"),
        DurabilityPolicy::LocalDisk => "local-disk".to_string(),
        DurabilityPolicy::ObjectStore => "object-store".to_string(),
    }
}

fn encode_hash_read_version(version: HashReadVersion) -> Option<u64> {
    match version {
        HashReadVersion::Current => None,
        HashReadVersion::Exact(version) => Some(version.0),
    }
}

fn encode_scan_options(options: ScanOptions) -> pb::ObjectScanOptions {
    pb::ObjectScanOptions {
        limit: options.limit,
        start_after: options.start_after,
        cursor: options.cursor.unwrap_or_default(),
        delimiter: options.delimiter,
    }
}

fn decode_hash_get(response: pb::HGetResponse) -> Result<Option<HashValue>, DmsError> {
    if !response.found {
        return Ok(None);
    }
    response
        .value
        .map(decode_hash_value)
        .transpose()?
        .ok_or_else(|| DmsError::client_protocol_violation("Hash hit has no value".to_string()))
        .map(Some)
}

fn decode_hash_value(value: pb::HashValueRead) -> Result<HashValue, DmsError> {
    let field = value.field.ok_or_else(|| {
        DmsError::client_protocol_violation("Hash value has no field".to_string())
    })?;
    if value.logical_length != value.inline_value.len() as u64 {
        return Err(DmsError::client_protocol_violation(
            "Hash inline value length does not match metadata".to_string(),
        ));
    }
    Ok(HashValue {
        field: HashField::new(field.value)
            .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?,
        hash_version: HashVersion(value.hash_version),
        value_version: ObjectVersion(value.value_version),
        bytes: value.inline_value,
    })
}

fn decode_object_info(info: pb::ObjectInfo) -> Result<ObjectInfo, DmsError> {
    let key = info
        .key
        .ok_or_else(|| DmsError::client_protocol_violation("ObjectInfo has no key".to_string()))?;
    Ok(ObjectInfo {
        key: key.value,
        length: info.length,
        modified_time: system_time_from_unix_millis(info.modified_time_unix_millis)
            .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?,
        version: ObjectVersion(info.version),
        is_prefix: info.is_prefix,
    })
}

pub(super) fn map_status(status: tonic::Status) -> DmsError {
    // DMS 对端会在 Status.details 放 ErrorDetail；普通代理/网络错误没有 detail，
    // 这时只能在 SDK 边界生成本地连接错误。
    status_to_dms_error_with(status, DmsError::client_connection_unavailable)
}

#[cfg(test)]
mod read_lifecycle_tests {
    use super::*;

    #[test]
    fn downloaded_owned_first_part_is_adopted_for_small_and_file_blocks() {
        for length in [1, 4096, 512 * 1024, 4 * 1024 * 1024] {
            let part = vec![7; length];
            let original = part.as_ptr();
            let mut output = Vec::new();
            append_downloaded_part(&mut output, part);
            assert_eq!(output.as_ptr(), original, "length={length}");
            assert_eq!(output, vec![7; length]);
            append_downloaded_part(&mut output, vec![8, 9]);
            assert_eq!(&output[length..], &[8, 9]);
        }
    }

    fn disconnected_node(releases: &Arc<ViewReleaseTracker>) -> NodeConnection {
        let channel = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        let grpc_config = GrpcConfig::default();
        NodeConnection {
            worker: worker_client(channel.clone(), &grpc_config),
            transfer: TransferEngine::new(channel, 1, None, &grpc_config, None, None),
            rpc_metrics: None,
            session_id: 1,
            inline_threshold_bytes: 0,
            view_releases: Arc::clone(releases),
            read_finishes: Arc::new(ReadRequestFinishTracker::default()),
            next_read_request_id: AtomicU64::new(1),
            write_releases: WriteLeaseReleaser::disabled(1),
        }
    }

    fn shared_response(epoch: u64) -> pb::GetResponse {
        pb::GetResponse {
            found: true,
            version: 1,
            logical_length: 4,
            segments: vec![pb::ReadSegment {
                logical_offset: 0,
                read_request_id: 0,
                target: Some(pb::PayloadTarget {
                    target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                        view_epoch: Some(epoch),
                        length: 4,
                        ..Default::default()
                    })),
                }),
            }],
            inline_value: None,
            read_request_id: 0,
        }
    }

    #[test]
    fn failed_batch_releases_even_unvisited_read_responses() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let releases = Arc::new(ViewReleaseTracker::default());
            let connection = disconnected_node(&releases);
            // 第一项 mmap 失败；第二项从未开始解码，但两份借用都已收到。
            let batch = pb::MGetResponse {
                items: vec![shared_response(1), shared_response(2)],
            };
            assert!(connection.decode_mget_response(batch, None).await.is_err());
            assert_eq!(releases.released_view_through(), Some(2));
        });
    }

    #[test]
    fn rejected_or_unmappable_explicit_view_releases_all_received_epochs() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let releases = Arc::new(ViewReleaseTracker::default());
            let connection = disconnected_node(&releases);
            let mut multi = shared_response(1);
            multi.segments.extend(shared_response(1).segments);
            assert!(
                connection
                    .decode_view_response(multi, None, false, None)
                    .await
                    .is_err()
            );
            assert_eq!(releases.released_view_through(), Some(1));
            assert!(
                connection
                    .decode_view_response(shared_response(2), None, false, None)
                    .await
                    .is_err()
            );
            assert_eq!(releases.released_view_through(), Some(2));
        });
    }

    #[test]
    fn long_lived_view_compacts_completed_read_epochs() {
        let releases = Arc::new(ViewReleaseTracker::default());
        for epoch in 2..=100_000 {
            releases.mark_released(epoch);
        }
        assert_eq!(releases.released_view_through(), None);
        assert_eq!(releases.inner.lock().unwrap().pending.len(), 1);
        releases.mark_released(1);
        assert_eq!(releases.released_view_through(), Some(100_000));
        assert!(releases.inner.lock().unwrap().pending.is_empty());
    }

    #[test]
    fn compressed_releases_match_individual_sequence_reference() {
        let releases = Arc::new(ViewReleaseTracker::default());
        let mut reference = std::collections::BTreeSet::new();
        let mut expected = 0;
        // 多个间隙、桥接左右区间、重复释放均不越过尚未释放的序号。
        for epoch in [3, 5, 4, 9, 7, 8, 2, 2, 6, 11, 1, 10, 13, 12] {
            reference.insert(epoch);
            while reference.remove(&(expected + 1)) {
                expected += 1;
            }
            releases.mark_released(epoch);
            assert_eq!(releases.released_view_through().unwrap_or(0), expected);
        }
    }

    #[test]
    fn failed_shm_read_releases_received_epoch() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let channel = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
            let grpc_config = GrpcConfig::default();
            let releases = Arc::new(ViewReleaseTracker::default());
            let connection = NodeConnection {
                worker: worker_client(channel.clone(), &grpc_config),
                transfer: TransferEngine::new(channel, 1, None, &grpc_config, None, None),
                rpc_metrics: None,
                session_id: 1,
                inline_threshold_bytes: 0,
                view_releases: Arc::clone(&releases),
                read_finishes: Arc::new(ReadRequestFinishTracker::default()),
                next_read_request_id: AtomicU64::new(1),
                write_releases: WriteLeaseReleaser::disabled(1),
            };
            let response = pb::GetResponse {
                found: true,
                version: 1,
                logical_length: 4,
                segments: vec![pb::ReadSegment {
                    logical_offset: 0,
                    read_request_id: 0,
                    target: Some(pb::PayloadTarget {
                        target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                            view_epoch: Some(1),
                            length: 4,
                            ..Default::default()
                        })),
                    }),
                }],
                inline_value: None,
                read_request_id: 0,
            };
            // 没有 FD Broker，映射一定失败；收到的读取保护仍必须归还。
            assert!(connection.decode_get_response(response).await.is_err());
            assert_eq!(releases.released_view_through(), Some(1));
        });
    }

    #[test]
    fn range_response_checks_entire_layout_before_mapping_or_download() {
        let segment = |offset, length| pb::ReadSegment {
            logical_offset: offset,
            read_request_id: 0,
            target: Some(pb::PayloadTarget {
                target: Some(pb::payload_target::Target::Grpc(pb::GrpcTarget {
                    transfer_id: vec![],
                    nonce: vec![],
                    length,
                })),
            }),
        };
        assert!(validate_read_segments(&[segment(0, 1)], 1).is_ok());
        assert!(validate_read_segments(&[segment(0, 6)], 1).is_err());
        assert!(validate_read_segments(&[segment(0, 2), segment(2, 1), segment(3, 3)], 6).is_ok());
        assert!(validate_read_segments(&[segment(0, 2), segment(1, 4)], 6).is_err());
        assert!(validate_read_segments(&[segment(0, 2), segment(3, 3)], 6).is_err());
        assert!(validate_read_segments(&[], 1).is_err());
        assert!(validate_read_segments(&[], 0).is_ok());
        assert!(validate_read_segments(&[segment(1, 1)], 1).is_err());
        assert!(validate_read_segments(&[segment(0, 1)], 2).is_err());
        assert!(validate_read_segments(&[segment(0, u64::MAX), segment(u64::MAX, 1)], 0).is_err());
        assert!(
            validate_read_segments(
                &[pb::ReadSegment {
                    logical_offset: 0,
                    read_request_id: 0,
                    target: None
                }],
                0
            )
            .is_err()
        );
        assert!(
            validate_read_segments(
                &[pb::ReadSegment {
                    logical_offset: 0,
                    read_request_id: 0,
                    target: Some(pb::PayloadTarget { target: None }),
                }],
                0
            )
            .is_err()
        );
        assert!(selected_read_length(6, Some(ByteRange { offset: 2, len: 1 }), false).is_ok());
        assert!(
            selected_read_length(
                6,
                Some(ByteRange {
                    offset: u64::MAX,
                    len: 2
                }),
                false
            )
            .is_err()
        );
        assert_eq!(
            selected_read_length(10, Some(ByteRange { offset: 8, len: 8 }), true).unwrap(),
            2
        );
        assert_eq!(
            selected_read_length(10, Some(ByteRange { offset: 10, len: 8 }), true).unwrap(),
            0
        );
        assert_eq!(
            selected_read_length(10, Some(ByteRange { offset: 99, len: 8 }), true).unwrap(),
            0
        );
        assert!(
            selected_read_length(
                10,
                Some(ByteRange {
                    offset: u64::MAX,
                    len: 8,
                }),
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn inline_read_response_is_used_only_when_the_request_allows_it() {
        assert_eq!(
            take_inline_value(Some(b"abc".to_vec()), &[], 3, 64)
                .unwrap()
                .unwrap(),
            b"abc"
        );
        assert_eq!(
            take_inline_value(Some(Vec::new()), &[], 0, 64)
                .unwrap()
                .unwrap(),
            Vec::<u8>::new()
        );
        assert!(take_inline_value(None, &[], 3, 64).unwrap().is_none());

        let segment = pb::ReadSegment {
            logical_offset: 0,
            read_request_id: 0,
            target: Some(pb::PayloadTarget {
                target: Some(pb::payload_target::Target::Grpc(pb::GrpcTarget {
                    transfer_id: b"t".to_vec(),
                    nonce: b"n".to_vec(),
                    length: 3,
                })),
            }),
        };
        assert!(
            take_inline_value(Some(b"abc".to_vec()), std::slice::from_ref(&segment), 3, 64)
                .is_err()
        );
        assert!(take_inline_value(Some(b"abc".to_vec()), &[], 4, 64).is_err());
        assert!(take_inline_value(Some(b"abc".to_vec()), &[], 3, 2).is_err());
        assert!(take_inline_value(Some(b"abc".to_vec()), &[], 3, 0).is_err());
    }

    #[test]
    fn inline_read_response_is_rejected_for_miss_view_and_mget_paths() {
        let missing_with_bytes = pb::GetResponse {
            found: false,
            version: 0,
            logical_length: 0,
            segments: Vec::new(),
            inline_value: Some(b"ghost".to_vec()),
            read_request_id: 0,
        };
        assert!(reject_inline_on_miss(&missing_with_bytes).is_err());

        let hit_with_bytes = pb::GetResponse {
            found: true,
            version: 9,
            logical_length: 3,
            segments: Vec::new(),
            inline_value: Some(b"abc".to_vec()),
            read_request_id: 0,
        };
        assert!(
            reject_inline_value(&hit_with_bytes).is_err(),
            "get_view and MGET keep their original segment/view path and pass zero inline budget"
        );
    }

    #[test]
    fn cancelled_read_and_out_of_order_release_do_not_skip_live_view() {
        let releases = Arc::new(ViewReleaseTracker::default());
        let segment = |epoch| pb::ReadSegment {
            logical_offset: 0,
            read_request_id: 0,
            target: Some(pb::PayloadTarget {
                target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                    view_epoch: Some(epoch),
                    ..Default::default()
                })),
            }),
        };
        let live_view = releases.protect([segment(1)].iter(), None);
        let cancelled = releases.protect([segment(2), segment(2), segment(3)].iter(), None);
        let task = async move {
            let _guard = cancelled;
            std::future::pending::<()>().await;
        };
        drop(task);
        assert_eq!(releases.released_view_through(), None);
        drop(live_view);
        assert_eq!(releases.released_view_through(), Some(3));
        // 模拟响应4未到达；不能因为5已释放就越过未知的4。
        drop(releases.protect([segment(5)].iter(), None));
        assert_eq!(releases.released_view_through(), Some(3));
        drop(releases.protect([segment(4)].iter(), None));
        assert_eq!(releases.released_view_through(), Some(5));
    }

    #[test]
    fn read_request_finish_tracker_does_not_skip_live_request() {
        let tracker = ReadRequestFinishTracker::default();
        tracker.mark_finished(2);
        tracker.mark_finished(4);
        assert_eq!(tracker.finished_read_request_through(), None);
        tracker.mark_finished(1);
        assert_eq!(tracker.finished_read_request_through(), Some(2));
        tracker.mark_finished(3);
        assert_eq!(tracker.finished_read_request_through(), Some(4));
    }

    #[test]
    fn explicit_view_keeps_read_request_live_until_view_drop() {
        let releases = Arc::new(ViewReleaseTracker::default());
        let finishes = Arc::new(ReadRequestFinishTracker::default());
        let segment = pb::ReadSegment {
            logical_offset: 0,
            read_request_id: 1,
            target: Some(pb::PayloadTarget {
                target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                    view_epoch: Some(1),
                    ..Default::default()
                })),
            }),
        };
        let protection = releases.protect(
            [segment].iter(),
            Some(ReadRequestGuard {
                finishes: Arc::clone(&finishes),
                read_request_id: 1,
            }),
        );

        assert_eq!(finishes.finished_read_request_through(), None);
        drop(protection);
        assert_eq!(releases.released_view_through(), Some(1));
        // 只有 View/ReadProtection drop 后，Node 才能收到 finished 水位。
        assert_eq!(finishes.finished_read_request_through(), Some(1));
    }

    #[test]
    fn reader_keeps_read_request_live_until_reader_drop() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let releases = Arc::new(ViewReleaseTracker::default());
            let connection = disconnected_node(&releases);
            let read = connection.begin_read_request();
            let finishes = Arc::clone(&read.finishes);

            let reader = connection
                .decode_reader_response(shared_response(1), None, false, Some(read))
                .await
                .expect("decode reader")
                .expect("hit");
            assert_eq!(reader.version(), ObjectVersion(1));
            assert_eq!(reader.len(), 4);
            assert_eq!(releases.released_view_through(), None);
            assert_eq!(finishes.finished_read_request_through(), None);

            drop(reader);
            assert_eq!(releases.released_view_through(), Some(1));
            assert_eq!(finishes.finished_read_request_through(), Some(1));
        });
    }

    #[test]
    fn reader_eof_releases_read_request_before_reader_drop() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let releases = Arc::new(ViewReleaseTracker::default());
            let connection = disconnected_node(&releases);
            let read = connection.begin_read_request();
            let finishes = Arc::clone(&read.finishes);
            let response = pb::GetResponse {
                found: true,
                version: 9,
                logical_length: 0,
                segments: Vec::new(),
                inline_value: None,
                read_request_id: read.id(),
            };
            let mut reader = connection
                .decode_reader_response(response, None, false, Some(read))
                .await
                .expect("decode reader")
                .expect("hit");

            let mut dst = [0_u8; 8];
            assert_eq!(
                connection
                    .read_value_reader(&mut reader, &mut dst)
                    .await
                    .expect("read EOF"),
                0
            );
            assert_eq!(finishes.finished_read_request_through(), Some(1));

            drop(reader);
            assert_eq!(finishes.finished_read_request_through(), Some(1));
        });
    }

    #[test]
    fn get_into_rejects_small_buffer_before_copy_and_completes_read() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let releases = Arc::new(ViewReleaseTracker::default());
            let connection = disconnected_node(&releases);
            let read = connection.begin_read_request();
            let finishes = Arc::clone(&read.finishes);
            let response = pb::GetResponse {
                found: true,
                version: 7,
                logical_length: 4,
                segments: Vec::new(),
                inline_value: Some(b"data".to_vec()),
                read_request_id: read.id(),
            };
            let mut dst = [0_u8; 3];

            let error = connection
                .decode_read_into_response(response, None, false, 64, Some(read), &mut dst)
                .await
                .expect_err("small buffer should fail");
            assert_eq!(error.kind(), dms_error::ErrorKind::InvalidArgument);
            assert_eq!(dst, [0, 0, 0]);
            assert_eq!(finishes.finished_read_request_through(), Some(1));
        });
    }

    #[test]
    fn get_into_accepts_clamped_tail_and_eof_inline_responses() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let releases = Arc::new(ViewReleaseTracker::default());
            let connection = disconnected_node(&releases);
            let mut dst = [0_u8; 8];
            let tail = pb::GetResponse {
                found: true,
                version: 11,
                logical_length: 10,
                segments: Vec::new(),
                inline_value: Some(b"ij".to_vec()),
                read_request_id: 0,
            };
            let result = connection
                .decode_read_into_response(
                    tail,
                    Some(ByteRange { offset: 8, len: 8 }),
                    true,
                    64,
                    None,
                    &mut dst,
                )
                .await
                .expect("decode clamped tail")
                .expect("hit");
            assert_eq!(result.version, ObjectVersion(11));
            assert_eq!(result.len, 2);
            assert_eq!(&dst[..2], b"ij");
            assert_eq!(&dst[2..], &[0; 6]);

            let eof = pb::GetResponse {
                found: true,
                version: 12,
                logical_length: 10,
                segments: Vec::new(),
                inline_value: Some(Vec::new()),
                read_request_id: 0,
            };
            let result = connection
                .decode_read_into_response(
                    eof,
                    Some(ByteRange { offset: 99, len: 8 }),
                    true,
                    64,
                    None,
                    &mut dst,
                )
                .await
                .expect("decode clamped eof")
                .expect("hit");
            assert_eq!(result.version, ObjectVersion(12));
            assert_eq!(result.len, 0);
            assert_eq!(&dst[2..], &[0; 6]);
        });
    }

    #[test]
    fn set_from_short_source_is_invalid_argument() {
        let mut source = std::io::Cursor::new(b"abc".to_vec());
        let error = read_exact_source(&mut source, 4).expect_err("short source");
        assert_eq!(error.kind(), dms_error::ErrorKind::InvalidArgument);
    }

    #[tokio::test]
    async fn shutdown_heartbeat_reports_completed_short_read_without_live_view() {
        let releases = Arc::new(ViewReleaseTracker::default());
        let connection = disconnected_node(&releases);
        let segment = pb::ReadSegment {
            logical_offset: 0,
            read_request_id: 1,
            target: Some(pb::PayloadTarget {
                target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                    view_epoch: Some(1),
                    ..Default::default()
                })),
            }),
        };
        {
            let read = connection.begin_read_request();
            let _copied_read = releases.protect([segment].iter(), Some(read));
        }

        let request = connection
            .shutdown_heartbeat_request(connection.write_releases.take_all_pending())
            .expect("completed read should produce final heartbeat");
        assert_eq!(request.released_view_through, Some(1));
        assert_eq!(request.finished_read_request_through, Some(1));
        assert!(request.released_write_allocations.is_empty());
    }

    #[tokio::test]
    async fn shutdown_heartbeat_does_not_finish_live_explicit_view() {
        let releases = Arc::new(ViewReleaseTracker::default());
        let connection = disconnected_node(&releases);
        let segment = pb::ReadSegment {
            logical_offset: 0,
            read_request_id: 1,
            target: Some(pb::PayloadTarget {
                target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                    view_epoch: Some(1),
                    ..Default::default()
                })),
            }),
        };
        let read = connection.begin_read_request();
        let _live_view = releases.protect([segment].iter(), Some(read));

        assert!(
            connection
                .shutdown_heartbeat_request(connection.write_releases.take_all_pending())
                .is_none(),
            "live explicit view must not be reported as a completed read"
        );
    }

    #[test]
    fn write_release_is_extracted_only_from_shm_tokens() {
        let shm = pb::PayloadTarget {
            target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                allocation_id: 7,
                release_token: b"lease-7".to_vec(),
                ..Default::default()
            })),
        };
        let release = write_release_from_target(&shm).expect("SHM token should be releasable");
        assert_eq!(release.allocation_id, 7);
        assert_eq!(release.release_token, b"lease-7");

        let grpc = pb::PayloadTarget {
            target: Some(pb::payload_target::Target::Grpc(pb::GrpcTarget {
                transfer_id: b"t".to_vec(),
                nonce: b"n".to_vec(),
                length: 4,
            })),
        };
        assert!(write_release_from_target(&grpc).is_none());
        assert!(
            write_release_from_receipt(&pb::TransferReceipt {
                transfer_id: b"t".to_vec(),
                length: 4,
                digest: Vec::new(),
                target_allocation_id: 0,
                release_token: Vec::new(),
            })
            .is_none()
        );
    }

    #[tokio::test]
    async fn write_lease_guard_releases_on_drop_and_consumes_on_commit() {
        let releaser = WriteLeaseReleaser::new(42, true, 8);
        {
            let _guard = WriteLeaseGuard::new(
                releaser.clone(),
                Some(pb::ReleasedWriteAllocation {
                    allocation_id: 9,
                    release_token: b"lease-9".to_vec(),
                }),
            );
        }
        assert_eq!(releaser.pending_len(), 1);

        {
            let mut guard = WriteLeaseGuard::new(
                releaser.clone(),
                Some(pb::ReleasedWriteAllocation {
                    allocation_id: 10,
                    release_token: b"lease-10".to_vec(),
                }),
            )
            .expect("guard");
            guard.consume();
        }
        assert_eq!(releaser.pending_len(), 1);
    }

    #[tokio::test]
    async fn write_release_backlog_flushes_after_session_queue_was_full() {
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .send(heartbeat(42, None, Vec::new(), None))
            .await
            .expect("prefill channel");
        let releaser = WriteLeaseReleaser::new(42, true, 8);

        releaser.release(Some(pb::ReleasedWriteAllocation {
            allocation_id: 11,
            release_token: b"lease-11".to_vec(),
        }));
        assert_eq!(releaser.pending_len(), 1);

        let _prefilled = receiver.recv().await.expect("prefilled heartbeat");
        assert!(flush_write_release_backlog(&sender, &releaser).await);
        assert_eq!(releaser.pending_len(), 0);
        let message = receiver.recv().await.expect("flushed release");
        let Some(pb::client_session_message::Message::Heartbeat(heartbeat)) = message.message
        else {
            panic!("write release must be carried by heartbeat");
        };
        assert_eq!(heartbeat.released_write_allocations.len(), 1);
        assert_eq!(heartbeat.released_write_allocations[0].allocation_id, 11);
    }

    #[tokio::test]
    async fn write_release_overflow_is_quarantined_not_marked_released() {
        let releaser = WriteLeaseReleaser::new(42, true, 1);
        releaser.enqueue_backlog(vec![
            pb::ReleasedWriteAllocation {
                allocation_id: 1,
                release_token: b"lease-1".to_vec(),
            },
            pb::ReleasedWriteAllocation {
                allocation_id: 2,
                release_token: b"lease-2".to_vec(),
            },
        ]);
        assert_eq!(releaser.pending_len(), 1);
        assert_eq!(releaser.quarantined_count(), 1);
    }
}

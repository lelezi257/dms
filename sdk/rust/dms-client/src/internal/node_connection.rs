//! SDK 到一个 dms-node 的连接，以及这条连接上的长生命周期 Session stream。
//!
//! `connect_channel` 是唯一判断 `unix://` 与 `http(s)://` 的位置。连接建立后，
//! `set/get` 都只看到同一种 Tonic `Channel`，因此业务逻辑不需要 transport 分支。

// Arc 是线程安全引用计数指针：Session 后台 Task 与前台 Client 共享同一个缓存对象。
use std::{
    collections::BTreeSet,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

// `as pb` 给生成代码起短别名，后续 `pb::SetRequest` 明确表示 wire DTO。
use dms_protocol::v1 as pb;
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

use crate::{
    ByteRange, ClientTlsOptions, DeleteResult, DmsError, DurabilityPolicy, GetOptions, GetResult,
    HashDeleteOptions, HashEntriesResult, HashField, HashGetOptions, HashMultiGetResult,
    HashRangeWriteOptions, HashRangeWriteResult, HashReadVersion, HashScanOptions, HashScanResult,
    HashSetResult, HashValue, HashVersion, HashWriteMode, HashWriteOptions, Key, KeyVersion,
    MSetResult, ObjectVersion, OperationId, RangeWriteOptions, ReadVersion, ScanCursor, SetOptions,
    SetResult, WriteCondition,
};

use super::client_cache::ClientCache;
use super::transfer_engine::{PayloadBuffer, TransferEngine};
use crate::client::ResolvedClientOptions;
use crate::metrics::{CacheInvalidation, ClientMetrics, NodeSessionEvent};
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
}

#[derive(Default)]
pub(crate) struct ViewReleaseTracker {
    inner: Mutex<ViewReleaseState>,
}

#[derive(Default)]
struct ViewReleaseState {
    released_through: u64,
    pending: BTreeSet<u64>,
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
        state.pending.insert(view_epoch);
        loop {
            let next = state.released_through + 1;
            if !state.pending.remove(&next) {
                break;
            }
            state.released_through += 1;
        }
    }

    fn released_view_through(&self) -> Option<u64> {
        let Ok(state) = self.inner.lock() else {
            return None;
        };
        (state.released_through > 0).then_some(state.released_through)
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
    view_epoch: Option<u64>,
    releases: Arc<ViewReleaseTracker>,
    buffer: PayloadBuffer,
}

/// 长连接 Task 自己消费的两个运行参数，避免把 SDK 的整份配置带进后台任务。
struct SessionTaskOptions {
    heartbeat_interval: Duration,
    channel_capacity: usize,
    cache_enabled: bool,
}

/// Stream 正常退出、panic 或 runtime 取消任务都必须撤销缓存资格。
struct SessionCacheGuard(Arc<ClientCache>);

impl Drop for SessionCacheGuard {
    fn drop(&mut self) {
        self.0.session_disconnected();
    }
}

struct SessionRenewalGuard(tokio::task::JoinHandle<()>);

impl Drop for SessionRenewalGuard {
    fn drop(&mut self) {
        self.0.abort();
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

impl Drop for SharedViewInner {
    fn drop(&mut self) {
        if let Some(view_epoch) = self.view_epoch {
            self.releases.mark_released(view_epoch);
        }
    }
}

impl NodeConnection {
    pub(crate) async fn connect(
        options: &ResolvedClientOptions,
        cache: Arc<ClientCache>,
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
        let mut worker = WorkerServiceClient::new(dms_tracing::traced_channel(channel.clone()));
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
            }),
        )
        .await;
        let session = session_result.map_err(map_status)?.into_inner();
        let session_id = session.session_id;
        let fd_broker_path = session.shm.map(|shm| shm.fd_broker_path);
        let view_releases = Arc::new(ViewReleaseTracker::default());
        // 再建立长连接 stream：后台发送 heartbeat，并接收 cache invalidation。
        start_session_task(
            worker,
            session_id,
            cache,
            Arc::clone(&view_releases),
            SessionTaskOptions {
                heartbeat_interval: options.heartbeat_interval,
                channel_capacity: options.session_channel_capacity,
                cache_enabled: !options.shared_memory && options.current_cache_bytes > 0,
            },
            metrics.clone(),
            rpc_metrics.as_ref(),
        )
        .await?;
        // 返回时 generated Client 本身可以丢弃；Channel 留在 NodeConnection 中复用。
        Ok(Self {
            worker: WorkerServiceClient::new(dms_tracing::traced_channel(channel.clone())),
            transfer: TransferEngine::new(
                channel,
                session_id,
                fd_broker_path,
                metrics.clone(),
                rpc_metrics.clone(),
            ),
            rpc_metrics,
            session_id,
            inline_threshold_bytes: options.inline_threshold_bytes,
            view_releases,
        })
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

        // 第二步由统一传输边界选择 provider，业务层不识别 gRPC/SHM/RDMA/UB。
        let receipt = match self.transfer.upload(self.session_id, target, value).await {
            Ok(receipt) => receipt,
            Err(error) => {
                // Allocation succeeded but payload transfer did not. Release is
                // idempotent, and failure to release must not hide the original
                // transfer error; Session close/TTL remain the final safety net.
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
                // successful no-op in the latter case, so one cleanup path is safe.
                let _ = observe_rpc(
                    self.rpc_metrics.as_ref(),
                    dms_metrics::RpcCall::WORKER_DELETE_STAGING,
                    worker.delete_staging(pb::DeleteStagingRequest {
                        session_id: self.session_id,
                        staging_id: allocation.staging_id,
                    }),
                )
                .await;
                return Err(map_status(status));
            }
        };
        // 把 protobuf DTO 转换为 SDK 的公开领域类型，避免用户依赖生成代码。
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
        let buffer = match self.transfer.map_target(self.session_id, target).await {
            Ok(buffer) => buffer,
            Err(error) => {
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
        })
    }

    pub(crate) async fn commit_shared(
        &self,
        write: SharedWriteInner,
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
                let _ = observe_rpc(
                    self.rpc_metrics.as_ref(),
                    dms_metrics::RpcCall::WORKER_DELETE_STAGING,
                    worker.delete_staging(pb::DeleteStagingRequest {
                        session_id: self.session_id,
                        staging_id: write.staging_id,
                    }),
                )
                .await;
                return Err(map_status(status));
            }
        };
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
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        if !response.found {
            return Ok(None);
        }
        if response.segments.len() != 1 {
            return Err(DmsError::node_transfer_unsupported(
                "shared-memory view currently supports exactly one read segment".to_string(),
            ));
        }
        let expected_length = requested_read_length(response.logical_length, options.range)?;
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
        let view_epoch = buffer.view_epoch();
        Ok(Some(SharedViewInner {
            version: ObjectVersion(response.version),
            view_epoch,
            releases: Arc::clone(&self.view_releases),
            buffer,
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
                self.delete_staging_many(&mut worker, &allocated_staging)
                    .await;
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
        // Get 的控制请求先解析版本和 payload 位置，不在响应里直接塞业务 bytes。
        let mut worker = self.worker.clone();
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
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        // “key 不存在”是正常业务结果，因此返回 Ok(None)，不是 Err(NotFound)。
        if !response.found {
            return Ok(None);
        }
        let expected_length = requested_read_length(response.logical_length, options.range)?;
        let bytes = self
            .download_segments(response.segments, expected_length)
            .await?;
        Ok(Some(GetResult {
            version: ObjectVersion(response.version),
            bytes,
        }))
    }

    pub(crate) async fn mget(&self, keys: &[Key]) -> Result<Vec<Option<GetResult>>, DmsError> {
        let mut worker = self.worker.clone();
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
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        let mut results = Vec::with_capacity(response.items.len());
        for item in response.items {
            results.push(self.decode_get_response(item).await?);
        }
        Ok(results)
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
                let _ = observe_rpc(
                    self.rpc_metrics.as_ref(),
                    dms_metrics::RpcCall::WORKER_DELETE_STAGING,
                    worker.delete_staging(pb::DeleteStagingRequest {
                        session_id: self.session_id,
                        staging_id,
                    }),
                )
                .await;
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
                // Node 在成功 HSET 时会一次性 consume 所有 staging；失败时由
                // Client 主动回收，避免等 Session TTL 才释放 Arena slot。
                self.delete_staging_many(&mut worker, &staging_ids).await;
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
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        decode_hash_get(response)
    }

    pub(crate) async fn hmget(
        &self,
        key: &Key,
        fields: &[HashField],
        options: HashGetOptions,
    ) -> Result<HashMultiGetResult, DmsError> {
        let mut worker = self.worker.clone();
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
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
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
        let response = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::WORKER_HGET_ALL,
            worker.h_get_all(pb::HGetAllRequest {
                session_id: self.session_id,
                key: Some(pb::Key {
                    value: key.as_bytes().to_vec(),
                }),
                exact_hash_version: encode_hash_read_version(options.version),
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
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
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
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
                self.delete_staging_many(&mut worker, &[staging_id]).await;
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
        let receipt = match self.transfer.upload(self.session_id, target, value).await {
            Ok(receipt) => receipt,
            Err(error) => {
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

    async fn decode_get_response(
        &self,
        response: pb::GetResponse,
    ) -> Result<Option<GetResult>, DmsError> {
        if !response.found {
            return Ok(None);
        }
        let bytes = self
            .download_segments(response.segments, response.logical_length)
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
            bytes.extend_from_slice(&part);
        }
        Ok(bytes)
    }
}

fn requested_read_length(logical_length: u64, range: Option<ByteRange>) -> Result<u64, DmsError> {
    match range {
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
    cache: Arc<ClientCache>,
    view_releases: Arc<ViewReleaseTracker>,
    options: SessionTaskOptions,
    metrics: Option<ClientMetrics>,
    rpc_metrics: Option<&dms_metrics::RpcMetrics>,
) -> Result<(), DmsError> {
    // 有界容量来自 SDK 配置：发送者过快时 `.send().await` 会等待，形成背压。
    let (outbound_sender, outbound_receiver) = mpsc::channel(options.channel_capacity);
    // stream 建立前先放入第一条 heartbeat；Server 用它识别 session_id。
    outbound_sender
        .send(heartbeat(session_id, view_releases.released_view_through()))
        .await
        .map_err(|_| {
            DmsError::client_connection_unavailable("DMS session stream is unavailable")
        })?;
    // ReceiverStream 消费 outbound_receiver，并持续把消息发给 Node。
    // 返回的 inbound 则是 Node→Client 的事件流，因此这是双向 stream。
    let mut inbound = observe_rpc(
        rpc_metrics,
        dms_metrics::RpcCall::WORKER_SESSION,
        worker.session(ReceiverStream::new(outbound_receiver)),
    )
    .await
    .map_err(map_status)?
    .into_inner();
    cache.session_connected();
    let cache_guard = SessionCacheGuard(Arc::clone(&cache));
    let connection_guard = metrics.as_ref().map(ClientMetrics::node_connection_guard);
    if let Some(metrics) = &metrics {
        metrics.record_node_session_event(NodeSessionEvent::Connected);
    }

    let renewal_cache = Arc::clone(&cache);
    let renewal_releases = Arc::clone(&view_releases);
    let renewal_rpc_metrics = rpc_metrics.cloned();
    let renewal_interval = options.heartbeat_interval;
    // unary 续租与 Session 事件消费独立推进，网络等待不堵住失效 ACK。
    let renewal = options.cache_enabled.then(|| {
        SessionRenewalGuard(tokio::spawn(async move {
            loop {
                let requested_at = Instant::now();
                let response = observe_rpc(
                    renewal_rpc_metrics.as_ref(),
                    dms_metrics::RpcCall::WORKER_HEARTBEAT,
                    worker.heartbeat(pb::HeartbeatRequest {
                        session_id,
                        released_view_through: renewal_releases.released_view_through(),
                    }),
                )
                .await;
                let pause = match response {
                    Ok(response) => {
                        let response = response.into_inner();
                        // 兼容携带失效的 heartbeat 响应，先处理失效再开放缓存。
                        for invalidation in response.current_invalidations {
                            if let Some(key) = invalidation.key {
                                renewal_cache.invalidate_current(
                                    &key.value,
                                    ObjectVersion(invalidation.minimum_version),
                                );
                            }
                        }
                        let ttl = Duration::from_millis(response.lease_ttl_millis);
                        renewal_cache.renew_lease(requested_at, ttl);
                        if ttl.is_zero() {
                            renewal_interval
                        } else {
                            renewal_interval.min(ttl / 3).max(Duration::from_millis(1))
                        }
                    }
                    Err(_) => {
                        renewal_cache.revoke_lease();
                        renewal_interval
                    }
                };
                tokio::time::sleep(pause).await;
            }
        }))
    });

    // spawn 创建独立 Tokio Task。connect() 返回后它继续运行；没有创建专用 OS 线程。
    tokio::spawn(async move {
        let cache_guard = cache_guard;
        let _renewal = renewal;
        let _connection_guard = connection_guard;
        let mut ticker = tokio::time::interval(options.heartbeat_interval);
        loop {
            // select! 同时等待“需要发 heartbeat”和“Node 发来事件”，谁先就绪先处理谁。
            tokio::select! {
                _ = ticker.tick() => {
                    if outbound_sender
                        .send(heartbeat(session_id, view_releases.released_view_through()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                message = inbound.message() => {
                    // Ok(None) 表示远端正常关闭 stream；Err 表示通信失败；两者都退出任务。
                    let Ok(Some(event)) = message else {
                        break;
                    };
                    // let-chain：必须同时是 invalidation 事件且其中携带 key 才执行失效。
                    if let Some(pb::node_session_event::Event::CurrentInvalidation(invalidation)) = event.event
                        && let Some(key) = invalidation.key
                    {
                        let evicted = cache.invalidate_current(
                            &key.value,
                            ObjectVersion(invalidation.minimum_version),
                        );
                        if let Some(metrics) = &metrics {
                            metrics.record_cache_invalidation(if evicted {
                                CacheInvalidation::Evicted
                            } else {
                                CacheInvalidation::Ignored
                            });
                        }
                    }
                    // 处理完成后把 event_sequence 回 ACK，Node 可据此推进事件游标。
                    let ack = pb::ClientSessionMessage {
                        session_id,
                        message: Some(pb::client_session_message::Message::EventAck(
                            pb::SessionEventAck {
                                event_sequence: event.event_sequence,
                            },
                        )),
                    };
                    if outbound_sender.send(ack).await.is_err() {
                        break;
                    }
                }
            }
        }
        // A closed Session stream removes the coherence guarantee. Drop every
        // Current entry before any reconnect/failover work is attempted.
        drop(cache_guard);
        log::warn!(
            "DMS node session disconnected; Current cache was cleared (session_id={session_id})"
        );
        if let Some(metrics) = &metrics {
            metrics.record_node_session_event(NodeSessionEvent::Disconnected);
        }
    });
    Ok(())
}

fn heartbeat(session_id: u64, released_view_through: Option<u64>) -> pb::ClientSessionMessage {
    // 纯构造函数：没有 I/O，只把领域参数编码成 protobuf DTO。
    pb::ClientSessionMessage {
        session_id,
        message: Some(pb::client_session_message::Message::Heartbeat(
            pb::SessionHeartbeat {
                released_view_through,
            },
        )),
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

pub(super) fn map_status(status: tonic::Status) -> DmsError {
    // DMS 对端会在 Status.details 放 ErrorDetail；普通代理/网络错误没有 detail，
    // 这时只能在 SDK 边界生成本地连接错误。
    status_to_dms_error_with(status, DmsError::client_connection_unavailable)
}

#[cfg(test)]
mod session_cache_tests {
    use super::*;

    #[test]
    fn range_response_checks_entire_layout_before_mapping_or_download() {
        let segment = |offset, length| pb::ReadSegment {
            logical_offset: offset,
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
        assert!(requested_read_length(6, Some(ByteRange { offset: 2, len: 1 })).is_ok());
        assert!(
            requested_read_length(
                6,
                Some(ByteRange {
                    offset: u64::MAX,
                    len: 2
                })
            )
            .is_err()
        );
    }

    #[test]
    fn cancellation_before_session_task_is_polled_fences_cache() {
        let cache = Arc::new(ClientCache::new(1024));
        cache.session_connected();
        cache.renew_lease(Instant::now(), Duration::from_secs(60));
        let key = Key::new(b"cancelled-session".to_vec()).unwrap();
        let token = cache.refill_token();
        let guard = SessionCacheGuard(Arc::clone(&cache));
        let task = async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        };
        drop(task);
        assert!(cache.refill_token().is_none());
        cache.insert_current(
            token,
            &key,
            super::super::client_cache::CachedValue {
                version: ObjectVersion(1),
                bytes: b"old".as_slice().into(),
            },
        );
        assert!(cache.get_current(&key).is_none());
    }
}

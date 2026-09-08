//! SDK 到一个 dms-node 的连接，以及这条连接上的长生命周期 Session stream。
//!
//! `connect_channel` 是唯一判断 `unix://` 与 `http(s)://` 的位置。连接建立后，
//! `set/get` 都只看到同一种 Tonic `Channel`，因此业务逻辑不需要 transport 分支。

// Session 后台 Task 与前台读操作共享释放水位，不共享跨请求 value 缓存。
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
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

use crate::{
    ByteRange, ClientTlsOptions, DeleteResult, DmsError, DurabilityPolicy, GetOptions, GetResult,
    HashDeleteOptions, HashEntriesResult, HashField, HashGetOptions, HashMultiGetResult,
    HashRangeWriteOptions, HashRangeWriteResult, HashReadVersion, HashScanOptions, HashScanResult,
    HashSetResult, HashValue, HashVersion, HashWriteMode, HashWriteOptions, Key, KeyVersion,
    MSetResult, ObjectVersion, OperationId, RangeWriteOptions, ReadVersion, ScanCursor, SetOptions,
    SetResult, WriteCondition,
};

use super::transfer_engine::{PayloadBuffer, TransferEngine};
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
    ) -> ReadProtection {
        ReadProtection {
            releases: Arc::clone(self),
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
    epochs: Vec<u64>,
}

impl Drop for ReadProtection {
    fn drop(&mut self) {
        for &epoch in &self.epochs {
            self.releases.mark_released(epoch);
        }
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
    buffer: PayloadBuffer,
    _protection: ReadProtection,
}

/// 长连接 Task 自己消费的两个运行参数，避免把 SDK 的整份配置带进后台任务。
struct SessionTaskOptions {
    heartbeat_interval: Duration,
    channel_capacity: usize,
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
        // 长连接负责 Session 存活与 View 释放水位；不申请 SDK value 缓存租约。
        start_session_task(
            worker,
            session_id,
            Arc::clone(&view_releases),
            SessionTaskOptions {
                heartbeat_interval: options.heartbeat_interval,
                channel_capacity: options.session_channel_capacity,
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
                max_inline_bytes: 0,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        self.decode_view_response(response, options.range).await
    }

    // 与普通响应解码一样：在任何可能失败的步骤前接管本次读取保护。
    async fn decode_view_response(
        &self,
        response: pb::GetResponse,
        range: Option<ByteRange>,
    ) -> Result<Option<SharedViewInner>, DmsError> {
        // 即使不是单段、协议校验失败或 FD 获取失败，也要释放整份响应的借用。
        let protection = self.view_releases.protect(response.segments.iter());
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
        let expected_length = requested_read_length(response.logical_length, range)?;
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
        // 小 TCP 值可以内联；其它情况由相同响应带回 payload 位置。
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
                // inline_threshold_bytes 同时控制 SET 小对象直写与 GET 小对象合并响应；
                // SDK 仍按协议上限截断，避免配置误把过大 bytes 塞进控制响应。
                max_inline_bytes: self.inline_read_budget() as u64,
            }),
        )
        .await
        .map_err(map_status)?
        .into_inner();
        self.decode_read_response(response, options.range, self.inline_read_budget())
            .await
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
        self.decode_mget_response(response).await
    }

    async fn decode_mget_response(
        &self,
        response: pb::MGetResponse,
    ) -> Result<Vec<Option<GetResult>>, DmsError> {
        // 前面的 item 失败时，后面的 item 尚未解码也已由 Node 发放借用。
        // 整批保护直到处理结束，不能只在逐 item 成功后才记录释放。
        let _batch_protection = self
            .view_releases
            .protect(response.items.iter().flat_map(|item| item.segments.iter()));
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
        self.decode_read_response(response, None, 0).await
    }

    async fn decode_read_response(
        &self,
        response: pb::GetResponse,
        range: Option<ByteRange>,
        max_inline_bytes: usize,
    ) -> Result<Option<GetResult>, DmsError> {
        // 普通 GET 最终交付 owned Vec。复制、验证、映射失败及取消都通过 Drop
        // 归还已收到的共享读 epoch；这里不保留跨请求 Buffer/value。
        let _protection = self.view_releases.protect(response.segments.iter());
        if !response.found {
            reject_inline_on_miss(&response)?;
            return Ok(None);
        }
        let expected_length = requested_read_length(response.logical_length, range)?;
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
            // download 已经返回独立拥有的 Vec；大首段可直接成为结果，避免
            // 再分配同样大小的 Vec 并复制一次。后续 Extent 仍按顺序追加，
            // 不借用 SHM 页，也不改变普通 GET 的所有权或版本校验。
            if bytes.is_empty() && part.len() >= 32 * 1024 * 1024 {
                bytes = part;
            } else {
                bytes.extend_from_slice(&part);
            }
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
    options: SessionTaskOptions,
    metrics: Option<ClientMetrics>,
    rpc_metrics: Option<&dms_metrics::RpcMetrics>,
) -> Result<(), DmsError> {
    let (outbound_sender, outbound_receiver) = mpsc::channel(options.channel_capacity);
    // 第一条心跳标识 session，后续心跳只维持生命周期并归还连续 View 水位。
    outbound_sender
        .send(heartbeat(session_id, view_releases.released_view_through()))
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
    tokio::spawn(async move {
        let _connection_guard = connection_guard;
        let mut ticker = tokio::time::interval(options.heartbeat_interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if outbound_sender
                        .send(heartbeat(session_id, view_releases.released_view_through()))
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
mod read_lifecycle_tests {
    use super::*;

    fn disconnected_node(releases: &Arc<ViewReleaseTracker>) -> NodeConnection {
        let channel = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        NodeConnection {
            worker: WorkerServiceClient::new(dms_tracing::traced_channel(channel.clone())),
            transfer: TransferEngine::new(channel, 1, None, None, None),
            rpc_metrics: None,
            session_id: 1,
            inline_threshold_bytes: 0,
            view_releases: Arc::clone(releases),
        }
    }

    fn shared_response(epoch: u64) -> pb::GetResponse {
        pb::GetResponse {
            found: true,
            version: 1,
            logical_length: 4,
            segments: vec![pb::ReadSegment {
                logical_offset: 0,
                target: Some(pb::PayloadTarget {
                    target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                        view_epoch: Some(epoch),
                        length: 4,
                        ..Default::default()
                    })),
                }),
            }],
            inline_value: None,
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
            assert!(connection.decode_mget_response(batch).await.is_err());
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
            assert!(connection.decode_view_response(multi, None).await.is_err());
            assert_eq!(releases.released_view_through(), Some(1));
            assert!(
                connection
                    .decode_view_response(shared_response(2), None)
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
            let releases = Arc::new(ViewReleaseTracker::default());
            let connection = NodeConnection {
                worker: WorkerServiceClient::new(dms_tracing::traced_channel(channel.clone())),
                transfer: TransferEngine::new(channel, 1, None, None, None),
                rpc_metrics: None,
                session_id: 1,
                inline_threshold_bytes: 0,
                view_releases: Arc::clone(&releases),
            };
            let response = pb::GetResponse {
                found: true,
                version: 1,
                logical_length: 4,
                segments: vec![pb::ReadSegment {
                    logical_offset: 0,
                    target: Some(pb::PayloadTarget {
                        target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                            view_epoch: Some(1),
                            length: 4,
                            ..Default::default()
                        })),
                    }),
                }],
                inline_value: None,
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
                    target: Some(pb::PayloadTarget { target: None }),
                }],
                0
            )
            .is_err()
        );
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
        };
        assert!(reject_inline_on_miss(&missing_with_bytes).is_err());

        let hit_with_bytes = pb::GetResponse {
            found: true,
            version: 9,
            logical_length: 3,
            segments: Vec::new(),
            inline_value: Some(b"abc".to_vec()),
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
            target: Some(pb::PayloadTarget {
                target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
                    view_epoch: Some(epoch),
                    ..Default::default()
                })),
            }),
        };
        let live_view = releases.protect([segment(1)].iter());
        let cancelled = releases.protect([segment(2), segment(2), segment(3)].iter());
        let task = async move {
            let _guard = cancelled;
            std::future::pending::<()>().await;
        };
        drop(task);
        assert_eq!(releases.released_view_through(), None);
        drop(live_view);
        assert_eq!(releases.released_view_through(), Some(3));
        // 模拟响应4未到达；不能因为5已释放就越过未知的4。
        drop(releases.protect([segment(5)].iter()));
        assert_eq!(releases.released_view_through(), Some(3));
        drop(releases.protect([segment(4)].iter()));
        assert_eq!(releases.released_view_through(), Some(5));
    }
}

//! Payload transport boundary used by SDK business operations.
//!
//! `NodeConnection` receives a transport-neutral `PayloadTarget` from the
//! Worker control service. This module is the only place that selects the SHM,
//! gRPC, RDMA, or UB provider. Therefore SET/GET orchestration never contains
//! provider-specific `if/else` branches.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use dms_protocol::v1 as pb;
use dms_shm::{BrokerToken, FdBrokerClient, FdRequest, MappedRegion};
use dms_tracing::tracing::Instrument as _;
use pb::worker_payload_service_client::WorkerPayloadServiceClient;
use pb::worker_service_client::WorkerServiceClient;
use tonic::transport::Channel;

use crate::DmsError;
use crate::metrics::{
    ClientMetrics, RegionMappingGuard, RegionMappingLookup, TransferDirection, TransferProvider,
};

#[derive(Clone)]
pub(crate) struct TransferEngine {
    grpc: WorkerPayloadServiceClient<dms_tracing::TracedChannel>,
    worker: WorkerServiceClient<dms_tracing::TracedChannel>,
    session_id: u64,
    fd_broker_path: Option<PathBuf>,
    mappings: Arc<RegionMappingCache>,
    metrics: Option<ClientMetrics>,
    rpc_metrics: Option<dms_metrics::RpcMetrics>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RegionMappingKey {
    region_id: u64,
}

/// SDK-side logical index over process-wide mmap objects.
///
/// Bytes live only in the `MappedRegion`; this cache does not duplicate object
/// payload. The key is scoped by one `NodeConnection`, so node identity is
/// implicit and the complete identity is one NodeConnection incarnation + Region.
#[derive(Default)]
struct RegionMappingCache {
    entries: Mutex<HashMap<RegionMappingKey, Arc<CachedMapping>>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

pub(crate) struct CachedMapping {
    region: MappedRegion,
    // 随最后一个 view/cache 引用释放，而不是随 cache 表项删除。
    _metric: Option<RegionMappingGuard>,
}

pub(crate) enum PayloadBuffer {
    Shm {
        descriptor: pb::ShmDescriptor,
        mapping: Arc<CachedMapping>,
    },
}

impl PayloadBuffer {
    pub(crate) fn as_slice(&self) -> Result<&[u8], DmsError> {
        match self {
            Self::Shm {
                descriptor,
                mapping,
            } => shm_slice(mapping, descriptor),
        }
    }

    pub(crate) fn as_mut_slice(&mut self) -> Result<&mut [u8], DmsError> {
        match self {
            Self::Shm {
                descriptor,
                mapping,
            } => shm_mut_slice(mapping, descriptor),
        }
    }

    pub(crate) fn receipt(&self) -> Result<pb::TransferReceipt, DmsError> {
        match self {
            Self::Shm { descriptor, .. } => {
                let bytes = self.as_slice()?;
                Ok(pb::TransferReceipt {
                    transfer_id: descriptor.transfer_id.clone(),
                    length: descriptor.length,
                    digest: digest(bytes),
                    target_allocation_id: descriptor.allocation_id,
                })
            }
        }
    }
}

impl TransferEngine {
    pub(crate) fn new(
        channel: Channel,
        session_id: u64,
        fd_broker_path: Option<String>,
        metrics: Option<ClientMetrics>,
        rpc_metrics: Option<dms_metrics::RpcMetrics>,
    ) -> Self {
        Self {
            grpc: WorkerPayloadServiceClient::new(dms_tracing::traced_channel(channel.clone())),
            worker: WorkerServiceClient::new(dms_tracing::traced_channel(channel)),
            session_id,
            fd_broker_path: fd_broker_path.map(PathBuf::from),
            mappings: Arc::new(RegionMappingCache::default()),
            metrics,
            rpc_metrics,
        }
    }

    /// Writes bytes into a target prepared by dms-node and returns its receipt.
    pub(crate) async fn upload(
        &self,
        session_id: u64,
        target: pb::PayloadTarget,
        value: &[u8],
    ) -> Result<pb::TransferReceipt, DmsError> {
        self.upload_candidates(session_id, vec![target], value)
            .await
    }

    async fn upload_candidates(
        &self,
        session_id: u64,
        targets: Vec<pb::PayloadTarget>,
        value: &[u8],
    ) -> Result<pb::TransferReceipt, DmsError> {
        let mut last_retryable_failure = None;
        for target in targets {
            match self.upload_one(session_id, target, value).await {
                Ok(receipt) => return Ok(receipt),
                Err(error) if is_recoverable_provider_failure(&error) => {
                    log::warn!(
                        "DMS payload provider failed; trying the next candidate: {}",
                        error
                    );
                    last_retryable_failure = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_retryable_failure.unwrap_or_else(|| {
            DmsError::client_protocol_violation(
                "payload transfer plan has no candidate target".to_string(),
            )
        }))
    }

    async fn upload_one(
        &self,
        session_id: u64,
        target: pb::PayloadTarget,
        value: &[u8],
    ) -> Result<pb::TransferReceipt, DmsError> {
        let provider = provider_name(&target);
        // 未启用宿主 Subscriber 时不构造带字段的禁用 Span，防止 log 回退。
        let span = if dms_tracing::tracing::level_filters::LevelFilter::current()
            >= dms_tracing::tracing::Level::INFO
        {
            dms_tracing::tracing::info_span!(
                "dms.payload.upload",
                otel.kind = "internal",
                direction = TransferDirection::Write.label(),
                provider = provider.label(),
                bytes = value.len() as u64,
                result = dms_tracing::tracing::field::Empty,
            )
        } else {
            dms_tracing::tracing::Span::none()
        };
        let mut transfer = self
            .metrics
            .as_ref()
            .map(|metrics| metrics.begin_transfer(TransferDirection::Write, provider));
        let result = async move {
            match target.target {
                Some(pb::payload_target::Target::Grpc(target)) => {
                    if target.length != value.len() as u64 {
                        return Err(DmsError::client_protocol_violation(
                            "payload target length does not match value".to_string(),
                        ));
                    }
                    let mut client = self.grpc.clone();
                    let mut rpc = self.rpc_metrics.as_ref().map(|metrics| {
                        metrics.begin_client_call(dms_metrics::RpcCall::PAYLOAD_UPLOAD)
                    });
                    let response = client
                        .upload(pb::UploadPayloadRequest {
                            transfer_id: target.transfer_id,
                            payload: value.to_vec(),
                            nonce: target.nonce,
                        })
                        .await;
                    if response.is_ok()
                        && let Some(rpc) = &mut rpc
                    {
                        rpc.success();
                    }
                    response
                        .map_err(super::node_connection::map_status)?
                        .into_inner()
                        .receipt
                        .ok_or_else(|| {
                            DmsError::client_protocol_violation(
                                "missing transfer receipt".to_string(),
                            )
                        })
                }
                Some(pb::payload_target::Target::Shm(target)) => {
                    self.upload_shm(session_id, target, value).await
                }
                Some(pb::payload_target::Target::Rdma(_)) => {
                    Err(DmsError::node_transfer_unsupported(
                        "RDMA provider is not enabled by this SDK build".to_string(),
                    ))
                }
                Some(pb::payload_target::Target::Ub(_)) => {
                    Err(DmsError::node_transfer_unsupported(
                        "UB provider is not enabled by this SDK build".to_string(),
                    ))
                }
                None => Err(DmsError::client_protocol_violation(
                    "empty payload target".to_string(),
                )),
            }
        }
        .instrument(span.clone())
        .await;
        if result.is_ok()
            && let Some(transfer) = &mut transfer
        {
            transfer.success(value.len());
        }
        match &result {
            Ok(_) => dms_tracing::record_ok(&span),
            Err(error) => dms_tracing::record_error(&span, error),
        }
        result
    }

    /// 读取一个目标并交付 owned bytes；SHM 复用映射，但普通 GET 在此复制。
    /// 读取保护由调用方覆盖整份响应，不由传输后端决定何时释放。
    pub(crate) async fn download(
        &self,
        session_id: u64,
        target: pb::PayloadTarget,
    ) -> Result<Vec<u8>, DmsError> {
        let provider = provider_name(&target);
        let expected_bytes = payload_length(&target);
        // 与 upload 相同：只读宿主级别提示，不接管日志或 Subscriber。
        let span = if dms_tracing::tracing::level_filters::LevelFilter::current()
            >= dms_tracing::tracing::Level::INFO
        {
            dms_tracing::tracing::info_span!(
                "dms.payload.download",
                otel.kind = "internal",
                direction = TransferDirection::Read.label(),
                provider = provider.label(),
                bytes = expected_bytes,
                result = dms_tracing::tracing::field::Empty,
            )
        } else {
            dms_tracing::tracing::Span::none()
        };
        let mut transfer = self
            .metrics
            .as_ref()
            .map(|metrics| metrics.begin_transfer(TransferDirection::Read, provider));
        let result = async move {
            match target.target {
                Some(pb::payload_target::Target::Grpc(target)) => {
                    let mut client = self.grpc.clone();
                    let mut rpc = self.rpc_metrics.as_ref().map(|metrics| {
                        metrics.begin_client_call(dms_metrics::RpcCall::PAYLOAD_DOWNLOAD)
                    });
                    let response = client
                        .download(pb::DownloadPayloadRequest {
                            transfer_id: target.transfer_id,
                            nonce: target.nonce,
                        })
                        .await;
                    if response.is_ok()
                        && let Some(rpc) = &mut rpc
                    {
                        rpc.success();
                    }
                    let bytes = response
                        .map_err(super::node_connection::map_status)?
                        .into_inner()
                        .payload;
                    if bytes.len() as u64 != target.length {
                        return Err(DmsError::node_transfer_corrupt_data("DMS data is corrupt"));
                    }
                    Ok(bytes)
                }
                Some(pb::payload_target::Target::Shm(target)) => {
                    self.download_shm(session_id, target).await
                }
                Some(pb::payload_target::Target::Rdma(_)) => {
                    Err(DmsError::node_transfer_unsupported(
                        "RDMA provider is not enabled by this SDK build".to_string(),
                    ))
                }
                Some(pb::payload_target::Target::Ub(_)) => {
                    Err(DmsError::node_transfer_unsupported(
                        "UB provider is not enabled by this SDK build".to_string(),
                    ))
                }
                None => Err(DmsError::client_protocol_violation(
                    "empty payload target".to_string(),
                )),
            }
        }
        .instrument(span.clone())
        .await;
        if let Ok(bytes) = &result
            && let Some(transfer) = &mut transfer
        {
            transfer.success(bytes.len());
        }
        match &result {
            Ok(_) => dms_tracing::record_ok(&span),
            Err(error) => dms_tracing::record_error(&span, error),
        }
        result
    }

    pub(crate) async fn map_target(
        &self,
        session_id: u64,
        target: pb::PayloadTarget,
    ) -> Result<PayloadBuffer, DmsError> {
        match target.target {
            Some(pb::payload_target::Target::Shm(target)) => {
                self.require_session(session_id)?;
                let mapping = self.mapping_for(&target).await?;
                Ok(PayloadBuffer::Shm {
                    descriptor: target,
                    mapping,
                })
            }
            Some(pb::payload_target::Target::Grpc(_)) => Err(DmsError::node_transfer_unsupported(
                "explicit mapped payload requires local shared-memory negotiation".to_string(),
            )),
            Some(pb::payload_target::Target::Rdma(_)) => Err(DmsError::node_transfer_unsupported(
                "RDMA mapped provider is not enabled by this SDK build".to_string(),
            )),
            Some(pb::payload_target::Target::Ub(_)) => Err(DmsError::node_transfer_unsupported(
                "UB mapped provider is not enabled by this SDK build".to_string(),
            )),
            None => Err(DmsError::client_protocol_violation(
                "empty payload target".to_string(),
            )),
        }
    }

    async fn upload_shm(
        &self,
        session_id: u64,
        target: pb::ShmDescriptor,
        value: &[u8],
    ) -> Result<pb::TransferReceipt, DmsError> {
        self.require_session(session_id)?;
        if target.length != value.len() as u64 {
            return Err(DmsError::client_protocol_violation(
                "SHM target length does not match value".to_string(),
            ));
        }
        let mapping = self.mapping_for(&target).await?;
        let offset = shm_offset(&target)?;
        if target.view_epoch.is_some() {
            return Err(DmsError::client_protocol_violation(
                "read-only SHM target used for upload",
            ));
        }
        // SAFETY: Node 授予一次独占 staging；此请求不公开引用，复制结束才发送 receipt。
        // 超时/取消的导出 allocation 由 Node 隔离，不复用给其它对象。
        unsafe {
            mapping
                .region
                .staging_slice_mut(offset, shm_length(&target)?)
        }
        .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?
        .copy_from_slice(value);
        Ok(pb::TransferReceipt {
            transfer_id: target.transfer_id,
            length: target.length,
            digest: digest(value),
            target_allocation_id: target.allocation_id,
        })
    }

    async fn download_shm(
        &self,
        session_id: u64,
        target: pb::ShmDescriptor,
    ) -> Result<Vec<u8>, DmsError> {
        self.require_session(session_id)?;
        let mapping = self.mapping_for(&target).await?;
        mapping
            .region
            .read_at(shm_offset(&target)?, shm_length(&target)?)
            .map_err(|error| DmsError::client_protocol_violation(error.to_string()))
    }

    async fn mapping_for(
        &self,
        target: &pb::ShmDescriptor,
    ) -> Result<Arc<CachedMapping>, DmsError> {
        let key = RegionMappingKey {
            region_id: target.region_id,
        };
        if let Some(mapping) = self.mappings.get(key)? {
            if let Some(metrics) = &self.metrics {
                metrics.record_region_mapping_lookup(RegionMappingLookup::Hit);
            }
            return Ok(mapping);
        }
        if let Some(metrics) = &self.metrics {
            metrics.record_region_mapping_lookup(RegionMappingLookup::Miss);
        }

        let broker_path = self.fd_broker_path.clone().ok_or_else(|| {
            DmsError::client_protocol_violation(
                "SHM target received without negotiated broker path".to_string(),
            )
        })?;
        let mut worker = self.worker.clone();
        let mut rpc = self
            .rpc_metrics
            .as_ref()
            .map(|metrics| metrics.begin_client_call(dms_metrics::RpcCall::WORKER_ACQUIRE_REGION));
        let response = worker
            .acquire_region(pb::AcquireRegionRequest {
                session_id: self.session_id,
                region_id: key.region_id,
            })
            .await;
        if response.is_ok()
            && let Some(rpc) = &mut rpc
        {
            rpc.success();
        }
        let grant = response
            .map_err(super::node_connection::map_status)?
            .into_inner();
        if grant.region_id != key.region_id || grant.region_length == 0 {
            return Err(DmsError::client_protocol_violation(
                "AcquireRegion returned a mismatched descriptor".to_string(),
            ));
        }
        let token = BrokerToken::new(grant.fd_token)
            .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?;
        let request = FdRequest::new(token, self.session_id, key.region_id);
        let region_length = usize::try_from(grant.region_length).map_err(|_| {
            DmsError::client_protocol_violation("SHM Region length is too large".to_string())
        })?;
        // SCM_RIGHTS uses blocking std Unix sockets. Keep that syscall boundary
        // off Tokio workers; this branch runs only on a Region cache miss.
        let metrics = self.metrics.clone();
        let mapping = tokio::task::spawn_blocking(move || {
            let fd = FdBrokerClient::request_fd(broker_path, &request)
                .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?;
            // SAFETY: 仅映射本 Session 授权的 Node memfd。Node 不截短 Region；
            // 已发布块不变，导出后失效的 allocation 隔离；SDK 不暴露 descriptor/mapping。
            let region = unsafe { MappedRegion::map(fd, region_length) }
                .map_err(|error| DmsError::client_protocol_violation(error.to_string()))?;
            Ok::<_, DmsError>(Arc::new(CachedMapping {
                region,
                _metric: metrics.as_ref().map(ClientMetrics::region_mapping_guard),
            }))
        })
        .await
        .map_err(|error| {
            DmsError::client_protocol_violation(format!("Region mapping task failed: {error}"))
        })??;
        let (mapping, _) = self.mappings.insert_or_get(key, mapping)?;
        Ok(mapping)
    }

    fn require_session(&self, session_id: u64) -> Result<(), DmsError> {
        if session_id == self.session_id {
            Ok(())
        } else {
            Err(DmsError::client_protocol_violation(
                "payload target belongs to another session".to_string(),
            ))
        }
    }
}

impl RegionMappingCache {
    fn get(&self, key: RegionMappingKey) -> Result<Option<Arc<CachedMapping>>, DmsError> {
        let mapping = self
            .entries
            .lock()
            .map_err(|_| {
                DmsError::client_protocol_violation("Region mapping cache is poisoned".to_string())
            })?
            .get(&key)
            .cloned();
        if mapping.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        Ok(mapping)
    }

    fn insert_or_get(
        &self,
        key: RegionMappingKey,
        mapping: Arc<CachedMapping>,
    ) -> Result<(Arc<CachedMapping>, bool), DmsError> {
        let mut entries = self.entries.lock().map_err(|_| {
            DmsError::client_protocol_violation("Region mapping cache is poisoned".to_string())
        })?;
        if let Some(existing) = entries.get(&key) {
            return Ok((Arc::clone(existing), false));
        }
        entries.insert(key, Arc::clone(&mapping));
        Ok((mapping, true))
    }
}

fn provider_name(target: &pb::PayloadTarget) -> TransferProvider {
    match &target.target {
        Some(pb::payload_target::Target::Grpc(_)) => TransferProvider::Grpc,
        Some(pb::payload_target::Target::Shm(_)) => TransferProvider::Shm,
        Some(pb::payload_target::Target::Rdma(_)) => TransferProvider::Rdma,
        Some(pb::payload_target::Target::Ub(_)) => TransferProvider::Ub,
        None => TransferProvider::Unknown,
    }
}

fn payload_length(target: &pb::PayloadTarget) -> u64 {
    match &target.target {
        Some(pb::payload_target::Target::Grpc(target)) => target.length,
        Some(pb::payload_target::Target::Shm(target)) => target.length,
        Some(pb::payload_target::Target::Rdma(target)) => target.length,
        Some(pb::payload_target::Target::Ub(target)) => target.length,
        None => 0,
    }
}

fn shm_offset(target: &pb::ShmDescriptor) -> Result<usize, DmsError> {
    usize::try_from(target.offset)
        .map_err(|_| DmsError::client_protocol_violation("SHM offset is too large".to_string()))
}

fn shm_length(target: &pb::ShmDescriptor) -> Result<usize, DmsError> {
    usize::try_from(target.length)
        .map_err(|_| DmsError::client_protocol_violation("SHM length is too large".to_string()))
}

fn shm_slice<'a>(
    mapping: &'a Arc<CachedMapping>,
    target: &pb::ShmDescriptor,
) -> Result<&'a [u8], DmsError> {
    // SAFETY: read view 只引用已发布不变块；staging 的只读借用与其 &mut buffer 互斥。
    unsafe {
        mapping
            .region
            .as_slice(shm_offset(target)?, shm_length(target)?)
    }
    .map_err(|error| DmsError::client_protocol_violation(error.to_string()))
}

fn shm_mut_slice<'a>(
    mapping: &'a mut Arc<CachedMapping>,
    target: &pb::ShmDescriptor,
) -> Result<&'a mut [u8], DmsError> {
    if target.view_epoch.is_some() {
        return Err(DmsError::client_protocol_violation(
            "read-only SHM view is not writable",
        ));
    }
    // SAFETY: PayloadBuffer 不可克隆，调用借用绑定到唯一 buffer 的 &mut；
    // SDK 不公开其它写引用。Node 对失效导出 allocation 的隔离保证跨 TTL 不复用。
    unsafe {
        mapping
            .region
            .staging_slice_mut(shm_offset(target)?, shm_length(target)?)
    }
    .map_err(|error| DmsError::client_protocol_violation(error.to_string()))
}

fn digest(bytes: &[u8]) -> Vec<u8> {
    dms_transport::checksum::fnv1a_bytes(bytes).to_vec()
}

fn is_recoverable_provider_failure(error: &DmsError) -> bool {
    error.code() == dms_error::NODE_TRANSFER_UNSUPPORTED
        || error.code() == dms_error::CLIENT_CONNECTION_UNAVAILABLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_shm_view_cannot_be_borrowed_as_a_write_buffer() {
        let region = dms_shm::SharedRegion::create("dms-sdk-read-view", 4096).unwrap();
        // SAFETY: 测试独占 backing，借用结束前不会写入或缩短它。
        let mapping = unsafe { MappedRegion::map(region.duplicate_fd().unwrap(), 4096) }.unwrap();
        let mut buffer = PayloadBuffer::Shm {
            descriptor: pb::ShmDescriptor {
                region_id: 1,
                offset: 0,
                length: 4,
                allocation_id: 1,
                view_epoch: Some(1),
                transfer_id: Vec::new(),
            },
            mapping: Arc::new(CachedMapping {
                region: mapping,
                _metric: None,
            }),
        };
        assert!(buffer.as_mut_slice().is_err());
        assert_eq!(buffer.as_slice().unwrap(), &[0; 4]);
    }

    #[test]
    fn mapped_region_gauge_follows_the_last_view_not_the_cache_entry() {
        let registry = dms_metrics::registry();
        let metrics = ClientMetrics::register(&registry).unwrap();
        let region = dms_shm::SharedRegion::create("dms-sdk-view-lifetime", 4096).unwrap();
        // SAFETY: 测试只读，region 在所有引用释放后才退出作用域。
        let mapped = unsafe { MappedRegion::map(region.duplicate_fd().unwrap(), 4096) }.unwrap();
        let mapping = Arc::new(CachedMapping {
            region: mapped,
            _metric: Some(metrics.region_mapping_guard()),
        });
        let cache = RegionMappingCache::default();
        cache
            .insert_or_get(RegionMappingKey { region_id: 1 }, Arc::clone(&mapping))
            .unwrap();
        let view = PayloadBuffer::Shm {
            descriptor: pb::ShmDescriptor {
                region_id: 1,
                length: 4,
                view_epoch: Some(1),
                ..Default::default()
            },
            mapping,
        };
        drop(cache);
        assert!(
            dms_metrics::encode_text(&registry)
                .unwrap()
                .contains("dms_client_region_mappings 1")
        );
        drop(view);
        assert!(
            dms_metrics::encode_text(&registry)
                .unwrap()
                .contains("dms_client_region_mappings 0")
        );
    }

    fn unsupported_target() -> pb::PayloadTarget {
        pb::PayloadTarget {
            target: Some(pb::payload_target::Target::Rdma(pb::RdmaTarget {
                descriptor: Vec::new(),
                transfer_id: b"rdma".to_vec(),
                length: 4,
            })),
        }
    }

    #[test]
    fn unsupported_provider_is_recoverable_inside_transfer_plan() {
        let error = DmsError::node_transfer_unsupported("RDMA unavailable in this SDK build");
        assert!(is_recoverable_provider_failure(&error));
    }

    #[tokio::test]
    async fn empty_candidate_plan_is_protocol_error() {
        let channel = Channel::from_static("http://127.0.0.1:1").connect_lazy();
        let engine = TransferEngine::new(channel, 7, None, None, None);
        let error = engine
            .upload_candidates(7, Vec::new(), b"data")
            .await
            .expect_err("empty plan fails");
        assert_eq!(error.code(), dms_error::CLIENT_PROTOCOL_VIOLATION);
    }

    #[tokio::test]
    async fn all_recoverable_candidates_fail_with_terminal_transfer_error() {
        let channel = Channel::from_static("http://127.0.0.1:1").connect_lazy();
        let engine = TransferEngine::new(channel, 7, None, None, None);
        let error = engine
            .upload_candidates(7, vec![unsupported_target()], b"data")
            .await
            .expect_err("unsupported provider fails after all candidates");
        assert_eq!(error.code(), dms_error::NODE_TRANSFER_UNSUPPORTED);
    }
}

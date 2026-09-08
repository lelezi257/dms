//! Client→Node 的 generated gRPC Service Handler。
//!
//! Tonic 负责 HTTP/2 路由和 protobuf DTO。本层只校验 wire 字段、通过
//! [`NodeHandle`] 投递业务命令，并把领域事件编码回 protobuf；不包含 UDS/TCP 分支，
//! 也不直接修改 Worker 状态。

// Pin 保证 Stream Future 在内存中的地址不再移动，是 async Stream trait object 的要求。
use std::pin::Pin;

use dms_protocol::v1 as pb;
use dms_transport::dms_error_to_status;
// 这两个 Trait 由 tonic-build 根据 `.proto service` 自动生成；我们必须实现其方法。
use pb::worker_payload_service_server::WorkerPayloadService;
use pb::worker_service_server::WorkerService;
use tokio::sync::mpsc;
// StreamExt 提供 `.next().await`；ReceiverStream 把 mpsc Receiver 适配为 gRPC stream。
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::{Request, Response, Status};

use super::arena_manager::{HostAllocationTarget, HostShmDescriptor};
use super::kkv_operations::{KkvOperations, KkvValue};
use super::runtime::{NodeEvent, NodeHandle, ReadTarget, SetRangeInput, WorkerError};

#[derive(Clone)]
pub(crate) struct WorkerServiceHandler {
    // Handle clone 只增加一个 mailbox Sender，不复制 NodeState。
    node: NodeHandle,
    // 只有本地 UDS listener 可以把 memfd broker 暴露给 SDK；TCP/远端 session
    // 即使请求 shared_memory，也必须降级为 gRPC payload target。
    local_shared_memory: bool,
    rpc_metrics: dms_metrics::RpcMetrics,
    error_metrics: dms_metrics::ErrorMetrics,
}

impl WorkerServiceHandler {
    /// 依赖由进程组合根创建并注入，Handler 不自行创建业务状态。
    #[cfg(test)]
    pub(crate) fn new(node: NodeHandle, local_shared_memory: bool) -> Self {
        let registry = dms_metrics::registry();
        let rpc_metrics = dms_metrics::RpcMetrics::register(&registry)
            .expect("test Worker RPC metrics registration");
        let error_metrics = dms_metrics::ErrorMetrics::register(&registry)
            .expect("test Worker error metrics registration");
        Self::with_metrics(node, local_shared_memory, rpc_metrics, error_metrics)
    }

    pub(crate) fn with_metrics(
        node: NodeHandle,
        local_shared_memory: bool,
        rpc_metrics: dms_metrics::RpcMetrics,
        error_metrics: dms_metrics::ErrorMetrics,
    ) -> Self {
        Self {
            node,
            local_shared_memory,
            rpc_metrics,
            error_metrics,
        }
    }

    fn map_worker_error(&self, error: WorkerError) -> Status {
        let error = super::runtime::worker_error_to_dms(error);
        self.error_metrics
            .record_if_component(dms_metrics::ErrorComponent::Node, &error);
        dms_error_to_status(error)
    }

    async fn handle_client_session_message(
        &self,
        message: pb::ClientSessionMessage,
    ) -> Result<(), Status> {
        handle_client_session_message(&self.node, message).await
    }
}

#[tonic::async_trait]
impl WorkerService for WorkerServiceHandler {
    // `.proto` 的 streaming response 在 Rust 中需要给出具体关联类型。
    // Box<dyn Stream> 隐藏 ReceiverStream 的具体实现，Pin 保证可安全轮询。
    type SessionStream =
        Pin<Box<dyn Stream<Item = Result<pb::NodeSessionEvent, Status>> + Send + 'static>>;

    async fn open_session(
        &self,
        request: Request<pb::OpenSessionRequest>,
    ) -> Result<Response<pb::OpenSessionResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_OPEN_SESSION);
        let request = request.into_inner();
        let shared_memory = self.local_shared_memory
            && request.shared_memory
            && request.zero_copy_read
            && request.zero_copy_write;
        // 真正 session_id 由 Node 状态 owner 串行分配。
        let session_id = self
            .node
            .open_session(shared_memory)
            .await
            .map_err(|error| self.map_worker_error(error))?;
        // Response::new 把 protobuf body 包装成 Tonic Response，后者还可携带 metadata。
        rpc.success();
        Ok(Response::new(pb::OpenSessionResponse {
            service_version: 1,
            session_id,
            initial_view_epoch: 1,
            lease_generation: 1,
            lease_ttl_millis: 30_000,
            shm: shared_memory
                .then(|| self.node.fd_broker_path())
                .flatten()
                .map(|path| pb::ShmCapability {
                    fd_broker_path: path.display().to_string(),
                }),
        }))
    }

    async fn session(
        &self,
        request: Request<tonic::Streaming<pb::ClientSessionMessage>>,
    ) -> Result<Response<Self::SessionStream>, Status> {
        // Streaming RPC 指标统计建流阶段；连接存活数由 Node session Gauge 表达。
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_SESSION);
        // into_inner 消费 Request wrapper，取出 Client→Node 的入站消息流。
        let mut inbound = request.into_inner();
        // 第一条消息用于识别 Session；没有首消息的空 stream 属于协议错误。
        let first = inbound
            .message()
            .await?
            .ok_or_else(|| node_invalid_argument("session stream requires a first message"))?;
        let session_id = first.session_id;
        // 先让 Worker 验证首消息中的 session_id，再继续建立下行事件通道。
        self.handle_client_session_message(first).await?;

        // domain channel 承载 NodeEvent，不依赖 protobuf；生产者保存在 NodeState。
        let (domain_sender, mut domain_receiver) = mpsc::channel::<NodeEvent>(64);
        self.node
            .attach_session(session_id, domain_sender)
            .await
            .map_err(map_worker_error)?;
        // wire channel 的 item 类型被后续 send 推导为 Result<NodeSessionEvent, Status>。
        let (wire_sender, wire_receiver) = mpsc::channel(64);
        // 后台 Task 需要拥有 Handle，因此 clone 一个 mailbox producer。
        let node = self.node.clone();

        // Task 1：持续消费 Client 上行 heartbeat/ACK，并异步投递到 Node 状态 owner。
        tokio::spawn(async move {
            while let Some(message) = inbound.next().await {
                // gRPC stream 单条解码失败就结束该方向，避免在损坏流上继续处理。
                let Ok(message) = message else {
                    break;
                };
                if handle_client_session_message(&node, message).await.is_err() {
                    break;
                }
            }
            let _ = node.close_session(session_id).await;
        });

        // Task 2：把领域事件转换为 protobuf 并送进返回给 Client 的 stream。
        tokio::spawn(async move {
            while let Some(event) = domain_receiver.recv().await {
                // Client 断开后 Receiver 被丢弃，send 返回 Err，于是退出 Task。
                if wire_sender.send(Ok(encode_event(event))).await.is_err() {
                    break;
                }
            }
        });

        // Handler 此时返回的是 Stream 对象而不是所有事件；Tonic 后续逐条 poll 它。
        rpc.success();
        Ok(Response::new(Box::pin(ReceiverStream::new(wire_receiver))))
    }

    async fn heartbeat(
        &self,
        request: Request<pb::HeartbeatRequest>,
    ) -> Result<Response<pb::HeartbeatResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_HEARTBEAT);
        // get_ref 只借用 Request body；此方法不需要取得 DTO 所有权。
        let lease_ttl_millis = self
            .node
            .renew_cache_lease(
                request.get_ref().session_id,
                request.get_ref().released_view_through,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::HeartbeatResponse {
            lease_generation: 1,
            lease_ttl_millis,
            released_view_through: request.get_ref().released_view_through,
            current_invalidations: Vec::new(),
            route_epoch: None,
        }))
    }

    async fn allocate_staging(
        &self,
        request: Request<pb::AllocateStagingRequest>,
    ) -> Result<Response<pb::AllocateStagingResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_ALLOCATE_STAGING);
        // 后面要多次移动字段，所以直接消费 wrapper 取得 owned DTO。
        let request = request.into_inner();
        let allocation = self
            .node
            .allocate_staging(request.session_id, request.length)
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::AllocateStagingResponse {
            staging_id: allocation.staging_id,
            target: Some(encode_allocation_target(
                allocation.target,
                allocation.transfer_id,
                allocation.length,
            )),
        }))
    }

    async fn acquire_region(
        &self,
        request: Request<pb::AcquireRegionRequest>,
    ) -> Result<Response<pb::AcquireRegionResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_ACQUIRE_REGION);
        let request = request.into_inner();
        let grant = self
            .node
            .acquire_region(request.session_id, request.region_id)
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::AcquireRegionResponse {
            region_id: grant.region_id,
            region_length: grant.region_length,
            fd_token: grant.fd_token,
        }))
    }

    async fn delete_staging(
        &self,
        request: Request<pb::DeleteStagingRequest>,
    ) -> Result<Response<pb::DeleteStagingResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_DELETE_STAGING);
        let request = request.into_inner();
        self.node
            .delete_staging(request.session_id, request.staging_id)
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::DeleteStagingResponse {}))
    }

    async fn set_inline(
        &self,
        request: Request<pb::SetInlineRequest>,
    ) -> Result<Response<pb::SetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_SET_INLINE);
        let request = request.into_inner();
        require_supported_durability(&request.durability)?;
        let key = request
            .key
            .ok_or_else(|| node_invalid_argument("missing key"))?
            .value;
        let operation_id = decode_operation_id(request.operation_id)?;
        let result = self
            .node
            .set_inline(
                request.session_id,
                key,
                request.value,
                operation_id,
                request.condition,
            )
            .await
            .map_err(|error| self.map_worker_error(error))?;
        rpc.success();
        Ok(Response::new(pb::SetResponse {
            version: result.version,
            length: result.length,
        }))
    }

    async fn set(
        &self,
        request: Request<pb::SetRequest>,
    ) -> Result<Response<pb::SetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_SET);
        let request = request.into_inner();
        require_supported_durability(&request.durability)?;
        // protobuf message 字段在 Rust 中是 Option；每一层都在 wire 边界检查完整性。
        let key = request
            .key
            .ok_or_else(|| node_invalid_argument("missing key"))?
            .value;
        let staged = request
            .value
            .ok_or_else(|| node_invalid_argument("missing staged value"))?;
        let receipt = staged
            .receipt
            .ok_or_else(|| node_invalid_argument("missing transfer receipt"))?;
        // transfer_id 在 wire 上固定为 8-byte big-endian bytes，解码失败返回 INVALID_ARGUMENT。
        let transfer_id = decode_id(&receipt.transfer_id)?;
        // 从这里开始不再处理 protobuf：只向 NodeHandle 传领域参数。
        let result = self
            .node
            .set(
                request.session_id,
                key,
                staged.staging_id,
                super::arena_manager::HostReceipt {
                    transfer_id,
                    length: receipt.length,
                    digest: receipt.digest,
                    allocation_id: receipt.target_allocation_id,
                },
                decode_operation_id(request.operation_id)?,
                request.condition,
            )
            .await
            .map_err(|error| self.map_worker_error(error))?;
        rpc.success();
        Ok(Response::new(pb::SetResponse {
            version: result.version,
            length: result.length,
        }))
    }

    async fn delete(
        &self,
        request: Request<pb::DeleteRequest>,
    ) -> Result<Response<pb::DeleteResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_DELETE);
        let request = request.into_inner();
        let key = request
            .key
            .ok_or_else(|| node_invalid_argument("missing key"))?
            .value;
        let result = self
            .node
            .delete(
                request.session_id,
                key,
                decode_operation_id(request.operation_id)?,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::DeleteResponse {
            deleted: result.deleted,
            version: result.version,
        }))
    }

    async fn get(
        &self,
        request: Request<pb::GetRequest>,
    ) -> Result<Response<pb::GetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_GET);
        let request = request.into_inner();
        let key = request
            .key
            .ok_or_else(|| node_invalid_argument("missing key"))?
            .value;
        // Option::map 只在 range 存在时转换；None 仍表示读取完整 value。
        let range = request.range.map(|range| (range.offset, range.length));
        match self
            .node
            .get_with_inline_limit(
                request.session_id,
                key,
                request.exact_version,
                range,
                request.max_inline_bytes,
            )
            .await
        {
            // 命中时返回 payload ticket；bytes 由后续 Download RPC 获取。
            Ok(ticket) => {
                rpc.success();
                Ok(Response::new(read_ticket_response(ticket)))
            }
            // Key/Exact version 不存在是协议定义的 found=false，不转换成 gRPC 错误。
            Err(error) if super::runtime::is_not_found_result(&error) => {
                rpc.not_found();
                Ok(Response::new(pb::GetResponse {
                    found: false,
                    version: 0,
                    logical_length: 0,
                    segments: Vec::new(),
                    inline_value: None,
                }))
            }
            Err(error) => Err(self.map_worker_error(error)),
        }
    }

    async fn m_set(
        &self,
        request: Request<pb::MSetRequest>,
    ) -> Result<Response<pb::MSetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_MSET);
        let request = request.into_inner();
        require_supported_durability(&request.durability)?;
        if request.entries.is_empty() {
            return Err(node_invalid_argument("MSet entries are empty"));
        }
        let mut entries = Vec::with_capacity(request.entries.len());
        for entry in request.entries {
            let key = entry
                .key
                .ok_or_else(|| node_invalid_argument("missing MSet key"))?
                .value;
            let staged = entry
                .value
                .ok_or_else(|| node_invalid_argument("missing MSet staged value"))?;
            let (staging_id, receipt) = decode_staged_value(staged)?;
            entries.push((key, staging_id, receipt));
        }
        let versions = self
            .node
            .mset(
                request.session_id,
                entries,
                decode_operation_id(request.operation_id)?,
            )
            .await
            .map_err(map_worker_error)?
            .into_iter()
            .map(|item| pb::KeyVersion {
                key: Some(pb::Key { value: item.key }),
                version: item.version,
            })
            .collect();
        rpc.success();
        Ok(Response::new(pb::MSetResponse { versions }))
    }

    async fn m_get(
        &self,
        request: Request<pb::MGetRequest>,
    ) -> Result<Response<pb::MGetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_MGET);
        let request = request.into_inner();
        if request.keys.is_empty() {
            return Err(node_invalid_argument("MGet keys are empty"));
        }
        let mut items = Vec::with_capacity(request.keys.len());
        for key in request.keys {
            match self
                .node
                .get(request.session_id, key.value, None, None)
                .await
            {
                Ok(ticket) => items.push(read_ticket_response(ticket)),
                Err(error) if super::runtime::is_not_found_result(&error) => {
                    items.push(missing_get_response());
                }
                Err(error) => return Err(map_worker_error(error)),
            }
        }
        rpc.success();
        Ok(Response::new(pb::MGetResponse { items }))
    }

    async fn set_range(
        &self,
        request: Request<pb::SetRangeRequest>,
    ) -> Result<Response<pb::SetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_SET_RANGE);
        let request = request.into_inner();
        require_supported_durability(&request.durability)?;
        let key = request
            .key
            .ok_or_else(|| node_invalid_argument("missing key"))?
            .value;
        let staged = request
            .value
            .ok_or_else(|| node_invalid_argument("missing staged value"))?;
        let (staging_id, receipt) = decode_staged_value(staged)?;
        let result = self
            .node
            .set_range(SetRangeInput {
                session_id: request.session_id,
                key,
                offset: request.offset,
                staging_id,
                receipt,
                operation_id: decode_operation_id(request.operation_id)?,
                expected_version: request.expected_version,
            })
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::SetResponse {
            version: result.version,
            length: result.length,
        }))
    }

    async fn h_set(
        &self,
        request: Request<pb::HSetRequest>,
    ) -> Result<Response<pb::HSetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_HSET);
        let request = request.into_inner();
        require_supported_durability(&request.durability)?;
        if request.entries.is_empty() {
            return Err(node_invalid_argument("HSet entries are empty"));
        }
        let key = decode_key(request.key)?;
        let mut entries = Vec::with_capacity(request.entries.len());
        for entry in request.entries {
            let field = decode_field(entry.field)?;
            let (staging_id, receipt) = decode_staged_value(
                entry
                    .value
                    .ok_or_else(|| node_invalid_argument("missing Hash staged value"))?,
            )?;
            entries.push((field, staging_id, receipt));
        }
        let result = KkvOperations::new(self.node.clone())
            .hset(
                request.session_id,
                key,
                entries,
                decode_operation_id(request.operation_id)?,
                &request.mode,
                request.expected_version,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::HSetResponse {
            hash_version: result.hash_version,
            field_count: result.field_count,
        }))
    }

    async fn h_get(
        &self,
        request: Request<pb::HGetRequest>,
    ) -> Result<Response<pb::HGetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_HGET);
        let request = request.into_inner();
        let value = KkvOperations::new(self.node.clone())
            .hget(
                request.session_id,
                decode_key(request.key)?,
                decode_field(request.field)?,
                request.exact_hash_version,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(encode_hash_get(value)))
    }

    async fn hm_get(
        &self,
        request: Request<pb::HmGetRequest>,
    ) -> Result<Response<pb::HmGetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_HMGET);
        let request = request.into_inner();
        let fields = request
            .fields
            .into_iter()
            .map(|field| field.value)
            .collect();
        let (version, values) = KkvOperations::new(self.node.clone())
            .hmget(
                request.session_id,
                decode_key(request.key)?,
                fields,
                request.exact_hash_version,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::HmGetResponse {
            hash_version: version,
            values: values.into_iter().map(encode_hash_get).collect(),
        }))
    }

    async fn h_get_all(
        &self,
        request: Request<pb::HGetAllRequest>,
    ) -> Result<Response<pb::HGetAllResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_HGET_ALL);
        let request = request.into_inner();
        let (version, entries) = KkvOperations::new(self.node.clone())
            .hget_all(
                request.session_id,
                decode_key(request.key)?,
                request.exact_hash_version,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::HGetAllResponse {
            hash_version: version,
            entries: entries.into_iter().map(encode_kkv_value).collect(),
        }))
    }

    async fn h_delete(
        &self,
        request: Request<pb::HDeleteRequest>,
    ) -> Result<Response<pb::HSetResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_HDELETE);
        let request = request.into_inner();
        require_supported_durability(&request.durability)?;
        let fields = request
            .fields
            .into_iter()
            .map(|field| field.value)
            .collect();
        let result = KkvOperations::new(self.node.clone())
            .hdelete(
                request.session_id,
                decode_key(request.key)?,
                fields,
                decode_operation_id(request.operation_id)?,
                request.expected_version,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::HSetResponse {
            hash_version: result.hash_version,
            field_count: result.field_count,
        }))
    }

    async fn h_scan(
        &self,
        request: Request<pb::HScanRequest>,
    ) -> Result<Response<pb::HScanResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_HSCAN);
        let request = request.into_inner();
        let (version, next_cursor, entries) = KkvOperations::new(self.node.clone())
            .hscan(
                request.session_id,
                decode_key(request.key)?,
                request.cursor,
                request.limit,
                request.exact_hash_version,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::HScanResponse {
            hash_version: version,
            next_cursor,
            entries: entries.into_iter().map(encode_kkv_value).collect(),
        }))
    }

    async fn h_write_at(
        &self,
        request: Request<pb::HWriteAtRequest>,
    ) -> Result<Response<pb::HWriteAtResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::WORKER_HWRITE_AT);
        let request = request.into_inner();
        require_supported_durability(&request.durability)?;
        let (staging_id, receipt) = decode_staged_value(
            request
                .value
                .ok_or_else(|| node_invalid_argument("missing Hash staged value"))?,
        )?;
        let result = KkvOperations::new(self.node.clone())
            .hwrite_at(
                request.session_id,
                decode_key(request.key)?,
                decode_field(request.field)?,
                request.offset,
                staging_id,
                receipt,
                decode_operation_id(request.operation_id)?,
                request.expected_hash_version,
            )
            .await
            .map_err(map_worker_error)?;
        rpc.success();
        Ok(Response::new(pb::HWriteAtResponse {
            hash_version: result.hash_version,
            value_version: result.value_version,
            length: result.length,
            field_count: result.field_count,
        }))
    }
}

#[tonic::async_trait]
impl WorkerPayloadService for WorkerServiceHandler {
    // 同一个 Rust 类型可以实现多个 generated Service Trait；注册时分别包进对应 Server。
    async fn upload(
        &self,
        request: Request<pb::UploadPayloadRequest>,
    ) -> Result<Response<pb::UploadPayloadResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::PAYLOAD_UPLOAD);
        let request = request.into_inner();
        let transfer_id = decode_id(&request.transfer_id)?;
        let receipt = self
            .node
            .upload(transfer_id, request.payload)
            .await
            .map_err(|error| self.map_worker_error(error))?;
        rpc.success();
        Ok(Response::new(pb::UploadPayloadResponse {
            receipt: Some(pb::TransferReceipt {
                transfer_id: encode_id(transfer_id),
                length: receipt.length,
                digest: receipt.digest,
                target_allocation_id: receipt.allocation_id,
            }),
        }))
    }

    async fn download(
        &self,
        request: Request<pb::DownloadPayloadRequest>,
    ) -> Result<Response<pb::DownloadPayloadResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::PAYLOAD_DOWNLOAD);
        let transfer_id = decode_id(&request.get_ref().transfer_id)?;
        // NodeState::download 使用 remove，因此同一 transfer_id 只可成功一次。
        let payload = self
            .node
            .download(transfer_id)
            .await
            .map_err(|error| self.map_worker_error(error))?;
        let length = payload.len() as u64;
        rpc.success();
        Ok(Response::new(pb::DownloadPayloadResponse {
            payload,
            receipt: Some(pb::TransferReceipt {
                transfer_id: encode_id(transfer_id),
                length,
                digest: Vec::new(),
                // Download 不提交 staging；0 表示此回执没有 Allocation 身份。
                target_allocation_id: 0,
            }),
        }))
    }
}

async fn handle_client_session_message(
    node: &NodeHandle,
    message: pb::ClientSessionMessage,
) -> Result<(), Status> {
    // oneof `message` 生成 Option<enum>；match 同时处理两种合法消息和空消息。
    match message.message {
        Some(pb::client_session_message::Message::Heartbeat(heartbeat)) => node
            .heartbeat(message.session_id, heartbeat.released_view_through)
            .await
            .map_err(map_worker_error),
        Some(pb::client_session_message::Message::EventAck(ack)) => node
            .acknowledge(message.session_id, ack.event_sequence)
            .await
            .map_err(map_worker_error),
        None => Err(node_invalid_argument("empty session message")),
    }
}

fn encode_event(event: NodeEvent) -> pb::NodeSessionEvent {
    // 领域 enum 与 protobuf oneof 在这里一一映射；业务 owner 不依赖 pb 类型。
    match event {
        NodeEvent::InvalidateCurrent {
            session_epoch,
            event_sequence,
            key,
            minimum_version,
        } => pb::NodeSessionEvent {
            session_epoch,
            event_sequence,
            event: Some(pb::node_session_event::Event::CurrentInvalidation(
                pb::CurrentInvalidation {
                    key: Some(pb::Key { value: key }),
                    minimum_version,
                    route_epoch: 0,
                },
            )),
        },
    }
}

fn grpc_target(transfer_id: u64, length: u64) -> pb::PayloadTarget {
    // 嵌套 Some 对应 protobuf message + oneof 两层可选结构。
    pb::PayloadTarget {
        target: Some(pb::payload_target::Target::Grpc(pb::GrpcTarget {
            transfer_id: encode_id(transfer_id),
            nonce: Vec::new(),
            length,
        })),
    }
}

fn encode_allocation_target(
    target: HostAllocationTarget,
    transfer_id: u64,
    length: u64,
) -> pb::PayloadTarget {
    match target {
        HostAllocationTarget::Grpc => grpc_target(transfer_id, length),
        HostAllocationTarget::Shm(descriptor) => shm_target(descriptor),
    }
}

fn encode_read_target(target: ReadTarget, length: u64) -> pb::PayloadTarget {
    match target {
        ReadTarget::Grpc { transfer_id } => grpc_target(transfer_id, length),
        ReadTarget::Shm(descriptor) => {
            // segment 长度必须与已裁剪的读 descriptor 一致，不能编码整块 allocation。
            debug_assert_eq!(descriptor.length, length);
            shm_target(descriptor)
        }
    }
}

fn shm_target(descriptor: HostShmDescriptor) -> pb::PayloadTarget {
    pb::PayloadTarget {
        target: Some(pb::payload_target::Target::Shm(pb::ShmDescriptor {
            region_id: descriptor.region_id,
            offset: descriptor.offset,
            length: descriptor.length,
            allocation_id: descriptor.allocation_id,
            view_epoch: descriptor.view_epoch,
            transfer_id: encode_id(descriptor.transfer_id),
        })),
    }
}

fn read_ticket_response(ticket: super::runtime::ReadTicket) -> pb::GetResponse {
    pb::GetResponse {
        found: true,
        version: ticket.version,
        logical_length: ticket.logical_length,
        inline_value: ticket.inline_value,
        segments: ticket
            .segments
            .into_iter()
            .map(|segment| pb::ReadSegment {
                target: Some(encode_read_target(segment.target, segment.payload_length)),
                logical_offset: segment.logical_offset,
            })
            .collect(),
    }
}

fn missing_get_response() -> pb::GetResponse {
    pb::GetResponse {
        found: false,
        version: 0,
        logical_length: 0,
        segments: Vec::new(),
        inline_value: None,
    }
}

fn decode_staged_value(
    value: pb::StagedValue,
) -> Result<(u64, super::arena_manager::HostReceipt), Status> {
    let receipt = value
        .receipt
        .ok_or_else(|| node_invalid_argument("missing transfer receipt"))?;
    Ok((
        value.staging_id,
        super::arena_manager::HostReceipt {
            transfer_id: decode_id(&receipt.transfer_id)?,
            length: receipt.length,
            digest: receipt.digest,
            allocation_id: receipt.target_allocation_id,
        },
    ))
}

/// The wire contract already reserves future durability policies, but the
/// current Worker only has a synchronous local-memory commit path. Rejecting
/// stronger policies at the boundary is essential: silently acknowledging a
/// local write as replicated/durable would violate the SDK contract.
fn require_supported_durability(durability: &str) -> Result<(), Status> {
    if durability.is_empty() || durability == "local-memory" {
        return Ok(());
    }
    Err(dms_error_to_status(dms_error::DmsError::new(
        dms_error::NODE_TRANSFER_UNSUPPORTED,
        dms_error::ErrorKind::Unimplemented,
        format!("unsupported durability policy in this release: {durability}"),
    )))
}

fn decode_key(key: Option<pb::Key>) -> Result<Vec<u8>, Status> {
    key.map(|key| key.value)
        .ok_or_else(|| node_invalid_argument("missing key"))
}

fn decode_field(field: Option<pb::HashField>) -> Result<Vec<u8>, Status> {
    field
        .map(|field| field.value)
        .ok_or_else(|| node_invalid_argument("missing Hash field"))
}

fn encode_hash_get(value: Option<KkvValue>) -> pb::HGetResponse {
    pb::HGetResponse {
        found: value.is_some(),
        value: value.map(encode_kkv_value),
    }
}

fn encode_kkv_value(value: KkvValue) -> pb::HashValueRead {
    pb::HashValueRead {
        field: Some(pb::HashField { value: value.field }),
        hash_version: value.hash_version,
        value_version: value.value_version,
        logical_length: value.bytes.len() as u64,
        segments: Vec::new(),
        inline_value: value.bytes,
    }
}

fn encode_id(id: u64) -> Vec<u8> {
    // 网络协议使用 big-endian，保证不同 CPU 架构得到相同字节序。
    id.to_be_bytes().to_vec()
}

fn decode_id(bytes: &[u8]) -> Result<u64, Status> {
    // TryInto 将任意长度 slice 校验并转换为恰好 8 字节的数组。
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| node_invalid_argument("transfer id must contain 8 bytes"))?;
    Ok(u64::from_be_bytes(bytes))
}

fn decode_operation_id(operation: Option<pb::OperationId>) -> Result<Vec<u8>, Status> {
    let operation = operation.ok_or_else(|| node_invalid_argument("missing operation id"))?;
    let client: [u8; 16] = operation
        .client_instance_id
        .try_into()
        .map_err(|_| node_invalid_argument("client instance id must contain 16 bytes"))?;
    if operation.sequence == 0 {
        return Err(node_invalid_argument(
            "operation sequence must be greater than zero",
        ));
    }
    let mut encoded = Vec::with_capacity(24);
    encoded.extend_from_slice(&client);
    encoded.extend_from_slice(&operation.sequence.to_be_bytes());
    Ok(encoded)
}

fn map_worker_error(error: WorkerError) -> Status {
    dms_error_to_status(super::runtime::worker_error_to_dms(error))
}

fn node_invalid_argument(message: impl Into<String>) -> Status {
    dms_error_to_status(dms_error::DmsError::new(
        dms_error::NODE_WORKER_INVALID_REQUEST,
        dms_error::ErrorKind::InvalidArgument,
        message,
    ))
}

#[cfg(test)]
mod tests {
    //! 这些测试启动真实 Tonic Server，再通过公开 SDK 验证 UDS/TCP 具有相同业务语义。
    use std::{
        path::PathBuf,
        pin::Pin,
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc as std_mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use dms_client::{
        ClientOptions, DmsClient, GetOptions, HashDeleteOptions, HashEntry, HashGetOptions,
        HashRangeWriteOptions, HashScanOptions, HashWriteOptions, Key, KvEntry, MSetOptions,
        ObjectVersion, RangeWriteOptions, ReadVersion, ScanCursor,
    };
    use dms_error::DmsError;
    use dms_metrics::{encode_text, registry};
    use dms_protocol::v1 as pb;
    use dms_protocol::v1::{
        metadata_service_server::MetadataServiceServer,
        worker_payload_service_server::WorkerPayloadServiceServer,
        worker_service_client::WorkerServiceClient, worker_service_server::WorkerServiceServer,
    };
    use dms_transport::{GrpcConfig, SecurityManager, TlsConfig, status_to_dms_error};
    use tokio::net::{TcpListener, UnixListener};
    use tokio::sync::{mpsc, oneshot};
    use tokio_stream::{
        Stream,
        wrappers::{ReceiverStream, TcpListenerStream, UnixListenerStream},
    };
    use tonic::Status;

    use super::{
        WorkerServiceHandler, dms_error_to_status, node_invalid_argument,
        require_supported_durability,
    };
    use crate::meta::{metadata_service::MetadataServiceHandler, runtime::MetaHandle};
    use crate::node::arena_manager::SharedFdBroker;
    use crate::node::metadata_client::MetadataClient;
    use crate::node::runtime::NodeHandle;

    // 并行测试需要唯一 UDS 文件名；AtomicU64 无需加 Mutex。
    static NEXT_SOCKET: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn stronger_durability_is_rejected_instead_of_silently_downgraded() {
        assert!(require_supported_durability("local-memory").is_ok());
        let status = require_supported_durability("memory-copies:2")
            .expect_err("replication is not implemented yet");
        assert_eq!(status.code(), tonic::Code::Unimplemented);
        assert_eq!(
            status_to_dms_error(status).code(),
            dms_error::NODE_TRANSFER_UNSUPPORTED
        );
    }

    #[test]
    fn wire_argument_errors_carry_dms_error_detail() {
        let status = super::decode_operation_id(None).expect_err("missing operation id");

        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        let error = status_to_dms_error(status);
        assert_eq!(error.code(), dms_error::NODE_WORKER_INVALID_REQUEST);
        assert_eq!(error.kind(), dms_error::ErrorKind::InvalidArgument);
    }

    enum TestAddress {
        // enum 让测试编译期只能选择支持的两种 listener。
        Tcp,
        Unix,
    }

    #[derive(Clone, Copy)]
    enum TestMetaBehavior {
        Real,
        CommitFailsWithJournalAppend,
    }

    #[derive(Default)]
    struct CommitFailingMetaService;

    #[tonic::async_trait]
    impl pb::metadata_service_server::MetadataService for CommitFailingMetaService {
        type WatchNodeEventsStream =
            Pin<Box<dyn Stream<Item = Result<pb::NodeEvent, Status>> + Send + 'static>>;

        async fn open_node_session(
            &self,
            request: tonic::Request<pb::OpenNodeSessionRequest>,
        ) -> Result<tonic::Response<pb::OpenNodeSessionResponse>, Status> {
            let request = request.into_inner();
            let registration = request
                .registration
                .ok_or_else(|| node_invalid_argument("missing node registration"))?;
            Ok(tonic::Response::new(pb::OpenNodeSessionResponse {
                session: Some(pb::NodeSessionIdentity {
                    session_id: b"fake-meta-session".to_vec(),
                    node_id: registration.node_id,
                    node_epoch: 1,
                }),
                heartbeat_interval_millis: 1_000,
                lease_ttl_millis: 30_000,
            }))
        }

        async fn heartbeat(
            &self,
            _request: tonic::Request<pb::NodeHeartbeatRequest>,
        ) -> Result<tonic::Response<pb::NodeHeartbeatResponse>, Status> {
            Ok(tonic::Response::new(pb::NodeHeartbeatResponse {
                lease_ttl_millis: 30_000,
                accepted_node_epoch: 1,
                event_high_watermark: 0,
            }))
        }

        async fn resolve_object(
            &self,
            _request: tonic::Request<pb::ResolveObjectRequest>,
        ) -> Result<tonic::Response<pb::ResolveObjectResponse>, Status> {
            Err(dms_error_to_status(DmsError::new(
                dms_error::META_CATALOG_NOT_FOUND,
                dms_error::ErrorKind::NotFound,
                "fake meta has no catalog entry",
            )))
        }

        async fn report_replicas(
            &self,
            _request: tonic::Request<pb::ReportReplicasRequest>,
        ) -> Result<tonic::Response<pb::ReportReplicasResponse>, Status> {
            Ok(tonic::Response::new(pb::ReportReplicasResponse {
                accepted: Vec::new(),
                rejected_block_ids: Vec::new(),
                catalog_watermark: 0,
            }))
        }

        async fn commit_version(
            &self,
            _request: tonic::Request<pb::CommitVersionRequest>,
        ) -> Result<tonic::Response<pb::CommitVersionResponse>, Status> {
            Err(dms_error_to_status(DmsError::new(
                dms_error::META_JOURNAL_APPEND_FAILED,
                dms_error::ErrorKind::Unavailable,
                "fake journal append failed",
            )))
        }

        async fn commit_batch(
            &self,
            _request: tonic::Request<pb::CommitBatchRequest>,
        ) -> Result<tonic::Response<pb::CommitBatchResponse>, Status> {
            Err(dms_error_to_status(DmsError::new(
                dms_error::META_JOURNAL_APPEND_FAILED,
                dms_error::ErrorKind::Unavailable,
                "fake journal append failed",
            )))
        }

        async fn get_operation(
            &self,
            _request: tonic::Request<pb::GetOperationRequest>,
        ) -> Result<tonic::Response<pb::GetOperationResponse>, Status> {
            Ok(tonic::Response::new(pb::GetOperationResponse {
                state: "not-found".to_string(),
                committed: None,
            }))
        }

        async fn plan_replicas(
            &self,
            _request: tonic::Request<pb::PlanReplicasRequest>,
        ) -> Result<tonic::Response<pb::PlanReplicasResponse>, Status> {
            Ok(tonic::Response::new(pb::PlanReplicasResponse {
                plan_id: Vec::new(),
                targets: Vec::new(),
                expires_after_millis: 0,
            }))
        }

        async fn watch_node_events(
            &self,
            _request: tonic::Request<pb::WatchNodeEventsRequest>,
        ) -> Result<tonic::Response<Self::WatchNodeEventsStream>, Status> {
            let (_sender, receiver) = mpsc::channel(1);
            Ok(tonic::Response::new(Box::pin(ReceiverStream::new(
                receiver,
            ))))
        }

        async fn acknowledge_node_event(
            &self,
            _request: tonic::Request<pb::AcknowledgeNodeEventRequest>,
        ) -> Result<tonic::Response<pb::AcknowledgeNodeEventResponse>, Status> {
            Ok(tonic::Response::new(pb::AcknowledgeNodeEventResponse {}))
        }
    }

    struct TestServer {
        // SDK 使用的统一 URI。
        endpoint: String,
        // Drop 时通过 oneshot 请求 Server 优雅退出。
        shutdown: Option<oneshot::Sender<()>>,
        // 保存 OS 线程句柄，确保测试结束前 join。
        thread: Option<thread::JoinHandle<()>>,
        // 仅 UDS 使用；Drop 时删除 socket 文件。
        socket_path: Option<PathBuf>,
        fd_broker: Option<SharedFdBroker>,
    }

    impl TestServer {
        fn start(address: TestAddress) -> Self {
            Self::start_with_capacity(address, 256 * 1024 * 1024)
        }

        fn start_with_capacity(address: TestAddress, arena_capacity_bytes: u64) -> Self {
            Self::start_internal(address, arena_capacity_bytes, TestMetaBehavior::Real)
        }

        fn start_with_meta_commit_failure(address: TestAddress) -> Self {
            Self::start_internal(
                address,
                256 * 1024 * 1024,
                TestMetaBehavior::CommitFailsWithJournalAppend,
            )
        }

        fn start_internal(
            address: TestAddress,
            arena_capacity_bytes: u64,
            meta_behavior: TestMetaBehavior,
        ) -> Self {
            // std mpsc 跨 OS 线程同步“监听地址已就绪”；这里不是业务 mailbox。
            let (ready_sender, ready_receiver) = std_mpsc::channel();
            let (shutdown_sender, shutdown_receiver) = oneshot::channel();
            let socket_path = match address {
                TestAddress::Tcp => None,
                TestAddress::Unix => Some(std::env::temp_dir().join(format!(
                    "dms-client-node-{}-{}.sock",
                    std::process::id(),
                    NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
                ))),
            };
            // 一份留给 TestServer::drop 清理，一份移动进 Server 线程。
            let thread_socket_path = socket_path.clone();
            let thread = thread::spawn(move || {
                // 测试线程内单独创建 Tokio runtime，模拟真实独立 Server 进程。
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("test runtime");
                runtime.block_on(async move {
                    // 先启动进程内单中心 Meta；Node 后续 SET/GET 的版本权威来自这里。
                    let meta_listener = TcpListener::bind("127.0.0.1:0")
                        .await
                        .expect("bind test meta");
                    let meta_endpoint = format!(
                        "http://{}",
                        meta_listener.local_addr().expect("meta address")
                    );
                    match meta_behavior {
                        TestMetaBehavior::Real => {
                            let meta_handler = MetadataServiceHandler::new(MetaHandle::spawn());
                            tokio::spawn(async move {
                                tonic::transport::Server::builder()
                                    .add_service(MetadataServiceServer::new(meta_handler))
                                    .serve_with_incoming(TcpListenerStream::new(meta_listener))
                                    .await
                                    .expect("serve test meta");
                            });
                        }
                        TestMetaBehavior::CommitFailsWithJournalAppend => {
                            tokio::spawn(async move {
                                tonic::transport::Server::builder()
                                    .add_service(MetadataServiceServer::new(
                                        CommitFailingMetaService,
                                    ))
                                    .serve_with_incoming(TcpListenerStream::new(meta_listener))
                                    .await
                                    .expect("serve failing test meta");
                            });
                        }
                    }
                    let metadata = MetadataClient::connect(
                        &meta_endpoint,
                        1,
                        "http://127.0.0.1:0".to_string(),
                        None,
                    )
                    .await
                    .expect("register test node");
                    let shared_fd_broker = thread_socket_path.as_ref().map(|path| {
                        let mut broker_path = path.clone();
                        broker_path.set_extension("fd.sock");
                        let broker = SharedFdBroker::bind(broker_path).expect("bind fd broker");
                        let broker_task = broker.clone();
                        std::thread::spawn(move || {
                            loop {
                                if broker_task.serve_one().is_err() {
                                    break;
                                }
                            }
                        });
                        broker
                    });
                    let broker_observer = shared_fd_broker.clone();
                    // 组合顺序与正式 node::serve 一致：业务 owner→RPC Handler→Tonic Server。
                    let node = NodeHandle::spawn(
                        "test-node".to_string(),
                        metadata.clone(),
                        arena_capacity_bytes,
                        Duration::from_secs(30),
                        shared_fd_broker,
                    );
                    let event_node = node.clone();
                    let mut stream = metadata.watch_events(0).await.expect("open Meta watch");
                    let lease_started = Instant::now();
                    let lease_ttl = metadata.heartbeat(0).await.expect("Meta cache lease");
                    node.metadata_lease(
                        Some(lease_started + Duration::from_millis(lease_ttl)),
                        Some(true),
                    )
                    .await
                    .expect("install Meta lease");
                    tokio::spawn(async move {
                        while let Ok(Some(event)) = stream.message().await {
                            if let Some(pb::node_event::Event::InvalidateCurrent(invalidation)) =
                                &event.event
                                && let Some(key) = &invalidation.key
                            {
                                let _ = event_node
                                    .invalidate_current(
                                        key.value.clone(),
                                        invalidation.minimum_version,
                                    )
                                    .await;
                            }
                            let _ = metadata.acknowledge_event(&event).await;
                        }
                    });
                    let handler =
                        WorkerServiceHandler::new(node, matches!(address, TestAddress::Unix));
                    let security = SecurityManager::new(TlsConfig::Disabled).expect("security");
                    let server =
                        GrpcConfig::default().configure_server(tonic::transport::Server::builder());
                    // 同一 Handler 同时实现 WorkerService 和 WorkerPayloadService。
                    let server = security
                        .configure_server(server)
                        .expect("server security")
                        .add_service(WorkerServiceServer::new(handler.clone()))
                        .add_service(WorkerPayloadServiceServer::new(handler));
                    match thread_socket_path {
                        Some(path) => {
                            // bind 会创建 UDS socket 文件；路径必须在 Client/Server 可见。
                            let listener = UnixListener::bind(&path).expect("bind UDS");
                            ready_sender
                                .send((format!("unix://{}", path.display()), broker_observer))
                                .expect("publish UDS endpoint");
                            // incoming 提供已 bind 的 UDS 连接流，shutdown Future 完成时停止。
                            server
                                .serve_with_incoming_shutdown(
                                    UnixListenerStream::new(listener),
                                    async {
                                        let _ = shutdown_receiver.await;
                                    },
                                )
                                .await
                                .expect("serve UDS");
                        }
                        None => {
                            // 端口 0 让 OS 自动选择空闲端口，避免并行测试冲突。
                            let listener =
                                TcpListener::bind("127.0.0.1:0").await.expect("bind TCP");
                            ready_sender
                                .send((
                                    format!(
                                        "http://{}",
                                        listener.local_addr().expect("local address")
                                    ),
                                    broker_observer,
                                ))
                                .expect("publish TCP endpoint");
                            server
                                .serve_with_incoming_shutdown(
                                    TcpListenerStream::new(listener),
                                    async {
                                        let _ = shutdown_receiver.await;
                                    },
                                )
                                .await
                                .expect("serve TCP");
                        }
                    }
                });
            });
            // 阻塞当前测试线程，直到 Server 完成 bind；超时说明启动失败。
            let (endpoint, fd_broker) = ready_receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("server ready");
            Self {
                endpoint,
                shutdown: Some(shutdown_sender),
                thread: Some(thread),
                socket_path,
                fd_broker,
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            // Option::take 把 Sender/JoinHandle 移出 &mut self，防止 Drop 中重复使用。
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(thread) = self.thread.take() {
                thread.join().expect("server thread");
            }
            if let Some(path) = &self.socket_path {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn set_get(endpoint: &str) {
        // 只改变 endpoint，不改变任何业务调用，证明 transport 分叉被隔离在 connect。
        let client = DmsClient::connect(endpoint, ClientOptions::default()).expect("connect");
        let key = Key::new("channel/basic").expect("key");
        let result = client.set(key.clone(), b"hello-dms").expect("set");
        assert_eq!(result.version, ObjectVersion(1));
        assert_eq!(client.get(key).expect("get"), Some(b"hello-dms".to_vec()));

        let range_key = b"range/object";
        let base = client.set(range_key, b"abcdefghij").expect("range base");
        let patched = client
            .set_range_with_options(
                range_key,
                4,
                b"X",
                RangeWriteOptions {
                    expected_version: Some(base.version),
                    durability: None,
                },
            )
            .expect("range patch");
        assert_eq!(
            client.get(range_key).expect("range get"),
            Some(b"abcdXfghij".to_vec())
        );
        assert_eq!(
            client
                .get_with_options(
                    range_key,
                    GetOptions {
                        version: ReadVersion::Exact(base.version),
                        range: None,
                    },
                )
                .expect("exact old version")
                .expect("old value")
                .bytes,
            b"abcdefghij"
        );
        assert!(patched.version > base.version);

        let batch = client
            .mset(
                &[
                    KvEntry::new(b"batch/a", b"A".to_vec()).expect("entry"),
                    KvEntry::new(b"batch/b", b"B".to_vec()).expect("entry"),
                ],
                MSetOptions::default(),
            )
            .expect("atomic mset");
        assert_eq!(batch.versions.len(), 2);
        let values = client
            .mget(&[b"batch/a".as_slice(), b"batch/b".as_slice()])
            .expect("mget");
        assert_eq!(values[0].as_ref().expect("a").bytes, b"A");
        assert_eq!(values[1].as_ref().expect("b").bytes, b"B");
        let with_miss = client
            .mget(&[b"batch/a".as_slice(), b"batch/missing".as_slice()])
            .expect("mget with miss");
        assert!(with_miss[0].is_some());
        assert!(
            with_miss[1].is_none(),
            "MGET miss is an item None, not an error"
        );

        // Hash semantics must cross the Worker RPC boundary. The SDK does not
        // encode a private blob and fall back to ordinary SET.
        let hash_key = b"checkpoint/manifest";
        let first = client
            .hset(
                hash_key,
                &[
                    HashEntry::new(b"rank-0", b"abcdef".to_vec()).expect("entry"),
                    HashEntry::new(b"rank-1", b"ghijkl".to_vec()).expect("entry"),
                ],
                HashWriteOptions::default(),
            )
            .expect("hset");
        assert_eq!(first.field_count, 2);

        // Two independent SDK instances may both observe the same Hash
        // version, but only one conditional writer can publish the successor.
        // This locks the multi-writer boundary at Worker→Meta CAS rather than
        // accidentally serializing writers inside one Client object.
        let competing_writer =
            DmsClient::connect(endpoint, ClientOptions::default()).expect("competing writer");
        let winner = client
            .hset(
                hash_key,
                &[HashEntry::new(b"rank-2", b"winner".to_vec()).expect("entry")],
                HashWriteOptions {
                    expected_version: Some(first.version),
                    ..HashWriteOptions::default()
                },
            )
            .expect("conditional winner");
        let stale = competing_writer
            .hset(
                hash_key,
                &[HashEntry::new(b"rank-3", b"stale".to_vec()).expect("entry")],
                HashWriteOptions {
                    expected_version: Some(first.version),
                    ..HashWriteOptions::default()
                },
            )
            .expect_err("stale writer must conflict");
        assert_eq!(stale.code(), dms_error::META_CATALOG_VERSION_CONFLICT);
        assert!(winner.version > first.version);
        assert_eq!(
            client
                .hget(hash_key, b"rank-0")
                .expect("hget")
                .expect("field")
                .bytes,
            b"abcdef"
        );
        let multi = client
            .hmget(
                hash_key,
                &[b"rank-1".as_slice(), b"missing".as_slice()],
                HashGetOptions::default(),
            )
            .expect("hmget");
        assert_eq!(multi.version, Some(winner.version));
        assert_eq!(multi.values[0].as_ref().expect("rank-1").bytes, b"ghijkl");
        assert!(multi.values[1].is_none());
        let patched = client
            .hwrite_at(
                hash_key,
                b"rank-0",
                2,
                b"XY",
                HashRangeWriteOptions::default(),
            )
            .expect("hwrite_at");
        assert_eq!(patched.field_count, 3);
        assert_eq!(
            client
                .hget(hash_key, b"rank-0")
                .expect("hget patched")
                .expect("field")
                .bytes,
            b"abXYef"
        );
        let scan = client
            .hscan(
                hash_key,
                ScanCursor(0),
                HashScanOptions {
                    limit: 1,
                    ..HashScanOptions::default()
                },
            )
            .expect("hscan");
        assert_eq!(scan.entries.len(), 1);
        assert_ne!(scan.next_cursor, ScanCursor(0));
        let deleted = client
            .hdel(
                hash_key,
                &[b"rank-1".as_slice()],
                HashDeleteOptions::default(),
            )
            .expect("hdel");
        assert_eq!(deleted.field_count, 2);
        assert_eq!(
            client
                .hgetall(hash_key, HashGetOptions::default())
                .expect("hgetall")
                .entries
                .len(),
            2
        );
    }

    #[test]
    fn same_business_methods_work_over_unix_domain_socket() {
        let server = TestServer::start(TestAddress::Unix);
        set_get(&server.endpoint);
    }

    #[test]
    fn same_business_methods_work_over_tcp() {
        let server = TestServer::start(TestAddress::Tcp);
        set_get(&server.endpoint);
    }

    // SDK 没有 value cache；把内联预算压到 1 byte，确保下面读取确实经过
    // download_segments，而不是被小对象内联绕过。
    fn segment_reads_preserve_complete_bytes(
        address: TestAddress,
        shared_memory: bool,
        length: usize,
    ) {
        let server = TestServer::start(address);
        let client = DmsClient::connect(
            &server.endpoint,
            ClientOptions {
                inline_threshold_bytes: Some(1),
                shared_memory: Some(shared_memory),
                ..ClientOptions::default()
            },
        )
        .expect("connect segment reader");
        let original: Vec<u8> = (0..length).map(|index| (index % 251) as u8).collect();
        let key = "segments/pattern";
        let base = client.set(key, &original).expect("set single block");
        assert_eq!(
            client.get(key).expect("single block get").unwrap(),
            original
        );

        let range = dms_client::ByteRange {
            offset: 101,
            len: 8193,
        };
        let read = |range| {
            client
                .get_with_options(
                    key,
                    GetOptions {
                        version: ReadVersion::Current,
                        range,
                    },
                )
                .expect("range get")
                .expect("present")
                .bytes
        };
        assert_eq!(read(Some(range)), original[101..8294]);

        // 中间 patch 使一个逻辑对象变成 base/patch/base 三段；既检查完整拼接，
        // 也检查跨两个边界的范围读取，不能以只校验长度代替内容正确性。
        // 大对象再覆盖“大首段被直接接管，后续仍需追加 patch”的路径；
        // 小对象仍在前部 patch，验证普通多段读取。
        let patch_offset = if length > 32 * 1024 * 1024 + 8 {
            (32 * 1024 * 1024) as u64
        } else {
            4097
        };
        let patch = b"changed";
        client
            .set_range(key, patch_offset, patch)
            .expect("patch middle");
        let mut expected = original.clone();
        expected[patch_offset as usize..patch_offset as usize + patch.len()].copy_from_slice(patch);
        assert_eq!(read(None), expected);
        assert_eq!(read(Some(range)), expected[101..8294]);
        let patch_range = dms_client::ByteRange {
            offset: patch_offset - 3,
            len: patch.len() as u64 + 6,
        };
        assert_eq!(
            read(Some(patch_range)),
            expected[patch_range.offset as usize..(patch_range.offset + patch_range.len) as usize],
        );
        assert_eq!(
            client
                .get_with_options(
                    key,
                    GetOptions {
                        version: ReadVersion::Exact(base.version),
                        range: None
                    }
                )
                .expect("old version")
                .unwrap()
                .bytes,
            original,
        );
        let batch = client
            .mget(&[key, "segments/missing"])
            .expect("mget segments");
        assert_eq!(batch[0].as_ref().unwrap().bytes, expected);
        assert!(batch[1].is_none());
    }

    #[test]
    fn segment_reads_preserve_complete_bytes_over_tcp() {
        segment_reads_preserve_complete_bytes(TestAddress::Tcp, false, 128 * 1024);
    }

    #[test]
    fn segment_reads_preserve_complete_bytes_over_shm() {
        segment_reads_preserve_complete_bytes(TestAddress::Unix, true, 128 * 1024);
    }

    #[test]
    fn large_segment_reads_preserve_complete_bytes_over_shm() {
        // 锁住大块普通GET的完整bytes、patch、范围与历史读取行为。
        // 此回归独立于具体复制策略，不能把返回View或命中缓存当作通过。
        segment_reads_preserve_complete_bytes(TestAddress::Unix, true, 32 * 1024 * 1024 + 17);
    }

    #[test]
    fn worker_get_wire_returns_inline_value_when_client_advertises_budget() {
        let server = TestServer::start(TestAddress::Tcp);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        runtime.block_on(async {
            let mut client = WorkerServiceClient::connect(server.endpoint.clone())
                .await
                .expect("connect worker");
            let session = client
                .open_session(pb::OpenSessionRequest {
                    min_version: 1,
                    max_version: 1,
                    shared_memory: false,
                    zero_copy_read: false,
                    zero_copy_write: false,
                })
                .await
                .expect("open session")
                .into_inner()
                .session_id;

            client
                .set_inline(pb::SetInlineRequest {
                    session_id: session,
                    key: Some(pb::Key {
                        value: b"wire/inline".to_vec(),
                    }),
                    value: b"hello".to_vec(),
                    operation_id: Some(pb::OperationId {
                        client_instance_id: vec![7; 16],
                        sequence: 1,
                    }),
                    condition: "any".to_string(),
                    durability: "local-memory".to_string(),
                })
                .await
                .expect("set inline");

            let response = client
                .get(pb::GetRequest {
                    session_id: session,
                    key: Some(pb::Key {
                        value: b"wire/inline".to_vec(),
                    }),
                    exact_version: None,
                    range: None,
                    max_inline_bytes: 64,
                })
                .await
                .expect("get inline")
                .into_inner();

            assert!(response.found);
            assert_eq!(response.inline_value.as_deref(), Some(b"hello".as_slice()));
            assert!(response.segments.is_empty());
        });
    }

    #[test]
    fn sdk_mget_returns_none_for_missing_keys() {
        let server = TestServer::start(TestAddress::Tcp);
        let client =
            DmsClient::connect(&server.endpoint, ClientOptions::default()).expect("connect");

        client
            .set("batch/present", b"visible")
            .expect("set present");
        let values = client
            .mget(&["batch/present", "batch/missing"])
            .expect("MGET with one missing key remains a successful SDK call");

        assert_eq!(values.len(), 2);
        assert_eq!(
            values[0].as_ref().map(|value| value.bytes.as_slice()),
            Some(&b"visible"[..])
        );
        assert!(
            values[1].is_none(),
            "missing MGET item must be Ok(None), not a DmsError"
        );
    }

    #[test]
    fn sdk_receives_node_arena_capacity_error_over_tcp() {
        let server = TestServer::start_with_capacity(TestAddress::Tcp, 4096);
        let client =
            DmsClient::connect(&server.endpoint, ClientOptions::default()).expect("connect");

        let error = client
            .set("channel/too-large", &[b'x'; 4097])
            .expect_err("payload exceeds the configured Node arena");

        assert_eq!(error.code(), dms_error::NODE_ARENA_CAPACITY_EXHAUSTED);
        assert_eq!(error.kind(), dms_error::ErrorKind::ResourceExhausted);
    }

    #[test]
    fn sdk_mset_cleans_prior_staging_when_later_entry_fails() {
        let server = TestServer::start_with_capacity(TestAddress::Tcp, 4096);
        let client =
            DmsClient::connect(&server.endpoint, ClientOptions::default()).expect("connect");

        let entries = [
            KvEntry::new("mset/cleanup/first", vec![b'a'; 3000]).expect("first entry"),
            KvEntry::new("mset/cleanup/second", vec![b'b'; 4097]).expect("second entry"),
        ];
        let error = client
            .mset(&entries, MSetOptions::default())
            .expect_err("second staging allocation must exceed the arena");

        assert_eq!(error.code(), dms_error::NODE_ARENA_CAPACITY_EXHAUSTED);

        client
            .mset(
                &[KvEntry::new("mset/cleanup/reused", vec![b'c'; 3000]).expect("reused entry")],
                MSetOptions::default(),
            )
            .expect("first staging allocation was cleaned immediately, not by TTL");
    }

    #[test]
    fn sdk_receives_meta_journal_code_without_node_remapping() {
        let server = TestServer::start_with_meta_commit_failure(TestAddress::Tcp);
        let client =
            DmsClient::connect(&server.endpoint, ClientOptions::default()).expect("connect");

        let error = client
            .set("channel/meta-journal-fails", b"value")
            .expect_err("fake Meta commit must fail");

        assert_eq!(error.code(), dms_error::META_JOURNAL_APPEND_FAILED);
        assert_eq!(error.kind(), dms_error::ErrorKind::Unavailable);
        assert!(
            error.message().contains("fake journal append failed"),
            "unexpected message: {}",
            error.message()
        );
    }

    #[test]
    fn thin_reader_observes_updates_without_a_private_value_cache() {
        let server = TestServer::start(TestAddress::Tcp);
        let reader_registry = registry();
        let writer =
            DmsClient::connect(&server.endpoint, ClientOptions::default()).expect("writer");
        let reader = DmsClient::connect(
            &server.endpoint,
            ClientOptions {
                metrics_registry: Some(reader_registry.clone()),
                ..ClientOptions::default()
            },
        )
        .expect("reader");
        let key = Key::new("channel/invalidation").expect("key");

        writer.set(key.clone(), b"v1").expect("set v1");
        assert_eq!(
            reader.get(key.clone()).expect("get v1"),
            Some(b"v1".to_vec())
        );
        // 重复调用只能复用 Node 侧数据，SDK 不再缓存另一份 value。
        for _ in 0..2 {
            assert_eq!(
                reader.get(key.clone()).expect("repeat get v1"),
                Some(b"v1".to_vec())
            );
        }
        writer.set(key.clone(), b"v2").expect("set v2");

        // SET 成功已跨过失效屏障；首次读取就必须看到新 Current，不能靠轮询掩盖旧值。
        assert_eq!(
            reader.get(key.clone()).expect("first current after SET"),
            Some(b"v2".to_vec())
        );

        writer.del(key.clone()).expect("delete");
        assert!(reader.get(key).expect("first current after DEL").is_none());

        let metrics = encode_text(&reader_registry).expect("encode reader metrics");
        assert!(
            !metrics.contains("dms_client_cache_"),
            "thin SDK must not register obsolete value cache metrics:\n{metrics}"
        );
    }

    #[test]
    fn explicit_shared_write_and_view_work_over_unix_domain_socket() {
        let server = TestServer::start(TestAddress::Unix);
        let client = DmsClient::connect(
            &server.endpoint,
            ClientOptions {
                shared_memory: Some(true),
                ..ClientOptions::default()
            },
        )
        .expect("connect");

        let mut buffer = client
            .allocate_write("channel/shm-explicit", 11)
            .expect("allocate shared write");
        buffer
            .as_mut_slice()
            .expect("write slice")
            .copy_from_slice(b"hello-shm!!");
        let result = client.commit_shared(buffer).expect("commit shared");
        assert_eq!(result.version, ObjectVersion(1));

        let view = client
            .get_view("channel/shm-explicit")
            .expect("get view")
            .expect("present");
        assert_eq!(view.version(), ObjectVersion(1));
        assert_eq!(view.as_slice().expect("view bytes"), b"hello-shm!!");

        // A second allocation stays in the same Region. The SDK sees a cache
        // hit, so no second AcquireRegion/SCM_RIGHTS/mmap cycle occurs.
        let mut second = client
            .allocate_write("channel/shm-second", 6)
            .expect("allocate second shared write");
        second
            .as_mut_slice()
            .expect("second write slice")
            .copy_from_slice(b"second");
        client.commit_shared(second).expect("commit second");
        assert_eq!(
            client.get("channel/shm-second").expect("read second"),
            Some(b"second".to_vec())
        );
        // 显式 View 仍指向旧版本；普通 GET 返回用户独立拥有的 Vec。
        // 不能因为另一次普通 GET 完成，就让活动 View 指向被覆盖的 bytes。
        client.set("channel/shm-explicit", b"new-value!!").unwrap();
        for _ in 0..16 {
            assert_eq!(
                client.get("channel/shm-explicit").unwrap(),
                Some(b"new-value!!".to_vec())
            );
            assert_eq!(view.as_slice().unwrap(), b"hello-shm!!");
        }
        drop(view);
        assert_eq!(
            client.get("channel/shm-explicit").unwrap(),
            Some(b"new-value!!".to_vec())
        );
        assert_eq!(
            server
                .fd_broker
                .as_ref()
                .expect("UDS broker")
                .served_count(),
            1,
            "one Client maps one Region exactly once"
        );
    }

    #[test]
    fn explicit_shared_write_is_not_silently_copied_over_tcp() {
        let server = TestServer::start(TestAddress::Tcp);
        let client = DmsClient::connect(
            &server.endpoint,
            ClientOptions {
                shared_memory: Some(true),
                ..ClientOptions::default()
            },
        )
        .expect("connect");

        let error = match client.allocate_write("channel/tcp-no-shm", 4) {
            Ok(_) => panic!("tcp must not receive fd broker descriptors"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), dms_client::ErrorKind::Unimplemented);
    }
}

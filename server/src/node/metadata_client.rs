//! dms-node 到单一 dms-meta 的 gRPC 客户端。
//!
//! 这里负责 protobuf DTO 和连接细节；Node actor 只调用 `resolve/commit/watch/ack`
//! 这些当前真实业务动作。`Channel` 可安全 clone，并共享底层 HTTP/2 连接。

use std::{future::Future, sync::Arc, time::Duration};

use dms_error::{DmsError, ErrorKind};
use dms_protocol::v1 as pb;
use dms_transport::{GrpcConfig, SecurityManager, TlsConfig, status_to_dms_error_with};
use pb::metadata_service_client::MetadataServiceClient as GrpcMetadataClient;
use tokio::sync::RwLock;
use tonic::{Response, Status, transport::Endpoint};

pub(crate) struct BatchValueCommit {
    pub(crate) key: Vec<u8>,
    pub(crate) block_id: Vec<u8>,
    pub(crate) length: u64,
    pub(crate) checksum: Vec<u8>,
    pub(crate) operation_id: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct MetadataClient {
    client: GrpcMetadataClient<dms_tracing::TracedChannel>,
    session: Arc<RwLock<pb::NodeSessionIdentity>>,
    node_id: u64,
    data_endpoint: String,
    rpc_metrics: Option<dms_metrics::RpcMetrics>,
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
    ) -> Result<Self, DmsError> {
        let endpoint = Endpoint::from_shared(endpoint.to_string()).map_err(|error| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::InvalidArgument,
                format!("invalid Meta endpoint: {error}"),
            )
        })?;
        let endpoint = GrpcConfig::default().configure_client(endpoint);
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
        let mut client = GrpcMetadataClient::new(dms_tracing::traced_channel(channel.clone()));
        let session =
            Self::open_session(&mut client, node_id, &data_endpoint, rpc_metrics.as_ref()).await?;
        Ok(Self {
            client: GrpcMetadataClient::new(dms_tracing::traced_channel(channel)),
            session: Arc::new(RwLock::new(session)),
            node_id,
            data_endpoint,
            rpc_metrics,
        })
    }

    async fn open_session(
        client: &mut GrpcMetadataClient<dms_tracing::TracedChannel>,
        node_id: u64,
        data_endpoint: &str,
        rpc_metrics: Option<&dms_metrics::RpcMetrics>,
    ) -> Result<pb::NodeSessionIdentity, DmsError> {
        let result = observe_rpc(
            rpc_metrics,
            dms_metrics::RpcCall::META_OPEN_NODE_SESSION,
            client.open_node_session(pb::OpenNodeSessionRequest {
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
        opened.session.ok_or_else(|| {
            DmsError::new(
                dms_error::NODE_METADATA_UNAVAILABLE,
                ErrorKind::Unavailable,
                "Meta OpenNodeSession response is missing session identity",
            )
        })
    }

    async fn current_session(&self) -> pb::NodeSessionIdentity {
        self.session.read().await.clone()
    }

    /// Current Node incarnation assigned by Meta. This is the sole fencing
    /// identity for in-memory replicas; there is no duplicate storage epoch.
    pub(crate) async fn node_epoch(&self) -> u64 {
        self.current_session().await.node_epoch
    }

    /// 返回本次续租的有效毫秒数；调用方以发请求前的 Instant 计算保守截止时间，
    /// 不能以收到响应的时间再加 TTL，否则网络延迟会让 Node 比 Meta 更晚过期。
    pub(crate) async fn heartbeat(&self, event_cursor: u64) -> Result<u64, DmsError> {
        let mut session = self.current_session().await;
        let request = pb::NodeHeartbeatRequest {
            context: Some(context(self.node_id)),
            session: Some(session.clone()),
            resources: Some(pb::ResourceSummary {
                total_host_memory_bytes: 0,
                available_host_memory_bytes: 0,
                staged_bytes: 0,
                replica_bytes: 0,
            }),
            catalog_watermark: 0,
            event_cursor,
        };
        let mut client = self.client();
        let mut result = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_HEARTBEAT,
            client.heartbeat(request.clone()),
        )
        .await
        .map(|response| response.into_inner().lease_ttl_millis)
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client();
            result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_HEARTBEAT,
                client.heartbeat(pb::NodeHeartbeatRequest {
                    session: Some(session),
                    ..request
                }),
            )
            .await
            .map(|response| response.into_inner().lease_ttl_millis)
            .map_err(map_status);
        }
        result
    }

    async fn reopen_session(&self) -> Result<pb::NodeSessionIdentity, DmsError> {
        let mut client = self.client();
        let session = Self::open_session(
            &mut client,
            self.node_id,
            &self.data_endpoint,
            self.rpc_metrics.as_ref(),
        )
        .await?;
        dms_logging::info!(
            "Meta session reopened";
            "event" => "node.meta_session.reopened",
            "node_id" => self.node_id,
            "node_epoch" => session.node_epoch,
        );
        *self.session.write().await = session.clone();
        Ok(session)
    }

    pub(crate) async fn resolve(
        &self,
        key: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<pb::ResolveObjectResponse, DmsError> {
        let selector = exact_version
            .map(pb::resolve_object_request::Selector::ExactVersion)
            .unwrap_or(pb::resolve_object_request::Selector::Current(true));
        let mut session = self.current_session().await;
        let mut client = self.client();
        let mut result = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_RESOLVE_OBJECT,
            client.resolve_object(pb::ResolveObjectRequest {
                context: Some(context(self.node_id)),
                session: Some(session.clone()),
                key: Some(pb::Key { value: key.clone() }),
                selector: Some(selector),
                range: None,
                cache_current: true,
            }),
        )
        .await
        .map(|response| response.into_inner())
        .map_err(map_status);
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client();
            result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_RESOLVE_OBJECT,
                client.resolve_object(pb::ResolveObjectRequest {
                    context: Some(context(self.node_id)),
                    session: Some(session),
                    key: Some(pb::Key { value: key }),
                    selector: exact_version
                        .map(pb::resolve_object_request::Selector::ExactVersion)
                        .or(Some(pb::resolve_object_request::Selector::Current(true))),
                    range: None,
                    cache_current: true,
                }),
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
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::ReportReplicasRequest {
            context: Some(context(self.node_id)),
            session: Some(session.clone()),
            replicas: vec![pb::ReplicaReport {
                block_id: block_id.clone(),
                length,
                checksum: checksum.clone(),
                durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
            }],
            operation_id: operation_id.clone(),
            desired_copies: desired_copies.max(1),
            repair_id: repair_id.clone(),
        };
        let mut client = self.client();
        let mut result = observe_rpc(
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
            result = observe_rpc(
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
        let mut session = self.current_session().await;
        let mut client = self.client();
        let mut result = observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_COMMIT_VERSION,
            client.commit_version(pb::CommitVersionRequest {
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
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client();
            result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_COMMIT_VERSION,
                client.commit_version(pb::CommitVersionRequest {
                    context: Some(context(self.node_id)),
                    session: Some(session),
                    key: Some(pb::Key { value: key }),
                    candidate: Some(candidate),
                    condition,
                    expected_version: None,
                    operation_id,
                    operation_digest,
                    durability: pb::DurabilityPolicy::LocalMemory as i32,
                    required_memory_copies: 1,
                    replica_proofs,
                    new_replicas,
                }),
            )
            .await
            .map(|response| response.into_inner())
            .map_err(map_status);
        }
        result
    }

    pub(crate) async fn watch_events(
        &self,
        last_acked_cursor: u64,
    ) -> Result<tonic::Streaming<pb::NodeEvent>, DmsError> {
        let mut session = self.current_session().await;
        let mut client = self.client.clone();
        let mut result = observe_rpc(
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
            result = observe_rpc(
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
        let mut result = observe_rpc(
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
            result = observe_rpc(
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

    fn client(&self) -> GrpcMetadataClient<dms_tracing::TracedChannel> {
        self.client.clone()
    }
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

fn context(node_id: u64) -> pb::RequestContext {
    pb::RequestContext {
        node_id,
        principal: format!("node-{node_id}"),
        timeout_millis: Duration::from_secs(30).as_millis() as u64,
    }
}

pub(crate) fn digest(bytes: &[u8]) -> Vec<u8> {
    dms_transport::checksum::fnv1a_bytes(bytes).to_vec()
}

fn is_reopenable_session_error(error: &DmsError) -> bool {
    error.code() == dms_error::META_SESSION_UNKNOWN
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
        let channel = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        let client = MetadataClient {
            client: GrpcMetadataClient::new(dms_tracing::traced_channel(channel)),
            session: Arc::new(RwLock::new(pb::NodeSessionIdentity {
                session_id: b"stale-session".to_vec(),
                node_id: 9,
                node_epoch: 1,
            })),
            node_id: 9,
            data_endpoint: "http://127.0.0.1:0".to_string(),
            rpc_metrics: None,
        };
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
}

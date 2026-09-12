//! dms-node 到单一 dms-meta 的 gRPC 客户端。
//!
//! 这里负责 protobuf DTO 和连接细节；Node actor 只调用 `resolve/commit/watch/ack`
//! 这些当前真实业务动作。`Channel` 可安全 clone，并共享底层 HTTP/2 连接。

use std::{
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
use pb::metadata_service_client::MetadataServiceClient as GrpcMetadataClient;
use tokio::sync::{Mutex, MutexGuard, RwLock, mpsc, oneshot};
use tonic::{Response, Status, transport::Endpoint};

const METADATA_RESOLVE_BATCH_MAX: usize = 64;

struct ResolveJob {
    key: Vec<u8>,
    exact_version: Option<u64>,
    reply: oneshot::Sender<Result<pb::ResolveObjectResponse, DmsError>>,
}

/// 解析批处理任务只持有发 RPC 所需的共享状态，不持有发送端，避免后台任务和
/// 自己消费的 channel 形成生命周期环。它不是新的业务模块。
struct ResolveBatchRpc {
    client: GrpcMetadataClient<dms_tracing::TracedChannel>,
    session: Arc<RwLock<pb::NodeSessionIdentity>>,
    next_commit_sequence: Arc<AtomicU64>,
    node_id: u64,
    data_endpoint: String,
    rpc_metrics: Option<dms_metrics::RpcMetrics>,
}

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
    // 每个逻辑提交仅分配一次，克隆连接与 RPC 重试共享计数器。它不是用户
    // operation_id 的替代；仅让 Meta 在历史幂等结果裁剪后拒绝旧 wire 请求。
    next_commit_sequence: Arc<AtomicU64>,
    // Meta 使用每个 Node 单调递增的 commit_sequence 拒绝旧请求。HTTP/2 允许
    // 并发 RPC 乱序抵达，因此必须在 Node 出口保持“分配序号 + 完成提交”的顺序；
    // 否则较大的序号先提交后，仍在途的较小序号会被误判为重放。锁只覆盖
    // commit_version/commit_batch，不串行化 resolve、watch、heartbeat 或数据面。
    commit_gate: Arc<Mutex<()>>,
    node_id: u64,
    data_endpoint: String,
    rpc_metrics: Option<dms_metrics::RpcMetrics>,
    resolve_tx: mpsc::Sender<ResolveJob>,
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
        let mut client = metadata_client(channel.clone());
        let (session, minimum_sequence) =
            Self::open_session(&mut client, node_id, &data_endpoint, rpc_metrics.as_ref()).await?;
        let client = metadata_client(channel);
        let session = Arc::new(RwLock::new(session));
        let next_commit_sequence = Arc::new(AtomicU64::new(minimum_sequence.max(1)));
        let (resolve_tx, resolve_rx) = mpsc::channel(METADATA_RESOLVE_BATCH_MAX * 4);
        tokio::spawn(run_resolve_batcher(
            ResolveBatchRpc {
                client: client.clone(),
                session: Arc::clone(&session),
                next_commit_sequence: Arc::clone(&next_commit_sequence),
                node_id,
                data_endpoint: data_endpoint.clone(),
                rpc_metrics: rpc_metrics.clone(),
            },
            resolve_rx,
        ));
        Ok(Self {
            client,
            session,
            next_commit_sequence,
            commit_gate: Arc::new(Mutex::new(())),
            node_id,
            data_endpoint,
            rpc_metrics,
            resolve_tx,
        })
    }

    async fn open_session(
        client: &mut GrpcMetadataClient<dms_tracing::TracedChannel>,
        node_id: u64,
        data_endpoint: &str,
        rpc_metrics: Option<&dms_metrics::RpcMetrics>,
    ) -> Result<(pb::NodeSessionIdentity, u64), DmsError> {
        let result = observe_rpc(
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
        self.session.read().await.clone()
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
        *self.session.write().await = session.clone();
        Ok(session)
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

    pub(crate) async fn stat(&self, key: Vec<u8>) -> Result<pb::MetaStatResponse, DmsError> {
        let mut session = self.current_session().await;
        let make_request = |session: pb::NodeSessionIdentity| pb::MetaStatRequest {
            context: Some(context(self.node_id)),
            session: Some(session),
            key: Some(pb::Key { value: key.clone() }),
        };
        let mut client = self.client();
        let mut result = observe_rpc(
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
            result = observe_rpc(
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
        let mut result = observe_rpc(
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
            result = observe_rpc(
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
        let (_commit_guard, commit_sequence) = self.begin_commit().await?;
        let mut session = self.current_session().await;
        let mut client = self.client();
        let mut result = observe_rpc(
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
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            let mut client = self.client();
            result = observe_rpc(
                self.rpc_metrics.as_ref(),
                dms_metrics::RpcCall::META_COMMIT_VERSION,
                client.commit_version(pb::CommitVersionRequest {
                    commit_sequence,
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
        let mut result = observe_rpc(
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
            result = observe_rpc(
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
}

async fn run_resolve_batcher(mut rpc: ResolveBatchRpc, mut receive: mpsc::Receiver<ResolveJob>) {
    while let Some(first) = receive.recv().await {
        // 单请求立即发送；仅吸收此刻已经排队的并发请求，不设置聚合定时器。
        let mut jobs = Vec::with_capacity(METADATA_RESOLVE_BATCH_MAX);
        jobs.push(first);
        while jobs.len() < METADATA_RESOLVE_BATCH_MAX {
            match receive.try_recv() {
                Ok(job) => jobs.push(job),
                Err(_) => break,
            }
        }
        let result = rpc.resolve(&jobs).await;
        deliver_resolve_results(jobs, result);
    }
}

impl ResolveBatchRpc {
    async fn resolve(
        &mut self,
        jobs: &[ResolveJob],
    ) -> Result<Vec<pb::ResolveObjectResult>, DmsError> {
        let mut session = self.session.read().await.clone();
        let mut result = self.resolve_once(jobs, session.clone()).await;
        if result.as_ref().is_err_and(is_reopenable_session_error) {
            session = self.reopen_session().await?;
            result = self.resolve_once(jobs, session).await;
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
        observe_rpc(
            self.rpc_metrics.as_ref(),
            dms_metrics::RpcCall::META_RESOLVE_OBJECTS,
            self.client.resolve_objects(request),
        )
        .await
        .map(|response| response.into_inner().results)
        .map_err(map_status)
    }

    async fn reopen_session(&mut self) -> Result<pb::NodeSessionIdentity, DmsError> {
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
        *self.session.write().await = session.clone();
        Ok(session)
    }
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
    dms_transport::checksum::stable_digest_bytes(bytes).to_vec()
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
    fn commit_sequence_is_shared_and_never_wraps() {
        let next = Arc::new(AtomicU64::new(1));
        let clone = next.clone();
        assert_eq!(allocate_commit_sequence(&next).unwrap(), 1);
        assert_eq!(allocate_commit_sequence(&clone).unwrap(), 2);
        next.store(u64::MAX, Ordering::Relaxed);
        assert!(allocate_commit_sequence(&next).is_err());
        assert_eq!(next.load(Ordering::Relaxed), u64::MAX);
    }

    #[tokio::test]
    async fn commit_sequence_is_allocated_only_after_commit_gate() {
        let channel = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        let client = MetadataClient {
            client: metadata_client(channel),
            session: Arc::new(RwLock::new(pb::NodeSessionIdentity {
                session_id: b"test-session".to_vec(),
                node_id: 9,
                node_epoch: 1,
            })),
            next_commit_sequence: Arc::new(AtomicU64::new(1)),
            commit_gate: Arc::new(Mutex::new(())),
            node_id: 9,
            data_endpoint: "http://127.0.0.1:0".to_string(),
            rpc_metrics: None,
            resolve_tx: mpsc::channel(1).0,
        };

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
            client: metadata_client(channel),
            session: Arc::new(RwLock::new(pb::NodeSessionIdentity {
                session_id: b"stale-session".to_vec(),
                node_id: 9,
                node_epoch: 1,
            })),
            next_commit_sequence: Arc::new(AtomicU64::new(1)),
            commit_gate: Arc::new(Mutex::new(())),
            node_id: 9,
            data_endpoint: "http://127.0.0.1:0".to_string(),
            rpc_metrics: None,
            resolve_tx: mpsc::channel(1).0,
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

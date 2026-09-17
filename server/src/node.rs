//! 每个计算节点上的 dms-node 进程组合根。
//!
//! 本文件负责把进程身份、health listener、共享 Node owner、generated Service 和
//! TCP/UDS listener 组装起来。具体对象、Arena、元数据逻辑仍在各业务模块中。

// 当前 crate 禁止使用 unsafe；未来引入 mmap/RDMA 时必须在更小的边界单独评审。
#![deny(unsafe_code)]
// 只有 Arena owner 承接可写 memfd 导出的跨进程互斥证明。
#[allow(unsafe_code)]
mod arena_manager;
mod current_cache;
mod data_core;
mod filesystem;
mod image;
mod kkv_operations;
mod metadata_client;
mod metrics;
mod peer_service;
#[cfg(test)]
mod read_cache_tests;
mod replica_reporter;
mod runtime;
mod version_layout;
mod worker_service;

use std::{
    collections::HashSet,
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use dms_logging::LevelController;
use dms_protocol::v1::{
    node_event, peer_service_server::PeerServiceServer,
    worker_payload_service_server::WorkerPayloadServiceServer,
    worker_service_server::WorkerServiceServer,
};
use dms_transport::{GrpcConfig, SecurityManager, TlsConfig};
use tokio::net::{TcpListener as TokioTcpListener, UnixListener};
use tokio_stream::wrappers::UnixListenerStream;

use crate::health::{Readiness, ReadinessState, serve_status};
use crate::{ComponentKind, NodeId};
use arena_manager::SharedFdBroker;
use filesystem::kernel_cache::KernelCacheInvalidator;
use metadata_client::{MetadataClient, retirement_block_ids};
use metrics::NodeMetrics;
use peer_service::PeerServiceHandler;
use runtime::{NodeHandle, NodeTaskConfig, ReplicaPrepareSpec};
use worker_service::WorkerServiceHandler;

type RetirementPhaseKey = (Vec<u8>, i32);
const RETIREMENT_PHASE_ACK_ATTEMPTS: usize = 5;

/// Minimal configuration required to start the data-node process shell.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    /// Identity reported by logs, metrics, and health responses.
    pub node_id: NodeId,
    /// TCP address of the M0 operational health listener.
    pub health_address: String,
    /// Optional remote Client endpoint, for example `127.0.0.1:19200`.
    pub worker_tcp_address: Option<String>,
    /// Optional local Client Unix-domain socket.
    pub worker_uds_path: Option<PathBuf>,
    /// Node→Meta gRPC endpoint，例如 `http://127.0.0.1:19300`。
    pub meta_endpoint: String,
    /// Host-memory payload arena capacity for this Node process.
    pub arena_capacity_bytes: u64,
    /// 每次向 OS 扩容的目标大小；多个 value 的 Slot 复用同一 Region。
    pub region_size_bytes: u64,
    /// How long an uncommitted staging allocation may remain idle.
    pub staging_ttl: Duration,
    /// 对 Client 授予的 Current 缓存租约上限；还会被 Meta 剩余租约裁短。
    pub client_cache_lease_ttl: Duration,
    /// Node 缓存布局的收费预算；0 关闭，不包含 payload bytes。
    pub node_current_cache_bytes: u64,
    /// 单个布局的最长缓存时间；实际还受 Meta grant 截断。
    pub node_current_cache_ttl: Duration,
    /// Shared handle used by the future online-config endpoint.
    pub log_level: LevelController,
    /// Process-owned trace runtime; SDKs deliberately do not install one.
    pub tracing: dms_tracing::TracingConfig,
    /// Optional Linux FUSE mountpoint. When absent, dms-node keeps the pure KV runtime shape.
    #[cfg(all(target_os = "linux", feature = "fuse"))]
    pub fuse_mountpoint: Option<PathBuf>,
}

/// Starts the data-node process shell and blocks until it is stopped.
pub fn serve(config: NodeConfig) -> Result<(), Box<dyn std::error::Error>> {
    if config.worker_tcp_address.is_none() && config.worker_uds_path.is_none() {
        return Err("dms-node requires --worker-tcp-address or --worker-uds-path".into());
    }
    // main/serve 本身是同步函数，因此在这里创建 Tokio 多线程 runtime。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // block_on 驱动异步 Server Future，直到 listener 停止或发生错误。
    runtime.block_on(serve_workers(config))?;
    Ok(())
}

async fn serve_workers(config: NodeConfig) -> Result<(), Box<dyn std::error::Error>> {
    let readiness = ReadinessState::default();
    let registry = dms_metrics::registry();
    let node_metrics = NodeMetrics::register(&registry)?;
    let rpc_metrics = dms_metrics::RpcMetrics::register(&registry)?;
    let error_metrics = dms_metrics::ErrorMetrics::register(&registry)?;
    let trace_metrics = dms_metrics::TraceRuntimeMetrics::register(&registry)?;
    let _tracing_guard = dms_tracing::init_process_tracing(
        &config.tracing,
        dms_tracing::ProcessIdentity::new("dms-node", config.node_id.to_string()),
        Some(trace_metrics),
    )?;
    let trace_periodic_operations = config.tracing.periodic_operations;
    #[cfg(feature = "reliability-faults")]
    let peer_pull_fault_gate = peer_service::PeerPullFaultGate::from_env()
        .map_err(|error| format!("invalid peer pull reliability fault config: {error}"))?;
    let health_listener = TokioTcpListener::bind(&config.health_address).await?;
    let health_address = health_listener.local_addr()?;

    // Bind every business listener before registering with Meta. A process can
    // therefore never report READY while its advertised endpoint is unusable.
    let tcp_listener = match config.worker_tcp_address.as_deref() {
        Some(address) => Some(TokioTcpListener::bind(address).await?),
        None => None,
    };
    let tcp_address = tcp_listener
        .as_ref()
        .map(TokioTcpListener::local_addr)
        .transpose()?;
    let uds_listener = match config.worker_uds_path.as_ref() {
        Some(path) => Some(bind_worker_uds(path)?),
        None => None,
    };
    let data_endpoint = tcp_address
        .map(|address| format!("http://{address}"))
        .or_else(|| {
            config
                .worker_uds_path
                .as_ref()
                .map(|path| format!("unix://{}", path.display()))
        })
        .expect("business endpoint was validated before runtime startup");
    let node_id = config.node_id.to_string();
    let numeric_node_id = stable_node_id(&node_id);
    #[cfg(all(target_os = "linux", feature = "fuse"))]
    let filesystem_enabled = config.fuse_mountpoint.is_some();
    #[cfg(not(all(target_os = "linux", feature = "fuse")))]
    let filesystem_enabled = false;
    let metadata = MetadataClient::connect(
        &config.meta_endpoint,
        numeric_node_id,
        data_endpoint,
        Some(rpc_metrics.clone()),
        filesystem_enabled,
    )
    .await
    .map_err(|error| format!("failed to register dms-node with Meta: {error:?}"))?;
    let shared_fd_broker = match config.worker_uds_path.as_ref() {
        Some(path) => {
            let mut broker_path = path.clone();
            broker_path.set_extension("fd.sock");
            let broker = SharedFdBroker::bind(broker_path)?;
            let broker_task = broker.clone();
            let broker_metrics = node_metrics.clone();
            std::thread::Builder::new()
                .name("dms-shm-fd-broker".to_string())
                .spawn(move || {
                    loop {
                        match broker_task.serve_one() {
                            Ok(()) => broker_metrics
                                .record_shm_fd_grant(metrics::ShmFdGrantResult::Claimed),
                            Err(error) => {
                                broker_metrics
                                    .record_shm_fd_grant(metrics::ShmFdGrantResult::Error);
                                dms_logging::error!(
                                    "shared-memory FD broker request failed";
                                    "event" => "node.shm.fd_broker.failed",
                                    "error" => error.to_string(),
                                );
                            }
                        }
                    }
                })?;
            Some(broker)
        }
        None => None,
    };
    // Client 与 Peer Handler 共享唯一 Node owner；不存在第二份 Peer/Worker 状态。
    let node = NodeHandle::spawn_with_metrics(
        node_id,
        metadata.clone(),
        NodeTaskConfig {
            arena_capacity_bytes: config.arena_capacity_bytes,
            region_size_bytes: config.region_size_bytes,
            staging_ttl: config.staging_ttl,
            client_cache_lease_ttl: config.client_cache_lease_ttl,
            node_current_cache_bytes: config.node_current_cache_bytes,
            node_current_cache_ttl: config.node_current_cache_ttl,
            shared_fd_broker,
            log_level: config.log_level,
            trace_periodic_operations,
        },
        node_metrics,
        rpc_metrics.clone(),
    );
    // Establish the Watch before opening Client listeners. This closes the
    // startup window in which a committed version could have no invalidation path.
    let initial_watch = metadata
        .watch_events(0)
        .await
        .map_err(|error| format!("failed to establish Meta watch: {error:?}"))?;
    let lease_started = std::time::Instant::now();
    let initial_resources = node
        .resource_summary()
        .await
        .map_err(|error| format!("failed to read initial Arena resources: {error:?}"))?;
    let lease_ttl = metadata
        .heartbeat(0, initial_resources)
        .await
        .map_err(|error| format!("failed to establish Meta lease: {error:?}"))?;
    node.metadata_lease(
        Some(lease_started + Duration::from_millis(lease_ttl)),
        // 缓存资格由实际 Watch 接收任务打开；这里只安装上游租约。
        None,
    )
    .await
    .map_err(|error| format!("failed to install Meta lease: {error:?}"))?;
    let event_cursor = Arc::new(AtomicU64::new(0));
    #[cfg(all(target_os = "linux", feature = "fuse"))]
    let kernel_cache =
        KernelCacheInvalidator::new(config.fuse_mountpoint.is_some(), node.metrics());
    #[cfg(not(all(target_os = "linux", feature = "fuse")))]
    let kernel_cache = KernelCacheInvalidator::disabled(node.metrics());
    #[cfg(all(target_os = "linux", feature = "fuse"))]
    // FUSE 和 Worker TCP/UDS 一样都是对外入口。这里已经建立 Meta Watch stream
    // 和 lease；挂载时先拿到内核 notifier 并安装到 KernelCacheInvalidator，
    // 然后才启动 Watch 消费任务。这样远端文件事件不会在 notifier 安装前被 ACK。
    let _fuse_session = match config.fuse_mountpoint.clone() {
        Some(mountpoint) => Some(filesystem::fuse::start(
            mountpoint.clone(),
            node.clone(),
            tokio::runtime::Handle::current(),
            kernel_cache.clone(),
        )?),
        None => None,
    };
    let watch_task = tokio::spawn(consume_meta_events(
        metadata.clone(),
        node.clone(),
        numeric_node_id,
        Some(initial_watch),
        event_cursor.clone(),
        kernel_cache,
    ));
    let heartbeat_task = tokio::spawn(send_meta_heartbeats(metadata, node.clone(), event_cursor));

    readiness.set(Readiness::Ready);
    dms_logging::info!(
        "dms-node is ready";
        "event" => "node.process.ready",
        "node_id" => config.node_id.to_string(),
        "health_address" => health_address.to_string(),
        "worker_tcp" => tcp_address.map(|address| address.to_string()),
        "worker_uds" => config.worker_uds_path.as_ref().map(|path| path.display().to_string()),
        "protocol_version" => 1,
    );

    let status = serve_status(
        health_listener,
        ComponentKind::Node,
        config.node_id,
        readiness.clone(),
        registry,
    );
    let workers = serve_bound_workers(
        tcp_listener,
        uds_listener,
        node,
        rpc_metrics,
        error_metrics,
        trace_periodic_operations,
        #[cfg(feature = "reliability-faults")]
        peer_pull_fault_gate,
    );
    tokio::pin!(status);
    tokio::pin!(workers);
    let result = tokio::select! {
        result = &mut status => result.map_err(|error| error.into()),
        result = &mut workers => result,
        result = watch_task => Err(format!("Meta watch task stopped unexpectedly: {result:?}").into()),
        result = heartbeat_task => Err(format!("Meta heartbeat task stopped unexpectedly: {result:?}").into()),
    };
    readiness.set(Readiness::Stopping);
    result
}

async fn consume_meta_events(
    metadata: MetadataClient,
    node: NodeHandle,
    local_node_id: u64,
    mut initial_stream: Option<tonic::Streaming<dms_protocol::v1::NodeEvent>>,
    acked_cursor: Arc<AtomicU64>,
    kernel_cache: KernelCacheInvalidator,
) {
    let mut last_acked_cursor = 0;
    let mut reconnect_delay = std::time::Duration::from_millis(100);
    let active_retirement_phases = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
    loop {
        let request_cursor = last_acked_cursor;
        let mut stream = match initial_stream.take() {
            Some(stream) => stream,
            None => {
                let Ok(stream) = metadata.watch_events(request_cursor).await else {
                    tokio::time::sleep(reconnect_delay).await;
                    reconnect_delay = (reconnect_delay * 2).min(std::time::Duration::from_secs(5));
                    continue;
                };
                stream
            }
        };
        dms_logging::info!(
            "Meta event watch established";
            "event" => "node.meta_watch.connected",
            "last_acked_cursor" => last_acked_cursor,
            "request_cursor" => request_cursor,
            "event_cursor" => acked_cursor.load(Ordering::Relaxed),
        );
        reconnect_delay = std::time::Duration::from_millis(100);
        if node.metadata_lease(None, Some(true)).await.is_err() {
            return;
        }
        loop {
            let event = match stream.message().await {
                Ok(Some(event)) => event,
                Ok(None) => {
                    dms_logging::info!(
                        "Meta event watch closed by server";
                        "event" => "node.meta_watch.disconnected",
                        "last_acked_cursor" => last_acked_cursor,
                        "request_cursor" => request_cursor,
                        "event_cursor" => acked_cursor.load(Ordering::Relaxed),
                    );
                    break;
                }
                Err(error) => {
                    dms_logging::warn!(
                        "Meta event watch failed";
                        "event" => "node.meta_watch.disconnected",
                        "last_acked_cursor" => last_acked_cursor,
                        "request_cursor" => request_cursor,
                        "event_cursor" => acked_cursor.load(Ordering::Relaxed),
                        "error" => error.to_string(),
                    );
                    break;
                }
            };
            let event_cursor = event.cursor;
            // Apply first, ACK second. A reconnect therefore safely replays an
            // event whose response was lost after the idempotent application.
            {
                match &event.event {
                    Some(node_event::Event::InvalidateCurrent(invalidation)) => {
                        if let Some(key) = &invalidation.key
                            && node
                                .invalidate_current(key.value.clone(), invalidation.minimum_version)
                                .await
                                .is_err()
                        {
                            break;
                        }
                    }
                    Some(node_event::Event::InvalidateFilesystemBinding(invalidation))
                        if should_apply_filesystem_invalidation(
                            local_node_id,
                            invalidation.source_node_id,
                        ) =>
                    {
                        let bindings = std::iter::once((
                            invalidation.inode,
                            invalidation.through_generation,
                            invalidation.minimum_inode_revision,
                        ))
                        .chain(invalidation.additional_inodes.iter().map(|item| {
                            (
                                item.inode,
                                item.through_generation,
                                item.minimum_inode_revision,
                            )
                        }));
                        let mut binding_invalidation_failed = false;
                        for (inode, through_generation, minimum_inode_revision) in bindings {
                            if node
                                .invalidate_filesystem_binding(
                                    inode,
                                    through_generation,
                                    minimum_inode_revision,
                                )
                                .await
                                .is_err()
                            {
                                binding_invalidation_failed = true;
                                break;
                            }
                            if let Err(error) = kernel_cache.invalidate_inode(inode) {
                                dms_logging::warn!(
                                    "FUSE kernel cache invalidation failed; replaying Watch event before ACK";
                                    "event" => "node.filesystem.kernel_cache_invalidate.failed",
                                    "inode" => inode,
                                    "through_generation" => through_generation,
                                    "minimum_inode_revision" => minimum_inode_revision,
                                    "error" => error.to_string(),
                                );
                                binding_invalidation_failed = true;
                                break;
                            }
                        }
                        if binding_invalidation_failed {
                            break;
                        }
                        let mut dentry_invalidation_failed = false;
                        for dentry in &invalidation.invalidated_dentries {
                            if let Err(error) =
                                kernel_cache.invalidate_entry(dentry.parent, &dentry.name)
                            {
                                dms_logging::warn!(
                                    "FUSE kernel dentry invalidation failed; replaying Watch event before ACK";
                                    "event" => "node.filesystem.kernel_dentry_invalidate.failed",
                                    "parent" => dentry.parent,
                                    "name_length" => dentry.name.len(),
                                    "error" => error.to_string(),
                                );
                                dentry_invalidation_failed = true;
                                break;
                            }
                        }
                        if dentry_invalidation_failed {
                            break;
                        }
                    }
                    // 本节点刚发布的写已经同步更新本地 binding/cache；Meta 仍会把同一
                    // invalidation 广播回来，但不能据此清掉写入方刚建立的热缓存。
                    Some(node_event::Event::InvalidateFilesystemBinding(_)) => {}
                    Some(node_event::Event::RepairReplica(repair)) => {
                        if let Err(error) = apply_repair_event(&metadata, &node, repair).await {
                            dms_logging::error!(
                                "replica repair failed";
                                "event" => "node.repair.failed",
                                "error" => error,
                            );
                            // Failure is represented by the absence of ACK.
                            // Breaking the stream causes Meta to replay this
                            // idempotent repair from the last durable cursor.
                            break;
                        }
                    }
                    Some(node_event::Event::EvictReplica(evict)) => {
                        dms_logging::info!(
                            "replica eviction event observed";
                            "event" => "node.replica.evict_requested",
                            "block_id" => hex(&evict.block_id),
                        );
                        apply_retirement_event(
                            metadata.clone(),
                            node.clone(),
                            evict.clone(),
                            active_retirement_phases.clone(),
                        )
                        .await;
                    }
                    Some(node_event::Event::FenceNode(fence)) => {
                        // epoch/fence 消息不能留下可再次命中的旧 Current 布局。
                        if node.metadata_lease(None, Some(false)).await.is_err() {
                            return;
                        }
                        dms_logging::warn!(
                            "stale node epoch was fenced";
                            "event" => "node.session.fenced",
                            "fenced_node_id" => fence.node_id,
                            "stale_epoch" => fence.stale_node_epoch,
                        );
                    }
                    Some(node_event::Event::Gap(gap)) => {
                        // 无法证明重放完整时关闭缓存；重建 Watch 后才重新申请资格。
                        if node.metadata_lease(None, Some(false)).await.is_err() {
                            return;
                        }
                        dms_logging::warn!(
                            "Meta event stream reported a replay gap";
                            "event" => "node.meta_watch.gap",
                            "next_cursor" => gap.next_cursor,
                        );
                    }
                    None => {}
                }
            }
            if let Err(error) = metadata.acknowledge_event(&event).await {
                // ACK 与 Watch 读取使用同一条逻辑事件通道。ACK 失败后必须丢弃
                // 当前 stream，并从最后一个已确认 cursor 重连重放；显式记录这一
                // 断点，避免现场只看到后续 reconnect，却不知道为什么重放。
                dms_logging::warn!(
                    "Meta event acknowledgement failed; reconnecting watch";
                    "event" => "node.meta_watch.disconnected",
                    "last_acked_cursor" => last_acked_cursor,
                    "request_cursor" => request_cursor,
                    "event_cursor" => event_cursor,
                    "error" => error.to_string(),
                );
                break;
            }
            last_acked_cursor = last_acked_cursor.max(event_cursor);
            acked_cursor.store(last_acked_cursor, Ordering::Relaxed);
            dms_logging::info!(
                "Meta event acknowledged";
                "event" => "node.meta_watch.acknowledged",
                "last_acked_cursor" => last_acked_cursor,
                "request_cursor" => request_cursor,
                "event_cursor" => event_cursor,
            );
        }
        // 只停止新缓存租约；已授出的租约仍由 Node/Meta 等待其真实到期。
        if node.metadata_lease(None, Some(false)).await.is_err() {
            return;
        }
    }
}

fn should_apply_filesystem_invalidation(local_node_id: u64, source_node_id: u64) -> bool {
    // 本 Node 的 namespace 提交已经用响应中的精确名字和 revision 更新本地缓存；
    // 再应用广播事件只会把刚安装的新 grant 清掉。空 source 来自旧 journal，必须按
    // 远端事件处理，保证滚动升级期间不会漏失效。
    source_node_id == 0 || source_node_id != local_node_id
}

async fn apply_repair_event(
    metadata: &MetadataClient,
    node: &NodeHandle,
    repair: &dms_protocol::v1::RepairReplicaEvent,
) -> Result<(), String> {
    let target = repair
        .target
        .as_ref()
        .ok_or_else(|| "repair event has no target".to_string())?;
    if target.node_id != metadata.node_id() {
        // Meta normally filters targeted events. Keeping this guard makes a
        // replay or older Meta harmless rather than copying onto every Node.
        return Ok(());
    }
    let source = repair
        .sources
        .iter()
        .find(|source| source.data_endpoint.starts_with("http://"))
        .ok_or_else(|| "repair event has no reachable gRPC source".to_string())?;
    let status = node
        .prepare_replica(ReplicaPrepareSpec {
            source_node_id: source.node_id.to_string(),
            source_endpoint: source.data_endpoint.clone(),
            plan_id: repair.repair_id.clone(),
            block_id: repair.block_id.clone(),
            expected_length: repair.expected_length,
            expected_checksum: source.checksum.clone(),
        })
        .await
        .map_err(|error| format!("prepare failed: {error:?}"))?;
    let active = if status.status == "active" {
        status
    } else {
        node.activate_replica(repair.repair_id.clone())
            .await
            .map_err(|error| format!("activate failed: {error:?}"))?
    };
    if let Err(error) = metadata
        .report_replica(
            active.block_id.clone(),
            active.length,
            active.checksum.clone(),
            repair.repair_id.clone(),
            repair.desired_copies,
            repair.repair_id.clone(),
        )
        .await
    {
        // A timed-out attempt may report after Meta has reassigned the repair.
        // Roll back its active-but-unpublished Arena copy before leaving the
        // event unacknowledged for the authoritative retry path.
        let _ = node.discard_replica_attempt(repair.repair_id.clone()).await;
        return Err(format!("report failed: {error:?}"));
    }
    dms_logging::info!(
        "replica repair applied";
        "event" => "node.repair.completed",
        "block_id" => hex(&repair.block_id),
        "source_node_id" => source.node_id,
        "source_node_epoch" => source.node_epoch,
        "target_node_id" => target.node_id,
        "target_node_epoch" => target.node_epoch,
        "desired_copies" => repair.desired_copies,
    );
    Ok(())
}

async fn apply_retirement_event(
    metadata: MetadataClient,
    node: NodeHandle,
    evict: dms_protocol::v1::EvictReplicaEvent,
    active_phases: Arc<tokio::sync::Mutex<HashSet<RetirementPhaseKey>>>,
) {
    let phase_key = (evict.retirement_id.clone(), evict.phase);
    {
        let mut active = active_phases.lock().await;
        if !active.insert(phase_key.clone()) {
            dms_logging::info!(
                "duplicate block retirement phase is already pending";
                "event" => "node.retirement.phase_duplicate",
                "phase" => evict.phase,
                "retirement_id" => hex(&evict.retirement_id),
            );
            return;
        }
    }
    tokio::spawn(async move {
        let phase = dms_protocol::v1::BlockRetirementPhase::try_from(evict.phase)
            .unwrap_or(dms_protocol::v1::BlockRetirementPhase::Unspecified);
        let block_ids = retirement_block_ids(&evict);
        let retirement_id = evict.retirement_id.clone();
        let result = match phase {
            dms_protocol::v1::BlockRetirementPhase::Prepare => node
                .prepare_block_retirement(retirement_id, block_ids)
                .await
                .map(|_| dms_protocol::v1::BlockRetirementAckKind::Prepared),
            dms_protocol::v1::BlockRetirementPhase::Final => node
                .finalize_block_retirement(retirement_id, block_ids)
                .await
                .map(|_| dms_protocol::v1::BlockRetirementAckKind::Released),
            dms_protocol::v1::BlockRetirementPhase::Unspecified => Err(
                runtime::WorkerError::InvalidArgument("retirement phase is missing"),
            ),
        };
        match result {
            Ok(ack_kind) => {
                let mut delay = Duration::from_millis(100);
                let mut acknowledged = false;
                let mut last_error = None;
                for attempt in 1..=RETIREMENT_PHASE_ACK_ATTEMPTS {
                    match metadata
                        .acknowledge_block_retirement(&evict, ack_kind, None)
                        .await
                    {
                        Ok(()) => {
                            acknowledged = true;
                            break;
                        }
                        Err(error) => {
                            last_error = Some(error);
                            if attempt < RETIREMENT_PHASE_ACK_ATTEMPTS {
                                tokio::time::sleep(delay).await;
                                delay = (delay * 2).min(Duration::from_secs(2));
                            }
                        }
                    }
                }
                if !acknowledged {
                    let error = last_error
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "unknown error".to_string());
                    dms_logging::warn!(
                        "block retirement phase ack failed after bounded retries";
                        "event" => "node.retirement.ack_failed",
                        "attempts" => RETIREMENT_PHASE_ACK_ATTEMPTS,
                        "error" => error,
                    );
                }
            }
            Err(error) => {
                dms_logging::warn!(
                    "block retirement phase did not complete";
                    "event" => "node.retirement.phase_incomplete",
                    "phase" => format!("{phase:?}"),
                    "error" => format!("{error:?}"),
                );
            }
        }
        active_phases.lock().await.remove(&phase_key);
    });
}

async fn send_meta_heartbeats(
    metadata: MetadataClient,
    node: NodeHandle,
    acked_cursor: Arc<AtomicU64>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        let event_cursor = acked_cursor.load(Ordering::Relaxed);
        let started = std::time::Instant::now();
        let resources = match node.resource_summary().await {
            Ok(resources) => resources,
            Err(_) => return,
        };
        match metadata.heartbeat(event_cursor, resources).await {
            Ok(ttl) => {
                if node
                    .metadata_lease(Some(started + Duration::from_millis(ttl)), None)
                    .await
                    .is_err()
                {
                    return;
                }
                let references = match node.filesystem_inode_reference_snapshot().await {
                    Ok(references) => references,
                    Err(_) => return,
                };
                if !references.is_empty() {
                    let wire_references = references
                        .iter()
                        .map(|(inode, reference_generation)| {
                            dms_protocol::v1::FilesystemInodeReferenceLease {
                                inode: *inode,
                                reference_generation: *reference_generation,
                            }
                        })
                        .collect();
                    match metadata
                        .filesystem_renew_inode_references(wire_references)
                        .await
                    {
                        Ok(response) => {
                            if node
                                .filesystem_renew_inode_reference_leases(
                                    references,
                                    response.lease_millis,
                                )
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        Err(error) => {
                            dms_logging::warn!(
                                "filesystem inode reference renewal failed";
                                "event" => "node.filesystem.reference.renew_failed",
                                "reference_count" => references.len(),
                                "error" => error.to_string(),
                            );
                        }
                    }
                }
            }
            Err(error) => {
                // Successful periodic heartbeats stay in Metrics only. A failure
                // creates one diagnostic span after the request fails, carrying
                // elapsed time without exporting a span for every healthy tick.
                let span = dms_tracing::tracing::warn_span!(
                    "dms.meta.heartbeat.failure",
                    otel.kind = "client",
                    event_cursor,
                    elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
                    result = "error",
                    error.code = dms_tracing::tracing::field::Empty,
                    error.kind = dms_tracing::tracing::field::Empty,
                );
                dms_tracing::record_error(&span, &error);
                let _scope = span.enter();
                dms_logging::warn!(
                    "Meta heartbeat failed; event watch will keep retrying";
                    "event" => "node.meta_heartbeat.failed",
                    "event_cursor" => event_cursor,
                    "error" => format!("{error:?}"),
                );
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn stable_node_id(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash.max(1)
}

async fn serve_bound_workers(
    tcp_listener: Option<TokioTcpListener>,
    uds_listener: Option<UnixListener>,
    node: NodeHandle,
    rpc_metrics: dms_metrics::RpcMetrics,
    error_metrics: dms_metrics::ErrorMetrics,
    trace_periodic_operations: bool,
    #[cfg(feature = "reliability-faults")] peer_pull_fault_gate: Option<
        peer_service::PeerPullFaultGate,
    >,
) -> Result<(), Box<dyn std::error::Error>> {
    match (tcp_listener, uds_listener) {
        (Some(tcp), Some(uds)) => {
            tokio::try_join!(
                serve_worker_tcp(
                    tcp,
                    node.clone(),
                    rpc_metrics.clone(),
                    error_metrics.clone(),
                    trace_periodic_operations,
                    #[cfg(feature = "reliability-faults")]
                    peer_pull_fault_gate.clone(),
                ),
                serve_worker_uds(
                    uds,
                    node,
                    rpc_metrics,
                    error_metrics,
                    trace_periodic_operations,
                ),
            )?;
        }
        (Some(tcp), None) => {
            serve_worker_tcp(
                tcp,
                node,
                rpc_metrics,
                error_metrics,
                trace_periodic_operations,
                #[cfg(feature = "reliability-faults")]
                peer_pull_fault_gate,
            )
            .await?
        }
        (None, Some(uds)) => {
            serve_worker_uds(
                uds,
                node,
                rpc_metrics,
                error_metrics,
                trace_periodic_operations,
            )
            .await?
        }
        (None, None) => unreachable!("business endpoint was validated before startup"),
    }
    Ok(())
}

async fn serve_worker_tcp(
    listener: TokioTcpListener,
    node: NodeHandle,
    rpc_metrics: dms_metrics::RpcMetrics,
    error_metrics: dms_metrics::ErrorMetrics,
    trace_periodic_operations: bool,
    #[cfg(feature = "reliability-faults")] peer_pull_fault_gate: Option<
        peer_service::PeerPullFaultGate,
    >,
) -> Result<(), Box<dyn std::error::Error>> {
    // Handler 只组合 Handle；不创建另一份 Worker/Peer 状态。
    let worker_handler =
        WorkerServiceHandler::with_metrics(node.clone(), false, rpc_metrics.clone(), error_metrics);
    #[cfg(feature = "reliability-faults")]
    let peer_handler = {
        let mut peer_handler = PeerServiceHandler::with_metrics(node, rpc_metrics);
        if let Some(gate) = peer_pull_fault_gate {
            peer_handler = peer_handler.with_peer_pull_fault_gate(gate);
        }
        peer_handler
    };
    #[cfg(not(feature = "reliability-faults"))]
    let peer_handler = PeerServiceHandler::with_metrics(node, rpc_metrics);
    let security = SecurityManager::new(TlsConfig::Disabled)?;
    let grpc_config = GrpcConfig::default();
    let server = grpc_config.configure_server(tonic::transport::Server::builder());
    // 一个 TCP/HTTP2 Server 注册三份 generated Service：控制、payload、peer。
    security
        .configure_server(server)?
        .layer(dms_tracing::GrpcServerTraceLayer::new(
            trace_periodic_operations,
        ))
        .add_service(configure_worker_service(
            WorkerServiceServer::new(worker_handler.clone()),
            &grpc_config,
        ))
        .add_service(configure_worker_payload_service(
            WorkerPayloadServiceServer::new(worker_handler),
            &grpc_config,
        ))
        .add_service(configure_peer_service(
            PeerServiceServer::new(peer_handler),
            &grpc_config,
        ))
        .serve_with_incoming(grpc_config.configure_tcp_incoming(listener.into()))
        .await?;
    Ok(())
}

async fn serve_worker_uds(
    listener: UnixListener,
    node: NodeHandle,
    rpc_metrics: dms_metrics::RpcMetrics,
    error_metrics: dms_metrics::ErrorMetrics,
    trace_periodic_operations: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let handler = WorkerServiceHandler::with_metrics(node, true, rpc_metrics, error_metrics);
    let security = SecurityManager::new(TlsConfig::Disabled)?;
    let grpc_config = GrpcConfig::default();
    let server = grpc_config.configure_server(tonic::transport::Server::builder());
    // UDS 只面向本机 Client，因此不注册 PeerService；业务 Worker API 与 TCP 相同。
    security
        .configure_server(server)?
        .layer(dms_tracing::GrpcServerTraceLayer::new(
            trace_periodic_operations,
        ))
        .add_service(configure_worker_service(
            WorkerServiceServer::new(handler.clone()),
            &grpc_config,
        ))
        .add_service(configure_worker_payload_service(
            WorkerPayloadServiceServer::new(handler),
            &grpc_config,
        ))
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;
    Ok(())
}

fn configure_worker_service(
    service: WorkerServiceServer<WorkerServiceHandler>,
    config: &GrpcConfig,
) -> WorkerServiceServer<WorkerServiceHandler> {
    service
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes)
}

fn configure_worker_payload_service(
    service: WorkerPayloadServiceServer<WorkerServiceHandler>,
    config: &GrpcConfig,
) -> WorkerPayloadServiceServer<WorkerServiceHandler> {
    service
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes)
}

fn configure_peer_service(
    service: PeerServiceServer<PeerServiceHandler>,
    config: &GrpcConfig,
) -> PeerServiceServer<PeerServiceHandler> {
    service
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes)
}

fn bind_worker_uds(path: &PathBuf) -> io::Result<UnixListener> {
    // UnixListener::bind 要求路径不存在；先清理上次异常退出遗留的 socket 文件。
    match std::fs::remove_file(path) {
        Ok(()) => {}
        // guard 只忽略“本来就不存在”，权限等其他错误仍向上传播。
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    UnixListener::bind(path)
}

#[cfg(test)]
mod invalidation_source_tests {
    use super::should_apply_filesystem_invalidation;

    #[test]
    fn local_filesystem_invalidation_does_not_revoke_directly_updated_cache() {
        assert!(!should_apply_filesystem_invalidation(17, 17));
    }

    #[test]
    fn remote_and_legacy_filesystem_invalidations_are_applied() {
        assert!(should_apply_filesystem_invalidation(17, 23));
        assert!(should_apply_filesystem_invalidation(17, 0));
    }
}

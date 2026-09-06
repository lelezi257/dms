//! dms-meta 进程组合根与 R2 N0 模块入口。
//!
//! `dms-meta` owns Node sessions, physical Replica reachability, authoritative
//! Object/Version ordering, conditional placement and Node events. External
//! stores supply declared fencing/log capabilities only.

#![forbid(unsafe_code)]
mod in_memory_journal;
mod local_wal_journal;
mod metadata_journal;
pub(crate) mod metadata_service;
mod metrics;
pub(crate) mod runtime;

use dms_protocol::v1::metadata_service_server::MetadataServiceServer;
use dms_transport::{GrpcConfig, SecurityManager, TlsConfig};
use tokio::net::TcpListener as TokioTcpListener;

use crate::health::{Readiness, ReadinessState, serve_status};
use crate::{ComponentKind, NodeId};
use local_wal_journal::LocalWalJournal;
use metadata_service::MetadataServiceHandler;
use metrics::MetaMetrics;
use runtime::MetaHandle;

/// Minimal configuration required to start the metadata process shell.
#[derive(Clone, Debug)]
pub struct MetaConfig {
    /// Identity reported by logs, metrics, and health responses.
    pub node_id: NodeId,
    /// TCP address of the operational health listener.
    pub health_address: String,
    /// Optional Node-facing gRPC listener, for example `127.0.0.1:19300`.
    pub grpc_address: String,
    /// Optional durable local WAL/checkpoint directory.
    pub journal_dir: Option<std::path::PathBuf>,
    /// 两次全量元数据快照之间累计的 journal 记录数，不是 WAL 刷盘间隔。
    pub checkpoint_every_records: u64,
    /// Process-owned trace runtime; disabled unless explicitly configured.
    pub tracing: dms_tracing::TracingConfig,
}

/// Starts the metadata process shell and blocks until it is stopped.
pub fn serve(config: MetaConfig) -> Result<(), Box<dyn std::error::Error>> {
    // 同步进程入口显式创建 runtime，便于 main 保持普通 fn。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(serve_process(config))?;
    Ok(())
}

async fn serve_process(config: MetaConfig) -> Result<(), Box<dyn std::error::Error>> {
    let readiness = ReadinessState::default();
    let registry = dms_metrics::registry();
    let meta_metrics = MetaMetrics::register(&registry)?;
    let rpc_metrics = dms_metrics::RpcMetrics::register(&registry)?;
    let error_metrics = dms_metrics::ErrorMetrics::register(&registry)?;
    let trace_metrics = dms_metrics::TraceRuntimeMetrics::register(&registry)?;
    let _tracing_guard = dms_tracing::init_process_tracing(
        &config.tracing,
        dms_tracing::ProcessIdentity::new("dms-meta", config.node_id.to_string()),
        Some(trace_metrics),
    )?;
    let trace_periodic_operations = config.tracing.periodic_operations;
    let health_listener = TokioTcpListener::bind(&config.health_address).await?;
    let health_address = health_listener.local_addr()?;
    let grpc_listener = TokioTcpListener::bind(&config.grpc_address).await?;
    let grpc_address = grpc_listener.local_addr()?;

    // The owner and handler are created only after both listeners are bound.
    // READY therefore means the process can actually accept metadata traffic.
    let journal = match config.journal_dir.as_ref() {
        Some(directory) => Box::new(LocalWalJournal::open(directory)?)
            as Box<dyn metadata_journal::MetadataJournal>,
        None => Box::<in_memory_journal::InMemoryJournal>::default()
            as Box<dyn metadata_journal::MetadataJournal>,
    };
    let handler = MetadataServiceHandler::with_metrics(
        MetaHandle::try_spawn_with_runtime_policy(
            journal,
            runtime::MetaCheckpointPolicy {
                every_records: config.checkpoint_every_records,
            },
            meta_metrics,
            trace_periodic_operations,
        )?,
        rpc_metrics,
        error_metrics,
    );
    readiness.set(Readiness::Ready);
    dms_logging::info!(
        "dms-meta is ready";
        "event" => "meta.process.ready",
        "node_id" => config.node_id.to_string(),
        "health_address" => health_address.to_string(),
        "grpc_address" => grpc_address.to_string(),
        "protocol_version" => 1,
    );

    let status = serve_status(
        health_listener,
        ComponentKind::Meta,
        config.node_id,
        readiness.clone(),
        registry,
    );
    let grpc = serve_grpc(grpc_listener, handler, trace_periodic_operations);
    tokio::pin!(status);
    tokio::pin!(grpc);
    let result = tokio::select! {
        result = &mut status => result.map_err(|error| error.into()),
        result = &mut grpc => result,
    };
    readiness.set(Readiness::Stopping);
    result
}

async fn serve_grpc(
    listener: TokioTcpListener,
    handler: MetadataServiceHandler,
    trace_periodic_operations: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let security = SecurityManager::new(TlsConfig::Disabled)?;
    let server = GrpcConfig::default().configure_server(tonic::transport::Server::builder());
    // R1 只注册 MetadataService；generated router 根据 RPC path 调用对应方法。
    security
        .configure_server(server)?
        .layer(dms_tracing::GrpcServerTraceLayer::new(
            trace_periodic_operations,
        ))
        .add_service(MetadataServiceServer::new(handler))
        .serve_with_incoming(GrpcConfig::default().configure_tcp_incoming(listener.into()))
        .await?;
    Ok(())
}

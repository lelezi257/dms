//! afs-node：近计算部署的单一节点进程。
//!
//! FUSE、SDK/REST、节点间 RPC 接入同一个 Node，内容经 P2P 直达数据节点。
//! 不另建 Home 进程，不保留 NFS 后端。OwnerFs/DFS 共用 Backend 与 FUSE 实现，
//! 但使用独立 mount、session、数据表示、缓存、一致性与恢复语义。
//! 阻塞 I/O/设备等待不可占住异步执行线程；调度和局部保护归各业务模块。

//!
//! 阅读启动顺序：run → Backend/Storage/会话表 → 本机 UDS → 可选 FUSE → TCP gRPC/REST。
//! gRPC 的控制与数据 service 共用 TCP listener；SDK 使用另一条本机 UDS listener。
//! OwnerFs 与 DFS 都通过 FUSE→Backend 进入各自的真实文件路径；
//! REST diagnostics/SDK→Storage 仍是独立诊断链，不属于任何文件系统的数据面。

pub mod api;
pub mod chunk;
pub mod fuse;
pub mod rpc;
pub mod storage;
pub mod vfs;

use crate::{
    config::Config,
    runtime::{BoxError, Observability, Services, cancelled},
};
use std::sync::Arc;

/// B-side slow-path connector. The root grant and Home location stay in the
/// OwnerFs business layer; this adapter only turns Meta's authenticated node
/// endpoint into a cached OwnerFiles transport client.
#[cfg(feature = "ownerfs")]
struct GrpcOwnerFilesFactory {
    meta: Arc<rpc::meta::GrpcRootMeta>,
    tls: afs_transport::grpc::TlsConfig,
    timeout: std::time::Duration,
    runtime: tokio::runtime::Handle,
    metrics: rpc::OwnerRpcMetrics,
}

#[cfg(feature = "ownerfs")]
impl vfs::ownerfs::RemoteFilesFactory for GrpcOwnerFilesFactory {
    fn connect(
        &self,
        home_node_id: &str,
    ) -> afs_error::Result<Arc<dyn vfs::ownerfs::remote::RemoteFiles>> {
        let uri = self.meta.lookup_node_endpoint(home_node_id)?;
        let endpoint = tonic::transport::Endpoint::from_shared(uri).map_err(|error| {
            afs_error::Error::coded(afs_error::CLIENT_ARGUMENT_INVALID, error.to_string())
        })?;
        let endpoint = endpoint.connect_timeout(self.timeout).timeout(self.timeout);
        let endpoint = afs_transport::grpc::SecurityManager::new(self.tls.clone())
            .and_then(|security| security.configure_client(endpoint))
            .map_err(|error| {
                afs_error::Error::coded(afs_error::CLIENT_ARGUMENT_INVALID, error.to_string())
            })?;
        let channel = self.runtime.block_on(endpoint.connect()).map_err(|error| {
            afs_error::Error::coded(afs_error::CLIENT_CONNECTION_UNAVAILABLE, error.to_string())
        })?;
        Ok(Arc::new(
            rpc::peer::owner_files_client_from_channel_with_runtime_and_metrics(
                channel,
                self.runtime.clone(),
                Some(self.metrics.clone()),
            ),
        ))
    }
}

/// REST 持有的进程级共享对象，不是另一个 Home 服务进程。
/// OwnerFs 和 DFS 分别绑定自己的 FUSE session；诊断 Storage 与 RDMA 会话表独立。
pub struct Node {
    pub config: Config,
    pub observability: Observability,
    /// 每次进程启动生成的新会话，旧远端句柄不能跨此边界复用。
    pub session_id: String,
    #[cfg(feature = "ownerfs")]
    pub ownerfs: Option<Arc<vfs::ownerfs::OwnerFs>>,
    #[cfg(feature = "dfs")]
    pub dfs: Option<Arc<vfs::dfs::DistributedFs>>,
}

/// 组装并持有 Node 的所有入口。启动失败清理已经建立的资源，正常退出卸载本进程挂载。
pub async fn run(cfg: Config, obs: Observability) -> Result<(), BoxError> {
    #[cfg(feature = "ownerfs")]
    let owner_rpc_metrics = rpc::OwnerRpcMetrics::register(&obs.registry)?;
    // Bind all TCP ingress before spawning services. A failed bind cannot leave a half-ready Node.
    let grpc = tokio::net::TcpListener::bind(cfg.grpc_listen).await?;
    let rest = tokio::net::TcpListener::bind(cfg.rest_listen).await?;
    // OwnerFs 生产路径必须先建立 Meta 会话和本机普通文件后端，不能挂载
    // 这里的 session ID 来自 Linux 内核随机源，也用于隔离 DFS 操作身份。
    let session_id = std::fs::read_to_string("/proc/sys/kernel/random/uuid")?
        .trim()
        .to_owned();
    let timeout = std::time::Duration::from_millis(cfg.timeout_ms);
    let needs_meta = cfg.ownerfs || cfg.dfs;
    let meta_endpoint = if needs_meta {
        Some(cfg.meta_endpoint.as_deref().ok_or_else(|| {
            afs_error::Error::coded(
                afs_error::CONFIG_INVALID,
                "OwnerFs and DFS require meta_endpoint",
            )
        })?)
    } else {
        None
    };
    let advertised = if needs_meta {
        Some(match cfg.advertise_endpoint.clone() {
            Some(value) => value,
            None if !cfg.grpc_listen.ip().is_unspecified() => format!("http://{}", cfg.grpc_listen),
            None => {
                return Err(afs_error::Error::coded(
                    afs_error::CONFIG_INVALID,
                    "Node listening on an unspecified address requires advertise_endpoint",
                )
                .into());
            }
        })
    } else {
        None
    };
    let node_descriptor = meta_endpoint.map(|endpoint| {
        let advertised = advertised
            .clone()
            .expect("needs_meta sets advertised endpoint");
        let mut capabilities = Vec::new();
        if cfg.ownerfs {
            capabilities.push("ownerfs".into());
        }
        if cfg.dfs {
            capabilities.push("dfs".into());
        }
        (
            endpoint,
            afs_protocol::meta::NodeDescriptor {
                node_id: cfg.id.clone(),
                endpoint: Some(afs_protocol::meta::NodeEndpoint {
                    grpc_addr: advertised.clone(),
                    data_addr: advertised,
                    rest_addr: format!("http://{}", cfg.rest_listen),
                }),
                labels: std::collections::HashMap::new(),
                capabilities,
                session_id: session_id.clone(),
            },
        )
    });
    if let Some((endpoint, descriptor)) = &node_descriptor {
        rpc::meta::register_node(endpoint, descriptor.clone(), timeout, cfg.tls_config()).await?;
    }

    #[cfg(feature = "ownerfs")]
    let ownerfs_instance = if cfg.ownerfs {
        let endpoint = meta_endpoint.expect("OwnerFs checked meta_endpoint");
        let root_meta = Arc::new(rpc::meta::GrpcRootMeta::new(
            endpoint,
            cfg.id.clone(),
            session_id.clone(),
            timeout,
            cfg.tls_config(),
        )?);
        let disk = Arc::new(storage::LocalFs::open(cfg.data_dir.join("ownerfs"))?);
        let recovery_disk = disk.clone();
        let recovery_node_id = cfg.id.clone();
        let recovery_session_id = session_id.clone();
        let remote_factory = Arc::new(GrpcOwnerFilesFactory {
            meta: root_meta.clone(),
            tls: cfg.tls_config(),
            timeout,
            runtime: tokio::runtime::Handle::current(),
            metrics: owner_rpc_metrics.clone(),
        });
        let roots = Arc::new(
            tokio::task::spawn_blocking(move || {
                vfs::ownerfs::root::RootManager::open(
                    recovery_node_id,
                    recovery_session_id,
                    root_meta,
                    recovery_disk,
                )
            })
            .await??,
        );
        Some(Arc::new(vfs::ownerfs::OwnerFs::new_local_with_remote(
            roots,
            disk,
            remote_factory,
        )))
    } else {
        None
    };

    #[cfg(feature = "dfs")]
    let dfs_instance = if cfg.dfs {
        let endpoint = meta_endpoint.expect("DFS checked meta_endpoint");
        let namespace = crate::dfs::NamespaceId::new("default");
        let meta = Arc::new(rpc::meta::GrpcDfsMeta::new(
            endpoint,
            cfg.id.clone(),
            session_id.clone(),
            namespace.clone(),
            timeout,
            cfg.tls_config(),
        )?);
        let chunks = Arc::new(chunk::LocalChunkStore::open(
            cfg.data_dir.join("dfs"),
            cfg.id.clone(),
        )?);
        Some(Arc::new(vfs::dfs::DistributedFs::new(
            namespace,
            cfg.id.clone(),
            session_id.clone(),
            meta,
            chunks,
        )))
    } else {
        None
    };

    // diagnostics is a separate test object directory, not an OwnerFs or DFS data path.
    let storage = Arc::new(storage::Storage::new(cfg.data_dir.join("diagnostics"))?);
    let sessions = rpc::control::RdmaSessionRegistry::new(cfg.rdma_device.clone());
    let local = api::local::serve_local_api_with_options(
        storage.clone(),
        &cfg.uds_path,
        api::local::LocalApiOptions {
            metrics_registry: Some(obs.registry.clone()),
        },
    )
    .await?;

    #[cfg(feature = "ownerfs")]
    let mounted_ownerfs = match (&cfg.ownerfs_mount, &ownerfs_instance) {
        (Some(path), Some(ownerfs)) => match fuse::mount_ownerfs(ownerfs.clone(), path) {
            Ok(session) => Some(session),
            Err(error) => {
                local.shutdown().await?;
                return Err(error.into());
            }
        },
        (Some(_), None) => {
            local.shutdown().await?;
            return Err(afs_error::Error::coded(
                afs_error::CONFIG_INVALID,
                "ownerfs_mount requires the OwnerFs backend",
            )
            .into());
        }
        (None, _) => None,
    };

    #[cfg(feature = "dfs")]
    let mounted_dfs = match (&cfg.dfs_mount, &dfs_instance) {
        (Some(path), Some(dfs)) => match fuse::mount_dfs(dfs.clone(), path) {
            Ok(session) => Some(session),
            Err(error) => {
                #[cfg(feature = "ownerfs")]
                drop(mounted_ownerfs);
                local.shutdown().await?;
                return Err(error.into());
            }
        },
        (Some(_), None) => {
            #[cfg(feature = "ownerfs")]
            drop(mounted_ownerfs);
            local.shutdown().await?;
            return Err(afs_error::Error::coded(
                afs_error::CONFIG_INVALID,
                "dfs_mount requires the DFS backend",
            )
            .into());
        }
        (None, _) => None,
    };

    let state = Arc::new(Node {
        config: cfg.clone(),
        observability: obs,
        session_id,
        #[cfg(feature = "ownerfs")]
        ownerfs: ownerfs_instance,
        #[cfg(feature = "dfs")]
        dfs: dfs_instance,
    });
    let mut services = Services::new();
    if let Some((endpoint, descriptor)) = node_descriptor {
        let endpoint = endpoint.to_owned();
        let timeout = std::time::Duration::from_millis(cfg.timeout_ms);
        let tls = cfg.tls_config();
        let stop = services.stop.subscribe();
        services.spawn(async move {
            // The Meta lease lasts longer than one refresh interval. A brief
            // Meta restart must not tear down the FUSE mount and all open FDs
            // just because one heartbeat raced the restart. Retry within the
            // lease window; fail closed if control cannot be restored in time.
            let mut last_success = tokio::time::Instant::now();
            let mut next_delay = std::time::Duration::from_secs(10);
            let shutdown = cancelled(stop);
            tokio::pin!(shutdown);
            loop {
                tokio::select! {
                    _ = &mut shutdown => return Ok(()),
                    _ = tokio::time::sleep(next_delay) => {
                        match rpc::meta::register_node(&endpoint, descriptor.clone(), timeout, tls.clone()).await {
                            Ok(()) => {
                                last_success = tokio::time::Instant::now();
                                next_delay = std::time::Duration::from_secs(10);
                            }
                            Err(error) => {
                                if last_success.elapsed() >= std::time::Duration::from_secs(25) {
                                    return Err(error.into());
                                }
                                afs_logging::warn!("node.meta_heartbeat_retry"; "error" => error.to_string());
                                next_delay = std::time::Duration::from_secs(1);
                            }
                        }
                    }
                }
            }
        });
    }
    #[cfg(feature = "ownerfs")]
    if let Some(ownerfs) = state.ownerfs.clone() {
        let stop = services.stop.subscribe();
        services.spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
            tick.tick().await;
            let shutdown = cancelled(stop);
            tokio::pin!(shutdown);
            loop {
                tokio::select! {
                    _ = &mut shutdown => return Ok(()),
                    _ = tick.tick() => {
                        let fs = ownerfs.clone();
                        match tokio::task::spawn_blocking(move || fs.reap_expired_peer_sessions()).await {
                            Ok(Ok(count)) if count > 0 => {
                                afs_logging::info!("ownerfs.peer_handles_reaped"; "count" => count);
                            }
                            Ok(Ok(_)) => {}
                            Ok(Err(error)) => {
                                // Unknown Meta state is never evidence that a peer died.
                                afs_logging::warn!("ownerfs.peer_reaper_retry"; "error" => error.to_string());
                            }
                            Err(error) => {
                                afs_logging::warn!("ownerfs.peer_reaper_worker_failed"; "error" => error.to_string());
                            }
                        }
                    }
                }
            }
        });
    }
    #[cfg(feature = "dfs")]
    if let Some(dfs) = state.dfs.clone() {
        let stop = services.stop.subscribe();
        services.spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
            tick.tick().await;
            let shutdown = cancelled(stop);
            tokio::pin!(shutdown);
            loop {
                tokio::select! {
                    _ = &mut shutdown => return Ok(()),
                    _ = tick.tick() => {
                        let fs = dfs.clone();
                        match tokio::task::spawn_blocking(move || fs.writeback_pending()).await {
                            Ok(Ok(count)) if count > 0 => {
                                afs_logging::info!("dfs.background_versions_committed"; "count" => count);
                            }
                            Ok(Ok(_)) => {}
                            Ok(Err(error)) => {
                                afs_logging::warn!("dfs.background_writeback_retry"; "error" => error.to_string());
                            }
                            Err(error) => {
                                afs_logging::warn!("dfs.background_writeback_worker_failed"; "error" => error.to_string());
                            }
                        }
                    }
                }
            }
        });
    }
    // local API 自己持有 JoinHandle；这里监控它，避免 UDS 已死而 TCP 健康检查仍成功。
    let local_task = local
        .abort_handle()
        .expect("local API task exists after startup");
    let stop = services.stop.subscribe();
    services.spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        tokio::pin! {let shutdown=cancelled(stop);}
        loop {
            tokio::select! {
                _ = &mut shutdown => return Ok(()),
                _ = tick.tick() => if local_task.is_finished() {
                    return Err(std::io::Error::other("local SDK service exited unexpectedly").into());
                }
            }
        }
    });
    let stop = services.stop.subscribe();
    let grpc_config = afs_transport::grpc::GrpcConfig::default();
    let grpc_security = afs_transport::grpc::SecurityManager::new(cfg.tls_config())?;
    let grpc_server = grpc_security
        .configure_server(grpc_config.configure_server(tonic::transport::Server::builder()))?;
    let incoming =
        grpc_config.configure_tcp_incoming(tonic::transport::server::TcpIncoming::from(grpc));
    // 业务 Handler 在 Node，公共 transport 只提供 builder 配置和低层搬运机制。
    let control = rpc::control::make_control_server(sessions.clone());
    let data = rpc::data::make_data_server(storage, sessions.clone());
    #[cfg(feature = "ownerfs")]
    let owner_files = if let Some(ownerfs) = state.ownerfs.as_ref() {
        let trusted = cfg
            .trusted_node_certs
            .iter()
            .map(|(node_id, path)| Ok((node_id.clone(), std::fs::read(path)?)))
            .collect::<std::io::Result<Vec<_>>>()?;
        let authenticator = Arc::new(rpc::data::MtlsPeerAuthenticator::new(trusted)?);
        let handler = rpc::data::make_owner_files_handler(ownerfs.peer_executor()?);
        rpc::data::make_owner_files_server_with_handler_and_metrics(
            handler,
            authenticator,
            owner_rpc_metrics.clone(),
        )
    } else {
        rpc::data::make_owner_files_server()
    };
    services.spawn(async move {
        let router = grpc_server
            .layer(afs_tracing::GrpcServerTraceLayer::default())
            .add_service(control)
            .add_service(data);
        #[cfg(feature = "ownerfs")]
        let router = router.add_service(owner_files);
        router
            .serve_with_incoming_shutdown(incoming, cancelled(stop))
            .await
            .map_err(Into::into)
    });
    #[cfg(feature = "dfs")]
    let dfs_for_drain = state.dfs.clone();
    let stop = services.stop.subscribe();
    services.spawn(async move {
        axum::serve(rest, api::rest::router(state))
            .with_graceful_shutdown(cancelled(stop))
            .await
            .map_err(Into::into)
    });
    let stop = services.stop.subscribe();
    services.spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        tokio::pin! {let shutdown=cancelled(stop);}
        loop {
            tokio::select! {_=tick.tick()=>sessions.cleanup_expired().await,_=&mut shutdown=>break}
        }
        Ok(())
    });
    afs_logging::info!("node.ready";"grpc"=>cfg.grpc_listen.to_string(),"rest"=>cfg.rest_listen.to_string(),"uds"=>cfg.uds_path.display().to_string(),"ownerfs"=>cfg.ownerfs,"dfs"=>cfg.dfs);
    let result = services.run().await;
    #[cfg(feature = "dfs")]
    if let Some(dfs) = dfs_for_drain {
        match tokio::task::spawn_blocking(move || dfs.drain()).await {
            Ok(Ok(count)) if count > 0 => {
                afs_logging::info!("dfs.node_drain_versions_committed"; "count" => count);
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                afs_logging::warn!("dfs.node_drain_incomplete"; "error" => error.to_string());
            }
            Err(error) => {
                afs_logging::warn!("dfs.node_drain_worker_failed"; "error" => error.to_string());
            }
        }
    }
    // BackgroundSession owns the FUSE mount. Unmount before dropping request services.
    #[cfg(feature = "dfs")]
    drop(mounted_dfs);
    #[cfg(feature = "ownerfs")]
    drop(mounted_ownerfs);
    let local_result =
        tokio::time::timeout(std::time::Duration::from_secs(10), local.shutdown()).await?;
    result?;
    Ok(local_result?)
}

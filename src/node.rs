//! afs-node：近计算部署的单一节点进程。
//!
//! FUSE、SDK/REST、节点间 RPC 接入同一个 Node，内容经 P2P 直达数据节点。
//! 不另建 Home 进程，不保留 NFS 后端。OwnerFs/BlobFs 共用 VFS 入口但 namespace、
//! 数据表示、缓存、一致性、恢复与发布语义分开；普通本地写不强制生成 Blob。
//! 阻塞 I/O/设备等待不可占住异步执行线程；调度和局部保护归各业务模块。

//!
//! 阅读启动顺序：run → Vfs/Storage/会话表 → 本机 UDS → 可选 FUSE → TCP gRPC/REST。
//! gRPC 的控制与数据 service 共用 TCP listener；SDK 使用另一条本机 UDS listener。
//! 当前两条验证链分开：FUSE→VFS→后端打印并返回 ENOSYS；
//! REST diagnostics/SDK→真实传输→Storage 读写诊断文件。后者尚未接上根授权业务。

pub mod api;
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
/// Vfs 用于入口分派；诊断 Storage 与 RDMA 会话表单独交给各 service。
pub struct Node {
    pub config: Config,
    pub observability: Observability,
    pub vfs: Arc<vfs::Vfs>,
    /// 每次进程启动生成的新会话，旧远端句柄不能跨此边界复用。
    pub session_id: String,
    #[cfg(feature = "ownerfs")]
    pub ownerfs: Option<Arc<vfs::ownerfs::OwnerFs>>,
}

/// 组装并持有 Node 的所有入口。启动失败清理已经建立的资源，正常退出卸载本进程挂载。
pub async fn run(cfg: Config, obs: Observability) -> Result<(), BoxError> {
    #[cfg(feature = "ownerfs")]
    let owner_rpc_metrics = rpc::OwnerRpcMetrics::register(&obs.registry)?;
    // Bind all TCP ingress before spawning services. A failed bind cannot leave a half-ready Node.
    let grpc = tokio::net::TcpListener::bind(cfg.grpc_listen).await?;
    let rest = tokio::net::TcpListener::bind(cfg.rest_listen).await?;
    // OwnerFs 生产路径必须先建立 Meta 会话和本机普通文件后端，不能挂载
    // Vfs::new 创建的无依赖诊断骨架。这里的 session ID 来自 Linux 内核随机源。
    let session_id = std::fs::read_to_string("/proc/sys/kernel/random/uuid")?
        .trim()
        .to_owned();
    #[cfg(feature = "ownerfs")]
    let (vfs, ownerfs_instance, node_descriptor) = if cfg.ownerfs {
        let endpoint = cfg.meta_endpoint.as_deref().ok_or_else(|| {
            afs_error::Error::coded(afs_error::CONFIG_INVALID, "OwnerFs requires meta_endpoint")
        })?;
        let advertised = match cfg.advertise_endpoint.clone() {
            Some(value) => value,
            None if !cfg.grpc_listen.ip().is_unspecified() => format!("http://{}", cfg.grpc_listen),
            None => {
                return Err(afs_error::Error::coded(
                    afs_error::CONFIG_INVALID,
                    "Node listening on an unspecified address requires advertise_endpoint",
                )
                .into());
            }
        };
        let timeout = std::time::Duration::from_millis(cfg.timeout_ms);
        let descriptor = afs_protocol::meta::NodeDescriptor {
            node_id: cfg.id.clone(),
            endpoint: Some(afs_protocol::meta::NodeEndpoint {
                grpc_addr: advertised.clone(),
                data_addr: advertised,
                rest_addr: format!("http://{}", cfg.rest_listen),
            }),
            labels: std::collections::HashMap::new(),
            capabilities: vec!["ownerfs".into()],
            session_id: session_id.clone(),
        };
        rpc::meta::register_node(endpoint, descriptor.clone(), timeout, cfg.tls_config()).await?;
        let root_meta = Arc::new(rpc::meta::GrpcRootMeta::new(
            endpoint,
            cfg.id.clone(),
            session_id.clone(),
            timeout,
            cfg.tls_config(),
        )?);
        let disk = Arc::new(storage::LocalFs::open(cfg.data_dir.join("ownerfs"))?);
        // Recovery calls Meta synchronously through RootMeta. Run it on a
        // blocking thread before mounting FUSE, so no request can observe a
        // partly recovered root namespace or block a Tokio worker.
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
        let ownerfs = Arc::new(vfs::ownerfs::OwnerFs::new_local_with_remote(
            roots,
            disk,
            remote_factory,
        ));
        let vfs = Arc::new(vfs::Vfs::with_ownerfs(
            ownerfs.clone(),
            cfg.blobfs,
            obs.registry.clone(),
        )?);
        (vfs, Some(ownerfs), Some(descriptor))
    } else {
        (
            Arc::new(vfs::Vfs::new(false, cfg.blobfs, obs.registry.clone())?),
            None,
            None,
        )
    };
    #[cfg(not(feature = "ownerfs"))]
    let vfs = Arc::new(vfs::Vfs::new(
        cfg.ownerfs,
        cfg.blobfs,
        obs.registry.clone(),
    )?);
    // diagnostics 是独立测试对象目录；不能据此认为 OwnerFs/BlobFs 已可存业务数据。
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
    let mount_result = match &cfg.mount {
        #[cfg(feature = "ownerfs")]
        Some(path) if ownerfs_instance.is_some() => fuse::mount_with_ownerfs_cache(
            vfs.clone(),
            ownerfs_instance.as_ref().expect("guarded above").clone(),
            path,
        )
        .map(Some),
        Some(path) => fuse::mount(vfs.clone(), path).map(Some),
        None => Ok(None),
    };
    let mounted = match mount_result {
        Ok(session) => session,
        Err(error) => {
            local.shutdown().await?;
            return Err(error.into());
        }
    };
    let state = Arc::new(Node {
        config: cfg.clone(),
        observability: obs,
        vfs,
        session_id,
        #[cfg(feature = "ownerfs")]
        ownerfs: ownerfs_instance,
    });
    let mut services = Services::new();
    #[cfg(feature = "ownerfs")]
    if let Some(descriptor) = node_descriptor {
        let endpoint = cfg
            .meta_endpoint
            .clone()
            .expect("OwnerFs checked meta_endpoint");
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
        let authenticator = Arc::new(rpc::data::owner::MtlsPeerAuthenticator::new(trusted)?);
        let handler = rpc::data::owner::make_owner_files_handler(ownerfs.peer_executor()?);
        rpc::data::owner::make_owner_files_server_with_handler_and_metrics(
            handler,
            authenticator,
            owner_rpc_metrics.clone(),
        )
    } else {
        rpc::data::owner::make_owner_files_server()
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
    afs_logging::info!("node.ready";"grpc"=>cfg.grpc_listen.to_string(),"rest"=>cfg.rest_listen.to_string(),"uds"=>cfg.uds_path.display().to_string(),"ownerfs"=>cfg.ownerfs,"blobfs"=>cfg.blobfs);
    let result = services.run().await;
    // BackgroundSession owns the FUSE mount. Unmount before dropping request services.
    drop(mounted);
    let local_result =
        tokio::time::timeout(std::time::Duration::from_secs(10), local.shutdown()).await?;
    result?;
    Ok(local_result?)
}

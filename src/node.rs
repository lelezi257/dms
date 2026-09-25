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

/// REST 持有的进程级共享对象，不是另一个 Home 服务进程。
/// Vfs 用于入口分派；诊断 Storage 与 RDMA 会话表单独交给各 service。
pub struct Node {
    pub config: Config,
    pub observability: Observability,
    pub vfs: Arc<vfs::Vfs>,
}

/// 组装并持有 Node 的所有入口。启动失败清理已经建立的资源，正常退出卸载本进程挂载。
pub async fn run(cfg: Config, obs: Observability) -> Result<(), BoxError> {
    // Bind all TCP ingress before spawning services. A failed bind cannot leave a half-ready Node.
    let grpc = tokio::net::TcpListener::bind(cfg.grpc_listen).await?;
    let rest = tokio::net::TcpListener::bind(cfg.rest_listen).await?;
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
    });
    let mut services = Services::new();
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
    let incoming =
        grpc_config.configure_tcp_incoming(tonic::transport::server::TcpIncoming::from(grpc));
    // 业务 Handler 在 Node，公共 transport 只提供 builder 配置和低层搬运机制。
    let control = rpc::control::make_control_server(sessions.clone());
    let data = rpc::data::make_data_server(storage, sessions.clone());
    services.spawn(async move {
        grpc_config
            .configure_server(tonic::transport::Server::builder())
            .layer(afs_tracing::GrpcServerTraceLayer::default())
            .add_service(control)
            .add_service(data)
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

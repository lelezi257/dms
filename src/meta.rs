//! afs-meta holds coarse authority. Foundation Ping demonstrates ingress only;
//! it does not create root grants or claim durable cluster metadata.
//!
//! `Meta` 是 gRPC/REST 共享的业务对象；`rpc::MetaRpc` 只转换协议并调用它。
//! 当前 Ping 不修改业务状态，所以没有为它引入 Mutex 或 actor。
//! 将来根归属/授权状态应在这里所属的领域模块实现，不能各在 REST 和 gRPC 存一份。
//! 当前尚未连接 etcd、签发 RootGrant 或提供真实位置查询。

pub mod rest;
pub mod rpc;
use crate::{
    config::Config,
    runtime::{BoxError, Observability, Services, cancelled},
};
use std::sync::Arc;
#[derive(Clone)]
pub struct Meta {
    pub id: String,
    pub observability: Observability,
}
impl Meta {
    pub fn ping(&self, node_id: &str) -> Result<String, afs_error::Error> {
        if node_id.len() > 128 {
            self.observability.record("meta", "ping", false);
            return Err(afs_error::Error::new(
                afs_error::ErrorKind::InvalidArgument,
                "node_id exceeds 128 bytes",
            ));
        }
        self.observability.record("meta", "ping", true);
        afs_logging::info!("meta.ping";"caller"=>node_id,"instance"=>&self.id);
        Ok(format!("pong from {}", self.id))
    }
}
/// 先成功绑定两个端口，再并发启动 gRPC 和 REST；它们共享同一个 Arc<Meta>。
pub async fn run(cfg: Config, obs: Observability) -> Result<(), BoxError> {
    let grpc = tokio::net::TcpListener::bind(cfg.grpc_listen).await?;
    let rest = tokio::net::TcpListener::bind(cfg.rest_listen).await?;
    let state = Arc::new(Meta {
        id: cfg.id.clone(),
        observability: obs,
    });
    let mut services = Services::new();
    let stop = services.stop.subscribe();
    let grpc_config = afs_transport::grpc::GrpcConfig::default();
    let incoming =
        grpc_config.configure_tcp_incoming(tonic::transport::server::TcpIncoming::from(grpc));
    let service = afs_protocol::meta::meta_server::MetaServer::new(rpc::MetaRpc(state.clone()));
    services.spawn(async move {
        grpc_config
            .configure_server(tonic::transport::Server::builder())
            .layer(afs_tracing::GrpcServerTraceLayer::default())
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, cancelled(stop))
            .await
            .map_err(Into::into)
    });
    let stop = services.stop.subscribe();
    services.spawn(async move {
        axum::serve(rest, rest::router(state))
            .with_graceful_shutdown(cancelled(stop))
            .await
            .map_err(Into::into)
    });
    afs_logging::info!("meta.ready";"grpc"=>cfg.grpc_listen.to_string(),"rest"=>cfg.rest_listen.to_string());
    services.run().await
}

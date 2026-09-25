//! Generated gRPC adapter around the same Meta service used by REST.
//!
//! Tonic 生成的 Meta trait 是协议适配入口，业务校验和指标在 Meta::ping。
//! Handler 本身运行在 Tokio 上，能直接 await 异步业务；不需要为每个 RPC 包一层 actor。
//! 此处 Ping 全是短同步工作。以后遇到阻塞文件 I/O，应像 node/storage 一样移出执行线程。
use afs_protocol::meta::{PingReply, PingRequest, meta_server::Meta as MetaService};
use std::sync::Arc;
use tonic::{Request, Response, Status};
pub struct MetaRpc(pub Arc<super::Meta>);
#[tonic::async_trait]
impl MetaService for MetaRpc {
    async fn ping(&self, request: Request<PingRequest>) -> Result<Response<PingReply>, Status> {
        let span = afs_tracing::tracing::info_span!("meta.ping");
        let _entered = span.enter();
        let message = self
            .0
            .ping(&request.into_inner().node_id)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        Ok(Response::new(PingReply {
            message,
            instance: self.0.id.clone(),
        }))
    }
}

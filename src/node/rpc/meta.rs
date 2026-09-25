//! Node owns its generated Meta caller; common transport only configures it.
//!
//! 调用端直接使用 Proto 生成的 MetaClient，再应用 common/transport/grpc 的配置。
//! 这是 Node 的内部控制 caller，不是对外的本机高性能 client/ SDK。
//! 示例每次显式 ping 建连；未来节点注册/watch 的连接复用由相应业务模块管理。
use crate::runtime::BoxError;
use afs_protocol::meta::{PingRequest, meta_client::MetaClient};
use afs_transport::grpc::GrpcConfig;
use std::time::Duration;
use tonic::transport::Endpoint;
pub async fn ping(endpoint: &str, node_id: &str, timeout: Duration) -> Result<String, BoxError> {
    let config = GrpcConfig {
        connect_timeout: timeout,
        request_timeout: timeout,
        ..Default::default()
    };
    let channel = config
        .configure_client(Endpoint::from_shared(endpoint.to_owned())?)
        .connect()
        .await?;
    let mut client = MetaClient::new(afs_tracing::traced_channel(channel))
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes);
    let reply = client
        .ping(afs_tracing::request_with_current_context(PingRequest {
            node_id: node_id.into(),
        }))
        .await?;
    Ok(reply.into_inner().message)
}

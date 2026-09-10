//! SDK 内部 generated gRPC client 构造入口。
//!
//! `GrpcConfig::configure_client()` 只配置 Tonic `Endpoint`/HTTP2 连接参数；
//! protobuf 单条消息的编码/解码上限必须继续设置到每个 generated typed
//! client 上。所有生产路径从这里创建 typed client，避免 Worker 与 Payload
//! 两条链路使用不同的消息预算。

use dms_protocol::v1 as pb;
use dms_transport::GrpcConfig;
use tonic::transport::Channel;

pub(crate) fn worker_client(
    channel: Channel,
    config: &GrpcConfig,
) -> pb::worker_service_client::WorkerServiceClient<dms_tracing::TracedChannel> {
    pb::worker_service_client::WorkerServiceClient::new(dms_tracing::traced_channel(channel))
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes)
}

pub(crate) fn worker_payload_client(
    channel: Channel,
    config: &GrpcConfig,
) -> pb::worker_payload_service_client::WorkerPayloadServiceClient<dms_tracing::TracedChannel> {
    pb::worker_payload_service_client::WorkerPayloadServiceClient::new(dms_tracing::traced_channel(
        channel,
    ))
    .max_encoding_message_size(config.max_encoding_message_bytes)
    .max_decoding_message_size(config.max_decoding_message_bytes)
}

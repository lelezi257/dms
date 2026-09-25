//! Runtime-facing REST. Explicit diagnostics exercise real APIs, never run on POSIX hot paths.
//!
//! `/v1/diagnostics` 是显式演示入口：Node B→Meta Ping→A 控制 Ping→A 数据写/读。
//! 它调用正式 caller/adapter，不是另写一套模拟传输。正常 FUSE 回调不会自动触发这些 RPC。
//! REST 面向 runtime 的 snapshot/publish API 尚未实现；不要用本诊断接口推断镜像发布成功。
use crate::node::{
    Node,
    rpc::{
        meta,
        peer::{DataClientOptions, DataMode, connect_data_client},
    },
};
use crate::runtime::BoxError;
use afs_protocol::node_control::{PingRequest, node_control_client::NodeControlClient};
use afs_tracing::{Instrument, tracing};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/ping", get(ping))
        .route("/v1/diagnostics", post(diagnostics))
        .route("/metrics", get(metrics))
        .with_state(node)
}
async fn health(State(node): State<Arc<Node>>) -> Json<Value> {
    Json(
        json!({"status":"ready","role":"node","id":node.config.id,"scope":"foundation","ownerfs":node.config.ownerfs,"blobfs":node.config.blobfs}),
    )
}
async fn ping(State(node): State<Arc<Node>>) -> Json<Value> {
    node.observability.record("node", "rest_ping", true);
    Json(json!({"message":format!("pong from {}",node.config.id)}))
}
async fn metrics(State(node): State<Arc<Node>>) -> Result<String, (StatusCode, String)> {
    afs_metrics::encode_text(&node.observability.registry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}
async fn diagnostics(State(node): State<Arc<Node>>) -> Result<Json<Value>, (StatusCode, String)> {
    let result = diagnose(node.clone())
        .instrument(tracing::info_span!("node.diagnostics"))
        .await;
    node.observability
        .record("node", "diagnostics", result.is_ok());
    result.map(Json).map_err(|e| {
        afs_logging::error!("node.diagnostics failed";"error"=>e.to_string());
        (StatusCode::BAD_GATEWAY, e.to_string())
    })
}
async fn diagnose(node: Arc<Node>) -> Result<Value, BoxError> {
    let config = &node.config;
    let meta_endpoint = config
        .meta_endpoint
        .as_ref()
        .ok_or("diagnostics requires meta_endpoint")?;
    let peer_endpoint = config
        .peer_endpoint
        .as_ref()
        .ok_or("diagnostics requires peer_endpoint")?;
    let timeout = Duration::from_millis(config.timeout_ms);
    let meta_pong = meta::ping(meta_endpoint, &config.id, timeout).await?;
    let grpc_config = afs_transport::grpc::GrpcConfig {
        connect_timeout: timeout,
        request_timeout: timeout,
        ..Default::default()
    };
    let channel = grpc_config
        .configure_client(tonic::transport::Endpoint::from_shared(
            peer_endpoint.clone(),
        )?)
        .connect()
        .await?;
    let mut control = NodeControlClient::new(afs_tracing::traced_channel(channel));
    let pong = control
        .ping(afs_tracing::request_with_current_context(PingRequest {
            payload: "ping".into(),
        }))
        .await?
        .into_inner();
    let mode = match config.data_mode.as_str() {
        "grpc" => DataMode::Grpc,
        "rdma" => DataMode::Rdma,
        _ => DataMode::Auto,
    };
    let mut data = connect_data_client(DataClientOptions {
        endpoint: peer_endpoint.clone(),
        mode,
        rdma_device: config.rdma_device.clone(),
        timeout,
    })
    .await?;
    let chosen = data.mode().to_owned();
    // Dedicated foundation diagnostic file, not an OwnerFs/BlobFs file or publication.
    let name = format!("probe-{}", config.id);
    let transfer = async {
        let written = data.write(&name, 0, b"AFShello".to_vec()).await?;
        let read = data.read(&name, 0, 8).await?;
        if written != 8 || read != b"AFShello" {
            return Err::<(), BoxError>("data roundtrip mismatch".into());
        }
        Ok(())
    }
    .await;
    let closed = data.close().await;
    transfer?;
    closed?;
    afs_logging::info!("node.diagnostics completed";"mode"=>&chosen,"bytes"=>8);
    Ok(json!({"ok":true,"mode":chosen,"bytes":8,"meta":meta_pong,"peer_control":pong.payload}))
}

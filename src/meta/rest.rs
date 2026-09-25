//! Management REST uses Meta business state. Root lookup is not fabricated in Ping.
//!
//! 管理面由普通 HTTP 客户端访问，不增加管理 SDK。
//! `/health` 只报告基础服务就绪；`/v1/ping` 复用 Meta::ping；`/metrics` 导出同一注册表。
//! 根位置查询接口属于后续业务实现，不能把当前 ping 的返回值当作节点注册/目录归属。
use super::Meta;
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde_json::{Value, json};
use std::sync::Arc;
pub fn router(meta: Arc<Meta>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/ping", get(ping))
        .route("/metrics", get(metrics))
        .with_state(meta)
}
async fn health(State(meta): State<Arc<Meta>>) -> Json<Value> {
    Json(json!({"status":"ready","role":"meta","id":meta.id,"scope":"foundation"}))
}
async fn ping(State(meta): State<Arc<Meta>>) -> Result<Json<Value>, (StatusCode, String)> {
    meta.ping("rest")
        .map(|message| Json(json!({"message":message})))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}
async fn metrics(State(meta): State<Arc<Meta>>) -> Result<String, (StatusCode, String)> {
    afs_metrics::encode_text(&meta.observability.registry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

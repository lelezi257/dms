//! Management REST uses Meta business state. Root lookup is not fabricated in Ping.
//!
//! 管理面由普通 HTTP 客户端访问，不增加管理 SDK。
//! `/health` 只报告基础服务就绪；`/v1/ping` 复用 Meta::ping；`/metrics` 导出同一注册表。
//! `/v1/roots/{root_id}` 是调度器查询 OwnerFs 根目录位置的管理面入口，
//! 只能读取配置的 MetaStore，不能把当前 ping 的返回值当作节点注册/目录归属。
use super::{
    Meta,
    store::{MetaEntity, MetaRead, unavailable_meta_store},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde_json::{Value, json};
use std::sync::Arc;
pub fn router(meta: Arc<Meta>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/ping", get(ping))
        .route("/v1/roots/{root_id}", get(root_location))
        .route("/metrics", get(metrics))
        .with_state(meta)
}
async fn health(State(meta): State<Arc<Meta>>) -> Json<Value> {
    Json(json!({"status":"ready","role":"meta","id":meta.id,"scope":"foundation"}))
}
async fn ping(State(meta): State<Arc<Meta>>) -> Result<Json<Value>, crate::error::RestError> {
    meta.ping("rest")
        .map(|message| Json(json!({"message":message})))
        .map_err(Into::into)
}
async fn metrics(State(meta): State<Arc<Meta>>) -> Result<String, crate::error::RestError> {
    afs_metrics::encode_text(&meta.observability.registry).map_err(|e| {
        crate::error::RestError(afs_error::Error::coded(
            afs_error::METRICS_FAILED,
            e.to_string(),
        ))
    })
}

async fn root_location(
    State(meta): State<Arc<Meta>>,
    Path(root_id): Path<String>,
) -> Result<Json<Value>, crate::error::RestError> {
    if root_id.is_empty() {
        return Err(crate::error::RestError(afs_error::Error::coded(
            afs_error::META_CATALOG_INVALID_REQUEST,
            "root_id is required",
        )));
    }
    let store = meta
        .store
        .as_deref()
        .ok_or_else(|| crate::error::RestError(unavailable_meta_store()))?;
    let snapshot = store
        .read(MetaRead::Root {
            root_id: root_id.clone(),
        })
        .await?;
    let Some(MetaEntity::Root(root)) = snapshot.entity else {
        return Err(crate::error::RestError(afs_error::Error::coded(
            afs_error::IO_NOT_FOUND,
            format!("root {root_id} is not registered"),
        )));
    };
    let owner = root.home_node_id.clone();
    Ok(Json(json!({
        "root_id": root.root_id,
        "status": "active",
        "owner": owner,
        "root_epoch": root.root_epoch,
        "home_node_id": root.home_node_id,
        "home_session_id": root.home_session_id,
        "revision": snapshot.revision.0,
    })))
}

//! Management REST uses Meta business state. Root lookup is not fabricated in Ping.
//!
//! 管理面由普通 HTTP 客户端访问，不增加管理 SDK。
//! `/health` reports ready only after the configured MetaStore can serve a read.
//! `/v1/ping` 复用 Meta::ping；`/metrics` 导出同一注册表。
//! `/v1/roots/{root_id}` 是调度器查询 OwnerFs 根目录位置的管理面入口，
//! 只能读取配置的 MetaStore，不能把当前 ping 的返回值当作节点注册/目录归属。
use super::{
    Meta,
    store::{MetaEntity, MetaRead, now_unix_ms, unavailable_meta_store},
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
async fn health(State(meta): State<Arc<Meta>>) -> Result<Json<Value>, crate::error::RestError> {
    let store = meta
        .store
        .as_deref()
        .ok_or_else(|| crate::error::RestError(unavailable_meta_store()))?;
    store.health().await?;
    Ok(Json(
        json!({"status":"ready","role":"meta","id":meta.id,"scope":"foundation"}),
    ))
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
    store.health().await?;
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

    let home_snapshot = store
        .read(MetaRead::NodeSession {
            node_id: root.home_node_id.clone(),
            session_id: root.home_session_id.clone(),
        })
        .await?;
    let now = now_unix_ms();
    let (
        home_serving,
        home_lease_epoch,
        home_session_expires_at_unix_ms,
        home_grpc_addr,
        home_data_addr,
        home_rest_addr,
    ) = match home_snapshot.entity {
        Some(MetaEntity::NodeSession(session)) => (
            session.is_live_at_unix_ms(now),
            Some(session.lease_epoch),
            Some(session.expires_at_unix_ms),
            Some(session.grpc_addr),
            Some(session.data_addr),
            Some(session.rest_addr),
        ),
        _ => (false, None, None, None, None, None),
    };
    let status = if home_serving {
        "serving"
    } else {
        "unavailable"
    };
    let owner = root.home_node_id.clone();
    Ok(Json(json!({
        "root_id": root.root_id,
        "status": status,
        "home_serving": home_serving,
        "owner": owner,
        "root_epoch": root.root_epoch,
        "home_node_id": root.home_node_id,
        "home_session_id": root.home_session_id,
        "home_lease_epoch": home_lease_epoch,
        "home_session_expires_at_unix_ms": home_session_expires_at_unix_ms,
        "home_grpc_addr": home_grpc_addr,
        "home_data_addr": home_data_addr,
        "home_rest_addr": home_rest_addr,
        "checked_at_unix_ms": now,
        "revision": snapshot.revision.0,
    })))
}

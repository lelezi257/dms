use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use afs::{
    meta::{
        Meta,
        owner_roots::{ActivateRootInput, ReserveRootInput, WireRootReservation},
        rest,
        store::{
            MetaFuture, NodeSessionLease, RequestKey, RootRight as StoreRootRight, Store,
            StoreBackend, memory::MemoryBackend,
        },
    },
    runtime::Observability,
};
use axum::{
    body::to_bytes,
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;

#[derive(Default)]
struct SwitchableBackend {
    inner: MemoryBackend,
    reject_load: AtomicBool,
}

impl SwitchableBackend {
    fn reject_load(&self) {
        self.reject_load.store(true, Ordering::Release);
    }
}

impl StoreBackend for SwitchableBackend {
    fn load(&self) -> MetaFuture<'_, Option<(u64, Vec<u8>)>> {
        Box::pin(async move {
            if self.reject_load.load(Ordering::Acquire) {
                Err(afs_error::Error::coded(
                    afs_error::IO_UNAVAILABLE,
                    "injected backend read failure",
                ))
            } else {
                self.inner.load().await
            }
        })
    }

    fn commit(&self, expected_version: u64, bytes: Vec<u8>) -> MetaFuture<'_, u64> {
        self.inner.commit(expected_version, bytes)
    }
}

async fn meta_with_memory_store() -> (Arc<Meta>, Arc<Store>) {
    let store = Arc::new(
        Store::open(Arc::new(MemoryBackend::default()))
            .await
            .unwrap(),
    );
    let meta = Arc::new(Meta::with_store(
        "meta-rest-test".into(),
        Observability::new().unwrap(),
        store.clone(),
    ));
    (meta, store)
}

async fn register_node_with_ttl(meta: &Meta, session_id: &str, ttl: Duration) {
    meta.register_node(
        RequestKey::new(
            format!("node-a/{session_id}"),
            format!("register-{session_id}"),
        ),
        NodeSessionLease {
            node_id: "node-a".into(),
            session_id: session_id.into(),
            grpc_addr: "http://node-a:7400".into(),
            data_addr: "http://node-a:7500".into(),
            rest_addr: "http://node-a:7600".into(),
            storage_devices: Vec::new(),
            lease_ttl: ttl,
        },
    )
    .await
    .unwrap();
}

async fn activate_root(meta: &Meta, root_id: &str, session_id: &str) {
    let reserved = meta
        .owner_roots
        .reserve_root(ReserveRootInput {
            request_id: format!("reserve-{root_id}"),
            root_id: root_id.into(),
            preferred_home_node_id: "node-a".into(),
            session_id: session_id.into(),
            rights: vec![StoreRootRight::Lookup, StoreRootRight::Write],
            expected_root_epoch: 1,
            create_intent_id: format!("mkdir-{root_id}"),
        })
        .await
        .unwrap()
        .reservation;
    meta.owner_roots
        .activate_root(ActivateRootInput {
            request_id: format!("activate-{root_id}"),
            reservation: WireRootReservation {
                root_id: reserved.root_id,
                root_epoch: reserved.root_epoch,
                home_node_id: reserved.home_node_id,
                session_id: reserved.home_session_id,
                create_intent_id: reserved.create_intent_id,
                prepare_token: reserved.prepare_token,
            },
            local_prepare_id: format!("local-{root_id}"),
        })
        .await
        .unwrap();
}

async fn get_json(meta: Arc<Meta>, path: &str) -> (StatusCode, Value) {
    let response = rest::router(meta)
        .oneshot(
            Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let json = serde_json::from_slice(&body).unwrap();
    (status, json)
}

#[tokio::test]
async fn health_requires_a_meta_store() {
    let meta = Arc::new(Meta::new(
        "meta-rest-test".into(),
        Observability::new().unwrap(),
    ));

    let (status, body) = get_json(meta, "/health").await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["error"]["kind"], "Unimplemented");
}

#[tokio::test]
async fn root_location_reports_serving_for_live_exact_home_session() {
    let (meta, _store) = meta_with_memory_store().await;
    register_node_with_ttl(&meta, "session-live", Duration::from_secs(30)).await;
    activate_root(&meta, "workspace-live", "session-live").await;

    let (status, body) = get_json(meta, "/v1/roots/workspace-live").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["root_id"], "workspace-live");
    assert_eq!(body["status"], "serving");
    assert_eq!(body["home_serving"], true);
    assert_eq!(body["home_node_id"], "node-a");
    assert_eq!(body["home_session_id"], "session-live");
    assert_eq!(body["home_grpc_addr"], "http://node-a:7400");
}

#[tokio::test]
async fn root_location_keeps_owner_but_reports_unavailable_for_expired_home_session() {
    let (meta, _store) = meta_with_memory_store().await;
    register_node_with_ttl(&meta, "session-expired", Duration::from_millis(50)).await;
    activate_root(&meta, "workspace-expired", "session-expired").await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (status, body) = get_json(meta, "/v1/roots/workspace-expired").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["root_id"], "workspace-expired");
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["home_serving"], false);
    assert_eq!(body["owner"], "node-a");
    assert_eq!(body["home_node_id"], "node-a");
    assert_eq!(body["home_session_id"], "session-expired");
    assert_eq!(body["home_grpc_addr"], "http://node-a:7400");
}

#[tokio::test]
async fn health_uses_backend_probe_not_cached_read_view() {
    let backend = Arc::new(SwitchableBackend::default());
    let store = Arc::new(Store::open(backend.clone()).await.unwrap());
    let meta = Arc::new(Meta::with_store(
        "meta-rest-test".into(),
        Observability::new().unwrap(),
        store,
    ));
    register_node_with_ttl(&meta, "session-live", Duration::from_secs(30)).await;
    activate_root(&meta, "workspace-live", "session-live").await;

    backend.reject_load();

    let (health_status, health_body) = get_json(meta.clone(), "/health").await;
    assert_eq!(health_status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(health_body["error"]["kind"], "Unavailable");

    let (root_status, root_body) = get_json(meta, "/v1/roots/workspace-live").await;
    assert_eq!(root_status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(root_body["error"]["kind"], "Unavailable");
}

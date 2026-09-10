//! 缺块合并的 owner 状态回归：不靠 TTL 或 sleep 证明回收安全。
use super::*;

fn setup() -> (NodeState, u64, PeerPullSpec) {
    let mut state = NodeState::new(
        "peer-target".into(),
        None,
        4096,
        Duration::from_secs(30),
        None,
    );
    let session = state.open_session(false);
    let spec = PeerPullSpec {
        endpoint: "http://127.0.0.1:1".into(),
        block_id: b"shared-block".to_vec(),
        expected_checksum: digest(b"abcdefgh"),
        expected_length: 8,
    };
    (state, session, spec)
}

// owner 测试读取真实容器/allocator 计数；零值也来自状态，不用预写的期望值冒充测量。
fn resource_counts(state: &NodeState) -> [u64; 3] {
    let arena = state.arena.stats();
    [
        arena.staging_count as u64,
        state.downloads.len() as u64,
        arena.quarantined_bytes,
    ]
}

fn print_cleanup_proof(
    name: &str,
    before: [u64; 3],
    peak: [u64; 3],
    after: [u64; 3],
    error_observed: bool,
) {
    assert!(error_observed);
    for (settled, initial) in after.into_iter().zip(before) {
        assert!(settled <= initial);
    }
    println!(
        "DMS_MECHANISM_PROOF {{\"mechanism\":\"failure_cleanup\",\"case\":\"{}\",\"failure_injected\":true,\"expected_error_observed\":{},\"before\":{{\"staging_allocations\":{},\"download_tickets\":{},\"quarantined_bytes\":{}}},\"peak\":{{\"staging_allocations\":{},\"download_tickets\":{},\"quarantined_bytes\":{}}},\"after_settled\":{{\"staging_allocations\":{},\"download_tickets\":{},\"quarantined_bytes\":{}}},\"measurement\":\"owner-state-transition-injection-not-network-fault\"}}",
        name,
        error_observed,
        before[0],
        before[1],
        before[2],
        peak[0],
        peak[1],
        peak[2],
        after[0],
        after[1],
        after[2]
    );
}

#[test]
fn mechanism_proof_transfer_failure_cleans_owner_resources() {
    let (mut state, session, spec) = setup();
    let before = resource_counts(&state);
    let scope = state.begin_read_scope(session).unwrap();
    let (tx, mut rx) = oneshot::channel();
    let attempt = state.begin_peer_import(scope, &spec, tx).unwrap();
    assert_eq!(state.peer_import_bytes, spec.expected_length);
    // 对应 Peer RPC 返回失败、尚未安装 payload 的 owner 完成分支。
    state.complete_peer_import(
        &spec.block_id,
        attempt,
        Err(PeerImportFailure {
            error: WorkerError::TransferUnavailable,
            location_failure: true,
        }),
    );
    let observed = matches!(
        rx.try_recv().unwrap(),
        Err(PeerImportFailure {
            error: WorkerError::TransferUnavailable,
            ..
        })
    );
    let peak = resource_counts(&state);
    state.finish_read_scope(scope);
    state.tick();
    assert!(state.peer_imports.is_empty());
    assert!(state.peer_import_failures.is_empty());
    assert_eq!(state.peer_import_bytes, 0);
    print_cleanup_proof(
        "transfer-failure",
        before,
        peak,
        resource_counts(&state),
        observed,
    );
}

#[test]
fn mechanism_proof_cancelled_read_reclaims_unconsumed_download_ticket() {
    let (mut state, session, spec) = setup();
    let before = resource_counts(&state);
    let scope = state.begin_read_scope(session).unwrap();
    let (tx, rx) = oneshot::channel();
    let attempt = state.begin_peer_import(scope, &spec, tx).unwrap();
    drop(rx); // 调用者取消，独立网络任务仍持有 origin scope。
    state.finish_read_scope(scope);
    assert!(state.peer_imports[&spec.block_id].waiters[0].is_closed());
    state
        .import_peer_block(
            scope,
            spec.block_id.clone(),
            b"abcdefgh".to_vec(),
            spec.expected_checksum,
            8,
        )
        .unwrap();
    state.complete_peer_import(&spec.block_id, attempt, Ok(()));
    let (read, _) = state.arena.open_read(&spec.block_id, None).unwrap();
    // 另一类取消窗口：GET 已创建下载票据，但回复未到 SDK；完成水位必须回收它。
    state.downloads.insert(
        901,
        DownloadTicket {
            read,
            session_id: session,
            read_request_id: 1,
            expires_at: Instant::now() + Duration::from_secs(30),
        },
    );
    let peak = resource_counts(&state);
    assert_eq!(peak[1], 1);
    state
        .heartbeat_inner(session, None, Vec::new(), Some(1), false)
        .unwrap();
    let observed = matches!(state.download(901), Err(WorkerError::UnknownTransfer));
    state.tick();
    assert!(state.peer_imports.is_empty());
    assert_eq!(state.peer_import_bytes, 0);
    print_cleanup_proof(
        "cancelled-read",
        before,
        peak,
        resource_counts(&state),
        observed,
    );
}

#[test]
fn peer_import_completed_block_rejects_late_same_length_different_checksum() {
    let (mut state, session, spec) = setup();
    let scope = state.begin_read_scope(session).unwrap();
    let (tx, mut rx) = oneshot::channel();
    let attempt = state.begin_peer_import(scope, &spec, tx).unwrap();
    state
        .import_peer_block(
            scope,
            spec.block_id.clone(),
            b"abcdefgh".to_vec(),
            spec.expected_checksum.clone(),
            8,
        )
        .unwrap();
    state.complete_peer_import(&spec.block_id, attempt, Ok(()));
    assert!(rx.try_recv().unwrap().is_ok());
    let mut changed = spec.clone();
    changed.expected_checksum = digest(b"ABCDEFGH");
    let (tx, mut rx) = oneshot::channel();
    assert!(state.begin_peer_import(scope, &changed, tx).is_none());
    assert!(
        matches!(
            rx.try_recv().unwrap(),
            Err(PeerImportFailure {
                error: WorkerError::Conflict,
                ..
            })
        ),
        "迟到的同长异摘要请求不能命中已完成 Block"
    );
    let (tx, mut rx) = oneshot::channel();
    assert!(state.begin_peer_import(scope, &spec, tx).is_none());
    assert!(rx.try_recv().unwrap().is_ok());
}

#[test]
fn peer_import_and_prepared_replica_keep_real_digest_when_legacy_checksum_is_empty() {
    let (mut state, session, spec) = setup();
    let scope = state.begin_read_scope(session).unwrap();
    state
        .import_peer_block(
            scope,
            spec.block_id.clone(),
            b"abcdefgh".to_vec(),
            Vec::new(),
            8,
        )
        .unwrap();
    assert_eq!(
        state.arena.block_length_and_digest(&spec.block_id),
        Some((8, spec.expected_checksum.as_slice()))
    );
    let mut legacy = spec.clone();
    legacy.expected_checksum.clear();
    let (tx, mut rx) = oneshot::channel();
    assert!(state.begin_peer_import(scope, &legacy, tx).is_none());
    assert!(
        rx.try_recv().unwrap().is_ok(),
        "旧协议不提供expected摘要仍可命中，stored摘要不能为空"
    );
    let plan = b"legacy-plan".to_vec();
    state
        .prepare_replica(
            plan.clone(),
            b"replica".to_vec(),
            b"next".to_vec(),
            Vec::new(),
            4,
        )
        .unwrap();
    state.activate_replica(plan).unwrap();
    assert_eq!(
        state.arena.block_length_and_digest(b"replica"),
        Some((4, digest(b"next").as_slice()))
    );
}

#[test]
fn peer_import_cancelled_leader_still_pins_prepare_until_job_and_followers_finish() {
    let (mut state, session, spec) = setup();
    let leader = state.begin_read_scope(session).unwrap();
    let follower = state.begin_read_scope(session).unwrap();
    let (tx1, rx1) = oneshot::channel();
    let attempt = state.begin_peer_import(leader, &spec, tx1).unwrap();
    let (tx2, mut rx2) = oneshot::channel();
    assert!(state.begin_peer_import(follower, &spec, tx2).is_none());
    drop(rx1);
    state.finish_read_scope(leader);
    let (prepare, mut prepared) = oneshot::channel();
    state.register_prepare_retirement(b"retirement".to_vec(), vec![spec.block_id.clone()], prepare);
    assert!(prepared.try_recv().is_err());
    let new_scope = state.begin_read_scope(session).unwrap();
    let (tx3, mut rx3) = oneshot::channel();
    assert!(state.begin_peer_import(new_scope, &spec, tx3).is_none());
    assert!(matches!(
        rx3.try_recv().unwrap().unwrap_err().error,
        WorkerError::NotFound
    ));
    // 初始调用者已取消，但任务的 origin scope 仍受 owner 保护，旧读可完成安装；
    // 正在回收的 Block 不能再次 Report 成为可达副本。
    assert!(
        !state
            .import_peer_block(
                leader,
                spec.block_id.clone(),
                b"abcdefgh".to_vec(),
                spec.expected_checksum.clone(),
                8
            )
            .unwrap()
    );
    state.complete_peer_import(&spec.block_id, attempt, Ok(()));
    assert!(rx2.try_recv().unwrap().is_ok());
    assert!(
        prepared.try_recv().is_err(),
        "follower 的独立 scope 仍需排空"
    );
    state.finish_read_scope(follower);
    assert!(prepared.try_recv().unwrap().is_ok());
    let (final_tx, mut final_rx) = oneshot::channel();
    state.register_final_retirement(
        b"retirement".to_vec(),
        vec![spec.block_id.clone()],
        final_tx,
    );
    assert!(final_rx.try_recv().unwrap().is_ok());
    assert!(state.arena.read_bytes(&spec.block_id).is_none());
    assert_eq!(state.peer_import_bytes, 0);
}

#[test]
fn peer_import_report_failure_reaches_delayed_old_plan_then_scope_fence_disappears() {
    let (mut state, session, spec) = setup();
    let leader = state.begin_read_scope(session).unwrap();
    let delayed = state.begin_read_scope(session).unwrap();
    let (tx, mut rx) = oneshot::channel();
    let attempt = state.begin_peer_import(leader, &spec, tx).unwrap();
    state
        .import_peer_block(
            leader,
            spec.block_id.clone(),
            b"abcdefgh".to_vec(),
            spec.expected_checksum.clone(),
            8,
        )
        .unwrap();
    state.complete_peer_import(
        &spec.block_id,
        attempt,
        Err(PeerImportFailure::terminal(
            WorkerError::MetadataUnavailable,
        )),
    );
    assert!(matches!(
        rx.try_recv().unwrap().unwrap_err().error,
        WorkerError::MetadataUnavailable
    ));
    state.finish_read_scope(leader);
    // delayed 已取到缺块计划，但尚未发 Ensure，不能因 bytes 已安装而吞掉 Report 错误。
    let (late_tx, mut late_rx) = oneshot::channel();
    assert!(state.begin_peer_import(delayed, &spec, late_tx).is_none());
    assert!(matches!(
        late_rx.try_recv().unwrap().unwrap_err().error,
        WorkerError::MetadataUnavailable
    ));
    assert_eq!(state.peer_import_failures.len(), 1);
    let fresh = state.begin_read_scope(session).unwrap();
    assert!(state.peer_import_failure(&spec.block_id, fresh).is_none());
    state.finish_read_scope(delayed);
    assert!(
        state.peer_import_failures.is_empty(),
        "旧 scope 排空即删除，不等定时器"
    );
    assert!(
        state.arena.read_bytes(&spec.block_id).is_some(),
        "未知 Report 结果不能回滚可能已登记的 bytes"
    );
}

#[test]
fn peer_import_location_failure_allows_same_scope_exact_retry_and_stale_completion_is_ignored() {
    let (mut state, session, spec) = setup();
    let scope = state.begin_read_scope(session).unwrap();
    let (tx1, mut rx1) = oneshot::channel();
    let attempt1 = state.begin_peer_import(scope, &spec, tx1).unwrap();
    assert!(
        state
            .validate_peer_import_attempt(&spec.block_id, scope, attempt1)
            .is_ok()
    );
    state.complete_peer_import(
        &spec.block_id,
        attempt1,
        Err(PeerImportFailure {
            error: WorkerError::TransferUnavailable,
            location_failure: true,
        }),
    );
    assert!(rx1.try_recv().unwrap().unwrap_err().location_failure);
    assert!(state.peer_import_failures.is_empty());
    assert!(
        state
            .validate_peer_import_attempt(&spec.block_id, scope, attempt1)
            .is_err()
    );
    let mut refreshed = spec.clone();
    refreshed.endpoint = "http://127.0.0.1:2".into();
    let (tx2, mut rx2) = oneshot::channel();
    let attempt2 = state.begin_peer_import(scope, &refreshed, tx2).unwrap();
    assert_ne!(attempt1, attempt2);
    assert!(
        state
            .validate_peer_import_attempt(&spec.block_id, scope, attempt1)
            .is_err()
    );
    assert!(
        state
            .validate_peer_import_attempt(&spec.block_id, scope, attempt2)
            .is_ok()
    );
    state.complete_peer_import(&spec.block_id, attempt1, Ok(()));
    assert_eq!(state.peer_imports[&spec.block_id].attempt, attempt2);
    assert!(rx2.try_recv().is_err());
    state.complete_peer_import(&spec.block_id, attempt2, Ok(()));
    assert!(rx2.try_recv().unwrap().is_ok());
    assert_eq!(state.peer_import_bytes, 0);
}

#[test]
fn peer_import_terminal_failure_before_install_does_not_poison_later_scope_retry() {
    let (mut state, session, spec) = setup();
    let old = state.begin_read_scope(session).unwrap();
    let (tx, mut rx) = oneshot::channel();
    let attempt = state.begin_peer_import(old, &spec, tx).unwrap();
    state.complete_peer_import(
        &spec.block_id,
        attempt,
        Err(PeerImportFailure::terminal(WorkerError::Conflict)),
    );
    assert!(rx.try_recv().unwrap().is_err());
    assert!(state.arena.read_bytes(&spec.block_id).is_none());
    // 无需等待旧请求排空，新请求可用自己的 scope 开始新批次。
    let fresh = state.begin_read_scope(session).unwrap();
    let (tx, mut rx) = oneshot::channel();
    let retry = state.begin_peer_import(fresh, &spec, tx).unwrap();
    assert_ne!(attempt, retry);
    assert!(
        state
            .validate_peer_import_attempt(&spec.block_id, old, attempt)
            .is_err()
    );
    assert!(
        state
            .validate_peer_import_attempt(&spec.block_id, fresh, retry)
            .is_ok()
    );
    state
        .import_peer_block(
            fresh,
            spec.block_id.clone(),
            b"abcdefgh".to_vec(),
            spec.expected_checksum,
            8,
        )
        .unwrap();
    state.complete_peer_import(&spec.block_id, retry, Ok(()));
    assert!(rx.try_recv().unwrap().is_ok());
    state.finish_read_scope(old);
    state.finish_read_scope(fresh);
    assert!(state.peer_import_failures.is_empty());
    assert_eq!(state.peer_import_bytes, 0);
}

#[test]
fn peer_import_same_identity_conflicts_and_waiters_are_bounded() {
    let (mut state, session, spec) = setup();
    let scope = state.begin_read_scope(session).unwrap();
    let mut receivers = Vec::new();
    for index in 0..NODE_MAILBOX_CAPACITY {
        let (tx, rx) = oneshot::channel();
        assert_eq!(
            state.begin_peer_import(scope, &spec, tx).is_some(),
            index == 0
        );
        receivers.push(rx);
    }
    let (overflow, mut overflow_rx) = oneshot::channel();
    assert!(state.begin_peer_import(scope, &spec, overflow).is_none());
    assert!(matches!(
        overflow_rx.try_recv().unwrap().unwrap_err().error,
        WorkerError::ResourceExhausted
    ));
    let mut wrong = spec.clone();
    wrong.expected_checksum = digest(b"different");
    let (conflict, mut conflict_rx) = oneshot::channel();
    assert!(state.begin_peer_import(scope, &wrong, conflict).is_none());
    assert!(matches!(
        conflict_rx.try_recv().unwrap().unwrap_err().error,
        WorkerError::Conflict
    ));
    assert_eq!(state.peer_import_bytes, 8, "followers 不重复占整块预算");
}

#[test]
fn peer_import_flights_and_failure_fences_share_one_bounded_budget() {
    let (mut state, session, mut spec) = setup();
    let scope = state.begin_read_scope(session).unwrap();
    for index in 0..NODE_MAILBOX_CAPACITY {
        spec.block_id = index.to_be_bytes().to_vec();
        let (tx, _rx) = oneshot::channel();
        let attempt = state.begin_peer_import(scope, &spec, tx).unwrap();
        state.complete_peer_import(
            &spec.block_id,
            attempt,
            Err(PeerImportFailure::terminal(WorkerError::Conflict)),
        );
    }
    assert_eq!(state.peer_import_failures.len(), NODE_MAILBOX_CAPACITY);
    spec.block_id = b"overflow".to_vec();
    let (tx, mut rx) = oneshot::channel();
    assert!(state.begin_peer_import(scope, &spec, tx).is_none());
    assert!(matches!(
        rx.try_recv().unwrap().unwrap_err().error,
        WorkerError::ResourceExhausted
    ));
    state.finish_read_scope(scope);
    assert!(state.peer_import_failures.is_empty());
    let fresh = state.begin_read_scope(session).unwrap();
    let (tx, _rx) = oneshot::channel();
    assert!(state.begin_peer_import(fresh, &spec, tx).is_some());
}

#[test]
fn peer_import_bytes_budget_rejects_oversubscription_without_charging_followers() {
    let (mut state, session, mut spec) = setup();
    let scope = state.begin_read_scope(session).unwrap();
    spec.expected_length = 4096;
    let (tx, _rx) = oneshot::channel();
    let attempt = state.begin_peer_import(scope, &spec, tx).unwrap();
    let mut second = spec.clone();
    second.block_id = b"other".to_vec();
    let (tx2, mut rx2) = oneshot::channel();
    assert!(state.begin_peer_import(scope, &second, tx2).is_none());
    assert!(matches!(
        rx2.try_recv().unwrap().unwrap_err().error,
        WorkerError::ResourceExhausted
    ));
    state.complete_peer_import(&spec.block_id, attempt, Ok(()));
    assert_eq!(state.peer_import_bytes, 0);
    let (tx3, _rx3) = oneshot::channel();
    assert!(state.begin_peer_import(scope, &second, tx3).is_some());
}

#[tokio::test]
async fn peer_import_task_panic_wakes_waiters_and_removes_prepare_pin() {
    let (mut state, session, spec) = setup();
    let scope = state.begin_read_scope(session).unwrap();
    let (tx, mut rx) = oneshot::channel();
    state.begin_peer_import(scope, &spec, tx).unwrap();
    let task = tokio::spawn(async {
        panic!("injected import task panic");
    });
    state.peer_imports.get_mut(&spec.block_id).unwrap().task_id = Some(task.id());
    let failed = task.await.unwrap_err();
    state.finish_read_scope(scope);
    state.fail_peer_import_task(failed.id());
    assert!(matches!(
        rx.try_recv().unwrap().unwrap_err().error,
        WorkerError::TransferUnavailable
    ));
    assert!(state.peer_imports.is_empty());
    assert!(state.peer_import_failures.is_empty());
    assert!(state.read_scopes_drained(scope));
    assert_eq!(state.peer_import_bytes, 0);
}

#[tokio::test]
async fn peer_import_owner_shutdown_cancels_child_without_waiting_for_network() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let service = crate::meta::metadata_service::MetadataServiceHandler::new(
        crate::meta::runtime::MetaHandle::spawn(),
    );
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(pb::metadata_service_server::MetadataServiceServer::new(
                service,
            ))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let metadata = MetadataClient::connect(&endpoint, 7, "http://127.0.0.1:19007".into(), None)
        .await
        .unwrap();
    let registry = dms_metrics::registry();
    let metrics = NodeMetrics::register(&registry).unwrap();
    let rpc_metrics = dms_metrics::RpcMetrics::register(&registry).unwrap();
    let (command_tx, command_rx) = mpsc::channel(NODE_MAILBOX_CAPACITY);
    let config = NodeTaskConfig {
        arena_capacity_bytes: 4096,
        region_size_bytes: 4096,
        staging_ttl: Duration::from_secs(30),
        client_cache_lease_ttl: CLIENT_CACHE_LEASE_TTL,
        node_current_cache_bytes: 4096,
        node_current_cache_ttl: Duration::from_secs(1),
        shared_fd_broker: None,
        log_level: LevelController::new(slog::Level::Info),
        trace_periodic_operations: false,
    };
    let owner = tokio::spawn(run_node(
        "shutdown-test".into(),
        Some(metadata.clone()),
        config,
        metrics.clone(),
        command_rx,
    ));
    let peer_channels = Arc::new(Mutex::new(HashMap::new()));
    let node = NodeHandle {
        command_tx,
        node_id: "shutdown-test".into(),
        metadata: Some(metadata),
        fd_broker_path: None,
        metrics,
        rpc_metrics,
        peer_channels: peer_channels.clone(),
    };
    let session = node.open_session(false).await.unwrap();
    let scope = ReadScopeGuard::begin(&node, session).await.unwrap();
    // 故意使连接获取永远 pending，验证 owner 取消会取消子任务，不能等外部网络释放。
    let _blocked_network = peer_channels.lock().await;
    let request_node = node.clone();
    let request =
        tokio::spawn(async move { request_node.ensure_peer_block(scope.id(), setup().2).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while node.debug_peer_imports().await.0 != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    owner.abort();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), owner)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    let error = tokio::time::timeout(Duration::from_secs(1), request)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(matches!(error.error, WorkerError::WorkerUnavailable));
    tokio::time::timeout(Duration::from_secs(1), async {
        while node.command_tx.strong_count() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("被取消的网络任务不能继续持有 NodeHandle");
    server.abort();
}

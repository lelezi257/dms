use afs::{
    meta::{
        Meta, rpc,
        store::{
            MetaEntity, MetaRead, MetaStore, RootRight as StoreRootRight, Store,
            local_file::LocalFileBackend, memory::MemoryBackend,
        },
    },
    runtime::Observability,
};
use afs_protocol::meta::{
    AbortRootRequest, AcquireRootRequest, ActivateRootReply, ActivateRootRequest,
    BeginSnapshotRequest, CommitVersionRequest, DraftKind, DraftRef, ListOwnerRootsRequest,
    LookupNodeRequest, LookupRootReply, LookupRootRequest, NodeDescriptor, NodeEndpoint,
    PresentedRootAccess, RecoverRootRequest, RegisterNodeRequest, ReplicaReceipt,
    ReserveDraftRequest, ReserveRootReply, ReserveRootRequest, RootAccess, RootCommand,
    RootCommandType, RootLocation, RootReservation, RootRight, SnapshotCut, SnapshotCutReceipt,
    ValidateRootAccessRequest, VersionRef, WatchRootCommandsRequest,
    blob_meta_server::BlobMeta as BlobMetaService, meta_server::Meta as MetaService,
    owner_roots_server::OwnerRoots as OwnerRootsService,
};
use std::sync::Arc;
use tonic::{Code, Request};

#[tokio::test]
async fn node_rpc_store_local_file_recovers_committed_session_and_reply() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(LocalFileBackend::open(dir.path()).unwrap());
    let store = Arc::new(Store::open(backend.clone()).await.unwrap());
    let meta = Arc::new(Meta::with_store(
        "meta-test".into(),
        Observability::new().unwrap(),
        store.clone(),
    ));
    let request = RegisterNodeRequest {
        request_id: "register-node-a".into(),
        node: Some(NodeDescriptor {
            node_id: "node-a".into(),
            endpoint: Some(NodeEndpoint {
                grpc_addr: "http://node-a:7400".into(),
                data_addr: "http://node-a:7500".into(),
                rest_addr: "http://node-a:7600".into(),
            }),
            labels: Default::default(),
            capabilities: Vec::new(),
            session_id: "session-a".into(),
        }),
        lease_seconds: 30,
    };
    let registered = rpc::MetaRpc(meta)
        .register_node(Request::new(request.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(registered.lease_epoch, 1);
    drop(store);

    let reopened = Arc::new(Meta::with_store(
        "meta-restarted".into(),
        Observability::new().unwrap(),
        Arc::new(Store::open(backend).await.unwrap()),
    ));
    let found = rpc::MetaRpc(reopened.clone())
        .lookup_node(Request::new(LookupNodeRequest {
            request_id: "lookup-node-a".into(),
            node_id: "node-a".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(found.found);
    assert_eq!(found.lease_epoch, 1);
    let replay = rpc::MetaRpc(reopened)
        .register_node(Request::new(request))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(replay.lease_epoch, 1);
}

#[test]
fn meta_services_share_one_package_but_remain_separate_routes() {
    use afs_protocol::meta::{
        blob_meta_server::BlobMetaServer, meta_server::MetaServer,
        owner_roots_server::OwnerRootsServer,
    };
    use tonic::server::NamedService;

    assert_eq!(
        <MetaServer<rpc::MetaRpc> as NamedService>::NAME,
        "afs.meta.v1.Meta"
    );
    assert_eq!(
        <OwnerRootsServer<rpc::OwnerRootsRpc> as NamedService>::NAME,
        "afs.meta.v1.OwnerRoots"
    );
    assert_eq!(
        <BlobMetaServer<rpc::BlobMetaRpc> as NamedService>::NAME,
        "afs.meta.v1.BlobMeta"
    );
}

#[test]
fn both_edges_can_reuse_the_same_meta_state() {
    let meta = Meta::new("meta-test".into(), Observability::new().unwrap());
    assert_eq!(meta.ping("node-a").unwrap(), "pong from meta-test");
    assert!(meta.ping(&"x".repeat(129)).is_err());
    let text = afs_metrics::encode_text(&meta.observability.registry).unwrap();
    assert!(text.contains("result=\"ok\"} 1"));
    assert!(text.contains("result=\"error\"} 1"));
}

#[tokio::test]
async fn contract_rpc_does_not_issue_fake_grants_without_store() {
    let meta = Arc::new(Meta::new("meta-test".into(), Observability::new().unwrap()));

    let register = rpc::MetaRpc(meta.clone())
        .register_node(Request::new(RegisterNodeRequest {
            request_id: "req-register".into(),
            node: Some(NodeDescriptor {
                node_id: "node-a".into(),
                endpoint: Some(NodeEndpoint {
                    grpc_addr: "http://node-a:7400".into(),
                    data_addr: "http://node-a:7500".into(),
                    rest_addr: "http://node-a:7600".into(),
                }),
                labels: Default::default(),
                capabilities: Vec::new(),
                session_id: "session-a".into(),
            }),
            lease_seconds: 30,
        }))
        .await
        .unwrap_err();
    assert_eq!(register.code(), Code::Unimplemented);

    let recover = rpc::OwnerRootsRpc(meta.clone())
        .recover_root(Request::new(RecoverRootRequest {
            request_id: "recover-a".into(),
            root_id: "workspace-a".into(),
            expected_root_epoch: 7,
            home_node_id: "node-a".into(),
            home_session_id: "session-a-new".into(),
            local_prepare_id: "local-a".into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(recover.code(), Code::Unimplemented);

    let validate = rpc::OwnerRootsRpc(meta.clone())
        .validate_root_access(Request::new(ValidateRootAccessRequest {
            request_id: "validate-b".into(),
            presented_access: Some(PresentedRootAccess {
                root_id: "workspace-a".into(),
                root_epoch: 1,
                home_node_id: "node-a".into(),
                holder_node_id: "node-b".into(),
                session_id: "session-b".into(),
                access_generation: 1,
                fencing_token: "fence-b".into(),
                home_session_id: "session-a".into(),
            }),
            observed_peer_node_id: "node-b".into(),
            validator_home_node_id: "node-a".into(),
            validator_home_session_id: "session-a".into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(validate.code(), Code::Unimplemented);

    let reserve_root = rpc::OwnerRootsRpc(meta.clone())
        .reserve_root(Request::new(ReserveRootRequest {
            request_id: "req-root".into(),
            root_id: "workspace-a".into(),
            preferred_home_node_id: "node-a".into(),
            session_id: "session-a".into(),
            create_intent_id: "mkdir-a".into(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(reserve_root.code(), Code::Unimplemented);

    let blob_rpc = rpc::BlobMetaRpc(meta);
    let reserve_draft = blob_rpc
        .reserve_draft(Request::new(ReserveDraftRequest {
            request_id: "req-draft".into(),
            namespace: "images".into(),
            object_id: "rootfs-a".into(),
            writer_node_id: "node-a".into(),
            session_id: "session-a".into(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(reserve_draft.code(), Code::Unimplemented);

    let begin_snapshot = blob_rpc
        .begin_snapshot(Request::new(BeginSnapshotRequest {
            request_id: "req-snapshot".into(),
            draft: None,
            expected_draft_generation: 1,
        }))
        .await
        .unwrap_err();
    assert_eq!(begin_snapshot.code(), Code::Unimplemented);

    let commit_version = blob_rpc
        .commit_version(Request::new(CommitVersionRequest {
            request_id: "req-commit".into(),
            cut: None,
            manifest_digest: "sha256:manifest".into(),
            logical_size: 4096,
            replica_receipts: Vec::new(),
            cut_receipt: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(commit_version.code(), Code::Unimplemented);
}

#[tokio::test]
async fn owner_authority_reserve_activate_acquire_validate_and_recover() {
    let store = Arc::new(
        Store::open(Arc::new(MemoryBackend::default()))
            .await
            .unwrap(),
    );
    let meta = Arc::new(Meta::with_store(
        "meta-test".into(),
        Observability::new().unwrap(),
        store.clone(),
    ));
    let meta_rpc = rpc::MetaRpc(meta.clone());
    let owner_rpc = rpc::OwnerRootsRpc(meta);

    for (node_id, session_id) in [("node-a", "session-a"), ("node-b", "session-b")] {
        let reply = meta_rpc
            .register_node(Request::new(RegisterNodeRequest {
                request_id: format!("register-{node_id}"),
                node: Some(NodeDescriptor {
                    node_id: node_id.into(),
                    endpoint: Some(NodeEndpoint {
                        grpc_addr: format!("http://{node_id}:7400"),
                        data_addr: format!("http://{node_id}:7500"),
                        rest_addr: format!("http://{node_id}:7600"),
                    }),
                    labels: Default::default(),
                    capabilities: vec!["ownerfs".into()],
                    session_id: session_id.into(),
                }),
                lease_seconds: 30,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(reply.node_id, node_id);
        assert_eq!(reply.lease_epoch, 1);
    }

    let reservation = owner_rpc
        .reserve_root(Request::new(ReserveRootRequest {
            request_id: "reserve-a".into(),
            root_id: "workspace-a".into(),
            preferred_home_node_id: "node-a".into(),
            session_id: "session-a".into(),
            rights: vec![RootRight::Lookup.into(), RootRight::Write.into()],
            expected_root_epoch: 1,
            parent_root_id: String::new(),
            create_intent_id: "mkdir-a".into(),
            conflict_policy: 0,
        }))
        .await
        .unwrap()
        .into_inner()
        .reservation
        .unwrap();
    assert_eq!(
        reservation.prepare_token,
        "prepare:workspace-a:session-a:mkdir-a:reserve-a"
    );

    let home_access = owner_rpc
        .activate_root(Request::new(ActivateRootRequest {
            request_id: "activate-a".into(),
            reservation: Some(reservation.clone()),
            local_prepare_id: "local-prepare-a".into(),
            parent_fsync_generation: 1,
            parent_fsync_complete: true,
        }))
        .await
        .unwrap()
        .into_inner()
        .access
        .unwrap();
    assert_eq!(home_access.holder_node_id, "node-a");
    assert_eq!(home_access.access_generation, 1);

    let location = owner_rpc
        .lookup_root(Request::new(LookupRootRequest {
            request_id: "lookup-a".into(),
            root_id: "workspace-a".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(location.found);
    assert_eq!(location.location.unwrap().home_node_id, "node-a");

    let listed = owner_rpc
        .list_owner_roots(Request::new(ListOwnerRootsRequest {
            request_id: "list-node-a".into(),
            home_node_id: "node-a".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.active_roots.len(), 1);
    assert_eq!(listed.active_roots[0].root_id, "workspace-a");
    assert!(listed.pending_reservations.is_empty());
    assert!(listed.authority_revision > 0);

    let remote_access = owner_rpc
        .acquire_root(Request::new(AcquireRootRequest {
            request_id: "acquire-b".into(),
            root_id: "workspace-a".into(),
            requester_node_id: "node-b".into(),
            session_id: "session-b".into(),
            rights: vec![RootRight::Lookup.into(), RootRight::Read.into()],
            expected_root_epoch: home_access.root_epoch,
            expected_access_generation: home_access.access_generation,
        }))
        .await
        .unwrap()
        .into_inner()
        .access
        .unwrap();
    assert_eq!(remote_access.home_node_id, "node-a");
    assert_eq!(remote_access.holder_node_id, "node-b");
    assert_eq!(
        remote_access.access_generation,
        home_access.access_generation
    );

    let widened_access = owner_rpc
        .acquire_root(Request::new(AcquireRootRequest {
            request_id: "acquire-b-write".into(),
            root_id: "workspace-a".into(),
            requester_node_id: "node-b".into(),
            session_id: "session-b".into(),
            rights: vec![RootRight::Write.into()],
            expected_root_epoch: home_access.root_epoch,
            expected_access_generation: home_access.access_generation,
        }))
        .await
        .unwrap()
        .into_inner()
        .access
        .unwrap();
    let widened_rights = widened_access
        .rights
        .iter()
        .filter_map(|right| RootRight::try_from(*right).ok())
        .collect::<Vec<_>>();
    assert!(widened_rights.contains(&RootRight::Lookup));
    assert!(widened_rights.contains(&RootRight::Read));
    assert!(widened_rights.contains(&RootRight::Write));

    let persisted_access = store
        .read(MetaRead::RootGrantByHolder {
            root_id: "workspace-a".into(),
            holder_node_id: "node-b".into(),
            holder_session_id: "session-b".into(),
        })
        .await
        .unwrap();
    let Some(MetaEntity::RootGrant(persisted_grant)) = persisted_access.entity else {
        panic!("node-b grant should persist");
    };
    assert!(persisted_grant.rights.contains(&StoreRootRight::Lookup));
    assert!(persisted_grant.rights.contains(&StoreRootRight::Read));
    assert!(persisted_grant.rights.contains(&StoreRootRight::Write));

    let home_node = meta_rpc
        .lookup_node(Request::new(LookupNodeRequest {
            request_id: "lookup-node-a".into(),
            node_id: remote_access.home_node_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(home_node.found);
    assert_eq!(
        home_node.node.unwrap().endpoint.unwrap().grpc_addr,
        "http://node-a:7400"
    );

    let bad_identity = owner_rpc
        .validate_root_access(Request::new(ValidateRootAccessRequest {
            request_id: "validate-bad".into(),
            presented_access: Some(PresentedRootAccess {
                root_id: remote_access.root_id.clone(),
                root_epoch: remote_access.root_epoch,
                home_node_id: remote_access.home_node_id.clone(),
                holder_node_id: remote_access.holder_node_id.clone(),
                session_id: remote_access.session_id.clone(),
                access_generation: remote_access.access_generation,
                fencing_token: remote_access.fencing_token.clone(),
                home_session_id: remote_access.home_session_id.clone(),
            }),
            observed_peer_node_id: "node-x".into(),
            validator_home_node_id: "node-a".into(),
            validator_home_session_id: "session-a".into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(bad_identity.code(), Code::InvalidArgument);

    let validated = owner_rpc
        .validate_root_access(Request::new(ValidateRootAccessRequest {
            request_id: "validate-b".into(),
            presented_access: Some(PresentedRootAccess {
                root_id: remote_access.root_id.clone(),
                root_epoch: remote_access.root_epoch,
                home_node_id: remote_access.home_node_id.clone(),
                holder_node_id: remote_access.holder_node_id.clone(),
                session_id: remote_access.session_id.clone(),
                access_generation: remote_access.access_generation,
                fencing_token: remote_access.fencing_token.clone(),
                home_session_id: remote_access.home_session_id.clone(),
            }),
            observed_peer_node_id: "node-b".into(),
            validator_home_node_id: "node-a".into(),
            validator_home_session_id: "session-a".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .access
        .unwrap();
    assert_eq!(validated.holder_node_id, "node-b");

    meta_rpc
        .register_node(Request::new(RegisterNodeRequest {
            request_id: "register-node-a-restart".into(),
            node: Some(NodeDescriptor {
                node_id: "node-a".into(),
                endpoint: Some(NodeEndpoint {
                    grpc_addr: "http://node-a:7400".into(),
                    data_addr: "http://node-a:7500".into(),
                    rest_addr: "http://node-a:7600".into(),
                }),
                labels: Default::default(),
                capabilities: vec!["ownerfs".into()],
                session_id: "session-a-2".into(),
            }),
            lease_seconds: 30,
        }))
        .await
        .unwrap();

    let recovered = owner_rpc
        .recover_root(Request::new(RecoverRootRequest {
            request_id: "recover-a".into(),
            root_id: "workspace-a".into(),
            expected_root_epoch: 1,
            home_node_id: "node-a".into(),
            home_session_id: "session-a-2".into(),
            local_prepare_id: "local-prepare-a".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .access
        .unwrap();
    assert_eq!(recovered.home_session_id, "session-a-2");
    assert_eq!(recovered.access_generation, 2);

    // B keeps its Node session across A's restart. Its old grant occupies the
    // same Meta key and must be replaced with one bound to A's new generation.
    let reacquired = owner_rpc
        .acquire_root(Request::new(AcquireRootRequest {
            request_id: "acquire-b-after-home-recover".into(),
            root_id: "workspace-a".into(),
            requester_node_id: "node-b".into(),
            session_id: "session-b".into(),
            rights: vec![RootRight::Lookup.into(), RootRight::Read.into()],
            expected_root_epoch: recovered.root_epoch,
            expected_access_generation: recovered.access_generation,
        }))
        .await
        .unwrap()
        .into_inner()
        .access
        .unwrap();
    assert_eq!(reacquired.home_session_id, "session-a-2");
    assert_eq!(reacquired.access_generation, 2);

    let old_remote_after_recover = store
        .read(MetaRead::RootGrant {
            root_id: remote_access.root_id,
            root_epoch: remote_access.root_epoch,
            home_session_id: remote_access.home_session_id,
            holder_node_id: remote_access.holder_node_id,
            holder_session_id: remote_access.session_id,
            access_generation: remote_access.access_generation,
            fencing_token: remote_access.fencing_token,
        })
        .await
        .unwrap();
    assert!(old_remote_after_recover.entity.is_none());
}

#[tokio::test]
async fn owner_authority_stale_abort_cannot_delete_new_pending_reservation() {
    let store = Arc::new(
        Store::open(Arc::new(MemoryBackend::default()))
            .await
            .unwrap(),
    );
    let meta = Arc::new(Meta::with_store(
        "meta-test".into(),
        Observability::new().unwrap(),
        store,
    ));
    let meta_rpc = rpc::MetaRpc(meta.clone());
    let owner_rpc = rpc::OwnerRootsRpc(meta);

    meta_rpc
        .register_node(Request::new(RegisterNodeRequest {
            request_id: "register-node-a".into(),
            node: Some(NodeDescriptor {
                node_id: "node-a".into(),
                endpoint: Some(NodeEndpoint {
                    grpc_addr: "http://node-a:7400".into(),
                    data_addr: "http://node-a:7500".into(),
                    rest_addr: "http://node-a:7600".into(),
                }),
                labels: Default::default(),
                capabilities: vec!["ownerfs".into()],
                session_id: "session-a".into(),
            }),
            lease_seconds: 30,
        }))
        .await
        .unwrap();

    let old = owner_rpc
        .reserve_root(Request::new(ReserveRootRequest {
            request_id: "reserve-old".into(),
            root_id: "workspace-race".into(),
            preferred_home_node_id: "node-a".into(),
            session_id: "session-a".into(),
            expected_root_epoch: 1,
            create_intent_id: "mkdir-old".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .reservation
        .unwrap();

    owner_rpc
        .abort_root(Request::new(AbortRootRequest {
            request_id: "abort-old".into(),
            root_id: old.root_id.clone(),
            root_epoch: old.root_epoch,
            session_id: old.session_id.clone(),
            create_intent_id: old.create_intent_id.clone(),
            prepare_token: old.prepare_token.clone(),
            reason: "local prepare failed".into(),
        }))
        .await
        .unwrap();

    let new = owner_rpc
        .reserve_root(Request::new(ReserveRootRequest {
            request_id: "reserve-new".into(),
            root_id: "workspace-race".into(),
            preferred_home_node_id: "node-a".into(),
            session_id: "session-a".into(),
            expected_root_epoch: 1,
            create_intent_id: "mkdir-new".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .reservation
        .unwrap();
    assert_ne!(old.prepare_token, new.prepare_token);

    let stale_abort = owner_rpc
        .abort_root(Request::new(AbortRootRequest {
            request_id: "abort-stale-old".into(),
            root_id: old.root_id,
            root_epoch: old.root_epoch,
            session_id: old.session_id,
            create_intent_id: old.create_intent_id,
            prepare_token: old.prepare_token,
            reason: "late retry from old prepare".into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(stale_abort.code(), Code::InvalidArgument);

    let listed = owner_rpc
        .list_owner_roots(Request::new(ListOwnerRootsRequest {
            request_id: "list-after-stale-abort".into(),
            home_node_id: "node-a".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(listed.active_roots.is_empty());
    assert_eq!(listed.pending_reservations.len(), 1);
    assert_eq!(
        listed.pending_reservations[0].prepare_token,
        new.prepare_token
    );
}

#[test]
fn owner_root_messages_keep_location_reservation_and_access_separate() {
    let location = RootLocation {
        root_id: "workspace-a".into(),
        root_epoch: 7,
        home_node_id: "node-a".into(),
        home_session_id: "session-a".into(),
    };
    let lookup = LookupRootReply {
        location: Some(location),
        found: true,
    };
    assert_eq!(lookup.location.unwrap().home_node_id, "node-a");

    let reservation = RootReservation {
        root_id: "workspace-a".into(),
        root_epoch: 7,
        home_node_id: "node-a".into(),
        session_id: "session-a".into(),
        create_intent_id: "intent-a".into(),
        prepare_token: "prepare-a".into(),
    };
    let reserve = ReserveRootReply {
        reservation: Some(reservation.clone()),
        already_reserved_by_same_intent: false,
    };
    assert_eq!(reserve.reservation.unwrap().prepare_token, "prepare-a");

    let access = RootAccess {
        root_id: reservation.root_id,
        root_epoch: reservation.root_epoch,
        home_node_id: reservation.home_node_id,
        holder_node_id: "node-b".into(),
        session_id: "session-b".into(),
        access_generation: 8,
        rights: vec![RootRight::Read.into(), RootRight::Write.into()],
        fencing_token: "fence-b".into(),
        home_session_id: "session-a".into(),
    };
    let activate = ActivateRootReply {
        access: Some(access),
    };
    let access = activate.access.unwrap();
    assert_eq!(access.home_node_id, "node-a");
    assert_eq!(access.holder_node_id, "node-b");
    assert_eq!(access.session_id, "session-b");

    let presented = PresentedRootAccess {
        root_id: access.root_id.clone(),
        root_epoch: access.root_epoch,
        home_node_id: access.home_node_id.clone(),
        holder_node_id: access.holder_node_id.clone(),
        session_id: access.session_id.clone(),
        access_generation: access.access_generation,
        fencing_token: access.fencing_token.clone(),
        home_session_id: access.home_session_id.clone(),
    };
    assert_eq!(presented.holder_node_id, "node-b");
    assert_eq!(presented.fencing_token, "fence-b");

    let watch = WatchRootCommandsRequest {
        node_id: "node-b".into(),
        session_id: "session-b".into(),
        after_revision: 41,
    };
    assert_eq!(watch.after_revision, 41);

    let command = RootCommand {
        command_id: "cmd-42".into(),
        command_type: RootCommandType::RevokeAccess.into(),
        access: Some(access),
        revision: 42,
    };
    assert_eq!(command.revision, 42);
}

#[test]
fn blob_messages_keep_snapshot_cut_and_version_commit_separate() {
    let draft = DraftRef {
        namespace: "images".into(),
        object_id: "rootfs-a".into(),
        draft_id: "draft-a".into(),
        writer_node_id: "node-a".into(),
        session_id: "session-a".into(),
        draft_epoch: 3,
        kind: DraftKind::ContainerRootfs.into(),
    };
    let cut = SnapshotCut {
        draft: Some(draft),
        snapshot_operation_id: "snapshot-op-a".into(),
        draft_generation: 11,
        cut_token: "cut-a".into(),
    };
    assert_eq!(cut.draft_generation, 11);

    let receipt = ReplicaReceipt {
        node_id: "node-b".into(),
        receipt_id: "receipt-b".into(),
        content_digest: "sha256:blob".into(),
        durable_bytes: 4096,
    };
    let commit = CommitVersionRequest {
        request_id: "commit-a".into(),
        cut: Some(cut),
        manifest_digest: "sha256:manifest".into(),
        logical_size: 4096,
        replica_receipts: vec![receipt],
        cut_receipt: Some(SnapshotCutReceipt {
            snapshot_operation_id: "snapshot-op-a".into(),
            writer_node_id: "node-a".into(),
            session_id: "session-a".into(),
            capture_id: "capture-a".into(),
            recovery_record_digest: "sha256:recovery".into(),
        }),
    };
    assert_eq!(commit.replica_receipts.len(), 1);
    assert_eq!(commit.cut_receipt.as_ref().unwrap().capture_id, "capture-a");

    let version = VersionRef {
        namespace: "images".into(),
        object_id: "rootfs-a".into(),
        version_id: "version-a".into(),
        manifest_digest: commit.manifest_digest,
        created_at_unix_ms: 123,
    };
    assert_eq!(version.version_id, "version-a");
}

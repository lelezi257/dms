//! Generated gRPC adapter around the same Meta authority used by REST.
//!
//! OwnerRoots tonic methods authenticate and translate protobuf/domain types;
//! their business transitions live in `owner_roots.rs` and `Meta`; RPC does
//! not own authority state or decide durable commits.
//! Without a store, authority RPCs fail closed instead of fabricating a grant.

use afs_protocol::meta::{
    AbortRootReply, AbortRootRequest, AckRevocationReply, AckRevocationRequest, AcquireRootReply,
    AcquireRootRequest, ActivateRootReply, ActivateRootRequest, CommitFileVersionReply,
    CommitFileVersionRequest, DfsCommitMetadataMode, DfsCreateReply, DfsCreateRequest,
    DfsInodeAttributes as PbDfsInodeAttributes, DfsInodeKind as PbDfsInodeKind,
    DfsInodeRecord as PbDfsInodeRecord, DfsLayoutRoot as PbDfsLayoutRoot, DfsLookupReply,
    DfsLookupRequest, DfsWriteLeaseReply, GetDfsChunkSourcesReply, GetDfsChunkSourcesRequest,
    GetDfsInodeReply, GetDfsInodeRequest, GetDfsPlacementSnapshotReply,
    GetDfsPlacementSnapshotRequest, GetFileVersionReply, GetFileVersionRequest,
    ListOwnerRootsReply, ListOwnerRootsRequest, LookupNodeReply, LookupNodeRequest,
    LookupRootReply, LookupRootRequest, NodeDescriptor, NodeEndpoint, OpenDfsWriteReply,
    OpenDfsWriteRequest, PingReply, PingRequest, PresentedRootAccess, RecoverRootReply,
    RecoverRootRequest, RegisterNodeReply, RegisterNodeRequest, RenewDfsWriteLeaseRequest,
    ReserveRootReply, ReserveRootRequest, RootCommand, RootCommandType, RootLocation,
    RootReservation, RootRight as PbRootRight, SyncDfsInodeMetadataReply,
    SyncDfsInodeMetadataRequest, ValidateRootAccessReply, ValidateRootAccessRequest,
    WatchRootCommandsRequest, dfs_meta_server::DfsMeta as DfsMetaService,
    meta_server::Meta as MetaService, owner_roots_server::OwnerRoots as OwnerRootsService,
};
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use super::dfs::CreateFileRequest;
use super::store::{
    NodeSessionLease, RequestKey, RootAccessGrant, RootCommandRecord, RootReservationRecord,
    RootRight, StoreRevision,
};

pub struct MetaRpc(pub Arc<super::Meta>);
pub struct OwnerRootsRpc(pub Arc<super::Meta>);
pub struct DfsMetaRpc(pub Arc<super::Meta>);

type RootCommandStream = Pin<Box<dyn Stream<Item = Result<RootCommand, Status>> + Send + 'static>>;

fn invalid(message: impl Into<String>) -> Status {
    afs_transport::grpc::error_status::error_to_status(afs_error::Error::coded(
        afs_error::META_CATALOG_INVALID_REQUEST,
        message,
    ))
}

fn permission_denied(message: impl Into<String>) -> Status {
    afs_transport::grpc::error_status::error_to_status(afs_error::Error::coded(
        afs_error::CLIENT_PERMISSION_DENIED,
        message,
    ))
}

fn authenticated_node_id<T>(
    meta: &super::Meta,
    request: &Request<T>,
) -> Result<Option<String>, Status> {
    if !meta.enforce_peer_identity {
        return Ok(None);
    }
    let certs = request.peer_certs().ok_or_else(|| {
        permission_denied("mTLS peer certificate is required for Meta authority RPC")
    })?;
    let leaf = certs.first().ok_or_else(|| {
        permission_denied("mTLS peer certificate chain did not contain a leaf certificate")
    })?;
    let node_id = meta
        .trusted_nodes_by_der
        .get(leaf.as_ref())
        .ok_or_else(|| permission_denied("mTLS peer certificate is not trusted for any node"))?;
    Ok(Some(node_id.clone()))
}

fn validate_root_access_identities(
    authenticated_meta_node_id: Option<&str>,
    observed_peer_node_id: &str,
    validator_home_node_id: &str,
    validator_home_session_id: &str,
    presented: &PresentedRootAccess,
) -> Result<(), Status> {
    if let Some(authenticated) = authenticated_meta_node_id
        && authenticated != validator_home_node_id
    {
        return Err(permission_denied(format!(
            "authenticated validator {authenticated} does not match validator_home_node_id {validator_home_node_id}",
        )));
    }
    if observed_peer_node_id != presented.holder_node_id {
        return Err(invalid(
            "observed peer node id does not match presented grant holder",
        ));
    }
    if validator_home_node_id != presented.home_node_id
        || validator_home_session_id != presented.home_session_id
    {
        return Err(invalid("validator is not the presented Home session"));
    }
    Ok(())
}

fn request_key(caller_id: impl Into<String>, request_id: impl Into<String>) -> RequestKey {
    RequestKey::new(caller_id, request_id)
}

fn require_text(value: &str, field: &'static str) -> Result<(), Status> {
    if value.is_empty() {
        Err(invalid(format!("{field} is required")))
    } else {
        Ok(())
    }
}

fn validate_caller(authenticated: Option<&str>, caller_id: &str) -> Result<(), Status> {
    require_text(caller_id, "caller_id")?;
    if let Some(authenticated) = authenticated
        && authenticated != caller_id
    {
        return Err(permission_denied(format!(
            "authenticated node {authenticated} does not match caller_id {caller_id}",
        )));
    }
    Ok(())
}

fn domain_rights(rights: &[i32]) -> Result<Vec<RootRight>, Status> {
    let mut out = Vec::new();
    for right in rights {
        let right = PbRootRight::try_from(*right).map_err(|_| invalid("unknown root right"))?;
        let right = match right {
            PbRootRight::Unspecified => continue,
            PbRootRight::Lookup => RootRight::Lookup,
            PbRootRight::Read => RootRight::Read,
            PbRootRight::Write => RootRight::Write,
            PbRootRight::Admin => RootRight::Admin,
        };
        if !out.contains(&right) {
            out.push(right);
        }
    }
    if out.is_empty() {
        out.extend([RootRight::Lookup, RootRight::Read, RootRight::Write]);
    }
    Ok(out)
}

fn wire_rights(rights: &[RootRight]) -> Vec<i32> {
    rights
        .iter()
        .map(|right| match right {
            RootRight::Lookup => PbRootRight::Lookup.into(),
            RootRight::Read => PbRootRight::Read.into(),
            RootRight::Write => PbRootRight::Write.into(),
            RootRight::Admin => PbRootRight::Admin.into(),
        })
        .collect()
}

fn wire_access(grant: RootAccessGrant) -> afs_protocol::meta::RootAccess {
    afs_protocol::meta::RootAccess {
        root_id: grant.root_id,
        root_epoch: grant.root_epoch,
        home_node_id: grant.home_node_id,
        holder_node_id: grant.holder_node_id,
        session_id: grant.holder_session_id,
        access_generation: grant.access_generation,
        rights: wire_rights(&grant.rights),
        fencing_token: grant.fencing_token,
        home_session_id: grant.home_session_id,
    }
}

fn wire_location(root: super::store::RootRecord) -> RootLocation {
    RootLocation {
        root_id: root.root_id,
        root_epoch: root.root_epoch,
        home_node_id: root.home_node_id,
        home_session_id: root.home_session_id,
    }
}

fn wire_node(session: super::store::NodeSession) -> NodeDescriptor {
    NodeDescriptor {
        node_id: session.node_id,
        endpoint: Some(NodeEndpoint {
            grpc_addr: session.grpc_addr,
            data_addr: session.data_addr,
            rest_addr: session.rest_addr,
        }),
        labels: Default::default(),
        capabilities: Vec::new(),
        session_id: session.session_id,
        storage_devices: session
            .storage_devices
            .into_iter()
            .map(|device| afs_protocol::meta::DfsStorageDevice {
                device_id: device.device_id,
                device_epoch: device.device_epoch,
                catalog_revision: device.catalog_revision,
                failure_domain: device.failure_domain,
            })
            .collect(),
    }
}

fn wire_reservation(reservation: RootReservationRecord) -> RootReservation {
    RootReservation {
        root_id: reservation.root_id,
        root_epoch: reservation.root_epoch,
        home_node_id: reservation.home_node_id,
        session_id: reservation.home_session_id,
        create_intent_id: reservation.create_intent_id,
        prepare_token: reservation.prepare_token,
    }
}

fn command_from_record(record: RootCommandRecord, revision: StoreRevision) -> RootCommand {
    let access = RootAccessGrant {
        root_id: record.root_id,
        root_epoch: record.root_epoch,
        home_node_id: record.home_node_id,
        home_session_id: record.home_session_id,
        holder_node_id: String::new(),
        holder_session_id: String::new(),
        access_generation: record.old_access_generation,
        rights: Vec::new(),
        fencing_token: String::new(),
        issued_at_revision: revision,
    };
    RootCommand {
        command_id: record.command_id,
        command_type: RootCommandType::RevokeAccess.into(),
        access: Some(wire_access(access)),
        revision: revision.0,
    }
}

#[tonic::async_trait]
impl MetaService for MetaRpc {
    async fn ping(&self, request: Request<PingRequest>) -> Result<Response<PingReply>, Status> {
        let span = afs_tracing::tracing::info_span!("meta.ping");
        let _entered = span.enter();
        let message = self
            .0
            .ping(&request.into_inner().node_id)
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(PingReply {
            message,
            instance: self.0.id.clone(),
        }))
    }

    async fn register_node(
        &self,
        request: Request<RegisterNodeRequest>,
    ) -> Result<Response<RegisterNodeReply>, Status> {
        self.0.observability.record("meta", "register_node", true);
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        let node = request
            .node
            .ok_or_else(|| invalid("node descriptor is required"))?;
        require_text(&request.request_id, "request_id")?;
        require_text(&node.node_id, "node.node_id")?;
        if let Some(authenticated) = authenticated
            && authenticated != node.node_id
        {
            return Err(permission_denied(format!(
                "authenticated node {authenticated} does not match node.node_id {}",
                node.node_id
            )));
        }
        require_text(&node.session_id, "node.session_id")?;
        let endpoint = node
            .endpoint
            .ok_or_else(|| invalid("node.endpoint is required"))?;
        let ttl = Duration::from_secs(request.lease_seconds.max(1));
        let session = self
            .0
            .register_node(
                request_key(
                    format!("{}/{}", node.node_id, node.session_id),
                    request.request_id,
                ),
                NodeSessionLease {
                    node_id: node.node_id,
                    session_id: node.session_id,
                    grpc_addr: endpoint.grpc_addr,
                    data_addr: endpoint.data_addr,
                    rest_addr: endpoint.rest_addr,
                    storage_devices: node
                        .storage_devices
                        .into_iter()
                        .map(|device| crate::dfs::StorageDeviceDescriptor {
                            device_id: device.device_id,
                            device_epoch: device.device_epoch,
                            catalog_revision: device.catalog_revision,
                            failure_domain: device.failure_domain,
                        })
                        .collect(),
                    lease_ttl: ttl,
                },
            )
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(RegisterNodeReply {
            node_id: session.node_id,
            lease_epoch: session.lease_epoch,
            expires_at_unix_ms: session.expires_at_unix_ms,
        }))
    }

    async fn lookup_node(
        &self,
        request: Request<LookupNodeRequest>,
    ) -> Result<Response<LookupNodeReply>, Status> {
        let request = request.into_inner();
        require_text(&request.node_id, "node_id")?;
        let session = self
            .0
            .lookup_node(&request.node_id)
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        let (node, lease_epoch, expires_at_unix_ms) = match session {
            Some(session) => {
                let lease_epoch = session.lease_epoch;
                let expires_at_unix_ms = session.expires_at_unix_ms;
                (Some(wire_node(session)), lease_epoch, expires_at_unix_ms)
            }
            _ => (None, 0, 0),
        };
        Ok(Response::new(LookupNodeReply {
            node,
            lease_epoch,
            expires_at_unix_ms,
            found: lease_epoch != 0,
        }))
    }
}

#[tonic::async_trait]
impl OwnerRootsService for OwnerRootsRpc {
    type WatchRootCommandsStream = RootCommandStream;

    async fn reserve_root(
        &self,
        request: Request<ReserveRootRequest>,
    ) -> Result<Response<ReserveRootReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        require_text(&request.request_id, "request_id")?;
        require_text(&request.root_id, "root_id")?;
        require_text(&request.preferred_home_node_id, "preferred_home_node_id")?;
        if let Some(authenticated) = authenticated
            && authenticated != request.preferred_home_node_id
        {
            return Err(permission_denied(format!(
                "authenticated node {authenticated} does not match preferred_home_node_id {}",
                request.preferred_home_node_id
            )));
        }
        require_text(&request.session_id, "session_id")?;
        require_text(&request.create_intent_id, "create_intent_id")?;
        let rights = domain_rights(&request.rights)?;
        let output = self
            .0
            .owner_roots
            .reserve_root(super::owner_roots::ReserveRootInput {
                request_id: request.request_id,
                root_id: request.root_id,
                preferred_home_node_id: request.preferred_home_node_id,
                session_id: request.session_id,
                rights,
                expected_root_epoch: request.expected_root_epoch,
                create_intent_id: request.create_intent_id,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(ReserveRootReply {
            reservation: Some(wire_reservation(output.reservation)),
            already_reserved_by_same_intent: output.already_reserved_by_same_intent,
        }))
    }

    async fn activate_root(
        &self,
        request: Request<ActivateRootRequest>,
    ) -> Result<Response<ActivateRootReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        require_text(&request.request_id, "request_id")?;
        require_text(&request.local_prepare_id, "local_prepare_id")?;
        if !request.parent_fsync_complete {
            return Err(invalid("parent_fsync_complete is required"));
        }
        let reservation = request
            .reservation
            .ok_or_else(|| invalid("reservation is required"))?;
        require_text(&reservation.root_id, "reservation.root_id")?;
        require_text(&reservation.home_node_id, "reservation.home_node_id")?;
        require_text(&reservation.session_id, "reservation.session_id")?;
        if let Some(authenticated) = authenticated
            && authenticated != reservation.home_node_id
        {
            return Err(permission_denied(format!(
                "authenticated node {authenticated} does not match reservation.home_node_id {}",
                reservation.home_node_id
            )));
        }
        let grant = self
            .0
            .owner_roots
            .activate_root(super::owner_roots::ActivateRootInput {
                request_id: request.request_id,
                reservation: super::owner_roots::WireRootReservation {
                    root_id: reservation.root_id,
                    root_epoch: reservation.root_epoch,
                    home_node_id: reservation.home_node_id,
                    session_id: reservation.session_id,
                    create_intent_id: reservation.create_intent_id,
                    prepare_token: reservation.prepare_token,
                },
                local_prepare_id: request.local_prepare_id,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(ActivateRootReply {
            access: Some(wire_access(grant)),
        }))
    }

    async fn abort_root(
        &self,
        request: Request<AbortRootRequest>,
    ) -> Result<Response<AbortRootReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        require_text(&request.request_id, "request_id")?;
        require_text(&request.root_id, "root_id")?;
        require_text(&request.session_id, "session_id")?;
        require_text(&request.create_intent_id, "create_intent_id")?;
        require_text(&request.prepare_token, "prepare_token")?;
        self.0
            .owner_roots
            .abort_root(super::owner_roots::AbortRootInput {
                request_id: request.request_id,
                root_id: request.root_id,
                root_epoch: request.root_epoch,
                session_id: request.session_id,
                create_intent_id: request.create_intent_id,
                prepare_token: request.prepare_token,
                authenticated_home_node_id: authenticated,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(AbortRootReply {}))
    }

    async fn lookup_root(
        &self,
        request: Request<LookupRootRequest>,
    ) -> Result<Response<LookupRootReply>, Status> {
        let request = request.into_inner();
        require_text(&request.root_id, "root_id")?;
        let location = self
            .0
            .owner_roots
            .lookup_root(super::owner_roots::LookupRootInput {
                root_id: request.root_id,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?
            .map(wire_location);
        Ok(Response::new(LookupRootReply {
            found: location.is_some(),
            location,
        }))
    }

    async fn list_owner_roots(
        &self,
        request: Request<ListOwnerRootsRequest>,
    ) -> Result<Response<ListOwnerRootsReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        require_text(&request.request_id, "request_id")?;
        require_text(&request.home_node_id, "home_node_id")?;
        if let Some(authenticated) = authenticated
            && authenticated != request.home_node_id
        {
            return Err(permission_denied(format!(
                "authenticated node {authenticated} does not match home_node_id {}",
                request.home_node_id
            )));
        }
        let output = self
            .0
            .owner_roots
            .list_owner_roots(super::owner_roots::ListOwnerRootsInput {
                home_node_id: request.home_node_id,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(ListOwnerRootsReply {
            active_roots: output.active_roots.into_iter().map(wire_location).collect(),
            pending_reservations: output
                .pending_reservations
                .into_iter()
                .map(wire_reservation)
                .collect(),
            authority_revision: output.authority_revision.0,
        }))
    }

    async fn acquire_root(
        &self,
        request: Request<AcquireRootRequest>,
    ) -> Result<Response<AcquireRootReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        require_text(&request.request_id, "request_id")?;
        require_text(&request.root_id, "root_id")?;
        require_text(&request.requester_node_id, "requester_node_id")?;
        if let Some(authenticated) = authenticated
            && authenticated != request.requester_node_id
        {
            return Err(permission_denied(format!(
                "authenticated node {authenticated} does not match requester_node_id {}",
                request.requester_node_id
            )));
        }
        require_text(&request.session_id, "session_id")?;
        let requested_rights = domain_rights(&request.rights)?;
        let grant = self
            .0
            .owner_roots
            .acquire_root(super::owner_roots::AcquireRootInput {
                request_id: request.request_id,
                root_id: request.root_id,
                requester_node_id: request.requester_node_id,
                session_id: request.session_id,
                rights: requested_rights,
                expected_root_epoch: request.expected_root_epoch,
                expected_access_generation: request.expected_access_generation,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(AcquireRootReply {
            access: Some(wire_access(grant)),
        }))
    }

    async fn validate_root_access(
        &self,
        request: Request<ValidateRootAccessRequest>,
    ) -> Result<Response<ValidateRootAccessReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        let presented = request
            .presented_access
            .ok_or_else(|| invalid("presented_access is required"))?;
        validate_root_access_identities(
            authenticated.as_deref(),
            &request.observed_peer_node_id,
            &request.validator_home_node_id,
            &request.validator_home_session_id,
            &presented,
        )?;
        let output = self
            .0
            .owner_roots
            .validate_root_access(super::owner_roots::ValidateRootAccessInput {
                root_id: presented.root_id,
                root_epoch: presented.root_epoch,
                home_session_id: presented.home_session_id,
                holder_node_id: presented.holder_node_id,
                holder_session_id: presented.session_id,
                access_generation: presented.access_generation,
                fencing_token: presented.fencing_token,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(ValidateRootAccessReply {
            access: Some(wire_access(output.grant)),
            authority_revision: output.authority_revision.0,
        }))
    }

    async fn watch_root_commands(
        &self,
        request: Request<WatchRootCommandsRequest>,
    ) -> Result<Response<Self::WatchRootCommandsStream>, Status> {
        let request = request.into_inner();
        require_text(&request.node_id, "node_id")?;
        require_text(&request.session_id, "session_id")?;
        let commands = self
            .0
            .owner_roots
            .watch_root_commands(super::owner_roots::WatchRootCommandsInput {
                node_id: request.node_id,
                session_id: request.session_id,
                after_revision: request.after_revision,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        let commands = commands
            .into_iter()
            .map(|command| Ok(command_from_record(command.command, command.revision)))
            .collect::<Vec<_>>();
        Ok(Response::new(Box::pin(tokio_stream::iter(commands))))
    }

    async fn ack_revocation(
        &self,
        request: Request<AckRevocationRequest>,
    ) -> Result<Response<AckRevocationReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        require_text(&request.request_id, "request_id")?;
        require_text(&request.command_id, "command_id")?;
        require_text(&request.node_id, "node_id")?;
        require_text(&request.session_id, "session_id")?;
        require_text(&request.root_id, "root_id")?;
        if let Some(authenticated) = authenticated
            && authenticated != request.node_id
        {
            return Err(permission_denied(format!(
                "authenticated node {authenticated} does not match node_id {}",
                request.node_id
            )));
        }
        let accepted_at_unix_ms = self
            .0
            .owner_roots
            .ack_revocation(super::owner_roots::AckRevocationInput {
                request_id: request.request_id,
                command_id: request.command_id,
                node_id: request.node_id,
                session_id: request.session_id,
                root_id: request.root_id,
                root_epoch: request.root_epoch,
                access_generation: request.access_generation,
                success: request.success,
                message: request.message,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(AckRevocationReply {
            accepted_at_unix_ms,
        }))
    }

    async fn recover_root(
        &self,
        request: Request<RecoverRootRequest>,
    ) -> Result<Response<RecoverRootReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        require_text(&request.request_id, "request_id")?;
        require_text(&request.root_id, "root_id")?;
        require_text(&request.home_node_id, "home_node_id")?;
        if let Some(authenticated) = authenticated
            && authenticated != request.home_node_id
        {
            return Err(permission_denied(format!(
                "authenticated node {authenticated} does not match home_node_id {}",
                request.home_node_id
            )));
        }
        require_text(&request.home_session_id, "home_session_id")?;
        require_text(&request.local_prepare_id, "local_prepare_id")?;
        let grant = self
            .0
            .owner_roots
            .recover_root(super::owner_roots::RecoverRootInput {
                request_id: request.request_id,
                root_id: request.root_id,
                expected_root_epoch: request.expected_root_epoch,
                home_node_id: request.home_node_id,
                home_session_id: request.home_session_id,
                local_prepare_id: request.local_prepare_id,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(RecoverRootReply {
            access: Some(wire_access(grant)),
        }))
    }
}

#[tonic::async_trait]
impl DfsMetaService for DfsMetaRpc {
    async fn lookup(
        &self,
        request: Request<DfsLookupRequest>,
    ) -> Result<Response<DfsLookupReply>, Status> {
        let request = request.into_inner();
        require_text(&request.namespace_id, "namespace_id")?;
        require_text(&request.parent_inode_id, "parent_inode_id")?;
        let inode = dfs_service(&self.0)?
            .lookup(
                crate::dfs::NamespaceId::new(request.namespace_id),
                crate::dfs::InodeId::new(request.parent_inode_id),
                request.name,
            )
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(DfsLookupReply {
            found: inode.is_some(),
            inode: inode.map(wire_dfs_inode),
        }))
    }

    async fn create(
        &self,
        request: Request<DfsCreateRequest>,
    ) -> Result<Response<DfsCreateReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        validate_caller(authenticated.as_deref(), &request.caller_id)?;
        require_text(&request.operation_id, "operation_id")?;
        require_text(&request.namespace_id, "namespace_id")?;
        require_text(&request.parent_inode_id, "parent_inode_id")?;
        require_text(&request.owner_session_id, "owner_session_id")?;
        let attributes = request
            .attributes
            .ok_or_else(|| invalid("DFS create requires attributes"))?;
        let (inode, lease) = dfs_service(&self.0)?
            .create(CreateFileRequest {
                caller_id: request.caller_id,
                owner_session_id: request.owner_session_id,
                operation_id: crate::dfs::OperationId::new(request.operation_id),
                namespace_id: crate::dfs::NamespaceId::new(request.namespace_id),
                parent_inode_id: crate::dfs::InodeId::new(request.parent_inode_id),
                name: request.name,
                attributes: domain_dfs_attributes(attributes),
                lease_seconds: request.lease_seconds,
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(DfsCreateReply {
            inode: Some(wire_dfs_inode(inode)),
            write_lease: Some(wire_dfs_write_lease(lease)),
        }))
    }

    async fn open_write(
        &self,
        request: Request<OpenDfsWriteRequest>,
    ) -> Result<Response<OpenDfsWriteReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        validate_caller(authenticated.as_deref(), &request.caller_id)?;
        require_text(&request.owner_session_id, "owner_session_id")?;
        require_text(&request.operation_id, "operation_id")?;
        require_text(&request.inode_id, "inode_id")?;
        let (inode, lease) = dfs_service(&self.0)?
            .open_write(
                request.caller_id,
                request.owner_session_id,
                crate::dfs::OperationId::new(request.operation_id),
                crate::dfs::InodeId::new(request.inode_id),
                request.lease_seconds,
            )
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(OpenDfsWriteReply {
            inode: Some(wire_dfs_inode(inode)),
            write_lease: Some(wire_dfs_write_lease(lease)),
        }))
    }

    async fn renew_write_lease(
        &self,
        request: Request<RenewDfsWriteLeaseRequest>,
    ) -> Result<Response<DfsWriteLeaseReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        validate_caller(authenticated.as_deref(), &request.caller_id)?;
        require_text(&request.owner_session_id, "owner_session_id")?;
        require_text(&request.operation_id, "operation_id")?;
        let current = domain_dfs_write_lease(
            request
                .current_lease
                .ok_or_else(|| invalid("DFS renew requires current lease"))?,
        );
        let lease = dfs_service(&self.0)?
            .renew_write_lease(
                request.caller_id,
                request.owner_session_id,
                crate::dfs::OperationId::new(request.operation_id),
                current,
                request.lease_seconds,
            )
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(DfsWriteLeaseReply {
            write_lease: Some(wire_dfs_write_lease(lease)),
        }))
    }

    async fn get_inode(
        &self,
        request: Request<GetDfsInodeRequest>,
    ) -> Result<Response<GetDfsInodeReply>, Status> {
        require_text(&request.get_ref().inode_id, "inode_id")?;
        let inode = dfs_service(&self.0)?
            .get_inode(crate::dfs::InodeId::new(request.into_inner().inode_id))
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(GetDfsInodeReply {
            inode: Some(wire_dfs_inode(inode)),
        }))
    }

    async fn get_file_version(
        &self,
        request: Request<GetFileVersionRequest>,
    ) -> Result<Response<GetFileVersionReply>, Status> {
        require_text(&request.get_ref().version_id, "version_id")?;
        let (version, layout) = dfs_service(&self.0)?
            .get_file_version(crate::dfs::FileVersionId::new(
                request.into_inner().version_id,
            ))
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(GetFileVersionReply {
            version: Some(wire_dfs_version(version)),
            layout: Some(wire_dfs_layout(layout)),
        }))
    }

    async fn get_placement_snapshot(
        &self,
        request: Request<GetDfsPlacementSnapshotRequest>,
    ) -> Result<Response<GetDfsPlacementSnapshotReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        validate_caller(authenticated.as_deref(), &request.caller_id)?;
        let snapshot = dfs_service(&self.0)?
            .placement_snapshot(request.caller_id)
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        if snapshot.revision < request.minimum_revision {
            return Err(invalid("placement snapshot is older than minimum_revision"));
        }
        Ok(Response::new(GetDfsPlacementSnapshotReply {
            snapshot: Some(wire_placement_snapshot(snapshot)),
        }))
    }

    async fn get_chunk_sources(
        &self,
        request: Request<GetDfsChunkSourcesRequest>,
    ) -> Result<Response<GetDfsChunkSourcesReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        validate_caller(authenticated.as_deref(), &request.caller_id)?;
        require_text(&request.namespace_id, "namespace_id")?;
        require_text(&request.file_version_id, "file_version_id")?;
        require_text(&request.layout_root_id, "layout_root_id")?;
        let reply = dfs_service(&self.0)?
            .chunk_sources(crate::dfs::DfsChunkSourcesRequest {
                caller_id: request.caller_id,
                namespace_id: crate::dfs::NamespaceId::new(request.namespace_id),
                file_version_id: crate::dfs::FileVersionId::new(request.file_version_id),
                layout_root_id: crate::dfs::LayoutRootId::new(request.layout_root_id),
                chunk_ids: request
                    .chunk_ids
                    .into_iter()
                    .map(crate::dfs::ChunkId::new)
                    .collect(),
            })
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(wire_chunk_sources_reply(reply)))
    }

    async fn sync_inode_metadata(
        &self,
        request: Request<SyncDfsInodeMetadataRequest>,
    ) -> Result<Response<SyncDfsInodeMetadataReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        validate_caller(authenticated.as_deref(), &request.caller_id)?;
        require_text(&request.operation_id, "operation_id")?;
        require_text(&request.inode_id, "inode_id")?;
        let inode = dfs_service(&self.0)?
            .sync_inode_metadata(
                request.caller_id,
                crate::dfs::SyncInodeMetadata {
                    operation_id: crate::dfs::OperationId::new(request.operation_id),
                    inode_id: crate::dfs::InodeId::new(request.inode_id),
                    write_lease: domain_dfs_write_lease(
                        request
                            .write_lease
                            .ok_or_else(|| invalid("DFS metadata sync requires write lease"))?,
                    ),
                    expected_inode_revision: request.expected_inode_revision,
                    expected_head_version: optional_id(request.expected_head_version_id)
                        .map(crate::dfs::FileVersionId::new),
                    metadata_delta: domain_dfs_metadata_delta(
                        request
                            .metadata_delta
                            .ok_or_else(|| invalid("DFS metadata sync requires metadata delta"))?,
                    )?,
                },
            )
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(SyncDfsInodeMetadataReply {
            inode: Some(wire_dfs_inode(inode)),
        }))
    }

    async fn commit_file_version(
        &self,
        request: Request<CommitFileVersionRequest>,
    ) -> Result<Response<CommitFileVersionReply>, Status> {
        let authenticated = authenticated_node_id(&self.0, &request)?;
        let request = request.into_inner();
        validate_caller(authenticated.as_deref(), &request.caller_id)?;
        require_text(&request.operation_id, "operation_id")?;
        require_text(&request.inode_id, "inode_id")?;
        let version = domain_dfs_version(
            request
                .version
                .ok_or_else(|| invalid("DFS commit requires FileVersion"))?,
        );
        let layout = domain_dfs_layout(
            request
                .layout
                .ok_or_else(|| invalid("DFS commit requires LayoutRoot"))?,
        );
        let receipts = request
            .chunk_receipts
            .into_iter()
            .map(domain_chunk_receipt)
            .collect::<Result<Vec<_>, _>>()?;
        let write_lease = domain_dfs_write_lease(
            request
                .write_lease
                .ok_or_else(|| invalid("DFS commit requires write lease"))?,
        );
        let metadata_delta = domain_dfs_metadata_delta(
            request
                .metadata_delta
                .ok_or_else(|| invalid("DFS commit requires metadata delta"))?,
        )?;
        let inode = dfs_service(&self.0)?
            .commit_file_version(
                request.caller_id,
                crate::dfs::CommitFileVersion {
                    operation_id: crate::dfs::OperationId::new(request.operation_id),
                    inode_id: crate::dfs::InodeId::new(request.inode_id),
                    write_lease,
                    expected_inode_revision: request.expected_inode_revision,
                    expected_head_version: optional_id(request.expected_head_version_id)
                        .map(crate::dfs::FileVersionId::new),
                    file_version: version,
                    layout_root: layout,
                    chunk_receipts: receipts,
                    metadata_delta,
                },
            )
            .await
            .map_err(afs_transport::grpc::error_status::error_to_status)?;
        Ok(Response::new(CommitFileVersionReply {
            inode: Some(wire_dfs_inode(inode)),
        }))
    }
}

fn dfs_service(meta: &super::Meta) -> Result<&super::dfs::DfsService, Status> {
    meta.dfs.as_ref().ok_or_else(|| {
        afs_transport::grpc::error_status::error_to_status(super::store::unavailable_meta_store())
    })
}

fn wire_dfs_inode(inode: crate::dfs::InodeRecord) -> PbDfsInodeRecord {
    PbDfsInodeRecord {
        namespace_id: inode.namespace_id.0,
        inode_id: inode.inode_id.0,
        kind: match inode.kind {
            crate::dfs::InodeKind::Regular => PbDfsInodeKind::Regular as i32,
            crate::dfs::InodeKind::Directory => PbDfsInodeKind::Directory as i32,
            crate::dfs::InodeKind::Symlink => PbDfsInodeKind::Symlink as i32,
        },
        attributes: Some(PbDfsInodeAttributes {
            mode: inode.attributes.mode,
            uid: inode.attributes.uid,
            gid: inode.attributes.gid,
            nlink: inode.attributes.nlink,
            atime_unix_ms: inode.attributes.atime_unix_ms,
            mtime_unix_ms: inode.attributes.mtime_unix_ms,
            ctime_unix_ms: inode.attributes.ctime_unix_ms,
        }),
        head_version_id: inode.head_version.map_or_else(String::new, |id| id.0),
        revision: inode.revision,
    }
}

fn domain_dfs_attributes(attributes: PbDfsInodeAttributes) -> crate::dfs::InodeAttributes {
    crate::dfs::InodeAttributes {
        mode: attributes.mode,
        uid: attributes.uid,
        gid: attributes.gid,
        nlink: attributes.nlink,
        atime_unix_ms: attributes.atime_unix_ms,
        mtime_unix_ms: attributes.mtime_unix_ms,
        ctime_unix_ms: attributes.ctime_unix_ms,
    }
}

fn wire_dfs_version(version: crate::dfs::FileVersion) -> afs_protocol::meta::DfsFileVersion {
    afs_protocol::meta::DfsFileVersion {
        version_id: version.id.0,
        inode_id: version.inode_id.0,
        parent_version_id: version.parent_version.map_or_else(String::new, |id| id.0),
        length: version.length,
        layout_root_id: version.layout_root.0,
        created_at_unix_ms: version.created_at_unix_ms,
    }
}

fn domain_dfs_version(version: afs_protocol::meta::DfsFileVersion) -> crate::dfs::FileVersion {
    crate::dfs::FileVersion {
        id: crate::dfs::FileVersionId::new(version.version_id),
        inode_id: crate::dfs::InodeId::new(version.inode_id),
        parent_version: optional_id(version.parent_version_id).map(crate::dfs::FileVersionId::new),
        length: version.length,
        layout_root: crate::dfs::LayoutRootId::new(version.layout_root_id),
        created_at_unix_ms: version.created_at_unix_ms,
    }
}

fn wire_dfs_layout(layout: crate::dfs::LayoutRoot) -> PbDfsLayoutRoot {
    PbDfsLayoutRoot {
        layout_root_id: layout.id.0,
        file_length: layout.file_length,
        inline_extents: layout
            .inline_extents
            .into_iter()
            .map(|extent| afs_protocol::meta::DfsExtent {
                file_offset: extent.file_offset,
                length: extent.length,
                chunk_id: extent.chunk_id.0,
                chunk_offset: extent.chunk_offset,
            })
            .collect(),
    }
}

fn wire_placement_snapshot(
    snapshot: crate::dfs::PlacementSnapshot,
) -> afs_protocol::meta::DfsPlacementSnapshot {
    use afs_protocol::meta::DfsLocalCopyPolicy;

    afs_protocol::meta::DfsPlacementSnapshot {
        revision: snapshot.revision,
        replication: Some(afs_protocol::meta::DfsReplicationConfig {
            desired_copies: u32::from(snapshot.replication.desired_copies),
            sync_required_copies: u32::from(snapshot.replication.sync_required_copies),
            min_distinct_nodes: u32::from(snapshot.replication.min_distinct_nodes),
            min_distinct_failure_domains: u32::from(
                snapshot.replication.min_distinct_failure_domains,
            ),
            local_copy: match snapshot.replication.local_copy {
                crate::dfs::LocalCopyPolicy::Required => DfsLocalCopyPolicy::Required as i32,
                crate::dfs::LocalCopyPolicy::Preferred => DfsLocalCopyPolicy::Preferred as i32,
                crate::dfs::LocalCopyPolicy::NotRequired => DfsLocalCopyPolicy::NotRequired as i32,
            },
        }),
        replica_groups: snapshot
            .replica_groups
            .into_iter()
            .map(|group| afs_protocol::meta::DfsReplicaGroup {
                replica_group_id: group.id.0,
                placement_epoch: group.placement_epoch,
                targets: group
                    .targets
                    .into_iter()
                    .map(|target| afs_protocol::meta::DfsReplicaTarget {
                        node_id: target.node_id,
                        node_epoch: target.node_epoch,
                        data_endpoint: target.data_endpoint,
                        device: Some(afs_protocol::meta::DfsStorageDevice {
                            device_id: target.device.device_id,
                            device_epoch: target.device.device_epoch,
                            catalog_revision: target.device.catalog_revision,
                            failure_domain: target.device.failure_domain,
                        }),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn wire_chunk_sources_reply(
    reply: crate::dfs::DfsChunkSourcesReply,
) -> afs_protocol::meta::GetDfsChunkSourcesReply {
    afs_protocol::meta::GetDfsChunkSourcesReply {
        revision: reply.revision,
        chunks: reply
            .chunks
            .into_iter()
            .map(|chunk| afs_protocol::meta::DfsChunkSources {
                chunk_id: chunk.chunk_id.0,
                sources: chunk
                    .sources
                    .into_iter()
                    .map(wire_source_candidate)
                    .collect(),
            })
            .collect(),
    }
}

fn wire_source_candidate(
    source: crate::dfs::SourceCandidate,
) -> afs_protocol::meta::DfsSourceCandidate {
    afs_protocol::meta::DfsSourceCandidate {
        copy_id: source.copy_id.0,
        chunk_id: source.chunk_id.0,
        role: match source.role {
            crate::dfs::CopyRole::DurableReplica => {
                afs_protocol::meta::DfsCopyRole::DurableReplica as i32
            }
            crate::dfs::CopyRole::VerifiedCache => {
                afs_protocol::meta::DfsCopyRole::VerifiedCache as i32
            }
            crate::dfs::CopyRole::ExternalCommitted => {
                afs_protocol::meta::DfsCopyRole::ExternalCommitted as i32
            }
        },
        state: match source.state {
            crate::dfs::CopyState::Ready => afs_protocol::meta::DfsCopyState::Ready as i32,
            crate::dfs::CopyState::Corrupt => afs_protocol::meta::DfsCopyState::Corrupt as i32,
            crate::dfs::CopyState::Deleting => afs_protocol::meta::DfsCopyState::Deleting as i32,
            crate::dfs::CopyState::LegacyStaging => {
                afs_protocol::meta::DfsCopyState::Unspecified as i32
            }
        },
        location: Some(wire_copy_location(source.location)),
        data_endpoint: source.data_endpoint,
        load_hint: source.load_hint,
        read_grant: Some(wire_read_grant(source.read_grant)),
    }
}

fn wire_copy_location(location: crate::dfs::CopyLocation) -> afs_protocol::meta::DfsCopyLocation {
    use afs_protocol::meta::dfs_copy_location::{External, Location, Node};

    afs_protocol::meta::DfsCopyLocation {
        location: Some(match location {
            crate::dfs::CopyLocation::Node {
                node_id,
                node_epoch,
                device_id,
                device_epoch,
                catalog_revision,
            } => Location::Node(Node {
                node_id,
                node_epoch,
                device_id,
                device_epoch,
                catalog_revision,
            }),
            crate::dfs::CopyLocation::External {
                store_id,
                object_key,
                object_revision,
            } => Location::External(External {
                store_id,
                object_key,
                object_revision,
            }),
        }),
    }
}

fn wire_read_grant(grant: crate::dfs::DfsReadGrant) -> afs_protocol::meta::DfsReadGrant {
    afs_protocol::meta::DfsReadGrant {
        namespace_id: grant.namespace_id.0,
        file_version_id: grant.file_version_id.0,
        layout_root_id: grant.layout_root_id.0,
        caller_node_id: grant.caller_node_id,
        caller_node_epoch: grant.caller_node_epoch,
        expires_at_unix_ms: grant.expires_at_unix_ms,
        fence: grant.fence,
        token: grant.token,
    }
}

fn domain_dfs_layout(layout: PbDfsLayoutRoot) -> crate::dfs::LayoutRoot {
    crate::dfs::LayoutRoot {
        id: crate::dfs::LayoutRootId::new(layout.layout_root_id),
        file_length: layout.file_length,
        inline_extents: layout
            .inline_extents
            .into_iter()
            .map(|extent| crate::dfs::Extent {
                file_offset: extent.file_offset,
                length: extent.length,
                chunk_id: crate::dfs::ChunkId::new(extent.chunk_id),
                chunk_offset: extent.chunk_offset,
            })
            .collect(),
    }
}

fn wire_dfs_write_lease(lease: crate::dfs::WriteLease) -> afs_protocol::meta::DfsWriteLease {
    afs_protocol::meta::DfsWriteLease {
        inode_id: lease.inode_id.0,
        owner_node_id: lease.owner_node_id,
        owner_session_id: lease.owner_session_id,
        lease_epoch: lease.lease_epoch,
        expires_at_unix_ms: lease.expires_at_unix_ms,
    }
}

fn domain_dfs_write_lease(lease: afs_protocol::meta::DfsWriteLease) -> crate::dfs::WriteLease {
    crate::dfs::WriteLease {
        inode_id: crate::dfs::InodeId::new(lease.inode_id),
        owner_node_id: lease.owner_node_id,
        owner_session_id: lease.owner_session_id,
        lease_epoch: lease.lease_epoch,
        expires_at_unix_ms: lease.expires_at_unix_ms,
    }
}

fn domain_dfs_metadata_delta(
    delta: afs_protocol::meta::DfsCommitMetadataDelta,
) -> Result<crate::dfs::CommitMetadataDelta, Status> {
    let mode = match DfsCommitMetadataMode::try_from(delta.mode)
        .map_err(|_| invalid("DFS commit metadata mode is invalid"))?
    {
        DfsCommitMetadataMode::DataOnly => crate::dfs::CommitMetadataMode::DataOnly,
        DfsCommitMetadataMode::Full => crate::dfs::CommitMetadataMode::Full,
        DfsCommitMetadataMode::Unspecified => {
            return Err(invalid("DFS commit metadata mode is required"));
        }
    };
    Ok(crate::dfs::CommitMetadataDelta {
        mode,
        mtime_unix_ms: (delta.mtime_unix_ms != 0).then_some(delta.mtime_unix_ms),
        ctime_unix_ms: (delta.ctime_unix_ms != 0).then_some(delta.ctime_unix_ms),
    })
}

fn domain_chunk_receipt(
    receipt: afs_protocol::meta::DfsChunkReceipt,
) -> Result<crate::dfs::ChunkReceipt, Status> {
    let digest: [u8; 32] = receipt
        .content_digest
        .try_into()
        .map_err(|_| invalid("DFS chunk digest must contain 32 bytes"))?;
    let digest = crate::dfs::ContentDigest {
        algorithm: domain_digest_algorithm(receipt.content_digest_algorithm)?,
        bytes: digest,
    };
    let chunk_id = crate::dfs::ChunkId::new(receipt.chunk_id);
    let durable_acks = receipt
        .durable_acks
        .into_iter()
        .map(|ack| {
            let verified_digest: [u8; 32] = ack
                .verified_digest
                .try_into()
                .map_err(|_| invalid("DFS replica digest must contain 32 bytes"))?;
            Ok(crate::dfs::ReplicaAck {
                operation_id: crate::dfs::OperationId::new(ack.operation_id),
                chunk_id: crate::dfs::ChunkId::new(ack.chunk_id),
                placement_revision: ack.placement_revision,
                placement_epoch: ack.placement_epoch,
                node_id: ack.node_id,
                node_epoch: ack.node_epoch,
                device_id: ack.device_id,
                device_epoch: ack.device_epoch,
                catalog_revision: ack.catalog_revision,
                persisted_bytes: ack.persisted_bytes,
                verified_digest: crate::dfs::ContentDigest {
                    algorithm: domain_digest_algorithm(ack.verified_digest_algorithm)?,
                    bytes: verified_digest,
                },
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    Ok(crate::dfs::ChunkReceipt {
        operation_id: crate::dfs::OperationId::new(receipt.operation_id),
        chunk: crate::dfs::ChunkObject {
            id: chunk_id.clone(),
            length: receipt.chunk_length,
            content_digest: digest.clone(),
            encoding: crate::dfs::ChunkEncoding::Raw,
        },
        placement_revision: receipt.placement_revision,
        placement_epoch: receipt.placement_epoch,
        replica_group_id: crate::dfs::ReplicaGroupId::new(receipt.replica_group_id),
        durable_acks,
    })
}

fn domain_digest_algorithm(value: i32) -> Result<crate::dfs::DigestAlgorithm, Status> {
    match afs_protocol::meta::DfsDigestAlgorithm::try_from(value)
        .map_err(|_| invalid("DFS digest algorithm is invalid"))?
    {
        afs_protocol::meta::DfsDigestAlgorithm::Blake3 => Ok(crate::dfs::DigestAlgorithm::Blake3),
        afs_protocol::meta::DfsDigestAlgorithm::Unspecified => {
            Err(invalid("DFS digest algorithm is required"))
        }
    }
}

fn optional_id(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    fn presented_access() -> PresentedRootAccess {
        PresentedRootAccess {
            root_id: "workspace-a".into(),
            root_epoch: 1,
            home_node_id: "node-a".into(),
            holder_node_id: "node-b".into(),
            session_id: "session-b".into(),
            access_generation: 1,
            fencing_token: "fence:workspace-a:session-b:1".into(),
            home_session_id: "session-a".into(),
        }
    }

    #[test]
    fn validate_root_access_binds_meta_tls_to_validator_not_presented_holder() {
        let presented = presented_access();
        validate_root_access_identities(
            Some("node-a"),
            "node-b",
            "node-a",
            "session-a",
            &presented,
        )
        .expect("Home A should validate B's grant when P2P observed peer is B");
    }

    #[test]
    fn validate_root_access_rejects_wrong_meta_tls_validator() {
        let presented = presented_access();
        let error = validate_root_access_identities(
            Some("node-b"),
            "node-b",
            "node-a",
            "session-a",
            &presented,
        )
        .unwrap_err();
        assert_eq!(error.code(), Code::PermissionDenied);
    }

    #[test]
    fn validate_root_access_rejects_wrong_observed_peer() {
        let presented = presented_access();
        let error = validate_root_access_identities(
            Some("node-a"),
            "node-x",
            "node-a",
            "session-a",
            &presented,
        )
        .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
    }
}

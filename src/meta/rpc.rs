//! Generated gRPC adapter around the same Meta authority used by REST.
//!
//! OwnerRoots tonic methods authenticate and translate protobuf/domain types;
//! their business transitions live in `owner_roots.rs` and `Meta`; RPC does
//! not own authority state or decide durable commits.
//! Without a store, authority RPCs fail closed instead of fabricating a grant.

use afs_protocol::meta::{
    AbortDraftReply, AbortDraftRequest, AbortRootReply, AbortRootRequest, AbortSnapshotReply,
    AbortSnapshotRequest, AckRevocationReply, AckRevocationRequest, AcquireRootReply,
    AcquireRootRequest, ActivateRootReply, ActivateRootRequest, BeginSnapshotReply,
    BeginSnapshotRequest, CommitVersionReply, CommitVersionRequest, ListOwnerRootsReply,
    ListOwnerRootsRequest, LookupNodeReply, LookupNodeRequest, LookupRootReply, LookupRootRequest,
    LookupVersionReply, LookupVersionRequest, NodeDescriptor, NodeEndpoint, PingReply, PingRequest,
    PresentedRootAccess, RecoverRootReply, RecoverRootRequest, RegisterNodeReply,
    RegisterNodeRequest, ReserveDraftReply, ReserveDraftRequest, ReserveRootReply,
    ReserveRootRequest, RootCommand, RootCommandType, RootLocation, RootReservation,
    RootRight as PbRootRight, ValidateRootAccessReply, ValidateRootAccessRequest,
    WatchRootCommandsRequest, blob_meta_server::BlobMeta as BlobMetaService,
    meta_server::Meta as MetaService, owner_roots_server::OwnerRoots as OwnerRootsService,
};
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use super::store::{
    NodeSessionLease, RequestKey, RootAccessGrant, RootCommandRecord, RootReservationRecord,
    RootRight, StoreRevision,
};

pub struct MetaRpc(pub Arc<super::Meta>);
pub struct OwnerRootsRpc(pub Arc<super::Meta>);
pub struct BlobMetaRpc(pub Arc<super::Meta>);

type RootCommandStream = Pin<Box<dyn Stream<Item = Result<RootCommand, Status>> + Send + 'static>>;

fn invalid(message: impl Into<String>) -> Status {
    afs_transport::grpc::error_status::error_to_status(afs_error::Error::coded(
        afs_error::META_CATALOG_INVALID_REQUEST,
        message,
    ))
}

fn unavailable_blob(operation: &'static str) -> Status {
    afs_transport::grpc::error_status::error_to_status(afs_error::Error::coded(
        afs_error::META_STORE_UNIMPLEMENTED,
        format!("{operation} is not implemented in the OwnerFs Meta authority slice"),
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
impl BlobMetaService for BlobMetaRpc {
    async fn reserve_draft(
        &self,
        _request: Request<ReserveDraftRequest>,
    ) -> Result<Response<ReserveDraftReply>, Status> {
        Err(unavailable_blob("BlobMeta.ReserveDraft"))
    }

    async fn begin_snapshot(
        &self,
        _request: Request<BeginSnapshotRequest>,
    ) -> Result<Response<BeginSnapshotReply>, Status> {
        Err(unavailable_blob("BlobMeta.BeginSnapshot"))
    }

    async fn commit_version(
        &self,
        _request: Request<CommitVersionRequest>,
    ) -> Result<Response<CommitVersionReply>, Status> {
        Err(unavailable_blob("BlobMeta.CommitVersion"))
    }

    async fn abort_snapshot(
        &self,
        _request: Request<AbortSnapshotRequest>,
    ) -> Result<Response<AbortSnapshotReply>, Status> {
        Err(unavailable_blob("BlobMeta.AbortSnapshot"))
    }

    async fn abort_draft(
        &self,
        _request: Request<AbortDraftRequest>,
    ) -> Result<Response<AbortDraftReply>, Status> {
        Err(unavailable_blob("BlobMeta.AbortDraft"))
    }

    async fn lookup_version(
        &self,
        _request: Request<LookupVersionRequest>,
    ) -> Result<Response<LookupVersionReply>, Status> {
        Err(unavailable_blob("BlobMeta.LookupVersion"))
    }
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

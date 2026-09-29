//! Node owns its generated Meta caller; common transport only configures it.
//!
//! 调用端直接使用 Proto 生成的 MetaClient，再应用 common/transport/grpc 的配置。
//! 这是 Node 的内部控制 caller，不是对外的本机高性能 client/ SDK。
//! 示例每次显式 ping 建连；未来节点注册/watch 的连接复用由相应业务模块管理。
#[cfg(feature = "ownerfs")]
use crate::node::vfs::ownerfs::{
    catalog::LocalRootRecord,
    root::{
        OwnerRootInventory, PreparedRoot, PresentedRootAccess, RootGrant, RootId, RootLocation,
        RootMeta, RootReservation, RootRight,
    },
};
#[cfg(any(feature = "ownerfs", feature = "dfs"))]
use afs_error::CLIENT_PROTOCOL_VIOLATION;
use afs_error::{CLIENT_ARGUMENT_INVALID, CLIENT_CONNECTION_UNAVAILABLE, Error, Result};
#[cfg(feature = "ownerfs")]
use afs_protocol::meta::{
    AbortRootRequest, AcquireRootRequest, ActivateRootRequest, ListOwnerRootsRequest,
    LookupNodeRequest, LookupRootRequest, RecoverRootRequest, ReserveConflictPolicy,
    ReserveRootRequest, ValidateRootAccessRequest, owner_roots_client::OwnerRootsClient,
};
use afs_protocol::meta::{PingRequest, RegisterNodeRequest, meta_client::MetaClient};
use afs_transport::grpc::{GrpcConfig, SecurityManager, TlsConfig};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "dfs")]
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(any(feature = "ownerfs", feature = "dfs"))]
use tonic::transport::Channel;
use tonic::transport::Endpoint;
pub async fn ping(endpoint: &str, node_id: &str, timeout: Duration) -> afs_error::Result<String> {
    let config = GrpcConfig {
        connect_timeout: timeout,
        request_timeout: timeout,
        ..Default::default()
    };
    let channel = config
        .configure_client(
            Endpoint::from_shared(endpoint.to_owned())
                .map_err(|e| Error::coded(CLIENT_ARGUMENT_INVALID, e.to_string()))?,
        )
        .connect()
        .await
        .map_err(|e| Error::coded(CLIENT_CONNECTION_UNAVAILABLE, e.to_string()))?;
    let mut client = MetaClient::new(afs_tracing::traced_channel(channel))
        .max_encoding_message_size(config.max_encoding_message_bytes)
        .max_decoding_message_size(config.max_decoding_message_bytes);
    let reply = client
        .ping(afs_tracing::request_with_current_context(PingRequest {
            node_id: node_id.into(),
        }))
        .await
        .map_err(afs_transport::grpc::error_status::status_to_error)?;
    Ok(reply.into_inner().message)
}

/// 同步 VFS/FUSE 回调的 Meta adapter。调用方必须在 FUSE 线程或
/// `spawn_blocking` 中，不能在 Tokio worker 上直接调用 RootMeta 方法。
/// 创建/授权/恢复是慢路径；根内操作不经过本模块。
#[cfg(feature = "ownerfs")]
pub struct GrpcRootMeta {
    channel: Channel,
    runtime: tokio::runtime::Handle,
    node_id: String,
    session_id: String,
    timeout: Duration,
    sequence: AtomicU64,
}

#[cfg(feature = "ownerfs")]
impl GrpcRootMeta {
    pub fn new(
        endpoint: &str,
        node_id: String,
        session_id: String,
        timeout: Duration,
        tls: TlsConfig,
    ) -> Result<Self> {
        let endpoint = GrpcConfig {
            connect_timeout: timeout,
            request_timeout: timeout,
            ..Default::default()
        }
        .configure_client(
            Endpoint::from_shared(endpoint.to_owned())
                .map_err(|e| Error::coded(CLIENT_ARGUMENT_INVALID, e.to_string()))?,
        );
        let endpoint = SecurityManager::new(tls)
            .map_err(|e| Error::coded(CLIENT_ARGUMENT_INVALID, e.to_string()))?
            .configure_client(endpoint)
            .map_err(|e| Error::coded(CLIENT_ARGUMENT_INVALID, e.to_string()))?;
        let channel = endpoint.connect_lazy();
        Ok(Self {
            channel,
            runtime: tokio::runtime::Handle::current(),
            node_id,
            session_id,
            timeout,
            sequence: AtomicU64::new(1),
        })
    }

    fn request_id(&self) -> String {
        format!(
            "{}-{}",
            self.session_id,
            self.sequence.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn client(&self) -> OwnerRootsClient<Channel> {
        OwnerRootsClient::new(self.channel.clone())
    }

    /// 节点位置来自 Meta 注册表，而非调用者在数据请求中自报的地址。
    /// 返回完整 URI，供 OwnerFs 在根授权缓存未命中时建立 P2P 连接。
    pub fn lookup_node_endpoint(&self, node_id: &str) -> Result<String> {
        let mut client = MetaClient::new(self.channel.clone());
        let reply = self
            .run(client.lookup_node(LookupNodeRequest {
                request_id: self.request_id(),
                node_id: node_id.to_owned(),
            }))?
            .into_inner();
        if !reply.found {
            return Err(Error::coded(
                afs_error::NODE_VFS_NOT_FOUND,
                "Home Node is not registered",
            ));
        }
        let node = required(reply.node, "LookupNode.node")?;
        let endpoint = required(node.endpoint, "LookupNode.node.endpoint")?.grpc_addr;
        if endpoint.is_empty() {
            return Err(Error::coded(
                CLIENT_PROTOCOL_VIOLATION,
                "Home Node has no gRPC endpoint",
            ));
        }
        Ok(endpoint)
    }

    fn run<T>(
        &self,
        future: impl std::future::Future<Output = std::result::Result<T, tonic::Status>>,
    ) -> Result<T> {
        self.runtime.block_on(async {
            tokio::time::timeout(self.timeout, future)
                .await
                .map_err(|_| {
                    Error::coded(
                        afs_error::CLIENT_DEADLINE_EXCEEDED,
                        "Meta control request timed out",
                    )
                })?
                .map_err(afs_transport::grpc::error_status::status_to_error)
        })
    }
}

#[cfg(any(feature = "ownerfs", feature = "dfs"))]
fn required<T>(value: Option<T>, field: &str) -> Result<T> {
    value.ok_or_else(|| {
        Error::coded(
            CLIENT_PROTOCOL_VIOLATION,
            format!("Meta reply missing {field}"),
        )
    })
}

#[cfg(feature = "ownerfs")]
fn right_wire(right: RootRight) -> i32 {
    use afs_protocol::meta::RootRight as Wire;
    (match right {
        RootRight::Lookup => Wire::Lookup,
        RootRight::Read => Wire::Read,
        RootRight::Write => Wire::Write,
        RootRight::Admin => Wire::Admin,
    }) as i32
}

#[cfg(feature = "ownerfs")]
fn remote_session_rights(requested: RootRight) -> Vec<i32> {
    let mut rights = vec![
        right_wire(RootRight::Lookup),
        right_wire(RootRight::Read),
        right_wire(RootRight::Write),
    ];
    if requested == RootRight::Admin {
        rights.push(right_wire(RootRight::Admin));
    }
    rights
}

#[cfg(feature = "ownerfs")]
fn grant_from_wire(value: afs_protocol::meta::RootAccess) -> Result<RootGrant> {
    use afs_protocol::meta::RootRight as Wire;
    let rights = value
        .rights
        .into_iter()
        .map(|right| match Wire::try_from(right).ok() {
            Some(Wire::Lookup) => Ok(RootRight::Lookup),
            Some(Wire::Read) => Ok(RootRight::Read),
            Some(Wire::Write) => Ok(RootRight::Write),
            Some(Wire::Admin) => Ok(RootRight::Admin),
            _ => Err(Error::coded(
                CLIENT_PROTOCOL_VIOLATION,
                "Meta returned invalid root right",
            )),
        })
        .collect::<Result<Vec<_>>>()?;
    if value.root_id.is_empty() || value.fencing_token.is_empty() || rights.is_empty() {
        return Err(Error::coded(
            CLIENT_PROTOCOL_VIOLATION,
            "Meta returned incomplete root access",
        ));
    }
    Ok(RootGrant {
        id: RootId(value.root_id),
        epoch: value.root_epoch,
        home_node_id: value.home_node_id,
        home_session_id: value.home_session_id,
        holder_node_id: value.holder_node_id,
        session_id: value.session_id,
        access_generation: value.access_generation,
        rights,
        fencing_token: value.fencing_token,
    })
}

#[cfg(feature = "ownerfs")]
fn reservation_wire(value: &RootReservation) -> afs_protocol::meta::RootReservation {
    afs_protocol::meta::RootReservation {
        root_id: value.id.0.clone(),
        root_epoch: value.epoch,
        home_node_id: value.home_node_id.clone(),
        session_id: value.session_id.clone(),
        create_intent_id: value.create_intent_id.clone(),
        prepare_token: value.prepare_token.clone(),
    }
}

#[cfg(feature = "ownerfs")]
impl RootMeta for GrpcRootMeta {
    fn lookup_node_endpoint(&self, node_id: &str) -> Result<String> {
        GrpcRootMeta::lookup_node_endpoint(self, node_id)
    }

    fn reserve_root(&self, id: &RootId, create_intent_id: &str) -> Result<RootReservation> {
        let request = ReserveRootRequest {
            request_id: self.request_id(),
            root_id: id.0.clone(),
            preferred_home_node_id: self.node_id.clone(),
            session_id: self.session_id.clone(),
            rights: [
                RootRight::Lookup,
                RootRight::Read,
                RootRight::Write,
                RootRight::Admin,
            ]
            .map(right_wire)
            .to_vec(),
            expected_root_epoch: 0,
            parent_root_id: String::new(),
            create_intent_id: create_intent_id.to_owned(),
            conflict_policy: ReserveConflictPolicy::ReuseIfSameIntent as i32,
        };
        let mut client = self.client();
        let reply = self.run(client.reserve_root(request))?.into_inner();
        let value = required(reply.reservation, "ReserveRoot.reservation")?;
        Ok(RootReservation {
            id: RootId(value.root_id),
            epoch: value.root_epoch,
            home_node_id: value.home_node_id,
            session_id: value.session_id,
            create_intent_id: value.create_intent_id,
            prepare_token: value.prepare_token,
        })
    }

    fn activate_root(&self, prepared: &PreparedRoot) -> Result<RootGrant> {
        let mut client = self.client();
        let reply = self
            .run(client.activate_root(ActivateRootRequest {
                request_id: self.request_id(),
                reservation: Some(reservation_wire(prepared.reservation())),
                local_prepare_id: prepared.local_prepare_id().to_owned(),
                parent_fsync_generation: prepared.parent_fsync_generation(),
                parent_fsync_complete: true,
            }))?
            .into_inner();
        grant_from_wire(required(reply.access, "ActivateRoot.access")?)
    }

    fn abort_root(&self, reservation: &RootReservation) -> Result<()> {
        let mut client = self.client();
        self.run(client.abort_root(AbortRootRequest {
            request_id: self.request_id(),
            root_id: reservation.id.0.clone(),
            root_epoch: reservation.epoch,
            session_id: reservation.session_id.clone(),
            create_intent_id: reservation.create_intent_id.clone(),
            reason: "local root preparation failed".into(),
            prepare_token: reservation.prepare_token.clone(),
        }))?;
        Ok(())
    }

    fn lookup_root(&self, id: &RootId) -> Result<Option<RootLocation>> {
        let mut client = self.client();
        let reply = self
            .run(client.lookup_root(LookupRootRequest {
                request_id: self.request_id(),
                root_id: id.0.clone(),
            }))?
            .into_inner();
        if !reply.found {
            return Ok(None);
        }
        let value = required(reply.location, "LookupRoot.location")?;
        Ok(Some(RootLocation {
            id: RootId(value.root_id),
            epoch: value.root_epoch,
            home_node_id: value.home_node_id,
            home_session_id: value.home_session_id,
        }))
    }

    fn list_owner_roots(&self, home_node_id: &str) -> Result<OwnerRootInventory> {
        let mut client = self.client();
        let reply = self
            .run(client.list_owner_roots(ListOwnerRootsRequest {
                request_id: self.request_id(),
                home_node_id: home_node_id.to_owned(),
            }))?
            .into_inner();
        Ok(OwnerRootInventory {
            active: reply
                .active_roots
                .into_iter()
                .map(|value| RootLocation {
                    id: RootId(value.root_id),
                    epoch: value.root_epoch,
                    home_node_id: value.home_node_id,
                    home_session_id: value.home_session_id,
                })
                .collect(),
            pending: reply
                .pending_reservations
                .into_iter()
                .map(|value| RootReservation {
                    id: RootId(value.root_id),
                    epoch: value.root_epoch,
                    home_node_id: value.home_node_id,
                    session_id: value.session_id,
                    create_intent_id: value.create_intent_id,
                    prepare_token: value.prepare_token,
                })
                .collect(),
        })
    }

    fn acquire_root(&self, id: &RootId, right: RootRight) -> Result<RootGrant> {
        let mut client = self.client();
        let reply = self
            .run(client.acquire_root(AcquireRootRequest {
                request_id: self.request_id(),
                root_id: id.0.clone(),
                requester_node_id: self.node_id.clone(),
                session_id: self.session_id.clone(),
                rights: remote_session_rights(right),
                expected_root_epoch: 0,
                expected_access_generation: 0,
            }))?
            .into_inner();
        grant_from_wire(required(reply.access, "AcquireRoot.access")?)
    }

    fn current_node_session(&self, node_id: &str) -> Result<Option<String>> {
        let mut client = MetaClient::new(self.channel.clone());
        let reply = self
            .run(client.lookup_node(LookupNodeRequest {
                request_id: self.request_id(),
                node_id: node_id.to_owned(),
            }))?
            .into_inner();
        if !reply.found {
            return Ok(None);
        }
        let node = required(reply.node, "LookupNode.node")?;
        if node.session_id.is_empty() {
            return Err(Error::coded(
                CLIENT_PROTOCOL_VIOLATION,
                "LookupNode returned an empty process session",
            ));
        }
        Ok(Some(node.session_id))
    }

    fn validate_root_access(
        &self,
        presented: &PresentedRootAccess,
        authenticated_peer_node_id: &str,
    ) -> Result<RootGrant> {
        // observed_peer_node_id 必须由传输认证层给出，不能照抄请求体里的 holder。
        let mut client = self.client();
        let reply = self
            .run(client.validate_root_access(ValidateRootAccessRequest {
                request_id: self.request_id(),
                presented_access: Some(afs_protocol::meta::PresentedRootAccess {
                    root_id: presented.id.0.clone(),
                    root_epoch: presented.epoch,
                    home_node_id: presented.home_node_id.clone(),
                    holder_node_id: presented.holder_node_id.clone(),
                    session_id: presented.session_id.clone(),
                    access_generation: presented.access_generation,
                    fencing_token: presented.fencing_token.clone(),
                    home_session_id: presented.home_session_id.clone(),
                }),
                observed_peer_node_id: authenticated_peer_node_id.to_owned(),
                validator_home_node_id: self.node_id.clone(),
                validator_home_session_id: self.session_id.clone(),
            }))?
            .into_inner();
        grant_from_wire(required(reply.access, "ValidateRootAccess.access")?)
    }

    fn recover_root(&self, record: &LocalRootRecord, new_session_id: &str) -> Result<RootGrant> {
        let mut client = self.client();
        let reply = self
            .run(client.recover_root(RecoverRootRequest {
                request_id: self.request_id(),
                root_id: record.id.0.clone(),
                expected_root_epoch: record.epoch,
                home_node_id: self.node_id.clone(),
                home_session_id: new_session_id.to_owned(),
                local_prepare_id: record.local_prepare_id.clone(),
            }))?
            .into_inner();
        grant_from_wire(required(reply.access, "RecoverRoot.access")?)
    }
}

/// 新 Node 会话注册完成之前不能挂载或服务 OwnerFs 根。
pub async fn register_node(
    endpoint: &str,
    node: afs_protocol::meta::NodeDescriptor,
    timeout: Duration,
    tls: TlsConfig,
) -> Result<()> {
    static REGISTER_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let config = GrpcConfig {
        connect_timeout: timeout,
        request_timeout: timeout,
        ..Default::default()
    };
    let endpoint = config.configure_client(
        Endpoint::from_shared(endpoint.to_owned())
            .map_err(|e| Error::coded(CLIENT_ARGUMENT_INVALID, e.to_string()))?,
    );
    let channel = SecurityManager::new(tls)
        .map_err(|e| Error::coded(CLIENT_ARGUMENT_INVALID, e.to_string()))?
        .configure_client(endpoint)
        .map_err(|e| Error::coded(CLIENT_ARGUMENT_INVALID, e.to_string()))?
        .connect()
        .await
        .map_err(|e| Error::coded(CLIENT_CONNECTION_UNAVAILABLE, e.to_string()))?;
    let mut client = MetaClient::new(afs_tracing::traced_channel(channel));
    client
        .register_node(RegisterNodeRequest {
            request_id: format!(
                "register-{}-{}",
                node.session_id,
                REGISTER_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            ),
            node: Some(node),
            lease_seconds: 30,
        })
        .await
        .map_err(afs_transport::grpc::error_status::status_to_error)?;
    Ok(())
}

/// Synchronous adapter used by the DFS Backend on FUSE worker threads.
#[cfg(feature = "dfs")]
pub struct GrpcDfsMeta {
    channel: Channel,
    runtime: tokio::runtime::Handle,
    node_id: String,
    session_id: String,
    namespace_id: crate::dfs::NamespaceId,
    timeout: Duration,
    sequence: AtomicU64,
    placement: Mutex<Option<Arc<crate::dfs::PlacementSnapshot>>>,
}

#[cfg(feature = "dfs")]
impl GrpcDfsMeta {
    pub fn new(
        endpoint: &str,
        node_id: String,
        session_id: String,
        namespace_id: crate::dfs::NamespaceId,
        timeout: Duration,
        tls: TlsConfig,
    ) -> afs_error::Result<Self> {
        let endpoint = GrpcConfig {
            connect_timeout: timeout,
            request_timeout: timeout,
            ..Default::default()
        }
        .configure_client(
            Endpoint::from_shared(endpoint.to_owned())
                .map_err(|error| Error::coded(CLIENT_ARGUMENT_INVALID, error.to_string()))?,
        );
        let endpoint = SecurityManager::new(tls)
            .map_err(|error| Error::coded(CLIENT_ARGUMENT_INVALID, error.to_string()))?
            .configure_client(endpoint)
            .map_err(|error| Error::coded(CLIENT_ARGUMENT_INVALID, error.to_string()))?;
        Ok(Self {
            channel: endpoint.connect_lazy(),
            runtime: tokio::runtime::Handle::current(),
            node_id,
            session_id,
            namespace_id,
            timeout,
            sequence: AtomicU64::new(1),
            placement: Mutex::new(None),
        })
    }

    fn request_id(&self) -> String {
        format!(
            "{}-dfs-{}",
            self.session_id,
            self.sequence.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn run<T>(
        &self,
        future: impl std::future::Future<Output = std::result::Result<T, tonic::Status>>,
    ) -> afs_error::Result<T> {
        self.runtime.block_on(async {
            tokio::time::timeout(self.timeout, future)
                .await
                .map_err(|_| {
                    Error::coded(
                        afs_error::CLIENT_DEADLINE_EXCEEDED,
                        "DFS Meta request timed out",
                    )
                })?
                .map_err(afs_transport::grpc::error_status::status_to_error)
        })
    }

    fn client(&self) -> afs_protocol::meta::dfs_meta_client::DfsMetaClient<Channel> {
        afs_protocol::meta::dfs_meta_client::DfsMetaClient::new(self.channel.clone())
    }
}

#[cfg(feature = "dfs")]
impl crate::node::replication::PlacementProvider for GrpcDfsMeta {
    fn snapshot(&self) -> afs_error::Result<Arc<crate::dfs::PlacementSnapshot>> {
        if let Some(snapshot) = self
            .placement
            .lock()
            .map_err(|_| {
                Error::coded(
                    afs_error::CLIENT_PROTOCOL_VIOLATION,
                    "DFS placement cache lock poisoned",
                )
            })?
            .clone()
        {
            return Ok(snapshot);
        }
        self.refresh(0)
    }

    fn refresh(
        &self,
        minimum_revision: u64,
    ) -> afs_error::Result<Arc<crate::dfs::PlacementSnapshot>> {
        let reply = self
            .run(self.client().get_placement_snapshot(
                afs_protocol::meta::GetDfsPlacementSnapshotRequest {
                    caller_id: self.node_id.clone(),
                    minimum_revision,
                },
            ))?
            .into_inner();
        let snapshot = Arc::new(domain_placement_snapshot(required(
            reply.snapshot,
            "GetPlacementSnapshot.snapshot",
        )?)?);
        *self.placement.lock().map_err(|_| {
            Error::coded(
                afs_error::CLIENT_PROTOCOL_VIOLATION,
                "DFS placement cache lock poisoned",
            )
        })? = Some(snapshot.clone());
        Ok(snapshot)
    }
}

#[cfg(feature = "dfs")]
impl crate::node::dfs_read::ReadSourceProvider for GrpcDfsMeta {
    fn sources_for(
        &self,
        request: crate::dfs::DfsChunkSourcesRequest,
    ) -> afs_error::Result<crate::dfs::DfsChunkSourcesReply> {
        let reply = self
            .run(
                self.client()
                    .get_chunk_sources(afs_protocol::meta::GetDfsChunkSourcesRequest {
                        caller_id: request.caller_id,
                        namespace_id: request.namespace_id.0,
                        file_version_id: request.file_version_id.0,
                        layout_root_id: request.layout_root_id.0,
                        chunk_ids: request
                            .chunk_ids
                            .into_iter()
                            .map(|chunk_id| chunk_id.0)
                            .collect(),
                    }),
            )?
            .into_inner();
        Ok(crate::dfs::DfsChunkSourcesReply {
            revision: reply.revision,
            chunks: reply
                .chunks
                .into_iter()
                .map(domain_dfs_chunk_sources)
                .collect::<afs_error::Result<Vec<_>>>()?,
        })
    }
}

#[cfg(feature = "dfs")]
impl crate::node::vfs::dfs::DfsMeta for GrpcDfsMeta {
    fn lookup(
        &self,
        parent: &crate::dfs::InodeId,
        name: &[u8],
    ) -> afs_error::Result<Option<crate::dfs::InodeRecord>> {
        let reply = self
            .run(self.client().lookup(afs_protocol::meta::DfsLookupRequest {
                namespace_id: self.namespace_id.0.clone(),
                parent_inode_id: parent.0.clone(),
                name: name.to_vec(),
            }))?
            .into_inner();
        if !reply.found {
            return Ok(None);
        }
        required(reply.inode, "DfsLookup.inode")
            .and_then(domain_dfs_inode)
            .map(Some)
    }

    fn create(
        &self,
        operation_id: &crate::dfs::OperationId,
        parent: &crate::dfs::InodeId,
        name: &[u8],
        attributes: crate::dfs::InodeAttributes,
    ) -> afs_error::Result<(crate::dfs::InodeRecord, crate::dfs::WriteLease)> {
        let reply = self
            .run(self.client().create(afs_protocol::meta::DfsCreateRequest {
                caller_id: self.node_id.clone(),
                operation_id: operation_id.0.clone(),
                namespace_id: self.namespace_id.0.clone(),
                parent_inode_id: parent.0.clone(),
                name: name.to_vec(),
                attributes: Some(wire_dfs_attributes(attributes)),
                owner_session_id: self.session_id.clone(),
                lease_seconds: crate::node::vfs::dfs::DFS_WRITE_LEASE_SECONDS,
            }))?
            .into_inner();
        Ok((
            domain_dfs_inode(required(reply.inode, "DfsCreate.inode")?)?,
            domain_dfs_write_lease(required(reply.write_lease, "DfsCreate.write_lease")?),
        ))
    }

    fn open_write(
        &self,
        inode_id: &crate::dfs::InodeId,
    ) -> afs_error::Result<(crate::dfs::InodeRecord, crate::dfs::WriteLease)> {
        let reply = self
            .run(
                self.client()
                    .open_write(afs_protocol::meta::OpenDfsWriteRequest {
                        caller_id: self.node_id.clone(),
                        owner_session_id: self.session_id.clone(),
                        operation_id: self.request_id(),
                        inode_id: inode_id.0.clone(),
                        lease_seconds: crate::node::vfs::dfs::DFS_WRITE_LEASE_SECONDS,
                    }),
            )?
            .into_inner();
        Ok((
            domain_dfs_inode(required(reply.inode, "OpenWrite.inode")?)?,
            domain_dfs_write_lease(required(reply.write_lease, "OpenWrite.write_lease")?),
        ))
    }

    fn renew_write_lease(
        &self,
        lease: crate::dfs::WriteLease,
    ) -> afs_error::Result<crate::dfs::WriteLease> {
        let reply =
            self.run(self.client().renew_write_lease(
                afs_protocol::meta::RenewDfsWriteLeaseRequest {
                    caller_id: self.node_id.clone(),
                    owner_session_id: self.session_id.clone(),
                    operation_id: self.request_id(),
                    current_lease: Some(wire_dfs_write_lease(lease)),
                    lease_seconds: crate::node::vfs::dfs::DFS_WRITE_LEASE_SECONDS,
                },
            ))?
            .into_inner();
        Ok(domain_dfs_write_lease(required(
            reply.write_lease,
            "RenewWriteLease.write_lease",
        )?))
    }

    fn get_inode(
        &self,
        inode_id: &crate::dfs::InodeId,
    ) -> afs_error::Result<crate::dfs::InodeRecord> {
        let reply = self
            .run(
                self.client()
                    .get_inode(afs_protocol::meta::GetDfsInodeRequest {
                        inode_id: inode_id.0.clone(),
                    }),
            )?
            .into_inner();
        required(reply.inode, "GetDfsInode.inode").and_then(domain_dfs_inode)
    }

    fn get_file_version(
        &self,
        version_id: &crate::dfs::FileVersionId,
    ) -> afs_error::Result<(crate::dfs::FileVersion, crate::dfs::LayoutRoot)> {
        let reply = self
            .run(
                self.client()
                    .get_file_version(afs_protocol::meta::GetFileVersionRequest {
                        version_id: version_id.0.clone(),
                    }),
            )?
            .into_inner();
        Ok((
            domain_dfs_version(required(reply.version, "GetFileVersion.version")?),
            domain_dfs_layout(required(reply.layout, "GetFileVersion.layout")?),
        ))
    }

    fn sync_inode_metadata(
        &self,
        sync: crate::dfs::SyncInodeMetadata,
    ) -> afs_error::Result<crate::dfs::InodeRecord> {
        let reply = self
            .run(
                self.client().sync_inode_metadata(
                    afs_protocol::meta::SyncDfsInodeMetadataRequest {
                        caller_id: self.node_id.clone(),
                        operation_id: sync.operation_id.0,
                        inode_id: sync.inode_id.0,
                        write_lease: Some(wire_dfs_write_lease(sync.write_lease)),
                        expected_inode_revision: sync.expected_inode_revision,
                        expected_head_version_id: sync
                            .expected_head_version
                            .map_or_else(String::new, |id| id.0),
                        metadata_delta: Some(wire_dfs_metadata_delta(sync.metadata_delta)),
                    },
                ),
            )?
            .into_inner();
        required(reply.inode, "SyncInodeMetadata.inode").and_then(domain_dfs_inode)
    }

    fn commit_file_version(
        &self,
        commit: crate::dfs::CommitFileVersion,
    ) -> afs_error::Result<crate::dfs::InodeRecord> {
        let reply = self
            .run(
                self.client()
                    .commit_file_version(afs_protocol::meta::CommitFileVersionRequest {
                        caller_id: self.node_id.clone(),
                        operation_id: commit.operation_id.0,
                        inode_id: commit.inode_id.0,
                        write_lease: Some(wire_dfs_write_lease(commit.write_lease)),
                        expected_inode_revision: commit.expected_inode_revision,
                        expected_head_version_id: commit
                            .expected_head_version
                            .map_or_else(String::new, |id| id.0),
                        version: Some(wire_dfs_version(commit.file_version)),
                        layout: Some(wire_dfs_layout(commit.layout_root)),
                        chunk_receipts: commit
                            .chunk_receipts
                            .into_iter()
                            .map(wire_chunk_receipt)
                            .collect(),
                        metadata_delta: Some(wire_dfs_metadata_delta(commit.metadata_delta)),
                    }),
            )?
            .into_inner();
        required(reply.inode, "CommitFileVersion.inode").and_then(domain_dfs_inode)
    }
}

#[cfg(feature = "dfs")]
fn wire_dfs_attributes(
    attributes: crate::dfs::InodeAttributes,
) -> afs_protocol::meta::DfsInodeAttributes {
    afs_protocol::meta::DfsInodeAttributes {
        mode: attributes.mode,
        uid: attributes.uid,
        gid: attributes.gid,
        nlink: attributes.nlink,
        atime_unix_ms: attributes.atime_unix_ms,
        mtime_unix_ms: attributes.mtime_unix_ms,
        ctime_unix_ms: attributes.ctime_unix_ms,
    }
}

#[cfg(feature = "dfs")]
fn domain_dfs_inode(
    inode: afs_protocol::meta::DfsInodeRecord,
) -> afs_error::Result<crate::dfs::InodeRecord> {
    let attributes = required(inode.attributes, "DfsInodeRecord.attributes")?;
    let kind = match afs_protocol::meta::DfsInodeKind::try_from(inode.kind) {
        Ok(afs_protocol::meta::DfsInodeKind::Regular) => crate::dfs::InodeKind::Regular,
        Ok(afs_protocol::meta::DfsInodeKind::Directory) => crate::dfs::InodeKind::Directory,
        Ok(afs_protocol::meta::DfsInodeKind::Symlink) => crate::dfs::InodeKind::Symlink,
        _ => {
            return Err(Error::coded(
                afs_error::CLIENT_PROTOCOL_VIOLATION,
                "DfsInodeRecord has an invalid inode kind",
            ));
        }
    };
    Ok(crate::dfs::InodeRecord {
        namespace_id: crate::dfs::NamespaceId::new(inode.namespace_id),
        inode_id: crate::dfs::InodeId::new(inode.inode_id),
        kind,
        attributes: crate::dfs::InodeAttributes {
            mode: attributes.mode,
            uid: attributes.uid,
            gid: attributes.gid,
            nlink: attributes.nlink,
            atime_unix_ms: attributes.atime_unix_ms,
            mtime_unix_ms: attributes.mtime_unix_ms,
            ctime_unix_ms: attributes.ctime_unix_ms,
        },
        head_version: (!inode.head_version_id.is_empty())
            .then(|| crate::dfs::FileVersionId::new(inode.head_version_id)),
        revision: inode.revision,
    })
}

#[cfg(feature = "dfs")]
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

#[cfg(feature = "dfs")]
fn domain_dfs_version(version: afs_protocol::meta::DfsFileVersion) -> crate::dfs::FileVersion {
    crate::dfs::FileVersion {
        id: crate::dfs::FileVersionId::new(version.version_id),
        inode_id: crate::dfs::InodeId::new(version.inode_id),
        parent_version: (!version.parent_version_id.is_empty())
            .then(|| crate::dfs::FileVersionId::new(version.parent_version_id)),
        length: version.length,
        layout_root: crate::dfs::LayoutRootId::new(version.layout_root_id),
        created_at_unix_ms: version.created_at_unix_ms,
    }
}

#[cfg(feature = "dfs")]
fn wire_dfs_layout(layout: crate::dfs::LayoutRoot) -> afs_protocol::meta::DfsLayoutRoot {
    afs_protocol::meta::DfsLayoutRoot {
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

#[cfg(feature = "dfs")]
fn domain_dfs_layout(layout: afs_protocol::meta::DfsLayoutRoot) -> crate::dfs::LayoutRoot {
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

#[cfg(feature = "dfs")]
fn wire_dfs_write_lease(lease: crate::dfs::WriteLease) -> afs_protocol::meta::DfsWriteLease {
    afs_protocol::meta::DfsWriteLease {
        inode_id: lease.inode_id.0,
        owner_node_id: lease.owner_node_id,
        owner_session_id: lease.owner_session_id,
        lease_epoch: lease.lease_epoch,
        expires_at_unix_ms: lease.expires_at_unix_ms,
    }
}

#[cfg(feature = "dfs")]
fn domain_dfs_write_lease(lease: afs_protocol::meta::DfsWriteLease) -> crate::dfs::WriteLease {
    crate::dfs::WriteLease {
        inode_id: crate::dfs::InodeId::new(lease.inode_id),
        owner_node_id: lease.owner_node_id,
        owner_session_id: lease.owner_session_id,
        lease_epoch: lease.lease_epoch,
        expires_at_unix_ms: lease.expires_at_unix_ms,
    }
}

#[cfg(feature = "dfs")]
fn wire_dfs_metadata_delta(
    delta: crate::dfs::CommitMetadataDelta,
) -> afs_protocol::meta::DfsCommitMetadataDelta {
    afs_protocol::meta::DfsCommitMetadataDelta {
        mode: match delta.mode {
            crate::dfs::CommitMetadataMode::DataOnly => {
                afs_protocol::meta::DfsCommitMetadataMode::DataOnly as i32
            }
            crate::dfs::CommitMetadataMode::Full => {
                afs_protocol::meta::DfsCommitMetadataMode::Full as i32
            }
        },
        mtime_unix_ms: delta.mtime_unix_ms.unwrap_or_default(),
        ctime_unix_ms: delta.ctime_unix_ms.unwrap_or_default(),
    }
}

#[cfg(feature = "dfs")]
fn wire_chunk_receipt(receipt: crate::dfs::ChunkReceipt) -> afs_protocol::meta::DfsChunkReceipt {
    afs_protocol::meta::DfsChunkReceipt {
        operation_id: receipt.operation_id.0,
        chunk_id: receipt.chunk.id.0,
        chunk_length: receipt.chunk.length,
        content_digest: receipt.chunk.content_digest.bytes.to_vec(),
        content_digest_algorithm: wire_digest_algorithm(receipt.chunk.content_digest.algorithm),
        placement_revision: receipt.placement_revision,
        placement_epoch: receipt.placement_epoch,
        replica_group_id: receipt.replica_group_id.0,
        durable_acks: receipt
            .durable_acks
            .into_iter()
            .map(wire_replica_ack)
            .collect(),
    }
}

#[cfg(feature = "dfs")]
fn wire_replica_ack(ack: crate::dfs::ReplicaAck) -> afs_protocol::meta::DfsReplicaAck {
    afs_protocol::meta::DfsReplicaAck {
        operation_id: ack.operation_id.0,
        chunk_id: ack.chunk_id.0,
        placement_revision: ack.placement_revision,
        placement_epoch: ack.placement_epoch,
        node_id: ack.node_id,
        node_epoch: ack.node_epoch,
        device_id: ack.device_id,
        device_epoch: ack.device_epoch,
        catalog_revision: ack.catalog_revision,
        persisted_bytes: ack.persisted_bytes,
        verified_digest: ack.verified_digest.bytes.to_vec(),
        verified_digest_algorithm: wire_digest_algorithm(ack.verified_digest.algorithm),
    }
}

#[cfg(feature = "dfs")]
fn wire_digest_algorithm(algorithm: crate::dfs::DigestAlgorithm) -> i32 {
    match algorithm {
        crate::dfs::DigestAlgorithm::Blake3 => {
            afs_protocol::meta::DfsDigestAlgorithm::Blake3 as i32
        }
    }
}

#[cfg(feature = "dfs")]
fn domain_placement_snapshot(
    snapshot: afs_protocol::meta::DfsPlacementSnapshot,
) -> afs_error::Result<crate::dfs::PlacementSnapshot> {
    let replication = required(snapshot.replication, "DfsPlacementSnapshot.replication")?;
    let local_copy = match afs_protocol::meta::DfsLocalCopyPolicy::try_from(replication.local_copy)
    {
        Ok(afs_protocol::meta::DfsLocalCopyPolicy::Required) => {
            crate::dfs::LocalCopyPolicy::Required
        }
        Ok(afs_protocol::meta::DfsLocalCopyPolicy::Preferred) => {
            crate::dfs::LocalCopyPolicy::Preferred
        }
        Ok(afs_protocol::meta::DfsLocalCopyPolicy::NotRequired) => {
            crate::dfs::LocalCopyPolicy::NotRequired
        }
        _ => {
            return Err(Error::coded(
                afs_error::CLIENT_PROTOCOL_VIOLATION,
                "DfsPlacementSnapshot has an invalid local-copy policy",
            ));
        }
    };
    let replication = crate::dfs::ReplicationConfig {
        desired_copies: u16::try_from(replication.desired_copies).map_err(|_| {
            Error::coded(
                afs_error::CLIENT_PROTOCOL_VIOLATION,
                "desired_copies exceeds u16",
            )
        })?,
        sync_required_copies: u16::try_from(replication.sync_required_copies).map_err(|_| {
            Error::coded(
                afs_error::CLIENT_PROTOCOL_VIOLATION,
                "sync_required_copies exceeds u16",
            )
        })?,
        min_distinct_nodes: u16::try_from(replication.min_distinct_nodes).map_err(|_| {
            Error::coded(
                afs_error::CLIENT_PROTOCOL_VIOLATION,
                "min_distinct_nodes exceeds u16",
            )
        })?,
        min_distinct_failure_domains: u16::try_from(replication.min_distinct_failure_domains)
            .map_err(|_| {
                Error::coded(
                    afs_error::CLIENT_PROTOCOL_VIOLATION,
                    "min_distinct_failure_domains exceeds u16",
                )
            })?,
        local_copy,
    };
    let replica_groups = snapshot
        .replica_groups
        .into_iter()
        .map(|group| {
            let targets = group
                .targets
                .into_iter()
                .map(|target| {
                    let device = required(target.device, "DfsReplicaTarget.device")?;
                    Ok(crate::dfs::ReplicaTarget {
                        node_id: target.node_id,
                        node_epoch: target.node_epoch,
                        data_endpoint: target.data_endpoint,
                        device: crate::dfs::StorageDeviceDescriptor {
                            device_id: device.device_id,
                            device_epoch: device.device_epoch,
                            catalog_revision: device.catalog_revision,
                            failure_domain: device.failure_domain,
                        },
                    })
                })
                .collect::<afs_error::Result<Vec<_>>>()?;
            Ok(crate::dfs::ReplicaGroup {
                id: crate::dfs::ReplicaGroupId::new(group.replica_group_id),
                placement_epoch: group.placement_epoch,
                targets,
            })
        })
        .collect::<afs_error::Result<Vec<_>>>()?;
    let result = crate::dfs::PlacementSnapshot {
        revision: snapshot.revision,
        replication,
        replica_groups,
    };
    if !result.replication.is_valid() {
        return Err(Error::coded(
            afs_error::CLIENT_PROTOCOL_VIOLATION,
            "Meta returned an invalid DFS replication config",
        ));
    }
    Ok(result)
}

#[cfg(feature = "dfs")]
fn domain_dfs_chunk_sources(
    value: afs_protocol::meta::DfsChunkSources,
) -> afs_error::Result<crate::dfs::ChunkSources> {
    Ok(crate::dfs::ChunkSources {
        chunk_id: crate::dfs::ChunkId::new(value.chunk_id),
        sources: value
            .sources
            .into_iter()
            .map(domain_dfs_source_candidate)
            .collect::<afs_error::Result<Vec<_>>>()?,
    })
}

#[cfg(feature = "dfs")]
fn domain_dfs_source_candidate(
    value: afs_protocol::meta::DfsSourceCandidate,
) -> afs_error::Result<crate::dfs::SourceCandidate> {
    Ok(crate::dfs::SourceCandidate {
        copy_id: crate::dfs::CopyId::new(value.copy_id),
        chunk_id: crate::dfs::ChunkId::new(value.chunk_id),
        role: domain_dfs_copy_role(value.role)?,
        state: domain_dfs_copy_state(value.state)?,
        location: domain_dfs_copy_location(required(
            value.location,
            "DfsSourceCandidate.location",
        )?)?,
        data_endpoint: value.data_endpoint,
        load_hint: value.load_hint,
        read_grant: domain_dfs_read_grant(required(
            value.read_grant,
            "DfsSourceCandidate.read_grant",
        )?),
    })
}

#[cfg(feature = "dfs")]
fn domain_dfs_copy_role(value: i32) -> afs_error::Result<crate::dfs::CopyRole> {
    match afs_protocol::meta::DfsCopyRole::try_from(value) {
        Ok(afs_protocol::meta::DfsCopyRole::DurableReplica) => {
            Ok(crate::dfs::CopyRole::DurableReplica)
        }
        Ok(afs_protocol::meta::DfsCopyRole::VerifiedCache) => {
            Ok(crate::dfs::CopyRole::VerifiedCache)
        }
        Ok(afs_protocol::meta::DfsCopyRole::ExternalCommitted) => {
            Ok(crate::dfs::CopyRole::ExternalCommitted)
        }
        _ => Err(Error::coded(
            afs_error::CLIENT_PROTOCOL_VIOLATION,
            "DfsSourceCandidate has an invalid copy role",
        )),
    }
}

#[cfg(feature = "dfs")]
fn domain_dfs_copy_state(value: i32) -> afs_error::Result<crate::dfs::CopyState> {
    match afs_protocol::meta::DfsCopyState::try_from(value) {
        Ok(afs_protocol::meta::DfsCopyState::Ready) => Ok(crate::dfs::CopyState::Ready),
        Ok(afs_protocol::meta::DfsCopyState::Corrupt) => Ok(crate::dfs::CopyState::Corrupt),
        Ok(afs_protocol::meta::DfsCopyState::Deleting) => Ok(crate::dfs::CopyState::Deleting),
        _ => Err(Error::coded(
            afs_error::CLIENT_PROTOCOL_VIOLATION,
            "DfsSourceCandidate has an invalid copy state",
        )),
    }
}

#[cfg(feature = "dfs")]
fn domain_dfs_copy_location(
    value: afs_protocol::meta::DfsCopyLocation,
) -> afs_error::Result<crate::dfs::CopyLocation> {
    match value.location {
        Some(afs_protocol::meta::dfs_copy_location::Location::Node(node)) => {
            Ok(crate::dfs::CopyLocation::Node {
                node_id: node.node_id,
                node_epoch: node.node_epoch,
                device_id: node.device_id,
                device_epoch: node.device_epoch,
                catalog_revision: node.catalog_revision,
            })
        }
        Some(afs_protocol::meta::dfs_copy_location::Location::External(external)) => {
            Ok(crate::dfs::CopyLocation::External {
                store_id: external.store_id,
                object_key: external.object_key,
                object_revision: external.object_revision,
            })
        }
        None => Err(Error::coded(
            afs_error::CLIENT_PROTOCOL_VIOLATION,
            "DfsCopyLocation is empty",
        )),
    }
}

#[cfg(feature = "dfs")]
fn domain_dfs_read_grant(value: afs_protocol::meta::DfsReadGrant) -> crate::dfs::DfsReadGrant {
    crate::dfs::DfsReadGrant {
        namespace_id: crate::dfs::NamespaceId::new(value.namespace_id),
        file_version_id: crate::dfs::FileVersionId::new(value.file_version_id),
        layout_root_id: crate::dfs::LayoutRootId::new(value.layout_root_id),
        caller_node_id: value.caller_node_id,
        caller_node_epoch: value.caller_node_epoch,
        expires_at_unix_ms: value.expires_at_unix_ms,
        fence: value.fence,
        token: value.token,
    }
}

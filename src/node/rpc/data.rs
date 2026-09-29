//! Node → Node 文件数据操作 Handler。
//!
//! 这个文件统一承载 Node 间入站数据服务：前半实现 node_data.proto 的诊断
//! handler，后半实现 OwnerFs 文件命令；二者不共用无授权的 diagnostics Storage 入口。
//! 对诊断请求而言，它是“共同服务端入口”：
//! 无论远端客户端选择 gRPC inline 还是 RDMA，最终都会进入这里，完成相同的
//! Storage read/write 诊断语义。区别只在文件内容怎么搬：
//! - gRPC inline：内容直接放在 DataReadReply/DataWriteRequest 的 proto bytes 里；
//! - RDMA one-sided：proto 只放命令、session_id、offset/len，不放文件内容。
//!
//! 单边方向要特别记住：
//! - 远端写文件：客户端先把待写 bytes 放入自己的 MR，服务端执行 RDMA READ 拉取，
//!   然后写入本地 Storage；
//! - 远端读文件：服务端先从 Storage 读出 bytes 放入自己的 MR，执行 RDMA WRITE 推到
//!   客户端 MR，客户端再从本地 MR 拷贝给调用者。
//!
//! CQ completion 只证明 DMA 完成，不证明文件落盘；文件成功标准仍由 Storage/业务层决定。

use afs_transport::grpc::error_status::{coded_status, error_to_status};
#[cfg(feature = "dfs")]
use std::pin::Pin;
use std::sync::Arc;
#[cfg(feature = "rdma")]
use std::sync::atomic::Ordering;

use afs_protocol::node_data::{
    DataReadReply, DataReadRequest, DataTransfer, DataWriteReply, DataWriteRequest,
    node_data_server::{NodeData, NodeDataServer},
};
#[cfg(feature = "dfs")]
use afs_protocol::node_data::{
    DfsConfirmReplicaRequest, DfsPutReplicaFrame, DfsPutReplicaRdmaRequest, DfsPutReplicaReply,
    DfsReadRangesCompletion, DfsReadRangesFrame, DfsReadRangesHeader, DfsReadRangesRequest,
    DfsReplicaAck,
    dfs_chunks_server::{DfsChunks, DfsChunksServer},
    dfs_read_ranges_frame,
};
use afs_tracing::Instrument;
#[cfg(feature = "dfs")]
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

#[cfg(feature = "dfs")]
use crate::node::chunk::LocalChunkStore;
use crate::node::{
    rpc::control::RdmaSessionRegistry,
    storage::{MAX_TRANSFER_BYTES, Storage, StorageError},
};

/// node_data 的服务端实现。
///
/// `storage` 是真实文件操作入口；`sessions` 只在 RDMA 模式下把 session_id 转为
/// 服务端 endpoint。这里不区分 OwnerFs/DFS，只实现第一版诊断用的 8-byte/文件数据路径。
#[derive(Clone)]
pub struct NodeDataService {
    storage: Arc<Storage>,
    sessions: RdmaSessionRegistry,
}

impl NodeDataService {
    #[must_use]
    pub fn new(storage: Arc<Storage>, sessions: RdmaSessionRegistry) -> Self {
        Self { storage, sessions }
    }
}

pub fn make_data_server(
    storage: Arc<Storage>,
    sessions: RdmaSessionRegistry,
) -> NodeDataServer<NodeDataService> {
    NodeDataServer::new(NodeDataService::new(storage, sessions))
}

/// DFS replica RPC boundary.
///
/// The service is registered now so gRPC streaming and RDMA have stable wire
/// contracts. RN execution remains fail-fast until replica staging, digest
/// verification, catalog persistence and idempotent replay are implemented as
/// one state machine; returning `UNIMPLEMENTED` here guarantees no partial
/// replica can be mistaken for a durable acknowledgement.
#[cfg(feature = "dfs")]
#[derive(Clone)]
pub struct DfsChunksService {
    #[allow(dead_code)]
    local_chunks: Option<Arc<LocalChunkStore>>,
}

#[cfg(feature = "dfs")]
#[must_use]
pub fn make_dfs_chunks_server(
    local_chunks: Option<Arc<LocalChunkStore>>,
) -> DfsChunksServer<DfsChunksService> {
    DfsChunksServer::new(DfsChunksService { local_chunks })
}

#[cfg(feature = "dfs")]
#[tonic::async_trait]
impl DfsChunks for DfsChunksService {
    type ReadRangesStream =
        Pin<Box<dyn Stream<Item = Result<DfsReadRangesFrame, Status>> + Send + 'static>>;

    async fn put_replica_stream(
        &self,
        _request: Request<tonic::Streaming<DfsPutReplicaFrame>>,
    ) -> Result<Response<DfsPutReplicaReply>, Status> {
        Err(Status::unimplemented(
            "DFS gRPC replica transfer is not implemented; request rejected before side effects",
        ))
    }

    async fn put_replica_rdma(
        &self,
        _request: Request<DfsPutReplicaRdmaRequest>,
    ) -> Result<Response<DfsPutReplicaReply>, Status> {
        Err(Status::unimplemented(
            "DFS RDMA replica transfer is not implemented; request rejected before side effects",
        ))
    }

    async fn confirm_replica(
        &self,
        _request: Request<DfsConfirmReplicaRequest>,
    ) -> Result<Response<DfsReplicaAck>, Status> {
        Err(Status::unimplemented(
            "DFS replica confirmation is not implemented; no durable acknowledgement exists",
        ))
    }

    async fn read_ranges(
        &self,
        request: Request<DfsReadRangesRequest>,
    ) -> Result<Response<Self::ReadRangesStream>, Status> {
        let local = self.local_chunks.clone().ok_or_else(|| {
            coded_status(
                afs_error::NODE_TRANSFER_UNAVAILABLE,
                "DFS local ChunkStore is not available",
            )
        })?;
        let request = request.into_inner();
        validate_dfs_read_request(&request)?;
        let mut frames = Vec::new();
        for op in request.operations {
            let length = usize::try_from(op.length).map_err(|_| {
                coded_status(
                    afs_error::NODE_TRANSFER_INVALID,
                    "DFS read range is too large",
                )
            })?;
            validate_length_u64(op.length)?;
            let mut data = vec![0; length];
            let read = local
                .read_at(
                    &crate::dfs::ChunkId::new(op.chunk_id.clone()),
                    op.chunk_offset,
                    &mut data,
                )
                .map_err(error_to_status)?;
            if read != length {
                return Err(coded_status(
                    afs_error::NODE_TRANSFER_CORRUPT_DATA,
                    "local Chunk ended before the requested DFS range",
                ));
            }
            frames.push(Ok(DfsReadRangesFrame {
                body: Some(dfs_read_ranges_frame::Body::Header(DfsReadRangesHeader {
                    read_id: request.read_id.clone(),
                    attempt_id: request.attempt_id.clone(),
                    operation_index: op.operation_index,
                    chunk_id: op.chunk_id,
                    chunk_offset: op.chunk_offset,
                    length: op.length,
                    source_copy_id: op.source_copy_id.clone(),
                })),
            }));
            frames.push(Ok(DfsReadRangesFrame {
                body: Some(dfs_read_ranges_frame::Body::Data(data)),
            }));
            frames.push(Ok(DfsReadRangesFrame {
                body: Some(dfs_read_ranges_frame::Body::Completion(
                    DfsReadRangesCompletion {
                        read_id: request.read_id.clone(),
                        attempt_id: request.attempt_id.clone(),
                        operation_index: op.operation_index,
                        source_copy_id: op.source_copy_id,
                        transferred_bytes: op.length,
                        range_checksum: Vec::new(),
                        range_checksum_algorithm: 0,
                    },
                )),
            }));
        }
        Ok(Response::new(Box::pin(tokio_stream::iter(frames))))
    }
}

#[tonic::async_trait]
impl NodeData for NodeDataService {
    /// 服务端处理“远端读文件”。
    ///
    /// gRPC inline：直接返回 `data`。
    /// RDMA：先校验 session，再读 Storage，然后服务端 RDMA WRITE 到客户端 MR，
    /// reply 只返回长度，`data` 为空。
    async fn read(
        &self,
        request: Request<DataReadRequest>,
    ) -> Result<Response<DataReadReply>, Status> {
        async move {
            let request = request.into_inner();
            validate_length(request.length)?;
            let transfer = transfer_mode(request.transfer)?;
            if transfer == DataTransfer::Unspecified {
                return Err(coded_status(
                    afs_error::NODE_TRANSFER_INVALID,
                    "transfer is required",
                ));
            }
            validate_read_session(&self.sessions, request.session_id, transfer).await?;
            let data = self
                .storage
                .read(&request.name, request.offset, request.length)
                .await
                .map_err(storage_status)?;
            debug_assert_eq!(data.len(), request.length as usize);
            let actual_len = u32::try_from(data.len()).map_err(|_| {
                coded_status(afs_error::NODE_TRANSFER_INTERNAL, "read length exceeds u32")
            })?;
            match transfer {
                DataTransfer::GrpcInline => Ok(Response::new(DataReadReply {
                    length: actual_len,
                    data,
                })),
                DataTransfer::RdmaOneSided => {
                    rdma_write_to_client(&self.sessions, request.session_id, data).await?;
                    Ok(Response::new(DataReadReply {
                        length: actual_len,
                        data: Vec::new(),
                    }))
                }
                DataTransfer::Unspecified => unreachable!("checked above"),
            }
        }
        .instrument(afs_tracing::tracing::info_span!("node.data.read"))
        .await
    }

    /// 服务端处理“远端写文件”。
    ///
    /// gRPC inline：从 request.data 取内容。
    /// RDMA：request.data 必须为空；服务端 RDMA READ 从客户端 MR 拉取内容，
    /// 再写入 Storage。
    async fn write(
        &self,
        request: Request<DataWriteRequest>,
    ) -> Result<Response<DataWriteReply>, Status> {
        async move {
            let request = request.into_inner();
            let data = match transfer_mode(request.transfer)? {
                DataTransfer::GrpcInline => {
                    if !request.length.eq(&0) && request.length as usize != request.data.len() {
                        return Err(coded_status(
                            afs_error::NODE_TRANSFER_INVALID,
                            "inline length/data mismatch",
                        ));
                    }
                    validate_length(request.data.len() as u32)?;
                    request.data
                }
                DataTransfer::RdmaOneSided => {
                    validate_length(request.length)?;
                    if !request.data.is_empty() {
                        return Err(coded_status(
                            afs_error::NODE_TRANSFER_INVALID,
                            "RDMA write must not include inline data",
                        ));
                    }
                    rdma_read_from_client(
                        &self.sessions,
                        request.session_id,
                        request.length as usize,
                    )
                    .await?
                }
                DataTransfer::Unspecified => {
                    return Err(coded_status(
                        afs_error::NODE_TRANSFER_INVALID,
                        "transfer is required",
                    ));
                }
            };
            let written = self
                .storage
                .write(&request.name, request.offset, data)
                .await
                .map_err(storage_status)?;
            Ok(Response::new(DataWriteReply {
                written: written as u32,
            }))
        }
        .instrument(afs_tracing::tracing::info_span!("node.data.write"))
        .await
    }
}

fn validate_length(length: u32) -> Result<(), Status> {
    if length as usize > MAX_TRANSFER_BYTES {
        return Err(coded_status(
            afs_error::NODE_TRANSFER_INVALID,
            "transfer exceeds 1MiB",
        ));
    }
    Ok(())
}

#[cfg(feature = "dfs")]
fn validate_length_u64(length: u64) -> Result<(), Status> {
    if length > MAX_TRANSFER_BYTES as u64 {
        return Err(coded_status(
            afs_error::NODE_TRANSFER_INVALID,
            "transfer exceeds 1MiB",
        ));
    }
    Ok(())
}

#[cfg(feature = "dfs")]
fn validate_dfs_read_request(request: &DfsReadRangesRequest) -> Result<(), Status> {
    if request.read_id.is_empty()
        || request.attempt_id.is_empty()
        || request.file_version_id.is_empty()
        || request.layout_root_id.is_empty()
        || request.operations.is_empty()
    {
        return Err(coded_status(
            afs_error::NODE_TRANSFER_INVALID,
            "DFS read request is incomplete",
        ));
    }
    let grant = request.grant.as_ref().ok_or_else(|| {
        coded_status(
            afs_error::NODE_TRANSFER_INVALID,
            "DFS read request is missing grant",
        )
    })?;
    if grant.namespace_id.is_empty()
        || grant.file_version_id != request.file_version_id
        || grant.layout_root_id != request.layout_root_id
        || grant.caller_node_id.is_empty()
        || grant.expires_at_unix_ms == 0
        || grant.token.is_empty()
    {
        return Err(coded_status(
            afs_error::NODE_TRANSFER_INVALID,
            "DFS read grant is structurally invalid",
        ));
    }
    for op in &request.operations {
        if op.chunk_id.is_empty() || op.source_copy_id.is_empty() {
            return Err(coded_status(
                afs_error::NODE_TRANSFER_INVALID,
                "DFS read operation is incomplete",
            ));
        }
        validate_length_u64(op.length)?;
    }
    Ok(())
}

fn transfer_mode(value: i32) -> Result<DataTransfer, Status> {
    DataTransfer::try_from(value)
        .map_err(|_| coded_status(afs_error::NODE_TRANSFER_INVALID, "unknown transfer mode"))
}

/// RDMA read 必须先校验 session，再碰 Storage。
///
/// 这样旧 session/poisoned session 不会因为文件不存在等业务错误掩盖掉传输授权错误。
async fn validate_read_session(
    sessions: &RdmaSessionRegistry,
    session_id: u64,
    transfer: DataTransfer,
) -> Result<(), Status> {
    if transfer == DataTransfer::RdmaOneSided {
        sessions.session(session_id).await?;
    }
    Ok(())
}

fn storage_status(error: StorageError) -> Status {
    error_to_status(error.into())
}

/// 写文件 RDMA 路径：服务端从客户端 MR 拉取 bytes。
///
/// verbs 方向是 server RDMA READ；函数名中的 from_client 表达业务视角。
#[cfg(feature = "rdma")]
async fn rdma_read_from_client(
    sessions: &RdmaSessionRegistry,
    session_id: u64,
    len: usize,
) -> Result<Vec<u8>, Status> {
    let session = sessions.session(session_id).await?;
    let endpoint = session.endpoint.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut endpoint = endpoint.blocking_lock();
        endpoint.transfer_read(len).map_err(rdma_status)?;
        endpoint.get_local(len).map_err(rdma_status)
    })
    .await
    .map_err(|error| coded_status(afs_error::NODE_TRANSFER_INTERNAL, error.to_string()))?;
    if result.is_err() {
        session.poisoned.store(true, Ordering::SeqCst);
    }
    result
}

/// 未编译 RDMA 时明确拒绝单边写文件请求，不在这里自动重放为 gRPC。
#[cfg(not(feature = "rdma"))]
async fn rdma_read_from_client(
    _sessions: &RdmaSessionRegistry,
    _session_id: u64,
    _len: usize,
) -> Result<Vec<u8>, Status> {
    Err(coded_status(
        afs_error::NODE_TRANSFER_UNSUPPORTED,
        "RDMA feature is not enabled",
    ))
}

/// 读文件 RDMA 路径：服务端把 bytes 推到客户端 MR。
///
/// verbs 方向是 server RDMA WRITE；函数名中的 to_client 表达业务视角。
#[cfg(feature = "rdma")]
async fn rdma_write_to_client(
    sessions: &RdmaSessionRegistry,
    session_id: u64,
    data: Vec<u8>,
) -> Result<(), Status> {
    let session = sessions.session(session_id).await?;
    let endpoint = session.endpoint.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut endpoint = endpoint.blocking_lock();
        endpoint.put_local(&data).map_err(rdma_status)?;
        endpoint.transfer_write(data.len()).map_err(rdma_status)
    })
    .await
    .map_err(|error| coded_status(afs_error::NODE_TRANSFER_INTERNAL, error.to_string()))?;
    if result.is_err() {
        session.poisoned.store(true, Ordering::SeqCst);
    }
    result
}

/// 未编译 RDMA 时明确拒绝单边读文件请求。
#[cfg(not(feature = "rdma"))]
async fn rdma_write_to_client(
    _sessions: &RdmaSessionRegistry,
    _session_id: u64,
    _data: Vec<u8>,
) -> Result<(), Status> {
    Err(coded_status(
        afs_error::NODE_TRANSFER_UNSUPPORTED,
        "RDMA feature is not enabled",
    ))
}

#[cfg(feature = "rdma")]
fn rdma_status(error: afs_transport::rdma::RdmaError) -> Status {
    coded_status(afs_error::NODE_TRANSFER_UNAVAILABLE, error.to_string())
}

// OwnerFs Peer 文件服务与诊断服务共处 data.rs；feature 属性只控制编译，
// 不形成独立文件或公开模块层级。
#[cfg(feature = "ownerfs")]
use std::{
    collections::HashMap,
    ffi::OsString,
    net::SocketAddr,
    os::unix::ffi::OsStringExt,
    time::{Duration, UNIX_EPOCH},
};

#[cfg(feature = "ownerfs")]
use afs_protocol::node_data::{
    OwnerCreateReply, OwnerCreateRequest, OwnerDirEntry, OwnerDirectoryHandle, OwnerFlushReply,
    OwnerFlushRequest, OwnerFsyncDirReply, OwnerFsyncDirRequest, OwnerFsyncReply,
    OwnerFsyncRequest, OwnerGetAttrReply, OwnerGetAttrRequest, OwnerLinkReply, OwnerLinkRequest,
    OwnerLookupReply, OwnerLookupRequest, OwnerMkdirReply, OwnerMkdirRequest, OwnerOpenReply,
    OwnerOpenRequest, OwnerOpendirReply, OwnerOpendirRequest, OwnerReadReply, OwnerReadRequest,
    OwnerReaddirReply, OwnerReaddirRequest, OwnerReadlinkReply, OwnerReadlinkRequest,
    OwnerReleaseDirReply, OwnerReleaseDirRequest, OwnerReleaseReply, OwnerReleaseRequest,
    OwnerRenameReply, OwnerRenameRequest, OwnerRmdirReply, OwnerRmdirRequest, OwnerSetAttrReply,
    OwnerSetAttrRequest, OwnerSymlinkReply, OwnerSymlinkRequest, OwnerUnlinkReply,
    OwnerUnlinkRequest, OwnerWriteReply, OwnerWriteRequest,
    owner_files_server::{OwnerFiles, OwnerFilesServer},
};
#[cfg(feature = "ownerfs")]
use tonic::metadata::MetadataMap;

#[cfg(feature = "ownerfs")]
use crate::node::vfs::{
    ownerfs::{
        OwnerFsPeerExecutor,
        files::{FileIdentity, RemoteDirectory, RemoteFile},
        root::{PresentedRootAccess, RootId},
    },
    types::{AttributeChange, FileAttributes, FileKind, RenameFlags},
};

/// Home 侧真实文件操作接口。
///
/// 该 trait 保持同步形态，因为 OwnerFs/FUSE 后端目前是同步 VFS 合同，且
/// RootMeta adapter 可能在阻塞线程里等待异步 gRPC。生产多线程 runtime
/// 使用 `block_in_place` 处理短本机操作，使 Tokio 能调度其他任务，同时省去
/// 每次文件 RPC 投递阻塞线程池的开销；单线程测试 runtime 则使用 `spawn_blocking`。
/// `authenticated_peer_node_id` 来自通道认证，不能从请求里的 holder_node_id 复制。
#[cfg(feature = "ownerfs")]
pub trait OwnerFilesHandler: Send + Sync + 'static {
    fn lookup(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerLookupRequest,
    ) -> afs_error::Result<OwnerLookupReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Lookup"))
    }
    fn get_attr(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerGetAttrRequest,
    ) -> afs_error::Result<OwnerGetAttrReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.GetAttr"))
    }
    fn set_attr(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerSetAttrRequest,
    ) -> afs_error::Result<OwnerSetAttrReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.SetAttr"))
    }
    fn create(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerCreateRequest,
    ) -> afs_error::Result<OwnerCreateReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Create"))
    }
    fn mkdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerMkdirRequest,
    ) -> afs_error::Result<OwnerMkdirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Mkdir"))
    }
    fn unlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerUnlinkRequest,
    ) -> afs_error::Result<OwnerUnlinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Unlink"))
    }
    fn rmdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerRmdirRequest,
    ) -> afs_error::Result<OwnerRmdirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Rmdir"))
    }
    fn rename(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerRenameRequest,
    ) -> afs_error::Result<OwnerRenameReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Rename"))
    }
    fn open(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerOpenRequest,
    ) -> afs_error::Result<OwnerOpenReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Open"))
    }
    fn readlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReadlinkRequest,
    ) -> afs_error::Result<OwnerReadlinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Readlink"))
    }
    fn read(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReadRequest,
    ) -> afs_error::Result<OwnerReadReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Read"))
    }
    fn write(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerWriteRequest,
    ) -> afs_error::Result<OwnerWriteReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Write"))
    }
    fn flush(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFlushRequest,
    ) -> afs_error::Result<OwnerFlushReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Flush"))
    }
    fn fsync(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFsyncRequest,
    ) -> afs_error::Result<OwnerFsyncReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Fsync"))
    }
    fn release(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReleaseRequest,
    ) -> afs_error::Result<OwnerReleaseReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Release"))
    }
    fn opendir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerOpendirRequest,
    ) -> afs_error::Result<OwnerOpendirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Opendir"))
    }
    fn readdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReaddirRequest,
    ) -> afs_error::Result<OwnerReaddirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Readdir"))
    }
    fn fsync_dir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFsyncDirRequest,
    ) -> afs_error::Result<OwnerFsyncDirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.FsyncDir"))
    }
    fn release_dir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReleaseDirRequest,
    ) -> afs_error::Result<OwnerReleaseDirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.ReleaseDir"))
    }
    fn symlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerSymlinkRequest,
    ) -> afs_error::Result<OwnerSymlinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Symlink"))
    }
    fn link(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerLinkRequest,
    ) -> afs_error::Result<OwnerLinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Link"))
    }
}

/// Home-side OwnerFiles handler backed by the real OwnerFs peer executor.
///
/// This is the production adapter boundary: gRPC/RDMA command handlers stay in
/// `node/rpc`, while ordinary-file semantics, root-grant validation, stale
/// handle detection, and old-FD behavior stay in `OwnerFsPeerExecutor`.
#[derive(Clone)]
#[cfg(feature = "ownerfs")]
pub struct OwnerFsPeerHandler {
    executor: OwnerFsPeerExecutor,
}

#[cfg(feature = "ownerfs")]
impl OwnerFsPeerHandler {
    #[must_use]
    pub fn new(executor: OwnerFsPeerExecutor) -> Self {
        Self { executor }
    }
}

#[cfg(feature = "ownerfs")]
pub fn make_owner_files_handler(executor: OwnerFsPeerExecutor) -> Arc<dyn OwnerFilesHandler> {
    Arc::new(OwnerFsPeerHandler::new(executor))
}

#[cfg(feature = "ownerfs")]
impl OwnerFilesHandler for OwnerFsPeerHandler {
    fn lookup(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerLookupRequest,
    ) -> afs_error::Result<OwnerLookupReply> {
        let access = presented_access(request.access)?;
        let path_is_root = request.path.is_empty();
        let expected_parent = if path_is_root {
            request
                .expected_parent_identity
                .map(|identity| FileIdentity(identity.opaque))
        } else {
            Some(required_identity(
                request.expected_parent_identity,
                "OwnerLookupRequest missing expected_parent_identity",
            )?)
        };
        let entry = self.executor.lookup(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected_parent.as_ref(),
        )?;
        Ok(OwnerLookupReply {
            attr: Some(owner_attr(entry.identity, entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn get_attr(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerGetAttrRequest,
    ) -> afs_error::Result<OwnerGetAttrReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let remote = request
            .handle
            .map(|handle| remote_file_for_handle(&access, handle.opaque));
        let entry = self.executor.getattr(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
            remote.as_ref(),
        )?;
        Ok(OwnerGetAttrReply {
            attr: Some(owner_attr(entry.identity, entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn create(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerCreateRequest,
    ) -> afs_error::Result<OwnerCreateReply> {
        let access = presented_access(request.access)?;
        let expected_parent = required_identity(
            request.expected_parent,
            "OwnerCreateRequest missing expected_parent",
        )?;
        let created = self.executor.create(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            request.flags as i32,
            request.mode,
            &expected_parent,
        )?;
        Ok(OwnerCreateReply {
            handle: Some(afs_protocol::node_data::OwnerHandle {
                opaque: created.file.handle,
            }),
            attr: Some(owner_attr(created.entry.identity, created.entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn mkdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerMkdirRequest,
    ) -> afs_error::Result<OwnerMkdirReply> {
        let access = presented_access(request.access)?;
        let expected_parent = required_identity(
            request.expected_parent,
            "OwnerMkdirRequest missing expected_parent",
        )?;
        let entry = self.executor.mkdir(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            request.mode,
            &expected_parent,
        )?;
        Ok(OwnerMkdirReply {
            attr: Some(owner_attr(entry.identity, entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn set_attr(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerSetAttrRequest,
    ) -> afs_error::Result<OwnerSetAttrReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let remote = request
            .handle
            .map(|handle| remote_file_for_handle(&access, handle.opaque));
        let change = attribute_change(request.attr)?;
        let entry = self.executor.setattr(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
            remote.as_ref(),
            &change,
        )?;
        Ok(OwnerSetAttrReply {
            attr: Some(owner_attr(entry.identity, entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn unlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerUnlinkRequest,
    ) -> afs_error::Result<OwnerUnlinkReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let expected_parent = required_identity(
            request.expected_parent,
            "OwnerUnlinkRequest missing expected_parent",
        )?;
        self.executor.unlink(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
            &expected_parent,
        )?;
        Ok(OwnerUnlinkReply {})
    }

    fn rmdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerRmdirRequest,
    ) -> afs_error::Result<OwnerRmdirReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let expected_parent = required_identity(
            request.expected_parent,
            "OwnerRmdirRequest missing expected_parent",
        )?;
        self.executor.rmdir(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
            &expected_parent,
        )?;
        Ok(OwnerRmdirReply {})
    }

    fn rename(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerRenameRequest,
    ) -> afs_error::Result<OwnerRenameReply> {
        let access = presented_access(request.access)?;
        let expected_old = request
            .expected_old_identity
            .map(|identity| FileIdentity(identity.opaque));
        let expected_new = request
            .expected_new_identity
            .map(|identity| FileIdentity(identity.opaque));
        let expected_old_parent = required_identity(
            request.expected_old_parent,
            "OwnerRenameRequest missing expected_old_parent",
        )?;
        let expected_new_parent = required_identity(
            request.expected_new_parent,
            "OwnerRenameRequest missing expected_new_parent",
        )?;
        self.executor.rename(
            authenticated_peer_node_id,
            &access,
            &path_os(request.old_path),
            &path_os(request.new_path),
            expected_old.as_ref(),
            expected_new.as_ref(),
            &expected_old_parent,
            &expected_new_parent,
            RenameFlags(request.flags),
        )?;
        Ok(OwnerRenameReply {})
    }

    fn open(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerOpenRequest,
    ) -> afs_error::Result<OwnerOpenReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let flags = request.flags as i32;
        let (file, attributes, prefetched_data) = self.executor.open(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            flags,
            expected.as_ref(),
        )?;
        Ok(OwnerOpenReply {
            handle: Some(afs_protocol::node_data::OwnerHandle {
                opaque: file.handle,
            }),
            file_identity: Some(afs_protocol::node_data::FileIdentity {
                opaque: file.identity.0.clone(),
            }),
            owner_session_id: file.owner_session_id,
            attr: Some(owner_attr(file.identity, attributes)),
            prefetched_data,
        })
    }

    fn read(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReadRequest,
    ) -> afs_error::Result<OwnerReadReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerReadRequest missing handle")?;
        let mut out = vec![0_u8; request.length as usize];
        let read = self.executor.read(
            authenticated_peer_node_id,
            &access,
            &remote_file_for_handle(&access, handle.opaque),
            request.offset,
            &mut out,
        )?;
        out.truncate(read);
        Ok(OwnerReadReply {
            data: out,
            read: read as u32,
            eof: read < request.length as usize,
        })
    }

    fn write(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerWriteRequest,
    ) -> afs_error::Result<OwnerWriteReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerWriteRequest missing handle")?;
        let length = request.length as usize;
        if request.data.len() < length {
            return Err(protocol_error("OwnerWriteRequest data shorter than length"));
        }
        let written = self.executor.write(
            authenticated_peer_node_id,
            &access,
            &remote_file_for_handle(&access, handle.opaque),
            request.offset,
            &request.data[..length],
        )?;
        Ok(OwnerWriteReply {
            written: written as u32,
        })
    }

    fn flush(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFlushRequest,
    ) -> afs_error::Result<OwnerFlushReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerFlushRequest missing handle")?;
        self.executor.flush(
            authenticated_peer_node_id,
            &access,
            &remote_file_for_handle(&access, handle.opaque),
        )?;
        Ok(OwnerFlushReply {})
    }

    fn fsync(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFsyncRequest,
    ) -> afs_error::Result<OwnerFsyncReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerFsyncRequest missing handle")?;
        self.executor.fsync(
            authenticated_peer_node_id,
            &access,
            &remote_file_for_handle(&access, handle.opaque),
            request.datasync,
        )?;
        Ok(OwnerFsyncReply {})
    }

    fn release(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReleaseRequest,
    ) -> afs_error::Result<OwnerReleaseReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerReleaseRequest missing handle")?;
        self.executor.release(
            authenticated_peer_node_id,
            &access,
            remote_file_for_handle(&access, handle.opaque),
        )?;
        Ok(OwnerReleaseReply {})
    }

    fn readlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReadlinkRequest,
    ) -> afs_error::Result<OwnerReadlinkReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let target = self.executor.readlink(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
        )?;
        Ok(OwnerReadlinkReply { target })
    }

    fn opendir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerOpendirRequest,
    ) -> afs_error::Result<OwnerOpendirReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let directory = self.executor.opendir(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
        )?;
        Ok(OwnerOpendirReply {
            handle: Some(OwnerDirectoryHandle {
                opaque: directory.handle,
            }),
            file_identity: Some(afs_protocol::node_data::FileIdentity {
                opaque: directory.identity.0,
            }),
            owner_session_id: directory.owner_session_id,
        })
    }

    fn readdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReaddirRequest,
    ) -> afs_error::Result<OwnerReaddirReply> {
        let access = presented_access(request.access)?;
        let max_entries = request.max_entries as usize;
        let handle = required_directory_handle(
            request.handle,
            "OwnerReaddirRequest missing directory handle",
        )?;
        let entries = self.executor.readdir(
            authenticated_peer_node_id,
            &access,
            &remote_directory_for_handle(&access, handle.opaque),
            request.offset,
            max_entries,
        )?;
        let eof = entries.len() < max_entries;
        Ok(OwnerReaddirReply {
            entries: entries
                .into_iter()
                .map(|entry| OwnerDirEntry {
                    name: entry.name.into_vec(),
                    attr: Some(owner_attr(entry.entry.identity, entry.entry.attributes)),
                    next_offset: entry.next_cookie,
                })
                .collect(),
            eof,
        })
    }

    fn fsync_dir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFsyncDirRequest,
    ) -> afs_error::Result<OwnerFsyncDirReply> {
        let access = presented_access(request.access)?;
        let handle = required_directory_handle(
            request.handle,
            "OwnerFsyncDirRequest missing directory handle",
        )?;
        self.executor.fsyncdir(
            authenticated_peer_node_id,
            &access,
            &remote_directory_for_handle(&access, handle.opaque),
            request.datasync,
        )?;
        Ok(OwnerFsyncDirReply {})
    }

    fn release_dir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReleaseDirRequest,
    ) -> afs_error::Result<OwnerReleaseDirReply> {
        let access = presented_access(request.access)?;
        let handle = required_directory_handle(
            request.handle,
            "OwnerReleaseDirRequest missing directory handle",
        )?;
        self.executor.releasedir(
            authenticated_peer_node_id,
            &access,
            remote_directory_for_handle(&access, handle.opaque),
        )?;
        Ok(OwnerReleaseDirReply {})
    }

    fn symlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerSymlinkRequest,
    ) -> afs_error::Result<OwnerSymlinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Symlink"))
    }

    fn link(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerLinkRequest,
    ) -> afs_error::Result<OwnerLinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Link"))
    }
}

#[cfg(feature = "ownerfs")]
fn presented_access(
    access: Option<afs_protocol::node_data::RootAccess>,
) -> afs_error::Result<PresentedRootAccess> {
    let access = access.ok_or_else(|| protocol_error("OwnerFiles request missing RootAccess"))?;
    Ok(PresentedRootAccess {
        id: RootId(access.root_id),
        epoch: access.root_epoch,
        home_node_id: access.home_node_id,
        home_session_id: access.home_session_id,
        holder_node_id: access.holder_node_id,
        session_id: access.session_id,
        access_generation: access.access_generation,
        fencing_token: access.fencing_token,
    })
}

#[cfg(feature = "ownerfs")]
fn path_os(path: Vec<u8>) -> OsString {
    OsString::from_vec(path)
}

#[cfg(feature = "ownerfs")]
fn required_handle(
    handle: Option<afs_protocol::node_data::OwnerHandle>,
    message: &'static str,
) -> afs_error::Result<afs_protocol::node_data::OwnerHandle> {
    handle.ok_or_else(|| protocol_error(message))
}

#[cfg(feature = "ownerfs")]
fn required_directory_handle(
    handle: Option<afs_protocol::node_data::OwnerDirectoryHandle>,
    message: &'static str,
) -> afs_error::Result<afs_protocol::node_data::OwnerDirectoryHandle> {
    handle.ok_or_else(|| protocol_error(message))
}

#[cfg(feature = "ownerfs")]
fn required_identity(
    identity: Option<afs_protocol::node_data::FileIdentity>,
    message: &'static str,
) -> afs_error::Result<FileIdentity> {
    identity
        .map(|identity| FileIdentity(identity.opaque))
        .ok_or_else(|| protocol_error(message))
}

#[cfg(feature = "ownerfs")]
fn attribute_change(
    attr: Option<afs_protocol::node_data::OwnerSetAttr>,
) -> afs_error::Result<AttributeChange> {
    let attr = attr.ok_or_else(|| protocol_error("OwnerSetAttrRequest missing attr"))?;
    Ok(AttributeChange {
        size: attr.size,
        mode: attr.mode,
        uid: attr.uid,
        gid: attr.gid,
        atime: attr.atime_ns.map(ns_to_time),
        mtime: attr.mtime_ns.map(ns_to_time),
    })
}

#[cfg(feature = "ownerfs")]
fn remote_file_for_handle(access: &PresentedRootAccess, handle: Vec<u8>) -> RemoteFile {
    RemoteFile {
        root_id: access.id.clone(),
        owner_node_id: access.home_node_id.clone(),
        owner_session_id: access.home_session_id.clone(),
        identity: FileIdentity(Vec::new()),
        handle,
    }
}

#[cfg(feature = "ownerfs")]
fn remote_directory_for_handle(access: &PresentedRootAccess, handle: Vec<u8>) -> RemoteDirectory {
    RemoteDirectory {
        root_id: access.id.clone(),
        owner_node_id: access.home_node_id.clone(),
        owner_session_id: access.home_session_id.clone(),
        identity: FileIdentity(Vec::new()),
        handle,
    }
}

#[cfg(feature = "ownerfs")]
fn ns_to_time(ns: u64) -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_nanos(ns)
}

#[cfg(feature = "ownerfs")]
fn owner_attr(
    identity: FileIdentity,
    attributes: FileAttributes,
) -> afs_protocol::node_data::OwnerFileAttr {
    afs_protocol::node_data::OwnerFileAttr {
        identity: Some(afs_protocol::node_data::FileIdentity { opaque: identity.0 }),
        kind: owner_kind(attributes.kind).into(),
        mode: attributes.mode,
        uid: attributes.uid,
        gid: attributes.gid,
        size: attributes.size,
        blocks: 0,
        atime_ns: time_ns(attributes.atime),
        mtime_ns: time_ns(attributes.mtime),
        ctime_ns: time_ns(attributes.ctime),
        nlink: attributes.nlink,
        blksize: 4096,
    }
}

#[cfg(feature = "ownerfs")]
fn owner_kind(kind: FileKind) -> afs_protocol::node_data::OwnerFileKind {
    match kind {
        FileKind::Regular => afs_protocol::node_data::OwnerFileKind::Regular,
        FileKind::Directory => afs_protocol::node_data::OwnerFileKind::Directory,
        FileKind::Symlink => afs_protocol::node_data::OwnerFileKind::Symlink,
    }
}

#[cfg(feature = "ownerfs")]
fn time_ns(time: std::time::SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(feature = "ownerfs")]
fn protocol_error(message: &'static str) -> afs_error::Error {
    afs_error::Error::coded(afs_error::CLIENT_PROTOCOL_VIOLATION, message)
}

/// 从已经认证过的 node-to-node 通道提取对端 Node 身份。
///
/// 最终生产实现应绑定 mTLS/SPIFFE SAN、或 Meta 下发的每节点 token 与 TLS
/// channel binding。这里故意不提供“信任 holder_node_id 字段”的实现。
#[cfg(feature = "ownerfs")]
pub trait PeerAuthenticator: Send + Sync + 'static {
    fn authenticate(
        &self,
        metadata: &MetadataMap,
        remote_addr: Option<SocketAddr>,
        peer_cert_der: Option<&[u8]>,
    ) -> afs_error::Result<String>;
}

/// Node-to-node mTLS identity checker backed by an exact certificate allow-list.
///
/// The first production version intentionally avoids parsing X.509 names: Node
/// config provides the verified peer certificate DER for each node id, tonic
/// verifies the TLS client certificate chain, and this authenticator binds the
/// presented leaf certificate bytes to one configured node id. If no certificate
/// is present, or if the bytes are unknown, OwnerFiles fails closed before the
/// request body is inspected.
#[derive(Clone, Debug)]
#[cfg(feature = "ownerfs")]
pub struct MtlsPeerAuthenticator {
    node_id_by_cert_der: Arc<HashMap<Vec<u8>, String>>,
}

#[cfg(feature = "ownerfs")]
impl MtlsPeerAuthenticator {
    pub fn new<I>(trusted_peer_certs: I) -> afs_error::Result<Self>
    where
        I: IntoIterator<Item = (String, Vec<u8>)>,
    {
        let mut node_id_by_cert_der = HashMap::new();
        for (node_id, cert_der) in trusted_peer_certs {
            if node_id.is_empty() {
                return Err(afs_error::Error::coded(
                    afs_error::CLIENT_ARGUMENT_INVALID,
                    "trusted peer certificate node_id is empty",
                ));
            }
            if cert_der.is_empty() {
                return Err(afs_error::Error::coded(
                    afs_error::CLIENT_ARGUMENT_INVALID,
                    format!("trusted peer certificate for {node_id} is empty"),
                ));
            }
            if let Some(previous) = node_id_by_cert_der.insert(cert_der, node_id.clone()) {
                return Err(afs_error::Error::coded(
                    afs_error::CLIENT_ARGUMENT_INVALID,
                    format!(
                        "the same peer certificate is configured for both {previous} and {node_id}"
                    ),
                ));
            }
        }
        Ok(Self {
            node_id_by_cert_der: Arc::new(node_id_by_cert_der),
        })
    }
}

#[cfg(feature = "ownerfs")]
impl PeerAuthenticator for MtlsPeerAuthenticator {
    fn authenticate(
        &self,
        metadata: &MetadataMap,
        remote_addr: Option<SocketAddr>,
        peer_cert_der: Option<&[u8]>,
    ) -> afs_error::Result<String> {
        let _ = (metadata, remote_addr);
        let Some(peer_cert_der) = peer_cert_der else {
            return Err(afs_error::Error::coded(
                afs_error::NODE_OWNER_INVALID_GRANT,
                "OwnerFiles peer connection has no verified mTLS client certificate",
            ));
        };
        self.node_id_by_cert_der
            .get(peer_cert_der)
            .cloned()
            .ok_or_else(|| {
                afs_error::Error::coded(
                    afs_error::NODE_OWNER_INVALID_GRANT,
                    "OwnerFiles peer certificate is not trusted for any configured node id",
                )
            })
    }
}

/// OwnerFs 远端文件数据面入口。
///
/// 无 handler 的默认实例继续返回 UNIMPLEMENTED，避免把 diagnostics 的 Storage
/// 成功误当成 workspace 文件成功。有 handler 时也必须提供 authenticator；
/// 否则服务 fail-closed，不允许依赖请求体里的 holder_node_id。
#[derive(Clone, Default)]
#[cfg(feature = "ownerfs")]
pub struct OwnerFilesService {
    handler: Option<Arc<dyn OwnerFilesHandler>>,
    authenticator: Option<Arc<dyn PeerAuthenticator>>,
    metrics: Option<super::OwnerRpcMetrics>,
}

#[cfg(feature = "ownerfs")]
impl OwnerFilesService {
    #[must_use]
    pub fn new(
        handler: Arc<dyn OwnerFilesHandler>,
        authenticator: Arc<dyn PeerAuthenticator>,
    ) -> Self {
        Self {
            handler: Some(handler),
            authenticator: Some(authenticator),
            metrics: None,
        }
    }

    #[must_use]
    pub fn with_metrics(mut self, metrics: super::OwnerRpcMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    async fn dispatch<Req, Reply, F>(
        &self,
        request: Request<Req>,
        operation: &'static str,
        call: F,
    ) -> Result<Response<Reply>, Status>
    where
        Req: Send + 'static,
        Reply: Send + 'static,
        F: FnOnce(Arc<dyn OwnerFilesHandler>, String, Req) -> afs_error::Result<Reply>
            + Send
            + 'static,
    {
        let Some(handler) = self.handler.clone() else {
            return Err(owner_files_unimplemented(operation));
        };
        let Some(authenticator) = self.authenticator.clone() else {
            return Err(coded_status(
                afs_error::NODE_OWNER_INVALID_GRANT,
                format!("{operation} has no authenticated peer identity source"),
            ));
        };
        let started = std::time::Instant::now();
        let metadata = request.metadata().clone();
        let remote_addr = request.remote_addr();
        let peer_cert_der = request
            .peer_certs()
            .and_then(|certs| certs.iter().next().map(|cert| cert.as_ref().to_vec()));
        let request = request.into_inner();
        let work = move || {
            let authenticated_peer_node_id =
                authenticator.authenticate(&metadata, remote_addr, peer_cert_der.as_deref())?;
            call(handler, authenticated_peer_node_id, request)
        };
        // Production Node uses a multi-thread runtime. Run the short local
        // file action on this worker and let Tokio hand other tasks off while
        // it blocks, avoiding a blocking-pool scheduling hop per file RPC.
        // Current-thread test runtimes still need spawn_blocking.
        let result = if tokio::runtime::Handle::current().runtime_flavor()
            == tokio::runtime::RuntimeFlavor::MultiThread
        {
            Ok(tokio::task::block_in_place(work))
        } else {
            tokio::task::spawn_blocking(work).await
        };
        if let Some(metrics) = &self.metrics {
            metrics.observe("server", operation, started.elapsed());
        }
        let reply = result
            .map_err(|error| {
                coded_status(
                    afs_error::CLIENT_WORKER_FAILED,
                    format!("{operation} blocking worker failed: {error}"),
                )
            })?
            .map_err(error_to_status)?;
        Ok(Response::new(reply))
    }
}

#[must_use]
#[cfg(feature = "ownerfs")]
pub fn make_owner_files_server() -> OwnerFilesServer<OwnerFilesService> {
    OwnerFilesServer::new(OwnerFilesService::default())
}

#[must_use]
#[cfg(feature = "ownerfs")]
pub fn make_owner_files_server_with_handler(
    handler: Arc<dyn OwnerFilesHandler>,
    authenticator: Arc<dyn PeerAuthenticator>,
) -> OwnerFilesServer<OwnerFilesService> {
    OwnerFilesServer::new(OwnerFilesService::new(handler, authenticator))
}

#[must_use]
#[cfg(feature = "ownerfs")]
pub fn make_owner_files_server_with_handler_and_metrics(
    handler: Arc<dyn OwnerFilesHandler>,
    authenticator: Arc<dyn PeerAuthenticator>,
    metrics: super::OwnerRpcMetrics,
) -> OwnerFilesServer<OwnerFilesService> {
    OwnerFilesServer::new(OwnerFilesService::new(handler, authenticator).with_metrics(metrics))
}

#[tonic::async_trait]
#[cfg(feature = "ownerfs")]
impl OwnerFiles for OwnerFilesService {
    async fn lookup(
        &self,
        request: Request<OwnerLookupRequest>,
    ) -> Result<Response<OwnerLookupReply>, Status> {
        self.dispatch(request, "OwnerFiles.Lookup", |handler, peer, request| {
            handler.lookup(&peer, request)
        })
        .await
    }

    async fn get_attr(
        &self,
        request: Request<OwnerGetAttrRequest>,
    ) -> Result<Response<OwnerGetAttrReply>, Status> {
        self.dispatch(request, "OwnerFiles.GetAttr", |handler, peer, request| {
            handler.get_attr(&peer, request)
        })
        .await
    }

    async fn set_attr(
        &self,
        request: Request<OwnerSetAttrRequest>,
    ) -> Result<Response<OwnerSetAttrReply>, Status> {
        self.dispatch(request, "OwnerFiles.SetAttr", |handler, peer, request| {
            handler.set_attr(&peer, request)
        })
        .await
    }

    async fn create(
        &self,
        request: Request<OwnerCreateRequest>,
    ) -> Result<Response<OwnerCreateReply>, Status> {
        self.dispatch(request, "OwnerFiles.Create", |handler, peer, request| {
            handler.create(&peer, request)
        })
        .await
    }

    async fn mkdir(
        &self,
        request: Request<OwnerMkdirRequest>,
    ) -> Result<Response<OwnerMkdirReply>, Status> {
        self.dispatch(request, "OwnerFiles.Mkdir", |handler, peer, request| {
            handler.mkdir(&peer, request)
        })
        .await
    }

    async fn unlink(
        &self,
        request: Request<OwnerUnlinkRequest>,
    ) -> Result<Response<OwnerUnlinkReply>, Status> {
        self.dispatch(request, "OwnerFiles.Unlink", |handler, peer, request| {
            handler.unlink(&peer, request)
        })
        .await
    }

    async fn rmdir(
        &self,
        request: Request<OwnerRmdirRequest>,
    ) -> Result<Response<OwnerRmdirReply>, Status> {
        self.dispatch(request, "OwnerFiles.Rmdir", |handler, peer, request| {
            handler.rmdir(&peer, request)
        })
        .await
    }

    async fn rename(
        &self,
        request: Request<OwnerRenameRequest>,
    ) -> Result<Response<OwnerRenameReply>, Status> {
        self.dispatch(request, "OwnerFiles.Rename", |handler, peer, request| {
            handler.rename(&peer, request)
        })
        .await
    }

    async fn open(
        &self,
        request: Request<OwnerOpenRequest>,
    ) -> Result<Response<OwnerOpenReply>, Status> {
        self.dispatch(request, "OwnerFiles.Open", |handler, peer, request| {
            handler.open(&peer, request)
        })
        .await
    }

    async fn readlink(
        &self,
        request: Request<OwnerReadlinkRequest>,
    ) -> Result<Response<OwnerReadlinkReply>, Status> {
        self.dispatch(request, "OwnerFiles.Readlink", |handler, peer, request| {
            handler.readlink(&peer, request)
        })
        .await
    }

    async fn read(
        &self,
        request: Request<OwnerReadRequest>,
    ) -> Result<Response<OwnerReadReply>, Status> {
        self.dispatch(request, "OwnerFiles.Read", |handler, peer, request| {
            handler.read(&peer, request)
        })
        .await
    }

    async fn write(
        &self,
        request: Request<OwnerWriteRequest>,
    ) -> Result<Response<OwnerWriteReply>, Status> {
        self.dispatch(request, "OwnerFiles.Write", |handler, peer, request| {
            handler.write(&peer, request)
        })
        .await
    }

    async fn flush(
        &self,
        request: Request<OwnerFlushRequest>,
    ) -> Result<Response<OwnerFlushReply>, Status> {
        self.dispatch(request, "OwnerFiles.Flush", |handler, peer, request| {
            handler.flush(&peer, request)
        })
        .await
    }

    async fn fsync(
        &self,
        request: Request<OwnerFsyncRequest>,
    ) -> Result<Response<OwnerFsyncReply>, Status> {
        self.dispatch(request, "OwnerFiles.Fsync", |handler, peer, request| {
            handler.fsync(&peer, request)
        })
        .await
    }

    async fn release(
        &self,
        request: Request<OwnerReleaseRequest>,
    ) -> Result<Response<OwnerReleaseReply>, Status> {
        self.dispatch(request, "OwnerFiles.Release", |handler, peer, request| {
            handler.release(&peer, request)
        })
        .await
    }

    async fn opendir(
        &self,
        request: Request<OwnerOpendirRequest>,
    ) -> Result<Response<OwnerOpendirReply>, Status> {
        self.dispatch(request, "OwnerFiles.Opendir", |handler, peer, request| {
            handler.opendir(&peer, request)
        })
        .await
    }

    async fn readdir(
        &self,
        request: Request<OwnerReaddirRequest>,
    ) -> Result<Response<OwnerReaddirReply>, Status> {
        self.dispatch(request, "OwnerFiles.Readdir", |handler, peer, request| {
            handler.readdir(&peer, request)
        })
        .await
    }

    async fn fsync_dir(
        &self,
        request: Request<OwnerFsyncDirRequest>,
    ) -> Result<Response<OwnerFsyncDirReply>, Status> {
        self.dispatch(request, "OwnerFiles.FsyncDir", |handler, peer, request| {
            handler.fsync_dir(&peer, request)
        })
        .await
    }

    async fn release_dir(
        &self,
        request: Request<OwnerReleaseDirRequest>,
    ) -> Result<Response<OwnerReleaseDirReply>, Status> {
        self.dispatch(
            request,
            "OwnerFiles.ReleaseDir",
            |handler, peer, request| handler.release_dir(&peer, request),
        )
        .await
    }

    async fn symlink(
        &self,
        request: Request<OwnerSymlinkRequest>,
    ) -> Result<Response<OwnerSymlinkReply>, Status> {
        self.dispatch(request, "OwnerFiles.Symlink", |handler, peer, request| {
            handler.symlink(&peer, request)
        })
        .await
    }

    async fn link(
        &self,
        request: Request<OwnerLinkRequest>,
    ) -> Result<Response<OwnerLinkReply>, Status> {
        self.dispatch(request, "OwnerFiles.Link", |handler, peer, request| {
            handler.link(&peer, request)
        })
        .await
    }
}

#[cfg(feature = "ownerfs")]
fn owner_files_unimplemented(operation: &'static str) -> Status {
    coded_status(
        afs_error::NODE_VFS_UNIMPLEMENTED,
        format!("{operation} is not wired to OwnerFs yet"),
    )
}

#[cfg(feature = "ownerfs")]
fn owner_handler_unimplemented(operation: &'static str) -> afs_error::Error {
    afs_error::Error::coded(
        afs_error::NODE_VFS_UNIMPLEMENTED,
        format!("{operation} is not implemented by the injected OwnerFs handler"),
    )
}

#[cfg(all(test, feature = "ownerfs"))]
mod owner_tests {
    use super::*;

    #[test]
    fn mtls_authenticator_binds_peer_identity_to_exact_leaf_der() {
        let authenticator = MtlsPeerAuthenticator::new(vec![("node-b".to_owned(), vec![1, 2, 3])])
            .expect("trusted cert config should be valid");
        let authenticated = authenticator
            .authenticate(&MetadataMap::new(), None, Some(&[1, 2, 3]))
            .expect("known cert should authenticate");
        assert_eq!(authenticated, "node-b");
    }

    #[test]
    fn mtls_authenticator_fails_closed_without_verified_cert() {
        let authenticator = MtlsPeerAuthenticator::new(vec![("node-b".to_owned(), vec![1, 2, 3])])
            .expect("trusted cert config should be valid");
        let error = authenticator
            .authenticate(&MetadataMap::new(), None, None)
            .expect_err("OwnerFiles must not accept unauthenticated peers");
        assert_eq!(error.code(), afs_error::NODE_OWNER_INVALID_GRANT);
    }

    #[test]
    fn mtls_authenticator_rejects_ambiguous_cert_config() {
        let error = MtlsPeerAuthenticator::new(vec![
            ("node-a".to_owned(), vec![9, 9]),
            ("node-b".to_owned(), vec![9, 9]),
        ])
        .expect_err("one certificate cannot identify two nodes");
        assert_eq!(error.code(), afs_error::CLIENT_ARGUMENT_INVALID);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ownerfs")]
    use super::make_owner_files_server;
    use super::*;
    use crate::node::rpc::{
        control::make_control_server,
        peer::{DataClientOptions, DataMode, connect_data_client},
    };
    #[cfg(feature = "ownerfs")]
    use afs_protocol::node_data::{
        OwnerDirectoryHandle, OwnerFsyncRequest, OwnerGetAttrRequest, OwnerHandle,
        OwnerOpenRequest, OwnerReaddirRequest, OwnerReadlinkRequest, RootAccess,
        owner_files_client::OwnerFilesClient,
    };
    #[cfg(feature = "ownerfs")]
    use afs_transport::grpc::error_status::status_to_error;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Server;

    #[tokio::test]
    async fn grpc_inline_write_then_read_uses_storage() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::new(temp.path()).unwrap();
        let service = NodeDataService::new(Arc::new(storage), RdmaSessionRegistry::new(None));

        let written = service
            .write(Request::new(DataWriteRequest {
                session_id: 0,
                transfer: DataTransfer::GrpcInline.into(),
                name: "eight.bin".into(),
                offset: 0,
                data: b"12345678".to_vec(),
                length: 0,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(written.written, 8);

        let read = service
            .read(Request::new(DataReadRequest {
                session_id: 0,
                transfer: DataTransfer::GrpcInline.into(),
                name: "eight.bin".into(),
                offset: 0,
                length: 8,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(read.length, 8);
        assert_eq!(read.data, b"12345678");
    }

    #[tokio::test]
    async fn rdma_read_validates_session_before_touching_storage() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::new(temp.path()).unwrap();
        let service = NodeDataService::new(Arc::new(storage), RdmaSessionRegistry::new(None));

        let error = service
            .read(Request::new(DataReadRequest {
                session_id: 99,
                transfer: DataTransfer::RdmaOneSided.into(),
                name: "missing.bin".into(),
                offset: 0,
                length: 8,
            }))
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn public_grpc_client_and_servers_move_eight_bytes() {
        let (_temp, endpoint, server) = spawn_grpc_server().await;

        let mut client = connect_data_client(DataClientOptions {
            endpoint: endpoint.clone(),
            mode: DataMode::Grpc,
            rdma_device: None,
            timeout: std::time::Duration::from_secs(5),
        })
        .await
        .unwrap();
        assert_eq!(client.mode(), "grpc");
        assert_eq!(
            client
                .write("eight.bin", 0, b"abcdefgh".to_vec())
                .await
                .unwrap(),
            8
        );
        assert_eq!(client.read("eight.bin", 0, 8).await.unwrap(), b"abcdefgh");
        client.close().await.unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn auto_without_rdma_device_falls_back_to_grpc() {
        let (_temp, endpoint, server) = spawn_grpc_server().await;

        let mut client = connect_data_client(DataClientOptions {
            endpoint,
            mode: DataMode::Auto,
            rdma_device: None,
            timeout: std::time::Duration::from_secs(5),
        })
        .await
        .unwrap();

        assert_eq!(client.mode(), "grpc");
        assert_eq!(
            client
                .write("auto.bin", 0, b"abcdefgh".to_vec())
                .await
                .unwrap(),
            8
        );
        assert_eq!(client.read("auto.bin", 0, 8).await.unwrap(), b"abcdefgh");
        server.abort();
    }

    #[tokio::test]
    async fn forced_rdma_without_device_does_not_fallback_to_grpc() {
        let (_temp, endpoint, server) = spawn_grpc_server().await;

        let result = connect_data_client(DataClientOptions {
            endpoint,
            mode: DataMode::Rdma,
            rdma_device: None,
            timeout: std::time::Duration::from_secs(5),
        })
        .await;
        let error = match result {
            Ok(_) => panic!("forced RDMA unexpectedly fell back to gRPC"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("RDMA"));
        server.abort();
    }

    #[cfg(feature = "ownerfs")]
    #[tokio::test]
    async fn owner_files_service_is_registered_but_not_faked() {
        let (_temp, endpoint, server) = spawn_grpc_server().await;
        let mut client = OwnerFilesClient::connect(endpoint).await.unwrap();

        let error = client
            .open(OwnerOpenRequest {
                access: Some(root_access()),
                path: b"notes.txt".to_vec(),
                flags: 0,
                mode: 0,
                expected_file_identity: None,
            })
            .await
            .unwrap_err();

        assert_owner_unimplemented(error);
        server.abort();
    }

    #[cfg(feature = "ownerfs")]
    #[tokio::test]
    async fn owner_files_attr_dir_and_fsync_rpcs_are_registered_but_not_faked() {
        let (_temp, endpoint, server) = spawn_grpc_server().await;
        let mut client = OwnerFilesClient::connect(endpoint).await.unwrap();

        assert_owner_unimplemented(
            client
                .get_attr(OwnerGetAttrRequest {
                    access: Some(root_access()),
                    path: b"notes.txt".to_vec(),
                    expected_file_identity: None,
                    handle: None,
                })
                .await
                .unwrap_err(),
        );
        assert_owner_unimplemented(
            client
                .readdir(OwnerReaddirRequest {
                    access: Some(root_access()),
                    handle: Some(OwnerDirectoryHandle {
                        opaque: b"dir-handle".to_vec(),
                    }),
                    offset: 0,
                    max_entries: 16,
                })
                .await
                .unwrap_err(),
        );
        assert_owner_unimplemented(
            client
                .fsync(OwnerFsyncRequest {
                    access: Some(root_access()),
                    handle: Some(OwnerHandle {
                        opaque: b"file-handle".to_vec(),
                    }),
                    datasync: true,
                })
                .await
                .unwrap_err(),
        );
        assert_owner_unimplemented(
            client
                .readlink(OwnerReadlinkRequest {
                    access: Some(root_access()),
                    path: b"link.txt".to_vec(),
                    expected_file_identity: None,
                })
                .await
                .unwrap_err(),
        );

        server.abort();
    }

    #[cfg(feature = "ownerfs")]
    fn root_access() -> RootAccess {
        RootAccess {
            root_id: "workspace-1".into(),
            root_epoch: 7,
            access_generation: 3,
            holder_node_id: "node-b".into(),
            home_node_id: "node-a".into(),
            session_id: "node-b-session-11".into(),
            fencing_token: "grant-token-7-3".into(),
            home_session_id: "node-a-session-9".into(),
        }
    }

    #[cfg(feature = "ownerfs")]
    fn assert_owner_unimplemented(error: tonic::Status) {
        assert_eq!(error.code(), tonic::Code::Unimplemented);
        assert_eq!(
            status_to_error(error).code(),
            afs_error::NODE_VFS_UNIMPLEMENTED
        );
    }

    async fn spawn_grpc_server() -> (tempfile::TempDir, String, tokio::task::JoinHandle<()>) {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("afs-test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let temp = tempfile::Builder::new().tempdir_in(base).unwrap();
        let storage = Arc::new(Storage::new(temp.path()).unwrap());
        let registry = RdmaSessionRegistry::new(None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn({
            let registry = registry.clone();
            async move {
                let server = Server::builder()
                    .add_service(make_control_server(registry.clone()))
                    .add_service(make_data_server(storage, registry));
                #[cfg(feature = "ownerfs")]
                let server = server.add_service(make_owner_files_server());
                server
                    .serve_with_incoming(TcpListenerStream::new(listener))
                    .await
                    .unwrap();
            }
        });
        (temp, endpoint, server)
    }
}

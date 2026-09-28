//! 调用其他 Node 的诊断数据 API 与 adapter 边界。
//!
//! 上层诊断调用 DataPeerClient::read/write/close，不关心底层走 gRPC inline
//! 还是 RDMA one-sided。服务端对应 data.rs 的诊断 handler；OwnerFs 文件
//! 业务使用 node_data.proto 中独立的 OwnerFiles service 和 remote.rs 合同，
//! 尚未接入本客户端。
//!
//! 模式含义：
//! - Grpc：控制命令和文件内容都进 node_data proto；
//! - Rdma：control/data 命令仍走 gRPC proto，文件内容走 RDMA MR；
//! - Auto：先尝试 RDMA 建连，失败才在“尚未发出业务请求”前回退到 gRPC。
//!
//! 已经发出的写如果结果不明，绝不换通道重放；RDMA in-flight 被取消时会 poison
//! 当前 client/session，后续复用必须失败，避免重复写或顺序错乱。

#[cfg(any(feature = "ownerfs", feature = "rdma"))]
use std::sync::Arc;
#[cfg(feature = "rdma")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
#[cfg(feature = "ownerfs")]
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "rdma")]
use afs_protocol::node_control::{
    CloseDataRequest, NegotiateDataRequest, node_control_client::NodeControlClient,
};
#[cfg(feature = "ownerfs")]
use afs_protocol::node_data::{
    DataPlane, FileIdentity as PbFileIdentity, OwnerCreateRequest, OwnerDirectoryHandle,
    OwnerFileAttr, OwnerFileKind, OwnerFlushRequest, OwnerFsyncDirRequest, OwnerFsyncRequest,
    OwnerGetAttrRequest, OwnerHandle, OwnerLookupRequest, OwnerMkdirRequest, OwnerOpenRequest,
    OwnerOpendirRequest, OwnerReadRequest, OwnerReaddirRequest, OwnerReadlinkRequest,
    OwnerReleaseDirRequest, OwnerReleaseRequest, OwnerRenameRequest, OwnerRmdirRequest,
    OwnerSetAttr, OwnerSetAttrRequest, OwnerUnlinkRequest, OwnerWriteRequest, RootAccess,
    owner_files_client::OwnerFilesClient,
};
use afs_protocol::node_data::{
    DataReadRequest, DataTransfer, DataWriteRequest, node_data_client::NodeDataClient,
};
#[cfg(feature = "rdma")]
use afs_tracing::Instrument;
use afs_tracing::request_with_current_context;
use tonic::transport::{Channel, Endpoint};

#[cfg(feature = "rdma")]
use tokio::sync::Mutex;

#[cfg(feature = "rdma")]
use super::control::RDMA_HANDSHAKE_VERSION;
#[cfg(feature = "rdma")]
use afs_transport::rdma::{CAPACITY, RdmaEndpoint};

#[cfg(feature = "ownerfs")]
use crate::node::vfs::{
    ownerfs::{
        files::{FileIdentity, OwnerEntry, RemoteDirectory, RemoteFile},
        remote::{RemoteCreatedFile, RemoteDirectoryEntry, RemoteFiles},
        root::RootGrant,
    },
    types::{AttributeChange, FileAttributes, FileKind, RenameFlags},
};

const MAX_TRANSFER_BYTES: usize = crate::node::storage::MAX_TRANSFER_BYTES;
#[cfg(feature = "ownerfs")]
const MAX_PENDING_RELEASES: usize = 64;
#[cfg(feature = "ownerfs")]
const RELEASE_MAX_ATTEMPTS: usize = 4;
#[cfg(feature = "ownerfs")]
const RELEASE_INITIAL_BACKOFF: Duration = Duration::from_millis(2);
#[cfg(feature = "ownerfs")]
const RELEASE_MAX_BACKOFF: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataMode {
    Grpc,
    Rdma,
    Auto,
}

#[derive(Clone, Debug)]
pub struct DataClientOptions {
    pub endpoint: String,
    pub mode: DataMode,
    pub rdma_device: Option<String>,
    pub timeout: Duration,
}

pub type PeerResult<T> = Result<T, PeerError>;

/// 两个数据 adapter 共用错误合同；远端 code 保留，不再压成 String。
#[derive(Debug)]
pub struct PeerError(pub afs_error::Error);
impl PeerError {
    fn coded(code: afs_error::ErrorCode, message: impl Into<String>) -> Self {
        Self(afs_error::Error::coded(code, message))
    }
    pub fn error(&self) -> &afs_error::Error {
        &self.0
    }
    pub fn code(&self) -> afs_error::ErrorCode {
        self.0.code()
    }
    pub fn kind(&self) -> afs_error::ErrorKind {
        self.0.kind()
    }
}
impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for PeerError {}
impl From<tonic::Status> for PeerError {
    fn from(value: tonic::Status) -> Self {
        Self(afs_transport::grpc::error_status::status_to_error(value))
    }
}
impl From<tonic::transport::Error> for PeerError {
    fn from(value: tonic::transport::Error) -> Self {
        Self::coded(afs_error::CLIENT_CONNECTION_UNAVAILABLE, value.to_string())
    }
}
impl From<tonic::codegen::http::uri::InvalidUri> for PeerError {
    fn from(value: tonic::codegen::http::uri::InvalidUri) -> Self {
        Self::coded(afs_error::CLIENT_ARGUMENT_INVALID, value.to_string())
    }
}

/// 远端 Node 数据面客户端统一入口。
///
/// 这是业务层看到的唯一类型：同一套 read/write API 后面可以接 gRPC inline 或 RDMA。
/// 它不是本地 SDK client；本地 SDK 只连接本机 node，这里用于 node-to-node。
pub struct DataPeerClient {
    inner: DataPeerClientInner,
}

enum DataPeerClientInner {
    Grpc(GrpcInlineClient),
    #[cfg(feature = "rdma")]
    Rdma(Box<RdmaDataClient>),
}

/// OwnerFs 的 node-to-node 文件客户端。
///
/// 它实现 `RemoteFiles`，供非 Home 节点把根内文件操作转发到 Home。当前只启用
/// gRPC inline 数据路径；RDMA one-sided 需要 OwnerFiles 专用的数据窗口协商，
/// 不能直接复用 diagnostics `NodeData` 会话。
#[cfg(feature = "ownerfs")]
pub struct OwnerPeerClient {
    client: StdMutex<OwnerFilesClient<Channel>>,
    runtime: OwnerRuntime,
    // Per-open immutable read snapshot returned by OwnerFiles.Open. It is
    // removed at release and never reused for another open of the same path.
    prefetched_reads: StdMutex<HashMap<Vec<u8>, Vec<u8>>>,
    pending_releases: Arc<AtomicUsize>,
    metrics: Option<super::OwnerRpcMetrics>,
}

/// Runtime used by synchronous OwnerFs/FUSE callbacks to drive async tonic RPCs.
///
/// `owner_files_client_from_channel` can be called from a Tokio task during tests
/// or from a plain FUSE worker thread in production slow paths. Capturing
/// `Handle::current()` unconditionally panics in the latter case, so we either
/// reuse the ambient runtime or keep a small private runtime alive with the
/// client.
#[cfg(feature = "ownerfs")]
#[derive(Clone)]
enum OwnerRuntime {
    Existing(tokio::runtime::Handle),
    Owned(Arc<tokio::runtime::Runtime>),
}

#[cfg(feature = "ownerfs")]
impl OwnerRuntime {
    fn current_or_new() -> PeerResult<Self> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            return Ok(Self::Existing(handle));
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                PeerError::coded(
                    afs_error::CLIENT_WORKER_FAILED,
                    format!("failed to build OwnerFiles client runtime: {error}"),
                )
            })?;
        Ok(Self::Owned(Arc::new(runtime)))
    }

    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        match self {
            Self::Existing(handle) => handle.block_on(future),
            Self::Owned(runtime) => runtime.block_on(future),
        }
    }
}

#[cfg(feature = "ownerfs")]
pub async fn connect_owner_files_client(options: DataClientOptions) -> PeerResult<OwnerPeerClient> {
    if options.mode != DataMode::Grpc {
        return Err(PeerError::coded(
            afs_error::NODE_TRANSFER_UNSUPPORTED,
            "OwnerFiles currently supports only gRPC inline transfer",
        ));
    }
    let channel = connect_channel(&options.endpoint, options.timeout).await?;
    Ok(owner_files_client_from_channel(channel))
}

/// Build an OwnerFiles client from an already configured tonic channel.
///
/// Node wiring should use this when peer traffic requires mTLS: construct the
/// `Channel` through the common SecurityManager/TLS config, then pass it here.
/// The legacy `connect_owner_files_client` remains for plaintext diagnostics and
/// tests until Node config owns secure endpoint construction.
#[cfg(feature = "ownerfs")]
pub fn owner_files_client_from_channel(channel: Channel) -> OwnerPeerClient {
    owner_files_client_from_channel_result(channel)
        .expect("OwnerFiles client runtime must be available or constructible")
}

/// Fallible variant used by code paths that want to surface runtime construction
/// errors instead of panicking. The public infallible helper is retained for the
/// existing Node/test wiring contract.
#[cfg(feature = "ownerfs")]
pub fn owner_files_client_from_channel_result(channel: Channel) -> PeerResult<OwnerPeerClient> {
    Ok(OwnerPeerClient {
        client: StdMutex::new(OwnerFilesClient::new(channel)),
        runtime: OwnerRuntime::current_or_new()?,
        prefetched_reads: StdMutex::new(HashMap::new()),
        pending_releases: Arc::new(AtomicUsize::new(0)),
        metrics: None,
    })
}

/// Production wiring passes the Node runtime so read-only CLOSE can be sent
/// without waiting for a response, like the prior HomeFs P2P path. The default
/// constructor above remains synchronous when it owns a current-thread runtime.
#[cfg(feature = "ownerfs")]
pub fn owner_files_client_from_channel_with_runtime(
    channel: Channel,
    runtime: tokio::runtime::Handle,
) -> OwnerPeerClient {
    owner_files_client_from_channel_with_runtime_and_metrics(channel, runtime, None)
}

#[cfg(feature = "ownerfs")]
pub fn owner_files_client_from_channel_with_runtime_and_metrics(
    channel: Channel,
    runtime: tokio::runtime::Handle,
    metrics: Option<super::OwnerRpcMetrics>,
) -> OwnerPeerClient {
    OwnerPeerClient {
        client: StdMutex::new(OwnerFilesClient::new(channel)),
        runtime: OwnerRuntime::Existing(runtime),
        prefetched_reads: StdMutex::new(HashMap::new()),
        pending_releases: Arc::new(AtomicUsize::new(0)),
        metrics,
    }
}

#[cfg(feature = "ownerfs")]
impl OwnerPeerClient {
    fn cloned_client(&self) -> PeerResult<OwnerFilesClient<Channel>> {
        Ok(self
            .client
            .lock()
            .map_err(|_| {
                PeerError::coded(
                    afs_error::CLIENT_WORKER_FAILED,
                    "OwnerFiles client lock poisoned",
                )
            })?
            .clone())
    }
}

#[cfg(feature = "ownerfs")]
macro_rules! owner_rpc {
    ($owner:expr, $method:ident, $request:expr) => {{
        let mut client = $owner.cloned_client().map_err(|error| error.0)?;
        let started = Instant::now();
        let result = $owner
            .runtime
            .block_on(async move { client.$method(request_with_current_context($request)).await })
            .map_err(|error| PeerError::from(error).0);
        if let Some(metrics) = &$owner.metrics {
            metrics.observe("client", stringify!($method), started.elapsed());
        }
        result?.into_inner()
    }};
}

#[cfg(feature = "ownerfs")]
impl RemoteFiles for OwnerPeerClient {
    fn lookup(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_parent: Option<&FileIdentity>,
    ) -> afs_error::Result<OwnerEntry> {
        let reply = owner_rpc!(
            self,
            lookup,
            OwnerLookupRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                expected_parent_identity: expected_parent.map(file_identity),
            }
        );
        owner_entry(grant, reply.attr)
    }

    fn getattr(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
        file: Option<&RemoteFile>,
    ) -> afs_error::Result<OwnerEntry> {
        let reply = owner_rpc!(
            self,
            get_attr,
            OwnerGetAttrRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                expected_file_identity: expected_identity.map(file_identity),
                handle: file.map(file_handle),
            }
        );
        owner_entry(grant, reply.attr)
    }

    fn setattr(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
        file: Option<&RemoteFile>,
        change: &AttributeChange,
    ) -> afs_error::Result<OwnerEntry> {
        let reply = owner_rpc!(
            self,
            set_attr,
            OwnerSetAttrRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                expected_file_identity: expected_identity.map(file_identity),
                attr: Some(owner_set_attr(change)),
                handle: file.map(file_handle),
            }
        );
        owner_entry(grant, reply.attr)
    }

    fn create(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        flags: i32,
        mode: u32,
        expected_parent: &FileIdentity,
    ) -> afs_error::Result<RemoteCreatedFile> {
        let reply = owner_rpc!(
            self,
            create,
            OwnerCreateRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                flags: flags as u32,
                mode,
                expected_parent: Some(file_identity(expected_parent)),
            }
        );
        let entry = owner_entry(grant, reply.attr)?;
        let handle = reply
            .handle
            .ok_or_else(|| protocol_error("OwnerCreateReply missing handle"))?;
        Ok(RemoteCreatedFile {
            file: remote_file(
                grant,
                entry.identity.clone(),
                handle.opaque,
                reply.owner_session_id,
            ),
            entry,
        })
    }

    fn mkdir(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        mode: u32,
        expected_parent: &FileIdentity,
    ) -> afs_error::Result<OwnerEntry> {
        let reply = owner_rpc!(
            self,
            mkdir,
            OwnerMkdirRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                mode,
                expected_parent: Some(file_identity(expected_parent)),
            }
        );
        owner_entry(grant, reply.attr)
    }

    fn unlink(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
        expected_parent: &FileIdentity,
    ) -> afs_error::Result<()> {
        let _reply = owner_rpc!(
            self,
            unlink,
            OwnerUnlinkRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                expected_file_identity: expected_identity.map(file_identity),
                expected_parent: Some(file_identity(expected_parent)),
            }
        );
        Ok(())
    }

    fn rmdir(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
        expected_parent: &FileIdentity,
    ) -> afs_error::Result<()> {
        let _reply = owner_rpc!(
            self,
            rmdir,
            OwnerRmdirRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                expected_file_identity: expected_identity.map(file_identity),
                expected_parent: Some(file_identity(expected_parent)),
            }
        );
        Ok(())
    }

    fn rename(
        &self,
        grant: &RootGrant,
        old_path: &OsStr,
        new_path: &OsStr,
        expected_old_identity: Option<&FileIdentity>,
        expected_new_identity: Option<&FileIdentity>,
        expected_old_parent: &FileIdentity,
        expected_new_parent: &FileIdentity,
        flags: RenameFlags,
    ) -> afs_error::Result<()> {
        let _reply = owner_rpc!(
            self,
            rename,
            OwnerRenameRequest {
                access: Some(root_access(grant)),
                old_path: old_path.as_bytes().to_vec(),
                new_path: new_path.as_bytes().to_vec(),
                expected_old_identity: expected_old_identity.map(file_identity),
                expected_new_identity: expected_new_identity.map(file_identity),
                flags: flags.0,
                expected_old_parent: Some(file_identity(expected_old_parent)),
                expected_new_parent: Some(file_identity(expected_new_parent)),
            }
        );
        Ok(())
    }

    fn open(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        flags: i32,
        expected_identity: Option<&FileIdentity>,
    ) -> afs_error::Result<(RemoteFile, FileAttributes)> {
        let reply = owner_rpc!(
            self,
            open,
            OwnerOpenRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                flags: flags as u32,
                mode: 0,
                expected_file_identity: expected_identity.map(file_identity),
            }
        );
        let identity = reply
            .file_identity
            .ok_or_else(|| protocol_error("OwnerOpenReply missing file_identity"))?;
        let handle = reply
            .handle
            .ok_or_else(|| protocol_error("OwnerOpenReply missing handle"))?;
        let attributes = file_attributes(
            reply
                .attr
                .ok_or_else(|| protocol_error("OwnerOpenReply missing attr"))?,
        )?;
        if let Some(bytes) = reply.prefetched_data {
            self.prefetched_reads
                .lock()
                .map_err(|_| protocol_error("OwnerFiles prefetch cache lock poisoned"))?
                .insert(handle.opaque.clone(), bytes);
        }
        Ok((
            remote_file(
                grant,
                FileIdentity(identity.opaque),
                handle.opaque,
                reply.owner_session_id,
            ),
            attributes,
        ))
    }

    fn readlink(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
    ) -> afs_error::Result<Vec<u8>> {
        let reply = owner_rpc!(
            self,
            readlink,
            OwnerReadlinkRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                expected_file_identity: expected_identity.map(file_identity),
            }
        );
        Ok(reply.target)
    }

    fn read(
        &self,
        grant: &RootGrant,
        file: &RemoteFile,
        offset: u64,
        out: &mut [u8],
    ) -> afs_error::Result<usize> {
        validate_length(out.len()).map_err(|error| error.0)?;
        if let Some(bytes) = self
            .prefetched_reads
            .lock()
            .map_err(|_| protocol_error("OwnerFiles prefetch cache lock poisoned"))?
            .get(&file.handle)
        {
            let start = usize::try_from(offset)
                .unwrap_or(usize::MAX)
                .min(bytes.len());
            let end = start.saturating_add(out.len()).min(bytes.len());
            out[..end - start].copy_from_slice(&bytes[start..end]);
            return Ok(end - start);
        }
        let length = out.len();
        let reply = owner_rpc!(
            self,
            read,
            OwnerReadRequest {
                access: Some(root_access(grant)),
                handle: Some(file_handle(file)),
                offset,
                length: length as u32,
                plane: Some(grpc_plane()),
            }
        );
        if reply.read as usize != reply.data.len() || reply.data.len() > length {
            return Err(protocol_error("OwnerReadReply shape mismatch"));
        }
        out[..reply.data.len()].copy_from_slice(&reply.data);
        Ok(reply.data.len())
    }

    fn write(
        &self,
        grant: &RootGrant,
        file: &RemoteFile,
        offset: u64,
        data: &[u8],
    ) -> afs_error::Result<usize> {
        validate_length(data.len()).map_err(|error| error.0)?;
        let len = data.len();
        let reply = owner_rpc!(
            self,
            write,
            OwnerWriteRequest {
                access: Some(root_access(grant)),
                handle: Some(file_handle(file)),
                offset,
                data: data.to_vec(),
                length: len as u32,
                plane: Some(grpc_plane()),
            }
        );
        Ok(reply.written as usize)
    }

    fn flush(&self, grant: &RootGrant, file: &RemoteFile) -> afs_error::Result<()> {
        let _reply = owner_rpc!(
            self,
            flush,
            OwnerFlushRequest {
                access: Some(root_access(grant)),
                handle: Some(file_handle(file)),
            }
        );
        Ok(())
    }

    fn fsync(
        &self,
        grant: &RootGrant,
        file: &RemoteFile,
        data_only: bool,
    ) -> afs_error::Result<()> {
        let _reply = owner_rpc!(
            self,
            fsync,
            OwnerFsyncRequest {
                access: Some(root_access(grant)),
                handle: Some(file_handle(file)),
                datasync: data_only,
            }
        );
        Ok(())
    }

    fn release(&self, grant: &RootGrant, file: RemoteFile) -> afs_error::Result<()> {
        self.prefetched_reads
            .lock()
            .map_err(|_| protocol_error("OwnerFiles prefetch cache lock poisoned"))?
            .remove(&file.handle);
        let request = OwnerReleaseRequest {
            access: Some(root_access(grant)),
            handle: Some(OwnerHandle {
                opaque: file.handle,
            }),
        };
        // All writes and explicit sync/flush calls have already received their
        // own result before this cleanup request. A FUSE RELEASE is not a
        // durability barrier; waiting for its RPC only adds close latency. We
        // still keep the handle in a bounded background job so transient
        // transport failures do not leak Home-side opens after a single packet
        // loss or reconnect window.
        if let OwnerRuntime::Existing(runtime) = &self.runtime
            && reserve_release_slot(&self.pending_releases)
        {
            let mut client = match self.cloned_client() {
                Ok(client) => client,
                Err(error) => {
                    self.pending_releases.fetch_sub(1, AtomicOrdering::AcqRel);
                    return Err(error.0);
                }
            };
            let pending = self.pending_releases.clone();
            let metrics = self.metrics.clone();
            runtime.spawn(async move {
                if let Err(error) = release_with_retry(&mut client, request, metrics).await {
                    afs_logging::warn!(
                        "ownerfs.release_failed";
                        "error" => error.to_string(),
                        "code" => error.code().to_string()
                    );
                }
                pending.fetch_sub(1, AtomicOrdering::AcqRel);
            });
            return Ok(());
        }
        let mut client = self.cloned_client().map_err(|error| error.0)?;
        self.runtime
            .block_on(release_with_retry(
                &mut client,
                request,
                self.metrics.clone(),
            ))
            .map_err(Into::into)
    }

    fn opendir(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
    ) -> afs_error::Result<RemoteDirectory> {
        let reply = owner_rpc!(
            self,
            opendir,
            OwnerOpendirRequest {
                access: Some(root_access(grant)),
                path: path.as_bytes().to_vec(),
                expected_file_identity: expected_identity.map(file_identity),
            }
        );
        let identity = reply
            .file_identity
            .ok_or_else(|| protocol_error("OwnerOpendirReply missing file_identity"))?;
        let handle = reply
            .handle
            .ok_or_else(|| protocol_error("OwnerOpendirReply missing handle"))?;
        Ok(RemoteDirectory {
            root_id: grant.id.clone(),
            owner_node_id: grant.home_node_id.clone(),
            owner_session_id: reply.owner_session_id,
            identity: FileIdentity(identity.opaque),
            handle: handle.opaque,
        })
    }

    fn readdir(
        &self,
        grant: &RootGrant,
        directory: &RemoteDirectory,
        cookie: u64,
        max_entries: usize,
    ) -> afs_error::Result<Vec<RemoteDirectoryEntry>> {
        let reply = owner_rpc!(
            self,
            readdir,
            OwnerReaddirRequest {
                access: Some(root_access(grant)),
                handle: Some(directory_handle(directory)),
                offset: cookie,
                max_entries: max_entries as u32,
            }
        );
        reply
            .entries
            .into_iter()
            .map(|entry| {
                Ok(RemoteDirectoryEntry {
                    name: OsString::from_vec(entry.name),
                    entry: owner_entry(grant, entry.attr)?,
                    next_cookie: entry.next_offset,
                })
            })
            .collect()
    }

    fn fsyncdir(
        &self,
        grant: &RootGrant,
        directory: &RemoteDirectory,
        data_only: bool,
    ) -> afs_error::Result<()> {
        let _reply = owner_rpc!(
            self,
            fsync_dir,
            OwnerFsyncDirRequest {
                access: Some(root_access(grant)),
                handle: Some(directory_handle(directory)),
                datasync: data_only,
            }
        );
        Ok(())
    }

    fn releasedir(&self, grant: &RootGrant, directory: RemoteDirectory) -> afs_error::Result<()> {
        let _reply = owner_rpc!(
            self,
            release_dir,
            OwnerReleaseDirRequest {
                access: Some(root_access(grant)),
                handle: Some(OwnerDirectoryHandle {
                    opaque: directory.handle,
                }),
            }
        );
        Ok(())
    }
}

#[cfg(feature = "ownerfs")]
fn reserve_release_slot(pending: &AtomicUsize) -> bool {
    pending
        .fetch_update(AtomicOrdering::AcqRel, AtomicOrdering::Acquire, |current| {
            (current < MAX_PENDING_RELEASES).then(|| current + 1)
        })
        .is_ok()
}

#[cfg(feature = "ownerfs")]
async fn release_with_retry(
    client: &mut OwnerFilesClient<Channel>,
    request: OwnerReleaseRequest,
    metrics: Option<super::OwnerRpcMetrics>,
) -> PeerResult<()> {
    let mut backoff = RELEASE_INITIAL_BACKOFF;
    for attempt in 1..=RELEASE_MAX_ATTEMPTS {
        let started = Instant::now();
        let result = client
            .release(request_with_current_context(request.clone()))
            .await
            .map(|_| ())
            .map_err(PeerError::from);
        if let Some(metrics) = &metrics {
            metrics.observe("client", "release", started.elapsed());
        }
        match result {
            Ok(()) => return Ok(()),
            Err(error) if is_idempotent_release_success(&error) => return Ok(()),
            Err(error) if attempt < RELEASE_MAX_ATTEMPTS && is_transient_release_error(&error) => {
                tokio::time::sleep(backoff).await;
                backoff = backoff.saturating_mul(2).min(RELEASE_MAX_BACKOFF);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("release retry loop always returns from its final attempt")
}

#[cfg(feature = "ownerfs")]
fn is_idempotent_release_success(error: &PeerError) -> bool {
    error.code() == afs_error::NODE_OWNER_STALE_HANDLE
        || error.kind() == afs_error::ErrorKind::NotFound
}

#[cfg(feature = "ownerfs")]
fn is_transient_release_error(error: &PeerError) -> bool {
    matches!(
        error.kind(),
        afs_error::ErrorKind::Unavailable
            | afs_error::ErrorKind::DeadlineExceeded
            | afs_error::ErrorKind::ResourceExhausted
            | afs_error::ErrorKind::Aborted
    )
}

impl DataPeerClient {
    pub async fn read(&mut self, name: &str, offset: u64, length: u32) -> PeerResult<Vec<u8>> {
        match &mut self.inner {
            DataPeerClientInner::Grpc(client) => client.read(name, offset, length).await,
            #[cfg(feature = "rdma")]
            DataPeerClientInner::Rdma(client) => client.read(name, offset, length).await,
        }
    }

    pub async fn write(&mut self, name: &str, offset: u64, data: Vec<u8>) -> PeerResult<u32> {
        match &mut self.inner {
            DataPeerClientInner::Grpc(client) => client.write(name, offset, data).await,
            #[cfg(feature = "rdma")]
            DataPeerClientInner::Rdma(client) => client.write(name, offset, data).await,
        }
    }

    pub async fn close(&mut self) -> PeerResult<()> {
        match &mut self.inner {
            DataPeerClientInner::Grpc(_) => Ok(()),
            #[cfg(feature = "rdma")]
            DataPeerClientInner::Rdma(client) => client.close().await,
        }
    }

    #[must_use]
    pub fn mode(&self) -> &'static str {
        match &self.inner {
            DataPeerClientInner::Grpc(client) => client.reported_mode,
            #[cfg(feature = "rdma")]
            DataPeerClientInner::Rdma(client) => client.reported_mode,
        }
    }
}

/// 根据配置建立数据客户端。
///
/// Auto 只允许在 RDMA 建连阶段失败时回退；一旦业务 read/write 已发出，
/// 结果未知就不能自动改走 gRPC 重放。
pub async fn connect_data_client(options: DataClientOptions) -> PeerResult<DataPeerClient> {
    let channel = connect_channel(&options.endpoint, options.timeout).await?;
    match options.mode {
        DataMode::Grpc => Ok(DataPeerClient {
            inner: DataPeerClientInner::Grpc(GrpcInlineClient::new(channel, "grpc")),
        }),
        DataMode::Rdma => connect_rdma(channel, options.rdma_device, "rdma").await,
        DataMode::Auto => match connect_rdma(channel.clone(), options.rdma_device, "rdma").await {
            Ok(client) => Ok(client),
            Err(_) => Ok(DataPeerClient {
                inner: DataPeerClientInner::Grpc(GrpcInlineClient::new(channel, "grpc")),
            }),
        },
    }
}

/// gRPC inline adapter：命令和内容都在 node_data proto 中传输。
struct GrpcInlineClient {
    data: NodeDataClient<Channel>,
    reported_mode: &'static str,
}

impl GrpcInlineClient {
    fn new(channel: Channel, reported_mode: &'static str) -> Self {
        Self {
            data: NodeDataClient::new(channel),
            reported_mode,
        }
    }

    /// gRPC 读文件：命令携带文件名和范围，响应的 data 字段直接携带内容。
    /// 不创建 RDMA endpoint，也不执行 NegotiateData 或 RDMA 就绪探测。
    async fn read(&mut self, name: &str, offset: u64, length: u32) -> PeerResult<Vec<u8>> {
        validate_length(length as usize)?;
        let reply = self
            .data
            .read(request_with_current_context(DataReadRequest {
                session_id: 0,
                transfer: DataTransfer::GrpcInline.into(),
                name: name.to_string(),
                offset,
                length,
            }))
            .await?
            .into_inner();
        if reply.length != length || reply.data.len() != length as usize {
            return Err(PeerError::coded(
                afs_error::CLIENT_PROTOCOL_VIOLATION,
                "gRPC read reply shape mismatch",
            ));
        }
        Ok(reply.data)
    }

    /// gRPC 写文件：命令的 data 字段携带全部内容，服务端写 Storage 后回复写入长度。
    /// Vec 在 Rust 中移入 Proto message；没有经过 RDMA 注册内存。
    async fn write(&mut self, name: &str, offset: u64, data: Vec<u8>) -> PeerResult<u32> {
        validate_length(data.len())?;
        let len = data.len();
        let reply = self
            .data
            .write(request_with_current_context(DataWriteRequest {
                session_id: 0,
                transfer: DataTransfer::GrpcInline.into(),
                name: name.to_string(),
                offset,
                length: data.len() as u32,
                data,
            }))
            .await?
            .into_inner();
        if reply.written as usize != len {
            return Err(PeerError::coded(
                afs_error::CLIENT_PROTOCOL_VIOLATION,
                "gRPC write reply count mismatch",
            ));
        }
        Ok(reply.written)
    }
}

#[cfg(feature = "rdma")]
/// RDMA adapter：命令走 gRPC，内容走已协商好的 RDMA endpoint。
///
/// `operation` 串行化同一 endpoint 上的读写，避免同一 MR 同时被两次 DMA 使用。
/// `poisoned` 是 fail-closed 开关：取消、CQ 错误、reply 形状错误都会让后续请求失败。
struct RdmaDataClient {
    data: NodeDataClient<Channel>,
    control: NodeControlClient<Channel>,
    endpoint: Arc<Mutex<RdmaEndpoint>>,
    operation: Arc<Mutex<()>>,
    poisoned: Arc<AtomicBool>,
    session_id: u64,
    reported_mode: &'static str,
    closed: bool,
}

#[cfg(feature = "rdma")]
impl RdmaDataClient {
    /// 建立 RDMA 数据客户端：
    /// 1. 本地 open endpoint + 导出 client_info；
    /// 2. gRPC NegotiateData 交给对端 control.rs；
    /// 3. 用返回的 server_info 连接 QP；
    /// 4. 通过 RDMA SEND_WITH_IMM 探测证明链路可用，发送完成后才发读写命令。
    ///
    /// 服务端首次数据请求消费接收完成；没有第二条 ReadyData gRPC，也没有每次重建连接。
    async fn connect(
        channel: Channel,
        device: String,
        reported_mode: &'static str,
    ) -> PeerResult<Self> {
        let mut endpoint = RdmaEndpoint::open(&device)
            .map_err(|error| PeerError::coded(afs_error::NODE_TRANSFER_UNAVAILABLE, error.0))?;
        let info = endpoint
            .info()
            .map_err(|error| PeerError::coded(afs_error::NODE_TRANSFER_UNAVAILABLE, error.0))?;
        let mut control = NodeControlClient::new(channel.clone());
        let negotiate = control
            .negotiate_data(request_with_current_context(NegotiateDataRequest {
                client_info: info.to_vec(),
                capacity: CAPACITY as u32,
                handshake_version: RDMA_HANDSHAKE_VERSION,
            }))
            .await?
            .into_inner();
        if !negotiate.rdma_supported {
            return Err(PeerError::coded(
                afs_error::NODE_TRANSFER_UNSUPPORTED,
                "peer does not support RDMA",
            ));
        }
        if negotiate.handshake_version != RDMA_HANDSHAKE_VERSION {
            close_session_best_effort(&mut control, negotiate.session_id).await;
            return Err(PeerError::coded(
                afs_error::NODE_RDMA_HANDSHAKE_VERSION,
                "peer uses unsupported RDMA handshake version",
            ));
        }
        let server_info = negotiate.server_info;
        // CQ 轮询属于阻塞 I/O；工作任务独占 endpoint，取消外层 future 不会释放在途资源。
        let connected = tokio::task::spawn_blocking(move || {
            endpoint.connect(&server_info)?;
            endpoint.send_probe(5000)?;
            Ok::<_, afs_transport::rdma::RdmaError>(endpoint)
        })
        .await
        .map_err(|error| PeerError::coded(afs_error::NODE_TRANSFER_UNAVAILABLE, error.to_string()))
        .and_then(|result| {
            result.map_err(|error| {
                PeerError::coded(afs_error::NODE_TRANSFER_UNAVAILABLE, error.to_string())
            })
        });
        let endpoint = match connected {
            Ok(endpoint) => endpoint,
            Err(error) => {
                close_session_best_effort(&mut control, negotiate.session_id).await;
                return Err(error);
            }
        };
        Ok(Self {
            data: NodeDataClient::new(channel),
            control,
            endpoint: Arc::new(Mutex::new(endpoint)),
            operation: Arc::new(Mutex::new(())),
            poisoned: Arc::new(AtomicBool::new(false)),
            session_id: negotiate.session_id,
            reported_mode,
            closed: false,
        })
    }

    /// RDMA 读文件客户端流程：
    /// 1. 发 node_data Read 命令，告诉服务端文件名、offset、len、session_id；
    /// 2. 服务端从 Storage 读文件，并 RDMA WRITE 到客户端 MR；
    /// 3. gRPC reply 返回后，客户端从本地 MR `get_local` 拷贝 bytes。
    async fn read(&mut self, name: &str, offset: u64, length: u32) -> PeerResult<Vec<u8>> {
        validate_open(self.closed, &self.poisoned)?;
        validate_length(length as usize)?;
        let cancel_guard = CancelPoisonGuard::new(self.poisoned.clone());
        let mut data_client = self.data.clone();
        let operation = self.operation.clone();
        let endpoint = self.endpoint.clone();
        let poisoned = self.poisoned.clone();
        let name = name.to_string();
        let session_id = self.session_id;
        let result = tokio::spawn(
            async move {
                let _operation = operation.lock_owned().await;
                validate_open(false, &poisoned)?;
                let reply = data_client
                    .read(request_with_current_context(DataReadRequest {
                        session_id,
                        transfer: DataTransfer::RdmaOneSided.into(),
                        name,
                        offset,
                        length,
                    }))
                    .await
                    .map_err(|error| {
                        poisoned.store(true, Ordering::SeqCst);
                        PeerError::from(error)
                    })?
                    .into_inner();
                if !reply.data.is_empty() || reply.length != length {
                    return Err(poison(
                        &poisoned,
                        afs_error::CLIENT_PROTOCOL_VIOLATION,
                        "RDMA read reply shape mismatch",
                    ));
                }
                let mut endpoint = endpoint.lock().await;
                endpoint.get_local(length as usize).map_err(|error| {
                    poison(&poisoned, afs_error::NODE_TRANSFER_UNAVAILABLE, error.0)
                })
            }
            .in_current_span(),
        )
        .await
        .map_err(|error| PeerError::coded(afs_error::CLIENT_WORKER_FAILED, error.to_string()))?;
        if result.is_ok() {
            cancel_guard.disarm();
        }
        result
    }

    /// RDMA 写文件客户端流程：
    /// 1. 客户端先把待写 bytes `put_local` 到自己的 MR；
    /// 2. 发 node_data Write 命令，proto 带文件名、范围和会话，不带文件内容；
    /// 3. 服务端 RDMA READ 拉取客户端 MR 内容并写 Storage；
    /// 4. reply.written 必须等于 len，否则 poison。
    async fn write(&mut self, name: &str, offset: u64, data: Vec<u8>) -> PeerResult<u32> {
        validate_open(self.closed, &self.poisoned)?;
        validate_length(data.len())?;
        let cancel_guard = CancelPoisonGuard::new(self.poisoned.clone());
        let mut data_client = self.data.clone();
        let operation = self.operation.clone();
        let endpoint = self.endpoint.clone();
        let poisoned = self.poisoned.clone();
        let name = name.to_string();
        let len = data.len();
        let session_id = self.session_id;
        let result = tokio::spawn(
            async move {
                let _operation = operation.lock_owned().await;
                validate_open(false, &poisoned)?;
                {
                    let mut endpoint = endpoint.lock().await;
                    endpoint.put_local(&data).map_err(|error| {
                        poison(&poisoned, afs_error::NODE_TRANSFER_UNAVAILABLE, error.0)
                    })?;
                }
                let reply = data_client
                    .write(request_with_current_context(DataWriteRequest {
                        session_id,
                        transfer: DataTransfer::RdmaOneSided.into(),
                        name,
                        offset,
                        data: Vec::new(),
                        length: len as u32,
                    }))
                    .await
                    .map_err(|error| {
                        poisoned.store(true, Ordering::SeqCst);
                        PeerError::from(error)
                    })?
                    .into_inner();
                if reply.written as usize != len {
                    return Err(poison(
                        &poisoned,
                        afs_error::CLIENT_PROTOCOL_VIOLATION,
                        "RDMA write reply count mismatch",
                    ));
                }
                Ok(reply.written)
            }
            .in_current_span(),
        )
        .await
        .map_err(|error| PeerError::coded(afs_error::CLIENT_WORKER_FAILED, error.to_string()))?;
        if result.is_ok() {
            cancel_guard.disarm();
        }
        result
    }

    async fn close(&mut self) -> PeerResult<()> {
        if self.closed {
            return Ok(());
        }
        let _operation = self.operation.clone().lock_owned().await;
        self.closed = true;
        self.poisoned.store(true, Ordering::SeqCst);
        self.control
            .close_data(request_with_current_context(CloseDataRequest {
                session_id: self.session_id,
            }))
            .await?;
        Ok(())
    }
}

#[cfg(feature = "rdma")]
impl Drop for RdmaDataClient {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let mut control = self.control.clone();
        let session_id = self.session_id;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = control
                    .close_data(request_with_current_context(CloseDataRequest {
                        session_id,
                    }))
                    .await;
            });
        }
    }
}

#[cfg(feature = "rdma")]
async fn close_session_best_effort(control: &mut NodeControlClient<Channel>, session_id: u64) {
    let _ = control
        .close_data(request_with_current_context(CloseDataRequest {
            session_id,
        }))
        .await;
}

#[cfg(feature = "rdma")]
async fn connect_rdma(
    channel: Channel,
    device: Option<String>,
    reported_mode: &'static str,
) -> PeerResult<DataPeerClient> {
    let device = device.ok_or_else(|| {
        PeerError::coded(
            afs_error::CLIENT_ARGUMENT_INVALID,
            "RDMA mode requires a device",
        )
    })?;
    Ok(DataPeerClient {
        inner: DataPeerClientInner::Rdma(Box::new(
            RdmaDataClient::connect(channel, device, reported_mode).await?,
        )),
    })
}

#[cfg(not(feature = "rdma"))]
async fn connect_rdma(
    _channel: Channel,
    _device: Option<String>,
    _reported_mode: &'static str,
) -> PeerResult<DataPeerClient> {
    Err(PeerError::coded(
        afs_error::NODE_TRANSFER_UNSUPPORTED,
        "RDMA feature is not enabled",
    ))
}

#[cfg(feature = "ownerfs")]
fn root_access(grant: &RootGrant) -> RootAccess {
    RootAccess {
        root_id: grant.id.0.clone(),
        root_epoch: grant.epoch,
        access_generation: grant.access_generation,
        holder_node_id: grant.holder_node_id.clone(),
        home_node_id: grant.home_node_id.clone(),
        session_id: grant.session_id.clone(),
        fencing_token: grant.fencing_token.clone(),
        home_session_id: grant.home_session_id.clone(),
    }
}

#[cfg(feature = "ownerfs")]
fn file_identity(identity: &FileIdentity) -> PbFileIdentity {
    PbFileIdentity {
        opaque: identity.0.clone(),
    }
}

#[cfg(feature = "ownerfs")]
fn file_handle(file: &RemoteFile) -> OwnerHandle {
    OwnerHandle {
        opaque: file.handle.clone(),
    }
}

#[cfg(feature = "ownerfs")]
fn directory_handle(directory: &RemoteDirectory) -> OwnerDirectoryHandle {
    OwnerDirectoryHandle {
        opaque: directory.handle.clone(),
    }
}

#[cfg(feature = "ownerfs")]
fn grpc_plane() -> DataPlane {
    DataPlane {
        transfer: DataTransfer::GrpcInline.into(),
        rdma_session_id: 0,
        buffer_offset: 0,
    }
}

#[cfg(feature = "ownerfs")]
fn remote_file(
    grant: &RootGrant,
    identity: FileIdentity,
    handle: Vec<u8>,
    owner_session_id: String,
) -> RemoteFile {
    RemoteFile {
        root_id: grant.id.clone(),
        owner_node_id: grant.home_node_id.clone(),
        owner_session_id,
        identity,
        handle,
    }
}

#[cfg(feature = "ownerfs")]
fn owner_entry(grant: &RootGrant, attr: Option<OwnerFileAttr>) -> afs_error::Result<OwnerEntry> {
    let attr = attr.ok_or_else(|| protocol_error("OwnerFileAttr missing"))?;
    let identity = attr
        .identity
        .clone()
        .ok_or_else(|| protocol_error("OwnerFileAttr missing identity"))?;
    Ok(OwnerEntry {
        root_id: grant.id.clone(),
        identity: FileIdentity(identity.opaque),
        attributes: file_attributes(attr)?,
    })
}

#[cfg(feature = "ownerfs")]
fn file_attributes(attr: OwnerFileAttr) -> afs_error::Result<FileAttributes> {
    let kind = match OwnerFileKind::try_from(attr.kind)
        .map_err(|_| protocol_error("unknown OwnerFileKind"))?
    {
        OwnerFileKind::Regular => FileKind::Regular,
        OwnerFileKind::Directory => FileKind::Directory,
        OwnerFileKind::Symlink => FileKind::Symlink,
        OwnerFileKind::Unspecified
        | OwnerFileKind::BlockDevice
        | OwnerFileKind::CharDevice
        | OwnerFileKind::Fifo
        | OwnerFileKind::Socket => {
            return Err(protocol_error(
                "OwnerFileKind is not supported by VFS types",
            ));
        }
    };
    Ok(FileAttributes {
        kind,
        size: attr.size,
        mode: attr.mode,
        uid: attr.uid,
        gid: attr.gid,
        nlink: attr.nlink,
        atime: ns_to_time(attr.atime_ns),
        mtime: ns_to_time(attr.mtime_ns),
        ctime: ns_to_time(attr.ctime_ns),
    })
}

#[cfg(feature = "ownerfs")]
fn owner_set_attr(change: &AttributeChange) -> OwnerSetAttr {
    OwnerSetAttr {
        mode: change.mode,
        uid: change.uid,
        gid: change.gid,
        size: change.size,
        atime_ns: change.atime.map(time_to_ns),
        mtime_ns: change.mtime.map(time_to_ns),
    }
}

#[cfg(feature = "ownerfs")]
fn ns_to_time(ns: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(ns)
}

#[cfg(feature = "ownerfs")]
fn time_to_ns(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(feature = "ownerfs")]
fn protocol_error(message: &'static str) -> afs_error::Error {
    afs_error::Error::coded(afs_error::CLIENT_PROTOCOL_VIOLATION, message)
}

async fn connect_channel(endpoint: &str, timeout: Duration) -> PeerResult<Channel> {
    Ok(Endpoint::from_shared(endpoint.to_string())?
        .connect_timeout(timeout)
        .timeout(timeout)
        .connect()
        .await?)
}

fn validate_length(length: usize) -> PeerResult<()> {
    if length > MAX_TRANSFER_BYTES {
        return Err(PeerError::coded(
            afs_error::CLIENT_ARGUMENT_INVALID,
            "transfer exceeds 1MiB",
        ));
    }
    Ok(())
}

#[cfg(feature = "rdma")]
fn validate_open(closed: bool, poisoned: &AtomicBool) -> PeerResult<()> {
    if closed {
        return Err(PeerError::coded(
            afs_error::NODE_RDMA_CLOSED,
            "RDMA session is closed",
        ));
    }
    if poisoned.load(Ordering::SeqCst) {
        return Err(PeerError::coded(
            afs_error::NODE_RDMA_SESSION_POISONED,
            "RDMA session is poisoned",
        ));
    }
    Ok(())
}

#[cfg(feature = "rdma")]
fn poison(
    poisoned: &AtomicBool,
    code: afs_error::ErrorCode,
    message: impl Into<String>,
) -> PeerError {
    poisoned.store(true, Ordering::SeqCst);
    PeerError::coded(code, message)
}

#[cfg(feature = "rdma")]
/// 取消保护：调用者 drop read/write future 时，把 session 标成 poisoned。
///
/// 内部 spawned task 仍持有 endpoint Arc，保证 DMA/CQ 生命周期不会因为外层 future
/// 取消而提前释放；但业务结果已经对调用者未知，所以后续复用必须失败。
struct CancelPoisonGuard {
    poisoned: Arc<AtomicBool>,
    disarmed: bool,
}

#[cfg(feature = "rdma")]
impl CancelPoisonGuard {
    fn new(poisoned: Arc<AtomicBool>) -> Self {
        Self {
            poisoned,
            disarmed: false,
        }
    }

    fn disarm(mut self) {
        self.disarmed = true;
    }
}

#[cfg(feature = "rdma")]
impl Drop for CancelPoisonGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            self.poisoned.store(true, Ordering::SeqCst);
        }
    }
}

impl From<PeerError> for afs_error::Error {
    fn from(error: PeerError) -> Self {
        error.0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "rdma")]
    fn poisoning_preserves_failure_category() {
        for code in [
            afs_error::CLIENT_PROTOCOL_VIOLATION,
            afs_error::NODE_TRANSFER_UNAVAILABLE,
        ] {
            let flag = std::sync::atomic::AtomicBool::new(false);
            let error = super::poison(&flag, code, "diagnostic");
            assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(error.code(), code);
        }
    }
    use super::*;
    #[cfg(feature = "ownerfs")]
    use crate::node::rpc::data::{
        OwnerFilesHandler, PeerAuthenticator, make_owner_files_server_with_handler,
    };
    #[cfg(feature = "ownerfs")]
    use crate::node::vfs::ownerfs::{
        files::{FileIdentity, RemoteFile},
        remote::RemoteFiles,
        root::{RootGrant, RootId, RootRight},
    };
    #[cfg(feature = "ownerfs")]
    use afs_protocol::node_data::OwnerReleaseReply;
    use afs_protocol::node_data::{
        DataReadReply, DataWriteReply,
        node_data_server::{NodeData, NodeDataServer},
    };
    #[cfg(feature = "ownerfs")]
    use std::sync::atomic::{AtomicUsize as TestAtomicUsize, Ordering as TestOrdering};
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, Status, transport::Server};

    #[derive(Clone, Copy)]
    enum BadReplyKind {
        ReadShape,
        WriteCount,
    }

    #[derive(Clone, Copy)]
    struct BadDataService {
        kind: BadReplyKind,
    }

    #[tonic::async_trait]
    impl NodeData for BadDataService {
        async fn read(
            &self,
            _request: Request<DataReadRequest>,
        ) -> Result<Response<DataReadReply>, Status> {
            match self.kind {
                BadReplyKind::ReadShape => Ok(Response::new(DataReadReply {
                    length: 8,
                    data: b"short".to_vec(),
                })),
                BadReplyKind::WriteCount => Err(Status::unimplemented("read not used")),
            }
        }

        async fn write(
            &self,
            request: Request<DataWriteRequest>,
        ) -> Result<Response<DataWriteReply>, Status> {
            match self.kind {
                BadReplyKind::ReadShape => Err(Status::unimplemented("write not used")),
                BadReplyKind::WriteCount => Ok(Response::new(DataWriteReply {
                    written: request.into_inner().data.len() as u32 + 1,
                })),
            }
        }
    }

    #[tokio::test]
    async fn grpc_client_rejects_read_reply_shape_mismatch() {
        let (endpoint, server) = spawn_bad_data_server(BadReplyKind::ReadShape).await;
        let mut client = connect_data_client(DataClientOptions {
            endpoint,
            mode: DataMode::Grpc,
            rdma_device: None,
            timeout: Duration::from_secs(5),
        })
        .await
        .unwrap();

        let error = client.read("bad.bin", 0, 8).await.unwrap_err();

        assert!(error.to_string().contains("read reply shape mismatch"));
        server.abort();
    }

    #[tokio::test]
    async fn grpc_client_rejects_write_count_mismatch() {
        let (endpoint, server) = spawn_bad_data_server(BadReplyKind::WriteCount).await;
        let mut client = connect_data_client(DataClientOptions {
            endpoint,
            mode: DataMode::Grpc,
            rdma_device: None,
            timeout: Duration::from_secs(5),
        })
        .await
        .unwrap();

        let error = client
            .write("bad.bin", 0, b"abcdefgh".to_vec())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("write reply count mismatch"));
        server.abort();
    }

    #[tokio::test]
    #[cfg(feature = "ownerfs")]
    async fn owner_release_retries_transient_failures_without_losing_handle() {
        let handler = std::sync::Arc::new(ReleaseTestHandler::transient_failures(2));
        let (channel, server) = spawn_owner_release_server(handler.clone()).await;
        let client = owner_files_client_from_channel_with_runtime(
            channel,
            tokio::runtime::Handle::current(),
        );
        let grant = test_grant();
        let file = test_remote_file(b"handle-retry".to_vec());

        client.release(&grant, file).unwrap();

        wait_until(Duration::from_secs(2), || handler.attempts() >= 3).await;
        assert_eq!(handler.attempts(), 3);
        assert_eq!(handler.seen_handles(), vec![b"handle-retry".to_vec(); 3]);
        server.abort();
    }

    #[tokio::test]
    #[cfg(feature = "ownerfs")]
    async fn owner_release_treats_stale_handle_as_idempotent_success() {
        let handler = std::sync::Arc::new(ReleaseTestHandler::stale_handle());
        let (channel, server) = spawn_owner_release_server(handler.clone()).await;
        let client = owner_files_client_from_channel_with_runtime(
            channel,
            tokio::runtime::Handle::current(),
        );

        client
            .release(&test_grant(), test_remote_file(b"handle-stale".to_vec()))
            .unwrap();

        wait_until(Duration::from_secs(2), || handler.attempts() >= 1).await;
        tokio::time::sleep(RELEASE_INITIAL_BACKOFF * 3).await;
        assert_eq!(handler.attempts(), 1);
        assert_eq!(handler.seen_handles(), vec![b"handle-stale".to_vec()]);
        server.abort();
    }

    #[cfg(feature = "rdma")]
    #[test]
    fn cancellation_guard_poisons_unfinished_session_and_disarm_preserves_it() {
        let poisoned = Arc::new(AtomicBool::new(false));
        {
            let _guard = CancelPoisonGuard::new(poisoned.clone());
        }
        assert!(poisoned.load(Ordering::SeqCst));

        let poisoned = Arc::new(AtomicBool::new(false));
        CancelPoisonGuard::new(poisoned.clone()).disarm();
        assert!(!poisoned.load(Ordering::SeqCst));
    }

    async fn spawn_bad_data_server(kind: BadReplyKind) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            Server::builder()
                .add_service(NodeDataServer::new(BadDataService { kind }))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        (endpoint, server)
    }

    #[cfg(feature = "ownerfs")]
    struct AllowPeer;

    #[cfg(feature = "ownerfs")]
    impl PeerAuthenticator for AllowPeer {
        fn authenticate(
            &self,
            _metadata: &tonic::metadata::MetadataMap,
            _remote_addr: Option<std::net::SocketAddr>,
            _peer_cert_der: Option<&[u8]>,
        ) -> afs_error::Result<String> {
            Ok("node-b".to_owned())
        }
    }

    #[cfg(feature = "ownerfs")]
    enum ReleaseFailureMode {
        Transient { remaining: TestAtomicUsize },
        Stale,
    }

    #[cfg(feature = "ownerfs")]
    struct ReleaseTestHandler {
        mode: ReleaseFailureMode,
        attempts: TestAtomicUsize,
        seen_handles: StdMutex<Vec<Vec<u8>>>,
    }

    #[cfg(feature = "ownerfs")]
    impl ReleaseTestHandler {
        fn transient_failures(failures: usize) -> Self {
            Self {
                mode: ReleaseFailureMode::Transient {
                    remaining: TestAtomicUsize::new(failures),
                },
                attempts: TestAtomicUsize::new(0),
                seen_handles: StdMutex::new(Vec::new()),
            }
        }

        fn stale_handle() -> Self {
            Self {
                mode: ReleaseFailureMode::Stale,
                attempts: TestAtomicUsize::new(0),
                seen_handles: StdMutex::new(Vec::new()),
            }
        }

        fn attempts(&self) -> usize {
            self.attempts.load(TestOrdering::SeqCst)
        }

        fn seen_handles(&self) -> Vec<Vec<u8>> {
            self.seen_handles.lock().unwrap().clone()
        }
    }

    #[cfg(feature = "ownerfs")]
    impl OwnerFilesHandler for ReleaseTestHandler {
        fn release(
            &self,
            _authenticated_peer_node_id: &str,
            request: OwnerReleaseRequest,
        ) -> afs_error::Result<OwnerReleaseReply> {
            self.attempts.fetch_add(1, TestOrdering::SeqCst);
            let handle = request
                .handle
                .ok_or_else(|| {
                    afs_error::Error::coded(
                        afs_error::CLIENT_ARGUMENT_INVALID,
                        "missing test handle",
                    )
                })?
                .opaque;
            self.seen_handles.lock().unwrap().push(handle);
            match &self.mode {
                ReleaseFailureMode::Transient { remaining } => {
                    if remaining
                        .fetch_update(TestOrdering::SeqCst, TestOrdering::SeqCst, |current| {
                            (current > 0).then(|| current - 1)
                        })
                        .is_ok()
                    {
                        return Err(afs_error::Error::coded(
                            afs_error::CLIENT_CONNECTION_UNAVAILABLE,
                            "transient release failure",
                        ));
                    }
                    Ok(OwnerReleaseReply {})
                }
                ReleaseFailureMode::Stale => Err(afs_error::Error::coded(
                    afs_error::NODE_OWNER_STALE_HANDLE,
                    "stale release handle",
                )),
            }
        }
    }

    #[cfg(feature = "ownerfs")]
    async fn spawn_owner_release_server(
        handler: std::sync::Arc<ReleaseTestHandler>,
    ) -> (Channel, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            Server::builder()
                .add_service(make_owner_files_server_with_handler(
                    handler,
                    std::sync::Arc::new(AllowPeer),
                ))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        let channel = Endpoint::from_shared(endpoint)
            .unwrap()
            .connect()
            .await
            .unwrap();
        (channel, server)
    }

    #[cfg(feature = "ownerfs")]
    fn test_grant() -> RootGrant {
        RootGrant {
            id: RootId("root-a".to_owned()),
            epoch: 1,
            home_node_id: "node-a".to_owned(),
            home_session_id: "home-session-a".to_owned(),
            holder_node_id: "node-b".to_owned(),
            session_id: "grant-session-b".to_owned(),
            access_generation: 1,
            rights: vec![RootRight::Write],
            fencing_token: "fence-a".to_owned(),
        }
    }

    #[cfg(feature = "ownerfs")]
    fn test_remote_file(handle: Vec<u8>) -> RemoteFile {
        RemoteFile {
            root_id: RootId("root-a".to_owned()),
            owner_node_id: "node-a".to_owned(),
            owner_session_id: "home-session-a".to_owned(),
            identity: FileIdentity(b"identity-a".to_vec()),
            handle,
        }
    }

    #[cfg(feature = "ownerfs")]
    async fn wait_until(deadline: Duration, condition: impl Fn() -> bool) {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("condition did not become true before timeout");
    }
}

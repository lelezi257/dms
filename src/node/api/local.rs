//! afs-node 内的 Local SDK 服务端。
//!
//! 这层只处理同机 SDK 的高性能数据入口：gRPC over UDS 是控制面，memfd/FD pass
//! 是数据面。服务端不会从 gRPC payload 里收发文件内容，也不会在本层接 RDMA。
//!
//! 写入 E2E：SDK 给 source grant -> node 通过 fd broker 取 memfd fd -> `read_fd_at`
//! 从 fd copy bytes -> 写入 Storage。
//!
//! 读取 E2E：node 从 Storage 读 bytes -> 通过 target grant 取 fd -> `write_fd_at`
//! copy 到 SDK memfd -> SDK 再本地读取。
//!
//! 当前安全边界是 trusted local host：UDS socket 权限 + 一次性 token/TTL/会话标识
//! 限制同机误用；它不是跨主机认证授权系统。

use std::{
    io,
    os::{
        fd::OwnedFd,
        unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
};

use afs_metrics::{IntCounterVec, Opts, Registry};
use afs_protocol::local_api::{
    LocalReadReply, LocalReadRequest, LocalShmGrant, LocalWriteReply, LocalWriteRequest,
    local_data_server::{LocalData, LocalDataServer},
};
use afs_transport::shm::{
    BrokerToken, FdBrokerClient, FdRequest, ShmError, read_fd_at, write_fd_at,
};
use tokio::{net::UnixListener, sync::oneshot, task::JoinHandle};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{Request, Response, Status, transport::Server};

use crate::node::storage::{Storage, StorageError};

pub async fn serve_local_api(
    storage: impl Into<Arc<Storage>>,
    socket_path: impl AsRef<Path>,
) -> io::Result<LocalApiServer> {
    serve_local_api_with_options(storage, socket_path, LocalApiOptions::default()).await
}

#[derive(Clone, Debug, Default)]
pub struct LocalApiOptions {
    pub metrics_registry: Option<Registry>,
}

/// 启动本机 Local API。
///
/// socket_path 必须是调用方独占的本机路径；如果底层文件系统不支持对 Unix socket
/// chmod，则要求父目录本身是 owner-only。启动失败会尽量删除自己创建的 socket。
pub async fn serve_local_api_with_options(
    storage: impl Into<Arc<Storage>>,
    socket_path: impl AsRef<Path>,
    options: LocalApiOptions,
) -> io::Result<LocalApiServer> {
    let socket_path = socket_path.as_ref().to_path_buf();
    if socket_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("local api socket already exists: {}", socket_path.display()),
        ));
    }
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let metrics = options
        .metrics_registry
        .as_ref()
        .map(LocalApiMetrics::register)
        .transpose()?;
    let listener = UnixListener::bind(&socket_path)?;
    let metadata = std::fs::symlink_metadata(&socket_path)?;
    let socket_identity = SocketIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    };
    if let Err(error) = secure_bound_socket(&socket_path).await {
        let _ = remove_owned_socket_sync(&socket_path, socket_identity);
        return Err(error);
    }
    let incoming = UnixListenerStream::new(listener);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let service = LocalDataServer::new(LocalDataService {
        storage: storage.into(),
        metrics,
    });
    let task = tokio::spawn(async move {
        afs_transport::grpc::GrpcConfig::default()
            .configure_server(Server::builder())
            .layer(afs_tracing::GrpcServerTraceLayer::default())
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await
            .map_err(|error| io::Error::other(error.to_string()))
    });
    Ok(LocalApiServer {
        shutdown_tx: Some(shutdown_tx),
        task: Some(task),
        socket_path,
        socket_identity,
    })
}

/// Local API 运行句柄。
///
/// `shutdown()` 是正常路径；`Drop` 是兜底路径，只发关闭信号并清理自有 socket，
/// 不承诺等待所有运行中请求完成。root/node supervisor 可用 `abort_handle()` 观察或中止。
pub struct LocalApiServer {
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<io::Result<()>>>,
    socket_path: PathBuf,
    socket_identity: SocketIdentity,
}

impl LocalApiServer {
    /// 供 node supervisor 轮询服务 task 是否已经自然退出。
    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// 返回 task abort handle，便于上层统一监督；LocalApiServer 本身仍保留 shutdown 语义。
    pub fn abort_handle(&self) -> Option<tokio::task::AbortHandle> {
        self.task.as_ref().map(JoinHandle::abort_handle)
    }

    /// 正常关闭：先通知 tonic server，再等待 task 退出，最后只删除自己创建的 socket。
    pub async fn shutdown(mut self) -> io::Result<()> {
        self.signal_shutdown();
        let task_result = if let Some(task) = self.task.take() {
            task.await
                .map_err(|error| io::Error::other(error.to_string()))?
        } else {
            Ok(())
        };
        let cleanup_result = remove_owned_socket(&self.socket_path, self.socket_identity).await;
        task_result?;
        cleanup_result
    }

    fn signal_shutdown(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

impl Drop for LocalApiServer {
    fn drop(&mut self) {
        self.signal_shutdown();
        let _ = remove_owned_socket_sync(&self.socket_path, self.socket_identity);
    }
}

#[derive(Clone)]
struct LocalDataService {
    storage: Arc<Storage>,
    metrics: Option<LocalApiMetrics>,
}

#[derive(Clone)]
struct LocalApiMetrics {
    requests: IntCounterVec,
}

#[derive(Clone, Copy)]
struct SocketIdentity {
    dev: u64,
    ino: u64,
}

impl LocalApiMetrics {
    fn register(registry: &Registry) -> io::Result<Self> {
        registry
            .get_or_register(|registry| {
                let requests = IntCounterVec::new(
                    Opts::new(
                        "afs_local_api_requests_total",
                        "Completed local SDK requests",
                    ),
                    &["operation", "result"],
                )?;
                registry.register(Box::new(requests.clone()))?;
                Ok(Self { requests })
            })
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn record(&self, operation: &'static str, ok: bool) {
        self.requests
            .with_label_values(&[operation, if ok { "ok" } else { "error" }])
            .inc();
    }
}

impl LocalDataService {
    fn record(&self, operation: &'static str, ok: bool) {
        if let Some(metrics) = &self.metrics {
            metrics.record(operation, ok);
        }
    }
}

#[tonic::async_trait]
impl LocalData for LocalDataService {
    async fn write(
        &self,
        request: Request<LocalWriteRequest>,
    ) -> Result<Response<LocalWriteReply>, Status> {
        let outcome = self.write_inner(request.into_inner()).await;
        self.record("write", outcome.is_ok());
        match &outcome {
            Ok(reply) => {
                afs_logging::info!("local_api.write";"result"=>"ok","written"=>reply.written);
            }
            Err(error) => {
                afs_logging::warn!("local_api.write";"result"=>"error","error"=>error.to_string());
            }
        }
        outcome.map(Response::new)
    }

    async fn read(
        &self,
        request: Request<LocalReadRequest>,
    ) -> Result<Response<LocalReadReply>, Status> {
        let outcome = self.read_inner(request.into_inner()).await;
        self.record("read", outcome.is_ok());
        match &outcome {
            Ok(reply) => {
                afs_logging::info!("local_api.read";"result"=>"ok","length"=>reply.length);
            }
            Err(error) => {
                afs_logging::warn!("local_api.read";"result"=>"error","error"=>error.to_string());
            }
        }
        outcome.map(Response::new)
    }
}

impl LocalDataService {
    async fn write_inner(&self, request: LocalWriteRequest) -> Result<LocalWriteReply, Status> {
        // 控制面必须带 source grant；没有 grant 就直接失败，不能退化为 gRPC payload 写入。
        let grant = request
            .source
            .ok_or_else(|| Status::invalid_argument("missing SHM source grant"))?;
        if request.length != grant.length {
            return Err(Status::invalid_argument(
                "request length does not match SHM grant",
            ));
        }
        let (fd, offset) = request_grant_fd(grant).await?;
        // 这里会从 memfd copy 出 Vec；第一版不是 zero-copy。seal 只校验 fd 大小稳定。
        let data = read_fd_at(fd, offset, request.length as usize).map_err(shm_status)?;
        let written = self
            .storage
            .write(&request.name, request.file_offset, data)
            .await
            .map_err(storage_status)?;
        Ok(LocalWriteReply {
            written: written as u32,
        })
    }

    async fn read_inner(&self, request: LocalReadRequest) -> Result<LocalReadReply, Status> {
        // 读取也必须带 target grant；node 把 Storage 读出的 bytes copy 进 SDK memfd。
        let grant = request
            .target
            .ok_or_else(|| Status::invalid_argument("missing SHM target grant"))?;
        if request.length != grant.length {
            return Err(Status::invalid_argument(
                "request length does not match SHM grant",
            ));
        }
        let data = self
            .storage
            .read(&request.name, request.file_offset, request.length)
            .await
            .map_err(storage_status)?;
        let length = u32::try_from(data.len())
            .map_err(|_| Status::internal("storage returned too many bytes"))?;
        let (fd, offset) = request_grant_fd(grant).await?;
        write_fd_at(fd, offset, &data).map_err(shm_status)?;
        Ok(LocalReadReply { length })
    }
}

// 通过 SDK 提供的 broker socket 取一次性 fd。这个函数运行在同机信任边界内：
// token/session/region 防误用与重放，UDS 权限限制本机其它用户访问；不承担远端认证。
async fn request_grant_fd(grant: LocalShmGrant) -> Result<(OwnedFd, usize), Status> {
    let offset = usize::try_from(grant.region_offset)
        .map_err(|_| Status::invalid_argument("SHM offset is too large"))?;
    let _len = grant_len(&grant)?;
    let token = BrokerToken::new(grant.token).map_err(shm_status)?;
    let request = FdRequest::new(token, grant.session_id, grant.region_id);
    let path = PathBuf::from(grant.broker_socket_path);
    let fd = tokio::task::spawn_blocking(move || FdBrokerClient::request_fd(path, &request))
        .await
        .map_err(|error| Status::internal(error.to_string()))?
        .map_err(shm_status)?;
    Ok((fd, offset))
}

fn grant_len(grant: &LocalShmGrant) -> Result<usize, Status> {
    let offset = usize::try_from(grant.region_offset)
        .map_err(|_| Status::invalid_argument("SHM offset is too large"))?;
    let length = usize::try_from(grant.length)
        .map_err(|_| Status::invalid_argument("SHM length is too large"))?;
    offset
        .checked_add(length.max(1))
        .ok_or_else(|| Status::invalid_argument("SHM range overflows"))
}

// Local API socket 的最小权限策略。优先 chmod socket 到 0600；如果挂载层不支持
// socket chmod，则只接受父目录已经 owner-only 的场景。
async fn secure_bound_socket(path: &Path) -> io::Result<()> {
    match tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            let parent = path.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent")
            })?;
            let parent_mode = tokio::fs::metadata(parent).await?.permissions().mode() & 0o077;
            if parent_mode == 0 { Ok(()) } else { Err(error) }
        }
        Err(error) => Err(error),
    }
}

fn remove_owned_socket_sync(path: &Path, identity: SocketIdentity) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_socket()
                && metadata.dev() == identity.dev
                && metadata.ino() == identity.ino =>
        {
            std::fs::remove_file(path)
        }
        Ok(_) | Err(_) => Ok(()),
    }
}

async fn remove_owned_socket(path: &Path, identity: SocketIdentity) -> io::Result<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata)
            if metadata.file_type().is_socket()
                && metadata.dev() == identity.dev
                && metadata.ino() == identity.ino =>
        {
            tokio::fs::remove_file(path).await
        }
        Ok(_) | Err(_) => Ok(()),
    }
}

fn storage_status(error: StorageError) -> Status {
    match error {
        StorageError::BadName | StorageError::TooLarge | StorageError::Range => {
            Status::invalid_argument(error.to_string())
        }
        StorageError::UnsafeFileType => Status::failed_precondition(error.to_string()),
        StorageError::Io(inner) if inner.kind() == io::ErrorKind::NotFound => {
            Status::not_found(inner.to_string())
        }
        StorageError::Io(inner) => Status::unavailable(inner.to_string()),
        StorageError::Join(inner) => Status::internal(inner.to_string()),
    }
}

// SHM 错误映射成 gRPC status；这些都是控制面错误码，不表示内容曾经过 gRPC payload。
fn shm_status(error: ShmError) -> Status {
    match error {
        ShmError::InvalidArgument { .. } | ShmError::InvalidToken | ShmError::Protocol(_) => {
            Status::invalid_argument(error.to_string())
        }
        ShmError::Unsupported => Status::failed_precondition(error.to_string()),
        ShmError::Syscall(_, inner) => Status::unavailable(inner.to_string()),
        ShmError::Poisoned => Status::internal(error.to_string()),
    }
}

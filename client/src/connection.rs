//! 本机 SDK 连接：只连同机 afs-node 的 UDS。
//!
//! E2E 写入流程：调用方 `write(name, offset, data)` -> SDK 创建 memfd 并写入 data
//! -> 启动一次性 fd broker -> gRPC 只携带 SHM grant -> node 通过 broker 取 fd
//! -> node 从 fd 复制 bytes 到 Storage。
//!
//! E2E 读取流程：SDK 创建空 memfd -> gRPC 携带 target grant -> node 从 Storage 读
//! -> node 把 bytes 复制到 fd -> SDK 从本地 memfd 取回 Vec。
//!
//! 注意：这里当前是“fd 传递 + pread/pwrite copy”，还不是真正 zero-copy；但它已经
//! 避免把文件内容塞进 gRPC message，也没有 RDMA 和远端 SDK 连接。

use std::{fmt, path::PathBuf, sync::Arc, time::Duration};

use afs_protocol::local_api::{
    LocalReadRequest, LocalWriteRequest, local_data_client::LocalDataClient,
};
use afs_tracing::{Instrument, TracedChannel, request_with_current_context, traced_channel};
use afs_transport::grpc::GrpcConfig;
use hyper_util::rt::TokioIo;
use tokio::{sync::Semaphore, task::JoinHandle};
use tonic::transport::Endpoint;
use tower::service_fn;

use crate::buffer::OperationBuffer;

#[derive(Clone, Debug)]
pub struct LocalClientConfig {
    socket_path: PathBuf,
}

impl LocalClientConfig {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }
}

#[derive(Clone)]
pub struct LocalClient {
    inner: LocalDataClient<TracedChannel>,
    request_timeout: Duration,
}

// 每个本地 SDK 操作都会临时占用一个 memfd 和一个 fd broker。
// 这里的 slot 上限是进程内背压，避免调用方用取消/并发把 fd、socket、线程资源打满。
// permit 使用 OwnedSemaphorePermit 交给 worker 持有：即使外层 future 被取消，worker
// 仍负责等待 broker 结束后再释放资源。
const SDK_SHM_SLOTS: usize = 64;
static BROKER_ADMISSION: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(SDK_SHM_SLOTS)));

pub fn max_parallel_shm_operations() -> usize {
    SDK_SHM_SLOTS
}

#[derive(Debug)]
pub enum LocalClientError {
    InvalidArgument {
        field: &'static str,
        reason: &'static str,
    },
    Transport(tonic::transport::Error),
    Status(tonic::Status),
    Shm(afs_transport::shm::ShmError),
    BrokerThread,
    Worker,
    Timeout,
    Closed,
}

impl fmt::Display for LocalClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument { field, reason } => {
                write!(formatter, "invalid argument `{field}`: {reason}")
            }
            Self::Transport(error) => write!(formatter, "transport error: {error}"),
            Self::Status(status) => write!(formatter, "rpc status: {status}"),
            Self::Shm(error) => write!(formatter, "shared memory error: {error}"),
            Self::BrokerThread => formatter.write_str("broker thread failed"),
            Self::Worker => formatter.write_str("local SDK worker task failed"),
            Self::Timeout => formatter.write_str("local SDK RPC timed out"),
            Self::Closed => formatter.write_str("local SDK client is closed"),
        }
    }
}

impl std::error::Error for LocalClientError {}

impl From<tonic::transport::Error> for LocalClientError {
    fn from(value: tonic::transport::Error) -> Self {
        Self::Transport(value)
    }
}

impl From<tonic::Status> for LocalClientError {
    fn from(value: tonic::Status) -> Self {
        Self::Status(value)
    }
}

impl From<afs_transport::shm::ShmError> for LocalClientError {
    fn from(value: afs_transport::shm::ShmError) -> Self {
        Self::Shm(value)
    }
}

impl LocalClient {
    pub async fn connect(config: LocalClientConfig) -> Result<Self, LocalClientError> {
        // UDS 连接仍复用统一 GrpcConfig：超时、HTTP/2 window、message size 等配置
        // 和其它 gRPC 通道一致；差异只在 connector 把“网络地址”替换成本机 socket。
        let path = Arc::new(config.socket_path);
        let grpc = GrpcConfig::default();
        let endpoint = grpc.configure_client(
            Endpoint::try_from("http://[::]:50051").expect("static endpoint URI"),
        );
        let channel = endpoint
            .connect_with_connector(service_fn(move |_| {
                let path = Arc::clone(&path);
                async move {
                    let stream = tokio::net::UnixStream::connect(path.as_ref()).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await?;
        Ok(Self {
            inner: LocalDataClient::new(traced_channel(channel))
                .max_encoding_message_size(grpc.max_encoding_message_bytes)
                .max_decoding_message_size(grpc.max_decoding_message_bytes),
            request_timeout: grpc.request_timeout,
        })
    }

    pub async fn write(
        &self,
        name: &str,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<usize, LocalClientError> {
        let length = u32::try_from(data.len()).map_err(|_| LocalClientError::InvalidArgument {
            field: "data",
            reason: "too large for local SDK request",
        })?;
        // 先拿 slot，再创建 memfd/broker。permit 会移动到 worker，防止调用方取消
        // `write()` future 时提前释放 slot，而 broker 线程还在等待 node 取 fd。
        let permit = BROKER_ADMISSION
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| LocalClientError::Closed)?;
        let worker = spawn_write_worker(
            permit,
            self.inner.clone(),
            self.request_timeout,
            name.to_owned(),
            offset,
            length,
            data,
        );
        worker.await.map_err(|_| LocalClientError::Worker)?
    }

    pub async fn read(
        &self,
        name: &str,
        offset: u64,
        length: u32,
    ) -> Result<Vec<u8>, LocalClientError> {
        let buffer_len =
            usize::try_from(length).map_err(|_| LocalClientError::InvalidArgument {
                field: "length",
                reason: "too large for local platform",
            })?;
        // read 同样先占 slot：target memfd 是 node 回填数据的唯一通道，
        // 不能在 RPC 返回前被调用方取消路径提前释放。
        let permit = BROKER_ADMISSION
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| LocalClientError::Closed)?;
        let worker = spawn_read_worker(
            permit,
            self.inner.clone(),
            self.request_timeout,
            name.to_owned(),
            offset,
            length,
            buffer_len,
        );
        worker.await.map_err(|_| LocalClientError::Worker)?
    }
}

// worker 是一次操作的资源所有者：permit、OperationBuffer、broker thread 都在这里闭环。
// 返回错误前仍会 join broker，避免“RPC 已失败但 fd broker 还活着”的资源泄漏。
fn spawn_write_worker(
    permit: tokio::sync::OwnedSemaphorePermit,
    mut client: LocalDataClient<TracedChannel>,
    timeout: Duration,
    name: String,
    offset: u64,
    length: u32,
    data: Vec<u8>,
) -> JoinHandle<Result<usize, LocalClientError>> {
    tokio::spawn(
        async move {
            let _permit = permit;
            let mut buffer = OperationBuffer::new(data.len())?;
            buffer.write_local(&data)?;
            let broker = buffer.serve_one();
            let request = request_with_current_context(LocalWriteRequest {
                name,
                file_offset: offset,
                length,
                source: Some(buffer.grant(length)),
            });
            // timeout 限制的是 gRPC 控制请求；broker join 仍必须执行，
            // 这样 node 未连接或连接失败时也能等到 broker 自己超时退出。
            let response = tokio::time::timeout(timeout, client.write(request)).await;
            let broker_result = join_broker(broker).await;
            let reply = response
                .map_err(|_| LocalClientError::Timeout)??
                .into_inner();
            broker_result?;
            if reply.written != length {
                return Err(LocalClientError::InvalidArgument {
                    field: "written",
                    reason: "reply length did not match request",
                });
            }
            Ok(reply.written as usize)
        }
        .in_current_span(),
    )
}

// 读取 worker 与写入对称：node 只拿到 target fd，不拿到 SDK 内存引用。
// node 写完 fd 后 SDK 再从本地 memfd copy 出 Vec。
fn spawn_read_worker(
    permit: tokio::sync::OwnedSemaphorePermit,
    mut client: LocalDataClient<TracedChannel>,
    timeout: Duration,
    name: String,
    offset: u64,
    length: u32,
    buffer_len: usize,
) -> JoinHandle<Result<Vec<u8>, LocalClientError>> {
    tokio::spawn(
        async move {
            let _permit = permit;
            let buffer = OperationBuffer::new(buffer_len)?;
            let broker = buffer.serve_one();
            let request = request_with_current_context(LocalReadRequest {
                name,
                file_offset: offset,
                length,
                target: Some(buffer.grant(length)),
            });
            // 先等待 broker 收尾，再解释 RPC 结果；对外优先保留 RPC 原始错误，
            // 但资源生命周期不能因为错误路径而跳过。
            let response = tokio::time::timeout(timeout, client.read(request)).await;
            let broker_result = join_broker(broker).await;
            let reply = response
                .map_err(|_| LocalClientError::Timeout)??
                .into_inner();
            broker_result?;
            if reply.length > length {
                return Err(LocalClientError::InvalidArgument {
                    field: "length",
                    reason: "reply length exceeded request",
                });
            }
            buffer.read_local(reply.length as usize).map_err(Into::into)
        }
        .in_current_span(),
    )
}

// std::thread::JoinHandle::join 是阻塞操作，必须放到 spawn_blocking，
// 否则 Tokio worker 可能被本地 broker 等待卡住。
async fn join_broker(
    broker: std::thread::JoinHandle<Result<(), afs_transport::shm::ShmError>>,
) -> Result<(), LocalClientError> {
    let broker_result = tokio::task::spawn_blocking(move || {
        broker.join().map_err(|_| LocalClientError::BrokerThread)
    })
    .await
    .map_err(|_| LocalClientError::BrokerThread)?
    .map_err(|_| LocalClientError::BrokerThread)?;
    broker_result.map_err(Into::into)
}

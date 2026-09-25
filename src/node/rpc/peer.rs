//! 调用其他 Node 的共同数据 API 与 adapter 边界。
//!
//! 这个文件是“共同客户端 adapter”：上层只调用 DataPeerClient::read/write/close，
//! 不关心底层走 gRPC inline 还是 RDMA one-sided。服务端侧对应 data.rs 的共同
//! handler；客户端侧对应这里的 GrpcInlineClient/RdmaDataClient adapter。
//!
//! 模式含义：
//! - Grpc：控制命令和文件内容都进 node_data proto；
//! - Rdma：control/data 命令仍走 gRPC proto，文件内容走 RDMA MR；
//! - Auto：先尝试 RDMA 建连，失败才在“尚未发出业务请求”前回退到 gRPC。
//!
//! 已经发出的写如果结果不明，绝不换通道重放；RDMA in-flight 被取消时会 poison
//! 当前 client/session，后续复用必须失败，避免重复写或顺序错乱。

#[cfg(feature = "rdma")]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[cfg(feature = "rdma")]
use afs_protocol::node_control::{
    CloseDataRequest, NegotiateDataRequest, node_control_client::NodeControlClient,
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

const MAX_TRANSFER_BYTES: usize = crate::node::storage::MAX_TRANSFER_BYTES;

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

#[derive(Debug)]
pub struct PeerError(String);

impl PeerError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PeerError {}

impl From<tonic::Status> for PeerError {
    fn from(value: tonic::Status) -> Self {
        Self(value.to_string())
    }
}

impl From<tonic::transport::Error> for PeerError {
    fn from(value: tonic::transport::Error) -> Self {
        Self(value.to_string())
    }
}

impl From<tonic::codegen::http::uri::InvalidUri> for PeerError {
    fn from(value: tonic::codegen::http::uri::InvalidUri) -> Self {
        Self(value.to_string())
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
            return Err(PeerError::new("gRPC read reply shape mismatch"));
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
            return Err(PeerError::new("gRPC write reply count mismatch"));
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
        let mut endpoint = RdmaEndpoint::open(&device).map_err(|error| PeerError(error.0))?;
        let info = endpoint.info().map_err(|error| PeerError(error.0))?;
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
            return Err(PeerError::new("peer does not support RDMA"));
        }
        if negotiate.handshake_version != RDMA_HANDSHAKE_VERSION {
            close_session_best_effort(&mut control, negotiate.session_id).await;
            return Err(PeerError::new(
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
        .map_err(|error| PeerError::new(error.to_string()))
        .and_then(|result| result.map_err(|error| PeerError::new(error.to_string())));
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
                    .map_err(|error| poison(&poisoned, error.to_string()))?
                    .into_inner();
                if !reply.data.is_empty() || reply.length != length {
                    return Err(poison(&poisoned, "RDMA read reply shape mismatch"));
                }
                let mut endpoint = endpoint.lock().await;
                endpoint
                    .get_local(length as usize)
                    .map_err(|error| poison(&poisoned, error.0))
            }
            .in_current_span(),
        )
        .await
        .map_err(|error| PeerError(error.to_string()))?;
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
                    endpoint
                        .put_local(&data)
                        .map_err(|error| poison(&poisoned, error.0))?;
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
                    .map_err(|error| poison(&poisoned, error.to_string()))?
                    .into_inner();
                if reply.written as usize != len {
                    return Err(poison(&poisoned, "RDMA write reply count mismatch"));
                }
                Ok(reply.written)
            }
            .in_current_span(),
        )
        .await
        .map_err(|error| PeerError(error.to_string()))?;
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
    let device = device.ok_or_else(|| PeerError::new("RDMA mode requires a device"))?;
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
    Err(PeerError::new("RDMA feature is not enabled"))
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
        return Err(PeerError::new("transfer exceeds 1MiB"));
    }
    Ok(())
}

#[cfg(feature = "rdma")]
fn validate_open(closed: bool, poisoned: &AtomicBool) -> PeerResult<()> {
    if closed {
        return Err(PeerError::new("RDMA session is closed"));
    }
    if poisoned.load(Ordering::SeqCst) {
        return Err(PeerError::new("RDMA session is poisoned"));
    }
    Ok(())
}

#[cfg(feature = "rdma")]
fn poison(poisoned: &AtomicBool, message: impl Into<String>) -> PeerError {
    poisoned.store(true, Ordering::SeqCst);
    PeerError(message.into())
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

#[cfg(test)]
mod tests {
    use super::*;
    use afs_protocol::node_data::{
        DataReadReply, DataWriteReply,
        node_data_server::{NodeData, NodeDataServer},
    };
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
}

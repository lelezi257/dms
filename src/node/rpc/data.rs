//! Node → Node 文件数据操作 Handler。
//!
//! 这个文件实现 node_data.proto 生成的服务端 handler。它是“共同服务端入口”：
//! 无论远端客户端选择 gRPC inline 还是 RDMA，最终都会进入这里，完成相同的
//! Storage read/write 业务语义。区别只在文件内容怎么搬：
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

use std::sync::Arc;
#[cfg(feature = "rdma")]
use std::sync::atomic::Ordering;

use afs_protocol::node_data::{
    DataReadReply, DataReadRequest, DataTransfer, DataWriteReply, DataWriteRequest,
    node_data_server::{NodeData, NodeDataServer},
};
use afs_tracing::Instrument;
use tonic::{Request, Response, Status};

use crate::node::{
    rpc::control::RdmaSessionRegistry,
    storage::{MAX_TRANSFER_BYTES, Storage, StorageError},
};

/// node_data 的服务端实现。
///
/// `storage` 是真实文件操作入口；`sessions` 只在 RDMA 模式下把 session_id 转为
/// 服务端 endpoint。这里不区分 OwnerFs/BlobFs，只实现第一版诊断用的 8-byte/文件数据路径。
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
                return Err(Status::invalid_argument("transfer is required"));
            }
            validate_read_session(&self.sessions, request.session_id, transfer).await?;
            let data = self
                .storage
                .read(&request.name, request.offset, request.length)
                .await
                .map_err(storage_status)?;
            debug_assert_eq!(data.len(), request.length as usize);
            let actual_len = u32::try_from(data.len())
                .map_err(|_| Status::internal("read length exceeds u32"))?;
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
                        return Err(Status::invalid_argument("inline length/data mismatch"));
                    }
                    validate_length(request.data.len() as u32)?;
                    request.data
                }
                DataTransfer::RdmaOneSided => {
                    validate_length(request.length)?;
                    if !request.data.is_empty() {
                        return Err(Status::invalid_argument(
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
                    return Err(Status::invalid_argument("transfer is required"));
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
        return Err(Status::invalid_argument("transfer exceeds 1MiB"));
    }
    Ok(())
}

fn transfer_mode(value: i32) -> Result<DataTransfer, Status> {
    DataTransfer::try_from(value).map_err(|_| Status::invalid_argument("unknown transfer mode"))
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
    match error {
        StorageError::BadName | StorageError::Range | StorageError::TooLarge => {
            Status::invalid_argument(error.to_string())
        }
        StorageError::UnsafeFileType => Status::failed_precondition(error.to_string()),
        StorageError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Status::not_found(error.to_string())
        }
        StorageError::Io(error) => Status::internal(error.to_string()),
        StorageError::Join(error) => Status::internal(error.to_string()),
    }
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
    .map_err(|error| Status::internal(error.to_string()))?;
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
    Err(Status::unavailable("RDMA feature is not enabled"))
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
    .map_err(|error| Status::internal(error.to_string()))?;
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
    Err(Status::unavailable("RDMA feature is not enabled"))
}

#[cfg(feature = "rdma")]
fn rdma_status(error: afs_transport::rdma::RdmaError) -> Status {
    Status::unavailable(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::rpc::{
        control::make_control_server,
        peer::{DataClientOptions, DataMode, connect_data_client},
    };
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

    async fn spawn_grpc_server() -> (tempfile::TempDir, String, tokio::task::JoinHandle<()>) {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::new(temp.path()).unwrap());
        let registry = RdmaSessionRegistry::new(None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn({
            let registry = registry.clone();
            async move {
                Server::builder()
                    .add_service(make_control_server(registry.clone()))
                    .add_service(make_data_server(storage, registry))
                    .serve_with_incoming(TcpListenerStream::new(listener))
                    .await
                    .unwrap();
            }
        });
        (temp, endpoint, server)
    }
}

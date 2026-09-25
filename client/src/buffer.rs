//! SDK 侧一次操作的共享内存缓冲。
//!
//! 每次 read/write 都创建一个独立 memfd 和一个一次性 fd broker。gRPC 只携带
//! `LocalShmGrant` 描述符；文件内容通过 broker 传出的 fd 读写。
//!
//! 这里的 sealed memfd 只保证 fd 大小不能被对端 shrink/grow，防止读写过程中
//! 因 resize 出现 SIGBUS/越界；它不表示内容不可变，node 仍会在 read 路径写入数据。

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use afs_protocol::local_api::LocalShmGrant;
use afs_transport::shm::{BrokerToken, FdBrokerServer, FdGrant, FdRequest, SharedRegion, ShmError};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

// OperationBuffer 是 SDK 与 node 之间一次数据交换的所有权边界。
// region 持有真实 bytes；broker 持有“谁能取 fd”的一次性授权；request 是授权 key。
pub(crate) struct OperationBuffer {
    region: SharedRegion,
    broker: FdBrokerServer,
    request: FdRequest,
}

impl OperationBuffer {
    pub(crate) fn new(len: usize) -> Result<Self, ShmError> {
        // len=0 时仍创建 1 byte memfd，因为 fd 传递需要一个实际对象；
        // grant.length 仍会记录调用方请求的真实长度。
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let region_len = len.max(1);
        let region = SharedRegion::create("afs-local-sdk", region_len)?;
        let token = BrokerToken::new(unique_token(id).into_bytes())?;
        let request = FdRequest::new(token, std::process::id().into(), id);
        let broker = FdBrokerServer::bind(socket_path(id))?;
        broker.register(FdGrant::with_ttl(
            request.clone(),
            region.duplicate_fd_owned()?,
            Duration::from_secs(5),
        ))?;
        Ok(Self {
            region,
            broker,
            request,
        })
    }

    pub(crate) fn write_local(&mut self, data: &[u8]) -> Result<(), ShmError> {
        self.region.write_at(0, data)
    }

    pub(crate) fn read_local(&self, len: usize) -> Result<Vec<u8>, ShmError> {
        self.region.read_at(0, len)
    }

    pub(crate) fn grant(&self, len: u32) -> LocalShmGrant {
        // grant 是控制面唯一看到的“数据地址”：broker socket + token/session/region。
        // token 只在本机 broker 内校验，不是跨主机认证体系；当前信任边界是同机进程。
        LocalShmGrant {
            broker_socket_path: self.broker.path().to_string_lossy().into_owned(),
            token: self.request.token.as_bytes().to_vec(),
            session_id: self.request.session_id,
            region_id: self.request.region_id,
            region_offset: 0,
            length: len,
        }
    }

    pub(crate) fn serve_one(&self) -> thread::JoinHandle<Result<(), ShmError>> {
        // broker 只服务一次 fd 领取；worker 必须 join 它，确保 socket 和 fd 生命周期闭合。
        let broker = self.broker.clone();
        thread::spawn(move || broker.serve_one())
    }
}

fn unique_token(id: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("afs-local-sdk-{}-{id}-{now}", std::process::id())
}

fn socket_path(id: u64) -> PathBuf {
    std::env::temp_dir().join(format!("afs-local-sdk-{}-{id}.sock", std::process::id()))
}

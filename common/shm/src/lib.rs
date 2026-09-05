#![allow(unsafe_code)]
//! Linux shared-memory primitives for DMS.
//!
//! This crate is intentionally small: it is the unsafe boundary for memfd,
//! mmap and SCM_RIGHTS. Higher-level crates exchange typed descriptors and call
//! wrappers instead of touching raw pointers or ancillary data. Borrowed mmap
//! access remains unsafe: callers must prove cross-process allocation lifetime.

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const PROTOCOL_VERSION: u32 = 2;
const MAX_TOKEN_BYTES: usize = 256;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum ShmError {
    #[error("shared memory is supported only on Linux")]
    Unsupported,
    #[error("invalid argument `{field}`: {reason}")]
    InvalidArgument {
        field: &'static str,
        reason: &'static str,
    },
    #[error("system call `{0}` failed: {1}")]
    Syscall(&'static str, std::io::Error),
    #[error("broker protocol error: {0}")]
    Protocol(&'static str),
    #[error("broker token was not accepted")]
    InvalidToken,
    #[error("broker registry lock was poisoned")]
    Poisoned,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BrokerToken(Vec<u8>);

impl BrokerToken {
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, ShmError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_TOKEN_BYTES {
            return Err(ShmError::InvalidArgument {
                field: "token",
                reason: "must be 1..=256 bytes",
            });
        }
        Ok(Self(value))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FdRequest {
    pub token: BrokerToken,
    pub session_id: u64,
    pub region_id: u64,
    pub protocol_version: u32,
}

impl FdRequest {
    pub fn new(token: BrokerToken, session_id: u64, region_id: u64) -> Self {
        Self {
            token,
            session_id,
            region_id,
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

pub struct FdGrant {
    request: FdRequest,
    fd: OwnedFd,
    expires_at: Instant,
}

impl FdGrant {
    pub fn new(request: FdRequest, fd: OwnedFd) -> Self {
        Self::with_ttl(request, fd, Duration::from_secs(30))
    }

    /// Creates a one-shot capability with a bounded lifetime. If a Client dies
    /// between `AcquireRegion` and `SCM_RIGHTS`, the duplicated fd is reclaimed
    /// when the next broker request purges expired grants.
    pub fn with_ttl(request: FdRequest, fd: OwnedFd, ttl: Duration) -> Self {
        Self {
            request,
            fd,
            expires_at: Instant::now() + ttl,
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    pub struct SharedRegion {
        fd: OwnedFd,
        mapped: MappedRegion,
    }

    impl SharedRegion {
        pub fn create(name: &str, len: usize) -> Result<Self, ShmError> {
            validate_len(len)?;
            let cname = std::ffi::CString::new(name).map_err(|_| ShmError::InvalidArgument {
                field: "name",
                reason: "must not contain NUL",
            })?;
            // SAFETY: memfd_create reads the NUL-terminated name and returns a
            // new fd on success. The fd is immediately wrapped in OwnedFd.
            let raw = unsafe {
                libc::memfd_create(cname.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
            };
            if raw < 0 {
                return Err(sys("memfd_create"));
            }
            // SAFETY: raw was returned by memfd_create and is uniquely owned.
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            // SAFETY: ftruncate only changes the size of this owned memfd.
            if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } != 0 {
                return Err(sys("ftruncate"));
            }
            // 导出的 dup fd 仍可写入 allocation bytes，但不能通过安全 File::set_len
            // 缩短 backing 导致现有 mmap SIGBUS，也不能绕过 Arena 预算扩容。
            // 不加 F_SEAL_WRITE：staging 写仍是正式协议能力；seal 集合自身不可再改变。
            // SAFETY: fd 为当前唯一 memfd；F_ADD_SEALS 不使用指针参数。
            if unsafe {
                libc::fcntl(
                    fd.as_raw_fd(),
                    libc::F_ADD_SEALS,
                    libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL,
                )
            } != 0
            {
                return Err(sys("F_ADD_SEALS"));
            }
            // SAFETY: 新建 memfd 已调整到 len；本 owner 控制所有后续访问。
            let mapped = unsafe { MappedRegion::map(dup_fd(fd.as_raw_fd())?, len)? };
            Ok(Self { fd, mapped })
        }

        /// 导出可写 memfd；尺寸已封印，但其它持有者仍可修改 bytes。
        ///
        /// # Safety
        /// 调用者必须协调此 fd 及其所有后续副本/映射与 Region owner 的访问：
        /// 同一范围写入时无其它读写，已发布读范围保持不变，失效导出范围不复用。
        /// FileExt::write_at 也属于写访问，不能因为它是 safe syscall 就跳过协调。
        ///
        /// ```compile_fail
        /// let region = dms_shm::SharedRegion::create("example", 4096).unwrap();
        /// let fd = region.duplicate_fd().unwrap();
        /// ```
        pub unsafe fn duplicate_fd(&self) -> Result<OwnedFd, ShmError> {
            dup_fd(self.fd.as_raw_fd())
        }

        pub fn len(&self) -> usize {
            self.mapped.len()
        }

        pub fn is_empty(&self) -> bool {
            self.mapped.is_empty()
        }

        pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> Result<(), ShmError> {
            self.mapped.write_at(offset, bytes)
        }

        pub fn read_at(&self, offset: usize, len: usize) -> Result<Vec<u8>, ShmError> {
            self.mapped.read_at(offset, len)
        }
    }

    pub struct MappedRegion {
        fd: OwnedFd,
        ptr: NonNull<u8>,
        len: usize,
    }

    // 多个异步操作可以共享 Region 地址；map 的不安全合同要求所有访问遵守
    // 跨进程读写协议。共享引用导出的借用需单独证明互斥，不能靠 Sync 代替证明。
    unsafe impl Send for MappedRegion {}
    unsafe impl Sync for MappedRegion {}

    impl MappedRegion {
        /// 映射协议授予的区域。长度检查不能证明跨进程生命周期。
        ///
        /// # Safety
        /// 调用者必须保证 fd backing 至少 len 字节且不缩短，所有映射间读写遵守独占写/
        /// 已发布不可变读协议；普通 Rust 借用不能约束另一个进程。
        pub unsafe fn map(fd: OwnedFd, len: usize) -> Result<Self, ShmError> {
            validate_len(len)?;
            // SAFETY: mmap is called with a valid fd and non-zero length. MAP_FAILED
            // is checked before constructing NonNull.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd.as_raw_fd(),
                    0,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(sys("mmap"));
            }
            let ptr =
                NonNull::new(ptr.cast::<u8>()).ok_or(ShmError::Protocol("mmap returned null"))?;
            Ok(Self { fd, ptr, len })
        }

        pub fn len(&self) -> usize {
            self.len
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> Result<(), ShmError> {
            let range = self.range(offset, bytes.len())?;
            // SAFETY: range was checked to stay within the owned mmap.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    self.ptr.as_ptr().add(range.start),
                    bytes.len(),
                );
            }
            Ok(())
        }

        pub fn read_at(&self, offset: usize, len: usize) -> Result<Vec<u8>, ShmError> {
            let range = self.range(offset, len)?;
            // SAFETY: range was checked to stay within the owned mmap, and the
            // returned Vec owns a copy independent of the mapping lifetime.
            let slice =
                unsafe { std::slice::from_raw_parts(self.ptr.as_ptr().add(range.start), len) };
            Ok(slice.to_vec())
        }

        /// # Safety
        /// 返回借用存活期间，该范围不得通过本映射、其它映射或其它进程写入/复用。
        pub unsafe fn as_slice(&self, offset: usize, len: usize) -> Result<&[u8], ShmError> {
            let range = self.range(offset, len)?;
            // SAFETY: range is inside the mmap and the borrow is tied to &self.
            Ok(unsafe { std::slice::from_raw_parts(self.ptr.as_ptr().add(range.start), len) })
        }

        pub fn as_mut_slice(&mut self, offset: usize, len: usize) -> Result<&mut [u8], ShmError> {
            let range = self.range(offset, len)?;
            // SAFETY: range is inside the mmap and the borrow is tied to &mut self.
            Ok(unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr().add(range.start), len) })
        }

        /// 返回协议独占的 staging 范围；此处不验证签名、Session 或租约。
        ///
        /// # Safety
        /// 调用者必须证明借用期间无重叠引用/读写，且 Node 不复用此范围。
        /// 失效 receipt 只能拒绝提交，不能撤销已存在的可写映射。
        ///
        /// ```compile_fail
        /// fn alias(mapping: &dms_shm::MappedRegion) {
        ///     let first = mapping.staging_slice_mut(0, 1).unwrap();
        ///     let second = mapping.staging_slice_mut(0, 1).unwrap();
        ///     first[0] = second[0];
        /// }
        /// ```
        #[allow(clippy::mut_from_ref)]
        pub unsafe fn staging_slice_mut(
            &self,
            offset: usize,
            len: usize,
        ) -> Result<&mut [u8], ShmError> {
            let range = self.range(offset, len)?;
            // SAFETY: the DMS staging protocol supplies the cross-process
            // exclusivity proof; range validation keeps the pointer in mmap.
            Ok(unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr().add(range.start), len) })
        }

        fn range(&self, offset: usize, len: usize) -> Result<std::ops::Range<usize>, ShmError> {
            let end = offset.checked_add(len).ok_or(ShmError::InvalidArgument {
                field: "range",
                reason: "offset + length overflow",
            })?;
            if end > self.len {
                return Err(ShmError::InvalidArgument {
                    field: "range",
                    reason: "out of bounds",
                });
            }
            Ok(offset..end)
        }
    }

    impl Drop for MappedRegion {
        fn drop(&mut self) {
            // SAFETY: ptr/len came from a successful mmap and are owned by this
            // value. fd closes automatically after munmap.
            unsafe {
                libc::munmap(self.ptr.as_ptr().cast(), self.len);
            }
            let _ = self.fd.as_raw_fd();
        }
    }

    #[derive(Clone)]
    pub struct FdBrokerServer {
        path: PathBuf,
        listener: Arc<UnixListener>,
        grants: Arc<Mutex<HashMap<BrokerToken, FdGrant>>>,
        served: Arc<AtomicU64>,
    }

    impl FdBrokerServer {
        pub fn bind(path: impl AsRef<Path>) -> Result<Self, ShmError> {
            let path = path.as_ref().to_path_buf();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| ShmError::Syscall("create_dir_all", error))?;
            }
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ShmError::Syscall("remove_file", error)),
            }
            let listener =
                UnixListener::bind(&path).map_err(|error| ShmError::Syscall("bind", error))?;
            Ok(Self {
                path,
                listener: Arc::new(listener),
                grants: Arc::new(Mutex::new(HashMap::new())),
                served: Arc::new(AtomicU64::new(0)),
            })
        }

        pub fn path(&self) -> &Path {
            &self.path
        }

        pub fn register(&self, grant: FdGrant) -> Result<(), ShmError> {
            let token = grant.request.token.clone();
            self.grants
                .lock()
                .map_err(|_| ShmError::Poisoned)?
                .insert(token, grant);
            Ok(())
        }

        /// Number of successful SCM_RIGHTS deliveries. This is useful for
        /// observability and proves that Region cache hits do not request FDs.
        pub fn served_count(&self) -> u64 {
            self.served.load(Ordering::Relaxed)
        }

        /// Drops capabilities whose Client never completed the SCM_RIGHTS
        /// handshake. The returned count is useful for Node maintenance logs.
        pub fn reap_expired(&self) -> Result<usize, ShmError> {
            let mut grants = self.grants.lock().map_err(|_| ShmError::Poisoned)?;
            let before = grants.len();
            let now = Instant::now();
            grants.retain(|_, grant| grant.expires_at > now);
            Ok(before.saturating_sub(grants.len()))
        }

        pub fn serve_one(&self) -> Result<(), ShmError> {
            let (stream, _) = self
                .listener
                .accept()
                .map_err(|error| ShmError::Syscall("accept", error))?;
            self.handle(stream)
        }

        fn handle(&self, stream: UnixStream) -> Result<(), ShmError> {
            let mut stream = DeadlineStream::new(stream, Instant::now() + HANDSHAKE_TIMEOUT)?;
            let request = read_request(&mut stream)?;
            let grant = {
                let mut grants = self.grants.lock().map_err(|_| ShmError::Poisoned)?;
                let now = Instant::now();
                grants.retain(|_, grant| grant.expires_at > now);
                let Some(grant) = grants.remove(&request.token) else {
                    stream
                        .write_all(&[0])
                        .map_err(|error| ShmError::Syscall("write", error))?;
                    return Err(ShmError::InvalidToken);
                };
                if grant.request != request {
                    return Err(ShmError::InvalidToken);
                }
                grant
            };
            stream
                .write_all(&[1])
                .map_err(|error| ShmError::Syscall("write", error))?;
            send_fd(&stream.stream, grant.fd.as_raw_fd(), stream.deadline)?;
            self.served.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    pub struct FdBrokerClient;

    impl FdBrokerClient {
        pub fn request_fd(
            path: impl AsRef<Path>,
            request: &FdRequest,
        ) -> Result<OwnedFd, ShmError> {
            // 留出一次排队等待服务端拒绝前一个坏连接的时间，仍有总截止时间。
            let deadline = Instant::now() + HANDSHAKE_TIMEOUT * 2;
            let stream = connect_before(path.as_ref(), deadline)?;
            let mut stream = DeadlineStream::new(stream, deadline)?;
            write_request(&mut stream, request)?;
            let mut status = [0_u8; 1];
            stream
                .read_exact(&mut status)
                .map_err(|error| ShmError::Syscall("read", error))?;
            if status[0] != 1 {
                return Err(ShmError::InvalidToken);
            }
            recv_fd(&stream.stream, stream.deadline)
        }
    }

    // 一个绝对截止时间覆盖完整握手；慢速逐字节发送不能反复重置计时。
    struct DeadlineStream {
        stream: UnixStream,
        deadline: Instant,
    }

    impl DeadlineStream {
        fn new(stream: UnixStream, deadline: Instant) -> Result<Self, ShmError> {
            stream
                .set_nonblocking(true)
                .map_err(|error| ShmError::Syscall("nonblocking", error))?;
            Ok(Self { stream, deadline })
        }
    }

    impl Read for DeadlineStream {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            loop {
                wait_ready(&self.stream, libc::POLLIN, self.deadline)?;
                match self.stream.read(bytes) {
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                    result => return result,
                }
            }
        }
    }

    impl Write for DeadlineStream {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            loop {
                wait_ready(&self.stream, libc::POLLOUT, self.deadline)?;
                match self.stream.write(bytes) {
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                    result => return result,
                }
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn wait_ready(
        stream: &UnixStream,
        events: libc::c_short,
        deadline: Instant,
    ) -> std::io::Result<()> {
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::TimedOut))?;
            let mut fd = libc::pollfd {
                fd: stream.as_raw_fd(),
                events,
                revents: 0,
            };
            let millis = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
            // SAFETY: poll 只访问一个存活 pollfd，不取得 fd 所有权。
            let ready = unsafe { libc::poll(&mut fd, 1, millis) };
            if ready > 0 {
                return Ok(());
            }
            if ready == 0 {
                continue;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    fn connect_before(path: &Path, deadline: Instant) -> Result<UnixStream, ShmError> {
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        // SAFETY: 全零是 sockaddr_un 的有效初始表示，后面填充 family/path。
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        if bytes.is_empty() || bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
            return Err(ShmError::Protocol("invalid Unix socket path"));
        }
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (target, byte) in address.sun_path.iter_mut().zip(bytes) {
            *target = *byte as libc::c_char;
        }
        loop {
            // SAFETY: socket 无指针输入；成功 fd 立即交给 UnixStream 唯一所有。
            let raw = unsafe {
                libc::socket(
                    libc::AF_UNIX,
                    libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                    0,
                )
            };
            if raw < 0 {
                return Err(sys("socket"));
            }
            let stream = unsafe { UnixStream::from_raw_fd(raw) };
            // SAFETY: address 指向已初始化 sockaddr_un，长度匹配。
            let result = unsafe {
                libc::connect(
                    raw,
                    (&address as *const libc::sockaddr_un).cast(),
                    std::mem::size_of_val(&address) as libc::socklen_t,
                )
            };
            if result == 0 {
                return Ok(stream);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::WouldBlock {
                return Err(ShmError::Syscall("connect", error));
            }
            // Linux AF_UNIX backlog 满时返回 EAGAIN；重试不阻塞无界线程。
            if Instant::now() >= deadline {
                return Err(ShmError::Syscall(
                    "connect",
                    std::io::ErrorKind::TimedOut.into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn dup_fd(fd: RawFd) -> Result<OwnedFd, ShmError> {
        // SAFETY: dup validates fd in the kernel and returns a new fd.
        let raw = unsafe { libc::dup(fd) };
        if raw < 0 {
            return Err(sys("dup"));
        }
        // SAFETY: raw is a fresh fd returned by dup.
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }

    fn send_fd(stream: &UnixStream, fd: RawFd, deadline: Instant) -> Result<(), ShmError> {
        wait_ready(stream, libc::POLLOUT, deadline)
            .map_err(|error| ShmError::Syscall("send deadline", error))?;
        let mut byte = [0_u8; 1];
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast(),
            iov_len: byte.len(),
        };
        let mut control = vec![0_u8; cmsg_space()];
        // SAFETY: msghdr points at live iovec/control buffers for this call.
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = control.len();
        // SAFETY: CMSG_FIRSTHDR returns a header inside msg_control.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&message);
            if cmsg.is_null() {
                return Err(ShmError::Protocol("missing cmsg header"));
            }
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as usize;
            let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
            *data = fd;
            message.msg_controllen = (*cmsg).cmsg_len;
            if libc::sendmsg(stream.as_raw_fd(), &message, 0) < 0 {
                return Err(sys("sendmsg"));
            }
        }
        Ok(())
    }

    fn recv_fd(stream: &UnixStream, deadline: Instant) -> Result<OwnedFd, ShmError> {
        wait_ready(stream, libc::POLLIN, deadline)
            .map_err(|error| ShmError::Syscall("receive deadline", error))?;
        let mut byte = [0_u8; 1];
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast(),
            iov_len: byte.len(),
        };
        let mut control = vec![0_u8; cmsg_space()];
        // SAFETY: msghdr points at live iovec/control buffers for this call.
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = control.len();
        // SAFETY: recvmsg initializes the provided buffers; cmsg metadata is
        // checked before reading the fd.
        unsafe {
            if libc::recvmsg(stream.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC) < 0 {
                return Err(sys("recvmsg"));
            }
            let cmsg = libc::CMSG_FIRSTHDR(&message);
            if cmsg.is_null()
                || (*cmsg).cmsg_level != libc::SOL_SOCKET
                || (*cmsg).cmsg_type != libc::SCM_RIGHTS
                || (*cmsg).cmsg_len < libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as usize
            {
                return Err(ShmError::Protocol("missing SCM_RIGHTS fd"));
            }
            let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
            let fd = *data;
            if fd < 0 {
                return Err(ShmError::Protocol("invalid fd"));
            }
            Ok(OwnedFd::from_raw_fd(fd))
        }
    }

    fn cmsg_space() -> usize {
        // SAFETY: CMSG_SPACE is a pure size calculation.
        unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as usize }
    }

    fn validate_len(len: usize) -> Result<(), ShmError> {
        if len == 0 {
            return Err(ShmError::InvalidArgument {
                field: "len",
                reason: "must be positive",
            });
        }
        Ok(())
    }

    fn sys(name: &'static str) -> ShmError {
        ShmError::Syscall(name, std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
mod linux {
    use super::*;

    pub struct SharedRegion;
    pub struct MappedRegion;
    #[derive(Clone)]
    pub struct FdBrokerServer;
    pub struct FdBrokerClient;

    impl SharedRegion {
        pub fn create(_name: &str, _len: usize) -> Result<Self, ShmError> {
            Err(ShmError::Unsupported)
        }
    }

    impl MappedRegion {
        pub fn map(_fd: OwnedFd, _len: usize) -> Result<Self, ShmError> {
            Err(ShmError::Unsupported)
        }
    }

    impl FdBrokerServer {
        pub fn bind(_path: impl AsRef<Path>) -> Result<Self, ShmError> {
            Err(ShmError::Unsupported)
        }
    }

    impl FdBrokerClient {
        pub fn request_fd(
            _path: impl AsRef<Path>,
            _request: &FdRequest,
        ) -> Result<OwnedFd, ShmError> {
            Err(ShmError::Unsupported)
        }
    }
}

pub use linux::{FdBrokerClient, FdBrokerServer, MappedRegion, SharedRegion};

fn write_request(stream: &mut impl Write, request: &FdRequest) -> Result<(), ShmError> {
    let token = request.token.as_bytes();
    let token_len = u32::try_from(token.len()).map_err(|_| ShmError::InvalidArgument {
        field: "token",
        reason: "too large",
    })?;
    stream
        .write_all(&request.protocol_version.to_be_bytes())
        .map_err(|error| ShmError::Syscall("write", error))?;
    stream
        .write_all(&request.session_id.to_be_bytes())
        .map_err(|error| ShmError::Syscall("write", error))?;
    stream
        .write_all(&request.region_id.to_be_bytes())
        .map_err(|error| ShmError::Syscall("write", error))?;
    stream
        .write_all(&token_len.to_be_bytes())
        .map_err(|error| ShmError::Syscall("write", error))?;
    stream
        .write_all(token)
        .map_err(|error| ShmError::Syscall("write", error))?;
    Ok(())
}

fn read_request(stream: &mut impl Read) -> Result<FdRequest, ShmError> {
    let protocol_version = read_u32(stream)?;
    if protocol_version != PROTOCOL_VERSION {
        return Err(ShmError::Protocol("unsupported protocol version"));
    }
    let session_id = read_u64(stream)?;
    let region_id = read_u64(stream)?;
    let token_len = read_u32(stream)? as usize;
    if token_len == 0 || token_len > MAX_TOKEN_BYTES {
        return Err(ShmError::InvalidArgument {
            field: "token",
            reason: "invalid length",
        });
    }
    let mut token = vec![0_u8; token_len];
    stream
        .read_exact(&mut token)
        .map_err(|error| ShmError::Syscall("read", error))?;
    Ok(FdRequest {
        token: BrokerToken(token),
        session_id,
        region_id,
        protocol_version,
    })
}

fn read_u32(stream: &mut impl Read) -> Result<u32, ShmError> {
    let mut bytes = [0_u8; 4];
    stream
        .read_exact(&mut bytes)
        .map_err(|error| ShmError::Syscall("read", error))?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_u64(stream: &mut impl Read) -> Result<u64, ShmError> {
    let mut bytes = [0_u8; 8];
    stream
        .read_exact(&mut bytes)
        .map_err(|error| ShmError::Syscall("read", error))?;
    Ok(u64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::*;

    fn socket_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dms-shm-{name}-{}-{}.sock",
            std::process::id(),
            unique()
        ))
    }

    fn unique() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    #[test]
    fn two_mappings_observe_the_same_physical_bytes() {
        let mut region = SharedRegion::create("dms-test-two-maps", 4096).expect("region");
        // SAFETY: 测试同步控制 fd 和两个映射的访问。
        let fd = unsafe { region.duplicate_fd().expect("dup") };
        // SAFETY: 测试同步控制这两个映射，写结束后才读。
        let view = unsafe { MappedRegion::map(fd, region.len()).expect("map") };

        region.write_at(128, b"hello shm").expect("write");

        assert_eq!(view.read_at(128, 9).expect("read"), b"hello shm");
    }

    #[test]
    fn bounds_checks_reject_invalid_access() {
        let mut region = SharedRegion::create("dms-test-bounds", 64).expect("region");

        assert!(matches!(
            region.write_at(60, b"too-long"),
            Err(ShmError::InvalidArgument { field: "range", .. })
        ));
        assert!(matches!(
            region.read_at(63, 2),
            Err(ShmError::InvalidArgument { field: "range", .. })
        ));
    }

    #[test]
    fn exported_fd_cannot_resize_the_live_mapping_but_can_write_bytes() {
        let mut region = SharedRegion::create("dms-sealed-size", 4096).unwrap();
        // SAFETY: 测试先检查封印，随后只进行有序、不并发的读写。
        let file = std::fs::File::from(unsafe { region.duplicate_fd().unwrap() });
        // 只检查 syscall 结果，不在未修复的实现上读取已缩短映射，避免 SIGBUS。
        assert!(
            file.set_len(0).is_err(),
            "exported fd must not shrink a live mapping"
        );
        assert!(
            file.set_len(8192).is_err(),
            "exported fd must not grow its Region budget"
        );
        region.write_at(0, b"safe").unwrap();
        assert_eq!(region.read_at(0, 4).unwrap(), b"safe");
        std::os::unix::fs::FileExt::write_all_at(&file, b"live", 0).unwrap();
        assert_eq!(region.read_at(0, 4).unwrap(), b"live");
    }

    #[test]
    fn broker_passes_fd_to_independent_client_mapping() {
        let path = socket_path("ok");
        let server = FdBrokerServer::bind(&path).expect("broker");
        let mut region = SharedRegion::create("dms-test-broker", 4096).expect("region");
        region.write_at(32, b"fd-by-token").expect("write");
        let token = BrokerToken::new(b"token-ok".to_vec()).expect("token");
        let request = FdRequest::new(token.clone(), 7, 11);
        server
            .register(FdGrant::new(
                request.clone(),
                // SAFETY: 发布后只读，测试不并发修改 backing。
                unsafe { region.duplicate_fd().expect("dup") },
            ))
            .expect("register");
        let serving = server.clone();
        let join = thread::spawn(move || serving.serve_one());

        wait_for_socket(&path);
        let fd = FdBrokerClient::request_fd(&path, &request).expect("fd");
        // SAFETY: 服务已写完且在测试结束前不修改/缩短 backing。
        let mapped = unsafe { MappedRegion::map(fd, 4096).expect("map") };

        assert_eq!(mapped.read_at(32, 11).expect("read"), b"fd-by-token");
        join.join().expect("join").expect("serve");
    }

    #[test]
    fn broker_rejects_wrong_and_repeated_token() {
        let path = socket_path("reject");
        let server = FdBrokerServer::bind(&path).expect("broker");
        let region = SharedRegion::create("dms-test-reject", 4096).expect("region");
        let good = FdRequest::new(BrokerToken::new(b"good".to_vec()).expect("token"), 1, 2);
        server
            .register(FdGrant::new(
                good.clone(),
                // SAFETY: 此测试只传 fd，不创建并发读写。
                unsafe { region.duplicate_fd().expect("dup") },
            ))
            .expect("register");
        let bad = FdRequest::new(BrokerToken::new(b"bad".to_vec()).expect("token"), 1, 2);
        let serving = server.clone();
        let first = thread::spawn(move || serving.serve_one());
        wait_for_socket(&path);
        assert!(matches!(
            FdBrokerClient::request_fd(&path, &bad),
            Err(ShmError::InvalidToken)
        ));
        assert!(matches!(
            first.join().expect("join"),
            Err(ShmError::InvalidToken)
        ));

        let serving = server.clone();
        let second = thread::spawn(move || serving.serve_one());
        wait_for_socket(&path);
        let fd = FdBrokerClient::request_fd(&path, &good).expect("fd");
        drop(fd);
        second.join().expect("join").expect("serve");

        let serving = server.clone();
        let third = thread::spawn(move || serving.serve_one());
        wait_for_socket(&path);
        assert!(matches!(
            FdBrokerClient::request_fd(&path, &good),
            Err(ShmError::InvalidToken)
        ));
        assert!(matches!(
            third.join().expect("join"),
            Err(ShmError::InvalidToken)
        ));
    }

    #[test]
    fn broker_reaps_an_unclaimed_expired_fd_grant() {
        let path = socket_path("expired");
        let server = FdBrokerServer::bind(&path).expect("broker");
        let region = SharedRegion::create("dms-test-expired", 4096).expect("region");
        let request = FdRequest::new(BrokerToken::new(b"expired".to_vec()).expect("token"), 1, 2);
        server
            .register(FdGrant::with_ttl(
                request,
                // SAFETY: grant 过期关闭，不访问共享 bytes。
                unsafe { region.duplicate_fd().expect("dup") },
                Duration::ZERO,
            ))
            .expect("register");
        assert_eq!(server.reap_expired().expect("reap"), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn incomplete_request_does_not_block_the_next_client() {
        let path = socket_path("deadline-server");
        let server = FdBrokerServer::bind(&path).unwrap();
        let region = SharedRegion::create("deadline", 4096).unwrap();
        let request = FdRequest::new(BrokerToken::new(b"deadline".to_vec()).unwrap(), 1, 1);
        server
            .register(FdGrant::new(
                request.clone(),
                // SAFETY: 此测试只传 fd，不访问共享 bytes。
                unsafe { region.duplicate_fd().unwrap() },
            ))
            .unwrap();
        let stalled = UnixStream::connect(&path).unwrap();
        let serving = server.clone();
        let worker = thread::spawn(move || {
            assert!(serving.serve_one().is_err());
            serving.serve_one().unwrap();
        });
        let started = Instant::now();
        // 正常请求直接排在不完整请求之后；无需由客户端等待坏请求自行关闭。
        let fd = FdBrokerClient::request_fd(&path, &request).unwrap();
        assert!(started.elapsed() < HANDSHAKE_TIMEOUT * 2);
        drop((fd, stalled));
        worker.join().unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn silent_broker_has_bounded_client_deadline() {
        let path = socket_path("deadline-client");
        let listener = UnixListener::bind(&path).unwrap();
        let worker = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(HANDSHAKE_TIMEOUT * 3);
        });
        let request = FdRequest::new(BrokerToken::new(b"silent".to_vec()).unwrap(), 1, 1);
        let started = Instant::now();
        assert!(FdBrokerClient::request_fd(&path, &request).is_err());
        assert!(started.elapsed() < HANDSHAKE_TIMEOUT * 2 + Duration::from_millis(700));
        worker.join().unwrap();
        let _ = fs::remove_file(path);
    }

    fn wait_for_socket(path: &Path) {
        for _ in 0..100 {
            if path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("socket did not appear: {}", path.display());
    }
}

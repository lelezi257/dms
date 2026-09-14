//! dms-node 内置的最小 Linux FUSE 内核接入层。
//!
//! 本模块只在 `fuse` feature 且 Linux 目标下编译。它的职责是把内核
//! FUSE 请求转换为进程内文件操作，再调用 `FileOperations -> DataCoreHandle`。
//! 这里不使用 dms-client，不调用 WorkerService，也不经过本地 gRPC；因此可以用来
//! 穿刺验证“文件系统入口与 KV 入口共享同一 Node core，但省掉 SDK/Worker RPC”。
//!
//! 当前命名空间是进程内临时穿刺：只支持根目录下一层普通文件，不把 inode/dentry
//! 持久化到 Meta。对象 bytes、版本、range overlay、Peer 拉取和 Arena 缓存仍由
//! DataCore/NodeState 管理。

#![cfg(all(target_os = "linux", feature = "fuse"))]

use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request, TimeOrNow,
};
use tokio::runtime::Handle;

use super::super::{data_core::DataCoreHandle, runtime::WorkerError};
use super::FileOperations;

const ROOT_INO: u64 = 1;
const FIRST_FILE_INO: u64 = 2;
const TTL: Duration = Duration::from_secs(0);
const BLOCK_SIZE: u32 = 4096;
const FOPEN_DIRECT_IO: u32 = 1 << 0;

/// 挂载一个后台 FUSE session。
///
/// `spawn_mount2` 先在当前调用中完成挂载初始化，成功后才返回持有后台服务线程和
/// mount 生命周期的 `BackgroundSession`。因此调用方只有拿到 `Ok` 后才能把 Node
/// 标记为 Ready；若 session 被释放，文件系统也会卸载。每个同步 FUSE callback
/// 再用传入的 `tokio::runtime::Handle` 回到现有 async DataCore。
pub(crate) fn start(
    mountpoint: PathBuf,
    core: DataCoreHandle,
    runtime: Handle,
) -> Result<BackgroundSession, Box<dyn std::error::Error>> {
    ensure_mountpoint(&mountpoint)?;
    let fs = DmsFuse::new(FileOperations::new(core), runtime);
    let options = vec![MountOption::FSName("dms-node".to_string())];
    fuser::spawn_mount2(fs, &mountpoint, &options).map_err(Into::into)
}

fn ensure_mountpoint(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(format!("FUSE mountpoint is not a directory: {}", path.display()).into());
    }
    Ok(())
}

#[derive(Clone)]
struct FileEntry {
    ino: u64,
    name: String,
    size: u64,
    mode: u16,
    uid: u32,
    gid: u32,
    created: SystemTime,
    modified: SystemTime,
}

impl FileEntry {
    fn attr(&self) -> FileAttr {
        FileAttr {
            ino: self.ino,
            size: self.size,
            blocks: self.size.div_ceil(u64::from(BLOCK_SIZE)),
            atime: self.modified,
            mtime: self.modified,
            ctime: self.modified,
            crtime: self.created,
            kind: FileType::RegularFile,
            perm: self.mode,
            nlink: 1,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: BLOCK_SIZE,
            flags: 0,
        }
    }
}

#[derive(Default)]
struct Namespace {
    by_ino: HashMap<u64, FileEntry>,
    by_name: HashMap<String, u64>,
}

struct DmsFuse {
    files: FileOperations,
    runtime: Handle,
    namespace: Arc<Mutex<Namespace>>,
    open_files: HashMap<u64, OpenFile>,
    /// 新文件首次提交前的共享内容，按 inode 管理而不是按 file handle 管理。
    ///
    /// 同一 inode 可以同时被多个 handle 打开。若 buffer 只挂在创建它的 fh 上，
    /// 第二个 handle 会误以为对象已经存在并绕过待提交内容。放到 inode 级后，所有
    /// handle 在首次 flush 前看到并修改的是同一份 bytes。
    pending_creates: HashMap<u64, PendingCreate>,
    next_ino: AtomicU64,
    next_fh: AtomicU64,
}

/// 一次 FUSE `open/create` 的进程内状态。
///
/// 新文件的待提交 buffer 存在 `DmsFuse::pending_creates`，这里仅记录 handle 与
/// inode/name 的绑定。这样多个 handle 不会各自形成一份新文件状态。
struct OpenFile {
    ino: u64,
    name: String,
}

enum PendingCreate {
    Buffered(Vec<u8>),
    Failed(i32),
}

impl DmsFuse {
    fn new(files: FileOperations, runtime: Handle) -> Self {
        Self {
            files,
            runtime,
            namespace: Arc::new(Mutex::new(Namespace::default())),
            open_files: HashMap::new(),
            pending_creates: HashMap::new(),
            next_ino: AtomicU64::new(FIRST_FILE_INO),
            next_fh: AtomicU64::new(1),
        }
    }

    fn root_attr() -> FileAttr {
        let now = SystemTime::now();
        FileAttr {
            ino: ROOT_INO,
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: BLOCK_SIZE,
            flags: 0,
        }
    }

    fn next_handle(&self) -> u64 {
        self.next_fh.fetch_add(1, Ordering::Relaxed)
    }

    fn lookup_entry(&self, parent: u64, name: &OsStr) -> Result<FileEntry, i32> {
        if parent != ROOT_INO {
            return Err(libc::ENOENT);
        }
        let name = parse_name(name)?;
        let namespace = self.namespace.lock().map_err(|_| libc::EIO)?;
        let ino = namespace.by_name.get(&name).ok_or(libc::ENOENT)?;
        namespace.by_ino.get(ino).cloned().ok_or(libc::ENOENT)
    }

    fn entry_by_inode(&self, ino: u64) -> Result<FileEntry, i32> {
        if ino == ROOT_INO {
            return Err(libc::EISDIR);
        }
        self.namespace
            .lock()
            .map_err(|_| libc::EIO)?
            .by_ino
            .get(&ino)
            .cloned()
            .ok_or(libc::ENOENT)
    }

    fn create_entry(
        &self,
        parent: u64,
        name: &OsStr,
        mode: u32,
        req: &Request,
    ) -> Result<FileEntry, i32> {
        if parent != ROOT_INO {
            return Err(libc::ENOENT);
        }
        let name = parse_name(name)?;
        {
            let namespace = self.namespace.lock().map_err(|_| libc::EIO)?;
            if namespace.by_name.contains_key(&name) {
                return Err(libc::EEXIST);
            }
        }

        let now = SystemTime::now();
        let entry = FileEntry {
            ino: self.next_ino.fetch_add(1, Ordering::Relaxed),
            name: name.clone(),
            size: 0,
            mode: (mode & 0o7777) as u16,
            uid: req.uid(),
            gid: req.gid(),
            created: now,
            modified: now,
        };
        let mut namespace = self.namespace.lock().map_err(|_| libc::EIO)?;
        namespace.by_name.insert(name, entry.ino);
        namespace.by_ino.insert(entry.ino, entry.clone());
        Ok(entry)
    }

    /// 本地命名空间未命中时，通过 DataCore 查询对象是否存在并建立本地 inode。
    ///
    /// 这不是完整的分布式目录服务：`readdir` 仍只列出本进程已发现的名字；但它
    /// 允许另一 Node 在已知 path 的情况下延迟发现对象，然后沿既有
    /// Meta resolve -> Peer pull 路径读取 bytes，足够验证跨节点数据主路径。
    fn discover_entry(&self, parent: u64, name: &OsStr, req: &Request) -> Result<FileEntry, i32> {
        match self.lookup_entry(parent, name) {
            Ok(entry) => return Ok(entry),
            Err(libc::ENOENT) => {}
            Err(errno) => return Err(errno),
        }
        let name = parse_name(name)?;
        let stat = self
            .runtime
            .block_on(self.files.stat(&name))
            .map_err(worker_to_errno)?
            .ok_or(libc::ENOENT)?;
        let now = SystemTime::now();
        let mut namespace = self.namespace.lock().map_err(|_| libc::EIO)?;
        if let Some(ino) = namespace.by_name.get(&name) {
            return namespace.by_ino.get(ino).cloned().ok_or(libc::ENOENT);
        }
        let entry = FileEntry {
            ino: self.next_ino.fetch_add(1, Ordering::Relaxed),
            name: name.clone(),
            size: stat.length,
            mode: 0o644,
            uid: req.uid(),
            gid: req.gid(),
            created: now,
            modified: now,
        };
        namespace.by_name.insert(name, entry.ino);
        namespace.by_ino.insert(entry.ino, entry.clone());
        Ok(entry)
    }

    fn register_open(&mut self, entry: &FileEntry) -> u64 {
        let fh = self.next_handle();
        self.open_files.insert(
            fh,
            OpenFile {
                ino: entry.ino,
                name: entry.name.clone(),
            },
        );
        fh
    }

    /// 把一个新建 inode 中聚合的 bytes 提交给 DataCore。
    ///
    /// `flush` 可能从同一 inode 的多个 handle 被调用，因此先把 inode 级 buffer 的
    /// Vec 所有权
    /// 直接移交 DataCore；成功后的后续 flush 是幂等 no-op。这样常见成功路径不会
    /// 在 FUSE buffer 与 DataCore 之间再复制一次完整文件。失败会直接反馈给调用者，
    /// 并把整个 inode 标记为失败，不让其它 handle 绕过同一次失败。
    fn flush_open(&mut self, fh: u64) -> Result<(), i32> {
        let Some(open) = self.open_files.get(&fh) else {
            return Ok(());
        };
        let ino = open.ino;
        let name = open.name.clone();
        let Some(pending) = self.pending_creates.remove(&ino) else {
            return Ok(());
        };
        let bytes = match pending {
            PendingCreate::Buffered(bytes) => bytes,
            PendingCreate::Failed(errno) => {
                self.pending_creates
                    .insert(ino, PendingCreate::Failed(errno));
                return Err(errno);
            }
        };
        let result = self
            .runtime
            .block_on(self.files.create_with_contents(&name, bytes))
            .map_err(|error| {
                if error.is_version_conflict() {
                    libc::EEXIST
                } else {
                    worker_to_errno(error)
                }
            });
        let write = match result {
            Ok(write) => write,
            Err(errno) => {
                self.pending_creates
                    .insert(ino, PendingCreate::Failed(errno));
                return Err(errno);
            }
        };
        // 数据已经提交成功；目录缓存更新失败不能把已经发生的提交伪装成失败。
        if let Err(errno) = self.update_entry_size(ino, write.length) {
            dms_logging::warn!(
                "FUSE namespace size cache update failed after create commit";
                "event" => "node.fuse.namespace_size_update_failed",
                "inode" => ino,
                "errno" => errno,
            );
        }
        Ok(())
    }

    fn update_entry_size(&self, ino: u64, size: u64) -> Result<(), i32> {
        let mut namespace = self.namespace.lock().map_err(|_| libc::EIO)?;
        let entry = namespace.by_ino.get_mut(&ino).ok_or(libc::ENOENT)?;
        entry.size = size;
        entry.modified = SystemTime::now();
        Ok(())
    }
}

impl Filesystem for DmsFuse {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        match self.discover_entry(parent, name, _req) {
            Ok(entry) => reply.entry(&TTL, &entry.attr(), 0),
            Err(errno) => reply.error(errno),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        if ino == ROOT_INO {
            reply.attr(&TTL, &Self::root_attr());
            return;
        }
        match self.entry_by_inode(ino) {
            // 已发现 inode 的长度由本进程 write/truncate/read 更新。不要为每个
            // getattr 再访问 Meta；远端首次发现已经在 lookup 中做过一次 stat。
            Ok(entry) => reply.attr(&TTL, &entry.attr()),
            Err(errno) => reply.error(errno),
        }
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let mut entry = match self.entry_by_inode(ino) {
            Ok(entry) => entry,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        if let Some(length) = size {
            let mut resized_pending_create = false;
            match self.pending_creates.get_mut(&ino) {
                Some(PendingCreate::Buffered(buffer)) => {
                    let Ok(length) = usize::try_from(length) else {
                        reply.error(libc::EFBIG);
                        return;
                    };
                    buffer.resize(length, 0);
                    entry.size = length as u64;
                    resized_pending_create = true;
                }
                Some(PendingCreate::Failed(errno)) => {
                    reply.error(*errno);
                    return;
                }
                None => {}
            }
            if !resized_pending_create {
                match self
                    .runtime
                    .block_on(self.files.truncate(&entry.name, length))
                {
                    Ok(result) => entry.size = result.length,
                    Err(error) => {
                        reply.error(worker_to_errno(error));
                        return;
                    }
                }
            }
        }
        if let Some(mode) = mode {
            entry.mode = (mode & 0o7777) as u16;
        }
        if let Some(uid) = uid {
            entry.uid = uid;
        }
        if let Some(gid) = gid {
            entry.gid = gid;
        }
        entry.modified = SystemTime::now();
        match self.namespace.lock() {
            Ok(mut namespace) => {
                namespace.by_ino.insert(ino, entry.clone());
                reply.attr(&TTL, &entry.attr());
            }
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if ino != ROOT_INO {
            reply.error(libc::ENOTDIR);
            return;
        }
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let mut entries = vec![
            (ROOT_INO, FileType::Directory, ".".to_string()),
            (ROOT_INO, FileType::Directory, "..".to_string()),
        ];
        match self.namespace.lock() {
            Ok(namespace) => {
                let mut files: Vec<_> = namespace.by_ino.values().cloned().collect();
                files.sort_by_key(|entry| entry.ino);
                entries.extend(
                    files
                        .into_iter()
                        .map(|entry| (entry.ino, FileType::RegularFile, entry.name)),
                );
            }
            Err(_) => {
                reply.error(libc::EIO);
                return;
            }
        }
        for (index, (entry_ino, kind, name)) in
            entries.into_iter().enumerate().skip(offset as usize)
        {
            if reply.add(entry_ino, (index + 1) as i64, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        match self.entry_by_inode(ino) {
            Ok(entry) => {
                if flags & libc::O_TRUNC != 0 {
                    match self.pending_creates.get_mut(&ino) {
                        Some(PendingCreate::Buffered(buffer)) => {
                            buffer.clear();
                            if let Err(error) = self.update_entry_size(ino, 0) {
                                reply.error(error);
                                return;
                            }
                        }
                        Some(PendingCreate::Failed(errno)) => {
                            reply.error(*errno);
                            return;
                        }
                        None => match self.runtime.block_on(self.files.truncate(&entry.name, 0)) {
                            Ok(_) => {
                                if let Err(error) = self.update_entry_size(ino, 0) {
                                    reply.error(error);
                                    return;
                                }
                            }
                            Err(error) => {
                                reply.error(worker_to_errno(error));
                                return;
                            }
                        },
                    }
                }
                // 关闭内核页缓存参与，便于穿刺时每次 read/write 都进入 DMS DataCore。
                let fh = self.register_open(&entry);
                reply.opened(fh, FOPEN_DIRECT_IO);
            }
            Err(errno) => reply.error(errno),
        }
    }

    fn create(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        match self.create_entry(parent, name, mode, req) {
            Ok(entry) => {
                self.pending_creates
                    .insert(entry.ino, PendingCreate::Buffered(Vec::new()));
                let fh = self.register_open(&entry);
                reply.created(&TTL, &entry.attr(), 0, fh, open_flags(flags));
            }
            Err(errno) => reply.error(errno),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let entry = match self.entry_by_inode(ino) {
            Ok(entry) => entry,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        match self.pending_creates.get(&ino) {
            Some(PendingCreate::Buffered(bytes)) => {
                let start = offset as usize;
                if start >= bytes.len() {
                    reply.data(&[]);
                    return;
                }
                let end = start.saturating_add(size as usize).min(bytes.len());
                reply.data(&bytes[start..end]);
                return;
            }
            Some(PendingCreate::Failed(errno)) => {
                reply.error(*errno);
                return;
            }
            None => {}
        }
        match self
            .runtime
            .block_on(self.files.read(&entry.name, offset as u64, u64::from(size)))
        {
            Ok(Some(read)) => reply.data(&read.bytes),
            Ok(None) => reply.error(libc::ENOENT),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let entry = match self.entry_by_inode(ino) {
            Ok(entry) => entry,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        match self.pending_creates.get_mut(&ino) {
            Some(PendingCreate::Buffered(buffer)) => {
                let start = offset as usize;
                let Some(end) = start.checked_add(data.len()) else {
                    reply.error(libc::EFBIG);
                    return;
                };
                if start == buffer.len() {
                    // 新文件最常见的是从 0 开始顺序写。直接 append 可避免先用 0
                    // 初始化同一段内存、随后又被用户 bytes 覆盖的第二次内存写。
                    buffer.extend_from_slice(data);
                } else {
                    if buffer.len() < end {
                        buffer.resize(end, 0);
                    }
                    buffer[start..end].copy_from_slice(data);
                }
                let length = buffer.len() as u64;
                if let Err(errno) = self.update_entry_size(ino, length) {
                    reply.error(errno);
                    return;
                }
                reply.written(data.len() as u32);
                return;
            }
            Some(PendingCreate::Failed(errno)) => {
                reply.error(*errno);
                return;
            }
            None => {}
        }
        match self
            .runtime
            .block_on(self.files.write(&entry.name, offset as u64, data))
        {
            Ok(write) => {
                // 数据已经提交成功，不能因为进程内目录缓存更新失败而伪装成
                // “本次写未发生”；这里记录本地缓存的独立异常。
                if let Err(errno) = self.update_entry_size(entry.ino, write.length) {
                    dms_logging::warn!(
                        "FUSE namespace size cache update failed after data commit";
                        "event" => "node.fuse.namespace_size_update_failed",
                        "inode" => entry.ino,
                        "errno" => errno,
                    );
                }
                reply.written(data.len() as u32);
            }
            Err(error) => {
                let message = format!("{error:?}");
                let errno = worker_to_errno(error);
                dms_logging::warn!(
                    "FUSE data write failed";
                    "event" => "node.fuse.write_failed",
                    "inode" => ino,
                    "offset" => offset,
                    "length" => data.len(),
                    "errno" => errno,
                    "error" => message,
                );
                reply.error(errno);
            }
        }
    }

    fn flush(&mut self, _req: &Request, _ino: u64, fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        match self.flush_open(fh) {
            Ok(()) => reply.ok(),
            Err(errno) => reply.error(errno),
        }
    }

    fn fsync(&mut self, _req: &Request, _ino: u64, fh: u64, _datasync: bool, reply: ReplyEmpty) {
        match self.flush_open(fh) {
            Ok(()) => reply.ok(),
            Err(errno) => reply.error(errno),
        }
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let result = self.flush_open(fh);
        self.open_files.remove(&fh);
        match result {
            Ok(()) => reply.ok(),
            Err(errno) => reply.error(errno),
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if parent != ROOT_INO {
            reply.error(libc::ENOENT);
            return;
        }
        let name = match parse_name(name) {
            Ok(name) => name,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };

        // 新文件在首次 flush 前只存在于本地 namespace 和 pending buffer，DataCore
        // 里还没有对应对象。这种 unlink 必须在本地闭合；若先调用 DataCore delete，
        // 会得到 NotFound，却已经删除目录项，并把 pending bytes 永久遗留在内存里。
        let pending_ino = match self.namespace.lock() {
            Ok(namespace) => namespace.by_name.get(&name).copied(),
            Err(_) => {
                reply.error(libc::EIO);
                return;
            }
        };
        if let Some(ino) = pending_ino.filter(|ino| self.pending_creates.contains_key(ino)) {
            let mut namespace = match self.namespace.lock() {
                Ok(namespace) => namespace,
                Err(_) => {
                    reply.error(libc::EIO);
                    return;
                }
            };
            namespace.by_name.remove(&name);
            namespace.by_ino.remove(&ino);
            drop(namespace);
            self.pending_creates.remove(&ino);
            self.open_files.retain(|_, open| open.ino != ino);
            reply.ok();
            return;
        }

        match self.runtime.block_on(self.files.delete(&name)) {
            Ok(result) => {
                let mut namespace = match self.namespace.lock() {
                    Ok(namespace) => namespace,
                    Err(_) => {
                        reply.error(libc::EIO);
                        return;
                    }
                };
                if let Some(ino) = namespace.by_name.remove(&name) {
                    namespace.by_ino.remove(&ino);
                    self.open_files.retain(|_, open| open.ino != ino);
                }
                if result.deleted {
                    reply.ok();
                } else {
                    // 本地目录项存在、DataCore 对象却已不存在，说明目录缓存已陈旧。
                    // 清理缓存并把真实的 ENOENT 返回给内核，不能静默报告成功。
                    reply.error(libc::ENOENT);
                }
            }
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }
}

fn parse_name(name: &OsStr) -> Result<String, i32> {
    let name = name.to_str().ok_or(libc::EINVAL)?;
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(libc::EINVAL);
    }
    Ok(name.to_string())
}

fn open_flags(_flags: i32) -> u32 {
    FOPEN_DIRECT_IO
}

fn worker_to_errno(error: WorkerError) -> i32 {
    match error {
        WorkerError::InvalidArgument(_) | WorkerError::ArenaInvalidRequest => libc::EINVAL,
        WorkerError::NotFound | WorkerError::UnknownSession | WorkerError::UnknownStaging => {
            libc::ENOENT
        }
        WorkerError::Conflict | WorkerError::ArenaStaleHandle => libc::ESTALE,
        WorkerError::NoLiveReplica | WorkerError::MetadataUnavailable => libc::EHOSTUNREACH,
        WorkerError::ResourceExhausted => libc::ENOSPC,
        WorkerError::WorkerUnavailable | WorkerError::TransferUnavailable => libc::EIO,
        WorkerError::ArenaShmUnavailable | WorkerError::ArenaAccessDenied => libc::EACCES,
        WorkerError::UnknownTransfer | WorkerError::Stable(_) => libc::EIO,
    }
}

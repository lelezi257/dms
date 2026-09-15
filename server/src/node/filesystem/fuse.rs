//! Linux FUSE 到共享文件主链的薄适配层。
//!
//! 本模块只负责把同步 FUSE callback 转换成 `SharedFileOperations` 调用，并把
//! `WorkerError` 映射成 errno。inode、open handle、内容 binding 与 DataCore 状态都
//! 由 Node 的唯一 owner 管理；这里不再维护第二套临时文件内容或提交状态。

#![cfg(all(target_os = "linux", feature = "fuse"))]

use std::{
    collections::HashMap,
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dms_error::{DmsError, ErrorKind};
use dms_tracing::Instrument as _;
use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyOpen, ReplyWrite, Request,
};
use tokio::runtime::Handle;

use super::SharedFileOperations;
use crate::filesystem::{InodeAttributes, InodeKind};
use crate::node::metrics::{FuseCallback, NodeMetrics};
use crate::node::runtime::{NodeHandle, WorkerError};

const TTL: Duration = Duration::from_secs(0);
const BLOCK_SIZE: u32 = 4096;
const FOPEN_DIRECT_IO: u32 = 1 << 0;

/// 挂载后台 FUSE session。
///
/// `SharedFileOperations::new` 复用 Node 已经建立的 Meta channel；挂载成功后，内核
/// callback 直接进入同进程 Node/DataCore，不经过 dms-client 或 WorkerService。
pub(crate) fn start(
    mountpoint: PathBuf,
    node: NodeHandle,
    runtime: Handle,
) -> Result<BackgroundSession, Box<dyn std::error::Error>> {
    ensure_mountpoint(&mountpoint)?;
    let metrics = node.metrics();
    let files = SharedFileOperations::new(node)
        .map_err(|error| format!("failed to initialize shared file operations: {error:?}"))?;
    let fs = DmsFuse::new(files, runtime, metrics);
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

struct DmsFuse {
    files: SharedFileOperations,
    runtime: Handle,
    metrics: NodeMetrics,
    next_directory_handle: u64,
    directory_handles: HashMap<u64, DirectoryHandle>,
}

/// 一次 `opendir` 到 `releasedir` 的枚举位置。
///
/// 这里只保存 FUSE cookie 到 Meta name-cursor 的小型映射，不保存完整目录内容。
/// 目录页本身由 Node 的授权缓存管理；revision 保证一次枚举不会跨 namespace 版本。
struct DirectoryHandle {
    inode: u64,
    parent: Option<u64>,
    revision: Option<u64>,
    positions: HashMap<i64, Option<Vec<u8>>>,
    cookies: HashMap<Vec<u8>, i64>,
    next_cookie: i64,
}

impl DmsFuse {
    fn new(files: SharedFileOperations, runtime: Handle, metrics: NodeMetrics) -> Self {
        Self {
            files,
            runtime,
            metrics,
            next_directory_handle: 1,
            directory_handles: HashMap::new(),
        }
    }

    fn allocate_directory_handle(&mut self, inode: u64) -> u64 {
        let handle = self.next_directory_handle;
        self.next_directory_handle = self.next_directory_handle.saturating_add(1).max(1);
        self.directory_handles.insert(
            handle,
            DirectoryHandle {
                inode,
                parent: None,
                revision: None,
                positions: HashMap::from([(0, None), (1, None), (2, None)]),
                cookies: HashMap::new(),
                next_cookie: 3,
            },
        );
        handle
    }
}

impl Filesystem for DmsFuse {
    fn opendir(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        let handle = self.allocate_directory_handle(ino);
        reply.opened(handle, 0);
    }

    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        self.metrics.record_fuse_callback(FuseCallback::Lookup);
        let name = name.as_bytes();
        let span = filesystem_span("dms.filesystem.lookup", Some(parent));
        match self
            .runtime
            .block_on(self.files.lookup(parent, name).instrument(span))
        {
            Ok(Some(resolved)) => {
                reply.entry(&TTL, &file_attr(&resolved.granted.inode.attributes), 0);
            }
            Ok(None) => reply.error(libc::ENOENT),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        self.metrics.record_fuse_callback(FuseCallback::Getattr);
        let span = filesystem_span("dms.filesystem.getattr", Some(ino));
        match self
            .runtime
            .block_on(self.files.get_inode(ino).instrument(span))
        {
            Ok(resolved) => reply.attr(&TTL, &file_attr(&resolved.granted.inode.attributes)),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        self.metrics.record_fuse_callback(FuseCallback::Readdir);
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let Some(handle) = self.directory_handles.get(&fh) else {
            reply.error(libc::EBADF);
            return;
        };
        if handle.inode != ino {
            reply.error(libc::EINVAL);
            return;
        }
        let Some(cursor) = handle.positions.get(&offset).cloned() else {
            reply.error(libc::EINVAL);
            return;
        };
        let expected_revision = handle.revision;
        let span = filesystem_span("dms.filesystem.readdir", Some(ino));
        let page = match self.runtime.block_on(
            self.files
                .read_directory_page(ino, cursor, expected_revision)
                .instrument(span),
        ) {
            Ok(result) => result,
            Err(error) => {
                reply.error(worker_to_errno(error));
                return;
            }
        };
        let handle = self
            .directory_handles
            .get_mut(&fh)
            .expect("directory handle was checked above");
        handle.parent = Some(page.parent);
        handle.revision = Some(page.grant.directory_revision);

        if offset == 0 && reply.add(ino, 1, FileType::Directory, OsStr::from_bytes(b".")) {
            reply.ok();
            return;
        }
        if offset <= 1
            && reply.add(
                page.parent,
                2,
                FileType::Directory,
                OsStr::from_bytes(b".."),
            )
        {
            reply.ok();
            return;
        }
        for entry in page.entries {
            let name = entry.dentry.name;
            let cookie = handle
                .cookies
                .get(&name)
                .copied()
                .unwrap_or(handle.next_cookie);
            if reply.add(
                entry.attributes.inode,
                cookie,
                file_type(entry.attributes.kind),
                OsStr::from_bytes(&name),
            ) {
                break;
            }
            if !handle.cookies.contains_key(&name) {
                handle.cookies.insert(name.clone(), cookie);
                handle.positions.insert(cookie, Some(name));
                handle.next_cookie = handle.next_cookie.saturating_add(1);
            }
        }
        reply.ok();
    }

    fn releasedir(&mut self, _req: &Request, _ino: u64, fh: u64, _flags: i32, reply: ReplyEmpty) {
        self.directory_handles.remove(&fh);
        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        self.metrics.record_fuse_callback(FuseCallback::Open);
        if flags & libc::O_TRUNC != 0 {
            // truncate 将在完整 POSIX 阶段使用相同原子发布机制接入；本纵向切片不能
            // 静默忽略 O_TRUNC，否则会把旧内容当成新空文件返回成功。
            reply.error(libc::ENOSYS);
            return;
        }
        let span = filesystem_span("dms.filesystem.open", Some(ino));
        match self
            .runtime
            .block_on(self.files.open(ino, flags).instrument(span))
        {
            Ok(opened) => reply.opened(opened.id, FOPEN_DIRECT_IO),
            Err(error) => reply.error(worker_to_errno(error)),
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
        self.metrics.record_fuse_callback(FuseCallback::Create);
        let name = name.as_bytes();
        let span = filesystem_span("dms.filesystem.create", Some(parent));
        let result = self.runtime.block_on(
            async {
                let created = self
                    .files
                    .create(parent, name, mode, req.uid(), req.gid())
                    .await?;
                let opened = self
                    .files
                    .open(created.granted.inode.attributes.inode, flags)
                    .await?;
                Ok::<_, WorkerError>((created, opened))
            }
            .instrument(span),
        );
        match result {
            Ok((created, opened)) => {
                reply.created(
                    &TTL,
                    &file_attr(&created.granted.inode.attributes),
                    0,
                    opened.id,
                    FOPEN_DIRECT_IO,
                );
            }
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn mkdir(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        self.metrics.record_fuse_callback(FuseCallback::Mkdir);
        let name = name.as_bytes();
        let span = filesystem_span("dms.filesystem.mkdir", Some(parent));
        match self.runtime.block_on(
            self.files
                .mkdir(parent, name, mode, req.uid(), req.gid())
                .instrument(span),
        ) {
            Ok(created) => reply.entry(&TTL, &file_attr(&created.granted.inode.attributes), 0),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        self.metrics.record_fuse_callback(FuseCallback::Unlink);
        let span = filesystem_span("dms.filesystem.unlink", Some(parent));
        match self
            .runtime
            .block_on(self.files.unlink(parent, name.as_bytes()).instrument(span))
        {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        self.metrics.record_fuse_callback(FuseCallback::Rmdir);
        let span = filesystem_span("dms.filesystem.rmdir", Some(parent));
        match self
            .runtime
            .block_on(self.files.rmdir(parent, name.as_bytes()).instrument(span))
        {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        flags: u32,
        reply: ReplyEmpty,
    ) {
        self.metrics.record_fuse_callback(FuseCallback::Rename);
        if flags != 0 {
            reply.error(libc::ENOSYS);
            return;
        }
        let span = filesystem_span("dms.filesystem.rename", Some(parent));
        match self.runtime.block_on(
            self.files
                .rename(parent, name.as_bytes(), newparent, newname.as_bytes(), true)
                .instrument(span),
        ) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        self.metrics.record_fuse_callback(FuseCallback::Read);
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let span = filesystem_span("dms.filesystem.read", Some(_ino));
        match self.runtime.block_on(
            self.files
                .read(fh, offset as u64, u64::from(size))
                .instrument(span),
        ) {
            Ok(Some(read)) => {
                self.metrics
                    .record_fuse_callback_bytes(FuseCallback::Read, read.bytes.len());
                reply.data(&read.bytes);
            }
            Ok(None) => reply.error(libc::ENOENT),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        self.metrics.record_fuse_callback(FuseCallback::Write);
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let span = filesystem_span("dms.filesystem.write", Some(ino));
        match self
            .runtime
            .block_on(self.files.write(fh, offset as u64, data).instrument(span))
        {
            Ok(_) => {
                self.metrics
                    .record_fuse_callback_bytes(FuseCallback::Write, data.len());
                reply.written(data.len() as u32);
            }
            Err(error) => {
                let errno = worker_to_errno(error);
                dms_logging::warn!(
                    "FUSE write-through commit failed";
                    "event" => "node.fuse.write_failed",
                    "inode" => ino,
                    "offset" => offset,
                    "length" => data.len(),
                    "errno" => errno,
                );
                reply.error(errno);
            }
        }
    }

    fn flush(&mut self, _req: &Request, _ino: u64, _fh: u64, _owner: u64, reply: ReplyEmpty) {
        self.metrics.record_fuse_callback(FuseCallback::Flush);
        // write-through 的每个 write callback 已经完成 Meta 原子发布。
        reply.ok();
    }

    fn fsync(&mut self, _req: &Request, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) {
        self.metrics.record_fuse_callback(FuseCallback::Fsync);
        // 当前持久性边界与 write 相同；没有尚未发布的本地 dirty buffer。
        reply.ok();
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
        self.metrics.record_fuse_callback(FuseCallback::Release);
        let span = filesystem_span("dms.filesystem.close", Some(_ino));
        match self.runtime.block_on(self.files.close(fh).instrument(span)) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(worker_to_errno(error)),
        }
    }

    fn ioctl(
        &mut self,
        _req: &Request,
        _ino: u64,
        _fh: u64,
        _flags: u32,
        _cmd: u32,
        _in_data: &[u8],
        _out_size: u32,
        reply: ReplyIoctl,
    ) {
        // Python 等运行时在把普通文件包装成 buffered reader 时会探测它是不是 TTY。
        // 普通文件应直接回答 ENOTTY；沿用 fuser 的默认实现会为每次探测记录一条
        // "Not Implemented" 警告，污染日志并给热读路径增加不必要的格式化开销。
        reply.error(libc::ENOTTY);
    }
}

fn filesystem_span(operation: &'static str, inode: Option<u64>) -> dms_tracing::tracing::Span {
    dms_tracing::tracing::info_span!(
        "dms.filesystem.operation",
        otel.name = operation,
        otel.kind = "internal",
        inode = inode.unwrap_or_default(),
    )
}

fn file_attr(attributes: &InodeAttributes) -> FileAttr {
    FileAttr {
        ino: attributes.inode,
        size: attributes.size,
        blocks: attributes.size.div_ceil(u64::from(BLOCK_SIZE)),
        atime: system_time(attributes.atime_unix_nanos),
        mtime: system_time(attributes.mtime_unix_nanos),
        ctime: system_time(attributes.ctime_unix_nanos),
        crtime: system_time(attributes.ctime_unix_nanos),
        kind: file_type(attributes.kind),
        perm: (attributes.mode & 0o7777) as u16,
        nlink: attributes.link_count,
        uid: attributes.uid,
        gid: attributes.gid,
        rdev: 0,
        blksize: BLOCK_SIZE,
        flags: 0,
    }
}

fn file_type(kind: InodeKind) -> FileType {
    match kind {
        InodeKind::RegularFile => FileType::RegularFile,
        InodeKind::Directory => FileType::Directory,
        InodeKind::SymbolicLink => FileType::Symlink,
    }
}

fn system_time(unix_nanos: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(unix_nanos.max(0) as u64)
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
        WorkerError::UnknownTransfer => libc::EIO,
        WorkerError::Stable(error) => stable_error_to_errno(&error),
    }
}

/// 把跨 Meta 边界保留下来的稳定错误语义翻译成 POSIX errno。
///
/// 精确的文件系统错误优先按数字错误码映射；其它错误再按公共 `ErrorKind` 降级。
/// 这样 Meta 的 `DirectoryNotEmpty` 不会在 Node/FUSE 边界被笼统吞成 `EIO`。
fn stable_error_to_errno(error: &DmsError) -> i32 {
    match error.code() {
        dms_error::META_FILESYSTEM_ALREADY_EXISTS => libc::EEXIST,
        dms_error::META_FILESYSTEM_NOT_DIRECTORY => libc::ENOTDIR,
        dms_error::META_FILESYSTEM_IS_DIRECTORY => libc::EISDIR,
        dms_error::META_FILESYSTEM_DIRECTORY_NOT_EMPTY => libc::ENOTEMPTY,
        dms_error::META_FILESYSTEM_STALE_REVISION => libc::ESTALE,
        _ => match error.kind() {
            ErrorKind::InvalidArgument => libc::EINVAL,
            ErrorKind::NotFound => libc::ENOENT,
            ErrorKind::AlreadyExists => libc::EEXIST,
            ErrorKind::PermissionDenied | ErrorKind::Unauthenticated => libc::EACCES,
            ErrorKind::ResourceExhausted => libc::ENOSPC,
            ErrorKind::FailedPrecondition | ErrorKind::Aborted => libc::ESTALE,
            ErrorKind::Unimplemented => libc::ENOSYS,
            ErrorKind::Unavailable | ErrorKind::DeadlineExceeded => libc::EHOSTUNREACH,
            ErrorKind::Unknown | ErrorKind::Internal | ErrorKind::DataLoss => libc::EIO,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable_error(code: dms_error::ErrorCode, kind: ErrorKind) -> WorkerError {
        WorkerError::Stable(DmsError::new(code, kind, "test error"))
    }

    #[test]
    fn filesystem_stable_errors_preserve_posix_errno() {
        assert_eq!(
            worker_to_errno(stable_error(
                dms_error::META_FILESYSTEM_ALREADY_EXISTS,
                ErrorKind::AlreadyExists,
            )),
            libc::EEXIST,
        );
        assert_eq!(
            worker_to_errno(stable_error(
                dms_error::META_FILESYSTEM_NOT_DIRECTORY,
                ErrorKind::FailedPrecondition,
            )),
            libc::ENOTDIR,
        );
        assert_eq!(
            worker_to_errno(stable_error(
                dms_error::META_FILESYSTEM_IS_DIRECTORY,
                ErrorKind::FailedPrecondition,
            )),
            libc::EISDIR,
        );
        assert_eq!(
            worker_to_errno(stable_error(
                dms_error::META_FILESYSTEM_DIRECTORY_NOT_EMPTY,
                ErrorKind::FailedPrecondition,
            )),
            libc::ENOTEMPTY,
        );
        assert_eq!(
            worker_to_errno(stable_error(
                dms_error::META_FILESYSTEM_STALE_REVISION,
                ErrorKind::Aborted,
            )),
            libc::ESTALE,
        );
    }
}

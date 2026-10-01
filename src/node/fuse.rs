//! 统一 POSIX/FUSE 入口。
//!
//! 接收内核回调，转换到 VFS `Backend` 接口并映射 errno/回复；管理 FUSE 会话内的
//! inode、目录项和打开句柄。每个 mount session 绑定一个 Backend；FUSE inode 编号
//! 不能当作后端持久文件身份。OwnerFs 的独用 Home 根可短暂缓存；首次远端
//! 访问先通过 FUSE notifier 失效本机缓存，再由 Home 执行该请求。

mod state;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use afs_error::{Error, Result};
use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, KernelConfig, LockOptions, MountOption,
    ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyLock,
    ReplyOpen, ReplyWrite, ReplyXattr, Request, TimeOrNow, consts,
};

#[cfg(feature = "ownerfs")]
use crate::node::vfs::ownerfs::OwnerFs;
use crate::node::vfs::{
    Backend,
    locks::{LockRequest, LockWaiterId},
    types::{
        AttributeChange, BackendInode, DirectoryEntry, Entry, FileAttributes, FileHandle, FileKind,
        FileLockConflict, FileLockKind, FileLockOwner, FileLockRange, FileLockType, OpenOptions,
        ReleaseKind, RenameFlags, RequestContext, SetAttrOptions, SpecialFileKind, SyncMode,
        WriteOptions,
    },
};

use self::state::{FuseNode, FuseState, ROOT_INO};

const TTL: Duration = Duration::ZERO;
const DIRECT_IO: u32 = consts::FOPEN_DIRECT_IO;
const LOCK_WORKERS: usize = 4;
const MAX_PENDING_LOCK_INTERRUPTS: usize = 4096;

static NEXT_INGRESS_SESSION: AtomicU64 = AtomicU64::new(1);

/// Process-owned mount with an observable backend cleanup result. Plain drop
/// still unmounts; a clean Node stop must call join and inspect its result.
#[derive(Debug)]
pub struct MountedFuse {
    session: BackgroundSession,
    cleanup_error: Arc<Mutex<Option<Error>>>,
}

impl MountedFuse {
    pub fn join(self) -> Result<()> {
        let joined = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.session.join()));
        let cleanup_error = self
            .cleanup_error
            .lock()
            .map_err(|_| Error::from(std::io::Error::other("FUSE cleanup state is poisoned")))?
            .take();
        if let Some(error) = cleanup_error {
            return Err(error);
        }
        joined.map_err(|_| Error::from(std::io::Error::other("FUSE session cleanup thread failed")))
    }
}

/// 建立真正的内核 FUSE 挂载。一个 session 只绑定一个业务 Backend。
pub fn mount_dfs(backend: Arc<dyn Backend>, path: &Path) -> Result<MountedFuse> {
    reject_existing_mount(path)?;
    let mut fs = AfsFuse::new(backend);
    // Cached write-through sends each syscall write to the inode owner and
    // keeps this mount's pages coherent. Do not enable writeback or KEEP_CACHE:
    // ordinary opens must refresh after another mount's close barrier.
    fs.cached_io = true;
    let cleanup_error = fs.cleanup_error.clone();
    let session = fuser::spawn_mount2(
        fs,
        path,
        &[
            MountOption::FSName("afs-dfs".into()),
            MountOption::NoAtime,
            MountOption::AllowOther,
            MountOption::DefaultPermissions,
        ],
    )
    .map_err(Error::from)?;
    Ok(MountedFuse {
        session,
        cleanup_error,
    })
}

/// OwnerFs 使用相同 FUSE 实现，并额外接入其本地 Home 缓存策略与 notifier。
#[cfg(feature = "ownerfs")]
pub fn mount_ownerfs(ownerfs: Arc<OwnerFs>, path: &Path) -> Result<MountedFuse> {
    reject_existing_mount(path)?;
    let mut fs = AfsFuse::new(ownerfs.clone());
    fs.ownerfs = Some(ownerfs.clone());
    let cleanup_error = fs.cleanup_error.clone();
    let session = fuser::spawn_mount2(
        fs,
        path,
        &[
            MountOption::FSName("afs-ownerfs".into()),
            MountOption::NoAtime,
            MountOption::AllowOther,
            MountOption::DefaultPermissions,
        ],
    )
    .map_err(Error::from)?;
    ownerfs.register_fuse_notifier(session.notifier());
    Ok(MountedFuse {
        session,
        cleanup_error,
    })
}

#[doc(hidden)]
pub fn mount_test_backend(backend: Arc<dyn Backend>, path: &Path) -> Result<MountedFuse> {
    reject_existing_mount(path)?;
    let fs = AfsFuse::new(backend);
    let cleanup_error = fs.cleanup_error.clone();
    let session = fuser::spawn_mount2(
        fs,
        path,
        &[MountOption::FSName("afs-test".into()), MountOption::NoAtime],
    )
    .map_err(Error::from)?;
    Ok(MountedFuse {
        session,
        cleanup_error,
    })
}

fn reject_existing_mount(path: &Path) -> Result<()> {
    let target = path.canonicalize().map_err(Error::from)?;
    for mountpoint in current_mountpoints()? {
        if mountpoint
            .canonicalize()
            .is_ok_and(|mounted| mounted == target)
        {
            return Err(afs_error::Error::coded(
                afs_error::NODE_MOUNT_CONFLICT,
                format!("mount point '{}' is already mounted", path.display()),
            ));
        }
    }
    Ok(())
}

fn current_mountpoints() -> Result<Vec<PathBuf>> {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").map_err(Error::from)?;
    mountinfo
        .lines()
        .map(|line| {
            line.split_whitespace()
                .nth(4)
                .ok_or_else(|| {
                    afs_error::Error::coded(
                        afs_error::RUNTIME_INTERNAL,
                        "malformed /proc/self/mountinfo line",
                    )
                })
                .and_then(decode_mountinfo_path)
        })
        .collect()
}

fn decode_mountinfo_path(encoded: &str) -> Result<PathBuf> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            if index + 3 >= bytes.len() {
                return Err(afs_error::Error::coded(
                    afs_error::RUNTIME_INTERNAL,
                    format!("malformed mountinfo escape in '{encoded}'"),
                ));
            }
            let value = parse_octal(&bytes[index + 1..index + 4]).ok_or_else(|| {
                afs_error::Error::coded(
                    afs_error::RUNTIME_INTERNAL,
                    format!("malformed mountinfo escape in '{encoded}'"),
                )
            })?;
            decoded.push(value);
            index += 4;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&decoded).into_owned(),
    ))
}

fn parse_octal(bytes: &[u8]) -> Option<u8> {
    bytes.iter().try_fold(0u8, |acc, byte| match byte {
        b'0'..=b'7' => acc.checked_mul(8)?.checked_add(byte - b'0'),
        _ => None,
    })
}

type FuseJob = Box<dyn FnOnce() + Send + 'static>;

// fuser dispatches callbacks on one receive thread. Different file handles may
// run concurrently, but requests for the same handle must remain FIFO so an
// fsync cannot pass its preceding write, or release pass fsync. The bounded
// queue backpressures the receive thread if peers stall.
struct FuseDispatch {
    shared: Arc<(Mutex<FuseQueue>, Condvar)>,
    capacity: usize,
}

struct FuseQueue {
    ready: VecDeque<(Option<u64>, FuseJob)>,
    waiting: HashMap<u64, VecDeque<FuseJob>>,
    active: HashSet<u64>,
    jobs: usize,
    closed: bool,
}

impl FuseDispatch {
    fn new(workers: usize) -> Self {
        let shared = Arc::new((
            Mutex::new(FuseQueue {
                ready: VecDeque::new(),
                waiting: HashMap::new(),
                active: HashSet::new(),
                jobs: 0,
                closed: false,
            }),
            Condvar::new(),
        ));
        for index in 0..workers {
            let shared = shared.clone();
            thread::Builder::new()
                .name(format!("afs-fuse-{index}"))
                .spawn(move || {
                    loop {
                        let (key, job) = {
                            let (queue, wake) = &*shared;
                            let mut queue = queue.lock().unwrap();
                            while queue.ready.is_empty() && !queue.closed {
                                queue = wake.wait(queue).unwrap();
                            }
                            let Some(job) = queue.ready.pop_front() else {
                                break;
                            };
                            job
                        };
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
                            afs_logging::error!("FUSE worker callback panicked");
                        }
                        let (queue, wake) = &*shared;
                        let mut queue = queue.lock().unwrap();
                        queue.jobs -= 1;
                        if let Some(key) = key {
                            let next = queue.waiting.get_mut(&key).and_then(VecDeque::pop_front);
                            if queue.waiting.get(&key).is_some_and(VecDeque::is_empty) {
                                queue.waiting.remove(&key);
                            }
                            if let Some(next) = next {
                                queue.ready.push_back((Some(key), next));
                            } else {
                                queue.active.remove(&key);
                            }
                        }
                        wake.notify_all();
                    }
                })
                .expect("FUSE worker thread must start");
        }
        Self {
            shared,
            capacity: workers * 8,
        }
    }

    fn submit(&self, job: impl FnOnce() + Send + 'static) {
        let (queue, wake) = &*self.shared;
        let mut queue = queue.lock().unwrap();
        if queue.jobs >= self.capacity || queue.closed {
            drop(queue);
            job();
            return;
        }
        queue.jobs += 1;
        queue.ready.push_back((None, Box::new(job)));
        wake.notify_one();
    }

    fn submit_keyed(&self, key: u64, job: impl FnOnce() + Send + 'static) {
        let (queue, wake) = &*self.shared;
        let mut queue = queue.lock().unwrap();
        while queue.jobs >= self.capacity && !queue.closed {
            queue = wake.wait(queue).unwrap();
        }
        if queue.closed {
            drop(queue);
            job();
            return;
        }
        queue.jobs += 1;
        if queue.active.insert(key) {
            queue.ready.push_back((Some(key), Box::new(job)));
            wake.notify_one();
        } else {
            queue
                .waiting
                .entry(key)
                .or_default()
                .push_back(Box::new(job));
        }
    }

    fn try_submit_or_reject(&self, job: impl FnOnce() -> FuseJob, reject: impl FnOnce()) {
        let (queue, wake) = &*self.shared;
        let mut queue = queue.lock().unwrap();
        if queue.jobs >= self.capacity || queue.closed {
            drop(queue);
            reject();
            return;
        }
        queue.jobs += 1;
        queue.ready.push_back((None, job()));
        wake.notify_one();
    }

    fn close_and_drain(&self) {
        let (queue, wake) = &*self.shared;
        let mut state = queue.lock().unwrap();
        state.closed = true;
        wake.notify_all();
        // Workers retain ownership of accepted callbacks. Wait for their
        // completion; the Node process deadline covers a blocked callback.
        while state.jobs != 0 {
            state = wake.wait(state).unwrap();
        }
    }
}

impl Drop for FuseDispatch {
    fn drop(&mut self) {
        self.close_and_drain();
    }
}

#[derive(Debug, Default)]
struct PendingLockRegistry {
    state: Mutex<PendingLockState>,
}

#[derive(Debug, Default)]
struct PendingLockState {
    pending: HashMap<u64, LockWaiterId>,
    interrupted_before_register: HashSet<u64>,
}

impl PendingLockRegistry {
    fn register(&self, unique: u64, waiter: LockWaiterId) -> bool {
        let mut state = self.state.lock().unwrap();
        let interrupted = state.interrupted_before_register.remove(&unique);
        state.pending.insert(unique, waiter);
        interrupted
    }

    fn finish(&self, unique: u64) {
        self.state.lock().unwrap().pending.remove(&unique);
    }

    fn interrupt(&self, unique: u64) -> Option<LockWaiterId> {
        let mut state = self.state.lock().unwrap();
        if let Some(waiter) = state.pending.remove(&unique) {
            return Some(waiter);
        }
        if state.interrupted_before_register.len() < MAX_PENDING_LOCK_INTERRUPTS {
            state.interrupted_before_register.insert(unique);
        }
        None
    }

    fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.pending.clear();
        state.interrupted_before_register.clear();
    }
}

pub struct AfsFuse {
    backend: Arc<dyn Backend>,
    cached_io: bool,
    state: Arc<Mutex<FuseState>>,
    file_handle_inodes: Arc<Mutex<HashMap<u64, u64>>>,
    dispatch: FuseDispatch,
    lock_dispatch: FuseDispatch,
    pending_locks: Arc<PendingLockRegistry>,
    ingress_session_id: String,
    lock_session_cleaned: bool,
    callbacks_drained: bool,
    cleanup_error: Arc<Mutex<Option<Error>>>,
    #[cfg(feature = "ownerfs")]
    ownerfs: Option<Arc<OwnerFs>>,
}

impl AfsFuse {
    #[must_use]
    fn new(backend: Arc<dyn Backend>) -> Self {
        let state = Arc::new(Mutex::new(FuseState::new(backend.root_inode())));
        Self {
            backend,
            cached_io: false,
            state,
            file_handle_inodes: Arc::new(Mutex::new(HashMap::new())),
            dispatch: FuseDispatch::new(8),
            lock_dispatch: FuseDispatch::new(LOCK_WORKERS),
            pending_locks: Arc::new(PendingLockRegistry::default()),
            ingress_session_id: next_ingress_session_id(),
            lock_session_cleaned: false,
            callbacks_drained: false,
            cleanup_error: Arc::new(Mutex::new(None)),
            #[cfg(feature = "ownerfs")]
            ownerfs: None,
        }
    }

    fn remember_cached_inode(&self, _inode: BackendInode, ino: u64) {
        #[cfg(feature = "ownerfs")]
        if let Some(ownerfs) = &self.ownerfs {
            ownerfs.remember_fuse_inode(ino);
        }
        #[cfg(not(feature = "ownerfs"))]
        let _ = ino;
    }

    fn with_cache_policy<T>(
        &self,
        inode: BackendInode,
        reply: impl FnOnce(Duration, bool) -> T,
    ) -> T {
        #[cfg(feature = "ownerfs")]
        if let Some(ownerfs) = &self.ownerfs {
            return ownerfs.with_fuse_cache_policy(inode, reply);
        }
        #[cfg(not(feature = "ownerfs"))]
        let _ = inode;
        reply(TTL, false)
    }

    fn context(req: &Request<'_>, umask: u32) -> RequestContext {
        RequestContext {
            uid: req.uid(),
            gid: req.gid(),
            pid: req.pid(),
            umask,
            supplementary_gids: Vec::new(),
        }
    }

    fn metadata_context(req: &Request<'_>, umask: u32) -> RequestContext {
        let mut context = Self::context(req, umask);
        context.supplementary_gids = request_groups(&context);
        context
    }

    fn backend_inode(&self, ino: u64) -> std::result::Result<BackendInode, i32> {
        self.state
            .lock()
            .unwrap()
            .backend_inode(ino)
            .ok_or(libc::ESTALE)
    }

    fn file_handle_for_inode_in(
        state: &Mutex<FuseState>,
        file_handle_inodes: &Mutex<HashMap<u64, u64>>,
        ino: u64,
        fh: u64,
    ) -> std::result::Result<FileHandle, i32> {
        Self::validate_handle_inode_in(state, ino)?;
        match file_handle_inodes.lock().unwrap().get(&fh).copied() {
            Some(mapped_ino) if mapped_ino == ino => state
                .lock()
                .unwrap()
                .file_handle(fh)
                .map(|file| file.handle)
                .ok_or(libc::ESTALE),
            _ => Err(libc::ESTALE),
        }
    }

    fn file_lock_owner(&self, kernel_owner: u64) -> FileLockOwner {
        FileLockOwner {
            ingress_session_id: self.ingress_session_id.clone(),
            kernel_owner,
        }
    }

    fn lock_waiter_id(&self, unique: u64) -> LockWaiterId {
        LockWaiterId {
            ingress_session_id: self.ingress_session_id.clone(),
            request_id: unique,
        }
    }

    fn cleanup_lock_session(&mut self) {
        if self.lock_session_cleaned {
            return;
        }
        self.lock_session_cleaned = true;
        self.pending_locks.clear();
        self.release_lock_session();
    }

    fn release_lock_session(&self) {
        if let Err(error) = self.backend.release_lock_session(&self.ingress_session_id) {
            let errno = errno(error.clone());
            self.cleanup_error.lock().unwrap().get_or_insert(error);
            afs_logging::error!("fuse.lock_session_cleanup_failed"; "errno" => errno);
        }
    }

    fn finish_callbacks(&mut self) {
        if self.callbacks_drained {
            return;
        }
        // Fence the session and cancel blocking lock waits first. OwnerFs,
        // DFS and LockTable reject late lock acquisition for this session.
        self.cleanup_lock_session();
        self.dispatch.close_and_drain();
        self.lock_dispatch.close_and_drain();
        // Final idempotent sweep observes every accepted callback's terminal
        // outcome before a joined mount can report successful cleanup.
        self.release_lock_session();
        self.callbacks_drained = true;
    }
}

impl Drop for AfsFuse {
    fn drop(&mut self) {
        self.finish_callbacks();
    }
}

impl Filesystem for AfsFuse {
    fn init(&mut self, _: &Request<'_>, config: &mut KernelConfig) -> std::result::Result<(), i32> {
        let _ = config.set_max_write(1024 * 1024);
        // Linux otherwise serializes LOOKUPs in one directory even when our
        // FUSE receive thread dispatches them to independent workers.
        let _ = config.add_capabilities(fuser::consts::FUSE_PARALLEL_DIROPS);
        if self.backend.supports_killpriv_v2() {
            let _ = config.add_capabilities(fuser::consts::FUSE_HANDLE_KILLPRIV_V2);
        }
        if self.backend.supports_advisory_locks() {
            let _ = config.add_capabilities(fuser::consts::FUSE_POSIX_LOCKS);
            let _ = config.add_capabilities(fuser::consts::FUSE_FLOCK_LOCKS);
        }
        Ok(())
    }

    fn destroy(&mut self) {
        self.finish_callbacks();
    }

    fn lookup(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let Ok(parent_inode) = self.backend_inode(parent) else {
            reply.error(libc::ESTALE);
            return;
        };
        // A local Home directory lookup has no network wait. Keep it on the
        // FUSE receive thread; remote lookups still need worker concurrency.
        #[cfg(feature = "ownerfs")]
        let inline_local_lookup = self
            .ownerfs
            .as_ref()
            .is_some_and(|ownerfs| ownerfs.is_local_inode(parent_inode));
        #[cfg(not(feature = "ownerfs"))]
        let inline_local_lookup = false;
        let backend = self.backend.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let name = name.to_os_string();
        #[cfg(feature = "ownerfs")]
        let ownerfs = self.ownerfs.clone();
        let run = move || {
            let result = backend.lookup(&context, parent_inode, &name).map_err(errno);
            match result {
                Ok(entry) => {
                    let ino = state.lock().unwrap().remember_lookup(&entry);
                    #[cfg(feature = "ownerfs")]
                    if let Some(ownerfs) = &ownerfs {
                        ownerfs.remember_fuse_inode(ino);
                        ownerfs.with_fuse_lookup_policy(entry.inode, |entry_ttl, attr_ttl| {
                            reply.entry_with_ttls(
                                &entry_ttl,
                                &attr_ttl,
                                &file_attr(ino, &entry.attributes),
                                0,
                            );
                        });
                        return;
                    }
                    reply.entry(&TTL, &file_attr(ino, &entry.attributes), 0);
                }
                Err(error) => reply.error(error),
            }
        };
        if inline_local_lookup {
            run();
        } else {
            self.dispatch.submit(run);
        }
    }

    fn forget(&mut self, _: &Request<'_>, ino: u64, nlookup: u64) {
        self.state.lock().unwrap().forget(ino, nlookup);
    }

    fn getattr(&mut self, req: &Request<'_>, ino: u64, fh: Option<u64>, reply: ReplyAttr) {
        let inline_local_read = fh.is_some_and(|fh| {
            self.state
                .lock()
                .unwrap()
                .file_handle(fh)
                .is_some_and(|file| file.inline_local_read)
        });
        let backend = self.backend.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        #[cfg(feature = "ownerfs")]
        let ownerfs = self.ownerfs.clone();
        let run = move || {
            let node = state.lock().unwrap().node(ino);
            let result = match node {
                Some(FuseNode::Root(inode)) | Some(FuseNode::Backend(inode)) => {
                    let handle = fh.and_then(|fh| state.lock().unwrap().file_handle(fh));
                    handle
                        .map_or(Ok(()), |_| AfsFuse::validate_handle_inode_in(&state, ino))
                        .and_then(|()| {
                            backend
                                .getattr(&context, inode, handle.map(|handle| handle.handle))
                                .map(|attributes| file_attr(ino, &attributes))
                                .map_err(errno)
                        })
                }
                None => Err(libc::ESTALE),
            };
            let backend_inode = state.lock().unwrap().backend_inode(ino);
            match result {
                Ok(attr) => match backend_inode {
                    Some(inode) => {
                        #[cfg(feature = "ownerfs")]
                        if let Some(ownerfs) = &ownerfs {
                            ownerfs.with_fuse_cache_policy(inode, |ttl, _| reply.attr(&ttl, &attr));
                            return;
                        }
                        #[cfg(not(feature = "ownerfs"))]
                        let _ = inode;
                        reply.attr(&TTL, &attr);
                    }
                    None => reply.attr(&TTL, &attr),
                },
                Err(error) => reply.error(error),
            }
        };
        if let Some(fh) = fh.filter(|_| !inline_local_read) {
            self.dispatch.submit_keyed(fh, run);
        } else {
            run();
        }
    }

    fn setattr(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        kill_suidgid: bool,
        reply: ReplyAttr,
    ) {
        let inline_local_read = fh.is_some_and(|fh| {
            self.state
                .lock()
                .unwrap()
                .file_handle(fh)
                .is_some_and(|file| file.inline_local_read)
        });
        let backend = self.backend.clone();
        let state = self.state.clone();
        let context = Self::metadata_context(req, 0);
        let timestamps_now = timestamps_now_only(atime.as_ref(), mtime.as_ref());
        let change = AttributeChange {
            size,
            mode,
            uid,
            gid,
            atime: atime.map(time_or_now),
            mtime: mtime.map(time_or_now),
        };
        let options = SetAttrOptions {
            kill_suidgid,
            timestamps_now,
        };
        let run = move || {
            let inode = state.lock().unwrap().backend_inode(ino);
            let result = inode.ok_or(libc::ESTALE).and_then(|inode| {
                let handle = fh.and_then(|fh| state.lock().unwrap().file_handle(fh));
                if handle.is_some() {
                    AfsFuse::validate_handle_inode_in(&state, ino)?;
                }
                Ok(backend.as_ref()).and_then(|backend| {
                    backend
                        .setattr_with_options(
                            &context,
                            inode,
                            handle.map(|handle| handle.handle),
                            &change,
                            options,
                        )
                        .map_err(errno)
                })
            });
            match result {
                Ok(attributes) => reply.attr(&TTL, &file_attr(ino, &attributes)),
                Err(error) => reply.error(error),
            }
        };
        if let Some(fh) = fh.filter(|_| !inline_local_read) {
            self.dispatch.submit_keyed(fh, run);
        } else {
            run();
        }
    }

    fn readlink(&mut self, req: &Request<'_>, ino: u64, reply: ReplyData) {
        let result = self.backend_inode(ino).and_then(|inode| {
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .readlink(&Self::context(req, 0), inode)
                    .map(|target| os_str_bytes(target.as_os_str()).to_vec())
                    .map_err(errno)
            })
        });
        match result {
            Ok(data) => reply.data(&data),
            Err(error) => reply.error(error),
        }
    }

    fn mkdir(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let result = self.backend_inode(parent).and_then(|parent_inode| {
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .mkdir(&Self::context(req, umask), parent_inode, name, mode)
                    .map_err(errno)
            })
        });
        match result {
            Ok(entry) => {
                let ino = self.state.lock().unwrap().remember_lookup(&entry);
                reply.entry(&TTL, &file_attr(ino, &entry.attributes), 0);
                #[cfg(feature = "ownerfs")]
                if parent == ROOT_INO
                    && let Some(ownerfs) = &self.ownerfs
                {
                    ownerfs.workspace_created(entry.inode, name);
                }
            }
            Err(error) => reply.error(error),
        }
    }

    fn unlink(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let result = self.backend_inode(parent).and_then(|parent_inode| {
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .unlink(&Self::context(req, 0), parent_inode, name)
                    .map_err(errno)
            })
        });
        reply_empty(reply, result);
    }

    fn rmdir(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let result = self.backend_inode(parent).and_then(|parent_inode| {
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .rmdir(&Self::context(req, 0), parent_inode, name)
                    .map_err(errno)
            })
        });
        reply_empty(reply, result);
    }

    fn symlink(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        let result = self.backend_inode(parent).and_then(|parent_inode| {
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .symlink(
                        &Self::context(req, 0),
                        parent_inode,
                        name,
                        target.as_os_str(),
                    )
                    .map_err(errno)
            })
        });
        match result {
            Ok(entry) => {
                let ino = self.state.lock().unwrap().remember_lookup(&entry);
                reply.entry(&TTL, &file_attr(ino, &entry.attributes), 0);
            }
            Err(error) => reply.error(error),
        }
    }

    fn rename(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        flags: u32,
        reply: ReplyEmpty,
    ) {
        afs_logging::debug!(
            "fuse.rename";
            "parent" => parent,
            "name" => name.to_string_lossy().into_owned(),
            "newparent" => newparent,
            "newname" => newname.to_string_lossy().into_owned(),
            "flags" => flags,
        );
        let result = self.backend_inode(parent).and_then(|from_parent| {
            let to_parent = self.backend_inode(newparent)?;
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .rename(
                        &Self::context(req, 0),
                        from_parent,
                        name,
                        to_parent,
                        newname,
                        RenameFlags(flags),
                    )
                    .map_err(errno)
            })
        });
        reply_empty(reply, result);
    }

    fn link(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        newparent: u64,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        let result = self.backend_inode(ino).and_then(|inode| {
            let parent = self.backend_inode(newparent)?;
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .link(&Self::context(req, 0), inode, parent, newname)
                    .map_err(errno)
            })
        });
        match result {
            Ok(entry) => {
                let ino = self.state.lock().unwrap().remember_lookup(&entry);
                reply.entry(&TTL, &file_attr(ino, &entry.attributes), 0);
            }
            Err(error) => reply.error(error),
        }
    }

    fn open(&mut self, req: &Request<'_>, ino: u64, flags: i32, open_flags: u32, reply: ReplyOpen) {
        let Ok(inode) = self.backend_inode(ino) else {
            reply.error(libc::ESTALE);
            return;
        };
        let options = OpenOptions {
            kill_suidgid: open_flags & consts::FUSE_OPEN_KILL_SUIDGID != 0,
        };
        // The kernel cannot submit operations on this fh until open replies.
        // This lets independent writable opens overlap with read-only ones;
        // O_PATH remains on the original path because it has no I/O handle.
        if flags & libc::O_PATH != 0 {
            let result = Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .open_with_options(&Self::context(req, 0), inode, flags, options)
                    .map_err(errno)
            });
            match result {
                Ok(handle) => {
                    let fh = self.state.lock().unwrap().insert_file_handle(handle);
                    self.file_handle_inodes.lock().unwrap().insert(fh, ino);
                    self.with_cache_policy(inode, |_, private| {
                        reply.opened(fh, if private { 0 } else { DIRECT_IO });
                    });
                }
                Err(error) => reply.error(error),
            }
            return;
        }
        #[cfg(feature = "ownerfs")]
        let inline_local_read = flags & libc::O_ACCMODE == libc::O_RDONLY
            && self
                .ownerfs
                .as_ref()
                .is_some_and(|ownerfs| ownerfs.is_local_inode(inode));
        #[cfg(not(feature = "ownerfs"))]
        let inline_local_read = false;
        let backend = self.backend.clone();
        let state = self.state.clone();
        let file_handle_inodes = self.file_handle_inodes.clone();
        let context = Self::context(req, 0);
        let cached_io = self.cached_io;
        #[cfg(feature = "ownerfs")]
        let ownerfs = self.ownerfs.clone();
        let run = move || {
            let result = backend
                .open_with_options(&context, inode, flags, options)
                .map_err(errno);
            match result {
                Ok(handle) => {
                    let fh = state
                        .lock()
                        .unwrap()
                        .insert_file_handle_with_policy(handle, inline_local_read);
                    file_handle_inodes.lock().unwrap().insert(fh, ino);
                    #[cfg(feature = "ownerfs")]
                    if let Some(ownerfs) = &ownerfs {
                        ownerfs.with_fuse_cache_policy(inode, |_, private| {
                            reply.opened(fh, if private { 0 } else { DIRECT_IO });
                        });
                        return;
                    }
                    reply.opened(fh, if cached_io { 0 } else { DIRECT_IO });
                }
                Err(error) => reply.error(error),
            }
        };
        if inline_local_read {
            run();
        } else {
            self.dispatch.submit(run);
        }
    }

    fn read(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let inline_local_read = self
            .state
            .lock()
            .unwrap()
            .file_handle(fh)
            .is_some_and(|file| file.inline_local_read);
        let backend = self.backend.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let run = move || {
            let result = checked_offset(offset).and_then(|offset| {
                let file = state.lock().unwrap().file_handle(fh).ok_or(libc::ESTALE)?;
                AfsFuse::validate_handle_inode_in(&state, ino)?;
                let mut out = vec![0; size as usize];
                Ok(backend.as_ref()).and_then(|backend| {
                    backend
                        .read(&context, file.handle, offset, &mut out)
                        .map(|n| {
                            out.truncate(n);
                            out
                        })
                        .map_err(errno)
                })
            });
            match result {
                Ok(data) => reply.data(&data),
                Err(error) => reply.error(error),
            }
        };
        if inline_local_read {
            run();
        } else {
            self.dispatch.submit_keyed(fh, run);
        }
    }

    fn write(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let backend = self.backend.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let data = data.to_vec();
        let options = WriteOptions {
            kill_suidgid: write_flags & consts::FUSE_WRITE_KILL_SUIDGID != 0,
        };
        self.dispatch.submit_keyed(fh, move || {
            let result = checked_offset(offset).and_then(|offset| {
                let file = state.lock().unwrap().file_handle(fh).ok_or(libc::ESTALE)?;
                AfsFuse::validate_handle_inode_in(&state, ino)?;
                Ok(backend.as_ref()).and_then(|backend| {
                    backend
                        .write_with_options(&context, file.handle, offset, &data, options)
                        .map_err(errno)
                })
            });
            match result {
                Ok(written) => match u32::try_from(written) {
                    Ok(written) => reply.written(written),
                    Err(_) => reply.error(libc::EIO),
                },
                Err(error) => reply.error(error),
            }
        });
    }

    fn flush(&mut self, req: &Request<'_>, ino: u64, fh: u64, lock_owner: u64, reply: ReplyEmpty) {
        let inline_local_read = self
            .state
            .lock()
            .unwrap()
            .file_handle(fh)
            .is_some_and(|file| file.inline_local_read);
        let backend = self.backend.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let owner = self.file_lock_owner(lock_owner);
        let run = move || {
            let file = state.lock().unwrap().file_handle(fh);
            let result = file.ok_or(libc::ESTALE).and_then(|file| {
                AfsFuse::validate_handle_inode_in(&state, ino)?;
                let flushed = backend.flush(&context, file.handle).map_err(errno);
                let released = backend
                    .release_locks(
                        &context,
                        state
                            .lock()
                            .unwrap()
                            .backend_inode(ino)
                            .unwrap_or(BackendInode { value: ino }),
                        file.handle,
                        owner,
                        ReleaseKind::PosixOwner,
                    )
                    .map_err(errno);
                combine_empty_results(flushed, released)
            });
            reply_empty(reply, result);
        };
        if inline_local_read {
            run();
        } else {
            self.dispatch.submit_keyed(fh, run);
        }
    }

    fn release(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        _flags: i32,
        lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let inline_local_read = self
            .state
            .lock()
            .unwrap()
            .file_handle(fh)
            .is_some_and(|file| file.inline_local_read);
        let backend = self.backend.clone();
        let state = self.state.clone();
        let file_handle_inodes = self.file_handle_inodes.clone();
        let context = Self::context(req, 0);
        let flock_owner = lock_owner.map(|owner| self.file_lock_owner(owner));
        let run = move || {
            let file = state.lock().unwrap().remove_file_handle(fh);
            file_handle_inodes.lock().unwrap().remove(&fh);
            let result = file.ok_or(libc::ESTALE).and_then(|file| {
                AfsFuse::validate_handle_inode_in(&state, ino)?;
                let inode = state
                    .lock()
                    .unwrap()
                    .backend_inode(ino)
                    .unwrap_or(BackendInode { value: ino });
                let released_flock = flock_owner.map_or(Ok(()), |owner| {
                    backend
                        .release_locks(&context, inode, file.handle, owner, ReleaseKind::FlockOwner)
                        .map_err(errno)
                });
                let released_handle = backend.release(&context, file.handle).map_err(errno);
                combine_empty_results(released_flock, released_handle)
            });
            reply_empty(reply, result);
        };
        if inline_local_read {
            run();
        } else {
            self.dispatch.submit_keyed(fh, run);
        }
    }

    fn fsync(&mut self, req: &Request<'_>, ino: u64, fh: u64, datasync: bool, reply: ReplyEmpty) {
        let inline_local_read = self
            .state
            .lock()
            .unwrap()
            .file_handle(fh)
            .is_some_and(|file| file.inline_local_read);
        let backend = self.backend.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let run = move || {
            let file = state.lock().unwrap().file_handle(fh);
            let result = file.ok_or(libc::ESTALE).and_then(|file| {
                AfsFuse::validate_handle_inode_in(&state, ino)?;
                Ok(backend.as_ref()).and_then(|backend| {
                    backend
                        .fsync(&context, file.handle, sync_mode(datasync))
                        .map_err(errno)
                })
            });
            reply_empty(reply, result);
        };
        if inline_local_read {
            run();
        } else {
            self.dispatch.submit_keyed(fh, run);
        }
    }

    fn opendir(&mut self, req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        let node = self.state.lock().unwrap().node(ino);
        match node {
            Some(FuseNode::Root(_)) | Some(FuseNode::Backend(_)) => {
                let result = self.backend_inode(ino).and_then(|inode| {
                    self.backend
                        .opendir(&Self::context(req, 0), inode)
                        .map_err(errno)
                });
                match result {
                    Ok(handle) => {
                        let fh = self.state.lock().unwrap().insert_directory_handle(handle);
                        reply.opened(fh, 0);
                    }
                    Err(error) => reply.error(error),
                }
            }
            None => reply.error(libc::ESTALE),
        }
    }

    fn readdir(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let node = self.state.lock().unwrap().node(ino);
        match node {
            Some(FuseNode::Root(_)) | Some(FuseNode::Backend(_)) => {
                let directory = self.state.lock().unwrap().directory_handle(fh);
                let result = directory.ok_or(libc::ESTALE).and_then(|directory| {
                    self.validate_handle_inode(ino)?;
                    let cookie = backend_cookie(offset);
                    Ok(self.backend.as_ref()).and_then(|backend| {
                        backend
                            .readdir(&Self::context(req, 0), directory.handle, cookie, 128)
                            .map_err(errno)
                    })
                });
                match result {
                    Ok(entries) => {
                        add_backend_entries(
                            &mut self.state.lock().unwrap(),
                            &mut reply,
                            ino,
                            offset,
                            &entries,
                        );
                        reply.ok();
                    }
                    Err(error) => reply.error(error),
                }
            }
            None => reply.error(libc::ESTALE),
        }
    }

    fn releasedir(&mut self, req: &Request<'_>, ino: u64, fh: u64, _flags: i32, reply: ReplyEmpty) {
        let directory = self.state.lock().unwrap().remove_directory_handle(fh);
        let result = directory.ok_or(libc::ESTALE).and_then(|directory| {
            self.validate_handle_inode(ino)?;
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .releasedir(&Self::context(req, 0), directory.handle)
                    .map_err(errno)
            })
        });
        reply_empty(reply, result);
    }

    fn fsyncdir(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        let directory = self.state.lock().unwrap().directory_handle(fh);
        let result = directory.ok_or(libc::ESTALE).and_then(|directory| {
            self.validate_handle_inode(ino)?;
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .fsyncdir(
                        &Self::context(req, 0),
                        directory.handle,
                        sync_mode(datasync),
                    )
                    .map_err(errno)
            })
        });
        reply_empty(reply, result);
    }

    fn create(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        open_flags: u32,
        reply: ReplyCreate,
    ) {
        afs_logging::info!("fuse.create"; "parent" => parent, "name" => name.to_string_lossy().into_owned());
        let options = OpenOptions {
            kill_suidgid: open_flags & consts::FUSE_OPEN_KILL_SUIDGID != 0,
        };
        let result = self.backend_inode(parent).and_then(|parent_inode| {
            Ok(self.backend.as_ref()).and_then(|backend| {
                backend
                    .create_with_options(
                        &Self::context(req, umask),
                        parent_inode,
                        name,
                        mode,
                        flags,
                        options,
                    )
                    .map_err(errno)
            })
        });
        match result {
            Ok(created) => {
                let ino = self.state.lock().unwrap().remember_lookup(&created.entry);
                self.remember_cached_inode(created.entry.inode, ino);
                let fh = self
                    .state
                    .lock()
                    .unwrap()
                    .insert_file_handle(created.handle);
                self.file_handle_inodes.lock().unwrap().insert(fh, ino);
                self.with_cache_policy(created.entry.inode, |ttl, private| {
                    reply.created(
                        &ttl,
                        &file_attr(ino, &created.entry.attributes),
                        0,
                        fh,
                        if private || self.cached_io {
                            0
                        } else {
                            DIRECT_IO
                        },
                    )
                });
            }
            Err(error) => reply.error(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn getlk_with_options(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        options: LockOptions,
        reply: ReplyLock,
    ) {
        let backend = self.backend.clone();
        let state = self.state.clone();
        let file_handle_inodes = self.file_handle_inodes.clone();
        let context = Self::context(req, 0);
        let owner = self.file_lock_owner(lock_owner);
        let result = self.backend_inode(ino).and_then(|inode| {
            let handle = Self::file_handle_for_inode_in(&state, &file_handle_inodes, ino, fh)?;
            let request = lock_request(options, owner, pid, start, end, typ)?;
            Ok((inode, handle, request))
        });
        self.dispatch.submit(move || {
            let result = result.and_then(|(inode, handle, request)| {
                backend
                    .getlk(&context, inode, handle, request)
                    .map_err(errno)
            });
            match result {
                Ok(Some(conflict)) => reply_conflict(reply, &conflict),
                Ok(None) => reply.locked(start, end, libc::F_UNLCK, 0),
                Err(error) => reply.error(error),
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn setlk_with_options(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        sleep: bool,
        options: LockOptions,
        reply: ReplyEmpty,
    ) {
        let owner = self.file_lock_owner(lock_owner);
        let unique = req.unique();
        let waiter = sleep.then(|| self.lock_waiter_id(unique));
        let registered_and_interrupted = waiter
            .clone()
            .is_some_and(|waiter| self.pending_locks.register(unique, waiter));
        if registered_and_interrupted {
            self.pending_locks.finish(unique);
            reply.error(libc::EINTR);
            return;
        }

        let backend = self.backend.clone();
        let state = self.state.clone();
        let file_handle_inodes = self.file_handle_inodes.clone();
        let context = Self::context(req, 0);
        let pending = self.pending_locks.clone();
        let result = self.backend_inode(ino).and_then(|inode| {
            let handle = Self::file_handle_for_inode_in(&state, &file_handle_inodes, ino, fh)?;
            let request = lock_request(options, owner, pid, start, end, typ)?;
            Ok((inode, handle, request))
        });
        let reply = Arc::new(Mutex::new(Some(reply)));
        let run = {
            let reply = reply.clone();
            move || {
                let result = result.and_then(|(inode, handle, request)| {
                    backend
                        .setlk(&context, inode, handle, request, waiter)
                        .map_err(errno)
                });
                if sleep {
                    pending.finish(unique);
                }
                if let Some(reply) = reply.lock().unwrap().take() {
                    reply_empty(reply, result);
                }
            }
        };
        if sleep {
            let pending = self.pending_locks.clone();
            let reply = reply.clone();
            self.lock_dispatch.try_submit_or_reject(
                move || -> FuseJob { Box::new(run) },
                move || {
                    pending.finish(unique);
                    if let Some(reply) = reply.lock().unwrap().take() {
                        reply.error(libc::ENOLCK);
                    }
                },
            );
        } else {
            self.dispatch.submit_keyed(fh, run);
        }
    }

    fn interrupt(&mut self, _: &Request<'_>, unique: u64, reply: ReplyEmpty) {
        let Some(waiter) = self.pending_locks.interrupt(unique) else {
            reply.error(libc::EAGAIN);
            return;
        };
        match self.backend.cancel_lock_wait(waiter).map_err(errno) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn mknod(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        rdev: u32,
        reply: ReplyEntry,
    ) {
        let raw_kind = mode & libc::S_IFMT;
        let context = Self::metadata_context(req, umask);
        let permissions = mode & !libc::S_IFMT;
        let result = self
            .backend_inode(parent)
            .and_then(|parent| match raw_kind {
                0 | libc::S_IFREG => {
                    let created = self
                        .backend
                        .create(
                            &context,
                            parent,
                            name,
                            permissions,
                            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY,
                        )
                        .map_err(errno)?;
                    // mknod has no application FD whose close could flush this handle.
                    // Finish the create boundary and release our temporary handle.
                    let flushed = self.backend.flush(&context, created.handle);
                    let released = self.backend.release(&context, created.handle);
                    flushed.map_err(errno)?;
                    released.map_err(errno)?;
                    Ok(created.entry)
                }
                libc::S_IFIFO => self
                    .backend
                    .mknod(&context, parent, name, SpecialFileKind::Fifo, permissions)
                    .map_err(errno),
                libc::S_IFSOCK => self
                    .backend
                    .mknod(&context, parent, name, SpecialFileKind::Socket, permissions)
                    .map_err(errno),
                libc::S_IFBLK => self
                    .backend
                    .mknod(
                        &context,
                        parent,
                        name,
                        SpecialFileKind::BlockDevice {
                            rdev: u64::from(rdev),
                        },
                        permissions,
                    )
                    .map_err(errno),
                libc::S_IFCHR => self
                    .backend
                    .mknod(
                        &context,
                        parent,
                        name,
                        SpecialFileKind::CharDevice {
                            rdev: u64::from(rdev),
                        },
                        permissions,
                    )
                    .map_err(errno),
                _ => Err(libc::EINVAL),
            });
        match result {
            Ok(entry) => {
                let ino = self.state.lock().unwrap().remember_lookup(&entry);
                self.remember_cached_inode(entry.inode, ino);
                reply.entry(&TTL, &file_attr(ino, &entry.attributes), 0);
            }
            Err(error) => reply.error(error),
        }
    }

    fn access(&mut self, _: &Request<'_>, _: u64, _: i32, reply: ReplyEmpty) {
        reply.error(libc::ENOSYS);
    }

    fn getxattr(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        size: u32,
        reply: ReplyXattr,
    ) {
        let Ok(inode) = self.backend_inode(ino) else {
            reply.error(libc::ESTALE);
            return;
        };
        let backend = self.backend.clone();
        let context = Self::metadata_context(req, 0);
        let name = name.to_os_string();
        self.dispatch.submit(move || {
            reply_xattr(reply, size, backend.getxattr(&context, inode, &name));
        });
    }

    fn listxattr(&mut self, req: &Request<'_>, ino: u64, size: u32, reply: ReplyXattr) {
        let Ok(inode) = self.backend_inode(ino) else {
            reply.error(libc::ESTALE);
            return;
        };
        let backend = self.backend.clone();
        let context = Self::metadata_context(req, 0);
        self.dispatch.submit(move || {
            reply_xattr(reply, size, backend.listxattr(&context, inode));
        });
    }

    fn setxattr(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        value: &[u8],
        flags: i32,
        position: u32,
        reply: ReplyEmpty,
    ) {
        if let Err(error) = validate_xattr_set(flags, position) {
            reply.error(error);
            return;
        }
        let Ok(inode) = self.backend_inode(ino) else {
            reply.error(libc::ESTALE);
            return;
        };
        let backend = self.backend.clone();
        let context = Self::metadata_context(req, 0);
        let name = name.to_os_string();
        let value = value.to_vec();
        self.dispatch.submit(move || {
            reply_empty(
                reply,
                backend
                    .setxattr(&context, inode, &name, &value, flags)
                    .map_err(errno),
            );
        });
    }

    fn removexattr(&mut self, req: &Request<'_>, ino: u64, name: &OsStr, reply: ReplyEmpty) {
        let Ok(inode) = self.backend_inode(ino) else {
            reply.error(libc::ESTALE);
            return;
        };
        let backend = self.backend.clone();
        let context = Self::metadata_context(req, 0);
        let name = name.to_os_string();
        self.dispatch.submit(move || {
            reply_empty(
                reply,
                backend.removexattr(&context, inode, &name).map_err(errno),
            );
        });
    }
}

impl AfsFuse {
    fn validate_handle_inode(&self, ino: u64) -> std::result::Result<(), i32> {
        Self::validate_handle_inode_in(&self.state, ino)
    }

    fn validate_handle_inode_in(
        state: &Mutex<FuseState>,
        ino: u64,
    ) -> std::result::Result<(), i32> {
        match state.lock().unwrap().node(ino) {
            Some(FuseNode::Root(_)) | Some(FuseNode::Backend(_)) => Ok(()),
            None => Ok(()),
        }
    }
}

fn add_backend_entries(
    state: &mut FuseState,
    reply: &mut ReplyDirectory,
    parent: u64,
    offset: i64,
    entries: &[DirectoryEntry],
) {
    if offset <= 0 {
        let _ = reply.add(parent, 1, FileType::Directory, ".");
        let _ = reply.add(ROOT_INO, 2, FileType::Directory, "..");
    }
    for entry in entries {
        let fuse_entry = Entry {
            inode: entry.inode,
            attributes: FileAttributes {
                kind: entry.kind,
                size: 0,
                blocks: 0,
                mode: fallback_mode(entry.kind),
                uid: 0,
                gid: 0,
                nlink: 1,
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
            },
        };
        let ino = state.remember_readdir_entry(&fuse_entry);
        let cookie = i64::try_from(entry.next_cookie.max(3)).unwrap_or(i64::MAX);
        if reply.add(ino, cookie, file_type(entry.kind), &entry.name) {
            break;
        }
    }
}

fn backend_cookie(offset: i64) -> u64 {
    if offset <= 2 { 0 } else { offset as u64 }
}

fn file_attr(ino: u64, attributes: &FileAttributes) -> FileAttr {
    FileAttr {
        ino,
        size: attributes.size,
        blocks: attributes.blocks,
        atime: attributes.atime,
        mtime: attributes.mtime,
        ctime: attributes.ctime,
        crtime: UNIX_EPOCH,
        kind: file_type(attributes.kind),
        perm: attributes.mode as u16,
        nlink: attributes.nlink,
        uid: attributes.uid,
        gid: attributes.gid,
        rdev: file_rdev(attributes.kind),
        blksize: 4096,
        flags: 0,
    }
}

fn file_type(kind: FileKind) -> FileType {
    match kind {
        FileKind::Regular => FileType::RegularFile,
        FileKind::Directory => FileType::Directory,
        FileKind::Symlink => FileType::Symlink,
        FileKind::Special(SpecialFileKind::Fifo) => FileType::NamedPipe,
        FileKind::Special(SpecialFileKind::Socket) => FileType::Socket,
        FileKind::Special(SpecialFileKind::BlockDevice { .. }) => FileType::BlockDevice,
        FileKind::Special(SpecialFileKind::CharDevice { .. }) => FileType::CharDevice,
    }
}

fn fallback_mode(kind: FileKind) -> u32 {
    match kind {
        FileKind::Directory => 0o755,
        FileKind::Regular => 0o644,
        FileKind::Symlink => 0o777,
        FileKind::Special(SpecialFileKind::Fifo) => 0o644,
        FileKind::Special(SpecialFileKind::Socket) => 0o777,
        FileKind::Special(SpecialFileKind::BlockDevice { .. })
        | FileKind::Special(SpecialFileKind::CharDevice { .. }) => 0o600,
    }
}

fn file_rdev(kind: FileKind) -> u32 {
    match kind {
        FileKind::Special(SpecialFileKind::BlockDevice { rdev })
        | FileKind::Special(SpecialFileKind::CharDevice { rdev }) => {
            u32::try_from(rdev).unwrap_or(u32::MAX)
        }
        _ => 0,
    }
}

fn checked_offset(offset: i64) -> std::result::Result<u64, i32> {
    u64::try_from(offset).map_err(|_| libc::EINVAL)
}

fn reply_empty(reply: ReplyEmpty, result: std::result::Result<(), i32>) {
    match result {
        Ok(()) => reply.ok(),
        Err(error) => reply.error(error),
    }
}

fn validate_xattr_set(flags: i32, position: u32) -> std::result::Result<(), i32> {
    // Linux has no positional xattrs. Never forward a platform extension or an
    // invalid CREATE+REPLACE combination as a successful upsert.
    if position != 0 || !matches!(flags, 0 | libc::XATTR_CREATE | libc::XATTR_REPLACE) {
        return Err(libc::EINVAL);
    }
    Ok(())
}

fn xattr_response_size(actual: usize, requested: u32) -> std::result::Result<Option<u32>, i32> {
    let actual = u32::try_from(actual).map_err(|_| libc::E2BIG)?;
    if requested == 0 {
        Ok(Some(actual))
    } else if actual <= requested {
        Ok(None)
    } else {
        Err(libc::ERANGE)
    }
}

fn reply_xattr(reply: ReplyXattr, requested: u32, result: Result<Vec<u8>>) {
    match result {
        Ok(data) => match xattr_response_size(data.len(), requested) {
            Ok(Some(size)) => reply.size(size),
            Ok(None) => reply.data(&data),
            Err(error) => reply.error(error),
        },
        Err(error) => reply.error(errno(error)),
    }
}

fn sync_mode(datasync: bool) -> SyncMode {
    if datasync {
        SyncMode::DataOnly
    } else {
        SyncMode::Full
    }
}

fn next_ingress_session_id() -> String {
    let sequence = NEXT_INGRESS_SESSION.fetch_add(1, Ordering::Relaxed);
    format!("fuse-{}-{sequence}", std::process::id())
}

fn lock_kind(options: LockOptions) -> FileLockKind {
    if options.is_flock() {
        FileLockKind::Flock
    } else {
        FileLockKind::Posix
    }
}

fn lock_type(typ: i32) -> std::result::Result<FileLockType, i32> {
    match typ {
        libc::F_RDLCK => Ok(FileLockType::Read),
        libc::F_WRLCK => Ok(FileLockType::Write),
        libc::F_UNLCK => Ok(FileLockType::Unlock),
        _ => Err(libc::EINVAL),
    }
}

fn linux_lock_type(lock_type: FileLockType) -> i32 {
    match lock_type {
        FileLockType::Read => libc::F_RDLCK,
        FileLockType::Write => libc::F_WRLCK,
        FileLockType::Unlock => libc::F_UNLCK,
    }
}

fn lock_request(
    options: LockOptions,
    owner: FileLockOwner,
    pid: u32,
    start: u64,
    end: u64,
    typ: i32,
) -> std::result::Result<LockRequest, i32> {
    let range = FileLockRange { start, end };
    if !range.is_valid() {
        return Err(libc::EINVAL);
    }
    Ok(LockRequest {
        kind: lock_kind(options),
        owner,
        pid,
        range,
        lock_type: lock_type(typ)?,
    })
}

fn reply_conflict(reply: ReplyLock, conflict: &FileLockConflict) {
    reply.locked(
        conflict.range.start,
        conflict.range.end,
        linux_lock_type(conflict.lock_type),
        conflict.pid,
    );
}

fn combine_empty_results(
    first: std::result::Result<(), i32>,
    second: std::result::Result<(), i32>,
) -> std::result::Result<(), i32> {
    match (first, second) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn time_or_now(value: TimeOrNow) -> SystemTime {
    match value {
        TimeOrNow::SpecificTime(time) => time,
        TimeOrNow::Now => SystemTime::now(),
    }
}

fn timestamps_now_only(atime: Option<&TimeOrNow>, mtime: Option<&TimeOrNow>) -> bool {
    let mut saw_timestamp = false;
    for timestamp in [atime, mtime].into_iter().flatten() {
        saw_timestamp = true;
        if !matches!(timestamp, TimeOrNow::Now) {
            return false;
        }
    }
    saw_timestamp
}

fn os_str_bytes(value: &OsStr) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes()
}

fn errno(error: Error) -> i32 {
    afs_logging::error!("FUSE request failed"; "code"=>error.code().raw(), "kind"=>format!("{:?}", error.kind()), "message"=>error.message());
    crate::error::errno(&error)
}

// The kernel still enforces DefaultPermissions. FUSE headers supply uid/gid,
// but not supplementary groups. Resolve groups only for metadata operations,
// and fail closed if the caller disappeared or changed credentials. Data IO
// does not pay for /proc reads. No PID/group cache survives setgroups().
fn request_groups(context: &RequestContext) -> Vec<u32> {
    let path = PathBuf::from(format!("/proc/{}", context.pid));
    let observed = (|| -> Option<Vec<u32>> {
        let before = std::fs::read_to_string(path.join("stat")).ok()?;
        let status = std::fs::read_to_string(path.join("status")).ok()?;
        let second = std::fs::read_to_string(path.join("status")).ok()?;
        let after = std::fs::read_to_string(path.join("stat")).ok()?;
        if process_start(&before)? != process_start(&after)? {
            return None;
        }
        let groups = status_groups(&status, context.uid, context.gid)?;
        if groups != status_groups(&second, context.uid, context.gid)? {
            return None;
        }
        Some(groups)
    })();
    observed.unwrap_or_default()
}

fn process_start(stat: &str) -> Option<&str> {
    // comm may contain spaces and ')'; fields after the last ')' start at 3.
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)
}

fn status_groups(status: &str, uid: u32, gid: u32) -> Option<Vec<u32>> {
    let fields = |name: &str| status.lines().find_map(|line| line.strip_prefix(name));
    let fs_id =
        |name: &str| -> Option<u32> { fields(name)?.split_whitespace().nth(3)?.parse().ok() };
    if fs_id("Uid:")? != uid || fs_id("Gid:")? != gid {
        return None;
    }
    let mut groups = fields("Groups:")?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    groups.sort_unstable();
    groups.dedup();
    Some(groups)
}

#[cfg(test)]
mod dispatch_tests {
    #[test]
    fn metadata_groups_require_matching_kernel_identity() {
        let status = "Uid:\t1000 1000 1000 1000\nGid:\t100 100 100 100\nGroups:\t200 100 200\n";
        assert_eq!(
            super::status_groups(status, 1000, 100),
            Some(vec![100, 200])
        );
        assert_eq!(super::status_groups(status, 1001, 100), None);
        assert_eq!(super::status_groups(status, 1000, 101), None);
        assert_eq!(super::status_groups("Uid: 1000\n", 1000, 100), None);
    }

    #[test]
    fn process_start_ignores_spaces_and_parentheses_in_command_name() {
        let stat = format!(
            "123 (a name ) b) {}",
            (3..=22)
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(super::process_start(&stat), Some("22"));
        assert_eq!(super::process_start("123 (dead) Z"), None);
    }

    use super::FuseDispatch;
    use std::{
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

    #[test]
    fn xattr_size_query_short_buffer_and_empty_value_are_distinct() {
        assert_eq!(super::xattr_response_size(4, 0), Ok(Some(4)));
        assert_eq!(super::xattr_response_size(4, 3), Err(libc::ERANGE));
        assert_eq!(super::xattr_response_size(4, 4), Ok(None));
        assert_eq!(super::xattr_response_size(4, 64), Ok(None));
        assert_eq!(super::xattr_response_size(0, 0), Ok(Some(0)));
        assert_eq!(super::xattr_response_size(0, 1), Ok(None));
        assert_eq!(super::xattr_response_size(usize::MAX, 0), Err(libc::E2BIG));
    }

    #[test]
    fn xattr_create_replace_and_invalid_flags_keep_linux_contract() {
        for flags in [0, libc::XATTR_CREATE, libc::XATTR_REPLACE] {
            assert_eq!(super::validate_xattr_set(flags, 0), Ok(()));
        }
        for flags in [libc::XATTR_CREATE | libc::XATTR_REPLACE, 4, -1] {
            assert_eq!(super::validate_xattr_set(flags, 0), Err(libc::EINVAL));
        }
        assert_eq!(super::validate_xattr_set(0, 1), Err(libc::EINVAL));
    }

    #[test]
    fn sparse_file_attributes_keep_allocated_blocks_separate_from_eof() {
        let attributes = super::FileAttributes {
            kind: super::FileKind::Regular,
            size: 8 * 1024 * 1024 * 1024,
            blocks: 8,
            mode: 0o600,
            uid: 1000,
            gid: 1000,
            nlink: 1,
            atime: super::UNIX_EPOCH,
            mtime: super::UNIX_EPOCH,
            ctime: super::UNIX_EPOCH,
        };
        let attr = super::file_attr(2, &attributes);
        assert_eq!(attr.size, attributes.size);
        assert_eq!(attr.blocks, 8);
    }

    #[test]
    fn dropping_fuse_dispatch_waits_for_admitted_jobs() {
        let dispatch = FuseDispatch::new(2);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (second_tx, second_rx) = mpsc::channel();
        dispatch.submit_keyed(7, move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        dispatch.submit_keyed(7, move || {
            second_tx.send(()).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(dispatch);
            dropped_tx.send(()).unwrap();
        });
        let early = dropped_rx.recv_timeout(Duration::from_millis(100));
        release_tx.send(()).unwrap();
        second_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        dropper.join().unwrap();
        assert!(
            early.is_err(),
            "dispatch teardown returned while accepted write remained active"
        );
        dropped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn independent_fuse_jobs_overlap_before_either_finishes() {
        let dispatch = FuseDispatch::new(2);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        for _ in 0..2 {
            let started_tx = started_tx.clone();
            let release_rx = release_rx.clone();
            dispatch.submit(move || {
                started_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
            });
        }
        let first = started_rx.recv_timeout(Duration::from_secs(1));
        let second = started_rx.recv_timeout(Duration::from_secs(1));
        release_tx.send(()).unwrap();
        release_tx.send(()).unwrap();
        assert!(
            first.is_ok() && second.is_ok(),
            "jobs remained serial at FUSE ingress"
        );
    }

    #[test]
    fn same_handle_callbacks_keep_order_while_other_handles_progress() {
        let dispatch = FuseDispatch::new(2);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_tx = started_tx.clone();
        dispatch.submit_keyed(11, move || {
            first_tx.send("first").unwrap();
            release_rx.recv().unwrap();
        });
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "first"
        );
        let second_tx = started_tx.clone();
        dispatch.submit_keyed(11, move || {
            second_tx.send("second").unwrap();
        });
        dispatch.submit_keyed(12, move || {
            started_tx.send("other").unwrap();
        });
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "other"
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "second"
        );
    }

    #[test]
    fn queued_same_handle_attribute_callbacks_do_not_block_unrelated_handles() {
        let dispatch = FuseDispatch::new(2);
        let (events_tx, events_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let first_tx = events_tx.clone();
        dispatch.submit_keyed(21, move || {
            first_tx.send("write:start").unwrap();
            release_rx.recv().unwrap();
            first_tx.send("write:finish").unwrap();
        });
        assert_eq!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "write:start"
        );

        let setattr_tx = events_tx.clone();
        let before_setattr_enqueue = std::time::Instant::now();
        dispatch.submit_keyed(21, move || {
            setattr_tx.send("setattr").unwrap();
        });
        assert!(
            before_setattr_enqueue.elapsed() < Duration::from_millis(100),
            "same-handle setattr enqueue blocked the FUSE receive path"
        );

        let getattr_tx = events_tx.clone();
        dispatch.submit_keyed(22, move || {
            getattr_tx.send("getattr:other-handle").unwrap();
        });
        assert_eq!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "getattr:other-handle",
            "queued setattr for one fh blocked another fh"
        );

        release_tx.send(()).unwrap();
        assert_eq!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "write:finish"
        );
        assert_eq!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "setattr"
        );
    }

    #[test]
    fn lock_request_conversion_preserves_flock_namespace_and_unlock() {
        let owner = super::FileLockOwner {
            ingress_session_id: "mount-a".to_owned(),
            kernel_owner: 55,
        };
        let request = super::lock_request(
            fuser::LockOptions {
                flags: fuser::consts::FUSE_LK_FLOCK,
            },
            owner.clone(),
            123,
            10,
            99,
            libc::F_UNLCK,
        )
        .unwrap();

        assert_eq!(request.kind, super::FileLockKind::Flock);
        assert_eq!(request.owner, owner);
        assert_eq!(request.pid, 123);
        assert_eq!(request.range, super::FileLockRange { start: 10, end: 99 });
        assert_eq!(request.lock_type, super::FileLockType::Unlock);
        assert_eq!(super::linux_lock_type(request.lock_type), libc::F_UNLCK);
        assert_eq!(
            super::lock_request(
                fuser::LockOptions::default(),
                request.owner,
                123,
                100,
                99,
                libc::F_WRLCK,
            )
            .unwrap_err(),
            libc::EINVAL
        );
    }

    #[test]
    fn pending_lock_registry_remembers_interrupt_before_worker_registration() {
        let registry = super::PendingLockRegistry::default();
        assert_eq!(registry.interrupt(88), None);
        let waiter = super::LockWaiterId {
            ingress_session_id: "mount-a".to_owned(),
            request_id: 88,
        };
        assert!(registry.register(88, waiter.clone()));
        assert_eq!(registry.interrupt(88), Some(waiter));
        registry.finish(88);
        assert_eq!(registry.interrupt(88), None);
    }

    #[test]
    fn blocking_lock_pool_rejects_when_queue_is_full_without_running_inline() {
        let dispatch = FuseDispatch::new(1);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut release_rx = Some(release_rx);

        for index in 0..8 {
            let started_tx = started_tx.clone();
            if index == 0 {
                let release_rx = release_rx.take().unwrap();
                dispatch.submit(move || {
                    started_tx.send(index).unwrap();
                    release_rx.recv().unwrap();
                });
            } else {
                dispatch.submit(move || {
                    started_tx.send(index).unwrap();
                });
            }
        }
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 0);

        let (rejected_tx, rejected_rx) = mpsc::channel();
        dispatch.try_submit_or_reject(
            || -> super::FuseJob {
                Box::new(move || {
                    panic!("full blocking lock pool ran a rejected job inline");
                })
            },
            move || rejected_tx.send(()).unwrap(),
        );
        rejected_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        release_tx.send(()).unwrap();
        for expected in 1..8 {
            assert_eq!(
                started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
                expected
            );
        }
    }
}

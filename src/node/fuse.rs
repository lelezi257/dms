//! 统一 POSIX/FUSE 入口。
//!
//! 接收内核回调，转换到 VFS `Backend` 接口并映射 errno/回复；管理 FUSE 会话内的
//! inode、目录项和打开句柄。两种 namespace 通过同一入口分派，FUSE inode 编号
//! 不能当作后端持久文件身份。OwnerFs 的独用 Home 根可短暂缓存；首次远端
//! 访问先通过 FUSE notifier 失效本机缓存，再由 Home 执行该请求。

mod state;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use afs_error::{Error, Result};
use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, KernelConfig, MountOption, ReplyAttr,
    ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request,
    TimeOrNow, consts,
};

#[cfg(feature = "ownerfs")]
use crate::node::vfs::ownerfs::OwnerFs;
use crate::node::vfs::{
    Backend, Namespace, Vfs,
    types::{
        AttributeChange, BackendInode, DirectoryEntry, Entry, FileAttributes, FileKind,
        RenameFlags, RequestContext, SyncMode,
    },
};

use self::state::{FuseNode, FuseState, ROOT_INO, namespace_ino};

const TTL: Duration = Duration::ZERO;
// Keep name-to-inode mappings briefly on P2P mounts without caching remote
// attributes. This separates redundant path lookups from close-to-open data
// freshness; the patched fuser reply supports independent TTLs.
const OWNER_ENTRY_TTL: Duration = Duration::from_secs(1);
const DIRECT_IO: u32 = consts::FOPEN_DIRECT_IO;

/// 建立真正的内核 FUSE 挂载，由 fuser 后台会话收取 POSIX 回调。
/// 返回值由 Node 持有；释放会话时卸载。没有 bind 子挂载，也不绕过 FUSE 访问后端。
pub fn mount(vfs: Arc<Vfs>, path: &Path) -> Result<BackgroundSession> {
    reject_existing_mount(path)?;
    let fs = AfsFuse::new(FuseBackends::Vfs(vfs));
    fuser::spawn_mount2(
        fs,
        path,
        &[MountOption::FSName("afs".into()), MountOption::NoAtime],
    )
    .map_err(Error::from)
}

/// Production OwnerFs mount with the same private-cache policy as the HomeFs
/// fast path. The concrete hook stays here; it does not expand the common VFS
/// Backend interface or affect BlobFs and test mounts.
#[cfg(feature = "ownerfs")]
pub fn mount_with_ownerfs_cache(
    vfs: Arc<Vfs>,
    ownerfs: Arc<OwnerFs>,
    path: &Path,
) -> Result<BackgroundSession> {
    reject_existing_mount(path)?;
    let mut fs = AfsFuse::new(FuseBackends::Vfs(vfs));
    fs.ownerfs = Some(ownerfs.clone());
    let session = fuser::spawn_mount2(
        fs,
        path,
        &[MountOption::FSName("afs".into()), MountOption::NoAtime],
    )
    .map_err(Error::from)?;
    ownerfs.register_fuse_notifier(session.notifier());
    Ok(session)
}

#[doc(hidden)]
pub fn mount_test_backend(
    namespace: Namespace,
    backend: Arc<dyn Backend>,
    path: &Path,
) -> Result<BackgroundSession> {
    reject_existing_mount(path)?;
    let fs = AfsFuse::new(FuseBackends::Single { namespace, backend });
    fuser::spawn_mount2(
        fs,
        path,
        &[MountOption::FSName("afs-test".into()), MountOption::NoAtime],
    )
    .map_err(Error::from)
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
}

impl Drop for FuseDispatch {
    fn drop(&mut self) {
        let (queue, wake) = &*self.shared;
        queue.lock().unwrap().closed = true;
        wake.notify_all();
    }
}

pub struct AfsFuse {
    backends: FuseBackends,
    state: Arc<Mutex<FuseState>>,
    dispatch: FuseDispatch,
    #[cfg(feature = "ownerfs")]
    ownerfs: Option<Arc<OwnerFs>>,
}

impl AfsFuse {
    #[must_use]
    fn new(backends: FuseBackends) -> Self {
        let state = Arc::new(Mutex::new(FuseState::new(backends.namespaces())));
        Self {
            backends,
            state,
            dispatch: FuseDispatch::new(8),
            #[cfg(feature = "ownerfs")]
            ownerfs: None,
        }
    }

    fn remember_cached_inode(&self, inode: BackendInode, ino: u64) {
        #[cfg(feature = "ownerfs")]
        if inode.namespace == Namespace::OwnerFs
            && let Some(ownerfs) = &self.ownerfs
        {
            ownerfs.remember_fuse_inode(ino);
        }
        #[cfg(not(feature = "ownerfs"))]
        let _ = (inode, ino);
    }

    fn with_cache_policy<T>(
        &self,
        inode: BackendInode,
        reply: impl FnOnce(Duration, bool) -> T,
    ) -> T {
        #[cfg(feature = "ownerfs")]
        if inode.namespace == Namespace::OwnerFs
            && let Some(ownerfs) = &self.ownerfs
        {
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
        }
    }

    fn backend(&self, namespace: Namespace) -> std::result::Result<&dyn Backend, i32> {
        self.backends.backend(namespace).ok_or(libc::ENOENT)
    }

    fn backend_inode(&self, ino: u64) -> std::result::Result<BackendInode, i32> {
        self.state
            .lock()
            .unwrap()
            .backend_inode(ino)
            .ok_or(libc::ESTALE)
    }

    fn lookup_root_child(&self, name: &OsStr) -> std::result::Result<FileAttr, i32> {
        let Some(name) = name.to_str() else {
            return Err(libc::EINVAL);
        };
        let namespace = Namespace::parse(name).map_err(errno)?;
        let ino = namespace_ino(namespace);
        let node = self.state.lock().unwrap().node(ino);
        match node {
            Some(FuseNode::NamespaceRoot(_)) => Ok(dir_attr(ino)),
            _ => Err(libc::ENOENT),
        }
    }
}

#[derive(Clone)]
enum FuseBackends {
    Vfs(Arc<Vfs>),
    Single {
        namespace: Namespace,
        backend: Arc<dyn Backend>,
    },
}

impl FuseBackends {
    fn namespaces(&self) -> Vec<Namespace> {
        match self {
            Self::Vfs(vfs) => vfs.namespaces(),
            Self::Single { namespace, .. } => vec![*namespace],
        }
    }

    fn backend(&self, namespace: Namespace) -> Option<&dyn Backend> {
        match self {
            Self::Vfs(vfs) => vfs.backend(namespace),
            Self::Single {
                namespace: enabled,
                backend,
            } if *enabled == namespace => Some(backend.as_ref()),
            Self::Single { .. } => None,
        }
    }
}

impl Filesystem for AfsFuse {
    fn init(&mut self, _: &Request<'_>, config: &mut KernelConfig) -> std::result::Result<(), i32> {
        let _ = config.set_max_write(1024 * 1024);
        // Linux otherwise serializes LOOKUPs in one directory even when our
        // FUSE receive thread dispatches them to independent workers.
        let _ = config.add_capabilities(fuser::consts::FUSE_PARALLEL_DIROPS);
        Ok(())
    }

    fn lookup(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent == ROOT_INO {
            match self.lookup_root_child(name) {
                Ok(attr) => reply.entry(&OWNER_ENTRY_TTL, &attr, 0),
                Err(error) => reply.error(error),
            }
            return;
        }
        let Ok(parent_inode) = self.backend_inode(parent) else {
            reply.error(libc::ESTALE);
            return;
        };
        // A local Home directory lookup has no network wait. Keep it on the
        // FUSE receive thread; remote lookups still need worker concurrency.
        #[cfg(feature = "ownerfs")]
        let inline_local_lookup = parent_inode.namespace == Namespace::OwnerFs
            && self
                .ownerfs
                .as_ref()
                .is_some_and(|ownerfs| ownerfs.is_local_inode(parent_inode));
        #[cfg(not(feature = "ownerfs"))]
        let inline_local_lookup = false;
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let name = name.to_os_string();
        #[cfg(feature = "ownerfs")]
        let ownerfs = self.ownerfs.clone();
        let run = move || {
            let result = backends
                .backend(parent_inode.namespace)
                .ok_or(libc::ENOENT)
                .and_then(|backend| backend.lookup(&context, parent_inode, &name).map_err(errno));
            match result {
                Ok(entry) => {
                    let ino = state.lock().unwrap().remember_lookup(&entry);
                    #[cfg(feature = "ownerfs")]
                    if entry.inode.namespace == Namespace::OwnerFs
                        && let Some(ownerfs) = &ownerfs
                    {
                        ownerfs.remember_fuse_inode(ino);
                        ownerfs.with_fuse_cache_policy(entry.inode, |ttl, private| {
                            if !private {
                                reply.entry_with_ttls(
                                    &OWNER_ENTRY_TTL,
                                    &ttl,
                                    &file_attr(ino, &entry.attributes),
                                    0,
                                );
                            } else {
                                reply.entry(&ttl, &file_attr(ino, &entry.attributes), 0);
                            }
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
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        #[cfg(feature = "ownerfs")]
        let ownerfs = self.ownerfs.clone();
        let run = move || {
            let node = state.lock().unwrap().node(ino);
            let result = match node {
                Some(FuseNode::Root) => Ok(dir_attr(ROOT_INO)),
                Some(FuseNode::NamespaceRoot(_)) => Ok(dir_attr(ino)),
                Some(FuseNode::Backend(inode)) => {
                    let handle = fh.and_then(|fh| state.lock().unwrap().file_handle(fh));
                    handle
                        .map_or(Ok(()), |handle| {
                            AfsFuse::validate_handle_inode_namespace_in(
                                &state,
                                ino,
                                handle.namespace,
                            )
                        })
                        .and_then(|()| {
                            backends
                                .backend(inode.namespace)
                                .ok_or(libc::ENOENT)
                                .and_then(|backend| {
                                    backend
                                        .getattr(
                                            &context,
                                            inode,
                                            handle.map(|handle| handle.handle),
                                        )
                                        .map(|attributes| file_attr(ino, &attributes))
                                        .map_err(errno)
                                })
                        })
                }
                None => Err(libc::ESTALE),
            };
            let backend_inode = state.lock().unwrap().backend_inode(ino);
            match result {
                Ok(attr) => match backend_inode {
                    Some(inode) => {
                        #[cfg(feature = "ownerfs")]
                        if inode.namespace == Namespace::OwnerFs
                            && let Some(ownerfs) = &ownerfs
                        {
                            ownerfs.with_fuse_cache_policy(inode, |ttl, _| reply.attr(&ttl, &attr));
                            return;
                        }
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
        reply: ReplyAttr,
    ) {
        let inline_local_read = fh.is_some_and(|fh| {
            self.state
                .lock()
                .unwrap()
                .file_handle(fh)
                .is_some_and(|file| file.inline_local_read)
        });
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let change = AttributeChange {
            size,
            mode,
            uid,
            gid,
            atime: atime.map(time_or_now),
            mtime: mtime.map(time_or_now),
        };
        let run = move || {
            let inode = state.lock().unwrap().backend_inode(ino);
            let result = inode.ok_or(libc::ESTALE).and_then(|inode| {
                let handle = fh.and_then(|fh| state.lock().unwrap().file_handle(fh));
                if let Some(handle) = handle {
                    AfsFuse::validate_handle_inode_namespace_in(&state, ino, handle.namespace)?;
                }
                backends
                    .backend(inode.namespace)
                    .ok_or(libc::ENOENT)
                    .and_then(|backend| {
                        backend
                            .setattr(&context, inode, handle.map(|handle| handle.handle), &change)
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
            self.backend(inode.namespace).and_then(|backend| {
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
            self.backend(parent_inode.namespace).and_then(|backend| {
                backend
                    .mkdir(&Self::context(req, umask), parent_inode, name, mode)
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

    fn unlink(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let result = self.backend_inode(parent).and_then(|parent_inode| {
            self.backend(parent_inode.namespace).and_then(|backend| {
                backend
                    .unlink(&Self::context(req, 0), parent_inode, name)
                    .map_err(errno)
            })
        });
        reply_empty(reply, result);
    }

    fn rmdir(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let result = self.backend_inode(parent).and_then(|parent_inode| {
            self.backend(parent_inode.namespace).and_then(|backend| {
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
            self.backend(parent_inode.namespace).and_then(|backend| {
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
        let result = self.backend_inode(parent).and_then(|from_parent| {
            let to_parent = self.backend_inode(newparent)?;
            if from_parent.namespace != to_parent.namespace {
                return Err(libc::EXDEV);
            }
            self.backend(from_parent.namespace).and_then(|backend| {
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
            if inode.namespace != parent.namespace {
                return Err(libc::EXDEV);
            }
            self.backend(inode.namespace).and_then(|backend| {
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

    fn open(&mut self, req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let Ok(inode) = self.backend_inode(ino) else {
            reply.error(libc::ESTALE);
            return;
        };
        // The kernel cannot submit operations on this fh until open replies.
        // This lets independent writable opens overlap with read-only ones;
        // O_PATH remains on the original path because it has no I/O handle.
        if flags & libc::O_PATH != 0 {
            let result = self.backend(inode.namespace).and_then(|backend| {
                backend
                    .open(&Self::context(req, 0), inode, flags)
                    .map_err(errno)
            });
            match result {
                Ok(handle) => {
                    let fh = self
                        .state
                        .lock()
                        .unwrap()
                        .insert_file_handle(inode.namespace, handle);
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
            && inode.namespace == Namespace::OwnerFs
            && self
                .ownerfs
                .as_ref()
                .is_some_and(|ownerfs| ownerfs.is_local_inode(inode));
        #[cfg(not(feature = "ownerfs"))]
        let inline_local_read = false;
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        #[cfg(feature = "ownerfs")]
        let ownerfs = self.ownerfs.clone();
        let run = move || {
            let result = backends
                .backend(inode.namespace)
                .ok_or(libc::ENOENT)
                .and_then(|backend| backend.open(&context, inode, flags).map_err(errno));
            match result {
                Ok(handle) => {
                    let fh = state.lock().unwrap().insert_file_handle_with_policy(
                        inode.namespace,
                        handle,
                        inline_local_read,
                    );
                    #[cfg(feature = "ownerfs")]
                    if inode.namespace == Namespace::OwnerFs
                        && let Some(ownerfs) = &ownerfs
                    {
                        ownerfs.with_fuse_cache_policy(inode, |_, private| {
                            reply.opened(fh, if private { 0 } else { DIRECT_IO });
                        });
                        return;
                    }
                    reply.opened(fh, DIRECT_IO);
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
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let run = move || {
            let result = checked_offset(offset).and_then(|offset| {
                let file = state.lock().unwrap().file_handle(fh).ok_or(libc::ESTALE)?;
                AfsFuse::validate_handle_inode_namespace_in(&state, ino, file.namespace)?;
                let mut out = vec![0; size as usize];
                backends
                    .backend(file.namespace)
                    .ok_or(libc::ENOENT)
                    .and_then(|backend| {
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
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let data = data.to_vec();
        self.dispatch.submit_keyed(fh, move || {
            let result = checked_offset(offset).and_then(|offset| {
                let file = state.lock().unwrap().file_handle(fh).ok_or(libc::ESTALE)?;
                AfsFuse::validate_handle_inode_namespace_in(&state, ino, file.namespace)?;
                backends
                    .backend(file.namespace)
                    .ok_or(libc::ENOENT)
                    .and_then(|backend| {
                        backend
                            .write(&context, file.handle, offset, &data)
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

    fn flush(&mut self, req: &Request<'_>, ino: u64, fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        let inline_local_read = self
            .state
            .lock()
            .unwrap()
            .file_handle(fh)
            .is_some_and(|file| file.inline_local_read);
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let run = move || {
            let file = state.lock().unwrap().file_handle(fh);
            let result = file.ok_or(libc::ESTALE).and_then(|file| {
                AfsFuse::validate_handle_inode_namespace_in(&state, ino, file.namespace)?;
                backends
                    .backend(file.namespace)
                    .ok_or(libc::ENOENT)
                    .and_then(|backend| backend.flush(&context, file.handle).map_err(errno))
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
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let inline_local_read = self
            .state
            .lock()
            .unwrap()
            .file_handle(fh)
            .is_some_and(|file| file.inline_local_read);
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let run = move || {
            let file = state.lock().unwrap().remove_file_handle(fh);
            let result = file.ok_or(libc::ESTALE).and_then(|file| {
                AfsFuse::validate_handle_inode_namespace_in(&state, ino, file.namespace)?;
                backends
                    .backend(file.namespace)
                    .ok_or(libc::ENOENT)
                    .and_then(|backend| backend.release(&context, file.handle).map_err(errno))
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
        let backends = self.backends.clone();
        let state = self.state.clone();
        let context = Self::context(req, 0);
        let run = move || {
            let file = state.lock().unwrap().file_handle(fh);
            let result = file.ok_or(libc::ESTALE).and_then(|file| {
                AfsFuse::validate_handle_inode_namespace_in(&state, ino, file.namespace)?;
                backends
                    .backend(file.namespace)
                    .ok_or(libc::ENOENT)
                    .and_then(|backend| {
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
            Some(FuseNode::Root) => reply.opened(0, 0),
            Some(FuseNode::NamespaceRoot(_)) | Some(FuseNode::Backend(_)) => {
                let result = self.backend_inode(ino).and_then(|inode| {
                    self.backend(inode.namespace).and_then(|backend| {
                        backend
                            .opendir(&Self::context(req, 0), inode)
                            .map(|handle| (inode.namespace, handle))
                            .map_err(errno)
                    })
                });
                match result {
                    Ok((namespace, handle)) => {
                        let fh = self
                            .state
                            .lock()
                            .unwrap()
                            .insert_directory_handle(namespace, handle);
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
            Some(FuseNode::Root) => {
                add_virtual_root_entries(
                    &mut reply,
                    offset,
                    self.backends
                        .namespaces()
                        .into_iter()
                        .map(|namespace| (namespace_ino(namespace), namespace.as_str())),
                );
                reply.ok();
            }
            Some(FuseNode::NamespaceRoot(_)) | Some(FuseNode::Backend(_)) => {
                let directory = self.state.lock().unwrap().directory_handle(fh);
                let result = directory.ok_or(libc::ESTALE).and_then(|directory| {
                    self.validate_handle_inode_namespace(ino, directory.namespace)?;
                    let cookie = backend_cookie(offset);
                    self.backend(directory.namespace).and_then(|backend| {
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
            self.validate_handle_inode_namespace(ino, directory.namespace)?;
            self.backend(directory.namespace).and_then(|backend| {
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
            self.validate_handle_inode_namespace(ino, directory.namespace)?;
            self.backend(directory.namespace).and_then(|backend| {
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
        reply: ReplyCreate,
    ) {
        afs_logging::info!("fuse.create"; "parent" => parent, "name" => name.to_string_lossy().into_owned());
        let result = self.backend_inode(parent).and_then(|parent_inode| {
            self.backend(parent_inode.namespace).and_then(|backend| {
                backend
                    .create(&Self::context(req, umask), parent_inode, name, mode, flags)
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
                    .insert_file_handle(created.entry.inode.namespace, created.handle);
                self.with_cache_policy(created.entry.inode, |ttl, private| {
                    reply.created(
                        &ttl,
                        &file_attr(ino, &created.entry.attributes),
                        0,
                        fh,
                        if private { 0 } else { DIRECT_IO },
                    )
                });
            }
            Err(error) => reply.error(error),
        }
    }

    fn mknod(
        &mut self,
        _: &Request<'_>,
        _: u64,
        _: &OsStr,
        _: u32,
        _: u32,
        _: u32,
        reply: ReplyEntry,
    ) {
        reply.error(libc::ENOSYS);
    }

    fn access(&mut self, _: &Request<'_>, _: u64, _: i32, reply: ReplyEmpty) {
        reply.error(libc::ENOSYS);
    }
}

impl AfsFuse {
    fn validate_handle_inode_namespace(
        &self,
        ino: u64,
        handle_namespace: Namespace,
    ) -> std::result::Result<(), i32> {
        Self::validate_handle_inode_namespace_in(&self.state, ino, handle_namespace)
    }

    fn validate_handle_inode_namespace_in(
        state: &Mutex<FuseState>,
        ino: u64,
        handle_namespace: Namespace,
    ) -> std::result::Result<(), i32> {
        let node = state.lock().unwrap().node(ino);
        match node {
            Some(FuseNode::NamespaceRoot(namespace))
            | Some(FuseNode::Backend(BackendInode { namespace, .. }))
                if namespace == handle_namespace =>
            {
                Ok(())
            }
            Some(FuseNode::Root) => Err(libc::EISDIR),
            Some(_) => Err(libc::ESTALE),
            None => Ok(()),
        }
    }
}

fn add_virtual_root_entries<'a>(
    reply: &mut ReplyDirectory,
    offset: i64,
    children: impl Iterator<Item = (u64, &'a str)>,
) {
    let mut entries = vec![
        (ROOT_INO, FileType::Directory, "."),
        (ROOT_INO, FileType::Directory, ".."),
    ];
    entries.extend(children.map(|(ino, name)| (ino, FileType::Directory, name)));

    for (index, (ino, kind, name)) in entries.into_iter().enumerate().skip(offset.max(0) as usize) {
        if reply.add(ino, (index + 1) as i64, kind, name) {
            break;
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
                mode: match entry.kind {
                    FileKind::Directory => 0o755,
                    FileKind::Regular => 0o644,
                    FileKind::Symlink => 0o777,
                },
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
        blocks: attributes.size.div_ceil(512),
        atime: attributes.atime,
        mtime: attributes.mtime,
        ctime: attributes.ctime,
        crtime: UNIX_EPOCH,
        kind: file_type(attributes.kind),
        perm: attributes.mode as u16,
        nlink: attributes.nlink,
        uid: attributes.uid,
        gid: attributes.gid,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

fn file_type(kind: FileKind) -> FileType {
    match kind {
        FileKind::Regular => FileType::RegularFile,
        FileKind::Directory => FileType::Directory,
        FileKind::Symlink => FileType::Symlink,
    }
}

fn dir_attr(ino: u64) -> FileAttr {
    FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: 0,
        gid: 0,
        rdev: 0,
        blksize: 4096,
        flags: 0,
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

fn sync_mode(datasync: bool) -> SyncMode {
    if datasync {
        SyncMode::DataOnly
    } else {
        SyncMode::Full
    }
}

fn time_or_now(value: TimeOrNow) -> SystemTime {
    match value {
        TimeOrNow::SpecificTime(time) => time,
        TimeOrNow::Now => SystemTime::now(),
    }
}

fn os_str_bytes(value: &OsStr) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes()
}

fn errno(error: Error) -> i32 {
    afs_logging::error!("FUSE request failed"; "code"=>error.code().raw(), "kind"=>format!("{:?}", error.kind()), "message"=>error.message());
    crate::error::errno(&error)
}

#[cfg(test)]
mod dispatch_tests {
    use super::FuseDispatch;
    use std::{
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

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
}

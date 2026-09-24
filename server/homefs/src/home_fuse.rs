use crate::{
    center,
    p2p_rpc::{Client, SetAttrSpec, TimeSpec},
};
use fuser::{
    FileAttr, FileType, Filesystem, KernelConfig, Notifier, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request, TimeOrNow,
};
use std::{
    collections::{HashMap, HashSet},
    ffi::{CString, OsStr},
    fs::{self, File},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileExt, MetadataExt},
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ROOT_INO: u64 = 1;
const TTL: Duration = Duration::from_secs(0);
const P2P_ENTRY_TTL: Duration = Duration::from_secs(1);
const PRIVATE_ATTR_TTL: Duration = Duration::from_secs(1);

pub(crate) struct PrivateAttrCache {
    shared_roots: Mutex<HashSet<String>>,
    next_ino: AtomicU64,
}

impl PrivateAttrCache {
    pub(crate) fn new() -> Self {
        Self {
            shared_roots: Mutex::new(HashSet::new()),
            next_ino: AtomicU64::new(ROOT_INO + 1),
        }
    }

    pub(crate) fn enter_shared_path(&self, path: &str, notifier: &Notifier) -> Result<(), i32> {
        if path == "/" {
            return Ok(());
        }
        let root = HomeFs::root_name(path)?;
        let mut shared = self.shared_roots.lock().unwrap();
        if shared.contains(root) {
            return Ok(());
        }
        // Hold the lock across invalidation. A local FUSE reply cannot grant
        // another private TTL between the last notification and the mode flip.
        for ino in ROOT_INO + 1..self.next_ino.load(Ordering::Acquire) {
            notifier.inval_inode(ino, 0, 0).map_err(|_| libc::EIO)?;
        }
        shared.insert(root.to_owned());
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendMode {
    Nfs,
    P2p,
}

impl BackendMode {
    pub fn parse(value: &str) -> io::Result<Self> {
        match value {
            "nfs" => Ok(Self::Nfs),
            "p2p" => Ok(Self::P2p),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported home backend {other:?}, expected nfs or p2p"),
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Location {
    Local,
    RemoteNfs(String),
    RemoteP2p(String),
}

enum Handle {
    File {
        file: File,
        needs_flush: bool,
    },
    P2p {
        owner: String,
        remote: u64,
        read_only: bool,
        needs_flush: bool,
        prefetched: Option<Vec<u8>>,
    },
}

impl Handle {
    fn needs_flush(&self) -> bool {
        match self {
            Self::File { needs_flush, .. } | Self::P2p { needs_flush, .. } => *needs_flush,
        }
    }

    fn set_needs_flush(&mut self, value: bool) {
        match self {
            Self::File { needs_flush, .. } | Self::P2p { needs_flush, .. } => *needs_flush = value,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RootLookup {
    Missing,
    Active(String),
    Pending(String),
    Deleting(String),
    Tombstone,
}

pub struct HomeFs {
    id: String,
    center: String,
    local: PathBuf,
    peers: PathBuf,
    mounted: Arc<RwLock<HashSet<String>>>,
    private_cache: Arc<PrivateAttrCache>,
    backend: BackendMode,
    token: String,
    owners: HashMap<String, String>,
    p2p_endpoints: HashMap<String, String>,
    p2p_clients: HashMap<String, Client>,
    paths: HashMap<u64, String>,
    ids: HashMap<String, u64>,
    backing_ids: HashMap<String, (String, u64, u64)>,
    next_ino: u64,
    handles: HashMap<u64, Handle>,
    handle_inodes: HashMap<u64, u64>,
    next_handle: u64,
}

impl HomeFs {
    pub fn new(
        id: String,
        center: String,
        local: PathBuf,
        peers: PathBuf,
        mounted: Arc<RwLock<HashSet<String>>>,
        backend: BackendMode,
        token: String,
    ) -> Self {
        Self {
            id,
            center,
            local,
            peers,
            mounted,
            private_cache: Arc::new(PrivateAttrCache::new()),
            backend,
            token,
            owners: HashMap::new(),
            p2p_endpoints: HashMap::new(),
            p2p_clients: HashMap::new(),
            paths: HashMap::from([(ROOT_INO, "/".to_owned())]),
            ids: HashMap::from([("/".to_owned(), ROOT_INO)]),
            backing_ids: HashMap::new(),
            next_ino: ROOT_INO + 1,
            handles: HashMap::new(),
            handle_inodes: HashMap::new(),
            next_handle: 1,
        }
    }

    pub(crate) fn with_private_cache(mut self, private_cache: Arc<PrivateAttrCache>) -> Self {
        self.private_cache = private_cache;
        self
    }

    fn path(&self, ino: u64) -> Result<&str, i32> {
        self.paths.get(&ino).map(String::as_str).ok_or(libc::ESTALE)
    }

    fn child(&self, parent: u64, name: &OsStr) -> Result<String, i32> {
        let name = name.to_str().ok_or(libc::EINVAL)?;
        if !center::safe(name) {
            return Err(libc::EINVAL);
        }
        Ok(join_path(self.path(parent)?, name))
    }

    fn ino(&mut self, path: &str) -> u64 {
        if let Some(ino) = self.ids.get(path) {
            return *ino;
        }
        let ino = self.next_ino;
        self.next_ino += 1;
        self.private_cache
            .next_ino
            .store(self.next_ino, Ordering::Release);
        self.ids.insert(path.to_owned(), ino);
        self.paths.insert(ino, path.to_owned());
        ino
    }

    fn ino_for_backing(&mut self, path: &str, owner: &str, dev: u64, ino: u64) -> u64 {
        let identity = (owner.to_owned(), dev, ino);
        if self
            .backing_ids
            .get(path)
            .is_some_and(|old| old != &identity)
        {
            // Keep the old inode alive for an open FD, but give a replacement
            // at the same pathname a different FUSE inode.
            self.ids.remove(path);
        }
        self.backing_ids.insert(path.to_owned(), identity);
        self.ino(path)
    }

    fn register_handle_inode(&mut self, handle: u64, ino: u64) {
        self.handle_inodes.insert(handle, ino);
    }

    fn open_handle_for_inode(&self, ino: u64) -> Option<u64> {
        self.handle_inodes
            .iter()
            .find_map(|(handle, known)| (*known == ino).then_some(*handle))
    }

    fn add_handle(&mut self, handle: Handle) -> u64 {
        let id = self.next_handle;
        self.next_handle += 1;
        self.handles.insert(id, handle);
        id
    }

    fn root_name(path: &str) -> Result<&str, i32> {
        if path == "/" {
            return Err(libc::EINVAL);
        }
        path.trim_start_matches('/')
            .split('/')
            .next()
            .filter(|root| center::safe(root))
            .ok_or(libc::EINVAL)
    }

    fn attr_ttl(&self, path: &str, shared_roots: &HashSet<String>) -> &'static Duration {
        if path == "/" {
            return if self.backend == BackendMode::P2p {
                &PRIVATE_ATTR_TTL
            } else {
                &TTL
            };
        }
        let Ok(root) = Self::root_name(path) else {
            return &TTL;
        };
        if self.backend == BackendMode::P2p
            && self.owners.get(root) == Some(&self.id)
            && !shared_roots.contains(root)
        {
            &PRIVATE_ATTR_TTL
        } else {
            &TTL
        }
    }

    fn owner(&mut self, root: &str) -> Result<String, i32> {
        if let Some(owner) = self.owners.get(root) {
            return Ok(owner.clone());
        }
        match self.lookup_root(root)? {
            RootLookup::Active(owner) => {
                self.owners.insert(root.to_owned(), owner.clone());
                Ok(owner)
            }
            RootLookup::Missing => Err(libc::ENOENT),
            RootLookup::Pending(_) | RootLookup::Deleting(_) => Err(libc::EAGAIN),
            RootLookup::Tombstone => Err(libc::ENOENT),
        }
    }

    fn lookup_root(&self, root: &str) -> Result<RootLookup, i32> {
        let response = center::query(&self.center, &format!("GET {root}")).map_err(io_error)?;
        parse_root_lookup(&response)
    }

    fn location(&mut self, path: &str) -> Result<Location, i32> {
        if path == "/" {
            return Ok(Location::Local);
        }
        let root = Self::root_name(path)?.to_owned();
        let owner = self.owner(&root)?;
        if owner == self.id {
            Ok(Location::Local)
        } else {
            match self.backend {
                BackendMode::Nfs => {
                    if self.mounted.read().unwrap().contains(&owner) {
                        Ok(Location::RemoteNfs(owner))
                    } else {
                        Err(libc::EHOSTUNREACH)
                    }
                }
                BackendMode::P2p => Ok(Location::RemoteP2p(owner)),
            }
        }
    }

    fn base_for(&self, owner: Option<&str>) -> PathBuf {
        match owner {
            Some(owner) => self.peers.join(owner),
            None => self.local.clone(),
        }
    }

    fn p2p_endpoint(&mut self, owner: &str) -> Result<String, i32> {
        if let Some(endpoint) = self.p2p_endpoints.get(owner) {
            return Ok(endpoint.clone());
        }
        let rows = center::query(&self.center, "NODES").map_err(io_error)?;
        for row in rows.lines() {
            let fields: Vec<_> = row.split_whitespace().collect();
            if fields.len() != 3 {
                continue;
            }
            if center::safe(fields[0]) {
                self.p2p_endpoints
                    .insert(fields[0].to_owned(), fields[2].to_owned());
            }
        }
        self.p2p_endpoints
            .get(owner)
            .cloned()
            .ok_or(libc::EHOSTUNREACH)
    }

    fn p2p_client(&mut self, owner: &str) -> Result<&mut Client, i32> {
        if !self.p2p_clients.contains_key(owner) {
            let endpoint = self.p2p_endpoint(owner)?;
            let client = Client::connect_authenticated(&endpoint, &self.token)
                .map_err(|_| libc::EHOSTUNREACH)?;
            self.p2p_clients.insert(owner.to_owned(), client);
        }
        Ok(self.p2p_clients.get_mut(owner).expect("client inserted"))
    }

    fn p2p_existing_client(&mut self, owner: &str) -> Result<&mut Client, i32> {
        self.p2p_clients.get_mut(owner).ok_or(libc::ESTALE)
    }

    fn p2p_path_call<T>(
        &mut self,
        owner: &str,
        mut call: impl FnMut(&mut Client) -> Result<T, i32>,
    ) -> Result<T, i32> {
        match self.p2p_client(owner).and_then(&mut call) {
            Err(error) if connection_lost(error) => {
                self.p2p_clients.remove(owner);
                self.p2p_endpoints.remove(owner);
                self.p2p_client(owner).and_then(call)
            }
            result => result,
        }
    }

    fn p2p_mutating_path_call<T>(
        &mut self,
        owner: &str,
        call: impl FnOnce(&mut Client) -> Result<T, i32>,
    ) -> Result<T, i32> {
        let result = self.p2p_client(owner).and_then(call);
        if result.as_ref().is_err_and(|error| connection_lost(*error)) {
            self.p2p_clients.remove(owner);
            self.p2p_endpoints.remove(owner);
        }
        result
    }

    fn p2p_handle_call<T>(
        &mut self,
        owner: &str,
        call: impl FnOnce(&mut Client) -> Result<T, i32>,
    ) -> Result<T, i32> {
        match self.p2p_existing_client(owner).and_then(call) {
            Err(error) if connection_lost(error) => {
                self.p2p_clients.remove(owner);
                self.p2p_endpoints.remove(owner);
                Err(libc::ESTALE)
            }
            result => result,
        }
    }

    fn attr(&mut self, path: &str) -> Result<FileAttr, i32> {
        if path == "/" {
            return self.root_attr();
        }
        match self.location(path)? {
            Location::Local => {
                let attr = checked_attr(&self.base_for(None), path, self.ino(path))?;
                Ok(attr)
            }
            Location::RemoteNfs(owner) => {
                let (mut attr, dev, backing_ino) =
                    checked_attr_with_identity(&self.base_for(Some(&owner)), path, 0)?;
                attr.ino = self.ino_for_backing(path, &owner, dev, backing_ino);
                Ok(attr)
            }
            Location::RemoteP2p(owner) => {
                let attr = self.p2p_path_call(&owner, |client| client.getattr(path))?;
                let ino = self.ino_for_backing(path, &owner, attr.dev, attr.ino);
                Ok(attr.file_attr(ino))
            }
        }
    }

    fn attr_handle(&mut self, handle: u64, ino: u64) -> Result<FileAttr, i32> {
        match self.handles.get(&handle).ok_or(libc::EBADF)? {
            Handle::File { file, .. } => {
                let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
                if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } < 0 {
                    return Err(io_error(io::Error::last_os_error()));
                }
                attr_from_stat(ino, unsafe { stat.assume_init() })
            }
            Handle::P2p { owner, remote, .. } => {
                let owner = owner.clone();
                let remote = *remote;
                self.p2p_handle_call(&owner, |client| client.getattr_handle(remote))
                    .map(|attr| attr.file_attr(ino))
            }
        }
    }

    fn root_attr(&mut self) -> Result<FileAttr, i32> {
        let metadata = fs::metadata(&self.local).map_err(io_error)?;
        attr_from_metadata(ROOT_INO, metadata)
    }

    fn close_handle(&mut self, handle: u64) -> Result<(), i32> {
        self.handle_inodes.remove(&handle);
        match self.handles.remove(&handle) {
            Some(Handle::File { .. }) => Ok(()),
            Some(Handle::P2p {
                owner,
                remote,
                read_only,
                ..
            }) => {
                if read_only {
                    self.p2p_handle_call(&owner, |client| client.close_no_reply(remote))
                } else {
                    self.p2p_handle_call(&owner, |client| client.close(remote))
                }
            }
            None => Err(libc::EBADF),
        }
    }

    fn fsync_handle(&mut self, handle: u64, datasync: bool) -> Result<(), i32> {
        let result = match self.handles.get(&handle).ok_or(libc::EBADF)? {
            Handle::File { file, .. } => if datasync {
                file.sync_data()
            } else {
                file.sync_all()
            }
            .map_err(io_error),
            Handle::P2p { owner, remote, .. } => {
                let owner = owner.clone();
                let remote = *remote;
                self.p2p_handle_call(&owner, |client| client.fsync(remote, datasync))
            }
        };
        if result.is_ok() {
            self.handles
                .get_mut(&handle)
                .unwrap()
                .set_needs_flush(false);
        }
        result
    }

    fn flush_handle(&mut self, handle: u64) -> Result<(), i32> {
        if !self.handles.get(&handle).ok_or(libc::EBADF)?.needs_flush() {
            return Ok(());
        }
        self.fsync_handle(handle, false)
    }

    fn set_attributes(&mut self, path: &str, spec: &SetAttrSpec) -> Result<(), i32> {
        match self.location(path)? {
            location @ (Location::Local | Location::RemoteNfs(_)) => {
                let base = match location {
                    Location::Local => self.base_for(None),
                    Location::RemoteNfs(owner) => self.base_for(Some(&owner)),
                    Location::RemoteP2p(_) => unreachable!(),
                };
                let file = checked_open(&base, path, libc::O_RDONLY, 0)?;
                if let Some(mode) = spec.mode
                    && unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } < 0
                {
                    return Err(io_error(io::Error::last_os_error()));
                }
                if spec.uid.is_some() || spec.gid.is_some() {
                    let uid = spec.uid.unwrap_or(!0) as libc::uid_t;
                    let gid = spec.gid.unwrap_or(!0) as libc::gid_t;
                    if unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } < 0 {
                        return Err(io_error(io::Error::last_os_error()));
                    }
                }
                if spec.atime.is_some() || spec.mtime.is_some() {
                    let stamps = [to_libc_timespec(spec.atime), to_libc_timespec(spec.mtime)];
                    if unsafe { libc::futimens(file.as_raw_fd(), stamps.as_ptr()) } < 0 {
                        return Err(io_error(io::Error::last_os_error()));
                    }
                }
                Ok(())
            }
            Location::RemoteP2p(owner) => self
                .p2p_mutating_path_call(&owner, |client| client.setattr(path, spec))
                .map(|_| ()),
        }
    }

    fn set_attributes_handle(&mut self, handle: u64, spec: &SetAttrSpec) -> Result<(), i32> {
        match self.handles.get(&handle).ok_or(libc::EBADF)? {
            Handle::File { file, .. } => {
                if let Some(mode) = spec.mode
                    && unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } < 0
                {
                    return Err(io_error(io::Error::last_os_error()));
                }
                if spec.uid.is_some() || spec.gid.is_some() {
                    let uid = spec.uid.unwrap_or(!0) as libc::uid_t;
                    let gid = spec.gid.unwrap_or(!0) as libc::gid_t;
                    if unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } < 0 {
                        return Err(io_error(io::Error::last_os_error()));
                    }
                }
                if spec.atime.is_some() || spec.mtime.is_some() {
                    let stamps = [to_libc_timespec(spec.atime), to_libc_timespec(spec.mtime)];
                    if unsafe { libc::futimens(file.as_raw_fd(), stamps.as_ptr()) } < 0 {
                        return Err(io_error(io::Error::last_os_error()));
                    }
                }
                Ok(())
            }
            Handle::P2p { owner, remote, .. } => {
                let owner = owner.clone();
                let remote = *remote;
                self.p2p_handle_call(&owner, |client| client.setattr_handle(remote, spec))
                    .map(|_| ())
            }
        }
    }

    fn open_existing(&mut self, path: &str, flags: i32, directory: bool) -> Result<u64, i32> {
        match self.location(path)? {
            Location::Local => {
                let flags = if directory {
                    flags | libc::O_DIRECTORY
                } else {
                    flags
                };
                let file = checked_open(&self.base_for(None), path, flags, 0)?;
                Ok(self.add_handle(Handle::File {
                    file,
                    needs_flush: flags & libc::O_ACCMODE != libc::O_RDONLY
                        || flags & libc::O_TRUNC != 0,
                }))
            }
            Location::RemoteNfs(owner) => {
                let flags = if directory {
                    flags | libc::O_DIRECTORY
                } else {
                    flags
                };
                let file = checked_open(&self.base_for(Some(&owner)), path, flags, 0)?;
                Ok(self.add_handle(Handle::File {
                    file,
                    needs_flush: flags & libc::O_ACCMODE != libc::O_RDONLY
                        || flags & libc::O_TRUNC != 0,
                }))
            }
            Location::RemoteP2p(owner) => {
                let open = |client: &mut Client| client.open(path, flags, directory);
                let (remote, _, prefetched) = if flags & (libc::O_TRUNC | libc::O_CREAT) != 0 {
                    self.p2p_mutating_path_call(&owner, open)?
                } else {
                    self.p2p_path_call(&owner, open)?
                };
                Ok(self.add_handle(Handle::P2p {
                    owner,
                    remote,
                    read_only: flags & libc::O_ACCMODE == libc::O_RDONLY,
                    needs_flush: flags & libc::O_ACCMODE != libc::O_RDONLY
                        || flags & libc::O_TRUNC != 0,
                    prefetched,
                }))
            }
        }
    }

    fn create_file(&mut self, path: &str, flags: i32, mode: u32) -> Result<(FileAttr, u64), i32> {
        match self.location(path)? {
            Location::Local => {
                let file = checked_open(&self.base_for(None), path, flags | libc::O_CREAT, mode)?;
                let handle = self.add_handle(Handle::File {
                    file,
                    needs_flush: true,
                });
                Ok((self.attr(path)?, handle))
            }
            Location::RemoteNfs(owner) => {
                let file = checked_open(
                    &self.base_for(Some(&owner)),
                    path,
                    flags | libc::O_CREAT,
                    mode,
                )?;
                let handle = self.add_handle(Handle::File {
                    file,
                    needs_flush: true,
                });
                Ok((self.attr(path)?, handle))
            }
            Location::RemoteP2p(owner) => {
                let (remote, attr) =
                    self.p2p_mutating_path_call(&owner, |client| client.create(path, flags, mode))?;
                let ino = self.ino_for_backing(path, &owner, attr.dev, attr.ino);
                Ok((
                    attr.file_attr(ino),
                    self.add_handle(Handle::P2p {
                        owner,
                        remote,
                        read_only: false,
                        needs_flush: true,
                        prefetched: None,
                    }),
                ))
            }
        }
    }

    fn mkdir_path(&mut self, path: &str, mode: u32) -> Result<FileAttr, i32> {
        match self.location(path)? {
            Location::Local => {
                checked_mkdir(&self.base_for(None), path, mode)?;
                self.attr(path)
            }
            Location::RemoteNfs(owner) => {
                checked_mkdir(&self.base_for(Some(&owner)), path, mode)?;
                self.attr(path)
            }
            Location::RemoteP2p(owner) => {
                let attr =
                    self.p2p_mutating_path_call(&owner, |client| client.mkdir(path, mode))?;
                let ino = self.ino_for_backing(path, &owner, attr.dev, attr.ino);
                Ok(attr.file_attr(ino))
            }
        }
    }

    fn mkdir_root(&mut self, root: &str, mode: u32) -> Result<FileAttr, i32> {
        match self.lookup_root(root)? {
            RootLookup::Active(owner) if owner == self.id => return Err(libc::EEXIST),
            RootLookup::Active(_) => return Err(libc::EEXIST),
            RootLookup::Pending(owner) if owner == self.id => {
                return self.ensure_root_dir_then_activate(root, mode);
            }
            RootLookup::Pending(_) | RootLookup::Deleting(_) => return Err(libc::EAGAIN),
            RootLookup::Tombstone => return Err(libc::EEXIST),
            RootLookup::Missing => {}
        }

        let reserve = center::query(&self.center, &format!("RESERVE {root} {}", self.id))
            .map_err(io_error)?;
        if reserve != "OK" {
            return Err(conflict_status(&reserve));
        }

        self.ensure_root_dir_then_activate(root, mode)
    }

    fn ensure_root_dir_then_activate(&mut self, root: &str, mode: u32) -> Result<FileAttr, i32> {
        let path = format!("/{root}");
        match checked_mkdir(&self.base_for(None), &path, mode) {
            Ok(()) => {}
            Err(error)
                if error == libc::EEXIST
                    && checked_attr(&self.base_for(None), &path, 0)
                        .is_ok_and(|attr| attr.kind == FileType::Directory) => {}
            Err(error) => return Err(error),
        }

        let activate = center::query(&self.center, &format!("ACTIVATE {root} {}", self.id))
            .map_err(io_error)?;
        if activate != "OK" {
            return Err(conflict_status(&activate));
        }

        self.owners.insert(root.to_owned(), self.id.clone());
        self.attr(&path)
    }

    fn remove_file_path(&mut self, path: &str) -> Result<(), i32> {
        match self.location(path)? {
            Location::Local => checked_unlink(&self.base_for(None), path, false),
            Location::RemoteNfs(owner) => checked_unlink(&self.base_for(Some(&owner)), path, false),
            Location::RemoteP2p(owner) => {
                self.p2p_mutating_path_call(&owner, |client| client.unlink(path, false))
            }
        }?;
        self.drop_paths(path, false);
        Ok(())
    }

    fn remove_dir_path(&mut self, path: &str) -> Result<(), i32> {
        match self.location(path)? {
            Location::Local => checked_unlink(&self.base_for(None), path, true),
            Location::RemoteNfs(owner) => checked_unlink(&self.base_for(Some(&owner)), path, true),
            Location::RemoteP2p(owner) => {
                self.p2p_mutating_path_call(&owner, |client| client.unlink(path, true))
            }
        }?;
        self.drop_paths(path, true);
        Ok(())
    }

    fn remove_root(&mut self, root: &str) -> Result<(), i32> {
        match self.lookup_root(root)? {
            RootLookup::Active(owner) if owner == self.id => {}
            RootLookup::Deleting(owner) if owner == self.id => {}
            RootLookup::Active(_) | RootLookup::Pending(_) | RootLookup::Deleting(_) => {
                return Err(libc::EXDEV);
            }
            RootLookup::Missing | RootLookup::Tombstone => return Err(libc::ENOENT),
        }
        let path = format!("/{root}");
        let base = self.base_for(None);
        let physical_missing = match checked_read_dir(&base, &path) {
            Ok(mut entries) => {
                if entries.pop().is_some() {
                    return Err(libc::ENOTEMPTY);
                }
                false
            }
            Err(error) if error == libc::ENOENT => true,
            Err(error) => return Err(error),
        };

        let prepare = center::query(&self.center, &format!("DELETE_PREPARE {root} {}", self.id))
            .map_err(io_error)?;
        if prepare != "OK" {
            return Err(conflict_status(&prepare));
        }

        if !physical_missing {
            match checked_unlink(&base, &path, true) {
                Ok(()) => {}
                Err(error) if error == libc::ENOENT => {}
                Err(error) => return Err(error),
            }
        }

        let commit = center::query(&self.center, &format!("DELETE_COMMIT {root} {}", self.id))
            .map_err(io_error)?;
        if commit != "OK" && commit != "MISSING" {
            return Err(conflict_status(&commit));
        }
        self.owners.remove(root);
        self.drop_paths(&path, true);
        Ok(())
    }

    fn rename_path(&mut self, old: &str, new: &str) -> Result<(), i32> {
        let old_root = Self::root_name(old)?;
        let new_root = Self::root_name(new)?;
        if old_root != new_root {
            return Err(libc::EXDEV);
        }
        match self.location(old)? {
            Location::Local => checked_rename(&self.base_for(None), old, new)?,
            Location::RemoteNfs(owner) => checked_rename(&self.base_for(Some(&owner)), old, new)?,
            Location::RemoteP2p(owner) => {
                let _ = self.p2p_mutating_path_call(&owner, |client| client.rename(old, new))?;
            }
        }
        self.shift_paths(old, new, true);
        Ok(())
    }

    fn readdir_rows(&mut self, path: &str) -> Result<Vec<(u64, FileType, String)>, i32> {
        let parent = parent_ino(&mut self.ids, &mut self.paths, &mut self.next_ino, path);
        let ino = self.ino(path);
        let mut rows = vec![
            (ino, FileType::Directory, ".".to_owned()),
            (parent, FileType::Directory, "..".to_owned()),
        ];
        if path == "/" {
            let roots = center::query(&self.center, "ROOTS").map_err(io_error)?;
            for line in roots.lines() {
                let fields: Vec<_> = line.split_whitespace().collect();
                if fields.len() < 2 || !center::safe(fields[0]) || !center::safe(fields[1]) {
                    continue;
                }
                self.owners
                    .insert(fields[0].to_owned(), fields[1].to_owned());
                rows.push((
                    self.ino(&format!("/{}", fields[0])),
                    FileType::Directory,
                    fields[0].to_owned(),
                ));
            }
            return Ok(rows);
        }

        match self.location(path)? {
            Location::Local => {
                self.push_local_dir_rows(&mut rows, path, &self.base_for(None))?;
            }
            Location::RemoteNfs(owner) => {
                self.push_local_dir_rows(&mut rows, path, &self.base_for(Some(&owner)))?;
            }
            Location::RemoteP2p(owner) => {
                for (name, kind) in self.p2p_path_call(&owner, |client| client.readdir(path))? {
                    rows.push((self.ino(&join_path(path, &name)), kind, name));
                }
            }
        }
        Ok(rows)
    }

    fn push_local_dir_rows(
        &mut self,
        rows: &mut Vec<(u64, FileType, String)>,
        logical: &str,
        base: &Path,
    ) -> Result<(), i32> {
        for (name, kind) in checked_read_dir(base, logical)? {
            rows.push((self.ino(&join_path(logical, &name)), kind, name));
        }
        Ok(())
    }

    fn set_len(&mut self, path: &str, handle: Option<u64>, size: u64) -> Result<(), i32> {
        if let Some(handle) = handle {
            self.handles
                .get_mut(&handle)
                .ok_or(libc::EBADF)?
                .set_needs_flush(true);
            match self.handles.get(&handle).ok_or(libc::EBADF)? {
                Handle::File { file, .. } => return file.set_len(size).map_err(io_error),
                Handle::P2p { owner, remote, .. } => {
                    let owner = owner.clone();
                    let remote = *remote;
                    return self
                        .p2p_handle_call(&owner, |client| client.set_len_handle(remote, size))
                        .map(|_| ());
                }
            }
        }
        match self.location(path)? {
            Location::Local => checked_open(&self.base_for(None), path, libc::O_WRONLY, 0)?
                .set_len(size)
                .map_err(io_error),
            Location::RemoteNfs(owner) => {
                checked_open(&self.base_for(Some(&owner)), path, libc::O_WRONLY, 0)?
                    .set_len(size)
                    .map_err(io_error)
            }
            Location::RemoteP2p(owner) => self
                .p2p_mutating_path_call(&owner, |client| client.set_len(path, size))
                .map(|_| ()),
        }
    }

    fn shift_paths(&mut self, old: &str, new: &str, recursive: bool) {
        shift_paths(&mut self.ids, &mut self.paths, old, new, recursive);
        shift_backing_ids(&mut self.backing_ids, old, new, recursive);
    }

    fn drop_paths(&mut self, path: &str, recursive: bool) {
        drop_paths(&mut self.ids, &mut self.paths, path, recursive);
        drop_backing_ids(&mut self.backing_ids, path, recursive);
    }
}

impl Filesystem for HomeFs {
    fn init(&mut self, _: &Request<'_>, cfg: &mut KernelConfig) -> Result<(), i32> {
        let _ = cfg.set_max_write(1024 * 1024);
        Ok(())
    }

    fn lookup(&mut self, _: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        // Serialize the attribute read and reply with the first remote access.
        // Otherwise an in-flight lookup could publish a stale private TTL after
        // the owner has already invalidated its cached attributes.
        let cache = Arc::clone(&self.private_cache);
        let shared = cache.shared_roots.lock().unwrap();
        match self.child(parent, name).and_then(|path| {
            let attr = self.attr(&path)?;
            Ok((path, attr))
        }) {
            Ok((path, attr)) if self.backend == BackendMode::P2p => {
                let ttl = self.attr_ttl(&path, &shared);
                reply.entry_with_ttls(&P2P_ENTRY_TTL, ttl, &attr, 0)
            }
            Ok((_, attr)) => reply.entry(&TTL, &attr, 0),
            Err(error) => reply.error(error),
        }
    }

    fn getattr(&mut self, _: &Request<'_>, ino: u64, handle: Option<u64>, reply: ReplyAttr) {
        let cache = Arc::clone(&self.private_cache);
        let shared = cache.shared_roots.lock().unwrap();
        let handle = handle.or_else(|| self.open_handle_for_inode(ino));
        let result = match handle {
            Some(handle) => self.attr_handle(handle, ino),
            None => self
                .path(ino)
                .map(str::to_owned)
                .and_then(|path| self.attr(&path))
                .and_then(|attr| (attr.ino == ino).then_some(attr).ok_or(libc::ESTALE)),
        };
        match result {
            Ok(attr) => {
                let ttl = self
                    .path(ino)
                    .ok()
                    .map(|path| self.attr_ttl(path, &shared))
                    .unwrap_or(&TTL);
                reply.attr(ttl, &attr)
            }
            Err(error) => reply.error(error),
        }
    }

    fn mkdir(
        &mut self,
        _: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _: u32,
        reply: ReplyEntry,
    ) {
        let result = (|| {
            let path = self.child(parent, name)?;
            if parent == ROOT_INO {
                self.mkdir_root(path.trim_start_matches('/'), mode)
            } else {
                self.mkdir_path(&path, mode)
            }
        })();
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, 0),
            Err(error) => reply.error(error),
        }
    }

    fn create(
        &mut self,
        _: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let result = (|| {
            if parent == ROOT_INO {
                return Err(libc::EOPNOTSUPP);
            }
            let path = self.child(parent, name)?;
            self.create_file(&path, flags, mode)
        })();
        match result {
            Ok((attr, handle)) => {
                self.register_handle_inode(handle, attr.ino);
                reply.created(&TTL, &attr, 0, handle, 0)
            }
            Err(error) => reply.error(error),
        }
    }

    fn open(&mut self, _: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        match self
            .path(ino)
            .map(str::to_owned)
            .and_then(|path| self.open_existing(&path, flags, false))
        {
            Ok(handle) => {
                self.register_handle_inode(handle, ino);
                reply.opened(handle, 0)
            }
            Err(error) => reply.error(error),
        }
    }

    fn read(
        &mut self,
        _: &Request<'_>,
        _: u64,
        handle: u64,
        offset: i64,
        size: u32,
        _: i32,
        _: Option<u64>,
        reply: ReplyData,
    ) {
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let mut buffer = vec![0; size as usize];
        let result = match self.handles.get_mut(&handle).ok_or(libc::EBADF) {
            Ok(Handle::File { file, .. }) => file
                .read_at(&mut buffer, offset as u64)
                .map(|count| buffer[..count].to_vec())
                .map_err(io_error),
            Ok(Handle::P2p {
                owner,
                remote,
                prefetched,
                ..
            }) => {
                if let Some(bytes) = prefetched {
                    let start = (offset as usize).min(bytes.len());
                    let end = start.saturating_add(size as usize).min(bytes.len());
                    Ok(bytes[start..end].to_vec())
                } else {
                    let owner = owner.clone();
                    let remote = *remote;
                    self.p2p_handle_call(&owner, |client| client.read(remote, offset as u64, size))
                }
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(bytes) => reply.data(&bytes),
            Err(error) => reply.error(error),
        }
    }

    fn write(
        &mut self,
        _: &Request<'_>,
        _: u64,
        handle: u64,
        offset: i64,
        data: &[u8],
        _: u32,
        _: i32,
        _: Option<u64>,
        reply: ReplyWrite,
    ) {
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        if let Some(entry) = self.handles.get_mut(&handle) {
            entry.set_needs_flush(true);
        }
        let result = match self.handles.get_mut(&handle).ok_or(libc::EBADF) {
            Ok(Handle::File { file, .. }) => file
                .write_at(data, offset as u64)
                .map(|n| n as u32)
                .map_err(io_error),
            Ok(Handle::P2p { owner, remote, .. }) => {
                let owner = owner.clone();
                let remote = *remote;
                self.p2p_handle_call(&owner, |client| client.write(remote, offset as u64, data))
                    .map(|(written, _)| written)
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(written) => reply.written(written),
            Err(error) => reply.error(error),
        }
    }

    fn flush(&mut self, _: &Request<'_>, _: u64, handle: u64, _: u64, reply: ReplyEmpty) {
        match self.flush_handle(handle) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn release(
        &mut self,
        _: &Request<'_>,
        _: u64,
        handle: u64,
        _: i32,
        _: Option<u64>,
        _: bool,
        reply: ReplyEmpty,
    ) {
        match self.close_handle(handle) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn fsync(&mut self, _: &Request<'_>, _: u64, handle: u64, datasync: bool, reply: ReplyEmpty) {
        match self.fsync_handle(handle, datasync) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn opendir(&mut self, _: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        match self
            .path(ino)
            .map(str::to_owned)
            .and_then(|path| self.open_existing(&path, flags, true))
        {
            Ok(handle) => {
                self.register_handle_inode(handle, ino);
                reply.opened(handle, 0)
            }
            Err(error) => reply.error(error),
        }
    }

    fn releasedir(&mut self, _: &Request<'_>, _: u64, handle: u64, _: i32, reply: ReplyEmpty) {
        match self.close_handle(handle) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn fsyncdir(
        &mut self,
        _: &Request<'_>,
        _: u64,
        handle: u64,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        match self.fsync_handle(handle, datasync) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn setattr(
        &mut self,
        _: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _: Option<SystemTime>,
        handle: Option<u64>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<u32>,
        reply: ReplyAttr,
    ) {
        let handle = handle.or_else(|| self.open_handle_for_inode(ino));
        let result = (|| {
            let path = match handle {
                Some(_) => None,
                None => Some(self.path(ino)?.to_owned()),
            };
            if let Some(size) = size {
                self.set_len(path.as_deref().unwrap_or(""), handle, size)?;
            }
            if mode.is_some()
                || uid.is_some()
                || gid.is_some()
                || atime.is_some()
                || mtime.is_some()
            {
                let spec = SetAttrSpec {
                    mode,
                    uid,
                    gid,
                    atime: atime.map(to_rpc_time),
                    mtime: mtime.map(to_rpc_time),
                };
                if let Some(handle) = handle {
                    self.set_attributes_handle(handle, &spec)?;
                } else {
                    self.set_attributes(path.as_deref().unwrap(), &spec)?;
                }
            }
            match handle {
                Some(handle) => self.attr_handle(handle, ino),
                None => self.attr(path.as_deref().unwrap()),
            }
        })();
        match result {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(error) => reply.error(error),
        }
    }

    fn rename(
        &mut self,
        _: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        flags: u32,
        reply: ReplyEmpty,
    ) {
        if flags != 0 {
            reply.error(libc::EOPNOTSUPP);
            return;
        }
        let result = (|| {
            if parent == ROOT_INO || newparent == ROOT_INO {
                return Err(libc::EXDEV);
            }
            let old = self.child(parent, name)?;
            let new = self.child(newparent, newname)?;
            self.rename_path(&old, &new)
        })();
        match result {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn unlink(&mut self, _: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let result = (|| {
            if parent == ROOT_INO {
                return Err(libc::EOPNOTSUPP);
            }
            let path = self.child(parent, name)?;
            self.remove_file_path(&path)
        })();
        match result {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn rmdir(&mut self, _: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let result = (|| {
            let path = self.child(parent, name)?;
            if parent == ROOT_INO {
                self.remove_root(path.trim_start_matches('/'))
            } else {
                self.remove_dir_path(&path)
            }
        })();
        match result {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn readdir(
        &mut self,
        _: &Request<'_>,
        ino: u64,
        _: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let result = self
            .path(ino)
            .map(str::to_owned)
            .and_then(|path| self.readdir_rows(&path));
        match result {
            Ok(rows) => {
                for (index, (ino, kind, name)) in
                    rows.into_iter().enumerate().skip(offset.max(0) as usize)
                {
                    if reply.add(ino, (index + 1) as i64, kind, name) {
                        break;
                    }
                }
                reply.ok();
            }
            Err(error) => reply.error(error),
        }
    }
}

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn to_rpc_time(value: TimeOrNow) -> TimeSpec {
    match value {
        TimeOrNow::Now => TimeSpec::Now,
        TimeOrNow::SpecificTime(time) => match time.duration_since(UNIX_EPOCH) {
            Ok(duration) => TimeSpec::At(duration.as_secs() as i64, duration.subsec_nanos()),
            Err(error) => {
                let duration = error.duration();
                let nanos = duration.subsec_nanos();
                if nanos == 0 {
                    TimeSpec::At(-(duration.as_secs() as i64), 0)
                } else {
                    TimeSpec::At(-(duration.as_secs() as i64) - 1, 1_000_000_000 - nanos)
                }
            }
        },
    }
}

fn to_libc_timespec(value: Option<TimeSpec>) -> libc::timespec {
    match value {
        None => libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_OMIT,
        },
        Some(TimeSpec::Now) => libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_NOW,
        },
        Some(TimeSpec::At(seconds, nanos)) => libc::timespec {
            tv_sec: seconds as libc::time_t,
            tv_nsec: nanos as libc::c_long,
        },
    }
}

fn parent_ino(
    ids: &mut HashMap<String, u64>,
    paths: &mut HashMap<u64, String>,
    next_ino: &mut u64,
    path: &str,
) -> u64 {
    let parent = if path == "/" {
        "/"
    } else {
        path.rsplit_once('/')
            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
            .unwrap_or("/")
    };
    if let Some(ino) = ids.get(parent) {
        *ino
    } else {
        let ino = *next_ino;
        *next_ino += 1;
        ids.insert(parent.to_owned(), ino);
        paths.insert(ino, parent.to_owned());
        ino
    }
}

fn logical_components(path: &str) -> Result<Vec<&str>, i32> {
    if !path.starts_with('/') {
        return Err(libc::EINVAL);
    }
    if path == "/" {
        return Ok(Vec::new());
    }
    let mut parts = Vec::new();
    for part in path.split('/').skip(1) {
        if !center::safe(part) || part.as_bytes().contains(&0) {
            return Err(libc::EINVAL);
        }
        parts.push(part);
    }
    Ok(parts)
}

fn cstring_path(path: &Path) -> Result<CString, i32> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| libc::EINVAL)
}

fn cstring_name(name: &str) -> Result<CString, i32> {
    CString::new(name.as_bytes()).map_err(|_| libc::EINVAL)
}

fn open_base_dir(base: &Path) -> Result<File, i32> {
    let name = cstring_path(base)?;
    let fd = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn open_child_dir(parent: &File, name: &str) -> Result<File, i32> {
    let name = cstring_name(name)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub(crate) fn open_parent_dir(base: &Path, path: &str) -> Result<(File, String), i32> {
    let mut parts = logical_components(path)?;
    let leaf = parts.pop().ok_or(libc::EINVAL)?.to_owned();
    let mut dir = open_base_dir(base)?;
    for part in parts {
        dir = open_child_dir(&dir, part)?;
    }
    Ok((dir, leaf))
}

pub(crate) fn checked_open(base: &Path, path: &str, flags: i32, mode: u32) -> Result<File, i32> {
    if path == "/" {
        return open_base_dir(base);
    }
    logical_components(path)?;
    let dir = open_base_dir(base)?;
    let name = cstring_name(&path[1..])?;
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    let how = OpenHow {
        flags: (flags | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64,
        mode: (mode & 0o7777) as u64,
        resolve: 0x04 | 0x08, // RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            dir.as_raw_fd(),
            name.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd >= 0 {
        return Ok(unsafe { File::from_raw_fd(fd as i32) });
    }
    let error = io_error(io::Error::last_os_error());
    if error != libc::ENOSYS {
        return Err(error);
    }
    // Older Linux kernels retain the component-by-component checked walk.
    let (parent, leaf) = open_parent_dir(base, path)?;
    let leaf = cstring_name(&leaf)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    };
    if fd < 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub(crate) fn checked_mkdir(base: &Path, path: &str, mode: u32) -> Result<(), i32> {
    let (parent, leaf) = open_parent_dir(base, path)?;
    let leaf = cstring_name(&leaf)?;
    let status = unsafe { libc::mkdirat(parent.as_raw_fd(), leaf.as_ptr(), mode) };
    if status < 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(())
}

pub(crate) fn checked_unlink(base: &Path, path: &str, directory: bool) -> Result<(), i32> {
    let (parent, leaf) = open_parent_dir(base, path)?;
    let leaf = cstring_name(&leaf)?;
    let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
    let status = unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), flags) };
    if status < 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(())
}

pub(crate) fn checked_rename(base: &Path, old: &str, new: &str) -> Result<(), i32> {
    let (old_parent, old_leaf) = open_parent_dir(base, old)?;
    let (new_parent, new_leaf) = open_parent_dir(base, new)?;
    let old_leaf = cstring_name(&old_leaf)?;
    let new_leaf = cstring_name(&new_leaf)?;
    let status = unsafe {
        libc::renameat(
            old_parent.as_raw_fd(),
            old_leaf.as_ptr(),
            new_parent.as_raw_fd(),
            new_leaf.as_ptr(),
        )
    };
    if status < 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(())
}

fn checked_attr(base: &Path, path: &str, ino: u64) -> Result<FileAttr, i32> {
    checked_attr_with_identity(base, path, ino).map(|(attr, _, _)| attr)
}

fn checked_attr_with_identity(
    base: &Path,
    path: &str,
    ino: u64,
) -> Result<(FileAttr, u64, u64), i32> {
    if path == "/" {
        let base = open_base_dir(base)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let status = unsafe { libc::fstat(base.as_raw_fd(), stat.as_mut_ptr()) };
        if status < 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        let stat = unsafe { stat.assume_init() };
        return Ok((attr_from_stat(ino, stat)?, stat.st_dev, stat.st_ino));
    }
    let (parent, leaf) = open_parent_dir(base, path)?;
    let leaf = cstring_name(&leaf)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let status = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status < 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    let stat = unsafe { stat.assume_init() };
    Ok((attr_from_stat(ino, stat)?, stat.st_dev, stat.st_ino))
}

pub(crate) fn checked_read_dir(base: &Path, path: &str) -> Result<Vec<(String, FileType)>, i32> {
    let dir = checked_open(base, path, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    let fd_path = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()));
    let mut rows = Vec::new();
    for entry in fs::read_dir(fd_path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let file_type = entry.file_type().map_err(io_error)?;
        let kind = if file_type.is_dir() {
            FileType::Directory
        } else if file_type.is_file() {
            FileType::RegularFile
        } else {
            return Err(libc::EOPNOTSUPP);
        };
        rows.push((entry.file_name().to_string_lossy().into_owned(), kind));
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rows)
}

fn attr_from_metadata(ino: u64, metadata: fs::Metadata) -> Result<FileAttr, i32> {
    let kind = if metadata.is_dir() {
        FileType::Directory
    } else if metadata.is_file() {
        FileType::RegularFile
    } else {
        return Err(libc::EOPNOTSUPP);
    };
    let stamp = |secs: i64, nanos: i64| -> SystemTime {
        if secs >= 0 {
            UNIX_EPOCH + Duration::new(secs as u64, nanos.max(0) as u32)
        } else {
            UNIX_EPOCH
        }
    };
    Ok(FileAttr {
        ino,
        size: metadata.size(),
        blocks: metadata.blocks(),
        atime: stamp(metadata.atime(), metadata.atime_nsec()),
        mtime: stamp(metadata.mtime(), metadata.mtime_nsec()),
        ctime: stamp(metadata.ctime(), metadata.ctime_nsec()),
        crtime: UNIX_EPOCH,
        kind,
        perm: metadata.mode() as u16 & 0o7777,
        nlink: metadata.nlink() as u32,
        uid: metadata.uid(),
        gid: metadata.gid(),
        rdev: metadata.rdev() as u32,
        blksize: metadata.blksize() as u32,
        flags: 0,
    })
}

fn attr_from_stat(ino: u64, stat: libc::stat) -> Result<FileAttr, i32> {
    let file_type = stat.st_mode & libc::S_IFMT;
    let kind = if file_type == libc::S_IFDIR {
        FileType::Directory
    } else if file_type == libc::S_IFREG {
        FileType::RegularFile
    } else {
        return Err(libc::EOPNOTSUPP);
    };
    let stamp = |secs: i64, nanos: i64| -> SystemTime {
        if secs >= 0 {
            UNIX_EPOCH + Duration::new(secs as u64, nanos.max(0) as u32)
        } else {
            UNIX_EPOCH
        }
    };
    Ok(FileAttr {
        ino,
        size: stat.st_size as u64,
        blocks: stat.st_blocks as u64,
        atime: stamp(stat.st_atime, stat.st_atime_nsec),
        mtime: stamp(stat.st_mtime, stat.st_mtime_nsec),
        ctime: stamp(stat.st_ctime, stat.st_ctime_nsec),
        crtime: UNIX_EPOCH,
        kind,
        perm: stat.st_mode as u16 & 0o7777,
        nlink: nlink_as_u32(stat.st_nlink),
        uid: stat.st_uid,
        gid: stat.st_gid,
        rdev: stat.st_rdev as u32,
        blksize: stat.st_blksize as u32,
        flags: 0,
    })
}

fn nlink_as_u32(nlink: impl Into<u64>) -> u32 {
    u32::try_from(nlink.into()).unwrap_or(u32::MAX)
}

fn io_error(error: io::Error) -> i32 {
    error.raw_os_error().unwrap_or(libc::EIO)
}

fn conflict_status(response: &str) -> i32 {
    if response.starts_with("CONFLICT") {
        libc::EEXIST
    } else if response.starts_with("PENDING") || response.starts_with("DELETING") {
        libc::EAGAIN
    } else if response.starts_with("TOMBSTONE") {
        libc::EEXIST
    } else if response == "MISSING" {
        libc::ENOENT
    } else {
        libc::EIO
    }
}

fn connection_lost(error: i32) -> bool {
    matches!(
        error,
        libc::EPIPE | libc::ECONNRESET | libc::ECONNABORTED | libc::ETIMEDOUT | libc::ENOTCONN
    )
}

fn parse_root_lookup(response: &str) -> Result<RootLookup, i32> {
    let fields: Vec<_> = response.split_whitespace().collect();
    match fields.as_slice() {
        ["MISSING"] => Ok(RootLookup::Missing),
        ["PENDING", owner] if center::safe(owner) => Ok(RootLookup::Pending((*owner).to_owned())),
        ["DELETING", owner] if center::safe(owner) => Ok(RootLookup::Deleting((*owner).to_owned())),
        ["TOMBSTONE"] | ["TOMBSTONE", _] => Ok(RootLookup::Tombstone),
        [owner] if center::safe(owner) => Ok(RootLookup::Active((*owner).to_owned())),
        _ => Err(libc::EIO),
    }
}

fn drop_paths(
    ids: &mut HashMap<String, u64>,
    paths: &mut HashMap<u64, String>,
    path: &str,
    recursive: bool,
) {
    if recursive {
        let prefix = format!("{path}/");
        let removed: Vec<_> = ids
            .iter()
            .filter(|(known, _)| *known == path || known.starts_with(&prefix))
            .map(|(known, ino)| (known.clone(), *ino))
            .collect();
        for (known, ino) in removed {
            ids.remove(&known);
            paths.remove(&ino);
        }
    } else if let Some(ino) = ids.remove(path) {
        paths.remove(&ino);
    }
}

fn drop_backing_ids(ids: &mut HashMap<String, (String, u64, u64)>, path: &str, recursive: bool) {
    let prefix = format!("{path}/");
    ids.retain(|known, _| known != path && !(recursive && known.starts_with(&prefix)));
}

fn shift_backing_ids(
    ids: &mut HashMap<String, (String, u64, u64)>,
    old: &str,
    new: &str,
    recursive: bool,
) {
    drop_backing_ids(ids, new, recursive);
    let prefix = format!("{old}/");
    let moved: Vec<_> = ids
        .iter()
        .filter(|(path, _)| *path == old || (recursive && path.starts_with(&prefix)))
        .map(|(path, identity)| (path.clone(), identity.clone()))
        .collect();
    for (path, identity) in moved {
        ids.remove(&path);
        ids.insert(format!("{new}{}", &path[old.len()..]), identity);
    }
}

fn shift_paths(
    ids: &mut HashMap<String, u64>,
    paths: &mut HashMap<u64, String>,
    old: &str,
    new: &str,
    recursive: bool,
) {
    drop_paths(ids, paths, new, recursive);
    if !recursive {
        if let Some(ino) = ids.remove(old) {
            ids.insert(new.to_owned(), ino);
            paths.insert(ino, new.to_owned());
        }
        return;
    }
    let prefix = format!("{old}/");
    let moved: Vec<_> = ids
        .iter()
        .filter(|(path, _)| *path == old || path.starts_with(&prefix))
        .map(|(path, ino)| (path.clone(), *ino))
        .collect();
    for (path, ino) in moved {
        ids.remove(&path);
        let replacement = format!("{new}{}", &path[old.len()..]);
        paths.insert(ino, replacement.clone());
        ids.insert(replacement, ino);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        os::unix::fs::symlink,
        sync::mpsc,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    fn read_test_frame(stream: &mut TcpStream) -> Vec<u8> {
        let mut length = [0; 4];
        stream.read_exact(&mut length).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(length) as usize];
        stream.read_exact(&mut bytes).unwrap();
        bytes
    }

    fn write_test_frame(stream: &mut TcpStream, bytes: &[u8]) {
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(bytes).unwrap();
    }

    #[test]
    fn truncating_remote_open_is_not_replayed_after_lost_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = listener.local_addr().unwrap().to_string();
        let center_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let center_endpoint = center_listener.local_addr().unwrap().to_string();
        let advertised = endpoint.clone();
        let center = thread::spawn(move || {
            center_listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                match center_listener.accept() {
                    Ok((mut peer, _)) => {
                        let mut request = [0; 64];
                        let size = peer.read(&mut request).unwrap();
                        assert_eq!(&request[..size], b"NODES\n");
                        peer.write_all(format!("A unused {advertised}\n").as_bytes())
                            .unwrap();
                        return;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("center accept failed: {error}"),
                }
            }
        });
        let (send_count, receive_count) = mpsc::channel();
        let server = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            let mut requests = 0;
            while std::time::Instant::now() < deadline {
                let (mut peer, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("accept failed: {error}"),
                };
                let auth = read_test_frame(&mut peer);
                assert_eq!(auth[0], 0);
                write_test_frame(&mut peer, &0_u32.to_be_bytes());
                let request = read_test_frame(&mut peer);
                assert_eq!(request[0], 4);
                requests += 1;
                if requests == 2 {
                    write_test_frame(&mut peer, &(libc::EIO as u32).to_be_bytes());
                }
            }
            send_count.send(requests).unwrap();
        });

        let mut fs = HomeFs::new(
            "B".to_owned(),
            center_endpoint,
            PathBuf::from("/tmp/local"),
            PathBuf::from("/tmp/peers"),
            Arc::new(RwLock::new(HashSet::new())),
            BackendMode::P2p,
            "token".to_owned(),
        );
        fs.owners.insert("job".to_owned(), "A".to_owned());
        fs.p2p_endpoints.insert("A".to_owned(), endpoint);
        assert!(
            fs.open_existing("/job/x", libc::O_WRONLY | libc::O_TRUNC, false)
                .is_err()
        );
        assert_eq!(
            receive_count.recv_timeout(Duration::from_secs(3)).unwrap(),
            1
        );
        center.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn stat_link_count_accepts_platform_width_and_saturates() {
        assert_eq!(nlink_as_u32(7_u32), 7);
        assert_eq!(nlink_as_u32(7_u64), 7);
        assert_eq!(nlink_as_u32(u64::from(u32::MAX) + 1), u32::MAX);
    }

    #[test]
    fn backend_mode_rejects_implicit_fallback() {
        assert_eq!(BackendMode::parse("nfs").unwrap(), BackendMode::Nfs);
        assert_eq!(BackendMode::parse("p2p").unwrap(), BackendMode::P2p);
        assert!(BackendMode::parse("auto").is_err());
    }

    #[test]
    fn remote_nfs_requires_pre_mounted_peer() {
        let mut fs = HomeFs::new(
            "A".to_owned(),
            "unused".to_owned(),
            PathBuf::from("/tmp/local"),
            PathBuf::from("/tmp/peers"),
            Arc::new(RwLock::new(HashSet::new())),
            BackendMode::Nfs,
            "token".to_owned(),
        );
        fs.owners.insert("job".to_owned(), "B".to_owned());
        assert_eq!(fs.location("/job/file").unwrap_err(), libc::EHOSTUNREACH);
    }

    #[test]
    fn remote_p2p_does_not_require_nfs_mount() {
        let mut fs = HomeFs::new(
            "A".to_owned(),
            "unused".to_owned(),
            PathBuf::from("/tmp/local"),
            PathBuf::from("/tmp/peers"),
            Arc::new(RwLock::new(HashSet::new())),
            BackendMode::P2p,
            "token".to_owned(),
        );
        fs.owners.insert("job".to_owned(), "B".to_owned());
        assert_eq!(
            fs.location("/job/file").unwrap(),
            Location::RemoteP2p("B".to_owned())
        );
    }

    #[test]
    fn private_attribute_ttl_ends_when_root_becomes_shared() {
        let mut fs = HomeFs::new(
            "A".to_owned(),
            "unused".to_owned(),
            PathBuf::from("/tmp/local"),
            PathBuf::from("/tmp/peers"),
            Arc::new(RwLock::new(HashSet::new())),
            BackendMode::P2p,
            "token".to_owned(),
        );
        fs.owners.insert("job".to_owned(), "A".to_owned());
        let mut shared = fs.private_cache.shared_roots.lock().unwrap();
        assert_eq!(fs.attr_ttl("/job/file", &shared), &PRIVATE_ATTR_TTL);
        assert_eq!(fs.attr_ttl("/", &shared), &PRIVATE_ATTR_TTL);
        shared.insert("job".to_owned());
        assert_eq!(fs.attr_ttl("/job/file", &shared), &TTL);
    }

    #[test]
    fn directory_rename_moves_descendants_and_drops_replaced_tree() {
        let mut ids = HashMap::from([
            ("/job/old".to_owned(), 2),
            ("/job/old/file".to_owned(), 3),
            ("/job/new".to_owned(), 4),
            ("/job/new/stale".to_owned(), 5),
        ]);
        let mut paths = ids.iter().map(|(path, ino)| (*ino, path.clone())).collect();
        shift_paths(&mut ids, &mut paths, "/job/old", "/job/new", true);
        assert_eq!(ids.get("/job/new"), Some(&2));
        assert_eq!(ids.get("/job/new/file"), Some(&3));
        assert!(!paths.contains_key(&4));
        assert!(!paths.contains_key(&5));
    }

    #[test]
    fn cross_home_paths_are_detectable_before_rename() {
        assert_ne!(
            HomeFs::root_name("/job-a/file").unwrap(),
            HomeFs::root_name("/job-b/file").unwrap()
        );
    }

    #[test]
    fn root_lookup_parser_separates_active_pending_and_delete_states() {
        assert_eq!(parse_root_lookup("MISSING").unwrap(), RootLookup::Missing);
        assert_eq!(
            parse_root_lookup("A").unwrap(),
            RootLookup::Active("A".to_owned())
        );
        assert_eq!(
            parse_root_lookup("PENDING A").unwrap(),
            RootLookup::Pending("A".to_owned())
        );
        assert_eq!(
            parse_root_lookup("DELETING A").unwrap(),
            RootLookup::Deleting("A".to_owned())
        );
        assert_eq!(
            parse_root_lookup("TOMBSTONE A").unwrap(),
            RootLookup::Tombstone
        );
        assert_eq!(parse_root_lookup("PENDING bad/id").unwrap_err(), libc::EIO);
    }

    #[test]
    fn p2p_connection_loss_errors_are_not_reusable_handle_errors() {
        for error in [
            libc::EPIPE,
            libc::ECONNRESET,
            libc::ECONNABORTED,
            libc::ETIMEDOUT,
            libc::ENOTCONN,
        ] {
            assert!(connection_lost(error));
        }
        assert!(!connection_lost(libc::ENOENT));
        assert_eq!(conflict_status("DELETING A"), libc::EAGAIN);
    }

    #[test]
    fn checked_paths_reject_symlink_parent_escape() {
        let base = test_dir("nofollow-base");
        let outside = test_dir("nofollow-outside");
        fs::write(outside.join("secret"), b"outside").unwrap();
        symlink(&outside, base.join("job")).unwrap();

        assert_path_escape_rejected(checked_open(&base, "/job/secret", libc::O_RDONLY, 0));
        assert_path_escape_rejected(checked_attr(&base, "/job/secret", 2));
        assert_path_escape_rejected(checked_read_dir(&base, "/job"));
        assert_path_escape_rejected(checked_mkdir(&base, "/job/new", 0o755));
        assert_path_escape_rejected(checked_rename(&base, "/job/secret", "/job/moved"));
        assert_eq!(fs::read(outside.join("secret")).unwrap(), b"outside");

        cleanup(base);
        cleanup(outside);
    }

    #[test]
    fn checked_paths_reject_final_symlink_as_file_or_dir() {
        let base = test_dir("nofollow-final-base");
        let outside = test_dir("nofollow-final-outside");
        fs::create_dir(base.join("job")).unwrap();
        fs::write(outside.join("secret"), b"outside").unwrap();
        symlink(outside.join("secret"), base.join("job/link")).unwrap();

        assert_path_escape_rejected(checked_open(&base, "/job/link", libc::O_RDONLY, 0));
        assert_eq!(
            checked_attr(&base, "/job/link", 2).unwrap_err(),
            libc::EOPNOTSUPP
        );
        assert_eq!(
            checked_read_dir(&base, "/job").unwrap_err(),
            libc::EOPNOTSUPP
        );

        cleanup(base);
        cleanup(outside);
    }

    #[test]
    fn checked_open_accepts_fuse_create_mode_with_file_type() {
        let base = test_dir("fuse-create-mode");
        fs::create_dir(base.join("job")).unwrap();
        let file = checked_open(
            &base,
            "/job/new",
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY,
            libc::S_IFREG | 0o644,
        )
        .unwrap();
        drop(file);
        assert!(base.join("job/new").is_file());
        cleanup(base);
    }

    fn assert_path_escape_rejected<T>(result: Result<T, i32>) {
        let error = match result {
            Ok(_) => panic!("symlink path must not be followed"),
            Err(error) => error,
        };
        assert!(
            matches!(error, libc::ELOOP | libc::ENOTDIR | libc::EOPNOTSUPP),
            "unexpected errno {error}"
        );
    }

    fn test_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!(
            "dms-home-fuse-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(path: PathBuf) {
        let _ = fs::remove_dir_all(path);
    }
}

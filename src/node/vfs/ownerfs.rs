//! OwnerFs：可修改、以根归属和计算亲和为核心的文件后端。
//!
//! 一级目录是 OwnerFs 的授权单位；根内文件仍是本机普通文件。本地 Home
//! 热路径只检查已缓存的 RootGrant，然后直接调用 LocalFs，不把每次写转换为
//! Blob，也不逐写访问 Meta。远端/P2P 后续复用相同文件身份与句柄语义。

use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{FileTypeExt, MetadataExt},
    },
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use afs_error::{Error, Result};
use fuser::Notifier;

use self::root::{PresentedRootAccess, RootGrant, RootId, RootManager, RootRight};
use super::{
    Backend, CreateRequest, Namespace,
    types::{
        AttributeChange, BackendInode, CreatedFile, DirectoryEntry, DirectoryHandle, Entry,
        FileAttributes, FileHandle, FileKind, RenameFlags, RequestContext, SyncMode,
    },
};
use crate::node::storage::{
    DirectoryHandle as StorageDirectoryHandle, FileHandle as StorageFileHandle, FileStore, LocalFs,
    OpenSpec, RenameMode, StoragePath,
};

pub mod catalog;
pub mod files;
pub mod remote;
pub mod root;

const OWNERFS_ROOT_INODE: u64 = 1;

/// OwnerFs backend entry. `new()` remains an unsupported skeleton for the
/// current VFS bootstrap; `new_local()` is the real local implementation used
/// by tests and later Node startup wiring.
pub struct OwnerFs {
    local: Option<Arc<LocalOwnerFs>>,
    private_cache: Arc<Mutex<PrivateFuseCache>>,
}

// Kernel page/attribute cache is profitable only while this node is the sole
// reader/writer of a Home root. First authenticated peer access invalidates all
// known FUSE inodes before granting that access. The state stays shared for the
// rest of this daemon session; there is no speculative switch back to private.
struct PrivateFuseCache {
    shared_roots: HashSet<RootId>,
    notifier: Option<Notifier>,
    next_fuse_ino: u64,
}

impl PrivateFuseCache {
    fn new() -> Self {
        Self {
            shared_roots: HashSet::new(),
            notifier: None,
            next_fuse_ino: 4,
        }
    }
}

impl Default for OwnerFs {
    fn default() -> Self {
        Self::new()
    }
}

impl OwnerFs {
    #[must_use]
    pub fn new() -> Self {
        Self {
            local: None,
            private_cache: Arc::new(Mutex::new(PrivateFuseCache::new())),
        }
    }

    /// Construct the local Home implementation.
    ///
    /// Node startup should pass the same `LocalFs` root used by `RootManager`.
    /// OwnerFs never stores Meta directly; all coarse authority is behind
    /// RootManager, and file operations receive only short-lived `RootUse`s.
    #[must_use]
    pub fn new_local(roots: Arc<RootManager>, disk: Arc<LocalFs>) -> Self {
        let private_cache = Arc::new(Mutex::new(PrivateFuseCache::new()));
        Self {
            local: Some(Arc::new(LocalOwnerFs::new(
                roots,
                disk,
                None,
                private_cache.clone(),
            ))),
            private_cache,
        }
    }

    /// Construct OwnerFs with B-side remote dispatch enabled.
    #[must_use]
    pub fn new_local_with_remote(
        roots: Arc<RootManager>,
        disk: Arc<LocalFs>,
        remote_factory: Arc<dyn RemoteFilesFactory>,
    ) -> Self {
        let private_cache = Arc::new(Mutex::new(PrivateFuseCache::new()));
        Self {
            local: Some(Arc::new(LocalOwnerFs::new(
                roots,
                disk,
                Some(remote_factory),
                private_cache.clone(),
            ))),
            private_cache,
        }
    }

    /// The mount owns the notifier. Register it before the peer service starts
    /// accepting requests so first share can invalidate private kernel cache.
    pub fn register_fuse_notifier(&self, notifier: Notifier) {
        self.private_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .notifier = Some(notifier);
    }

    pub(crate) fn remember_fuse_inode(&self, ino: u64) {
        let mut cache = self
            .private_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.next_fuse_ino = cache.next_fuse_ino.max(ino.saturating_add(1));
    }

    /// An existing inode whose Home is this node uses a local file handle.
    /// This differs from the private-cache decision: another node may be
    /// sharing the root while this node still reads its own ordinary file.
    pub(crate) fn is_local_inode(&self, inode: BackendInode) -> bool {
        self.local
            .as_ref()
            .and_then(|local| local.private_root_for_inode(inode.value))
            .is_some()
    }

    /// Periodic Home-side cleanup for peer process sessions that Meta has
    /// authoritatively expired or replaced. Meta errors retain every handle.
    /// This runs off the FUSE/data hot path.
    pub fn reap_expired_peer_sessions(&self) -> Result<usize> {
        let local = self.require_local()?;
        let mut reclaimed = 0;
        for (node_id, session_id) in local.peer_handle_sessions()? {
            if local.roots.current_node_session(&node_id)?.as_deref() != Some(&session_id) {
                reclaimed += local.reap_peer_session(&node_id, &session_id)?;
            }
        }
        Ok(reclaimed)
    }

    /// Hold the cache lock through the FUSE reply. Otherwise a peer could
    /// invalidate, then a delayed local reply could reintroduce a private TTL.
    pub(crate) fn with_fuse_cache_policy<T>(
        &self,
        inode: BackendInode,
        reply: impl FnOnce(Duration, bool) -> T,
    ) -> T {
        let cache = self
            .private_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let private = self
            .local
            .as_ref()
            .and_then(|local| local.private_root_for_inode(inode.value))
            .is_some_and(|id| !cache.shared_roots.contains(&id));
        let ttl = if private {
            Duration::from_secs(1)
        } else {
            Duration::ZERO
        };
        reply(ttl, private)
    }

    fn require_local(&self) -> Result<&LocalOwnerFs> {
        self.local.as_deref().ok_or_else(|| {
            Error::coded(
                afs_error::NODE_VFS_UNIMPLEMENTED,
                "OwnerFs local file operations are not wired in this VFS instance",
            )
        })
    }

    /// Build a Home-side file executor for the P2P OwnerFiles service.
    ///
    /// The RPC adapter remains responsible for Proto conversion and channel
    /// authentication. This executor accepts only business types and reuses the
    /// same local file table as FUSE, so opened remote handles keep old-file FD
    /// semantics across unlink/recreate.
    pub fn peer_executor(&self) -> Result<OwnerFsPeerExecutor> {
        let local = self.local.as_ref().cloned().ok_or_else(|| {
            Error::coded(
                afs_error::NODE_VFS_UNIMPLEMENTED,
                "OwnerFs local file operations are not wired in this VFS instance",
            )
        })?;
        Ok(OwnerFsPeerExecutor { local })
    }
}

/// Home-side OwnerFs executor used by node-to-node RPC handlers.
///
/// Each path operation validates the presented root grant once through
/// `RootManager`; the hot file action then runs against ordinary local files.
/// Open handles are encoded as opaque little-endian `u64` values scoped to this
/// afs-node process session. They become stale after release or process restart.
#[derive(Clone)]
pub struct OwnerFsPeerExecutor {
    local: Arc<LocalOwnerFs>,
}

impl OwnerFsPeerExecutor {
    pub fn lookup(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected_parent: Option<&files::FileIdentity>,
    ) -> Result<files::OwnerEntry> {
        self.local
            .peer_lookup(peer_node_id, access, path, expected_parent)
    }

    pub fn getattr(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        file: Option<&files::RemoteFile>,
    ) -> Result<files::OwnerEntry> {
        self.local
            .peer_getattr(peer_node_id, access, path, expected, file)
    }

    pub fn create(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        mode: u32,
        expected_parent: &files::FileIdentity,
    ) -> Result<remote::RemoteCreatedFile> {
        self.local
            .peer_create(peer_node_id, access, path, flags, mode, expected_parent)
    }

    pub fn mkdir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        mode: u32,
        expected_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.local
            .peer_mkdir(peer_node_id, access, path, mode, expected_parent)
    }

    pub fn setattr(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        file: Option<&files::RemoteFile>,
        change: &AttributeChange,
    ) -> Result<files::OwnerEntry> {
        self.local
            .peer_setattr(peer_node_id, access, path, expected, file, change)
    }

    pub fn unlink(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        expected_parent: &files::FileIdentity,
    ) -> Result<()> {
        self.local
            .peer_unlink(peer_node_id, access, path, expected, expected_parent)
    }

    pub fn rmdir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        expected_parent: &files::FileIdentity,
    ) -> Result<()> {
        self.local
            .peer_rmdir(peer_node_id, access, path, expected, expected_parent)
    }

    // Keep the agreed peer contract explicit instead of allocating a request
    // object just to satisfy the argument-count lint on this cold operation.
    #[allow(clippy::too_many_arguments)]
    pub fn rename(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        old_path: &OsStr,
        new_path: &OsStr,
        expected_old: Option<&files::FileIdentity>,
        expected_new: Option<&files::FileIdentity>,
        expected_old_parent: &files::FileIdentity,
        expected_new_parent: &files::FileIdentity,
        flags: RenameFlags,
    ) -> Result<()> {
        self.local.peer_rename(
            peer_node_id,
            access,
            old_path,
            new_path,
            expected_old,
            expected_new,
            expected_old_parent,
            expected_new_parent,
            flags,
        )
    }

    pub fn open(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        expected: Option<&files::FileIdentity>,
    ) -> Result<(files::RemoteFile, FileAttributes, Option<Vec<u8>>)> {
        self.local
            .peer_open(peer_node_id, access, path, flags, expected)
    }

    pub fn read(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize> {
        self.local
            .peer_read(peer_node_id, access, file, offset, out)
    }

    pub fn write(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        offset: u64,
        data: &[u8],
    ) -> Result<usize> {
        self.local
            .peer_write(peer_node_id, access, file, offset, data)
    }

    pub fn flush(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
    ) -> Result<()> {
        self.local.peer_flush(peer_node_id, access, file)
    }

    pub fn fsync(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        data_only: bool,
    ) -> Result<()> {
        self.local.peer_fsync(peer_node_id, access, file, data_only)
    }

    pub fn release(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: files::RemoteFile,
    ) -> Result<()> {
        self.local.peer_release(peer_node_id, access, file)
    }

    pub fn opendir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
    ) -> Result<files::RemoteDirectory> {
        self.local
            .peer_opendir(peer_node_id, access, path, expected)
    }

    pub fn readdir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        directory: &files::RemoteDirectory,
        cookie: u64,
        max_entries: usize,
    ) -> Result<Vec<remote::RemoteDirectoryEntry>> {
        self.local
            .peer_readdir(peer_node_id, access, directory, cookie, max_entries)
    }

    pub fn fsyncdir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        directory: &files::RemoteDirectory,
        data_only: bool,
    ) -> Result<()> {
        self.local
            .peer_fsyncdir(peer_node_id, access, directory, data_only)
    }

    pub fn releasedir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        directory: files::RemoteDirectory,
    ) -> Result<()> {
        self.local.peer_releasedir(peer_node_id, access, directory)
    }

    pub fn readlink(
        &self,
        _peer_node_id: &str,
        _access: &PresentedRootAccess,
        _path: &OsStr,
        _expected: Option<&files::FileIdentity>,
    ) -> Result<Vec<u8>> {
        Err(Error::coded(
            afs_error::NODE_VFS_UNIMPLEMENTED,
            "OwnerFs symlink/readlink is not wired yet",
        ))
    }
}

/// Resolves a Home node endpoint into a `RemoteFiles` client.
///
/// Node wiring can back this with `RootMeta::lookup_node_endpoint` plus
/// `node::rpc::peer::connect_owner_files_client`. Tests can inject an in-memory
/// implementation. OwnerFs keeps the trait here so B-side remote dispatch does
/// not depend on Proto/gRPC/RDMA types.
pub trait RemoteFilesFactory: Send + Sync {
    fn connect(&self, home_node_id: &str) -> Result<Arc<dyn remote::RemoteFiles>>;
}

impl Backend for OwnerFs {
    fn namespace(&self) -> Namespace {
        Namespace::OwnerFs
    }

    fn probe_create(&self, request: &CreateRequest) -> Result<()> {
        afs_logging::info!("ownerfs.create"; "namespace" => request.namespace.as_str(), "path" => request.name.as_str());
        Err(Error::coded(
            afs_error::NODE_VFS_UNIMPLEMENTED,
            "OwnerFs probe_create is a bootstrap diagnostic, not a real file operation",
        ))
    }

    fn lookup(&self, _: &RequestContext, parent: BackendInode, name: &OsStr) -> Result<Entry> {
        self.require_local()?.lookup(parent, name)
    }

    fn getattr(
        &self,
        _: &RequestContext,
        inode: BackendInode,
        handle: Option<FileHandle>,
    ) -> Result<FileAttributes> {
        self.require_local()?.getattr(inode, handle)
    }

    fn setattr(
        &self,
        _: &RequestContext,
        inode: BackendInode,
        handle: Option<FileHandle>,
        change: &AttributeChange,
    ) -> Result<FileAttributes> {
        self.require_local()?.setattr(inode, handle, change)
    }

    fn create(
        &self,
        _: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
        flags: i32,
    ) -> Result<CreatedFile> {
        self.require_local()?.create(parent, name, mode, flags)
    }

    fn open(&self, _: &RequestContext, inode: BackendInode, flags: i32) -> Result<FileHandle> {
        self.require_local()?.open(inode, flags)
    }

    fn read(
        &self,
        _: &RequestContext,
        handle: FileHandle,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize> {
        self.require_local()?.read(handle, offset, out)
    }

    fn write(
        &self,
        _: &RequestContext,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize> {
        self.require_local()?.write(handle, offset, data)
    }

    fn flush(&self, _: &RequestContext, handle: FileHandle) -> Result<()> {
        self.require_local()?.flush(handle)
    }

    fn fsync(&self, _: &RequestContext, handle: FileHandle, mode: SyncMode) -> Result<()> {
        self.require_local()?.fsync(handle, mode)
    }

    fn release(&self, _: &RequestContext, handle: FileHandle) -> Result<()> {
        self.require_local()?.release(handle)
    }

    fn opendir(&self, _: &RequestContext, inode: BackendInode) -> Result<DirectoryHandle> {
        self.require_local()?.opendir(inode)
    }

    fn readdir(
        &self,
        _: &RequestContext,
        handle: DirectoryHandle,
        cookie: u64,
        max_entries: usize,
    ) -> Result<Vec<DirectoryEntry>> {
        self.require_local()?.readdir(handle, cookie, max_entries)
    }

    fn fsyncdir(&self, _: &RequestContext, handle: DirectoryHandle, mode: SyncMode) -> Result<()> {
        self.require_local()?.fsyncdir(handle, mode)
    }

    fn releasedir(&self, _: &RequestContext, handle: DirectoryHandle) -> Result<()> {
        self.require_local()?.releasedir(handle)
    }

    fn mkdir(
        &self,
        _: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
    ) -> Result<Entry> {
        self.require_local()?.mkdir(parent, name, mode)
    }

    fn unlink(&self, _: &RequestContext, parent: BackendInode, name: &OsStr) -> Result<()> {
        self.require_local()?.unlink(parent, name)
    }

    fn rmdir(&self, _: &RequestContext, parent: BackendInode, name: &OsStr) -> Result<()> {
        self.require_local()?.rmdir(parent, name)
    }

    fn rename(
        &self,
        _: &RequestContext,
        from_parent: BackendInode,
        from_name: &OsStr,
        to_parent: BackendInode,
        to_name: &OsStr,
        flags: RenameFlags,
    ) -> Result<()> {
        self.require_local()?
            .rename(from_parent, from_name, to_parent, to_name, flags)
    }
}

struct LocalOwnerFs {
    roots: Arc<RootManager>,
    disk: Arc<LocalFs>,
    remote_factory: Option<Arc<dyn RemoteFilesFactory>>,
    remote_roots: Mutex<HashMap<RootId, RemoteRoot>>,
    // Serializes Home namespace changes with identity checks made by peer RPCs.
    // File read/write handles do not take this lock.
    namespace_lock: Mutex<()>,
    // Serialize first-share invalidation without holding private_cache across
    // FUSE writes. A second peer must wait for the first invalidation to finish.
    share_lock: Mutex<()>,
    state: Mutex<OwnerState>,
    private_cache: Arc<Mutex<PrivateFuseCache>>,
}

#[derive(Clone)]
struct RemoteRoot {
    grant: RootGrant,
    files: Arc<dyn remote::RemoteFiles>,
}

impl LocalOwnerFs {
    fn new(
        roots: Arc<RootManager>,
        disk: Arc<LocalFs>,
        remote_factory: Option<Arc<dyn RemoteFilesFactory>>,
        private_cache: Arc<Mutex<PrivateFuseCache>>,
    ) -> Self {
        let mut state = OwnerState::new();
        for root in roots.cached_local_roots().unwrap_or_default() {
            if let Ok(attributes) = disk
                .metadata(&root.data_dir)
                .map_err(Error::from)
                .and_then(attributes_from_metadata)
            {
                let identity = disk
                    .metadata(&root.data_dir)
                    .map_err(Error::from)
                    .and_then(|metadata| identity_from_metadata(&metadata))
                    .unwrap_or_else(|_| identity_from_attributes(&attributes));
                state.insert_root(root.name, root.id, identity, attributes);
            }
        }
        Self {
            roots,
            disk,
            remote_factory,
            remote_roots: Mutex::new(HashMap::new()),
            namespace_lock: Mutex::new(()),
            share_lock: Mutex::new(()),
            state: Mutex::new(state),
            private_cache,
        }
    }

    fn private_root_for_inode(&self, inode: u64) -> Option<RootId> {
        let id = self.state.lock().ok()?.inodes.get(&inode)?.root_id.clone();
        self.roots.has_active_local_root(&id).then_some(id)
    }

    fn remote_for_record(&self, record: &NodeRecord, right: RootRight) -> Result<RemoteRoot> {
        self.acquire_remote_root(&record.root_id, right)
    }

    fn remote_expected_identity(
        &self,
        root_id: &RootId,
        relative: &StoragePath,
    ) -> Result<Option<files::FileIdentity>> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        Ok(state
            .paths
            .get(&(root_id.clone(), relative.clone()))
            .and_then(|inode| state.inodes.get(inode))
            .map(|record| record.identity.clone()))
    }

    fn open_file_handle(&self, handle: FileHandle) -> Result<Arc<Mutex<OpenFileHandleSlot>>> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        state
            .file_handles
            .get(&handle)
            .cloned()
            .ok_or_else(|| stale("unknown file handle"))
    }

    fn check_peer_file_handle(
        &self,
        handle: FileHandle,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        identity: &files::FileIdentity,
    ) -> Result<()> {
        self.open_file_handle(handle)?
            .lock()
            .map_err(|_| poisoned())?
            .check_peer(peer_node_id, access, identity)
    }

    fn check_peer_directory_handle(
        &self,
        handle: DirectoryHandle,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        identity: &files::FileIdentity,
    ) -> Result<()> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        let directory = state
            .dir_handles
            .get(&handle)
            .ok_or_else(|| stale("unknown directory handle"))?;
        let OpenLocalDirectory::Local(local) = &directory.handle else {
            return Err(stale("peer directory handle is not local to Home"));
        };
        if local.root_id != access.id
            || (!identity.0.is_empty() && local.identity != *identity)
            || local
                .peer
                .as_ref()
                .is_none_or(|scope| scope.node_id != peer_node_id || scope.access != *access)
        {
            return Err(stale(
                "peer directory handle belongs to another open or grant",
            ));
        }
        Ok(())
    }

    fn peer_handle_sessions(&self) -> Result<HashSet<(String, String)>> {
        let mut sessions = self.roots.cached_peer_sessions()?;
        let (files, directories) = {
            let state = self.state.lock().map_err(|_| poisoned())?;
            (
                state.file_handles.values().cloned().collect::<Vec<_>>(),
                state
                    .dir_handles
                    .values()
                    .filter_map(|open| match &open.handle {
                        OpenLocalDirectory::Local(local) => local.peer.clone(),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            )
        };
        for scope in directories {
            sessions.insert((scope.node_id, scope.access.session_id));
        }
        for slot in files {
            let slot = slot.lock().map_err(|_| poisoned())?;
            if let OpenFileHandle::Local(local) = &slot.file
                && let Some(scope) = &local.handle.peer
            {
                sessions.insert((scope.node_id.clone(), scope.access.session_id.clone()));
            }
        }
        Ok(sessions)
    }

    fn reap_peer_session(&self, node_id: &str, session_id: &str) -> Result<usize> {
        self.roots.fence_peer_session(node_id, session_id)?;
        let files = {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            state
                .fenced_peer_sessions
                .insert((node_id.to_owned(), session_id.to_owned()));
            state.dir_handles.retain(|_, open| match &open.handle {
                OpenLocalDirectory::Local(local) => local.peer.as_ref().is_none_or(|scope| {
                    scope.node_id != node_id || scope.access.session_id != session_id
                }),
                _ => true,
            });
            state
                .file_handles
                .iter()
                .map(|(id, slot)| (*id, slot.clone()))
                .collect::<Vec<_>>()
        };
        let mut reclaimed = 0;
        for (id, slot) in files {
            let belongs = {
                let slot = slot.lock().map_err(|_| poisoned())?;
                matches!(&slot.file, OpenFileHandle::Local(local)
                    if local.handle.peer.as_ref().is_some_and(|scope|
                        scope.node_id == node_id && scope.access.session_id == session_id))
            };
            if !belongs {
                continue;
            }
            if self
                .state
                .lock()
                .map_err(|_| poisoned())?
                .file_handles
                .remove(&id)
                .is_some()
            {
                // A prior operation that already cloned this slot must finish
                // before we mark it closed; any later one sees STALE.
                slot.lock().map_err(|_| poisoned())?.closed = true;
                reclaimed += 1;
            }
        }
        Ok(reclaimed)
    }

    /// A peer names a parent by path, but its FUSE inode denotes a particular
    /// directory object. Check the actual opened parent under namespace_lock
    /// before applying a child mutation, so rename + same-name recreation
    /// cannot redirect that mutation into a different directory.
    fn check_peer_parent(
        &self,
        data_dir: &StoragePath,
        parent: &StoragePath,
        expected: &files::FileIdentity,
    ) -> Result<()> {
        // The workspace root cannot be renamed inside OwnerFs. Its grant and
        // epoch fence recreation, so the common root-level hot path needs no
        // additional disk lookup for each file operation.
        if parent.is_root() {
            return Ok(());
        }
        let physical = data_dir.join_path(parent).map_err(Error::from)?;
        let dir = self.disk.open_dir(&physical).map_err(Error::from)?;
        let actual = identity_from_metadata(&dir.metadata().map_err(Error::from)?)?;
        check_expected_identity(Some(expected), &actual)
    }

    fn peer_lookup(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected_parent: Option<&files::FileIdentity>,
    ) -> Result<files::OwnerEntry> {
        self.validate_peer(access, peer_node_id, RootRight::Lookup)?;
        let relative = storage_path_from_os(path)?;
        if !relative.is_root() {
            let (parent, _) = split_parent_name(path)?;
            if !parent.is_root() {
                let expected = expected_parent.ok_or_else(|| {
                    Error::coded(
                        afs_error::NODE_VFS_INVALID,
                        "lookup under a nested directory requires its identity",
                    )
                })?;
                let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
                let root_use = self.roots.enter_root(&access.id, RootRight::Lookup)?;
                self.check_peer_parent(root_use.data_dir(), &parent, expected)?;
                return self.owner_entry_for_path(&access.id, relative, None);
            }
        }
        self.owner_entry_for_path(&access.id, relative, None)
    }

    fn peer_getattr(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        file: Option<&files::RemoteFile>,
    ) -> Result<files::OwnerEntry> {
        self.validate_peer(access, peer_node_id, RootRight::Lookup)?;
        if let Some(file) = file {
            check_remote_file_scope(access, file)?;
            let handle = decode_file_handle(file)?;
            let open = self.open_file_handle(handle)?;
            let open = open.lock().map_err(|_| poisoned())?;
            open.check_peer(peer_node_id, access, &file.identity)?;
            let OpenFileHandle::Local(local) = &open.file else {
                return Err(stale("Home peer handle is not local to this OwnerFs"));
            };
            check_expected_identity(expected, &local.handle.identity)?;
            let attributes = local.attributes()?;
            return Ok(files::OwnerEntry {
                root_id: access.id.clone(),
                identity: local.handle.identity.clone(),
                attributes,
            });
        }
        self.owner_entry_for_path(&access.id, storage_path_from_os(path)?, expected)
    }

    fn peer_create(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        mode: u32,
        expected_parent: &files::FileIdentity,
    ) -> Result<remote::RemoteCreatedFile> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let (parent, name) = split_parent_name(path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        self.check_peer_parent(root_use.data_dir(), &parent, expected_parent)?;
        let child = parent.join_component(&name).map_err(Error::from)?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        let file = self
            .disk
            .open_file(&physical, OpenSpec::new(flags | libc::O_CREAT, mode))
            .map_err(Error::from)?;
        let metadata = file.metadata().map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.ensure_peer_session(peer_node_id, access)?;
        state.inode_for_path(
            access.id.clone(),
            child,
            identity.clone(),
            attributes.clone(),
            attributes.kind,
        );
        let local_handle = state.insert_file_handle(files::LocalOpenFile {
            root_id: access.id.clone(),
            identity: identity.clone(),
            file,
            peer: Some(files::PeerOpenScope {
                node_id: peer_node_id.to_owned(),
                access: access.clone(),
            }),
        });
        Ok(remote::RemoteCreatedFile {
            entry: files::OwnerEntry {
                root_id: access.id.clone(),
                identity: identity.clone(),
                attributes: attributes.clone(),
            },
            file: remote_file(access, identity, local_handle),
        })
    }

    fn peer_mkdir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        mode: u32,
        expected_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let (parent, _) = split_parent_name(path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        self.check_peer_parent(root_use.data_dir(), &parent, expected_parent)?;
        let relative = storage_path_from_os(path)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        self.disk.mkdir(&physical, mode).map_err(Error::from)?;
        let dir = self.disk.open_dir(&physical).map_err(Error::from)?;
        dir.sync_all().map_err(Error::from)?;
        self.owner_entry_for_path(&access.id, relative, None)
    }

    fn peer_setattr(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        file: Option<&files::RemoteFile>,
        change: &AttributeChange,
    ) -> Result<files::OwnerEntry> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        if let Some(file) = file {
            check_remote_file_scope(access, file)?;
            let handle = decode_file_handle(file)?;
            self.check_peer_file_handle(handle, peer_node_id, access, &file.identity)?;
            let identity = {
                let slot = self.open_file_handle(handle)?;
                let slot = slot.lock().map_err(|_| poisoned())?;
                let OpenFileHandle::Local(local) = &slot.file else {
                    return Err(stale("Home peer handle is not local to this OwnerFs"));
                };
                local.handle.identity.clone()
            };
            let attributes =
                self.setattr(backend_inode(OWNERFS_ROOT_INODE), Some(handle), change)?;
            return Ok(files::OwnerEntry {
                root_id: access.id.clone(),
                identity,
                attributes,
            });
        }
        let entry = self.owner_entry_for_path(&access.id, storage_path_from_os(path)?, expected)?;
        let inode = self
            .state
            .lock()
            .map_err(|_| poisoned())?
            .paths
            .get(&(entry.root_id.clone(), storage_path_from_os(path)?))
            .copied()
            .ok_or_else(|| stale("peer setattr inode missing"))?;
        let attributes = self.setattr(backend_inode(inode), None, change)?;
        self.owner_entry_for_path(
            &access.id,
            storage_path_from_os(path)?,
            Some(&entry.identity),
        )
        .map(|mut refreshed| {
            refreshed.attributes = attributes;
            refreshed
        })
    }

    fn peer_unlink(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        expected_parent: &files::FileIdentity,
    ) -> Result<()> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let relative = storage_path_from_os(path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        let (parent, _) = split_parent_name(path)?;
        self.check_peer_parent(root_use.data_dir(), &parent, expected_parent)?;
        self.owner_entry_for_path(&access.id, relative.clone(), expected)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        self.disk.remove_file(&physical).map_err(Error::from)?;
        self.state
            .lock()
            .map_err(|_| poisoned())?
            .paths
            .remove(&(access.id.clone(), relative));
        Ok(())
    }

    fn peer_rmdir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        expected_parent: &files::FileIdentity,
    ) -> Result<()> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let relative = storage_path_from_os(path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        let (parent, _) = split_parent_name(path)?;
        self.check_peer_parent(root_use.data_dir(), &parent, expected_parent)?;
        self.owner_entry_for_path(&access.id, relative.clone(), expected)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        self.disk.remove_dir(&physical).map_err(Error::from)?;
        self.state
            .lock()
            .map_err(|_| poisoned())?
            .paths
            .remove(&(access.id.clone(), relative));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn peer_rename(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        old_path: &OsStr,
        new_path: &OsStr,
        expected_old: Option<&files::FileIdentity>,
        expected_new: Option<&files::FileIdentity>,
        expected_old_parent: &files::FileIdentity,
        expected_new_parent: &files::FileIdentity,
        flags: RenameFlags,
    ) -> Result<()> {
        let rename_mode = rename_mode_from_flags(flags)?;
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let old_relative = storage_path_from_os(old_path)?;
        let new_relative = storage_path_from_os(new_path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        let (old_parent, _) = split_parent_name(old_path)?;
        let (new_parent, _) = split_parent_name(new_path)?;
        self.check_peer_parent(root_use.data_dir(), &old_parent, expected_old_parent)?;
        self.check_peer_parent(root_use.data_dir(), &new_parent, expected_new_parent)?;
        self.owner_entry_for_path(&access.id, old_relative.clone(), expected_old)?;
        if expected_new.is_some() {
            self.owner_entry_for_path(&access.id, new_relative.clone(), expected_new)?;
        }
        let physical_old = root_use
            .data_dir()
            .join_path(&old_relative)
            .map_err(Error::from)?;
        let physical_new = root_use
            .data_dir()
            .join_path(&new_relative)
            .map_err(Error::from)?;
        self.disk
            .rename(&physical_old, &physical_new, rename_mode)
            .map_err(Error::from)?;
        self.state.lock().map_err(|_| poisoned())?.rename_path(
            access.id.clone(),
            old_relative,
            new_relative,
        );
        Ok(())
    }

    fn peer_opendir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
    ) -> Result<files::RemoteDirectory> {
        self.validate_peer(access, peer_node_id, RootRight::Lookup)?;
        let relative = storage_path_from_os(path)?;
        let entry = self.owner_entry_for_path(&access.id, relative.clone(), expected)?;
        if entry.attributes.kind != FileKind::Directory {
            return Err(Error::from(std::io::Error::from(
                std::io::ErrorKind::NotADirectory,
            )));
        }
        let root_use = self.roots.enter_root(&access.id, RootRight::Lookup)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        let directory = self.disk.open_dir(&physical).map_err(Error::from)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.ensure_peer_session(peer_node_id, access)?;
        let inode = state.inode_for_path(
            access.id.clone(),
            relative,
            entry.identity.clone(),
            entry.attributes.clone(),
            FileKind::Directory,
        );
        let handle = state.insert_dir_handle(
            inode,
            OpenLocalDirectory::Local(files::LocalOpenDirectory {
                root_id: access.id.clone(),
                identity: entry.identity.clone(),
                directory,
                peer: Some(files::PeerOpenScope {
                    node_id: peer_node_id.to_owned(),
                    access: access.clone(),
                }),
            }),
        );
        Ok(remote_directory(access, entry.identity, handle))
    }

    fn peer_readdir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        directory: &files::RemoteDirectory,
        cookie: u64,
        max_entries: usize,
    ) -> Result<Vec<remote::RemoteDirectoryEntry>> {
        self.validate_peer(access, peer_node_id, RootRight::Lookup)?;
        check_remote_directory_scope(access, directory)?;
        let handle = decode_directory_handle(directory)?;
        self.check_peer_directory_handle(handle, peer_node_id, access, &directory.identity)?;
        let entries = self.readdir(handle, cookie, max_entries)?;
        entries
            .into_iter()
            .map(|entry| {
                let relative = self
                    .state
                    .lock()
                    .map_err(|_| poisoned())?
                    .inodes
                    .get(&entry.inode.value)
                    .ok_or_else(|| stale("peer readdir entry inode missing"))?
                    .relative
                    .clone();
                Ok(remote::RemoteDirectoryEntry {
                    name: entry.name,
                    entry: self.owner_entry_for_path(&access.id, relative, None)?,
                    next_cookie: entry.next_cookie,
                })
            })
            .collect()
    }

    fn peer_fsyncdir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        directory: &files::RemoteDirectory,
        data_only: bool,
    ) -> Result<()> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        check_remote_directory_scope(access, directory)?;
        self.check_peer_directory_handle(
            decode_directory_handle(directory)?,
            peer_node_id,
            access,
            &directory.identity,
        )?;
        self.fsyncdir(
            decode_directory_handle(directory)?,
            if data_only {
                SyncMode::DataOnly
            } else {
                SyncMode::Full
            },
        )
    }

    fn peer_releasedir(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        directory: files::RemoteDirectory,
    ) -> Result<()> {
        self.validate_peer(access, peer_node_id, RootRight::Lookup)?;
        check_remote_directory_scope(access, &directory)?;
        self.check_peer_directory_handle(
            decode_directory_handle(&directory)?,
            peer_node_id,
            access,
            &directory.identity,
        )?;
        self.releasedir(decode_directory_handle(&directory)?)
    }

    fn peer_open(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        expected: Option<&files::FileIdentity>,
    ) -> Result<(files::RemoteFile, FileAttributes, Option<Vec<u8>>)> {
        let right = if flags & libc::O_ACCMODE == libc::O_RDONLY && flags & libc::O_TRUNC == 0 {
            RootRight::Read
        } else {
            RootRight::Write
        };
        self.validate_peer(access, peer_node_id, right)?;
        let relative = storage_path_from_os(path)?;
        let root_use = self.roots.enter_root(&access.id, right)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        let open_flags = flags & !libc::O_TRUNC;
        let file = self
            .disk
            .open_file(&physical, OpenSpec::new(open_flags, 0))
            .map_err(Error::from)?;
        let metadata = file.metadata().map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        check_expected_identity(expected, &identity)?;
        if flags & libc::O_TRUNC != 0 {
            file.set_len(0).map_err(Error::from)?;
        }
        let attributes = file
            .metadata()
            .map_err(Error::from)
            .and_then(attributes_from_metadata)?;
        // A small read-only OPEN carries the contents in its reply. Read from
        // the just-opened OS file before inserting the handle: a separate
        // peer_read would repeat grant validation and a handle-table lookup.
        // A failed or short speculative read only disables prefetch; OPEN
        // itself still succeeds and the caller can issue a normal READ.
        const OPEN_PREFETCH_LIMIT: u64 = 4096;
        let prefetched_data = if flags & libc::O_ACCMODE == libc::O_RDONLY
            && flags & (libc::O_PATH | libc::O_DIRECT) == 0
            && attributes.size <= OPEN_PREFETCH_LIMIT
        {
            let mut bytes = vec![0; attributes.size as usize];
            match file.read_at(0, &mut bytes) {
                Ok(read) if read == bytes.len() => Some(bytes),
                _ => None,
            }
        } else {
            None
        };
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.ensure_peer_session(peer_node_id, access)?;
        state.inode_for_path(
            access.id.clone(),
            relative,
            identity.clone(),
            attributes.clone(),
            attributes.kind,
        );
        let handle = state.insert_file_handle(files::LocalOpenFile {
            root_id: access.id.clone(),
            identity: identity.clone(),
            file,
            peer: Some(files::PeerOpenScope {
                node_id: peer_node_id.to_owned(),
                access: access.clone(),
            }),
        });
        Ok((
            remote_file(access, identity, handle),
            attributes,
            prefetched_data,
        ))
    }

    fn peer_read(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize> {
        self.validate_peer(access, peer_node_id, RootRight::Read)?;
        check_remote_file_scope(access, file)?;
        let handle = decode_file_handle(file)?;
        self.check_peer_file_handle(handle, peer_node_id, access, &file.identity)?;
        self.read(handle, offset, out)
    }

    fn peer_write(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        offset: u64,
        data: &[u8],
    ) -> Result<usize> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        check_remote_file_scope(access, file)?;
        let handle = decode_file_handle(file)?;
        self.check_peer_file_handle(handle, peer_node_id, access, &file.identity)?;
        self.write(handle, offset, data)
    }

    fn peer_flush(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
    ) -> Result<()> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        check_remote_file_scope(access, file)?;
        let handle = decode_file_handle(file)?;
        self.check_peer_file_handle(handle, peer_node_id, access, &file.identity)?;
        self.flush(handle)
    }

    fn peer_fsync(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        data_only: bool,
    ) -> Result<()> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        check_remote_file_scope(access, file)?;
        let handle = decode_file_handle(file)?;
        self.check_peer_file_handle(handle, peer_node_id, access, &file.identity)?;
        let mode = if data_only {
            SyncMode::DataOnly
        } else {
            SyncMode::Full
        };
        self.fsync(handle, mode)
    }

    fn peer_release(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: files::RemoteFile,
    ) -> Result<()> {
        self.validate_peer(access, peer_node_id, RootRight::Lookup)?;
        check_remote_file_scope(access, &file)?;
        let handle = decode_file_handle(&file)?;
        // A retry after Home applied RELEASE but lost its ACK is successful.
        // Monotonic handle IDs are never reused; an existing handle still
        // requires the full peer/root/session/identity check below.
        if !self
            .state
            .lock()
            .map_err(|_| poisoned())?
            .file_handles
            .contains_key(&handle)
        {
            return Ok(());
        }
        self.check_peer_file_handle(handle, peer_node_id, access, &file.identity)?;
        match self.release(handle) {
            Err(error) if error.code() == afs_error::NODE_OWNER_STALE_HANDLE => Ok(()),
            other => other,
        }
    }

    fn validate_peer(
        &self,
        access: &PresentedRootAccess,
        peer_node_id: &str,
        right: RootRight,
    ) -> Result<RootGrant> {
        let grant = self
            .roots
            .validate_peer_root_access(access, peer_node_id, right)?;
        let _share = self.share_lock.lock().map_err(|_| poisoned())?;
        let (notifier, next_fuse_ino) = {
            let mut cache = self.private_cache.lock().map_err(|_| poisoned())?;
            if !cache.shared_roots.insert(access.id.clone()) {
                return Ok(grant);
            }
            (cache.notifier.clone(), cache.next_fuse_ino)
        };
        // The shared bit is visible to new replies. The separate share lock
        // keeps further peer admissions behind this invalidation barrier.
        if let Some(notifier) = notifier {
            for ino in 2..next_fuse_ino {
                if let Err(error) = notifier.inval_inode(ino, 0, 0) {
                    self.private_cache
                        .lock()
                        .map_err(|_| poisoned())?
                        .shared_roots
                        .remove(&access.id);
                    return Err(Error::from(error));
                }
            }
        }
        Ok(grant)
    }

    fn owner_entry_for_path(
        &self,
        root_id: &RootId,
        relative: StoragePath,
        expected: Option<&files::FileIdentity>,
    ) -> Result<files::OwnerEntry> {
        let root_use = self.roots.enter_root(root_id, RootRight::Lookup)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let kind = kind_from_metadata(&metadata)?;
        let identity = identity_from_metadata(&metadata)?;
        check_expected_identity(expected, &identity)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.inode_for_path(
            root_id.clone(),
            relative,
            identity.clone(),
            attributes.clone(),
            kind,
        );
        Ok(files::OwnerEntry {
            root_id: root_id.clone(),
            identity,
            attributes,
        })
    }

    fn lookup(&self, parent: BackendInode, name: &OsStr) -> Result<Entry> {
        self.check_inode_namespace(parent)?;
        if parent.value == OWNERFS_ROOT_INODE {
            return self.lookup_root_entry(name);
        }
        let parent_record = self.record(parent.value)?.clone();
        if parent_record.kind != FileKind::Directory {
            return Err(Error::from(std::io::Error::from(
                std::io::ErrorKind::NotADirectory,
            )));
        }
        let child = parent_record
            .relative
            .join_component(name)
            .map_err(Error::from)?;
        self.lookup_storage_entry(&parent_record, child)
    }

    fn getattr(&self, inode: BackendInode, handle: Option<FileHandle>) -> Result<FileAttributes> {
        self.check_inode_namespace(inode)?;
        if let Some(handle) = handle {
            let file = self.open_file_handle(handle)?;
            let file = file.lock().map_err(|_| poisoned())?;
            return file.attributes();
        }
        if inode.value == OWNERFS_ROOT_INODE {
            return self.owner_root_attributes();
        }
        let record = self.record(inode.value)?.clone();
        let root_use = match self.roots.enter_root(&record.root_id, RootRight::Lookup) {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let entry =
                    self.with_remote_root_retry(&record.root_id, RootRight::Lookup, |remote| {
                        remote.files.getattr(
                            &remote.grant,
                            record.relative.as_path().as_os_str(),
                            Some(&record.identity),
                            None,
                        )
                    })?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                state.update_record(
                    inode.value,
                    entry.identity,
                    entry.attributes.clone(),
                    entry.attributes.kind,
                )?;
                return Ok(entry.attributes);
            }
            Err(error) => return Err(error),
        };
        let physical = root_use
            .data_dir()
            .join_path(&record.relative)
            .map_err(Error::from)?;
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        if identity != record.identity {
            return Err(stale("inode no longer names the same local file"));
        }
        attributes_from_metadata(metadata)
    }

    fn setattr(
        &self,
        inode: BackendInode,
        handle: Option<FileHandle>,
        change: &AttributeChange,
    ) -> Result<FileAttributes> {
        if change.mode.is_some()
            || change.uid.is_some()
            || change.gid.is_some()
            || change.atime.is_some()
            || change.mtime.is_some()
        {
            return Err(Error::coded(
                afs_error::NODE_VFS_UNIMPLEMENTED,
                "OwnerFs local chmod/chown/time updates are not wired yet",
            ));
        }
        if let Some(size) = change.size {
            if let Some(handle) = handle {
                let file = self.open_file_handle(handle)?;
                let mut file = file.lock().map_err(|_| poisoned())?;
                file.ensure_open()?;
                match &mut file.file {
                    OpenFileHandle::Local(file) => {
                        file.handle.file.set_len(size).map_err(Error::from)?;
                        return file.attributes();
                    }
                    OpenFileHandle::Remote(file) => {
                        return Ok(file
                            .files
                            .setattr(
                                &file.grant,
                                OsStr::new(""),
                                Some(&file.handle.identity),
                                Some(&file.handle),
                                change,
                            )?
                            .attributes);
                    }
                }
            }
            let record = self.record(inode.value)?.clone();
            let root_use = match self.roots.enter_root(&record.root_id, RootRight::Write) {
                Ok(root_use) => root_use,
                Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                    let remote = self.remote_for_record(&record, RootRight::Write)?;
                    let entry = remote.files.setattr(
                        &remote.grant,
                        record.relative.as_path().as_os_str(),
                        Some(&record.identity),
                        None,
                        change,
                    )?;
                    let mut state = self.state.lock().map_err(|_| poisoned())?;
                    state.update_record(
                        inode.value,
                        entry.identity,
                        entry.attributes.clone(),
                        entry.attributes.kind,
                    )?;
                    return Ok(entry.attributes);
                }
                Err(error) => return Err(error),
            };
            let physical = root_use
                .data_dir()
                .join_path(&record.relative)
                .map_err(Error::from)?;
            let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
            let file = self
                .disk
                .open_file(&physical, OpenSpec::new(libc::O_WRONLY, 0))
                .map_err(Error::from)?;
            let identity = identity_from_metadata(&file.metadata().map_err(Error::from)?)?;
            check_expected_identity(Some(&record.identity), &identity)?;
            file.set_len(size).map_err(Error::from)?;
            let metadata = file.metadata().map_err(Error::from)?;
            let identity = identity_from_metadata(&metadata)?;
            let attributes = attributes_from_metadata(metadata)?;
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            state.update_record(inode.value, identity, attributes.clone(), attributes.kind)?;
            Ok(attributes)
        } else {
            self.getattr(inode, handle)
        }
    }

    fn create(
        &self,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
        flags: i32,
    ) -> Result<CreatedFile> {
        self.check_inode_namespace(parent)?;
        if parent.value == OWNERFS_ROOT_INODE {
            return Err(Error::coded(
                afs_error::NODE_VFS_INVALID,
                "OwnerFs root accepts mkdir for workspace roots, not create",
            ));
        }
        let parent_record = self.record(parent.value)?.clone();
        let child = parent_record
            .relative
            .join_component(name)
            .map_err(Error::from)?;
        if let Err(error) = self
            .roots
            .enter_root(&parent_record.root_id, RootRight::Write)
        {
            if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE {
                let remote = self.remote_for_record(&parent_record, RootRight::Write)?;
                let created = remote.files.create(
                    &remote.grant,
                    child.as_path().as_os_str(),
                    flags,
                    mode,
                    &parent_record.identity,
                )?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                let inode = state.inode_for_path(
                    created.entry.root_id.clone(),
                    child,
                    created.entry.identity.clone(),
                    created.entry.attributes.clone(),
                    created.entry.attributes.kind,
                );
                let handle =
                    state.insert_remote_file_handle(remote.grant, remote.files, created.file, true);
                return Ok(CreatedFile {
                    entry: Entry {
                        inode: backend_inode(inode),
                        attributes: created.entry.attributes,
                    },
                    handle,
                });
            }
            return Err(error);
        }
        let root_use = self
            .roots
            .enter_root(&parent_record.root_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        self.check_peer_parent(
            root_use.data_dir(),
            &parent_record.relative,
            &parent_record.identity,
        )?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        let file = self
            .disk
            .open_file(&physical, OpenSpec::new(flags | libc::O_CREAT, mode))
            .map_err(Error::from)?;
        let metadata = file.metadata().map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let inode = state.inode_for_path(
            parent_record.root_id,
            child,
            identity.clone(),
            attributes.clone(),
            attributes.kind,
        );
        let handle = state.insert_file_handle(files::LocalOpenFile {
            root_id: root_use.root_id().clone(),
            identity,
            file,
            peer: None,
        });
        Ok(CreatedFile {
            entry: Entry {
                inode: backend_inode(inode),
                attributes,
            },
            handle,
        })
    }

    fn open(&self, inode: BackendInode, flags: i32) -> Result<FileHandle> {
        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            return Err(Error::coded(
                afs_error::NODE_VFS_INVALID,
                "OwnerFs root is a directory",
            ));
        }
        let record = self.record(inode.value)?.clone();
        let right = if flags & libc::O_ACCMODE == libc::O_RDONLY && flags & libc::O_TRUNC == 0 {
            RootRight::Read
        } else {
            RootRight::Write
        };
        let root_use = match self.roots.enter_root(&record.root_id, right) {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let mut opened = None;
                let (grant, files) =
                    self.with_remote_root_retry(&record.root_id, right, |remote| {
                        let (file, _attributes) = remote.files.open(
                            &remote.grant,
                            record.relative.as_path().as_os_str(),
                            flags,
                            Some(&record.identity),
                        )?;
                        opened = Some(file);
                        Ok((remote.grant.clone(), remote.files.clone()))
                    })?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                return Ok(state.insert_remote_file_handle(
                    grant,
                    files,
                    opened.ok_or_else(|| stale("remote open did not return a file handle"))?,
                    flags & libc::O_ACCMODE != libc::O_RDONLY || flags & libc::O_TRUNC != 0,
                ));
            }
            Err(error) => return Err(error),
        };
        let physical = root_use
            .data_dir()
            .join_path(&record.relative)
            .map_err(Error::from)?;
        let open_flags = flags & !libc::O_TRUNC;
        let file = self
            .disk
            .open_file(&physical, OpenSpec::new(open_flags, 0))
            .map_err(Error::from)?;
        let metadata = file.metadata().map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        if identity != record.identity {
            return Err(stale("open saw a replacement at the same path"));
        }
        if flags & libc::O_TRUNC != 0 {
            file.set_len(0).map_err(Error::from)?;
            let metadata = file.metadata().map_err(Error::from)?;
            let attributes = attributes_from_metadata(metadata)?;
            self.state.lock().map_err(|_| poisoned())?.update_record(
                inode.value,
                identity.clone(),
                attributes.clone(),
                attributes.kind,
            )?;
        }
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Ok(state.insert_file_handle(files::LocalOpenFile {
            root_id: root_use.root_id().clone(),
            identity,
            file,
            peer: None,
        }))
    }

    fn read(&self, handle: FileHandle, offset: u64, out: &mut [u8]) -> Result<usize> {
        let file = self.open_file_handle(handle)?;
        let file = file.lock().map_err(|_| poisoned())?;
        file.ensure_open()?;
        match &file.file {
            OpenFileHandle::Local(file) => {
                file.handle.file.read_at(offset, out).map_err(Error::from)
            }
            OpenFileHandle::Remote(file) => file.files.read(&file.grant, &file.handle, offset, out),
        }
    }

    fn write(&self, handle: FileHandle, offset: u64, data: &[u8]) -> Result<usize> {
        let file = self.open_file_handle(handle)?;
        let mut file = file.lock().map_err(|_| poisoned())?;
        file.ensure_open()?;
        match &mut file.file {
            OpenFileHandle::Local(file) => {
                file.handle.file.write_at(offset, data).map_err(Error::from)
            }
            OpenFileHandle::Remote(file) => {
                file.needs_flush = true;
                file.files.write(&file.grant, &file.handle, offset, data)
            }
        }
    }

    fn flush(&self, handle: FileHandle) -> Result<()> {
        let file = self.open_file_handle(handle)?;
        let mut file = file.lock().map_err(|_| poisoned())?;
        file.ensure_open()?;
        match &mut file.file {
            OpenFileHandle::Local(file) => file.handle.file.flush().map_err(Error::from),
            OpenFileHandle::Remote(file) if file.needs_flush => {
                file.files.flush(&file.grant, &file.handle)?;
                file.needs_flush = false;
                Ok(())
            }
            OpenFileHandle::Remote(_) => Ok(()),
        }
    }

    fn fsync(&self, handle: FileHandle, mode: SyncMode) -> Result<()> {
        let file = self.open_file_handle(handle)?;
        let mut file = file.lock().map_err(|_| poisoned())?;
        file.ensure_open()?;
        match &mut file.file {
            OpenFileHandle::Local(file) => match mode {
                SyncMode::DataOnly => file.handle.file.sync_data(),
                SyncMode::Full => file.handle.file.sync_all(),
            }
            .map_err(Error::from),
            OpenFileHandle::Remote(file) => {
                file.files.fsync(
                    &file.grant,
                    &file.handle,
                    matches!(mode, SyncMode::DataOnly),
                )?;
                file.needs_flush = false;
                Ok(())
            }
        }
    }

    fn release(&self, handle: FileHandle) -> Result<()> {
        let file = {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            state
                .file_handles
                .remove(&handle)
                .ok_or_else(|| stale("unknown file handle"))?
        };
        let mut file = file.lock().map_err(|_| poisoned())?;
        file.ensure_open()?;
        file.closed = true;
        match &file.file {
            OpenFileHandle::Local(_) => Ok(()),
            OpenFileHandle::Remote(remote) => {
                remote.files.release(&remote.grant, remote.handle.clone())
            }
        }
    }

    fn opendir(&self, inode: BackendInode) -> Result<DirectoryHandle> {
        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            return Ok(state.insert_dir_handle(OWNERFS_ROOT_INODE, OpenLocalDirectory::OwnerRoot));
        }
        let record = self.record(inode.value)?.clone();
        if record.kind != FileKind::Directory {
            return Err(Error::from(std::io::Error::from(
                std::io::ErrorKind::NotADirectory,
            )));
        }
        if let Err(error) = self.roots.enter_root(&record.root_id, RootRight::Lookup) {
            if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE {
                let mut opened = None;
                let (grant, files) =
                    self.with_remote_root_retry(&record.root_id, RootRight::Lookup, |remote| {
                        let directory = remote.files.opendir(
                            &remote.grant,
                            record.relative.as_path().as_os_str(),
                            Some(&record.identity),
                        )?;
                        opened = Some(directory);
                        Ok((remote.grant.clone(), remote.files.clone()))
                    })?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                return Ok(state.insert_dir_handle(
                    inode.value,
                    OpenLocalDirectory::Remote(OpenRemoteDirectory {
                        grant,
                        files,
                        handle: opened
                            .ok_or_else(|| stale("remote opendir did not return a handle"))?,
                    }),
                ));
            }
            return Err(error);
        }
        let root_use = self.roots.enter_root(&record.root_id, RootRight::Lookup)?;
        let physical = root_use
            .data_dir()
            .join_path(&record.relative)
            .map_err(Error::from)?;
        let directory = self.disk.open_dir(&physical).map_err(Error::from)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Ok(state.insert_dir_handle(
            inode.value,
            OpenLocalDirectory::Local(files::LocalOpenDirectory {
                root_id: root_use.root_id().clone(),
                identity: record.identity,
                directory,
                peer: None,
            }),
        ))
    }

    fn readdir(
        &self,
        handle: DirectoryHandle,
        cookie: u64,
        max_entries: usize,
    ) -> Result<Vec<DirectoryEntry>> {
        let rows = {
            let state = self.state.lock().map_err(|_| poisoned())?;
            let directory = state
                .dir_handles
                .get(&handle)
                .ok_or_else(|| stale("unknown directory handle"))?;
            match &directory.handle {
                OpenLocalDirectory::OwnerRoot => drop(state),
                OpenLocalDirectory::Local(open) => {
                    let names = open.directory.read_dir().map_err(Error::from)?;
                    let mut parent = state
                        .inodes
                        .get(&directory.inode)
                        .ok_or_else(|| stale("directory inode missing"))?
                        .clone();
                    parent.identity = open.identity.clone();
                    let parent_relative = parent.relative.clone();
                    drop(state);
                    let mut entries = Vec::new();
                    for name in names {
                        let child = parent_relative.join_component(&name).map_err(Error::from)?;
                        let entry = self.lookup_storage_entry(&parent, child)?;
                        entries.push(DirectoryEntry {
                            name,
                            inode: entry.inode,
                            kind: entry.attributes.kind,
                            next_cookie: 0,
                        });
                    }
                    return slice_directory_entries(entries, cookie, max_entries);
                }
                OpenLocalDirectory::Remote(open) => {
                    let files = open.files.clone();
                    let grant = open.grant.clone();
                    let remote_dir = open.handle.clone();
                    let directory_inode = directory.inode;
                    let record = state
                        .inodes
                        .get(&directory_inode)
                        .ok_or_else(|| stale("directory inode missing"))?
                        .clone();
                    let parent_relative = record.relative.clone();
                    drop(state);
                    let entries = match files.readdir(&grant, &remote_dir, cookie, max_entries) {
                        Ok(entries) => entries,
                        Err(error) if should_refresh_remote_root(&error) => {
                            self.invalidate_remote_root(&record.root_id)?;
                            let mut refreshed_dir = None;
                            let (grant, files, entries) = self.with_remote_root_retry(
                                &record.root_id,
                                RootRight::Lookup,
                                |remote| {
                                    let directory = remote.files.opendir(
                                        &remote.grant,
                                        record.relative.as_path().as_os_str(),
                                        Some(&record.identity),
                                    )?;
                                    let entries = remote.files.readdir(
                                        &remote.grant,
                                        &directory,
                                        cookie,
                                        max_entries,
                                    )?;
                                    refreshed_dir = Some(directory);
                                    Ok((remote.grant.clone(), remote.files.clone(), entries))
                                },
                            )?;
                            let mut state = self.state.lock().map_err(|_| poisoned())?;
                            if let Some(directory) = state.dir_handles.get_mut(&handle) {
                                directory.handle =
                                    OpenLocalDirectory::Remote(OpenRemoteDirectory {
                                        grant,
                                        files,
                                        handle: refreshed_dir.ok_or_else(|| {
                                            stale("remote readdir did not reopen directory")
                                        })?,
                                    });
                            }
                            entries
                        }
                        Err(error) => return Err(error),
                    };
                    let mut rows = Vec::new();
                    for entry in entries {
                        let child = parent_relative
                            .join_component(&entry.name)
                            .map_err(Error::from)?;
                        let inode = self.insert_remote_entry(child, entry.entry.clone())?;
                        rows.push(DirectoryEntry {
                            name: entry.name,
                            inode: backend_inode(inode),
                            kind: entry.entry.attributes.kind,
                            next_cookie: entry.next_cookie,
                        });
                    }
                    return Ok(rows);
                }
            }
            self.root_directory_entries()?
        };
        slice_directory_entries(rows, cookie, max_entries)
    }

    fn fsyncdir(&self, handle: DirectoryHandle, mode: SyncMode) -> Result<()> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        let directory = state
            .dir_handles
            .get(&handle)
            .ok_or_else(|| stale("unknown directory handle"))?;
        match &directory.handle {
            OpenLocalDirectory::OwnerRoot => self.disk.sync_root().map_err(Error::from),
            OpenLocalDirectory::Local(open) => match mode {
                SyncMode::DataOnly | SyncMode::Full => {
                    open.directory.sync_all().map_err(Error::from)
                }
            },
            OpenLocalDirectory::Remote(open) => open.files.fsyncdir(
                &open.grant,
                &open.handle,
                matches!(mode, SyncMode::DataOnly),
            ),
        }
    }

    fn releasedir(&self, handle: DirectoryHandle) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state
            .dir_handles
            .remove(&handle)
            .ok_or_else(|| stale("unknown directory handle"))?;
        Ok(())
    }

    fn mkdir(&self, parent: BackendInode, name: &OsStr, mode: u32) -> Result<Entry> {
        self.check_inode_namespace(parent)?;
        if parent.value == OWNERFS_ROOT_INODE {
            let root_use = self.roots.create_root(name, mode)?;
            let attributes = self.root_entry_attributes(root_use.data_dir())?;
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            let identity = self.root_identity(root_use.data_dir())?;
            let inode = state.insert_root(
                name.to_os_string(),
                root_use.root_id().clone(),
                identity,
                attributes.clone(),
            );
            return Ok(Entry {
                inode: backend_inode(inode),
                attributes,
            });
        }
        let parent_record = self.record(parent.value)?.clone();
        let child = parent_record
            .relative
            .join_component(name)
            .map_err(Error::from)?;
        let root_use = match self
            .roots
            .enter_root(&parent_record.root_id, RootRight::Write)
        {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let remote = self.remote_for_record(&parent_record, RootRight::Write)?;
                let entry = remote.files.mkdir(
                    &remote.grant,
                    child.as_path().as_os_str(),
                    mode,
                    &parent_record.identity,
                )?;
                let inode = self.insert_remote_entry(child, entry.clone())?;
                return Ok(Entry {
                    inode: backend_inode(inode),
                    attributes: entry.attributes,
                });
            }
            Err(error) => return Err(error),
        };
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        self.check_peer_parent(
            root_use.data_dir(),
            &parent_record.relative,
            &parent_record.identity,
        )?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        self.disk.mkdir(&physical, mode).map_err(Error::from)?;
        let dir = self.disk.open_dir(&physical).map_err(Error::from)?;
        dir.sync_all().map_err(Error::from)?;
        let metadata = dir.metadata().map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let inode = state.inode_for_path(
            parent_record.root_id,
            child,
            identity,
            attributes.clone(),
            FileKind::Directory,
        );
        Ok(Entry {
            inode: backend_inode(inode),
            attributes,
        })
    }

    fn unlink(&self, parent: BackendInode, name: &OsStr) -> Result<()> {
        let parent_record = self.directory_record(parent)?;
        let child = parent_record
            .relative
            .join_component(name)
            .map_err(Error::from)?;
        let root_use = match self
            .roots
            .enter_root(&parent_record.root_id, RootRight::Write)
        {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let remote = self.remote_for_record(&parent_record, RootRight::Write)?;
                let expected = self.remote_expected_identity(&parent_record.root_id, &child)?;
                remote.files.unlink(
                    &remote.grant,
                    child.as_path().as_os_str(),
                    expected.as_ref(),
                    &parent_record.identity,
                )?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                state.paths.remove(&(parent_record.root_id, child));
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        self.check_peer_parent(
            root_use.data_dir(),
            &parent_record.relative,
            &parent_record.identity,
        )?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        self.disk.remove_file(&physical).map_err(Error::from)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.paths.remove(&(parent_record.root_id, child));
        Ok(())
    }

    fn rmdir(&self, parent: BackendInode, name: &OsStr) -> Result<()> {
        let parent_record = self.directory_record(parent)?;
        let child = parent_record
            .relative
            .join_component(name)
            .map_err(Error::from)?;
        let root_use = match self
            .roots
            .enter_root(&parent_record.root_id, RootRight::Write)
        {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let remote = self.remote_for_record(&parent_record, RootRight::Write)?;
                let expected = self.remote_expected_identity(&parent_record.root_id, &child)?;
                remote.files.rmdir(
                    &remote.grant,
                    child.as_path().as_os_str(),
                    expected.as_ref(),
                    &parent_record.identity,
                )?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                state.paths.remove(&(parent_record.root_id, child));
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        self.check_peer_parent(
            root_use.data_dir(),
            &parent_record.relative,
            &parent_record.identity,
        )?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        self.disk.remove_dir(&physical).map_err(Error::from)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.paths.remove(&(parent_record.root_id, child));
        Ok(())
    }

    fn rename(
        &self,
        from_parent: BackendInode,
        from_name: &OsStr,
        to_parent: BackendInode,
        to_name: &OsStr,
        flags: RenameFlags,
    ) -> Result<()> {
        let rename_mode = rename_mode_from_flags(flags)?;
        let from_parent = self.directory_record(from_parent)?;
        let to_parent = self.directory_record(to_parent)?;
        if from_parent.root_id != to_parent.root_id {
            return Err(Error::from(std::io::Error::from_raw_os_error(libc::EXDEV)));
        }
        let from = from_parent
            .relative
            .join_component(from_name)
            .map_err(Error::from)?;
        let to = to_parent
            .relative
            .join_component(to_name)
            .map_err(Error::from)?;
        let root_use = match self
            .roots
            .enter_root(&from_parent.root_id, RootRight::Write)
        {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let remote = self.remote_for_record(&from_parent, RootRight::Write)?;
                let expected_old = self.remote_expected_identity(&from_parent.root_id, &from)?;
                let expected_new = self.remote_expected_identity(&from_parent.root_id, &to)?;
                remote.files.rename(
                    &remote.grant,
                    from.as_path().as_os_str(),
                    to.as_path().as_os_str(),
                    expected_old.as_ref(),
                    expected_new.as_ref(),
                    &from_parent.identity,
                    &to_parent.identity,
                    flags,
                )?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                state.rename_path(from_parent.root_id, from, to);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        self.check_peer_parent(
            root_use.data_dir(),
            &from_parent.relative,
            &from_parent.identity,
        )?;
        self.check_peer_parent(
            root_use.data_dir(),
            &to_parent.relative,
            &to_parent.identity,
        )?;
        let physical_from = root_use.data_dir().join_path(&from).map_err(Error::from)?;
        let physical_to = root_use.data_dir().join_path(&to).map_err(Error::from)?;
        self.disk
            .rename(&physical_from, &physical_to, rename_mode)
            .map_err(Error::from)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.rename_path(from_parent.root_id, from, to);
        Ok(())
    }

    fn lookup_root_entry(&self, name: &OsStr) -> Result<Entry> {
        if let Some(entry) = self.cached_root_entry(name)? {
            return Ok(entry);
        }
        let id = root::root_id_from_name(name)?;
        let entry = self.with_remote_root_retry(&id, RootRight::Lookup, |remote| {
            remote.files.lookup(&remote.grant, OsStr::new(""), None)
        })?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let inode = state.insert_root(
            name.to_os_string(),
            entry.root_id,
            entry.identity,
            entry.attributes.clone(),
        );
        Ok(Entry {
            inode: backend_inode(inode),
            attributes: entry.attributes,
        })
    }

    fn cached_root_entry(&self, name: &OsStr) -> Result<Option<Entry>> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        let Some(inode) = state.root_names.get(name).copied() else {
            return Ok(None);
        };
        let record = state
            .inodes
            .get(&inode)
            .ok_or_else(|| stale("root inode missing"))?;
        Ok(Some(Entry {
            inode: backend_inode(inode),
            attributes: record.attributes.clone(),
        }))
    }

    fn acquire_remote_root(&self, id: &RootId, right: RootRight) -> Result<RemoteRoot> {
        if let Some(remote) = self
            .remote_roots
            .lock()
            .map_err(|_| poisoned())?
            .get(id)
            .cloned()
            && remote.grant.rights.contains(&right)
        {
            return Ok(remote);
        }
        self.acquire_remote_root_uncached(id, right)
    }

    fn acquire_remote_root_uncached(&self, id: &RootId, right: RootRight) -> Result<RemoteRoot> {
        let Some(location) = self.roots.lookup_root_location(id)? else {
            return Err(Error::coded(
                afs_error::NODE_VFS_NOT_FOUND,
                "OwnerFs root is not registered in Meta",
            ));
        };
        let factory = self.remote_factory.as_ref().ok_or_else(|| {
            Error::coded(
                afs_error::NODE_VFS_UNIMPLEMENTED,
                "OwnerFs remote dispatch is not wired in this Node",
            )
        })?;
        let grant = self.roots.acquire_remote_root(id, right)?;
        if grant.id != *id
            || grant.home_node_id != location.home_node_id
            || grant.home_session_id != location.home_session_id
        {
            return Err(Error::coded(
                afs_error::NODE_OWNER_INVALID_GRANT,
                "remote root grant does not match Meta location",
            ));
        }
        let remote = RemoteRoot {
            files: factory.connect(&grant.home_node_id)?,
            grant,
        };
        self.remote_roots
            .lock()
            .map_err(|_| poisoned())?
            .insert(id.clone(), remote.clone());
        Ok(remote)
    }

    fn invalidate_remote_root(&self, id: &RootId) -> Result<()> {
        self.remote_roots.lock().map_err(|_| poisoned())?.remove(id);
        Ok(())
    }

    fn with_remote_root_retry<T>(
        &self,
        id: &RootId,
        right: RootRight,
        mut op: impl FnMut(&RemoteRoot) -> Result<T>,
    ) -> Result<T> {
        let remote = self.acquire_remote_root(id, right)?;
        match op(&remote) {
            Err(error) if should_refresh_remote_root(&error) => {
                self.invalidate_remote_root(id)?;
                let refreshed = self.acquire_remote_root_uncached(id, right)?;
                op(&refreshed)
            }
            result => result,
        }
    }

    fn insert_remote_entry(&self, relative: StoragePath, entry: files::OwnerEntry) -> Result<u64> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Ok(state.inode_for_path(
            entry.root_id,
            relative,
            entry.identity,
            entry.attributes.clone(),
            entry.attributes.kind,
        ))
    }

    fn lookup_storage_entry(&self, parent: &NodeRecord, relative: StoragePath) -> Result<Entry> {
        let root_id = &parent.root_id;
        let root_use = match self.roots.enter_root(root_id, RootRight::Lookup) {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let entry = self.with_remote_root_retry(root_id, RootRight::Lookup, |remote| {
                    remote.files.lookup(
                        &remote.grant,
                        relative.as_path().as_os_str(),
                        Some(&parent.identity),
                    )
                })?;
                let inode = self.insert_remote_entry(relative, entry.clone())?;
                return Ok(Entry {
                    inode: backend_inode(inode),
                    attributes: entry.attributes,
                });
            }
            Err(error) => return Err(error),
        };
        let _namespace_guard = if parent.relative.is_root() {
            None
        } else {
            let guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
            self.check_peer_parent(root_use.data_dir(), &parent.relative, &parent.identity)?;
            Some(guard)
        };
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let kind = kind_from_metadata(&metadata)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let inode = state.inode_for_path(
            root_id.clone(),
            relative,
            identity,
            attributes.clone(),
            kind,
        );
        Ok(Entry {
            inode: backend_inode(inode),
            attributes,
        })
    }

    fn root_directory_entries(&self) -> Result<Vec<DirectoryEntry>> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        let mut rows = Vec::new();
        for (name, inode) in &state.root_names {
            let Some(record) = state.inodes.get(inode) else {
                continue;
            };
            rows.push(DirectoryEntry {
                name: name.clone(),
                inode: backend_inode(*inode),
                kind: record.kind,
                next_cookie: 0,
            });
        }
        rows.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(rows)
    }

    fn root_entry_attributes(&self, data_dir: &StoragePath) -> Result<FileAttributes> {
        self.disk
            .metadata(data_dir)
            .map_err(Error::from)
            .and_then(attributes_from_metadata)
    }

    fn root_identity(&self, data_dir: &StoragePath) -> Result<files::FileIdentity> {
        self.disk
            .metadata(data_dir)
            .map_err(Error::from)
            .and_then(|metadata| identity_from_metadata(&metadata))
    }

    fn owner_root_attributes(&self) -> Result<FileAttributes> {
        Ok(FileAttributes {
            kind: FileKind::Directory,
            size: 0,
            mode: 0o755,
            uid: 0,
            gid: 0,
            nlink: 2,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
        })
    }

    fn check_inode_namespace(&self, inode: BackendInode) -> Result<()> {
        if inode.namespace != Namespace::OwnerFs {
            Err(Error::coded(
                afs_error::NODE_VFS_INVALID,
                "inode belongs to another namespace",
            ))
        } else {
            Ok(())
        }
    }

    fn directory_record(&self, inode: BackendInode) -> Result<NodeRecord> {
        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            return Err(Error::coded(
                afs_error::NODE_VFS_INVALID,
                "operation requires a workspace root directory",
            ));
        }
        let record = self.record(inode.value)?.clone();
        if record.kind != FileKind::Directory {
            return Err(Error::from(std::io::Error::from(
                std::io::ErrorKind::NotADirectory,
            )));
        }
        Ok(record)
    }

    fn record(&self, inode: u64) -> Result<NodeRecord> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        state
            .inodes
            .get(&inode)
            .cloned()
            .ok_or_else(|| stale("unknown inode"))
    }
}

#[derive(Clone, Debug)]
struct NodeRecord {
    root_id: RootId,
    relative: StoragePath,
    identity: files::FileIdentity,
    attributes: FileAttributes,
    kind: FileKind,
}

struct OwnerState {
    next_inode: u64,
    next_handle: u64,
    next_dir_handle: u64,
    root_names: HashMap<OsString, u64>,
    paths: HashMap<(RootId, StoragePath), u64>,
    inodes: HashMap<u64, NodeRecord>,
    file_handles: HashMap<FileHandle, Arc<Mutex<OpenFileHandleSlot>>>,
    dir_handles: HashMap<DirectoryHandle, OpenDirectoryHandle>,
    fenced_peer_sessions: HashSet<(String, String)>,
}

impl OwnerState {
    fn new() -> Self {
        Self {
            next_inode: OWNERFS_ROOT_INODE + 1,
            next_handle: 1,
            next_dir_handle: 1,
            root_names: HashMap::new(),
            paths: HashMap::new(),
            inodes: HashMap::new(),
            file_handles: HashMap::new(),
            dir_handles: HashMap::new(),
            fenced_peer_sessions: HashSet::new(),
        }
    }

    fn ensure_peer_session(&self, peer_node_id: &str, access: &PresentedRootAccess) -> Result<()> {
        if self
            .fenced_peer_sessions
            .contains(&(peer_node_id.to_owned(), access.session_id.clone()))
        {
            return Err(stale("peer process session has been reaped"));
        }
        Ok(())
    }

    fn insert_root(
        &mut self,
        name: OsString,
        root_id: RootId,
        identity: files::FileIdentity,
        attributes: FileAttributes,
    ) -> u64 {
        let relative = StoragePath::root();
        if let Some(inode) = self.root_names.get(&name).copied() {
            self.paths
                .insert((root_id.clone(), relative.clone()), inode);
            self.inodes.insert(
                inode,
                NodeRecord {
                    root_id,
                    relative,
                    identity,
                    attributes,
                    kind: FileKind::Directory,
                },
            );
            return inode;
        }
        let inode = self.allocate_inode();
        self.root_names.insert(name, inode);
        self.paths
            .insert((root_id.clone(), relative.clone()), inode);
        self.inodes.insert(
            inode,
            NodeRecord {
                root_id,
                relative,
                identity,
                attributes,
                kind: FileKind::Directory,
            },
        );
        inode
    }

    fn inode_for_path(
        &mut self,
        root_id: RootId,
        relative: StoragePath,
        identity: files::FileIdentity,
        attributes: FileAttributes,
        kind: FileKind,
    ) -> u64 {
        let key = (root_id.clone(), relative.clone());
        if let Some(inode) = self.paths.get(&key).copied()
            && self
                .inodes
                .get(&inode)
                .is_some_and(|record| record.identity == identity)
        {
            return inode;
        }
        let inode = self.allocate_inode();
        self.paths.insert(key, inode);
        self.inodes.insert(
            inode,
            NodeRecord {
                root_id,
                relative,
                identity,
                attributes,
                kind,
            },
        );
        inode
    }

    fn update_record(
        &mut self,
        inode: u64,
        identity: files::FileIdentity,
        attributes: FileAttributes,
        kind: FileKind,
    ) -> Result<()> {
        let record = self
            .inodes
            .get_mut(&inode)
            .ok_or_else(|| stale("unknown inode"))?;
        record.identity = identity;
        record.attributes = attributes;
        record.kind = kind;
        Ok(())
    }

    fn rename_path(&mut self, root_id: RootId, from: StoragePath, to: StoragePath) {
        // A directory rename changes every cached descendant's path. FUSE
        // still addresses those descendants by inode, so retaining their old
        // relative names would direct later operations at the wrong object.
        let moved: Vec<_> = self
            .paths
            .iter()
            .filter_map(|((id, path), inode)| {
                if id != &root_id {
                    return None;
                }
                let suffix = path.as_path().strip_prefix(from.as_path()).ok()?;
                let new_path = if suffix.as_os_str().is_empty() {
                    to.clone()
                } else {
                    StoragePath::new(to.as_path().join(suffix)).ok()?
                };
                Some((path.clone(), new_path, *inode))
            })
            .collect();
        for (old_path, _, _) in &moved {
            self.paths.remove(&(root_id.clone(), old_path.clone()));
        }
        // An overwritten destination's old inode records remain so existing
        // handles can still be released, but no path lookup may reuse them.
        self.paths.retain(|(id, path), _| {
            id != &root_id || path.as_path().strip_prefix(to.as_path()).is_err()
        });
        for (_, new_path, inode) in moved {
            self.paths
                .insert((root_id.clone(), new_path.clone()), inode);
            if let Some(record) = self.inodes.get_mut(&inode) {
                record.relative = new_path;
            }
        }
    }

    fn insert_file_handle(&mut self, handle: files::LocalOpenFile) -> FileHandle {
        self.insert_open_file(OpenFileHandle::Local(OpenLocalFile { handle }))
    }

    fn insert_remote_file_handle(
        &mut self,
        grant: RootGrant,
        files: Arc<dyn remote::RemoteFiles>,
        handle: files::RemoteFile,
        needs_flush: bool,
    ) -> FileHandle {
        self.insert_open_file(OpenFileHandle::Remote(OpenRemoteFile {
            grant,
            files,
            handle,
            needs_flush,
        }))
    }

    fn insert_open_file(&mut self, handle: OpenFileHandle) -> FileHandle {
        let id = FileHandle(self.next_handle);
        self.next_handle += 1;
        self.file_handles
            .insert(id, Arc::new(Mutex::new(OpenFileHandleSlot::new(handle))));
        id
    }

    fn insert_dir_handle(&mut self, inode: u64, handle: OpenLocalDirectory) -> DirectoryHandle {
        let id = DirectoryHandle(self.next_dir_handle);
        self.next_dir_handle += 1;
        self.dir_handles
            .insert(id, OpenDirectoryHandle { inode, handle });
        id
    }

    fn allocate_inode(&mut self) -> u64 {
        let inode = self.next_inode;
        self.next_inode += 1;
        inode
    }
}

// Boxing the remote arm would add a heap allocation to every remote open.
#[allow(clippy::large_enum_variant)]
enum OpenFileHandle {
    Local(OpenLocalFile),
    Remote(OpenRemoteFile),
}

struct OpenFileHandleSlot {
    file: OpenFileHandle,
    closed: bool,
}

impl OpenFileHandleSlot {
    fn new(file: OpenFileHandle) -> Self {
        Self {
            file,
            closed: false,
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            Err(stale("file handle is closed"))
        } else {
            Ok(())
        }
    }

    fn check_peer(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        identity: &files::FileIdentity,
    ) -> Result<()> {
        self.ensure_open()?;
        let OpenFileHandle::Local(local) = &self.file else {
            return Err(stale("peer file handle is not local to Home"));
        };
        if local.handle.root_id != access.id
            || (!identity.0.is_empty() && local.handle.identity != *identity)
            || local
                .handle
                .peer
                .as_ref()
                .is_none_or(|scope| scope.node_id != peer_node_id || scope.access != *access)
        {
            return Err(stale("peer file handle belongs to another open or grant"));
        }
        Ok(())
    }

    fn attributes(&self) -> Result<FileAttributes> {
        self.ensure_open()?;
        self.file.attributes()
    }
}

#[derive(Debug)]
struct OpenLocalFile {
    handle: files::LocalOpenFile,
}

struct OpenRemoteFile {
    grant: RootGrant,
    files: Arc<dyn remote::RemoteFiles>,
    handle: files::RemoteFile,
    needs_flush: bool,
}

impl OpenFileHandle {
    fn attributes(&self) -> Result<FileAttributes> {
        match self {
            Self::Local(file) => file.attributes(),
            Self::Remote(file) => Ok(file
                .files
                .getattr(
                    &file.grant,
                    OsStr::new(""),
                    Some(&file.handle.identity),
                    Some(&file.handle),
                )?
                .attributes),
        }
    }
}

impl OpenLocalFile {
    fn attributes(&self) -> Result<FileAttributes> {
        self.handle
            .file
            .metadata()
            .map_err(Error::from)
            .and_then(attributes_from_metadata)
    }
}

#[allow(clippy::large_enum_variant)]
enum OpenLocalDirectory {
    OwnerRoot,
    Local(files::LocalOpenDirectory),
    Remote(OpenRemoteDirectory),
}

struct OpenRemoteDirectory {
    grant: RootGrant,
    files: Arc<dyn remote::RemoteFiles>,
    handle: files::RemoteDirectory,
}

struct OpenDirectoryHandle {
    inode: u64,
    handle: OpenLocalDirectory,
}

fn storage_path_from_os(path: &OsStr) -> Result<StoragePath> {
    if path.as_bytes().is_empty() {
        return Ok(StoragePath::root());
    }
    StoragePath::new(PathBuf::from(OsString::from_vec(path.as_bytes().to_vec())))
        .map_err(Error::from)
}

fn split_parent_name(path: &OsStr) -> Result<(StoragePath, OsString)> {
    let bytes = path.as_bytes();
    if bytes.is_empty() || bytes.ends_with(b"/") {
        return Err(Error::coded(
            afs_error::NODE_VFS_INVALID,
            "OwnerFs peer path must name a file below the root",
        ));
    }
    let Some(index) = bytes.iter().rposition(|byte| *byte == b'/') else {
        return Ok((StoragePath::root(), path.to_os_string()));
    };
    let parent = StoragePath::new(PathBuf::from(OsString::from_vec(bytes[..index].to_vec())))
        .map_err(Error::from)?;
    let name = OsString::from_vec(bytes[index + 1..].to_vec());
    if name.as_bytes().is_empty() {
        return Err(Error::coded(
            afs_error::NODE_VFS_INVALID,
            "OwnerFs peer path has an empty leaf",
        ));
    }
    Ok((parent, name))
}

fn remote_file(
    access: &PresentedRootAccess,
    identity: files::FileIdentity,
    handle: FileHandle,
) -> files::RemoteFile {
    files::RemoteFile {
        root_id: access.id.clone(),
        owner_node_id: access.home_node_id.clone(),
        owner_session_id: access.home_session_id.clone(),
        identity,
        handle: handle.0.to_le_bytes().to_vec(),
    }
}

fn remote_directory(
    access: &PresentedRootAccess,
    identity: files::FileIdentity,
    handle: DirectoryHandle,
) -> files::RemoteDirectory {
    files::RemoteDirectory {
        root_id: access.id.clone(),
        owner_node_id: access.home_node_id.clone(),
        owner_session_id: access.home_session_id.clone(),
        identity,
        handle: handle.0.to_le_bytes().to_vec(),
    }
}

fn decode_directory_handle(directory: &files::RemoteDirectory) -> Result<DirectoryHandle> {
    let bytes: [u8; 8] = directory.handle.as_slice().try_into().map_err(|_| {
        Error::coded(
            afs_error::NODE_OWNER_STALE_HANDLE,
            "remote directory handle has invalid size",
        )
    })?;
    Ok(DirectoryHandle(u64::from_le_bytes(bytes)))
}

fn decode_file_handle(file: &files::RemoteFile) -> Result<FileHandle> {
    let bytes: [u8; 8] = file.handle.as_slice().try_into().map_err(|_| {
        Error::coded(
            afs_error::NODE_OWNER_STALE_HANDLE,
            "remote file handle has invalid size",
        )
    })?;
    Ok(FileHandle(u64::from_le_bytes(bytes)))
}

fn check_remote_directory_scope(
    access: &PresentedRootAccess,
    directory: &files::RemoteDirectory,
) -> Result<()> {
    if directory.root_id != access.id
        || directory.owner_node_id != access.home_node_id
        || directory.owner_session_id != access.home_session_id
    {
        return Err(stale(
            "remote directory handle belongs to another root or Home session",
        ));
    }
    Ok(())
}

fn check_remote_file_scope(access: &PresentedRootAccess, file: &files::RemoteFile) -> Result<()> {
    if file.root_id != access.id
        || file.owner_node_id != access.home_node_id
        || file.owner_session_id != access.home_session_id
    {
        return Err(stale(
            "remote file handle belongs to another root or Home session",
        ));
    }
    Ok(())
}

fn check_expected_identity(
    expected: Option<&files::FileIdentity>,
    actual: &files::FileIdentity,
) -> Result<()> {
    if expected.is_some_and(|expected| expected != actual) {
        return Err(stale("file identity no longer matches expected identity"));
    }
    Ok(())
}

fn backend_inode(value: u64) -> BackendInode {
    BackendInode {
        namespace: Namespace::OwnerFs,
        value,
    }
}

fn slice_directory_entries(
    mut rows: Vec<DirectoryEntry>,
    cookie: u64,
    max_entries: usize,
) -> Result<Vec<DirectoryEntry>> {
    let start = usize::try_from(cookie).map_err(|_| {
        Error::coded(
            afs_error::NODE_VFS_INVALID,
            "directory cookie does not fit usize",
        )
    })?;
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    let mut selected = Vec::new();
    for (index, mut entry) in rows.into_iter().enumerate().skip(start).take(max_entries) {
        entry.next_cookie = u64::try_from(index + 1).unwrap_or(u64::MAX);
        selected.push(entry);
    }
    Ok(selected)
}

fn attributes_from_metadata(metadata: fs::Metadata) -> Result<FileAttributes> {
    Ok(FileAttributes {
        kind: kind_from_metadata(&metadata)?,
        size: metadata.len(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        nlink: metadata.nlink() as u32,
        atime: UNIX_EPOCH + Duration::new(metadata.atime() as u64, metadata.atime_nsec() as u32),
        mtime: UNIX_EPOCH + Duration::new(metadata.mtime() as u64, metadata.mtime_nsec() as u32),
        ctime: UNIX_EPOCH + Duration::new(metadata.ctime() as u64, metadata.ctime_nsec() as u32),
    })
}

fn identity_from_attributes(attributes: &FileAttributes) -> files::FileIdentity {
    DecodedIdentity {
        dev: 0,
        ino: 0,
        kind: attributes.kind,
        birth_sec: 0,
        birth_nsec: 0,
    }
    .into_identity()
}

fn identity_from_metadata(metadata: &fs::Metadata) -> Result<files::FileIdentity> {
    // dev+ino alone is insufficient: Linux may reuse the same inode immediately
    // after unlink, while B still has a positive dentry cached. Birth time is
    // stable across writes and renames, unlike ctime, and distinguishes the
    // replacement without making ordinary writes invalidate open handles.
    let birth = metadata.created().map_err(Error::from)?;
    let since_epoch = birth.duration_since(UNIX_EPOCH).map_err(|_| {
        Error::coded(
            afs_error::NODE_VFS_INVALID,
            "file birth time predates Unix epoch",
        )
    })?;
    Ok(DecodedIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        kind: kind_from_metadata(metadata)?,
        birth_sec: since_epoch.as_secs(),
        birth_nsec: since_epoch.subsec_nanos(),
    }
    .into_identity())
}

struct DecodedIdentity {
    dev: u64,
    ino: u64,
    kind: FileKind,
    birth_sec: u64,
    birth_nsec: u32,
}

impl DecodedIdentity {
    fn into_identity(self) -> files::FileIdentity {
        let mut bytes = Vec::with_capacity(29);
        bytes.extend_from_slice(&self.dev.to_le_bytes());
        bytes.extend_from_slice(&self.ino.to_le_bytes());
        bytes.extend_from_slice(&self.birth_sec.to_le_bytes());
        bytes.extend_from_slice(&self.birth_nsec.to_le_bytes());
        bytes.push(match self.kind {
            FileKind::Regular => 1,
            FileKind::Directory => 2,
            FileKind::Symlink => 3,
        });
        files::FileIdentity(bytes)
    }
}

fn kind_from_metadata(metadata: &fs::Metadata) -> Result<FileKind> {
    let ty = metadata.file_type();
    if ty.is_file() {
        Ok(FileKind::Regular)
    } else if ty.is_dir() {
        Ok(FileKind::Directory)
    } else if ty.is_symlink() {
        Ok(FileKind::Symlink)
    } else if ty.is_socket() || ty.is_fifo() || ty.is_block_device() || ty.is_char_device() {
        Err(Error::coded(
            afs_error::NODE_VFS_UNIMPLEMENTED,
            "OwnerFs local special files are not wired yet",
        ))
    } else {
        Err(Error::coded(
            afs_error::NODE_VFS_INVALID,
            "unknown local file kind",
        ))
    }
}

const RENAME_NOREPLACE_FLAG: u32 = 1;
const RENAME_EXCHANGE_FLAG: u32 = 2;

fn rename_mode_from_flags(flags: RenameFlags) -> Result<RenameMode> {
    match flags.0 {
        0 => Ok(RenameMode::Replace),
        RENAME_NOREPLACE_FLAG => Ok(RenameMode::NoReplace),
        RENAME_EXCHANGE_FLAG => Err(Error::coded(
            afs_error::NODE_VFS_UNIMPLEMENTED,
            "OwnerFs RENAME_EXCHANGE is not wired yet",
        )),
        _ => Err(Error::coded(
            afs_error::NODE_VFS_UNIMPLEMENTED,
            "OwnerFs rename flags are not wired yet",
        )),
    }
}

fn stale(message: &'static str) -> Error {
    Error::coded(afs_error::NODE_OWNER_STALE_HANDLE, message)
}

fn should_refresh_remote_root(error: &Error) -> bool {
    matches!(
        error.code(),
        afs_error::NODE_OWNER_INVALID_GRANT
            | afs_error::NODE_OWNER_STALE_ACCESS
            | afs_error::NODE_OWNER_GRANT_UNAVAILABLE
            | afs_error::CLIENT_CONNECTION_UNAVAILABLE
            | afs_error::NODE_TRANSFER_UNAVAILABLE
    )
}

fn poisoned() -> Error {
    Error::coded(afs_error::RUNTIME_INTERNAL, "OwnerFs state lock poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::node::vfs::ownerfs::root::{PreparedRoot, RootGrant, RootMeta, RootReservation};

    struct LocalMeta {
        node_id: String,
        session_id: String,
        next_epoch: Mutex<u64>,
        recover_calls: Mutex<u64>,
        active: Mutex<HashMap<RootId, root::RootLocation>>,
    }

    impl RootMeta for LocalMeta {
        fn reserve_root(&self, id: &RootId, create_intent_id: &str) -> Result<RootReservation> {
            let mut next = self.next_epoch.lock().unwrap();
            let epoch = *next;
            *next += 1;
            Ok(RootReservation {
                id: id.clone(),
                epoch,
                home_node_id: self.node_id.clone(),
                session_id: self.session_id.clone(),
                create_intent_id: create_intent_id.to_owned(),
                prepare_token: format!("prepare-{epoch}"),
            })
        }

        fn activate_root(&self, prepared: &PreparedRoot) -> Result<RootGrant> {
            self.active.lock().unwrap().insert(
                prepared.reservation().id.clone(),
                root::RootLocation {
                    id: prepared.reservation().id.clone(),
                    epoch: prepared.reservation().epoch,
                    home_node_id: self.node_id.clone(),
                    home_session_id: self.session_id.clone(),
                },
            );
            Ok(RootGrant {
                id: prepared.reservation().id.clone(),
                epoch: prepared.reservation().epoch,
                home_node_id: self.node_id.clone(),
                home_session_id: self.session_id.clone(),
                holder_node_id: self.node_id.clone(),
                session_id: self.session_id.clone(),
                access_generation: prepared.reservation().epoch,
                rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write],
                fencing_token: format!("fence-{}", prepared.reservation().epoch),
            })
        }

        fn abort_root(&self, _: &RootReservation) -> Result<()> {
            Ok(())
        }

        fn lookup_root(&self, _: &RootId) -> Result<Option<root::RootLocation>> {
            Ok(None)
        }

        fn list_owner_roots(&self, _: &str) -> Result<root::OwnerRootInventory> {
            Ok(root::OwnerRootInventory {
                active: self.active.lock().unwrap().values().cloned().collect(),
                pending: Vec::new(),
            })
        }

        fn acquire_root(&self, _: &RootId, _: RootRight) -> Result<RootGrant> {
            Err(Error::coded(
                afs_error::NODE_OWNER_GRANT_UNAVAILABLE,
                "test",
            ))
        }

        fn validate_root_access(
            &self,
            _: &root::PresentedRootAccess,
            _: &str,
        ) -> Result<RootGrant> {
            Err(Error::coded(
                afs_error::NODE_OWNER_GRANT_UNAVAILABLE,
                "test",
            ))
        }

        fn recover_root(
            &self,
            record: &catalog::LocalRootRecord,
            new_session_id: &str,
        ) -> Result<RootGrant> {
            *self.recover_calls.lock().unwrap() += 1;
            self.active.lock().unwrap().insert(
                record.id.clone(),
                root::RootLocation {
                    id: record.id.clone(),
                    epoch: record.epoch,
                    home_node_id: self.node_id.clone(),
                    home_session_id: new_session_id.to_owned(),
                },
            );
            Ok(RootGrant {
                id: record.id.clone(),
                epoch: record.epoch,
                home_node_id: self.node_id.clone(),
                home_session_id: new_session_id.to_owned(),
                holder_node_id: self.node_id.clone(),
                session_id: new_session_id.to_owned(),
                access_generation: record.epoch,
                rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write],
                fencing_token: format!("recover-fence-{}", record.epoch),
            })
        }
    }

    struct RestartingRemoteMeta {
        root_id: RootId,
        restarted: Arc<AtomicBool>,
        acquire_calls: AtomicUsize,
    }

    impl RestartingRemoteMeta {
        fn home_session(&self) -> String {
            if self.restarted.load(Ordering::SeqCst) {
                "home-new".to_owned()
            } else {
                "home-old".to_owned()
            }
        }
    }

    impl RootMeta for RestartingRemoteMeta {
        fn reserve_root(&self, _: &RootId, _: &str) -> Result<RootReservation> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn activate_root(&self, _: &PreparedRoot) -> Result<RootGrant> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn abort_root(&self, _: &RootReservation) -> Result<()> {
            Ok(())
        }

        fn lookup_root(&self, id: &RootId) -> Result<Option<root::RootLocation>> {
            if id != &self.root_id {
                return Ok(None);
            }
            Ok(Some(root::RootLocation {
                id: id.clone(),
                epoch: 1,
                home_node_id: "node-a".to_owned(),
                home_session_id: self.home_session(),
            }))
        }

        fn list_owner_roots(&self, _: &str) -> Result<root::OwnerRootInventory> {
            Ok(root::OwnerRootInventory::default())
        }

        fn acquire_root(&self, id: &RootId, right: RootRight) -> Result<RootGrant> {
            self.acquire_calls.fetch_add(1, Ordering::SeqCst);
            Ok(RootGrant {
                id: id.clone(),
                epoch: 1,
                home_node_id: "node-a".to_owned(),
                home_session_id: self.home_session(),
                holder_node_id: "node-b".to_owned(),
                session_id: "session-b".to_owned(),
                access_generation: 1,
                rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write, right],
                fencing_token: "fence-1".to_owned(),
            })
        }

        fn validate_root_access(
            &self,
            _: &root::PresentedRootAccess,
            _: &str,
        ) -> Result<RootGrant> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn recover_root(&self, _: &catalog::LocalRootRecord, _: &str) -> Result<RootGrant> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }
    }

    struct StaticRemoteFactory {
        files: Arc<dyn remote::RemoteFiles>,
    }

    impl RemoteFilesFactory for StaticRemoteFactory {
        fn connect(&self, _: &str) -> Result<Arc<dyn remote::RemoteFiles>> {
            Ok(self.files.clone())
        }
    }

    struct RestartingRemoteFiles {
        root_id: RootId,
        restarted: Arc<AtomicBool>,
        lookup_calls: AtomicUsize,
        open_calls: AtomicUsize,
    }

    impl RestartingRemoteFiles {
        fn entry(&self, kind: FileKind) -> files::OwnerEntry {
            files::OwnerEntry {
                root_id: self.root_id.clone(),
                identity: files::FileIdentity(vec![match kind {
                    FileKind::Regular => 1,
                    FileKind::Directory => 2,
                    FileKind::Symlink => 3,
                }]),
                attributes: test_attrs(kind),
            }
        }

        fn reject_old_child_access(&self, grant: &RootGrant, path: &OsStr) -> Result<()> {
            if grant.home_session_id == "home-old" && !path.as_bytes().is_empty() {
                self.restarted.store(true, Ordering::SeqCst);
                return Err(Error::coded(
                    afs_error::NODE_OWNER_INVALID_GRANT,
                    "old home session",
                ));
            }
            Ok(())
        }
    }

    struct SlowRemoteFiles {
        root_id: RootId,
        active_writes: AtomicUsize,
        max_active_writes: AtomicUsize,
        started_writes: AtomicUsize,
        release_calls: AtomicUsize,
        write_delay: Duration,
    }

    impl SlowRemoteFiles {
        fn new(root_id: RootId, write_delay: Duration) -> Self {
            Self {
                root_id,
                active_writes: AtomicUsize::new(0),
                max_active_writes: AtomicUsize::new(0),
                started_writes: AtomicUsize::new(0),
                release_calls: AtomicUsize::new(0),
                write_delay,
            }
        }

        fn unsupported<T>(&self) -> Result<T> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn observe_active_write(&self) {
            let current = self.active_writes.fetch_add(1, Ordering::SeqCst) + 1;
            self.started_writes.fetch_add(1, Ordering::SeqCst);
            let _ =
                self.max_active_writes
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |seen| {
                        (current > seen).then_some(current)
                    });
        }
    }

    impl remote::RemoteFiles for SlowRemoteFiles {
        fn lookup(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
        ) -> Result<files::OwnerEntry> {
            self.unsupported()
        }

        fn getattr(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: Option<&files::RemoteFile>,
        ) -> Result<files::OwnerEntry> {
            self.unsupported()
        }

        fn setattr(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: Option<&files::RemoteFile>,
            _: &AttributeChange,
        ) -> Result<files::OwnerEntry> {
            self.unsupported()
        }

        fn create(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: i32,
            _: u32,
            _: &files::FileIdentity,
        ) -> Result<remote::RemoteCreatedFile> {
            self.unsupported()
        }

        fn mkdir(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: u32,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
            self.unsupported()
        }

        fn unlink(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
        ) -> Result<()> {
            self.unsupported()
        }

        fn rmdir(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
        ) -> Result<()> {
            self.unsupported()
        }

        fn rename(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
            _: &files::FileIdentity,
            _: RenameFlags,
        ) -> Result<()> {
            self.unsupported()
        }

        fn open(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: i32,
            _: Option<&files::FileIdentity>,
        ) -> Result<(files::RemoteFile, FileAttributes)> {
            self.unsupported()
        }

        fn readlink(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
        ) -> Result<Vec<u8>> {
            self.unsupported()
        }

        fn read(
            &self,
            _: &RootGrant,
            _: &files::RemoteFile,
            _: u64,
            out: &mut [u8],
        ) -> Result<usize> {
            out.fill(0);
            Ok(out.len())
        }

        fn write(
            &self,
            _: &RootGrant,
            _: &files::RemoteFile,
            _: u64,
            data: &[u8],
        ) -> Result<usize> {
            self.observe_active_write();
            std::thread::sleep(self.write_delay);
            self.active_writes.fetch_sub(1, Ordering::SeqCst);
            Ok(data.len())
        }

        fn flush(&self, _: &RootGrant, _: &files::RemoteFile) -> Result<()> {
            Ok(())
        }

        fn fsync(&self, _: &RootGrant, _: &files::RemoteFile, _: bool) -> Result<()> {
            Ok(())
        }

        fn release(&self, _: &RootGrant, _: files::RemoteFile) -> Result<()> {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn opendir(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
        ) -> Result<files::RemoteDirectory> {
            self.unsupported()
        }

        fn readdir(
            &self,
            _: &RootGrant,
            _: &files::RemoteDirectory,
            _: u64,
            _: usize,
        ) -> Result<Vec<remote::RemoteDirectoryEntry>> {
            self.unsupported()
        }

        fn fsyncdir(&self, _: &RootGrant, _: &files::RemoteDirectory, _: bool) -> Result<()> {
            self.unsupported()
        }

        fn releasedir(&self, _: &RootGrant, _: files::RemoteDirectory) -> Result<()> {
            Ok(())
        }
    }

    impl remote::RemoteFiles for RestartingRemoteFiles {
        fn lookup(
            &self,
            grant: &RootGrant,
            path: &OsStr,
            _: Option<&files::FileIdentity>,
        ) -> Result<files::OwnerEntry> {
            self.lookup_calls.fetch_add(1, Ordering::SeqCst);
            self.reject_old_child_access(grant, path)?;
            Ok(if path.as_bytes().is_empty() {
                self.entry(FileKind::Directory)
            } else {
                self.entry(FileKind::Regular)
            })
        }

        fn getattr(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: Option<&files::RemoteFile>,
        ) -> Result<files::OwnerEntry> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn setattr(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: Option<&files::RemoteFile>,
            _: &AttributeChange,
        ) -> Result<files::OwnerEntry> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn create(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: i32,
            _: u32,
            _: &files::FileIdentity,
        ) -> Result<remote::RemoteCreatedFile> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn mkdir(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: u32,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn unlink(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
        ) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn rmdir(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
        ) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn rename(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
            _: &files::FileIdentity,
            _: RenameFlags,
        ) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn open(
            &self,
            grant: &RootGrant,
            path: &OsStr,
            _: i32,
            expected_identity: Option<&files::FileIdentity>,
        ) -> Result<(files::RemoteFile, FileAttributes)> {
            self.open_calls.fetch_add(1, Ordering::SeqCst);
            self.reject_old_child_access(grant, path)?;
            let identity = expected_identity
                .cloned()
                .unwrap_or_else(|| self.entry(FileKind::Regular).identity);
            Ok((
                files::RemoteFile {
                    root_id: self.root_id.clone(),
                    owner_node_id: grant.home_node_id.clone(),
                    owner_session_id: grant.home_session_id.clone(),
                    identity,
                    handle: vec![9],
                },
                test_attrs(FileKind::Regular),
            ))
        }

        fn readlink(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
        ) -> Result<Vec<u8>> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn read(
            &self,
            _: &RootGrant,
            _: &files::RemoteFile,
            _: u64,
            _: &mut [u8],
        ) -> Result<usize> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn write(&self, _: &RootGrant, _: &files::RemoteFile, _: u64, _: &[u8]) -> Result<usize> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn flush(&self, _: &RootGrant, _: &files::RemoteFile) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn fsync(&self, _: &RootGrant, _: &files::RemoteFile, _: bool) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn release(&self, _: &RootGrant, _: files::RemoteFile) -> Result<()> {
            Ok(())
        }

        fn opendir(
            &self,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
        ) -> Result<files::RemoteDirectory> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn readdir(
            &self,
            _: &RootGrant,
            _: &files::RemoteDirectory,
            _: u64,
            _: usize,
        ) -> Result<Vec<remote::RemoteDirectoryEntry>> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn fsyncdir(&self, _: &RootGrant, _: &files::RemoteDirectory, _: bool) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn releasedir(&self, _: &RootGrant, _: files::RemoteDirectory) -> Result<()> {
            Ok(())
        }
    }

    fn test_attrs(kind: FileKind) -> FileAttributes {
        FileAttributes {
            kind,
            size: 0,
            mode: 0o644,
            uid: 1000,
            gid: 1000,
            nlink: 1,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
        }
    }

    fn remote_fixture() -> (
        tempfile::TempDir,
        OwnerFs,
        RequestContext,
        Arc<RestartingRemoteMeta>,
        Arc<RestartingRemoteFiles>,
        RootId,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let disk = Arc::new(LocalFs::open(temp.path()).unwrap());
        let root_id = root::root_id_from_name(OsStr::new("job-42")).unwrap();
        let restarted = Arc::new(AtomicBool::new(false));
        let meta = Arc::new(RestartingRemoteMeta {
            root_id: root_id.clone(),
            restarted: restarted.clone(),
            acquire_calls: AtomicUsize::new(0),
        });
        let remote = Arc::new(RestartingRemoteFiles {
            root_id: root_id.clone(),
            restarted,
            lookup_calls: AtomicUsize::new(0),
            open_calls: AtomicUsize::new(0),
        });
        let roots = Arc::new(RootManager::new(
            "node-b".into(),
            "session-b".into(),
            meta.clone(),
            disk.clone(),
        ));
        let factory = Arc::new(StaticRemoteFactory {
            files: remote.clone(),
        });
        let fs = OwnerFs::new_local_with_remote(roots, disk, factory);
        let ctx = RequestContext {
            uid: 1000,
            gid: 1000,
            pid: 42,
            umask: 0,
        };
        (temp, fs, ctx, meta, remote, root_id)
    }

    fn fixture() -> (tempfile::TempDir, OwnerFs, RequestContext) {
        let temp = tempfile::tempdir().unwrap();
        let disk = Arc::new(LocalFs::open(temp.path()).unwrap());
        let meta = Arc::new(LocalMeta {
            node_id: "node-a".into(),
            session_id: "session-a".into(),
            next_epoch: Mutex::new(1),
            recover_calls: Mutex::new(0),
            active: Mutex::new(HashMap::new()),
        });
        let roots = Arc::new(RootManager::new(
            "node-a".into(),
            "session-a".into(),
            meta,
            disk.clone(),
        ));
        let ctx = RequestContext {
            uid: 1000,
            gid: 1000,
            pid: 42,
            umask: 0,
        };
        (temp, OwnerFs::new_local(roots, disk), ctx)
    }

    fn test_grant(root_id: RootId) -> RootGrant {
        RootGrant {
            id: root_id,
            epoch: 1,
            home_node_id: "node-a".to_owned(),
            home_session_id: "session-a".to_owned(),
            holder_node_id: "node-b".to_owned(),
            session_id: "session-b".to_owned(),
            access_generation: 1,
            rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write],
            fencing_token: "fence-1".to_owned(),
        }
    }

    fn presented(grant: &RootGrant) -> PresentedRootAccess {
        PresentedRootAccess {
            id: grant.id.clone(),
            epoch: grant.epoch,
            home_node_id: grant.home_node_id.clone(),
            home_session_id: grant.home_session_id.clone(),
            holder_node_id: grant.holder_node_id.clone(),
            session_id: grant.session_id.clone(),
            access_generation: grant.access_generation,
            fencing_token: grant.fencing_token.clone(),
        }
    }

    #[test]
    fn home_handle_is_bound_to_its_peer_root_session_and_identity() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-a"), 0o755).unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        let local = fs.require_local().unwrap();
        let access_a = presented(&test_grant(
            root::root_id_from_name(OsStr::new("job-a")).unwrap(),
        ));
        let access_b = presented(&test_grant(
            root::root_id_from_name(OsStr::new("job-b")).unwrap(),
        ));
        let identity = {
            let slot = local.open_file_handle(created.handle).unwrap();
            let mut slot = slot.lock().unwrap();
            let OpenFileHandle::Local(open) = &mut slot.file else {
                unreachable!()
            };
            let identity = open.handle.identity.clone();
            open.handle.peer = Some(files::PeerOpenScope {
                node_id: "node-b".to_owned(),
                access: access_a.clone(),
            });
            identity
        };
        local
            .check_peer_file_handle(created.handle, "node-b", &access_a, &identity)
            .unwrap();
        for (peer, access, file_identity) in [
            ("node-b", &access_b, &identity),
            ("node-c", &access_a, &identity),
            ("node-b", &access_a, &files::FileIdentity(vec![0])),
        ] {
            assert_eq!(
                local
                    .check_peer_file_handle(created.handle, peer, access, file_identity)
                    .unwrap_err()
                    .code(),
                afs_error::NODE_OWNER_STALE_HANDLE,
            );
        }
        fs.release(&ctx, created.handle).unwrap();
    }

    #[test]
    fn expired_peer_session_reaps_only_its_home_handles() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-a"), 0o755).unwrap();
        let peer_file = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("peer"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        let local_file = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("local"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        let peer_dir = fs.opendir(&ctx, workspace.inode).unwrap();
        let local = fs.require_local().unwrap();
        let access = presented(&test_grant(
            root::root_id_from_name(OsStr::new("job-a")).unwrap(),
        ));
        {
            let slot = local.open_file_handle(peer_file.handle).unwrap();
            let mut slot = slot.lock().unwrap();
            let OpenFileHandle::Local(open) = &mut slot.file else {
                unreachable!()
            };
            open.handle.peer = Some(files::PeerOpenScope {
                node_id: "node-b".to_owned(),
                access: access.clone(),
            });
        }
        {
            let mut state = local.state.lock().unwrap();
            let directory = state.dir_handles.get_mut(&peer_dir).unwrap();
            let OpenLocalDirectory::Local(open) = &mut directory.handle else {
                unreachable!()
            };
            open.peer = Some(files::PeerOpenScope {
                node_id: "node-b".to_owned(),
                access: access.clone(),
            });
        }
        assert!(
            local
                .peer_handle_sessions()
                .unwrap()
                .contains(&("node-b".to_owned(), "session-b".to_owned(),))
        );
        assert_eq!(local.reap_peer_session("node-b", "session-b").unwrap(), 1);
        assert_eq!(
            fs.write(&ctx, peer_file.handle, 0, b"denied")
                .unwrap_err()
                .code(),
            afs_error::NODE_OWNER_STALE_HANDLE,
        );
        assert_eq!(
            fs.readdir(&ctx, peer_dir, 0, 10).unwrap_err().code(),
            afs_error::NODE_OWNER_STALE_HANDLE,
        );
        assert_eq!(fs.write(&ctx, local_file.handle, 0, b"kept").unwrap(), 4);
        assert_eq!(
            local
                .state
                .lock()
                .unwrap()
                .ensure_peer_session("node-b", &access)
                .unwrap_err()
                .code(),
            afs_error::NODE_OWNER_STALE_HANDLE,
        );
        fs.release(&ctx, local_file.handle).unwrap();
    }

    fn insert_slow_remote_handle(
        fs: &OwnerFs,
        remote: Arc<SlowRemoteFiles>,
        identity_byte: u8,
    ) -> FileHandle {
        let local = fs.require_local().unwrap();
        let root_id = remote.root_id.clone();
        local.state.lock().unwrap().insert_remote_file_handle(
            test_grant(root_id.clone()),
            remote,
            files::RemoteFile {
                root_id,
                owner_node_id: "node-a".to_owned(),
                owner_session_id: "session-a".to_owned(),
                identity: files::FileIdentity(vec![identity_byte]),
                handle: vec![identity_byte],
            },
            false,
        )
    }

    #[test]
    fn independent_remote_file_handles_write_without_global_owner_lock() {
        let (_temp, fs, ctx) = fixture();
        let root_id = root::root_id_from_name(OsStr::new("job-42")).unwrap();
        let remote = Arc::new(SlowRemoteFiles::new(root_id, Duration::from_millis(120)));
        let first = insert_slow_remote_handle(&fs, remote.clone(), 1);
        let second = insert_slow_remote_handle(&fs, remote.clone(), 2);
        let fs = Arc::new(fs);
        let start = std::time::Instant::now();

        let left = {
            let fs = fs.clone();
            std::thread::spawn(move || fs.write(&ctx, first, 0, b"left").unwrap())
        };
        let right = {
            let fs = fs.clone();
            std::thread::spawn(move || fs.write(&ctx, second, 0, b"right").unwrap())
        };

        assert_eq!(left.join().unwrap(), 4);
        assert_eq!(right.join().unwrap(), 5);
        assert_eq!(remote.max_active_writes.load(Ordering::SeqCst), 2);
        assert!(
            start.elapsed() < Duration::from_millis(220),
            "independent handles serialized for {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn release_waits_for_same_handle_and_marks_it_closed() {
        let (_temp, fs, ctx) = fixture();
        let root_id = root::root_id_from_name(OsStr::new("job-42")).unwrap();
        let remote = Arc::new(SlowRemoteFiles::new(root_id, Duration::from_millis(120)));
        let handle = insert_slow_remote_handle(&fs, remote.clone(), 1);
        let fs = Arc::new(fs);
        let writer = {
            let fs = fs.clone();
            std::thread::spawn(move || fs.write(&ctx, handle, 0, b"same").unwrap())
        };

        let wait_started = std::time::Instant::now();
        while remote.started_writes.load(Ordering::SeqCst) == 0 {
            assert!(
                wait_started.elapsed() < Duration::from_secs(1),
                "remote write did not start"
            );
            std::thread::sleep(Duration::from_millis(1));
        }

        let release_start = std::time::Instant::now();
        fs.release(&ctx, handle).unwrap();
        assert_eq!(writer.join().unwrap(), 4);
        assert!(
            release_start.elapsed() >= Duration::from_millis(80),
            "release did not wait for the in-flight same-handle write"
        );
        assert_eq!(remote.release_calls.load(Ordering::SeqCst), 1);

        let error = fs.write(&ctx, handle, 0, b"again").unwrap_err();
        assert_eq!(error.code(), afs_error::NODE_OWNER_STALE_HANDLE);
    }

    #[test]
    fn remote_lookup_refreshes_cached_grant_after_home_restart() {
        let (_temp, fs, ctx, meta, remote, _root_id) = remote_fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };

        let workspace = fs.lookup(&ctx, root, OsStr::new("job-42")).unwrap();
        let child = fs
            .lookup(&ctx, workspace.inode, OsStr::new("a.txt"))
            .unwrap();

        assert_eq!(child.attributes.kind, FileKind::Regular);
        assert!(remote.restarted.load(Ordering::SeqCst));
        assert_eq!(meta.acquire_calls.load(Ordering::SeqCst), 2);
        assert_eq!(remote.lookup_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn remote_open_refreshes_cached_grant_after_home_restart() {
        let (_temp, fs, ctx, meta, remote, root_id) = remote_fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.lookup(&ctx, root, OsStr::new("job-42")).unwrap();
        let identity = files::FileIdentity(vec![1]);
        let child_inode = {
            let local = fs.require_local().unwrap();
            let mut state = local.state.lock().unwrap();
            state.inode_for_path(
                root_id,
                StoragePath::new("a.txt").unwrap(),
                identity,
                test_attrs(FileKind::Regular),
                FileKind::Regular,
            )
        };
        assert_eq!(workspace.attributes.kind, FileKind::Directory);

        let handle = fs
            .open(&ctx, backend_inode(child_inode), libc::O_RDONLY)
            .unwrap();

        assert!(remote.restarted.load(Ordering::SeqCst));
        assert_eq!(meta.acquire_calls.load(Ordering::SeqCst), 2);
        assert_eq!(remote.open_calls.load(Ordering::SeqCst), 2);
        fs.release(&ctx, handle).unwrap();
    }

    #[test]
    fn local_root_and_file_round_trip_without_per_write_meta() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-42"), 0o755).unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("log.txt"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        assert_eq!(fs.write(&ctx, created.handle, 0, b"hello").unwrap(), 5);
        fs.fsync(&ctx, created.handle, SyncMode::DataOnly).unwrap();
        let mut out = [0_u8; 5];
        assert_eq!(fs.read(&ctx, created.handle, 0, &mut out).unwrap(), 5);
        assert_eq!(&out, b"hello");
        fs.release(&ctx, created.handle).unwrap();
    }

    #[test]
    fn freshly_created_root_getattr_uses_real_directory_identity() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-42"), 0o755).unwrap();

        let attrs = fs.getattr(&ctx, workspace.inode, None).unwrap();

        assert_eq!(attrs.kind, FileKind::Directory);
    }

    #[test]
    fn old_fd_survives_unlink_and_same_name_recreate() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-42"), 0o755).unwrap();
        let first = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("same.txt"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&ctx, first.handle, 0, b"AAAA").unwrap();
        fs.unlink(&ctx, workspace.inode, OsStr::new("same.txt"))
            .unwrap();
        let second = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("same.txt"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&ctx, second.handle, 0, b"BBBB").unwrap();

        let mut old = [0_u8; 4];
        fs.read(&ctx, first.handle, 0, &mut old).unwrap();
        assert_eq!(&old, b"AAAA");
        let mut new = [0_u8; 4];
        fs.read(&ctx, second.handle, 0, &mut new).unwrap();
        assert_eq!(&new, b"BBBB");
        assert_ne!(first.entry.inode, second.entry.inode);
    }

    #[test]
    fn stale_open_with_truncate_does_not_modify_recreated_path() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-42"), 0o755).unwrap();
        let first = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("same.txt"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&ctx, first.handle, 0, b"AAAA").unwrap();
        fs.unlink(&ctx, workspace.inode, OsStr::new("same.txt"))
            .unwrap();
        let second = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("same.txt"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&ctx, second.handle, 0, b"BBBB").unwrap();

        let error = fs
            .open(&ctx, first.entry.inode, libc::O_WRONLY | libc::O_TRUNC)
            .unwrap_err();
        assert_eq!(error.code(), afs_error::NODE_OWNER_STALE_HANDLE);
        let mut new = [0_u8; 4];
        fs.read(&ctx, second.handle, 0, &mut new).unwrap();
        assert_eq!(&new, b"BBBB");
    }

    #[test]
    fn rename_no_replace_moves_when_destination_absent_and_preserves_existing_destination() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-42"), 0o755).unwrap();
        fs.create(
            &ctx,
            workspace.inode,
            OsStr::new("old"),
            0o644,
            libc::O_RDWR,
        )
        .unwrap();

        fs.rename(
            &ctx,
            workspace.inode,
            OsStr::new("old"),
            workspace.inode,
            OsStr::new("new"),
            RenameFlags(RENAME_NOREPLACE_FLAG),
        )
        .unwrap();
        assert!(fs.lookup(&ctx, workspace.inode, OsStr::new("new")).is_ok());

        fs.create(
            &ctx,
            workspace.inode,
            OsStr::new("old"),
            0o644,
            libc::O_RDWR,
        )
        .unwrap();
        let error = fs
            .rename(
                &ctx,
                workspace.inode,
                OsStr::new("old"),
                workspace.inode,
                OsStr::new("new"),
                RenameFlags(RENAME_NOREPLACE_FLAG),
            )
            .unwrap_err();
        assert_eq!(error.code(), afs_error::IO_ALREADY_EXISTS);
    }

    #[test]
    fn rename_across_roots_returns_exdev_io_error() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let left = fs.mkdir(&ctx, root, OsStr::new("left"), 0o755).unwrap();
        let right = fs.mkdir(&ctx, root, OsStr::new("right"), 0o755).unwrap();
        fs.create(&ctx, left.inode, OsStr::new("a.txt"), 0o644, libc::O_RDWR)
            .unwrap();

        let error = fs
            .rename(
                &ctx,
                left.inode,
                OsStr::new("a.txt"),
                right.inode,
                OsStr::new("a.txt"),
                RenameFlags(0),
            )
            .unwrap_err();

        assert_eq!(error.code(), afs_error::IO_CROSS_DEVICE);
        assert_eq!(crate::error::errno(&error), libc::EXDEV);
        assert!(
            error.message().contains("cross-device")
                || error.message().contains("Invalid cross-device")
        );
    }

    #[test]
    fn startup_recovery_hydrates_root_entries_from_local_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let meta = Arc::new(LocalMeta {
            node_id: "node-a".into(),
            session_id: "session-a".into(),
            next_epoch: Mutex::new(1),
            recover_calls: Mutex::new(0),
            active: Mutex::new(HashMap::new()),
        });
        let ctx = RequestContext {
            uid: 1000,
            gid: 1000,
            pid: 42,
            umask: 0,
        };
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        {
            let disk = Arc::new(LocalFs::open(temp.path()).unwrap());
            let roots = Arc::new(RootManager::new(
                "node-a".into(),
                "session-a".into(),
                meta.clone(),
                disk.clone(),
            ));
            let fs = OwnerFs::new_local(roots, disk);
            fs.mkdir(&ctx, root, OsStr::new("job-42"), 0o755).unwrap();
        }

        let disk = Arc::new(LocalFs::open(temp.path()).unwrap());
        let recovered_roots = Arc::new(RootManager::new(
            "node-a".into(),
            "session-b".into(),
            meta.clone(),
            disk.clone(),
        ));
        let recovered = OwnerFs::new_local(recovered_roots, disk);
        let entry = recovered.lookup(&ctx, root, OsStr::new("job-42")).unwrap();
        assert_eq!(entry.attributes.kind, FileKind::Directory);
        assert_eq!(*meta.recover_calls.lock().unwrap(), 1);
    }

    #[test]
    fn local_directory_listing_uses_backend_entries() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-42"), 0o755).unwrap();
        fs.mkdir(&ctx, workspace.inode, OsStr::new("logs"), 0o755)
            .unwrap();
        fs.create(
            &ctx,
            workspace.inode,
            OsStr::new("README"),
            0o644,
            libc::O_RDWR,
        )
        .unwrap();
        let dir = fs.opendir(&ctx, workspace.inode).unwrap();
        let names: Vec<_> = fs
            .readdir(&ctx, dir, 0, 10)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(
            names,
            vec![OsString::from("README"), OsString::from("logs")]
        );
    }

    #[test]
    fn renamed_directory_keeps_cached_descendant_inodes_at_new_path() {
        let (_temp, fs, ctx) = fixture();
        let root = backend_inode(OWNERFS_ROOT_INODE);
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job"), 0o755).unwrap();
        let dir = fs
            .mkdir(&ctx, workspace.inode, OsStr::new("before"), 0o755)
            .unwrap();
        let nested = fs
            .mkdir(&ctx, dir.inode, OsStr::new("nested"), 0o755)
            .unwrap();
        let child = fs
            .create(&ctx, nested.inode, OsStr::new("file"), 0o644, libc::O_RDWR)
            .unwrap();
        fs.rename(
            &ctx,
            workspace.inode,
            OsStr::new("before"),
            workspace.inode,
            OsStr::new("after"),
            RenameFlags(0),
        )
        .unwrap();
        fs.open(&ctx, child.entry.inode, libc::O_RDONLY).unwrap();
    }

    #[test]
    fn rename_uncached_source_purges_cached_destination_path() {
        let (_temp, fs, ctx) = fixture();
        let root = backend_inode(OWNERFS_ROOT_INODE);
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job"), 0o755).unwrap();
        let cached = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("destination"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        let local = fs.local.as_ref().unwrap();
        let root_id = local.directory_record(workspace.inode).unwrap().root_id;
        let mut state = local.state.lock().unwrap();
        let destination = StoragePath::new("destination").unwrap();
        assert_eq!(
            state.paths.get(&(root_id.clone(), destination.clone())),
            Some(&cached.entry.inode.value)
        );
        state.rename_path(
            root_id.clone(),
            StoragePath::new("uncached-source").unwrap(),
            destination.clone(),
        );
        assert!(!state.paths.contains_key(&(root_id, destination)));
    }
}

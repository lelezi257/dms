//! OwnerFs：可修改、以根归属和计算亲和为核心的文件后端。
//!
//! 一级目录是 OwnerFs 的授权单位；根内文件仍是本机普通文件。本地 Home
//! 热路径只检查已缓存的 RootGrant，然后直接调用 LocalFs，不把每次写转换为
//! Chunk，也不逐写访问 Meta。远端/P2P 后续复用相同文件身份与句柄语义。

use std::{
    collections::{HashMap, HashSet},
    ffi::{CString, OsStr, OsString},
    fs, io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, UNIX_EPOCH},
};

use afs_error::{Error, Result};
use fuser::Notifier;

use self::root::{PresentedRootAccess, RootGrant, RootId, RootManager, RootRight};
use super::{
    Backend,
    locks::{LockError, LockRequest, LockTable, LockTableLimits, LockWaiterId, LockWaiterOutcome},
    types::{
        AttributeChange, BackendInode, CreatedFile, DirectoryEntry, DirectoryHandle, Entry,
        FileAttributes, FileHandle, FileKind, FileLockConflict, FileLockKind, FileLockOwner,
        FileLockType, OpenOptions, ReleaseKind, RenameFlags, RequestContext, SetAttrOptions,
        SpecialFileKind, SyncMode, WriteOptions,
    },
};
use crate::node::storage::{
    DirectoryHandle as StorageDirectoryHandle, FileHandle as StorageFileHandle, FileStore, LocalFs,
    OpenSpec, RenameMode, StoragePath,
};

pub mod catalog;
pub mod files;
pub mod native;
pub mod remote;
pub mod root;

const OWNERFS_ROOT_INODE: u64 = 1;

/// OwnerFs backend entry. `new()` remains an unsupported skeleton for the
/// current VFS bootstrap; `new_local()` is the real local implementation used
/// by tests and later Node startup wiring.
pub struct OwnerFs {
    local: Option<Arc<LocalOwnerFs>>,
    private_cache: Arc<Mutex<PrivateFuseCache>>,
    workspace_events: OnceLock<native::WorkspaceEventSender>,
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
            workspace_events: OnceLock::new(),
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
            workspace_events: OnceLock::new(),
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
            workspace_events: OnceLock::new(),
        }
    }

    /// Install the post-reply hint sink once, before the future opt-in mount
    /// bootstrap. This does not change cache policy or grant native admission.
    pub fn register_workspace_events(
        &self,
        sender: native::WorkspaceEventSender,
    ) -> io::Result<()> {
        self.workspace_events
            .set(sender)
            .map_err(|_| io::Error::from_raw_os_error(libc::EALREADY))
    }

    /// Called by the FUSE adapter only after a successful root mkdir reply.
    /// Publishing performs no authority lookup, mount, or manager transaction.
    pub(crate) fn workspace_created(&self, inode: BackendInode, name: &OsStr) {
        if let Some(sender) = self.workspace_events.get() {
            // Full/disconnected publication sets the observable rescan flag.
            // The successful mkdir is not revoked by a failed optional hint.
            let _ = sender.try_publish(native::WorkspaceCreated {
                name: name.to_os_string(),
                inode,
            });
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
        let mut first_error = local.retry_pending_remote_lock_cleanup().err();
        if let Err(error) = local.reap_lock_authorities() {
            first_error.get_or_insert(error);
        }
        let mut reclaimed = 0;
        for (node_id, session_id) in local.peer_handle_sessions()? {
            match local.roots.current_node_session(&node_id) {
                Ok(current) if current.as_deref() != Some(&session_id) => {
                    match local.reap_peer_session(&node_id, &session_id) {
                        Ok(count) => reclaimed += count,
                        Err(error) => {
                            first_error.get_or_insert(error);
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(reclaimed), Err)
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
    pub fn getlk(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        request: LockRequest,
    ) -> Result<Option<FileLockConflict>> {
        self.local.peer_get_file_lock(peer, access, file, request)
    }
    pub fn setlk(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        request: LockRequest,
        waiter: Option<LockWaiterId>,
    ) -> Result<()> {
        self.local
            .peer_set_file_lock(peer, access, file, request, waiter)
    }
    pub fn cancel_lock_wait(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        waiter: LockWaiterId,
    ) -> Result<LockWaiterOutcome> {
        self.local.peer_cancel_file_lock(peer, access, waiter)
    }
    pub fn acknowledge_lock_wait(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        waiter: LockWaiterId,
    ) -> Result<()> {
        self.local.peer_acknowledge_lock_wait(peer, access, waiter)
    }
    pub fn release_locks(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        owner: FileLockOwner,
        kind: ReleaseKind,
    ) -> Result<()> {
        self.local
            .peer_release_file_locks(peer, access, file, owner, kind)
    }
    pub fn release_lock_session(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        scope: &str,
    ) -> Result<()> {
        self.local
            .peer_release_file_lock_session(peer, access, scope)
    }

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

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        mode: u32,
        expected_parent: &files::FileIdentity,
    ) -> Result<remote::RemoteCreatedFile> {
        self.create_with_options(
            ctx,
            peer_node_id,
            access,
            path,
            flags,
            mode,
            expected_parent,
            OpenOptions::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_with_options(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        mode: u32,
        expected_parent: &files::FileIdentity,
        options: OpenOptions,
    ) -> Result<remote::RemoteCreatedFile> {
        self.local.peer_create_with_options(
            ctx,
            peer_node_id,
            access,
            path,
            flags,
            mode,
            expected_parent,
            options,
        )
    }

    #[allow(clippy::too_many_arguments)] // Mirrors the authenticated OwnerFs peer contract.
    pub fn mknod(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        kind: SpecialFileKind,
        mode: u32,
        expected_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.local
            .peer_mknod(ctx, peer_node_id, access, path, kind, mode, expected_parent)
    }

    pub fn mkdir(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        mode: u32,
        expected_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.local
            .peer_mkdir(ctx, peer_node_id, access, path, mode, expected_parent)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn setattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        file: Option<&files::RemoteFile>,
        change: &AttributeChange,
    ) -> Result<files::OwnerEntry> {
        self.setattr_with_options(
            ctx,
            peer_node_id,
            access,
            path,
            expected,
            file,
            change,
            SetAttrOptions::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn setattr_with_options(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        file: Option<&files::RemoteFile>,
        change: &AttributeChange,
        options: SetAttrOptions,
    ) -> Result<files::OwnerEntry> {
        self.local.peer_setattr_with_options(
            ctx,
            peer_node_id,
            access,
            path,
            expected,
            file,
            change,
            options,
        )
    }

    pub fn getxattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
        name: &OsStr,
    ) -> Result<Vec<u8>> {
        self.local
            .peer_getxattr(ctx, peer_node_id, access, path, expected, name)
    }

    pub fn listxattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
    ) -> Result<Vec<u8>> {
        self.local
            .peer_listxattr(ctx, peer_node_id, access, path, expected)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn setxattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
        name: &OsStr,
        value: &[u8],
        flags: i32,
    ) -> Result<()> {
        self.local.peer_setxattr(
            ctx,
            peer_node_id,
            access,
            path,
            expected,
            name,
            value,
            flags,
        )
    }

    pub fn removexattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
        name: &OsStr,
    ) -> Result<()> {
        self.local
            .peer_removexattr(ctx, peer_node_id, access, path, expected, name)
    }

    pub fn unlink(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        expected_parent: &files::FileIdentity,
    ) -> Result<()> {
        self.local
            .peer_unlink(ctx, peer_node_id, access, path, expected, expected_parent)
    }

    pub fn rmdir(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        expected_parent: &files::FileIdentity,
    ) -> Result<()> {
        self.local
            .peer_rmdir(ctx, peer_node_id, access, path, expected, expected_parent)
    }

    // Keep the agreed peer contract explicit instead of allocating a request
    // object just to satisfy the argument-count lint on this cold operation.
    #[allow(clippy::too_many_arguments)]
    pub fn rename(
        &self,
        ctx: &RequestContext,
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
            ctx,
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
        self.open_with_options(
            peer_node_id,
            access,
            path,
            flags,
            expected,
            OpenOptions::default(),
        )
    }

    pub fn open_with_options(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        expected: Option<&files::FileIdentity>,
        options: OpenOptions,
    ) -> Result<(files::RemoteFile, FileAttributes, Option<Vec<u8>>)> {
        self.local
            .peer_open_with_options(peer_node_id, access, path, flags, expected, options)
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
        self.write_with_options(
            peer_node_id,
            access,
            file,
            offset,
            data,
            WriteOptions::default(),
        )
    }

    pub fn write_with_options(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        offset: u64,
        data: &[u8],
        options: WriteOptions,
    ) -> Result<usize> {
        self.local
            .peer_write_with_options(peer_node_id, access, file, offset, data, options)
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
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
    ) -> Result<Vec<u8>> {
        self.local
            .peer_readlink(peer_node_id, access, path, expected)
    }

    pub fn symlink(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        target: &OsStr,
        expected_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.local
            .peer_symlink(ctx, peer_node_id, access, path, target, expected_parent)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn link(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        old_path: &OsStr,
        new_path: &OsStr,
        expected_old: &files::FileIdentity,
        expected_new_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.local.peer_link(
            ctx,
            peer_node_id,
            access,
            old_path,
            new_path,
            expected_old,
            expected_new_parent,
        )
    }
}

/// Resolves a Home node endpoint into a `RemoteFiles` client.
///
/// Node wiring can back this with `RootMeta::lookup_node_endpoint` plus
/// `node::rpc::peer::connect_owner_files_client`. Tests can inject an in-memory
/// implementation. OwnerFs keeps the trait here so B-side remote dispatch does
/// not depend on Proto/gRPC/RDMA types.
pub trait RemoteFilesFactory: Send + Sync {
    fn supports_advisory_locks(&self) -> bool {
        false
    }
    fn connect(&self, home_node_id: &str) -> Result<Arc<dyn remote::RemoteFiles>>;

    fn supports_killpriv_v2(&self) -> bool {
        false
    }
}

impl Backend for OwnerFs {
    fn root_inode(&self) -> BackendInode {
        backend_inode(OWNERFS_ROOT_INODE)
    }

    fn supports_killpriv_v2(&self) -> bool {
        self.local.as_ref().is_some_and(|local| {
            local
                .remote_factory
                .as_ref()
                .is_none_or(|factory| factory.supports_killpriv_v2())
        })
    }

    fn supports_advisory_locks(&self) -> bool {
        self.local.as_ref().is_some_and(|local| {
            local
                .remote_factory
                .as_ref()
                .is_none_or(|factory| factory.supports_advisory_locks())
        })
    }

    fn getlk(
        &self,
        _: &RequestContext,
        _: BackendInode,
        handle: FileHandle,
        request: LockRequest,
    ) -> Result<Option<FileLockConflict>> {
        self.require_local()?.get_file_lock(handle, request)
    }
    fn setlk(
        &self,
        _: &RequestContext,
        _: BackendInode,
        handle: FileHandle,
        request: LockRequest,
        waiter: Option<LockWaiterId>,
    ) -> Result<()> {
        self.require_local()?.set_file_lock(handle, request, waiter)
    }
    fn cancel_lock_wait(&self, waiter: LockWaiterId) -> Result<()> {
        self.require_local()?.cancel_file_lock(waiter).map(|_| ())
    }
    fn release_locks(
        &self,
        _: &RequestContext,
        _: BackendInode,
        handle: FileHandle,
        owner: FileLockOwner,
        kind: ReleaseKind,
    ) -> Result<()> {
        self.require_local()?
            .release_file_locks(handle, owner, kind)
    }
    fn release_lock_session(&self, ingress_session_id: &str) -> Result<()> {
        self.require_local()?
            .release_file_lock_session(ingress_session_id)
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
        ctx: &RequestContext,
        inode: BackendInode,
        handle: Option<FileHandle>,
        change: &AttributeChange,
    ) -> Result<FileAttributes> {
        self.setattr_with_options(ctx, inode, handle, change, SetAttrOptions::default())
    }

    fn setattr_with_options(
        &self,
        ctx: &RequestContext,
        inode: BackendInode,
        handle: Option<FileHandle>,
        change: &AttributeChange,
        options: SetAttrOptions,
    ) -> Result<FileAttributes> {
        self.require_local()?
            .setattr_with_options(ctx, inode, handle, change, options)
    }

    fn create(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
        flags: i32,
    ) -> Result<CreatedFile> {
        self.create_with_options(ctx, parent, name, mode, flags, OpenOptions::default())
    }

    fn create_with_options(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
        flags: i32,
        options: OpenOptions,
    ) -> Result<CreatedFile> {
        self.require_local()?.create_with_options(
            ctx,
            parent,
            name,
            mode & !ctx.umask,
            flags,
            options,
        )
    }

    fn mknod(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        kind: SpecialFileKind,
        mode: u32,
    ) -> Result<Entry> {
        self.require_local()?
            .mknod(ctx, parent, name, kind, mode & !ctx.umask)
    }

    fn open(&self, ctx: &RequestContext, inode: BackendInode, flags: i32) -> Result<FileHandle> {
        self.open_with_options(ctx, inode, flags, OpenOptions::default())
    }

    fn open_with_options(
        &self,
        _: &RequestContext,
        inode: BackendInode,
        flags: i32,
        options: OpenOptions,
    ) -> Result<FileHandle> {
        self.require_local()?
            .open_with_options(inode, flags, options)
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
        ctx: &RequestContext,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize> {
        self.write_with_options(ctx, handle, offset, data, WriteOptions::default())
    }

    fn write_with_options(
        &self,
        _: &RequestContext,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
        options: WriteOptions,
    ) -> Result<usize> {
        self.require_local()?
            .write_with_options(handle, offset, data, options)
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
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
    ) -> Result<Entry> {
        self.require_local()?
            .mkdir(ctx, parent, name, mode & !ctx.umask)
    }

    fn unlink(&self, ctx: &RequestContext, parent: BackendInode, name: &OsStr) -> Result<()> {
        self.require_local()?.unlink(ctx, parent, name)
    }

    fn rmdir(&self, ctx: &RequestContext, parent: BackendInode, name: &OsStr) -> Result<()> {
        self.require_local()?.rmdir(ctx, parent, name)
    }

    fn rename(
        &self,
        ctx: &RequestContext,
        from_parent: BackendInode,
        from_name: &OsStr,
        to_parent: BackendInode,
        to_name: &OsStr,
        flags: RenameFlags,
    ) -> Result<()> {
        self.require_local()?
            .rename(ctx, from_parent, from_name, to_parent, to_name, flags)
    }

    fn symlink(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        target: &OsStr,
    ) -> Result<Entry> {
        self.require_local()?.symlink(ctx, parent, name, target)
    }

    fn readlink(&self, _: &RequestContext, inode: BackendInode) -> Result<OsString> {
        self.require_local()?.readlink(inode)
    }

    fn link(
        &self,
        ctx: &RequestContext,
        inode: BackendInode,
        new_parent: BackendInode,
        name: &OsStr,
    ) -> Result<Entry> {
        self.require_local()?.link(ctx, inode, new_parent, name)
    }

    fn getxattr(&self, ctx: &RequestContext, inode: BackendInode, name: &OsStr) -> Result<Vec<u8>> {
        self.require_local()?.getxattr(ctx, inode, name)
    }

    fn listxattr(&self, ctx: &RequestContext, inode: BackendInode) -> Result<Vec<u8>> {
        self.require_local()?.listxattr(ctx, inode)
    }

    fn setxattr(
        &self,
        ctx: &RequestContext,
        inode: BackendInode,
        name: &OsStr,
        value: &[u8],
        flags: i32,
    ) -> Result<()> {
        self.require_local()?
            .setxattr(ctx, inode, name, value, flags)
    }

    fn removexattr(&self, ctx: &RequestContext, inode: BackendInode, name: &OsStr) -> Result<()> {
        self.require_local()?.removexattr(ctx, inode, name)
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
    locks: Mutex<OwnerLockRegistry>,
    private_cache: Arc<Mutex<PrivateFuseCache>>,
}

#[derive(Clone)]
struct RemoteRoot {
    grant: RootGrant,
    files: Arc<dyn remote::RemoteFiles>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct OwnerLockKey {
    root_id: RootId,
    epoch: u64,
    identity: Vec<u8>,
}

#[derive(Clone)]
enum OwnerLockTarget {
    Local {
        key: OwnerLockKey,
        table: Arc<LockTable>,
    },
    Remote {
        grant: Arc<RootGrant>,
        files: Arc<dyn remote::RemoteFiles>,
        file: files::RemoteFile,
    },
}

#[derive(Clone)]
struct OwnerLockRoute {
    target: OwnerLockTarget,
    owner: FileLockOwner,
    kind: FileLockKind,
    completed: bool,
    acknowledged: bool,
}

#[derive(Default)]
struct OwnerLockRegistry {
    admission_exhausted: bool,
    tables: HashMap<OwnerLockKey, Arc<LockTable>>,
    waiters: HashMap<LockWaiterId, OwnerLockRoute>,
    cancelled: HashSet<LockWaiterId>,
    closed_sessions: HashSet<String>,
    active_scopes: HashSet<String>,
    remote_targets: HashMap<String, Vec<OwnerLockTarget>>,
    pending_remote_cleanup: HashSet<String>,
    peer_sessions: HashMap<(String, String), HashSet<String>>,
}

const MAX_OWNER_LOCK_TABLES: usize = 512;
const MAX_OWNER_LOCK_WAITERS: usize = 1024;
const MAX_OWNER_LOCK_SCOPES: usize = 4096;

impl OwnerLockRegistry {
    fn table(&mut self, key: OwnerLockKey) -> Result<Arc<LockTable>> {
        if let Some(table) = self.tables.get(&key) {
            return Ok(table.clone());
        }
        if self.tables.len() >= MAX_OWNER_LOCK_TABLES {
            self.tables.retain(|_, table| {
                Arc::strong_count(table) != 1 || !table.is_idle().unwrap_or(false)
            });
        }
        if self.tables.len() >= MAX_OWNER_LOCK_TABLES {
            return Err(owner_lock_capacity());
        }
        let table = Arc::new(LockTable::new(LockTableLimits {
            max_locks: 1024,
            max_waiters: 64,
            max_cancelled_waiters: 128,
        }));
        self.tables.insert(key, table.clone());
        Ok(table)
    }

    fn admit_scope(&mut self, scope: &str) -> Result<()> {
        if scope.is_empty() || scope.len() > 1024 {
            return Err(owner_lock_invalid());
        }
        if self.closed_sessions.contains(scope) {
            return Err(owner_lock_error(LockError::Interrupted));
        }
        if !self.active_scopes.contains(scope)
            && (self.admission_exhausted || self.active_scopes.len() >= MAX_OWNER_LOCK_SCOPES)
        {
            return Err(owner_lock_capacity());
        }
        self.active_scopes.insert(scope.to_owned());
        Ok(())
    }

    fn register_waiter(
        &mut self,
        waiter: &LockWaiterId,
        target: OwnerLockTarget,
        owner: FileLockOwner,
        kind: FileLockKind,
    ) -> Result<()> {
        self.admit_scope(&waiter.ingress_session_id)?;
        if self.cancelled.contains(waiter) {
            return Err(owner_lock_error(LockError::Interrupted));
        }
        if self.waiters.len() >= MAX_OWNER_LOCK_WAITERS {
            return Err(owner_lock_capacity());
        }
        if self.waiters.contains_key(waiter) {
            return Err(owner_lock_invalid());
        }
        self.waiters.insert(
            waiter.clone(),
            OwnerLockRoute {
                target,
                owner,
                kind,
                completed: false,
                acknowledged: false,
            },
        );
        Ok(())
    }

    fn cancel(&mut self, waiter: LockWaiterId) -> Result<Option<OwnerLockTarget>> {
        if let Some(target) = self.waiters.get(&waiter) {
            return Ok(Some(target.target.clone()));
        }
        self.admit_scope(&waiter.ingress_session_id)?;
        if self.cancelled.len() >= MAX_OWNER_LOCK_WAITERS {
            return Err(owner_lock_capacity());
        }
        self.cancelled.insert(waiter);
        Ok(None)
    }

    fn close_scope(&mut self, scope: &str) -> Result<()> {
        self.close_scope_with_limit(scope, MAX_OWNER_LOCK_SCOPES)
    }

    fn close_scope_with_limit(&mut self, scope: &str, limit: usize) -> Result<()> {
        if !self.closed_sessions.contains(scope) && self.closed_sessions.len() >= limit {
            // The missing tombstone must never admit this closed scope again,
            // but exhaustion is not evidence that other live scopes lost their
            // authority. Keep their existing locks and allow their cleanup.
            self.admission_exhausted = true;
        } else {
            self.closed_sessions.insert(scope.to_owned());
        }
        self.active_scopes.remove(scope);
        self.cancelled
            .retain(|waiter| waiter.ingress_session_id != scope);
        self.waiters
            .retain(|waiter, _| waiter.ingress_session_id != scope);
        for sessions in self.peer_sessions.values_mut() {
            sessions.remove(scope);
        }
        Ok(())
    }
}

fn owner_lock_error(error: LockError) -> Error {
    Error::from(io::Error::from_raw_os_error(error.errno()))
}
fn owner_lock_capacity() -> Error {
    owner_lock_error(LockError::Capacity)
}
fn owner_lock_invalid() -> Error {
    owner_lock_error(LockError::InvalidRange)
}

fn owner_local_lock_scope(raw: &str) -> String {
    format!("local:{}:{raw}", raw.len())
}
fn owner_peer_lock_scope(peer: &str, access: &PresentedRootAccess, raw: &str) -> String {
    format!(
        "peer:{}:{peer}:{}:{}:{}:{}:{}:{}:{raw}",
        peer.len(),
        access.session_id.len(),
        access.session_id,
        access.id.0.len(),
        access.id.0,
        access.epoch,
        raw.len()
    )
}

impl LocalOwnerFs {
    fn lock_target(
        &self,
        handle: FileHandle,
        request: Option<&LockRequest>,
    ) -> Result<OwnerLockTarget> {
        let slot = self.open_file_handle(handle)?;
        let slot = slot.lock().map_err(|_| poisoned())?;
        slot.ensure_open()?;
        match &slot.file {
            OpenFileHandle::Local(file) => {
                if let Some(request) = request
                    && request.kind == FileLockKind::Posix
                    && ((request.lock_type == FileLockType::Write && !file.writable)
                        || (request.lock_type == FileLockType::Read && !file.readable))
                {
                    return Err(Error::from(io::Error::from_raw_os_error(libc::EBADF)));
                }
                let root_id = file.handle.root_id.clone();
                let identity = file.handle.identity.0.clone();
                drop(slot);
                let root_use = self.roots.enter_root(&root_id, RootRight::Lookup)?;
                let key = OwnerLockKey {
                    root_id,
                    epoch: root_use.grant().epoch,
                    identity,
                };
                drop(root_use);
                let table = self
                    .locks
                    .lock()
                    .map_err(|_| poisoned())?
                    .table(key.clone())?;
                Ok(OwnerLockTarget::Local { key, table })
            }
            OpenFileHandle::Remote(file) => Ok(OwnerLockTarget::Remote {
                grant: Arc::new(file.grant.clone()),
                files: file.files.clone(),
                file: file.handle.clone(),
            }),
        }
    }

    fn validate_lock_target(&self, target: &OwnerLockTarget) -> Result<()> {
        if let OwnerLockTarget::Local { key, table } = target {
            let valid = self
                .roots
                .enter_root(&key.root_id, RootRight::Lookup)
                .and_then(|root_use| {
                    if root_use.grant().epoch == key.epoch {
                        Ok(())
                    } else {
                        Err(stale("Owner lock root epoch changed"))
                    }
                });
            if valid.is_err() {
                table.invalidate().map_err(owner_lock_error)?;
            }
            valid?;
        }
        Ok(())
    }

    fn get_file_lock(
        &self,
        handle: FileHandle,
        mut request: LockRequest,
    ) -> Result<Option<FileLockConflict>> {
        let target = self.lock_target(handle, None)?;
        match target {
            OwnerLockTarget::Local { table, .. } => {
                request.owner.ingress_session_id =
                    owner_local_lock_scope(&request.owner.ingress_session_id);
                self.locks
                    .lock()
                    .map_err(|_| poisoned())?
                    .admit_scope(&request.owner.ingress_session_id)?;
                table.getlk(&request).map_err(owner_lock_error)
            }
            OwnerLockTarget::Remote { grant, files, file } => files.getlk(&grant, &file, request),
        }
    }

    fn set_file_lock(
        &self,
        handle: FileHandle,
        mut request: LockRequest,
        mut waiter: Option<LockWaiterId>,
    ) -> Result<()> {
        let target = self.lock_target(handle, Some(&request))?;
        let raw_scope = request.owner.ingress_session_id.clone();
        // Local wrappers retain raw ingress identities for interrupt routing;
        // Home prefixes peer requests separately after authentication.
        if matches!(target, OwnerLockTarget::Local { .. }) {
            request.owner.ingress_session_id = owner_local_lock_scope(&raw_scope);
        }
        {
            let mut locks = self.locks.lock().map_err(|_| poisoned())?;
            locks.admit_scope(&owner_local_lock_scope(&raw_scope))?;
            if matches!(target, OwnerLockTarget::Remote { .. }) {
                let tracked = locks.remote_targets.get(&raw_scope).is_some_and(|targets| {
                    targets.iter().any(|old| match (old, &target) {
                        (
                            OwnerLockTarget::Remote { grant: a, .. },
                            OwnerLockTarget::Remote { grant: b, .. },
                        ) => a == b,
                        _ => false,
                    })
                });
                if !tracked {
                    let count: usize = locks.remote_targets.values().map(Vec::len).sum();
                    if count >= MAX_OWNER_LOCK_SCOPES {
                        return Err(owner_lock_capacity());
                    }
                    locks
                        .remote_targets
                        .entry(raw_scope.clone())
                        .or_default()
                        .push(target.clone());
                }
            }
            if let Some(id) = &waiter {
                if id.ingress_session_id != raw_scope {
                    return Err(owner_lock_invalid());
                }
                locks.register_waiter(id, target.clone(), request.owner.clone(), request.kind)?;
            }
        }
        let raw_waiter = waiter.clone();
        let result = match &target {
            OwnerLockTarget::Local { table, .. } => {
                if let Some(id) = &mut waiter {
                    id.ingress_session_id = owner_local_lock_scope(&id.ingress_session_id);
                }
                match waiter {
                    Some(id) => table.setlk_blocking(request, id),
                    None => table.setlk_nonblocking(request),
                }
                .map_err(owner_lock_error)
            }
            OwnerLockTarget::Remote { grant, files, file } => {
                files.setlk(grant, file, request, waiter)
            }
        };
        if let Err(error) = result {
            // A local table failure definitively completed without acquiring a
            // lock. Retire only that waiter; unrelated owner locks remain.
            if let (Some(id), OwnerLockTarget::Local { table, .. }) = (&raw_waiter, &target) {
                let scoped = LockWaiterId {
                    ingress_session_id: owner_local_lock_scope(&id.ingress_session_id),
                    request_id: id.request_id,
                };
                let _ = table.acknowledge_waiter(&scoped);
                self.locks
                    .lock()
                    .map_err(|_| poisoned())?
                    .waiters
                    .remove(id);
            } else if let Some(id) = &raw_waiter {
                // Remote ambiguity is retained. A received definitive server
                // rejection is acknowledged inside the remote client.
                if error.code() != afs_error::IO_UNAVAILABLE {
                    self.locks
                        .lock()
                        .map_err(|_| poisoned())?
                        .waiters
                        .remove(id);
                }
            }
            return Err(error);
        }
        if let Err(error) = self.validate_lock_target(&target) {
            if let Some(id) = &raw_waiter {
                self.locks
                    .lock()
                    .map_err(|_| poisoned())?
                    .waiters
                    .remove(id);
            }
            return Err(error);
        }
        if let Some(id) = raw_waiter {
            if let OwnerLockTarget::Local { table, .. } = &target {
                let scoped = LockWaiterId {
                    ingress_session_id: owner_local_lock_scope(&id.ingress_session_id),
                    request_id: id.request_id,
                };
                table
                    .acknowledge_waiter(&scoped)
                    .map_err(owner_lock_error)?;
            }
            self.locks
                .lock()
                .map_err(|_| poisoned())?
                .waiters
                .remove(&id);
        }
        Ok(())
    }

    fn cancel_file_lock(&self, waiter: LockWaiterId) -> Result<LockWaiterOutcome> {
        let target = self
            .locks
            .lock()
            .map_err(|_| poisoned())?
            .cancel(waiter.clone())?;
        match target {
            Some(OwnerLockTarget::Local { table, .. }) => table
                .cancel_waiter_with_outcome(LockWaiterId {
                    ingress_session_id: owner_local_lock_scope(&waiter.ingress_session_id),
                    ..waiter
                })
                .map_err(owner_lock_error),
            Some(OwnerLockTarget::Remote { grant, files, .. }) => {
                files.cancel_lock_wait(&grant, waiter)
            }
            None => Ok(LockWaiterOutcome::Unknown),
        }
    }

    fn forget_lock_routes(
        &self,
        target: &OwnerLockTarget,
        owner: &FileLockOwner,
        kind: ReleaseKind,
    ) -> Result<()> {
        let kind = match kind {
            ReleaseKind::PosixOwner => FileLockKind::Posix,
            ReleaseKind::FlockOwner => FileLockKind::Flock,
        };
        self.locks
            .lock()
            .map_err(|_| poisoned())?
            .waiters
            .retain(|_, route| {
                route.owner != *owner
                    || route.kind != kind
                    || !same_owner_lock_target(&route.target, target)
            });
        Ok(())
    }

    fn existing_file_lock_targets(&self, handle: FileHandle) -> Result<Vec<OwnerLockTarget>> {
        let slot = self.open_file_handle(handle)?;
        let slot = slot.lock().map_err(|_| poisoned())?;
        slot.ensure_open()?;
        match &slot.file {
            OpenFileHandle::Local(file) => {
                let root = file.handle.root_id.clone();
                let identity = file.handle.identity.0.clone();
                drop(slot);
                Ok(self
                    .locks
                    .lock()
                    .map_err(|_| poisoned())?
                    .tables
                    .iter()
                    .filter(|(key, _)| key.root_id == root && key.identity == identity)
                    .map(|(key, table)| OwnerLockTarget::Local {
                        key: key.clone(),
                        table: table.clone(),
                    })
                    .collect())
            }
            OpenFileHandle::Remote(file) => Ok(vec![OwnerLockTarget::Remote {
                grant: Arc::new(file.grant.clone()),
                files: file.files.clone(),
                file: file.handle.clone(),
            }]),
        }
    }

    fn release_file_locks(
        &self,
        handle: FileHandle,
        owner: FileLockOwner,
        kind: ReleaseKind,
    ) -> Result<()> {
        let mut first = None;
        for target in self.existing_file_lock_targets(handle)? {
            let mut owner = owner.clone();
            let result = match &target {
                OwnerLockTarget::Local { table, .. } => {
                    owner.ingress_session_id = owner_local_lock_scope(&owner.ingress_session_id);
                    release_owner_locks(table, &owner, kind)
                }
                OwnerLockTarget::Remote { grant, files, file } => {
                    files.release_locks(grant, file, owner.clone(), kind)
                }
            };
            match result {
                Ok(()) => {
                    if let Err(error) = self.forget_lock_routes(&target, &owner, kind) {
                        first.get_or_insert(error);
                    }
                }
                Err(error) => {
                    first.get_or_insert(error);
                }
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn release_file_lock_session(&self, raw: &str) -> Result<()> {
        let (tables, targets) = {
            let mut locks = self.locks.lock().map_err(|_| poisoned())?;
            locks.close_scope(&owner_local_lock_scope(raw))?;
            // Late queued local requests use the raw identity at registration.
            locks.close_scope(raw)?;
            (locks.tables.values().cloned().collect::<Vec<_>>(), {
                let targets = locks.remote_targets.get(raw).cloned().unwrap_or_default();
                if !targets.is_empty() {
                    locks.pending_remote_cleanup.insert(raw.to_owned());
                }
                targets
            })
        };
        let mut first = None;
        for table in tables {
            if let Err(error) = table
                .release_session(&owner_local_lock_scope(raw))
                .map_err(owner_lock_error)
            {
                first.get_or_insert(error);
            }
        }
        if let Err(error) = self.flush_remote_lock_cleanup(raw, targets) {
            first.get_or_insert(error);
        }
        first.map_or(Ok(()), Err)
    }

    fn flush_remote_lock_cleanup(&self, raw: &str, targets: Vec<OwnerLockTarget>) -> Result<()> {
        let mut first = None;
        for target in targets {
            if let OwnerLockTarget::Remote { grant, files, .. } = &target {
                match files.release_lock_session(grant, raw) {
                    Ok(()) => {
                        let mut locks = self.locks.lock().map_err(|_| poisoned())?;
                        if let Some(remaining) = locks.remote_targets.get_mut(raw) {
                            remaining.retain(|old| !same_owner_lock_target(old, &target));
                            if remaining.is_empty() {
                                locks.remote_targets.remove(raw);
                                locks.pending_remote_cleanup.remove(raw);
                            }
                        }
                    }
                    Err(error) => {
                        first.get_or_insert(error);
                    }
                }
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn retry_pending_remote_lock_cleanup(&self) -> Result<()> {
        let pending = {
            let locks = self.locks.lock().map_err(|_| poisoned())?;
            locks
                .pending_remote_cleanup
                .iter()
                .map(|raw| {
                    (
                        raw.clone(),
                        locks.remote_targets.get(raw).cloned().unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut first = None;
        for (raw, targets) in pending {
            if let Err(error) = self.flush_remote_lock_cleanup(&raw, targets) {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn peer_lock_target(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        request: Option<&LockRequest>,
    ) -> Result<OwnerLockTarget> {
        if file.identity.0.is_empty() {
            return Err(stale("Owner lock requires opened file identity"));
        }
        self.validate_peer(access, peer, RootRight::Lookup)?;
        check_remote_file_scope(access, file)?;
        let handle = decode_file_handle(file)?;
        self.check_peer_file_handle(handle, peer, access, &file.identity)?;
        let target = self.lock_target(handle, request)?;
        if !matches!(target, OwnerLockTarget::Local { .. }) {
            return Err(stale("Owner lock authority is not Home"));
        }
        Ok(target)
    }

    fn peer_lock_scope(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        raw: &str,
    ) -> Result<String> {
        if raw.is_empty() || raw.len() > 256 {
            return Err(owner_lock_invalid());
        }
        let scope = owner_peer_lock_scope(peer, access, raw);
        let mut locks = self.locks.lock().map_err(|_| poisoned())?;
        locks.admit_scope(&scope)?;
        let count: usize = locks.peer_sessions.values().map(HashSet::len).sum();
        let tracked = locks
            .peer_sessions
            .get(&(peer.into(), access.session_id.clone()))
            .is_some_and(|sessions| sessions.contains(&scope));
        if count >= MAX_OWNER_LOCK_SCOPES && !tracked {
            return Err(owner_lock_capacity());
        }
        locks
            .peer_sessions
            .entry((peer.into(), access.session_id.clone()))
            .or_default()
            .insert(scope.clone());
        Ok(scope)
    }

    fn peer_get_file_lock(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        mut request: LockRequest,
    ) -> Result<Option<FileLockConflict>> {
        let target = self.peer_lock_target(peer, access, file, None)?;
        request.owner.ingress_session_id =
            self.peer_lock_scope(peer, access, &request.owner.ingress_session_id)?;
        let OwnerLockTarget::Local { table, .. } = target else {
            unreachable!()
        };
        table.getlk(&request).map_err(owner_lock_error)
    }

    fn peer_set_file_lock(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        mut request: LockRequest,
        mut waiter: Option<LockWaiterId>,
    ) -> Result<()> {
        let target = self.peer_lock_target(peer, access, file, Some(&request))?;
        let scope = self.peer_lock_scope(peer, access, &request.owner.ingress_session_id)?;
        if let Some(id) = &mut waiter {
            if id.ingress_session_id != request.owner.ingress_session_id {
                return Err(owner_lock_invalid());
            }
            id.ingress_session_id = scope.clone();
        }
        request.owner.ingress_session_id = scope.clone();
        let OwnerLockTarget::Local { table, .. } = &target else {
            unreachable!()
        };
        if let Some(id) = &waiter {
            self.locks.lock().map_err(|_| poisoned())?.register_waiter(
                id,
                target.clone(),
                request.owner.clone(),
                request.kind,
            )?;
        }
        let waiter_id = waiter.clone();
        let result = match waiter {
            Some(id) => table.setlk_blocking(request, id),
            None => table.setlk_nonblocking(request),
        };
        if let Some(id) = &waiter_id {
            let acknowledged = {
                let mut locks = self.locks.lock().map_err(|_| poisoned())?;
                if matches!(&result, Err(error) if *error != LockError::Interrupted) {
                    locks.waiters.remove(id);
                    false
                } else if let Some(route) = locks.waiters.get_mut(id) {
                    route.completed = true;
                    route.acknowledged
                } else {
                    false
                }
            };
            if acknowledged {
                table.acknowledge_waiter(id).map_err(owner_lock_error)?;
                self.locks
                    .lock()
                    .map_err(|_| poisoned())?
                    .waiters
                    .remove(id);
            }
        }
        result.map_err(owner_lock_error)?;
        // Always inspect the Home epoch as well as the peer admission after
        // waiting. Failed peer authority fences that exact peer/root ingress;
        // it cannot leave a granted lock live until the periodic reaper.
        let peer_validation = self.validate_peer(access, peer, RootRight::Lookup);
        let target_validation = self.validate_lock_target(&target);
        if let Err(error) = peer_validation {
            let _ = self.release_home_lock_scopes(vec![scope]);
            return Err(error);
        }
        target_validation
    }

    fn peer_cancel_file_lock(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        mut waiter: LockWaiterId,
    ) -> Result<LockWaiterOutcome> {
        self.validate_peer(access, peer, RootRight::Lookup)?;
        waiter.ingress_session_id =
            self.peer_lock_scope(peer, access, &waiter.ingress_session_id)?;
        let target = self
            .locks
            .lock()
            .map_err(|_| poisoned())?
            .cancel(waiter.clone())?;
        if let Some(OwnerLockTarget::Local { key, table }) = target {
            if key.root_id != access.id || key.epoch != access.epoch {
                return Err(stale(
                    "Owner waiter cancellation belongs to another root epoch",
                ));
            }
            return table
                .cancel_waiter_with_outcome(waiter)
                .map_err(owner_lock_error);
        }
        Ok(LockWaiterOutcome::Unknown)
    }

    fn peer_acknowledge_lock_wait(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        mut waiter: LockWaiterId,
    ) -> Result<()> {
        self.validate_peer(access, peer, RootRight::Lookup)?;
        waiter.ingress_session_id = owner_peer_lock_scope(peer, access, &waiter.ingress_session_id);
        let target = {
            let mut locks = self.locks.lock().map_err(|_| poisoned())?;
            match locks.waiters.get_mut(&waiter) {
                Some(route) => {
                    let OwnerLockTarget::Local { key, .. } = &route.target else {
                        return Err(stale("Owner lock ACK is not at Home"));
                    };
                    if key.root_id != access.id || key.epoch != access.epoch {
                        return Err(stale("Owner lock ACK belongs to another root epoch"));
                    }
                    // Publish acknowledgement and inspect completion under one
                    // lock, so either this handler or the completing worker
                    // retires the outcome even when both race.
                    route.acknowledged = true;
                    Some(route.clone())
                }
                None => None,
            }
        };
        if let Some(OwnerLockRoute {
            target: OwnerLockTarget::Local { table, .. },
            completed: true,
            ..
        }) = target
        {
            table
                .acknowledge_waiter(&waiter)
                .map_err(owner_lock_error)?;
            self.locks
                .lock()
                .map_err(|_| poisoned())?
                .waiters
                .remove(&waiter);
        }
        Ok(())
    }

    fn peer_release_file_locks(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        mut owner: FileLockOwner,
        kind: ReleaseKind,
    ) -> Result<()> {
        if file.identity.0.is_empty() {
            return Err(stale("Owner lock release requires file identity"));
        }
        self.validate_peer(access, peer, RootRight::Lookup)?;
        check_remote_file_scope(access, file)?;
        let handle = decode_file_handle(file)?;
        self.check_peer_file_handle(handle, peer, access, &file.identity)?;
        owner.ingress_session_id = owner_peer_lock_scope(peer, access, &owner.ingress_session_id);
        for target in self.existing_file_lock_targets(handle)? {
            if let OwnerLockTarget::Local { key, table } = &target {
                if key.epoch != access.epoch {
                    continue;
                }
                release_owner_locks(table, &owner, kind)?;
                self.forget_lock_routes(&target, &owner, kind)?;
            }
        }
        Ok(())
    }

    fn peer_release_file_lock_session(
        &self,
        peer: &str,
        access: &PresentedRootAccess,
        raw: &str,
    ) -> Result<()> {
        self.validate_peer(access, peer, RootRight::Lookup)?;
        let scope = owner_peer_lock_scope(peer, access, raw);
        self.release_home_lock_scopes(vec![scope])
    }

    fn release_home_lock_scopes(&self, scopes: Vec<String>) -> Result<()> {
        let tables = {
            let mut locks = self.locks.lock().map_err(|_| poisoned())?;
            for scope in &scopes {
                locks.close_scope(scope)?;
            }
            locks.tables.values().cloned().collect::<Vec<_>>()
        };
        let mut first = None;
        for table in tables {
            for scope in &scopes {
                if let Err(error) = table.release_session(scope).map_err(owner_lock_error) {
                    first.get_or_insert(error);
                }
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn reap_lock_authorities(&self) -> Result<()> {
        let targets = self
            .locks
            .lock()
            .map_err(|_| poisoned())?
            .tables
            .iter()
            .map(|(key, table)| OwnerLockTarget::Local {
                key: key.clone(),
                table: table.clone(),
            })
            .collect::<Vec<_>>();
        for target in targets {
            let _ = self.validate_lock_target(&target);
        }
        Ok(())
    }

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
            locks: Mutex::new(OwnerLockRegistry::default()),
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
        let scopes = self
            .locks
            .lock()
            .map_err(|_| poisoned())?
            .peer_sessions
            .remove(&(node_id.into(), session_id.into()))
            .unwrap_or_default();
        let lock_cleanup_error = self
            .release_home_lock_scopes(scopes.into_iter().collect())
            .err();
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
        if let Some(error) = lock_cleanup_error {
            return Err(error);
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

    fn namespace_parent_attributes(
        &self,
        data_dir: &StoragePath,
        parent: &StoragePath,
        expected: &files::FileIdentity,
    ) -> Result<FileAttributes> {
        self.check_peer_parent(data_dir, parent, expected)?;
        let physical = data_dir.join_path(parent).map_err(Error::from)?;
        let dir = self.disk.open_dir(&physical).map_err(Error::from)?;
        attributes_from_metadata(dir.metadata().map_err(Error::from)?)
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

    #[allow(clippy::too_many_arguments)]
    fn peer_create_with_options(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        mode: u32,
        expected_parent: &files::FileIdentity,
        options: OpenOptions,
    ) -> Result<remote::RemoteCreatedFile> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let (parent, name) = split_parent_name(path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        let parent_attributes =
            self.namespace_parent_attributes(root_use.data_dir(), &parent, expected_parent)?;
        let child = parent.join_component(&name).map_err(Error::from)?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        let create_flags = create_open_flags_for_backend(flags) | libc::O_CREAT;
        let created_new = path_missing_before_create(&self.disk, &physical)?;
        if created_new {
            authorize_create_in_directory(ctx, &parent_attributes)?;
        }
        let create_file = self
            .disk
            .open_file(&physical, OpenSpec::new(create_flags, mode))
            .map_err(Error::from)?;
        if created_new {
            apply_created_file_owner_and_mode(
                &create_file,
                ctx,
                created_gid(ctx, &parent_attributes),
                mode,
            )?;
        }
        let created_metadata = create_file.metadata().map_err(Error::from)?;
        let created_identity = identity_from_metadata(&created_metadata)?;
        drop(create_file);
        let file = self
            .disk
            .open_file(&physical, OpenSpec::new(flags & !libc::O_CREAT, 0))
            .map_err(Error::from)?;
        let reopened_identity = identity_from_metadata(&file.metadata().map_err(Error::from)?)?;
        if reopened_identity != created_identity {
            return Err(stale("created file was replaced before readonly reopen"));
        }
        let write_sync = write_sync_mode_from_flags(flags);
        let needs_flush = flags & libc::O_TRUNC != 0 && write_sync == WriteSyncMode::None;
        if flags & libc::O_TRUNC != 0 {
            write_sync.sync(&file)?;
        }
        if options.kill_suidgid {
            clear_suidgid_on_file(&file)?;
        }
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
        let local_handle = state.insert_file_handle(
            files::LocalOpenFile {
                root_id: access.id.clone(),
                identity: identity.clone(),
                file,
                peer: Some(files::PeerOpenScope {
                    node_id: peer_node_id.to_owned(),
                    access: access.clone(),
                }),
            },
            needs_flush,
            write_sync,
            flags,
        );
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
        ctx: &RequestContext,
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
        let parent_attributes =
            self.namespace_parent_attributes(root_use.data_dir(), &parent, expected_parent)?;
        let relative = storage_path_from_os(path)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        authorize_create_in_directory(ctx, &parent_attributes)?;
        let mode = mode | (parent_attributes.mode & libc::S_ISGID);
        self.disk.mkdir(&physical, mode).map_err(Error::from)?;
        apply_created_path_owner_with_gid_and_mode(
            &self.disk,
            &physical,
            ctx.uid,
            created_gid(ctx, &parent_attributes),
            mode,
        )?;
        let dir = self.disk.open_dir(&physical).map_err(Error::from)?;
        dir.sync_all().map_err(Error::from)?;
        self.owner_entry_for_path(&access.id, relative, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn peer_mknod(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        kind: SpecialFileKind,
        mode: u32,
        expected_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        reject_unprivileged_device_node(ctx, kind)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let (parent, _) = split_parent_name(path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        self.check_peer_parent(root_use.data_dir(), &parent, expected_parent)?;
        let parent_metadata = self
            .disk
            .metadata(
                &root_use
                    .data_dir()
                    .join_path(&parent)
                    .map_err(Error::from)?,
            )
            .map_err(Error::from)?;
        let parent_attributes = attributes_from_metadata(parent_metadata)?;
        authorize_create_in_directory(ctx, &parent_attributes)?;
        let relative = storage_path_from_os(path)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        mknod_at_storage_path(&self.disk, &physical, kind, mode).map_err(Error::from)?;
        apply_created_path_owner_with_gid_and_mode(
            &self.disk,
            &physical,
            ctx.uid,
            created_gid(ctx, &parent_attributes),
            mode,
        )?;
        self.owner_entry_for_path(&access.id, relative, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn peer_setattr_with_options(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
        file: Option<&files::RemoteFile>,
        change: &AttributeChange,
        options: SetAttrOptions,
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
            let attributes = self.setattr_with_options(
                ctx,
                backend_inode(OWNERFS_ROOT_INODE),
                Some(handle),
                change,
                options,
            )?;
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
        let attributes =
            self.setattr_with_options(ctx, backend_inode(inode), None, change, options)?;
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

    fn peer_xattr_target(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
        right: RootRight,
    ) -> Result<(StoragePath, FileAttributes)> {
        self.validate_peer(access, peer_node_id, right)?;
        let relative = storage_path_from_os(path)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let root_use = self.roots.enter_root(&access.id, right)?;
        let entry = self.owner_entry_for_path(&access.id, relative.clone(), Some(expected))?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        Ok((physical, entry.attributes))
    }

    fn peer_getxattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
        name: &OsStr,
    ) -> Result<Vec<u8>> {
        validate_user_xattr_name(name)?;
        let (physical, attributes) =
            self.peer_xattr_target(peer_node_id, access, path, expected, RootRight::Read)?;
        authorize_read(ctx, &attributes)?;
        self.disk.get_xattr(&physical, name).map_err(Error::from)
    }

    fn peer_listxattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
    ) -> Result<Vec<u8>> {
        let (physical, attributes) =
            self.peer_xattr_target(peer_node_id, access, path, expected, RootRight::Read)?;
        authorize_read(ctx, &attributes)?;
        let list = self.disk.list_xattr(&physical).map_err(Error::from)?;
        Ok(filter_user_xattr_list(list))
    }

    #[allow(clippy::too_many_arguments)]
    fn peer_setxattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
        name: &OsStr,
        value: &[u8],
        flags: i32,
    ) -> Result<()> {
        validate_user_xattr_name(name)?;
        let (physical, attributes) =
            self.peer_xattr_target(peer_node_id, access, path, expected, RootRight::Write)?;
        authorize_write(ctx, &attributes)?;
        self.disk
            .set_xattr(&physical, name, value, flags)
            .map_err(Error::from)
    }

    fn peer_removexattr(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: &files::FileIdentity,
        name: &OsStr,
    ) -> Result<()> {
        validate_user_xattr_name(name)?;
        let (physical, attributes) =
            self.peer_xattr_target(peer_node_id, access, path, expected, RootRight::Write)?;
        authorize_write(ctx, &attributes)?;
        self.disk.remove_xattr(&physical, name).map_err(Error::from)
    }

    fn peer_readlink(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        expected: Option<&files::FileIdentity>,
    ) -> Result<Vec<u8>> {
        self.validate_peer(access, peer_node_id, RootRight::Lookup)?;
        let relative = storage_path_from_os(path)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Lookup)?;
        self.owner_entry_for_path(&access.id, relative.clone(), expected)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        self.disk
            .read_link(&physical)
            .map(|target| target.into_vec())
            .map_err(Error::from)
    }

    fn peer_symlink(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        target: &OsStr,
        expected_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let (parent, _) = split_parent_name(path)?;
        let relative = storage_path_from_os(path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        self.check_peer_parent(root_use.data_dir(), &parent, expected_parent)?;
        let parent_entry = self.owner_entry_for_path(&access.id, parent, Some(expected_parent))?;
        authorize_create_in_directory(ctx, &parent_entry.attributes)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        self.disk.symlink(&physical, target).map_err(Error::from)?;
        apply_created_path_owner_with_gid(
            &self.disk,
            &physical,
            ctx.uid,
            created_gid(ctx, &parent_entry.attributes),
        )?;
        self.owner_entry_for_path(&access.id, relative, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn peer_link(
        &self,
        ctx: &RequestContext,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        old_path: &OsStr,
        new_path: &OsStr,
        expected_old: &files::FileIdentity,
        expected_new_parent: &files::FileIdentity,
    ) -> Result<files::OwnerEntry> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let old_relative = storage_path_from_os(old_path)?;
        let new_relative = storage_path_from_os(new_path)?;
        let (new_parent, _) = split_parent_name(new_path)?;
        let root_use = self.roots.enter_root(&access.id, RootRight::Write)?;
        self.check_peer_parent(root_use.data_dir(), &new_parent, expected_new_parent)?;
        let source_entry =
            self.owner_entry_for_path(&access.id, old_relative.clone(), Some(expected_old))?;
        let parent_entry =
            self.owner_entry_for_path(&access.id, new_parent.clone(), Some(expected_new_parent))?;
        authorize_read(ctx, &source_entry.attributes)?;
        authorize_create_in_directory(ctx, &parent_entry.attributes)?;
        let source = root_use
            .data_dir()
            .join_path(&old_relative)
            .map_err(Error::from)?;
        let target = root_use
            .data_dir()
            .join_path(&new_relative)
            .map_err(Error::from)?;
        self.disk.hard_link(&source, &target).map_err(Error::from)?;

        let target_metadata = self.disk.metadata(&target).map_err(Error::from)?;
        let target_kind = kind_from_metadata(&target_metadata)?;
        let target_identity = identity_from_metadata(&target_metadata)?;
        let target_attributes = attributes_from_metadata(target_metadata)?;
        let refreshed_source = self.disk.metadata(&source).map_err(Error::from)?;
        let refreshed_source_identity = identity_from_metadata(&refreshed_source)?;
        let refreshed_source_kind = kind_from_metadata(&refreshed_source)?;
        let refreshed_source_attributes = attributes_from_metadata(refreshed_source)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if let Some(source_inode) = state.paths.get(&(access.id.clone(), old_relative)).copied() {
            state.update_record(
                source_inode,
                refreshed_source_identity,
                refreshed_source_attributes,
                refreshed_source_kind,
            )?;
        }
        state.inode_for_path(
            access.id.clone(),
            new_relative,
            target_identity.clone(),
            target_attributes.clone(),
            target_kind,
        );
        Ok(files::OwnerEntry {
            root_id: access.id.clone(),
            identity: target_identity,
            attributes: target_attributes,
        })
    }

    fn peer_unlink(
        &self,
        ctx: &RequestContext,
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
        let parent_attributes =
            self.namespace_parent_attributes(root_use.data_dir(), &parent, expected_parent)?;
        let victim = self.owner_entry_for_path(&access.id, relative.clone(), expected)?;
        authorize_namespace_remove(ctx, &parent_attributes, &victim.attributes)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        self.disk.remove_file(&physical).map_err(Error::from)?;
        let removed = {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            state.remove_path(&access.id, relative)
        };
        if let Some((inode, Some(replacement))) = removed {
            self.refresh_cached_inode_at(&access.id, inode, root_use.data_dir(), &replacement)?;
        }
        Ok(())
    }

    fn peer_rmdir(
        &self,
        ctx: &RequestContext,
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
        let parent_attributes =
            self.namespace_parent_attributes(root_use.data_dir(), &parent, expected_parent)?;
        let victim = self.owner_entry_for_path(&access.id, relative.clone(), expected)?;
        authorize_namespace_remove(ctx, &parent_attributes, &victim.attributes)?;
        let physical = root_use
            .data_dir()
            .join_path(&relative)
            .map_err(Error::from)?;
        self.disk.remove_dir(&physical).map_err(Error::from)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.remove_path(&access.id, relative);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn peer_rename(
        &self,
        ctx: &RequestContext,
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
        let old_parent_attributes = self.namespace_parent_attributes(
            root_use.data_dir(),
            &old_parent,
            expected_old_parent,
        )?;
        let new_parent_attributes = self.namespace_parent_attributes(
            root_use.data_dir(),
            &new_parent,
            expected_new_parent,
        )?;
        let old_entry =
            self.owner_entry_for_path(&access.id, old_relative.clone(), expected_old)?;
        let physical_old = root_use
            .data_dir()
            .join_path(&old_relative)
            .map_err(Error::from)?;
        let physical_new = root_use
            .data_dir()
            .join_path(&new_relative)
            .map_err(Error::from)?;
        let new_entry = if expected_new.is_some() {
            Some(self.owner_entry_for_path(&access.id, new_relative.clone(), expected_new)?)
        } else {
            match self.disk.metadata(&physical_new) {
                Ok(metadata) => {
                    let identity = identity_from_metadata(&metadata)?;
                    let attributes = attributes_from_metadata(metadata)?;
                    Some(files::OwnerEntry {
                        root_id: access.id.clone(),
                        identity,
                        attributes,
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(Error::from(error)),
            }
        };
        authorize_namespace_remove(ctx, &old_parent_attributes, &old_entry.attributes)?;
        authorize_create_in_directory(ctx, &new_parent_attributes)?;
        if let Some(victim) = &new_entry {
            authorize_namespace_remove(ctx, &new_parent_attributes, &victim.attributes)?;
        }
        let same_hardlink = flags.0 == 0
            && new_entry
                .as_ref()
                .is_some_and(|entry| entry.identity == old_entry.identity);
        self.disk
            .rename(&physical_old, &physical_new, rename_mode)
            .map_err(Error::from)?;
        if same_hardlink {
            let entry = new_entry.ok_or_else(|| stale("rename hardlink target disappeared"))?;
            self.remember_alias_path(
                &access.id,
                new_relative,
                entry.identity,
                entry.attributes.clone(),
                entry.attributes.kind,
            )?;
        } else {
            self.state.lock().map_err(|_| poisoned())?.rename_path(
                access.id.clone(),
                old_relative,
                new_relative,
            );
        }
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

    fn peer_open_with_options(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        path: &OsStr,
        flags: i32,
        expected: Option<&files::FileIdentity>,
        options: OpenOptions,
    ) -> Result<(files::RemoteFile, FileAttributes, Option<Vec<u8>>)> {
        let right = if !options.kill_suidgid
            && flags & libc::O_ACCMODE == libc::O_RDONLY
            && flags & libc::O_TRUNC == 0
        {
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
        let write_sync = write_sync_mode_from_flags(flags);
        let mut needs_flush = flags & libc::O_TRUNC != 0;
        if flags & libc::O_TRUNC != 0 {
            file.set_len(0).map_err(Error::from)?;
            write_sync.sync(&file)?;
            if write_sync != WriteSyncMode::None {
                needs_flush = false;
            }
        }
        if options.kill_suidgid {
            clear_suidgid_on_file(&file)?;
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
        let handle = state.insert_file_handle(
            files::LocalOpenFile {
                root_id: access.id.clone(),
                identity: identity.clone(),
                file,
                peer: Some(files::PeerOpenScope {
                    node_id: peer_node_id.to_owned(),
                    access: access.clone(),
                }),
            },
            needs_flush,
            write_sync,
            flags,
        );
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

    fn peer_write_with_options(
        &self,
        peer_node_id: &str,
        access: &PresentedRootAccess,
        file: &files::RemoteFile,
        offset: u64,
        data: &[u8],
        options: WriteOptions,
    ) -> Result<usize> {
        self.validate_peer(access, peer_node_id, RootRight::Write)?;
        check_remote_file_scope(access, file)?;
        let handle = decode_file_handle(file)?;
        self.check_peer_file_handle(handle, peer_node_id, access, &file.identity)?;
        self.write_with_options(handle, offset, data, options)
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

    fn refresh_cached_inode_at(
        &self,
        root_id: &RootId,
        inode: u64,
        data_dir: &StoragePath,
        relative: &StoragePath,
    ) -> Result<()> {
        let physical = data_dir.join_path(relative).map_err(Error::from)?;
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let kind = kind_from_metadata(&metadata)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if let Some(record) = state.inodes.get(&inode)
            && &record.root_id != root_id
        {
            return Err(stale("cached inode root changed while refreshing hardlink"));
        }
        state.update_record(inode, identity, attributes, kind)
    }

    fn remember_alias_path(
        &self,
        root_id: &RootId,
        relative: StoragePath,
        identity: files::FileIdentity,
        attributes: FileAttributes,
        kind: FileKind,
    ) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.inode_for_path(root_id.clone(), relative, identity, attributes, kind);
        Ok(())
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

    fn open_inode_attributes(&self, record: &NodeRecord) -> Result<Option<FileAttributes>> {
        // Clone slots before locking a file: release/write may take file -> state locks.
        let slots: Vec<_> = self
            .state
            .lock()
            .map_err(|_| poisoned())?
            .file_handles
            .values()
            .cloned()
            .collect();
        for slot in slots {
            let slot = slot.lock().map_err(|_| poisoned())?;
            if slot.closed {
                continue;
            }
            let matches = match &slot.file {
                OpenFileHandle::Local(file) => {
                    file.handle.root_id == record.root_id && file.handle.identity == record.identity
                }
                OpenFileHandle::Remote(file) => {
                    file.handle.root_id == record.root_id && file.handle.identity == record.identity
                }
            };
            if matches {
                return slot.attributes().map(Some);
            }
        }
        Ok(None)
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
        let has_path = self
            .state
            .lock()
            .map_err(|_| poisoned())?
            .paths
            .get(&(record.root_id.clone(), record.relative.clone()))
            .is_some_and(|mapped| *mapped == inode.value);
        if !has_path && let Some(attributes) = self.open_inode_attributes(&record)? {
            return Ok(attributes);
        }
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

    fn setattr_with_options(
        &self,
        ctx: &RequestContext,
        inode: BackendInode,
        handle: Option<FileHandle>,
        change: &AttributeChange,
        options: SetAttrOptions,
    ) -> Result<FileAttributes> {
        if let Some(handle) = handle {
            let file = self.open_file_handle(handle)?;
            let mut file = file.lock().map_err(|_| poisoned())?;
            file.ensure_open()?;
            match &mut file.file {
                OpenFileHandle::Local(file) => {
                    let current = attributes_from_metadata(
                        file.handle.file.metadata().map_err(Error::from)?,
                    )?;
                    if change.size.is_some() && !file.writable {
                        return Err(bad_file_descriptor(
                            "ftruncate requires a writable open handle",
                        ));
                    }
                    // An open writable FD retains its write authority even after
                    // chmod(0). Other requested attribute changes still use caller permissions.
                    let mut non_size_change = change.clone();
                    non_size_change.size = None;
                    authorize_setattr_with_options(ctx, &current, &non_size_change, options)?;
                    apply_local_file_attr_change(&file.handle.file, change)?;
                    if options.kill_suidgid && apply_killpriv_to_change(change) {
                        clear_suidgid_on_file(&file.handle.file)?;
                    }
                    if change.size.is_some() {
                        file.needs_flush = true;
                        file.write_sync.sync(&file.handle.file)?;
                        if file.write_sync != WriteSyncMode::None {
                            file.needs_flush = false;
                        }
                    }
                    return file.attributes();
                }
                OpenFileHandle::Remote(file) => {
                    return Ok(file
                        .files
                        .setattr_with_options(
                            ctx,
                            &file.grant,
                            OsStr::new(""),
                            Some(&file.handle.identity),
                            Some(&file.handle),
                            change,
                            options,
                        )?
                        .attributes);
                }
            }
        }

        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            let current = self.owner_root_attributes()?;
            authorize_setattr_with_options(ctx, &current, change, options)?;
            return Err(Error::coded(
                afs_error::NODE_VFS_UNIMPLEMENTED,
                "OwnerFs root attributes are not mutable through setattr",
            ));
        }

        let record = self.record(inode.value)?.clone();
        let root_use = match self.roots.enter_root(&record.root_id, RootRight::Write) {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let remote = self.remote_for_record(&record, RootRight::Write)?;
                let entry = remote.files.setattr_with_options(
                    ctx,
                    &remote.grant,
                    record.relative.as_path().as_os_str(),
                    Some(&record.identity),
                    None,
                    change,
                    options,
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
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        check_expected_identity(Some(&record.identity), &identity)?;
        let current = attributes_from_metadata(metadata)?;
        authorize_setattr_with_options(ctx, &current, change, options)?;
        apply_local_path_attr_change(&self.disk, &physical, change)?;
        if options.kill_suidgid && apply_killpriv_to_change(change) {
            clear_suidgid_on_path(&self.disk, &physical)?;
        }
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.update_record(inode.value, identity, attributes.clone(), attributes.kind)?;
        Ok(attributes)
    }

    fn local_entry_for_xattr(
        &self,
        inode: BackendInode,
        right: RootRight,
    ) -> Result<(StoragePath, FileAttributes)> {
        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            return Err(Error::coded(
                afs_error::NODE_VFS_INVALID,
                "OwnerFs mount root has no user xattrs",
            ));
        }
        let record = self.record(inode.value)?.clone();
        let root_use = match self.roots.enter_root(&record.root_id, right) {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                return Err(Error::coded(
                    afs_error::NODE_VFS_UNIMPLEMENTED,
                    "OwnerFs remote xattr RPC is not wired yet",
                ));
            }
            Err(error) => return Err(error),
        };
        let physical = root_use
            .data_dir()
            .join_path(&record.relative)
            .map_err(Error::from)?;
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        check_expected_identity(Some(&record.identity), &identity)?;
        let attributes = attributes_from_metadata(metadata)?;
        Ok((physical, attributes))
    }

    fn remote_entry_for_xattr(
        &self,
        inode: BackendInode,
        right: RootRight,
    ) -> Result<Option<(NodeRecord, RemoteRoot)>> {
        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            return Err(Error::coded(
                afs_error::NODE_VFS_INVALID,
                "OwnerFs mount root has no user xattrs",
            ));
        }
        let record = self.record(inode.value)?.clone();
        match self.roots.enter_root(&record.root_id, right) {
            Ok(_) => Ok(None),
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let remote = self.remote_for_record(&record, right)?;
                Ok(Some((record, remote)))
            }
            Err(error) => Err(error),
        }
    }

    fn getxattr(&self, ctx: &RequestContext, inode: BackendInode, name: &OsStr) -> Result<Vec<u8>> {
        validate_user_xattr_name(name)?;
        if let Some((record, remote)) = self.remote_entry_for_xattr(inode, RootRight::Read)? {
            return remote.files.getxattr(
                ctx,
                &remote.grant,
                record.relative.as_path().as_os_str(),
                &record.identity,
                name,
            );
        }
        let (physical, attributes) = self.local_entry_for_xattr(inode, RootRight::Read)?;
        authorize_read(ctx, &attributes)?;
        self.disk.get_xattr(&physical, name).map_err(Error::from)
    }

    fn listxattr(&self, ctx: &RequestContext, inode: BackendInode) -> Result<Vec<u8>> {
        if let Some((record, remote)) = self.remote_entry_for_xattr(inode, RootRight::Read)? {
            return remote.files.listxattr(
                ctx,
                &remote.grant,
                record.relative.as_path().as_os_str(),
                &record.identity,
            );
        }
        let (physical, attributes) = self.local_entry_for_xattr(inode, RootRight::Read)?;
        authorize_read(ctx, &attributes)?;
        let list = self.disk.list_xattr(&physical).map_err(Error::from)?;
        Ok(filter_user_xattr_list(list))
    }

    fn setxattr(
        &self,
        ctx: &RequestContext,
        inode: BackendInode,
        name: &OsStr,
        value: &[u8],
        flags: i32,
    ) -> Result<()> {
        validate_user_xattr_name(name)?;
        if let Some((record, remote)) = self.remote_entry_for_xattr(inode, RootRight::Write)? {
            return remote.files.setxattr(
                ctx,
                &remote.grant,
                record.relative.as_path().as_os_str(),
                &record.identity,
                name,
                value,
                flags,
            );
        }
        let (physical, attributes) = self.local_entry_for_xattr(inode, RootRight::Write)?;
        authorize_write(ctx, &attributes)?;
        self.disk
            .set_xattr(&physical, name, value, flags)
            .map_err(Error::from)
    }

    fn removexattr(&self, ctx: &RequestContext, inode: BackendInode, name: &OsStr) -> Result<()> {
        validate_user_xattr_name(name)?;
        if let Some((record, remote)) = self.remote_entry_for_xattr(inode, RootRight::Write)? {
            return remote.files.removexattr(
                ctx,
                &remote.grant,
                record.relative.as_path().as_os_str(),
                &record.identity,
                name,
            );
        }
        let (physical, attributes) = self.local_entry_for_xattr(inode, RootRight::Write)?;
        authorize_write(ctx, &attributes)?;
        self.disk.remove_xattr(&physical, name).map_err(Error::from)
    }

    fn create_with_options(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
        flags: i32,
        options: OpenOptions,
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
                let created = remote.files.create_with_options(
                    ctx,
                    &remote.grant,
                    child.as_path().as_os_str(),
                    flags,
                    mode,
                    &parent_record.identity,
                    options,
                )?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                let inode = state.inode_for_path(
                    created.entry.root_id.clone(),
                    child,
                    created.entry.identity.clone(),
                    created.entry.attributes.clone(),
                    created.entry.attributes.kind,
                );
                let handle = state.insert_remote_file_handle(
                    remote.grant,
                    remote.files,
                    created.file,
                    true,
                    flags_allow_write(flags),
                );
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
        let parent_attributes = self.namespace_parent_attributes(
            root_use.data_dir(),
            &parent_record.relative,
            &parent_record.identity,
        )?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        let create_flags = create_open_flags_for_backend(flags) | libc::O_CREAT;
        let created_new = path_missing_before_create(&self.disk, &physical)?;
        if created_new {
            authorize_create_in_directory(ctx, &parent_attributes)?;
        }
        let create_file = self
            .disk
            .open_file(&physical, OpenSpec::new(create_flags, mode))
            .map_err(Error::from)?;
        if created_new {
            apply_created_file_owner_and_mode(
                &create_file,
                ctx,
                created_gid(ctx, &parent_attributes),
                mode,
            )?;
        }
        let created_metadata = create_file.metadata().map_err(Error::from)?;
        let created_identity = identity_from_metadata(&created_metadata)?;
        drop(create_file);
        let file = self
            .disk
            .open_file(&physical, OpenSpec::new(flags & !libc::O_CREAT, 0))
            .map_err(Error::from)?;
        let reopened_identity = identity_from_metadata(&file.metadata().map_err(Error::from)?)?;
        if reopened_identity != created_identity {
            return Err(stale("created file was replaced before readonly reopen"));
        }
        let write_sync = write_sync_mode_from_flags(flags);
        let needs_flush = flags & libc::O_TRUNC != 0 && write_sync == WriteSyncMode::None;
        if flags & libc::O_TRUNC != 0 {
            write_sync.sync(&file)?;
        }
        if options.kill_suidgid {
            clear_suidgid_on_file(&file)?;
        }
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
        let handle = state.insert_file_handle(
            files::LocalOpenFile {
                root_id: root_use.root_id().clone(),
                identity,
                file,
                peer: None,
            },
            needs_flush,
            write_sync,
            flags,
        );
        Ok(CreatedFile {
            entry: Entry {
                inode: backend_inode(inode),
                attributes,
            },
            handle,
        })
    }

    fn open_with_options(
        &self,
        inode: BackendInode,
        flags: i32,
        options: OpenOptions,
    ) -> Result<FileHandle> {
        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            return Err(Error::coded(
                afs_error::NODE_VFS_INVALID,
                "OwnerFs root is a directory",
            ));
        }
        let record = self.record(inode.value)?.clone();
        let right = if !options.kill_suidgid
            && flags & libc::O_ACCMODE == libc::O_RDONLY
            && flags & libc::O_TRUNC == 0
        {
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
                        let (file, _attributes) = remote.files.open_with_options(
                            &remote.grant,
                            record.relative.as_path().as_os_str(),
                            flags,
                            Some(&record.identity),
                            options,
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
                    flags_allow_write(flags),
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
            let write_sync = write_sync_mode_from_flags(flags);
            write_sync.sync(&file)?;
        }
        if options.kill_suidgid {
            clear_suidgid_on_file(&file)?;
        }
        if flags & libc::O_TRUNC != 0 || options.kill_suidgid {
            let metadata = file.metadata().map_err(Error::from)?;
            let attributes = attributes_from_metadata(metadata)?;
            self.state.lock().map_err(|_| poisoned())?.update_record(
                inode.value,
                identity.clone(),
                attributes.clone(),
                attributes.kind,
            )?;
        }
        let write_sync = write_sync_mode_from_flags(flags);
        let needs_flush = flags & libc::O_TRUNC != 0 && write_sync == WriteSyncMode::None;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Ok(state.insert_file_handle(
            files::LocalOpenFile {
                root_id: root_use.root_id().clone(),
                identity,
                file,
                peer: None,
            },
            needs_flush,
            write_sync,
            flags,
        ))
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

    fn write_with_options(
        &self,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
        options: WriteOptions,
    ) -> Result<usize> {
        let file = self.open_file_handle(handle)?;
        let mut file = file.lock().map_err(|_| poisoned())?;
        file.ensure_open()?;
        match &mut file.file {
            OpenFileHandle::Local(file) => {
                if !file.writable {
                    return Err(bad_file_descriptor("file handle is not open for writing"));
                }
                let written = file
                    .handle
                    .file
                    .write_at(offset, data)
                    .map_err(Error::from)?;
                if written > 0 {
                    if options.kill_suidgid {
                        clear_suidgid_on_file(&file.handle.file)?;
                    }
                    file.needs_flush = true;
                    file.write_sync.sync(&file.handle.file)?;
                    if file.write_sync != WriteSyncMode::None {
                        file.needs_flush = false;
                    }
                }
                Ok(written)
            }
            OpenFileHandle::Remote(file) => {
                if !file.writable {
                    return Err(bad_file_descriptor("file handle is not open for writing"));
                }
                let written = file.files.write_with_options(
                    &file.grant,
                    &file.handle,
                    offset,
                    data,
                    options,
                )?;
                if written > 0 {
                    file.needs_flush = true;
                }
                Ok(written)
            }
        }
    }

    fn flush(&self, handle: FileHandle) -> Result<()> {
        let file = self.open_file_handle(handle)?;
        let mut file = file.lock().map_err(|_| poisoned())?;
        file.ensure_open()?;
        match &mut file.file {
            OpenFileHandle::Local(file) if file.needs_flush => {
                file.handle.file.sync_data().map_err(Error::from)?;
                file.needs_flush = false;
                Ok(())
            }
            OpenFileHandle::Local(_) => Ok(()),
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
            .map_err(Error::from)
            .map(|()| file.needs_flush = false),
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

    fn mkdir(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
    ) -> Result<Entry> {
        self.check_inode_namespace(parent)?;
        if parent.value == OWNERFS_ROOT_INODE {
            let root_use = self.roots.create_root(name, mode)?;
            apply_created_path_owner_and_mode(&self.disk, root_use.data_dir(), ctx, mode)?;
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
                    ctx,
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
        let parent_attributes = self.namespace_parent_attributes(
            root_use.data_dir(),
            &parent_record.relative,
            &parent_record.identity,
        )?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        authorize_create_in_directory(ctx, &parent_attributes)?;
        let mode = mode | (parent_attributes.mode & libc::S_ISGID);
        self.disk.mkdir(&physical, mode).map_err(Error::from)?;
        apply_created_path_owner_with_gid_and_mode(
            &self.disk,
            &physical,
            ctx.uid,
            created_gid(ctx, &parent_attributes),
            mode,
        )?;
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

    fn mknod(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        kind: SpecialFileKind,
        mode: u32,
    ) -> Result<Entry> {
        self.check_inode_namespace(parent)?;
        if parent.value == OWNERFS_ROOT_INODE {
            return Err(Error::coded(
                afs_error::NODE_VFS_INVALID,
                "OwnerFs root accepts mkdir for workspace roots, not mknod",
            ));
        }
        reject_unprivileged_device_node(ctx, kind)?;
        let parent_record = self.directory_record(parent)?;
        authorize_create_in_directory(ctx, &parent_record.attributes)?;
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
                let entry = remote.files.mknod(
                    ctx,
                    &remote.grant,
                    child.as_path().as_os_str(),
                    kind,
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
        mknod_at_storage_path(&self.disk, &physical, kind, mode).map_err(Error::from)?;
        apply_created_path_owner_with_gid_and_mode(
            &self.disk,
            &physical,
            ctx.uid,
            created_gid(ctx, &parent_record.attributes),
            mode,
        )?;
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let inode = state.inode_for_path(
            parent_record.root_id,
            child,
            identity,
            attributes.clone(),
            attributes.kind,
        );
        Ok(Entry {
            inode: backend_inode(inode),
            attributes,
        })
    }

    fn unlink(&self, ctx: &RequestContext, parent: BackendInode, name: &OsStr) -> Result<()> {
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
                    ctx,
                    &remote.grant,
                    child.as_path().as_os_str(),
                    expected.as_ref(),
                    &parent_record.identity,
                )?;
                let removed = {
                    let mut state = self.state.lock().map_err(|_| poisoned())?;
                    state.remove_path(&parent_record.root_id, child)
                };
                if let Some((inode, Some(replacement))) = removed {
                    let expected = self.record(inode)?.identity;
                    let entry = remote.files.getattr(
                        &remote.grant,
                        replacement.as_path().as_os_str(),
                        Some(&expected),
                        None,
                    )?;
                    let mut state = self.state.lock().map_err(|_| poisoned())?;
                    state.update_record(
                        inode,
                        entry.identity,
                        entry.attributes.clone(),
                        entry.attributes.kind,
                    )?;
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let parent_attributes = self.namespace_parent_attributes(
            root_use.data_dir(),
            &parent_record.relative,
            &parent_record.identity,
        )?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        let victim = attributes_from_metadata(self.disk.metadata(&physical).map_err(Error::from)?)?;
        authorize_namespace_remove(ctx, &parent_attributes, &victim)?;
        self.disk.remove_file(&physical).map_err(Error::from)?;
        let removed = {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            state.remove_path(&parent_record.root_id, child)
        };
        if let Some((inode, Some(replacement))) = removed {
            self.refresh_cached_inode_at(
                &parent_record.root_id,
                inode,
                root_use.data_dir(),
                &replacement,
            )?;
        }
        Ok(())
    }

    fn rmdir(&self, ctx: &RequestContext, parent: BackendInode, name: &OsStr) -> Result<()> {
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
                    ctx,
                    &remote.grant,
                    child.as_path().as_os_str(),
                    expected.as_ref(),
                    &parent_record.identity,
                )?;
                let mut state = self.state.lock().map_err(|_| poisoned())?;
                state.remove_path(&parent_record.root_id, child);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let parent_attributes = self.namespace_parent_attributes(
            root_use.data_dir(),
            &parent_record.relative,
            &parent_record.identity,
        )?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        let victim = attributes_from_metadata(self.disk.metadata(&physical).map_err(Error::from)?)?;
        authorize_namespace_remove(ctx, &parent_attributes, &victim)?;
        self.disk.remove_dir(&physical).map_err(Error::from)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.remove_path(&parent_record.root_id, child);
        Ok(())
    }

    fn rename(
        &self,
        ctx: &RequestContext,
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
                    ctx,
                    &remote.grant,
                    from.as_path().as_os_str(),
                    to.as_path().as_os_str(),
                    expected_old.as_ref(),
                    expected_new.as_ref(),
                    &from_parent.identity,
                    &to_parent.identity,
                    flags,
                )?;
                let same_hardlink = flags.0 == 0
                    && if let Some(expected_old) = expected_old.as_ref() {
                        remote
                            .files
                            .lookup(
                                &remote.grant,
                                from.as_path().as_os_str(),
                                Some(expected_old),
                            )
                            .is_ok()
                    } else {
                        false
                    };
                if same_hardlink {
                    if let Some(expected_old) = expected_old {
                        let entry = remote.files.lookup(
                            &remote.grant,
                            to.as_path().as_os_str(),
                            Some(&expected_old),
                        )?;
                        let mut state = self.state.lock().map_err(|_| poisoned())?;
                        state.inode_for_path(
                            entry.root_id,
                            to,
                            entry.identity,
                            entry.attributes.clone(),
                            entry.attributes.kind,
                        );
                    }
                } else {
                    let mut state = self.state.lock().map_err(|_| poisoned())?;
                    state.rename_path(from_parent.root_id, from, to);
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let _namespace_guard = self.namespace_lock.lock().map_err(|_| poisoned())?;
        let old_parent_attributes = self.namespace_parent_attributes(
            root_use.data_dir(),
            &from_parent.relative,
            &from_parent.identity,
        )?;
        let new_parent_attributes = self.namespace_parent_attributes(
            root_use.data_dir(),
            &to_parent.relative,
            &to_parent.identity,
        )?;
        let physical_from = root_use.data_dir().join_path(&from).map_err(Error::from)?;
        let physical_to = root_use.data_dir().join_path(&to).map_err(Error::from)?;
        let from_metadata = self.disk.metadata(&physical_from).map_err(Error::from)?;
        let from_identity = identity_from_metadata(&from_metadata)?;
        let source_attributes = attributes_from_metadata(from_metadata)?;
        authorize_namespace_remove(ctx, &old_parent_attributes, &source_attributes)?;
        authorize_create_in_directory(ctx, &new_parent_attributes)?;
        let destination_metadata = match self.disk.metadata(&physical_to) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(Error::from(error)),
        };
        let same_hardlink = flags.0 == 0
            && destination_metadata.as_ref().is_some_and(|metadata| {
                identity_from_metadata(metadata).ok().as_ref() == Some(&from_identity)
            });
        if let Some(metadata) = destination_metadata {
            let victim = attributes_from_metadata(metadata)?;
            authorize_namespace_remove(ctx, &new_parent_attributes, &victim)?;
        }
        self.disk
            .rename(&physical_from, &physical_to, rename_mode)
            .map_err(Error::from)?;
        if same_hardlink {
            let metadata = self.disk.metadata(&physical_to).map_err(Error::from)?;
            let kind = kind_from_metadata(&metadata)?;
            let identity = identity_from_metadata(&metadata)?;
            let attributes = attributes_from_metadata(metadata)?;
            self.remember_alias_path(&from_parent.root_id, to, identity, attributes, kind)?;
        } else {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            state.rename_path(from_parent.root_id, from, to);
        }
        Ok(())
    }

    fn symlink(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        target: &OsStr,
    ) -> Result<Entry> {
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
                let entry = remote.files.symlink(
                    ctx,
                    &remote.grant,
                    child.as_path().as_os_str(),
                    target,
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
        let parent_metadata = self
            .disk
            .metadata(
                &root_use
                    .data_dir()
                    .join_path(&parent_record.relative)
                    .map_err(Error::from)?,
            )
            .map_err(Error::from)?;
        let parent_attributes = attributes_from_metadata(parent_metadata)?;
        authorize_create_in_directory(ctx, &parent_attributes)?;
        let physical = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        self.disk.symlink(&physical, target).map_err(Error::from)?;
        apply_created_path_owner_with_gid(
            &self.disk,
            &physical,
            ctx.uid,
            created_gid(ctx, &parent_attributes),
        )?;
        let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
        let kind = kind_from_metadata(&metadata)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let inode = state.inode_for_path(
            parent_record.root_id,
            child,
            identity,
            attributes.clone(),
            kind,
        );
        Ok(Entry {
            inode: backend_inode(inode),
            attributes,
        })
    }

    fn readlink(&self, inode: BackendInode) -> Result<OsString> {
        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            return Err(Error::from(std::io::Error::from(
                std::io::ErrorKind::InvalidInput,
            )));
        }
        let record = self.record(inode.value)?.clone();
        match self.roots.enter_root(&record.root_id, RootRight::Lookup) {
            Ok(root_use) => {
                let physical = root_use
                    .data_dir()
                    .join_path(&record.relative)
                    .map_err(Error::from)?;
                let metadata = self.disk.metadata(&physical).map_err(Error::from)?;
                let identity = identity_from_metadata(&metadata)?;
                check_expected_identity(Some(&record.identity), &identity)?;
                self.disk.read_link(&physical).map_err(Error::from)
            }
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                let remote = self.remote_for_record(&record, RootRight::Lookup)?;
                remote
                    .files
                    .readlink(
                        &remote.grant,
                        record.relative.as_path().as_os_str(),
                        Some(&record.identity),
                    )
                    .map(OsString::from_vec)
            }
            Err(error) => Err(error),
        }
    }

    fn link(
        &self,
        _ctx: &RequestContext,
        inode: BackendInode,
        new_parent: BackendInode,
        name: &OsStr,
    ) -> Result<Entry> {
        self.check_inode_namespace(inode)?;
        if inode.value == OWNERFS_ROOT_INODE {
            return Err(Error::from(std::io::Error::from(
                std::io::ErrorKind::InvalidInput,
            )));
        }
        let source_record = self.record(inode.value)?.clone();
        let parent_record = self.directory_record(new_parent)?;
        if source_record.root_id != parent_record.root_id {
            return Err(Error::from(std::io::Error::from_raw_os_error(libc::EXDEV)));
        }
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
                let entry = remote.files.link(
                    _ctx,
                    &remote.grant,
                    source_record.relative.as_path().as_os_str(),
                    child.as_path().as_os_str(),
                    &source_record.identity,
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
        let parent_metadata = self
            .disk
            .metadata(
                &root_use
                    .data_dir()
                    .join_path(&parent_record.relative)
                    .map_err(Error::from)?,
            )
            .map_err(Error::from)?;
        authorize_create_in_directory(_ctx, &attributes_from_metadata(parent_metadata)?)?;
        let source = root_use
            .data_dir()
            .join_path(&source_record.relative)
            .map_err(Error::from)?;
        let source_metadata = self.disk.metadata(&source).map_err(Error::from)?;
        let source_identity = identity_from_metadata(&source_metadata)?;
        check_expected_identity(Some(&source_record.identity), &source_identity)?;
        authorize_read(_ctx, &attributes_from_metadata(source_metadata)?)?;
        let target = root_use.data_dir().join_path(&child).map_err(Error::from)?;
        self.disk.hard_link(&source, &target).map_err(Error::from)?;

        let target_metadata = self.disk.metadata(&target).map_err(Error::from)?;
        let target_kind = kind_from_metadata(&target_metadata)?;
        let target_identity = identity_from_metadata(&target_metadata)?;
        let target_attributes = attributes_from_metadata(target_metadata)?;
        let refreshed_source = self.disk.metadata(&source).map_err(Error::from)?;
        let refreshed_source_identity = identity_from_metadata(&refreshed_source)?;
        let refreshed_source_kind = kind_from_metadata(&refreshed_source)?;
        let refreshed_source_attributes = attributes_from_metadata(refreshed_source)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        state.update_record(
            inode.value,
            refreshed_source_identity,
            refreshed_source_attributes,
            refreshed_source_kind,
        )?;
        let new_inode = state.inode_for_path(
            parent_record.root_id,
            child,
            target_identity,
            target_attributes.clone(),
            target_kind,
        );
        Ok(Entry {
            inode: backend_inode(new_inode),
            attributes: target_attributes,
        })
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
        let id = root::root_id_from_name(name)?;
        let root_use = match self.roots.enter_root(&id, RootRight::Lookup) {
            Ok(root_use) => root_use,
            Err(error) if error.code() == afs_error::NODE_OWNER_GRANT_UNAVAILABLE => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let metadata = self
            .disk
            .metadata(root_use.data_dir())
            .map_err(Error::from)?;
        let identity = identity_from_metadata(&metadata)?;
        let attributes = attributes_from_metadata(metadata)?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let inode = state.insert_root(name.to_os_string(), id, identity, attributes.clone());
        Ok(Some(Entry {
            inode: backend_inode(inode),
            attributes,
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
            blocks: 0,
            mode: 0o755,
            uid: 0,
            gid: 0,
            nlink: 2,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
        })
    }

    fn check_inode_namespace(&self, _inode: BackendInode) -> Result<()> {
        Ok(())
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
    identities: HashMap<(RootId, Vec<u8>), u64>,
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
            identities: HashMap::new(),
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
        if let Some(inode) = self.root_names.get(&name).copied()
            && self
                .inodes
                .get(&inode)
                .is_some_and(|record| record.root_id == root_id && record.identity == identity)
        {
            self.paths
                .insert((root_id.clone(), relative.clone()), inode);
            self.identities
                .insert((root_id.clone(), identity.0.clone()), inode);
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
        self.identities
            .insert((root_id.clone(), identity.0.clone()), inode);
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
        self.rebuild_identity_index();
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
            if let Some(record) = self.inodes.get_mut(&inode) {
                record.attributes = attributes;
                record.kind = kind;
            }
            self.rebuild_identity_index();
            return inode;
        }
        let identity_key = (root_id.clone(), identity.0.clone());
        if let Some(inode) = self.identities.get(&identity_key).copied()
            && self
                .inodes
                .get(&inode)
                .is_some_and(|record| record.root_id == root_id && record.identity == identity)
        {
            // Directory hardlinks are not supported: finding this directory's
            // identity at a different path means its cached subtree moved. Native
            // writes bypass rename_path, so reconcile every known descendant here
            // before retaining the inode for the newly observed name.
            let moved_from = self.inodes.get(&inode).and_then(|record| {
                (kind == FileKind::Directory
                    && record.kind == FileKind::Directory
                    && record.relative != relative)
                    .then(|| record.relative.clone())
            });
            if let Some(from) = moved_from {
                self.rename_path(root_id.clone(), from, relative.clone());
            }
            let had_active_path = self.active_path_for_inode(&root_id, inode).is_some();
            self.paths.insert(key, inode);
            if let Some(record) = self.inodes.get_mut(&inode) {
                record.attributes = attributes;
                record.kind = kind;
                if !had_active_path {
                    record.relative = relative;
                }
            }
            self.rebuild_identity_index();
            return inode;
        }
        let inode = self.allocate_inode();
        self.paths.insert(key, inode);
        self.inodes.insert(
            inode,
            NodeRecord {
                root_id: root_id.clone(),
                relative,
                identity: identity.clone(),
                attributes,
                kind,
            },
        );
        self.identities.insert((root_id, identity.0), inode);
        inode
    }

    fn remove_path(
        &mut self,
        root_id: &RootId,
        relative: StoragePath,
    ) -> Option<(u64, Option<StoragePath>)> {
        let inode = self.paths.remove(&(root_id.clone(), relative))?;
        let replacement = self.active_path_for_inode(root_id, inode);
        if let Some(replacement) = replacement.clone()
            && let Some(record) = self.inodes.get_mut(&inode)
        {
            record.relative = replacement.clone();
        }
        self.rebuild_identity_index();
        Some((inode, replacement))
    }

    fn active_path_for_inode(&self, root_id: &RootId, inode: u64) -> Option<StoragePath> {
        self.paths.iter().find_map(|((id, path), candidate)| {
            (id == root_id && *candidate == inode).then(|| path.clone())
        })
    }

    fn rebuild_identity_index(&mut self) {
        self.identities.clear();
        for ((root_id, _path), inode) in &self.paths {
            if let Some(record) = self.inodes.get(inode) {
                self.identities
                    .insert((root_id.clone(), record.identity.0.clone()), *inode);
            }
        }
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
        self.rebuild_identity_index();
        Ok(())
    }

    fn rename_path(&mut self, root_id: RootId, from: StoragePath, to: StoragePath) {
        let from_key = (root_id.clone(), from.clone());
        let to_key = (root_id.clone(), to.clone());
        if self.paths.contains_key(&from_key)
            && self.paths.get(&from_key) == self.paths.get(&to_key)
        {
            return;
        }
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
        // An overwritten destination's exact path may be one of several hardlink
        // aliases for the same inode. Drop only the overwritten path from path
        // lookup, then rebind that inode's canonical path to a surviving alias
        // so getattr by an already-known inode does not stat the replacement.
        let overwritten_inodes: Vec<_> = self
            .paths
            .iter()
            .filter_map(|((id, path), inode)| {
                (id == &root_id && path.as_path().strip_prefix(to.as_path()).is_ok())
                    .then_some(*inode)
            })
            .collect();
        self.paths.retain(|(id, path), _| {
            id != &root_id || path.as_path().strip_prefix(to.as_path()).is_err()
        });
        for inode in overwritten_inodes {
            if let Some(replacement) = self.active_path_for_inode(&root_id, inode)
                && let Some(record) = self.inodes.get_mut(&inode)
            {
                record.relative = replacement;
            }
        }
        for (_, new_path, inode) in moved {
            self.paths
                .insert((root_id.clone(), new_path.clone()), inode);
            if let Some(record) = self.inodes.get_mut(&inode) {
                record.relative = new_path;
            }
        }
        self.rebuild_identity_index();
    }

    fn insert_file_handle(
        &mut self,
        handle: files::LocalOpenFile,
        needs_flush: bool,
        write_sync: WriteSyncMode,
        flags: i32,
    ) -> FileHandle {
        self.insert_open_file(OpenFileHandle::Local(OpenLocalFile {
            handle,
            needs_flush,
            write_sync,
            writable: flags_allow_write(flags),
            readable: flags & libc::O_ACCMODE != libc::O_WRONLY,
        }))
    }

    fn insert_remote_file_handle(
        &mut self,
        grant: RootGrant,
        files: Arc<dyn remote::RemoteFiles>,
        handle: files::RemoteFile,
        needs_flush: bool,
        writable: bool,
    ) -> FileHandle {
        self.insert_open_file(OpenFileHandle::Remote(OpenRemoteFile {
            grant,
            files,
            handle,
            needs_flush,
            writable,
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
    readable: bool,
    handle: files::LocalOpenFile,
    needs_flush: bool,
    write_sync: WriteSyncMode,
    writable: bool,
}

struct OpenRemoteFile {
    grant: RootGrant,
    files: Arc<dyn remote::RemoteFiles>,
    handle: files::RemoteFile,
    needs_flush: bool,
    writable: bool,
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

fn create_open_flags_for_backend(flags: i32) -> i32 {
    if flags & libc::O_ACCMODE == libc::O_RDONLY {
        (flags & !libc::O_ACCMODE) | libc::O_WRONLY
    } else {
        flags
    }
}

fn flags_allow_write(flags: i32) -> bool {
    flags & libc::O_ACCMODE != libc::O_RDONLY || flags & libc::O_TRUNC != 0
}

fn bad_file_descriptor(message: &str) -> Error {
    Error::coded(afs_error::IO_BAD_FILE_DESCRIPTOR, message)
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

fn is_root(ctx: &RequestContext) -> bool {
    ctx.uid == 0
}

fn is_owner(ctx: &RequestContext, attributes: &FileAttributes) -> bool {
    ctx.uid == attributes.uid
}

fn is_group_member(ctx: &RequestContext, gid: u32) -> bool {
    ctx.gid == gid || ctx.supplementary_gids.contains(&gid)
}

fn permission_class_bits(ctx: &RequestContext, attributes: &FileAttributes) -> u32 {
    let mode = attributes.mode;
    if ctx.uid == attributes.uid {
        (mode >> 6) & 0o7
    } else if is_group_member(ctx, attributes.gid) {
        (mode >> 3) & 0o7
    } else {
        mode & 0o7
    }
}

fn kill_suidgid_mode(mode: u32) -> u32 {
    let mut mode = mode & !libc::S_ISUID;
    if mode & libc::S_IXGRP != 0 {
        mode &= !libc::S_ISGID;
    }
    mode
}

fn clear_suidgid_on_file(file: &impl crate::node::storage::FileHandle) -> Result<()> {
    let metadata = file.metadata().map_err(Error::from)?;
    let current = metadata.mode();
    let updated = kill_suidgid_mode(current);
    if updated != current {
        file.chmod(updated & 0o7777).map_err(Error::from)?;
    }
    Ok(())
}

fn clear_suidgid_on_path(disk: &LocalFs, path: &StoragePath) -> Result<()> {
    let metadata = disk.metadata(path).map_err(Error::from)?;
    let current = metadata.mode();
    let updated = kill_suidgid_mode(current);
    if updated != current {
        disk.chmod(path, updated & 0o7777).map_err(Error::from)?;
    }
    Ok(())
}

fn apply_killpriv_to_change(change: &AttributeChange) -> bool {
    change.size.is_some()
}

fn authorize_read(ctx: &RequestContext, attributes: &FileAttributes) -> Result<()> {
    if is_root(ctx) || permission_class_bits(ctx, attributes) & 0o4 != 0 {
        Ok(())
    } else {
        Err(permission_denied("OwnerFs read permission denied"))
    }
}

fn authorize_write(ctx: &RequestContext, attributes: &FileAttributes) -> Result<()> {
    if is_root(ctx) || permission_class_bits(ctx, attributes) & 0o2 != 0 {
        Ok(())
    } else {
        Err(permission_denied("OwnerFs write permission denied"))
    }
}

fn authorize_create_in_directory(ctx: &RequestContext, attributes: &FileAttributes) -> Result<()> {
    if attributes.kind != FileKind::Directory {
        return Err(Error::from(std::io::Error::from(
            std::io::ErrorKind::NotADirectory,
        )));
    }
    if is_root(ctx) || permission_class_bits(ctx, attributes) & 0o3 == 0o3 {
        Ok(())
    } else {
        Err(permission_denied(
            "OwnerFs namespace mutation requires directory write and search permission",
        ))
    }
}

fn authorize_namespace_remove(
    ctx: &RequestContext,
    parent: &FileAttributes,
    victim: &FileAttributes,
) -> Result<()> {
    authorize_create_in_directory(ctx, parent)?;
    if parent.mode & libc::S_ISVTX != 0
        && !is_root(ctx)
        && ctx.uid != parent.uid
        && ctx.uid != victim.uid
    {
        return Err(Error::from(std::io::Error::from_raw_os_error(libc::EPERM)));
    }
    Ok(())
}

fn reject_unprivileged_device_node(ctx: &RequestContext, kind: SpecialFileKind) -> Result<()> {
    if matches!(
        kind,
        SpecialFileKind::BlockDevice { .. } | SpecialFileKind::CharDevice { .. }
    ) && !is_root(ctx)
    {
        Err(permission_denied(
            "OwnerFs device node creation requires root caller",
        ))
    } else {
        Ok(())
    }
}

fn created_gid(ctx: &RequestContext, parent: &FileAttributes) -> u32 {
    if parent.mode & libc::S_ISGID != 0 {
        parent.gid
    } else {
        ctx.gid
    }
}

fn authorize_owner_or_root(
    ctx: &RequestContext,
    attributes: &FileAttributes,
    message: &'static str,
) -> Result<()> {
    if is_root(ctx) || is_owner(ctx, attributes) {
        Ok(())
    } else {
        Err(permission_denied(message))
    }
}

fn authorize_setattr_with_options(
    ctx: &RequestContext,
    attributes: &FileAttributes,
    change: &AttributeChange,
    options: SetAttrOptions,
) -> Result<()> {
    if let Some(uid) = change.uid
        && !is_root(ctx)
        && !(is_owner(ctx, attributes) && uid == attributes.uid)
    {
        return Err(permission_denied(
            "OwnerFs chown requires root caller or owner uid no-op",
        ));
    }
    if let Some(gid) = change.gid
        && !is_root(ctx)
        && !(is_owner(ctx, attributes) && is_group_member(ctx, gid))
    {
        return Err(permission_denied(
            "OwnerFs chgrp requires root caller or owner group membership",
        ));
    }
    if change.mode.is_some() {
        authorize_owner_or_root(ctx, attributes, "OwnerFs chmod requires file owner")?;
    }
    if change.atime.is_some() || change.mtime.is_some() {
        if options.timestamps_now {
            if !(is_root(ctx) || is_owner(ctx, attributes)) {
                authorize_write(ctx, attributes)?;
            }
        } else {
            authorize_owner_or_root(
                ctx,
                attributes,
                "OwnerFs timestamp setattr requires file owner",
            )?;
        }
    }
    if change.size.is_some() {
        authorize_write(ctx, attributes)?;
    }
    Ok(())
}

fn apply_local_file_attr_change(
    file: &impl crate::node::storage::FileHandle,
    change: &AttributeChange,
) -> Result<()> {
    if let Some(size) = change.size {
        file.set_len(size).map_err(Error::from)?;
    }
    if let Some(mode) = change.mode {
        file.chmod(mode & 0o7777).map_err(Error::from)?;
    }
    if change.uid.is_some() || change.gid.is_some() {
        file.chown(change.uid, change.gid).map_err(Error::from)?;
    }
    if change.atime.is_some() || change.mtime.is_some() {
        file.set_times(change.atime, change.mtime)
            .map_err(Error::from)?;
    }
    Ok(())
}

fn apply_local_path_attr_change(
    disk: &LocalFs,
    path: &StoragePath,
    change: &AttributeChange,
) -> Result<()> {
    if let Some(size) = change.size {
        let file = disk
            .open_file(path, OpenSpec::new(libc::O_WRONLY, 0))
            .map_err(Error::from)?;
        file.set_len(size).map_err(Error::from)?;
    }
    if let Some(mode) = change.mode {
        disk.chmod(path, mode & 0o7777).map_err(Error::from)?;
    }
    if change.uid.is_some() || change.gid.is_some() {
        disk.chown(path, change.uid, change.gid)
            .map_err(Error::from)?;
    }
    if change.atime.is_some() || change.mtime.is_some() {
        disk.set_times(path, change.atime, change.mtime)
            .map_err(Error::from)?;
    }
    Ok(())
}

fn path_missing_before_create(disk: &LocalFs, path: &StoragePath) -> Result<bool> {
    match disk.metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(Error::from(error)),
    }
}

fn apply_created_file_owner(
    file: &impl crate::node::storage::FileHandle,
    ctx: &RequestContext,
    gid: u32,
) -> Result<()> {
    let metadata = file.metadata().map_err(Error::from)?;
    if metadata.uid() == ctx.uid && metadata.gid() == gid {
        return Ok(());
    }
    file.chown(Some(ctx.uid), Some(gid)).map_err(Error::from)
}

fn apply_created_file_owner_and_mode(
    file: &impl crate::node::storage::FileHandle,
    ctx: &RequestContext,
    gid: u32,
    mode: u32,
) -> Result<()> {
    apply_created_file_owner(file, ctx, gid)?;
    file.chmod(mode & 0o7777).map_err(Error::from)
}

fn apply_created_path_owner(
    disk: &LocalFs,
    path: &StoragePath,
    ctx: &RequestContext,
) -> Result<()> {
    apply_created_path_owner_with_gid(disk, path, ctx.uid, ctx.gid)
}

fn apply_created_path_owner_with_gid(
    disk: &LocalFs,
    path: &StoragePath,
    uid: u32,
    gid: u32,
) -> Result<()> {
    let metadata = disk.metadata(path).map_err(Error::from)?;
    if metadata.uid() == uid && metadata.gid() == gid {
        return Ok(());
    }
    disk.chown(path, Some(uid), Some(gid)).map_err(Error::from)
}

fn apply_created_path_owner_and_mode(
    disk: &LocalFs,
    path: &StoragePath,
    ctx: &RequestContext,
    mode: u32,
) -> Result<()> {
    apply_created_path_owner(disk, path, ctx)?;
    disk.chmod(path, mode & 0o7777).map_err(Error::from)
}

fn apply_created_path_owner_with_gid_and_mode(
    disk: &LocalFs,
    path: &StoragePath,
    uid: u32,
    gid: u32,
    mode: u32,
) -> Result<()> {
    apply_created_path_owner_with_gid(disk, path, uid, gid)?;
    disk.chmod(path, mode & 0o7777).map_err(Error::from)
}

fn validate_user_xattr_name(name: &OsStr) -> Result<()> {
    let bytes = name.as_bytes();
    if bytes.starts_with(b"user.") && bytes.len() > b"user.".len() {
        Ok(())
    } else {
        Err(Error::coded(
            afs_error::IO_NOT_SUPPORTED,
            "OwnerFs currently supports only user.* xattrs",
        ))
    }
}

fn filter_user_xattr_list(list: Vec<u8>) -> Vec<u8> {
    let mut filtered = Vec::new();
    for name in list.split(|byte| *byte == 0) {
        if name.starts_with(b"user.") && !name.is_empty() {
            filtered.extend_from_slice(name);
            filtered.push(0);
        }
    }
    filtered
}

fn permission_denied(message: &'static str) -> Error {
    Error::coded(afs_error::IO_PERMISSION_DENIED, message)
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
    BackendInode { value }
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
        blocks: metadata.blocks(),
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
            FileKind::Special(kind) => special_kind_code(kind),
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
    } else if ty.is_fifo() {
        Ok(FileKind::Special(SpecialFileKind::Fifo))
    } else if ty.is_socket() {
        Ok(FileKind::Special(SpecialFileKind::Socket))
    } else if ty.is_block_device() {
        Ok(FileKind::Special(SpecialFileKind::BlockDevice {
            rdev: metadata.rdev(),
        }))
    } else if ty.is_char_device() {
        Ok(FileKind::Special(SpecialFileKind::CharDevice {
            rdev: metadata.rdev(),
        }))
    } else {
        Err(Error::coded(
            afs_error::NODE_VFS_INVALID,
            "unknown local file kind",
        ))
    }
}

fn special_kind_code(kind: SpecialFileKind) -> u8 {
    match kind {
        SpecialFileKind::Fifo => 4,
        SpecialFileKind::Socket => 5,
        SpecialFileKind::BlockDevice { .. } => 6,
        SpecialFileKind::CharDevice { .. } => 7,
    }
}

fn special_mode_and_rdev(kind: SpecialFileKind, mode: u32) -> (libc::mode_t, libc::dev_t) {
    let file_type = match kind {
        SpecialFileKind::Fifo => libc::S_IFIFO,
        SpecialFileKind::Socket => libc::S_IFSOCK,
        SpecialFileKind::BlockDevice { .. } => libc::S_IFBLK,
        SpecialFileKind::CharDevice { .. } => libc::S_IFCHR,
    };
    let rdev = match kind {
        SpecialFileKind::BlockDevice { rdev } | SpecialFileKind::CharDevice { rdev } => {
            rdev as libc::dev_t
        }
        SpecialFileKind::Fifo | SpecialFileKind::Socket => 0,
    };
    ((file_type | (mode & 0o7777)) as libc::mode_t, rdev)
}

#[allow(unsafe_code)]
fn mknod_at_storage_path(
    disk: &LocalFs,
    path: &StoragePath,
    kind: SpecialFileKind,
    mode: u32,
) -> io::Result<()> {
    let leaf = path.as_path().file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "mknod requires a non-root path",
        )
    })?;
    let parent = path.as_path().parent().unwrap_or_else(|| Path::new(""));
    let parent = open_storage_dir_no_follow(disk, parent)?;
    let leaf = CString::new(leaf.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path component contains NUL"))?;
    let (mode, rdev) = special_mode_and_rdev(kind, mode);
    // SAFETY: parent fd is live, leaf is a validated NUL-terminated single
    // component, and mknodat does not retain either pointer or fd.
    let result = unsafe { libc::mknodat(parent.as_raw_fd(), leaf.as_ptr(), mode, rdev) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[allow(unsafe_code)]
fn open_storage_dir_no_follow(disk: &LocalFs, path: &Path) -> io::Result<fs::File> {
    let mut dir = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(disk.root_path())?;
    if path.as_os_str().is_empty() {
        return Ok(dir);
    }
    for component in path.as_os_str().as_bytes().split(|byte| *byte == b'/') {
        if component.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid storage path component",
            ));
        }
        let component = CString::new(component).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "path component contains NUL")
        })?;
        // SAFETY: dir fd is live, component is a validated NUL-terminated
        // single component, and openat returns a new owned fd on success.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd was just returned by openat and ownership transfers to File.
        dir = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(dir)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriteSyncMode {
    None,
    DataOnly,
    Full,
}

impl WriteSyncMode {
    fn sync(self, file: &impl crate::node::storage::FileHandle) -> Result<()> {
        match self {
            Self::None => Ok(()),
            Self::DataOnly => file.sync_data().map_err(Error::from),
            Self::Full => file.sync_all().map_err(Error::from),
        }
    }
}

fn write_sync_mode_from_flags(flags: i32) -> WriteSyncMode {
    if flags & libc::O_SYNC == libc::O_SYNC {
        WriteSyncMode::Full
    } else if flags & libc::O_DSYNC != 0 {
        WriteSyncMode::DataOnly
    } else {
        WriteSyncMode::None
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

fn release_owner_locks(table: &LockTable, owner: &FileLockOwner, kind: ReleaseKind) -> Result<()> {
    match kind {
        ReleaseKind::PosixOwner => table.release_posix_owner(owner),
        ReleaseKind::FlockOwner => table.release_flock_owner(owner),
    }
    .map_err(owner_lock_error)
}

fn same_owner_lock_target(a: &OwnerLockTarget, b: &OwnerLockTarget) -> bool {
    match (a, b) {
        (OwnerLockTarget::Local { key: a, .. }, OwnerLockTarget::Local { key: b, .. }) => a == b,
        (
            OwnerLockTarget::Remote {
                grant: a, file: x, ..
            },
            OwnerLockTarget::Remote {
                grant: b, file: y, ..
            },
        ) => a == b && x.identity == y.identity,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::{env, process::Command};

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
            presented: &root::PresentedRootAccess,
            authenticated_peer_node_id: &str,
        ) -> Result<RootGrant> {
            let active = self.active.lock().unwrap();
            let location = active.get(&presented.id).ok_or_else(|| {
                Error::coded(afs_error::NODE_OWNER_GRANT_UNAVAILABLE, "test root")
            })?;
            if location.epoch != presented.epoch
                || location.home_node_id != presented.home_node_id
                || location.home_session_id != presented.home_session_id
            {
                return Err(Error::coded(
                    afs_error::NODE_OWNER_INVALID_GRANT,
                    "test root grant mismatch",
                ));
            }
            Ok(RootGrant {
                id: presented.id.clone(),
                epoch: presented.epoch,
                home_node_id: presented.home_node_id.clone(),
                home_session_id: presented.home_session_id.clone(),
                holder_node_id: authenticated_peer_node_id.to_owned(),
                session_id: presented.session_id.clone(),
                access_generation: presented.access_generation,
                rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write],
                fencing_token: presented.fencing_token.clone(),
            })
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

        fn supports_killpriv_v2(&self) -> bool {
            true
        }
    }

    struct RestartingRemoteFiles {
        root_id: RootId,
        restarted: Arc<AtomicBool>,
        lookup_calls: AtomicUsize,
        open_calls: AtomicUsize,
        last_open_killpriv: AtomicBool,
    }

    impl RestartingRemoteFiles {
        fn entry(&self, kind: FileKind) -> files::OwnerEntry {
            files::OwnerEntry {
                root_id: self.root_id.clone(),
                identity: files::FileIdentity(vec![match kind {
                    FileKind::Regular => 1,
                    FileKind::Directory => 2,
                    FileKind::Symlink => 3,
                    FileKind::Special(kind) => special_kind_code(kind),
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
        flush_calls: AtomicUsize,
        release_calls: AtomicUsize,
        lock_session_release_calls: AtomicUsize,
        fail_lock_session_release: AtomicBool,
        last_write_killpriv: AtomicBool,
        write_delay: Duration,
    }

    impl SlowRemoteFiles {
        fn new(root_id: RootId, write_delay: Duration) -> Self {
            Self {
                root_id,
                active_writes: AtomicUsize::new(0),
                max_active_writes: AtomicUsize::new(0),
                started_writes: AtomicUsize::new(0),
                flush_calls: AtomicUsize::new(0),
                release_calls: AtomicUsize::new(0),
                lock_session_release_calls: AtomicUsize::new(0),
                fail_lock_session_release: AtomicBool::new(false),
                last_write_killpriv: AtomicBool::new(false),
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
        fn release_lock_session(&self, _: &RootGrant, _: &str) -> Result<()> {
            self.lock_session_release_calls
                .fetch_add(1, Ordering::SeqCst);
            if self.fail_lock_session_release.swap(false, Ordering::SeqCst) {
                return Err(Error::coded(
                    afs_error::IO_UNAVAILABLE,
                    "injected lock cleanup failure",
                ));
            }
            Ok(())
        }

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
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: Option<&files::RemoteFile>,
            _: &AttributeChange,
        ) -> Result<files::OwnerEntry> {
            self.unsupported()
        }

        fn getxattr(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &files::FileIdentity,
            _: &OsStr,
        ) -> Result<Vec<u8>> {
            self.unsupported()
        }

        fn listxattr(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &files::FileIdentity,
        ) -> Result<Vec<u8>> {
            self.unsupported()
        }

        fn setxattr(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &files::FileIdentity,
            _: &OsStr,
            _: &[u8],
            _: i32,
        ) -> Result<()> {
            self.unsupported()
        }

        fn removexattr(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &files::FileIdentity,
            _: &OsStr,
        ) -> Result<()> {
            self.unsupported()
        }

        fn create(
            &self,
            _: &RequestContext,
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
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: u32,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
            self.unsupported()
        }

        fn mknod(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: SpecialFileKind,
            _: u32,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
            self.unsupported()
        }

        fn unlink(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
        ) -> Result<()> {
            self.unsupported()
        }

        fn rmdir(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
        ) -> Result<()> {
            self.unsupported()
        }

        fn rename(
            &self,
            _: &RequestContext,
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

        fn symlink(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &OsStr,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
            self.unsupported()
        }

        fn link(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &OsStr,
            _: &files::FileIdentity,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
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
            grant: &RootGrant,
            file: &files::RemoteFile,
            offset: u64,
            data: &[u8],
        ) -> Result<usize> {
            self.write_with_options(grant, file, offset, data, WriteOptions::default())
        }

        fn write_with_options(
            &self,
            _: &RootGrant,
            _: &files::RemoteFile,
            _: u64,
            data: &[u8],
            options: WriteOptions,
        ) -> Result<usize> {
            self.last_write_killpriv
                .store(options.kill_suidgid, Ordering::SeqCst);
            self.observe_active_write();
            std::thread::sleep(self.write_delay);
            self.active_writes.fetch_sub(1, Ordering::SeqCst);
            Ok(data.len())
        }

        fn flush(&self, _: &RootGrant, _: &files::RemoteFile) -> Result<()> {
            self.flush_calls.fetch_add(1, Ordering::SeqCst);
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
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: Option<&files::RemoteFile>,
            _: &AttributeChange,
        ) -> Result<files::OwnerEntry> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn getxattr(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &files::FileIdentity,
            _: &OsStr,
        ) -> Result<Vec<u8>> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn listxattr(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &files::FileIdentity,
        ) -> Result<Vec<u8>> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn setxattr(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &files::FileIdentity,
            _: &OsStr,
            _: &[u8],
            _: i32,
        ) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn removexattr(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &files::FileIdentity,
            _: &OsStr,
        ) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn create(
            &self,
            _: &RequestContext,
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
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: u32,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn mknod(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: SpecialFileKind,
            _: u32,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn unlink(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
        ) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn rmdir(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: Option<&files::FileIdentity>,
            _: &files::FileIdentity,
        ) -> Result<()> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn rename(
            &self,
            _: &RequestContext,
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
            flags: i32,
            expected_identity: Option<&files::FileIdentity>,
        ) -> Result<(files::RemoteFile, FileAttributes)> {
            self.open_with_options(
                grant,
                path,
                flags,
                expected_identity,
                OpenOptions::default(),
            )
        }

        fn open_with_options(
            &self,
            grant: &RootGrant,
            path: &OsStr,
            _: i32,
            expected_identity: Option<&files::FileIdentity>,
            options: OpenOptions,
        ) -> Result<(files::RemoteFile, FileAttributes)> {
            self.last_open_killpriv
                .store(options.kill_suidgid, Ordering::SeqCst);
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

        fn symlink(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &OsStr,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
            Err(Error::coded(afs_error::NODE_VFS_UNIMPLEMENTED, "test"))
        }

        fn link(
            &self,
            _: &RequestContext,
            _: &RootGrant,
            _: &OsStr,
            _: &OsStr,
            _: &files::FileIdentity,
            _: &files::FileIdentity,
        ) -> Result<files::OwnerEntry> {
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
            blocks: 0,
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
            last_open_killpriv: AtomicBool::new(false),
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
        let ctx = test_context_for_path(temp.path());
        (temp, fs, ctx, meta, remote, root_id)
    }

    fn test_context_for_path(path: &std::path::Path) -> RequestContext {
        let metadata = fs::metadata(path).unwrap();
        RequestContext {
            uid: metadata.uid(),
            gid: metadata.gid(),
            pid: 42,
            umask: 0,
            supplementary_gids: Vec::new(),
        }
    }

    fn context_for_attrs(base: RequestContext, attributes: &FileAttributes) -> RequestContext {
        RequestContext {
            uid: attributes.uid,
            gid: attributes.gid,
            ..base
        }
    }

    fn non_owner_context(owner: RequestContext) -> RequestContext {
        let uid = if owner.uid == 0 {
            1000
        } else {
            owner.uid.saturating_add(1)
        };
        let gid = if owner.gid == 0 {
            1000
        } else {
            owner.gid.saturating_add(1)
        };
        RequestContext { uid, gid, ..owner }
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
        let ctx = test_context_for_path(temp.path());
        (temp, OwnerFs::new_local(roots, disk), ctx)
    }

    fn test_owner_lock(scope: &str, kernel_owner: u64, lock_type: FileLockType) -> LockRequest {
        LockRequest {
            kind: FileLockKind::Posix,
            owner: FileLockOwner {
                ingress_session_id: scope.into(),
                kernel_owner,
            },
            pid: kernel_owner as u32,
            range: super::super::types::FileLockRange { start: 0, end: 99 },
            lock_type,
        }
    }

    #[test]
    fn owner_closed_scope_capacity_preserves_live_locks_and_cleanup() {
        let mut registry = OwnerLockRegistry::default();
        registry.admit_scope("live").unwrap();
        let table = registry
            .table(OwnerLockKey {
                root_id: RootId("root".into()),
                epoch: 1,
                identity: vec![1],
            })
            .unwrap();
        let held = test_owner_lock("live", 1, FileLockType::Write);
        table.setlk_nonblocking(held.clone()).unwrap();
        registry.close_scope_with_limit("closed-a", 1).unwrap();
        registry.close_scope_with_limit("closed-b", 1).unwrap();
        assert!(registry.admission_exhausted);
        assert!(registry.admit_scope("new").is_err());
        assert!(registry.admit_scope("closed-b").is_err());
        registry.admit_scope("live").unwrap();
        assert!(
            table
                .getlk(&test_owner_lock("other", 2, FileLockType::Write))
                .unwrap()
                .is_some()
        );
        release_owner_locks(&table, &held.owner, ReleaseKind::PosixOwner).unwrap();
        assert!(
            table
                .getlk(&test_owner_lock("other", 2, FileLockType::Write))
                .unwrap()
                .is_none()
        );
        registry.close_scope_with_limit("live", 1).unwrap();
        table.release_session("live").unwrap();
        assert!(registry.admit_scope("live").is_err());
    }

    #[test]
    fn owner_remote_session_cleanup_retries_only_unacknowledged_targets() {
        let (_temp, fs, _ctx) = fixture();
        let local = fs.require_local().unwrap();
        let failed = Arc::new(SlowRemoteFiles::new(
            RootId("remote-a".into()),
            Duration::ZERO,
        ));
        failed
            .fail_lock_session_release
            .store(true, Ordering::SeqCst);
        let success = Arc::new(SlowRemoteFiles::new(
            RootId("remote-b".into()),
            Duration::ZERO,
        ));
        let targets = [(failed.clone(), "remote-a"), (success.clone(), "remote-b")]
            .into_iter()
            .map(|(files, id)| {
                let grant = test_grant(RootId(id.into()));
                let file = remote_file(
                    &presented(&grant),
                    files::FileIdentity(vec![1]),
                    FileHandle(17),
                );
                OwnerLockTarget::Remote {
                    grant: Arc::new(grant),
                    files,
                    file,
                }
            })
            .collect();
        local
            .locks
            .lock()
            .unwrap()
            .remote_targets
            .insert("mount".into(), targets);
        assert!(local.release_file_lock_session("mount").is_err());
        {
            let locks = local.locks.lock().unwrap();
            assert!(locks.pending_remote_cleanup.contains("mount"));
            assert_eq!(locks.remote_targets["mount"].len(), 1);
        }
        assert_eq!(failed.lock_session_release_calls.load(Ordering::SeqCst), 1);
        assert_eq!(success.lock_session_release_calls.load(Ordering::SeqCst), 1);
        fs.reap_expired_peer_sessions().unwrap();
        let locks = local.locks.lock().unwrap();
        assert!(!locks.pending_remote_cleanup.contains("mount"));
        assert!(!locks.remote_targets.contains_key("mount"));
        assert_eq!(failed.lock_session_release_calls.load(Ordering::SeqCst), 2);
        assert_eq!(success.lock_session_release_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn owner_cancelled_waits_do_not_leak_routed_slots() {
        let (_temp, fs, ctx) = fixture();
        let fs = Arc::new(fs);
        let dir = fs
            .mkdir(
                &ctx,
                backend_inode(1),
                OsStr::new("lock-repeat-cancel"),
                0o755,
            )
            .unwrap();
        let file = fs
            .create(&ctx, dir.inode, OsStr::new("file"), 0o600, libc::O_RDWR)
            .unwrap();
        fs.setlk(
            &ctx,
            file.entry.inode,
            file.handle,
            test_owner_lock("holder", 1, FileLockType::Write),
            None,
        )
        .unwrap();
        for request_id in 1..=32 {
            let id = LockWaiterId {
                ingress_session_id: "waiter".into(),
                request_id,
            };
            let fs2 = fs.clone();
            let ctx2 = ctx.clone();
            let id2 = id.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let waiting = std::thread::spawn(move || {
                tx.send(fs2.setlk(
                    &ctx2,
                    file.entry.inode,
                    file.handle,
                    test_owner_lock("waiter", 2, FileLockType::Write),
                    Some(id2),
                ))
                .unwrap();
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !fs
                .require_local()
                .unwrap()
                .locks
                .lock()
                .unwrap()
                .waiters
                .contains_key(&id)
            {
                assert!(
                    std::time::Instant::now() < deadline,
                    "waiter was not routed"
                );
                std::thread::yield_now();
            }
            fs.cancel_lock_wait(id).unwrap();
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .unwrap_err()
                    .code(),
                Error::from(io::Error::from_raw_os_error(libc::EINTR)).code()
            );
            waiting.join().unwrap();
            assert!(
                fs.require_local()
                    .unwrap()
                    .locks
                    .lock()
                    .unwrap()
                    .waiters
                    .is_empty()
            );
        }
        fs.release_lock_session("waiter").unwrap();
        fs.release_lock_session("holder").unwrap();
        fs.release(&ctx, file.handle).unwrap();
    }

    #[test]
    fn owner_peer_waiter_scope_binds_root_epoch_and_process() {
        let a = presented(&test_grant(RootId("root-a".into())));
        let mut b = a.clone();
        b.id = RootId("root-b".into());
        let mut later = a.clone();
        later.epoch += 1;
        let mut process = a.clone();
        process.session_id = "session-b-next".into();
        let scope = owner_peer_lock_scope("node-b", &a, "mount");
        assert_ne!(scope, owner_peer_lock_scope("node-b", &b, "mount"));
        assert_ne!(scope, owner_peer_lock_scope("node-b", &later, "mount"));
        assert_ne!(scope, owner_peer_lock_scope("node-b", &process, "mount"));
        assert_ne!(scope, owner_peer_lock_scope("node-c", &a, "mount"));
        let mut registry = OwnerLockRegistry::default();
        registry
            .cancel(LockWaiterId {
                ingress_session_id: owner_peer_lock_scope("node-b", &b, "mount"),
                request_id: 1,
            })
            .unwrap();
        assert!(!registry.cancelled.contains(&LockWaiterId {
            ingress_session_id: scope,
            request_id: 1
        }));
    }

    #[test]
    fn owner_locks_share_inode_identity_across_handles_and_unlink() {
        let (_temp, fs, ctx) = fixture();
        let dir = fs
            .mkdir(&ctx, backend_inode(1), OsStr::new("lock-identity"), 0o755)
            .unwrap();
        let file = fs
            .create(&ctx, dir.inode, OsStr::new("file"), 0o600, libc::O_RDWR)
            .unwrap();
        let other = fs.open(&ctx, file.entry.inode, libc::O_RDWR).unwrap();
        fs.setlk(
            &ctx,
            file.entry.inode,
            file.handle,
            test_owner_lock("mount-a", 1, FileLockType::Write),
            None,
        )
        .unwrap();
        assert_eq!(
            fs.setlk(
                &ctx,
                file.entry.inode,
                other,
                test_owner_lock("mount-b", 2, FileLockType::Write),
                None
            )
            .unwrap_err()
            .code(),
            Error::from(io::Error::from_raw_os_error(libc::EAGAIN)).code()
        );
        let conflict = fs
            .getlk(
                &ctx,
                file.entry.inode,
                other,
                test_owner_lock("mount-b", 2, FileLockType::Write),
            )
            .unwrap()
            .unwrap();
        assert_eq!(conflict.pid, 1);
        fs.unlink(&ctx, dir.inode, OsStr::new("file")).unwrap();
        assert!(
            fs.getlk(
                &ctx,
                file.entry.inode,
                other,
                test_owner_lock("mount-b", 2, FileLockType::Write)
            )
            .unwrap()
            .is_some()
        );
        fs.release_locks(
            &ctx,
            file.entry.inode,
            file.handle,
            test_owner_lock("mount-a", 1, FileLockType::Write).owner,
            ReleaseKind::PosixOwner,
        )
        .unwrap();
        fs.setlk(
            &ctx,
            file.entry.inode,
            other,
            test_owner_lock("mount-b", 2, FileLockType::Write),
            None,
        )
        .unwrap();
        fs.release_lock_session("mount-b").unwrap();
        assert!(
            fs.getlk(
                &ctx,
                file.entry.inode,
                other,
                test_owner_lock("mount-a", 1, FileLockType::Write)
            )
            .unwrap()
            .is_none()
        );
        fs.release(&ctx, file.handle).unwrap();
        fs.release(&ctx, other).unwrap();
    }

    #[test]
    fn owner_posix_fd_access_and_flock_conflict_namespaces_are_distinct() {
        let (_temp, fs, ctx) = fixture();
        let dir = fs
            .mkdir(&ctx, backend_inode(1), OsStr::new("lock-fd-mode"), 0o755)
            .unwrap();
        let file = fs
            .create(&ctx, dir.inode, OsStr::new("file"), 0o600, libc::O_RDWR)
            .unwrap();
        let readonly = fs.open(&ctx, file.entry.inode, libc::O_RDONLY).unwrap();
        let writeonly = fs.open(&ctx, file.entry.inode, libc::O_WRONLY).unwrap();
        let badfd = Error::from(io::Error::from_raw_os_error(libc::EBADF)).code();
        assert_eq!(
            fs.setlk(
                &ctx,
                file.entry.inode,
                readonly,
                test_owner_lock("m", 1, FileLockType::Write),
                None
            )
            .unwrap_err()
            .code(),
            badfd
        );
        assert_eq!(
            fs.setlk(
                &ctx,
                file.entry.inode,
                writeonly,
                test_owner_lock("m", 1, FileLockType::Read),
                None
            )
            .unwrap_err()
            .code(),
            badfd
        );
        fs.setlk(
            &ctx,
            file.entry.inode,
            file.handle,
            test_owner_lock("m", 1, FileLockType::Write),
            None,
        )
        .unwrap();
        let mut flock = test_owner_lock("m", 2, FileLockType::Write);
        flock.kind = FileLockKind::Flock;
        fs.setlk(&ctx, file.entry.inode, readonly, flock.clone(), None)
            .unwrap();
        flock.owner.kernel_owner = 3;
        assert!(
            fs.setlk(&ctx, file.entry.inode, writeonly, flock, None)
                .is_err()
        );
        fs.release_lock_session("m").unwrap();
        for handle in [file.handle, readonly, writeonly] {
            fs.release(&ctx, handle).unwrap();
        }
    }

    #[test]
    fn owner_lock_interrupt_before_registration_and_closed_session_do_not_revive() {
        let (_temp, fs, ctx) = fixture();
        let dir = fs
            .mkdir(&ctx, backend_inode(1), OsStr::new("lock-cancel"), 0o755)
            .unwrap();
        let file = fs
            .create(&ctx, dir.inode, OsStr::new("file"), 0o600, libc::O_RDWR)
            .unwrap();
        let waiter = LockWaiterId {
            ingress_session_id: "m".into(),
            request_id: 77,
        };
        fs.cancel_lock_wait(waiter.clone()).unwrap();
        let interrupted = Error::from(io::Error::from_raw_os_error(libc::EINTR)).code();
        assert_eq!(
            fs.setlk(
                &ctx,
                file.entry.inode,
                file.handle,
                test_owner_lock("m", 1, FileLockType::Write),
                Some(waiter.clone())
            )
            .unwrap_err()
            .code(),
            interrupted
        );
        // The same pre-cancelled identity remains fenced on every replay.
        assert_eq!(
            fs.setlk(
                &ctx,
                file.entry.inode,
                file.handle,
                test_owner_lock("m", 1, FileLockType::Write),
                Some(waiter)
            )
            .unwrap_err()
            .code(),
            interrupted
        );
        fs.release_lock_session("m").unwrap();
        assert_eq!(
            fs.setlk(
                &ctx,
                file.entry.inode,
                file.handle,
                test_owner_lock("m", 1, FileLockType::Write),
                None
            )
            .unwrap_err()
            .code(),
            interrupted
        );
        fs.release(&ctx, file.handle).unwrap();
    }

    #[test]
    fn owner_blocked_lock_wakes_on_unlock_and_authority_invalidation() {
        let (_temp, fs, ctx) = fixture();
        let fs = Arc::new(fs);
        let dir = fs
            .mkdir(&ctx, backend_inode(1), OsStr::new("lock-blocking"), 0o755)
            .unwrap();
        let file = fs
            .create(&ctx, dir.inode, OsStr::new("file"), 0o600, libc::O_RDWR)
            .unwrap();
        fs.setlk(
            &ctx,
            file.entry.inode,
            file.handle,
            test_owner_lock("m", 1, FileLockType::Write),
            None,
        )
        .unwrap();
        let fs2 = fs.clone();
        let ctx2 = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            tx.send(fs2.setlk(
                &ctx2,
                file.entry.inode,
                file.handle,
                test_owner_lock("m", 2, FileLockType::Write),
                Some(LockWaiterId {
                    ingress_session_id: "m".into(),
                    request_id: 88,
                }),
            ))
            .unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        fs.setlk(
            &ctx,
            file.entry.inode,
            file.handle,
            test_owner_lock("m", 1, FileLockType::Unlock),
            None,
        )
        .unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        thread.join().unwrap();
        let local = fs.require_local().unwrap();
        let table = local
            .locks
            .lock()
            .unwrap()
            .tables
            .values()
            .next()
            .unwrap()
            .clone();
        table.invalidate().unwrap();
        assert!(
            fs.setlk(
                &ctx,
                file.entry.inode,
                file.handle,
                test_owner_lock("m", 3, FileLockType::Write),
                None
            )
            .is_err()
        );
        fs.release(&ctx, file.handle).unwrap();
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
    fn local_readonly_create_does_not_require_created_file_write_access() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-readonly-create"), 0o755)
            .unwrap();

        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("created.txt"),
                0o444,
                libc::O_RDONLY | libc::O_EXCL,
            )
            .unwrap();

        assert_eq!(created.entry.attributes.size, 0);
        assert_eq!(created.entry.attributes.mode & 0o777, 0o444);
        let mut empty = [0_u8; 1];
        assert_eq!(fs.read(&ctx, created.handle, 0, &mut empty).unwrap(), 0);
        let error = fs.write(&ctx, created.handle, 0, b"x").unwrap_err();
        assert_eq!(error.code(), afs_error::IO_BAD_FILE_DESCRIPTOR);
        fs.release(&ctx, created.handle).unwrap();
    }

    #[test]
    fn local_mknod_fifo_records_special_metadata_and_permissions() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-local-mknod"), 0o755)
            .unwrap();

        let fifo = fs
            .mknod(
                &ctx,
                workspace.inode,
                OsStr::new("events.fifo"),
                SpecialFileKind::Fifo,
                0o640,
            )
            .unwrap();
        assert_eq!(
            fifo.attributes.kind,
            FileKind::Special(SpecialFileKind::Fifo)
        );
        assert_eq!(fifo.attributes.mode & 0o777, 0o640);
        let looked_up = fs
            .lookup(&ctx, workspace.inode, OsStr::new("events.fifo"))
            .unwrap();
        assert_eq!(
            looked_up.attributes.kind,
            FileKind::Special(SpecialFileKind::Fifo)
        );

        let private = fs
            .mkdir(&ctx, root, OsStr::new("job-local-mknod-private"), 0o555)
            .unwrap();
        let denied = fs
            .mknod(
                &ctx,
                private.inode,
                OsStr::new("denied.fifo"),
                SpecialFileKind::Fifo,
                0o600,
            )
            .expect_err("directory write permission is required for mknod");
        assert_eq!(denied.code(), afs_error::IO_PERMISSION_DENIED);

        let device_dir = fs
            .mkdir(&ctx, root, OsStr::new("job-local-mknod-device"), 0o777)
            .unwrap();
        let non_root = non_owner_context(ctx);
        let denied = fs
            .mknod(
                &non_root,
                device_dir.inode,
                OsStr::new("device"),
                SpecialFileKind::CharDevice { rdev: 0 },
                0o600,
            )
            .expect_err("non-root callers cannot create device nodes");
        assert_eq!(denied.code(), afs_error::IO_PERMISSION_DENIED);
    }

    #[test]
    fn local_create_mkdir_and_mknod_preserve_kernel_filtered_modes() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-mode-preserve"), 0o755)
            .unwrap();

        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("setuid-file"),
                0o4777,
                libc::O_RDWR | libc::O_EXCL,
            )
            .unwrap();
        assert_eq!(created.entry.attributes.mode & 0o7777, 0o4777);
        fs.release(&ctx, created.handle).unwrap();

        let subdir = fs
            .mkdir(&ctx, workspace.inode, OsStr::new("setgid-dir"), 0o2777)
            .unwrap();
        assert_eq!(subdir.attributes.mode & 0o7777, 0o2777);

        let fifo = fs
            .mknod(
                &ctx,
                workspace.inode,
                OsStr::new("events.fifo"),
                SpecialFileKind::Fifo,
                0o777,
            )
            .unwrap();
        assert_eq!(fifo.attributes.mode & 0o7777, 0o777);
    }

    #[test]
    fn local_create_existing_does_not_reset_owner_or_mode() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-create-existing"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o600,
                libc::O_RDWR | libc::O_EXCL,
            )
            .unwrap();
        fs.release(&ctx, created.handle).unwrap();

        let reopened = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o4777,
                libc::O_RDWR,
            )
            .unwrap();
        assert_eq!(reopened.entry.attributes.mode & 0o7777, 0o600);
        fs.release(&ctx, reopened.handle).unwrap();
    }

    #[test]
    fn setattr_authorization_allows_owner_uid_noop_and_member_chgrp() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-chgrp-auth"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o600,
                libc::O_RDWR | libc::O_EXCL,
            )
            .unwrap();
        let attrs = created.entry.attributes.clone();
        fs.release(&ctx, created.handle).unwrap();

        let allowed_gid = attrs.gid.saturating_add(1);
        let owner_with_group = RequestContext {
            uid: attrs.uid,
            gid: attrs.gid,
            supplementary_gids: vec![allowed_gid],
            ..ctx
        };
        authorize_setattr_with_options(
            &owner_with_group,
            &attrs,
            &AttributeChange {
                uid: Some(attrs.uid),
                gid: Some(allowed_gid),
                ..AttributeChange::default()
            },
            SetAttrOptions::default(),
        )
        .unwrap();

        let denied = authorize_setattr_with_options(
            &owner_with_group,
            &attrs,
            &AttributeChange {
                uid: Some(attrs.uid.saturating_add(1)),
                gid: Some(allowed_gid),
                ..AttributeChange::default()
            },
            SetAttrOptions::default(),
        )
        .expect_err("non-root owner cannot change uid to another uid");
        assert_eq!(denied.code(), afs_error::IO_PERMISSION_DENIED);
    }

    #[test]
    fn peer_mknod_uses_caller_permissions_and_parent_identity() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-peer-mknod"), 0o700)
            .unwrap();
        let owner_ctx = context_for_attrs(ctx.clone(), &workspace.attributes);
        let denied_ctx = non_owner_context(owner_ctx.clone());
        let local = fs.require_local().unwrap();
        let parent = local.record(workspace.inode.value).unwrap();
        let grant = test_grant(parent.root_id.clone());
        let access = presented(&grant);

        let denied = local
            .peer_mknod(
                &denied_ctx,
                "node-b",
                &access,
                OsStr::new("events-denied.fifo"),
                SpecialFileKind::Fifo,
                0o600,
                &parent.identity,
            )
            .expect_err("remote non-owner cannot create under a private Home dir");
        assert_eq!(denied.code(), afs_error::IO_PERMISSION_DENIED);

        let fifo = local
            .peer_mknod(
                &owner_ctx,
                "node-b",
                &access,
                OsStr::new("events-ok.fifo"),
                SpecialFileKind::Fifo,
                0o600,
                &parent.identity,
            )
            .unwrap();
        assert_eq!(
            fifo.attributes.kind,
            FileKind::Special(SpecialFileKind::Fifo)
        );
        assert_eq!(fifo.attributes.mode & 0o777, 0o600);
    }

    #[test]
    fn local_setattr_updates_mode_and_times() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-attr"), 0o755).unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        let ctx = context_for_attrs(ctx, &created.entry.attributes);
        let atime = UNIX_EPOCH + Duration::from_secs(1234);
        let mtime = UNIX_EPOCH + Duration::from_secs(5678);
        let updated = fs
            .setattr(
                &ctx,
                created.entry.inode,
                None,
                &AttributeChange {
                    mode: Some(0o600),
                    atime: Some(atime),
                    mtime: Some(mtime),
                    ..AttributeChange::default()
                },
            )
            .unwrap();
        assert_eq!(updated.mode & 0o7777, 0o600);
        assert_eq!(updated.atime, atime);
        assert_eq!(updated.mtime, mtime);
    }

    #[test]
    fn local_setattr_rejects_non_owner_chmod() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-deny"), 0o755).unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o600,
                libc::O_RDWR,
            )
            .unwrap();
        let owner_ctx = context_for_attrs(ctx, &created.entry.attributes);
        let error = fs
            .setattr(
                &non_owner_context(owner_ctx),
                created.entry.inode,
                None,
                &AttributeChange {
                    mode: Some(0o644),
                    ..AttributeChange::default()
                },
            )
            .unwrap_err();
        assert_eq!(error.code(), afs_error::IO_PERMISSION_DENIED);
    }

    #[test]
    fn local_killpriv_write_clears_suid_and_executable_sgid_only_after_positive_write() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-killpriv-write"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o755,
                libc::O_RDWR,
            )
            .unwrap();
        let owner_ctx = context_for_attrs(ctx, &created.entry.attributes);
        fs.setattr(
            &owner_ctx,
            created.entry.inode,
            None,
            &AttributeChange {
                mode: Some(0o6755),
                ..AttributeChange::default()
            },
        )
        .unwrap();

        assert_eq!(
            fs.write_with_options(
                &owner_ctx,
                created.handle,
                0,
                b"x",
                WriteOptions { kill_suidgid: true },
            )
            .unwrap(),
            1
        );
        let attrs = fs
            .getattr(&owner_ctx, created.entry.inode, Some(created.handle))
            .unwrap();
        assert_eq!(attrs.mode & 0o6000, 0);
        fs.release(&owner_ctx, created.handle).unwrap();
    }

    #[test]
    fn local_killpriv_zero_and_failed_writes_do_not_clear_privilege_bits() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-killpriv-noop"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("zero"),
                0o755,
                libc::O_RDWR,
            )
            .unwrap();
        let owner_ctx = context_for_attrs(ctx.clone(), &created.entry.attributes);
        fs.setattr(
            &owner_ctx,
            created.entry.inode,
            None,
            &AttributeChange {
                mode: Some(0o4755),
                ..AttributeChange::default()
            },
        )
        .unwrap();
        assert_eq!(
            fs.write_with_options(
                &owner_ctx,
                created.handle,
                0,
                b"",
                WriteOptions { kill_suidgid: true },
            )
            .unwrap(),
            0
        );
        let attrs = fs
            .getattr(&owner_ctx, created.entry.inode, Some(created.handle))
            .unwrap();
        assert_ne!(attrs.mode & libc::S_ISUID, 0);
        fs.release(&owner_ctx, created.handle).unwrap();

        let readonly = fs
            .open(&owner_ctx, created.entry.inode, libc::O_RDONLY)
            .unwrap();
        let error = fs
            .write_with_options(
                &owner_ctx,
                readonly,
                0,
                b"x",
                WriteOptions { kill_suidgid: true },
            )
            .unwrap_err();
        assert_eq!(error.code(), afs_error::IO_BAD_FILE_DESCRIPTOR);
        let attrs = fs
            .getattr(&owner_ctx, created.entry.inode, Some(readonly))
            .unwrap();
        assert_ne!(attrs.mode & libc::S_ISUID, 0);
        fs.release(&owner_ctx, readonly).unwrap();
    }

    #[test]
    fn local_killpriv_preserves_sgid_without_group_execute() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-killpriv-sgid"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        let owner_ctx = context_for_attrs(ctx, &created.entry.attributes);
        fs.setattr(
            &owner_ctx,
            created.entry.inode,
            None,
            &AttributeChange {
                mode: Some(0o2644),
                ..AttributeChange::default()
            },
        )
        .unwrap();
        assert_eq!(
            fs.write_with_options(
                &owner_ctx,
                created.handle,
                0,
                b"x",
                WriteOptions { kill_suidgid: true },
            )
            .unwrap(),
            1
        );
        let attrs = fs
            .getattr(&owner_ctx, created.entry.inode, Some(created.handle))
            .unwrap();
        assert_ne!(attrs.mode & libc::S_ISGID, 0);
        assert_eq!(attrs.mode & libc::S_IXGRP, 0);
        fs.release(&owner_ctx, created.handle).unwrap();
    }

    #[test]
    fn local_killpriv_truncate_setattr_and_create_existing_clear_privilege_bits() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-killpriv-truncate"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o755,
                libc::O_RDWR,
            )
            .unwrap();
        let owner_ctx = context_for_attrs(ctx, &created.entry.attributes);
        fs.setattr(
            &owner_ctx,
            created.entry.inode,
            None,
            &AttributeChange {
                mode: Some(0o4755),
                ..AttributeChange::default()
            },
        )
        .unwrap();
        let trunc = fs
            .open_with_options(
                &owner_ctx,
                created.entry.inode,
                libc::O_WRONLY | libc::O_TRUNC,
                OpenOptions { kill_suidgid: true },
            )
            .unwrap();
        fs.release(&owner_ctx, trunc).unwrap();
        assert_eq!(
            fs.getattr(&owner_ctx, created.entry.inode, None)
                .unwrap()
                .mode
                & libc::S_ISUID,
            0
        );

        fs.setattr(
            &owner_ctx,
            created.entry.inode,
            None,
            &AttributeChange {
                mode: Some(0o4755),
                ..AttributeChange::default()
            },
        )
        .unwrap();
        fs.setattr_with_options(
            &owner_ctx,
            created.entry.inode,
            None,
            &AttributeChange {
                size: Some(0),
                ..AttributeChange::default()
            },
            SetAttrOptions {
                kill_suidgid: true,
                timestamps_now: false,
            },
        )
        .unwrap();
        assert_eq!(
            fs.getattr(&owner_ctx, created.entry.inode, None)
                .unwrap()
                .mode
                & libc::S_ISUID,
            0
        );

        fs.setattr(
            &owner_ctx,
            created.entry.inode,
            None,
            &AttributeChange {
                mode: Some(0o4755),
                ..AttributeChange::default()
            },
        )
        .unwrap();
        let opened_existing = fs
            .create_with_options(
                &owner_ctx,
                workspace.inode,
                OsStr::new("data"),
                0o755,
                libc::O_RDWR,
                OpenOptions { kill_suidgid: true },
            )
            .unwrap();
        assert_eq!(
            opened_existing.entry.attributes.mode & libc::S_ISUID,
            0,
            "create on an existing file must honor the kernel killpriv cause"
        );
        fs.release(&owner_ctx, opened_existing.handle).unwrap();
        fs.release(&owner_ctx, created.handle).unwrap();
    }

    #[test]
    fn local_user_xattr_roundtrip_and_remove() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-xattr"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        let ctx = context_for_attrs(ctx, &created.entry.attributes);
        let name = OsStr::new("user.afs.test");
        fs.setxattr(&ctx, created.entry.inode, name, b"value", 0)
            .unwrap();
        assert_eq!(
            fs.getxattr(&ctx, created.entry.inode, name).unwrap(),
            b"value"
        );
        let names = fs.listxattr(&ctx, created.entry.inode).unwrap();
        assert!(
            names
                .split(|byte| *byte == 0)
                .any(|name| name == b"user.afs.test")
        );
        fs.removexattr(&ctx, created.entry.inode, name).unwrap();
        let error = fs.getxattr(&ctx, created.entry.inode, name).unwrap_err();
        assert_eq!(error.code(), afs_error::IO_NO_DATA);
    }

    #[test]
    fn local_non_user_xattr_does_not_disable_getxattr_operation() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-xattr-ns"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        let ctx = context_for_attrs(ctx, &created.entry.attributes);

        let unsupported = fs
            .getxattr(&ctx, created.entry.inode, OsStr::new("security.capability"))
            .unwrap_err();
        assert_eq!(unsupported.code(), afs_error::IO_NOT_SUPPORTED);

        fs.setxattr(
            &ctx,
            created.entry.inode,
            OsStr::new("user.afs.after-security"),
            b"value",
            0,
        )
        .unwrap();
        assert_eq!(
            fs.getxattr(
                &ctx,
                created.entry.inode,
                OsStr::new("user.afs.after-security")
            )
            .unwrap(),
            b"value"
        );
    }

    #[test]
    fn local_symlink_readlink_roundtrip() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-symlink"), 0o755)
            .unwrap();
        let link = fs
            .symlink(
                &ctx,
                workspace.inode,
                OsStr::new("link"),
                OsStr::new("target-file"),
            )
            .unwrap();

        assert_eq!(link.attributes.kind, FileKind::Symlink);
        assert_eq!(
            fs.readlink(&ctx, link.inode).unwrap(),
            OsString::from("target-file")
        );
    }

    #[test]
    fn local_hardlink_refreshes_nlink_and_shares_data() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-hardlink"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&ctx, created.handle, 0, b"same-bytes").unwrap();
        fs.flush(&ctx, created.handle).unwrap();
        fs.release(&ctx, created.handle).unwrap();

        let linked = fs
            .link(
                &ctx,
                created.entry.inode,
                workspace.inode,
                OsStr::new("linked"),
            )
            .unwrap();
        assert_eq!(linked.inode, created.entry.inode);
        assert_eq!(linked.attributes.nlink, 2);
        assert_eq!(
            fs.getattr(&ctx, created.entry.inode, None).unwrap().nlink,
            2
        );

        fs.unlink(&ctx, workspace.inode, OsStr::new("data"))
            .unwrap();
        assert_eq!(fs.getattr(&ctx, linked.inode, None).unwrap().nlink, 1);

        let handle = fs.open(&ctx, linked.inode, libc::O_RDONLY).unwrap();
        let mut data = [0_u8; 10];
        let read = fs.read(&ctx, handle, 0, &mut data).unwrap();
        fs.release(&ctx, handle).unwrap();
        assert_eq!(&data[..read], b"same-bytes");
    }

    #[test]
    fn local_rename_hardlink_to_itself_keeps_source_alias_cached() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-rename-hardlink"), 0o755)
            .unwrap();
        let created = fs
            .create(&ctx, workspace.inode, OsStr::new("a"), 0o644, libc::O_RDWR)
            .unwrap();
        fs.release(&ctx, created.handle).unwrap();
        let linked = fs
            .link(&ctx, created.entry.inode, workspace.inode, OsStr::new("b"))
            .unwrap();
        assert_eq!(linked.inode, created.entry.inode);

        fs.rename(
            &ctx,
            workspace.inode,
            OsStr::new("a"),
            workspace.inode,
            OsStr::new("b"),
            RenameFlags(0),
        )
        .unwrap();
        fs.unlink(&ctx, workspace.inode, OsStr::new("b")).unwrap();

        let remaining = fs.lookup(&ctx, workspace.inode, OsStr::new("a")).unwrap();
        assert_eq!(remaining.inode, created.entry.inode);
        assert_eq!(fs.getattr(&ctx, remaining.inode, None).unwrap().nlink, 1);
    }

    #[test]
    fn lookup_of_externally_moved_directory_rebinds_cached_descendants() {
        let (_temp, fs, ctx) = fixture();
        let workspace = fs
            .mkdir(
                &ctx,
                BackendInode {
                    value: OWNERFS_ROOT_INODE,
                },
                OsStr::new("native-move"),
                0o755,
            )
            .unwrap();
        let left = fs
            .mkdir(&ctx, workspace.inode, OsStr::new("left"), 0o755)
            .unwrap();
        let right = fs
            .mkdir(&ctx, workspace.inode, OsStr::new("right"), 0o755)
            .unwrap();
        let moving = fs
            .mkdir(&ctx, left.inode, OsStr::new("moving"), 0o755)
            .unwrap();
        let child = fs
            .create(&ctx, moving.inode, OsStr::new("data"), 0o644, libc::O_RDWR)
            .unwrap();
        fs.write(&ctx, child.handle, 0, b"same object").unwrap();
        fs.release(&ctx, child.handle).unwrap();
        let local = fs.require_local().unwrap();
        let record = local.record(workspace.inode.value).unwrap();
        let authority = local
            .roots
            .enter_root(&record.root_id, RootRight::Write)
            .unwrap();
        let data = local.disk.root_path().join(authority.data_dir().as_path());
        std::fs::rename(data.join("left/moving"), data.join("right/moving")).unwrap();
        // Diagnostic refresh is explicit here; automatic old-dirfd/cwd repair
        // remains a separate native contract case, not claimed by this test.
        let refreshed = fs.lookup(&ctx, right.inode, OsStr::new("moving")).unwrap();
        assert_eq!(refreshed.inode, moving.inode);
        let retained_child = fs.lookup(&ctx, moving.inode, OsStr::new("data")).unwrap();
        assert_eq!(retained_child.inode, child.entry.inode);
        let handle = fs.open(&ctx, child.entry.inode, libc::O_RDONLY).unwrap();
        let mut bytes = [0_u8; 64];
        let count = fs.read(&ctx, handle, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..count], b"same object");
        fs.release(&ctx, handle).unwrap();
    }

    #[test]
    fn local_rename_over_linked_destination_rebinds_surviving_alias() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-rename-linked-dst"), 0o755)
            .unwrap();
        let source = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("src"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.release(&ctx, source.handle).unwrap();
        let destination = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("dst"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.release(&ctx, destination.handle).unwrap();

        let destination_primary = fs.lookup(&ctx, workspace.inode, OsStr::new("dst")).unwrap();
        assert_eq!(destination_primary.inode, destination.entry.inode);
        let alias = fs
            .link(
                &ctx,
                destination_primary.inode,
                workspace.inode,
                OsStr::new("dstlnk"),
            )
            .unwrap();
        assert_eq!(alias.inode, destination.entry.inode);

        fs.rename(
            &ctx,
            workspace.inode,
            OsStr::new("src"),
            workspace.inode,
            OsStr::new("dst"),
            RenameFlags(0),
        )
        .unwrap();

        let replacement = fs.lookup(&ctx, workspace.inode, OsStr::new("dst")).unwrap();
        assert_eq!(replacement.inode, source.entry.inode);
        let remaining = fs
            .lookup(&ctx, workspace.inode, OsStr::new("dstlnk"))
            .unwrap();
        assert_eq!(remaining.inode, destination.entry.inode);
        let remaining_attrs = fs.getattr(&ctx, remaining.inode, None).unwrap();
        assert_eq!(remaining_attrs.kind, FileKind::Regular);
        assert_eq!(remaining_attrs.nlink, 1);
    }

    #[test]
    fn writable_created_mode_zero_handle_can_ftruncate_but_readonly_cannot() {
        let (_temp, fs, ctx) = fixture();
        let workspace = fs
            .mkdir(
                &ctx,
                backend_inode(OWNERFS_ROOT_INODE),
                OsStr::new("job-fd-truncate"),
                0o777,
            )
            .unwrap();
        let mut creator = non_owner_context(ctx.clone());
        // Physical chown is privileged; non-root runners exercise their own uid.
        if ctx.uid != 0 {
            creator = ctx.clone();
        }
        let created = fs
            .create(
                &creator,
                workspace.inode,
                OsStr::new("mode-zero"),
                if ctx.uid == 0 { 0 } else { 0o600 },
                libc::O_RDWR,
            )
            .unwrap();
        if ctx.uid != 0 {
            // Unprivileged daemon cannot reopen mode zero after create; make
            // the already writable descriptor's current mode zero instead.
            fs.setattr(
                &creator,
                created.entry.inode,
                Some(created.handle),
                &AttributeChange {
                    mode: Some(0),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        fs.setattr(
            &creator,
            created.entry.inode,
            Some(created.handle),
            &AttributeChange {
                size: Some(3),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            fs.getattr(&creator, created.entry.inode, Some(created.handle))
                .unwrap()
                .size,
            3
        );
        fs.release(&creator, created.handle).unwrap();
        let readable = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("readonly"),
                0o644,
                libc::O_RDONLY,
            )
            .unwrap();
        assert_eq!(
            fs.setattr(
                &ctx,
                readable.entry.inode,
                Some(readable.handle),
                &AttributeChange {
                    size: Some(0),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code(),
            afs_error::IO_BAD_FILE_DESCRIPTOR
        );
        fs.release(&ctx, readable.handle).unwrap();
    }

    #[test]
    fn getattr_without_fh_observes_held_unlinked_inode_not_reused_path() {
        let (_temp, fs, ctx) = fixture();
        let workspace = fs
            .mkdir(
                &ctx,
                backend_inode(OWNERFS_ROOT_INODE),
                OsStr::new("job-fstat-unlinked"),
                0o755,
            )
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("name"),
                0o644,
                libc::O_RDONLY,
            )
            .unwrap();
        fs.unlink(&ctx, workspace.inode, OsStr::new("name"))
            .unwrap();
        assert_eq!(
            fs.getattr(&ctx, created.entry.inode, None).unwrap().nlink,
            0
        );
        let replacement = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("name"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        assert_ne!(created.entry.inode, replacement.entry.inode);
        assert_eq!(
            fs.getattr(&ctx, created.entry.inode, None).unwrap().nlink,
            0
        );
        fs.release(&ctx, created.handle).unwrap();
        assert!(fs.getattr(&ctx, created.entry.inode, None).is_err());
        fs.release(&ctx, replacement.handle).unwrap();
    }

    #[test]
    fn local_namespace_mutations_enforce_parent_permissions_and_sticky() {
        let (_temp, fs, ctx) = fixture();
        let denied = non_owner_context(ctx.clone());
        for (name, mode, error) in [
            ("job-private-remove", 0o700, afs_error::IO_PERMISSION_DENIED),
            (
                "job-sticky-remove",
                0o1777,
                afs_error::IO_OPERATION_NOT_PERMITTED,
            ),
        ] {
            let workspace = fs
                .mkdir(
                    &ctx,
                    backend_inode(OWNERFS_ROOT_INODE),
                    OsStr::new(name),
                    mode,
                )
                .unwrap();
            let file = fs
                .create(
                    &ctx,
                    workspace.inode,
                    OsStr::new("file"),
                    0o600,
                    libc::O_RDWR,
                )
                .unwrap();
            fs.release(&ctx, file.handle).unwrap();
            fs.mkdir(&ctx, workspace.inode, OsStr::new("directory"), 0o700)
                .unwrap();
            assert_eq!(
                fs.unlink(&denied, workspace.inode, OsStr::new("file"))
                    .unwrap_err()
                    .code(),
                error
            );
            assert_eq!(
                fs.rmdir(&denied, workspace.inode, OsStr::new("directory"))
                    .unwrap_err()
                    .code(),
                error
            );
            assert_eq!(
                fs.rename(
                    &denied,
                    workspace.inode,
                    OsStr::new("file"),
                    workspace.inode,
                    OsStr::new("renamed"),
                    RenameFlags(0)
                )
                .unwrap_err()
                .code(),
                error
            );
            // Permission rejection must leave the original namespace intact.
            fs.lookup(&ctx, workspace.inode, OsStr::new("file"))
                .unwrap();
            fs.lookup(&ctx, workspace.inode, OsStr::new("directory"))
                .unwrap();
            fs.unlink(&ctx, workspace.inode, OsStr::new("file"))
                .unwrap();
            fs.rmdir(&ctx, workspace.inode, OsStr::new("directory"))
                .unwrap();
        }
    }

    #[test]
    fn peer_namespace_mutations_enforce_parent_permissions_and_sticky() {
        let (_temp, fs, ctx) = fixture();
        let denied = non_owner_context(ctx.clone());
        for (name, mode, error) in [
            (
                "job-peer-private-remove",
                0o700,
                afs_error::IO_PERMISSION_DENIED,
            ),
            (
                "job-peer-sticky-remove",
                0o1777,
                afs_error::IO_OPERATION_NOT_PERMITTED,
            ),
        ] {
            let workspace = fs
                .mkdir(
                    &ctx,
                    backend_inode(OWNERFS_ROOT_INODE),
                    OsStr::new(name),
                    mode,
                )
                .unwrap();
            let file = fs
                .create(
                    &ctx,
                    workspace.inode,
                    OsStr::new("file"),
                    0o600,
                    libc::O_RDWR,
                )
                .unwrap();
            fs.release(&ctx, file.handle).unwrap();
            let directory = fs
                .mkdir(&ctx, workspace.inode, OsStr::new("directory"), 0o700)
                .unwrap();
            let local = fs.require_local().unwrap();
            let parent = local.record(workspace.inode.value).unwrap();
            let source = local.record(file.entry.inode.value).unwrap();
            let victim_dir = local.record(directory.inode.value).unwrap();
            let root_use = local
                .roots
                .enter_root(&parent.root_id, RootRight::Lookup)
                .unwrap();
            let mut peer_grant = root_use.grant().clone();
            peer_grant.holder_node_id = "node-b".into();
            peer_grant.session_id = "session-b".into();
            drop(root_use);
            let access = presented(&peer_grant);
            assert_eq!(
                local
                    .peer_unlink(
                        &denied,
                        "node-b",
                        &access,
                        OsStr::new("file"),
                        Some(&source.identity),
                        &parent.identity
                    )
                    .unwrap_err()
                    .code(),
                error
            );
            assert_eq!(
                local
                    .peer_rmdir(
                        &denied,
                        "node-b",
                        &access,
                        OsStr::new("directory"),
                        Some(&victim_dir.identity),
                        &parent.identity
                    )
                    .unwrap_err()
                    .code(),
                error
            );
            assert_eq!(
                local
                    .peer_rename(
                        &denied,
                        "node-b",
                        &access,
                        OsStr::new("file"),
                        OsStr::new("renamed"),
                        Some(&source.identity),
                        None,
                        &parent.identity,
                        &parent.identity,
                        RenameFlags(0)
                    )
                    .unwrap_err()
                    .code(),
                error
            );
            fs.lookup(&ctx, workspace.inode, OsStr::new("file"))
                .unwrap();
            fs.lookup(&ctx, workspace.inode, OsStr::new("directory"))
                .unwrap();
        }
    }

    #[test]
    fn local_and_peer_rename_enforce_sticky_destination_victim() {
        let (_temp, fs, ctx) = fixture();
        let denied = non_owner_context(ctx.clone());
        let workspace = fs
            .mkdir(
                &ctx,
                backend_inode(OWNERFS_ROOT_INODE),
                OsStr::new("job-sticky-destination"),
                0o777,
            )
            .unwrap();
        let destination = fs
            .mkdir(&ctx, workspace.inode, OsStr::new("sticky"), 0o1777)
            .unwrap();
        let source = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("source"),
                0o600,
                libc::O_RDWR,
            )
            .unwrap();
        fs.release(&ctx, source.handle).unwrap();
        let target = fs
            .create(
                &ctx,
                destination.inode,
                OsStr::new("target"),
                0o600,
                libc::O_RDWR,
            )
            .unwrap();
        fs.release(&ctx, target.handle).unwrap();
        assert_eq!(
            fs.rename(
                &denied,
                workspace.inode,
                OsStr::new("source"),
                destination.inode,
                OsStr::new("target"),
                RenameFlags(0)
            )
            .unwrap_err()
            .code(),
            afs_error::IO_OPERATION_NOT_PERMITTED
        );
        let local = fs.require_local().unwrap();
        let parent = local.record(workspace.inode.value).unwrap();
        let target_parent = local.record(destination.inode.value).unwrap();
        let source_record = local.record(source.entry.inode.value).unwrap();
        let access = presented(&test_grant(parent.root_id.clone()));
        // No expected_new_identity models an uncached existing destination.
        assert_eq!(
            local
                .peer_rename(
                    &denied,
                    "node-b",
                    &access,
                    OsStr::new("source"),
                    OsStr::new("sticky/target"),
                    Some(&source_record.identity),
                    None,
                    &parent.identity,
                    &target_parent.identity,
                    RenameFlags(0)
                )
                .unwrap_err()
                .code(),
            afs_error::IO_OPERATION_NOT_PERMITTED
        );
        fs.lookup(&ctx, workspace.inode, OsStr::new("source"))
            .unwrap();
        fs.lookup(&ctx, destination.inode, OsStr::new("target"))
            .unwrap();
    }

    #[test]
    fn namespace_remove_checks_search_and_sticky_source_or_destination() {
        let (_temp, fs, ctx) = fixture();
        let workspace = fs
            .mkdir(
                &ctx,
                backend_inode(OWNERFS_ROOT_INODE),
                OsStr::new("job-permission-helper"),
                0o777,
            )
            .unwrap();
        let mut parent = workspace.attributes;
        let mut caller = non_owner_context(ctx.clone());
        let mut victim = parent.clone();
        victim.kind = FileKind::Regular;
        parent.mode = 0o666; // Write without directory search is insufficient.
        assert_eq!(
            authorize_namespace_remove(&caller, &parent, &victim)
                .unwrap_err()
                .code(),
            afs_error::IO_PERMISSION_DENIED
        );
        parent.mode = 0o1777;
        assert_eq!(
            authorize_namespace_remove(&caller, &parent, &victim)
                .unwrap_err()
                .code(),
            afs_error::IO_OPERATION_NOT_PERMITTED
        );
        victim.uid = caller.uid;
        authorize_namespace_remove(&caller, &parent, &victim).unwrap();
        victim.uid = ctx.uid;
        parent.uid = caller.uid;
        authorize_namespace_remove(&caller, &parent, &victim).unwrap();
        parent.uid = ctx.uid;
        caller.uid = 0;
        authorize_namespace_remove(&caller, &parent, &victim).unwrap();
    }

    #[test]
    fn local_and_peer_create_inherit_sgid_parent() {
        let (_temp, fs, ctx) = fixture();
        let workspace = fs
            .mkdir(
                &ctx,
                backend_inode(OWNERFS_ROOT_INODE),
                OsStr::new("job-sgid"),
                0o2777,
            )
            .unwrap();
        let mut creator = ctx.clone();
        // A privileged daemon can exercise a parent group different from the caller.
        // Unprivileged test runners still verify directory SGID bit inheritance.
        if ctx.uid == 0 {
            creator.gid = workspace.attributes.gid + 1;
        }
        let inherited = workspace.attributes.gid;
        let file = fs
            .create(
                &creator,
                workspace.inode,
                OsStr::new("file"),
                0o600,
                libc::O_RDWR,
            )
            .unwrap();
        assert_eq!(file.entry.attributes.gid, inherited);
        fs.release(&creator, file.handle).unwrap();
        let directory = fs
            .mkdir(&creator, workspace.inode, OsStr::new("directory"), 0o755)
            .unwrap();
        assert_eq!(directory.attributes.gid, inherited);
        assert_ne!(directory.attributes.mode & libc::S_ISGID, 0);
        let link = fs
            .symlink(
                &creator,
                workspace.inode,
                OsStr::new("link"),
                OsStr::new("target"),
            )
            .unwrap();
        assert_eq!(link.attributes.gid, inherited);
        let local = fs.require_local().unwrap();
        let parent = local.record(workspace.inode.value).unwrap();
        let access = presented(&test_grant(parent.root_id.clone()));
        let peer_file = local
            .peer_create_with_options(
                &creator,
                "node-b",
                &access,
                OsStr::new("peer-file"),
                libc::O_RDWR,
                0o600,
                &parent.identity,
                OpenOptions::default(),
            )
            .unwrap();
        assert_eq!(peer_file.entry.attributes.gid, inherited);
        let peer_dir = local
            .peer_mkdir(
                &creator,
                "node-b",
                &access,
                OsStr::new("peer-directory"),
                0o755,
                &parent.identity,
            )
            .unwrap();
        assert_eq!(peer_dir.attributes.gid, inherited);
        assert_ne!(peer_dir.attributes.mode & libc::S_ISGID, 0);
        let peer_link = local
            .peer_symlink(
                &creator,
                "node-b",
                &access,
                OsStr::new("peer-link"),
                OsStr::new("target"),
                &parent.identity,
            )
            .unwrap();
        assert_eq!(peer_link.attributes.gid, inherited);
    }

    #[test]
    fn peer_symlink_uses_caller_permissions_and_parent_identity() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-peer-symlink"), 0o700)
            .unwrap();
        let owner_ctx = context_for_attrs(ctx.clone(), &workspace.attributes);
        let denied_ctx = non_owner_context(owner_ctx.clone());
        let local = fs.require_local().unwrap();
        let parent = local.record(workspace.inode.value).unwrap();
        let grant = test_grant(parent.root_id.clone());
        let access = presented(&grant);

        let denied = local
            .peer_symlink(
                &denied_ctx,
                "node-b",
                &access,
                OsStr::new("link-denied"),
                OsStr::new("target"),
                &parent.identity,
            )
            .expect_err("remote non-owner cannot create under a private Home dir");
        assert_eq!(denied.code(), afs_error::IO_PERMISSION_DENIED);

        let link = local
            .peer_symlink(
                &owner_ctx,
                "node-b",
                &access,
                OsStr::new("link-ok"),
                OsStr::new("target"),
                &parent.identity,
            )
            .unwrap();
        assert_eq!(link.attributes.kind, FileKind::Symlink);
        assert_eq!(
            local
                .peer_readlink(
                    "node-b",
                    &access,
                    OsStr::new("link-ok"),
                    Some(&link.identity)
                )
                .unwrap(),
            b"target"
        );

        let stale_parent = files::FileIdentity(b"stale-parent".to_vec());
        let stale = local
            .peer_symlink(
                &owner_ctx,
                "node-b",
                &access,
                OsStr::new("link-stale"),
                OsStr::new("target"),
                &stale_parent,
            )
            .expect_err("stale parent identity must be rejected");
        assert_eq!(stale.code(), afs_error::NODE_OWNER_STALE_HANDLE);
    }

    #[test]
    fn peer_hardlink_uses_caller_permissions_and_refreshes_nlink() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-peer-link"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o600,
                libc::O_RDWR,
            )
            .unwrap();
        fs.release(&ctx, created.handle).unwrap();
        let owner_ctx = context_for_attrs(ctx.clone(), &created.entry.attributes);
        let denied_ctx = non_owner_context(owner_ctx.clone());
        let local = fs.require_local().unwrap();
        let source = local.record(created.entry.inode.value).unwrap();
        let parent = local.record(workspace.inode.value).unwrap();
        let grant = test_grant(parent.root_id.clone());
        let access = presented(&grant);

        let denied = local
            .peer_link(
                &denied_ctx,
                "node-b",
                &access,
                source.relative.as_path().as_os_str(),
                OsStr::new("linked-denied"),
                &source.identity,
                &parent.identity,
            )
            .expect_err("remote non-owner cannot link a private file");
        assert_eq!(denied.code(), afs_error::IO_PERMISSION_DENIED);

        let linked = local
            .peer_link(
                &owner_ctx,
                "node-b",
                &access,
                source.relative.as_path().as_os_str(),
                OsStr::new("linked-ok"),
                &source.identity,
                &parent.identity,
            )
            .unwrap();
        assert_eq!(linked.attributes.nlink, 2);
        let linked_lookup = fs
            .lookup(&ctx, workspace.inode, OsStr::new("linked-ok"))
            .unwrap();
        assert_eq!(linked_lookup.inode, created.entry.inode);
        assert_eq!(
            fs.getattr(&ctx, created.entry.inode, None).unwrap().nlink,
            2
        );
        let stale_source = files::FileIdentity(b"stale-source".to_vec());
        let stale = local
            .peer_link(
                &owner_ctx,
                "node-b",
                &access,
                source.relative.as_path().as_os_str(),
                OsStr::new("linked-stale"),
                &stale_source,
                &parent.identity,
            )
            .expect_err("stale source identity must be rejected");
        assert_eq!(stale.code(), afs_error::NODE_OWNER_STALE_HANDLE);

        local
            .peer_unlink(
                &owner_ctx,
                "node-b",
                &access,
                source.relative.as_path().as_os_str(),
                Some(&source.identity),
                &parent.identity,
            )
            .unwrap();
        assert_eq!(
            fs.getattr(&ctx, linked_lookup.inode, None).unwrap().nlink,
            1
        );
    }

    #[test]
    fn peer_rename_hardlink_to_itself_keeps_source_alias_cached() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-peer-rename-hardlink"), 0o755)
            .unwrap();
        let created = fs
            .create(&ctx, workspace.inode, OsStr::new("a"), 0o644, libc::O_RDWR)
            .unwrap();
        fs.release(&ctx, created.handle).unwrap();
        let owner_ctx = context_for_attrs(ctx.clone(), &created.entry.attributes);
        let local = fs.require_local().unwrap();
        let source = local.record(created.entry.inode.value).unwrap();
        let parent = local.record(workspace.inode.value).unwrap();
        let grant = test_grant(parent.root_id.clone());
        let access = presented(&grant);
        let linked = local
            .peer_link(
                &owner_ctx,
                "node-b",
                &access,
                source.relative.as_path().as_os_str(),
                OsStr::new("b"),
                &source.identity,
                &parent.identity,
            )
            .unwrap();

        local
            .peer_rename(
                &owner_ctx,
                "node-b",
                &access,
                source.relative.as_path().as_os_str(),
                OsStr::new("b"),
                Some(&source.identity),
                Some(&linked.identity),
                &parent.identity,
                &parent.identity,
                RenameFlags(0),
            )
            .unwrap();
        local
            .peer_unlink(
                &owner_ctx,
                "node-b",
                &access,
                OsStr::new("b"),
                Some(&linked.identity),
                &parent.identity,
            )
            .unwrap();

        let remaining = fs.lookup(&ctx, workspace.inode, OsStr::new("a")).unwrap();
        assert_eq!(remaining.inode, created.entry.inode);
        assert_eq!(fs.getattr(&ctx, remaining.inode, None).unwrap().nlink, 1);
    }

    #[test]
    fn peer_rename_over_linked_destination_rebinds_surviving_alias() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-peer-rename-linked-dst"), 0o755)
            .unwrap();
        let source = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("src"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.release(&ctx, source.handle).unwrap();
        let destination = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("dst"),
                0o644,
                libc::O_RDWR,
            )
            .unwrap();
        fs.release(&ctx, destination.handle).unwrap();

        let destination_primary = fs.lookup(&ctx, workspace.inode, OsStr::new("dst")).unwrap();
        assert_eq!(destination_primary.inode, destination.entry.inode);

        let owner_ctx = context_for_attrs(ctx.clone(), &destination.entry.attributes);
        let local = fs.require_local().unwrap();
        let source_record = local.record(source.entry.inode.value).unwrap();
        let destination_record = local.record(destination_primary.inode.value).unwrap();
        let parent = local.record(workspace.inode.value).unwrap();
        let grant = test_grant(parent.root_id.clone());
        let access = presented(&grant);
        let alias = local
            .peer_link(
                &owner_ctx,
                "node-b",
                &access,
                destination_record.relative.as_path().as_os_str(),
                OsStr::new("dstlnk"),
                &destination_record.identity,
                &parent.identity,
            )
            .unwrap();
        assert_eq!(alias.attributes.nlink, 2);

        local
            .peer_rename(
                &owner_ctx,
                "node-b",
                &access,
                source_record.relative.as_path().as_os_str(),
                destination_record.relative.as_path().as_os_str(),
                Some(&source_record.identity),
                Some(&destination_record.identity),
                &parent.identity,
                &parent.identity,
                RenameFlags(0),
            )
            .unwrap();

        let replacement = fs.lookup(&ctx, workspace.inode, OsStr::new("dst")).unwrap();
        assert_eq!(replacement.inode, source.entry.inode);
        let remaining = fs
            .lookup(&ctx, workspace.inode, OsStr::new("dstlnk"))
            .unwrap();
        assert_eq!(remaining.inode, destination.entry.inode);
        let remaining_attrs = fs.getattr(&ctx, remaining.inode, None).unwrap();
        assert_eq!(remaining_attrs.kind, FileKind::Regular);
        assert_eq!(remaining_attrs.nlink, 1);
    }

    #[test]
    fn peer_rename_uncached_hardlink_destination_keeps_source_alias_cached() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(
                &ctx,
                root,
                OsStr::new("job-peer-rename-hardlink-uncached"),
                0o755,
            )
            .unwrap();
        let created = fs
            .create(&ctx, workspace.inode, OsStr::new("a"), 0o644, libc::O_RDWR)
            .unwrap();
        fs.release(&ctx, created.handle).unwrap();
        let owner_ctx = context_for_attrs(ctx.clone(), &created.entry.attributes);
        let local = fs.require_local().unwrap();
        let source = local.record(created.entry.inode.value).unwrap();
        let parent = local.record(workspace.inode.value).unwrap();
        let grant = test_grant(parent.root_id.clone());
        let access = presented(&grant);
        let linked = local
            .peer_link(
                &owner_ctx,
                "node-b",
                &access,
                source.relative.as_path().as_os_str(),
                OsStr::new("b"),
                &source.identity,
                &parent.identity,
            )
            .unwrap();

        local
            .peer_rename(
                &owner_ctx,
                "node-b",
                &access,
                source.relative.as_path().as_os_str(),
                OsStr::new("b"),
                Some(&source.identity),
                None,
                &parent.identity,
                &parent.identity,
                RenameFlags(0),
            )
            .unwrap();
        local
            .peer_unlink(
                &owner_ctx,
                "node-b",
                &access,
                OsStr::new("b"),
                Some(&linked.identity),
                &parent.identity,
            )
            .unwrap();

        let remaining = fs.lookup(&ctx, workspace.inode, OsStr::new("a")).unwrap();
        assert_eq!(remaining.inode, created.entry.inode);
        assert_eq!(fs.getattr(&ctx, remaining.inode, None).unwrap().nlink, 1);
    }

    #[test]
    fn peer_user_xattr_roundtrip_uses_original_caller_permissions() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs
            .mkdir(&ctx, root, OsStr::new("job-peer-xattr"), 0o755)
            .unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("data"),
                0o600,
                libc::O_RDWR,
            )
            .unwrap();
        let owner_ctx = context_for_attrs(ctx.clone(), &created.entry.attributes);
        let denied_ctx = non_owner_context(owner_ctx.clone());
        let local = fs.require_local().unwrap();
        let record = local.record(created.entry.inode.value).unwrap();
        let grant = test_grant(record.root_id.clone());
        let access = presented(&grant);
        let path = record.relative.as_path().as_os_str();
        let name = OsStr::new("user.afs.peer");

        local
            .peer_setxattr(
                &owner_ctx,
                "node-b",
                &access,
                path,
                &record.identity,
                name,
                b"peer",
                0,
            )
            .unwrap();
        assert_eq!(
            local
                .peer_getxattr(&owner_ctx, "node-b", &access, path, &record.identity, name)
                .unwrap(),
            b"peer"
        );
        let names = local
            .peer_listxattr(&owner_ctx, "node-b", &access, path, &record.identity)
            .unwrap();
        assert!(
            names
                .split(|byte| *byte == 0)
                .any(|entry| entry == b"user.afs.peer")
        );

        let denied = local
            .peer_setxattr(
                &denied_ctx,
                "node-b",
                &access,
                path,
                &record.identity,
                OsStr::new("user.afs.denied"),
                b"no",
                0,
            )
            .unwrap_err();
        assert_eq!(denied.code(), afs_error::IO_PERMISSION_DENIED);

        local
            .peer_removexattr(&owner_ctx, "node-b", &access, path, &record.identity, name)
            .unwrap();
        let removed = local
            .peer_getxattr(&owner_ctx, "node-b", &access, path, &record.identity, name)
            .unwrap_err();
        assert_eq!(removed.code(), afs_error::IO_NO_DATA);
    }

    #[test]
    fn home_handle_is_bound_to_its_peer_root_session_and_identity() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
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
            true,
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
            let ctx = ctx.clone();
            std::thread::spawn(move || fs.write(&ctx, first, 0, b"left").unwrap())
        };
        let right = {
            let fs = fs.clone();
            let ctx = ctx.clone();
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
            let ctx = ctx.clone();
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
    fn remote_write_receives_kernel_killpriv_cause() {
        let (_temp, fs, ctx) = fixture();
        let root_id = root::root_id_from_name(OsStr::new("job-42")).unwrap();
        let remote = Arc::new(SlowRemoteFiles::new(root_id, Duration::ZERO));
        let handle = insert_slow_remote_handle(&fs, remote.clone(), 1);

        assert_eq!(
            fs.write_with_options(
                &ctx,
                handle,
                0,
                b"data",
                WriteOptions { kill_suidgid: true },
            )
            .unwrap(),
            4
        );
        assert!(remote.last_write_killpriv.load(Ordering::SeqCst));
        fs.release(&ctx, handle).unwrap();
    }

    #[test]
    fn remote_flush_runs_once_after_write_and_release_only_cleans_up() {
        let (_temp, fs, ctx) = fixture();
        let root_id = root::root_id_from_name(OsStr::new("job-42")).unwrap();
        let remote = Arc::new(SlowRemoteFiles::new(root_id, Duration::ZERO));
        let handle = insert_slow_remote_handle(&fs, remote.clone(), 1);

        fs.flush(&ctx, handle).unwrap();
        assert_eq!(remote.flush_calls.load(Ordering::SeqCst), 0);

        assert_eq!(fs.write(&ctx, handle, 0, b"data").unwrap(), 4);
        fs.flush(&ctx, handle).unwrap();
        fs.flush(&ctx, handle).unwrap();
        assert_eq!(remote.flush_calls.load(Ordering::SeqCst), 1);

        fs.release(&ctx, handle).unwrap();
        assert_eq!(remote.release_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn remote_lookup_refreshes_cached_grant_after_home_restart() {
        let (_temp, fs, ctx, meta, remote, _root_id) = remote_fixture();
        let root = BackendInode {
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
    fn remote_open_receives_kernel_killpriv_cause() {
        let (_temp, fs, ctx, _meta, remote, root_id) = remote_fixture();
        let root = BackendInode {
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
            .open_with_options(
                &ctx,
                backend_inode(child_inode),
                libc::O_WRONLY | libc::O_TRUNC,
                OpenOptions { kill_suidgid: true },
            )
            .unwrap();

        assert!(remote.last_open_killpriv.load(Ordering::SeqCst));
        fs.release(&ctx, handle).unwrap();
    }

    #[test]
    fn local_root_and_file_round_trip_without_per_write_meta() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
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
    fn local_flush_after_write_reaches_file_data_sync() {
        const CHILD_ENV: &str = "AFS_OWNERFS_FLUSH_TRACE_CHILD";
        if env::var_os(CHILD_ENV).is_some() {
            run_local_flush_trace_child();
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let trace = temp.path().join("ownerfs-flush.strace");
        let status = Command::new("strace")
            .arg("-qq")
            .arg("-f")
            .arg("-e")
            .arg("trace=fdatasync,fsync")
            .arg("-o")
            .arg(&trace)
            .env(CHILD_ENV, "1")
            .arg(env::current_exe().unwrap())
            .arg("--exact")
            .arg("node::vfs::ownerfs::tests::local_flush_after_write_reaches_file_data_sync")
            .arg("--nocapture")
            .status()
            .expect("strace must be installed in the Linux validation environment");
        assert!(status.success(), "trace child failed with {status}");

        let raw = fs::read_to_string(&trace).unwrap();
        assert!(
            raw.contains("fdatasync("),
            "OwnerFs local flush returned without a file data sync syscall:\n{raw}"
        );
    }

    fn run_local_flush_trace_child() {
        let (_temp, fs, ctx) = fixture();
        let root_id = root::root_id_from_name(OsStr::new("job-42")).unwrap();
        let read_only = {
            let local = fs.require_local().unwrap();
            let path = StoragePath::new("readonly.txt").unwrap();
            let file = local
                .disk
                .open_file(&path, OpenSpec::new(libc::O_RDWR | libc::O_CREAT, 0o644))
                .unwrap();
            let identity = identity_from_metadata(&file.metadata().unwrap()).unwrap();
            local.state.lock().unwrap().insert_file_handle(
                files::LocalOpenFile {
                    root_id: root_id.clone(),
                    identity,
                    file,
                    peer: None,
                },
                false,
                WriteSyncMode::None,
                libc::O_RDONLY,
            )
        };
        fs.flush(&ctx, read_only).unwrap();
        fs.release(&ctx, read_only).unwrap();

        let writable = {
            let local = fs.require_local().unwrap();
            let path = StoragePath::new("log.txt").unwrap();
            let file = local
                .disk
                .open_file(&path, OpenSpec::new(libc::O_RDWR | libc::O_CREAT, 0o644))
                .unwrap();
            let identity = identity_from_metadata(&file.metadata().unwrap()).unwrap();
            local.state.lock().unwrap().insert_file_handle(
                files::LocalOpenFile {
                    root_id,
                    identity,
                    file,
                    peer: None,
                },
                false,
                WriteSyncMode::None,
                libc::O_RDWR,
            )
        };
        assert_eq!(fs.write(&ctx, writable, 0, b"hello").unwrap(), 5);
        fs.flush(&ctx, writable).unwrap();
        fs.flush(&ctx, writable).unwrap();
        fs.release(&ctx, writable).unwrap();
    }

    #[test]
    fn local_odsync_write_reaches_file_data_sync_before_return() {
        const CHILD_ENV: &str = "AFS_OWNERFS_ODSYNC_TRACE_CHILD";
        if env::var_os(CHILD_ENV).is_some() {
            run_local_odsync_write_trace_child();
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let trace = temp.path().join("ownerfs-odsync-write.strace");
        let status = Command::new("strace")
            .arg("-qq")
            .arg("-f")
            .arg("-e")
            .arg("trace=fdatasync,fsync,pwrite64")
            .arg("-o")
            .arg(&trace)
            .env(CHILD_ENV, "1")
            .arg(env::current_exe().unwrap())
            .arg("--exact")
            .arg("node::vfs::ownerfs::tests::local_odsync_write_reaches_file_data_sync_before_return")
            .arg("--nocapture")
            .status()
            .expect("strace must be installed in the Linux validation environment");
        assert!(status.success(), "trace child failed with {status}");

        let raw = fs::read_to_string(&trace).unwrap();
        let write_at = raw
            .find("pwrite64(")
            .expect("trace must include the write syscall");
        let sync_at = raw
            .find("fdatasync(")
            .expect("O_DSYNC OwnerFs write must call the file data sync boundary");
        assert!(
            sync_at > write_at,
            "O_DSYNC data sync must follow the write syscall:\n{raw}"
        );
    }

    fn run_local_odsync_write_trace_child() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let workspace = fs.mkdir(&ctx, root, OsStr::new("job-sync"), 0o755).unwrap();
        let created = fs
            .create(
                &ctx,
                workspace.inode,
                OsStr::new("log.txt"),
                0o644,
                libc::O_RDWR | libc::O_DSYNC,
            )
            .unwrap();
        assert_eq!(fs.write(&ctx, created.handle, 0, b"sync").unwrap(), 4);
        fs.release(&ctx, created.handle).unwrap();
    }

    #[test]
    fn write_sync_mode_distinguishes_odsync_from_osync() {
        assert_eq!(write_sync_mode_from_flags(0), WriteSyncMode::None);
        assert_eq!(
            write_sync_mode_from_flags(libc::O_DSYNC),
            WriteSyncMode::DataOnly
        );
        assert_eq!(
            write_sync_mode_from_flags(libc::O_SYNC),
            WriteSyncMode::Full
        );
    }

    #[test]
    fn cached_root_entry_allocates_fresh_inode_when_active_root_identity_changes() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
            value: OWNERFS_ROOT_INODE,
        };
        let first = fs
            .mkdir(&ctx, root, OsStr::new("job-root-cache"), 0o755)
            .unwrap();
        let local = fs.require_local().unwrap();
        {
            let mut state = local.state.lock().unwrap();
            let record = state.inodes.get_mut(&first.inode.value).unwrap();
            record.identity = files::FileIdentity(b"stale-root-incarnation".to_vec());
            state.rebuild_identity_index();
        }

        let refreshed = fs.lookup(&ctx, root, OsStr::new("job-root-cache")).unwrap();

        assert_ne!(refreshed.inode, first.inode);
        assert_eq!(refreshed.attributes.kind, FileKind::Directory);
    }

    #[test]
    fn freshly_created_root_getattr_uses_real_directory_identity() {
        let (_temp, fs, ctx) = fixture();
        let root = BackendInode {
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
        let ctx = test_context_for_path(temp.path());
        let root = BackendInode {
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

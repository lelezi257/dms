//! DistributedFs POSIX backend for the first local R=1 vertical slice.
//!
//! Open handles are process-local identities. Mutable file contents live in an
//! inode-level dirty view shared by every writable handle for that inode. A
//! normal `write` changes that dirty view and the local visibility sequence;
//! explicit `fdatasync`/`fsync` turns the dirty view into immutable Chunk data
//! and asks Meta to publish a new FileVersion. `flush` reports already known
//! background writeback errors, and `release` only drops the handle.

use std::{
    collections::HashMap,
    ffi::OsStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use afs_error::{Error, Result};

use super::{
    Backend,
    types::{
        BackendInode, CreatedFile, Entry, FileAttributes, FileHandle, FileKind, RequestContext,
        SyncMode,
    },
};
use crate::{
    dfs::{
        CommitFileVersion, CommitMetadataDelta, CommitMetadataMode, DfsWriteSessionId, Extent,
        FileVersion, FileVersionId, InodeAttributes, InodeId, InodeKind, InodeRecord, LayoutRoot,
        LayoutRootId, NamespaceId, OperationId, SyncInodeMetadata, WriteLease,
    },
    node::chunk::{ChunkBuilder, ChunkStore},
};

pub const DFS_WRITE_LEASE_SECONDS: u64 = 30;
const ROOT_INODE: u64 = 1;

pub trait DfsMeta: Send + Sync {
    fn lookup(&self, parent: &InodeId, name: &[u8]) -> Result<Option<InodeRecord>>;
    fn create(
        &self,
        operation_id: &OperationId,
        parent: &InodeId,
        name: &[u8],
        attributes: InodeAttributes,
    ) -> Result<(InodeRecord, WriteLease)>;
    fn get_inode(&self, inode_id: &InodeId) -> Result<InodeRecord>;
    fn get_file_version(&self, version_id: &FileVersionId) -> Result<(FileVersion, LayoutRoot)>;
    fn open_write(&self, inode_id: &InodeId) -> Result<(InodeRecord, WriteLease)>;
    fn renew_write_lease(&self, lease: WriteLease) -> Result<WriteLease>;
    fn sync_inode_metadata(&self, sync: SyncInodeMetadata) -> Result<InodeRecord>;
    fn commit_file_version(&self, commit: CommitFileVersion) -> Result<InodeRecord>;
}

type SharedInodeWriteState = Arc<Mutex<InodeWriteState>>;

pub struct DistributedFs {
    namespace_id: NamespaceId,
    node_id: String,
    session_id: String,
    meta: Arc<dyn DfsMeta>,
    chunk_store: Arc<dyn ChunkStore>,
    handles: Mutex<HashMap<u64, DfsFileHandle>>,
    inode_writes: Mutex<HashMap<InodeId, SharedInodeWriteState>>,
    inode_to_backend: Mutex<HashMap<InodeId, u64>>,
    backend_to_inode: Mutex<HashMap<u64, InodeId>>,
    next_inode: AtomicU64,
    next_handle: AtomicU64,
    next_operation: AtomicU64,
}

struct DfsFileHandle {
    inode_id: InodeId,
    opened_inode: InodeRecord,
    write_session: Option<DfsWriteSession>,
    flags: i32,
}

#[derive(Clone)]
pub struct DfsWriteSession {
    pub id: DfsWriteSessionId,
    pub inode_id: InodeId,
    pub open_flags: i32,
    pub lease_epoch: u64,
    pub last_accepted_seq: u64,
    pub last_synced_seq: u64,
    pub error_cursor: u64,
}

struct InodeWriteState {
    inode: InodeRecord,
    write_lease: WriteLease,
    base_version_id: Option<FileVersionId>,
    logical_length: u64,
    metadata_dirty: bool,
    dirty_extents: DirtyExtentMap,
    dirty: bool,
    next_write_seq: u64,
    visible_write_seq: u64,
    durable_write_seq: u64,
    committed_write_seq: u64,
    open_writers: u64,
    last_writer_background_requested: bool,
    background_error: Option<ObservedWriteError>,
}

#[derive(Clone)]
struct ObservedWriteError {
    cursor: u64,
    error: Error,
}

struct DirtyExtent {
    file_offset: u64,
    bytes: Vec<u8>,
}

struct DirtyExtentMap {
    base: Vec<u8>,
    extents: Vec<DirtyExtent>,
}

struct CommitBatch {
    through_seq: u64,
    materialized: Vec<u8>,
    commit: CommitFileVersion,
}

#[derive(Clone, Copy)]
enum CommitReason {
    DataSync,
    FullSync,
    Background,
    LastWriter,
    NodeDrain,
}

impl DistributedFs {
    pub fn new(
        namespace_id: NamespaceId,
        node_id: impl Into<String>,
        session_id: impl Into<String>,
        meta: Arc<dyn DfsMeta>,
        chunk_store: Arc<dyn ChunkStore>,
    ) -> Self {
        Self {
            namespace_id,
            node_id: node_id.into(),
            session_id: session_id.into(),
            meta,
            chunk_store,
            handles: Mutex::new(HashMap::new()),
            inode_writes: Mutex::new(HashMap::new()),
            inode_to_backend: Mutex::new(HashMap::from([(InodeId::new("1"), ROOT_INODE)])),
            backend_to_inode: Mutex::new(HashMap::from([(ROOT_INODE, InodeId::new("1"))])),
            next_inode: AtomicU64::new(ROOT_INODE + 1),
            next_handle: AtomicU64::new(1),
            next_operation: AtomicU64::new(1),
        }
    }

    fn operation_id(&self, prefix: &str) -> OperationId {
        OperationId::new(format!(
            "{}-{prefix}-{}",
            self.session_id,
            self.next_operation.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn validate_inode(&self, inode: InodeRecord) -> Result<InodeRecord> {
        if inode.namespace_id != self.namespace_id {
            return Err(invalid("Meta returned an inode from another DFS namespace"));
        }
        Ok(inode)
    }

    fn inode_size(&self, inode: &InodeRecord) -> Result<u64> {
        match inode.head_version.as_ref() {
            Some(version_id) => self
                .meta
                .get_file_version(version_id)
                .map(|(version, _)| version.length),
            None => Ok(0),
        }
    }

    fn inode_id(&self, inode: BackendInode) -> Result<InodeId> {
        self.backend_to_inode
            .lock()
            .map_err(|_| unavailable("DFS inode table is poisoned"))?
            .get(&inode.value)
            .cloned()
            .ok_or_else(|| stale("DFS mount inode is no longer known"))
    }

    fn backend_inode(&self, inode: &InodeId) -> Result<BackendInode> {
        if let Some(value) = self
            .inode_to_backend
            .lock()
            .map_err(|_| unavailable("DFS inode table is poisoned"))?
            .get(inode)
            .copied()
        {
            return Ok(BackendInode { value });
        }
        let value = self.next_inode.fetch_add(1, Ordering::Relaxed);
        self.inode_to_backend
            .lock()
            .map_err(|_| unavailable("DFS inode table is poisoned"))?
            .insert(inode.clone(), value);
        self.backend_to_inode
            .lock()
            .map_err(|_| unavailable("DFS inode table is poisoned"))?
            .insert(value, inode.clone());
        Ok(BackendInode { value })
    }

    fn allocate_handle(&self, handle: DfsFileHandle) -> Result<FileHandle> {
        let id = self.next_handle.fetch_add(1, Ordering::Relaxed);
        self.handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?
            .insert(id, handle);
        Ok(FileHandle(id))
    }

    fn load_version_bytes(&self, version_id: Option<&FileVersionId>) -> Result<Vec<u8>> {
        let Some(version_id) = version_id else {
            return Ok(Vec::new());
        };
        let (version, layout) = self.meta.get_file_version(version_id)?;
        let mut bytes =
            vec![0; usize::try_from(version.length).map_err(|_| invalid("file too large"))?];
        for extent in layout.inline_extents {
            let start = usize::try_from(extent.file_offset)
                .map_err(|_| invalid("extent offset too large"))?;
            let length =
                usize::try_from(extent.length).map_err(|_| invalid("extent length too large"))?;
            let end = start
                .checked_add(length)
                .ok_or_else(|| invalid("extent range overflow"))?;
            if end > bytes.len() {
                return Err(invalid("extent exceeds FileVersion length"));
            }
            let mut chunk_bytes = vec![0; length];
            let read = self.chunk_store.read_at(
                &extent.chunk_id,
                extent.chunk_offset,
                &mut chunk_bytes,
            )?;
            if read != length {
                return Err(Error::coded(
                    afs_error::NODE_TRANSFER_CORRUPT_DATA,
                    "chunk ended before the referenced extent",
                ));
            }
            bytes[start..end].copy_from_slice(&chunk_bytes);
        }
        Ok(bytes)
    }

    fn write_state(&self, inode_id: &InodeId) -> Result<Option<SharedInodeWriteState>> {
        Ok(self
            .inode_writes
            .lock()
            .map_err(|_| unavailable("DFS inode write table is poisoned"))?
            .get(inode_id)
            .cloned())
    }

    fn install_write_state(&self, inode: InodeRecord, write_lease: WriteLease) -> Result<()> {
        self.ensure_local_write_owner(&write_lease)?;
        if self.write_state(&inode.inode_id)?.is_some() {
            return Ok(());
        }
        let base = self.load_version_bytes(inode.head_version.as_ref())?;
        let state = Arc::new(Mutex::new(InodeWriteState {
            write_lease,
            base_version_id: inode.head_version.clone(),
            logical_length: base.len() as u64,
            metadata_dirty: false,
            dirty_extents: DirtyExtentMap::new(base),
            inode: inode.clone(),
            dirty: false,
            next_write_seq: 0,
            visible_write_seq: 0,
            durable_write_seq: 0,
            committed_write_seq: 0,
            open_writers: 0,
            last_writer_background_requested: false,
            background_error: None,
        }));
        self.inode_writes
            .lock()
            .map_err(|_| unavailable("DFS inode write table is poisoned"))?
            .entry(inode.inode_id)
            .or_insert(state);
        Ok(())
    }

    fn ensure_write_state(&self, inode: &InodeRecord, open_flags: i32) -> Result<DfsWriteSession> {
        let state = match self.write_state(&inode.inode_id)? {
            Some(state) => state,
            None => {
                let (fresh_inode, write_lease) = self.meta.open_write(&inode.inode_id)?;
                let fresh_inode = self.validate_inode(fresh_inode)?;
                self.install_write_state(fresh_inode, write_lease)?;
                self.write_state(&inode.inode_id)?
                    .ok_or_else(|| unavailable("DFS write state was not installed"))?
            }
        };
        let mut state = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        if should_renew(&state.write_lease) {
            state.write_lease = self.meta.renew_write_lease(state.write_lease.clone())?;
        }
        state.open_writers = state.open_writers.saturating_add(1);
        state.last_writer_background_requested = false;
        let session_id = DfsWriteSessionId::new(format!(
            "{}-write-session-{}",
            self.session_id,
            self.next_operation.fetch_add(1, Ordering::Relaxed)
        ));
        Ok(write_session(&state, open_flags, session_id))
    }

    fn visible_size(&self, inode: &InodeRecord) -> Result<u64> {
        let Some(state) = self.write_state(&inode.inode_id)? else {
            return self.inode_size(inode);
        };
        let state = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        Ok(state.logical_length)
    }

    fn current_bytes(
        &self,
        inode_id: &InodeId,
        committed: Option<&FileVersionId>,
    ) -> Result<Vec<u8>> {
        if let Some(state) = self.write_state(inode_id)? {
            let state = state
                .lock()
                .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
            return state.dirty_extents.materialize(state.logical_length);
        }
        self.load_version_bytes(committed)
    }

    fn handle_snapshot(&self, handle: FileHandle) -> Result<DfsFileHandleSnapshot> {
        self.handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?
            .get(&handle.0)
            .map(DfsFileHandleSnapshot::from)
            .ok_or_else(|| stale("DFS file handle is no longer open"))
    }

    fn commit_handle(&self, handle: FileHandle, reason: CommitReason) -> Result<()> {
        self.observe_handle_error(handle)?;
        let snapshot = self.handle_snapshot(handle)?;
        let committed_seq = self.commit_inode(&snapshot.inode_id, reason)?;
        if let Some(committed_seq) = committed_seq {
            self.update_handles_after_commit(&snapshot.inode_id, committed_seq)?;
        }
        Ok(())
    }

    fn commit_inode(&self, inode_id: &InodeId, reason: CommitReason) -> Result<Option<u64>> {
        let Some(state) = self.write_state(inode_id)? else {
            return Ok(None);
        };
        let mut state = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        if !(state.dirty || matches!(reason, CommitReason::FullSync) && state.metadata_dirty) {
            return Ok(None);
        }
        if should_renew(&state.write_lease) {
            state.write_lease = self.meta.renew_write_lease(state.write_lease.clone())?;
        }
        if !state.dirty {
            let now = now_unix_ms();
            let updated =
                self.validate_inode(self.meta.sync_inode_metadata(SyncInodeMetadata {
                    operation_id: self.operation_id("fsync-metadata"),
                    inode_id: state.inode.inode_id.clone(),
                    write_lease: state.write_lease.clone(),
                    expected_inode_revision: state.inode.revision,
                    expected_head_version: state.base_version_id.clone(),
                    metadata_delta: CommitMetadataDelta {
                        mode: CommitMetadataMode::Full,
                        mtime_unix_ms: Some(now),
                        ctime_unix_ms: Some(now),
                    },
                })?)?;
            state.inode = updated;
            state.metadata_dirty = false;
            return Ok(Some(state.committed_write_seq));
        }
        let batch = self.prepare_commit(&mut state, reason)?;
        let committed_full_metadata = batch.commit.metadata_delta.mode == CommitMetadataMode::Full;
        let updated = self.validate_inode(self.meta.commit_file_version(batch.commit)?)?;
        state.inode = updated.clone();
        state.base_version_id = updated.head_version;
        state.logical_length = batch.materialized.len() as u64;
        state.dirty_extents = DirtyExtentMap::new(batch.materialized);
        state.dirty = false;
        state.durable_write_seq = batch.through_seq;
        state.committed_write_seq = batch.through_seq;
        state.last_writer_background_requested = false;
        if committed_full_metadata {
            state.metadata_dirty = false;
        }
        Ok(Some(batch.through_seq))
    }

    fn prepare_commit(
        &self,
        state: &mut InodeWriteState,
        reason: CommitReason,
    ) -> Result<CommitBatch> {
        let materialized = state.dirty_extents.materialize(state.logical_length)?;
        let mut builder = ChunkBuilder::default();
        builder.replace(materialized.clone());
        let operation_id = self.operation_id(reason.operation_prefix());
        let receipt = (!materialized.is_empty())
            .then(|| self.chunk_store.put(builder.stage(operation_id.clone())))
            .transpose()?;
        let generation = self.next_operation.fetch_add(1, Ordering::Relaxed);
        let layout = LayoutRoot {
            id: LayoutRootId::new(format!("{}-layout-{generation}", self.session_id)),
            file_length: state.logical_length,
            inline_extents: if state.logical_length == 0 {
                Vec::new()
            } else {
                vec![Extent {
                    file_offset: 0,
                    length: state.logical_length,
                    chunk_id: receipt
                        .as_ref()
                        .expect("non-empty materialized data has a receipt")
                        .chunk
                        .id
                        .clone(),
                    chunk_offset: 0,
                }]
            },
        };
        let now = now_unix_ms();
        let version = FileVersion {
            id: FileVersionId::new(format!("{}-version-{generation}", self.session_id)),
            inode_id: state.inode.inode_id.clone(),
            parent_version: state.base_version_id.clone(),
            length: state.logical_length,
            layout_root: layout.id.clone(),
            created_at_unix_ms: now,
        };
        Ok(CommitBatch {
            through_seq: state.visible_write_seq,
            materialized,
            commit: CommitFileVersion {
                operation_id,
                inode_id: state.inode.inode_id.clone(),
                write_lease: state.write_lease.clone(),
                expected_inode_revision: state.inode.revision,
                expected_head_version: state.base_version_id.clone(),
                file_version: version,
                layout_root: layout,
                chunk_receipts: receipt.into_iter().collect(),
                metadata_delta: reason.metadata_delta(now),
            },
        })
    }

    fn observe_handle_error(&self, handle: FileHandle) -> Result<()> {
        let snapshot = self.handle_snapshot(handle)?;
        let Some(session) = snapshot.write_session else {
            return Ok(());
        };
        let Some(state) = self.write_state(&snapshot.inode_id)? else {
            return Ok(());
        };
        let observed = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?
            .background_error
            .clone();
        let Some(observed) = observed.filter(|error| error.cursor > session.error_cursor) else {
            return Ok(());
        };
        self.update_handle_error_cursor(handle, observed.cursor)?;
        Err(observed.error)
    }

    fn truncate_dirty_inode(&self, inode_id: &InodeId, length: u64) -> Result<()> {
        let state = self
            .write_state(inode_id)?
            .ok_or_else(|| stale("DFS write state is no longer open"))?;
        let mut state = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        state.dirty_extents.truncate(length)?;
        state.logical_length = length;
        state.metadata_dirty = true;
        state.next_write_seq = state.next_write_seq.saturating_add(1);
        state.visible_write_seq = state.next_write_seq;
        state.dirty = true;
        Ok(())
    }

    fn release_writer(&self, session: &DfsWriteSession) -> Result<()> {
        let Some(state) = self.write_state(&session.inode_id)? else {
            return Ok(());
        };
        let mut state = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        state.open_writers = state.open_writers.saturating_sub(1);
        if state.open_writers == 0 && state.dirty {
            state.last_writer_background_requested = true;
        }
        Ok(())
    }

    fn ensure_local_write_owner(&self, lease: &WriteLease) -> Result<()> {
        if lease.owner_node_id != self.node_id || lease.owner_session_id != self.session_id {
            return Err(Error::coded(
                afs_error::NODE_VFS_UNIMPLEMENTED,
                "DFS remote write owner routing is not implemented in this slice",
            ));
        }
        Ok(())
    }

    /// Execute pending background commits without changing the user-visible
    /// close contract. Failures remain attached to the inode and are reported
    /// once per open writer by a later write/flush/sync operation.
    pub fn writeback_pending(&self) -> Result<usize> {
        let candidates = self
            .inode_writes
            .lock()
            .map_err(|_| unavailable("DFS inode write table is poisoned"))?
            .iter()
            .filter_map(|(inode_id, state)| {
                let state = state.lock().ok()?;
                state.dirty.then(|| {
                    (
                        inode_id.clone(),
                        if state.last_writer_background_requested {
                            CommitReason::LastWriter
                        } else {
                            CommitReason::Background
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        let mut committed = 0;
        for (inode_id, reason) in candidates {
            match self.commit_inode(&inode_id, reason) {
                Ok(Some(seq)) => {
                    self.update_handles_after_commit(&inode_id, seq)?;
                    committed += 1;
                }
                Ok(None) => {}
                Err(error) => self.record_background_error(&inode_id, error)?,
            }
        }
        Ok(committed)
    }

    /// Best-effort graceful drain. It gives a clean Node shutdown a chance to
    /// publish dirty state, while an abrupt crash is still allowed to recover
    /// only the last committed FileVersion.
    pub fn drain(&self) -> Result<usize> {
        let inode_ids = self
            .inode_writes
            .lock()
            .map_err(|_| unavailable("DFS inode write table is poisoned"))?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let mut committed = 0;
        for inode_id in inode_ids {
            if let Some(seq) = self.commit_inode(&inode_id, CommitReason::NodeDrain)? {
                self.update_handles_after_commit(&inode_id, seq)?;
                committed += 1;
            }
        }
        Ok(committed)
    }

    fn record_background_error(&self, inode_id: &InodeId, error: Error) -> Result<()> {
        let Some(state) = self.write_state(inode_id)? else {
            return Ok(());
        };
        let mut state = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        let cursor = state
            .background_error
            .as_ref()
            .map_or(1, |previous| previous.cursor.saturating_add(1));
        state.background_error = Some(ObservedWriteError { cursor, error });
        Ok(())
    }
}

#[derive(Clone)]
struct DfsFileHandleSnapshot {
    inode_id: InodeId,
    opened_inode: InodeRecord,
    write_session: Option<DfsWriteSession>,
    flags: i32,
}

impl From<&DfsFileHandle> for DfsFileHandleSnapshot {
    fn from(handle: &DfsFileHandle) -> Self {
        Self {
            inode_id: handle.inode_id.clone(),
            opened_inode: handle.opened_inode.clone(),
            write_session: handle.write_session.clone(),
            flags: handle.flags,
        }
    }
}

impl DirtyExtentMap {
    fn new(base: Vec<u8>) -> Self {
        Self {
            base,
            extents: Vec::new(),
        }
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<usize> {
        let _ = usize::try_from(offset).map_err(|_| invalid("write offset is too large"))?;
        if !data.is_empty() {
            self.extents.push(DirtyExtent {
                file_offset: offset,
                bytes: data.to_vec(),
            });
        }
        Ok(data.len())
    }

    fn truncate(&mut self, length: u64) -> Result<()> {
        let length = usize::try_from(length).map_err(|_| invalid("file length is too large"))?;
        let mut materialized = self.materialize(length as u64)?;
        materialized.resize(length, 0);
        self.base = materialized;
        self.extents.clear();
        Ok(())
    }

    fn materialize(&self, length: u64) -> Result<Vec<u8>> {
        let target_len =
            usize::try_from(length).map_err(|_| invalid("file length is too large"))?;
        let mut bytes = self.base.clone();
        bytes.resize(target_len, 0);
        for extent in &self.extents {
            let start = usize::try_from(extent.file_offset)
                .map_err(|_| invalid("dirty extent offset is too large"))?;
            let end = start
                .checked_add(extent.bytes.len())
                .ok_or_else(|| invalid("dirty extent range overflow"))?;
            if end > bytes.len() {
                bytes.resize(end, 0);
            }
            bytes[start..end].copy_from_slice(&extent.bytes);
        }
        bytes.truncate(target_len);
        Ok(bytes)
    }
}

impl CommitReason {
    fn operation_prefix(self) -> &'static str {
        match self {
            Self::DataSync => "fdatasync",
            Self::FullSync => "fsync",
            Self::Background => "background",
            Self::LastWriter => "last-writer",
            Self::NodeDrain => "node-drain",
        }
    }

    fn metadata_delta(self, now: u64) -> CommitMetadataDelta {
        if !matches!(self, Self::DataSync) {
            CommitMetadataDelta {
                mode: CommitMetadataMode::Full,
                mtime_unix_ms: Some(now),
                ctime_unix_ms: Some(now),
            }
        } else {
            CommitMetadataDelta {
                mode: CommitMetadataMode::DataOnly,
                mtime_unix_ms: None,
                ctime_unix_ms: None,
            }
        }
    }
}

impl Backend for DistributedFs {
    fn root_inode(&self) -> BackendInode {
        BackendInode { value: ROOT_INODE }
    }

    fn lookup(&self, _: &RequestContext, parent: BackendInode, name: &OsStr) -> Result<Entry> {
        let inode = self.validate_inode(
            self.meta
                .lookup(&self.inode_id(parent)?, name.as_encoded_bytes())?
                .ok_or_else(|| {
                    Error::coded(afs_error::NODE_VFS_NOT_FOUND, "DFS dentry not found")
                })?,
        )?;
        let size = self.visible_size(&inode)?;
        Ok(Entry {
            inode: self.backend_inode(&inode.inode_id)?,
            attributes: attributes(&inode, size),
        })
    }

    fn getattr(
        &self,
        _: &RequestContext,
        inode: BackendInode,
        handle: Option<FileHandle>,
    ) -> Result<FileAttributes> {
        if let Some(handle) = handle {
            let snapshot = self.handle_snapshot(handle)?;
            let size = self.visible_size(&snapshot.opened_inode)?;
            return Ok(attributes(&snapshot.opened_inode, size));
        }
        let record = self.validate_inode(self.meta.get_inode(&self.inode_id(inode)?)?)?;
        let size = self.visible_size(&record)?;
        Ok(attributes(&record, size))
    }

    fn create(
        &self,
        ctx: &RequestContext,
        parent: BackendInode,
        name: &OsStr,
        mode: u32,
        flags: i32,
    ) -> Result<CreatedFile> {
        let now = now_unix_ms();
        let (inode, write_lease) = self.meta.create(
            &self.operation_id("create"),
            &self.inode_id(parent)?,
            name.as_encoded_bytes(),
            InodeAttributes {
                mode: mode & !ctx.umask,
                uid: ctx.uid,
                gid: ctx.gid,
                nlink: 1,
                atime_unix_ms: now,
                mtime_unix_ms: now,
                ctime_unix_ms: now,
            },
        )?;
        let inode = self.validate_inode(inode)?;
        self.install_write_state(inode.clone(), write_lease)?;
        let session = self.ensure_write_state(&inode, flags)?;
        let handle = self.allocate_handle(DfsFileHandle {
            inode_id: inode.inode_id.clone(),
            opened_inode: inode.clone(),
            write_session: Some(session),
            flags,
        })?;
        Ok(CreatedFile {
            entry: Entry {
                inode: self.backend_inode(&inode.inode_id)?,
                attributes: attributes(&inode, 0),
            },
            handle,
        })
    }

    fn open(&self, _: &RequestContext, inode: BackendInode, flags: i32) -> Result<FileHandle> {
        let inode_id = self.inode_id(inode)?;
        let writable = flags & libc::O_ACCMODE != libc::O_RDONLY;
        let (opened_inode, write_session) = if writable {
            let (inode, write_lease) = self.meta.open_write(&inode_id)?;
            let inode = self.validate_inode(inode)?;
            self.install_write_state(inode.clone(), write_lease)?;
            if flags & libc::O_TRUNC != 0 {
                self.truncate_dirty_inode(&inode.inode_id, 0)?;
            }
            let session = self.ensure_write_state(&inode, flags)?;
            (inode, Some(session))
        } else {
            (self.validate_inode(self.meta.get_inode(&inode_id)?)?, None)
        };
        self.allocate_handle(DfsFileHandle {
            inode_id: opened_inode.inode_id.clone(),
            opened_inode,
            write_session,
            flags,
        })
    }

    fn read(
        &self,
        _: &RequestContext,
        handle: FileHandle,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize> {
        let snapshot = self.handle_snapshot(handle)?;
        let bytes = self.current_bytes(
            &snapshot.inode_id,
            snapshot.opened_inode.head_version.as_ref(),
        )?;
        let start = usize::try_from(offset).map_err(|_| invalid("read offset too large"))?;
        if start >= bytes.len() {
            return Ok(0);
        }
        let count = out.len().min(bytes.len() - start);
        out[..count].copy_from_slice(&bytes[start..start + count]);
        Ok(count)
    }

    fn write(
        &self,
        _: &RequestContext,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<usize> {
        self.observe_handle_error(handle)?;
        let snapshot = self.handle_snapshot(handle)?;
        if snapshot.flags & libc::O_ACCMODE == libc::O_RDONLY {
            return Err(Error::from(std::io::Error::from_raw_os_error(libc::EBADF)));
        }
        if snapshot.write_session.is_none() {
            return Err(invalid("writable DFS handle has no write session"));
        }
        let state = self
            .write_state(&snapshot.inode_id)?
            .ok_or_else(|| stale("DFS write state is no longer open"))?;
        let (written, accepted_seq) = {
            let mut state = state
                .lock()
                .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
            let offset = if snapshot.flags & libc::O_APPEND != 0 {
                state.logical_length
            } else {
                offset
            };
            let written = state.dirty_extents.write_at(offset, data)?;
            state.logical_length = state
                .logical_length
                .max(offset.saturating_add(written as u64));
            if written > 0 {
                state.next_write_seq = state.next_write_seq.saturating_add(1);
                state.visible_write_seq = state.next_write_seq;
                state.dirty = true;
                state.metadata_dirty = true;
            }
            (written, state.visible_write_seq)
        };
        self.update_handle_write_progress(handle, accepted_seq)?;
        if snapshot.flags & libc::O_SYNC != 0 {
            self.commit_handle(handle, CommitReason::FullSync)?;
        } else if has_o_dsync(snapshot.flags) {
            self.commit_handle(handle, CommitReason::DataSync)?;
        }
        Ok(written)
    }

    fn flush(&self, _: &RequestContext, handle: FileHandle) -> Result<()> {
        self.observe_handle_error(handle)
    }

    fn fsync(&self, _: &RequestContext, handle: FileHandle, mode: SyncMode) -> Result<()> {
        self.commit_handle(
            handle,
            match mode {
                SyncMode::DataOnly => CommitReason::DataSync,
                SyncMode::Full => CommitReason::FullSync,
            },
        )
    }

    fn release(&self, _: &RequestContext, handle: FileHandle) -> Result<()> {
        let removed = self
            .handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?
            .remove(&handle.0)
            .ok_or_else(|| stale("DFS file handle is no longer open"))?;
        if let Some(session) = removed.write_session.as_ref() {
            self.release_writer(session)?;
        }
        Ok(())
    }
}

impl DistributedFs {
    fn update_handle_write_progress(&self, handle: FileHandle, accepted_seq: u64) -> Result<()> {
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?;
        let file = handles
            .get_mut(&handle.0)
            .ok_or_else(|| stale("DFS file handle is no longer open"))?;
        if let Some(session) = file.write_session.as_mut() {
            session.last_accepted_seq = accepted_seq;
        }
        Ok(())
    }

    fn update_handle_error_cursor(&self, handle: FileHandle, cursor: u64) -> Result<()> {
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?;
        let file = handles
            .get_mut(&handle.0)
            .ok_or_else(|| stale("DFS file handle is no longer open"))?;
        if let Some(session) = file.write_session.as_mut() {
            session.error_cursor = cursor;
        }
        Ok(())
    }

    fn update_handles_after_commit(&self, inode_id: &InodeId, committed_seq: u64) -> Result<()> {
        let inode = self
            .write_state(inode_id)?
            .ok_or_else(|| stale("DFS write state is no longer open"))?
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?
            .inode
            .clone();
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?;
        for handle in handles
            .values_mut()
            .filter(|handle| handle.inode_id == *inode_id)
        {
            handle.opened_inode = inode.clone();
            if let Some(session) = handle.write_session.as_mut() {
                session.last_synced_seq = session.last_synced_seq.max(committed_seq);
            }
        }
        Ok(())
    }
}

fn write_session(
    state: &InodeWriteState,
    open_flags: i32,
    id: DfsWriteSessionId,
) -> DfsWriteSession {
    DfsWriteSession {
        id,
        inode_id: state.inode.inode_id.clone(),
        open_flags,
        lease_epoch: state.write_lease.lease_epoch,
        last_accepted_seq: state.visible_write_seq,
        last_synced_seq: state.committed_write_seq,
        error_cursor: state
            .background_error
            .as_ref()
            .map_or(0, |error| error.cursor),
    }
}

fn should_renew(lease: &WriteLease) -> bool {
    lease.expires_at_unix_ms <= now_unix_ms().saturating_add(5_000)
}

fn has_o_dsync(flags: i32) -> bool {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        flags & libc::O_DSYNC != 0
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = flags;
        false
    }
}

fn attributes(inode: &InodeRecord, size: u64) -> FileAttributes {
    let attrs = &inode.attributes;
    FileAttributes {
        kind: match inode.kind {
            InodeKind::Regular => FileKind::Regular,
            InodeKind::Directory => FileKind::Directory,
            InodeKind::Symlink => FileKind::Symlink,
        },
        size,
        mode: attrs.mode,
        uid: attrs.uid,
        gid: attrs.gid,
        nlink: attrs.nlink,
        atime: unix_ms(attrs.atime_unix_ms),
        mtime: unix_ms(attrs.mtime_unix_ms),
        ctime: unix_ms(attrs.ctime_unix_ms),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn unix_ms(value: u64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_millis(value)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_VFS_INVALID, message)
}

fn unavailable(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_VFS_UNAVAILABLE, message)
}

fn stale(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_DFS_STALE_HANDLE, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::chunk::LocalChunkStore;

    struct RecordingMeta {
        inode: Mutex<InodeRecord>,
        lease: Mutex<WriteLease>,
        commits: Mutex<Vec<CommitFileVersion>>,
    }

    impl RecordingMeta {
        fn new() -> Self {
            Self {
                inode: Mutex::new(InodeRecord {
                    namespace_id: NamespaceId::new("default"),
                    inode_id: InodeId::new("inode:test"),
                    kind: InodeKind::Regular,
                    attributes: InodeAttributes {
                        mode: 0o640,
                        uid: 1000,
                        gid: 1000,
                        nlink: 1,
                        atime_unix_ms: 1,
                        mtime_unix_ms: 1,
                        ctime_unix_ms: 1,
                    },
                    head_version: None,
                    revision: 1,
                }),
                lease: Mutex::new(WriteLease {
                    inode_id: InodeId::new("inode:test"),
                    owner_node_id: "node-a".into(),
                    owner_session_id: "session-a".into(),
                    lease_epoch: 1,
                    expires_at_unix_ms: u64::MAX,
                }),
                commits: Mutex::new(Vec::new()),
            }
        }

        fn commit_count(&self) -> usize {
            self.commits.lock().unwrap().len()
        }
    }

    impl DfsMeta for RecordingMeta {
        fn lookup(&self, _: &InodeId, _: &[u8]) -> Result<Option<InodeRecord>> {
            Ok(Some(self.inode.lock().unwrap().clone()))
        }

        fn create(
            &self,
            _: &OperationId,
            _: &InodeId,
            _: &[u8],
            _: InodeAttributes,
        ) -> Result<(InodeRecord, WriteLease)> {
            Ok((
                self.inode.lock().unwrap().clone(),
                self.lease.lock().unwrap().clone(),
            ))
        }

        fn get_inode(&self, _: &InodeId) -> Result<InodeRecord> {
            Ok(self.inode.lock().unwrap().clone())
        }

        fn get_file_version(
            &self,
            version_id: &FileVersionId,
        ) -> Result<(FileVersion, LayoutRoot)> {
            self.commits
                .lock()
                .unwrap()
                .iter()
                .find(|commit| commit.file_version.id == *version_id)
                .map(|commit| (commit.file_version.clone(), commit.layout_root.clone()))
                .ok_or_else(|| invalid("test FileVersion not found"))
        }

        fn open_write(&self, _: &InodeId) -> Result<(InodeRecord, WriteLease)> {
            Ok((
                self.inode.lock().unwrap().clone(),
                self.lease.lock().unwrap().clone(),
            ))
        }

        fn renew_write_lease(&self, lease: WriteLease) -> Result<WriteLease> {
            *self.lease.lock().unwrap() = lease.clone();
            Ok(lease)
        }

        fn sync_inode_metadata(&self, sync: SyncInodeMetadata) -> Result<InodeRecord> {
            let mut inode = self.inode.lock().unwrap();
            if inode.revision != sync.expected_inode_revision
                || inode.head_version != sync.expected_head_version
            {
                return Err(unavailable("test inode metadata CAS failed"));
            }
            inode.revision = inode.revision.saturating_add(1);
            inode.attributes.mtime_unix_ms = sync.metadata_delta.mtime_unix_ms.unwrap();
            inode.attributes.ctime_unix_ms = sync.metadata_delta.ctime_unix_ms.unwrap();
            Ok(inode.clone())
        }

        fn commit_file_version(&self, commit: CommitFileVersion) -> Result<InodeRecord> {
            let mut inode = self.inode.lock().unwrap();
            if inode.revision != commit.expected_inode_revision
                || inode.head_version != commit.expected_head_version
            {
                return Err(unavailable("test inode CAS failed"));
            }
            inode.revision = inode.revision.saturating_add(1);
            inode.head_version = Some(commit.file_version.id.clone());
            if commit.metadata_delta.mode == CommitMetadataMode::Full {
                inode.attributes.mtime_unix_ms = commit.metadata_delta.mtime_unix_ms.unwrap();
                inode.attributes.ctime_unix_ms = commit.metadata_delta.ctime_unix_ms.unwrap();
            }
            self.commits.lock().unwrap().push(commit);
            Ok(inode.clone())
        }
    }

    fn context() -> RequestContext {
        RequestContext {
            uid: 1000,
            gid: 1000,
            pid: 42,
            umask: 0,
        }
    }

    #[test]
    fn dirty_view_is_inode_shared_and_only_sync_commits_a_version() {
        let meta = Arc::new(RecordingMeta::new());
        let temp = tempfile::tempdir().unwrap();
        let chunks = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
        let fs = DistributedFs::new(
            NamespaceId::new("default"),
            "node-a",
            "session-a",
            meta.clone(),
            chunks,
        );
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("hello.txt"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();

        assert_eq!(
            fs.write(&context(), created.handle, 0, b"hello").unwrap(),
            5
        );
        fs.flush(&context(), created.handle).unwrap();
        assert_eq!(meta.commit_count(), 0, "flush must not create FileVersion");

        let reader = fs
            .open(&context(), created.entry.inode, libc::O_RDONLY)
            .unwrap();
        let mut out = [0; 5];
        assert_eq!(fs.read(&context(), reader, 0, &mut out).unwrap(), 5);
        assert_eq!(&out, b"hello", "another handle must read the dirty overlay");

        let writer = fs
            .open(&context(), created.entry.inode, libc::O_RDWR)
            .unwrap();
        fs.release(&context(), created.handle).unwrap();
        assert_eq!(
            meta.commit_count(),
            0,
            "release must not synchronously commit"
        );

        fs.fsync(&context(), writer, SyncMode::DataOnly).unwrap();
        assert_eq!(meta.commit_count(), 1);
        assert_eq!(
            meta.commits.lock().unwrap()[0].metadata_delta.mode,
            CommitMetadataMode::DataOnly
        );
        let data_version = meta.inode.lock().unwrap().head_version.clone();
        fs.fsync(&context(), writer, SyncMode::Full).unwrap();
        assert_eq!(
            meta.commit_count(),
            1,
            "fsync after fdatasync must sync inode metadata without inventing another FileVersion"
        );
        assert_eq!(meta.inode.lock().unwrap().head_version, data_version);

        fs.write(&context(), writer, 0, b"H").unwrap();
        fs.fsync(&context(), writer, SyncMode::Full).unwrap();
        let commits = meta.commits.lock().unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[1].metadata_delta.mode, CommitMetadataMode::Full);
        assert_eq!(
            commits[1].file_version.parent_version,
            Some(commits[0].file_version.id.clone())
        );
        drop(commits);

        let mut latest = [0; 5];
        assert_eq!(fs.read(&context(), reader, 0, &mut latest).unwrap(), 5);
        assert_eq!(&latest, b"Hello");
        fs.release(&context(), writer).unwrap();
        fs.release(&context(), reader).unwrap();
        assert_eq!(fs.writeback_pending().unwrap(), 0);
    }
}

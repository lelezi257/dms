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
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use afs_error::{Error, Result};

use super::{
    Backend,
    types::{
        AttributeChange, BackendInode, CreatedFile, Entry, FileAttributes, FileHandle, FileKind,
        RequestContext, SyncMode,
    },
};
use crate::{
    dfs::{
        CommitFileVersion, CommitMetadataDelta, CommitMetadataMode, DfsWriteSessionId, Extent,
        FileVersion, FileVersionId, InodeAttributes, InodeId, InodeKind, InodeRecord, LayoutRoot,
        LayoutRootId, NamespaceId, OperationId, SyncInodeMetadata, WriteLease,
    },
    node::chunk::{ChunkBuilder, ChunkStore, StagedChunk},
    node::dfs_read::{ChunkReadOp, DfsReadEngine, ReadBatch},
};

pub const DFS_WRITE_LEASE_SECONDS: u64 = 30;
const ROOT_INODE: u64 = 1;
const COMMIT_CHUNK_BYTES: u64 = 4 * 1024 * 1024;

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

type SharedInodeWriteState = Arc<InodeWriteStateCell>;

struct InodeWriteStateCell {
    state: Mutex<InodeWriteState>,
    cv: Condvar,
}

pub struct DistributedFs {
    namespace_id: NamespaceId,
    node_id: String,
    session_id: String,
    meta: Arc<dyn DfsMeta>,
    chunk_store: Arc<dyn ChunkStore>,
    read_engine: Arc<DfsReadEngine>,
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
    opened_version: Option<OpenedFileVersion>,
    write_session: Option<DfsWriteSession>,
    flags: i32,
}

#[derive(Clone)]
struct OpenedFileVersion {
    version: Option<FileVersion>,
    layout: LayoutRoot,
    length: u64,
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
    base_version: Option<FileVersion>,
    base_layout: LayoutRoot,
    logical_length: u64,
    metadata_dirty: bool,
    dirty_extents: DirtyExtentMap,
    in_flight: Option<InFlightCommit>,
    commit_busy: bool,
    dirty: bool,
    next_write_seq: u64,
    visible_write_seq: u64,
    durable_write_seq: u64,
    committed_write_seq: u64,
    open_writers: u64,
    last_writer_background_requested: bool,
    background_error: Option<ObservedWriteError>,
    terminal_error: Option<Error>,
}

#[derive(Clone)]
struct ObservedWriteError {
    cursor: u64,
    error: Error,
}

#[derive(Clone)]
struct DirtyExtent {
    file_offset: u64,
    length: u64,
    write_seq: u64,
    data: Option<Arc<[u8]>>,
}

#[derive(Clone, Default)]
struct DirtyExtentMap {
    extents: Vec<DirtyExtent>,
}

#[derive(Clone)]
struct FrozenCommit {
    through_seq: u64,
    logical_length: u64,
    inode: InodeRecord,
    write_lease: WriteLease,
    base_version: Option<FileVersion>,
    base_layout: LayoutRoot,
    dirty_extents: DirtyExtentMap,
}

#[derive(Clone)]
enum InFlightCommit {
    Preparing(Box<FrozenCommit>),
    File(Box<PendingFileCommit>),
    Metadata(SyncInodeMetadata),
}

#[derive(Clone)]
struct PendingFileCommit {
    frozen: FrozenCommit,
    batch: CommitBatch,
}

struct CommitPlan {
    layout_root: LayoutRoot,
    staged_chunks: Vec<StagedChunk>,
}

#[derive(Clone)]
struct OverlaySegment {
    file_offset: u64,
    length: u64,
    data: Option<(Arc<[u8]>, usize)>,
}

#[derive(Clone)]
struct CommitBatch {
    through_seq: u64,
    commit: CommitFileVersion,
}

enum AppliedCommit {
    File {
        through_seq: u64,
        committed_full_metadata: bool,
        version: FileVersion,
        layout: LayoutRoot,
        updated: InodeRecord,
    },
    Metadata {
        updated: InodeRecord,
    },
}

#[derive(Clone, Copy)]
enum CommitReason {
    DataSync,
    FullSync,
    Background,
    LastWriter,
    NodeDrain,
}

impl InFlightCommit {
    fn dirty_extents(&self) -> Option<DirtyExtentMap> {
        match self {
            InFlightCommit::Preparing(frozen) => Some(frozen.dirty_extents.clone()),
            InFlightCommit::File(pending) => Some(pending.frozen.dirty_extents.clone()),
            InFlightCommit::Metadata(_) => None,
        }
    }
}

impl InodeWriteStateCell {
    fn new(state: InodeWriteState) -> Self {
        Self {
            state: Mutex::new(state),
            cv: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, InodeWriteState>> {
        self.state.lock()
    }

    fn wait<'a>(
        &self,
        guard: std::sync::MutexGuard<'a, InodeWriteState>,
    ) -> std::sync::LockResult<std::sync::MutexGuard<'a, InodeWriteState>> {
        self.cv.wait(guard)
    }

    fn notify_all(&self) {
        self.cv.notify_all();
    }
}

impl DistributedFs {
    pub fn new(
        namespace_id: NamespaceId,
        node_id: impl Into<String>,
        session_id: impl Into<String>,
        meta: Arc<dyn DfsMeta>,
        chunk_store: Arc<dyn ChunkStore>,
        read_engine: Arc<DfsReadEngine>,
    ) -> Self {
        Self {
            namespace_id,
            node_id: node_id.into(),
            session_id: session_id.into(),
            meta,
            chunk_store,
            read_engine,
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

    fn load_version(
        &self,
        version_id: Option<&FileVersionId>,
    ) -> Result<(Option<FileVersion>, LayoutRoot)> {
        let Some(version_id) = version_id else {
            return Ok((
                None,
                LayoutRoot {
                    id: LayoutRootId::new("empty"),
                    file_length: 0,
                    inline_extents: Vec::new(),
                },
            ));
        };
        self.meta
            .get_file_version(version_id)
            .map(|(version, layout)| (Some(version), layout))
    }

    fn opened_version(&self, inode: &InodeRecord) -> Result<OpenedFileVersion> {
        let (version, layout) = self.load_version(inode.head_version.as_ref())?;
        let length = version.as_ref().map_or(0, |version| version.length);
        Ok(OpenedFileVersion {
            version,
            layout,
            length,
        })
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
        let (base_version, base_layout) = self.load_version(inode.head_version.as_ref())?;
        let logical_length = base_version.as_ref().map_or(0, |version| version.length);
        let state = Arc::new(InodeWriteStateCell::new(InodeWriteState {
            write_lease,
            base_version,
            base_layout,
            logical_length,
            metadata_dirty: false,
            dirty_extents: DirtyExtentMap::default(),
            in_flight: None,
            commit_busy: false,
            inode: inode.clone(),
            dirty: false,
            next_write_seq: 0,
            visible_write_seq: 0,
            durable_write_seq: 0,
            committed_write_seq: 0,
            open_writers: 0,
            last_writer_background_requested: false,
            background_error: None,
            terminal_error: None,
        }));
        self.inode_writes
            .lock()
            .map_err(|_| unavailable("DFS inode write table is poisoned"))?
            .entry(inode.inode_id)
            .or_insert(state);
        Ok(())
    }

    fn ensure_inode_write_state(&self, inode: &InodeRecord) -> Result<SharedInodeWriteState> {
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
        Ok(state)
    }

    fn open_write_session(&self, inode: &InodeRecord, open_flags: i32) -> Result<DfsWriteSession> {
        let state = self.ensure_inode_write_state(inode)?;
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

    fn visible_attributes(&self, inode: &InodeRecord) -> Result<FileAttributes> {
        let Some(state) = self.write_state(&inode.inode_id)? else {
            return Ok(attributes(inode, self.inode_size(inode)?));
        };
        let state = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        Ok(attributes(&state.inode, state.logical_length))
    }

    fn read_visible(
        &self,
        inode_id: &InodeId,
        committed: Option<&FileVersionId>,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize> {
        let (length, layout, frozen, active) = if let Some(state) = self.write_state(inode_id)? {
            let state = state
                .lock()
                .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
            (
                state.logical_length,
                state.base_layout.clone(),
                state
                    .in_flight
                    .as_ref()
                    .and_then(InFlightCommit::dirty_extents),
                state.dirty_extents.clone(),
            )
        } else {
            let (version, layout) = self.load_version(committed)?;
            (
                version.as_ref().map_or(0, |value| value.length),
                layout,
                None,
                DirtyExtentMap::default(),
            )
        };
        if offset >= length || out.is_empty() {
            return Ok(0);
        }
        let count = usize::try_from((length - offset).min(out.len() as u64))
            .map_err(|_| invalid("read length is too large"))?;
        out[..count].fill(0);
        self.read_layout_range(committed, &layout, offset, &mut out[..count])?;
        if let Some(frozen) = frozen {
            frozen.overlay(offset, &mut out[..count])?;
        }
        active.overlay(offset, &mut out[..count])?;
        Ok(count)
    }

    fn read_opened_version(
        &self,
        opened: &OpenedFileVersion,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize> {
        if offset >= opened.length || out.is_empty() {
            return Ok(0);
        }
        let count = usize::try_from((opened.length - offset).min(out.len() as u64))
            .map_err(|_| invalid("read length is too large"))?;
        out[..count].fill(0);
        self.read_layout_range(
            opened.version.as_ref().map(|version| &version.id),
            &opened.layout,
            offset,
            &mut out[..count],
        )?;
        Ok(count)
    }

    fn read_layout_range(
        &self,
        committed: Option<&FileVersionId>,
        layout: &LayoutRoot,
        offset: u64,
        out: &mut [u8],
    ) -> Result<()> {
        let end = offset
            .checked_add(out.len() as u64)
            .ok_or_else(|| invalid("read range overflow"))?;
        let mut ops = Vec::new();
        for extent in &layout.inline_extents {
            let extent_end = extent
                .file_offset
                .checked_add(extent.length)
                .ok_or_else(|| invalid("extent range overflow"))?;
            let start = offset.max(extent.file_offset);
            let stop = end.min(extent_end);
            if start >= stop {
                continue;
            }
            let output_offset = usize::try_from(start - offset)
                .map_err(|_| invalid("read output offset is too large"))?;
            let length =
                usize::try_from(stop - start).map_err(|_| invalid("read length is too large"))?;
            let chunk_offset = extent
                .chunk_offset
                .checked_add(start - extent.file_offset)
                .ok_or_else(|| invalid("chunk read offset overflow"))?;
            ops.push(ChunkReadOp {
                chunk_id: extent.chunk_id.clone(),
                chunk_offset,
                length: length as u64,
                output_offset,
            });
        }
        self.read_engine.read_batch(
            &ReadBatch {
                file_version_id: committed.cloned(),
                layout_root_id: layout.id.clone(),
                ops,
            },
            out,
        )?;
        Ok(())
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
        let Some(cell) = self.write_state(inode_id)? else {
            return Ok(None);
        };
        enum CommitAction {
            Prepare(Box<FrozenCommit>),
            Send(InFlightCommit),
        }

        loop {
            let action = loop {
                let mut state = cell
                    .lock()
                    .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
                while state.commit_busy {
                    state = cell
                        .wait(state)
                        .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
                }
                if let Some(error) = state.terminal_error.clone() {
                    return Err(error);
                }
                if let Some(in_flight) = state.in_flight.clone() {
                    let pending = match in_flight {
                        InFlightCommit::Preparing(_) => {
                            state = cell
                                .wait(state)
                                .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
                            drop(state);
                            continue;
                        }
                        InFlightCommit::File(pending) => InFlightCommit::File(pending),
                        InFlightCommit::Metadata(sync) => InFlightCommit::Metadata(sync),
                    };
                    state.commit_busy = true;
                    break CommitAction::Send(pending);
                } else {
                    if !(state.dirty
                        || matches!(reason, CommitReason::FullSync) && state.metadata_dirty)
                    {
                        return Ok(None);
                    }
                    if should_renew(&state.write_lease) {
                        state.write_lease =
                            self.meta.renew_write_lease(state.write_lease.clone())?;
                    }
                    if !state.dirty {
                        let sync = SyncInodeMetadata {
                            operation_id: self.operation_id("fsync-metadata"),
                            inode_id: state.inode.inode_id.clone(),
                            write_lease: state.write_lease.clone(),
                            expected_inode_revision: state.inode.revision,
                            expected_head_version: state
                                .base_version
                                .as_ref()
                                .map(|version| version.id.clone()),
                            metadata_delta: CommitMetadataDelta {
                                mode: CommitMetadataMode::Full,
                                mtime_unix_ms: Some(state.inode.attributes.mtime_unix_ms),
                                ctime_unix_ms: Some(state.inode.attributes.ctime_unix_ms),
                            },
                        };
                        let pending = InFlightCommit::Metadata(sync);
                        state.in_flight = Some(pending.clone());
                        state.commit_busy = true;
                        break CommitAction::Send(pending);
                    } else {
                        let frozen = FrozenCommit {
                            through_seq: state.visible_write_seq,
                            logical_length: state.logical_length,
                            inode: state.inode.clone(),
                            write_lease: state.write_lease.clone(),
                            base_version: state.base_version.clone(),
                            base_layout: state.base_layout.clone(),
                            dirty_extents: std::mem::take(&mut state.dirty_extents),
                        };
                        state.in_flight = Some(InFlightCommit::Preparing(Box::new(frozen.clone())));
                        state.commit_busy = true;
                        state.dirty = false;
                        break CommitAction::Prepare(Box::new(frozen));
                    }
                }
            };

            let pending = match action {
                CommitAction::Send(pending) => pending,
                CommitAction::Prepare(frozen) => match self.prepare_commit(&frozen, reason) {
                    Ok(batch) => {
                        let pending = InFlightCommit::File(Box::new(PendingFileCommit {
                            frozen: *frozen,
                            batch,
                        }));
                        let mut state = cell
                            .lock()
                            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
                        state.in_flight = Some(pending.clone());
                        pending
                    }
                    Err(error) => {
                        let mut state = cell
                            .lock()
                            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
                        if let Some(InFlightCommit::Preparing(failed)) = state.in_flight.take() {
                            state.dirty_extents.restore_before(failed.dirty_extents);
                        }
                        state.dirty = !state.dirty_extents.is_empty();
                        state.commit_busy = false;
                        cell.notify_all();
                        return Err(error);
                    }
                },
            };

            let committed = self.finish_commit(cell.clone(), pending)?;
            if matches!(reason, CommitReason::FullSync) && self.needs_full_metadata_sync(&cell)? {
                continue;
            }
            return Ok(committed);
        }
    }

    fn finish_commit(
        &self,
        cell: SharedInodeWriteState,
        pending: InFlightCommit,
    ) -> Result<Option<u64>> {
        let result = (|| match &pending {
            InFlightCommit::Preparing(_) => Err(unavailable("DFS inode commit is still preparing")),
            InFlightCommit::File(pending) => {
                let committed_full_metadata =
                    pending.batch.commit.metadata_delta.mode == CommitMetadataMode::Full;
                let version = pending.batch.commit.file_version.clone();
                let layout = pending.batch.commit.layout_root.clone();
                let updated = self.validate_inode(
                    self.meta
                        .commit_file_version(pending.batch.commit.clone())?,
                )?;
                Ok(AppliedCommit::File {
                    through_seq: pending.batch.through_seq,
                    committed_full_metadata,
                    version,
                    layout,
                    updated,
                })
            }
            InFlightCommit::Metadata(sync) => {
                let updated = self.validate_inode(self.meta.sync_inode_metadata(sync.clone())?)?;
                Ok(AppliedCommit::Metadata { updated })
            }
        })();

        let mut state = cell
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        state.commit_busy = false;
        let outcome = match result {
            Ok(AppliedCommit::File {
                through_seq,
                committed_full_metadata,
                version,
                layout,
                updated,
            }) => {
                state.in_flight = None;
                let mut updated = updated;
                if !committed_full_metadata {
                    updated.attributes.mtime_unix_ms = state.inode.attributes.mtime_unix_ms;
                    updated.attributes.ctime_unix_ms = state.inode.attributes.ctime_unix_ms;
                }
                state.inode = updated;
                state.base_version = Some(version);
                state.base_layout = layout;
                state.dirty = !state.dirty_extents.is_empty();
                state.durable_write_seq = through_seq;
                state.committed_write_seq = through_seq;
                state.last_writer_background_requested = false;
                if committed_full_metadata
                    && state.dirty_extents.is_empty()
                    && state.visible_write_seq <= through_seq
                {
                    state.metadata_dirty = false;
                }
                Ok(Some(through_seq))
            }
            Ok(AppliedCommit::Metadata { updated }) => {
                state.in_flight = None;
                state.inode = updated;
                state.metadata_dirty = false;
                Ok(Some(state.committed_write_seq))
            }
            Err(error) => {
                if is_definite_commit_rejection(&error) {
                    state.in_flight = None;
                    state.terminal_error = Some(error.clone());
                    if let InFlightCommit::File(pending) = pending {
                        state
                            .dirty_extents
                            .restore_before(pending.frozen.dirty_extents);
                        state.dirty = !state.dirty_extents.is_empty();
                    }
                }
                Err(error)
            }
        };
        cell.notify_all();
        outcome
    }

    fn needs_full_metadata_sync(&self, cell: &SharedInodeWriteState) -> Result<bool> {
        let state = cell
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        Ok(state.metadata_dirty)
    }

    fn lock_for_mutation<'a>(
        &self,
        inode_id: &InodeId,
        cell: &'a SharedInodeWriteState,
    ) -> Result<std::sync::MutexGuard<'a, InodeWriteState>> {
        loop {
            let mut state = cell
                .lock()
                .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
            while state.commit_busy {
                state = cell
                    .wait(state)
                    .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
            }
            if let Some(error) = state.terminal_error.clone() {
                return Err(error);
            }
            if state.in_flight.is_none() {
                return Ok(state);
            }
            drop(state);
            self.commit_inode(inode_id, CommitReason::Background)?;
        }
    }

    fn prepare_commit(&self, frozen: &FrozenCommit, reason: CommitReason) -> Result<CommitBatch> {
        let operation_id = self.operation_id(reason.operation_prefix());
        let generation = self.next_operation.fetch_add(1, Ordering::Relaxed);
        let plan = CommitPlanner.plan(
            frozen,
            operation_id.clone(),
            LayoutRootId::new(format!("{}-layout-{generation}", self.session_id)),
        )?;
        let receipts = self.chunk_store.put_batch(plan.staged_chunks)?;
        let now = now_unix_ms();
        let version = FileVersion {
            id: FileVersionId::new(format!("{}-version-{generation}", self.session_id)),
            inode_id: frozen.inode.inode_id.clone(),
            parent_version: frozen
                .base_version
                .as_ref()
                .map(|version| version.id.clone()),
            length: frozen.logical_length,
            layout_root: plan.layout_root.id.clone(),
            created_at_unix_ms: now,
        };
        Ok(CommitBatch {
            through_seq: frozen.through_seq,
            commit: CommitFileVersion {
                operation_id,
                inode_id: frozen.inode.inode_id.clone(),
                write_lease: frozen.write_lease.clone(),
                expected_inode_revision: frozen.inode.revision,
                expected_head_version: frozen
                    .base_version
                    .as_ref()
                    .map(|version| version.id.clone()),
                file_version: version,
                layout_root: plan.layout_root,
                chunk_receipts: receipts,
                metadata_delta: reason.metadata_delta(
                    frozen.inode.attributes.mtime_unix_ms,
                    frozen.inode.attributes.ctime_unix_ms,
                ),
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
        let state = state
            .lock()
            .map_err(|_| unavailable("DFS inode write state is poisoned"))?;
        if let Some(error) = state.terminal_error.clone() {
            return Err(error);
        }
        let observed = state.background_error.clone();
        let Some(observed) = observed.filter(|error| error.cursor > session.error_cursor) else {
            return Ok(());
        };
        self.update_handle_error_cursor(handle, observed.cursor)?;
        Err(observed.error)
    }

    fn resize_dirty_inode(&self, inode_id: &InodeId, length: u64) -> Result<u64> {
        let state = self
            .write_state(inode_id)?
            .ok_or_else(|| stale("DFS write state is no longer open"))?;
        let mut state = self.lock_for_mutation(inode_id, &state)?;
        state.next_write_seq = state.next_write_seq.saturating_add(1);
        let write_seq = state.next_write_seq;
        let old_length = state.logical_length;
        if old_length != length {
            state.dirty_extents.resize(old_length, length, write_seq)?;
            state.logical_length = length;
            state.dirty = true;
        }
        state.visible_write_seq = write_seq;
        let now = now_unix_ms();
        state.inode.attributes.mtime_unix_ms = now;
        state.inode.attributes.ctime_unix_ms = now;
        state.metadata_dirty = true;
        if state.open_writers == 0 && state.dirty {
            state.last_writer_background_requested = true;
        }
        Ok(state.visible_write_seq)
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
                (state.dirty || state.in_flight.is_some()).then(|| {
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
    opened_version: Option<OpenedFileVersion>,
    write_session: Option<DfsWriteSession>,
    flags: i32,
}

impl From<&DfsFileHandle> for DfsFileHandleSnapshot {
    fn from(handle: &DfsFileHandle) -> Self {
        Self {
            inode_id: handle.inode_id.clone(),
            opened_inode: handle.opened_inode.clone(),
            opened_version: handle.opened_version.clone(),
            write_session: handle.write_session.clone(),
            flags: handle.flags,
        }
    }
}

impl DirtyExtentMap {
    fn write_at(&mut self, offset: u64, data: &[u8], write_seq: u64) -> Result<usize> {
        let _ = usize::try_from(offset).map_err(|_| invalid("write offset is too large"))?;
        if !data.is_empty() {
            self.extents.push(DirtyExtent {
                file_offset: offset,
                length: data.len() as u64,
                write_seq,
                data: Some(Arc::from(data)),
            });
        }
        Ok(data.len())
    }

    fn resize(&mut self, old_length: u64, new_length: u64, write_seq: u64) -> Result<()> {
        let (start, stop) = if new_length < old_length {
            (new_length, old_length)
        } else {
            (old_length, new_length)
        };
        if start < stop {
            self.extents.push(DirtyExtent {
                file_offset: start,
                length: stop - start,
                write_seq,
                data: None,
            });
        }
        Ok(())
    }

    fn overlay(&self, offset: u64, out: &mut [u8]) -> Result<()> {
        let end = offset
            .checked_add(out.len() as u64)
            .ok_or_else(|| invalid("dirty read range overflow"))?;
        for extent in &self.extents {
            let extent_end = extent
                .file_offset
                .checked_add(extent.length)
                .ok_or_else(|| invalid("dirty extent range overflow"))?;
            let start = offset.max(extent.file_offset);
            let stop = end.min(extent_end);
            if start >= stop {
                continue;
            }
            let output_start = usize::try_from(start - offset)
                .map_err(|_| invalid("dirty output offset is too large"))?;
            let length = usize::try_from(stop - start)
                .map_err(|_| invalid("dirty overlay length is too large"))?;
            match &extent.data {
                Some(data) => {
                    let data_start = usize::try_from(start - extent.file_offset)
                        .map_err(|_| invalid("dirty data offset is too large"))?;
                    out[output_start..output_start + length]
                        .copy_from_slice(&data[data_start..data_start + length]);
                }
                None => out[output_start..output_start + length].fill(0),
            }
        }
        Ok(())
    }

    fn restore_before(&mut self, mut older: DirtyExtentMap) {
        older.extents.append(&mut self.extents);
        older.extents.sort_by_key(|extent| extent.write_seq);
        self.extents = older.extents;
    }

    fn is_empty(&self) -> bool {
        self.extents.is_empty()
    }
}

struct CommitPlanner;

impl CommitPlanner {
    fn plan(
        &self,
        frozen: &FrozenCommit,
        operation_id: OperationId,
        layout_id: LayoutRootId,
    ) -> Result<CommitPlan> {
        let overlay = normalized_overlay(&frozen.dirty_extents, frozen.logical_length)?;
        let mut extents = frozen
            .base_layout
            .inline_extents
            .iter()
            .filter_map(|extent| clip_extent(extent, frozen.logical_length))
            .collect::<Vec<_>>();
        for segment in &overlay {
            extents = subtract_range(extents, segment.file_offset, segment.length)?;
        }

        let mut staged_chunks = Vec::new();
        let mut pending_start = None;
        let mut pending = Vec::new();
        for segment in overlay {
            let Some((bytes, source_offset)) = segment.data else {
                flush_pending(
                    &mut pending_start,
                    &mut pending,
                    &operation_id,
                    &mut extents,
                    &mut staged_chunks,
                );
                continue;
            };
            let mut source_offset = source_offset;
            let mut file_offset = segment.file_offset;
            let mut remaining = usize::try_from(segment.length)
                .map_err(|_| invalid("dirty data length is too large"))?;
            while remaining > 0 {
                let expected = pending_start.map(|start| start + pending.len() as u64);
                if expected.is_some_and(|expected| expected != file_offset)
                    || pending.len() == COMMIT_CHUNK_BYTES as usize
                {
                    flush_pending(
                        &mut pending_start,
                        &mut pending,
                        &operation_id,
                        &mut extents,
                        &mut staged_chunks,
                    );
                }
                let capacity = COMMIT_CHUNK_BYTES as usize - pending.len();
                let take = capacity.min(remaining);
                pending_start.get_or_insert(file_offset);
                pending.extend_from_slice(&bytes[source_offset..source_offset + take]);
                source_offset += take;
                file_offset += take as u64;
                remaining -= take;
            }
        }
        flush_pending(
            &mut pending_start,
            &mut pending,
            &operation_id,
            &mut extents,
            &mut staged_chunks,
        );
        extents.sort_by_key(|extent| extent.file_offset);
        Ok(CommitPlan {
            layout_root: LayoutRoot {
                id: layout_id,
                file_length: frozen.logical_length,
                inline_extents: extents,
            },
            staged_chunks,
        })
    }
}

fn normalized_overlay(map: &DirtyExtentMap, file_length: u64) -> Result<Vec<OverlaySegment>> {
    let mut output: Vec<OverlaySegment> = Vec::new();
    for dirty in &map.extents {
        if dirty.file_offset >= file_length || dirty.length == 0 {
            continue;
        }
        let dirty_end = dirty
            .file_offset
            .checked_add(dirty.length)
            .ok_or_else(|| invalid("dirty extent range overflow"))?
            .min(file_length);
        let mut next = Vec::with_capacity(output.len().saturating_add(1));
        for segment in output {
            let segment_end = segment.file_offset + segment.length;
            if dirty_end <= segment.file_offset || dirty.file_offset >= segment_end {
                next.push(segment);
                continue;
            }
            if dirty.file_offset > segment.file_offset {
                next.push(OverlaySegment {
                    file_offset: segment.file_offset,
                    length: dirty.file_offset - segment.file_offset,
                    data: segment.data.clone(),
                });
            }
            if dirty_end < segment_end {
                let data = match segment.data {
                    Some((bytes, source_offset)) => {
                        let delta = usize::try_from(dirty_end - segment.file_offset)
                            .map_err(|_| invalid("overlay source offset is too large"))?;
                        let source_offset = source_offset
                            .checked_add(delta)
                            .ok_or_else(|| invalid("overlay source offset overflow"))?;
                        Some((bytes, source_offset))
                    }
                    None => None,
                };
                next.push(OverlaySegment {
                    file_offset: dirty_end,
                    length: segment_end - dirty_end,
                    data,
                });
            }
        }
        next.push(OverlaySegment {
            file_offset: dirty.file_offset,
            length: dirty_end - dirty.file_offset,
            data: dirty.data.clone().map(|bytes| (bytes, 0)),
        });
        next.sort_by_key(|segment| segment.file_offset);
        output = next;
    }
    Ok(output)
}

fn flush_pending(
    pending_start: &mut Option<u64>,
    pending: &mut Vec<u8>,
    operation_id: &OperationId,
    extents: &mut Vec<Extent>,
    staged_chunks: &mut Vec<StagedChunk>,
) {
    let Some(file_offset) = pending_start.take() else {
        return;
    };
    if pending.is_empty() {
        return;
    }
    let mut builder = ChunkBuilder::default();
    builder.replace(std::mem::take(pending));
    let staged = builder.stage(operation_id.clone());
    extents.push(Extent {
        file_offset,
        length: staged.chunk.length,
        chunk_id: staged.chunk.id.clone(),
        chunk_offset: 0,
    });
    staged_chunks.push(staged);
}

fn clip_extent(extent: &Extent, file_length: u64) -> Option<Extent> {
    if extent.file_offset >= file_length {
        return None;
    }
    let length = extent.length.min(file_length - extent.file_offset);
    (length > 0).then(|| Extent {
        file_offset: extent.file_offset,
        length,
        chunk_id: extent.chunk_id.clone(),
        chunk_offset: extent.chunk_offset,
    })
}

fn subtract_range(extents: Vec<Extent>, offset: u64, length: u64) -> Result<Vec<Extent>> {
    if length == 0 {
        return Ok(extents);
    }
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid("subtracted extent range overflow"))?;
    let mut output = Vec::with_capacity(extents.len().saturating_add(1));
    for extent in extents {
        let extent_end = extent
            .file_offset
            .checked_add(extent.length)
            .ok_or_else(|| invalid("base extent range overflow"))?;
        if end <= extent.file_offset || offset >= extent_end {
            output.push(extent);
            continue;
        }
        if offset > extent.file_offset {
            output.push(Extent {
                file_offset: extent.file_offset,
                length: offset - extent.file_offset,
                chunk_id: extent.chunk_id.clone(),
                chunk_offset: extent.chunk_offset,
            });
        }
        if end < extent_end {
            output.push(Extent {
                file_offset: end,
                length: extent_end - end,
                chunk_id: extent.chunk_id,
                chunk_offset: extent
                    .chunk_offset
                    .checked_add(end - extent.file_offset)
                    .ok_or_else(|| invalid("base extent chunk offset overflow"))?,
            });
        }
    }
    Ok(output)
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

    fn metadata_delta(self, mtime_unix_ms: u64, ctime_unix_ms: u64) -> CommitMetadataDelta {
        if !matches!(self, Self::DataSync) {
            CommitMetadataDelta {
                mode: CommitMetadataMode::Full,
                mtime_unix_ms: Some(mtime_unix_ms),
                ctime_unix_ms: Some(ctime_unix_ms),
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
        Ok(Entry {
            inode: self.backend_inode(&inode.inode_id)?,
            attributes: self.visible_attributes(&inode)?,
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
            if let Some(opened) = snapshot.opened_version.as_ref() {
                return Ok(attributes(&snapshot.opened_inode, opened.length));
            }
            return self.visible_attributes(&snapshot.opened_inode);
        }
        let record = self.validate_inode(self.meta.get_inode(&self.inode_id(inode)?)?)?;
        self.visible_attributes(&record)
    }

    fn setattr(
        &self,
        ctx: &RequestContext,
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
                "DFS chmod/chown/time updates are not wired yet",
            ));
        }

        let Some(length) = change.size else {
            return self.getattr(ctx, inode, handle);
        };
        let inode_id = self.inode_id(inode)?;
        let (record, accepted_seq) = if let Some(handle) = handle {
            self.observe_handle_error(handle)?;
            let snapshot = self.handle_snapshot(handle)?;
            if snapshot.inode_id != inode_id {
                return Err(stale("DFS file handle does not name the requested inode"));
            }
            if snapshot.flags & libc::O_ACCMODE == libc::O_RDONLY {
                return Err(bad_file_descriptor("DFS handle was not opened for writing"));
            }
            if snapshot.write_session.is_none() {
                return Err(invalid("writable DFS handle has no write session"));
            }
            if snapshot.opened_inode.kind != InodeKind::Regular {
                return Err(Error::from(std::io::Error::from_raw_os_error(libc::EISDIR)));
            }
            let accepted_seq = self.resize_dirty_inode(&snapshot.inode_id, length)?;
            (snapshot.opened_inode, Some((handle, accepted_seq)))
        } else {
            let record = self.validate_inode(self.meta.get_inode(&inode_id)?)?;
            if record.kind != InodeKind::Regular {
                return Err(Error::from(std::io::Error::from_raw_os_error(libc::EISDIR)));
            }
            self.ensure_inode_write_state(&record)?;
            self.resize_dirty_inode(&record.inode_id, length)?;
            (record, None)
        };
        if let Some((handle, accepted_seq)) = accepted_seq {
            self.update_handle_write_progress(handle, accepted_seq)?;
        }
        self.visible_attributes(&record)
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
        let session = self.open_write_session(&inode, flags)?;
        let handle = self.allocate_handle(DfsFileHandle {
            inode_id: inode.inode_id.clone(),
            opened_inode: inode.clone(),
            opened_version: None,
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
        let (opened_inode, opened_version, write_session) = if writable {
            let (inode, write_lease) = self.meta.open_write(&inode_id)?;
            let inode = self.validate_inode(inode)?;
            self.install_write_state(inode.clone(), write_lease)?;
            let session = self.open_write_session(&inode, flags)?;
            if flags & libc::O_TRUNC != 0
                && let Err(error) = self.resize_dirty_inode(&inode.inode_id, 0)
            {
                self.release_writer(&session)?;
                return Err(error);
            }
            (inode, None, Some(session))
        } else {
            let inode = self.validate_inode(self.meta.get_inode(&inode_id)?)?;
            let opened_version = self.opened_version(&inode)?;
            (inode, Some(opened_version), None)
        };
        self.allocate_handle(DfsFileHandle {
            inode_id: opened_inode.inode_id.clone(),
            opened_inode,
            opened_version,
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
        if let Some(opened) = snapshot.opened_version.as_ref() {
            return self.read_opened_version(opened, offset, out);
        }
        self.read_visible(
            &snapshot.inode_id,
            snapshot.opened_inode.head_version.as_ref(),
            offset,
            out,
        )
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
            return Err(bad_file_descriptor("DFS handle was not opened for writing"));
        }
        if snapshot.write_session.is_none() {
            return Err(invalid("writable DFS handle has no write session"));
        }
        let state = self
            .write_state(&snapshot.inode_id)?
            .ok_or_else(|| stale("DFS write state is no longer open"))?;
        let (written, accepted_seq) = {
            let mut state = self.lock_for_mutation(&snapshot.inode_id, &state)?;
            let offset = if snapshot.flags & libc::O_APPEND != 0 {
                state.logical_length
            } else {
                offset
            };
            let next_seq = state.next_write_seq.saturating_add(1);
            let written = state.dirty_extents.write_at(offset, data, next_seq)?;
            state.logical_length = state
                .logical_length
                .max(offset.saturating_add(written as u64));
            if written > 0 {
                state.next_write_seq = next_seq;
                state.visible_write_seq = state.next_write_seq;
                state.dirty = true;
                state.metadata_dirty = true;
                let now = now_unix_ms();
                state.inode.attributes.mtime_unix_ms = now;
                state.inode.attributes.ctime_unix_ms = now;
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
            if let Some(session) = handle.write_session.as_mut() {
                handle.opened_inode = inode.clone();
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

fn is_definite_commit_rejection(error: &Error) -> bool {
    matches!(
        error.code(),
        afs_error::META_CATALOG_INVALID_REQUEST | afs_error::META_DFS_CONFLICT
    )
}

fn stale(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_DFS_STALE_HANDLE, message)
}

fn bad_file_descriptor(message: impl Into<String>) -> Error {
    Error::coded(afs_error::IO_BAD_FILE_DESCRIPTOR, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::chunk::LocalChunkStore;

    struct RecordingMeta {
        inode: Mutex<InodeRecord>,
        lease: Mutex<WriteLease>,
        commits: Mutex<Vec<CommitFileVersion>>,
        failed_commits: Mutex<Vec<CommitFileVersion>>,
        metadata_syncs: Mutex<Vec<SyncInodeMetadata>>,
        failed_metadata_syncs: Mutex<Vec<SyncInodeMetadata>>,
        next_commit_error: Mutex<Option<Error>>,
        next_metadata_sync_error: Mutex<Option<Error>>,
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
                failed_commits: Mutex::new(Vec::new()),
                metadata_syncs: Mutex::new(Vec::new()),
                failed_metadata_syncs: Mutex::new(Vec::new()),
                next_commit_error: Mutex::new(None),
                next_metadata_sync_error: Mutex::new(None),
            }
        }

        fn commit_count(&self) -> usize {
            self.commits.lock().unwrap().len()
        }

        fn fail_next_commit(&self) {
            self.fail_next_commit_with(unavailable("injected test commit failure"));
        }

        fn fail_next_metadata_sync(&self) {
            self.fail_next_metadata_sync_with(unavailable("injected test metadata sync failure"));
        }

        fn fail_next_commit_with(&self, error: Error) {
            *self.next_commit_error.lock().unwrap() = Some(error);
        }

        fn fail_next_metadata_sync_with(&self, error: Error) {
            *self.next_metadata_sync_error.lock().unwrap() = Some(error);
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
            let next_error = self.next_metadata_sync_error.lock().unwrap().take();
            if let Some(error) = next_error {
                self.failed_metadata_syncs.lock().unwrap().push(sync);
                return Err(error);
            }
            let mut inode = self.inode.lock().unwrap();
            if inode.revision != sync.expected_inode_revision
                || inode.head_version != sync.expected_head_version
            {
                return Err(unavailable("test inode metadata CAS failed"));
            }
            inode.revision = inode.revision.saturating_add(1);
            inode.attributes.mtime_unix_ms = sync.metadata_delta.mtime_unix_ms.unwrap();
            inode.attributes.ctime_unix_ms = sync.metadata_delta.ctime_unix_ms.unwrap();
            self.metadata_syncs.lock().unwrap().push(sync);
            Ok(inode.clone())
        }

        fn commit_file_version(&self, commit: CommitFileVersion) -> Result<InodeRecord> {
            let next_error = self.next_commit_error.lock().unwrap().take();
            if let Some(error) = next_error {
                self.failed_commits.lock().unwrap().push(commit);
                return Err(error);
            }
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

    fn test_fs() -> (tempfile::TempDir, Arc<RecordingMeta>, DistributedFs) {
        let meta = Arc::new(RecordingMeta::new());
        let temp = tempfile::tempdir().unwrap();
        let chunks = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
        let read_engine = Arc::new(crate::node::dfs_read::DfsReadEngine::new(
            NamespaceId::new("default"),
            "node-a".into(),
            chunks.clone(),
            Arc::new(crate::node::dfs_read::UnimplementedReadSourceProvider),
            Arc::new(crate::node::dfs_read::UnimplementedChunkTransfer),
            crate::node::dfs_read::DfsReadConfig::default(),
        ));
        let fs = DistributedFs::new(
            NamespaceId::new("default"),
            "node-a",
            "session-a",
            meta.clone(),
            chunks,
            read_engine,
        );
        (temp, meta, fs)
    }

    #[test]
    fn readonly_handles_keep_opened_file_version_while_writer_sees_dirty() {
        let (_temp, meta, fs) = test_fs();
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
        assert_eq!(
            fs.read(&context(), reader, 0, &mut out).unwrap(),
            0,
            "readonly handle opened before the first FileVersion must stay on that empty snapshot"
        );

        let writer = fs
            .open(&context(), created.entry.inode, libc::O_RDWR)
            .unwrap();
        let mut writer_view = [0; 5];
        assert_eq!(fs.read(&context(), writer, 0, &mut writer_view).unwrap(), 5);
        assert_eq!(
            &writer_view, b"hello",
            "writer must keep seeing owner dirty"
        );
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
        let committed_reader = fs
            .open(&context(), created.entry.inode, libc::O_RDONLY)
            .unwrap();
        let mut committed = [0; 5];
        assert_eq!(
            fs.read(&context(), committed_reader, 0, &mut committed)
                .unwrap(),
            5
        );
        assert_eq!(&committed, b"hello");

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
        assert_eq!(fs.read(&context(), reader, 0, &mut latest).unwrap(), 0);
        assert_eq!(
            fs.read(&context(), committed_reader, 0, &mut latest)
                .unwrap(),
            5
        );
        assert_eq!(&latest, b"hello");
        let fresh_reader = fs
            .open(&context(), created.entry.inode, libc::O_RDONLY)
            .unwrap();
        assert_eq!(
            fs.read(&context(), fresh_reader, 0, &mut latest).unwrap(),
            5
        );
        assert_eq!(&latest, b"Hello");
        fs.release(&context(), writer).unwrap();
        fs.release(&context(), reader).unwrap();
        fs.release(&context(), committed_reader).unwrap();
        fs.release(&context(), fresh_reader).unwrap();
        assert_eq!(fs.writeback_pending().unwrap(), 0);
    }

    #[test]
    fn sparse_write_materializes_only_payload_chunk() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("sparse.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        let offset = 1024 * 1024;
        assert_eq!(
            fs.write(&context(), created.handle, offset, b"tail")
                .unwrap(),
            4
        );

        let attrs = fs
            .getattr(&context(), created.entry.inode, Some(created.handle))
            .unwrap();
        assert_eq!(attrs.size, offset + 4);
        let mut head = [1; 16];
        assert_eq!(
            fs.read(&context(), created.handle, 0, &mut head).unwrap(),
            head.len()
        );
        assert_eq!(head, [0; 16]);

        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();
        let commits = meta.commits.lock().unwrap();
        let commit = commits.last().unwrap();
        assert_eq!(commit.file_version.length, offset + 4);
        assert_eq!(commit.layout_root.file_length, offset + 4);
        assert_eq!(commit.layout_root.inline_extents.len(), 1);
        assert_eq!(commit.layout_root.inline_extents[0].file_offset, offset);
        assert_eq!(commit.layout_root.inline_extents[0].length, 4);
        assert_eq!(commit.chunk_receipts.len(), 1);
    }

    #[test]
    fn handle_resize_shrink_then_grow_masks_old_tail_without_new_chunk() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("resize.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&context(), created.handle, 0, b"abcdefgh")
            .unwrap();
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();
        let before_seq = fs
            .handle_snapshot(created.handle)
            .unwrap()
            .write_session
            .unwrap()
            .last_accepted_seq;

        let shrink = AttributeChange {
            size: Some(4),
            ..AttributeChange::default()
        };
        let grow = AttributeChange {
            size: Some(8),
            ..AttributeChange::default()
        };
        fs.setattr(
            &context(),
            created.entry.inode,
            Some(created.handle),
            &shrink,
        )
        .unwrap();
        let attrs = fs
            .setattr(&context(), created.entry.inode, Some(created.handle), &grow)
            .unwrap();
        assert_eq!(attrs.size, 8);
        let after_seq = fs
            .handle_snapshot(created.handle)
            .unwrap()
            .write_session
            .unwrap()
            .last_accepted_seq;
        assert!(after_seq > before_seq);

        let mut visible = [1; 8];
        assert_eq!(
            fs.read(&context(), created.handle, 0, &mut visible)
                .unwrap(),
            visible.len()
        );
        assert_eq!(&visible, b"abcd\0\0\0\0");
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();

        let commits = meta.commits.lock().unwrap();
        assert_eq!(commits.len(), 2);
        let resized = commits.last().unwrap();
        assert_eq!(resized.file_version.length, 8);
        assert!(resized.chunk_receipts.is_empty());
        assert_eq!(resized.layout_root.inline_extents.len(), 1);
        assert_eq!(resized.layout_root.inline_extents[0].length, 4);
        drop(commits);

        fs.setattr(&context(), created.entry.inode, Some(created.handle), &grow)
            .unwrap();
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();
        assert_eq!(
            meta.commit_count(),
            2,
            "same-length resize must not create another FileVersion"
        );
    }

    #[test]
    fn path_resize_uses_inode_state_without_opening_a_writer() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("path-resize.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&context(), created.handle, 0, b"abcdefgh")
            .unwrap();
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();
        fs.release(&context(), created.handle).unwrap();

        let resize = AttributeChange {
            size: Some(3),
            ..AttributeChange::default()
        };
        let attrs = fs
            .setattr(&context(), created.entry.inode, None, &resize)
            .unwrap();
        assert_eq!(attrs.size, 3);
        let state = fs
            .write_state(&meta.inode.lock().unwrap().inode_id)
            .unwrap()
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.open_writers, 0);
        assert!(state.last_writer_background_requested);
        drop(state);

        let reader = fs
            .open(&context(), created.entry.inode, libc::O_RDONLY)
            .unwrap();
        let readonly_error = fs
            .setattr(
                &context(),
                created.entry.inode,
                Some(reader),
                &AttributeChange {
                    size: Some(2),
                    ..AttributeChange::default()
                },
            )
            .unwrap_err();
        assert_eq!(readonly_error.code(), afs_error::IO_BAD_FILE_DESCRIPTOR);
        let mut visible = [0; 8];
        assert_eq!(fs.read(&context(), reader, 0, &mut visible).unwrap(), 8);
        assert_eq!(&visible, b"abcdefgh");

        assert_eq!(fs.writeback_pending().unwrap(), 1);
        assert_eq!(meta.commit_count(), 2);
        let commits = meta.commits.lock().unwrap();
        let resized = commits.last().unwrap();
        assert_eq!(resized.file_version.length, 3);
        assert!(resized.chunk_receipts.is_empty());
    }

    #[test]
    fn failed_metadata_only_sync_keeps_request_identity_and_blocks_writes() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("metadata-retry.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&context(), created.handle, 0, b"data").unwrap();
        fs.fsync(&context(), created.handle, SyncMode::DataOnly)
            .unwrap();
        assert_eq!(meta.commit_count(), 1);
        assert!(meta.metadata_syncs.lock().unwrap().is_empty());

        meta.fail_next_metadata_sync();
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap_err();
        let failed = meta.failed_metadata_syncs.lock().unwrap()[0].clone();
        assert_eq!(
            fs.write(&context(), created.handle, 4, b"!").unwrap(),
            1,
            "write must wait for the pending metadata sync retry before creating new dirty state"
        );
        let metadata_syncs = meta.metadata_syncs.lock().unwrap();
        assert_eq!(metadata_syncs.len(), 1);
        assert_eq!(failed.operation_id, metadata_syncs[0].operation_id);
        assert_eq!(
            failed.expected_inode_revision,
            metadata_syncs[0].expected_inode_revision
        );
        assert_eq!(
            failed.expected_head_version,
            metadata_syncs[0].expected_head_version
        );
    }

    #[test]
    fn full_sync_after_pending_data_only_commit_also_syncs_metadata() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("full-after-data-pending.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&context(), created.handle, 0, b"data").unwrap();
        let dirty_mtime = {
            let state = fs
                .write_state(&meta.inode.lock().unwrap().inode_id)
                .unwrap()
                .unwrap();
            state.lock().unwrap().inode.attributes.mtime_unix_ms
        };

        meta.fail_next_commit();
        fs.fsync(&context(), created.handle, SyncMode::DataOnly)
            .unwrap_err();
        let failed = meta.failed_commits.lock().unwrap()[0].clone();

        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();
        let commits = meta.commits.lock().unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(failed.operation_id, commits[0].operation_id);
        assert_eq!(failed.file_version.id, commits[0].file_version.id);
        assert_eq!(commits[0].metadata_delta.mode, CommitMetadataMode::DataOnly);
        drop(commits);
        let metadata_syncs = meta.metadata_syncs.lock().unwrap();
        assert_eq!(metadata_syncs.len(), 1);
        assert_eq!(
            metadata_syncs[0].metadata_delta.mtime_unix_ms,
            Some(dirty_mtime)
        );
        assert_eq!(
            metadata_syncs[0].expected_head_version,
            meta.inode.lock().unwrap().head_version
        );
    }

    #[test]
    fn deadline_commit_error_preserves_exact_batch_for_retry() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("deadline-retry.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&context(), created.handle, 0, b"data").unwrap();

        meta.fail_next_commit_with(Error::coded(
            afs_error::CLIENT_DEADLINE_EXCEEDED,
            "deadline while committing file version",
        ));
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap_err();
        let failed = meta.failed_commits.lock().unwrap()[0].clone();

        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();
        let commits = meta.commits.lock().unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(failed.operation_id, commits[0].operation_id);
        assert_eq!(failed.file_version.id, commits[0].file_version.id);
        assert_eq!(failed.layout_root.id, commits[0].layout_root.id);
    }

    #[test]
    fn remote_status_metadata_error_preserves_exact_request_for_retry() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("remote-status-metadata-retry.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&context(), created.handle, 0, b"data").unwrap();
        fs.fsync(&context(), created.handle, SyncMode::DataOnly)
            .unwrap();

        meta.fail_next_metadata_sync_with(Error::new(
            afs_error::CLIENT_REMOTE_STATUS,
            afs_error::ErrorKind::Unavailable,
            "remote status without AFS detail",
        ));
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap_err();
        let failed = meta.failed_metadata_syncs.lock().unwrap()[0].clone();

        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();
        let metadata_syncs = meta.metadata_syncs.lock().unwrap();
        assert_eq!(metadata_syncs.len(), 1);
        assert_eq!(failed.operation_id, metadata_syncs[0].operation_id);
        assert_eq!(
            failed.expected_head_version,
            metadata_syncs[0].expected_head_version
        );
    }

    #[test]
    fn definite_meta_invalid_blocks_subsequent_mutation() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("terminal-invalid.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&context(), created.handle, 0, b"data").unwrap();

        meta.fail_next_commit_with(Error::coded(
            afs_error::META_CATALOG_INVALID_REQUEST,
            "definite invalid commit",
        ));
        let error = fs
            .fsync(&context(), created.handle, SyncMode::Full)
            .unwrap_err();
        assert_eq!(error.code(), afs_error::META_CATALOG_INVALID_REQUEST);

        let mut visible = [0; 4];
        assert_eq!(
            fs.read(&context(), created.handle, 0, &mut visible)
                .unwrap(),
            4
        );
        assert_eq!(&visible, b"data");
        assert_eq!(
            fs.write(&context(), created.handle, 4, b"!")
                .unwrap_err()
                .code(),
            afs_error::META_CATALOG_INVALID_REQUEST
        );
        assert_eq!(
            fs.fsync(&context(), created.handle, SyncMode::Full)
                .unwrap_err()
                .code(),
            afs_error::META_CATALOG_INVALID_REQUEST
        );
        assert_eq!(meta.commit_count(), 0);
    }

    #[test]
    fn failed_resize_commit_keeps_batch_identity_and_blocks_new_dirty_for_retry() {
        let (_temp, meta, fs) = test_fs();
        let created = fs
            .create(
                &context(),
                fs.root_inode(),
                OsStr::new("retry-resize.bin"),
                0o640,
                libc::O_RDWR,
            )
            .unwrap();
        fs.write(&context(), created.handle, 0, b"abcdefgh")
            .unwrap();
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap();
        fs.setattr(
            &context(),
            created.entry.inode,
            Some(created.handle),
            &AttributeChange {
                size: Some(4),
                ..AttributeChange::default()
            },
        )
        .unwrap();

        meta.fail_next_commit();
        fs.fsync(&context(), created.handle, SyncMode::Full)
            .unwrap_err();
        let failed = meta.failed_commits.lock().unwrap()[0].clone();
        let attrs = fs
            .getattr(&context(), created.entry.inode, Some(created.handle))
            .unwrap();
        assert_eq!(attrs.size, 4);
        let mut visible = [0; 8];
        assert_eq!(
            fs.read(&context(), created.handle, 0, &mut visible)
                .unwrap(),
            4
        );
        assert_eq!(&visible[..4], b"abcd");
        assert_eq!(
            fs.write(&context(), created.handle, 4, b"Z").unwrap(),
            1,
            "write must wait for the pending FileVersion retry before appending new dirty state"
        );
        assert_eq!(meta.commit_count(), 2);
        let commits = meta.commits.lock().unwrap();
        let retried = commits.last().unwrap();
        assert_eq!(failed.operation_id, retried.operation_id);
        assert_eq!(failed.file_version.id, retried.file_version.id);
        assert_eq!(failed.layout_root.id, retried.layout_root.id);
        assert_eq!(retried.file_version.length, 4);
        assert_eq!(retried.layout_root.file_length, 4);
        assert!(retried.chunk_receipts.is_empty());
        drop(commits);
        let attrs = fs
            .setattr(
                &context(),
                created.entry.inode,
                Some(created.handle),
                &AttributeChange {
                    size: Some(6),
                    ..AttributeChange::default()
                },
            )
            .unwrap();
        assert_eq!(attrs.size, 6);
    }
}

//! DistributedFs POSIX backend for the first local R=1 vertical slice.
//!
//! One open writer owns a `DfsWriteSession`. FUSE write fragments update its
//! `ChunkBuilder`; fsync finalizes one immutable local Chunk and atomically asks
//! Meta to move the mutable inode head to a new immutable FileVersion. 第一阶段的
//! flush/release 也保守地提交脏句柄；其最终语义由后续专题收敛。

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
        CommitFileVersion, DfsWriteSessionId, DurabilityPolicy, Extent, FileVersion, FileVersionId,
        InodeAttributes, InodeId, InodeKind, InodeRecord, LayoutRoot, LayoutRootId, NamespaceId,
        OperationId,
    },
    node::chunk::{ChunkBuilder, ChunkStore},
};

const ROOT_INODE: u64 = 1;

pub trait DfsMeta: Send + Sync {
    fn lookup(&self, parent: &InodeId, name: &[u8]) -> Result<Option<InodeRecord>>;
    fn create(
        &self,
        operation_id: &OperationId,
        parent: &InodeId,
        name: &[u8],
        attributes: InodeAttributes,
    ) -> Result<InodeRecord>;
    fn get_inode(&self, inode_id: &InodeId) -> Result<InodeRecord>;
    fn get_file_version(&self, version_id: &FileVersionId) -> Result<(FileVersion, LayoutRoot)>;
    fn commit_file_version(&self, commit: CommitFileVersion) -> Result<InodeRecord>;
}

pub struct DistributedFs {
    namespace_id: NamespaceId,
    instance_id: String,
    meta: Arc<dyn DfsMeta>,
    chunk_store: Arc<dyn ChunkStore>,
    handles: Mutex<HashMap<u64, DfsFileHandle>>,
    inode_to_backend: Mutex<HashMap<InodeId, u64>>,
    backend_to_inode: Mutex<HashMap<u64, InodeId>>,
    next_inode: AtomicU64,
    next_handle: AtomicU64,
    next_operation: AtomicU64,
}

struct DfsFileHandle {
    inode: InodeRecord,
    opened_version_id: Option<FileVersionId>,
    write_session: Option<DfsWriteSession>,
    flags: i32,
}

pub struct DfsWriteSession {
    pub id: DfsWriteSessionId,
    pub operation_id: OperationId,
    pub inode_id: InodeId,
    pub base_version_id: Option<FileVersionId>,
    pub logical_length: u64,
    pub durability_policy: DurabilityPolicy,
    builder: ChunkBuilder,
    dirty: bool,
}

impl DistributedFs {
    pub fn new(
        namespace_id: NamespaceId,
        instance_id: impl Into<String>,
        meta: Arc<dyn DfsMeta>,
        chunk_store: Arc<dyn ChunkStore>,
    ) -> Self {
        Self {
            namespace_id,
            instance_id: instance_id.into(),
            meta,
            chunk_store,
            handles: Mutex::new(HashMap::new()),
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
            self.instance_id,
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

    fn new_write_session(&self, inode: &InodeRecord) -> Result<DfsWriteSession> {
        let operation_id = self.operation_id("write");
        let mut builder = ChunkBuilder::default();
        builder.replace(self.load_version_bytes(inode.head_version.as_ref())?);
        Ok(DfsWriteSession {
            id: DfsWriteSessionId::new(operation_id.0.clone()),
            operation_id,
            inode_id: inode.inode_id.clone(),
            base_version_id: inode.head_version.clone(),
            logical_length: builder.len(),
            durability_policy: DurabilityPolicy::local_single_copy(),
            builder,
            dirty: false,
        })
    }

    fn commit_handle(&self, handle: FileHandle) -> Result<()> {
        let commit = {
            let mut handles = self
                .handles
                .lock()
                .map_err(|_| unavailable("DFS handle table is poisoned"))?;
            let file = handles
                .get_mut(&handle.0)
                .ok_or_else(|| stale("DFS file handle is no longer open"))?;
            let Some(session) = file.write_session.as_mut() else {
                return Ok(());
            };
            if !session.dirty {
                return Ok(());
            }
            session.builder.truncate(session.logical_length)?;
            let receipt = self.chunk_store.put(
                session.builder.stage(session.operation_id.clone()),
                &session.durability_policy,
            )?;
            let generation = self.next_operation.fetch_add(1, Ordering::Relaxed);
            let layout = LayoutRoot {
                id: LayoutRootId::new(format!("{}-layout-{generation}", self.instance_id)),
                file_length: session.logical_length,
                inline_extents: if session.logical_length == 0 {
                    Vec::new()
                } else {
                    vec![Extent {
                        file_offset: 0,
                        length: session.logical_length,
                        chunk_id: receipt.chunk.id.clone(),
                        chunk_offset: 0,
                    }]
                },
            };
            let version = FileVersion {
                id: FileVersionId::new(format!("{}-version-{generation}", self.instance_id)),
                inode_id: session.inode_id.clone(),
                parent_version: session.base_version_id.clone(),
                length: session.logical_length,
                layout_root: layout.id.clone(),
                created_at_unix_ms: now_unix_ms(),
            };
            CommitFileVersion {
                operation_id: session.operation_id.clone(),
                inode_id: session.inode_id.clone(),
                expected_inode_revision: file.inode.revision,
                expected_head_version: session.base_version_id.clone(),
                file_version: version,
                layout_root: layout,
                chunk_receipts: vec![receipt],
            }
        };
        let updated = self.validate_inode(self.meta.commit_file_version(commit)?)?;
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?;
        let file = handles
            .get_mut(&handle.0)
            .ok_or_else(|| stale("DFS file handle closed during commit"))?;
        file.inode = updated.clone();
        file.opened_version_id = updated.head_version.clone();
        if let Some(session) = file.write_session.as_mut() {
            session.base_version_id = updated.head_version;
            session.operation_id = self.operation_id("write");
            session.id = DfsWriteSessionId::new(session.operation_id.0.clone());
            session.dirty = false;
        }
        Ok(())
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
        let size = self.inode_size(&inode)?;
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
            let handles = self
                .handles
                .lock()
                .map_err(|_| unavailable("DFS handle table is poisoned"))?;
            let file = handles
                .get(&handle.0)
                .ok_or_else(|| stale("DFS file handle is no longer open"))?;
            let size = file.write_session.as_ref().map_or_else(
                || self.inode_size(&file.inode),
                |session| Ok(session.logical_length),
            )?;
            return Ok(attributes(&file.inode, size));
        }
        let record = self.validate_inode(self.meta.get_inode(&self.inode_id(inode)?)?)?;
        let size = self.inode_size(&record)?;
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
        let inode = self.validate_inode(self.meta.create(
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
        )?)?;
        let session = self.new_write_session(&inode)?;
        let handle = self.allocate_handle(DfsFileHandle {
            inode: inode.clone(),
            opened_version_id: inode.head_version.clone(),
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
        let inode = self.validate_inode(self.meta.get_inode(&self.inode_id(inode)?)?)?;
        let writable = flags & libc::O_ACCMODE != libc::O_RDONLY;
        let write_session = writable
            .then(|| self.new_write_session(&inode))
            .transpose()?;
        self.allocate_handle(DfsFileHandle {
            opened_version_id: inode.head_version.clone(),
            inode,
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
        let handles = self
            .handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?;
        let file = handles
            .get(&handle.0)
            .ok_or_else(|| stale("DFS file handle is no longer open"))?;
        let bytes = match &file.write_session {
            Some(session) if session.dirty => session.builder.as_bytes().to_vec(),
            _ => self.load_version_bytes(file.opened_version_id.as_ref())?,
        };
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
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?;
        let file = handles
            .get_mut(&handle.0)
            .ok_or_else(|| stale("DFS file handle is no longer open"))?;
        if file.flags & libc::O_ACCMODE == libc::O_RDONLY {
            return Err(Error::from(std::io::Error::from_raw_os_error(libc::EBADF)));
        }
        let session = file
            .write_session
            .as_mut()
            .ok_or_else(|| invalid("writable DFS handle has no write session"))?;
        let offset = if file.flags & libc::O_APPEND != 0 {
            session.logical_length
        } else {
            offset
        };
        let written = session.builder.write_at(offset, data)?;
        session.logical_length = session
            .logical_length
            .max(offset.saturating_add(written as u64));
        session.dirty = true;
        Ok(written)
    }

    fn flush(&self, _: &RequestContext, handle: FileHandle) -> Result<()> {
        self.commit_handle(handle)
    }

    fn fsync(&self, _: &RequestContext, handle: FileHandle, _: SyncMode) -> Result<()> {
        self.commit_handle(handle)
    }

    fn release(&self, _: &RequestContext, handle: FileHandle) -> Result<()> {
        self.commit_handle(handle)?;
        self.handles
            .lock()
            .map_err(|_| unavailable("DFS handle table is poisoned"))?
            .remove(&handle.0)
            .ok_or_else(|| stale("DFS file handle is no longer open"))?;
        Ok(())
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

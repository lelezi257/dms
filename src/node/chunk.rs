//! Local immutable chunk lifecycle shared by every DFS replica.
//!
//! `ChunkBuilder` accepts stream-like FUSE write fragments. `LocalChunkStore`
//! finalizes one complete byte range with an atomic rename and directory fsync.
//! Replication coordination and Meta receipt construction live above this file.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use afs_error::{Error, Result};

use crate::dfs::{
    ChunkEncoding, ChunkId, ChunkObject, ChunkReceipt, ContentDigest, OperationId, ReplicaAck,
    ReplicaTarget,
};
#[cfg(test)]
use crate::dfs::{ReplicaGroupId, StorageDeviceDescriptor};

#[derive(Debug, Default)]
pub struct ChunkBuilder {
    bytes: Vec<u8>,
}

impl ChunkBuilder {
    pub fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<usize> {
        let offset = usize::try_from(offset).map_err(|_| invalid("chunk offset is too large"))?;
        let end = offset
            .checked_add(data.len())
            .ok_or_else(|| invalid("chunk write range overflow"))?;
        if end > self.bytes.len() {
            self.bytes.resize(end, 0);
        }
        self.bytes[offset..end].copy_from_slice(data);
        Ok(data.len())
    }

    pub fn truncate(&mut self, length: u64) -> Result<()> {
        let length = usize::try_from(length).map_err(|_| invalid("chunk length is too large"))?;
        self.bytes.resize(length, 0);
        Ok(())
    }

    pub fn stage(&self, operation_id: OperationId) -> StagedChunk {
        let content_digest = digest(&self.bytes);
        let chunk = ChunkObject {
            id: ChunkId::new(format!(
                "{}-{}",
                digest_hex(&content_digest),
                self.bytes.len()
            )),
            length: self.bytes.len() as u64,
            content_digest,
            encoding: ChunkEncoding::Raw,
        };
        StagedChunk {
            operation_id,
            chunk,
            bytes: self.bytes.clone(),
        }
    }

    pub fn replace(&mut self, bytes: Vec<u8>) {
        self.bytes = bytes;
    }

    pub fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug)]
pub struct StagedChunk {
    pub operation_id: OperationId,
    pub chunk: ChunkObject,
    bytes: Vec<u8>,
}

impl StagedChunk {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub trait ChunkStore: Send + Sync {
    fn put(&self, staged: StagedChunk) -> Result<ChunkReceipt>;
    fn read_at(&self, chunk_id: &ChunkId, offset: u64, out: &mut [u8]) -> Result<usize>;
    fn read_all(&self, chunk_id: &ChunkId) -> Result<Vec<u8>>;
    fn verify(&self, chunk_id: &ChunkId, expected: &ContentDigest) -> Result<()>;
}

#[derive(Debug)]
pub struct LocalChunkStore {
    node_id: String,
    device_id: String,
    chunks: PathBuf,
    staging: PathBuf,
    next_staging: AtomicU64,
}

impl LocalChunkStore {
    pub fn open(root: impl AsRef<Path>, node_id: impl Into<String>) -> Result<Self> {
        let root = root.as_ref();
        let chunks = root.join("chunks");
        let staging = root.join("staging");
        fs::create_dir_all(&chunks).map_err(Error::from)?;
        fs::create_dir_all(&staging).map_err(Error::from)?;
        Ok(Self {
            node_id: node_id.into(),
            device_id: "local-0".into(),
            chunks,
            staging,
            next_staging: AtomicU64::new(1),
        })
    }

    fn chunk_path(&self, id: &ChunkId) -> PathBuf {
        self.chunks.join(&id.0)
    }

    pub fn persist(
        &self,
        staged: &StagedChunk,
        target: &ReplicaTarget,
        placement_revision: u64,
        placement_epoch: u64,
    ) -> Result<ReplicaAck> {
        if target.node_id != self.node_id || target.device.device_id != self.device_id {
            return Err(invalid(
                "replica target does not identify this local chunk store",
            ));
        }
        let digest_hex = digest_hex(&staged.chunk.content_digest);
        let final_path = self.chunk_path(&staged.chunk.id);
        if !final_path.exists() {
            let temp_path = self.staging.join(format!(
                "{}.{}.{}.tmp",
                safe_id(&staged.operation_id.0),
                digest_hex,
                self.next_staging.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .map_err(Error::from)?;
            file.write_all(&staged.bytes).map_err(Error::from)?;
            file.sync_all().map_err(Error::from)?;
            match fs::rename(&temp_path, &final_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&temp_path);
                }
                Err(error) => return Err(Error::from(error)),
            }
            File::open(&self.chunks)
                .and_then(|directory| directory.sync_all())
                .map_err(Error::from)?;
        }
        self.verify(&staged.chunk.id, &staged.chunk.content_digest)?;
        Ok(ReplicaAck {
            operation_id: staged.operation_id.clone(),
            chunk_id: staged.chunk.id.clone(),
            placement_revision,
            placement_epoch,
            node_id: target.node_id.clone(),
            node_epoch: target.node_epoch,
            device_id: target.device.device_id.clone(),
            device_epoch: target.device.device_epoch,
            catalog_revision: target.device.catalog_revision,
            persisted_bytes: staged.chunk.length,
            verified_digest: staged.chunk.content_digest.clone(),
        })
    }

    pub fn read_at(&self, chunk_id: &ChunkId, offset: u64, out: &mut [u8]) -> Result<usize> {
        let bytes = fs::read(self.chunk_path(chunk_id)).map_err(Error::from)?;
        let actual_digest = digest(&bytes);
        let actual_id = ChunkId::new(format!("{}-{}", digest_hex(&actual_digest), bytes.len()));
        if actual_id != *chunk_id {
            return Err(Error::coded(
                afs_error::NODE_TRANSFER_CORRUPT_DATA,
                format!(
                    "chunk '{}' content does not match its immutable ID",
                    chunk_id.0
                ),
            ));
        }
        let start = usize::try_from(offset).map_err(|_| invalid("chunk offset is too large"))?;
        if start >= bytes.len() {
            return Ok(0);
        }
        let count = out.len().min(bytes.len() - start);
        out[..count].copy_from_slice(&bytes[start..start + count]);
        Ok(count)
    }

    pub fn read_all(&self, chunk_id: &ChunkId) -> Result<Vec<u8>> {
        fs::read(self.chunk_path(chunk_id)).map_err(Error::from)
    }

    pub fn verify(&self, chunk_id: &ChunkId, expected: &ContentDigest) -> Result<()> {
        let bytes = self.read_all(chunk_id)?;
        if digest(&bytes) != *expected {
            return Err(Error::coded(
                afs_error::NODE_TRANSFER_CORRUPT_DATA,
                format!("chunk '{}' digest mismatch", chunk_id.0),
            ));
        }
        Ok(())
    }
}

// Unit tests below the replication layer keep using the concrete local store.
// Production wiring always wraps it in `DfsChunkStore`, where authoritative
// placement and the filesystem-wide replication contract are enforced.
#[cfg(test)]
impl ChunkStore for LocalChunkStore {
    fn put(&self, staged: StagedChunk) -> Result<ChunkReceipt> {
        let target = ReplicaTarget {
            node_id: self.node_id.clone(),
            node_epoch: 1,
            data_endpoint: String::new(),
            device: StorageDeviceDescriptor {
                device_id: self.device_id.clone(),
                device_epoch: 1,
                catalog_revision: 0,
                failure_domain: self.node_id.clone(),
            },
        };
        let ack = self.persist(&staged, &target, 1, 1)?;
        Ok(ChunkReceipt {
            operation_id: staged.operation_id,
            chunk: staged.chunk,
            placement_revision: 1,
            placement_epoch: 1,
            replica_group_id: ReplicaGroupId::new(format!("local:{}", self.node_id)),
            durable_acks: vec![ack],
        })
    }

    fn read_at(&self, chunk_id: &ChunkId, offset: u64, out: &mut [u8]) -> Result<usize> {
        LocalChunkStore::read_at(self, chunk_id, offset, out)
    }

    fn read_all(&self, chunk_id: &ChunkId) -> Result<Vec<u8>> {
        LocalChunkStore::read_all(self, chunk_id)
    }

    fn verify(&self, chunk_id: &ChunkId, expected: &ContentDigest) -> Result<()> {
        LocalChunkStore::verify(self, chunk_id, expected)
    }
}

fn digest(bytes: &[u8]) -> ContentDigest {
    // Two independent FNV-1a lanes provide a dependency-free corruption check
    // for the first local slice. The on-disk digest algorithm remains versioned
    // by ChunkEncoding and can be upgraded before distributed deduplication.
    let mut a = 0xcbf29ce484222325u64;
    let mut b = 0x84222325cbf29ce4u64;
    for (index, byte) in bytes.iter().copied().enumerate() {
        a ^= u64::from(byte);
        a = a.wrapping_mul(0x100000001b3);
        b ^= u64::from(byte) ^ index as u64;
        b = b.wrapping_mul(0x100000001b3);
    }
    let mut out = [0; 16];
    out[..8].copy_from_slice(&a.to_be_bytes());
    out[8..].copy_from_slice(&b.to_be_bytes());
    ContentDigest(out)
}

fn digest_hex(digest: &ContentDigest) -> String {
    digest.0.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn safe_id(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn invalid(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_STORAGE_INVALID, message)
}

//! Crash-safe local storage for immutable DFS chunks.
//!
//! A batch becomes durable in three ordered barriers: every staging file is
//! synced, final names are installed without replacement and the chunk
//! directory is synced, then one LocalCatalog transaction is appended and
//! synced. Only the catalog revision produced by the final barrier may appear
//! in a `ReplicaAck`.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use afs_error::{Error, Result};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::FileExt;

#[cfg(test)]
use crate::dfs::ReplicaGroupId;
use crate::dfs::{
    ChunkEncoding, ChunkId, ChunkObject, ChunkReceipt, ContentDigest, DigestAlgorithm, OperationId,
    ReplicaAck, ReplicaTarget, StorageDeviceDescriptor,
};

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

    pub fn replace(&mut self, bytes: Vec<u8>) {
        self.bytes = bytes;
    }

    pub fn stage(self, operation_id: OperationId) -> StagedChunk {
        StagedChunk::new(operation_id, self.bytes)
    }
}

#[derive(Clone, Debug)]
pub struct StagedChunk {
    pub operation_id: OperationId,
    pub chunk: ChunkObject,
    bytes: Arc<[u8]>,
}

impl StagedChunk {
    pub fn new(operation_id: OperationId, bytes: Vec<u8>) -> Self {
        let bytes: Arc<[u8]> = bytes.into();
        let content_digest = digest(&bytes);
        let chunk = ChunkObject {
            id: ChunkId::new(format!(
                "blake3-{}-{}",
                digest_hex(&content_digest),
                bytes.len()
            )),
            length: bytes.len() as u64,
            content_digest,
            encoding: ChunkEncoding::Raw,
        };
        Self {
            operation_id,
            chunk,
            bytes,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub trait ChunkStore: Send + Sync {
    fn put_batch(&self, staged: Vec<StagedChunk>) -> Result<Vec<ChunkReceipt>>;

    fn put(&self, staged: StagedChunk) -> Result<ChunkReceipt> {
        self.put_batch(vec![staged])?
            .pop()
            .ok_or_else(|| invalid("ChunkStore returned no receipt for one staged Chunk"))
    }

    fn read_at(&self, chunk_id: &ChunkId, offset: u64, out: &mut [u8]) -> Result<usize>;
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LocalChunkState {
    Durable,
    Quarantined,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PhysicalLocation {
    PerChunkFile { relative_path: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PhysicalEncoding {
    Raw,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalChunkRecord {
    pub chunk: ChunkObject,
    pub stored_length: u64,
    pub stored_checksum: ContentDigest,
    pub device_id: String,
    pub device_epoch: u64,
    pub location: PhysicalLocation,
    pub encoding: PhysicalEncoding,
    pub state: LocalChunkState,
    pub catalog_revision: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveryReport {
    pub removed_staging_files: u64,
    pub durable_chunks: u64,
    pub orphan_files: u64,
}

#[derive(Debug)]
pub struct PinnedChunkReader {
    file: File,
    chunk: ChunkObject,
}

impl PinnedChunkReader {
    pub fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<usize> {
        if offset >= self.chunk.length {
            return Ok(0);
        }
        let allowed = usize::try_from((self.chunk.length - offset).min(out.len() as u64))
            .map_err(|_| invalid("Chunk read length is too large"))?;
        read_positioned(&self.file, offset, &mut out[..allowed])
    }
}

#[cfg(unix)]
fn read_positioned(file: &File, offset: u64, out: &mut [u8]) -> Result<usize> {
    file.read_at(out, offset).map_err(Error::from)
}

#[cfg(not(unix))]
fn read_positioned(file: &File, offset: u64, out: &mut [u8]) -> Result<usize> {
    let mut file = file.try_clone().map_err(Error::from)?;
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset)).map_err(Error::from)?;
    file.read(out).map_err(Error::from)
}

#[derive(Debug, Deserialize, Serialize)]
struct CatalogTxn {
    revision: u64,
    records: Vec<LocalChunkRecord>,
    checksum: String,
}

impl CatalogTxn {
    fn new(revision: u64, records: Vec<LocalChunkRecord>) -> Result<Self> {
        let checksum = catalog_checksum(revision, &records)?;
        Ok(Self {
            revision,
            records,
            checksum,
        })
    }

    fn verify(&self) -> Result<()> {
        if self.checksum != catalog_checksum(self.revision, &self.records)? {
            return Err(invalid("local chunk catalog transaction checksum mismatch"));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct LocalCatalog {
    revision: u64,
    records: HashMap<ChunkId, LocalChunkRecord>,
}

impl LocalCatalog {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn recover(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let file = File::open(path).map_err(Error::from)?;
        let mut catalog = Self::default();
        for line in BufReader::new(file).lines() {
            let line = line.map_err(Error::from)?;
            if line.trim().is_empty() {
                continue;
            }
            let txn: CatalogTxn = serde_json::from_str(&line)
                .map_err(|error| invalid(format!("local chunk catalog is corrupt: {error}")))?;
            txn.verify()?;
            if txn.revision != catalog.revision.saturating_add(1) {
                return Err(invalid("local chunk catalog revision is not contiguous"));
            }
            for record in txn.records {
                if record.catalog_revision != txn.revision {
                    return Err(invalid("local chunk record has the wrong catalog revision"));
                }
                catalog.records.insert(record.chunk.id.clone(), record);
            }
            catalog.revision = txn.revision;
        }
        Ok(catalog)
    }
}

#[derive(Debug)]
pub struct LocalChunkStore {
    node_id: String,
    device_id: String,
    device_epoch: u64,
    root: PathBuf,
    chunks: PathBuf,
    staging: PathBuf,
    catalog_path: PathBuf,
    catalog: Mutex<LocalCatalog>,
    next_staging: AtomicU64,
}

impl LocalChunkStore {
    pub fn open(root: impl AsRef<Path>, node_id: impl Into<String>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let chunks = root.join("chunks");
        let staging = root.join("staging");
        fs::create_dir_all(&chunks).map_err(Error::from)?;
        fs::create_dir_all(&staging).map_err(Error::from)?;
        let device_epoch = open_device_epoch(&root)?;
        let catalog_path = root.join("catalog.wal");
        let catalog = LocalCatalog::recover(&catalog_path)?;
        let store = Self {
            node_id: node_id.into(),
            device_id: "local-0".into(),
            device_epoch,
            root,
            chunks,
            staging,
            catalog_path,
            catalog: Mutex::new(catalog),
            next_staging: AtomicU64::new(1),
        };
        store.recover()?;
        Ok(store)
    }

    pub fn device_descriptor(&self) -> Result<StorageDeviceDescriptor> {
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| invalid("local chunk catalog lock is poisoned"))?;
        Ok(StorageDeviceDescriptor {
            device_id: self.device_id.clone(),
            device_epoch: self.device_epoch,
            catalog_revision: catalog.revision,
            failure_domain: self.node_id.clone(),
        })
    }

    pub fn recover(&self) -> Result<RecoveryReport> {
        let mut report = RecoveryReport::default();
        for entry in fs::read_dir(&self.staging).map_err(Error::from)? {
            let entry = entry.map_err(Error::from)?;
            if entry.file_type().map_err(Error::from)?.is_file() {
                fs::remove_file(entry.path()).map_err(Error::from)?;
                report.removed_staging_files = report.removed_staging_files.saturating_add(1);
            }
        }
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| invalid("local chunk catalog lock is poisoned"))?;
        for record in catalog.records.values() {
            if record.state != LocalChunkState::Durable {
                continue;
            }
            if record.device_id != self.device_id
                || record.device_epoch != self.device_epoch
                || record.stored_length != record.chunk.length
                || record.stored_checksum != record.chunk.content_digest
            {
                return Err(invalid(
                    "durable local Chunk record is internally inconsistent",
                ));
            }
            let path = self.path_for_record(record)?;
            if !path.is_file() {
                return Err(invalid(format!(
                    "durable local Chunk '{}' is missing",
                    record.chunk.id.0
                )));
            }
            if path.metadata().map_err(Error::from)?.len() != record.chunk.length {
                return Err(corrupt(&record.chunk.id, "length mismatch during recovery"));
            }
            report.durable_chunks = report.durable_chunks.saturating_add(1);
        }
        for entry in fs::read_dir(&self.chunks).map_err(Error::from)? {
            let entry = entry.map_err(Error::from)?;
            let id = ChunkId::new(entry.file_name().to_string_lossy().into_owned());
            if !catalog.records.contains_key(&id) {
                report.orphan_files = report.orphan_files.saturating_add(1);
            }
        }
        Ok(report)
    }

    fn chunk_path(&self, id: &ChunkId) -> PathBuf {
        self.chunks.join(&id.0)
    }

    fn path_for_record(&self, record: &LocalChunkRecord) -> Result<PathBuf> {
        match &record.location {
            PhysicalLocation::PerChunkFile { relative_path } => {
                if relative_path != &format!("chunks/{}", record.chunk.id.0) {
                    return Err(invalid("local chunk record has a non-canonical path"));
                }
                let path = self.root.join(relative_path);
                if !path.starts_with(&self.chunks) {
                    return Err(invalid("local chunk record escapes the chunk directory"));
                }
                Ok(path)
            }
        }
    }

    pub fn persist_batch(
        &self,
        staged: &[StagedChunk],
        target: &ReplicaTarget,
        placement_revision: u64,
        placement_epoch: u64,
    ) -> Result<Vec<ReplicaAck>> {
        if target.node_id != self.node_id
            || target.device.device_id != self.device_id
            || target.device.device_epoch != self.device_epoch
        {
            return Err(invalid(
                "replica target does not identify this local chunk store",
            ));
        }
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| invalid("local chunk catalog lock is poisoned"))?;
        if catalog.revision < target.device.catalog_revision {
            return Err(invalid(
                "local catalog is older than the placement revision floor",
            ));
        }

        let mut new_chunks = Vec::new();
        let mut installed_chunk_ids = HashSet::new();
        for item in staged {
            if let Some(existing) = catalog.records.get(&item.chunk.id) {
                if existing.chunk != item.chunk || existing.state != LocalChunkState::Durable {
                    return Err(invalid(
                        "immutable Chunk ID collides with another local record",
                    ));
                }
                verify_file(&self.path_for_record(existing)?, &item.chunk)?;
                continue;
            }
            let temp_path = self.staging.join(format!(
                "{}.{}.tmp",
                safe_id(&item.operation_id.0),
                self.next_staging.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .map_err(Error::from)?;
            file.write_all(item.bytes()).map_err(Error::from)?;
            file.sync_all().map_err(Error::from)?;
            let final_path = self.chunk_path(&item.chunk.id);
            match fs::hard_link(&temp_path, &final_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    verify_file(&final_path, &item.chunk)?;
                }
                Err(error) => return Err(Error::from(error)),
            }
            fs::remove_file(&temp_path).map_err(Error::from)?;
            if installed_chunk_ids.insert(item.chunk.id.clone()) {
                new_chunks.push(item.chunk.clone());
            }
        }

        if !new_chunks.is_empty() {
            File::open(&self.chunks)
                .and_then(|directory| directory.sync_all())
                .map_err(Error::from)?;
            let revision = catalog.revision.saturating_add(1);
            let records = new_chunks
                .into_iter()
                .map(|chunk| LocalChunkRecord {
                    location: PhysicalLocation::PerChunkFile {
                        relative_path: format!("chunks/{}", chunk.id.0),
                    },
                    stored_length: chunk.length,
                    stored_checksum: chunk.content_digest.clone(),
                    device_id: self.device_id.clone(),
                    device_epoch: self.device_epoch,
                    chunk,
                    encoding: PhysicalEncoding::Raw,
                    state: LocalChunkState::Durable,
                    catalog_revision: revision,
                })
                .collect::<Vec<_>>();
            append_catalog_txn(
                &self.catalog_path,
                &CatalogTxn::new(revision, records.clone())?,
            )?;
            for record in records {
                catalog.records.insert(record.chunk.id.clone(), record);
            }
            catalog.revision = revision;
        }
        let catalog_revision = catalog.revision;
        Ok(staged
            .iter()
            .map(|item| ReplicaAck {
                operation_id: item.operation_id.clone(),
                chunk_id: item.chunk.id.clone(),
                placement_revision,
                placement_epoch,
                node_id: target.node_id.clone(),
                node_epoch: target.node_epoch,
                device_id: target.device.device_id.clone(),
                device_epoch: target.device.device_epoch,
                catalog_revision,
                persisted_bytes: item.chunk.length,
                verified_digest: item.chunk.content_digest.clone(),
            })
            .collect())
    }

    pub fn persist(
        &self,
        staged: &StagedChunk,
        target: &ReplicaTarget,
        placement_revision: u64,
        placement_epoch: u64,
    ) -> Result<ReplicaAck> {
        self.persist_batch(
            std::slice::from_ref(staged),
            target,
            placement_revision,
            placement_epoch,
        )?
        .pop()
        .ok_or_else(|| invalid("local batch finalize returned no ReplicaAck"))
    }

    pub fn read_at(&self, chunk_id: &ChunkId, offset: u64, out: &mut [u8]) -> Result<usize> {
        self.open_verified(chunk_id)?.read_at(offset, out)
    }

    pub fn open_verified(&self, chunk_id: &ChunkId) -> Result<PinnedChunkReader> {
        let record = self
            .catalog
            .lock()
            .map_err(|_| invalid("local chunk catalog lock is poisoned"))?
            .records
            .get(chunk_id)
            .cloned()
            .ok_or_else(|| invalid(format!("local Chunk '{}' is not cataloged", chunk_id.0)))?;
        if record.state != LocalChunkState::Durable {
            return Err(invalid("local Chunk is not readable"));
        }
        let path = self.path_for_record(&record)?;
        verify_file(&path, &record.chunk)?;
        Ok(PinnedChunkReader {
            file: File::open(path).map_err(Error::from)?,
            chunk: record.chunk,
        })
    }
}

#[cfg(test)]
impl ChunkStore for LocalChunkStore {
    fn put_batch(&self, staged: Vec<StagedChunk>) -> Result<Vec<ChunkReceipt>> {
        let descriptor = self.device_descriptor()?;
        let target = ReplicaTarget {
            node_id: self.node_id.clone(),
            node_epoch: 1,
            data_endpoint: String::new(),
            device: descriptor,
        };
        let acks = self.persist_batch(&staged, &target, 1, 1)?;
        Ok(staged
            .into_iter()
            .zip(acks)
            .map(|(item, ack)| ChunkReceipt {
                operation_id: item.operation_id,
                chunk: item.chunk,
                placement_revision: 1,
                placement_epoch: 1,
                replica_group_id: ReplicaGroupId::new(format!("local:{}", self.node_id)),
                durable_acks: vec![ack],
            })
            .collect())
    }

    fn read_at(&self, chunk_id: &ChunkId, offset: u64, out: &mut [u8]) -> Result<usize> {
        LocalChunkStore::read_at(self, chunk_id, offset, out)
    }
}

fn append_catalog_txn(path: &Path, txn: &CatalogTxn) -> Result<()> {
    let mut encoded = serde_json::to_vec(txn)
        .map_err(|error| invalid(format!("cannot encode local chunk catalog: {error}")))?;
    encoded.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(Error::from)?;
    file.write_all(&encoded).map_err(Error::from)?;
    file.sync_all().map_err(Error::from)?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("local chunk catalog has no parent directory"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(Error::from)
}

fn catalog_checksum(revision: u64, records: &[LocalChunkRecord]) -> Result<String> {
    let encoded = serde_json::to_vec(&(revision, records))
        .map_err(|error| invalid(format!("cannot checksum local chunk catalog: {error}")))?;
    Ok(blake3::hash(&encoded).to_hex().to_string())
}

fn open_device_epoch(root: &Path) -> Result<u64> {
    let path = root.join("device_epoch");
    if path.exists() {
        let value = fs::read_to_string(path).map_err(Error::from)?;
        return value
            .trim()
            .parse::<u64>()
            .map_err(|_| invalid("local chunk device epoch is corrupt"));
    }
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("system clock is before UNIX epoch"))?
        .as_nanos() as u64
        ^ u64::from(std::process::id());
    let temp = root.join("device_epoch.tmp");
    if temp.exists() {
        fs::remove_file(&temp).map_err(Error::from)?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(Error::from)?;
    writeln!(file, "{epoch}").map_err(Error::from)?;
    file.sync_all().map_err(Error::from)?;
    fs::rename(&temp, &path).map_err(Error::from)?;
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(Error::from)?;
    Ok(epoch)
}

fn verify_file(path: &Path, chunk: &ChunkObject) -> Result<()> {
    let mut file = File::open(path).map_err(Error::from)?;
    let metadata = file.metadata().map_err(Error::from)?;
    if metadata.len() != chunk.length {
        return Err(corrupt(&chunk.id, "length mismatch"));
    }
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher).map_err(Error::from)?;
    let actual = ContentDigest {
        algorithm: DigestAlgorithm::Blake3,
        bytes: *hasher.finalize().as_bytes(),
    };
    if actual != chunk.content_digest {
        return Err(corrupt(&chunk.id, "digest mismatch"));
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest {
        algorithm: DigestAlgorithm::Blake3,
        bytes: *blake3::hash(bytes).as_bytes(),
    }
}

fn digest_hex(digest: &ContentDigest) -> String {
    digest
        .bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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

fn corrupt(id: &ChunkId, detail: &str) -> Error {
    Error::coded(
        afs_error::NODE_TRANSFER_CORRUPT_DATA,
        format!("local Chunk '{}' {detail}", id.0),
    )
}

fn invalid(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_STORAGE_INVALID, message)
}

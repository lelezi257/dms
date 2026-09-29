//! DFS fixed-version Chunk read path.
//!
//! DistributedFs resolves a file range into immutable Chunk ranges, then this
//! module chooses local or peer sources. Local reads use verified pinned
//! readers. Peer reads land in a scratch buffer first and publish into the
//! caller buffer only after the whole requested range succeeds.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use afs_error::{Error, Result};

use crate::{
    dfs::{
        ChunkId, ChunkSources, CopyLocation, CopyRole, DfsChunkSourcesReply,
        DfsChunkSourcesRequest, FileVersionId, LayoutRootId, SourceCandidate,
    },
    node::chunk::{LocalChunkStore, PinnedChunkReader},
};

#[derive(Clone, Debug)]
pub struct DfsReadConfig {
    pub max_ops_per_batch: usize,
    pub max_inflight_bytes: u64,
    pub source_cache_ttl: Duration,
}

impl Default for DfsReadConfig {
    fn default() -> Self {
        Self {
            max_ops_per_batch: 128,
            max_inflight_bytes: 8 * 1024 * 1024,
            source_cache_ttl: Duration::from_millis(500),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkReadOp {
    pub chunk_id: ChunkId,
    pub chunk_offset: u64,
    pub length: u64,
    pub output_offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadBatch {
    pub file_version_id: Option<FileVersionId>,
    pub layout_root_id: LayoutRootId,
    pub ops: Vec<ChunkReadOp>,
}

impl ReadBatch {
    pub fn validate_for_output(&self, output_len: usize, config: &DfsReadConfig) -> Result<()> {
        if self.layout_root_id.0.is_empty() {
            return Err(invalid("DFS read batch has no LayoutRoot"));
        }
        if self.ops.len() > config.max_ops_per_batch {
            return Err(invalid("DFS read batch contains too many ranges"));
        }
        let mut total = 0_u64;
        for op in &self.ops {
            if op.length == 0 {
                continue;
            }
            total = total
                .checked_add(op.length)
                .ok_or_else(|| invalid("DFS read batch byte count overflow"))?;
            let length = usize::try_from(op.length)
                .map_err(|_| invalid("DFS read range length is too large"))?;
            let end = op
                .output_offset
                .checked_add(length)
                .ok_or_else(|| invalid("DFS read output range overflow"))?;
            if end > output_len {
                return Err(invalid("DFS read output range exceeds buffer"));
            }
        }
        if total > config.max_inflight_bytes {
            return Err(invalid("DFS read batch exceeds inflight byte budget"));
        }
        Ok(())
    }

    fn source_request(
        &self,
        caller_id: String,
        namespace_id: crate::dfs::NamespaceId,
    ) -> Result<DfsChunkSourcesRequest> {
        let file_version_id = self
            .file_version_id
            .clone()
            .ok_or_else(|| invalid("DFS peer read requires a fixed FileVersion"))?;
        let chunk_ids = self
            .ops
            .iter()
            .map(|op| op.chunk_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(DfsChunkSourcesRequest {
            caller_id,
            namespace_id,
            file_version_id,
            layout_root_id: self.layout_root_id.clone(),
            chunk_ids,
        })
    }
}

pub trait ReadSourceProvider: Send + Sync {
    fn sources_for(&self, request: DfsChunkSourcesRequest) -> Result<DfsChunkSourcesReply>;
}

pub trait ChunkTransfer: Send + Sync {
    fn read_ranges(
        &self,
        source: &SourceCandidate,
        batch: &ReadBatch,
        out: &mut [u8],
    ) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct TransferAttempt {
    pub source: SourceCandidate,
    pub ops: Vec<ChunkReadOp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferCompletion {
    pub bytes: u64,
}

pub struct DfsReadEngine {
    namespace_id: crate::dfs::NamespaceId,
    local_node_id: String,
    local: Arc<LocalChunkStore>,
    source_provider: Arc<dyn ReadSourceProvider>,
    transfer: Arc<dyn ChunkTransfer>,
    config: DfsReadConfig,
    readers: Mutex<HashMap<ChunkId, Arc<PinnedChunkReader>>>,
    source_cache: Mutex<HashMap<ChunkId, CachedSources>>,
}

#[derive(Clone)]
struct CachedSources {
    expires_at: Instant,
    sources: ChunkSources,
}

impl DfsReadEngine {
    pub fn new(
        namespace_id: crate::dfs::NamespaceId,
        local_node_id: String,
        local: Arc<LocalChunkStore>,
        source_provider: Arc<dyn ReadSourceProvider>,
        transfer: Arc<dyn ChunkTransfer>,
        config: DfsReadConfig,
    ) -> Self {
        Self {
            namespace_id,
            local_node_id,
            local,
            source_provider,
            transfer,
            config,
            readers: Mutex::new(HashMap::new()),
            source_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn read_batch(&self, batch: &ReadBatch, out: &mut [u8]) -> Result<()> {
        batch.validate_for_output(out.len(), &self.config)?;
        let mut remote_ops = Vec::new();
        for op in &batch.ops {
            if op.length == 0 {
                continue;
            }
            if self.read_local(op, out).is_err() {
                remote_ops.push(op.clone());
            }
        }
        if remote_ops.is_empty() {
            return Ok(());
        }
        let remote_batch = ReadBatch {
            file_version_id: batch.file_version_id.clone(),
            layout_root_id: batch.layout_root_id.clone(),
            ops: remote_ops,
        };
        let sources = self.sources_for(&remote_batch)?;
        for op in &remote_batch.ops {
            let source_set = sources
                .iter()
                .find(|set| set.chunk_id == op.chunk_id)
                .ok_or_else(|| unavailable("DFS source lookup omitted a requested Chunk"))?;
            self.read_remote_op(op, source_set, out)?;
        }
        Ok(())
    }

    fn read_local(&self, op: &ChunkReadOp, out: &mut [u8]) -> Result<()> {
        let reader = self.local_reader(&op.chunk_id)?;
        let length =
            usize::try_from(op.length).map_err(|_| invalid("DFS read length is too large"))?;
        let end = op
            .output_offset
            .checked_add(length)
            .ok_or_else(|| invalid("DFS read output range overflow"))?;
        let read = reader.read_at(op.chunk_offset, &mut out[op.output_offset..end])?;
        if read != length {
            return Err(corrupt("local Chunk ended before the requested range"));
        }
        Ok(())
    }

    fn local_reader(&self, chunk_id: &ChunkId) -> Result<Arc<PinnedChunkReader>> {
        if let Some(reader) = self
            .readers
            .lock()
            .map_err(|_| unavailable("DFS local reader cache is poisoned"))?
            .get(chunk_id)
            .cloned()
        {
            return Ok(reader);
        }
        let reader = Arc::new(self.local.open_verified(chunk_id)?);
        self.readers
            .lock()
            .map_err(|_| unavailable("DFS local reader cache is poisoned"))?
            .insert(chunk_id.clone(), reader.clone());
        Ok(reader)
    }

    fn sources_for(&self, batch: &ReadBatch) -> Result<Vec<ChunkSources>> {
        let now = Instant::now();
        let mut resolved = Vec::new();
        let mut misses = Vec::new();
        {
            let cache = self
                .source_cache
                .lock()
                .map_err(|_| unavailable("DFS read source cache is poisoned"))?;
            for op in &batch.ops {
                match cache
                    .get(&op.chunk_id)
                    .filter(|entry| entry.expires_at > now)
                {
                    Some(entry) => resolved.push(entry.sources.clone()),
                    None => misses.push(op.clone()),
                }
            }
        }
        if misses.is_empty() {
            return Ok(resolved);
        }
        let request = ReadBatch {
            file_version_id: batch.file_version_id.clone(),
            layout_root_id: batch.layout_root_id.clone(),
            ops: misses,
        }
        .source_request(self.local_node_id.clone(), self.namespace_id.clone())?;
        let fresh = self.source_provider.sources_for(request)?.chunks;
        {
            let mut cache = self
                .source_cache
                .lock()
                .map_err(|_| unavailable("DFS read source cache is poisoned"))?;
            let expires_at = now + self.config.source_cache_ttl;
            for set in &fresh {
                cache.insert(
                    set.chunk_id.clone(),
                    CachedSources {
                        expires_at,
                        sources: set.clone(),
                    },
                );
            }
        }
        resolved.extend(fresh);
        Ok(resolved)
    }

    fn read_remote_op(
        &self,
        op: &ChunkReadOp,
        sources: &ChunkSources,
        out: &mut [u8],
    ) -> Result<()> {
        let length =
            usize::try_from(op.length).map_err(|_| invalid("DFS read length is too large"))?;
        let mut last_error = None;
        for source in sources.sources.iter().filter(|source| {
            source.state.is_readable()
                && matches!(
                    source.role,
                    CopyRole::DurableReplica | CopyRole::VerifiedCache
                )
                && !matches!(
                    source.location,
                    CopyLocation::Node { ref node_id, .. } if node_id == &self.local_node_id
                )
        }) {
            let mut scratch = vec![0; length];
            let scratch_batch = ReadBatch {
                file_version_id: Some(source.read_grant.file_version_id.clone()),
                layout_root_id: source.read_grant.layout_root_id.clone(),
                ops: vec![ChunkReadOp {
                    output_offset: 0,
                    ..op.clone()
                }],
            };
            match self
                .transfer
                .read_ranges(source, &scratch_batch, &mut scratch)
            {
                Ok(()) => {
                    let end = op
                        .output_offset
                        .checked_add(length)
                        .ok_or_else(|| invalid("DFS read output range overflow"))?;
                    out[op.output_offset..end].copy_from_slice(&scratch);
                    return Ok(());
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| unavailable("DFS read has no usable source")))
    }
}

pub struct UnimplementedReadSourceProvider;

impl ReadSourceProvider for UnimplementedReadSourceProvider {
    fn sources_for(&self, _request: DfsChunkSourcesRequest) -> Result<DfsChunkSourcesReply> {
        Err(Error::coded(
            afs_error::NODE_TRANSFER_UNSUPPORTED,
            "DFS read source lookup is not wired",
        ))
    }
}

pub struct UnimplementedChunkTransfer;

impl ChunkTransfer for UnimplementedChunkTransfer {
    fn read_ranges(
        &self,
        _source: &SourceCandidate,
        _batch: &ReadBatch,
        _out: &mut [u8],
    ) -> Result<()> {
        Err(Error::coded(
            afs_error::NODE_TRANSFER_UNSUPPORTED,
            "DFS peer range read is not wired",
        ))
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_TRANSFER_INVALID, message)
}

fn unavailable(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_TRANSFER_UNAVAILABLE, message)
}

fn corrupt(message: impl Into<String>) -> Error {
    Error::coded(afs_error::NODE_TRANSFER_CORRUPT_DATA, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dfs::{CopyId, CopyState, DfsReadGrant, OperationId},
        node::chunk::{ChunkBuilder, ChunkStore},
    };

    #[derive(Default)]
    struct StaticSources {
        reply: Mutex<DfsChunkSourcesReply>,
        calls: Mutex<u64>,
    }

    impl StaticSources {
        fn new(reply: DfsChunkSourcesReply) -> Self {
            Self {
                reply: Mutex::new(reply),
                calls: Mutex::new(0),
            }
        }
    }

    impl ReadSourceProvider for StaticSources {
        fn sources_for(&self, _request: DfsChunkSourcesRequest) -> Result<DfsChunkSourcesReply> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.reply.lock().unwrap().clone())
        }
    }

    struct StaticTransfer {
        bytes: Vec<u8>,
        fail_first: Mutex<bool>,
    }

    impl ChunkTransfer for StaticTransfer {
        fn read_ranges(
            &self,
            _source: &SourceCandidate,
            batch: &ReadBatch,
            out: &mut [u8],
        ) -> Result<()> {
            let mut fail_first = self.fail_first.lock().unwrap();
            if *fail_first {
                *fail_first = false;
                return Err(unavailable("injected peer failure"));
            }
            let op = batch.ops.first().unwrap();
            let length = usize::try_from(op.length).unwrap();
            out[..length].copy_from_slice(&self.bytes[..length]);
            Ok(())
        }
    }

    fn source(copy: &str, chunk_id: &ChunkId, node_id: &str) -> SourceCandidate {
        SourceCandidate {
            copy_id: CopyId::new(copy),
            chunk_id: chunk_id.clone(),
            role: CopyRole::DurableReplica,
            state: CopyState::Ready,
            location: CopyLocation::Node {
                node_id: node_id.into(),
                node_epoch: 1,
                device_id: "local-0".into(),
                device_epoch: 1,
                catalog_revision: 1,
            },
            data_endpoint: format!("http://{node_id}"),
            load_hint: 0,
            read_grant: DfsReadGrant {
                namespace_id: crate::dfs::NamespaceId::new("default"),
                file_version_id: FileVersionId::new("version"),
                layout_root_id: LayoutRootId::new("layout"),
                caller_node_id: "node-a".into(),
                caller_node_epoch: 1,
                expires_at_unix_ms: 1,
                fence: 1,
                token: "test-token".into(),
            },
        }
    }

    #[test]
    fn local_read_uses_verified_reader_without_source_lookup() {
        let temp = tempfile::tempdir().unwrap();
        let local = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
        let mut builder = ChunkBuilder::default();
        builder.replace(b"abcdef".to_vec());
        let staged = builder.stage(OperationId::new("op"));
        let chunk_id = staged.chunk.id.clone();
        local.put(staged).unwrap();
        let sources = Arc::new(StaticSources::default());
        let engine = DfsReadEngine::new(
            crate::dfs::NamespaceId::new("default"),
            "node-a".into(),
            local,
            sources.clone(),
            Arc::new(UnimplementedChunkTransfer),
            DfsReadConfig::default(),
        );
        let mut out = [0; 3];
        engine
            .read_batch(
                &ReadBatch {
                    file_version_id: Some(FileVersionId::new("version")),
                    layout_root_id: LayoutRootId::new("layout"),
                    ops: vec![ChunkReadOp {
                        chunk_id,
                        chunk_offset: 2,
                        length: 3,
                        output_offset: 0,
                    }],
                },
                &mut out,
            )
            .unwrap();
        assert_eq!(&out, b"cde");
        assert_eq!(*sources.calls.lock().unwrap(), 0);
    }

    #[test]
    fn remote_read_uses_scratch_and_falls_back_between_sources() {
        let temp = tempfile::tempdir().unwrap();
        let local = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
        let chunk_id = ChunkId::new("missing");
        let sources = Arc::new(StaticSources::new(DfsChunkSourcesReply {
            revision: 1,
            chunks: vec![ChunkSources {
                chunk_id: chunk_id.clone(),
                sources: vec![
                    source("copy-b", &chunk_id, "node-b"),
                    source("copy-c", &chunk_id, "node-c"),
                ],
            }],
        }));
        let engine = DfsReadEngine::new(
            crate::dfs::NamespaceId::new("default"),
            "node-a".into(),
            local,
            sources,
            Arc::new(StaticTransfer {
                bytes: b"remote".to_vec(),
                fail_first: Mutex::new(true),
            }),
            DfsReadConfig::default(),
        );
        let mut out = [9; 8];
        engine
            .read_batch(
                &ReadBatch {
                    file_version_id: Some(FileVersionId::new("version")),
                    layout_root_id: LayoutRootId::new("layout"),
                    ops: vec![ChunkReadOp {
                        chunk_id,
                        chunk_offset: 0,
                        length: 6,
                        output_offset: 1,
                    }],
                },
                &mut out,
            )
            .unwrap();
        assert_eq!(&out, &[9, b'r', b'e', b'm', b'o', b't', b'e', 9]);
    }
}

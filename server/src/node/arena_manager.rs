//! Physical Region/Allocation ownership and stale-handle-safe reclamation.
//!
//! `ArenaManager` is the only module allowed to own mappings, slots and block
//! locations. Object keys and versions never enter its allocator API.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    os::fd::OwnedFd,
    path::PathBuf,
    time::{Duration, Instant},
};

use super::metrics::{ArenaMetricsSnapshot, NodeMetrics, ShmFdGrantResult, StagingReclaimReason};
use dms_shm::{BrokerToken, FdGrant, FdRequest, SharedRegion, ShmError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AllocationHandle {
    pub(crate) region_id: u64,
    pub(crate) offset: u64,
    pub(crate) length: u64,
    pub(crate) capacity: u64,
    pub(crate) allocation_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArenaReadTicket {
    pub(crate) handle: AllocationHandle,
    pub(crate) offset: u64,
    pub(crate) length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArenaError {
    EmptyPayload,
    CapacityExhausted,
    UnknownTransfer,
    UnknownStaging,
    UnknownBlock,
    StagingNotWritable,
    ReceiptConflict,
    LengthMismatch,
    RangeOutOfBounds,
    StaleHandle,
    RegionOverflow,
    RegionCreateFailed,
    UnknownRegion,
    SharedMemoryUnavailable,
    RegionAccessDenied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReclaimReport {
    pub(crate) reclaimed_bytes: u64,
    pub(crate) remaining_retired_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostStagingState {
    Writable,
    Sealed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostAllocation {
    pub(crate) staging_id: u64,
    pub(crate) transfer_id: u64,
    pub(crate) allocation_id: u64,
    pub(crate) length: u64,
    pub(crate) target: HostAllocationTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HostAllocationTarget {
    Grpc,
    Shm(HostShmDescriptor),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostShmDescriptor {
    pub(crate) region_id: u64,
    pub(crate) offset: u64,
    pub(crate) length: u64,
    pub(crate) allocation_id: u64,
    pub(crate) view_epoch: Option<u64>,
    pub(crate) transfer_id: u64,
    pub(crate) release_token: Vec<u8>,
}

/// One short-lived authorization for mapping a whole Region. It is issued
/// only when an SDK cache misses; ordinary slice descriptors never carry it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostRegionGrant {
    pub(crate) region_id: u64,
    pub(crate) region_length: u64,
    pub(crate) fd_token: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostReceipt {
    pub(crate) transfer_id: u64,
    pub(crate) length: u64,
    pub(crate) digest: Vec<u8>,
    pub(crate) allocation_id: u64,
    pub(crate) release_token: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReleasedWriteAllocation {
    pub(crate) allocation_id: u64,
    pub(crate) release_token: Vec<u8>,
}

struct HostStaging {
    session_id: u64,
    transfer_id: u64,
    handle: AllocationHandle,
    state: HostStagingState,
    digest: Vec<u8>,
    created_at: Instant,
}

struct HostBlock {
    handle: AllocationHandle,
    // 提交时从真实 bytes 计算/验证的不可变身份；随 Block 一起释放，不另设缓存表。
    digest: Vec<u8>,
    committed_version: Option<u64>,
}

struct WriteExportLease {
    session_id: u64,
    release_token: Vec<u8>,
}

struct Region {
    id: u64,
    group_id: u64,
    backing: RegionBacking,
    next_offset: u64,
    free: Vec<FreeSlot>,
}

/// Policy boundary above physical Regions.
///
/// R1 routes every Session into the default Host-memory group. Keeping this
/// boundary explicit prevents tenant/security/quota policy from being baked
/// into Region IDs or Client descriptors when multi-tenant routing arrives.
struct RegionGroup {
    capacity_bytes: u64,
    resident_bytes: u64,
}

const DEFAULT_REGION_GROUP_ID: u64 = 1;

enum RegionBacking {
    Private(Vec<u8>),
    Shared(SharedRegion),
}

impl Region {
    fn len(&self) -> usize {
        match &self.backing {
            RegionBacking::Private(bytes) => bytes.len(),
            RegionBacking::Shared(region) => region.len(),
        }
    }

    fn write_at(&mut self, offset: usize, bytes: &[u8]) -> Result<(), ArenaError> {
        match &mut self.backing {
            RegionBacking::Private(storage) => {
                let end = offset
                    .checked_add(bytes.len())
                    .ok_or(ArenaError::RegionOverflow)?;
                let Some(target) = storage.get_mut(offset..end) else {
                    return Err(ArenaError::RangeOutOfBounds);
                };
                target.copy_from_slice(bytes);
                Ok(())
            }
            RegionBacking::Shared(region) => region
                .write_at(offset, bytes)
                .map_err(|_| ArenaError::RangeOutOfBounds),
        }
    }

    fn read_at(&self, offset: usize, len: usize) -> Result<Vec<u8>, ArenaError> {
        match &self.backing {
            RegionBacking::Private(storage) => {
                let end = offset.checked_add(len).ok_or(ArenaError::RegionOverflow)?;
                storage
                    .get(offset..end)
                    .map(<[u8]>::to_vec)
                    .ok_or(ArenaError::RangeOutOfBounds)
            }
            RegionBacking::Shared(region) => region
                .read_at(offset, len)
                .map_err(|_| ArenaError::RangeOutOfBounds),
        }
    }

    fn borrow_at(&self, offset: usize, len: usize) -> Result<&[u8], ArenaError> {
        match &self.backing {
            RegionBacking::Private(storage) => {
                let end = offset.checked_add(len).ok_or(ArenaError::RegionOverflow)?;
                storage.get(offset..end).ok_or(ArenaError::RangeOutOfBounds)
            }
            RegionBacking::Shared(region) => {
                // SAFETY: Arena 是状态唯一 owner，借用只用于同步读取/校验。
                // 可信 SDK 完成写入后才发 receipt（SHM 不依赖 staging 的 Sealed 状态）；
                // 已发布 Block 不变，导出后退休的 allocation 不复用。
                // 局部 &self 借用阻止本 owner 同时写入，调用处不让借用跨 await 或逃出 Arena。
                unsafe { region.as_slice(offset, len) }.map_err(|_| ArenaError::RangeOutOfBounds)
            }
        }
    }

    fn duplicate_fd(&self) -> Option<OwnedFd> {
        match &self.backing {
            RegionBacking::Private(_) => None,
            RegionBacking::Shared(region) => {
                // SAFETY: Arena 只向已获 descriptor 的可信 SDK Session 授权。
                // SDK 独占 staging，写完且释放借用后才提交 receipt；Node 此后才读。
                // 已发布块不可变，导出后退休的范围隔离，旧写不能碰到新 allocation。
                // 此合同不把恶意拥有整个 memfd 的本机进程当成隔离租户。
                unsafe { region.duplicate_fd().ok() }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct FreeSlot {
    offset: u64,
    capacity: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArenaStats {
    pub(crate) capacity_bytes: u64,
    pub(crate) logical_bytes: u64,
    pub(crate) allocated_bytes: u64,
    pub(crate) resident_bytes: u64,
    pub(crate) free_slot_bytes: u64,
    pub(crate) largest_free_slot_bytes: u64,
    pub(crate) fragmentation_bytes: u64,
    pub(crate) staging_count: usize,
    pub(crate) block_count: usize,
    pub(crate) reclaim_count: u64,
    pub(crate) quarantined_bytes: u64,
}

/// Runtime Host-memory owner used by the Node actor.
///
/// Allocation reserves a stable slot in a private or shared region immediately.
/// Upload only fills that slot, then sealing records a digest. The transfer
/// index makes the payload path O(1); globally monotonic Allocation IDs reject
/// stale receipts when a physical range is reused.
pub(crate) struct ArenaManager {
    capacity_bytes: u64,
    // Region 是批量向 OS 获取的 backing；Slot 才是每次 value/patch 的申请粒度。
    region_size_bytes: u64,
    logical_bytes: u64,
    allocated_bytes: u64,
    resident_bytes: u64,
    next_region_id: u64,
    next_staging_id: u64,
    next_transfer_id: u64,
    next_allocation_id: u64,
    regions: Vec<Region>,
    region_groups: HashMap<u64, RegionGroup>,
    session_groups: HashMap<u64, u64>,
    /// Fine-grained export authorization. RegionGroup establishes the policy
    /// domain; this set proves the Session actually received a slice in the
    /// Region before it can request the memfd.
    session_regions: HashMap<u64, HashSet<u64>>,
    staging: HashMap<u64, HostStaging>,
    transfer_index: HashMap<u64, u64>,
    blocks: HashMap<Vec<u8>, HostBlock>,
    // allocation id 索引同时验证旧 ticket，避免每次 range read 扫描对象表。
    live_allocations: HashMap<u64, AllocationHandle>,
    write_exports: HashMap<u64, WriteExportLease>,
    quarantined_bytes: u64,
    #[cfg(test)]
    fail_next_region_creation: bool,
    staging_ttl: Duration,
    reclaim_count: u64,
    metrics: NodeMetrics,
    shared_fd_broker: Option<SharedFdBroker>,
}

#[derive(Clone)]
pub(crate) struct SharedFdBroker {
    pub(crate) path: PathBuf,
    server: dms_shm::FdBrokerServer,
}

impl SharedFdBroker {
    pub(crate) fn bind(path: PathBuf) -> Result<Self, ShmError> {
        Ok(Self {
            server: dms_shm::FdBrokerServer::bind(&path)?,
            path,
        })
    }

    pub(crate) fn serve_one(&self) -> Result<(), ShmError> {
        self.server.serve_one()
    }

    #[cfg(test)]
    pub(crate) fn served_count(&self) -> u64 {
        self.server.served_count()
    }

    fn reap_expired(&self) -> Result<usize, ShmError> {
        self.server.reap_expired()
    }

    fn register(&self, request: FdRequest, fd: OwnedFd) -> Result<(), ShmError> {
        self.server.register(FdGrant::new(request, fd))
    }
}

impl ArenaManager {
    #[cfg(test)]
    pub(crate) fn new(capacity_bytes: u64, staging_ttl: Duration) -> Self {
        let registry = dms_metrics::registry();
        let metrics = NodeMetrics::register(&registry).expect("test Arena metrics");
        Self::with_metrics(capacity_bytes, staging_ttl, metrics)
    }

    pub(crate) fn with_metrics(
        capacity_bytes: u64,
        staging_ttl: Duration,
        metrics: NodeMetrics,
    ) -> Self {
        metrics.set_arena_capacity(capacity_bytes);
        let arena = Self {
            capacity_bytes,
            region_size_bytes: crate::config::DEFAULT_REGION_SIZE_BYTES,
            logical_bytes: 0,
            allocated_bytes: 0,
            resident_bytes: 0,
            next_region_id: 1,
            next_staging_id: 1,
            next_transfer_id: 1,
            next_allocation_id: 1,
            regions: Vec::new(),
            region_groups: HashMap::from([(
                DEFAULT_REGION_GROUP_ID,
                RegionGroup {
                    capacity_bytes,
                    resident_bytes: 0,
                },
            )]),
            session_groups: HashMap::new(),
            session_regions: HashMap::new(),
            staging: HashMap::new(),
            transfer_index: HashMap::new(),
            blocks: HashMap::new(),
            live_allocations: HashMap::new(),
            write_exports: HashMap::new(),
            quarantined_bytes: 0,
            #[cfg(test)]
            fail_next_region_creation: false,
            staging_ttl,
            reclaim_count: 0,
            metrics,
            shared_fd_broker: None,
        };
        arena.record_state();
        arena
    }

    pub(crate) fn enable_shared_region(&mut self, broker: SharedFdBroker) {
        self.shared_fd_broker = Some(broker);
    }

    /// 启动时设置扩容目标；不移动已经分配的 Region，不能改变既有 FD/mmap 身份。
    pub(crate) fn set_region_size(&mut self, bytes: u64) {
        self.region_size_bytes = bytes;
    }

    /// Bind a Session to its default RegionGroup before any allocation.
    ///
    /// Inline writes do not call `allocate`, but a later SHM read still needs
    /// Region authorization. Registration therefore belongs to OpenSession,
    /// rather than being an accidental side effect of the first staging alloc.
    pub(crate) fn register_session(&mut self, session_id: u64) {
        self.session_groups
            .entry(session_id)
            .or_insert(DEFAULT_REGION_GROUP_ID);
    }

    pub(crate) fn allocate(
        &mut self,
        session_id: u64,
        length: u64,
    ) -> Result<HostAllocation, ArenaError> {
        let started = Instant::now();
        let mut allocation_metric = self.metrics.begin_arena_allocation();
        self.reclaim_expired();
        if length == 0 {
            self.record_failure("arena.allocate.failed");
            return Err(ArenaError::EmptyPayload);
        }
        let group_id = *self
            .session_groups
            .entry(session_id)
            .or_insert(DEFAULT_REGION_GROUP_ID);
        let handle = match self.allocate_slot(group_id, length) {
            Ok(handle) => handle,
            Err(error) => {
                if error == ArenaError::CapacityExhausted {
                    allocation_metric.capacity_exhausted();
                }
                return Err(error);
            }
        };
        let allocation = HostAllocation {
            staging_id: self.next_staging_id,
            transfer_id: self.next_transfer_id,
            allocation_id: handle.allocation_id,
            length,
            // Capability negotiation lives in NodeState: a TCP session must not
            // receive this process' fd broker path just because the Arena uses
            // shared backing internally.
            target: HostAllocationTarget::Grpc,
        };
        self.next_staging_id += 1;
        self.next_transfer_id += 1;
        self.logical_bytes += length;
        self.allocated_bytes += handle.capacity;
        self.live_allocations.insert(handle.allocation_id, handle);
        self.transfer_index
            .insert(allocation.transfer_id, allocation.staging_id);
        self.staging.insert(
            allocation.staging_id,
            HostStaging {
                session_id,
                transfer_id: allocation.transfer_id,
                handle,
                state: HostStagingState::Writable,
                digest: Vec::new(),
                created_at: Instant::now(),
            },
        );
        let _ = started;
        allocation_metric.success();
        self.record_state();
        Ok(allocation)
    }

    pub(crate) fn upload(
        &mut self,
        transfer_id: u64,
        bytes: &[u8],
    ) -> Result<HostReceipt, ArenaError> {
        self.reclaim_expired();
        let staging_id = *self
            .transfer_index
            .get(&transfer_id)
            .ok_or(ArenaError::UnknownTransfer)?;
        let handle = {
            let staging = self
                .staging
                .get(&staging_id)
                .ok_or(ArenaError::UnknownStaging)?;
            if staging.state != HostStagingState::Writable {
                return Err(ArenaError::StagingNotWritable);
            }
            if staging.handle.length != bytes.len() as u64 {
                return Err(ArenaError::LengthMismatch);
            }
            staging.handle
        };
        let digest = digest(bytes);
        self.copy_into_slot(handle, bytes)?;
        let staging = self
            .staging
            .get_mut(&staging_id)
            .ok_or(ArenaError::UnknownStaging)?;
        if staging.state != HostStagingState::Writable {
            return Err(ArenaError::StagingNotWritable);
        }
        staging.digest = digest;
        staging.state = HostStagingState::Sealed;
        Ok(HostReceipt {
            transfer_id,
            length: bytes.len() as u64,
            digest: staging.digest.clone(),
            allocation_id: staging.handle.allocation_id,
            release_token: Vec::new(),
        })
    }

    pub(crate) fn commit_staging(
        &mut self,
        session_id: u64,
        staging_id: u64,
        receipt: &HostReceipt,
        block_id: Vec<u8>,
    ) -> Result<(), ArenaError> {
        let staging = self
            .staging
            .get(&staging_id)
            .ok_or(ArenaError::UnknownStaging)?;
        if staging.session_id != session_id
            || staging.transfer_id != receipt.transfer_id
            || staging.handle.allocation_id != receipt.allocation_id
            || staging.handle.length != receipt.length
        {
            return Err(ArenaError::ReceiptConflict);
        }
        let staging_handle = staging.handle;
        let current = self
            .slot_bytes_checked(staging_handle)
            .ok_or(ArenaError::StaleHandle)?;
        if current.len() as u64 != receipt.length || digest(current) != receipt.digest {
            return Err(ArenaError::ReceiptConflict);
        }
        self.accept_receipt_release(
            session_id,
            staging_handle.allocation_id,
            &receipt.release_token,
        )?;
        if let Some(block) = self.blocks.get(&block_id) {
            let existing_matches = self.slot_bytes_checked(block.handle).is_some_and(|bytes| {
                bytes.len() as u64 == receipt.length && digest(bytes) == receipt.digest
            });
            let staging = self.staging.remove(&staging_id).expect("checked staging");
            self.transfer_index.remove(&staging.transfer_id);
            self.release_handle(staging.handle);
            self.record_state();
            // Idempotent retry after a prior publish must not leak the newly
            // allocated staging slot. If the same block id points at different
            // bytes, reject it instead of silently letting metadata describe
            // bytes that Arena does not actually own.
            return existing_matches
                .then_some(())
                .ok_or(ArenaError::ReceiptConflict);
        }
        let staging = self.staging.remove(&staging_id).expect("checked staging");
        self.transfer_index.remove(&staging.transfer_id);
        self.blocks.insert(
            block_id,
            HostBlock {
                handle: staging.handle,
                digest: receipt.digest.clone(),
                committed_version: None,
            },
        );
        self.record_state();
        Ok(())
    }

    pub(crate) fn take_staging_bytes(
        &mut self,
        session_id: u64,
        staging_id: u64,
        receipt: &HostReceipt,
    ) -> Result<Vec<u8>, ArenaError> {
        let staging = self
            .staging
            .get(&staging_id)
            .ok_or(ArenaError::UnknownStaging)?;
        if staging.session_id != session_id
            || staging.transfer_id != receipt.transfer_id
            || staging.handle.allocation_id != receipt.allocation_id
            || staging.handle.length != receipt.length
        {
            return Err(ArenaError::ReceiptConflict);
        }
        let staging_handle = staging.handle;
        let bytes = self
            .slot_bytes_checked(staging_handle)
            .ok_or(ArenaError::StaleHandle)?;
        if bytes.len() as u64 != receipt.length || digest(bytes) != receipt.digest {
            return Err(ArenaError::ReceiptConflict);
        }
        // 这个接口把 bytes 交出 Arena，必须在释放 staging 前创建必要的 owned 副本；
        // 之后才能可变更新写租约状态。
        let bytes = bytes.to_vec();
        self.accept_receipt_release(
            session_id,
            staging_handle.allocation_id,
            &receipt.release_token,
        )?;
        let staging = self.staging.remove(&staging_id).expect("checked staging");
        self.transfer_index.remove(&staging.transfer_id);
        self.release_handle(staging.handle);
        self.record_state();
        Ok(bytes)
    }

    #[cfg(test)]
    pub(crate) fn commit_inline(
        &mut self,
        block_id: Vec<u8>,
        bytes: Vec<u8>,
    ) -> Result<(), ArenaError> {
        let checksum = digest(&bytes);
        self.commit_inline_with_verified_digest(block_id, bytes, checksum)
    }

    /// Node owner 已在写入/peer 完整校验处计算真实摘要；随 owned bytes 一起移交，
    /// 避免 Arena 为保存身份再次遍历整个 payload。调用者不得传 wire 上未验证的摘要。
    pub(crate) fn commit_inline_with_verified_digest(
        &mut self,
        block_id: Vec<u8>,
        bytes: Vec<u8>,
        verified_digest: Vec<u8>,
    ) -> Result<(), ArenaError> {
        let length = bytes.len() as u64;
        if length == 0 {
            return Err(ArenaError::EmptyPayload);
        }
        if verified_digest.is_empty() {
            return Err(ArenaError::ReceiptConflict);
        }
        if let Some(block) = self.blocks.get(&block_id) {
            return self
                .slot_bytes_checked(block.handle)
                .is_some_and(|existing| {
                    existing.len() as u64 == length && existing == bytes.as_slice()
                })
                .then_some(())
                .ok_or(ArenaError::ReceiptConflict);
        }
        let handle = self.allocate_slot(DEFAULT_REGION_GROUP_ID, length)?;
        self.copy_into_slot(handle, &bytes)?;
        self.logical_bytes += length;
        self.allocated_bytes += handle.capacity;
        self.live_allocations.insert(handle.allocation_id, handle);
        self.blocks.insert(
            block_id,
            HostBlock {
                handle,
                digest: verified_digest,
                committed_version: None,
            },
        );
        self.record_state();
        Ok(())
    }

    pub(crate) fn mark_committed(&mut self, block_id: &[u8], version: u64) {
        if let Some(block) = self.blocks.get_mut(block_id) {
            block.committed_version = Some(version);
        }
    }

    pub(crate) fn release_write_allocation(
        &mut self,
        session_id: u64,
        released: &ReleasedWriteAllocation,
    ) -> Result<(), ArenaError> {
        let Some(lease) = self.write_exports.get(&released.allocation_id) else {
            // Duplicate/lost-response releases are idempotent. Allocation IDs are
            // monotonic, so this cannot free a newer object occupying the same
            // physical slot.
            return Ok(());
        };
        if lease.session_id != session_id || lease.release_token != released.release_token {
            return Err(ArenaError::ReceiptConflict);
        }
        self.write_exports.remove(&released.allocation_id);
        Ok(())
    }

    fn accept_receipt_release(
        &mut self,
        session_id: u64,
        allocation_id: u64,
        release_token: &[u8],
    ) -> Result<(), ArenaError> {
        if release_token.is_empty() {
            return Ok(());
        }
        self.release_write_allocation(
            session_id,
            &ReleasedWriteAllocation {
                allocation_id,
                release_token: release_token.to_vec(),
            },
        )
    }

    pub(crate) fn block_has_unreturned_write(&self, block_id: &[u8]) -> bool {
        self.blocks
            .get(block_id)
            .is_some_and(|block| self.write_exports.contains_key(&block.handle.allocation_id))
    }

    pub(crate) fn session_has_unreturned_write(&self, session_id: u64) -> bool {
        self.write_exports
            .values()
            .any(|lease| lease.session_id == session_id)
    }

    pub(crate) fn block_allocation_id(&self, block_id: &[u8]) -> Option<u64> {
        self.blocks
            .get(block_id)
            .map(|block| block.handle.allocation_id)
    }

    /// 借用 Block 的提交身份；不读取/复制 payload，也不重新计算 checksum。
    pub(crate) fn block_length_and_digest(&self, block_id: &[u8]) -> Option<(u64, &[u8])> {
        self.blocks
            .get(block_id)
            .map(|block| (block.handle.length, block.digest.as_slice()))
    }

    pub(crate) fn read_bytes(&self, block_id: &[u8]) -> Option<Vec<u8>> {
        self.blocks
            .get(block_id)
            .and_then(|block| self.slot_bytes_checked(block.handle))
            .map(<[u8]>::to_vec)
    }

    pub(crate) fn open_read(
        &self,
        block_id: &[u8],
        range: Option<(u64, u64)>,
    ) -> Result<(ArenaReadTicket, u64), ArenaError> {
        let block = self.blocks.get(block_id).ok_or(ArenaError::UnknownBlock)?;
        let handle = block.handle;
        let (offset, length) = range.unwrap_or((0, handle.length));
        offset
            .checked_add(length)
            .filter(|end| *end <= handle.length)
            .ok_or(ArenaError::RangeOutOfBounds)?;
        Ok((
            ArenaReadTicket {
                handle,
                offset,
                length,
            },
            handle.length,
        ))
    }

    pub(crate) fn read_ticket(&self, ticket: ArenaReadTicket) -> Result<Vec<u8>, ArenaError> {
        if self.live_allocations.get(&ticket.handle.allocation_id) != Some(&ticket.handle) {
            return Err(ArenaError::StaleHandle);
        }
        ticket
            .offset
            .checked_add(ticket.length)
            .filter(|end| *end <= ticket.handle.length)
            .ok_or(ArenaError::RangeOutOfBounds)?;
        let offset = ticket
            .handle
            .offset
            .checked_add(ticket.offset)
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or(ArenaError::RangeOutOfBounds)?;
        let length = usize::try_from(ticket.length).map_err(|_| ArenaError::RangeOutOfBounds)?;
        let region_index = ticket
            .handle
            .region_id
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or(ArenaError::StaleHandle)?;
        let region = self
            .regions
            .get(region_index)
            .ok_or(ArenaError::StaleHandle)?;
        // 只复制所请求的区间，不先复制整块再截取。
        region.read_at(offset, length)
    }

    pub(crate) fn shm_descriptor_for_read(
        &mut self,
        session_id: u64,
        ticket: ArenaReadTicket,
        transfer_id: u64,
        view_epoch: u64,
    ) -> Result<Option<HostShmDescriptor>, ArenaError> {
        if self.live_allocations.get(&ticket.handle.allocation_id) != Some(&ticket.handle) {
            return Err(ArenaError::StaleHandle);
        }
        ticket
            .offset
            .checked_add(ticket.length)
            .filter(|end| *end <= ticket.handle.length)
            .ok_or(ArenaError::RangeOutOfBounds)?;
        let offset = ticket
            .handle
            .offset
            .checked_add(ticket.offset)
            .ok_or(ArenaError::RangeOutOfBounds)?;
        Ok(self
            .shared_descriptor(
                session_id,
                ticket.handle,
                transfer_id,
                Some(view_epoch),
                Vec::new(),
            )
            .map(|mut descriptor| {
                descriptor.offset = offset;
                descriptor.length = ticket.length;
                descriptor
            }))
    }

    pub(crate) fn shm_descriptor_for_staging(
        &mut self,
        session_id: u64,
        staging_id: u64,
    ) -> Result<Option<HostShmDescriptor>, ArenaError> {
        let staging = self
            .staging
            .get(&staging_id)
            .ok_or(ArenaError::UnknownStaging)?;
        if staging.session_id != session_id {
            return Err(ArenaError::UnknownStaging);
        }
        let handle = staging.handle;
        let transfer_id = staging.transfer_id;
        let token = shm_token().ok_or(ArenaError::SharedMemoryUnavailable)?;
        self.write_exports.insert(
            handle.allocation_id,
            WriteExportLease {
                session_id,
                release_token: token.clone(),
            },
        );
        Ok(self.shared_descriptor(session_id, handle, transfer_id, None, token))
    }

    /// Issues the low-frequency capability used by an SDK cache miss.
    ///
    /// The descriptor is authorized against the Session's RegionGroup before
    /// a duplicate fd is registered with the one-shot SCM_RIGHTS broker.
    pub(crate) fn acquire_region(
        &mut self,
        session_id: u64,
        region_id: u64,
    ) -> Result<HostRegionGrant, ArenaError> {
        let group_id = self
            .session_groups
            .get(&session_id)
            .copied()
            .ok_or(ArenaError::RegionAccessDenied)?;
        let region = self
            .regions
            .iter()
            .find(|region| region.id == region_id)
            .ok_or(ArenaError::UnknownRegion)?;
        if region.group_id != group_id {
            return Err(ArenaError::RegionAccessDenied);
        }
        if !self
            .session_regions
            .get(&session_id)
            .is_some_and(|regions| regions.contains(&region_id))
        {
            return Err(ArenaError::RegionAccessDenied);
        }
        let broker = self
            .shared_fd_broker
            .as_ref()
            .ok_or(ArenaError::SharedMemoryUnavailable)?;
        let fd = region
            .duplicate_fd()
            .ok_or(ArenaError::SharedMemoryUnavailable)?;
        let token = shm_token().ok_or(ArenaError::SharedMemoryUnavailable)?;
        let request = FdRequest::new(
            BrokerToken::new(token.clone()).map_err(|_| ArenaError::SharedMemoryUnavailable)?,
            session_id,
            region_id,
        );
        broker
            .register(request, fd)
            .map_err(|_| ArenaError::SharedMemoryUnavailable)?;
        self.metrics.record_shm_fd_grant(ShmFdGrantResult::Issued);
        Ok(HostRegionGrant {
            region_id,
            region_length: region.len() as u64,
            fd_token: token,
        })
    }

    pub(crate) fn retire_block(&mut self, block_id: &[u8]) {
        if let Some(block) = self.blocks.remove(block_id) {
            self.release_handle(block.handle);
            self.record_state();
        }
    }

    pub(crate) fn delete_staging(&mut self, session_id: u64, staging_id: u64) {
        let owned = self
            .staging
            .get(&staging_id)
            .is_some_and(|staging| staging.session_id == session_id);
        if owned {
            self.retire_staging(staging_id, StagingReclaimReason::Cancel);
        }
    }

    pub(crate) fn reclaim_session(&mut self, session_id: u64) {
        let staging_ids = self
            .staging
            .iter()
            .filter_map(|(id, staging)| (staging.session_id == session_id).then_some(*id))
            .collect::<Vec<_>>();
        for staging_id in staging_ids {
            self.retire_staging(staging_id, StagingReclaimReason::SessionClosed);
        }
        self.session_groups.remove(&session_id);
        self.session_regions.remove(&session_id);
    }

    pub(crate) fn tick(&mut self) -> ReclaimReport {
        let before = self.allocated_bytes;
        self.reclaim_expired();
        if let Some(broker) = &self.shared_fd_broker
            && let Ok(reaped) = broker.reap_expired()
            && reaped > 0
        {
            dms_logging::warn!(
                "expired shared-memory fd grants were reclaimed";
                "event" => "node.shm.fd_grant_expired",
                "count" => reaped,
            );
        }
        ReclaimReport {
            reclaimed_bytes: before.saturating_sub(self.allocated_bytes),
            remaining_retired_bytes: self.free_slot_bytes(),
        }
    }

    pub(crate) fn stats(&self) -> ArenaStats {
        let free_slot_bytes = self.free_slot_bytes();
        let largest_free_slot_bytes = self
            .regions
            .iter()
            .flat_map(|region| region.free.iter())
            .map(|slot| slot.capacity)
            .max()
            .unwrap_or(0);
        ArenaStats {
            capacity_bytes: self.capacity_bytes,
            logical_bytes: self.logical_bytes,
            allocated_bytes: self.allocated_bytes,
            resident_bytes: self.resident_bytes,
            free_slot_bytes,
            largest_free_slot_bytes,
            fragmentation_bytes: free_slot_bytes.saturating_sub(largest_free_slot_bytes),
            staging_count: self.staging.len(),
            block_count: self.blocks.len(),
            reclaim_count: self.reclaim_count,
            quarantined_bytes: self.quarantined_bytes,
        }
    }

    pub(crate) fn set_staging_ttl(&mut self, staging_ttl: Duration) {
        self.staging_ttl = staging_ttl;
    }

    #[cfg(test)]
    pub(crate) fn staging_ttl(&self) -> Duration {
        self.staging_ttl
    }

    fn allocate_slot(
        &mut self,
        group_id: u64,
        length: u64,
    ) -> Result<AllocationHandle, ArenaError> {
        let aligned_length = length.checked_add(63).ok_or(ArenaError::RegionOverflow)? & !63;
        let allocation_id = self.next_allocation_id;
        for region in self
            .regions
            .iter_mut()
            .filter(|region| region.group_id == group_id)
        {
            if let Some(index) = region
                .free
                .iter()
                .position(|slot| slot.capacity >= aligned_length)
            {
                let slot = remove_slot(&mut region.free, index, aligned_length);
                self.next_allocation_id += 1;
                return Ok(AllocationHandle {
                    region_id: region.id,
                    offset: slot.offset,
                    length,
                    capacity: aligned_length,
                    allocation_id,
                });
            }
            let end = region.next_offset.saturating_add(aligned_length);
            if end <= region.len() as u64 {
                let offset = region.next_offset;
                region.next_offset = end;
                self.next_allocation_id += 1;
                return Ok(AllocationHandle {
                    region_id: region.id,
                    offset,
                    length,
                    capacity: aligned_length,
                    allocation_id,
                });
            }
        }
        let (group_resident, group_capacity) = self
            .region_groups
            .get(&group_id)
            .map(|group| (group.resident_bytes, group.capacity_bytes))
            .ok_or(ArenaError::RegionAccessDenied)?;
        let remaining = group_capacity
            .saturating_sub(group_resident)
            .min(self.capacity_bytes.saturating_sub(self.resident_bytes));
        if aligned_length > remaining {
            self.record_failure("arena.allocate.exhausted");
            return Err(ArenaError::CapacityExhausted);
        }
        // 小写共用一个大 Region；大于目标的写单独扩大 backing。预算尾部允许
        // 小 Region，避免还有可用容量却因凑不够默认 Region 大小而拒绝写入。
        // Region/Slot 都按 64B 对齐；OS 页大小不是用户的最小申请单元。
        let preferred = self
            .region_size_bytes
            .checked_add(63)
            .ok_or(ArenaError::RegionOverflow)?
            & !63;
        let region_capacity = aligned_length.max(preferred).min(remaining & !63);
        let region_len = usize::try_from(region_capacity).map_err(|_| {
            self.record_failure("arena.allocate.failed");
            ArenaError::RegionOverflow
        })?;
        let region_id = self.next_region_id;
        // backing 创建失败前不发布 id 或容量；故障后可在原预算内重试。
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_region_creation) {
            return Err(ArenaError::RegionCreateFailed);
        }
        let backing = if self.shared_fd_broker.is_some() {
            RegionBacking::Shared(
                SharedRegion::create(&format!("dms-region-{region_id}"), region_len)
                    .map_err(|error| {
                        dms_logging::warn!("failed to create shared Region";
                            "event" => "node.arena.region_create_failed", "error" => error.to_string());
                        ArenaError::RegionCreateFailed
                    })?,
            )
        } else {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(region_len)
                .map_err(|_| ArenaError::RegionCreateFailed)?;
            bytes.resize(region_len, 0);
            RegionBacking::Private(bytes)
        };
        self.next_region_id += 1;
        self.next_allocation_id += 1;
        self.resident_bytes += region_capacity;
        self.region_groups
            .get_mut(&group_id)
            .expect("RegionGroup was validated above")
            .resident_bytes += region_capacity;
        self.regions.push(Region {
            id: region_id,
            group_id,
            backing,
            next_offset: aligned_length,
            free: Vec::new(),
        });
        self.metrics.record_region_expanded(region_capacity);
        Ok(AllocationHandle {
            region_id,
            offset: 0,
            length,
            capacity: aligned_length,
            allocation_id,
        })
    }

    fn copy_into_slot(&mut self, handle: AllocationHandle, bytes: &[u8]) -> Result<(), ArenaError> {
        let region = self
            .regions
            .iter_mut()
            .find(|region| region.id == handle.region_id)
            .ok_or(ArenaError::StaleHandle)?;
        let start = usize::try_from(handle.offset).map_err(|_| ArenaError::RegionOverflow)?;
        let end = start
            .checked_add(bytes.len())
            .ok_or(ArenaError::RegionOverflow)?;
        if end > region.len() || bytes.len() as u64 > handle.capacity {
            return Err(ArenaError::RangeOutOfBounds);
        }
        region.write_at(start, bytes)?;
        Ok(())
    }

    // 先验证 Allocation identity 和物理边界，再局部借用；不跨 await、不向 Arena 外暴露。
    // 提交校验与幂等匹配只需读取，不为 digest 创建整块临时 Vec。
    fn slot_bytes_checked(&self, handle: AllocationHandle) -> Option<&[u8]> {
        if self.live_allocations.get(&handle.allocation_id) != Some(&handle) {
            return None;
        }
        let region = self
            .regions
            .get(usize::try_from(handle.region_id.checked_sub(1)?).ok()?)?;
        if region.id != handle.region_id {
            return None;
        }
        let start = usize::try_from(handle.offset).ok()?;
        let end = start.checked_add(usize::try_from(handle.length).ok()?)?;
        if end > region.len() {
            return None;
        }
        region
            .borrow_at(start, usize::try_from(handle.length).ok()?)
            .ok()
    }

    fn shared_descriptor(
        &mut self,
        session_id: u64,
        handle: AllocationHandle,
        transfer_id: u64,
        view_epoch: Option<u64>,
        release_token: Vec<u8>,
    ) -> Option<HostShmDescriptor> {
        self.shared_fd_broker.as_ref()?;
        self.regions
            .iter()
            .find(|region| region.id == handle.region_id)?;
        self.session_regions
            .entry(session_id)
            .or_default()
            .insert(handle.region_id);
        Some(HostShmDescriptor {
            region_id: handle.region_id,
            offset: handle.offset,
            length: handle.length,
            allocation_id: handle.allocation_id,
            view_epoch,
            transfer_id,
            release_token,
        })
    }

    fn free_slot(&mut self, handle: AllocationHandle) {
        if let Some(region) = self
            .regions
            .iter_mut()
            .find(|region| region.id == handle.region_id)
        {
            insert_free_slot(
                &mut region.free,
                FreeSlot {
                    offset: handle.offset,
                    capacity: handle.capacity,
                },
            );
        }
    }

    fn release_handle(&mut self, handle: AllocationHandle) {
        self.live_allocations.remove(&handle.allocation_id);
        self.logical_bytes = self.logical_bytes.saturating_sub(handle.length);
        if self.write_exports.remove(&handle.allocation_id).is_some() {
            // 只有未归还的可写 SHM borrow 需要隔离。读 descriptor 由 View/Ticket
            // 生命周期保护，不应永久占用 free-list；旧 SDK、丢消息或 token 缺失
            // 都会留在此分支，宁可容量耗尽也不让旧写污染新 allocation。
            self.quarantined_bytes += handle.capacity;
            return;
        }
        self.allocated_bytes = self.allocated_bytes.saturating_sub(handle.capacity);
        self.free_slot(handle);
        self.reclaim_count += 1;
    }

    fn free_slot_bytes(&self) -> u64 {
        self.regions
            .iter()
            .flat_map(|region| region.free.iter())
            .map(|slot| slot.capacity)
            .sum()
    }

    fn reclaim_expired(&mut self) {
        let now = Instant::now();
        let expired = self
            .staging
            .iter()
            .filter_map(|(id, staging)| {
                (now.duration_since(staging.created_at) >= self.staging_ttl).then_some(*id)
            })
            .collect::<Vec<_>>();
        for staging_id in expired {
            self.retire_staging(staging_id, StagingReclaimReason::Expired);
        }
    }

    fn retire_staging(&mut self, staging_id: u64, reason: StagingReclaimReason) {
        if let Some(staging) = self.staging.remove(&staging_id) {
            self.transfer_index.remove(&staging.transfer_id);
            self.release_handle(staging.handle);
            self.metrics.record_staging_reclaimed(reason);
            self.record_state();
        }
    }

    fn record_failure(&self, name: &'static str) {
        dms_logging::warn!(
            "arena allocation failed";
            "event" => name,
        );
    }

    fn record_state(&self) {
        let stats = self.stats();
        let free_bytes = stats.capacity_bytes.saturating_sub(stats.allocated_bytes);
        let fragmentation = if stats.free_slot_bytes == 0 {
            0.0
        } else {
            1.0 - (stats.largest_free_slot_bytes as f64 / stats.free_slot_bytes as f64)
        };
        let oldest = self
            .staging
            .values()
            .map(|staging| staging.created_at.elapsed().as_secs_f64())
            .fold(0.0, f64::max);
        self.metrics.set_arena_state(ArenaMetricsSnapshot {
            allocated_bytes: stats.allocated_bytes,
            quarantined_bytes: stats.quarantined_bytes,
            logical_bytes: stats.logical_bytes,
            free_bytes,
            fragmentation_ratio: fragmentation,
            staging_allocations: stats.staging_count,
            regions: self.regions.len(),
            oldest_staging_age_seconds: oldest,
        });
    }
}

fn remove_slot(free: &mut Vec<FreeSlot>, index: usize, requested: u64) -> FreeSlot {
    let slot = free.remove(index);
    let remainder = slot.capacity.saturating_sub(requested);
    if remainder > 0 {
        insert_free_slot(
            free,
            FreeSlot {
                offset: slot.offset + requested,
                capacity: remainder,
            },
        );
    }
    FreeSlot {
        offset: slot.offset,
        capacity: requested,
    }
}

fn insert_free_slot(free: &mut Vec<FreeSlot>, slot: FreeSlot) {
    free.push(slot);
    free.sort_by_key(|slot| slot.offset);

    let mut coalesced: Vec<FreeSlot> = Vec::with_capacity(free.len());
    for slot in free.drain(..) {
        if let Some(last) = coalesced.last_mut()
            && last.offset + last.capacity == slot.offset
        {
            last.capacity += slot.capacity;
            continue;
        }
        coalesced.push(slot);
    }
    *free = coalesced;
}

fn shm_token() -> Option<Vec<u8>> {
    let mut token = vec![0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut token))
        .ok()?;
    Some(token)
}

fn digest(bytes: &[u8]) -> Vec<u8> {
    dms_transport::checksum::fnv1a_bytes(bytes).to_vec()
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    fn stage54_arena(shared: bool, label: &str) -> (ArenaManager, Option<PathBuf>) {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        let path = shared.then(|| {
            std::env::temp_dir().join(format!("dms-stage54-{label}-{}.sock", std::process::id()))
        });
        if let Some(path) = &path {
            arena.enable_shared_region(SharedFdBroker::bind(path.clone()).unwrap());
        }
        (arena, path)
    }

    fn stage54_assert_slot_borrows_backing(shared: bool) {
        let (mut arena, path) = stage54_arena(shared, "borrow");
        arena
            .commit_inline(b"padding".to_vec(), vec![9; 64])
            .unwrap();
        let allocation = arena.allocate(7, 129).unwrap();
        let payload = vec![0x5a; 129];
        arena.upload(allocation.transfer_id, &payload).unwrap();
        let handle = arena.staging[&allocation.staging_id].handle;
        assert_ne!(handle.offset, 0);
        let first = arena.slot_bytes_checked(handle).unwrap();
        let second = arena.slot_bytes_checked(handle).unwrap();
        assert_eq!(first, payload.as_slice());
        assert_eq!(second, payload.as_slice());
        // 两份结果同时存活：旧实现的两个 Vec 不可能复用同一分配地址。
        assert_eq!(
            first.as_ptr(),
            second.as_ptr(),
            "checksum input must borrow its backing"
        );
        if let RegionBacking::Private(bytes) = &arena.regions[0].backing {
            assert_eq!(first.as_ptr(), bytes[handle.offset as usize..].as_ptr());
        }
        if let Some(path) = path {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn stage54_private_checksum_input_does_not_copy_payload() {
        stage54_assert_slot_borrows_backing(false);
    }

    #[test]
    fn stage54_shared_checksum_input_does_not_copy_payload() {
        stage54_assert_slot_borrows_backing(true);
    }

    #[test]
    fn stage54_slot_identity_and_bounds_are_checked_before_access() {
        for shared in [false, true] {
            let (mut arena, path) = stage54_arena(shared, "bounds");
            let allocation = arena.allocate(7, 4).unwrap();
            arena.upload(allocation.transfer_id, b"data").unwrap();
            let handle = arena.staging[&allocation.staging_id].handle;
            assert!(
                arena
                    .slot_bytes_checked(AllocationHandle {
                        allocation_id: handle.allocation_id + 1,
                        ..handle
                    })
                    .is_none()
            );
            for invalid in [
                AllocationHandle {
                    region_id: 0,
                    ..handle
                },
                AllocationHandle {
                    region_id: u64::MAX,
                    ..handle
                },
                AllocationHandle {
                    offset: u64::MAX,
                    ..handle
                },
                AllocationHandle {
                    length: u64::MAX,
                    ..handle
                },
                AllocationHandle {
                    offset: 4096,
                    length: 1,
                    ..handle
                },
            ] {
                // 单独穿刺物理边界：即使测试注入同 identity 的异常 handle，也不得借用越界。
                arena.live_allocations.insert(handle.allocation_id, invalid);
                assert!(arena.slot_bytes_checked(invalid).is_none());
            }
            arena.live_allocations.insert(handle.allocation_id, handle);
            arena.delete_staging(7, allocation.staging_id);
            assert!(arena.slot_bytes_checked(handle).is_none());
            if let Some(path) = path {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    #[test]
    fn committed_block_identity_reuses_verified_digest_and_retires_with_block() {
        for shared in [false, true] {
            let (mut arena, path) = stage54_arena(shared, "block-identity");
            let checksum = digest(b"data");
            let digest_pointer = checksum.as_ptr();
            arena
                .commit_inline_with_verified_digest(b"inline".to_vec(), b"data".to_vec(), checksum)
                .unwrap();
            let (length, stored) = arena.block_length_and_digest(b"inline").unwrap();
            assert_eq!(length, 4);
            assert_eq!(stored, digest(b"data"));
            assert_eq!(
                stored.as_ptr(),
                digest_pointer,
                "移动已有摘要，不重复分配/哈希 payload"
            );
            let allocation = arena.allocate(7, 4).unwrap();
            let receipt = arena.upload(allocation.transfer_id, b"next").unwrap();
            arena
                .commit_staging(7, allocation.staging_id, &receipt, b"staged".to_vec())
                .unwrap();
            assert_eq!(
                arena.block_length_and_digest(b"staged"),
                Some((4, receipt.digest.as_slice()))
            );
            assert_eq!(
                arena.commit_inline_with_verified_digest(
                    b"empty-digest".to_vec(),
                    b"data".to_vec(),
                    Vec::new()
                ),
                Err(ArenaError::ReceiptConflict)
            );
            assert!(arena.block_length_and_digest(b"empty-digest").is_none());
            arena.retire_block(b"inline");
            arena.retire_block(b"staged");
            assert!(arena.block_length_and_digest(b"inline").is_none());
            assert!(arena.block_length_and_digest(b"staged").is_none());
            if let Some(path) = path {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    #[test]
    fn stage54_commit_rechecks_bytes_and_receipt_for_both_backings() {
        for shared in [false, true] {
            let (mut arena, path) = stage54_arena(shared, "corrupt");
            let allocation = arena.allocate(7, 4).unwrap();
            let receipt = arena.upload(allocation.transfer_id, b"data").unwrap();
            let handle = arena.staging[&allocation.staging_id].handle;
            let mut wrong_length = receipt.clone();
            wrong_length.length += 1;
            assert_eq!(
                arena.commit_staging(7, allocation.staging_id, &wrong_length, b"block".to_vec()),
                Err(ArenaError::ReceiptConflict)
            );
            arena.copy_into_slot(handle, b"evil").unwrap();
            assert_eq!(
                arena.commit_staging(7, allocation.staging_id, &receipt, b"block".to_vec()),
                Err(ArenaError::ReceiptConflict)
            );
            assert!(arena.blocks.is_empty());
            assert!(arena.staging.contains_key(&allocation.staging_id));
            arena.copy_into_slot(handle, b"data").unwrap();
            arena
                .commit_staging(7, allocation.staging_id, &receipt, b"block".to_vec())
                .unwrap();
            assert_eq!(arena.read_bytes(b"block").unwrap(), b"data");
            assert_eq!(
                arena.commit_staging(7, allocation.staging_id, &receipt, b"other".to_vec()),
                Err(ArenaError::UnknownStaging)
            );
            if let Some(path) = path {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    #[test]
    fn stage54_duplicate_block_checks_bytes_and_keeps_owned_reads_independent() {
        for shared in [false, true] {
            let (mut arena, path) = stage54_arena(shared, "duplicate");
            arena
                .commit_inline(b"block".to_vec(), b"data".to_vec())
                .unwrap();
            for (payload, expected) in [
                (b"data", Ok(())),
                (b"evil", Err(ArenaError::ReceiptConflict)),
            ] {
                let allocation = arena.allocate(7, 4).unwrap();
                let receipt = arena.upload(allocation.transfer_id, payload).unwrap();
                assert_eq!(
                    arena.commit_staging(7, allocation.staging_id, &receipt, b"block".to_vec()),
                    expected
                );
                assert!(!arena.staging.contains_key(&allocation.staging_id));
            }
            assert_eq!(arena.stats().block_count, 1);
            let mut owned = arena.read_bytes(b"block").unwrap();
            owned[0] = b'X';
            assert_eq!(arena.read_bytes(b"block").unwrap(), b"data");
            assert!(
                arena
                    .commit_inline(b"block".to_vec(), b"data".to_vec())
                    .is_ok()
            );
            assert_eq!(
                arena.commit_inline(b"block".to_vec(), b"evil".to_vec()),
                Err(ArenaError::ReceiptConflict)
            );
            if let Some(path) = path {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    #[test]
    fn stage54_mmap_receipt_commits_while_staging_is_writable() {
        let (mut arena, path) = stage54_arena(true, "mmap-receipt");
        let payload = b"direct-shm";
        let allocation = arena.allocate(7, payload.len() as u64).unwrap();
        let descriptor = arena
            .shm_descriptor_for_staging(7, allocation.staging_id)
            .unwrap()
            .unwrap();
        let region = &arena.regions[usize::try_from(descriptor.region_id - 1).unwrap()];
        let fd = region.duplicate_fd().unwrap();
        // SAFETY: 测试独占此 staging 的写入；第二映射完成写入后才构造 receipt，
        // 此后不再写入，Arena 的同步校验期间没有跨映射可变借用。
        let mut mapping = unsafe { dms_shm::MappedRegion::map(fd, region.len()) }.unwrap();
        mapping
            .write_at(usize::try_from(descriptor.offset).unwrap(), payload)
            .unwrap();
        // 真实 SHM 写入不调用 upload，因此不能依赖网络路径设置的 Sealed 状态。
        assert_eq!(
            arena.staging[&allocation.staging_id].state,
            HostStagingState::Writable
        );
        let receipt = HostReceipt {
            transfer_id: allocation.transfer_id,
            length: allocation.length,
            digest: digest(payload),
            allocation_id: allocation.allocation_id,
            release_token: descriptor.release_token.clone(),
        };
        arena
            .commit_staging(7, allocation.staging_id, &receipt, b"mmap-block".to_vec())
            .unwrap();
        assert_eq!(arena.read_bytes(b"mmap-block").unwrap(), payload);
        drop(mapping);
        if let Some(path) = path {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn stage54_taken_owned_bytes_survive_slot_release_and_reuse() {
        for shared in [false, true] {
            let (mut arena, path) = stage54_arena(shared, "take-owned");
            // 共享 backing 仍走未导出路径，释放后可安全复用；不削弱已导出范围的隔离。
            let allocation = arena.allocate(7, 4).unwrap();
            let receipt = arena.upload(allocation.transfer_id, b"old!").unwrap();
            let old_handle = arena.staging[&allocation.staging_id].handle;
            let owned = arena
                .take_staging_bytes(7, allocation.staging_id, &receipt)
                .unwrap();
            assert!(!arena.staging.contains_key(&allocation.staging_id));
            assert!(
                !arena
                    .live_allocations
                    .contains_key(&old_handle.allocation_id)
            );
            let reused = arena.allocate(7, 4).unwrap();
            let new_handle = arena.staging[&reused.staging_id].handle;
            assert_eq!(new_handle.region_id, old_handle.region_id);
            assert_eq!(new_handle.offset, old_handle.offset);
            assert_ne!(new_handle.allocation_id, old_handle.allocation_id);
            arena.upload(reused.transfer_id, b"new!").unwrap();
            assert_eq!(arena.slot_bytes_checked(new_handle).unwrap(), b"new!");
            assert_eq!(owned, b"old!");
            if let Some(path) = path {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    #[test]
    fn small_allocations_share_a_large_region_and_reuse_safe_slots() {
        let mut arena = ArenaManager::new(128 * 1024 * 1024, Duration::from_secs(30));
        let first = arena.allocate(7, 1024).unwrap();
        let first_handle = arena.staging[&first.staging_id].handle;
        for _ in 0..1024 {
            let allocation = arena.allocate(7, 1024).unwrap();
            assert_eq!(
                arena.staging[&allocation.staging_id].handle.region_id,
                first_handle.region_id
            );
        }
        assert_eq!(arena.regions.len(), 1);
        assert_eq!(arena.stats().resident_bytes, 64 * 1024 * 1024);
        arena.delete_staging(7, first.staging_id);
        let reused = arena.allocate(7, 1024).unwrap();
        let reused_handle = arena.staging[&reused.staging_id].handle;
        assert_eq!(reused_handle.region_id, first_handle.region_id);
        assert_eq!(reused_handle.offset, first_handle.offset);
        assert_ne!(reused_handle.allocation_id, first_handle.allocation_id);
    }

    #[test]
    fn region_growth_honors_config_large_values_and_budget_tail() {
        let mut arena = ArenaManager::new(10 * 1024, Duration::from_secs(30));
        arena.set_region_size(4096);
        let first = arena.allocate(7, 5120).unwrap(); // 大于 Region 目标，不拆用户 allocation。
        assert_eq!(arena.regions[0].len(), 5120);
        arena.allocate(7, 4096).unwrap();
        arena.allocate(7, 1024).unwrap(); // 最后 1KiB 预算仍能使用。
        assert_eq!(arena.stats().resident_bytes, 10 * 1024);
        assert_eq!(arena.regions.len(), 3);
        assert_eq!(arena.allocate(7, 1), Err(ArenaError::CapacityExhausted));
        arena.delete_staging(7, first.staging_id);
        arena.allocate(7, 5120).unwrap();
        assert_eq!(arena.regions.len(), 3); // 复用已安全释放的 Slot，不扩容。
        assert_eq!(arena.allocate(7, u64::MAX), Err(ArenaError::RegionOverflow));
    }

    #[test]
    fn failed_region_creation_does_not_consume_capacity_or_identity() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        arena.fail_next_region_creation = true;
        assert_eq!(arena.allocate(7, 1), Err(ArenaError::RegionCreateFailed));
        assert_eq!(arena.stats().resident_bytes, 0);
        assert_eq!(
            arena.region_groups[&DEFAULT_REGION_GROUP_ID].resident_bytes,
            0
        );
        assert!(arena.regions.is_empty());
        assert_eq!(arena.next_region_id, 1);
        assert!(arena.allocate(7, 1).is_ok());
    }

    #[test]
    fn exported_cancel_and_session_close_quarantine_is_capacity_bounded() {
        let path = std::env::temp_dir().join(format!("dms-quarantine-{}.sock", std::process::id()));
        let registry = dms_metrics::registry();
        let metrics = NodeMetrics::register(&registry).unwrap();
        let mut arena = ArenaManager::with_metrics(4096, Duration::from_secs(30), metrics);
        arena.enable_shared_region(SharedFdBroker::bind(path.clone()).unwrap());
        for index in 0..64 {
            let allocation = arena.allocate(7, 1).unwrap();
            arena
                .shm_descriptor_for_staging(7, allocation.staging_id)
                .unwrap()
                .unwrap();
            if index % 2 == 0 {
                arena.delete_staging(7, allocation.staging_id);
            } else {
                arena.reclaim_session(7);
            }
        }
        assert_eq!(arena.stats().quarantined_bytes, 4096);
        assert_eq!(arena.stats().logical_bytes, 0);
        assert_eq!(arena.stats().allocated_bytes, 4096);
        assert_eq!(arena.allocate(7, 1), Err(ArenaError::CapacityExhausted));
        assert_eq!(arena.stats().resident_bytes, 4096);
        assert!(arena.live_allocations.is_empty());
        assert!(arena.write_exports.is_empty());
        let text = dms_metrics::encode_text(&registry).unwrap();
        assert!(text.contains("dms_node_arena_quarantined_bytes 4096"));
        assert!(text.contains("dms_node_arena_allocated_bytes 4096"));
        assert!(text.contains("dms_node_arena_free_bytes 0"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shm_range_descriptor_preserves_allocation_and_selects_exact_bytes() {
        let path = std::env::temp_dir().join(format!("dms-range-{}.sock", std::process::id()));
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        arena.enable_shared_region(SharedFdBroker::bind(path.clone()).unwrap());
        arena
            .commit_inline(b"padding".to_vec(), b"padding".to_vec())
            .unwrap();
        arena
            .commit_inline(b"block".to_vec(), b"abcdef".to_vec())
            .unwrap();
        let (ticket, _) = arena.open_read(b"block", Some((2, 1))).unwrap();
        let descriptor = arena
            .shm_descriptor_for_read(7, ticket, 1, 1)
            .unwrap()
            .unwrap();
        assert_eq!(descriptor.offset, ticket.handle.offset + 2);
        assert_eq!(descriptor.length, 1);
        assert_eq!(descriptor.allocation_id, ticket.handle.allocation_id);
        assert_eq!(arena.read_ticket(ticket).unwrap(), b"c");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_shm_descriptor_retirement_reuses_slot_without_quarantine() {
        let path =
            std::env::temp_dir().join(format!("dms-read-release-{}.sock", std::process::id()));
        let mut arena = ArenaManager::new(128, Duration::from_secs(30));
        arena.enable_shared_region(SharedFdBroker::bind(path.clone()).unwrap());
        arena
            .commit_inline(b"block".to_vec(), b"abcdef".to_vec())
            .unwrap();
        let (ticket, _) = arena.open_read(b"block", Some((0, 6))).unwrap();
        let descriptor = arena
            .shm_descriptor_for_read(7, ticket, 1, 1)
            .unwrap()
            .unwrap();
        assert_eq!(descriptor.view_epoch, Some(1));

        arena.retire_block(b"block");

        assert_eq!(arena.stats().quarantined_bytes, 0);
        assert_eq!(arena.stats().allocated_bytes, 0);
        assert_eq!(arena.stats().free_slot_bytes, 64);
        arena.allocate(7, 64).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn returned_write_export_reuses_capacity_for_many_cycles() {
        let path =
            std::env::temp_dir().join(format!("dms-write-release-{}.sock", std::process::id()));
        let mut arena = ArenaManager::new(64, Duration::from_secs(30));
        arena.enable_shared_region(SharedFdBroker::bind(path.clone()).unwrap());

        for cycle in 0..100 {
            let block_id = format!("block-{cycle}").into_bytes();
            let allocation = arena.allocate(7, 1).unwrap();
            let descriptor = arena
                .shm_descriptor_for_staging(7, allocation.staging_id)
                .unwrap()
                .unwrap();
            let mut receipt = arena.upload(allocation.transfer_id, b"x").unwrap();
            receipt.release_token = descriptor.release_token;
            arena
                .commit_staging(7, allocation.staging_id, &receipt, block_id.clone())
                .unwrap();
            assert!(!arena.block_has_unreturned_write(&block_id));
            arena.retire_block(&block_id);
            assert_eq!(arena.stats().allocated_bytes, 0);
            assert_eq!(arena.stats().quarantined_bytes, 0);
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_write_release_token_keeps_retired_block_isolated() {
        let path =
            std::env::temp_dir().join(format!("dms-write-isolate-{}.sock", std::process::id()));
        let mut arena = ArenaManager::new(64, Duration::from_secs(30));
        arena.enable_shared_region(SharedFdBroker::bind(path.clone()).unwrap());
        let allocation = arena.allocate(7, 1).unwrap();
        arena
            .shm_descriptor_for_staging(7, allocation.staging_id)
            .unwrap()
            .unwrap();
        let receipt = arena.upload(allocation.transfer_id, b"x").unwrap();
        arena
            .commit_staging(7, allocation.staging_id, &receipt, b"block".to_vec())
            .unwrap();
        assert!(arena.block_has_unreturned_write(b"block"));

        arena.retire_block(b"block");

        assert_eq!(arena.stats().allocated_bytes, 64);
        assert_eq!(arena.stats().quarantined_bytes, 64);
        assert_eq!(arena.allocate(7, 1), Err(ArenaError::CapacityExhausted));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn expired_exported_staging_cannot_corrupt_a_new_allocation() {
        let path =
            std::env::temp_dir().join(format!("dms-expired-write-{}.sock", std::process::id()));
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        arena.enable_shared_region(SharedFdBroker::bind(path.clone()).unwrap());
        let old = arena.allocate(7, 5).unwrap();
        let descriptor = arena
            .shm_descriptor_for_staging(7, old.staging_id)
            .unwrap()
            .unwrap();
        let old_fd = File::from(arena.regions[0].duplicate_fd().unwrap());
        arena.staging.get_mut(&old.staging_id).unwrap().created_at =
            Instant::now() - Duration::from_secs(60);
        arena.tick();
        let new = arena.allocate(7, 5).unwrap();
        arena.upload(new.transfer_id, b"fresh").unwrap();
        // 保留的 fd 与旧 mmap 指向同一物理页；此写模拟暂停的旧 Client 恢复。
        std::os::unix::fs::FileExt::write_all_at(&old_fd, b"stale", descriptor.offset).unwrap();
        let handle = arena.staging[&new.staging_id].handle;
        assert_eq!(arena.slot_bytes_checked(handle).unwrap(), b"fresh");
        assert_eq!(arena.stats().allocated_bytes, 128);
        assert_eq!(arena.stats().quarantined_bytes, 64);
        assert_eq!(arena.stats().free_slot_bytes, 0);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn region_fd_requires_a_slice_issued_to_the_same_session() {
        let path = std::env::temp_dir().join(format!(
            "dms-region-auth-{}-{}.sock",
            std::process::id(),
            shm_token().expect("random token")[0]
        ));
        let broker = SharedFdBroker::bind(path.clone()).expect("bind broker");
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        arena.enable_shared_region(broker);

        let owner = arena.allocate(7, 4).expect("owner allocation");
        let descriptor = arena
            .shm_descriptor_for_staging(7, owner.staging_id)
            .expect("owner descriptor")
            .expect("SHM descriptor");
        // Session 8 belongs to the same default RegionGroup, but has never
        // received a slice in Session 7's Region.
        arena.allocate(8, 4).expect("bind second session group");
        assert_eq!(
            arena.acquire_region(8, descriptor.region_id),
            Err(ArenaError::RegionAccessDenied)
        );
        assert!(arena.acquire_region(7, descriptor.region_id).is_ok());
        let _ = std::fs::remove_file(path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn registered_session_can_map_an_inline_block_without_prior_staging() {
        let path = std::env::temp_dir().join(format!(
            "dms-inline-region-auth-{}-{}.sock",
            std::process::id(),
            shm_token().expect("random token")[0]
        ));
        let broker = SharedFdBroker::bind(path.clone()).expect("bind broker");
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        arena.enable_shared_region(broker);
        arena.register_session(7);
        arena
            .commit_inline(b"inline-block".to_vec(), b"inline-value".to_vec())
            .expect("commit inline block");
        let (ticket, _) = arena
            .open_read(b"inline-block", None)
            .expect("open inline read");
        let descriptor = arena
            .shm_descriptor_for_read(7, ticket, 1, 1)
            .expect("descriptor")
            .expect("shared descriptor");

        assert!(arena.acquire_region(7, descriptor.region_id).is_ok());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn host_arena_allocates_seals_commits_and_rejects_stale_receipt() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        let allocation = arena.allocate(7, 4).expect("allocate");
        let receipt = arena
            .upload(allocation.transfer_id, b"data")
            .expect("upload");
        arena
            .commit_staging(7, allocation.staging_id, &receipt, b"block-1".to_vec())
            .expect("commit");
        assert_eq!(arena.read_bytes(b"block-1"), Some(b"data".to_vec()));
        assert!(
            arena
                .commit_staging(7, allocation.staging_id, &receipt, b"block-2".to_vec())
                .is_err()
        );
    }

    #[test]
    fn delete_is_idempotent_and_capacity_is_reclaimed() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        let allocation = arena.allocate(9, 4).expect("allocate");
        arena.delete_staging(9, allocation.staging_id);
        arena.delete_staging(9, allocation.staging_id);
        assert!(arena.allocate(9, 4).is_ok());
    }

    #[test]
    fn commit_reuses_the_same_region_slot_without_moving_payload_bytes() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        let allocation = arena.allocate(7, 5).expect("allocate");
        let receipt = arena
            .upload(allocation.transfer_id, b"hello")
            .expect("upload");
        let staging_handle = arena
            .staging
            .get(&allocation.staging_id)
            .expect("staging")
            .handle;

        arena
            .commit_staging(7, allocation.staging_id, &receipt, b"block-1".to_vec())
            .expect("commit");

        let block_handle = arena
            .blocks
            .get(b"block-1".as_slice())
            .expect("block")
            .handle;
        assert_eq!(block_handle, staging_handle);
        assert_eq!(arena.read_bytes(b"block-1"), Some(b"hello".to_vec()));
    }

    #[test]
    fn retired_slots_are_reused_with_a_new_allocation_id() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        let first = arena.allocate(7, 5).expect("first allocate");
        let first_handle = arena.staging.get(&first.staging_id).expect("first").handle;
        arena.delete_staging(7, first.staging_id);

        let second = arena.allocate(7, 4).expect("second allocate");
        let second_handle = arena
            .staging
            .get(&second.staging_id)
            .expect("second")
            .handle;

        assert_eq!(second_handle.region_id, first_handle.region_id);
        assert_eq!(second_handle.offset, first_handle.offset);
        assert_ne!(second_handle.allocation_id, first_handle.allocation_id);
    }

    #[test]
    fn free_slots_split_coalesce_and_reuse() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        let first = arena.allocate(7, 96).expect("first allocate");
        let second = arena.allocate(7, 64).expect("second allocate");
        let first_handle = arena.staging.get(&first.staging_id).expect("first").handle;
        let second_handle = arena
            .staging
            .get(&second.staging_id)
            .expect("second")
            .handle;

        arena.delete_staging(7, first.staging_id);
        arena.delete_staging(7, second.staging_id);

        let stats = arena.stats();
        assert_eq!(
            stats.free_slot_bytes,
            first_handle.capacity + second_handle.capacity
        );
        assert_eq!(stats.largest_free_slot_bytes, stats.free_slot_bytes);

        let third = arena.allocate(7, 32).expect("third allocate");
        let third_handle = arena.staging.get(&third.staging_id).expect("third").handle;
        assert_eq!(third_handle.offset, first_handle.offset);
        assert!(arena.stats().free_slot_bytes < stats.free_slot_bytes);
    }

    #[test]
    fn duplicate_block_id_does_not_leak_retried_staging() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        let first = arena.allocate(7, 5).expect("first allocate");
        let first_receipt = arena
            .upload(first.transfer_id, b"hello")
            .expect("first upload");
        arena
            .commit_staging(7, first.staging_id, &first_receipt, b"block-1".to_vec())
            .expect("first commit");

        let second = arena.allocate(7, 5).expect("second allocate");
        let second_receipt = arena
            .upload(second.transfer_id, b"hello")
            .expect("second upload");
        arena
            .commit_staging(7, second.staging_id, &second_receipt, b"block-1".to_vec())
            .expect("idempotent commit");

        assert_eq!(arena.read_bytes(b"block-1"), Some(b"hello".to_vec()));
        assert_eq!(arena.stats().block_count, 1);
        assert_eq!(arena.stats().logical_bytes, 5);
    }

    #[test]
    fn duplicate_block_id_rejects_different_payload_without_leak() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        let first = arena.allocate(7, 5).expect("first allocate");
        let first_receipt = arena
            .upload(first.transfer_id, b"hello")
            .expect("first upload");
        arena
            .commit_staging(7, first.staging_id, &first_receipt, b"block-1".to_vec())
            .expect("first commit");

        let second = arena.allocate(7, 5).expect("second allocate");
        let second_receipt = arena
            .upload(second.transfer_id, b"other")
            .expect("second upload");
        assert_eq!(
            arena.commit_staging(7, second.staging_id, &second_receipt, b"block-1".to_vec()),
            Err(ArenaError::ReceiptConflict)
        );

        assert_eq!(arena.read_bytes(b"block-1"), Some(b"hello".to_vec()));
        assert_eq!(arena.stats().block_count, 1);
        assert_eq!(arena.stats().logical_bytes, 5);
        assert_eq!(arena.stats().staging_count, 0);
    }

    #[test]
    fn read_ticket_carries_handle_and_rejects_stale_allocation_id() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        arena
            .commit_inline(b"block-1".to_vec(), b"abcdef".to_vec())
            .expect("commit inline");
        let (ticket, logical_length) = arena
            .open_read(b"block-1", Some((2, 3)))
            .expect("open read");

        assert_eq!(logical_length, 6);
        assert_eq!(ticket.length, 3);
        assert_eq!(arena.read_ticket(ticket).expect("read ticket"), b"cde");

        arena.retire_block(b"block-1");
        assert_eq!(arena.read_ticket(ticket), Err(ArenaError::StaleHandle));
    }

    #[test]
    fn ttl_tick_reclaims_only_uncommitted_staging() {
        let mut arena = ArenaManager::new(4096, Duration::from_millis(0));
        let staging = arena.allocate(7, 5).expect("staging");
        arena
            .commit_inline(b"block-1".to_vec(), b"abc".to_vec())
            .expect("commit inline");

        let report = arena.tick();

        assert!(report.reclaimed_bytes >= 64);
        assert!(!arena.staging.contains_key(&staging.staging_id));
        assert_eq!(arena.read_bytes(b"block-1"), Some(b"abc".to_vec()));
    }

    #[test]
    fn capacity_rejects_oversized_allocation() {
        let mut arena = ArenaManager::new(4096, Duration::from_secs(30));
        assert_eq!(arena.allocate(7, 4097), Err(ArenaError::CapacityExhausted));
        assert_eq!(arena.stats().resident_bytes, 0);
    }

    #[test]
    fn growing_regions_keeps_existing_handles_valid() {
        let mut arena = ArenaManager::new(16 * 1024, Duration::from_secs(30));
        arena
            .commit_inline(b"small".to_vec(), b"still-here".to_vec())
            .expect("small commit");
        let (ticket, _) = arena.open_read(b"small", None).expect("open read");

        arena
            .commit_inline(b"large".to_vec(), vec![7; 5000])
            .expect("large commit");

        assert!(arena.stats().resident_bytes > 4096);
        assert_eq!(
            arena.read_ticket(ticket).expect("old ticket"),
            b"still-here"
        );
    }
}

//! Meta owner 内的权威文件锁表。
//!
//! 锁是易失协调状态，不写入文件内容 WAL。所有 mutation 仍只由 Meta actor 调用；
//! 本结构不启动 task，也不持有第二个 runtime。阻塞请求只保存 reply sender，绝不
//! 阻塞 actor turn。

use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Instant,
};

use tokio::sync::oneshot;

use crate::filesystem::{
    FileLockMode, FileLockOutcome, FileLockOwner, FileLockRange, FileLockRequest, GrantedFileLock,
    InodeId,
};

const MAX_WAITERS_PER_INODE: usize = 1_024;
const MAX_ACTIVE_WAITERS_GLOBAL: usize = 4_096;
const MAX_ATTACHED_REPLIES_PER_WAITER: usize = 64;
const MAX_COMPLETED_RECEIPTS_PER_OWNER: usize = 4_096;
const MAX_COMPLETED_RECEIPTS_GLOBAL: usize = 16_384;
const MAX_RELEASE_RECEIPTS_GLOBAL: usize = 16_384;
const MAX_RELEASE_FENCES_GLOBAL: usize = 4_096;

pub(crate) type FileLockReply = oneshot::Sender<FileLockMutation>;

struct LockWaiter {
    request: FileLockRequest,
    replies: Vec<FileLockReply>,
}

#[derive(Clone, Copy)]
struct LockMutationReceipt {
    outcome: FileLockOutcome,
    fingerprint: LockMutationFingerprint,
    owner_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LockMutationFingerprint {
    inode: InodeId,
    range: FileLockRange,
    mode: FileLockMode,
    pid: u32,
    wait: bool,
}

impl From<FileLockRequest> for LockMutationFingerprint {
    fn from(request: FileLockRequest) -> Self {
        Self {
            inode: request.inode,
            range: request.range,
            mode: request.mode,
            pid: request.pid,
            wait: request.wait,
        }
    }
}

#[derive(Default)]
struct InodeLocks {
    granted: Vec<GrantedFileLock>,
    waiters: VecDeque<LockWaiter>,
}

/// MetaState 持有的唯一锁状态。`recovering_nodes` 只在 Meta 重启保护窗口内使用。
#[derive(Default)]
pub(crate) struct FilesystemLockTable {
    inodes: HashMap<InodeId, InodeLocks>,
    /// 全局活跃等待请求数。per-inode cap 只能限制单个热点 inode；异常 Node
    /// 仍可能对无限 inode 各挂一个 waiter，导致 Meta 内存和阻塞 RPC task 无界。
    /// 所有新增/唤醒/取消/release 路径必须对称维护这个计数。
    active_waiters: usize,
    /// 已经完成的 set/unlock 结果。key 使用 Node incarnation 与 request_id，
    /// 只服务“响应在网络边界丢失后的同一请求安全重试”。它不写 WAL；Meta 重启后
    /// 仍依赖 Node reclaim 完整快照恢复锁表。
    completed: HashMap<(FileLockOwner, u64), LockMutationReceipt>,
    completed_order: VecDeque<(FileLockOwner, u64)>,
    /// `release_owner` 的 exact receipt。release 成功响应丢失后，同一个
    /// `(owner, request_id)` 必须返回第一次影响的锁数量；但被窗口淘汰后的非 exact
    /// stale release 不能静默成功，否则 Node 会删除本地 mirror 而 Meta 仍可能持锁。
    release_completed: HashMap<(FileLockOwner, u64), ReleaseLockReceipt>,
    release_completed_order: VecDeque<(FileLockOwner, u64)>,
    /// 已被窗口淘汰的最小安全边界。Node 在一个 session epoch 内分配全局严格
    /// 单调 mutation sequence，因此 floor 只需要按 `(node_id, node_epoch)` 记录。
    /// `request_id <= floor` 的重试不能重新执行，否则响应丢失后被淘汰的旧
    /// request_id 会复活成一把新锁。仍在等待队列里的请求先于 floor 检查处理，
    /// 这样长等待的低 sequence 重连可以重新挂接同一个终态。
    completed_floor: HashMap<(u64, u64), u64>,
    /// 同一个 POSIX owner 的 release tombstone。`release_owner` 是这个 owner
    /// mutation 域内的一条有序操作；它只终结/拒绝同 owner 且 sequence 不大于
    /// release sequence 的晚到 set，不能推进整个 node epoch 的 floor，否则会误伤
    /// 其它 owner 已在途但 sequence 更小的合法请求。
    release_fence: HashMap<FileLockOwner, u64>,
    release_fence_order: VecDeque<FileLockOwner>,
    /// Meta 实际应用锁状态修改的全局顺序。request_id 是 Node 分配顺序，
    /// 但 RPC 到达 Meta 的顺序可能反转；Node mirror 只能按这个 revision 做
    /// last-write-wins，不能用 request_id 猜测应用顺序。使用单个全局计数器，
    /// 避免长生命周期 session 持续创建 lock owner 时 Meta revision map 无界增长。
    next_lock_revision: u64,
    recovery_until: Option<Instant>,
    recovering_nodes: HashSet<u64>,
}

pub(crate) enum SetLockResult {
    Ready(FileLockMutation),
    Waiting,
    QueueFull,
    ReplyQueueFull,
    StaleRequestId,
    RequestMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileLockMutation {
    pub(crate) outcome: FileLockOutcome,
    pub(crate) owner_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReleaseLockReceipt {
    pub(crate) released: usize,
    pub(crate) owner_revision: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ReleaseLockOwnerResult {
    Released(ReleaseLockReceipt),
    QueueFull,
    StaleRequestId,
}

impl FilesystemLockTable {
    pub(crate) fn begin_recovery(
        &mut self,
        recovery_until: Instant,
        nodes: impl IntoIterator<Item = u64>,
    ) {
        self.recovery_until = Some(recovery_until);
        self.recovering_nodes = nodes.into_iter().collect();
    }

    /// 同一 Node 换 incarnation 时，在一个 Meta actor turn 内先建立恢复屏障。
    /// 旧 epoch 的锁随后会被清理，新 session 必须以完整快照调用 reclaim 才解除屏障。
    pub(crate) fn begin_node_recovery(&mut self, node_id: u64, recovery_until: Instant) {
        self.recovery_until = Some(
            self.recovery_until
                .map_or(recovery_until, |current| current.max(recovery_until)),
        );
        self.recovering_nodes.insert(node_id);
    }

    fn refresh_recovery(&mut self, now: Instant) {
        if self.recovery_until.is_some_and(|deadline| deadline <= now) {
            self.recovery_until = None;
            self.recovering_nodes.clear();
            self.wake_all_waiters();
        }
    }

    fn recovery_pending(&self) -> bool {
        self.recovery_until.is_some() && !self.recovering_nodes.is_empty()
    }

    /// 返回指定 Node 是否仍欠本轮完整锁快照。调用会顺便推进超时状态，确保已经
    /// 到达保守租约边界时不再要求一个永久失联的 Node 阻塞所有新锁。
    pub(crate) fn reclaim_required(&mut self, node_id: u64, now: Instant) -> bool {
        self.refresh_recovery(now);
        self.recovering_nodes.contains(&node_id)
    }

    pub(crate) fn test(&mut self, request: FileLockRequest, now: Instant) -> FileLockOutcome {
        self.refresh_recovery(now);
        if self.recovery_pending() {
            return FileLockOutcome::RecoveryPending;
        }
        self.first_conflict(request)
            .map_or(FileLockOutcome::Acquired, FileLockOutcome::Conflict)
    }

    pub(crate) fn set(
        &mut self,
        request: FileLockRequest,
        reply: Option<FileLockReply>,
        now: Instant,
    ) -> SetLockResult {
        self.refresh_recovery(now);
        let mutation_key = (request.owner, request.request_id);
        let fingerprint = LockMutationFingerprint::from(request);
        if let Some(receipt) = self.completed.get(&mutation_key).copied() {
            if receipt.fingerprint != fingerprint {
                return SetLockResult::RequestMismatch;
            }
            // request_id 是一次锁 mutation 的幂等身份。重试时即使调用方重新构造了
            // 请求，也只能拿到第一次终态，不能再次改写 granted 集合形成 ghost lock。
            return SetLockResult::Ready(FileLockMutation {
                outcome: receipt.outcome,
                owner_revision: receipt.owner_revision,
            });
        }
        if self.has_waiter(mutation_key) {
            if !self.waiter_matches(mutation_key, fingerprint) {
                return SetLockResult::RequestMismatch;
            }
            let Some(reply) = reply else {
                return SetLockResult::Waiting;
            };
            return if self.attach_waiter_reply(mutation_key, reply) {
                SetLockResult::Waiting
            } else {
                SetLockResult::ReplyQueueFull
            };
        }
        if self.request_id_below_floor(mutation_key) {
            return SetLockResult::StaleRequestId;
        }
        if self.request_id_released(mutation_key) {
            return SetLockResult::StaleRequestId;
        }
        if self.recovery_pending() {
            // 恢复窗口是瞬时状态，不能固化为 request receipt；否则同 request_id
            // 在恢复完成后仍会永久返回 RecoveryPending。
            return SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::RecoveryPending,
                owner_revision: 0,
            });
        }
        if request.mode == FileLockMode::Unlock {
            self.unlock(request.inode, request.owner, request.range);
            let outcome = FileLockOutcome::Released;
            let owner_revision = self.advance_lock_revision();
            self.record_completed(mutation_key, outcome, fingerprint, owner_revision);
            self.wake_waiters(request.inode);
            return SetLockResult::Ready(FileLockMutation {
                outcome,
                owner_revision,
            });
        }
        if let Some(conflict) = self.first_conflict(request) {
            if !request.wait {
                let outcome = FileLockOutcome::Conflict(conflict);
                self.record_completed(mutation_key, outcome, fingerprint, 0);
                return SetLockResult::Ready(FileLockMutation {
                    outcome,
                    owner_revision: 0,
                });
            }
            let locks = self.inodes.entry(request.inode).or_default();
            if locks.waiters.len() >= MAX_WAITERS_PER_INODE {
                return SetLockResult::QueueFull;
            }
            if self.active_waiters >= MAX_ACTIVE_WAITERS_GLOBAL {
                return SetLockResult::QueueFull;
            }
            let Some(reply) = reply else {
                return SetLockResult::Ready(FileLockMutation {
                    outcome: FileLockOutcome::Conflict(conflict),
                    owner_revision: 0,
                });
            };
            locks.waiters.push_back(LockWaiter {
                request,
                replies: vec![reply],
            });
            self.active_waiters += 1;
            return SetLockResult::Waiting;
        }
        self.replace_owner_range(request);
        let outcome = FileLockOutcome::Acquired;
        let owner_revision = self.advance_lock_revision();
        self.record_completed(mutation_key, outcome, fingerprint, owner_revision);
        SetLockResult::Ready(FileLockMutation {
            outcome,
            owner_revision,
        })
    }

    pub(crate) fn cancel(&mut self, request_id: u64, owner: FileLockOwner) -> bool {
        if self
            .completed
            .get(&(owner, request_id))
            .is_some_and(|receipt| receipt.outcome == FileLockOutcome::Interrupted)
        {
            return true;
        }
        // 活跃 waiter 是已经被 Meta 接纳但尚未产生终态的 mutation。即使同一
        // Node epoch 的后续 mutation 已经把 completed floor 推过了它，FUSE
        // interrupt 仍必须能取消这条等待；floor 只用于拒绝“已经不在 active /
        // completed 窗口里的旧 request_id 重新执行”。
        for locks in self.inodes.values_mut() {
            let Some(index) = locks.waiters.iter().position(|waiter| {
                waiter.request.request_id == request_id && waiter.request.owner == owner
            }) else {
                continue;
            };
            if let Some(waiter) = locks.waiters.remove(index) {
                self.active_waiters = self.active_waiters.saturating_sub(1);
                let mutation = FileLockMutation {
                    outcome: FileLockOutcome::Interrupted,
                    owner_revision: 0,
                };
                self.record_waiter_outcome(waiter.request, mutation);
                for reply in waiter.replies {
                    let _ = reply.send(mutation);
                }
                return true;
            }
        }
        if self.request_id_below_floor((owner, request_id)) {
            return false;
        }
        false
    }

    pub(crate) fn release_owner(
        &mut self,
        owner: FileLockOwner,
        request_id: u64,
    ) -> ReleaseLockOwnerResult {
        let mutation_key = (owner, request_id);
        if let Some(receipt) = self.release_completed.get(&mutation_key).copied() {
            return ReleaseLockOwnerResult::Released(receipt);
        }
        if self.request_id_released(mutation_key) || self.request_id_below_floor(mutation_key) {
            return ReleaseLockOwnerResult::StaleRequestId;
        }
        if !self.can_record_release_fence(owner, request_id) {
            return ReleaseLockOwnerResult::QueueFull;
        }
        let mut released = 0;
        let mut interrupted_waiters = Vec::new();
        let mut affected_inodes = Vec::new();
        let inodes = self.inodes.keys().copied().collect::<Vec<_>>();
        for inode in inodes {
            let mut affected = false;
            if let Some(locks) = self.inodes.get_mut(&inode) {
                let before = locks.granted.len();
                locks.granted.retain(|lock| lock.owner != owner);
                released += before - locks.granted.len();
                affected |= before != locks.granted.len();
                let mut index = 0;
                while index < locks.waiters.len() {
                    if locks.waiters[index].request.owner == owner
                        && locks.waiters[index].request.request_id <= request_id
                    {
                        if let Some(waiter) = locks.waiters.remove(index) {
                            self.active_waiters = self.active_waiters.saturating_sub(1);
                            interrupted_waiters.push(waiter);
                            affected = true;
                        }
                    } else {
                        index += 1;
                    }
                }
            }
            if affected {
                affected_inodes.push(inode);
            }
        }
        for waiter in interrupted_waiters {
            let mutation = FileLockMutation {
                outcome: FileLockOutcome::Interrupted,
                owner_revision: 0,
            };
            self.record_waiter_outcome(waiter.request, mutation);
            for reply in waiter.replies {
                let _ = reply.send(mutation);
            }
        }
        let owner_revision = self.advance_lock_revision();
        let receipt = ReleaseLockReceipt {
            released,
            owner_revision,
        };
        self.record_release_completed(mutation_key, receipt);
        self.advance_release_fence(owner, request_id);
        for inode in affected_inodes {
            if !self.recovery_pending() {
                self.wake_waiters(inode);
            }
            self.remove_empty(inode);
        }
        ReleaseLockOwnerResult::Released(receipt)
    }

    pub(crate) fn release_node_epoch(&mut self, node_id: u64, node_epoch: u64) -> usize {
        let owners = self
            .inodes
            .values()
            .flat_map(|locks| locks.granted.iter().map(|lock| lock.owner))
            .chain(
                self.inodes
                    .values()
                    .flat_map(|locks| locks.waiters.iter().map(|waiter| waiter.request.owner)),
            )
            .filter(|owner| owner.node_id == node_id && owner.node_epoch == node_epoch)
            .collect::<HashSet<_>>();
        let released = owners
            .into_iter()
            .map(|owner| match self.release_owner(owner, u64::MAX) {
                ReleaseLockOwnerResult::Released(receipt) => receipt.released,
                ReleaseLockOwnerResult::QueueFull | ReleaseLockOwnerResult::StaleRequestId => 0,
            })
            .sum();
        self.clear_completed_for_node_epoch(node_id, node_epoch);
        released
    }

    pub(crate) fn reclaim_entry(&mut self, inode: InodeId, lock: GrantedFileLock) {
        self.inodes.entry(inode).or_default().granted.push(lock);
        self.normalize(inode);
    }

    /// 一个 Node 必须先重报完整本地镜像，再显式结束本轮 reclaim。否则第一条锁就
    /// 会过早解除恢复屏障，让新请求在剩余旧锁尚未恢复时进入。
    pub(crate) fn finish_reclaim(&mut self, node_id: u64) {
        self.recovering_nodes.remove(&node_id);
        if self.recovering_nodes.is_empty() {
            self.recovery_until = None;
            self.wake_all_waiters();
        }
    }

    fn first_conflict(&self, request: FileLockRequest) -> Option<GrantedFileLock> {
        self.inodes
            .get(&request.inode)?
            .granted
            .iter()
            .copied()
            .filter(|lock| lock.conflicts_with(request))
            .min_by_key(|lock| (lock.range.start, lock.range.end_inclusive))
    }

    fn replace_owner_range(&mut self, request: FileLockRequest) {
        self.unlock(request.inode, request.owner, request.range);
        self.inodes
            .entry(request.inode)
            .or_default()
            .granted
            .push(GrantedFileLock {
                owner: request.owner,
                range: request.range,
                mode: request.mode,
                pid: request.pid,
            });
        self.normalize(request.inode);
    }

    fn unlock(&mut self, inode: InodeId, owner: FileLockOwner, range: FileLockRange) {
        let Some(locks) = self.inodes.get_mut(&inode) else {
            return;
        };
        let mut retained = Vec::with_capacity(locks.granted.len() + 1);
        for held in locks.granted.drain(..) {
            if held.owner != owner || !held.range.overlaps(range) {
                retained.push(held);
                continue;
            }
            if held.range.start < range.start {
                retained.push(GrantedFileLock {
                    range: FileLockRange {
                        start: held.range.start,
                        end_inclusive: range.start - 1,
                    },
                    ..held
                });
            }
            if held.range.end_inclusive > range.end_inclusive && range.end_inclusive != u64::MAX {
                retained.push(GrantedFileLock {
                    range: FileLockRange {
                        start: range.end_inclusive + 1,
                        end_inclusive: held.range.end_inclusive,
                    },
                    ..held
                });
            }
        }
        locks.granted = retained;
        self.normalize(inode);
        self.remove_empty(inode);
    }

    fn attach_waiter_reply(
        &mut self,
        mutation_key: (FileLockOwner, u64),
        reply: FileLockReply,
    ) -> bool {
        for locks in self.inodes.values_mut() {
            let Some(waiter) = locks
                .waiters
                .iter_mut()
                .find(|waiter| (waiter.request.owner, waiter.request.request_id) == mutation_key)
            else {
                continue;
            };
            if waiter.replies.len() >= MAX_ATTACHED_REPLIES_PER_WAITER {
                return false;
            }
            waiter.replies.push(reply);
            return true;
        }
        false
    }

    fn has_waiter(&self, mutation_key: (FileLockOwner, u64)) -> bool {
        self.inodes.values().any(|locks| {
            locks
                .waiters
                .iter()
                .any(|waiter| (waiter.request.owner, waiter.request.request_id) == mutation_key)
        })
    }

    fn waiter_matches(
        &self,
        mutation_key: (FileLockOwner, u64),
        fingerprint: LockMutationFingerprint,
    ) -> bool {
        self.inodes.values().any(|locks| {
            locks.waiters.iter().any(|waiter| {
                (waiter.request.owner, waiter.request.request_id) == mutation_key
                    && LockMutationFingerprint::from(waiter.request) == fingerprint
            })
        })
    }

    fn request_id_below_floor(&self, mutation_key: (FileLockOwner, u64)) -> bool {
        self.completed_floor
            .get(&owner_epoch_key(mutation_key.0))
            .is_some_and(|floor| mutation_key.1 <= *floor)
    }

    fn request_id_released(&self, mutation_key: (FileLockOwner, u64)) -> bool {
        self.release_fence
            .get(&mutation_key.0)
            .is_some_and(|fence| mutation_key.1 <= *fence)
    }

    fn record_waiter_outcome(&mut self, request: FileLockRequest, mutation: FileLockMutation) {
        self.record_completed(
            (request.owner, request.request_id),
            mutation.outcome,
            LockMutationFingerprint::from(request),
            mutation.owner_revision,
        );
    }

    fn record_completed(
        &mut self,
        mutation_key: (FileLockOwner, u64),
        outcome: FileLockOutcome,
        fingerprint: LockMutationFingerprint,
        owner_revision: u64,
    ) {
        if self.completed.contains_key(&mutation_key) {
            return;
        }
        self.completed.insert(
            mutation_key,
            LockMutationReceipt {
                outcome,
                fingerprint,
                owner_revision,
            },
        );
        self.completed_order.push_back(mutation_key);
        self.prune_completed_for_owner(mutation_key.0);
        self.prune_completed_global();
    }

    fn record_release_completed(
        &mut self,
        mutation_key: (FileLockOwner, u64),
        receipt: ReleaseLockReceipt,
    ) {
        if self.release_completed.contains_key(&mutation_key) {
            return;
        }
        self.release_completed.insert(mutation_key, receipt);
        self.release_completed_order.push_back(mutation_key);
        self.prune_release_completed_global();
    }

    fn prune_release_completed_global(&mut self) {
        while self.release_completed_order.len() > MAX_RELEASE_RECEIPTS_GLOBAL {
            let Some(key) = self.release_completed_order.pop_front() else {
                break;
            };
            self.evict_release_completed(key);
        }
    }

    fn evict_release_completed(&mut self, mutation_key: (FileLockOwner, u64)) {
        self.release_completed.remove(&mutation_key);
    }

    fn prune_completed_for_owner(&mut self, owner: FileLockOwner) {
        while self
            .completed_order
            .iter()
            .filter(|(receipt_owner, _)| *receipt_owner == owner)
            .count()
            > MAX_COMPLETED_RECEIPTS_PER_OWNER
        {
            let Some(index) = self
                .completed_order
                .iter()
                .position(|(receipt_owner, _)| *receipt_owner == owner)
            else {
                break;
            };
            if let Some(key) = self.completed_order.remove(index) {
                self.evict_completed(key);
            }
        }
    }

    fn prune_completed_global(&mut self) {
        while self.completed_order.len() > MAX_COMPLETED_RECEIPTS_GLOBAL {
            let Some(key) = self.completed_order.pop_front() else {
                break;
            };
            self.evict_completed(key);
        }
    }

    fn evict_completed(&mut self, mutation_key: (FileLockOwner, u64)) {
        if self.completed.remove(&mutation_key).is_some() {
            self.advance_completed_floor(mutation_key.0, mutation_key.1);
        }
    }

    fn advance_completed_floor(&mut self, owner: FileLockOwner, request_id: u64) {
        self.completed_floor
            .entry(owner_epoch_key(owner))
            .and_modify(|floor| *floor = (*floor).max(request_id))
            .or_insert(request_id);
    }

    fn can_record_release_fence(&self, owner: FileLockOwner, request_id: u64) -> bool {
        self.release_fence.contains_key(&owner)
            || self.release_fence_order.len() < MAX_RELEASE_FENCES_GLOBAL
            || request_id == u64::MAX
    }

    fn advance_release_fence(&mut self, owner: FileLockOwner, request_id: u64) {
        if let Some(fence) = self.release_fence.get_mut(&owner) {
            *fence = (*fence).max(request_id);
        } else {
            self.release_fence.insert(owner, request_id);
            self.release_fence_order.push_back(owner);
        }
    }

    fn clear_completed_for_node_epoch(&mut self, node_id: u64, node_epoch: u64) {
        self.completed_order.retain(|(owner, request_id)| {
            let retain = owner.node_id != node_id || owner.node_epoch != node_epoch;
            if !retain {
                self.completed.remove(&(*owner, *request_id));
            }
            retain
        });
        self.release_completed_order.retain(|(owner, request_id)| {
            let retain = owner.node_id != node_id || owner.node_epoch != node_epoch;
            if !retain {
                self.release_completed.remove(&(*owner, *request_id));
            }
            retain
        });
        self.completed_floor
            .retain(|(floor_node_id, floor_epoch), _| {
                *floor_node_id != node_id || *floor_epoch != node_epoch
            });
        self.release_fence
            .retain(|owner, _| owner.node_id != node_id || owner.node_epoch != node_epoch);
        self.release_fence_order
            .retain(|owner| owner.node_id != node_id || owner.node_epoch != node_epoch);
    }

    fn wake_waiters(&mut self, inode: InodeId) {
        loop {
            let request = self
                .inodes
                .get(&inode)
                .and_then(|locks| locks.waiters.front().map(|waiter| waiter.request));
            let Some(request) = request else {
                break;
            };
            if self.first_conflict(request).is_some() {
                break;
            }
            let waiter = self
                .inodes
                .get_mut(&inode)
                .and_then(|locks| locks.waiters.pop_front())
                .expect("front waiter existed");
            self.active_waiters = self.active_waiters.saturating_sub(1);
            self.replace_owner_range(waiter.request);
            let outcome = FileLockOutcome::Acquired;
            let owner_revision = self.advance_lock_revision();
            let mutation = FileLockMutation {
                outcome,
                owner_revision,
            };
            self.record_waiter_outcome(waiter.request, mutation);
            for reply in waiter.replies {
                let _ = reply.send(mutation);
            }
        }
    }

    fn advance_lock_revision(&mut self) -> u64 {
        self.next_lock_revision = self
            .next_lock_revision
            .checked_add(1)
            .expect("filesystem lock revision exhausted");
        self.next_lock_revision
    }

    fn wake_all_waiters(&mut self) {
        let inodes = self.inodes.keys().copied().collect::<Vec<_>>();
        for inode in inodes {
            self.wake_waiters(inode);
            self.remove_empty(inode);
        }
    }

    fn normalize(&mut self, inode: InodeId) {
        let Some(locks) = self.inodes.get_mut(&inode) else {
            return;
        };
        locks.granted.sort_by_key(|lock| {
            (
                lock.owner.node_id,
                lock.owner.node_epoch,
                lock.owner.lock_owner,
                lock.mode as u8,
                lock.range.start,
            )
        });
        let mut normalized: Vec<GrantedFileLock> = Vec::with_capacity(locks.granted.len());
        for lock in locks.granted.drain(..) {
            if let Some(previous) = normalized.last_mut()
                && previous.owner == lock.owner
                && previous.mode == lock.mode
                && previous.range.adjacent_or_overlapping(lock.range)
            {
                previous.range.end_inclusive =
                    previous.range.end_inclusive.max(lock.range.end_inclusive);
                previous.pid = lock.pid;
                continue;
            }
            normalized.push(lock);
        }
        normalized.sort_by_key(|lock| (lock.range.start, lock.range.end_inclusive));
        locks.granted = normalized;
    }

    fn remove_empty(&mut self, inode: InodeId) {
        if self
            .inodes
            .get(&inode)
            .is_some_and(|locks| locks.granted.is_empty() && locks.waiters.is_empty())
        {
            self.inodes.remove(&inode);
        }
    }

    #[cfg(test)]
    fn granted(&self, inode: InodeId) -> &[GrantedFileLock] {
        self.inodes
            .get(&inode)
            .map_or(&[], |locks| locks.granted.as_slice())
    }

    #[cfg(test)]
    fn waiter_count(&self, inode: InodeId) -> usize {
        self.inodes
            .get(&inode)
            .map_or(0, |locks| locks.waiters.len())
    }

    #[cfg(test)]
    fn waiter_reply_count(&self, inode: InodeId) -> usize {
        self.inodes
            .get(&inode)
            .and_then(|locks| locks.waiters.front())
            .map_or(0, |waiter| waiter.replies.len())
    }

    #[cfg(test)]
    fn active_waiter_count(&self) -> usize {
        self.active_waiters
    }

    #[cfg(test)]
    fn release_fence_count(&self) -> usize {
        self.release_fence.len()
    }

    #[cfg(test)]
    fn release_fence_order_count(&self) -> usize {
        self.release_fence_order.len()
    }

    #[cfg(test)]
    fn release_completed_count(&self) -> usize {
        self.release_completed.len()
    }
}

fn owner_epoch_key(owner: FileLockOwner) -> (u64, u64) {
    (owner.node_id, owner.node_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(node: u64, epoch: u64, lock_owner: u64) -> FileLockOwner {
        FileLockOwner {
            node_id: node,
            node_epoch: epoch,
            lock_owner,
        }
    }

    fn request(
        id: u64,
        owner: FileLockOwner,
        start: u64,
        end: u64,
        mode: FileLockMode,
        wait: bool,
    ) -> FileLockRequest {
        FileLockRequest {
            request_id: id,
            inode: 9,
            owner,
            range: FileLockRange::new(start, end).unwrap(),
            mode,
            pid: id as u32,
            wait,
        }
    }

    fn request_for_inode(
        id: u64,
        inode: InodeId,
        owner: FileLockOwner,
        start: u64,
        end: u64,
        mode: FileLockMode,
        wait: bool,
    ) -> FileLockRequest {
        FileLockRequest {
            inode,
            ..request(id, owner, start, end, mode, wait)
        }
    }

    fn fill_global_waiter_cap(table: &mut FilesystemLockTable) {
        let holder = owner(1, 1, 10);
        let waiting = owner(2, 1, 20);
        for index in 1..=(MAX_ACTIVE_WAITERS_GLOBAL as u64) {
            let inode = 10_000 + index;
            assert!(matches!(
                table.set(
                    request_for_inode(index, inode, holder, 0, 9, FileLockMode::Exclusive, false),
                    None,
                    Instant::now(),
                ),
                SetLockResult::Ready(FileLockMutation {
                    outcome: FileLockOutcome::Acquired,
                    ..
                })
            ));
            let (reply, _receive) = oneshot::channel();
            assert!(matches!(
                table.set(
                    request_for_inode(index, inode, waiting, 0, 9, FileLockMode::Exclusive, true,),
                    Some(reply),
                    Instant::now(),
                ),
                SetLockResult::Waiting
            ));
        }
        assert_eq!(table.active_waiter_count(), MAX_ACTIVE_WAITERS_GLOBAL);
    }

    fn assert_released(result: ReleaseLockOwnerResult, released: usize) -> u64 {
        match result {
            ReleaseLockOwnerResult::Released(receipt) => {
                assert_eq!(receipt.released, released);
                assert!(
                    receipt.owner_revision > 0,
                    "accepted release must carry a Meta lock revision"
                );
                receipt.owner_revision
            }
            other => panic!("expected release affected={released}, got {other:?}"),
        }
    }

    #[test]
    fn partial_unlock_splits_an_existing_range() {
        let mut table = FilesystemLockTable::default();
        let held_by = owner(1, 1, 10);
        assert!(matches!(
            table.set(
                request(1, held_by, 0, 99, FileLockMode::Exclusive, false),
                None,
                Instant::now()
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        table.set(
            request(2, held_by, 40, 59, FileLockMode::Unlock, false),
            None,
            Instant::now(),
        );
        assert_eq!(
            table
                .granted(9)
                .iter()
                .map(|lock| lock.range)
                .collect::<Vec<_>>(),
            vec![
                FileLockRange::new(0, 39).unwrap(),
                FileLockRange::new(60, 99).unwrap(),
            ]
        );
    }

    #[tokio::test]
    async fn blocking_waiter_is_woken_after_conflicting_unlock() {
        let mut table = FilesystemLockTable::default();
        let first = owner(1, 1, 10);
        let second = owner(2, 1, 20);
        table.set(
            request(1, first, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        );
        let (reply, receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(2, second, 0, 9, FileLockMode::Exclusive, true),
                Some(reply),
                Instant::now()
            ),
            SetLockResult::Waiting
        ));
        table.set(
            request(3, first, 0, 9, FileLockMode::Unlock, false),
            None,
            Instant::now(),
        );
        assert_eq!(receive.await.unwrap().outcome, FileLockOutcome::Acquired);
    }

    #[tokio::test]
    async fn cancellation_has_one_terminal_reply() {
        let mut table = FilesystemLockTable::default();
        let first = owner(1, 1, 10);
        let second = owner(2, 1, 20);
        table.set(
            request(1, first, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        );
        let (reply, receive) = oneshot::channel();
        table.set(
            request(2, second, 0, 9, FileLockMode::Exclusive, true),
            Some(reply),
            Instant::now(),
        );
        assert!(table.cancel(2, second));
        assert!(
            table.cancel(2, second),
            "cancel 响应丢失后的同 request_id retry 必须命中 Interrupted receipt"
        );
        assert_eq!(receive.await.unwrap().outcome, FileLockOutcome::Interrupted);
    }

    #[test]
    fn stale_epoch_release_cannot_remove_new_epoch_lock() {
        let mut table = FilesystemLockTable::default();
        let old = owner(7, 3, 10);
        let current = owner(7, 4, 10);
        table.set(
            request(1, current, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        );
        table.release_node_epoch(old.node_id, old.node_epoch);
        assert_eq!(table.granted(9).len(), 1);
        assert_eq!(table.granted(9)[0].owner, current);
    }

    #[test]
    fn retrying_completed_request_id_does_not_mutate_granted_locks_again() {
        let mut table = FilesystemLockTable::default();
        let owner = owner(1, 1, 10);
        assert!(matches!(
            table.set(
                request(7, owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));

        let retry = table.set(
            request(7, owner, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        );

        assert!(matches!(
            retry,
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        assert_eq!(
            table
                .granted(9)
                .iter()
                .map(|lock| lock.range)
                .collect::<Vec<_>>(),
            vec![FileLockRange::new(0, 9).unwrap()],
            "同一 request_id 的重试只能返回第一次终态，不能把锁范围扩大成第二把锁"
        );
    }

    #[tokio::test]
    async fn retrying_waiting_request_broadcasts_one_terminal_result_without_duplicate_waiters() {
        let mut table = FilesystemLockTable::default();
        let first = owner(1, 1, 10);
        let second = owner(2, 1, 20);
        table.set(
            request(1, first, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        );

        let (first_reply, first_receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(2, second, 0, 9, FileLockMode::Exclusive, true),
                Some(first_reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));
        let (retry_reply, retry_receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(2, second, 0, 9, FileLockMode::Exclusive, true),
                Some(retry_reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));
        assert_eq!(table.waiter_count(9), 1);

        table.set(
            request(3, first, 0, 9, FileLockMode::Unlock, false),
            None,
            Instant::now(),
        );

        assert_eq!(
            first_receive.await.unwrap().outcome,
            FileLockOutcome::Acquired
        );
        assert_eq!(
            retry_receive.await.unwrap().outcome,
            FileLockOutcome::Acquired
        );
        assert_eq!(table.waiter_count(9), 0);
        assert_eq!(
            table
                .granted(9)
                .iter()
                .filter(|lock| lock.owner == second)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn retrying_waiter_reply_cap_applies_backpressure_without_losing_original() {
        let mut table = FilesystemLockTable::default();
        let first = owner(1, 1, 10);
        let second = owner(2, 1, 20);
        table.set(
            request(1, first, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        );

        let (original_reply, original_receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(2, second, 0, 9, FileLockMode::Exclusive, true),
                Some(original_reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));
        for _ in 1..MAX_ATTACHED_REPLIES_PER_WAITER {
            let (reply, _receive) = oneshot::channel();
            assert!(matches!(
                table.set(
                    request(2, second, 0, 9, FileLockMode::Exclusive, true),
                    Some(reply),
                    Instant::now(),
                ),
                SetLockResult::Waiting
            ));
        }
        assert_eq!(table.waiter_count(9), 1);
        assert_eq!(table.waiter_reply_count(9), MAX_ATTACHED_REPLIES_PER_WAITER);

        let (overflow_reply, _overflow_receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(2, second, 0, 9, FileLockMode::Exclusive, true),
                Some(overflow_reply),
                Instant::now(),
            ),
            SetLockResult::ReplyQueueFull
        ));
        assert_eq!(
            table.waiter_reply_count(9),
            MAX_ATTACHED_REPLIES_PER_WAITER,
            "超过上限的 retry 不能继续增长 replies Vec"
        );

        table.set(
            request(3, first, 0, 9, FileLockMode::Unlock, false),
            None,
            Instant::now(),
        );
        assert_eq!(
            original_receive.await.unwrap().outcome,
            FileLockOutcome::Acquired
        );
    }

    #[test]
    fn global_waiter_cap_spans_inodes() {
        let mut table = FilesystemLockTable::default();
        fill_global_waiter_cap(&mut table);

        let holder = owner(1, 1, 10);
        let waiting = owner(2, 1, 20);
        let overflow_inode = 99_999;
        assert!(matches!(
            table.set(
                request_for_inode(
                    90_000,
                    overflow_inode,
                    holder,
                    0,
                    9,
                    FileLockMode::Exclusive,
                    false,
                ),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        let (reply, _receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request_for_inode(
                    90_000,
                    overflow_inode,
                    waiting,
                    0,
                    9,
                    FileLockMode::Exclusive,
                    true,
                ),
                Some(reply),
                Instant::now(),
            ),
            SetLockResult::QueueFull
        ));
        assert_eq!(table.active_waiter_count(), MAX_ACTIVE_WAITERS_GLOBAL);
    }

    #[test]
    fn global_waiter_capacity_recovers_after_cancel_release_and_wakeup() {
        let holder = owner(1, 1, 10);
        let waiting = owner(2, 1, 20);

        let mut cancel_table = FilesystemLockTable::default();
        fill_global_waiter_cap(&mut cancel_table);
        assert!(cancel_table.cancel(1, waiting));
        assert_eq!(
            cancel_table.active_waiter_count(),
            MAX_ACTIVE_WAITERS_GLOBAL - 1
        );
        assert!(matches!(
            cancel_table.set(
                request_for_inode(99_001, 99_001, holder, 0, 9, FileLockMode::Exclusive, false,),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        let (reply, _receive) = oneshot::channel();
        assert!(matches!(
            cancel_table.set(
                request_for_inode(99_001, 99_001, waiting, 0, 9, FileLockMode::Exclusive, true,),
                Some(reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));

        let mut release_table = FilesystemLockTable::default();
        fill_global_waiter_cap(&mut release_table);
        release_table.release_owner(waiting, 90_000);
        assert_eq!(release_table.active_waiter_count(), 0);
        assert!(matches!(
            release_table.set(
                request_for_inode(90_001, 99_002, holder, 0, 9, FileLockMode::Exclusive, false,),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        let (reply, _receive) = oneshot::channel();
        assert!(matches!(
            release_table.set(
                request_for_inode(90_001, 99_002, waiting, 0, 9, FileLockMode::Exclusive, true,),
                Some(reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));

        let mut wake_table = FilesystemLockTable::default();
        fill_global_waiter_cap(&mut wake_table);
        assert!(matches!(
            wake_table.set(
                request_for_inode(90_000, 10_001, holder, 0, 9, FileLockMode::Unlock, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Released,
                ..
            })
        ));
        assert_eq!(
            wake_table.active_waiter_count(),
            MAX_ACTIVE_WAITERS_GLOBAL - 1
        );
        assert!(matches!(
            wake_table.set(
                request_for_inode(90_001, 99_003, holder, 0, 9, FileLockMode::Exclusive, false,),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        let (reply, _receive) = oneshot::channel();
        assert!(matches!(
            wake_table.set(
                request_for_inode(90_001, 99_003, waiting, 0, 9, FileLockMode::Exclusive, true,),
                Some(reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));
    }

    #[tokio::test]
    async fn waiting_retry_attaches_before_node_epoch_floor_check() {
        let mut table = FilesystemLockTable::default();
        let first = owner(1, 1, 10);
        let waiting = owner(2, 1, 20);
        table.set(
            request(1, first, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        );

        let (first_reply, first_receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(2, waiting, 0, 9, FileLockMode::Exclusive, true),
                Some(first_reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));

        for index in 3..=(MAX_COMPLETED_RECEIPTS_PER_OWNER as u64 + 3) {
            assert!(matches!(
                table.set(
                    request(
                        index,
                        waiting,
                        1_000 + index,
                        1_000 + index,
                        FileLockMode::Shared,
                        false,
                    ),
                    None,
                    Instant::now(),
                ),
                SetLockResult::Ready(FileLockMutation {
                    outcome: FileLockOutcome::Acquired,
                    ..
                })
            ));
        }
        assert!(
            table.request_id_below_floor((waiting, 2)),
            "test setup must advance the node epoch floor beyond the waiting mutation"
        );

        let (retry_reply, retry_receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(2, waiting, 0, 9, FileLockMode::Exclusive, true),
                Some(retry_reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));
        assert_eq!(table.waiter_count(9), 1);

        table.set(
            request(
                MAX_COMPLETED_RECEIPTS_PER_OWNER as u64 + 4,
                first,
                0,
                9,
                FileLockMode::Unlock,
                false,
            ),
            None,
            Instant::now(),
        );

        assert_eq!(
            first_receive.await.unwrap().outcome,
            FileLockOutcome::Acquired
        );
        assert_eq!(
            retry_receive.await.unwrap().outcome,
            FileLockOutcome::Acquired
        );
    }

    #[tokio::test]
    async fn active_waiter_cancel_ignores_node_epoch_floor() {
        let mut table = FilesystemLockTable::default();
        let holder = owner(1, 1, 10);
        let waiting = owner(2, 1, 20);
        assert!(matches!(
            table.set(
                request(1, holder, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));

        let (reply, receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(1, waiting, 0, 9, FileLockMode::Exclusive, true),
                Some(reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));

        for index in 2..=(MAX_COMPLETED_RECEIPTS_PER_OWNER as u64 + 2) {
            assert!(matches!(
                table.set(
                    request(
                        index,
                        waiting,
                        10_000 + index,
                        10_000 + index,
                        FileLockMode::Shared,
                        false,
                    ),
                    None,
                    Instant::now(),
                ),
                SetLockResult::Ready(FileLockMutation {
                    outcome: FileLockOutcome::Acquired,
                    ..
                })
            ));
        }
        assert!(
            table.request_id_below_floor((waiting, 1)),
            "test setup must advance node-epoch floor past the active waiter"
        );

        assert!(
            table.cancel(1, waiting),
            "active waiter must be cancellable even after later receipts advance floor"
        );
        assert_eq!(table.waiter_count(9), 0);
        assert_eq!(receive.await.unwrap().outcome, FileLockOutcome::Interrupted);
        assert!(
            table.cancel(1, waiting),
            "lost cancel reply retry must hit the Interrupted receipt"
        );
    }

    #[test]
    fn release_owner_fence_rejects_late_set_from_same_owner() {
        let mut table = FilesystemLockTable::default();
        let lock_owner = owner(1, 1, 10);
        let other_owner = owner(1, 1, 11);

        assert_released(table.release_owner(lock_owner, 10), 0);
        assert!(matches!(
            table.set(
                request(9, lock_owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::StaleRequestId
        ));
        assert!(matches!(
            table.set(
                request(11, lock_owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        assert!(matches!(
            table.set(
                request(5, other_owner, 10, 19, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
    }

    #[test]
    fn release_owner_fences_are_globally_bounded_and_evicted_sets_do_not_resurrect() {
        let mut table = FilesystemLockTable::default();
        for index in 1..=(MAX_RELEASE_FENCES_GLOBAL as u64) {
            assert_released(table.release_owner(owner(1, 1, index), index), 0);
        }
        assert_eq!(
            table.release_owner(
                owner(1, 1, MAX_RELEASE_FENCES_GLOBAL as u64 + 1),
                MAX_RELEASE_FENCES_GLOBAL as u64 + 1
            ),
            ReleaseLockOwnerResult::QueueFull,
            "cap overflow must apply explicit backpressure, not unsafe tombstone eviction"
        );

        assert_eq!(table.release_fence_count(), MAX_RELEASE_FENCES_GLOBAL);
        assert_eq!(table.release_fence_order_count(), MAX_RELEASE_FENCES_GLOBAL);
        assert_eq!(
            table.completed_floor.len(),
            0,
            "release tombstone cap must not advance node-epoch floor and reject other owners"
        );
        assert!(matches!(
            table.set(
                request(1, owner(1, 1, 1), 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::StaleRequestId
        ));
        assert!(
            matches!(
                table.set(
                    request(
                        1,
                        owner(1, 1, MAX_RELEASE_FENCES_GLOBAL as u64 + 2),
                        10,
                        19,
                        FileLockMode::Exclusive,
                        false
                    ),
                    None,
                    Instant::now(),
                ),
                SetLockResult::Ready(FileLockMutation {
                    outcome: FileLockOutcome::Acquired,
                    ..
                })
            ),
            "different owner with a low in-flight sequence must not be rejected by fence cap"
        );
    }

    #[test]
    fn release_owner_exact_retry_returns_original_affected_count() {
        let mut table = FilesystemLockTable::default();
        let lock_owner = owner(1, 1, 10);
        assert!(matches!(
            table.set(
                request(1, lock_owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        assert!(matches!(
            table.set(
                request(2, lock_owner, 20, 29, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));

        let first_release_revision = assert_released(table.release_owner(lock_owner, 3), 2);
        assert_eq!(table.granted(9), &[]);
        assert_eq!(table.granted(29), &[]);
        let retry_release_revision = assert_released(table.release_owner(lock_owner, 3), 2);
        assert_eq!(
            retry_release_revision, first_release_revision,
            "lost release response retry must hit the exact receipt, not recompute 0"
        );
    }

    #[test]
    fn owner_revision_follows_meta_application_order_not_request_sequence() {
        let mut table = FilesystemLockTable::default();
        let lock_owner = owner(1, 1, 10);
        let set_revision = match table.set(
            request(3, lock_owner, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        ) {
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                owner_revision,
            }) => owner_revision,
            _ => panic!("expected acquired set"),
        };

        let release_revision = assert_released(table.release_owner(lock_owner, 2), 1);

        assert!(
            release_revision > set_revision,
            "Meta revision must describe actual actor application order, not Node request sequence"
        );
        assert_eq!(table.granted(9), &[]);
    }

    #[tokio::test]
    async fn release_revision_precedes_waiters_woken_by_that_release() {
        let mut table = FilesystemLockTable::default();
        let releasing_owner = owner(1, 1, 10);
        let waiting_owner = owner(2, 1, 20);
        assert!(matches!(
            table.set(
                request(1, releasing_owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        let (reply, receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(3, waiting_owner, 0, 9, FileLockMode::Exclusive, true),
                Some(reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));

        let release_revision = assert_released(table.release_owner(releasing_owner, 2), 1);
        let waiter_mutation = receive.await.unwrap();

        assert_eq!(waiter_mutation.outcome, FileLockOutcome::Acquired);
        assert!(
            waiter_mutation.owner_revision > release_revision,
            "waiter woken by release must be linearized after the release mutation"
        );
    }

    #[test]
    fn evicted_release_receipt_rejects_stale_retry_instead_of_silent_success() {
        let mut table = FilesystemLockTable::default();
        let lock_owner = owner(1, 1, 7);
        for index in 1..=(MAX_RELEASE_RECEIPTS_GLOBAL as u64 + 1) {
            assert_released(table.release_owner(lock_owner, index), 0);
        }

        assert_eq!(table.release_completed_count(), MAX_RELEASE_RECEIPTS_GLOBAL);
        assert_eq!(
            table.release_owner(lock_owner, 1),
            ReleaseLockOwnerResult::StaleRequestId,
            "once exact receipt is evicted, stale release must not look like success"
        );
    }

    #[test]
    fn release_owner_fence_cleanup_follows_node_epoch_lifecycle() {
        let mut table = FilesystemLockTable::default();
        for index in 1..=8 {
            assert_released(table.release_owner(owner(1, 7, index), index), 0);
        }
        assert_eq!(table.release_fence_count(), 8);
        table.release_node_epoch(1, 7);
        assert_eq!(table.release_fence_count(), 0);
        assert_eq!(table.release_fence_order_count(), 0);
        assert!(table.completed_floor.is_empty());
    }

    #[tokio::test]
    async fn release_owner_records_interrupted_receipt_for_removed_waiter() {
        let mut table = FilesystemLockTable::default();
        let holder = owner(1, 1, 10);
        let waiting = owner(2, 1, 20);
        table.set(
            request(1, holder, 0, 9, FileLockMode::Exclusive, false),
            None,
            Instant::now(),
        );
        let (reply, receive) = oneshot::channel();
        assert!(matches!(
            table.set(
                request(2, waiting, 0, 9, FileLockMode::Exclusive, true),
                Some(reply),
                Instant::now(),
            ),
            SetLockResult::Waiting
        ));

        assert_released(table.release_owner(waiting, 3), 0);
        assert_eq!(receive.await.unwrap().outcome, FileLockOutcome::Interrupted);
        assert!(matches!(
            table.set(
                request(2, waiting, 0, 9, FileLockMode::Exclusive, true),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Interrupted,
                ..
            })
        ));
        assert!(matches!(
            table.set(
                request(1, waiting, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::StaleRequestId
        ));
    }

    #[test]
    fn same_request_id_with_different_payload_is_rejected() {
        let mut table = FilesystemLockTable::default();
        let owner = owner(1, 1, 10);
        assert!(matches!(
            table.set(
                request(7, owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));

        assert!(matches!(
            table.set(
                request(7, owner, 10, 19, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::RequestMismatch
        ));
    }

    #[test]
    fn recovery_pending_is_not_persisted_as_completed_receipt() {
        let mut table = FilesystemLockTable::default();
        let owner = owner(1, 1, 10);
        table.begin_recovery(Instant::now() + std::time::Duration::from_secs(30), [9]);
        assert!(matches!(
            table.set(
                request(1, owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::RecoveryPending,
                ..
            })
        ));
        assert!(!table.completed.contains_key(&(owner, 1)));

        table.finish_reclaim(9);
        assert!(matches!(
            table.set(
                request(1, owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
    }

    #[test]
    fn completed_receipts_are_pruned_by_node_epoch() {
        let mut table = FilesystemLockTable::default();
        let old_owner = owner(7, 3, 10);
        let current_owner = owner(7, 4, 10);
        assert!(matches!(
            table.set(
                request(1, old_owner, 0, 9, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        assert!(matches!(
            table.set(
                request(1, current_owner, 10, 19, FileLockMode::Exclusive, false),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));

        table.release_node_epoch(old_owner.node_id, old_owner.node_epoch);

        assert!(!table.completed.contains_key(&(old_owner, 1)));
        assert!(table.completed.contains_key(&(current_owner, 1)));
    }

    #[test]
    fn completed_receipts_are_bounded_per_owner_but_retry_window_still_works() {
        let mut table = FilesystemLockTable::default();
        let owner = owner(1, 1, 10);
        for index in 1..=(MAX_COMPLETED_RECEIPTS_PER_OWNER as u64 + 1) {
            assert!(matches!(
                table.set(
                    request(
                        index,
                        owner,
                        index * 2,
                        index * 2,
                        FileLockMode::Shared,
                        false,
                    ),
                    None,
                    Instant::now(),
                ),
                SetLockResult::Ready(FileLockMutation {
                    outcome: FileLockOutcome::Acquired,
                    ..
                })
            ));
        }

        assert_eq!(
            table
                .completed
                .keys()
                .filter(|(receipt_owner, _)| *receipt_owner == owner)
                .count(),
            MAX_COMPLETED_RECEIPTS_PER_OWNER
        );
        assert!(
            !table.completed.contains_key(&(owner, 1)),
            "超过窗口的最旧 receipt 必须被裁剪，避免 HashMap 无界增长"
        );
        assert!(matches!(
            table.set(
                request(
                    MAX_COMPLETED_RECEIPTS_PER_OWNER as u64 + 1,
                    owner,
                    (MAX_COMPLETED_RECEIPTS_PER_OWNER as u64 + 1) * 2,
                    (MAX_COMPLETED_RECEIPTS_PER_OWNER as u64 + 1) * 2,
                    FileLockMode::Shared,
                    false,
                ),
                None,
                Instant::now(),
            ),
            SetLockResult::Ready(FileLockMutation {
                outcome: FileLockOutcome::Acquired,
                ..
            })
        ));
        assert!(matches!(
            table.set(
                request(1, owner, 0, 0, FileLockMode::Shared, false),
                None,
                Instant::now(),
            ),
            SetLockResult::StaleRequestId
        ));
    }

    #[test]
    fn completed_receipts_have_global_cap_and_evicted_requests_do_not_resurrect() {
        let mut table = FilesystemLockTable::default();
        for index in 1..=(MAX_COMPLETED_RECEIPTS_GLOBAL as u64 + 1) {
            assert!(matches!(
                table.set(
                    request(
                        index,
                        owner(1, 1, index),
                        index * 2,
                        index * 2,
                        FileLockMode::Shared,
                        false,
                    ),
                    None,
                    Instant::now(),
                ),
                SetLockResult::Ready(FileLockMutation {
                    outcome: FileLockOutcome::Acquired,
                    ..
                })
            ));
        }

        assert_eq!(table.completed.len(), MAX_COMPLETED_RECEIPTS_GLOBAL);
        assert_eq!(table.completed_order.len(), MAX_COMPLETED_RECEIPTS_GLOBAL);
        assert_eq!(
            table.completed_floor.len(),
            1,
            "floor 必须按 node epoch 聚合，不能随 lock owner 数量增长"
        );
        assert!(matches!(
            table.set(
                request(1, owner(1, 1, 1), 0, 0, FileLockMode::Shared, false),
                None,
                Instant::now(),
            ),
            SetLockResult::StaleRequestId
        ));
    }

    #[test]
    fn recovery_window_fences_new_acquires_until_reclaim_finishes() {
        let mut table = FilesystemLockTable::default();
        table.begin_recovery(Instant::now() + std::time::Duration::from_secs(30), [1]);
        let requested = request(1, owner(2, 1, 1), 0, 9, FileLockMode::Exclusive, false);
        assert_eq!(
            table.test(requested, Instant::now()),
            FileLockOutcome::RecoveryPending
        );
        table.reclaim_entry(
            9,
            GrantedFileLock {
                owner: owner(1, 2, 7),
                range: FileLockRange::new(0, 9).unwrap(),
                mode: FileLockMode::Exclusive,
                pid: 10,
            },
        );
        table.finish_reclaim(1);
        assert!(matches!(
            table.test(requested, Instant::now()),
            FileLockOutcome::Conflict(_)
        ));
    }
}

//! 跨 Node 文件锁的无状态领域合同。
//!
//! 这里没有锁表、RPC 或 FUSE 类型。Meta 用这些值做唯一冲突判断，Node 用同一组
//! 值维护本挂载已取得锁的镜像；`pid` 只用于 `F_GETLK` 诊断，不参与 owner 身份。

use super::InodeId;

/// 一个跨 Node 唯一的 POSIX 锁 owner。
///
/// `lock_owner` 由内核/FUSE 提供；Node incarnation 由 Meta session 分配。三者组合
/// 避免 Node 重启后复用相同数值而误操作旧锁。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FileLockOwner {
    pub(crate) node_id: u64,
    pub(crate) node_epoch: u64,
    pub(crate) lock_owner: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileLockRange {
    pub(crate) start: u64,
    /// `u64::MAX` 表示直到 EOF。
    pub(crate) end_inclusive: u64,
}

impl FileLockRange {
    pub(crate) fn new(start: u64, end_inclusive: u64) -> Result<Self, FileLockContractError> {
        if start > end_inclusive {
            return Err(FileLockContractError::InvalidRange);
        }
        Ok(Self {
            start,
            end_inclusive,
        })
    }

    pub(crate) fn overlaps(self, other: Self) -> bool {
        self.start <= other.end_inclusive && other.start <= self.end_inclusive
    }

    pub(crate) fn adjacent_or_overlapping(self, other: Self) -> bool {
        self.overlaps(other)
            || self.end_inclusive.checked_add(1) == Some(other.start)
            || other.end_inclusive.checked_add(1) == Some(self.start)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileLockMode {
    Shared,
    Exclusive,
    Unlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileLockRequest {
    pub(crate) request_id: u64,
    pub(crate) inode: InodeId,
    pub(crate) owner: FileLockOwner,
    pub(crate) range: FileLockRange,
    pub(crate) mode: FileLockMode,
    pub(crate) pid: u32,
    pub(crate) wait: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GrantedFileLock {
    pub(crate) owner: FileLockOwner,
    pub(crate) range: FileLockRange,
    pub(crate) mode: FileLockMode,
    pub(crate) pid: u32,
}

impl GrantedFileLock {
    pub(crate) fn conflicts_with(self, request: FileLockRequest) -> bool {
        self.owner != request.owner
            && self.range.overlaps(request.range)
            && !matches!(
                (self.mode, request.mode),
                (FileLockMode::Shared, FileLockMode::Shared)
            )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileLockOutcome {
    Acquired,
    Released,
    Conflict(GrantedFileLock),
    Interrupted,
    /// Meta 正在等待重启前的存活 Node 重报锁；调用方应有界重试。
    RecoveryPending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileLockContractError {
    InvalidRange,
    InvalidMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_range_accepts_eof_and_rejects_reverse_order() {
        assert_eq!(
            FileLockRange::new(7, u64::MAX).expect("EOF range"),
            FileLockRange {
                start: 7,
                end_inclusive: u64::MAX,
            }
        );
        assert_eq!(
            FileLockRange::new(8, 7),
            Err(FileLockContractError::InvalidRange)
        );
    }

    #[test]
    fn shared_locks_are_compatible_but_exclusive_locks_conflict() {
        let held = GrantedFileLock {
            owner: FileLockOwner {
                node_id: 1,
                node_epoch: 1,
                lock_owner: 10,
            },
            range: FileLockRange::new(0, 99).unwrap(),
            mode: FileLockMode::Shared,
            pid: 100,
        };
        let mut requested = FileLockRequest {
            request_id: 1,
            inode: 7,
            owner: FileLockOwner {
                node_id: 2,
                node_epoch: 1,
                lock_owner: 20,
            },
            range: FileLockRange::new(50, 150).unwrap(),
            mode: FileLockMode::Shared,
            pid: 200,
            wait: false,
        };
        assert!(!held.conflicts_with(requested));
        requested.mode = FileLockMode::Exclusive;
        assert!(held.conflicts_with(requested));
    }
}

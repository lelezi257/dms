//! Same-path native exports for managed OwnerFs workspaces.
//!
//! This module controls mount identity and export state. It does not grant root
//! authority, delete backing data, fence peers, or prove Agent drainage. Those
//! are prerequisites of the OwnerFs lifecycle integration. A detached export
//! must never be reported as a reclaimed workspace solely from this state.

mod events;
mod flock;
mod home;
mod journal;
mod linux;
mod manager;
mod mountinfo;

pub use events::{
    WorkspaceCreated, WorkspaceEventReceiver, WorkspaceEventSender, workspace_event_channel,
};
pub use home::HomeExportAuthority;
pub use journal::{JournalRecord, JournalSnapshot, MountJournal, OrphanMaintenance};
pub use linux::{LinuxMountBackend, MountPolicy};
pub use manager::{MountBackend, NativeMountManager};
pub use mountinfo::MountInfo;
use serde::{Deserialize, Serialize};

use super::{
    super::{
        locks::{
            LockError, LockRequest, LockTable, LockTableLimits, LockWaiterId, LockWaiterOutcome,
        },
        types::{FileHandle, FileLockConflict, FileLockKind, FileLockOwner},
    },
    OpenFileHandleSlot,
};
use std::{
    fs::File,
    sync::{Mutex, Weak},
};

/// OwnerFs-only routing; DFS and ordinary FUSE-only mounts keep LockTable.
pub(super) struct OwnerLockTable {
    model: LockTable,
    flock: Option<flock::NativeFlocks>,
}

impl OwnerLockTable {
    pub(super) fn new(limits: LockTableLimits, native: bool) -> Self {
        Self {
            model: LockTable::new(limits),
            flock: native.then(flock::NativeFlocks::default),
        }
    }

    pub(super) fn prepare_flock(
        &self,
        handle: FileHandle,
        owner: &FileLockOwner,
        file: File,
        slot: Weak<Mutex<OpenFileHandleSlot>>,
    ) -> Result<(), LockError> {
        if let Some(flock) = &self.flock {
            flock.prepare(handle, owner, file, slot)?;
        }
        Ok(())
    }

    pub(super) fn native_flock(&self) -> bool {
        self.flock.is_some()
    }

    pub(super) fn getlk(
        &self,
        request: &LockRequest,
    ) -> Result<Option<FileLockConflict>, LockError> {
        if request.kind == FileLockKind::Flock && self.flock.is_some() {
            // Linux flock has no GETLK query. Do not invent a native conflict
            // PID or claim that a userspace table describes kernel ownership.
            return Err(LockError::InvalidRange);
        }
        self.model.getlk(request)
    }

    pub(super) fn setlk_nonblocking(&self, request: LockRequest) -> Result<(), LockError> {
        if request.kind == FileLockKind::Flock
            && let Some(flock) = &self.flock
        {
            return flock.setlk(request, None);
        }
        self.model.setlk_nonblocking(request)
    }

    pub(super) fn setlk_blocking(
        &self,
        request: LockRequest,
        waiter: LockWaiterId,
    ) -> Result<(), LockError> {
        if request.kind == FileLockKind::Flock
            && let Some(flock) = &self.flock
        {
            return flock.setlk(request, Some(waiter));
        }
        self.model.setlk_blocking(request, waiter)
    }

    pub(super) fn cancel_waiter_with_outcome(
        &self,
        waiter: LockWaiterId,
    ) -> Result<LockWaiterOutcome, LockError> {
        let native = match &self.flock {
            Some(flock) => flock.cancel(waiter.clone())?,
            None => LockWaiterOutcome::Unknown,
        };
        let model = self.model.cancel_waiter_with_outcome(waiter)?;
        Ok(if native == LockWaiterOutcome::Unknown {
            model
        } else {
            native
        })
    }

    pub(super) fn acknowledge_waiter(
        &self,
        waiter: &LockWaiterId,
    ) -> Result<Option<LockWaiterOutcome>, LockError> {
        let native = match &self.flock {
            Some(flock) => flock.acknowledge(waiter)?,
            None => None,
        };
        let model = self.model.acknowledge_waiter(waiter)?;
        Ok(native.or(model))
    }

    pub(super) fn release_posix_owner(&self, owner: &FileLockOwner) -> Result<(), LockError> {
        self.model.release_posix_owner(owner)
    }

    pub(super) fn release_flock_owner(&self, owner: &FileLockOwner) -> Result<(), LockError> {
        if let Some(flock) = &self.flock {
            flock.release_owner(owner)?;
        }
        self.model.release_flock_owner(owner)
    }

    pub(super) fn release_handle(&self, handle: FileHandle) -> Result<(), LockError> {
        if let Some(flock) = &self.flock {
            flock.release_handle(handle)?;
        }
        Ok(())
    }

    pub(super) fn release_session(&self, scope: &str) -> Result<(), LockError> {
        let native = match &self.flock {
            Some(flock) => flock.release_session(scope),
            None => Ok(()),
        };
        let model = self.model.release_session(scope);
        native.and(model)
    }

    pub(super) fn invalidate(&self) -> Result<(), LockError> {
        let native = match &self.flock {
            Some(flock) => flock.invalidate(),
            None => Ok(()),
        };
        let model = self.model.invalidate();
        native.and(model)
    }

    pub(super) fn is_idle(&self) -> Result<bool, LockError> {
        let native = match &self.flock {
            Some(flock) => flock.is_idle()?,
            None => true,
        };
        Ok(native && self.model.is_idle()?)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct NamespaceIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryIdentity {
    pub device: u64,
    pub inode: u64,
}

/// Identity supplied by the trusted Home authority, not a user pathname.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceIdentity {
    pub root_id: String,
    pub epoch: u64,
    pub home_node_id: String,
    pub home_session_id: String,
    pub namespace: NamespaceIdentity,
}

/// Prepared, pinned directory identities. `target` identifies the FUSE
/// directory covered by the export, not the native inode visible afterward.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceMount {
    pub identity: WorkspaceIdentity,
    pub source: DirectoryIdentity,
    pub target: DirectoryIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MountIdentity {
    /// Diagnostic ID matching mountinfo; Linux can recycle this number.
    pub mount_id: u64,
    /// STATX_MNT_ID_UNIQUE, never reused within one boot (Linux >= 6.8).
    /// Boot identity is persisted separately in the recovery journal.
    pub unique_mount_id: u64,
    pub namespace: NamespaceIdentity,
    pub source: DirectoryIdentity,
    pub covered_target: DirectoryIdentity,
}

impl MountIdentity {
    fn matches(&self, spec: &WorkspaceMount) -> bool {
        self.mount_id != 0
            && self.unique_mount_id != 0
            && self.namespace == spec.identity.namespace
            && self.source == spec.source
            && self.covered_target == spec.target
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NativeState {
    FuseReady,
    Mounting,
    NativeActive,
    FuseOnly,
    Quiescing,
    Unmounting,
    Draining,
    Detached,
    Recovering,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NativeDesiredState {
    Native,
    Detached,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeStatus {
    pub identity: WorkspaceIdentity,
    pub desired: NativeDesiredState,
    pub state: NativeState,
    pub operation_seq: u64,
    /// Observation is not ownership. Foreign observations are diagnostic only.
    pub observed: Option<MountIdentity>,
    pub last_errno: Option<i32>,
    pub last_error: Option<String>,
}

//! Same-path native exports for managed OwnerFs workspaces.
//!
//! This module controls mount identity and export state. It does not grant root
//! authority, delete backing data, fence peers, or prove Agent drainage. Those
//! are prerequisites of the OwnerFs lifecycle integration. A detached export
//! must never be reported as a reclaimed workspace solely from this state.

mod events;
mod journal;
mod linux;
mod manager;
mod mountinfo;

pub use events::{
    WorkspaceCreated, WorkspaceEventReceiver, WorkspaceEventSender, workspace_event_channel,
};
pub use journal::{JournalRecord, JournalSnapshot, MountJournal, OrphanMaintenance};
pub use linux::{LinuxMountBackend, MountPolicy};
pub use manager::{MountBackend, NativeMountManager};
pub use mountinfo::MountInfo;
use serde::{Deserialize, Serialize};

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

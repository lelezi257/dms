//! Shared DistributedFs identities and immutable metadata records.
//!
//! Mutable POSIX state is represented by an `InodeRecord` whose head points at
//! an immutable `FileVersion`. A version owns an immutable extent layout; every
//! extent refers to an immutable chunk. Physical copies are tracked separately.

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
        }
    };
}

string_id!(NamespaceId);
string_id!(InodeId);
string_id!(FileVersionId);
string_id!(LayoutRootId);
string_id!(ChunkId);
string_id!(CopyId);
string_id!(OperationId);
string_id!(DfsWriteSessionId);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum InodeKind {
    Regular,
    Directory,
    Symlink,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InodeAttributes {
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub atime_unix_ms: u64,
    pub mtime_unix_ms: u64,
    pub ctime_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InodeRecord {
    pub namespace_id: NamespaceId,
    pub inode_id: InodeId,
    pub kind: InodeKind,
    pub attributes: InodeAttributes,
    pub head_version: Option<FileVersionId>,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct DentryKey {
    pub namespace_id: NamespaceId,
    pub parent_inode_id: InodeId,
    pub name: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Dentry {
    pub key: DentryKey,
    pub inode_id: InodeId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileVersion {
    pub id: FileVersionId,
    pub inode_id: InodeId,
    pub parent_version: Option<FileVersionId>,
    pub length: u64,
    pub layout_root: LayoutRootId,
    pub created_at_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Extent {
    pub file_offset: u64,
    pub length: u64,
    pub chunk_id: ChunkId,
    pub chunk_offset: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LayoutRoot {
    pub id: LayoutRootId,
    pub file_length: u64,
    /// The R=1 vertical slice keeps extents inline. A later accepted extent-tree
    /// format can replace this field without changing FileVersion or Chunk IDs.
    pub inline_extents: Vec<Extent>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentDigest(pub [u8; 16]);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ChunkEncoding {
    Raw,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChunkObject {
    pub id: ChunkId,
    pub length: u64,
    pub content_digest: ContentDigest,
    pub encoding: ChunkEncoding,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CopyState {
    Staging,
    Durable,
    Corrupt,
    Deleting,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CopyRecord {
    pub id: CopyId,
    pub chunk_id: ChunkId,
    pub node_id: String,
    pub device_id: String,
    pub state: CopyState,
    pub persisted_bytes: u64,
    pub verified_digest: ContentDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlacementRecord {
    pub chunk_id: ChunkId,
    pub policy_id: String,
    pub replica_group_id: String,
    pub epoch: u64,
    pub copies: Vec<CopyId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChunkReceipt {
    pub operation_id: OperationId,
    pub chunk: ChunkObject,
    pub copy: CopyRecord,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DurabilityPolicy {
    pub id: String,
    pub required_copies: u16,
}

impl DurabilityPolicy {
    pub fn local_single_copy() -> Self {
        Self {
            id: "local-r1".into(),
            required_copies: 1,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommitFileVersion {
    pub operation_id: OperationId,
    pub inode_id: InodeId,
    pub expected_inode_revision: u64,
    pub expected_head_version: Option<FileVersionId>,
    pub file_version: FileVersion,
    pub layout_root: LayoutRoot,
    pub chunk_receipts: Vec<ChunkReceipt>,
}

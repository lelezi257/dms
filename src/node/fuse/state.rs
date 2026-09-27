//! FUSE session-local inode and handle tables.
//!
//! FUSE inode numbers and file handles are kernel-facing session numbers. They are
//! deliberately separate from `BackendInode`, `FileHandle`, and `DirectoryHandle`:
//! backends own file identity, while this table only remembers how the current
//! mounted daemon should route future callbacks.

use std::collections::HashMap;

use crate::node::vfs::{
    Namespace,
    types::{BackendInode, DirectoryHandle, Entry, FileHandle},
};

pub const ROOT_INO: u64 = 1;
pub const OWNERFS_INO: u64 = 2;
pub const BLOBFS_INO: u64 = 3;

const FIRST_BACKEND_INO: u64 = 4;
const FIRST_HANDLE: u64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FuseNode {
    Root,
    NamespaceRoot(Namespace),
    Backend(BackendInode),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FuseFileHandle {
    pub namespace: Namespace,
    pub handle: FileHandle,
    /// This handle was opened read-only on its local Home. Its callbacks can
    /// run on the FUSE receive thread without blocking on a remote RPC.
    pub inline_local_read: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FuseDirectoryHandle {
    pub namespace: Namespace,
    pub handle: DirectoryHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FuseHandle {
    File(FuseFileHandle),
    Directory(FuseDirectoryHandle),
}

#[derive(Debug)]
struct NodeRecord {
    node: FuseNode,
    lookup_count: u64,
}

#[derive(Debug)]
pub struct FuseState {
    next_ino: u64,
    next_handle: u64,
    nodes: HashMap<u64, NodeRecord>,
    backend_to_fuse: HashMap<BackendInode, u64>,
    handles: HashMap<u64, FuseHandle>,
}

impl FuseState {
    pub fn new(namespaces: impl IntoIterator<Item = Namespace>) -> Self {
        let mut nodes = HashMap::from([(
            ROOT_INO,
            NodeRecord {
                node: FuseNode::Root,
                lookup_count: u64::MAX,
            },
        )]);
        for namespace in namespaces {
            let ino = namespace_ino(namespace);
            nodes.insert(
                ino,
                NodeRecord {
                    node: FuseNode::NamespaceRoot(namespace),
                    lookup_count: u64::MAX,
                },
            );
        }
        Self {
            next_ino: FIRST_BACKEND_INO,
            next_handle: FIRST_HANDLE,
            nodes,
            backend_to_fuse: HashMap::new(),
            handles: HashMap::new(),
        }
    }

    pub fn node(&self, ino: u64) -> Option<FuseNode> {
        self.nodes.get(&ino).map(|record| record.node)
    }

    pub fn backend_inode(&self, ino: u64) -> Option<BackendInode> {
        match self.node(ino)? {
            FuseNode::NamespaceRoot(namespace) => Some(BackendInode {
                namespace,
                value: 1,
            }),
            FuseNode::Backend(inode) => Some(inode),
            FuseNode::Root => None,
        }
    }

    pub fn remember_lookup(&mut self, entry: &Entry) -> u64 {
        let ino = self.remember_backend_inode(entry.inode);
        if let Some(record) = self.nodes.get_mut(&ino) {
            record.lookup_count = record.lookup_count.saturating_add(1);
        }
        ino
    }

    pub fn remember_readdir_entry(&mut self, entry: &Entry) -> u64 {
        self.remember_backend_inode(entry.inode)
    }

    pub fn forget(&mut self, ino: u64, nlookup: u64) {
        let Some(record) = self.nodes.get_mut(&ino) else {
            return;
        };
        if matches!(record.node, FuseNode::Root | FuseNode::NamespaceRoot(_)) {
            return;
        }
        record.lookup_count = record.lookup_count.saturating_sub(nlookup);
        if record.lookup_count == 0 {
            if let FuseNode::Backend(inode) = record.node {
                self.backend_to_fuse.remove(&inode);
            }
            self.nodes.remove(&ino);
        }
    }

    pub fn insert_file_handle(&mut self, namespace: Namespace, handle: FileHandle) -> u64 {
        self.insert_file_handle_with_policy(namespace, handle, false)
    }

    pub fn insert_file_handle_with_policy(
        &mut self,
        namespace: Namespace,
        handle: FileHandle,
        inline_local_read: bool,
    ) -> u64 {
        self.insert_handle(FuseHandle::File(FuseFileHandle {
            namespace,
            handle,
            inline_local_read,
        }))
    }

    pub fn insert_directory_handle(
        &mut self,
        namespace: Namespace,
        handle: DirectoryHandle,
    ) -> u64 {
        self.insert_handle(FuseHandle::Directory(FuseDirectoryHandle {
            namespace,
            handle,
        }))
    }

    pub fn file_handle(&self, fh: u64) -> Option<FuseFileHandle> {
        match self.handles.get(&fh).copied()? {
            FuseHandle::File(handle) => Some(handle),
            FuseHandle::Directory(_) => None,
        }
    }

    pub fn directory_handle(&self, fh: u64) -> Option<FuseDirectoryHandle> {
        match self.handles.get(&fh).copied()? {
            FuseHandle::Directory(handle) => Some(handle),
            FuseHandle::File(_) => None,
        }
    }

    pub fn remove_file_handle(&mut self, fh: u64) -> Option<FuseFileHandle> {
        match self.handles.remove(&fh)? {
            FuseHandle::File(handle) => Some(handle),
            FuseHandle::Directory(handle) => {
                self.handles.insert(fh, FuseHandle::Directory(handle));
                None
            }
        }
    }

    pub fn remove_directory_handle(&mut self, fh: u64) -> Option<FuseDirectoryHandle> {
        match self.handles.remove(&fh)? {
            FuseHandle::Directory(handle) => Some(handle),
            FuseHandle::File(handle) => {
                self.handles.insert(fh, FuseHandle::File(handle));
                None
            }
        }
    }

    fn remember_backend_inode(&mut self, inode: BackendInode) -> u64 {
        if let Some(ino) = self.backend_to_fuse.get(&inode).copied() {
            return ino;
        }
        let ino = self.next_ino;
        self.next_ino = self.next_ino.saturating_add(1);
        self.backend_to_fuse.insert(inode, ino);
        self.nodes.insert(
            ino,
            NodeRecord {
                node: FuseNode::Backend(inode),
                lookup_count: 0,
            },
        );
        ino
    }

    fn insert_handle(&mut self, handle: FuseHandle) -> u64 {
        let fh = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        self.handles.insert(fh, handle);
        fh
    }
}

pub fn namespace_ino(namespace: Namespace) -> u64 {
    match namespace {
        Namespace::OwnerFs => OWNERFS_INO,
        Namespace::BlobFs => BLOBFS_INO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::vfs::types::{FileAttributes, FileKind};
    use std::time::UNIX_EPOCH;

    #[test]
    fn forget_drops_backend_inode_after_lookup_refs_are_released() {
        let mut state = FuseState::new([Namespace::OwnerFs]);
        let entry = Entry {
            inode: BackendInode {
                namespace: Namespace::OwnerFs,
                value: 42,
            },
            attributes: FileAttributes {
                kind: FileKind::Regular,
                size: 0,
                mode: 0o644,
                uid: 0,
                gid: 0,
                nlink: 1,
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
            },
        };

        let ino = state.remember_lookup(&entry);
        assert_eq!(state.backend_inode(ino), Some(entry.inode));

        state.forget(ino, 1);
        assert_eq!(state.backend_inode(ino), None);
    }

    #[test]
    fn forget_never_drops_virtual_roots() {
        let mut state = FuseState::new([Namespace::OwnerFs]);
        state.forget(ROOT_INO, u64::MAX);
        state.forget(OWNERFS_INO, u64::MAX);

        assert_eq!(state.node(ROOT_INO), Some(FuseNode::Root));
        assert_eq!(
            state.node(OWNERFS_INO),
            Some(FuseNode::NamespaceRoot(Namespace::OwnerFs))
        );
    }

    #[test]
    fn opened_handles_keep_namespace_after_inode_lookup_is_forgotten() {
        let mut state = FuseState::new([Namespace::OwnerFs]);
        let entry = Entry {
            inode: BackendInode {
                namespace: Namespace::OwnerFs,
                value: 99,
            },
            attributes: FileAttributes {
                kind: FileKind::Regular,
                size: 0,
                mode: 0o644,
                uid: 0,
                gid: 0,
                nlink: 1,
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
            },
        };

        let ino = state.remember_lookup(&entry);
        let fh = state.insert_file_handle(entry.inode.namespace, FileHandle(7));
        state.forget(ino, 1);

        assert_eq!(state.backend_inode(ino), None);
        assert_eq!(
            state.file_handle(fh),
            Some(FuseFileHandle {
                namespace: Namespace::OwnerFs,
                handle: FileHandle(7),
                inline_local_read: false,
            })
        );
        assert_eq!(
            state.remove_file_handle(fh),
            Some(FuseFileHandle {
                namespace: Namespace::OwnerFs,
                handle: FileHandle(7),
                inline_local_read: false,
            })
        );
    }

    #[test]
    fn opened_directory_handles_keep_namespace_after_inode_lookup_is_forgotten() {
        let mut state = FuseState::new([Namespace::OwnerFs]);
        let entry = Entry {
            inode: BackendInode {
                namespace: Namespace::OwnerFs,
                value: 100,
            },
            attributes: FileAttributes {
                kind: FileKind::Directory,
                size: 0,
                mode: 0o755,
                uid: 0,
                gid: 0,
                nlink: 2,
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
            },
        };

        let ino = state.remember_lookup(&entry);
        let fh = state.insert_directory_handle(entry.inode.namespace, DirectoryHandle(9));
        state.forget(ino, 1);

        assert_eq!(state.backend_inode(ino), None);
        assert_eq!(
            state.directory_handle(fh),
            Some(FuseDirectoryHandle {
                namespace: Namespace::OwnerFs,
                handle: DirectoryHandle(9),
            })
        );
        assert_eq!(
            state.remove_directory_handle(fh),
            Some(FuseDirectoryHandle {
                namespace: Namespace::OwnerFs,
                handle: DirectoryHandle(9),
            })
        );
    }
}

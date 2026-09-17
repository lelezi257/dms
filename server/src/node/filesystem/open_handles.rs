//! Node 本地的 POSIX open handle 生命周期。
//!
//! 表中只保存一次 `open()` 的本地状态，不保存可撤销的内容绑定或 CacheGrant。
//! rename/unlink 改变 namespace 后，已经打开的 handle 仍通过 inode 存活。

use std::collections::HashMap;

use crate::filesystem::{FileHandleId, InodeId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenHandle {
    pub(crate) id: FileHandleId,
    pub(crate) inode: InodeId,
    pub(crate) flags: i32,
    pub(crate) lock_owner: Option<u64>,
}

/// `DmsFuse` 所在 Node 拥有的 handle 表；它不是共享 Meta 状态。
#[derive(Debug)]
pub(crate) struct OpenHandleTable {
    next_handle: FileHandleId,
    handles: HashMap<FileHandleId, OpenHandle>,
}

impl Default for OpenHandleTable {
    fn default() -> Self {
        Self {
            next_handle: 1,
            handles: HashMap::new(),
        }
    }
}

impl OpenHandleTable {
    pub(crate) fn open(
        &mut self,
        inode: InodeId,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> OpenHandle {
        let id = self.next_handle;
        self.next_handle = self.next_handle.checked_add(1).unwrap_or(1);
        let handle = OpenHandle {
            id,
            inode,
            flags,
            lock_owner,
        };
        self.handles.insert(id, handle.clone());
        handle
    }

    pub(crate) fn get(&self, handle: FileHandleId) -> Option<&OpenHandle> {
        self.handles.get(&handle)
    }

    pub(crate) fn close(&mut self, handle: FileHandleId) -> Option<OpenHandle> {
        self.handles.remove(&handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_handle_lives_by_inode_not_by_path() {
        let mut handles = OpenHandleTable::default();
        let opened = handles.open(100, 2, Some(9));

        assert_eq!(handles.get(opened.id).map(|handle| handle.inode), Some(100));
        assert_eq!(handles.close(opened.id).expect("closed handle").inode, 100);
    }
}

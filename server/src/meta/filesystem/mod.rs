//! Filesystem 共享 namespace 在 Meta owner 内的状态落点。
//!
//! `FilesystemCatalog` 是 `MetaState` 持有的普通字段集合，不是第二个 actor、runtime
//! 或锁 owner。首条主链的 create 与文件版本提交都在同一个 Meta actor turn 内先写
//! journal、再 apply；状态已接入 WAL/checkpoint 与独立 Filesystem gRPC service。
//! 尚未实现的 rename/link/unlink 不在当前协议与接口中伪装成可用能力。

use std::collections::HashMap;

use crate::filesystem::{
    CacheGrant, DentrySnapshot, DirectoryEntry, DirectoryGrant, DirectoryPage, DirectoryVersion,
    GrantedInode, InodeAttributes, InodeId, InodeKind, InodeSnapshot, ROOT_INODE,
};

pub(crate) mod service;

type DentryKey = (InodeId, Vec<u8>);
type FilesystemCatalogSnapshot = (
    InodeId,
    Vec<InodeSnapshot>,
    Vec<DentrySnapshot>,
    Vec<(InodeId, u64)>,
);

#[derive(Clone)]
pub(crate) struct FilesystemCatalog {
    next_inode: InodeId,
    inodes: HashMap<InodeId, InodeSnapshot>,
    dentries: HashMap<DentryKey, DentrySnapshot>,
    grant_generations: HashMap<InodeId, u64>,
}

impl Default for FilesystemCatalog {
    fn default() -> Self {
        Self::restored(2, [], [], [])
    }
}

impl FilesystemCatalog {
    pub(crate) fn restored(
        next_inode: InodeId,
        inodes: impl IntoIterator<Item = InodeSnapshot>,
        dentries: impl IntoIterator<Item = DentrySnapshot>,
        grant_generations: impl IntoIterator<Item = (InodeId, u64)>,
    ) -> Self {
        let mut catalog = Self {
            next_inode,
            inodes: inodes
                .into_iter()
                .map(|inode| (inode.attributes.inode, inode))
                .collect(),
            dentries: dentries
                .into_iter()
                .map(|dentry| ((dentry.parent, dentry.name.clone()), dentry))
                .collect(),
            grant_generations: grant_generations.into_iter().collect(),
        };
        // 旧快照没有 Filesystem 字段；恢复时必须补回固定 root，而不是产生一个
        // 看似成功但所有 lookup/create 都从 NotFound 开始的空 namespace。
        catalog.inodes.entry(ROOT_INODE).or_insert_with(root_inode);
        catalog.grant_generations.entry(ROOT_INODE).or_insert(1);
        catalog.next_inode = catalog.next_inode.max(2);
        catalog
    }

    pub(crate) fn inode(&self, inode: InodeId) -> Option<&InodeSnapshot> {
        self.inodes.get(&inode)
    }

    pub(crate) fn lookup(&self, parent: InodeId, name: &[u8]) -> Option<&DentrySnapshot> {
        self.dentries.get(&(parent, name.to_vec()))
    }

    pub(crate) fn directory_is_empty(&self, directory: InodeId) -> bool {
        !self.dentries.keys().any(|(parent, _)| *parent == directory)
    }

    /// 判断 `candidate` 是否位于 `ancestor` 的目录子树中。
    ///
    /// rename 目录前由 Meta owner 调用这个权威检查，避免把 `/a` 移动到
    /// `/a/b` 后形成环。遍历只读取当前 catalog；步数上限同时防御已损坏快照中
    /// 可能存在的历史环，不能让一次请求无限循环。
    pub(crate) fn is_directory_descendant_of(&self, candidate: InodeId, ancestor: InodeId) -> bool {
        let mut current = candidate;
        for _ in 0..=self.inodes.len() {
            if current == ancestor {
                return true;
            }
            if current == ROOT_INODE {
                return false;
            }
            let Some(parent) = self.parent_of(current) else {
                return false;
            };
            current = parent;
        }
        // 超过 namespace 中 inode 的数量仍未到 root，只可能是 parent 链存在环。
        // 将其视为目标位于危险子树中，保守拒绝 mutation。
        true
    }

    pub(crate) fn read_directory(
        &self,
        directory: InodeId,
        cursor: &[u8],
        limit: usize,
        lease_millis: u64,
    ) -> Option<DirectoryPage> {
        let inode = self.inode(directory)?;
        if inode.attributes.kind != InodeKind::Directory {
            return None;
        }
        let parent = self.parent_of(directory).unwrap_or(directory);
        let grant = DirectoryGrant {
            directory_revision: inode.revision,
            grant: CacheGrant {
                generation: self.grant_generation(directory),
                lease_millis,
            },
        };
        let mut entries = self
            .dentries
            .values()
            .filter(|dentry| dentry.parent == directory && dentry.name.as_slice() > cursor)
            .filter_map(|dentry| {
                self.inode(dentry.inode).map(|inode| DirectoryEntry {
                    dentry: dentry.clone(),
                    attributes: inode.attributes.clone(),
                })
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.dentry.name.cmp(&right.dentry.name));
        let has_more = entries.len() > limit;
        if has_more {
            entries.truncate(limit);
        }
        let next_cursor = has_more
            .then(|| entries.last().map(|entry| entry.dentry.name.clone()))
            .flatten();
        Some(DirectoryPage {
            directory,
            parent,
            grant,
            entries,
            next_cursor,
        })
    }

    pub(crate) fn grant_generation(&self, inode: InodeId) -> u64 {
        self.grant_generations.get(&inode).copied().unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn next_inode(&self) -> InodeId {
        self.next_inode
    }

    pub(crate) fn next_inode_hint(&self) -> InodeId {
        self.next_inode
    }

    pub(crate) fn granted(&self, inode: InodeId, lease_millis: u64) -> Option<GrantedInode> {
        self.inode(inode).cloned().map(|inode| GrantedInode {
            grant: CacheGrant {
                generation: self.grant_generation(inode.attributes.inode),
                lease_millis,
            },
            inode,
        })
    }

    pub(crate) fn allocate_inode(&self) -> Option<(InodeId, InodeId)> {
        let inode = self.next_inode.max(2);
        inode.checked_add(1).map(|next| (inode, next))
    }

    pub(crate) fn apply_inode_created(
        &mut self,
        inode: InodeSnapshot,
        dentry: DentrySnapshot,
        next_inode: InodeId,
        grant_generation: u64,
    ) {
        self.next_inode = self.next_inode.max(next_inode);
        self.grant_generations
            .insert(inode.attributes.inode, grant_generation);
        self.dentries
            .insert((dentry.parent, dentry.name.clone()), dentry);
        self.inodes.insert(inode.attributes.inode, inode);
    }

    pub(crate) fn apply_inode_version(&mut self, inode: InodeSnapshot, grant_generation: u64) {
        self.grant_generations
            .insert(inode.attributes.inode, grant_generation);
        self.inodes.insert(inode.attributes.inode, inode);
    }

    pub(crate) fn apply_namespace_mutation(
        &mut self,
        upsert_inodes: impl IntoIterator<Item = InodeSnapshot>,
        upsert_dentries: impl IntoIterator<Item = DentrySnapshot>,
        remove_dentries: impl IntoIterator<Item = DentrySnapshot>,
        changed_directories: impl IntoIterator<Item = DirectoryVersion>,
        next_inode: InodeId,
    ) {
        self.next_inode = self.next_inode.max(next_inode);
        for dentry in remove_dentries {
            self.dentries.remove(&(dentry.parent, dentry.name));
        }
        for inode in upsert_inodes {
            self.grant_generations
                .entry(inode.attributes.inode)
                .or_insert(1);
            self.inodes.insert(inode.attributes.inode, inode);
        }
        for dentry in upsert_dentries {
            self.dentries
                .insert((dentry.parent, dentry.name.clone()), dentry);
        }
        for directory in changed_directories {
            self.grant_generations
                .insert(directory.inode, directory.grant_generation);
        }
    }

    pub(crate) fn snapshot(&self) -> FilesystemCatalogSnapshot {
        (
            self.next_inode,
            self.inodes.values().cloned().collect(),
            self.dentries.values().cloned().collect(),
            self.grant_generations
                .iter()
                .map(|(inode, generation)| (*inode, *generation))
                .collect(),
        )
    }
}

impl FilesystemCatalog {
    fn parent_of(&self, inode: InodeId) -> Option<InodeId> {
        if inode == ROOT_INODE {
            return Some(ROOT_INODE);
        }
        self.dentries
            .values()
            .find(|dentry| dentry.inode == inode)
            .map(|dentry| dentry.parent)
    }
}

fn root_inode() -> InodeSnapshot {
    InodeSnapshot {
        revision: 1,
        attributes: InodeAttributes {
            inode: ROOT_INODE,
            kind: InodeKind::Directory,
            mode: 0o755,
            uid: 0,
            gid: 0,
            link_count: 2,
            size: 0,
            atime_unix_nanos: 0,
            mtime_unix_nanos: 0,
            ctime_unix_nanos: 0,
        },
        content: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{FileContentBinding, InodeAttributes, InodeKind, ROOT_INODE};

    #[test]
    fn catalog_shape_keeps_namespace_and_exact_content_binding_together() {
        let inode = InodeSnapshot {
            revision: 9,
            attributes: InodeAttributes {
                inode: 100,
                kind: InodeKind::RegularFile,
                mode: 0o644,
                uid: 1,
                gid: 1,
                link_count: 1,
                size: 10,
                atime_unix_nanos: 0,
                mtime_unix_nanos: 1,
                ctime_unix_nanos: 1,
            },
            content: Some(FileContentBinding {
                object_key: b"fs/content/100".to_vec(),
                exact_version: 7,
            }),
        };
        let dentry = DentrySnapshot {
            parent: ROOT_INODE,
            name: b"a.txt".to_vec(),
            inode: 100,
            directory_revision: 3,
        };
        let catalog = FilesystemCatalog::restored(101, [inode], [dentry], [(100, 4)]);

        let found = catalog.lookup(ROOT_INODE, b"a.txt").expect("dentry");
        assert_eq!(found.inode, 100);
        assert_eq!(
            catalog
                .inode(found.inode)
                .and_then(|inode| inode.content.as_ref())
                .map(|binding| binding.exact_version),
            Some(7)
        );
        assert_eq!(catalog.grant_generation(100), 4);
        assert_eq!(catalog.next_inode(), 101);
    }
}

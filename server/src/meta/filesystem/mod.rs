//! Filesystem 共享 namespace 在 Meta owner 内的状态落点。
//!
//! `FilesystemCatalog` 是 `MetaState` 持有的普通字段集合，不是第二个 actor、runtime
//! 或锁 owner。首条主链的 create 与文件版本提交都在同一个 Meta actor turn 内先写
//! journal、再 apply；状态已接入 WAL/checkpoint 与独立 Filesystem gRPC service。
//! rename/unlink/mkdir/rmdir 已复用同一份目录索引；hard link 等后续能力不能在这里
//! 另建第二套 namespace 状态。

use std::{
    collections::{BTreeMap, HashMap},
    ops::Bound,
};

use crate::filesystem::{
    CacheGrant, DentrySnapshot, DirectoryEntry, DirectoryGrant, DirectoryPage, DirectoryVersion,
    GrantedInode, InodeAttributes, InodeId, InodeKind, InodeSnapshot, ROOT_INODE, XattrUpdate,
};

pub(crate) mod locks;
pub(crate) mod service;

type DentryKey = (InodeId, Vec<u8>);
type FilesystemCatalogSnapshot = (
    InodeId,
    Vec<InodeSnapshot>,
    Vec<DentrySnapshot>,
    Vec<(InodeId, u64)>,
    Vec<(InodeId, Vec<u8>, Vec<u8>)>,
);

#[derive(Clone)]
pub(crate) struct FilesystemCatalog {
    next_inode: InodeId,
    inodes: HashMap<InodeId, InodeSnapshot>,
    /// `(parent inode, raw name bytes)` 的有序索引。
    /// lookup 仍是 O(log n)；readdir 可直接从 cursor 开始顺序取一页，不再全表扫描和排序。
    dentries: BTreeMap<DentryKey, DentrySnapshot>,
    /// 仅目录 inode 的父目录索引。
    ///
    /// regular/symlink 允许多个 dentry，因此不能再维护“一 inode 一个 parent”的反向索引。
    /// `..` 与目录 rename 环检查只对目录有意义，仍可用这个索引 O(depth) 完成。
    directory_parents: HashMap<InodeId, InodeId>,
    grant_generations: HashMap<InodeId, u64>,
    /// xattr 与 inode namespace 共用一个 Meta owner；ACL 也只占用这里两个受约束名称。
    xattrs: BTreeMap<(InodeId, Vec<u8>), Vec<u8>>,
}

impl Default for FilesystemCatalog {
    fn default() -> Self {
        Self::restored(2, [], [], [], [])
    }
}

impl FilesystemCatalog {
    pub(crate) fn restored(
        next_inode: InodeId,
        inodes: impl IntoIterator<Item = InodeSnapshot>,
        dentries: impl IntoIterator<Item = DentrySnapshot>,
        grant_generations: impl IntoIterator<Item = (InodeId, u64)>,
        xattrs: impl IntoIterator<Item = (InodeId, Vec<u8>, Vec<u8>)>,
    ) -> Self {
        let inodes = inodes
            .into_iter()
            .map(|inode| (inode.attributes.inode, inode))
            .collect::<HashMap<_, _>>();
        let dentries = dentries
            .into_iter()
            .map(|dentry| ((dentry.parent, dentry.name.clone()), dentry))
            .collect::<BTreeMap<_, _>>();
        let directory_parents = dentries
            .values()
            .filter_map(|dentry| {
                let inode = inodes.get(&dentry.inode)?;
                (inode.attributes.kind == InodeKind::Directory)
                    .then_some((dentry.inode, dentry.parent))
            })
            .collect();
        let mut catalog = Self {
            next_inode,
            inodes,
            dentries,
            directory_parents,
            grant_generations: grant_generations.into_iter().collect(),
            xattrs: xattrs
                .into_iter()
                .map(|(inode, name, value)| ((inode, name), value))
                .collect(),
        };
        // 旧快照没有 Filesystem 字段；恢复时必须补回固定 root，而不是产生一个
        // 看似成功但所有 lookup/create 都从 NotFound 开始的空 namespace。
        catalog.inodes.entry(ROOT_INODE).or_insert_with(root_inode);
        catalog.grant_generations.entry(ROOT_INODE).or_insert(1);
        catalog.next_inode = catalog.next_inode.max(2);
        catalog
    }

    pub(crate) fn xattr(&self, inode: InodeId, name: &[u8]) -> Option<&[u8]> {
        self.xattrs.get(&(inode, name.to_vec())).map(Vec::as_slice)
    }

    pub(crate) fn list_xattrs(&self, inode: InodeId) -> Vec<Vec<u8>> {
        self.xattrs
            .range((Bound::Included((inode, Vec::new())), Bound::Unbounded))
            .take_while(|((entry_inode, _), _)| *entry_inode == inode)
            .map(|((_, name), _)| name.clone())
            .collect()
    }

    pub(crate) fn xattr_usage(&self, inode: InodeId) -> (usize, usize) {
        let entries = self
            .xattrs
            .range((Bound::Included((inode, Vec::new())), Bound::Unbounded))
            .take_while(|((entry_inode, _), _)| *entry_inode == inode);
        entries.fold((0, 0), |(count, bytes), ((_, name), value)| {
            (count + 1, bytes + name.len() + value.len())
        })
    }

    pub(crate) fn apply_xattr_updates(&mut self, updates: impl IntoIterator<Item = XattrUpdate>) {
        for update in updates {
            let key = (update.inode, update.name);
            if let Some(value) = update.value {
                self.xattrs.insert(key, value);
            } else {
                self.xattrs.remove(&key);
            }
        }
    }

    pub(crate) fn inode_count(&self) -> usize {
        self.inodes.len()
    }

    pub(crate) fn inode(&self, inode: InodeId) -> Option<&InodeSnapshot> {
        self.inodes.get(&inode)
    }

    pub(crate) fn lookup(&self, parent: InodeId, name: &[u8]) -> Option<&DentrySnapshot> {
        self.dentries.get(&(parent, name.to_vec()))
    }

    pub(crate) fn directory_is_empty(&self, directory: InodeId) -> bool {
        self.dentries
            .range((Bound::Included((directory, Vec::new())), Bound::Unbounded))
            .next()
            .is_none_or(|((parent, _), _)| *parent != directory)
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
        let start = (directory, cursor.to_vec());
        let mut candidates = self
            .dentries
            .range((Bound::Excluded(start), Bound::Unbounded))
            .take_while(|((parent, _), _)| *parent == directory)
            .map(|(_, dentry)| dentry)
            .filter_map(|dentry| {
                self.inode(dentry.inode).map(|inode| DirectoryEntry {
                    dentry: dentry.clone(),
                    attributes: inode.attributes.clone(),
                })
            });
        let entries = candidates.by_ref().take(limit).collect::<Vec<_>>();
        let has_more = candidates.next().is_some();
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

    /// 从已命中名字之后取一个有界的 dentry 窗口。
    ///
    /// 这不是新的 namespace 权威状态，只是 lookup 响应用的顺序视图。从当前名字
    /// 向后读，能匹配内核顺序遍历目录后逐个 lookup 的常见访问模式；严格
    /// `limit` 防止一次请求把大目录的全部布局都带回 Node。
    pub(crate) fn dentries_after(
        &self,
        directory: InodeId,
        name: &[u8],
        limit: usize,
    ) -> Vec<DentrySnapshot> {
        self.dentries
            .range((
                Bound::Excluded((directory, name.to_vec())),
                Bound::Unbounded,
            ))
            .take_while(|((parent, _), _)| *parent == directory)
            .take(limit)
            .map(|(_, dentry)| dentry.clone())
            .collect()
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
            .insert((dentry.parent, dentry.name.clone()), dentry.clone());
        if inode.attributes.kind == InodeKind::Directory {
            self.directory_parents.insert(dentry.inode, dentry.parent);
        }
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
        changed_inodes: impl IntoIterator<Item = crate::filesystem::InodeVersion>,
        next_inode: InodeId,
    ) {
        self.next_inode = self.next_inode.max(next_inode);
        for dentry in remove_dentries {
            self.dentries.remove(&(dentry.parent, dentry.name.clone()));
            if self
                .inodes
                .get(&dentry.inode)
                .is_some_and(|inode| inode.attributes.kind == InodeKind::Directory)
            {
                self.directory_parents.remove(&dentry.inode);
            }
        }
        for inode in upsert_inodes {
            self.grant_generations
                .entry(inode.attributes.inode)
                .or_insert(1);
            self.inodes.insert(inode.attributes.inode, inode);
        }
        for dentry in upsert_dentries {
            if self
                .inodes
                .get(&dentry.inode)
                .is_some_and(|inode| inode.attributes.kind == InodeKind::Directory)
            {
                self.directory_parents.insert(dentry.inode, dentry.parent);
            }
            self.dentries
                .insert((dentry.parent, dentry.name.clone()), dentry);
        }
        for directory in changed_directories {
            self.grant_generations
                .insert(directory.inode, directory.grant_generation);
        }
        for inode in changed_inodes {
            self.grant_generations
                .insert(inode.inode, inode.grant_generation);
        }
    }

    /// 返回当前 durable catalog 中等待生命周期回收的 inode。
    ///
    /// namespace mutation 只把 link_count 降为 0；真正删除必须由 Meta owner 在确认
    /// 所有 Node 引用租约都结束后决定。这里不做时间或租约判断，避免把生命周期策略
    /// 下沉到纯 catalog 容器。
    pub(crate) fn orphan_candidates(&self) -> Vec<InodeSnapshot> {
        let mut candidates = self
            .inodes
            .values()
            .filter(|inode| {
                inode.attributes.inode != ROOT_INODE && inode.attributes.link_count == 0
            })
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|inode| inode.attributes.inode);
        candidates
    }

    /// 应用已经 journal 化的 orphan 回收决定。
    pub(crate) fn apply_orphan_reaped(&mut self, inode: InodeId) {
        if inode == ROOT_INODE {
            return;
        }
        self.inodes.remove(&inode);
        self.grant_generations.remove(&inode);
        self.directory_parents.remove(&inode);
        self.xattrs
            .retain(|(entry_inode, _), _| *entry_inode != inode);
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
            self.xattrs
                .iter()
                .map(|((inode, name), value)| (*inode, name.clone(), value.clone()))
                .collect(),
        )
    }
}

impl FilesystemCatalog {
    fn parent_of(&self, inode: InodeId) -> Option<InodeId> {
        if inode == ROOT_INODE {
            return Some(ROOT_INODE);
        }
        self.directory_parents.get(&inode).copied()
    }
}

fn root_inode() -> InodeSnapshot {
    InodeSnapshot {
        revision: 1,
        attributes: InodeAttributes {
            inode: ROOT_INODE,
            kind: InodeKind::Directory,
            // 首版只有一棵共享 namespace，还没有 workspace root 的显式创建/授权
            // 接口。启用 FUSE `default_permissions` 后，若固定为 root:root 0755，普通
            // 挂载用户将无法在根目录创建任何内容。这里采用与 /tmp 相同的 01777：
            // 所有用户可创建，sticky bit 仍阻止普通用户删除他人的目录项。未来引入
            // workspace root 时，由创建请求持久化实际 owner/mode，不能继续硬编码。
            mode: 0o1777,
            uid: 0,
            gid: 0,
            link_count: 2,
            size: 0,
            atime_unix_nanos: 0,
            mtime_unix_nanos: 0,
            ctime_unix_nanos: 0,
        },
        content: None,
        reservations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{FileContentBinding, InodeAttributes, InodeKind, ROOT_INODE};

    #[test]
    fn default_root_allows_non_root_mount_users_without_disabling_kernel_permissions() {
        let catalog = FilesystemCatalog::default();
        let root = catalog.inode(ROOT_INODE).expect("default root inode");
        assert_eq!(root.attributes.uid, 0);
        assert_eq!(root.attributes.gid, 0);
        assert_eq!(root.attributes.mode, 0o1777);
    }

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
            reservations: Vec::new(),
        };
        let dentry = DentrySnapshot {
            parent: ROOT_INODE,
            name: b"a.txt".to_vec(),
            inode: 100,
            directory_revision: 3,
        };
        let catalog = FilesystemCatalog::restored(101, [inode], [dentry], [(100, 4)], []);

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

    #[test]
    fn readdir_uses_ordered_cursor_pages_without_rescanning_previous_names() {
        let inodes = (2..=6).map(|inode| InodeSnapshot {
            revision: 9,
            attributes: InodeAttributes {
                inode,
                kind: InodeKind::RegularFile,
                mode: 0o644,
                uid: 1,
                gid: 1,
                link_count: 1,
                size: 0,
                atime_unix_nanos: 0,
                mtime_unix_nanos: 0,
                ctime_unix_nanos: 0,
            },
            content: None,
            reservations: Vec::new(),
        });
        let dentries =
            [b"e", b"a", b"d", b"b", b"c"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| DentrySnapshot {
                    parent: ROOT_INODE,
                    name: name.to_vec(),
                    inode: index as u64 + 2,
                    directory_revision: 9,
                });
        let catalog = FilesystemCatalog::restored(7, inodes, dentries, [(ROOT_INODE, 3)], []);

        let first = catalog
            .read_directory(ROOT_INODE, b"", 2, 1_000)
            .expect("first page");
        assert_eq!(
            first
                .entries
                .iter()
                .map(|entry| entry.dentry.name.as_slice())
                .collect::<Vec<_>>(),
            vec![b"a".as_slice(), b"b".as_slice()]
        );
        assert_eq!(first.next_cursor, Some(b"b".to_vec()));

        let second = catalog
            .read_directory(ROOT_INODE, b"b", 2, 1_000)
            .expect("second page");
        assert_eq!(
            second
                .entries
                .iter()
                .map(|entry| entry.dentry.name.as_slice())
                .collect::<Vec<_>>(),
            vec![b"c".as_slice(), b"d".as_slice()]
        );
        assert_eq!(second.next_cursor, Some(b"d".to_vec()));
    }

    #[test]
    fn lookup_prefetch_window_is_ordered_bounded_and_stays_in_one_directory() {
        let inodes = (2..=8).map(|inode| InodeSnapshot {
            revision: 1,
            attributes: InodeAttributes {
                inode,
                kind: InodeKind::RegularFile,
                mode: 0o644,
                uid: 1,
                gid: 1,
                link_count: 1,
                size: 0,
                atime_unix_nanos: 0,
                mtime_unix_nanos: 0,
                ctime_unix_nanos: 0,
            },
            content: None,
            reservations: Vec::new(),
        });
        let dentries = [b"a", b"b", b"c", b"d", b"e", b"f"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| DentrySnapshot {
                parent: ROOT_INODE,
                name: name.to_vec(),
                inode: index as u64 + 2,
                directory_revision: 9,
            })
            .chain(std::iter::once(DentrySnapshot {
                parent: 99,
                name: b"z".to_vec(),
                inode: 8,
                directory_revision: 1,
            }));
        let catalog = FilesystemCatalog::restored(9, inodes, dentries, [], []);

        let window = catalog.dentries_after(ROOT_INODE, b"b", 3);
        assert_eq!(
            window
                .iter()
                .map(|entry| entry.name.as_slice())
                .collect::<Vec<_>>(),
            vec![b"c".as_slice(), b"d".as_slice(), b"e".as_slice()]
        );
        assert!(catalog.dentries_after(ROOT_INODE, b"f", 3).is_empty());
    }
}

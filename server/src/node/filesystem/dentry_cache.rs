//! Node 本地的 dentry/目录页授权缓存。
//!
//! 它不是权威 namespace；权威目录树只在 Meta。Node 只缓存 Meta 带 grant 返回的
//! 结果，并在 grant 过期、Watch 断流或目录级 revoke 到达时删除。这样热 lookup /
//! readdir 可以少走 Meta，冲突写仍由 Meta 的目录 revision 和主动失效兜住。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::filesystem::{DentrySnapshot, DirectoryGrant, DirectoryPage, InodeId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DentryLookup {
    Hit(DentrySnapshot),
    Negative,
    Unknown,
}

#[derive(Clone, Debug)]
struct DentryCacheEntry {
    dentry: Option<DentrySnapshot>,
    directory_revision: u64,
    grant_generation: u64,
    valid_until: Instant,
}

#[derive(Clone, Debug)]
struct DirectoryPageCacheEntry {
    page: DirectoryPage,
    valid_until: Instant,
}

#[derive(Default)]
pub(crate) struct DentryCache {
    entries: HashMap<(InodeId, Vec<u8>), DentryCacheEntry>,
    /// 按“目录 + 本页起始 cursor”缓存独立页，避免把大目录聚合成一份完整 Vec。
    directory_pages: HashMap<(InodeId, Vec<u8>), DirectoryPageCacheEntry>,
}

impl DentryCache {
    pub(crate) fn insert_positive(
        &mut self,
        dentry: DentrySnapshot,
        grant: DirectoryGrant,
        received_at: Instant,
    ) {
        let entry = DentryCacheEntry {
            dentry: Some(dentry.clone()),
            directory_revision: grant.directory_revision,
            grant_generation: grant.grant.generation,
            valid_until: grant_deadline(grant, received_at),
        };
        self.entries
            .insert((dentry.parent, dentry.name.clone()), entry);
    }

    pub(crate) fn insert_negative(
        &mut self,
        parent: InodeId,
        name: Vec<u8>,
        grant: DirectoryGrant,
        received_at: Instant,
    ) {
        let entry = DentryCacheEntry {
            dentry: None,
            directory_revision: grant.directory_revision,
            grant_generation: grant.grant.generation,
            valid_until: grant_deadline(grant, received_at),
        };
        self.entries.insert((parent, name), entry);
    }

    pub(crate) fn insert_directory_page(
        &mut self,
        cursor: Option<Vec<u8>>,
        page: DirectoryPage,
        received_at: Instant,
    ) {
        let valid_until = grant_deadline(page.grant, received_at);
        for entry in &page.entries {
            self.entries.insert(
                (page.directory, entry.dentry.name.clone()),
                DentryCacheEntry {
                    dentry: Some(entry.dentry.clone()),
                    directory_revision: page.grant.directory_revision,
                    grant_generation: page.grant.grant.generation,
                    valid_until,
                },
            );
        }
        self.directory_pages.insert(
            (page.directory, cursor.unwrap_or_default()),
            DirectoryPageCacheEntry { page, valid_until },
        );
    }

    pub(crate) fn lookup(&mut self, parent: InodeId, name: &[u8], now: Instant) -> DentryLookup {
        let key = (parent, name.to_vec());
        let expired = self
            .entries
            .get(&key)
            .is_some_and(|entry| now >= entry.valid_until);
        if expired {
            self.entries.remove(&key);
            return DentryLookup::Unknown;
        }
        match self
            .entries
            .get(&key)
            .and_then(|entry| entry.dentry.clone())
        {
            Some(dentry) => DentryLookup::Hit(dentry),
            None if self.entries.contains_key(&key) => DentryLookup::Negative,
            None => DentryLookup::Unknown,
        }
    }

    pub(crate) fn directory_page(
        &mut self,
        directory: InodeId,
        cursor: Option<&[u8]>,
        expected_revision: Option<u64>,
        now: Instant,
    ) -> Option<DirectoryPage> {
        let key = (directory, cursor.unwrap_or_default().to_vec());
        let expired = self
            .directory_pages
            .get(&key)
            .is_some_and(|entry| now >= entry.valid_until);
        if expired {
            self.directory_pages.remove(&key);
            return None;
        }
        self.directory_pages.get(&key).and_then(|entry| {
            expected_revision
                .is_none_or(|revision| revision == entry.page.grant.directory_revision)
                .then(|| entry.page.clone())
        })
    }

    /// 删除某个目录 grant 下派生出的 lookup/readdir 缓存。
    ///
    /// through_generation 防止乱序旧 revoke 删除更新后的授权；minimum_revision 是
    /// 诊断水位，当前 M1 不用它单独做准入。
    pub(crate) fn revoke_directory(
        &mut self,
        directory: InodeId,
        through_generation: u64,
        minimum_revision: u64,
    ) {
        self.directory_pages.retain(|(parent, _), entry| {
            *parent != directory
                || (entry.page.grant.grant.generation > through_generation
                    && entry.page.grant.directory_revision >= minimum_revision)
        });
        self.entries.retain(|(parent, _), entry| {
            *parent != directory
                || (entry.grant_generation > through_generation
                    && entry.directory_revision >= minimum_revision)
        });
    }

    /// 应用本 Node 已成功提交的精确目录变更。
    ///
    /// 本地写入方知道具体改了哪些名字，因此不需要像远端 invalidation 一样清空整个
    /// 目录缓存。未被点名的正/负 dentry 仍然成立，只把它们推进到新 revision；目录页
    /// 因为排序集合已经变化，仍须整体删除。deadline 不延长，Watch 断线后的安全上界
    /// 仍由原 grant 决定。
    pub(crate) fn apply_local_mutation(
        &mut self,
        directory: InodeId,
        directory_revision: u64,
        grant_generation: u64,
        removed_names: &[Vec<u8>],
    ) {
        self.directory_pages
            .retain(|(parent, _), _| *parent != directory);
        self.entries.retain(|(parent, name), entry| {
            if *parent != directory {
                return true;
            }
            if removed_names.iter().any(|removed| removed == name) {
                return false;
            }
            entry.directory_revision = directory_revision;
            entry.grant_generation = grant_generation;
            true
        });
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.directory_pages.clear();
    }
}

fn grant_deadline(grant: DirectoryGrant, received_at: Instant) -> Instant {
    received_at
        .checked_add(Duration::from_millis(grant.grant.lease_millis))
        .unwrap_or(received_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{
        CacheGrant, DentrySnapshot, DirectoryEntry, DirectoryGrant, DirectoryPage, InodeAttributes,
        InodeKind,
    };

    fn grant(revision: u64, generation: u64, lease_millis: u64) -> DirectoryGrant {
        DirectoryGrant {
            directory_revision: revision,
            grant: CacheGrant {
                generation,
                lease_millis,
            },
        }
    }

    fn dentry(parent: InodeId, name: &[u8], inode: InodeId, revision: u64) -> DentrySnapshot {
        DentrySnapshot {
            parent,
            name: name.to_vec(),
            inode,
            directory_revision: revision,
        }
    }

    fn attrs(inode: InodeId, kind: InodeKind) -> InodeAttributes {
        InodeAttributes {
            inode,
            kind,
            mode: 0o644,
            uid: 0,
            gid: 0,
            link_count: 1,
            size: 0,
            atime_unix_nanos: 0,
            mtime_unix_nanos: 0,
            ctime_unix_nanos: 0,
        }
    }

    #[test]
    fn positive_and_negative_lookup_follow_directory_grant() {
        let now = Instant::now();
        let mut cache = DentryCache::default();
        assert_eq!(cache.lookup(1, b"shared.txt", now), DentryLookup::Unknown);

        cache.insert_negative(1, b"missing.txt".to_vec(), grant(3, 7, 1_000), now);
        assert_eq!(cache.lookup(1, b"missing.txt", now), DentryLookup::Negative);

        cache.insert_positive(dentry(1, b"shared.txt", 9, 3), grant(3, 7, 1_000), now);
        assert_eq!(
            cache.lookup(1, b"shared.txt", now),
            DentryLookup::Hit(dentry(1, b"shared.txt", 9, 3))
        );
        assert_eq!(
            cache.lookup(1, b"shared.txt", now + Duration::from_millis(1_000)),
            DentryLookup::Unknown
        );
    }

    #[test]
    fn directory_pages_are_cached_independently_and_revoked_as_one_unit() {
        let now = Instant::now();
        let mut cache = DentryCache::default();
        cache.insert_directory_page(
            None,
            DirectoryPage {
                directory: 1,
                parent: 1,
                grant: grant(11, 5, 1_000),
                entries: vec![DirectoryEntry {
                    dentry: dentry(1, b"a", 2, 11),
                    attributes: attrs(2, InodeKind::RegularFile),
                }],
                next_cursor: None,
            },
            now,
        );

        assert_eq!(
            cache
                .directory_page(1, None, Some(11), now)
                .map(|page| page.entries.len()),
            Some(1)
        );
        assert!(matches!(
            cache.lookup(1, b"a", now),
            DentryLookup::Hit(found) if found.inode == 2
        ));

        cache.revoke_directory(1, 5, 11);
        assert!(cache.directory_page(1, None, Some(11), now).is_none());
        assert_eq!(cache.lookup(1, b"a", now), DentryLookup::Unknown);
    }

    #[test]
    fn directory_page_cache_binds_cursor_and_revision() {
        let now = Instant::now();
        let mut cache = DentryCache::default();
        cache.insert_directory_page(
            Some(b"a".to_vec()),
            DirectoryPage {
                directory: 1,
                parent: 1,
                grant: grant(12, 6, 1_000),
                entries: vec![DirectoryEntry {
                    dentry: dentry(1, b"b", 3, 12),
                    attributes: attrs(3, InodeKind::RegularFile),
                }],
                next_cursor: Some(b"b".to_vec()),
            },
            now,
        );

        assert!(cache.directory_page(1, Some(b"a"), Some(12), now).is_some());
        assert!(cache.directory_page(1, None, Some(12), now).is_none());
        assert!(cache.directory_page(1, Some(b"a"), Some(11), now).is_none());
    }

    #[test]
    fn revoke_from_same_commit_keeps_response_grant() {
        let started = Instant::now();
        let mut cache = DentryCache::default();
        cache.insert_positive(dentry(1, b"fresh", 9, 12), grant(12, 6, 1_000), started);

        // 同一次 namespace commit 的事件只撤销旧 generation；响应携带的新 grant
        // 与 minimum revision 相等，已经代表该 commit，不能被迟到的本事件误删。
        cache.revoke_directory(1, 5, 12);

        assert_eq!(
            cache.lookup(1, b"fresh", started + Duration::from_millis(1)),
            DentryLookup::Hit(dentry(1, b"fresh", 9, 12))
        );
    }

    #[test]
    fn local_mutation_preserves_unaffected_names_without_extending_lease() {
        let started = Instant::now();
        let mut cache = DentryCache::default();
        cache.insert_positive(dentry(1, b"old", 8, 11), grant(11, 5, 100), started);
        cache.insert_negative(1, b"new".to_vec(), grant(11, 5, 100), started);

        cache.apply_local_mutation(1, 12, 6, &[b"new".to_vec()]);

        assert_eq!(
            cache.lookup(1, b"old", started + Duration::from_millis(99)),
            DentryLookup::Hit(dentry(1, b"old", 8, 11))
        );
        assert_eq!(
            cache.lookup(1, b"new", started + Duration::from_millis(1)),
            DentryLookup::Unknown
        );
        assert_eq!(
            cache.lookup(1, b"old", started + Duration::from_millis(100)),
            DentryLookup::Unknown
        );
    }
}

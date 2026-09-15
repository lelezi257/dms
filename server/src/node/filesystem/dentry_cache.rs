//! Node 本地的 dentry/目录页授权缓存。
//!
//! 它不是权威 namespace；权威目录树只在 Meta。Node 只缓存 Meta 带 grant 返回的
//! 结果，并在 grant 过期、Watch 断流或目录级 revoke 到达时删除。这样热 lookup /
//! readdir 可以少走 Meta，冲突写仍由 Meta 的目录 revision 和主动失效兜住。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::filesystem::{DentrySnapshot, DirectoryEntry, DirectoryGrant, DirectoryPage, InodeId};

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
struct DirectoryCacheEntry {
    parent: InodeId,
    entries: Vec<DirectoryEntry>,
    revision: u64,
    grant_generation: u64,
    valid_until: Instant,
}

#[derive(Default)]
pub(crate) struct DentryCache {
    entries: HashMap<(InodeId, Vec<u8>), DentryCacheEntry>,
    directories: HashMap<InodeId, DirectoryCacheEntry>,
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

    pub(crate) fn insert_directory(&mut self, page: DirectoryPage, received_at: Instant) {
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
        self.directories.insert(
            page.directory,
            DirectoryCacheEntry {
                parent: page.parent,
                entries: page.entries,
                revision: page.grant.directory_revision,
                grant_generation: page.grant.grant.generation,
                valid_until,
            },
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

    pub(crate) fn directory(
        &mut self,
        directory: InodeId,
        now: Instant,
    ) -> Option<(InodeId, Vec<DirectoryEntry>)> {
        let expired = self
            .directories
            .get(&directory)
            .is_some_and(|entry| now >= entry.valid_until);
        if expired {
            self.remove_directory(directory);
            return None;
        }
        self.directories
            .get(&directory)
            .map(|entry| (entry.parent, entry.entries.clone()))
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
        let remove_directory = self.directories.get(&directory).is_some_and(|entry| {
            entry.grant_generation <= through_generation || entry.revision <= minimum_revision
        });
        if remove_directory {
            self.directories.remove(&directory);
        }
        self.entries.retain(|(parent, _), entry| {
            *parent != directory
                || (entry.grant_generation > through_generation
                    && entry.directory_revision > minimum_revision)
        });
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.directories.clear();
    }

    fn remove_directory(&mut self, directory: InodeId) {
        self.directories.remove(&directory);
        self.entries.retain(|(parent, _), _| *parent != directory);
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
    fn complete_directory_cache_can_be_revoked_as_one_unit() {
        let now = Instant::now();
        let mut cache = DentryCache::default();
        cache.insert_directory(
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
            cache.directory(1, now).map(|(_, entries)| entries.len()),
            Some(1)
        );
        assert!(matches!(
            cache.lookup(1, b"a", now),
            DentryLookup::Hit(found) if found.inode == 2
        ));

        cache.revoke_directory(1, 5, 11);
        assert!(cache.directory(1, now).is_none());
        assert_eq!(cache.lookup(1, b"a", now), DentryLookup::Unknown);
    }
}

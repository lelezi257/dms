//! Node 本地的 inode 内容绑定缓存。
//!
//! CacheGrant 仍有效时，热读可直接得到 `inode -> ObjectKey + ExactVersion`，无需每次
//! 访问 Meta。远端提交产生 revoke 后必须先删除条目，再通过既有 Watch ACK 通道确认；
//! Watch 断线时 deadline 到期也会使条目失效。

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crate::filesystem::{InodeId, ResolvedInode};

#[derive(Clone, Debug)]
pub(crate) struct BindingCacheEntry {
    pub(crate) resolved: ResolvedInode,
    valid_until: Instant,
}

#[derive(Default)]
pub(crate) struct BindingCache {
    entries: HashMap<InodeId, BindingCacheEntry>,
}

impl BindingCache {
    /// deadline 从收到 resolve 响应的时刻计算，不能从数据下载结束重新延长。
    pub(crate) fn insert(&mut self, resolved: ResolvedInode, received_at: Instant) {
        let lease = Duration::from_millis(resolved.granted.grant.lease_millis);
        let valid_until = received_at.checked_add(lease).unwrap_or(received_at);
        self.entries.insert(
            resolved.granted.inode.attributes.inode,
            BindingCacheEntry {
                resolved,
                valid_until,
            },
        );
    }

    /// 返回一份带“剩余租约”的快照。FUSE 可以直接把这个剩余时间
    /// 作为 kernel entry/attr TTL，不会因 Node 命中旧缓存而重新获得完整租约。
    pub(crate) fn get_authorized(&mut self, inode: InodeId, now: Instant) -> Option<ResolvedInode> {
        let expired = self
            .entries
            .get(&inode)
            .is_some_and(|entry| now >= entry.valid_until);
        if expired {
            self.entries.remove(&inode);
            return None;
        }
        self.entries.get(&inode).map(|entry| {
            let mut resolved = entry.resolved.clone();
            let remaining_millis = entry
                .valid_until
                .saturating_duration_since(now)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64;
            resolved.granted.grant.lease_millis = remaining_millis;
            resolved
        })
    }

    /// 撤销不高于 `through_generation` 的授权。返回值表示是否真的删除了条目，调用方
    /// 只有在这一步完成后才能 ACK 对应 Meta event。
    pub(crate) fn revoke(&mut self, inode: InodeId, through_generation: u64) -> bool {
        let should_remove = self
            .entries
            .get(&inode)
            .is_some_and(|entry| entry.resolved.granted.grant.generation <= through_generation);
        if should_remove {
            self.entries.remove(&inode);
        }
        should_remove
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{
        CacheGrant, FileContentBinding, GrantedInode, InodeAttributes, InodeKind, InodeSnapshot,
    };

    fn resolved(generation: u64, lease_millis: u64) -> ResolvedInode {
        ResolvedInode {
            granted: GrantedInode {
                inode: InodeSnapshot {
                    revision: 7,
                    attributes: InodeAttributes {
                        inode: 100,
                        kind: InodeKind::RegularFile,
                        mode: 0o644,
                        uid: 1,
                        gid: 1,
                        link_count: 1,
                        size: 10,
                        atime_unix_nanos: 0,
                        mtime_unix_nanos: 0,
                        ctime_unix_nanos: 0,
                    },
                    content: Some(FileContentBinding {
                        object_key: b"fs/content/100".to_vec(),
                        exact_version: 3,
                    }),
                    reservations: Vec::new(),
                },
                grant: CacheGrant {
                    generation,
                    lease_millis,
                },
            },
            object: None,
            access_acl: None,
        }
    }

    #[test]
    fn authorized_hit_avoids_meta_until_revoke_or_expiry() {
        let started = Instant::now();
        let mut cache = BindingCache::default();
        cache.insert(resolved(5, 1_000), started);

        let almost_expired = cache
            .get_authorized(100, started + Duration::from_millis(999))
            .expect("grant remains valid");
        assert_eq!(
            almost_expired.granted.grant.lease_millis, 1,
            "kernel TTL must use remaining lease rather than renewing the original second"
        );
        assert!(!cache.revoke(100, 4), "旧 revoke 不能删除更新后的授权");
        assert!(cache.revoke(100, 5));
        assert!(cache.get_authorized(100, started).is_none());

        cache.insert(resolved(6, 10), started);
        assert!(
            cache
                .get_authorized(100, started + Duration::from_millis(10))
                .is_none()
        );
    }
}

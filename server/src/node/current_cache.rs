//! Node 私有的 Current 布局缓存；不保存 value，也不把旧 replica 地址当成存活证明。
//!
//! 所有方法只由 NodeState 调用。Meta 的剩余租约是上限，截止时间从请求开始算；
//! 心跳不能延长已有条目。generation 防止失效 ACK 后迟到的 resolve 重新填回旧版。
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use dms_protocol::v1 as pb;

struct CachedLayout {
    layout: Arc<pb::VersionLayout>,
    node_epoch: u64,
    expires_at: Instant,
    charge: u64,
}

pub(super) struct CurrentCache {
    entries: HashMap<Vec<u8>, CachedLayout>,
    generation: u64,
    charged: u64,
    budget: u64,
    ttl: Duration,
}

impl CurrentCache {
    pub(super) fn new(budget: u64, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            charged: 0,
            budget,
            ttl,
        }
    }

    pub(super) fn token(&self) -> Option<u64> {
        (self.budget > 0 && self.generation < u64::MAX).then_some(self.generation)
    }

    pub(super) fn charged(&self) -> u64 {
        self.charged
    }

    /// 全局代数只限制在途回填；单 key 失效不清理其它已缓存 key。
    pub(super) fn invalidate(&mut self, key: &[u8]) {
        self.generation = self.generation.saturating_add(1);
        self.remove(key);
    }

    pub(super) fn clear(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.entries.clear();
        self.charged = 0;
    }

    fn remove(&mut self, key: &[u8]) {
        if let Some(entry) = self.entries.remove(key) {
            self.charged -= entry.charge;
        }
    }

    pub(super) fn get(
        &mut self,
        key: &[u8],
        node_epoch: u64,
        now: Instant,
    ) -> Option<Arc<pb::VersionLayout>> {
        if self.generation == u64::MAX {
            return None;
        }
        let entry = self.entries.get(key)?;
        if entry.node_epoch != node_epoch || entry.expires_at <= now {
            self.remove(key);
            return None;
        }
        Some(Arc::clone(&entry.layout))
    }

    /// 无 grant（例如旧 Meta）只是不启用缓存，正常读取结果仍然可用。
    /// 乱序响应不得降低版本；过期、错误 incarnation 或越过失效围栏的响应不回填。
    pub(super) fn insert(
        &mut self,
        token: u64,
        key: Vec<u8>,
        resolved: &pb::ResolveObjectResponse,
        requested_at: Instant,
        node_epoch: u64,
        now: Instant,
    ) -> bool {
        if self.token() != Some(token) {
            return false;
        }
        let (Some(layout), Some(grant)) = (&resolved.layout, &resolved.current_lease) else {
            return false;
        };
        if grant.version != layout.version
            || grant.lease_epoch != node_epoch
            || grant.ttl_millis == 0
            || layout.kind == pb::VersionKind::Tombstone as i32
        {
            return false;
        }
        let Some(expires_at) = requested_at
            .checked_add(Duration::from_millis(grant.ttl_millis).min(self.ttl))
            .filter(|deadline| *deadline > now)
        else {
            return false;
        };
        if self
            .entries
            .get(&key)
            .is_some_and(|entry| entry.layout.version > layout.version)
        {
            return false;
        }
        // 按复制后数据长度 + 固定结构开销收费，同时限制小条目数量；不是进程 RSS 承诺。
        let charge = key.len() as u64
            + 256
            + layout.digest.len() as u64
            + layout
                .extents
                .iter()
                .map(|extent| 128 + extent.block_id.len() as u64 + extent.digest.len() as u64)
                .sum::<u64>();
        if charge > self.budget {
            return false;
        }
        self.remove(&key);
        if charge > self.budget - self.charged {
            // 与 SDK 缓存相同的简单整批淘汰；命中不维护额外 LRU 链，容量始终有界。
            self.entries.clear();
            self.charged = 0;
        }
        self.charged += charge;
        self.entries.insert(
            key,
            CachedLayout {
                layout: Arc::new(layout.clone()),
                node_epoch,
                expires_at,
                charge,
            },
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(version: u64) -> pb::ResolveObjectResponse {
        pb::ResolveObjectResponse {
            layout: Some(pb::VersionLayout {
                version,
                logical_length: 1,
                kind: pb::VersionKind::Value as i32,
                ..Default::default()
            }),
            current_lease: Some(pb::CurrentLeaseGrant {
                version,
                lease_epoch: 7,
                ttl_millis: 500,
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    #[test]
    fn invalidation_and_disconnect_reject_late_refill() {
        let mut cache = CurrentCache::new(4096, Duration::from_secs(1));
        let now = Instant::now();
        let old = cache.token().unwrap();
        cache.invalidate(b"k");
        assert!(!cache.insert(old, b"k".to_vec(), &response(1), now, 7, now));
        let token = cache.token().unwrap();
        assert!(cache.insert(token, b"k".to_vec(), &response(2), now, 7, now));
        cache.clear();
        assert!(cache.get(b"k", 7, now).is_none());
        assert!(!cache.insert(token, b"k".to_vec(), &response(2), now, 7, now));
    }
    #[test]
    fn deadline_is_from_request_start_and_epoch_is_checked() {
        let mut cache = CurrentCache::new(4096, Duration::from_secs(1));
        let start = Instant::now();
        let token = cache.token().unwrap();
        assert!(!cache.insert(
            token,
            b"k".to_vec(),
            &response(1),
            start,
            7,
            start + Duration::from_secs(1)
        ));
        assert!(!cache.insert(token, b"k".to_vec(), &response(1), start, 8, start));
        assert!(cache.insert(token, b"k".to_vec(), &response(1), start, 7, start));
        assert!(
            cache
                .get(b"k", 7, start + Duration::from_millis(499))
                .is_some()
        );
        assert!(
            cache
                .get(b"k", 7, start + Duration::from_millis(500))
                .is_none()
        );
        assert!(cache.insert(token, b"k".to_vec(), &response(1), start, 7, start));
        assert!(cache.get(b"k", 8, start).is_none());
    }
    #[test]
    fn bounded_budget_no_grant_disabled_and_older_versions() {
        let now = Instant::now();
        let mut cache = CurrentCache::new(520, Duration::from_secs(1));
        let token = cache.token().unwrap();
        assert!(cache.insert(token, b"a".to_vec(), &response(2), now, 7, now));
        assert!(!cache.insert(token, b"a".to_vec(), &response(1), now, 7, now));
        assert!(cache.insert(token, b"b".to_vec(), &response(1), now, 7, now));
        assert!(cache.insert(token, b"c".to_vec(), &response(1), now, 7, now));
        assert!(cache.charged() <= 520);
        assert_eq!(cache.entries.len(), 1);
        let mut no_grant = response(1);
        no_grant.current_lease = None;
        assert!(!cache.insert(token, b"k".to_vec(), &no_grant, now, 7, now));
        assert!(
            CurrentCache::new(0, Duration::from_secs(1))
                .token()
                .is_none()
        );
        cache.generation = u64::MAX - 1;
        cache.invalidate(b"c");
        assert!(cache.token().is_none());
        assert!(cache.get(b"c", 7, now).is_none());
    }
}

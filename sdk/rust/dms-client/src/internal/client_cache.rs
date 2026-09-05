//! Current 缓存只在 Session 有效时服务；回填与失效在同一把锁下线性化。
use crate::{Key, ObjectVersion};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CachedValue {
    pub(crate) version: ObjectVersion,
    // 锁内只 clone Arc；公开 API 所需 Vec 在释放锁后复制。
    pub(crate) bytes: Arc<[u8]>,
}

/// 请求开始时取得的回填资格，不是返回结果的版本或租约。
#[derive(Clone, Copy)]
pub(crate) struct RefillToken(u64);

pub(crate) struct ClientCache {
    state: Mutex<CacheState>,
    budget_bytes: usize,
}

#[derive(Default)]
struct CacheState {
    current: HashMap<Vec<u8>, CachedValue>,
    charged_bytes: usize,
    generation: u64,
    connected: bool,
    lease_deadline: Option<Instant>,
}

impl CacheState {
    fn expire_if_needed(&mut self, now: Instant) -> bool {
        if !self.connected {
            return false;
        }
        if self.lease_deadline.is_some_and(|deadline| now < deadline) {
            return true;
        }
        if self.lease_deadline.take().is_some() {
            self.fence();
            self.current.clear();
            self.charged_bytes = 0;
        }
        false
    }
    fn fence(&mut self) {
        // 全局代数保守拒绝无关 key 的在途回填，避免无限增长的 per-key tombstone。
        // 代数耗尽时永久停用缓存，不能回绕复活旧 token。
        match self.generation.checked_add(1) {
            Some(next) => {
                self.generation = next;
                if next == u64::MAX {
                    self.connected = false;
                }
            }
            None => self.connected = false,
        }
    }

    fn remove(&mut self, key: &[u8]) {
        if let Some(value) = self.current.remove(key) {
            self.charged_bytes -= entry_charge(key, &value);
        }
    }
}

fn entry_charge(key: &[u8], value: &CachedValue) -> usize {
    // key/value 加固定条目开销同时约束小值数量；不声称等于 allocator RSS。
    key.len()
        .saturating_add(value.bytes.len())
        .saturating_add(128)
}

impl ClientCache {
    pub(crate) fn new(budget_bytes: usize) -> Self {
        Self {
            state: Mutex::new(CacheState::default()),
            budget_bytes,
        }
    }

    pub(crate) fn session_connected(&self) {
        let mut state = self.state.lock().expect("client cache poisoned");
        state.fence();
        state.current.clear();
        state.charged_bytes = 0;
        state.connected = state.generation < u64::MAX;
        state.lease_deadline = None;
    }

    pub(crate) fn session_disconnected(&self) {
        let mut state = self.state.lock().expect("client cache poisoned");
        state.fence();
        state.connected = false;
        state.lease_deadline = None;
        state.current.clear();
        state.charged_bytes = 0;
    }

    pub(crate) fn refill_token(&self) -> Option<RefillToken> {
        let mut state = self.state.lock().expect("client cache poisoned");
        (state.expire_if_needed(Instant::now()) && self.budget_bytes > 0)
            .then_some(RefillToken(state.generation))
    }

    /// 起点必须在 unary Heartbeat 发出前捕获，网络延迟不得延长本地租约。
    pub(crate) fn renew_lease(&self, requested_at: Instant, ttl: Duration) {
        self.renew_lease_at(requested_at, ttl, Instant::now());
    }

    fn renew_lease_at(&self, requested_at: Instant, ttl: Duration, now: Instant) {
        let mut state = self.state.lock().expect("client cache poisoned");
        state.expire_if_needed(now);
        if state.connected {
            state.lease_deadline = requested_at
                .checked_add(ttl)
                .filter(|deadline| *deadline > now);
        }
        // 迟到的零 TTL/过期续租也要清掉当前条目，不能让下次续租复活旧缓存。
        if state.lease_deadline.is_none() {
            state.fence();
            state.current.clear();
            state.charged_bytes = 0;
        }
    }

    pub(crate) fn revoke_lease(&self) {
        let mut state = self.state.lock().expect("client cache poisoned");
        state.fence();
        state.lease_deadline = None;
        state.current.clear();
        state.charged_bytes = 0;
    }

    pub(crate) fn accepts_value(&self, key: &Key, bytes: usize) -> bool {
        key.as_bytes()
            .len()
            .saturating_add(bytes)
            .saturating_add(128)
            <= self.budget_bytes
    }

    pub(crate) fn get_current(&self, key: &Key) -> Option<CachedValue> {
        let mut state = self.state.lock().expect("client cache poisoned");
        state
            .expire_if_needed(Instant::now())
            .then(|| state.current.get(key.as_bytes()).cloned())
            .flatten()
    }

    /// 返回容量压力淘汰的条目数量；失效和断线不计为容量淘汰。
    pub(crate) fn insert_current(
        &self,
        token: Option<RefillToken>,
        key: &Key,
        value: CachedValue,
    ) -> usize {
        let Some(token) = token else {
            return 0;
        };
        let charge = entry_charge(key.as_bytes(), &value);
        if charge > self.budget_bytes {
            return 0;
        }
        let mut state = self.state.lock().expect("client cache poisoned");
        if !state.expire_if_needed(Instant::now()) || state.generation != token.0 {
            return 0;
        }
        // 同一代数内响应仍可能乱序；不允许旧版本覆盖已经回填的新版本。
        if state
            .current
            .get(key.as_bytes())
            .is_some_and(|cached| cached.version > value.version)
        {
            return 0;
        }
        state.remove(key.as_bytes());
        let evicted = if charge > self.budget_bytes - state.charged_bytes {
            // 简单整批淘汰，不建立第二份 LRU 链或每次命中写锁。
            let count = state.current.len();
            state.current.clear();
            state.charged_bytes = 0;
            count
        } else {
            0
        };
        state.charged_bytes += charge;
        state.current.insert(key.as_bytes().to_vec(), value);
        evicted
    }

    pub(crate) fn remove_current(&self, key: &Key) {
        let mut state = self.state.lock().expect("client cache poisoned");
        state.fence();
        state.remove(key.as_bytes());
    }

    /// 先撤销在途回填资格，再允许 Session owner 发送 ACK，即使当时没有条目。
    pub(crate) fn invalidate_current(&self, key: &[u8], minimum_version: ObjectVersion) -> bool {
        let mut state = self.state.lock().expect("client cache poisoned");
        state.fence();
        let evicted = state
            .current
            .get(key)
            .is_some_and(|entry| entry.version < minimum_version);
        if evicted {
            state.remove(key);
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_cache(budget: usize) -> ClientCache {
        let cache = ClientCache::new(budget);
        cache.session_connected();
        cache.renew_lease(Instant::now(), Duration::from_secs(60));
        cache
    }

    fn value(version: u64, bytes: &[u8]) -> CachedValue {
        CachedValue {
            version: ObjectVersion(version),
            bytes: bytes.into(),
        }
    }

    #[test]
    fn delayed_refill_cannot_undo_an_acknowledged_invalidation() {
        // GET、SET、MGET 共用同一 token 合同；闸门模拟 RPC 已得到旧值但尚未回填。
        for operation in ["get", "set", "mget"] {
            let cache = Arc::new(active_cache(1024));
            let key = Key::new(operation.as_bytes().to_vec()).unwrap();
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (resume_tx, resume_rx) = std::sync::mpsc::channel();
            let reader_cache = cache.clone();
            let reader_key = key.clone();
            let reader = std::thread::spawn(move || {
                let token = reader_cache.refill_token();
                started_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
                reader_cache.insert_current(token, &reader_key, value(1, b"old"));
            });
            started_rx.recv().unwrap();
            cache.invalidate_current(key.as_bytes(), ObjectVersion(2));
            cache.insert_current(cache.refill_token(), &key, value(2, b"new"));
            resume_tx.send(()).unwrap();
            reader.join().unwrap();
            assert_eq!(
                cache.get_current(&key),
                Some(value(2, b"new")),
                "{operation}"
            );
        }
    }

    #[test]
    fn disconnected_session_cannot_refill_current() {
        let cache = active_cache(1024);
        let key = Key::new(b"late/disconnect".to_vec()).unwrap();
        let token = cache.refill_token();
        cache.session_disconnected();
        cache.insert_current(token, &key, value(1, b"old"));
        assert!(cache.get_current(&key).is_none());
        assert!(cache.refill_token().is_none());
        cache.session_connected();
        cache.insert_current(token, &key, value(1, b"old"));
        assert!(
            cache.get_current(&key).is_none(),
            "reconnect must not revive old requests"
        );
    }

    #[test]
    fn invalidation_reports_evicted_only_for_an_older_cached_version() {
        let cache = active_cache(1024);
        let key = Key::new(b"checkpoint/latest".to_vec()).unwrap();
        cache.insert_current(cache.refill_token(), &key, value(7, b"v7"));
        assert!(!cache.invalidate_current(key.as_bytes(), ObjectVersion(7)));
        assert!(cache.invalidate_current(key.as_bytes(), ObjectVersion(8)));
        assert!(!cache.invalidate_current(key.as_bytes(), ObjectVersion(9)));
    }

    #[test]
    fn capacity_is_bounded_and_hot_handles_share_bytes() {
        let cache = active_cache(270);
        let a = Key::new(b"a".to_vec()).unwrap();
        let b = Key::new(b"b".to_vec()).unwrap();
        let c = Key::new(b"c".to_vec()).unwrap();
        cache.insert_current(cache.refill_token(), &a, value(1, b"abc"));
        let one = cache.get_current(&a).unwrap();
        let two = cache.get_current(&a).unwrap();
        assert!(Arc::ptr_eq(&one.bytes, &two.bytes));
        cache.insert_current(cache.refill_token(), &b, value(1, b"abc"));
        assert_eq!(
            cache.insert_current(cache.refill_token(), &c, value(1, b"abc")),
            2
        );
        assert!(cache.get_current(&a).is_none());
        assert_eq!(
            one.bytes.as_ref(),
            b"abc",
            "eviction leaves outstanding handle valid"
        );
        assert!(cache.state.lock().unwrap().charged_bytes <= 270);
        cache.insert_current(cache.refill_token(), &a, value(1, &[0; 512]));
        assert!(cache.get_current(&a).is_none());
        for n in 0..10000 {
            cache.invalidate_current(format!("missing-{n}").as_bytes(), ObjectVersion(9));
        }
        assert_eq!(cache.state.lock().unwrap().current.len(), 1);
    }

    #[test]
    fn local_remove_fences_pending_refill_and_zero_budget_disables_cache() {
        let cache = active_cache(1024);
        let key = Key::new(b"delete".to_vec()).unwrap();
        let token = cache.refill_token();
        cache.remove_current(&key);
        cache.insert_current(token, &key, value(1, b"old"));
        assert!(cache.get_current(&key).is_none());
        assert!(active_cache(0).refill_token().is_none());
    }

    #[test]
    fn out_of_order_refills_do_not_replace_newer_version() {
        let cache = active_cache(1024);
        let key = Key::new(b"order".to_vec()).unwrap();
        let token = cache.refill_token();
        cache.insert_current(token, &key, value(2, b"new"));
        cache.insert_current(token, &key, value(1, b"old"));
        assert_eq!(cache.get_current(&key), Some(value(2, b"new")));
    }

    #[test]
    fn expired_lease_and_delayed_renewal_cannot_revive_cached_bytes() {
        let cache = active_cache(1024);
        let key = Key::new(b"lease".to_vec()).unwrap();
        let token = cache.refill_token();
        cache.insert_current(token, &key, value(1, b"old"));
        let now = Instant::now();
        // 用注入的时刻确定性跨越租约，无 sleep 或墙钟假设。
        let expiry = cache.state.lock().unwrap().lease_deadline.unwrap();
        cache.renew_lease_at(expiry, Duration::from_secs(60), expiry);
        cache.insert_current(token, &key, value(1, b"old"));
        assert!(
            cache.get_current(&key).is_none(),
            "renewal after gap fences pending response"
        );
        let fresh = cache.refill_token();
        cache.insert_current(fresh, &key, value(2, b"new"));
        cache.renew_lease_at(now, Duration::from_millis(1), now + Duration::from_secs(1));
        assert!(
            cache.get_current(&key).is_none(),
            "delayed response cannot start a new lease at receipt time"
        );
        assert!(cache.refill_token().is_none());
    }

    #[test]
    fn cache_hit_checks_deadline_even_without_background_progress() {
        let cache = active_cache(1024);
        let key = Key::new(b"paused-process".to_vec()).unwrap();
        let token = cache.refill_token();
        cache.insert_current(token, &key, value(1, b"old"));
        cache.state.lock().unwrap().lease_deadline = Some(Instant::now());
        assert!(cache.get_current(&key).is_none());
        cache.insert_current(token, &key, value(1, b"old"));
        assert!(cache.get_current(&key).is_none());
    }

    #[test]
    fn generation_exhaustion_disables_cache_instead_of_wrapping() {
        let cache = active_cache(1024);
        cache.state.lock().unwrap().generation = u64::MAX - 1;
        cache.invalidate_current(b"missing", ObjectVersion(1));
        assert!(cache.refill_token().is_none());
        cache.session_connected();
        cache.renew_lease(Instant::now(), Duration::from_secs(60));
        assert!(cache.refill_token().is_none());
    }
}

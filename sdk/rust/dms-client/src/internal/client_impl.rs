//! 同步公开 API 与异步 NodeConnection 之间的协调层。
//!
//! 用户调用 `DmsClient::set()` 是同步函数；这里通过私有 Tokio Runtime 的
//! `block_on` 驱动异步 gRPC。这样首版 Rust SDK 对用户保持普通阻塞编程模型。

// Arc 共享 ClientCache；AtomicU64 生成无需 Mutex 的进程内 sequence。
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

use crate::client::{
    ClientOptions, ConnectError, DeleteResult, GetOptions, GetResult, HashDeleteOptions,
    HashEntriesResult, HashGetOptions, HashMultiGetResult, HashRangeWriteOptions,
    HashRangeWriteResult, HashScanOptions, HashScanResult, HashSetResult, HashValue,
    HashWriteOptions, MSetOptions, MSetResult, RangeWriteOptions, ResolvedClientOptions,
    SetOptions, SetResult,
};
use crate::metrics::{CacheLookup, ClientMetrics, ClientOperation};
use crate::{DmsError, HashEntry, HashField, Key, KvEntry, OperationId, ReadVersion, ScanCursor};

use super::{
    client_cache::{CachedValue, ClientCache, RefillToken},
    node_connection::{NodeConnection, SharedViewInner, SharedWriteInner},
};

type SharedMetrics = (
    ClientMetrics,
    dms_metrics::RpcMetrics,
    dms_metrics::ErrorMetrics,
);

fn shared_metrics(
    registry: &dms_metrics::Registry,
) -> Result<SharedMetrics, dms_metrics::MetricsError> {
    // 注册句柄的生命周期归宿主 Registry；独立连接与重连只 clone 固定标签的句柄。
    registry.get_or_register(|registry| {
        Ok((
            ClientMetrics::register(registry)?,
            dms_metrics::RpcMetrics::register(registry)?,
            dms_metrics::ErrorMetrics::register(registry)?,
        ))
    })
}

pub(crate) struct DmsClientImpl {
    // SDK 自己拥有 runtime，公开 API 无需要求用户处于 Tokio 环境。
    runtime: Runtime,
    // 当前只连接一个 Node；后续 NodeManager 会管理本地和远端多个连接。
    connection: NodeConnection,
    // Session 后台 Task 与同步 get/set 共享的 Current cache。
    cache: Arc<ClientCache>,
    // 保存连接级默认超时和 durability。
    options: ResolvedClientOptions,
    // 每个 SDK 实例启动时只生成一次；与 sequence 组成跨 Client 唯一的 OperationId。
    client_instance_id: [u8; 16],
    // 原子计数器允许多个线程共享同一个 DmsClient 时安全分配 ID。
    next_operation_id: AtomicU64,
    // None means the embedding application did not inject a Registry. Keeping
    // this optional avoids a hidden global exporter and any label work when disabled.
    metrics: Option<ClientMetrics>,
    error_metrics: Option<dms_metrics::ErrorMetrics>,
}

impl DmsClientImpl {
    pub(crate) fn connect(options: ClientOptions) -> Result<Self, ConnectError> {
        // Installs only a read-only correlation callback. The SDK still does
        // not create a subscriber, sampler, exporter, thread, or listener.
        dms_metrics::install_exemplar_provider(dms_tracing::current_exemplar);
        let resolved_options = options.resolve().map_err(ConnectError)?;
        let metrics = resolved_options
            .metrics_registry
            .as_ref()
            .map(shared_metrics)
            .transpose()
            .map_err(|error| {
                ConnectError(DmsError::client_protocol_violation(format!(
                    "failed to register DMS Client metrics: {error}"
                )))
            })?;
        let client_metrics = metrics.as_ref().map(|metrics| metrics.0.clone());
        let rpc_metrics = metrics.as_ref().map(|metrics| metrics.1.clone());
        let error_metrics = metrics.as_ref().map(|metrics| metrics.2.clone());
        // 创建多线程 Tokio Runtime；enable_all 打开网络、定时器等 I/O driver。
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .thread_name("dms-client")
            .build()
            // closure 捕获底层错误并包装成稳定的 SDK ConnectError。
            .map_err(|error| {
                ConnectError(DmsError::client_protocol_violation(format!(
                    "failed to create Tokio runtime: {error}"
                )))
            })?;
        // Arc::new 把缓存放到堆上，返回可在线程间 clone 的共享所有权指针。
        let cache = Arc::new(ClientCache::new(resolved_options.current_cache_bytes));
        // block_on 会阻塞当前调用线程，但 Runtime 内的网络 Task 正常异步调度。
        let connection = runtime
            .block_on(NodeConnection::connect(
                &resolved_options,
                // Arc::clone 只增加引用计数，不复制 HashMap 或缓存 bytes。
                Arc::clone(&cache),
                client_metrics.clone(),
                rpc_metrics,
            ))
            .map_err(ConnectError)?;
        Ok(Self {
            runtime,
            connection,
            cache,
            options: resolved_options,
            client_instance_id: *Uuid::new_v4().as_bytes(),
            next_operation_id: AtomicU64::new(1),
            metrics: client_metrics,
            error_metrics,
        })
    }

    /// Wraps one public SDK contract with the same count/latency/inflight semantics.
    pub(crate) fn observe<T>(
        &self,
        operation: ClientOperation,
        call: impl FnOnce() -> Result<T, DmsError>,
    ) -> Result<T, DmsError> {
        let span = operation.span();
        // The SDK creates spans but never installs a subscriber. `enter` is a
        // no-op in uninstrumented host applications and scopes the complete
        // synchronous API call when the host opted in.
        let _entered = span.enter();
        let mut guard = self
            .metrics
            .as_ref()
            .map(|metrics| metrics.begin_operation(operation));
        let result = call();
        if let Err(error) = &result
            && let Some(metrics) = &self.error_metrics
        {
            metrics.record_if_component(dms_metrics::ErrorComponent::Client, error);
        }
        if result.is_ok()
            && let Some(guard) = &mut guard
        {
            guard.success();
        }
        match &result {
            Ok(_) => dms_tracing::record_ok(&span),
            Err(error) => dms_tracing::record_error(&span, error),
        }
        result
    }

    pub(crate) fn set(
        &self,
        key: Key,
        value: &[u8],
        options: SetOptions,
    ) -> Result<SetResult, DmsError> {
        // Relaxed 足以保证 ID 不重复；这里不依赖该原子操作与其他内存的先后顺序。
        let operation_id = self.next_operation_id();
        let refill = self.cache.refill_token();
        // 同步 API 在这里等待完整 Allocate→Upload→Set 异步链结束。
        let result = self.runtime.block_on(self.connection.set(
            &key,
            value,
            options,
            self.options.default_durability,
            operation_id,
        ))?;
        // SHM 模式下 payload 本身已经在 mmap/slot 路径里，不把同一份 value 再
        // 复制进 Vec cache；非 SHM 模式保留写入方本地 Current cache 快路径。
        if self.current_bytes_cache_enabled() {
            self.refill_current(refill, &key, result.version, value);
        }
        Ok(result)
    }

    pub(crate) fn get(&self, key: Key, options: GetOptions) -> Result<Option<GetResult>, DmsError> {
        // 只有“完整 Current”可使用这个缓存；Exact/range 具有不同 cache key 语义。
        let cacheable = self.current_bytes_cache_enabled()
            && options.version == ReadVersion::Current
            && options.range.is_none();
        // let-chain 同时检查可缓存和命中，命中后直接返回，不访问 Node。
        if cacheable && let Some(cached) = self.cache.get_current(&key) {
            if let Some(metrics) = &self.metrics {
                metrics.record_cache_lookup(CacheLookup::Hit);
            }
            return Ok(Some(GetResult {
                version: cached.version,
                bytes: cached.bytes.to_vec(),
            }));
        }
        if cacheable && let Some(metrics) = &self.metrics {
            metrics.record_cache_lookup(CacheLookup::Miss);
        }
        // 未命中时同步等待异步 Get→Download 链路。
        let refill = self.cache.refill_token();
        let result = self.runtime.block_on(self.connection.get(&key, options))?;
        // `&result` 只借用 Option 内的值，确保最后仍能把 result 返回给调用者。
        if cacheable && let Some(value) = &result {
            self.refill_current(refill, &key, value.version, &value.bytes);
        }
        Ok(result)
    }

    pub(crate) fn allocate_write(
        &self,
        key: Key,
        len: usize,
        options: SetOptions,
    ) -> Result<SharedWriteInner, DmsError> {
        let operation_id = self.next_operation_id();
        self.runtime.block_on(self.connection.allocate_write(
            key,
            len,
            options,
            self.options.default_durability,
            operation_id,
        ))
    }

    pub(crate) fn commit_shared(&self, write: SharedWriteInner) -> Result<SetResult, DmsError> {
        let key = write.key.clone();
        let result = self
            .runtime
            .block_on(self.connection.commit_shared(write))?;
        self.cache.remove_current(&key);
        Ok(result)
    }

    pub(crate) fn get_view(
        &self,
        key: Key,
        options: GetOptions,
    ) -> Result<Option<SharedViewInner>, DmsError> {
        self.runtime
            .block_on(self.connection.get_view(&key, options))
    }

    pub(crate) fn del(&self, key: Key) -> Result<DeleteResult, DmsError> {
        let operation_id = self.next_operation_id();
        let result = self
            .runtime
            .block_on(self.connection.del(&key, operation_id))?;
        // DEL 成功后本 Client 不能继续返回自己旧的 Current 缓存。
        self.cache.remove_current(&key);
        Ok(result)
    }

    pub(crate) fn mset(
        &self,
        entries: &[KvEntry],
        options: MSetOptions,
    ) -> Result<MSetResult, DmsError> {
        if entries.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "MSET entries are empty".to_string(),
            ));
        }
        reject_duplicate_keys(entries.iter().map(|entry| entry.key.as_bytes()))?;
        let operation_id = self.next_operation_id();
        let result = self.runtime.block_on(self.connection.mset(
            entries,
            options,
            self.options.default_durability,
            operation_id,
        ))?;
        for entry in entries {
            self.cache.remove_current(&entry.key);
        }
        Ok(result)
    }

    pub(crate) fn mget(&self, keys: &[Key]) -> Result<Vec<Option<GetResult>>, DmsError> {
        if keys.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "MGET keys are empty".to_string(),
            ));
        }
        let refill = self.cache.refill_token();
        let result = self.runtime.block_on(self.connection.mget(keys))?;
        if self.current_bytes_cache_enabled() {
            for (key, value) in keys.iter().zip(result.iter()) {
                if let Some(value) = value {
                    self.refill_current(refill, key, value.version, &value.bytes);
                }
            }
        }
        Ok(result)
    }

    /// SHM values already live in Node-owned mmap Regions. Keeping a second
    /// owned `Vec<u8>` cache would add a full payload copy and two divergent
    /// lifecycles, so every Current-cache entry point shares this one policy.
    fn current_bytes_cache_enabled(&self) -> bool {
        current_bytes_cache_enabled_for(self.options.shared_memory)
            && self.options.current_cache_bytes > 0
    }

    fn refill_current(
        &self,
        token: Option<RefillToken>,
        key: &Key,
        version: crate::ObjectVersion,
        bytes: &[u8],
    ) {
        // 关闭、断线或单值超预算时不先做一份注定无法保留的 payload 复制。
        if token.is_none() || !self.cache.accepts_value(key, bytes.len()) {
            return;
        }
        let evicted = self.cache.insert_current(
            token,
            key,
            CachedValue {
                version,
                bytes: bytes.into(),
            },
        );
        if evicted > 0
            && let Some(metrics) = &self.metrics
        {
            metrics.record_cache_capacity_evictions(evicted);
        }
    }

    pub(crate) fn set_range(
        &self,
        key: Key,
        offset: u64,
        data: &[u8],
        options: RangeWriteOptions,
    ) -> Result<SetResult, DmsError> {
        if data.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "SET_RANGE data is empty".to_string(),
            ));
        }
        let operation_id = self.next_operation_id();
        let result = self.runtime.block_on(self.connection.set_range(
            &key,
            offset,
            data,
            options,
            self.options.default_durability,
            operation_id,
        ))?;
        self.cache.remove_current(&key);
        Ok(result)
    }

    pub(crate) fn hset(
        &self,
        key: Key,
        entries: &[HashEntry],
        options: HashWriteOptions,
    ) -> Result<HashSetResult, DmsError> {
        if entries.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "HSET entries are empty".to_string(),
            ));
        }
        reject_duplicate_keys(entries.iter().map(|entry| entry.field.as_bytes()))?;
        let operation_id = self.next_operation_id();
        let entries = entries
            .iter()
            .map(|entry| (entry.field.clone(), entry.value.clone()))
            .collect::<Vec<_>>();
        let result = self.runtime.block_on(self.connection.hset(
            &key,
            &entries,
            options,
            self.options.default_durability,
            operation_id,
        ))?;
        self.cache.remove_current(&key);
        Ok(result)
    }

    pub(crate) fn hget(
        &self,
        key: Key,
        field: HashField,
        options: HashGetOptions,
    ) -> Result<Option<HashValue>, DmsError> {
        self.runtime
            .block_on(self.connection.hget(&key, &field, options))
    }

    pub(crate) fn hmget(
        &self,
        key: Key,
        fields: &[HashField],
        options: HashGetOptions,
    ) -> Result<HashMultiGetResult, DmsError> {
        self.runtime
            .block_on(self.connection.hmget(&key, fields, options))
    }

    pub(crate) fn hgetall(
        &self,
        key: Key,
        options: HashGetOptions,
    ) -> Result<HashEntriesResult, DmsError> {
        self.runtime
            .block_on(self.connection.hget_all(&key, options))
    }

    pub(crate) fn hdel(
        &self,
        key: Key,
        fields: &[HashField],
        options: HashDeleteOptions,
    ) -> Result<HashSetResult, DmsError> {
        if fields.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "HDEL fields are empty".to_string(),
            ));
        }
        reject_duplicate_keys(fields.iter().map(HashField::as_bytes))?;
        let operation_id = self.next_operation_id();
        let result = self.runtime.block_on(self.connection.hdelete(
            &key,
            fields,
            options,
            self.options.default_durability,
            operation_id,
        ))?;
        self.cache.remove_current(&key);
        Ok(result)
    }

    pub(crate) fn hscan(
        &self,
        key: Key,
        cursor: ScanCursor,
        options: HashScanOptions,
    ) -> Result<HashScanResult, DmsError> {
        if options.limit == 0 {
            return Err(DmsError::client_invalid_argument(
                "HSCAN limit must be positive".to_string(),
            ));
        }
        self.runtime
            .block_on(self.connection.hscan(&key, cursor, options))
    }

    pub(crate) fn hwrite_at(
        &self,
        key: Key,
        field: HashField,
        offset: u64,
        data: &[u8],
        options: HashRangeWriteOptions,
    ) -> Result<HashRangeWriteResult, DmsError> {
        if data.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "HWRITE_AT data is empty".to_string(),
            ));
        }
        let operation_id = self.next_operation_id();
        let result = self.runtime.block_on(self.connection.hwrite_at(
            &key,
            &field,
            offset,
            data,
            options,
            self.options.default_durability,
            operation_id,
        ))?;
        self.cache.remove_current(&key);
        Ok(result)
    }

    fn next_operation_id(&self) -> OperationId {
        OperationId::new(
            self.client_instance_id,
            self.next_operation_id.fetch_add(1, Ordering::Relaxed),
        )
    }
}

fn current_bytes_cache_enabled_for(shared_memory: bool) -> bool {
    !shared_memory
}

fn reject_duplicate_keys<'a>(keys: impl Iterator<Item = &'a [u8]>) -> Result<(), DmsError> {
    let mut seen = HashSet::new();
    for key in keys {
        if !seen.insert(key.to_vec()) {
            return Err(DmsError::client_invalid_argument(
                "duplicate key or field in batch".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod cache_policy_tests {
    use super::current_bytes_cache_enabled_for;

    #[test]
    fn shm_disables_the_owned_current_bytes_cache_for_every_call_site() {
        assert!(!current_bytes_cache_enabled_for(true));
        assert!(current_bytes_cache_enabled_for(false));
    }

    #[test]
    fn independent_clients_share_registry_handles_after_drop_and_reconnect() {
        let registry = dms_metrics::registry();
        let first = super::shared_metrics(&registry).unwrap();
        let second = super::shared_metrics(&registry.clone()).unwrap();
        let first_connection = first.0.node_connection_guard();
        let second_connection = second.0.node_connection_guard();
        drop(
            first
                .0
                .begin_operation(crate::metrics::ClientOperation::Get),
        );
        drop(
            second
                .0
                .begin_operation(crate::metrics::ClientOperation::Get),
        );
        let text = dms_metrics::encode_text(&registry).unwrap();
        assert!(text.contains("dms_client_node_connection_up 2"), "{text}");
        drop(first_connection);
        drop(first);
        let third = super::shared_metrics(&registry).unwrap();
        drop(
            third
                .0
                .begin_operation(crate::metrics::ClientOperation::Get),
        );
        drop(second_connection);
        let text = dms_metrics::encode_text(&registry).unwrap();
        assert!(text.contains("dms_client_node_connection_up 0"), "{text}");
        assert!(
            text.contains("dms_client_operations_total{operation=\"get\",result=\"error\"} 3"),
            "{text}"
        );
    }
}

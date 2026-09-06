//! DMS 提供给应用程序的北向 Rust API。
//!
//! 方法名有意采用 Redis String/Hash 词汇来降低学习成本，但不兼容 RESP 或 Redis
//! Cluster。DMS value 可以位于共享内存并跨节点。`hset` 额外提供 Merge/Replace：
//! Merge 保留未出现字段，Replace 以本次字段集合发布完整的新 Hash 版本。

// fmt 用于错误 Display；Arc 让 DmsClient clone 后共享同一个实现；Duration 表示超时。
use std::{fmt, sync::Arc, time::Duration};

use dms_error::DmsError;
use dms_metrics::Registry;

use crate::{
    ByteRange, DurabilityPolicy, HashEntry, HashField, HashReadVersion, HashVersion, HashWriteMode,
    Key, KvEntry, ObjectVersion, ReadVersion, ScanCursor, WriteCondition,
};

use crate::internal::client_impl::DmsClientImpl;
use crate::metrics::ClientOperation;

/// 应用直接持有的 DMS Client。
///
/// Applications only depend on this northbound type and the public domain
/// values re-exported by `dms-client`. Transport proxies, shared-memory mapping,
/// session lifecycle, and retry bookkeeping remain SDK implementation details.
#[derive(Clone)]
pub struct DmsClient {
    // Pimpl 风格：公开类型很薄，内部模块和依赖不会暴露给 SDK 用户。
    // Arc 使 `DmsClient::clone()` 只增加引用计数，不复制连接、Runtime 或缓存。
    client_impl: Arc<DmsClientImpl>,
}

impl DmsClient {
    /// Connects to a local or remote DMS worker.
    ///
    /// `unix:///run/dms/node.sock`, `http://host:port`, and
    /// `https://host:port` all create the same business client. Only the
    /// connector interprets the endpoint scheme.
    pub fn connect(
        // impl AsRef<str> 允许调用方传 `&str`、`String` 或其他可借用为 str 的类型。
        endpoint: impl AsRef<str>,
        mut options: ClientOptions,
    ) -> Result<Self, ConnectError> {
        options.endpoint = Some(endpoint.as_ref().to_string());
        Self::connect_with_options(options)
    }

    /// Connects after resolving endpoint and tuning from options/environment.
    ///
    /// This is the entry point for applications that want the endpoint to come
    /// from `DMS_ENDPOINT` in ordinary deployments and only override it in code
    /// for tests or explicit remote-node failover.
    pub fn connect_with_options(options: ClientOptions) -> Result<Self, ConnectError> {
        // `?` 把 ConnectError 提前返回；成功后将实现放入 Arc。
        Ok(Self {
            client_impl: Arc::new(DmsClientImpl::connect(options)?),
        })
    }

    /// Creates or completely replaces one top-level value.
    pub fn set(&self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<SetResult, DmsError> {
        self.client_impl.observe(ClientOperation::Set, || {
            self.client_impl
                .set(valid_key(key)?, value, SetOptions::default())
        })
    }

    /// Creates or replaces one value with an explicit condition or durability.
    pub fn set_with_options(
        &self,
        key: impl AsRef<[u8]>,
        value: &[u8],
        options: SetOptions,
    ) -> Result<SetResult, DmsError> {
        self.client_impl.observe(ClientOperation::Set, || {
            self.client_impl.set(valid_key(key)?, value, options)
        })
    }

    /// Reads the complete current value, returning `None` for a missing key.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>, DmsError> {
        // 第一个 `?` 传播 DmsError；Option::map 只在命中时取出 bytes。
        self.client_impl.observe(ClientOperation::Get, || {
            Ok(self
                .client_impl
                .get(valid_key(key)?, GetOptions::default())?
                .map(|result| result.bytes))
        })
    }

    /// Allocates a node-owned shared-memory write buffer for one future `SET`.
    ///
    /// This is the explicit zero-copy path. The caller writes directly into the
    /// returned buffer, then passes it to [`Self::commit_shared`]. Remote TCP
    /// endpoints or nodes without SHM negotiation return an unsupported DMS error
    /// instead of silently falling back to a copied `Vec`.
    pub fn allocate_write(
        &self,
        key: impl AsRef<[u8]>,
        len: usize,
    ) -> Result<SharedWriteBuffer, DmsError> {
        self.allocate_write_with_options(key, len, SetOptions::default())
    }

    /// Allocates a shared-memory write buffer with explicit commit options.
    pub fn allocate_write_with_options(
        &self,
        key: impl AsRef<[u8]>,
        len: usize,
        options: SetOptions,
    ) -> Result<SharedWriteBuffer, DmsError> {
        self.client_impl
            .observe(ClientOperation::AllocateWrite, || {
                Ok(SharedWriteBuffer {
                    inner: self
                        .client_impl
                        .allocate_write(valid_key(key)?, len, options)?,
                })
            })
    }

    /// Publishes a previously allocated shared-memory write buffer.
    ///
    /// The SDK computes the transfer receipt from the current mmap contents at
    /// commit time. After this call the buffer is consumed, so user code cannot
    /// accidentally mutate bytes behind a committed version.
    pub fn commit_shared(&self, buffer: SharedWriteBuffer) -> Result<SetResult, DmsError> {
        self.client_impl.observe(ClientOperation::CommitShared, || {
            self.client_impl.commit_shared(buffer.inner)
        })
    }

    /// Reads one value as an exact shared-memory view when local SHM is enabled.
    ///
    /// The ordinary [`Self::get`] API still materializes `Vec<u8>`. This method
    /// keeps the mapped bytes alive inside [`SharedValueView`] and avoids the
    /// extra copy for local consumers.
    pub fn get_view(&self, key: impl AsRef<[u8]>) -> Result<Option<SharedValueView>, DmsError> {
        self.get_view_with_options(key, GetOptions::default())
    }

    /// Reads a selected version/range as a shared-memory view.
    pub fn get_view_with_options(
        &self,
        key: impl AsRef<[u8]>,
        options: GetOptions,
    ) -> Result<Option<SharedValueView>, DmsError> {
        self.client_impl.observe(ClientOperation::GetView, || {
            Ok(self
                .client_impl
                .get_view(valid_key(key)?, options)?
                .map(|inner| SharedValueView { inner }))
        })
    }

    /// Deletes the logical Current value by publishing a Tombstone version.
    ///
    /// Returns `deleted=false` when the key was already missing. This matches
    /// Redis DEL's idempotent spirit while preserving DMS version metadata.
    pub fn del(&self, key: impl AsRef<[u8]>) -> Result<DeleteResult, DmsError> {
        self.client_impl.observe(ClientOperation::Delete, || {
            self.client_impl.del(valid_key(key)?)
        })
    }

    /// Reads an exact version and/or byte range.
    pub fn get_with_options(
        &self,
        key: impl AsRef<[u8]>,
        options: GetOptions,
    ) -> Result<Option<GetResult>, DmsError> {
        self.client_impl.observe(ClientOperation::Get, || {
            self.client_impl.get(valid_key(key)?, options)
        })
    }

    /// Replaces several independent keys in one Meta-native atomic batch.
    ///
    /// Meta validates every entry before appending one journal record, so either
    /// every new version becomes visible at the same commit index or none does.
    pub fn mset(&self, entries: &[KvEntry], options: MSetOptions) -> Result<MSetResult, DmsError> {
        self.client_impl.observe(ClientOperation::MSet, || {
            self.client_impl.mset(entries, options)
        })
    }

    /// Reads several independent current keys and preserves input order.
    ///
    /// Each key is resolved independently; this first contract does not promise
    /// a cross-key snapshot. Missing keys appear as `None` in the result vector.
    pub fn mget<T: AsRef<[u8]>>(&self, keys: &[T]) -> Result<Vec<Option<GetResult>>, DmsError> {
        self.client_impl.observe(ClientOperation::MGet, || {
            let keys = keys.iter().map(valid_key).collect::<Result<Vec<_>, _>>()?;
            self.client_impl.mget(&keys)
        })
    }

    /// Overwrites a byte range of an existing value and publishes a new version.
    pub fn set_range(
        &self,
        key: impl AsRef<[u8]>,
        offset: u64,
        data: &[u8],
    ) -> Result<SetResult, DmsError> {
        self.client_impl.observe(ClientOperation::SetRange, || {
            self.client_impl
                .set_range(valid_key(key)?, offset, data, RangeWriteOptions::default())
        })
    }

    /// Performs a conditional range write against an explicit base version.
    pub fn set_range_with_options(
        &self,
        key: impl AsRef<[u8]>,
        offset: u64,
        data: &[u8],
        options: RangeWriteOptions,
    ) -> Result<SetResult, DmsError> {
        self.client_impl.observe(ClientOperation::SetRange, || {
            self.client_impl
                .set_range(valid_key(key)?, offset, data, options)
        })
    }

    /// Sets one or more fields and publishes one new Hash/KKV version.
    ///
    /// [`HashWriteMode::Merge`] preserves omitted fields and matches ordinary
    /// Redis `HSET`. [`HashWriteMode::Replace`] removes omitted fields, which is
    /// useful when publishing a complete checkpoint manifest. In both modes all
    /// supplied fields become visible together under one new [`HashVersion`].
    pub fn hset(
        &self,
        key: impl AsRef<[u8]>,
        entries: &[HashEntry],
        options: HashWriteOptions,
    ) -> Result<HashSetResult, DmsError> {
        self.client_impl.observe(ClientOperation::HSet, || {
            self.client_impl.hset(valid_key(key)?, entries, options)
        })
    }

    /// Reads one field from one current Hash field-map version.
    pub fn hget(
        &self,
        key: impl AsRef<[u8]>,
        field: impl AsRef<[u8]>,
    ) -> Result<Option<HashValue>, DmsError> {
        self.client_impl.observe(ClientOperation::HGet, || {
            self.client_impl.hget(
                valid_key(key)?,
                valid_field(field)?,
                HashGetOptions::default(),
            )
        })
    }

    /// Reads one field from a selected Hash version.
    pub fn hget_with_options(
        &self,
        key: impl AsRef<[u8]>,
        field: impl AsRef<[u8]>,
        options: HashGetOptions,
    ) -> Result<Option<HashValue>, DmsError> {
        self.client_impl.observe(ClientOperation::HGet, || {
            self.client_impl
                .hget(valid_key(key)?, valid_field(field)?, options)
        })
    }

    /// Reads selected fields from one shared Hash field-map version.
    pub fn hmget(
        &self,
        key: impl AsRef<[u8]>,
        fields: &[impl AsRef<[u8]>],
        options: HashGetOptions,
    ) -> Result<HashMultiGetResult, DmsError> {
        self.client_impl.observe(ClientOperation::HMGet, || {
            let fields = fields
                .iter()
                .map(valid_field)
                .collect::<Result<Vec<_>, _>>()?;
            self.client_impl.hmget(valid_key(key)?, &fields, options)
        })
    }

    /// Reads all fields from a bounded Hash object.
    ///
    /// Large checkpoint or filesystem manifests should use [`Self::hscan`] or
    /// [`Self::hmget`] so one call cannot materialize unbounded metadata/bytes.
    pub fn hgetall(
        &self,
        key: impl AsRef<[u8]>,
        options: HashGetOptions,
    ) -> Result<HashEntriesResult, DmsError> {
        self.client_impl.observe(ClientOperation::HGetAll, || {
            self.client_impl.hgetall(valid_key(key)?, options)
        })
    }

    /// Removes selected fields and publishes one new merged Hash version.
    pub fn hdel(
        &self,
        key: impl AsRef<[u8]>,
        fields: &[impl AsRef<[u8]>],
        options: HashDeleteOptions,
    ) -> Result<HashSetResult, DmsError> {
        self.client_impl.observe(ClientOperation::HDelete, || {
            let fields = fields
                .iter()
                .map(valid_field)
                .collect::<Result<Vec<_>, _>>()?;
            self.client_impl.hdel(valid_key(key)?, &fields, options)
        })
    }

    /// Iterates through a bounded page of one Hash field-map version.
    pub fn hscan(
        &self,
        key: impl AsRef<[u8]>,
        cursor: ScanCursor,
        options: HashScanOptions,
    ) -> Result<HashScanResult, DmsError> {
        self.client_impl.observe(ClientOperation::HScan, || {
            self.client_impl.hscan(valid_key(key)?, cursor, options)
        })
    }

    /// Applies a random byte-range update to one existing Hash field.
    pub fn hwrite_at(
        &self,
        key: impl AsRef<[u8]>,
        field: impl AsRef<[u8]>,
        offset: u64,
        data: &[u8],
        options: HashRangeWriteOptions,
    ) -> Result<HashRangeWriteResult, DmsError> {
        self.client_impl.observe(ClientOperation::HWriteAt, || {
            self.client_impl
                .hwrite_at(valid_key(key)?, valid_field(field)?, offset, data, options)
        })
    }
}

/// Writable bytes backed by a dms-node owned shared-memory slot.
pub struct SharedWriteBuffer {
    inner: crate::internal::node_connection::SharedWriteInner,
}

impl SharedWriteBuffer {
    /// Mutable view over the staged bytes. User code fills this before commit.
    pub fn as_mut_slice(&mut self) -> Result<&mut [u8], DmsError> {
        self.inner.as_mut_slice()
    }

    /// Length reserved by `allocate_write`.
    pub fn len(&self) -> Result<usize, DmsError> {
        self.inner.len()
    }

    /// Whether this buffer contains zero bytes.
    pub fn is_empty(&self) -> Result<bool, DmsError> {
        Ok(self.len()? == 0)
    }
}

/// Read-only bytes backed by a dms-node owned shared-memory slot.
pub struct SharedValueView {
    inner: crate::internal::node_connection::SharedViewInner,
}

impl SharedValueView {
    /// Version selected by the read operation.
    #[must_use]
    pub fn version(&self) -> ObjectVersion {
        self.inner.version()
    }

    /// Borrow the mapped bytes. The slice remains valid while this view lives.
    pub fn as_slice(&self) -> Result<&[u8], DmsError> {
        self.inner.as_slice()
    }

    /// Number of mapped bytes exposed by this view.
    pub fn len(&self) -> Result<usize, DmsError> {
        self.inner.len()
    }

    /// Whether this view contains zero bytes.
    pub fn is_empty(&self) -> Result<bool, DmsError> {
        Ok(self.len()? == 0)
    }
}

fn valid_key(value: impl AsRef<[u8]>) -> Result<Key, DmsError> {
    Key::new(value.as_ref().to_vec())
        .map_err(|error| DmsError::client_invalid_argument(error.to_string()))
}

fn valid_field(value: impl AsRef<[u8]>) -> Result<HashField, DmsError> {
    HashField::new(value.as_ref().to_vec())
        .map_err(|error| DmsError::client_invalid_argument(error.to_string()))
}

/// Connection-level overrides shared by simple API calls.
///
/// `None` means "the application did not explicitly choose this field". The
/// SDK resolves values in one place with this order:
///
/// `built-in default < DMS_* environment < explicit ClientOptions field`.
#[derive(Clone, Debug, Default)]
pub struct ClientOptions {
    /// Local or remote Worker endpoint, e.g. `unix:///run/dms/worker.sock` or
    /// `http://host:19200`.
    pub endpoint: Option<String>,
    /// End-to-end operation timeout override.
    pub timeout: Option<Duration>,
    /// Reliability override used when an operation does not override it.
    pub default_durability: Option<DurabilityPolicy>,
    /// 小对象 SET 请求内联阈值，同时作为非 SHM 单 GET 响应的内联预算。
    /// GET 预算另受协议 64 KiB 上限约束；不影响 SHM View 或 MGET。
    pub inline_threshold_bytes: Option<usize>,
    /// Session heartbeat interval；缓存续租还会按 Node 返回 TTL 缩短此间隔。
    pub heartbeat_interval: Option<Duration>,
    /// Bounded queue capacity for the session stream.
    pub session_channel_capacity: Option<usize>,
    /// Current owned cache 预算，默认 64 MiB；0 关闭。按 key、payload 和固定条目
    /// 开销计费，超预算整批淘汰；不是进程 RSS 限额。SHM 模式不保留 owned cache。
    pub current_cache_bytes: Option<usize>,
    /// TLS mode for the gRPC transport baseline.
    pub tls: Option<ClientTlsOptions>,
    /// Prefer local shared-memory payload targets when the endpoint is UDS and
    /// the node accepts zero-copy negotiation. TCP/remote sessions are ignored
    /// by the server and fall back to gRPC targets.
    pub shared_memory: Option<bool>,
    /// Optional application-owned Prometheus registry.
    ///
    /// The SDK never opens an HTTP listener. Embedding applications register
    /// DMS Client metrics in their existing registry and expose it themselves.
    /// 同一 Registry 的独立 Client 复用已注册句柄；统计聚合，不增加 Client ID 标签。
    pub metrics_registry: Option<Registry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientTlsOptions {
    Disabled,
}

/// Fully resolved SDK configuration used by private implementation modules.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedClientOptions {
    pub(crate) endpoint: String,
    pub(crate) timeout: Duration,
    pub(crate) default_durability: DurabilityPolicy,
    pub(crate) inline_threshold_bytes: usize,
    pub(crate) heartbeat_interval: Duration,
    pub(crate) session_channel_capacity: usize,
    pub(crate) current_cache_bytes: usize,
    pub(crate) tls: ClientTlsOptions,
    pub(crate) shared_memory: bool,
    pub(crate) metrics_registry: Option<Registry>,
}

impl ClientOptions {
    pub(crate) fn resolve(self) -> Result<ResolvedClientOptions, DmsError> {
        self.resolve_with_env(|name| std::env::var(name).ok())
    }

    pub(crate) fn resolve_with_env(
        self,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<ResolvedClientOptions, DmsError> {
        let mut resolved = ResolvedClientOptions::builtin_default();
        if let Some(value) = env("DMS_ENDPOINT") {
            if value.is_empty() {
                return Err(DmsError::client_invalid_argument(
                    "DMS_ENDPOINT must not be empty".to_string(),
                ));
            }
            resolved.endpoint = value;
        }
        if let Some(value) = env("DMS_TIMEOUT_MILLIS") {
            resolved.timeout = parse_duration_millis("DMS_TIMEOUT_MILLIS", &value)?;
        }
        if let Some(value) = env("DMS_DEFAULT_DURABILITY") {
            resolved.default_durability = parse_durability("DMS_DEFAULT_DURABILITY", &value)?;
        }
        if let Some(value) = env("DMS_INLINE_THRESHOLD_BYTES") {
            resolved.inline_threshold_bytes =
                parse_positive_usize("DMS_INLINE_THRESHOLD_BYTES", &value)?;
        }
        if let Some(value) = env("DMS_HEARTBEAT_INTERVAL_MILLIS") {
            resolved.heartbeat_interval =
                parse_duration_millis("DMS_HEARTBEAT_INTERVAL_MILLIS", &value)?;
        }
        if let Some(value) = env("DMS_SESSION_CHANNEL_CAPACITY") {
            resolved.session_channel_capacity =
                parse_positive_usize("DMS_SESSION_CHANNEL_CAPACITY", &value)?;
        }
        if let Some(value) = env("DMS_CURRENT_CACHE_BYTES") {
            resolved.current_cache_bytes = value.parse::<usize>().map_err(|_| {
                DmsError::client_invalid_argument(
                    "DMS_CURRENT_CACHE_BYTES must be an unsigned integer".to_string(),
                )
            })?;
        }
        if let Some(value) = env("DMS_TLS_MODE") {
            resolved.tls = parse_tls_mode("DMS_TLS_MODE", &value)?;
        }
        if let Some(value) = env("DMS_SHARED_MEMORY") {
            resolved.shared_memory = parse_bool("DMS_SHARED_MEMORY", &value)?;
        }
        if let Some(endpoint) = self.endpoint {
            if endpoint.is_empty() {
                return Err(DmsError::client_invalid_argument(
                    "endpoint must not be empty".to_string(),
                ));
            }
            resolved.endpoint = endpoint;
        }
        if let Some(timeout) = self.timeout {
            resolved.timeout = timeout;
        }
        if let Some(durability) = self.default_durability {
            resolved.default_durability = durability;
        }
        if let Some(threshold) = self.inline_threshold_bytes {
            if threshold == 0 {
                return Err(DmsError::client_invalid_argument(
                    "inline_threshold_bytes must be positive".to_string(),
                ));
            }
            resolved.inline_threshold_bytes = threshold;
        }
        if let Some(heartbeat) = self.heartbeat_interval {
            if heartbeat.is_zero() {
                return Err(DmsError::client_invalid_argument(
                    "heartbeat_interval must be positive".to_string(),
                ));
            }
            resolved.heartbeat_interval = heartbeat;
        }
        if let Some(capacity) = self.session_channel_capacity {
            if capacity == 0 {
                return Err(DmsError::client_invalid_argument(
                    "session_channel_capacity must be positive".to_string(),
                ));
            }
            resolved.session_channel_capacity = capacity;
        }
        if let Some(tls) = self.tls {
            resolved.tls = tls;
        }
        if let Some(budget) = self.current_cache_bytes {
            resolved.current_cache_bytes = budget;
        }
        if let Some(shared_memory) = self.shared_memory {
            resolved.shared_memory = shared_memory;
        }
        if let Some(metrics_registry) = self.metrics_registry {
            resolved.metrics_registry = Some(metrics_registry);
        }
        if resolved.endpoint.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "DMS endpoint is required; pass DmsClient::connect(endpoint, ...) or set ClientOptions.endpoint/DMS_ENDPOINT".to_string(),
            ));
        }
        Ok(resolved)
    }
}

impl ResolvedClientOptions {
    fn builtin_default() -> Self {
        Self {
            endpoint: String::new(),
            timeout: Duration::from_secs(30),
            default_durability: DurabilityPolicy::LocalMemory,
            inline_threshold_bytes: 64 * 1024,
            heartbeat_interval: Duration::from_secs(10),
            session_channel_capacity: 64,
            current_cache_bytes: 64 * 1024 * 1024,
            tls: ClientTlsOptions::Disabled,
            shared_memory: false,
            metrics_registry: None,
        }
    }
}

fn parse_duration_millis(name: &'static str, value: &str) -> Result<Duration, DmsError> {
    let millis = value.parse::<u64>().map_err(|_| {
        DmsError::client_invalid_argument(format!(
            "{name} must be an unsigned integer in milliseconds"
        ))
    })?;
    if millis == 0 {
        return Err(DmsError::client_invalid_argument(format!(
            "{name} must be positive"
        )));
    }
    Ok(Duration::from_millis(millis))
}

fn parse_positive_usize(name: &'static str, value: &str) -> Result<usize, DmsError> {
    let parsed = value.parse::<usize>().map_err(|_| {
        DmsError::client_invalid_argument(format!("{name} must be a positive unsigned integer"))
    })?;
    if parsed == 0 {
        return Err(DmsError::client_invalid_argument(format!(
            "{name} must be positive"
        )));
    }
    Ok(parsed)
}

fn parse_durability(name: &'static str, value: &str) -> Result<DurabilityPolicy, DmsError> {
    match value {
        "local-memory" | "LOCAL_MEMORY" => Ok(DurabilityPolicy::LocalMemory),
        "local-disk" | "LOCAL_DISK" => Ok(DurabilityPolicy::LocalDisk),
        "object-store" | "OBJECT_STORE" => Ok(DurabilityPolicy::ObjectStore),
        _ => Err(DmsError::client_invalid_argument(format!(
            "{name} must be one of local-memory, local-disk, object-store"
        ))),
    }
}

fn parse_tls_mode(name: &'static str, value: &str) -> Result<ClientTlsOptions, DmsError> {
    match value {
        "disabled" | "DISABLED" => Ok(ClientTlsOptions::Disabled),
        _ => Err(DmsError::client_invalid_argument(format!(
            "{name} must be disabled in this build"
        ))),
    }
}

fn parse_bool(name: &'static str, value: &str) -> Result<bool, DmsError> {
    match value {
        "1" | "true" | "TRUE" | "yes" | "YES" => Ok(true),
        "0" | "false" | "FALSE" | "no" | "NO" => Ok(false),
        _ => Err(DmsError::client_invalid_argument(format!(
            "{name} must be true/false, yes/no or 1/0"
        ))),
    }
}

#[cfg(test)]
mod client_options_tests {
    use super::*;

    #[test]
    fn api_options_override_environment_and_builtin_defaults() {
        let options = ClientOptions {
            endpoint: Some("http://api".to_string()),
            timeout: Some(Duration::from_millis(7)),
            default_durability: Some(DurabilityPolicy::ObjectStore),
            inline_threshold_bytes: Some(99),
            heartbeat_interval: Some(Duration::from_millis(11)),
            session_channel_capacity: Some(13),
            current_cache_bytes: Some(256),
            tls: Some(ClientTlsOptions::Disabled),
            shared_memory: Some(true),
            metrics_registry: None,
        };

        let resolved = options
            .resolve_with_env(|name| match name {
                "DMS_TIMEOUT_MILLIS" => Some("5".to_string()),
                "DMS_DEFAULT_DURABILITY" => Some("local-disk".to_string()),
                "DMS_INLINE_THRESHOLD_BYTES" => Some("42".to_string()),
                "DMS_ENDPOINT" => Some("http://env".to_string()),
                "DMS_HEARTBEAT_INTERVAL_MILLIS" => Some("9".to_string()),
                "DMS_SESSION_CHANNEL_CAPACITY" => Some("10".to_string()),
                "DMS_CURRENT_CACHE_BYTES" => Some("128".to_string()),
                "DMS_TLS_MODE" => Some("disabled".to_string()),
                "DMS_SHARED_MEMORY" => Some("false".to_string()),
                _ => None,
            })
            .expect("resolve");

        assert_eq!(resolved.endpoint, "http://api");
        assert_eq!(resolved.timeout, Duration::from_millis(7));
        assert_eq!(resolved.default_durability, DurabilityPolicy::ObjectStore);
        assert_eq!(resolved.inline_threshold_bytes, 99);
        assert_eq!(resolved.heartbeat_interval, Duration::from_millis(11));
        assert_eq!(resolved.session_channel_capacity, 13);
        assert_eq!(resolved.current_cache_bytes, 256);
        assert!(resolved.shared_memory);
    }

    #[test]
    fn environment_overrides_builtin_defaults_when_api_is_unspecified() {
        let resolved = ClientOptions::default()
            .resolve_with_env(|name| match name {
                "DMS_TIMEOUT_MILLIS" => Some("5".to_string()),
                "DMS_DEFAULT_DURABILITY" => Some("local-disk".to_string()),
                "DMS_INLINE_THRESHOLD_BYTES" => Some("42".to_string()),
                "DMS_ENDPOINT" => Some("http://env".to_string()),
                "DMS_HEARTBEAT_INTERVAL_MILLIS" => Some("9".to_string()),
                "DMS_SESSION_CHANNEL_CAPACITY" => Some("10".to_string()),
                "DMS_SHARED_MEMORY" => Some("true".to_string()),
                _ => None,
            })
            .expect("resolve");

        assert_eq!(resolved.endpoint, "http://env");
        assert_eq!(resolved.timeout, Duration::from_millis(5));
        assert_eq!(resolved.default_durability, DurabilityPolicy::LocalDisk);
        assert_eq!(resolved.inline_threshold_bytes, 42);
        assert_eq!(resolved.heartbeat_interval, Duration::from_millis(9));
        assert_eq!(resolved.session_channel_capacity, 10);
        assert!(resolved.shared_memory);
    }

    #[test]
    fn invalid_environment_value_is_rejected() {
        let error = ClientOptions::default()
            .resolve_with_env(|name| (name == "DMS_TIMEOUT_MILLIS").then(|| "0".to_string()))
            .expect_err("zero timeout");

        assert_eq!(error.kind(), dms_error::ErrorKind::InvalidArgument);
    }

    #[test]
    fn cache_budget_accepts_zero_and_rejects_invalid_environment() {
        let resolved = ClientOptions {
            endpoint: Some("http://node".to_string()),
            ..Default::default()
        }
        .resolve_with_env(|name| (name == "DMS_CURRENT_CACHE_BYTES").then(|| "0".to_string()))
        .unwrap();
        assert_eq!(resolved.current_cache_bytes, 0);
        assert!(
            ClientOptions::default()
                .resolve_with_env(
                    |name| (name == "DMS_CURRENT_CACHE_BYTES").then(|| "-1".to_string())
                )
                .is_err()
        );
    }
}

/// Optional controls for one top-level `SET`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetOptions {
    /// Atomic create/update/version condition.
    pub condition: WriteCondition,
    /// Per-call reliability override.
    pub durability: Option<DurabilityPolicy>,
}

impl Default for SetOptions {
    fn default() -> Self {
        Self {
            condition: WriteCondition::Any,
            durability: None,
        }
    }
}

/// Controls one current or exact-version read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GetOptions {
    /// Version to resolve.
    pub version: ReadVersion,
    /// Optional half-open byte range.
    pub range: Option<ByteRange>,
}

impl Default for GetOptions {
    fn default() -> Self {
        Self {
            version: ReadVersion::Current,
            range: None,
        }
    }
}

/// Controls one ordered multi-key `MSET` batch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MSetOptions {
    /// Reliability override applied to every entry.
    pub durability: Option<DurabilityPolicy>,
}

/// Controls a random write to a normal object.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RangeWriteOptions {
    /// Optional exact base version. Without it, the runtime binds Current once.
    pub expected_version: Option<ObjectVersion>,
    /// Reliability override for the newly published version.
    pub durability: Option<DurabilityPolicy>,
}

/// Controls one versioned `HSET` publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HashWriteOptions {
    /// How this call treats fields omitted from `entries`.
    pub mode: HashWriteMode,
    /// Optional compare-and-set guard on the whole Hash field map.
    pub expected_version: Option<HashVersion>,
    /// Reliability override for every new field value.
    pub durability: Option<DurabilityPolicy>,
}

impl Default for HashWriteOptions {
    fn default() -> Self {
        Self {
            mode: HashWriteMode::Merge,
            expected_version: None,
            durability: None,
        }
    }
}

/// Controls one incremental `HDEL` publication.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HashDeleteOptions {
    /// Optional compare-and-set guard on the whole Hash field map.
    pub expected_version: Option<HashVersion>,
    /// Reliability override for the resulting field-map publication.
    pub durability: Option<DurabilityPolicy>,
}

/// Controls version selection for Hash reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HashGetOptions {
    /// Hash field-map version from which every requested field must be resolved.
    pub version: HashReadVersion,
}

impl Default for HashGetOptions {
    fn default() -> Self {
        Self {
            version: HashReadVersion::Current,
        }
    }
}

/// Controls one bounded Hash scan page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HashScanOptions {
    /// Hash field-map version to scan.
    pub version: HashReadVersion,
    /// Maximum number of fields requested for this page.
    pub limit: usize,
}

impl Default for HashScanOptions {
    fn default() -> Self {
        Self {
            version: HashReadVersion::Current,
            limit: 128,
        }
    }
}

/// Controls a byte-range update of one Hash field.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HashRangeWriteOptions {
    /// Optional exact base Hash field-map version.
    pub expected_version: Option<HashVersion>,
    /// Reliability override for the new field/object version.
    pub durability: Option<DurabilityPolicy>,
}

/// Result of one normal object commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetResult {
    /// Newly published immutable version.
    pub version: ObjectVersion,
    /// Logical length after the commit.
    pub len: u64,
}

/// Result of one top-level `DEL`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteResult {
    /// Whether this call changed a visible value into a Tombstone.
    pub deleted: bool,
    /// Authoritative version after processing the delete.
    pub version: ObjectVersion,
}

/// Result of one normal object read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetResult {
    /// Immutable version from which `bytes` were copied.
    pub version: ObjectVersion,
    /// Complete or range-selected bytes.
    pub bytes: Vec<u8>,
}

/// Version result for one entry in an ordered `MSET`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyVersion {
    /// Key committed by the batch.
    pub key: Key,
    /// New version assigned to that key.
    pub version: ObjectVersion,
}

/// Result of one ordered multi-key `MSET` batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MSetResult {
    /// Versions aligned to input key order.
    pub versions: Vec<KeyVersion>,
}

/// Result of one Hash field-map commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashSetResult {
    /// New field-map version shared by all updated fields.
    pub version: HashVersion,
    /// Total fields visible after the commit.
    pub field_count: u64,
}

/// One materialized Hash field value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashValue {
    /// Secondary key.
    pub field: HashField,
    /// Hash field-map version that selected this field.
    pub hash_version: HashVersion,
    /// Immutable value object backing this field.
    pub value_version: ObjectVersion,
    /// Field bytes.
    pub bytes: Vec<u8>,
}

/// Ordered result of `HMGET`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashMultiGetResult {
    /// One field-map version shared by every result slot.
    pub version: Option<HashVersion>,
    /// Values aligned to the requested field order; misses are `None`.
    pub values: Vec<Option<HashValue>>,
}

/// Complete bounded result of `HGETALL`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashEntriesResult {
    /// Field-map version shared by all entries.
    pub version: Option<HashVersion>,
    /// Existing fields returned by the service.
    pub entries: Vec<HashValue>,
}

/// One bounded page returned by `HSCAN`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashScanResult {
    /// Field-map version shared by this page.
    pub version: Option<HashVersion>,
    /// Cursor to pass to the next call; zero means complete.
    pub next_cursor: ScanCursor,
    /// Existing fields in this page.
    pub entries: Vec<HashValue>,
}

/// Result of a byte-range update to one Hash field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashRangeWriteResult {
    /// New Hash field-map version selected by subsequent readers.
    pub hash_version: HashVersion,
    /// New immutable value version backing the patched field.
    pub value_version: ObjectVersion,
    /// Field length after applying the patch.
    pub len: u64,
    /// Total fields visible in the new field map.
    pub field_count: u64,
}

/// Failure to establish the node-local SDK session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectError(pub DmsError);

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Formatter<'_> 的匿名生命周期由编译器推导，只在本次格式化调用内有效。
        write!(
            formatter,
            "failed to connect DMS client runtime: {}",
            self.0
        )
    }
}

// Error Trait 没有必须实现的方法；Display+Debug 已足够使用默认实现。
impl std::error::Error for ConnectError {}

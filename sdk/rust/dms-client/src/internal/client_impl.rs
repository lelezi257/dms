//! 同步公开 API 与异步 NodeConnection 之间的协调层。
//!
//! 用户调用 `DmsClient::set()` 是同步函数；这里通过私有 Tokio Runtime 的
//! `block_on` 驱动异步 gRPC。这样首版 Rust SDK 对用户保持普通阻塞编程模型。

// AtomicU64 生成无需 Mutex 的进程内 sequence。
use std::{
    collections::HashSet,
    io::{self, Read},
    sync::{
        RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::runtime::{Builder, Runtime};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::client::{
    ClientOptions, ConnectError, DeleteResult, GetIntoResult, GetOptions, GetResult,
    HashDeleteOptions, HashEntriesResult, HashGetOptions, HashMultiGetResult,
    HashRangeWriteOptions, HashRangeWriteResult, HashScanOptions, HashScanResult, HashSetResult,
    HashValue, HashWriteOptions, MSetOptions, MSetResult, RangeWriteOptions, ResolvedClientOptions,
    SetOptions, SetResult,
};
use crate::metrics::{ClientMetrics, ClientOperation};
use crate::{
    DmsError, HashEntry, HashField, Key, KvEntry, ObjectInfo, OperationId, ScanCursor, ScanOptions,
    ScanResult,
};

use super::node_connection::{NodeConnection, SharedViewInner, SharedWriteInner, ValueReaderInner};

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
    // 当前只连接一个 Node。这里用可替换的连接快照承载 Session 换代：
    // 前台请求先 clone 当前快照，失败确认是 Session 级故障后再原子替换。
    connection: RwLock<NodeConnection>,
    // Session 故障恢复必须单飞：同一代 Session 断开时，只允许一个前台请求
    // 真正执行 OpenSession；其他并发请求等待它完成后复用新连接。
    reconnect_gate: AsyncMutex<()>,
    // 保存连接级默认超时和 durability。
    options: ResolvedClientOptions,
    // 每个 SDK 实例启动时只生成一次；与 sequence 组成跨 Client 唯一的 OperationId。
    client_instance_id: [u8; 16],
    // 原子计数器允许多个线程共享同一个 DmsClient 时安全分配 ID。
    next_operation_id: AtomicU64,
    // None means the embedding application did not inject a Registry. Keeping
    // this optional avoids a hidden global exporter and any label work when disabled.
    metrics: Option<ClientMetrics>,
    rpc_metrics: Option<dms_metrics::RpcMetrics>,
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
        // block_on 会阻塞当前调用线程，但 Runtime 内的网络 Task 正常异步调度。
        let connection = runtime
            .block_on(NodeConnection::connect(
                &resolved_options,
                client_metrics.clone(),
                rpc_metrics.clone(),
            ))
            .map_err(ConnectError)?;
        Ok(Self {
            runtime,
            connection: RwLock::new(connection),
            reconnect_gate: AsyncMutex::new(()),
            options: resolved_options,
            client_instance_id: *Uuid::new_v4().as_bytes(),
            next_operation_id: AtomicU64::new(1),
            metrics: client_metrics,
            rpc_metrics,
            error_metrics,
        })
    }

    fn current_connection(&self) -> Result<NodeConnection, DmsError> {
        self.connection
            .read()
            .map_err(|_| {
                DmsError::client_protocol_violation("DMS client connection lock is poisoned")
            })
            .map(|connection| connection.clone())
    }

    async fn reconnect_after(&self, failed_session_id: u64) -> Result<NodeConnection, DmsError> {
        if let Ok(connection) = self.current_connection()
            && connection.session_id() != failed_session_id
        {
            return Ok(connection);
        }

        let _singleflight = self.reconnect_gate.lock().await;
        if let Ok(connection) = self.current_connection()
            && connection.session_id() != failed_session_id
        {
            return Ok(connection);
        }

        let new_connection = NodeConnection::connect(
            &self.options,
            self.metrics.clone(),
            self.rpc_metrics.clone(),
        )
        .await?;
        let mut guard = self.connection.write().map_err(|_| {
            DmsError::client_protocol_violation("DMS client connection lock is poisoned")
        })?;
        if guard.session_id() == failed_session_id {
            *guard = new_connection.clone();
            Ok(new_connection)
        } else {
            Ok(guard.clone())
        }
    }

    async fn run_with_session_recovery<T, Fut>(
        &self,
        should_reopen: fn(&DmsError) -> bool,
        mut call: impl FnMut(NodeConnection) -> Fut,
    ) -> Result<T, DmsError>
    where
        Fut: std::future::Future<Output = Result<T, DmsError>>,
    {
        let first = self.current_connection()?;
        let failed_session_id = first.session_id();
        match call(first).await {
            Ok(value) => Ok(value),
            Err(error) if should_reopen(&error) => {
                let second = self.reconnect_after(failed_session_id).await?;
                call(second).await
            }
            Err(error) => Err(error),
        }
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
        // 同步 API 在这里等待完整 Allocate→Upload→Set 异步链结束。
        self.runtime.block_on(self.run_with_session_recovery(
            should_reopen_idempotent_write,
            |connection| {
                let key = key.clone();
                async move {
                    connection
                        .set(
                            &key,
                            value,
                            options,
                            self.options.default_durability,
                            operation_id,
                        )
                        .await
                }
            },
        ))
    }

    pub(crate) fn set_from<R: Read>(
        &self,
        key: Key,
        src: R,
        length: u64,
        options: SetOptions,
    ) -> Result<SetResult, DmsError> {
        let operation_id = self.next_operation_id();
        self.runtime.block_on(async {
            let mut src = CountingReader::new(src);
            let first = self.current_connection()?;
            let failed_session_id = first.session_id();
            match first
                .set_from(
                    &key,
                    &mut src,
                    length,
                    options,
                    self.options.default_durability,
                    operation_id,
                )
                .await
            {
                Ok(result) => Ok(result),
                Err(error) if should_reopen_idempotent_write(&error) && src.bytes_read() == 0 => {
                    let second = self.reconnect_after(failed_session_id).await?;
                    second
                        .set_from(
                            &key,
                            &mut src,
                            length,
                            options,
                            self.options.default_durability,
                            operation_id,
                        )
                        .await
                }
                Err(error) if should_reopen_idempotent_write(&error) => {
                    Err(set_from_reader_consumed_error(src.bytes_read(), error))
                }
                Err(error) => Err(error),
            }
        })
    }

    pub(crate) fn get(&self, key: Key, options: GetOptions) -> Result<Option<GetResult>, DmsError> {
        // 薄 Client 原则：每次读取都到 Node，由 Node 复用本地 Current/Block。
        // SDK 只返回 owned Vec 或显式 SharedValueView，不再跨请求保存 value bytes。
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let key = key.clone();
                async move { connection.get(&key, options).await }
            }),
        )
    }

    pub(crate) fn get_into(
        &self,
        key: Key,
        dst: &mut [u8],
        options: GetOptions,
    ) -> Result<Option<GetIntoResult>, DmsError> {
        self.runtime.block_on(async {
            let (connection, plan) = self
                .run_with_session_recovery(should_reopen_read, |connection| {
                    let key = key.clone();
                    async move {
                        let plan = connection.get_into_plan(&key, options).await?;
                        Ok::<_, DmsError>((connection, plan))
                    }
                })
                .await?;
            match plan {
                Some(plan) => connection.copy_get_into_plan(plan, dst).await.map(Some),
                None => Ok(None),
            }
        })
    }

    pub(crate) fn get_reader(
        &self,
        key: Key,
        options: GetOptions,
    ) -> Result<Option<ValueReaderInner>, DmsError> {
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let key = key.clone();
                async move { connection.get_reader(&key, options).await }
            }),
        )
    }

    pub(crate) fn read_value_reader(
        &self,
        reader: &mut ValueReaderInner,
        dst: &mut [u8],
    ) -> Result<usize, DmsError> {
        self.runtime.block_on(async {
            let connection = self.current_connection()?;
            connection.read_value_reader(reader, dst).await
        })
    }

    pub(crate) fn stat(&self, key: Key) -> Result<Option<ObjectInfo>, DmsError> {
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let key = key.clone();
                async move { connection.stat(&key).await }
            }),
        )
    }

    pub(crate) fn scan(&self, prefix: &[u8], options: ScanOptions) -> Result<ScanResult, DmsError> {
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let options = options.clone();
                async move { connection.scan(prefix, options).await }
            }),
        )
    }

    pub(crate) fn allocate_write(
        &self,
        key: Key,
        len: usize,
        options: SetOptions,
    ) -> Result<SharedWriteInner, DmsError> {
        let operation_id = self.next_operation_id();
        self.runtime.block_on(async {
            let connection = self.current_connection()?;
            connection
                .allocate_write(
                    key,
                    len,
                    options,
                    self.options.default_durability,
                    operation_id,
                )
                .await
        })
    }

    pub(crate) fn commit_shared(&self, write: SharedWriteInner) -> Result<SetResult, DmsError> {
        self.runtime.block_on(async {
            let connection = self.current_connection()?;
            connection.commit_shared(write).await
        })
    }

    pub(crate) fn get_view(
        &self,
        key: Key,
        options: GetOptions,
    ) -> Result<Option<SharedViewInner>, DmsError> {
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let key = key.clone();
                async move { connection.get_view(&key, options).await }
            }),
        )
    }

    pub(crate) fn del(&self, key: Key) -> Result<DeleteResult, DmsError> {
        let operation_id = self.next_operation_id();
        self.runtime.block_on(self.run_with_session_recovery(
            should_reopen_idempotent_write,
            |connection| {
                let key = key.clone();
                async move { connection.del(&key, operation_id).await }
            },
        ))
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
        self.runtime.block_on(self.run_with_session_recovery(
            should_reopen_idempotent_write,
            |connection| async move {
                connection
                    .mset(
                        entries,
                        options,
                        self.options.default_durability,
                        operation_id,
                    )
                    .await
            },
        ))
    }

    pub(crate) fn mget(&self, keys: &[Key]) -> Result<Vec<Option<GetResult>>, DmsError> {
        if keys.is_empty() {
            return Err(DmsError::client_invalid_argument(
                "MGET keys are empty".to_string(),
            ));
        }
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| async move {
                connection.mget(keys).await
            }),
        )
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
        self.runtime.block_on(self.run_with_session_recovery(
            should_reopen_idempotent_write,
            |connection| {
                let key = key.clone();
                async move {
                    connection
                        .set_range(
                            &key,
                            offset,
                            data,
                            options,
                            self.options.default_durability,
                            operation_id,
                        )
                        .await
                }
            },
        ))
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
        self.runtime.block_on(self.run_with_session_recovery(
            should_reopen_idempotent_write,
            |connection| {
                let key = key.clone();
                let entries = entries.clone();
                async move {
                    connection
                        .hset(
                            &key,
                            &entries,
                            options,
                            self.options.default_durability,
                            operation_id,
                        )
                        .await
                }
            },
        ))
    }

    pub(crate) fn hget(
        &self,
        key: Key,
        field: HashField,
        options: HashGetOptions,
    ) -> Result<Option<HashValue>, DmsError> {
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let key = key.clone();
                let field = field.clone();
                async move { connection.hget(&key, &field, options).await }
            }),
        )
    }

    pub(crate) fn hmget(
        &self,
        key: Key,
        fields: &[HashField],
        options: HashGetOptions,
    ) -> Result<HashMultiGetResult, DmsError> {
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let key = key.clone();
                async move { connection.hmget(&key, fields, options).await }
            }),
        )
    }

    pub(crate) fn hgetall(
        &self,
        key: Key,
        options: HashGetOptions,
    ) -> Result<HashEntriesResult, DmsError> {
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let key = key.clone();
                async move { connection.hget_all(&key, options).await }
            }),
        )
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
        self.runtime.block_on(self.run_with_session_recovery(
            should_reopen_idempotent_write,
            |connection| {
                let key = key.clone();
                async move {
                    connection
                        .hdelete(
                            &key,
                            fields,
                            options,
                            self.options.default_durability,
                            operation_id,
                        )
                        .await
                }
            },
        ))
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
        self.runtime.block_on(
            self.run_with_session_recovery(should_reopen_read, |connection| {
                let key = key.clone();
                async move { connection.hscan(&key, cursor, options).await }
            }),
        )
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
        self.runtime.block_on(self.run_with_session_recovery(
            should_reopen_idempotent_write,
            |connection| {
                let key = key.clone();
                let field = field.clone();
                async move {
                    connection
                        .hwrite_at(
                            &key,
                            &field,
                            offset,
                            data,
                            options,
                            self.options.default_durability,
                            operation_id,
                        )
                        .await
                }
            },
        ))
    }

    fn next_operation_id(&self) -> OperationId {
        OperationId::new(
            self.client_instance_id,
            self.next_operation_id.fetch_add(1, Ordering::Relaxed),
        )
    }
}

impl Drop for DmsClientImpl {
    fn drop(&mut self) {
        // 正常关闭时给 Node 一次有界 final heartbeat：把已完成普通读的
        // finished_read_request_through、已释放 View 水位，以及未冲刷的 SHM 写
        // lease token 发出去。它只发生在 Client 实例销毁时，不给每次 GET 增加
        // RPC；仍被用户持有的 SharedValueView 不会提前进入完成水位。
        //
        // 当前 Rust SDK 是同步 API + 私有 Tokio Runtime：和现有 set/get 一样，
        // Drop 也假定不在另一个 Tokio Runtime 的执行上下文里阻塞调用。这个
        // 使用限制先记录在这里，后续如要支持 async SDK 再单独扩展架构。
        let connection = match self.current_connection() {
            Ok(connection) => connection,
            Err(error) => {
                log::warn!("DMS client shutdown could not read current connection: {error}");
                return;
            }
        };
        if let Err(error) = self.runtime.block_on(connection.close()) {
            log::warn!("DMS client shutdown flush did not complete: {error}");
        }
    }
}

fn should_reopen_read(error: &DmsError) -> bool {
    error.code() == dms_error::NODE_SESSION_UNKNOWN
        || error.code() == dms_error::CLIENT_CONNECTION_UNAVAILABLE
}

fn should_reopen_idempotent_write(error: &DmsError) -> bool {
    // 写请求只有在调用点确认输入可重放、且同一个 OperationId 会被复用时，
    // 才能把连接级 unknown outcome 交给服务端幂等表处理。
    error.code() == dms_error::NODE_SESSION_UNKNOWN
        || error.code() == dms_error::CLIENT_CONNECTION_UNAVAILABLE
}

fn set_from_reader_consumed_error(bytes_read: u64, original: DmsError) -> DmsError {
    DmsError::client_connection_unavailable(format!(
        "SET_FROM cannot be retried after reading {bytes_read} bytes from the caller's Reader; original error: {original}"
    ))
}

struct CountingReader<R> {
    inner: R,
    bytes_read: u64,
}

impl<R> CountingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            bytes_read: 0,
        }
    }

    fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.bytes_read = self.bytes_read.saturating_add(read as u64);
        Ok(read)
    }
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
mod thin_client_tests {
    use std::io::Read;

    use crate::DmsError;

    #[test]
    fn session_recovery_strategies_keep_business_errors_terminal() {
        let session_unknown = DmsError::new(
            dms_error::NODE_SESSION_UNKNOWN,
            dms_error::ErrorKind::Unauthenticated,
            "session lost",
        );
        let connection_lost = DmsError::client_connection_unavailable("transport closed");
        let invalid_argument = DmsError::client_invalid_argument("bad key");

        assert!(super::should_reopen_read(&session_unknown));
        assert!(super::should_reopen_read(&connection_lost));
        assert!(!super::should_reopen_read(&invalid_argument));

        // 写响应丢失属于 unknown outcome；只有已确认可重放且复用 OperationId
        // 的写路径才会选用这个策略，普通业务错误不能被重试掩盖。
        assert!(super::should_reopen_idempotent_write(&session_unknown));
        assert!(super::should_reopen_idempotent_write(&connection_lost));
        assert!(!super::should_reopen_idempotent_write(&invalid_argument));
    }

    #[test]
    fn set_from_retry_knows_whether_reader_was_consumed() {
        let mut reader = super::CountingReader::new(std::io::Cursor::new(b"abcdef".to_vec()));
        let mut buf = [0_u8; 3];

        assert_eq!(reader.bytes_read(), 0);
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(reader.bytes_read(), 3);

        let error = super::set_from_reader_consumed_error(
            reader.bytes_read(),
            DmsError::client_connection_unavailable("lost response"),
        );
        assert_eq!(error.code(), dms_error::CLIENT_CONNECTION_UNAVAILABLE);
        assert!(
            error
                .message()
                .contains("cannot be retried after reading 3 bytes"),
            "{error}"
        );
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

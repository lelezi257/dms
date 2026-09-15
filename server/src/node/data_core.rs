//! dms-node 内部统一对象数据核心。
//!
//! `DataCoreHandle` 不是第二个 runtime，也不拥有任何对象状态。它只是把
//! 文件、镜像、KV 等进程内入口都收敛到已有的 [`NodeHandle`]：版本解析、Node
//! Current cache、Peer pull singleflight、Arena/Region、Meta 提交和失效屏障仍然
//! 只发生在唯一 `NodeState` owner 内。
//!
//! 本模块的公开类型只表达 DMS 对象语义：object key、version、byte range 和用户
//! 目标 buffer。它故意不出现进程边界或上层入口的专用术语，避免上层领域语义污染
//! 核心数据路径。

use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

use uuid::Uuid;

use super::metrics::DataCoreOperation;
use super::runtime::{DataCoreReadAttempt, NodeHandle, WorkerError};
use crate::filesystem::{PreparedObjectVersion, ResolvedObject};

static DATA_CORE_INSTANCE_ID: OnceLock<[u8; 16]> = OnceLock::new();
static DATA_CORE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 用户可见对象在 dms-node 内部的稳定名字。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ObjectKey(Vec<u8>);

impl ObjectKey {
    pub(crate) fn new(value: impl Into<Vec<u8>>) -> Result<Self, WorkerError> {
        let value = value.into();
        super::runtime::validate_user_key(&value)?;
        Ok(Self(value))
    }

    fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl From<ObjectKey> for Vec<u8> {
    fn from(key: ObjectKey) -> Self {
        key.0
    }
}

/// 同一对象版本内的一段逻辑字节范围。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ByteRange {
    pub(crate) offset: u64,
    pub(crate) length: u64,
}

impl ByteRange {
    pub(crate) fn new(offset: u64, length: u64) -> Result<Self, WorkerError> {
        offset
            .checked_add(length)
            .ok_or(WorkerError::InvalidArgument("byte range overflows u64"))?;
        Ok(Self { offset, length })
    }
}

/// 读取哪个逻辑版本。默认 `Current` 表示读最新可见版本。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum VersionSelector {
    #[default]
    Current,
    #[allow(dead_code, reason = "由已固化但尚未挂载的 Image 读取入口使用")]
    Exact(u64),
}

impl VersionSelector {
    fn exact_version(self) -> Option<u64> {
        match self {
            Self::Current => None,
            Self::Exact(version) => Some(version),
        }
    }
}

/// 一次对象读请求；`clamp_range` 让文件/Image 入口可按自身语义裁剪尾部读取。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReadOptions {
    pub(crate) version: VersionSelector,
    pub(crate) range: Option<ByteRange>,
    pub(crate) clamp_range: bool,
}

impl ReadOptions {
    #[cfg(test)]
    pub(crate) fn current_range(range: ByteRange) -> Self {
        Self {
            version: VersionSelector::Current,
            range: Some(range),
            clamp_range: true,
        }
    }
}

/// 成功读取到的对象字节。`logical_length` 始终是完整对象长度，不是本次返回长度。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectRead {
    pub(crate) version: u64,
    pub(crate) logical_length: u64,
    pub(crate) bytes: Vec<u8>,
}

/// `read_into` 的轻量返回值；bytes 已经被写入调用者提供的 buffer。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code, reason = "保留用户目标 buffer 合同，当前由回归测试穿刺")]
pub(crate) struct ObjectReadMeta {
    pub(crate) version: u64,
    pub(crate) logical_length: u64,
    pub(crate) bytes_read: usize,
}

/// 写入或删除后生成的新对象状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ObjectWrite {
    pub(crate) version: u64,
    pub(crate) length: u64,
}

/// 删除结果需要保留“本次是否真的删除了一个存在对象”。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ObjectDelete {
    pub(crate) deleted: bool,
    pub(crate) version: u64,
}

/// 同一对象版本上的 range patch。扩容/稀疏文件语义由文件子系统先转换。
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RangeWrite {
    pub(crate) offset: u64,
    pub(crate) bytes: Vec<u8>,
    pub(crate) expected_version: Option<u64>,
}

/// 对象元数据快照，不暴露进程边界上的传输结构。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ObjectStat {
    pub(crate) version: u64,
    pub(crate) length: u64,
}

/// Node 进程内子系统共用的对象数据句柄。
///
/// 句柄只保存一个 `NodeHandle` clone。clone `NodeHandle` 只是复制 mailbox 发送端；
/// 所有读写仍回到唯一 `NodeState`。它不承担 Worker RPC 的 session/staging 语义，
/// 对外 KV 请求由 `WorkerService` 直接调用 `NodeHandle`。
#[derive(Clone)]
pub(crate) struct DataCoreHandle {
    node: NodeHandle,
}

impl DataCoreHandle {
    /// 为进程内子系统创建 DataCore 入口。它不经过 Worker gRPC，也不创建
    /// Client/KV session；Peer singleflight、Arena、Meta 提交都仍在 Node owner 内共享。
    #[cfg_attr(
        not(any(test, feature = "fuse")),
        allow(dead_code, reason = "进程内句柄由可选 FUSE 支持和回归测试创建")
    )]
    pub(crate) fn new(node: NodeHandle) -> Self {
        Self { node }
    }

    pub(crate) fn new_operation_id(&self) -> Vec<u8> {
        next_operation_id()
    }

    pub(crate) async fn stat(&self, key: ObjectKey) -> Result<Option<ObjectStat>, WorkerError> {
        let key = key.into_bytes();
        let response = self.node.data_core_stat(key).await?;
        let Some(info) = response.info else {
            return Ok(None);
        };
        Ok(response.found.then_some(ObjectStat {
            version: info.version,
            length: info.length,
        }))
    }

    pub(crate) async fn read(
        &self,
        key: ObjectKey,
        options: ReadOptions,
    ) -> Result<Option<ObjectRead>, WorkerError> {
        let capacity = match options.range {
            // Range 读的调用方已经给出最大返回长度。这里不再为了预分配 buffer
            // 额外 stat 一次 Meta；实际尾部裁剪和 bytes_read 由 Node owner 的
            // 同一套 layout/range 校验完成。
            Some(range) => {
                usize::try_from(range.length).map_err(|_| WorkerError::ResourceExhausted)?
            }
            None => {
                let Some(capacity) = self.read_capacity(key.clone(), options).await? else {
                    return Ok(None);
                };
                capacity
            }
        };
        let range = options.range.map(|range| (range.offset, range.length));
        let first = self
            .node
            .data_core_read_into(
                key.clone().into_bytes(),
                options.version.exact_version(),
                range,
                options.clamp_range,
                vec![0; capacity],
            )
            .await?;
        let result = match first {
            DataCoreReadAttempt::Ready(result) => result,
            DataCoreReadAttempt::NotFound => return Ok(None),
            DataCoreReadAttempt::BufferTooSmall { version, required } => {
                // `read_capacity` 与真实 resolve 之间 Current 可能变长，Exact 版本也
                // 可能比 Current 更长。第二次固定读取第一次已经解析出的 version，
                // 所以扩容 buffer 不会把两个 Current 版本混进同一次返回。
                match self
                    .node
                    .data_core_read_into(
                        key.into_bytes(),
                        Some(version),
                        range,
                        options.clamp_range,
                        vec![0; required],
                    )
                    .await?
                {
                    DataCoreReadAttempt::Ready(result) => result,
                    DataCoreReadAttempt::NotFound => return Ok(None),
                    DataCoreReadAttempt::BufferTooSmall { .. } => {
                        return Err(WorkerError::ResourceExhausted);
                    }
                }
            }
        };
        let mut bytes = result.bytes;
        bytes.truncate(result.bytes_read);
        Ok(Some(ObjectRead {
            version: result.version,
            logical_length: result.logical_length,
            bytes,
        }))
    }

    #[allow(dead_code, reason = "保留用户目标 buffer 合同，当前由回归测试穿刺")]
    pub(crate) async fn read_into(
        &self,
        key: ObjectKey,
        options: ReadOptions,
        output: &mut [u8],
    ) -> Result<Option<ObjectReadMeta>, WorkerError> {
        let range = options.range.map(|range| (range.offset, range.length));
        // mailbox command 需要把请求所有权移动给 Node owner，不能把调用方栈上的
        // `&mut [u8]` 借用跨 async 发送过去。这里仍只让 owner 填充一份 owned
        // Vec，返回后再拷贝到调用者 buffer；这是 `read_into` 这个“用户提供
        // buffer”API 在当前 actor 边界下的显式成本。需要 owned 返回值时应走
        // `read()`，它不会再通过本函数多分配一份中间 buffer。
        let result = match self
            .node
            .data_core_read_into(
                key.into_bytes(),
                options.version.exact_version(),
                range,
                options.clamp_range,
                vec![0; output.len()],
            )
            .await?
        {
            DataCoreReadAttempt::Ready(result) => result,
            DataCoreReadAttempt::NotFound => return Ok(None),
            DataCoreReadAttempt::BufferTooSmall { .. } => {
                return Err(WorkerError::ResourceExhausted);
            }
        };
        if output.len() < result.bytes_read {
            return Err(WorkerError::ResourceExhausted);
        }
        output[..result.bytes_read].copy_from_slice(&result.bytes[..result.bytes_read]);
        Ok(Some(ObjectReadMeta {
            version: result.version,
            logical_length: result.logical_length,
            bytes_read: result.bytes_read,
        }))
    }

    /// 使用上层已授权的精确版本计划读取，避免 inode binding 命中后再次访问 Meta。
    pub(crate) async fn read_resolved(
        &self,
        resolved: ResolvedObject,
        range: ByteRange,
    ) -> Result<Option<ObjectRead>, WorkerError> {
        let metrics = self.node.metrics();
        metrics.record_data_core_operation(DataCoreOperation::ReadResolved);
        let capacity = usize::try_from(range.length).map_err(|_| WorkerError::ResourceExhausted)?;
        let result = self
            .node
            .data_core_read_pre_resolved_into(
                resolved,
                Some((range.offset, range.length)),
                true,
                vec![0; capacity],
            )
            .await?;
        let result = match result {
            DataCoreReadAttempt::Ready(result) => result,
            DataCoreReadAttempt::NotFound => return Ok(None),
            DataCoreReadAttempt::BufferTooSmall { .. } => {
                return Err(WorkerError::ResourceExhausted);
            }
        };
        let mut bytes = result.bytes;
        bytes.truncate(result.bytes_read);
        metrics.record_data_core_bytes(DataCoreOperation::ReadResolved, bytes.len());
        Ok(Some(ObjectRead {
            version: result.version,
            logical_length: result.logical_length,
            bytes,
        }))
    }

    #[allow(dead_code, reason = "由跨子系统回归测试使用")]
    pub(crate) async fn put(
        &self,
        key: ObjectKey,
        bytes: Vec<u8>,
    ) -> Result<ObjectWrite, WorkerError> {
        self.put_with_operation(key, bytes, next_operation_id(), "any".to_string())
            .await
    }

    /// 仅当对象仍是 `expected_version` 时替换完整 value。
    ///
    /// 文件扩容必须先读取旧 value 再构造新 value；把读到的版本带回提交，可以防止
    /// 并发写在“读旧值”和“提交新值”之间成功后又被本次扩容静默覆盖。
    #[cfg(test)]
    pub(crate) async fn put_if_version(
        &self,
        key: ObjectKey,
        bytes: Vec<u8>,
        expected_version: u64,
    ) -> Result<ObjectWrite, WorkerError> {
        self.put_with_operation(
            key,
            bytes,
            next_operation_id(),
            format!("if-version:{expected_version}"),
        )
        .await
    }

    /// 仅在对象不存在时创建完整 value。
    #[cfg(test)]
    pub(crate) async fn put_if_absent(
        &self,
        key: ObjectKey,
        bytes: Vec<u8>,
    ) -> Result<ObjectWrite, WorkerError> {
        self.put_with_operation(key, bytes, next_operation_id(), "if-absent".to_string())
            .await
    }

    pub(crate) async fn put_with_operation(
        &self,
        key: ObjectKey,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        condition: String,
    ) -> Result<ObjectWrite, WorkerError> {
        let result = self
            .node
            .data_core_set_inline(key.into_bytes(), bytes, operation_id, condition)
            .await?;
        Ok(ObjectWrite {
            version: result.version,
            length: result.length,
        })
    }

    pub(crate) async fn prepare_put(
        &self,
        key: ObjectKey,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        expected_version: Option<u64>,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let metrics = self.node.metrics();
        metrics.record_data_core_operation(DataCoreOperation::PreparePut);
        metrics.record_data_core_bytes(DataCoreOperation::PreparePut, bytes.len());
        self.node
            .data_core_prepare_inline(key.into_bytes(), bytes, operation_id, expected_version)
            .await
    }

    pub(crate) async fn prepare_range(
        &self,
        key: ObjectKey,
        offset: u64,
        bytes: Vec<u8>,
        operation_id: Vec<u8>,
        resolved: ResolvedObject,
    ) -> Result<PreparedObjectVersion, WorkerError> {
        let metrics = self.node.metrics();
        metrics.record_data_core_operation(DataCoreOperation::PrepareRange);
        metrics.record_data_core_bytes(DataCoreOperation::PrepareRange, bytes.len());
        self.node
            .data_core_prepare_range(key.into_bytes(), offset, bytes, operation_id, resolved)
            .await
    }

    pub(crate) async fn finish_prepared(
        &self,
        prepared: PreparedObjectVersion,
        version: Option<u64>,
        rejected: bool,
    ) -> Result<(), WorkerError> {
        self.node
            .metrics()
            .record_data_core_operation(DataCoreOperation::FinishPrepared);
        self.node
            .data_core_finish_prepared(prepared, version, rejected)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn write_range(
        &self,
        key: ObjectKey,
        patch: RangeWrite,
    ) -> Result<ObjectWrite, WorkerError> {
        let result = self
            .node
            .data_core_set_range_inline(
                key.into_bytes(),
                patch.offset,
                patch.bytes,
                next_operation_id(),
                patch.expected_version,
            )
            .await?;
        Ok(ObjectWrite {
            version: result.version,
            length: result.length,
        })
    }

    #[cfg(test)]
    pub(crate) async fn delete(&self, key: ObjectKey) -> Result<ObjectDelete, WorkerError> {
        self.delete_with_operation(key, next_operation_id()).await
    }

    #[cfg(test)]
    pub(crate) async fn delete_with_operation(
        &self,
        key: ObjectKey,
        operation_id: Vec<u8>,
    ) -> Result<ObjectDelete, WorkerError> {
        let result = self
            .node
            .data_core_delete(key.into_bytes(), operation_id)
            .await?;
        Ok(ObjectDelete {
            deleted: result.deleted,
            version: result.version,
        })
    }

    async fn read_capacity(
        &self,
        key: ObjectKey,
        options: ReadOptions,
    ) -> Result<Option<usize>, WorkerError> {
        let Some(stat) = self.stat(key).await? else {
            return Ok(None);
        };
        let length = match options.range {
            Some(range) if options.clamp_range && range.offset >= stat.length => 0,
            Some(range) if options.clamp_range => range.length.min(stat.length - range.offset),
            Some(range) => range.length,
            None => stat.length,
        };
        usize::try_from(length)
            .map(Some)
            .map_err(|_| WorkerError::ResourceExhausted)
    }
}

fn next_operation_id() -> Vec<u8> {
    let sequence = DATA_CORE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut id = Vec::with_capacity(24);
    id.extend_from_slice(DATA_CORE_INSTANCE_ID.get_or_init(data_core_instance_id));
    id.extend_from_slice(&sequence.to_be_bytes());
    id
}

fn data_core_instance_id() -> [u8; 16] {
    *Uuid::new_v4().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::next_operation_id;

    #[test]
    fn operation_id_uses_one_process_uuid_and_a_monotonic_sequence() {
        let first = next_operation_id();
        let second = next_operation_id();

        assert_eq!(first.len(), 24);
        assert_eq!(second.len(), 24);
        assert_eq!(first[..16], second[..16], "同一进程实例必须复用 UUID 前缀");
        assert_ne!(first[16..], second[16..], "每次操作必须使用不同序号");
    }
}

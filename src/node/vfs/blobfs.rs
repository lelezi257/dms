//! BlobFs：私有写入、显式发布、发布后不可变且多点读取的后端（当前返回 Unsupported，业务阶段补齐）。
//!
//! 单个私有版本写入阶段仅写者可见；runtime 显式请求稳定切点/发布，close/fsync
//! 不代表封存。发布后的版本不能被后续写入修改，未完成产物不能提前暴露。
//! 负责版本/内容表示、发布、副本与缓存、读者保护及回收等独立业务语义。
//! 共用 storage 的本地 I/O 和 peer 的传输，不共用 OwnerFs 的可修改文件状态机。
//! 不在此预先固定 extent/chunk 布局或把所有写回调映射成一个 Blob。

use super::{Backend, CreateRequest, Namespace};
use afs_error::{Error, ErrorKind, Result};

#[derive(Debug, Default)]
/// 当前是无状态的后端接入点，下面只实现日志和明确拒绝。
/// 模块头描述的是后续业务应遵守的边界，不是已经实现了这些业务能力。
pub struct BlobFs;

impl BlobFs {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Backend for BlobFs {
    fn namespace(&self) -> Namespace {
        Namespace::BlobFs
    }

    fn create(&self, request: &CreateRequest) -> Result<()> {
        afs_logging::info!("blobfs.create"; "namespace" => request.namespace.as_str(), "path" => request.name.as_str());
        Err(Error::new(
            ErrorKind::Unsupported,
            "BlobFs draft create is not implemented in the foundation skeleton",
        ))
    }
}

//! OwnerFs：可修改、以根归属和计算亲和为核心的文件后端（当前返回 Unsupported，业务阶段补齐）。
//!
//! 根目录是位置/授权管理单位；已有有效授权时根内操作复用授权，本地直接操作普通文件。
//! 跨节点经 peer 访问同一份数据；首次共享须完成原节点缓存/在途请求屏障和权威授权。
//! 负责文件身份、句柄、共享可见性与本后端恢复；不把每次写转换为不可变 Blob。
//! 共享不改变数据为 Blob，业务语义不因 gRPC/RDMA adapter 选择而改变。

use super::{Backend, CreateRequest, Namespace};
use afs_error::{Error, ErrorKind, Result};

#[derive(Debug, Default)]
/// 当前是无状态的后端接入点，下面只实现日志和明确拒绝。
/// 模块头描述的是后续业务应遵守的边界，不是已经实现了这些业务能力。
pub struct OwnerFs;

impl OwnerFs {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Backend for OwnerFs {
    fn namespace(&self) -> Namespace {
        Namespace::OwnerFs
    }

    fn create(&self, request: &CreateRequest) -> Result<()> {
        afs_logging::info!("ownerfs.create"; "namespace" => request.namespace.as_str(), "path" => request.name.as_str());
        Err(Error::new(
            ErrorKind::Unsupported,
            "OwnerFs create is not implemented in the foundation skeleton",
        ))
    }
}

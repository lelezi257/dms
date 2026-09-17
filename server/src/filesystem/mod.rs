//! Node 与 Meta 共用的文件系统领域合同。
//!
//! 本模块只描述文件系统跨进程边界两侧都必须理解的业务事实，例如 inode、目录项、
//! 文件内容绑定和一次原子文件提交。它不拥有状态，不启动 runtime，也不实现 FUSE。
//! `VersionLayout/Extent/Block` 仍由既有 DataCore/Meta 对象模型定义；这里仅在提交时
//! 携带现有 `VersionLayout`，不会建立第二套文件数据布局。

mod acl;
mod locks;
mod model;
pub(crate) mod space_sync;
mod wire;

pub(crate) use acl::*;
pub(crate) use locks::*;
pub(crate) use model::*;
pub(crate) use wire::*;

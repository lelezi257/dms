//! 文件后端共用的 POSIX 接入接口。
//!
//! 每个 FUSE session 在挂载时绑定一个 Backend。FUSE 节点号、持久文件身份与
//! 后端版本身份不能混为一谈；共用接口不意味着统一缓存、恢复或持久化模型。

#[cfg(feature = "dfs")]
pub mod dfs;
#[cfg(feature = "ownerfs")]
pub mod ownerfs;
pub mod types;

use std::ffi::OsStr;

use afs_error::{Error, Result};

/// 两种后端接收同一组文件操作；每项默认拒绝，未实现时不得报告成功。
///
/// 这里仅规定入口必须提供的参数与返回值。根授权、文件身份、缓存和持久化
/// 由具体后端决定。回调是同步的，以匹配当前 fuser 入口；后端可以用自己的
/// I/O 执行器，但不能在锁内无限等待远端或把未完成的操作假装完成。
pub trait Backend: Send + Sync {
    /// 当前 Backend 在本 mount 内的根 inode。它不是 FUSE inode，也不是跨进程身份。
    fn root_inode(&self) -> types::BackendInode;

    /// 仅在已选后端的父 inode 下查一个目录项。OwnerFs 应先由父 inode
    /// 找到 WorkspaceRoot，再校验其 RootGrant；不能按名字再查一次中心。
    fn lookup(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
    ) -> Result<types::Entry> {
        Err(unsupported("lookup"))
    }

    /// 传入 handle 时查询打开的旧对象；同名文件被删除重建后不能转向新文件。
    fn getattr(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
        _handle: Option<types::FileHandle>,
    ) -> Result<types::FileAttributes> {
        Err(unsupported("getattr"))
    }

    /// chmod/chown/truncate/时间更新共用一个可选字段结构；已打开 FD 的
    /// 属性更新优先按 handle 执行，不能依靠可能已变化的路径。
    fn setattr(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
        _handle: Option<types::FileHandle>,
        _change: &types::AttributeChange,
    ) -> Result<types::FileAttributes> {
        Err(unsupported("setattr"))
    }

    /// 一个 create 回调同时产生目录项与打开句柄；根首次创建涉及 Meta
    /// reserve/activate，是 OwnerFs 私有慢路径，不让 VFS 逐文件提交 Meta。
    fn create(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
        _mode: u32,
        _flags: i32,
    ) -> Result<types::CreatedFile> {
        Err(unsupported("create"))
    }

    /// 原始 Linux flags 由 FUSE 边缘验证后传入，后端仍必须执行权限/授权检查。
    /// O_TRUNC 必须在确认期望文件身份后生效。
    fn open(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
        _flags: i32,
    ) -> Result<types::FileHandle> {
        Err(unsupported("open"))
    }

    /// 返回写入调用者缓冲区的实际字节数；EOF 可返回 0，不补齐短读。
    fn read(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::FileHandle,
        _offset: u64,
        _out: &mut [u8],
    ) -> Result<usize> {
        Err(unsupported("read"))
    }

    /// 返回底层确认的实际字节数；不能把部分成功或未知结果报成整笔成功。
    /// O_APPEND 的末尾定位属于打开句柄语义，后端不能盲信传入 offset。
    fn write(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::FileHandle,
        _offset: u64,
        _data: &[u8],
    ) -> Result<usize> {
        Err(unsupported("write"))
    }

    /// 处理前序异步错误。后端可以采用更强的保守语义；显式耐久合同由 fsync 表达。
    fn flush(&self, _ctx: &types::RequestContext, _handle: types::FileHandle) -> Result<()> {
        Err(unsupported("flush"))
    }

    /// 只有显式调用才同步；DataOnly/Full 分别对应 fdatasync/fsync。
    fn fsync(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::FileHandle,
        _mode: types::SyncMode,
    ) -> Result<()> {
        Err(unsupported("fsync"))
    }

    /// 释放本进程的句柄；不能以 path 重新寻找被 rename/unlink 的文件。
    fn release(&self, _ctx: &types::RequestContext, _handle: types::FileHandle) -> Result<()> {
        Err(unsupported("release"))
    }

    /// 返回独立目录句柄，支持 rename/unlink 后旧目录引用及 fsyncdir。
    fn opendir(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
    ) -> Result<types::DirectoryHandle> {
        Err(unsupported("opendir"))
    }

    /// `cookie` 是上一次 DirectoryEntry.next_cookie；0 表示从头开始。
    fn readdir(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::DirectoryHandle,
        _cookie: u64,
        _max_entries: usize,
    ) -> Result<Vec<types::DirectoryEntry>> {
        Err(unsupported("readdir"))
    }

    /// 目录项持久化与文件内容持久化是两个边界；调用后端的目录同步。
    fn fsyncdir(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::DirectoryHandle,
        _mode: types::SyncMode,
    ) -> Result<()> {
        Err(unsupported("fsyncdir"))
    }

    fn releasedir(
        &self,
        _ctx: &types::RequestContext,
        _handle: types::DirectoryHandle,
    ) -> Result<()> {
        Err(unsupported("releasedir"))
    }

    /// 子目录继承其 WorkspaceRoot 的归属，不在 Meta 创建另一个 root。
    fn mkdir(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
        _mode: u32,
    ) -> Result<types::Entry> {
        Err(unsupported("mkdir"))
    }

    fn unlink(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
    ) -> Result<()> {
        Err(unsupported("unlink"))
    }

    fn rmdir(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
    ) -> Result<()> {
        Err(unsupported("rmdir"))
    }

    /// 同一后端内重命名；跨 workspace root 或跨 namespace 的策略由后端
    /// 明确拒绝，不能静默复制到另一个存储位置。
    fn rename(
        &self,
        _ctx: &types::RequestContext,
        _from_parent: types::BackendInode,
        _from_name: &OsStr,
        _to_parent: types::BackendInode,
        _to_name: &OsStr,
        _flags: types::RenameFlags,
    ) -> Result<()> {
        Err(unsupported("rename"))
    }

    fn symlink(
        &self,
        _ctx: &types::RequestContext,
        _parent: types::BackendInode,
        _name: &OsStr,
        _target: &OsStr,
    ) -> Result<types::Entry> {
        Err(unsupported("symlink"))
    }

    fn readlink(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
    ) -> Result<std::ffi::OsString> {
        Err(unsupported("readlink"))
    }

    fn link(
        &self,
        _ctx: &types::RequestContext,
        _inode: types::BackendInode,
        _new_parent: types::BackendInode,
        _name: &OsStr,
    ) -> Result<types::Entry> {
        Err(unsupported("link"))
    }
}

fn unsupported(operation: &str) -> Error {
    Error::coded(
        afs_error::NODE_VFS_UNIMPLEMENTED,
        format!("VFS backend operation '{operation}' is not implemented"),
    )
}

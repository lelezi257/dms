//! OwnerFs 的远端文件操作合同，供非 Home 节点按根授权转发到 Home。
//!
//! 本层接收业务类型，不依赖 Proto、gRPC 或 RDMA 类型。`node/rpc/peer.rs`
//! 负责 wire 转换；A 侧 `node/rpc/data/owner.rs` 校验授权后交给本机 OwnerFs。
//! 当前仅定义接口，尚无实现或可用的远端文件业务。

use std::ffi::{OsStr, OsString};

use afs_error::Result;

use super::{
    files::{FileIdentity, OwnerEntry, RemoteDirectory, RemoteFile},
    root::RootGrant,
};

use crate::node::vfs::types::{AttributeChange, FileAttributes, RenameFlags};

/// `create` 的远端结果必须同时返回目录项和打开句柄，匹配 FUSE/POSIX 的
/// 原子 create+open 语义。远端节点不能先 lookup 再 open 来拼凑结果，
/// 否则同名删除重建会让返回的 entry 与 handle 指向不同文件。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteCreatedFile {
    pub entry: OwnerEntry,
    pub file: RemoteFile,
}

/// Home 返回的目录项事实。访问节点必须在 `files.rs` 中把 `OwnerEntry.identity`
/// 映射成本进程 BackendInode；remote 层不能凭远端 identity 伪造本地 inode。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteDirectoryEntry {
    pub name: OsString,
    pub entry: OwnerEntry,
    pub next_cookie: u64,
}

/// 每次操作使用当前 RootGrant。位置查询不能充当授权；B 的 grant
/// 与 A 的 Home grant 可以并存，真正撤权或 Home 重启后旧 grant 才失效。
/// 实现不得持根锁等待网络，也不得自动重放
/// 结果未知的写入。同步签名匹配现有 VFS；RPC 适配须在非 Tokio 工作线程
/// 上安全等待异步网络调用。
pub trait RemoteFiles: Send + Sync {
    fn lookup(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_parent: Option<&FileIdentity>,
    ) -> Result<OwnerEntry>;
    fn getattr(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
        file: Option<&RemoteFile>,
    ) -> Result<OwnerEntry>;
    fn setattr(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
        file: Option<&RemoteFile>,
        change: &AttributeChange,
    ) -> Result<OwnerEntry>;
    fn create(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        flags: i32,
        mode: u32,
        expected_parent: &FileIdentity,
    ) -> Result<RemoteCreatedFile>;
    fn mkdir(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        mode: u32,
        expected_parent: &FileIdentity,
    ) -> Result<OwnerEntry>;
    fn unlink(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
        expected_parent: &FileIdentity,
    ) -> Result<()>;
    fn rmdir(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
        expected_parent: &FileIdentity,
    ) -> Result<()>;
    // Both parent and entry identities must travel with the two paths so Home
    // can reject a stale pathname before changing the namespace.
    #[allow(clippy::too_many_arguments)]
    fn rename(
        &self,
        grant: &RootGrant,
        old_path: &OsStr,
        new_path: &OsStr,
        expected_old_identity: Option<&FileIdentity>,
        expected_new_identity: Option<&FileIdentity>,
        expected_old_parent: &FileIdentity,
        expected_new_parent: &FileIdentity,
        flags: RenameFlags,
    ) -> Result<()>;

    /// Home 在 O_TRUNC 等副作用发生前校验 expected_identity，避免同名重建
    /// 后把调用者原本查到的另一个文件截断。
    fn open(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        flags: i32,
        expected_identity: Option<&FileIdentity>,
    ) -> Result<(RemoteFile, FileAttributes)>;
    fn readlink(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
    ) -> Result<Vec<u8>>;

    /// 打开后的读写凭 Home 签发的令牌，不再从可能已改变的路径重新寻址。
    /// 返回实际字节数；0 字节读取可表示 EOF，不补齐短读。
    fn read(
        &self,
        grant: &RootGrant,
        file: &RemoteFile,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize>;
    fn write(
        &self,
        grant: &RootGrant,
        file: &RemoteFile,
        offset: u64,
        data: &[u8],
    ) -> Result<usize>;

    /// flush 不提升为耐久承诺；显式 fsync 才由 Home 同步普通文件。
    fn flush(&self, grant: &RootGrant, file: &RemoteFile) -> Result<()>;
    fn fsync(&self, grant: &RootGrant, file: &RemoteFile, data_only: bool) -> Result<()>;
    fn release(&self, grant: &RootGrant, file: RemoteFile) -> Result<()>;

    /// 目录游标由 Home 签发，readdir 的 cookie 只能在同一打开会话里续用。
    fn opendir(
        &self,
        grant: &RootGrant,
        path: &OsStr,
        expected_identity: Option<&FileIdentity>,
    ) -> Result<RemoteDirectory>;
    fn readdir(
        &self,
        grant: &RootGrant,
        directory: &RemoteDirectory,
        cookie: u64,
        max_entries: usize,
    ) -> Result<Vec<RemoteDirectoryEntry>>;
    fn fsyncdir(
        &self,
        grant: &RootGrant,
        directory: &RemoteDirectory,
        data_only: bool,
    ) -> Result<()>;
    fn releasedir(&self, grant: &RootGrant, directory: RemoteDirectory) -> Result<()>;
}

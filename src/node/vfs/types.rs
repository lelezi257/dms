//! FUSE 与业务后端之间的文件操作合同；这里不保存 OwnerFs 或 BlobFs 的权威状态。
//!
//! 入口的 FUSE inode 只在一个挂载会话内有效。后端 inode/句柄也是 Node 进程内的
//! 不透明编号；OwnerFs 的可恢复文件身份和 BlobFs 的版本身份由各自后端另行保存，
//! 不能把下面的编号写进 Meta 充当持久身份。

use std::{ffi::OsString, time::SystemTime};

use super::Namespace;

/// 由入口认证后的调用者身份；FUSE 的 `Request` 不泄露给业务实现。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestContext {
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    /// 仅创建操作使用；后端仍须执行自己的权限检查。
    pub umask: u32,
}

/// 路由到某个后端的会话内 inode。后端必须拒绝 namespace 不匹配或已失效的
/// 编号；`value` 不能当作磁盘 inode、文件身份、Meta RootId 或 Blob 版本。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BackendInode {
    pub namespace: Namespace,
    pub value: u64,
}

/// 一次成功 open 后产生的进程内句柄，始终引用打开时的文件身份。
/// rename/unlink 后不能退化为按旧路径重新打开；daemon 重启后旧句柄失效。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileHandle(pub u64);

/// `opendir` 的游标身份，与普通文件句柄分开，避免误用 `fsyncdir`。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DirectoryHandle(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    Regular,
    Directory,
    Symlink,
}

/// 后端返回的 POSIX 可见属性；时间取自事实源，不从 FUSE 缓存推测。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAttributes {
    pub kind: FileKind,
    pub size: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub atime: SystemTime,
    pub mtime: SystemTime,
    pub ctime: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub inode: BackendInode,
    pub attributes: FileAttributes,
}

/// `create` 必须同时返回新目录项和已打开句柄，供 FUSE 原子回复。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedFile {
    pub entry: Entry,
    pub handle: FileHandle,
}

/// 目录 cookie 由后端产生；入口不以数组下标假装稳定的继续位置。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    /// Linux 文件名允许非 UTF-8 字节。
    pub name: OsString,
    pub inode: BackendInode,
    pub kind: FileKind,
    pub next_cookie: u64,
}

/// 只有显式出现的字段才修改；`handle` 优先于路径身份，保护旧 FD 语义。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AttributeChange {
    pub size: Option<u64>,
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub atime: Option<SystemTime>,
    pub mtime: Option<SystemTime>,
}

/// `DataOnly` 对应 fdatasync；`Full` 对应 fsync。
/// 两者都不能被普通 write/flush/close 自动代替。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncMode {
    DataOnly,
    Full,
}

/// 除 Linux `RENAME_NOREPLACE`/`RENAME_EXCHANGE` 外的标志必须显式拒绝。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenameFlags(pub u32);

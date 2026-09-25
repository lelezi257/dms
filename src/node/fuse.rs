//! 统一 POSIX/FUSE 入口。
//!
//! 接收内核回调，转换到 VFS 接口并映射 errno/回复；管理 FUSE 特有的请求与缓存通知。
//! 两种 namespace 通过同一入口分派，不能把 FUSE inode 编号直接当作后端持久文件身份。
//! 根授权与后端读写由 VFS/业务处理；不在每个回调中重新查询 Meta。
//! 不引入 bind 子挂载方案，不复制每种后端的文件业务。

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use afs_error::{Error, ErrorKind, Result};
use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, KernelConfig, MountOption, ReplyAttr,
    ReplyCreate, ReplyDirectory, ReplyEntry, Request,
};

use crate::node::vfs::{Namespace, Vfs};

// 当前只提供三个稳定的“挂载内虚拟目录号”：挂载根、OwnerFs 入口、BlobFs 入口。
// 它们不是磁盘 inode，更不是跨进程恢复用的文件身份；实际 inode/dentry/handle 表尚未实现。
const ROOT_INO: u64 = 1;
const OWNERFS_INO: u64 = 2;
const BLOBFS_INO: u64 = 3;
// 这里只控制 entry/attribute 缓存 TTL，不表示完整文件 page cache 一致性已解决。
const TTL: Duration = Duration::ZERO;

/// 建立真正的内核 FUSE 挂载，由 fuser 后台会话收取 POSIX 回调。
/// 返回值由 Node 持有；释放会话时卸载。没有 bind 子挂载，也不绕过 FUSE 访问后端。
pub fn mount(vfs: Arc<Vfs>, path: &Path) -> Result<BackgroundSession> {
    reject_existing_mount(path)?;
    let fs = AfsFuse::new(vfs);
    fuser::spawn_mount2(
        fs,
        path,
        &[MountOption::FSName("afs".into()), MountOption::NoAtime],
    )
    .map_err(Error::from)
}

fn reject_existing_mount(path: &Path) -> Result<()> {
    let target = path.canonicalize().map_err(Error::from)?;
    for mountpoint in current_mountpoints()? {
        if mountpoint
            .canonicalize()
            .is_ok_and(|mounted| mounted == target)
        {
            return Err(Error::new(
                ErrorKind::Conflict,
                format!("mount point '{}' is already mounted", path.display()),
            ));
        }
    }
    Ok(())
}

fn current_mountpoints() -> Result<Vec<PathBuf>> {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").map_err(Error::from)?;
    mountinfo
        .lines()
        .map(|line| {
            line.split_whitespace()
                .nth(4)
                .ok_or_else(|| Error::new(ErrorKind::Io, "malformed /proc/self/mountinfo line"))
                .and_then(decode_mountinfo_path)
        })
        .collect()
}

fn decode_mountinfo_path(encoded: &str) -> Result<PathBuf> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            if index + 3 >= bytes.len() {
                return Err(Error::new(
                    ErrorKind::Io,
                    format!("malformed mountinfo escape in '{encoded}'"),
                ));
            }
            let value = parse_octal(&bytes[index + 1..index + 4]).ok_or_else(|| {
                Error::new(
                    ErrorKind::Io,
                    format!("malformed mountinfo escape in '{encoded}'"),
                )
            })?;
            decoded.push(value);
            index += 4;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&decoded).into_owned(),
    ))
}

fn parse_octal(bytes: &[u8]) -> Option<u8> {
    bytes.iter().try_fold(0u8, |acc, byte| match byte {
        b'0'..=b'7' => acc.checked_mul(8)?.checked_add(byte - b'0'),
        _ => None,
    })
}

#[derive(Debug)]
pub struct AfsFuse {
    vfs: Arc<Vfs>,
}

impl AfsFuse {
    #[must_use]
    pub fn new(vfs: Arc<Vfs>) -> Self {
        Self { vfs }
    }

    fn namespace_for_ino(&self, ino: u64) -> Option<Namespace> {
        match ino {
            OWNERFS_INO if self.vfs.has_namespace(Namespace::OwnerFs) => Some(Namespace::OwnerFs),
            BLOBFS_INO if self.vfs.has_namespace(Namespace::BlobFs) => Some(Namespace::BlobFs),
            _ => None,
        }
    }

    fn lookup_root_child(&self, name: &OsStr) -> std::result::Result<FileAttr, i32> {
        let Some(name) = name.to_str() else {
            return Err(libc::EINVAL);
        };
        let namespace = Namespace::parse(name).map_err(errno)?;
        let Some(ino) = ino_for_namespace(namespace).filter(|_| self.vfs.has_namespace(namespace))
        else {
            return Err(libc::ENOENT);
        };
        Ok(dir_attr(ino))
    }

    /// 例如 touch /mnt/ownerfs/hello：parent=OWNERFS_INO，name=hello。
    /// 先映射 namespace，再调用同一 VFS 接口，最后把领域错误变成 POSIX errno。
    /// 真实 workspace 的 RootId/授权解析是下一步，不能用这里的 namespace 选择代替。
    fn create_namespace_file(&self, parent: u64, name: &OsStr) -> std::result::Result<(), i32> {
        let namespace = self.namespace_for_ino(parent).ok_or(libc::ENOENT)?;
        let name = name.to_str().ok_or(libc::EINVAL)?;
        self.vfs.create_file(namespace, name).map_err(errno)
    }
}

impl Filesystem for AfsFuse {
    fn init(&mut self, _: &Request<'_>, config: &mut KernelConfig) -> std::result::Result<(), i32> {
        let _ = config.set_max_write(1024 * 1024);
        Ok(())
    }

    fn lookup(&mut self, _: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let result = match parent {
            ROOT_INO => self.lookup_root_child(name),
            OWNERFS_INO | BLOBFS_INO => Err(libc::ENOENT),
            _ => Err(libc::ESTALE),
        };
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, 0),
            Err(error) => reply.error(error),
        }
    }

    fn getattr(&mut self, _: &Request<'_>, ino: u64, _: Option<u64>, reply: ReplyAttr) {
        let result = match ino {
            ROOT_INO => Ok(dir_attr(ROOT_INO)),
            OWNERFS_INO | BLOBFS_INO if self.namespace_for_ino(ino).is_some() => Ok(dir_attr(ino)),
            OWNERFS_INO | BLOBFS_INO => Err(libc::ENOENT),
            _ => Err(libc::ESTALE),
        };
        match result {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(error) => reply.error(error),
        }
    }

    fn readdir(
        &mut self,
        _: &Request<'_>,
        ino: u64,
        _: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let result = match ino {
            ROOT_INO => {
                add_entries(
                    &mut reply,
                    offset,
                    self.vfs.namespaces().into_iter().filter_map(|namespace| {
                        ino_for_namespace(namespace).map(|ino| (ino, namespace.as_str()))
                    }),
                );
                Ok(())
            }
            OWNERFS_INO | BLOBFS_INO if self.namespace_for_ino(ino).is_some() => {
                add_entries(&mut reply, offset, std::iter::empty());
                Ok(())
            }
            OWNERFS_INO | BLOBFS_INO => Err(libc::ENOENT),
            _ => Err(libc::ESTALE),
        };

        match result {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn create(
        &mut self,
        _: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _: u32,
        _: u32,
        _: i32,
        reply: ReplyCreate,
    ) {
        afs_logging::info!("fuse.create"; "parent" => parent, "name" => name.to_string_lossy().into_owned());
        match self.create_namespace_file(parent, name) {
            Ok(()) => unreachable!("foundation backends do not create files yet"),
            Err(error) => reply.error(error),
        }
    }

    // Linux 收到 create 的 ENOSYS 后可能改用 mknod；两入口必须保持相同分派和错误语义。
    fn mknod(
        &mut self,
        _: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _: u32,
        _: u32,
        reply: ReplyEntry,
    ) {
        afs_logging::info!(
            "fuse.mknod";
            "parent" => parent,
            "name" => name.to_string_lossy().into_owned(),
            "mode" => mode,
        );

        let file_type = mode & libc::S_IFMT;
        if file_type != libc::S_IFREG {
            reply.error(libc::ENOSYS);
            return;
        }

        match self.create_namespace_file(parent, name) {
            Ok(()) => unreachable!("foundation backends do not create files yet"),
            Err(error) => reply.error(error),
        }
    }
}

fn add_entries<'a>(
    reply: &mut ReplyDirectory,
    offset: i64,
    children: impl Iterator<Item = (u64, &'a str)>,
) {
    let mut entries = vec![
        (ROOT_INO, FileType::Directory, "."),
        (ROOT_INO, FileType::Directory, ".."),
    ];
    entries.extend(children.map(|(ino, name)| (ino, FileType::Directory, name)));

    for (index, (ino, kind, name)) in entries.into_iter().enumerate().skip(offset.max(0) as usize) {
        if reply.add(ino, (index + 1) as i64, kind, name) {
            break;
        }
    }
}

fn ino_for_namespace(namespace: Namespace) -> Option<u64> {
    match namespace {
        Namespace::OwnerFs => Some(OWNERFS_INO),
        Namespace::BlobFs => Some(BLOBFS_INO),
    }
}

fn dir_attr(ino: u64) -> FileAttr {
    FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: 0,
        gid: 0,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

fn errno(error: Error) -> i32 {
    match error.kind() {
        ErrorKind::InvalidArgument => libc::EINVAL,
        ErrorKind::Unsupported => libc::ENOSYS,
        ErrorKind::NotFound => libc::ENOENT,
        ErrorKind::Unavailable => libc::EHOSTUNREACH,
        ErrorKind::Io => libc::EIO,
        ErrorKind::Conflict => libc::EAGAIN,
    }
}

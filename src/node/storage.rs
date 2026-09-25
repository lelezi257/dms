//! Node 内可复用的本地 I/O 基础机制。
//!
//! OwnerFs 普通文件和 BlobFs 内容存储按需共用文件 I/O、缓冲池与同步等机制。
//! 不拥有根授权、不可变发布、目录事务或统一恢复状态机；这些由具体后端决定。
//! 不强制每次 write 都 sync，也不强制分片或 Blob 化；成功/耐久标准来自调用业务。
//! 先保持 Node 内模块，未出现实际外部调用者前不拆通用存储 crate。

use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{Read, Seek, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const MAX_TRANSFER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
/// 当前最小诊断存储：共同被节点数据 RPC 与本机 SDK Handler 调用。
/// 仅支持受限单文件名和有界范围 I/O；不解析根授权、不提供 POSIX inode/句柄语义。
pub struct Storage {
    root: Arc<PathBuf>,
}

#[derive(Debug)]
pub enum StorageError {
    BadName,
    TooLarge,
    Range,
    UnsafeFileType,
    Io(std::io::Error),
    Join(tokio::task::JoinError),
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName => formatter.write_str("name must be one trusted path component"),
            Self::TooLarge => formatter.write_str("transfer exceeds 1MiB"),
            Self::Range => formatter.write_str("requested range is outside the file"),
            Self::UnsafeFileType => formatter.write_str("target is not a regular file"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Join(error) => write!(formatter, "blocking storage task failed: {error}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<tokio::task::JoinError> for StorageError {
    fn from(value: tokio::task::JoinError) -> Self {
        Self::Join(value)
    }
}

impl Storage {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        fs::create_dir_all(path.as_ref())?;
        Ok(Self {
            root: Arc::new(path.as_ref().canonicalize()?),
        })
    }

    /// 读取精确长度；越过 EOF 返回错误，不补零也不返回 POSIX 式短读。
    /// 这是当前数据面诊断合同；未来完整文件 read 的语义要由业务层明确。
    pub async fn read(
        &self,
        name: &str,
        offset: u64,
        length: u32,
    ) -> Result<Vec<u8>, StorageError> {
        let length = usize::try_from(length).map_err(|_| StorageError::TooLarge)?;
        if length > MAX_TRANSFER_BYTES {
            return Err(StorageError::TooLarge);
        }
        let path = self.resolve(name)?;
        tokio::task::spawn_blocking(move || read_blocking(path, offset, length)).await?
    }

    /// 完成普通文件 write_all 后返回长度，不隐式 sync_data/fsync。
    /// 因此成功表示文件 I/O 完成，不承诺进程外的掉电耐久或 Blob 版本发布。
    pub async fn write(
        &self,
        name: &str,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<usize, StorageError> {
        if data.len() > MAX_TRANSFER_BYTES {
            return Err(StorageError::TooLarge);
        }
        offset
            .checked_add(u64::try_from(data.len()).map_err(|_| StorageError::Range)?)
            .ok_or(StorageError::Range)?;
        let path = self.resolve(name)?;
        // std::fs 是阻塞 API。放到 Tokio blocking pool，使慢盘等待不占异步 RPC 执行线程。
        tokio::task::spawn_blocking(move || write_blocking(path, offset, data)).await?
    }

    fn resolve(&self, name: &str) -> Result<PathBuf, StorageError> {
        if !is_simple_component(name) {
            return Err(StorageError::BadName);
        }
        Ok(self.root.join(name))
    }
}

fn read_blocking(path: PathBuf, offset: u64, length: usize) -> Result<Vec<u8>, StorageError> {
    let mut file = open_regular(&path, false)?;
    let end = offset
        .checked_add(u64::try_from(length).map_err(|_| StorageError::Range)?)
        .ok_or(StorageError::Range)?;
    if end > file.metadata()?.len() {
        return Err(StorageError::Range);
    }
    file.seek(std::io::SeekFrom::Start(offset))?;
    let mut data = vec![0; length];
    file.read_exact(&mut data)?;
    Ok(data)
}

fn write_blocking(path: PathBuf, offset: u64, data: Vec<u8>) -> Result<usize, StorageError> {
    let mut file = open_regular(&path, true)?;
    file.seek(std::io::SeekFrom::Start(offset))?;
    file.write_all(&data)?;
    Ok(data.len())
}

// 打开时拒绝符号链接，打开后检查实际 FD 类型；不靠检查路径再打开的两步检查防护。
fn open_regular(path: &Path, write: bool) -> Result<std::fs::File, StorageError> {
    let mut options = OpenOptions::new();
    options
        .read(!write)
        .write(write)
        .create(write)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(StorageError::UnsafeFileType);
    }
    Ok(file)
}

fn is_simple_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_and_reads_exact_requested_range() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::new(temp.path()).unwrap();

        assert_eq!(
            storage
                .write("alpha.bin", 0, b"abcdefgh".to_vec())
                .await
                .unwrap(),
            8
        );
        assert_eq!(storage.read("alpha.bin", 2, 4).await.unwrap(), b"cdef");
    }

    #[tokio::test]
    async fn rejects_paths_outside_single_component_scope() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::new(temp.path()).unwrap();

        assert!(matches!(
            storage.write("../escape", 0, b"no".to_vec()).await,
            Err(StorageError::BadName)
        ));
        assert!(matches!(
            storage.write("nested/file", 0, b"no".to_vec()).await,
            Err(StorageError::BadName)
        ));
    }

    #[tokio::test]
    async fn rejects_reads_past_end_without_padding() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::new(temp.path()).unwrap();

        storage
            .write("alpha.bin", 0, b"abcd".to_vec())
            .await
            .unwrap();

        assert!(matches!(
            storage.read("alpha.bin", 2, 4).await,
            Err(StorageError::Range)
        ));
    }
}

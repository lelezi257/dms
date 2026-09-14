//! 内核无关的文件语义壳。
//!
//! 这里故意不引入内核挂载依赖。未来真正的文件入口只负责把自身的文件标识
//! 转成这里的 path 和 byte range，再由本模块调用 DataCore。

#[cfg(all(target_os = "linux", feature = "fuse"))]
pub(crate) mod fuse;

use super::data_core::{
    ByteRange, DataCoreHandle, ObjectDelete, ObjectKey, ObjectRead, ObjectStat, ObjectWrite,
    RangeWrite, ReadOptions, VersionSelector,
};
use super::runtime::WorkerError;

#[derive(Clone)]
pub(crate) struct FileOperations {
    core: DataCoreHandle,
}

#[cfg_attr(
    not(any(test, feature = "fuse")),
    allow(dead_code, reason = "文件语义层由可选 FUSE 支持和回归测试使用")
)]
impl FileOperations {
    pub(crate) fn new(core: DataCoreHandle) -> Self {
        Self { core }
    }

    #[allow(
        dead_code,
        reason = "保留空文件语义合同；FUSE 热路径使用 create_with_contents 合并首批数据"
    )]
    pub(crate) async fn create(&self, path: impl AsRef<str>) -> Result<ObjectWrite, WorkerError> {
        self.core.put_if_absent(path_key(path)?, Vec::new()).await
    }

    /// 一次提交新文件及其首批内容。
    ///
    /// FUSE 的一次用户写入可能被内核拆成多个 `write` callback。文件入口先在
    /// open handle 内聚合这些片段，close/flush 时调用本接口，使 DataCore 只看到
    /// 一次“若不存在则创建完整对象”的提交；对象竞争仍由 Meta 的条件提交裁决。
    #[allow(dead_code, reason = "新文件聚合提交由可选 FUSE 支持使用")]
    pub(crate) async fn create_with_contents(
        &self,
        path: impl AsRef<str>,
        bytes: Vec<u8>,
    ) -> Result<ObjectWrite, WorkerError> {
        self.core.put_if_absent(path_key(path)?, bytes).await
    }

    pub(crate) async fn write(
        &self,
        path: impl AsRef<str>,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ObjectWrite, WorkerError> {
        let key = path_key(path)?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(WorkerError::InvalidArgument(
                "file write range overflows u64",
            ))?;
        for _ in 0..3 {
            let Some(stat) = self.core.stat(key.clone()).await? else {
                if offset != 0 {
                    return Err(WorkerError::InvalidArgument(
                        "cannot create sparse file through the minimal file operations",
                    ));
                }
                match self.core.put_if_absent(key.clone(), bytes.to_vec()).await {
                    Ok(result) => return Ok(result),
                    Err(error) if error.is_version_conflict() => continue,
                    Err(error) => return Err(error),
                }
            };
            if end <= stat.length {
                match self
                    .write_range_key(key.clone(), offset, bytes, stat.version)
                    .await
                {
                    Ok(result) => return Ok(result),
                    Err(error) if error.is_version_conflict() => continue,
                    Err(error) => return Err(error),
                }
            }

            // DataCore 的 range write 不表达文件扩容。文件层先按自身语义构造
            // 新 value，再带读到的版本做条件提交；若并发写抢先提交，就重新读取并
            // 最多重试两次，不能用无条件 put 覆盖对方已经成功的数据。
            // `stat` 给出当前权威版本。按 Exact 读取可以避免 CAS 冲突后
            // 再次命中本 Node 的旧 Current cache，否则会用同一旧版本无效重试。
            let current = self
                .core
                .read(
                    key.clone(),
                    ReadOptions {
                        version: VersionSelector::Exact(stat.version),
                        ..ReadOptions::default()
                    },
                )
                .await?
                .ok_or(WorkerError::NotFound)?;
            if end <= current.logical_length {
                match self
                    .write_range_key(key.clone(), offset, bytes, current.version)
                    .await
                {
                    Ok(result) => return Ok(result),
                    Err(error) if error.is_version_conflict() => continue,
                    Err(error) => return Err(error),
                }
            }
            let new_len = usize::try_from(end).map_err(|_| WorkerError::ResourceExhausted)?;
            let write_offset =
                usize::try_from(offset).map_err(|_| WorkerError::ResourceExhausted)?;
            let mut next = current.bytes;
            next.resize(new_len, 0);
            next[write_offset..write_offset + bytes.len()].copy_from_slice(bytes);
            match self
                .core
                .put_if_version(key.clone(), next, current.version)
                .await
            {
                Ok(result) => return Ok(result),
                Err(error) if error.is_version_conflict() => continue,
                Err(error) => return Err(error),
            }
        }
        Err(WorkerError::Conflict)
    }

    /// 把文件调整为 `length` 字节。缩短时丢弃尾部，扩展时补零。
    pub(crate) async fn truncate(
        &self,
        path: impl AsRef<str>,
        length: u64,
    ) -> Result<ObjectWrite, WorkerError> {
        let key = path_key(path)?;
        let target_len = usize::try_from(length).map_err(|_| WorkerError::ResourceExhausted)?;
        for _ in 0..3 {
            let stat = self
                .core
                .stat(key.clone())
                .await?
                .ok_or(WorkerError::NotFound)?;
            // 与扩容写一样，固定 `stat` 返回的版本，确保冲突后
            // 下一轮不会重复使用旧 Current cache。
            let current = self
                .core
                .read(
                    key.clone(),
                    ReadOptions {
                        version: VersionSelector::Exact(stat.version),
                        ..ReadOptions::default()
                    },
                )
                .await?
                .ok_or(WorkerError::NotFound)?;
            if current.logical_length == length {
                return Ok(ObjectWrite {
                    version: current.version,
                    length,
                });
            }
            let mut next = current.bytes;
            next.resize(target_len, 0);
            match self
                .core
                .put_if_version(key.clone(), next, current.version)
                .await
            {
                Ok(result) => return Ok(result),
                Err(error) if error.is_version_conflict() => continue,
                Err(error) => return Err(error),
            }
        }
        Err(WorkerError::Conflict)
    }

    #[allow(
        dead_code,
        reason = "保留显式文件 range 合同，普通 FUSE write 已内联使用"
    )]
    pub(crate) async fn write_range(
        &self,
        path: impl AsRef<str>,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ObjectWrite, WorkerError> {
        let key = path_key(path)?;
        let stat = self
            .core
            .stat(key.clone())
            .await?
            .ok_or(WorkerError::NotFound)?;
        self.write_range_key(key, offset, bytes, stat.version).await
    }

    async fn write_range_key(
        &self,
        key: ObjectKey,
        offset: u64,
        bytes: &[u8],
        expected_version: u64,
    ) -> Result<ObjectWrite, WorkerError> {
        self.core
            .write_range(
                key,
                RangeWrite {
                    offset,
                    bytes: bytes.to_vec(),
                    expected_version: Some(expected_version),
                },
            )
            .await
    }

    pub(crate) async fn read(
        &self,
        path: impl AsRef<str>,
        offset: u64,
        length: u64,
    ) -> Result<Option<ObjectRead>, WorkerError> {
        self.core
            .read(
                path_key(path)?,
                ReadOptions::current_range(ByteRange::new(offset, length)?),
            )
            .await
    }

    pub(crate) async fn delete(&self, path: impl AsRef<str>) -> Result<ObjectDelete, WorkerError> {
        self.core.delete(path_key(path)?).await
    }

    pub(crate) async fn stat(
        &self,
        path: impl AsRef<str>,
    ) -> Result<Option<ObjectStat>, WorkerError> {
        self.core.stat(path_key(path)?).await
    }
}

fn path_key(path: impl AsRef<str>) -> Result<ObjectKey, WorkerError> {
    ObjectKey::new(format!("fs:{}", path.as_ref()))
}

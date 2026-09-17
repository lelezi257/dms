//! M1.5 文件空间与同步合同的跨层领域值。
//!
//! 本文件只承载跨层必须使用同一含义的输入类型，不持有空间状态。真实实现分别落在
//! 现有 owner：FUSE 只解码 Linux 参数，
//! `SharedFileOperations` 编排文件语义，DataCore/Arena 管理物理空间，Meta 负责原子发布。
//! 这里不会引入第二套 Extent、空间分配器或状态 owner。

/// `fsync` 与 `fdatasync` 的语义差别。
///
/// 它只表达“本次屏障包含哪些状态”，不表达副本数或介质等级。实际持久性等级必须
/// 复用 DMS 已有 durability 配置；不能再定义一套只给 Filesystem 使用的策略。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileSyncMode {
    /// `fdatasync`：文件内容以及正确读取内容所必需的元数据（例如 size）。
    DataOnly,
    /// `fsync`：内容、size、时间、权限等该文件已经发布的全部元数据。
    DataAndMetadata,
}

/// 一个经过溢出校验的文件字节范围。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SpaceRange {
    offset: u64,
    length: u64,
}

impl SpaceRange {
    pub(crate) fn new(offset: u64, length: u64) -> Result<Self, SpaceSyncContractError> {
        if length == 0 {
            return Err(SpaceSyncContractError::EmptyRange);
        }
        offset
            .checked_add(length)
            .ok_or(SpaceSyncContractError::RangeOverflow)?;
        Ok(Self { offset, length })
    }

    pub(crate) fn offset(self) -> u64 {
        self.offset
    }

    pub(crate) fn length(self) -> u64 {
        self.length
    }
}

/// Filesystem 请求的逻辑空间变化。
///
/// `Preallocate` 表示未来写入不能再因该范围缺少空间而失败；它需要 DataCore 的独立
/// reservation，不能伪装成 HOLE 或全零 DATA Block。`PunchHole` 只改变逻辑布局，
/// 文件 size 必须保持不变。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileSpaceMutation {
    Preallocate { keep_size: bool },
    PunchHole,
}

/// FUSE 参数解码完成后交给文件业务层的请求。
///
/// Linux flag 只存在于 FUSE adapter；`SharedFileOperations` 以下只接收该领域值。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileSpaceMutationRequest {
    range: SpaceRange,
    mutation: FileSpaceMutation,
}

impl FileSpaceMutationRequest {
    pub(crate) const fn new(range: SpaceRange, mutation: FileSpaceMutation) -> Self {
        Self { range, mutation }
    }

    pub(crate) const fn range(self) -> SpaceRange {
        self.range
    }

    pub(crate) const fn mutation(self) -> FileSpaceMutation {
        self.mutation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpaceSyncContractError {
    EmptyRange,
    RangeOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_range_rejects_empty_and_overflowing_requests() {
        assert_eq!(
            SpaceRange::new(0, 0),
            Err(SpaceSyncContractError::EmptyRange)
        );
        assert_eq!(
            SpaceRange::new(u64::MAX, 1),
            Err(SpaceSyncContractError::RangeOverflow)
        );
        let range = SpaceRange::new(8, 4).expect("valid range");
        assert_eq!(range.offset(), 8);
        assert_eq!(range.length(), 4);
    }

    #[test]
    fn preallocation_and_hole_punching_are_distinct_requests() {
        let range = SpaceRange::new(4_096, 8_192).expect("valid range");
        assert_ne!(
            FileSpaceMutationRequest::new(
                range,
                FileSpaceMutation::Preallocate { keep_size: true },
            ),
            FileSpaceMutationRequest::new(range, FileSpaceMutation::PunchHole)
        );
        assert_eq!(
            FileSpaceMutationRequest::new(range, FileSpaceMutation::PunchHole).range(),
            range
        );
        assert_eq!(
            FileSpaceMutationRequest::new(range, FileSpaceMutation::PunchHole).mutation(),
            FileSpaceMutation::PunchHole
        );
    }

    #[test]
    fn fdatasync_and_fsync_do_not_collapse_into_one_boolean_below_fuse() {
        assert_ne!(FileSyncMode::DataOnly, FileSyncMode::DataAndMetadata);
    }
}

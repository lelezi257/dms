//! 文件系统稳定值类型。
//!
//! 这些类型不包含锁、连接或后台任务。Node 可以安全地缓存它们，Meta 可以把它们
//! 写入 journal；真正的唯一写 owner 仍分别是 `NodeState` 和 `MetaState`。

pub(crate) type InodeId = u64;
pub(crate) type InodeRevision = u64;
pub(crate) type DirectoryRevision = u64;
pub(crate) type FileHandleId = u64;

pub(crate) const ROOT_INODE: InodeId = 1;

/// POSIX namespace 中 inode 的种类；它不是 DataCore 对象类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InodeKind {
    RegularFile,
    Directory,
    SymbolicLink,
}

/// 一次文件读写的逻辑范围。
///
/// `FileRange` 是用户 `pread/pwrite` 参数的领域表达，不持久化 Block 映射。Node 在
/// 调用 DataCore 时把它转换成现有 `ByteRange`，因此 Filesystem 不会产生第二套
/// Extent。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileRange {
    pub(crate) offset: u64,
    pub(crate) length: u64,
}

#[cfg(test)]
impl FileRange {
    pub(crate) fn new(offset: u64, length: u64) -> Result<Self, FileContractError> {
        offset
            .checked_add(length)
            .ok_or(FileContractError::RangeOverflow)?;
        Ok(Self { offset, length })
    }

    pub(crate) fn end(self) -> u64 {
        // `new` 已经验证；字段只在 crate 内可见，所有构造点都应调用 `new`。
        self.offset + self.length
    }
}

/// 一个文件 inode 对 DataCore 不可变对象版本的精确引用。
///
/// rename 不改变它；hard link 的多个 dentry 通过同一 inode 共享它。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileContentBinding {
    pub(crate) object_key: Vec<u8>,
    pub(crate) exact_version: u64,
}

/// inode 的共享 POSIX 属性快照。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InodeAttributes {
    pub(crate) inode: InodeId,
    pub(crate) kind: InodeKind,
    pub(crate) mode: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) link_count: u32,
    pub(crate) size: u64,
    pub(crate) atime_unix_nanos: i64,
    pub(crate) mtime_unix_nanos: i64,
    pub(crate) ctime_unix_nanos: i64,
}

/// 发起 POSIX 属性修改的调用者身份。
///
/// Node 从可信 FUSE `Request` 读取它，Meta 在唯一 actor turn 中完成最终权限判断。
/// `pid` 当前只用于诊断与后续审计预留，不参与首版 owner/root 判定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FilesystemCaller {
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) pid: u32,
}

/// `utimens` 对单个时间字段的三态更新。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeUpdate {
    Omit,
    Now,
    Exact(i64),
}

/// 一次 inode 属性修改。
///
/// `ctime` 不由调用者设置；Meta 在同一条权威提交中自动更新。`size` 不属于本结构，
/// 它必须与内容版本通过 [`CommitFileVersionRequest`] 原子发布。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttributePatch {
    pub(crate) mode: Option<u32>,
    pub(crate) uid: Option<u32>,
    pub(crate) gid: Option<u32>,
    pub(crate) atime: TimeUpdate,
    pub(crate) mtime: TimeUpdate,
}

impl Default for AttributePatch {
    fn default() -> Self {
        Self {
            mode: None,
            uid: None,
            gid: None,
            atime: TimeUpdate::Omit,
            mtime: TimeUpdate::Omit,
        }
    }
}

impl AttributePatch {
    pub(crate) const fn is_empty(self) -> bool {
        self.mode.is_none()
            && self.uid.is_none()
            && self.gid.is_none()
            && matches!(self.atime, TimeUpdate::Omit)
            && matches!(self.mtime, TimeUpdate::Omit)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetAttributesRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) commit_sequence: u64,
    pub(crate) inode: InodeId,
    pub(crate) expected_inode_revision: InodeRevision,
    pub(crate) caller: FilesystemCaller,
    pub(crate) patch: AttributePatch,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AttributeMutationResult {
    pub(crate) resolved: ResolvedInode,
    pub(crate) invalidation_cursor: u64,
    pub(crate) commit_index: u64,
}

/// xattr set 的存在性约束，对应 Linux `XATTR_CREATE/XATTR_REPLACE`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XattrSetMode {
    Upsert,
    CreateOnly,
    ReplaceOnly,
}

/// 一次 xattr 写入的领域请求。Node 在本地生成 operation id，并用 inode revision
/// 做 CAS；Meta 在单个 owner turn 内完成权限校验、ACL 联动和 WAL 提交。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetXattrRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) inode: InodeId,
    pub(crate) expected_inode_revision: InodeRevision,
    pub(crate) caller: FilesystemCaller,
    pub(crate) name: Vec<u8>,
    pub(crate) value: Vec<u8>,
    pub(crate) mode: XattrSetMode,
}

/// 一次 xattr 删除的领域请求。删除不存在属性由 Meta 返回稳定的 NotFound 错误，
/// FUSE 边界再映射为 `ENODATA`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoveXattrRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) inode: InodeId,
    pub(crate) expected_inode_revision: InodeRevision,
    pub(crate) caller: FilesystemCaller,
    pub(crate) name: Vec<u8>,
}

/// 一次持久化的 xattr delta。`None` 表示删除；它与 inode revision/grant 更新
/// 位于同一条 WAL 记录中，恢复时不会出现属性已经变化但 xattr 尚未变化的中间态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct XattrUpdate {
    pub(crate) inode: InodeId,
    pub(crate) name: Vec<u8>,
    pub(crate) value: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FilesystemStats {
    pub(crate) block_size: u64,
    pub(crate) total_blocks: u64,
    pub(crate) free_blocks: u64,
    pub(crate) available_blocks: u64,
    pub(crate) total_inodes: u64,
    pub(crate) free_inodes: u64,
    pub(crate) max_name_length: u32,
    pub(crate) reporting_nodes: u32,
    pub(crate) capacity_revision: u64,
}

/// 一次一致的 inode 读取结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InodeSnapshot {
    pub(crate) revision: InodeRevision,
    pub(crate) attributes: InodeAttributes,
    /// 目录没有内容对象；普通文件和符号链接在首次发布后拥有精确版本绑定。
    pub(crate) content: Option<FileContentBinding>,
    /// 尚未物化为 DATA Block 的容量保证。它不参与读布局；只有写入、打洞、
    /// truncate 与 Node incarnation 回收会修改它。
    pub(crate) reservations: Vec<FileSpaceReservation>,
}

/// 一段由特定 Node incarnation 在 Arena 中兑现的文件空间预留。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileSpaceReservation {
    pub(crate) reservation_id: Vec<u8>,
    pub(crate) node_id: u64,
    pub(crate) node_epoch: u64,
    pub(crate) offset: u64,
    pub(crate) length: u64,
}

/// 文件版本提交对 reservation 做的一段增加或扣减。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReservationRangeChange {
    pub(crate) reservation_id: Vec<u8>,
    pub(crate) offset: u64,
    pub(crate) length: u64,
}

/// 父目录中的一个名字到 inode 的映射。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DentrySnapshot {
    pub(crate) parent: InodeId,
    pub(crate) name: Vec<u8>,
    pub(crate) inode: InodeId,
    pub(crate) directory_revision: DirectoryRevision,
}

/// 对一个目录内容快照的短期授权。
///
/// `directory_revision` 直接等于目录 inode revision。`grant` 负责乱序失效与 Watch
/// 断流后的租约上界；两者一起决定 Node 能否继续复用 lookup/readdir 结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DirectoryGrant {
    pub(crate) directory_revision: DirectoryRevision,
    pub(crate) grant: CacheGrant,
}

/// `readdir` 返回的一项。目录枚举只需要 dentry 和 inode 属性，不携带文件内容布局。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectoryEntry {
    pub(crate) dentry: DentrySnapshot,
    pub(crate) attributes: InodeAttributes,
}

/// Meta 的稳定目录分页结果。
///
/// `next_cursor` 是本页最后一个名字；后续页必须携带同一个 `directory_revision`。
/// 若并发 mutation 改变 revision，Meta 拒绝旧游标，Node 从第一页重读。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectoryPage {
    pub(crate) directory: InodeId,
    pub(crate) parent: InodeId,
    pub(crate) grant: DirectoryGrant,
    pub(crate) entries: Vec<DirectoryEntry>,
    pub(crate) next_cursor: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RemoveKind {
    File,
    Directory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RenameEntryRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) commit_sequence: u64,
    pub(crate) source_parent: InodeId,
    pub(crate) source_name: Vec<u8>,
    pub(crate) target_parent: InodeId,
    pub(crate) target_name: Vec<u8>,
    pub(crate) expected_source_revision: Option<DirectoryRevision>,
    pub(crate) expected_target_revision: Option<DirectoryRevision>,
    pub(crate) replace_existing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LinkEntryRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) commit_sequence: u64,
    pub(crate) source_inode: InodeId,
    pub(crate) target_parent: InodeId,
    pub(crate) target_name: Vec<u8>,
    pub(crate) expected_target_revision: Option<DirectoryRevision>,
    pub(crate) reference_generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CreateSymlinkRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) commit_sequence: u64,
    pub(crate) parent: InodeId,
    pub(crate) name: Vec<u8>,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) expected_parent_revision: Option<DirectoryRevision>,
    pub(crate) prepared: super::wire::PreparedObjectVersion,
    pub(crate) target_size: u64,
    pub(crate) mtime_unix_nanos: i64,
    pub(crate) reference_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoveEntryRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) operation_digest: Vec<u8>,
    pub(crate) commit_sequence: u64,
    pub(crate) parent: InodeId,
    pub(crate) name: Vec<u8>,
    pub(crate) kind: RemoveKind,
    pub(crate) expected_parent_revision: Option<DirectoryRevision>,
}

/// 一次 namespace mutation 对受影响目录产生的新水位。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DirectoryVersion {
    pub(crate) inode: InodeId,
    pub(crate) revision: DirectoryRevision,
    pub(crate) grant_generation: u64,
}

/// inode 属性或内容 binding 的新水位。
///
/// 目录失效只说明“某个父目录的名字集合变了”；hardlink/unlink/rename replace 还会改变
/// 被引用 inode 的 `link_count` 或内容授权。Node 必须单独按 inode 清 binding cache，
/// 否则热 `getattr`/read 可能继续看到旧 nlink。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InodeVersion {
    pub(crate) inode: InodeId,
    pub(crate) revision: InodeRevision,
    pub(crate) grant_generation: u64,
}

/// create/mkdir/rename/unlink/rmdir 的统一结果形状。
///
/// `inode` 是被创建、移动或移除名字所指向的 inode 快照。unlink/rmdir 后 inode
/// 可能已经 `link_count=0`，但为保护跨 Node open handle，M1 不立即回收其内容。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NamespaceMutationResult {
    pub(crate) dentry: Option<DentrySnapshot>,
    pub(crate) inode: Option<InodeSnapshot>,
    pub(crate) changed_directories: Vec<DirectoryVersion>,
    pub(crate) changed_inodes: Vec<InodeVersion>,
    pub(crate) invalidation_cursor: u64,
    pub(crate) commit_index: u64,
    /// 返回 entry 的 mutation 在 Meta 同一 actor turn 中建立的 nlookup=1 租约。
    /// 非 entry-returning mutation 保持 0。
    pub(crate) entry_reference_lease_millis: u64,
    pub(crate) entry_reference_generation: u64,
}

/// Meta 对某个 inode 内容绑定授予的短期缓存资格。
///
/// Node 从收到响应的时刻计算本地单调 deadline。`lease_millis` 不是让读者容忍旧值；
/// revoke 事件到达时必须先删除缓存，再 ACK。租约只负责断流后的有界收敛。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CacheGrant {
    pub(crate) generation: u64,
    pub(crate) lease_millis: u64,
}

/// Node 一次解析同时得到 inode 快照与缓存授权。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrantedInode {
    pub(crate) inode: InodeSnapshot,
    pub(crate) grant: CacheGrant,
}

/// 一次 Meta 解析返回的完整文件读取授权。
///
/// `object` 与 inode 的 `exact_version` 来自同一个 MetaState 读取点。Node 把整份
/// 结果放入 BindingCache，后续热读可直接交给 DataCore，不再额外 ResolveObject。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedInode {
    pub(crate) granted: GrantedInode,
    /// 对象解析计划是一个不透明领域值；protobuf 只在 `wire` 适配文件中出现。
    pub(crate) object: Option<super::wire::ResolvedObject>,
}

/// `pwrite/truncate` 的唯一发布请求。
///
/// `expected_inode_revision` 与 `expected_object_version` 同时做 CAS。成功时对象版本、
/// inode 精确绑定、size/mtime 与失效事件共用一个 Meta journal 序号。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CommitFileVersionRequest {
    pub(crate) operation_id: Vec<u8>,
    pub(crate) inode: InodeId,
    pub(crate) expected_inode_revision: InodeRevision,
    pub(crate) prepared: super::wire::PreparedObjectVersion,
    pub(crate) new_size: u64,
    pub(crate) mtime_unix_nanos: i64,
    /// 只有内容提交同时携带 chmod/chown/utimens 时才存在。调用者身份与 patch
    /// 必须成对出现，由 Meta 在同一个 actor turn 内完成权限检查和原子发布。
    pub(crate) caller: Option<FilesystemCaller>,
    pub(crate) attribute_patch: AttributePatch,
    pub(crate) reservation_additions: Vec<ReservationRangeChange>,
    pub(crate) reservation_reductions: Vec<ReservationRangeChange>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CommitFileVersionResult {
    /// 与本次原子发布来自同一个 MetaState 读取点的完整结果。写入 Node 必须用它
    /// 回填 BindingCache；该 Node 不会收到发给“其他 Node”的 revoke 事件。
    pub(crate) resolved: ResolvedInode,
    pub(crate) invalidation_cursor: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileContractError {
    #[cfg(test)]
    RangeOverflow,
    MissingInodeAttributes,
    MissingResolvedInode,
    InvalidInodeKind,
    InvalidAttributePatch,
    InvalidXattrSetMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_range_is_an_operation_range_not_a_layout() {
        let range = FileRange::new(4_096, 3).expect("valid pwrite range");
        assert_eq!(range.end(), 4_099);
        assert_eq!(
            FileRange::new(u64::MAX, 1),
            Err(FileContractError::RangeOverflow)
        );
    }

    #[test]
    fn rename_can_keep_the_same_content_binding() {
        let binding = FileContentBinding {
            object_key: b"fs/content/100".to_vec(),
            exact_version: 7,
        };
        let before = DentrySnapshot {
            parent: ROOT_INODE,
            name: b"before".to_vec(),
            inode: 100,
            directory_revision: 10,
        };
        let after = DentrySnapshot {
            name: b"after".to_vec(),
            directory_revision: 11,
            ..before.clone()
        };

        assert_eq!(before.inode, after.inode);
        assert_eq!(binding.object_key, b"fs/content/100");
    }
}

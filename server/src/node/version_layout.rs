//! Node 使用的不可变 value 布局算法。
//!
//! `VersionLayout` 用有序 Extent 描述一个逻辑 value。DATA Extent 把逻辑范围映射到
//! immutable Block 的一段；HOLE Extent 表达 sparse 零区间，但没有 Block、副本、
//! Allocation、摘要或传输。本模块只负责无状态布局计算，不持有 payload bytes 或元数据。

use dms_protocol::v1 as pb;

use super::metadata_client::digest as digest_bytes;
use super::runtime::WorkerError;

/// Sparse 文件洞的显式协议语义。
///
/// 旧消息没有 `kind` 字段，因此 `UNSPECIFIED` 仍按 DATA 解释；只有明确标记为
/// `HOLE` 的 Extent 才能省略 Block。这样读/校验/Meta proof 不再依赖
/// “block_id 为空”这种隐式哨兵。
pub(crate) fn is_hole(extent: &pb::ExtentRecord) -> bool {
    extent.kind == pb::ExtentKind::Hole as i32
}

fn is_data_kind(kind: i32) -> bool {
    kind == pb::ExtentKind::Unspecified as i32 || kind == pb::ExtentKind::Data as i32
}

fn data_extent(
    offset: u64,
    length: u64,
    block_id: Vec<u8>,
    block_offset: u64,
    digest: Vec<u8>,
) -> pb::ExtentRecord {
    pb::ExtentRecord {
        logical: Some(pb::ByteRange { offset, length }),
        block_id,
        block_offset,
        digest,
        kind: pb::ExtentKind::Data as i32,
    }
}

fn hole_extent(offset: u64, length: u64) -> pb::ExtentRecord {
    pb::ExtentRecord {
        logical: Some(pb::ByteRange { offset, length }),
        block_id: Vec::new(),
        block_offset: 0,
        digest: Vec::new(),
        kind: pb::ExtentKind::Hole as i32,
    }
}

/// 校验 Extent 是否按顺序完整覆盖 `[0, logical_length)`。
pub(crate) fn validate(
    logical_length: u64,
    extents: &[pb::ExtentRecord],
) -> Result<(), WorkerError> {
    let mut cursor = 0_u64;
    for extent in extents {
        let logical = extent.logical.as_ref().ok_or(WorkerError::InvalidArgument(
            "extent logical range is missing",
        ))?;
        if logical.length == 0 || logical.offset != cursor {
            return Err(WorkerError::InvalidArgument(
                "version layout contains a gap, overlap, or empty extent",
            ));
        }
        cursor = cursor
            .checked_add(logical.length)
            .ok_or(WorkerError::InvalidArgument("extent range overflows u64"))?;
        if is_hole(extent) {
            if extent.block_offset != 0 {
                return Err(WorkerError::InvalidArgument(
                    "sparse hole must not point into a block",
                ));
            }
            if !extent.block_id.is_empty() || !extent.digest.is_empty() {
                return Err(WorkerError::InvalidArgument(
                    "sparse hole must not carry block identity or digest",
                ));
            }
            continue;
        }
        if !is_data_kind(extent.kind) {
            return Err(WorkerError::InvalidArgument("unknown extent kind"));
        }
        if extent.block_id.is_empty() {
            return Err(WorkerError::InvalidArgument(
                "data extent requires a block identity",
            ));
        }
        extent
            .block_offset
            .checked_add(logical.length)
            .ok_or(WorkerError::InvalidArgument("block range overflows u64"))?;
    }
    if cursor != logical_length {
        return Err(WorkerError::InvalidArgument(
            "version layout does not cover its logical length",
        ));
    }
    Ok(())
}

/// 构造 `SET_RANGE` 布局，不复制未修改的 base bytes。
pub(crate) fn overlay(
    base: &[pb::ExtentRecord],
    logical_length: u64,
    patch_start: u64,
    patch_length: u64,
    patch_block: &[u8],
    patch_digest: &[u8],
) -> Result<Vec<pb::ExtentRecord>, WorkerError> {
    validate(logical_length, base)?;
    if patch_length == 0 {
        return Err(WorkerError::InvalidArgument(
            "range patch must not be empty",
        ));
    }
    let patch_end = patch_start
        .checked_add(patch_length)
        .ok_or(WorkerError::InvalidArgument("range patch overflows u64"))?;
    if patch_end > logical_length {
        return Err(WorkerError::InvalidArgument(
            "range patch extends beyond the base value",
        ));
    }

    overlay_with_length(
        base,
        logical_length,
        logical_length,
        patch_start,
        patch_length,
        patch_block,
        patch_digest,
    )
}

/// 构造文件写的 Extent overlay，并允许写入从当前 EOF 之外继续向后扩展。
///
/// 普通 KV `SET_RANGE` 不能改变 value 长度；文件 `write(2)` 可以覆盖旧范围并
/// 越过 EOF。未覆盖的旧 Extent 继续复用，新写入只形成一个 immutable Block，
/// 不能为了扩容把整个旧文件读回并重新提交。若写入起点超过 EOF，中间空洞用
/// sparse hole Extent 表示，读路径填零，但不会分配或传输零 Block。
pub(crate) fn overlay_file_write(
    base: &[pb::ExtentRecord],
    logical_length: u64,
    patch_start: u64,
    patch_length: u64,
    patch_block: &[u8],
    patch_digest: &[u8],
) -> Result<Vec<pb::ExtentRecord>, WorkerError> {
    validate(logical_length, base)?;
    if patch_length == 0 {
        return Err(WorkerError::InvalidArgument(
            "file write patch must not be empty",
        ));
    }
    let patch_end = patch_start
        .checked_add(patch_length)
        .ok_or(WorkerError::InvalidArgument(
            "file write patch overflows u64",
        ))?;
    overlay_with_length(
        base,
        logical_length,
        logical_length.max(patch_end),
        patch_start,
        patch_length,
        patch_block,
        patch_digest,
    )
}

/// 构造文件 truncate 的新 Extent 布局。
///
/// 缩短文件只改变逻辑视图：保留仍被新文件引用的 Extent 前缀，必要时裁剪最后一个
/// Extent 的逻辑长度；不会读回或复制 Block bytes。扩展文件只追加 sparse hole，
/// 读路径按 POSIX 语义返回零，不分配全零 Block。
pub(crate) fn truncate_file(
    base: &[pb::ExtentRecord],
    logical_length: u64,
    next_logical_length: u64,
) -> Result<Vec<pb::ExtentRecord>, WorkerError> {
    validate(logical_length, base)?;
    if next_logical_length == logical_length {
        return Ok(base.to_vec());
    }
    if next_logical_length > logical_length {
        let mut next = base.to_vec();
        next.push(hole_extent(
            logical_length,
            next_logical_length - logical_length,
        ));
        return compact_and_validate(next_logical_length, next);
    }

    let mut next = Vec::new();
    for extent in base {
        let logical = extent.logical.as_ref().expect("layout was validated above");
        if logical.offset >= next_logical_length {
            break;
        }
        let kept_end = logical
            .offset
            .checked_add(logical.length)
            .ok_or(WorkerError::InvalidArgument("extent range overflows u64"))?
            .min(next_logical_length);
        next.push(slice_extent(
            extent,
            logical.offset,
            kept_end - logical.offset,
            0,
        )?);
    }
    compact_and_validate(next_logical_length, next)
}

/// 把文件现有范围替换为 sparse HOLE，同时保持文件长度不变。
///
/// Linux `FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE` 允许请求尾部越过 EOF；只有
/// 与 `[0, logical_length)` 的交集参与布局更新。被覆盖 DATA Extent 的未修改前后缀
/// 继续引用原 immutable Block，不读回、不复制 payload；HOLE 不创建零 Block。
pub(crate) fn punch_hole_file(
    base: &[pb::ExtentRecord],
    logical_length: u64,
    offset: u64,
    length: u64,
) -> Result<Vec<pb::ExtentRecord>, WorkerError> {
    validate(logical_length, base)?;
    if length == 0 {
        return Err(WorkerError::InvalidArgument(
            "hole punch range must not be empty",
        ));
    }
    let requested_end = offset
        .checked_add(length)
        .ok_or(WorkerError::InvalidArgument(
            "hole punch range overflows u64",
        ))?;
    if offset >= logical_length {
        return Ok(base.to_vec());
    }
    let punch_end = requested_end.min(logical_length);
    let mut next = Vec::with_capacity(base.len() + 2);
    for extent in base {
        let logical = extent.logical.as_ref().expect("layout was validated above");
        let extent_end = logical.offset + logical.length;
        if extent_end <= offset || logical.offset >= punch_end {
            next.push(extent.clone());
            continue;
        }
        if logical.offset < offset {
            next.push(slice_extent(
                extent,
                logical.offset,
                offset - logical.offset,
                0,
            )?);
        }
        if extent_end > punch_end {
            next.push(slice_extent(
                extent,
                punch_end,
                extent_end - punch_end,
                punch_end - logical.offset,
            )?);
        }
    }
    next.push(hole_extent(offset, punch_end - offset));
    next.sort_by_key(|extent| extent.logical.as_ref().map_or(0, |range| range.offset));
    compact_and_validate(logical_length, next)
}

fn overlay_with_length(
    base: &[pb::ExtentRecord],
    base_logical_length: u64,
    next_logical_length: u64,
    patch_start: u64,
    patch_length: u64,
    patch_block: &[u8],
    patch_digest: &[u8],
) -> Result<Vec<pb::ExtentRecord>, WorkerError> {
    let patch_end = patch_start
        .checked_add(patch_length)
        .ok_or(WorkerError::InvalidArgument("range patch overflows u64"))?;

    let mut next = Vec::with_capacity(base.len() + 3);
    for extent in base {
        let logical = extent.logical.as_ref().expect("layout was validated above");
        let extent_end = logical.offset + logical.length;
        if extent_end <= patch_start || logical.offset >= patch_end {
            next.push(extent.clone());
            continue;
        }
        if logical.offset < patch_start {
            next.push(slice_extent(
                extent,
                logical.offset,
                patch_start - logical.offset,
                0,
            )?);
        }
        if extent_end > patch_end {
            next.push(slice_extent(
                extent,
                patch_end,
                extent_end - patch_end,
                patch_end - logical.offset,
            )?);
        }
    }
    next.push(data_extent(
        patch_start,
        patch_length,
        patch_block.to_vec(),
        0,
        patch_digest.to_vec(),
    ));
    if patch_start > base_logical_length {
        next.push(hole_extent(
            base_logical_length,
            patch_start - base_logical_length,
        ));
    }
    next.sort_by_key(|extent| extent.logical.as_ref().map_or(0, |range| range.offset));
    compact_and_validate(next_logical_length, next)
}

fn slice_extent(
    extent: &pb::ExtentRecord,
    logical_offset: u64,
    logical_length: u64,
    block_delta: u64,
) -> Result<pb::ExtentRecord, WorkerError> {
    if is_hole(extent) {
        return Ok(hole_extent(logical_offset, logical_length));
    }
    Ok(data_extent(
        logical_offset,
        logical_length,
        extent.block_id.clone(),
        extent
            .block_offset
            .checked_add(block_delta)
            .ok_or(WorkerError::InvalidArgument("block range overflows u64"))?,
        extent.digest.clone(),
    ))
}

fn compact_and_validate(
    logical_length: u64,
    extents: Vec<pb::ExtentRecord>,
) -> Result<Vec<pb::ExtentRecord>, WorkerError> {
    let mut compacted: Vec<pb::ExtentRecord> = Vec::with_capacity(extents.len());
    for extent in extents {
        if let Some(previous) = compacted.last_mut() {
            let can_merge_holes = is_hole(previous) && is_hole(&extent) && {
                let previous_logical = previous
                    .logical
                    .as_ref()
                    .expect("previous extent was produced with logical range");
                let logical = extent
                    .logical
                    .as_ref()
                    .expect("new extent was produced with logical range");
                previous_logical
                    .offset
                    .checked_add(previous_logical.length)
                    .is_some_and(|end| end == logical.offset)
            };
            if can_merge_holes {
                let additional = extent
                    .logical
                    .as_ref()
                    .expect("new extent was produced with logical range")
                    .length;
                let previous_logical = previous
                    .logical
                    .as_mut()
                    .expect("previous extent was produced with logical range");
                previous_logical.length = previous_logical
                    .length
                    .checked_add(additional)
                    .ok_or(WorkerError::InvalidArgument("extent range overflows u64"))?;
                continue;
            }
        }
        compacted.push(extent);
    }
    validate(logical_length, &compacted)?;
    Ok(compacted)
}

/// 计算布局元数据摘要；每个 immutable Block 仍保留自己的 payload 摘要。
pub(crate) fn digest(logical_length: u64, extents: &[pb::ExtentRecord]) -> Vec<u8> {
    let mut bytes = logical_length.to_be_bytes().to_vec();
    for extent in extents {
        if let Some(logical) = &extent.logical {
            bytes.extend_from_slice(&logical.offset.to_be_bytes());
            bytes.extend_from_slice(&logical.length.to_be_bytes());
        }
        let effective_kind = if is_hole(extent) { 2_u8 } else { 1_u8 };
        bytes.push(effective_kind);
        bytes.extend_from_slice(&(extent.block_id.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&extent.block_id);
        bytes.extend_from_slice(&extent.block_offset.to_be_bytes());
        bytes.extend_from_slice(&extent.digest);
    }
    digest_bytes(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(start: u64, length: u64, block: &[u8], block_offset: u64) -> pb::ExtentRecord {
        data_extent(start, length, block.to_vec(), block_offset, block.to_vec())
    }

    fn patch_extent(start: u64, length: u64, block: &[u8], digest: &[u8]) -> pb::ExtentRecord {
        data_extent(start, length, block.to_vec(), 0, digest.to_vec())
    }

    #[test]
    fn one_byte_patch_reuses_the_base_block_on_both_sides() {
        let next =
            overlay(&[extent(0, 6, b"base", 0)], 6, 2, 1, b"patch", b"p").expect("valid overlay");
        assert_eq!(
            next,
            vec![
                extent(0, 2, b"base", 0),
                patch_extent(2, 1, b"patch", b"p"),
                extent(3, 3, b"base", 3),
            ]
        );
    }

    #[test]
    fn rejects_gap_and_out_of_range_patch() {
        assert!(validate(3, &[extent(1, 2, b"base", 0)]).is_err());
        assert!(overlay(&[extent(0, 3, b"base", 0)], 3, 3, 1, b"patch", b"p").is_err());
    }

    #[test]
    fn file_append_reuses_base_and_adds_one_extent() {
        let next = overlay_file_write(&[extent(0, 3, b"base", 0)], 3, 3, 2, b"tail", b"t")
            .expect("append at EOF");
        assert_eq!(
            next,
            vec![extent(0, 3, b"base", 0), patch_extent(3, 2, b"tail", b"t"),]
        );
    }

    #[test]
    fn file_write_can_replace_suffix_and_extend() {
        let next = overlay_file_write(&[extent(0, 6, b"base", 0)], 6, 4, 4, b"tail", b"t")
            .expect("replace suffix and extend");
        assert_eq!(
            next,
            vec![extent(0, 4, b"base", 0), patch_extent(4, 4, b"tail", b"t"),]
        );
    }

    #[test]
    fn truncate_file_clips_suffix_without_copying_blocks() {
        let next = truncate_file(&[extent(0, 8, b"base", 2)], 8, 5).expect("truncate suffix");
        assert_eq!(next, vec![extent(0, 5, b"base", 2)]);
    }

    #[test]
    fn truncate_file_keeps_only_referenced_extents() {
        let next = truncate_file(&[extent(0, 4, b"left", 0), extent(4, 4, b"right", 0)], 8, 4)
            .expect("truncate to extent boundary");
        assert_eq!(next, vec![extent(0, 4, b"left", 0)]);
    }

    #[test]
    fn truncate_file_allows_empty_layout() {
        let next = truncate_file(&[extent(0, 8, b"base", 0)], 8, 0).expect("truncate empty");
        assert!(next.is_empty());
        validate(0, &next).expect("empty file layout is valid");
    }

    #[test]
    fn file_write_beyond_eof_uses_sparse_hole_without_zero_block() {
        let next = overlay_file_write(&[extent(0, 3, b"base", 0)], 3, 5, 2, b"data", b"d")
            .expect("sparse pwrite");
        assert_eq!(
            next,
            vec![
                extent(0, 3, b"base", 0),
                hole_extent(3, 2),
                patch_extent(5, 2, b"data", b"d"),
            ]
        );
    }

    #[test]
    fn truncate_grow_extends_with_sparse_hole() {
        let next = truncate_file(&[extent(0, 3, b"base", 0)], 3, 6).expect("sparse grow");
        assert_eq!(next, vec![extent(0, 3, b"base", 0), hole_extent(3, 3)]);
    }

    #[test]
    fn patch_can_replace_the_middle_of_a_sparse_hole() {
        let next = overlay_file_write(&[hole_extent(0, 8)], 8, 3, 2, b"data", b"d")
            .expect("write inside hole");
        assert_eq!(
            next,
            vec![
                hole_extent(0, 3),
                patch_extent(3, 2, b"data", b"d"),
                hole_extent(5, 3),
            ]
        );
    }

    #[test]
    fn punch_hole_reuses_data_block_prefix_and_suffix() {
        let next =
            punch_hole_file(&[extent(0, 8, b"base", 2)], 8, 3, 2).expect("punch middle range");
        assert_eq!(
            next,
            vec![
                extent(0, 3, b"base", 2),
                hole_extent(3, 2),
                extent(5, 3, b"base", 7),
            ]
        );
    }

    #[test]
    fn punch_hole_clips_at_eof_and_keeps_file_size() {
        let next =
            punch_hole_file(&[extent(0, 8, b"base", 0)], 8, 6, 8).expect("punch through eof");
        assert_eq!(next, vec![extent(0, 6, b"base", 0), hole_extent(6, 2)]);
        validate(8, &next).expect("file size remains unchanged");
    }

    #[test]
    fn punch_hole_past_eof_is_a_layout_noop() {
        let base = vec![extent(0, 8, b"base", 0)];
        assert_eq!(
            punch_hole_file(&base, 8, 12, 4).expect("range past eof"),
            base
        );
    }

    #[test]
    fn explicit_hole_is_not_encoded_as_empty_data() {
        let hole = hole_extent(0, 4);
        assert!(is_hole(&hole));
        validate(4, std::slice::from_ref(&hole)).expect("explicit hole is valid");

        let legacy_data = pb::ExtentRecord {
            logical: Some(pb::ByteRange {
                offset: 0,
                length: 4,
            }),
            block_id: b"old".to_vec(),
            block_offset: 0,
            digest: b"old".to_vec(),
            // 旧协议没有 kind 字段；proto3 默认 0 必须继续按 DATA 处理。
            kind: pb::ExtentKind::Unspecified as i32,
        };
        assert!(!is_hole(&legacy_data));
        validate(4, &[legacy_data]).expect("legacy data extent remains compatible");

        let invalid = pb::ExtentRecord {
            block_id: b"bad".to_vec(),
            digest: b"bad".to_vec(),
            ..hole
        };
        assert!(validate(4, &[invalid]).is_err());
    }
}

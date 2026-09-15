//! Immutable value-layout algorithms used by the real Node runtime.
//!
//! A `VersionLayout` describes one logical value as ordered Extents. Each
//! Extent maps a logical range to a range in an immutable Block. This module is
//! deliberately stateless: it owns calculation, not payload bytes or metadata.

use dms_protocol::v1 as pb;

use super::metadata_client::digest as digest_bytes;
use super::runtime::WorkerError;

/// Verifies that Extents cover exactly `[0, logical_length)` in order.
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

/// Builds a `SET_RANGE` layout without copying unchanged base bytes.
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
        patch_start,
        patch_length,
        patch_block,
        patch_digest,
    )
}

/// 构造文件写的 Extent overlay，并允许写入从当前 EOF 继续向后扩展。
///
/// 普通 KV `SET_RANGE` 不能改变 value 长度；文件 `write(2)` 可以覆盖旧范围并
/// 越过 EOF。未覆盖的旧 Extent 继续复用，新写入只形成一个 immutable Block，
/// 不能为了扩容把整个旧文件读回并重新提交。
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
    if patch_start > logical_length {
        return Err(WorkerError::InvalidArgument(
            "file write patch starts beyond EOF",
        ));
    }
    let patch_end = patch_start
        .checked_add(patch_length)
        .ok_or(WorkerError::InvalidArgument(
            "file write patch overflows u64",
        ))?;
    overlay_with_length(
        base,
        logical_length.max(patch_end),
        patch_start,
        patch_length,
        patch_block,
        patch_digest,
    )
}

fn overlay_with_length(
    base: &[pb::ExtentRecord],
    next_logical_length: u64,
    patch_start: u64,
    patch_length: u64,
    patch_block: &[u8],
    patch_digest: &[u8],
) -> Result<Vec<pb::ExtentRecord>, WorkerError> {
    let patch_end = patch_start
        .checked_add(patch_length)
        .ok_or(WorkerError::InvalidArgument("range patch overflows u64"))?;

    let mut next = Vec::with_capacity(base.len() + 2);
    for extent in base {
        let logical = extent.logical.as_ref().expect("layout was validated above");
        let extent_end = logical.offset + logical.length;
        if extent_end <= patch_start || logical.offset >= patch_end {
            next.push(extent.clone());
            continue;
        }
        if logical.offset < patch_start {
            next.push(pb::ExtentRecord {
                logical: Some(pb::ByteRange {
                    offset: logical.offset,
                    length: patch_start - logical.offset,
                }),
                block_id: extent.block_id.clone(),
                block_offset: extent.block_offset,
                digest: extent.digest.clone(),
            });
        }
        if extent_end > patch_end {
            next.push(pb::ExtentRecord {
                logical: Some(pb::ByteRange {
                    offset: patch_end,
                    length: extent_end - patch_end,
                }),
                block_id: extent.block_id.clone(),
                block_offset: extent
                    .block_offset
                    .checked_add(patch_end - logical.offset)
                    .ok_or(WorkerError::InvalidArgument("block range overflows u64"))?,
                digest: extent.digest.clone(),
            });
        }
    }
    next.push(pb::ExtentRecord {
        logical: Some(pb::ByteRange {
            offset: patch_start,
            length: patch_length,
        }),
        block_id: patch_block.to_vec(),
        block_offset: 0,
        digest: patch_digest.to_vec(),
    });
    next.sort_by_key(|extent| extent.logical.as_ref().map_or(0, |range| range.offset));
    validate(next_logical_length, &next)?;
    Ok(next)
}

/// Names the layout metadata; each immutable Block keeps its own payload digest.
pub(crate) fn digest(logical_length: u64, extents: &[pb::ExtentRecord]) -> Vec<u8> {
    let mut bytes = logical_length.to_be_bytes().to_vec();
    for extent in extents {
        if let Some(logical) = &extent.logical {
            bytes.extend_from_slice(&logical.offset.to_be_bytes());
            bytes.extend_from_slice(&logical.length.to_be_bytes());
        }
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
        pb::ExtentRecord {
            logical: Some(pb::ByteRange {
                offset: start,
                length,
            }),
            block_id: block.to_vec(),
            block_offset,
            digest: block.to_vec(),
        }
    }

    #[test]
    fn one_byte_patch_reuses_the_base_block_on_both_sides() {
        let next =
            overlay(&[extent(0, 6, b"base", 0)], 6, 2, 1, b"patch", b"p").expect("valid overlay");
        assert_eq!(
            next,
            vec![
                extent(0, 2, b"base", 0),
                pb::ExtentRecord {
                    logical: Some(pb::ByteRange {
                        offset: 2,
                        length: 1,
                    }),
                    block_id: b"patch".to_vec(),
                    block_offset: 0,
                    digest: b"p".to_vec(),
                },
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
            vec![
                extent(0, 3, b"base", 0),
                pb::ExtentRecord {
                    logical: Some(pb::ByteRange {
                        offset: 3,
                        length: 2,
                    }),
                    block_id: b"tail".to_vec(),
                    block_offset: 0,
                    digest: b"t".to_vec(),
                },
            ]
        );
    }

    #[test]
    fn file_write_can_replace_suffix_and_extend() {
        let next = overlay_file_write(&[extent(0, 6, b"base", 0)], 6, 4, 4, b"tail", b"t")
            .expect("replace suffix and extend");
        assert_eq!(
            next,
            vec![
                extent(0, 4, b"base", 0),
                pb::ExtentRecord {
                    logical: Some(pb::ByteRange {
                        offset: 4,
                        length: 4,
                    }),
                    block_id: b"tail".to_vec(),
                    block_offset: 0,
                    digest: b"t".to_vec(),
                },
            ]
        );
    }
}

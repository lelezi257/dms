//! Node-owned KKV semantics.
//!
//! The SDK only stages field bytes and sends H* requests. This module owns the
//! read-modify-CAS operation, so concurrent writers are serialized by Meta's
//! version condition rather than by a client-side read/SET convention.

use std::collections::{BTreeMap, HashSet};

use dms_error::ErrorKind;

use super::arena_manager::HostReceipt;
use super::runtime::{NodeHandle, WorkerError};

const KKV_MAGIC: &[u8; 4] = b"DMSH";
const KKV_FORMAT_VERSION: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KkvValue {
    pub(crate) field: Vec<u8>,
    pub(crate) hash_version: u64,
    pub(crate) value_version: u64,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KkvWriteOutcome {
    pub(crate) hash_version: u64,
    pub(crate) field_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KkvRangeWriteOutcome {
    pub(crate) hash_version: u64,
    pub(crate) value_version: u64,
    pub(crate) length: u64,
    pub(crate) field_count: u64,
}

#[derive(Clone)]
pub(crate) struct KkvOperations {
    node: NodeHandle,
}

impl KkvOperations {
    pub(crate) fn new(node: NodeHandle) -> Self {
        Self { node }
    }

    pub(crate) async fn hset(
        &self,
        session_id: u64,
        key: Vec<u8>,
        entries: Vec<(Vec<u8>, u64, HostReceipt)>,
        operation_id: Vec<u8>,
        mode: &str,
        expected_version: Option<u64>,
    ) -> Result<KkvWriteOutcome, WorkerError> {
        validate_key_and_fields(&key, entries.iter().map(|(field, _, _)| field.as_slice()))?;
        let base = self
            .load_optional(session_id, key.clone(), expected_version)
            .await?;
        if expected_version.is_some() && base.is_none() {
            return Err(WorkerError::NotFound);
        }
        let mut fields = match mode {
            "merge" => base
                .as_ref()
                .map_or_else(BTreeMap::new, |base| base.fields.clone()),
            "replace" => BTreeMap::new(),
            _ => return Err(WorkerError::InvalidArgument("invalid Hash write mode")),
        };
        for (field, staging_id, receipt) in entries {
            let bytes = self
                .node
                .consume_staging(session_id, staging_id, receipt)
                .await?;
            fields.insert(field, bytes);
        }
        let field_count = fields.len() as u64;
        let condition = write_condition(base.as_ref(), expected_version);
        let committed = self
            .node
            .set_inline(
                session_id,
                key,
                encode_field_map(&fields)?,
                operation_id,
                condition,
            )
            .await?;
        Ok(KkvWriteOutcome {
            hash_version: committed.version,
            field_count,
        })
    }

    pub(crate) async fn hget(
        &self,
        session_id: u64,
        key: Vec<u8>,
        field: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<Option<KkvValue>, WorkerError> {
        validate_key_and_fields(&key, std::iter::once(field.as_slice()))?;
        let Some(hash) = self.load_optional(session_id, key, exact_version).await? else {
            return Ok(None);
        };
        Ok(hash.fields.get(&field).map(|bytes| KkvValue {
            field,
            hash_version: hash.version,
            value_version: hash.version,
            bytes: bytes.clone(),
        }))
    }

    pub(crate) async fn hmget(
        &self,
        session_id: u64,
        key: Vec<u8>,
        fields: Vec<Vec<u8>>,
        exact_version: Option<u64>,
    ) -> Result<(Option<u64>, Vec<Option<KkvValue>>), WorkerError> {
        validate_key_and_fields(&key, fields.iter().map(Vec::as_slice))?;
        let Some(hash) = self.load_optional(session_id, key, exact_version).await? else {
            return Ok((None, vec![None; fields.len()]));
        };
        let values = fields
            .into_iter()
            .map(|field| {
                hash.fields.get(&field).map(|bytes| KkvValue {
                    field,
                    hash_version: hash.version,
                    value_version: hash.version,
                    bytes: bytes.clone(),
                })
            })
            .collect();
        Ok((Some(hash.version), values))
    }

    pub(crate) async fn hget_all(
        &self,
        session_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<(Option<u64>, Vec<KkvValue>), WorkerError> {
        if key.is_empty() {
            return Err(WorkerError::InvalidArgument("key must not be empty"));
        }
        let Some(hash) = self.load_optional(session_id, key, exact_version).await? else {
            return Ok((None, Vec::new()));
        };
        let values = materialize_entries(&hash);
        Ok((Some(hash.version), values))
    }

    pub(crate) async fn hdelete(
        &self,
        session_id: u64,
        key: Vec<u8>,
        fields: Vec<Vec<u8>>,
        operation_id: Vec<u8>,
        expected_version: Option<u64>,
    ) -> Result<KkvWriteOutcome, WorkerError> {
        validate_key_and_fields(&key, fields.iter().map(Vec::as_slice))?;
        let mut hash = self
            .load_optional(session_id, key.clone(), expected_version)
            .await?
            .ok_or(WorkerError::NotFound)?;
        for field in fields {
            hash.fields.remove(&field);
        }
        let field_count = hash.fields.len() as u64;
        let committed = self
            .node
            .set_inline(
                session_id,
                key,
                encode_field_map(&hash.fields)?,
                operation_id,
                format!("if-version:{}", hash.version),
            )
            .await?;
        Ok(KkvWriteOutcome {
            hash_version: committed.version,
            field_count,
        })
    }

    pub(crate) async fn hscan(
        &self,
        session_id: u64,
        key: Vec<u8>,
        cursor: u64,
        limit: u32,
        exact_version: Option<u64>,
    ) -> Result<(Option<u64>, u64, Vec<KkvValue>), WorkerError> {
        if limit == 0 {
            return Err(WorkerError::InvalidArgument("HSCAN limit must be positive"));
        }
        let Some(hash) = self.load_optional(session_id, key, exact_version).await? else {
            return Ok((None, 0, Vec::new()));
        };
        let all = materialize_entries(&hash);
        let start = usize::try_from(cursor).map_err(|_| WorkerError::ResourceExhausted)?;
        if start >= all.len() {
            return Ok((Some(hash.version), 0, Vec::new()));
        }
        let end = start.saturating_add(limit as usize).min(all.len());
        let next = if end == all.len() { 0 } else { end as u64 };
        Ok((Some(hash.version), next, all[start..end].to_vec()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn hwrite_at(
        &self,
        session_id: u64,
        key: Vec<u8>,
        field: Vec<u8>,
        offset: u64,
        staging_id: u64,
        receipt: HostReceipt,
        operation_id: Vec<u8>,
        expected_version: Option<u64>,
    ) -> Result<KkvRangeWriteOutcome, WorkerError> {
        validate_key_and_fields(&key, std::iter::once(field.as_slice()))?;
        let mut hash = self
            .load_optional(session_id, key.clone(), expected_version)
            .await?
            .ok_or(WorkerError::NotFound)?;
        let patch = self
            .node
            .consume_staging(session_id, staging_id, receipt)
            .await?;
        let value = hash.fields.get_mut(&field).ok_or(WorkerError::NotFound)?;
        let start = usize::try_from(offset).map_err(|_| WorkerError::ResourceExhausted)?;
        let end = start
            .checked_add(patch.len())
            .ok_or(WorkerError::ResourceExhausted)?;
        if end > value.len() {
            return Err(WorkerError::InvalidArgument(
                "HWRITE_AT extends beyond field value",
            ));
        }
        value[start..end].copy_from_slice(&patch);
        let length = value.len() as u64;
        let field_count = hash.fields.len() as u64;
        let committed = self
            .node
            .set_inline(
                session_id,
                key,
                encode_field_map(&hash.fields)?,
                operation_id,
                format!("if-version:{}", hash.version),
            )
            .await?;
        Ok(KkvRangeWriteOutcome {
            hash_version: committed.version,
            value_version: committed.version,
            length,
            field_count,
        })
    }

    async fn load_optional(
        &self,
        session_id: u64,
        key: Vec<u8>,
        exact_version: Option<u64>,
    ) -> Result<Option<KkvFieldMap>, WorkerError> {
        match self
            .node
            .get_materialized(session_id, key, exact_version)
            .await
        {
            Ok((version, bytes)) => Ok(Some(KkvFieldMap {
                version,
                fields: decode_field_map(&bytes)?,
            })),
            Err(WorkerError::NotFound) => Ok(None),
            // Meta 的结构化 NotFound 现在会保留原始 DMS code 透传到 Node。
            // 但在 KKV 读-改-写里，“底层对象不存在”正是创建第一个
            // Hash/KKV 对象的合法起点，所以此处按业务语义显式转成 None。
            Err(WorkerError::Stable(error)) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

struct KkvFieldMap {
    version: u64,
    fields: BTreeMap<Vec<u8>, Vec<u8>>,
}

fn materialize_entries(hash: &KkvFieldMap) -> Vec<KkvValue> {
    hash.fields
        .iter()
        .map(|(field, bytes)| KkvValue {
            field: field.clone(),
            hash_version: hash.version,
            value_version: hash.version,
            bytes: bytes.clone(),
        })
        .collect()
}

fn validate_key_and_fields<'a>(
    key: &[u8],
    fields: impl Iterator<Item = &'a [u8]>,
) -> Result<(), WorkerError> {
    if key.is_empty() {
        return Err(WorkerError::InvalidArgument("key must not be empty"));
    }
    let mut seen = HashSet::new();
    for field in fields {
        if field.is_empty() {
            return Err(WorkerError::InvalidArgument("field must not be empty"));
        }
        if !seen.insert(field.to_vec()) {
            return Err(WorkerError::InvalidArgument("duplicate Hash field"));
        }
    }
    Ok(())
}

fn write_condition(base: Option<&KkvFieldMap>, expected: Option<u64>) -> String {
    match expected.or_else(|| base.map(|base| base.version)) {
        Some(version) => format!("if-version:{version}"),
        None => "if-absent".to_string(),
    }
}

fn encode_field_map(fields: &BTreeMap<Vec<u8>, Vec<u8>>) -> Result<Vec<u8>, WorkerError> {
    let mut out = Vec::new();
    out.extend_from_slice(KKV_MAGIC);
    out.push(KKV_FORMAT_VERSION);
    let count = u32::try_from(fields.len()).map_err(|_| WorkerError::ResourceExhausted)?;
    out.extend_from_slice(&count.to_be_bytes());
    for (field, value) in fields {
        let field_len = u32::try_from(field.len()).map_err(|_| WorkerError::ResourceExhausted)?;
        let value_len = u64::try_from(value.len()).map_err(|_| WorkerError::ResourceExhausted)?;
        out.extend_from_slice(&field_len.to_be_bytes());
        out.extend_from_slice(field);
        out.extend_from_slice(&value_len.to_be_bytes());
        out.extend_from_slice(value);
    }
    Ok(out)
}

fn decode_field_map(bytes: &[u8]) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, WorkerError> {
    if bytes.len() < 9 || &bytes[..4] != KKV_MAGIC || bytes[4] != KKV_FORMAT_VERSION {
        return Err(WorkerError::InvalidArgument(
            "object is not a DMS KKV field map",
        ));
    }
    let mut cursor = 5;
    let count = read_u32(bytes, &mut cursor)?;
    let mut fields = BTreeMap::new();
    for _ in 0..count {
        let field_len = read_u32(bytes, &mut cursor)? as usize;
        let field = read_bytes(bytes, &mut cursor, field_len)?.to_vec();
        let value_len = usize::try_from(read_u64(bytes, &mut cursor)?)
            .map_err(|_| WorkerError::ResourceExhausted)?;
        let value = read_bytes(bytes, &mut cursor, value_len)?.to_vec();
        if fields.insert(field, value).is_some() {
            return Err(WorkerError::InvalidArgument(
                "KKV field map contains duplicate fields",
            ));
        }
    }
    if cursor != bytes.len() {
        return Err(WorkerError::InvalidArgument(
            "KKV field map contains trailing bytes",
        ));
    }
    Ok(fields)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, WorkerError> {
    Ok(u32::from_be_bytes(read_array(bytes, cursor)?))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, WorkerError> {
    Ok(u64::from_be_bytes(read_array(bytes, cursor)?))
}

fn read_array<const N: usize>(bytes: &[u8], cursor: &mut usize) -> Result<[u8; N], WorkerError> {
    let end = cursor
        .checked_add(N)
        .ok_or(WorkerError::ResourceExhausted)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(WorkerError::InvalidArgument("truncated KKV field map"))?
        .try_into()
        .expect("slice length matches array");
    *cursor = end;
    Ok(value)
}

fn read_bytes<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    len: usize,
) -> Result<&'a [u8], WorkerError> {
    let end = cursor
        .checked_add(len)
        .ok_or(WorkerError::ResourceExhausted)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(WorkerError::InvalidArgument("truncated KKV field map"))?;
    *cursor = end;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_map_round_trip_is_ordered_and_binary_safe() {
        let fields = BTreeMap::from([
            (b"b".to_vec(), vec![0, 1, 2]),
            (b"a".to_vec(), b"value".to_vec()),
        ]);
        let encoded = encode_field_map(&fields).expect("encode");
        assert_eq!(decode_field_map(&encoded).expect("decode"), fields);
    }

    #[test]
    fn field_map_rejects_non_hash_object() {
        assert!(matches!(
            decode_field_map(b"ordinary value"),
            Err(WorkerError::InvalidArgument(_))
        ));
    }
}

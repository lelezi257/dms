//! Stable object-domain values shared by the Rust SDK and DMS processes.
//!
//! The public API deliberately uses Redis-familiar names (`SET`, `MSET`, and
//! Hash/KKV operations), while these types keep DMS-specific version and
//! durability semantics explicit. Keys and fields are binary-safe because a
//! future file-system client may use encoded inode or extent identifiers rather
//! than UTF-8 paths.

use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Maximum key length accepted by the first version of the public contract.
///
/// This is a protocol guardrail, not an on-disk layout choice. Raising it later
/// is backward compatible; silently truncating keys would not be.
pub const MAX_KEY_LEN: usize = 1_024;

/// Maximum Hash field length accepted by the first public contract.
pub const MAX_HASH_FIELD_LEN: usize = 1_024;

/// A binary-safe top-level key in the DMS keyspace.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Key(Vec<u8>);

impl Key {
    /// Validates and owns a key supplied by an SDK caller.
    ///
    /// Empty keys are rejected so they cannot accidentally become an implicit
    /// global namespace. The byte representation is otherwise opaque to DMS.
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, InvalidKey> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_KEY_LEN {
            return Err(InvalidKey { len: value.len() });
        }
        Ok(Self(value))
    }

    /// Returns the exact binary key sent by the caller.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for Key {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Key")
            .field(&String::from_utf8_lossy(&self.0))
            .finish()
    }
}

/// A binary-safe secondary key inside one Hash/KKV object.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HashField(Vec<u8>);

impl HashField {
    /// Validates and owns one secondary key.
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, InvalidHashField> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_HASH_FIELD_LEN {
            return Err(InvalidHashField { len: value.len() });
        }
        Ok(Self(value))
    }

    /// Returns the exact binary field supplied by the caller.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for HashField {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Debug for HashField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("HashField")
            .field(&String::from_utf8_lossy(&self.0))
            .finish()
    }
}

/// Monotonic version of a normal `key -> value` object.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectVersion(pub u64);

/// Metadata of one top-level object without reading its bytes.
///
/// `stat()` returns this structure for file-system style `Head` calls. The key
/// remains binary bytes, not UTF-8 text, so a future filesystem adapter can use
/// encoded chunk names directly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectInfo {
    /// Original top-level key.
    pub key: Vec<u8>,
    /// Logical object length in bytes.
    pub length: u64,
    /// Commit timestamp recorded by Meta and converted to Rust's native time.
    pub modified_time: SystemTime,
    /// Current immutable version selected by this stat result.
    pub version: ObjectVersion,
}

/// Monotonic version of an entire Hash/KKV field map.
///
/// Multiple fields published by one `HSET` share this version.
/// Readers therefore bind to one field-map version instead of mixing fields from two
/// concurrent commits.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HashVersion(pub u64);

/// Stable identifier for one logical write across retries.
///
/// A transport retry must reuse the same identifier. Generating a new value
/// could turn a lost reply into a duplicate committed write.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId {
    /// UUID bytes generated once when one SDK instance starts.
    pub client_instance_id: [u8; 16],
    /// Monotonic logical-operation number within that SDK instance.
    pub sequence: u64,
}

impl OperationId {
    pub(crate) fn new(client_instance_id: [u8; 16], sequence: u64) -> Self {
        Self {
            client_instance_id,
            sequence,
        }
    }
}

/// Opaque continuation position for a bounded Hash scan.
///
/// The SDK exposes this user-facing cursor, but its encoded value remains an
/// implementation detail of the selected immutable Hash field-map version.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ScanCursor(pub u64);

/// Controls one top-level prefix scan page.
///
/// This is intentionally separate from [`ScanCursor`], which belongs to Hash
/// field scans. The `cursor` string here is an opaque service cursor for global
/// object listing; SDK callers must not parse or combine it with Hash cursors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanOptions {
    /// Maximum number of objects requested in this page.
    pub limit: u32,
    /// Optional first-page marker. Returned items are strictly greater than it.
    pub start_after: Option<Vec<u8>>,
    /// Opaque cursor returned by the previous `scan()` page.
    pub cursor: Option<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            limit: 128,
            start_after: None,
            cursor: None,
        }
    }
}

/// One top-level prefix scan page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanResult {
    /// Objects returned in byte-order for the requested prefix/page.
    pub items: Vec<ObjectInfo>,
    /// Cursor for the next page. `None` means the scan is complete.
    pub next_cursor: Option<String>,
}

pub(crate) fn system_time_from_unix_millis(millis: i64) -> Result<SystemTime, TimeConversionError> {
    if millis >= 0 {
        return Ok(UNIX_EPOCH + Duration::from_millis(millis as u64));
    }
    UNIX_EPOCH
        .checked_sub(Duration::from_millis(millis.unsigned_abs()))
        .ok_or(TimeConversionError { millis })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimeConversionError {
    millis: i64,
}

impl fmt::Display for TimeConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DMS object modified time {}ms is outside SystemTime range",
            self.millis
        )
    }
}

/// Reliability condition that must be met before a write is acknowledged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurabilityPolicy {
    /// The local node owns the only required in-memory copy.
    LocalMemory,
    /// The requested number of memory replicas must be prepared and committed.
    MemoryCopies(u8),
    /// The value must reach a configured local durable tier.
    LocalDisk,
    /// The value must reach the configured object-store tier.
    ObjectStore,
}

/// Condition checked atomically against the current object version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteCondition {
    /// Create or overwrite regardless of the current state.
    Any,
    /// Commit only when the key does not exist.
    IfAbsent,
    /// Commit only when the key already exists.
    IfPresent,
    /// Commit only when `expected` is still the current version.
    IfVersion(ObjectVersion),
}

/// Version selected by a normal object read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadVersion {
    /// Resolve the current version at the start of this read operation.
    Current,
    /// Read an immutable historical version directly.
    Exact(ObjectVersion),
}

/// Version selected by a Hash/KKV read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HashReadVersion {
    /// Resolve the current Hash field-map version when the operation begins.
    Current,
    /// Read every requested field from one immutable historical field-map version.
    Exact(HashVersion),
}

/// Half-open byte range `[offset, offset + len)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRange {
    /// First byte to return.
    pub offset: u64,
    /// Number of bytes to return.
    pub len: u64,
}

/// One top-level entry supplied to `MSET`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvEntry {
    /// Independent top-level key.
    pub key: Key,
    /// Complete replacement value for that key.
    pub value: Vec<u8>,
}

impl KvEntry {
    /// Builds one batch entry without requiring callers to construct [`Key`].
    pub fn new(key: impl AsRef<[u8]>, value: impl Into<Vec<u8>>) -> Result<Self, InvalidKey> {
        Ok(Self {
            key: Key::new(key.as_ref().to_vec())?,
            value: value.into(),
        })
    }
}

/// One secondary entry supplied to `HSET`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashEntry {
    /// Secondary key within the Hash/KKV object.
    pub field: HashField,
    /// Complete replacement value for this field.
    pub value: Vec<u8>,
}

impl HashEntry {
    /// Builds one Hash entry without exposing the validated internal field type.
    pub fn new(
        field: impl AsRef<[u8]>,
        value: impl Into<Vec<u8>>,
    ) -> Result<Self, InvalidHashField> {
        Ok(Self {
            field: HashField::new(field.as_ref().to_vec())?,
            value: value.into(),
        })
    }
}

/// How `HSET` treats fields omitted from the request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HashWriteMode {
    /// Preserve omitted fields, matching Redis `HSET` behavior.
    Merge,
    /// Publish exactly the supplied field set and remove omitted fields.
    Replace,
}

/// Invalid top-level key supplied by an SDK caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidKey {
    len: usize,
}

impl fmt::Display for InvalidKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DMS key length {} is outside 1..={MAX_KEY_LEN}",
            self.len
        )
    }
}

impl std::error::Error for InvalidKey {}

/// Invalid secondary key supplied by an SDK caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidHashField {
    len: usize,
}

impl fmt::Display for InvalidHashField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DMS Hash field length {} is outside 1..={MAX_HASH_FIELD_LEN}",
            self.len
        )
    }
}

impl std::error::Error for InvalidHashField {}

#[cfg(test)]
mod object_metadata_tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    #[test]
    fn object_scan_options_use_an_independent_opaque_cursor() {
        let options = ScanOptions {
            limit: 64,
            start_after: Some(b"file/a".to_vec()),
            cursor: None,
        };
        assert_eq!(options.limit, 64);
        assert_eq!(options.start_after.as_deref(), Some(&b"file/a"[..]));
        assert_ne!(format!("{:?}", ScanCursor(1)), format!("{:?}", options));
    }

    #[test]
    fn object_info_keeps_native_system_time() {
        let modified_time = system_time_from_unix_millis(1_500).expect("positive millis");
        let info = ObjectInfo {
            key: b"chunk/1".to_vec(),
            length: 7,
            modified_time,
            version: ObjectVersion(3),
        };
        assert_eq!(
            info.modified_time,
            UNIX_EPOCH + Duration::from_millis(1_500)
        );
        assert_eq!(info.key, b"chunk/1");
        assert_eq!(info.version, ObjectVersion(3));
    }

    #[test]
    fn unix_millis_can_represent_times_before_epoch_when_platform_allows_it() {
        let modified_time = system_time_from_unix_millis(-1).expect("negative millis");
        assert_eq!(
            UNIX_EPOCH.duration_since(modified_time).unwrap(),
            Duration::from_millis(1)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{HashField, Key, MAX_HASH_FIELD_LEN, MAX_KEY_LEN};

    #[test]
    fn key_and_hash_field_are_binary_safe_but_bounded() {
        assert_eq!(Key::new([0, 1, 255]).unwrap().as_bytes(), &[0, 1, 255]);
        assert_eq!(HashField::new([0, 255]).unwrap().as_bytes(), &[0, 255]);
        assert!(Key::new(Vec::<u8>::new()).is_err());
        assert!(HashField::new(Vec::<u8>::new()).is_err());
        assert!(Key::new(vec![0; MAX_KEY_LEN + 1]).is_err());
        assert!(HashField::new(vec![0; MAX_HASH_FIELD_LEN + 1]).is_err());
    }
}

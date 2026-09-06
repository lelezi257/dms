//! Stable numeric error identities.
//!
//! `ErrorCode` deliberately is not a closed enum. An old SDK must be able to
//! carry and print a future server code even when it does not know the symbol.

use std::fmt;

/// Opaque public DMS error identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ErrorCode(u32);

impl ErrorCode {
    /// Builds a code from its raw wire/storage value.
    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// Returns the exact value programs compare against known constants.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "0x{:08x}", self.0)
    }
}

pub const CLIENT_ARGUMENT_INVALID: ErrorCode = ErrorCode::from_raw(0x0101_0001);
pub const CLIENT_CONNECTION_UNAVAILABLE: ErrorCode = ErrorCode::from_raw(0x0102_0001);
pub const CLIENT_DEADLINE_EXCEEDED: ErrorCode = ErrorCode::from_raw(0x0102_0002);
pub const CLIENT_PROTOCOL_INVALID_ERROR_DETAIL: ErrorCode = ErrorCode::from_raw(0x0103_0001);
pub const CLIENT_PROTOCOL_VIOLATION: ErrorCode = ErrorCode::from_raw(0x0103_0002);
pub const CLIENT_PERMISSION_DENIED: ErrorCode = ErrorCode::from_raw(0x0104_0001);

pub const NODE_ARENA_CAPACITY_EXHAUSTED: ErrorCode = ErrorCode::from_raw(0x0201_0001);
pub const NODE_ARENA_INVALID_REQUEST: ErrorCode = ErrorCode::from_raw(0x0201_0002);
pub const NODE_ARENA_STALE_HANDLE: ErrorCode = ErrorCode::from_raw(0x0201_0003);
pub const NODE_ARENA_SHM_UNAVAILABLE: ErrorCode = ErrorCode::from_raw(0x0201_0004);
pub const NODE_ARENA_ACCESS_DENIED: ErrorCode = ErrorCode::from_raw(0x0201_0005);
pub const NODE_ARENA_ALLOCATION_FAILED: ErrorCode = ErrorCode::from_raw(0x0201_0006);
pub const NODE_OBJECT_NOT_FOUND: ErrorCode = ErrorCode::from_raw(0x0202_0001);
pub const NODE_VERSION_CONFLICT: ErrorCode = ErrorCode::from_raw(0x0202_0002);
pub const NODE_SESSION_UNKNOWN: ErrorCode = ErrorCode::from_raw(0x0203_0001);
pub const NODE_TRANSFER_UNSUPPORTED: ErrorCode = ErrorCode::from_raw(0x0204_0001);
pub const NODE_TRANSFER_CORRUPT_DATA: ErrorCode = ErrorCode::from_raw(0x0204_0002);
pub const NODE_TRANSFER_UNAVAILABLE: ErrorCode = ErrorCode::from_raw(0x0204_0003);
pub const NODE_METADATA_UNAVAILABLE: ErrorCode = ErrorCode::from_raw(0x0205_0001);
pub const NODE_WORKER_INVALID_REQUEST: ErrorCode = ErrorCode::from_raw(0x0206_0001);
pub const NODE_WORKER_UNAVAILABLE: ErrorCode = ErrorCode::from_raw(0x0206_0002);

pub const META_CATALOG_INVALID_REQUEST: ErrorCode = ErrorCode::from_raw(0x0301_0001);
pub const META_CATALOG_NOT_FOUND: ErrorCode = ErrorCode::from_raw(0x0301_0002);
pub const META_CATALOG_VERSION_CONFLICT: ErrorCode = ErrorCode::from_raw(0x0301_0003);
pub const META_JOURNAL_APPEND_FAILED: ErrorCode = ErrorCode::from_raw(0x0302_0001);
pub const META_JOURNAL_CORRUPT: ErrorCode = ErrorCode::from_raw(0x0302_0002);
pub const META_JOURNAL_UNAVAILABLE: ErrorCode = ErrorCode::from_raw(0x0302_0003);
pub const META_SESSION_UNKNOWN: ErrorCode = ErrorCode::from_raw(0x0303_0001);

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn known_constants_are_unique() {
        let codes = [
            CLIENT_ARGUMENT_INVALID,
            CLIENT_CONNECTION_UNAVAILABLE,
            CLIENT_DEADLINE_EXCEEDED,
            CLIENT_PROTOCOL_INVALID_ERROR_DETAIL,
            CLIENT_PROTOCOL_VIOLATION,
            CLIENT_PERMISSION_DENIED,
            NODE_ARENA_CAPACITY_EXHAUSTED,
            NODE_ARENA_INVALID_REQUEST,
            NODE_ARENA_STALE_HANDLE,
            NODE_ARENA_SHM_UNAVAILABLE,
            NODE_ARENA_ACCESS_DENIED,
            NODE_ARENA_ALLOCATION_FAILED,
            NODE_OBJECT_NOT_FOUND,
            NODE_VERSION_CONFLICT,
            NODE_SESSION_UNKNOWN,
            NODE_TRANSFER_UNSUPPORTED,
            NODE_TRANSFER_CORRUPT_DATA,
            NODE_TRANSFER_UNAVAILABLE,
            NODE_METADATA_UNAVAILABLE,
            NODE_WORKER_INVALID_REQUEST,
            NODE_WORKER_UNAVAILABLE,
            META_CATALOG_INVALID_REQUEST,
            META_CATALOG_NOT_FOUND,
            META_CATALOG_VERSION_CONFLICT,
            META_JOURNAL_APPEND_FAILED,
            META_JOURNAL_CORRUPT,
            META_JOURNAL_UNAVAILABLE,
            META_SESSION_UNKNOWN,
        ];
        let unique = codes.iter().map(|code| code.raw()).collect::<HashSet<_>>();
        assert_eq!(unique.len(), codes.len());
    }
}

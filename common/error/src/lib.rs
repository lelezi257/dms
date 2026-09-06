//! Native DMS error contract.
//!
//! This crate is intentionally independent from protobuf and gRPC. SDK users
//! see this native shape; transport crates merely encode/decode it at process
//! boundaries.

#![forbid(unsafe_code)]

mod code;
mod kind;

use std::fmt;

pub use code::*;
pub use kind::ErrorKind;

/// Convenient result alias for stable DMS subsystem boundaries.
pub type DmsResult<T> = Result<T, DmsError>;

/// Public DMS failure shape.
///
/// `code` is the precise machine identity, `kind` is the broad handling class,
/// and `message` describes this concrete occurrence. Program code must not
/// parse or match the message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DmsError {
    code: ErrorCode,
    kind: ErrorKind,
    message: String,
}

impl DmsError {
    #[must_use]
    pub fn new(code: ErrorCode, kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            code,
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub fn client_invalid_argument(message: impl Into<String>) -> Self {
        Self::new(CLIENT_ARGUMENT_INVALID, ErrorKind::InvalidArgument, message)
    }

    #[must_use]
    pub fn client_protocol_violation(message: impl Into<String>) -> Self {
        Self::new(CLIENT_PROTOCOL_VIOLATION, ErrorKind::Internal, message)
    }

    #[must_use]
    pub fn client_invalid_error_detail(message: impl Into<String>) -> Self {
        Self::new(
            CLIENT_PROTOCOL_INVALID_ERROR_DETAIL,
            ErrorKind::Internal,
            message,
        )
    }

    #[must_use]
    pub fn client_connection_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            CLIENT_CONNECTION_UNAVAILABLE,
            ErrorKind::Unavailable,
            message,
        )
    }

    #[must_use]
    pub fn node_object_not_found(message: impl Into<String>) -> Self {
        Self::new(NODE_OBJECT_NOT_FOUND, ErrorKind::NotFound, message)
    }

    #[must_use]
    pub fn node_transfer_unsupported(message: impl Into<String>) -> Self {
        Self::new(NODE_TRANSFER_UNSUPPORTED, ErrorKind::Unimplemented, message)
    }

    #[must_use]
    pub fn node_transfer_corrupt_data(message: impl Into<String>) -> Self {
        Self::new(NODE_TRANSFER_CORRUPT_DATA, ErrorKind::DataLoss, message)
    }
}

impl fmt::Display for DmsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {:?}: {}", self.code, self.kind, self.message)
    }
}

impl std::error::Error for DmsError {}

#[cfg(test)]
mod catalog_tests {
    use std::{collections::HashMap, collections::HashSet, fs};

    use crate::ErrorKind;

    struct ExpectedError {
        name: &'static str,
        code: crate::ErrorCode,
        kind: ErrorKind,
        component: &'static str,
        subsystem: &'static str,
    }

    const EXPECTED_ERRORS: &[ExpectedError] = &[
        ExpectedError {
            name: "CLIENT_ARGUMENT_INVALID",
            code: crate::CLIENT_ARGUMENT_INVALID,
            kind: ErrorKind::InvalidArgument,
            component: "Client",
            subsystem: "Argument",
        },
        ExpectedError {
            name: "CLIENT_CONNECTION_UNAVAILABLE",
            code: crate::CLIENT_CONNECTION_UNAVAILABLE,
            kind: ErrorKind::Unavailable,
            component: "Client",
            subsystem: "Connection",
        },
        ExpectedError {
            name: "CLIENT_PROTOCOL_INVALID_ERROR_DETAIL",
            code: crate::CLIENT_PROTOCOL_INVALID_ERROR_DETAIL,
            kind: ErrorKind::Internal,
            component: "Client",
            subsystem: "Protocol",
        },
        ExpectedError {
            name: "CLIENT_PROTOCOL_VIOLATION",
            code: crate::CLIENT_PROTOCOL_VIOLATION,
            kind: ErrorKind::Internal,
            component: "Client",
            subsystem: "Protocol",
        },
        ExpectedError {
            name: "CLIENT_DEADLINE_EXCEEDED",
            code: crate::CLIENT_DEADLINE_EXCEEDED,
            kind: ErrorKind::DeadlineExceeded,
            component: "Client",
            subsystem: "Connection",
        },
        ExpectedError {
            name: "CLIENT_PERMISSION_DENIED",
            code: crate::CLIENT_PERMISSION_DENIED,
            kind: ErrorKind::PermissionDenied,
            component: "Client",
            subsystem: "Permission",
        },
        ExpectedError {
            name: "NODE_ARENA_CAPACITY_EXHAUSTED",
            code: crate::NODE_ARENA_CAPACITY_EXHAUSTED,
            kind: ErrorKind::ResourceExhausted,
            component: "Node",
            subsystem: "Arena",
        },
        ExpectedError {
            name: "NODE_ARENA_INVALID_REQUEST",
            code: crate::NODE_ARENA_INVALID_REQUEST,
            kind: ErrorKind::InvalidArgument,
            component: "Node",
            subsystem: "Arena",
        },
        ExpectedError {
            name: "NODE_ARENA_STALE_HANDLE",
            code: crate::NODE_ARENA_STALE_HANDLE,
            kind: ErrorKind::Unavailable,
            component: "Node",
            subsystem: "Arena",
        },
        ExpectedError {
            name: "NODE_ARENA_SHM_UNAVAILABLE",
            code: crate::NODE_ARENA_SHM_UNAVAILABLE,
            kind: ErrorKind::Unavailable,
            component: "Node",
            subsystem: "Arena",
        },
        ExpectedError {
            name: "NODE_ARENA_ACCESS_DENIED",
            code: crate::NODE_ARENA_ACCESS_DENIED,
            kind: ErrorKind::PermissionDenied,
            component: "Node",
            subsystem: "Arena",
        },
        ExpectedError {
            name: "NODE_OBJECT_NOT_FOUND",
            code: crate::NODE_OBJECT_NOT_FOUND,
            kind: ErrorKind::NotFound,
            component: "Node",
            subsystem: "Object",
        },
        ExpectedError {
            name: "NODE_ARENA_ALLOCATION_FAILED",
            code: crate::NODE_ARENA_ALLOCATION_FAILED,
            kind: ErrorKind::ResourceExhausted,
            component: "Node",
            subsystem: "Arena",
        },
        ExpectedError {
            name: "NODE_VERSION_CONFLICT",
            code: crate::NODE_VERSION_CONFLICT,
            kind: ErrorKind::FailedPrecondition,
            component: "Node",
            subsystem: "Object",
        },
        ExpectedError {
            name: "NODE_SESSION_UNKNOWN",
            code: crate::NODE_SESSION_UNKNOWN,
            kind: ErrorKind::Unauthenticated,
            component: "Node",
            subsystem: "Session",
        },
        ExpectedError {
            name: "NODE_TRANSFER_UNSUPPORTED",
            code: crate::NODE_TRANSFER_UNSUPPORTED,
            kind: ErrorKind::Unimplemented,
            component: "Node",
            subsystem: "Transfer",
        },
        ExpectedError {
            name: "NODE_TRANSFER_CORRUPT_DATA",
            code: crate::NODE_TRANSFER_CORRUPT_DATA,
            kind: ErrorKind::DataLoss,
            component: "Node",
            subsystem: "Transfer",
        },
        ExpectedError {
            name: "NODE_TRANSFER_UNAVAILABLE",
            code: crate::NODE_TRANSFER_UNAVAILABLE,
            kind: ErrorKind::Unavailable,
            component: "Node",
            subsystem: "Transfer",
        },
        ExpectedError {
            name: "NODE_METADATA_UNAVAILABLE",
            code: crate::NODE_METADATA_UNAVAILABLE,
            kind: ErrorKind::Unavailable,
            component: "Node",
            subsystem: "Metadata",
        },
        ExpectedError {
            name: "NODE_WORKER_INVALID_REQUEST",
            code: crate::NODE_WORKER_INVALID_REQUEST,
            kind: ErrorKind::InvalidArgument,
            component: "Node",
            subsystem: "Worker",
        },
        ExpectedError {
            name: "NODE_WORKER_UNAVAILABLE",
            code: crate::NODE_WORKER_UNAVAILABLE,
            kind: ErrorKind::Unavailable,
            component: "Node",
            subsystem: "Worker",
        },
        ExpectedError {
            name: "META_CATALOG_INVALID_REQUEST",
            code: crate::META_CATALOG_INVALID_REQUEST,
            kind: ErrorKind::InvalidArgument,
            component: "Meta",
            subsystem: "Catalog",
        },
        ExpectedError {
            name: "META_CATALOG_NOT_FOUND",
            code: crate::META_CATALOG_NOT_FOUND,
            kind: ErrorKind::NotFound,
            component: "Meta",
            subsystem: "Catalog",
        },
        ExpectedError {
            name: "META_CATALOG_VERSION_CONFLICT",
            code: crate::META_CATALOG_VERSION_CONFLICT,
            kind: ErrorKind::Aborted,
            component: "Meta",
            subsystem: "Catalog",
        },
        ExpectedError {
            name: "META_JOURNAL_APPEND_FAILED",
            code: crate::META_JOURNAL_APPEND_FAILED,
            kind: ErrorKind::Unavailable,
            component: "Meta",
            subsystem: "Journal",
        },
        ExpectedError {
            name: "META_JOURNAL_CORRUPT",
            code: crate::META_JOURNAL_CORRUPT,
            kind: ErrorKind::DataLoss,
            component: "Meta",
            subsystem: "Journal",
        },
        ExpectedError {
            name: "META_JOURNAL_UNAVAILABLE",
            code: crate::META_JOURNAL_UNAVAILABLE,
            kind: ErrorKind::Unavailable,
            component: "Meta",
            subsystem: "Journal",
        },
        ExpectedError {
            name: "META_SESSION_UNKNOWN",
            code: crate::META_SESSION_UNKNOWN,
            kind: ErrorKind::FailedPrecondition,
            component: "Meta",
            subsystem: "Session",
        },
    ];

    fn parse_code(entry: &toml::Value, field: &str) -> u32 {
        let value = entry
            .get(field)
            .unwrap_or_else(|| panic!("catalog entry has {field}"));
        if let Some(raw) = value.as_integer() {
            return u32::try_from(raw).expect("numeric error code fits u32");
        }
        let raw = value
            .as_str()
            .unwrap_or_else(|| panic!("{field} is a number or a hex string"));
        let digits = raw.strip_prefix("0x").unwrap_or(raw);
        u32::from_str_radix(digits, 16).expect("hex error code is valid")
    }

    #[test]
    fn catalog_names_and_values_are_unique() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../error-codes.toml");
        let catalog = fs::read_to_string(path).expect("catalog is readable");
        let value: toml::Value = toml::from_str(&catalog).expect("catalog is valid TOML");
        let entries = value
            .get("error")
            .and_then(toml::Value::as_array)
            .expect("catalog has error entries");
        let mut names = HashSet::new();
        let mut values = HashSet::new();
        for entry in entries {
            let name = entry
                .get("name")
                .and_then(toml::Value::as_str)
                .expect("error entry has name");
            let raw = parse_code(entry, "value");
            assert!(
                names.insert(name.to_string()),
                "duplicate error name {name}"
            );
            assert!(values.insert(raw), "duplicate error value {raw:#x}");
            assert!(raw > 0, "zero is not a valid public DMS error code");
        }
        if let Some(reserved) = value.get("reserved").and_then(toml::Value::as_array) {
            for entry in reserved {
                let name = entry
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .expect("reserved entry has name");
                let raw = parse_code(entry, "value");
                assert!(
                    !names.contains(name),
                    "reserved name {name} must not be active"
                );
                assert!(
                    !values.contains(&raw),
                    "reserved value {raw:#x} must not be active"
                );
            }
        }
    }

    #[test]
    fn catalog_matches_rust_constants_and_layout() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../error-codes.toml");
        let catalog = fs::read_to_string(path).expect("catalog is readable");
        let value: toml::Value = toml::from_str(&catalog).expect("catalog is valid TOML");
        let entries = value
            .get("error")
            .and_then(toml::Value::as_array)
            .expect("catalog has error entries");
        let expected = EXPECTED_ERRORS
            .iter()
            .map(|item| (item.name, item))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            entries.len(),
            expected.len(),
            "every active Rust constant must have exactly one catalog row"
        );

        for entry in entries {
            let name = field(entry, "name");
            let expected = expected
                .get(name)
                .unwrap_or_else(|| panic!("catalog entry {name} has no matching Rust constant"));
            let raw = parse_code(entry, "value");
            assert_eq!(raw, expected.code.raw(), "value drift for {name}");
            assert_eq!(field(entry, "kind"), format!("{:?}", expected.kind));
            assert_eq!(field(entry, "component"), expected.component);
            assert_eq!(field(entry, "subsystem"), expected.subsystem);
            assert_layout(raw, expected.component, expected.subsystem);
        }
    }

    #[test]
    fn unknown_future_code_is_preserved() {
        let code = crate::ErrorCode::from_raw(0x7f01_0001);
        let error = crate::DmsError::new(code, crate::ErrorKind::Unknown, "future code");
        assert_eq!(error.code().raw(), 0x7f01_0001);
    }

    fn field<'a>(entry: &'a toml::Value, name: &str) -> &'a str {
        entry
            .get(name)
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("catalog entry has string field {name}"))
    }

    fn assert_layout(raw: u32, component: &str, subsystem: &str) {
        let component_id = (raw >> 24) as u8;
        let subsystem_id = ((raw >> 16) & 0xff) as u8;
        let reason_id = raw & 0xffff;
        assert_ne!(reason_id, 0, "reason id must be non-zero for {raw:#010x}");
        assert_eq!(
            component_id,
            component_layout_id(component),
            "component layout mismatch for {raw:#010x}"
        );
        assert_eq!(
            subsystem_id,
            subsystem_layout_id(component, subsystem),
            "subsystem layout mismatch for {raw:#010x}"
        );
    }

    fn component_layout_id(component: &str) -> u8 {
        match component {
            "Client" => 0x01,
            "Node" => 0x02,
            "Meta" => 0x03,
            other => panic!("unknown component {other}"),
        }
    }

    fn subsystem_layout_id(component: &str, subsystem: &str) -> u8 {
        match (component, subsystem) {
            ("Client", "Argument") => 0x01,
            ("Client", "Connection") => 0x02,
            ("Client", "Protocol") => 0x03,
            ("Client", "Permission") => 0x04,
            ("Node", "Arena") => 0x01,
            ("Node", "Object") => 0x02,
            ("Node", "Session") => 0x03,
            ("Node", "Transfer") => 0x04,
            ("Node", "Metadata") => 0x05,
            ("Node", "Worker") => 0x06,
            ("Meta", "Catalog") => 0x01,
            ("Meta", "Journal") => 0x02,
            ("Meta", "Session") => 0x03,
            (component, subsystem) => panic!("unknown subsystem {component}/{subsystem}"),
        }
    }
}

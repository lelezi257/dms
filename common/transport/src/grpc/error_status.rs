//! Conversion between native DMS errors and gRPC `Status`.
//!
//! This file is the only place where the common transport crate understands
//! both the native error contract and protobuf `ErrorDetail`.

use dms_error::{DmsError, ErrorCode, ErrorKind};
use dms_protocol::v1 as pb;
use prost::Message;
use tonic::{Code, Status};

/// Encodes a terminal DMS error into a whole-call gRPC failure.
#[must_use]
pub fn dms_error_to_status(error: DmsError) -> Status {
    let detail = pb::ErrorDetail {
        dms_code: error.code().raw(),
        kind: wire_kind(error.kind()).into(),
        message: error.message().to_string(),
    };
    Status::with_details(
        grpc_code(error.kind()),
        error.message().to_string(),
        detail.encode_to_vec().into(),
    )
}

/// Decodes a DMS error from a gRPC status.
///
/// Missing or malformed details are interpreted as a client-side protocol
/// problem, which is the right default for SDK callers.
#[must_use]
pub fn status_to_dms_error(status: Status) -> DmsError {
    status_to_dms_error_with(status, DmsError::client_invalid_error_detail)
}

/// Decodes a status while letting a caller choose the error for non-DMS peers.
#[must_use]
pub fn status_to_dms_error_with(
    status: Status,
    fallback: impl FnOnce(String) -> DmsError,
) -> DmsError {
    let details = status.details();
    if details.is_empty() {
        return fallback(format!("{}: {}", status.code(), status.message()));
    }
    match pb::ErrorDetail::decode(details) {
        Ok(detail) => DmsError::new(
            ErrorCode::from_raw(detail.dms_code),
            native_kind(detail.kind()),
            detail.message,
        ),
        Err(error) => DmsError::client_invalid_error_detail(format!(
            "invalid DMS ErrorDetail from peer: {error}"
        )),
    }
}

fn grpc_code(kind: ErrorKind) -> Code {
    match kind {
        ErrorKind::Unknown => Code::Unknown,
        ErrorKind::InvalidArgument => Code::InvalidArgument,
        ErrorKind::NotFound => Code::NotFound,
        ErrorKind::AlreadyExists => Code::AlreadyExists,
        ErrorKind::PermissionDenied => Code::PermissionDenied,
        ErrorKind::ResourceExhausted => Code::ResourceExhausted,
        ErrorKind::FailedPrecondition => Code::FailedPrecondition,
        ErrorKind::Aborted => Code::Aborted,
        ErrorKind::Unimplemented => Code::Unimplemented,
        ErrorKind::Internal => Code::Internal,
        ErrorKind::Unavailable => Code::Unavailable,
        ErrorKind::DataLoss => Code::DataLoss,
        ErrorKind::Unauthenticated => Code::Unauthenticated,
        ErrorKind::DeadlineExceeded => Code::DeadlineExceeded,
    }
}

fn wire_kind(kind: ErrorKind) -> pb::ErrorKind {
    match kind {
        ErrorKind::Unknown => pb::ErrorKind::Unknown,
        ErrorKind::InvalidArgument => pb::ErrorKind::InvalidArgument,
        ErrorKind::NotFound => pb::ErrorKind::NotFound,
        ErrorKind::AlreadyExists => pb::ErrorKind::AlreadyExists,
        ErrorKind::PermissionDenied => pb::ErrorKind::PermissionDenied,
        ErrorKind::ResourceExhausted => pb::ErrorKind::ResourceExhausted,
        ErrorKind::FailedPrecondition => pb::ErrorKind::FailedPrecondition,
        ErrorKind::Aborted => pb::ErrorKind::Aborted,
        ErrorKind::Unimplemented => pb::ErrorKind::Unimplemented,
        ErrorKind::Internal => pb::ErrorKind::Internal,
        ErrorKind::Unavailable => pb::ErrorKind::Unavailable,
        ErrorKind::DataLoss => pb::ErrorKind::DataLoss,
        ErrorKind::Unauthenticated => pb::ErrorKind::Unauthenticated,
        ErrorKind::DeadlineExceeded => pb::ErrorKind::DeadlineExceeded,
    }
}

fn native_kind(kind: pb::ErrorKind) -> ErrorKind {
    match kind {
        pb::ErrorKind::Unknown => ErrorKind::Unknown,
        pb::ErrorKind::InvalidArgument => ErrorKind::InvalidArgument,
        pb::ErrorKind::NotFound => ErrorKind::NotFound,
        pb::ErrorKind::AlreadyExists => ErrorKind::AlreadyExists,
        pb::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied,
        pb::ErrorKind::ResourceExhausted => ErrorKind::ResourceExhausted,
        pb::ErrorKind::FailedPrecondition => ErrorKind::FailedPrecondition,
        pb::ErrorKind::Aborted => ErrorKind::Aborted,
        pb::ErrorKind::Unimplemented => ErrorKind::Unimplemented,
        pb::ErrorKind::Internal => ErrorKind::Internal,
        pb::ErrorKind::Unavailable => ErrorKind::Unavailable,
        pb::ErrorKind::DataLoss => ErrorKind::DataLoss,
        pb::ErrorKind::Unauthenticated => ErrorKind::Unauthenticated,
        pb::ErrorKind::DeadlineExceeded => ErrorKind::DeadlineExceeded,
    }
}

#[cfg(test)]
mod tests {
    use dms_error::{META_JOURNAL_APPEND_FAILED, NODE_ARENA_CAPACITY_EXHAUSTED};

    use super::*;

    #[test]
    fn known_error_roundtrips_through_status_details() {
        let original = DmsError::new(
            NODE_ARENA_CAPACITY_EXHAUSTED,
            ErrorKind::ResourceExhausted,
            "requested=11 available=4",
        );
        let decoded = status_to_dms_error(dms_error_to_status(original.clone()));
        assert_eq!(decoded, original);
    }

    #[test]
    fn unknown_future_code_is_not_lost() {
        let original = DmsError::new(
            ErrorCode::from_raw(0x7f01_0001),
            ErrorKind::Unavailable,
            "future server code",
        );
        let decoded = status_to_dms_error(dms_error_to_status(original));
        assert_eq!(decoded.code().raw(), 0x7f01_0001);
        assert_eq!(decoded.kind(), ErrorKind::Unavailable);
    }

    #[test]
    fn unknown_future_wire_kind_becomes_unknown_without_losing_code() {
        let detail = pb::ErrorDetail {
            dms_code: 0x7f01_0002,
            kind: 999,
            message: "future kind".to_string(),
        };
        let status =
            Status::with_details(Code::Unknown, "future kind", detail.encode_to_vec().into());

        let decoded = status_to_dms_error(status);

        assert_eq!(decoded.code().raw(), 0x7f01_0002);
        assert_eq!(decoded.kind(), ErrorKind::Unknown);
        assert_eq!(decoded.message(), "future kind");
    }

    #[test]
    fn malformed_detail_becomes_protocol_error() {
        let status = Status::with_details(Code::Internal, "bad", vec![1, 2, 3].into());
        let decoded = status_to_dms_error(status);
        assert_eq!(
            decoded.code(),
            dms_error::CLIENT_PROTOCOL_INVALID_ERROR_DETAIL
        );
    }

    #[test]
    fn missing_detail_uses_call_boundary_fallback() {
        let status = Status::unavailable("plain grpc unavailable");
        let decoded = status_to_dms_error_with(status, |message| {
            DmsError::new(META_JOURNAL_APPEND_FAILED, ErrorKind::Unavailable, message)
        });
        assert_eq!(decoded.code(), META_JOURNAL_APPEND_FAILED);
    }
}

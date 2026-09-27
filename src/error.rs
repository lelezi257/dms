//! 进程边缘的错误映射。领域错误不依赖 Axum 或 libc；协议只在此转换。
//! POSIX 无法携带 AFS 数字码：返回 errno，同时在结构化日志保留 code/kind。
use afs_error::{Error, ErrorKind, IO_DIRECTORY_NOT_EMPTY, IO_IS_DIRECTORY, IO_NOT_DIRECTORY};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
pub struct RestError(pub Error);
impl From<Error> for RestError {
    fn from(error: Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        let error = self.0;
        afs_logging::error!("request failed"; "code"=>error.code().raw(), "kind"=>format!("{:?}", error.kind()), "message"=>error.message());
        (http_status(error.kind()), Json(json!({"error": {"code": error.code().raw(), "kind": format!("{:?}", error.kind()), "message": error.message()}}))).into_response()
    }
}
pub fn http_status(kind: ErrorKind) -> StatusCode {
    use ErrorKind::*;
    match kind {
        InvalidArgument | OutOfRange => StatusCode::BAD_REQUEST,
        NotFound => StatusCode::NOT_FOUND,
        AlreadyExists | Aborted => StatusCode::CONFLICT,
        PermissionDenied => StatusCode::FORBIDDEN,
        Unauthenticated => StatusCode::UNAUTHORIZED,
        ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
        FailedPrecondition => StatusCode::PRECONDITION_FAILED,
        Unimplemented => StatusCode::NOT_IMPLEMENTED,
        Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        Cancelled => StatusCode::REQUEST_TIMEOUT,
        Unknown | Internal | DataLoss => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
pub fn errno(error: &Error) -> i32 {
    if error.code() == afs_error::NODE_OWNER_STALE_ACCESS
        || error.code() == afs_error::NODE_OWNER_STALE_HANDLE
    {
        return libc::ESTALE;
    }
    if error.code() == IO_NOT_DIRECTORY {
        return libc::ENOTDIR;
    }
    if error.code() == IO_IS_DIRECTORY {
        return libc::EISDIR;
    }
    if error.code() == IO_DIRECTORY_NOT_EMPTY {
        return libc::ENOTEMPTY;
    }
    if error.code() == afs_error::IO_CROSS_DEVICE {
        return libc::EXDEV;
    }
    if error.code() == afs_error::IO_OUT_OF_MEMORY {
        return libc::ENOMEM;
    }
    use ErrorKind::*;
    match error.kind() {
        InvalidArgument => libc::EINVAL,
        OutOfRange => libc::ERANGE,
        NotFound => libc::ENOENT,
        AlreadyExists => libc::EEXIST,
        PermissionDenied | Unauthenticated => libc::EACCES,
        ResourceExhausted => {
            if error.code() == afs_error::IO_CAPACITY {
                libc::ENOSPC
            } else {
                libc::EAGAIN
            }
        }
        FailedPrecondition => libc::EBUSY,
        Aborted => libc::EAGAIN,
        Unimplemented => libc::ENOSYS,
        Unavailable => libc::EHOSTUNREACH,
        DeadlineExceeded => libc::ETIMEDOUT,
        Cancelled => libc::EINTR,
        Unknown | Internal | DataLoss => libc::EIO,
    }
}

//! 协议无关的基础错误；具体业务仍由自身定义，协议边缘负责映射。
#![forbid(unsafe_code)]
use std::fmt;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidArgument,
    Unsupported,
    NotFound,
    Unavailable,
    Io,
    Conflict,
}
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}
impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(
            match e.kind() {
                std::io::ErrorKind::NotFound => ErrorKind::NotFound,
                std::io::ErrorKind::InvalidInput => ErrorKind::InvalidArgument,
                _ => ErrorKind::Io,
            },
            e.to_string(),
        )
    }
}
pub type Result<T> = std::result::Result<T, Error>;

//! Generated Rust view of the versioned DMS wire contract.
//!
//! The `.proto` files remain the language-neutral source of truth. This crate
//! is private infrastructure linked into the Rust SDK and server bundle.

#![forbid(unsafe_code)]

/// Worker 单次 GET 响应内联 bytes 的安全上限。
/// 更大读取继续使用 payload 票据，避免控制响应变成无界数据传输。
pub const MAX_INLINE_READ_BYTES: u64 = 64 * 1024;

/// Version 1 Client ↔ Node protobuf DTOs and generated Tonic service types.
pub mod v1 {
    tonic::include_proto!("dms.v1");
}

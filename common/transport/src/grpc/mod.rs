//! Shared gRPC configuration and security policy.
//!
//! Business services, generated protobuf clients and generated server traits
//! deliberately stay in their callers. This module only applies the common
//! transport knobs that every DMS process relation must share.

mod config;
mod error_status;
mod security;

pub use config::GrpcConfig;
pub use error_status::{dms_error_to_status, status_to_dms_error, status_to_dms_error_with};
pub use security::{SecurityManager, TlsConfig};

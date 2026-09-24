//! Shared gRPC configuration and security policy.
//!
//! Business services, generated protobuf clients and generated server traits
//! deliberately stay in their callers. This module only applies the common
//! transport knobs that every DMS process relation must share.

mod config;
mod security;

pub use config::GrpcConfig;
pub use security::{SecurityManager, TlsConfig};

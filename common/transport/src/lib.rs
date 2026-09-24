//! Shared gRPC connection configuration and TLS policy.
//!
//! Business protocols, routing and request lifecycles belong to their backends.

#![forbid(unsafe_code)]

mod error;
mod grpc;

pub use error::GrpcError;
pub use grpc::{GrpcConfig, SecurityManager, TlsConfig};

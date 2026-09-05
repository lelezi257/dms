//! Generated Rust view of the versioned DMS wire contract.
//!
//! The `.proto` files remain the language-neutral source of truth. This crate
//! is private infrastructure linked into the Rust SDK and server bundle.

#![forbid(unsafe_code)]

/// Version 1 Client ↔ Node protobuf DTOs and generated Tonic service types.
pub mod v1 {
    tonic::include_proto!("dms.v1");
}

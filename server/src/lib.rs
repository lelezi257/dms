//! Server-side implementation shared by the `dms-node` and `dms-meta`
//! processes.
//!
//! This Cargo package is built into deployable binaries. It is not a client SDK
//! dependency and is not published to the Cargo registry.

#![deny(unsafe_code)]

pub mod config;
mod error;
pub mod health;
mod identity;
pub mod meta;
pub mod node;

pub use error::{InvalidNodeId, ParseComponentKindError};
pub use identity::{ComponentKind, NodeId};

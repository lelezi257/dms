//! Private implementation modules of the Rust SDK.
//!
//! The public `DmsClient` plus these four implementation modules form the five
//! reviewed Client responsibilities: orchestration, one Node connection,
//! payload-provider selection, and the invalidation-aware Current cache.

pub(crate) mod client_cache;
pub(crate) mod client_impl;
pub(crate) mod node_connection;
// mmap 借用的协议证明只在传输边界审计，其余 SDK 仍拒绝 unsafe。
#[allow(unsafe_code)]
pub(crate) mod transfer_engine;

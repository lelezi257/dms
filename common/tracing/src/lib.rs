//! DMS distributed-tracing boundary.
//!
//! The default feature set is intentionally small enough for `dms-client`:
//! it creates spans, propagates W3C Trace Context and exposes correlation
//! helpers, but it never installs a global subscriber. Long-running DMS
//! processes enable the `runtime` feature and explicitly call
//! [`init_process_tracing`].

#![forbid(unsafe_code)]

mod config;
mod context;
mod grpc;

#[cfg(feature = "runtime")]
mod init;
#[cfg(feature = "runtime")]
mod server;

pub use config::{ProcessIdentity, TracingConfig, TracingError};
pub use context::{
    TraceContext, TraceCorrelation, capture_current_context, current_correlation, current_exemplar,
    record_error, record_ok, set_parent,
};
pub use grpc::{
    TraceContextInterceptor, TracedChannel, extract_remote_context, inject_current_context,
    request_with_current_context, traced_channel,
};
#[cfg(feature = "runtime")]
pub use init::{TracingGuard, init_process_tracing};
#[cfg(feature = "runtime")]
pub use server::GrpcServerTraceLayer;

pub use tracing;
pub use tracing::Instrument;

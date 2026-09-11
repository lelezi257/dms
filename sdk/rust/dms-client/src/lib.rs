//! Rust client SDK for DMS applications.
//!
//! This crate is the only Rust dependency required by an application. It owns
//! the public `DmsClient` API and public data types. Placement, active-node
//! management, endpoint providers, physical buffers and cache stay private so
//! future Python, C++, and Go SDKs can implement the same process contract
//! without depending on Rust internals.

#![deny(unsafe_code)]

mod client;
mod internal;
mod metrics;
mod types;

pub use client::{
    ClientOptions, ClientTlsOptions, ConnectError, DeleteResult, DmsClient, DmsValueReader,
    GetIntoResult, GetOptions, GetResult, HashDeleteOptions, HashEntriesResult, HashGetOptions,
    HashMultiGetResult, HashRangeWriteOptions, HashRangeWriteResult, HashScanOptions,
    HashScanResult, HashSetResult, HashValue, HashWriteOptions, KeyVersion, MSetOptions,
    MSetResult, RangeWriteOptions, ReadResult, SetOptions, SetResult, SharedValueView,
    SharedWriteBuffer,
};
pub use dms_error::{DmsError, DmsResult, ErrorCode, ErrorKind};
pub use dms_metrics::{
    MetricsError, Registry as MetricsRegistry, encode_text as encode_metrics_text,
};
pub use types::{
    ByteRange, DurabilityPolicy, HashEntry, HashField, HashReadVersion, HashVersion, HashWriteMode,
    InvalidHashField, InvalidKey, Key, KvEntry, MAX_HASH_FIELD_LEN, MAX_KEY_LEN, ObjectInfo,
    ObjectVersion, OperationId, ReadVersion, ScanCursor, ScanOptions, ScanResult, WriteCondition,
};

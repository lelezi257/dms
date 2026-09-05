//! SDK 和服务端共用的连接配置、TLS、错误边界及字节校验。
//!
//! 业务 handler 仍归各服务；SHM 的 fd/mmap 在独立公共内存组件内。
//! 本 crate 不接管对象提交、路由或生命周期，也不承诺尚未实现的传输后端。

#![forbid(unsafe_code)]

pub mod checksum;
mod error;
mod grpc;

pub use error::GrpcError;
pub use grpc::{
    GrpcConfig, SecurityManager, TlsConfig, dms_error_to_status, status_to_dms_error,
    status_to_dms_error_with,
};

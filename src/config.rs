//! Defaults < TOML < explicit CLI, following main's typed configuration mechanism.
//! Configuration belongs to processes; libraries receive resolved value objects.
//!
//! 例：TOML 写 fs="ownerfs"，启动时传 --fs blobfs，则运行 BlobFs。
//! Cli 字段采用 Option，避免 clap 默认值把 TOML 的显式设置覆盖掉。
//! 编译 feature 决定代码是否存在，运行配置决定现有代码是否实例化；两者不能混用。
//! 这里只选后端和通道，不定义文件授权、存储位置或镜像发布策略。

use afs_error::{Error, ErrorKind, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Meta,
    Node,
}
#[derive(Debug, Clone, Parser, Default)]
#[command(version, about = "AFS process foundation")]
/// 命令行输入层。None 表示用户未传入，因此应继续考虑 TOML 与最终默认值。
pub struct Cli {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub grpc_listen: Option<SocketAddr>,
    #[arg(long)]
    pub rest_listen: Option<SocketAddr>,
    #[arg(long)]
    pub meta_endpoint: Option<String>,
    #[arg(long)]
    pub peer_endpoint: Option<String>,
    #[arg(long)]
    pub fs: Option<String>,
    #[arg(long)]
    pub data_mode: Option<String>,
    #[arg(long)]
    pub rdma_device: Option<String>,
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    #[arg(long)]
    pub uds_path: Option<PathBuf>,
    #[arg(long)]
    pub mount: Option<PathBuf>,
    #[arg(long)]
    pub timeout_ms: Option<u64>,
    #[arg(long)]
    pub log_level: Option<String>,
    #[arg(long)]
    pub trace_enabled: Option<bool>,
    #[arg(long)]
    pub trace_endpoint: Option<String>,
    #[arg(long)]
    pub trace_sample_ratio: Option<f64>,
    #[arg(long)]
    pub print_config: bool,
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
/// TOML 输入层。拒绝未知字段，让拼错开关在启动时失败，而不是静默使用默认行为。
struct FileConfig {
    id: Option<String>,
    grpc_listen: Option<SocketAddr>,
    rest_listen: Option<SocketAddr>,
    meta_endpoint: Option<String>,
    peer_endpoint: Option<String>,
    fs: Option<String>,
    data_mode: Option<String>,
    rdma_device: Option<String>,
    data_dir: Option<PathBuf>,
    uds_path: Option<PathBuf>,
    mount: Option<PathBuf>,
    timeout_ms: Option<u64>,
    log_level: Option<String>,
    trace_enabled: Option<bool>,
    trace_endpoint: Option<String>,
    trace_sample_ratio: Option<f64>,
}
#[derive(Debug, Clone, Serialize)]
/// 解析和校验后的进程配置。模块接收这个值，不各自重复读取配置文件或环境变量。
pub struct Config {
    pub id: String,
    pub grpc_listen: SocketAddr,
    pub rest_listen: SocketAddr,
    pub meta_endpoint: Option<String>,
    pub peer_endpoint: Option<String>,
    pub ownerfs: bool,
    pub blobfs: bool,
    pub data_mode: String,
    pub rdma_device: Option<String>,
    pub data_dir: PathBuf,
    pub uds_path: PathBuf,
    pub mount: Option<PathBuf>,
    pub timeout_ms: u64,
    pub log_level: String,
    pub trace_enabled: bool,
    pub trace_endpoint: String,
    pub trace_sample_ratio: f64,
    #[serde(skip)]
    pub print_config: bool,
}
impl Config {
    /// 按默认值 < TOML < 显式 CLI 合并；所有可发现的配置错误在监听服务之前返回。
    /// Meta 可以没有文件后端；Node 至少启用一个。未指定 fs 时使用当前编译进去的集合。
    pub fn resolve(role: Role, cli: Cli) -> Result<Self> {
        let file: FileConfig = match cli.config {
            Some(path) => toml::from_str(&std::fs::read_to_string(path)?)
                .map_err(|e| invalid(e.to_string()))?,
            None => FileConfig::default(),
        };
        let id = cli.id.or(file.id).unwrap_or_else(|| {
            match role {
                Role::Node => "node",
                Role::Meta => "meta",
            }
            .into()
        });
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        {
            return Err(invalid("id must contain 1..128 letters, digits, _ or -"));
        }
        // cfg! 查询本次二进制的编译能力；VFS 内的 #[cfg] 才实际裁掉后端模块。
        let requested = cli.fs.or(file.fs);
        let (ownerfs, blobfs) = match requested.as_deref() {
            None => (cfg!(feature = "ownerfs"), cfg!(feature = "blobfs")),
            Some("all") => (true, true),
            Some("ownerfs") => (true, false),
            Some("blobfs") => (false, true),
            _ => return Err(invalid("fs must be ownerfs, blobfs or all")),
        };
        if role == Role::Node
            && ((ownerfs && !cfg!(feature = "ownerfs")) || (blobfs && !cfg!(feature = "blobfs")))
        {
            return Err(invalid("requested filesystem backend is not compiled in"));
        }
        if role == Role::Node && !ownerfs && !blobfs {
            return Err(invalid(
                "node requires at least one compiled filesystem backend",
            ));
        }
        let data_mode = cli
            .data_mode
            .or(file.data_mode)
            .unwrap_or_else(|| "grpc".into());
        if !["grpc", "rdma", "auto"].contains(&data_mode.as_str()) {
            return Err(invalid("data_mode must be grpc, rdma or auto"));
        }
        if data_mode == "rdma" && !cfg!(feature = "rdma") {
            return Err(invalid("rdma feature is not compiled in"));
        }
        let cfg = Self {
            grpc_listen: cli.grpc_listen.or(file.grpc_listen).unwrap_or_else(|| {
                ([127, 0, 0, 1], if role == Role::Meta { 7400 } else { 7500 }).into()
            }),
            rest_listen: cli.rest_listen.or(file.rest_listen).unwrap_or_else(|| {
                ([127, 0, 0, 1], if role == Role::Meta { 7401 } else { 7501 }).into()
            }),
            meta_endpoint: cli.meta_endpoint.or(file.meta_endpoint),
            peer_endpoint: cli.peer_endpoint.or(file.peer_endpoint),
            data_dir: cli
                .data_dir
                .or(file.data_dir)
                .unwrap_or_else(|| PathBuf::from(format!("/tmp/afs-{id}/data"))),
            uds_path: cli
                .uds_path
                .or(file.uds_path)
                .unwrap_or_else(|| PathBuf::from(format!("/tmp/afs-{id}/local.sock"))),
            id,
            ownerfs,
            blobfs,
            data_mode,
            rdma_device: cli.rdma_device.or(file.rdma_device),
            mount: cli.mount.or(file.mount),
            timeout_ms: cli.timeout_ms.or(file.timeout_ms).unwrap_or(5000),
            log_level: cli
                .log_level
                .or(file.log_level)
                .unwrap_or_else(|| "info".into()),
            trace_enabled: cli.trace_enabled.or(file.trace_enabled).unwrap_or(false),
            trace_endpoint: cli
                .trace_endpoint
                .or(file.trace_endpoint)
                .unwrap_or_else(|| "http://127.0.0.1:4317".into()),
            trace_sample_ratio: cli
                .trace_sample_ratio
                .or(file.trace_sample_ratio)
                .unwrap_or(0.01),
            print_config: cli.print_config,
        };
        if cfg.timeout_ms == 0 || cfg.timeout_ms > 300_000 {
            return Err(invalid("timeout_ms must be in 1..300000"));
        }
        if !(0.0..=1.0).contains(&cfg.trace_sample_ratio) {
            return Err(invalid("trace_sample_ratio must be 0..1"));
        }
        afs_logging::parse_level(&cfg.log_level).map_err(|e| invalid(e.to_string()))?;
        for endpoint in [&cfg.meta_endpoint, &cfg.peer_endpoint]
            .into_iter()
            .flatten()
        {
            if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
                return Err(invalid("gRPC endpoint must be an http(s) URI"));
            }
            tonic::transport::Endpoint::from_shared(endpoint.clone())
                .map_err(|e| invalid(e.to_string()))?;
        }
        if !cfg.uds_path.is_absolute() || !cfg.data_dir.is_absolute() {
            return Err(invalid("uds_path and data_dir must be absolute"));
        }
        Ok(cfg)
    }
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidArgument, message)
}

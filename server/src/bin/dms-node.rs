//! `dms-node` process entry point.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use dms_logging::{ProcessIdentity, init_process_logging};
use dms_server::config::{
    LoggingCliOverrides, NodeCliOverrides, NodeConfigFile, ResolvedNodeConfig, TracingCliOverrides,
};

#[derive(Debug, Parser)]
#[command(name = "dms-node")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ServeArgs),
}

#[derive(Clone, Debug, clap::Args)]
struct ServeArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    node_id: Option<String>,
    #[arg(long)]
    health_address: Option<String>,
    #[arg(long)]
    worker_tcp_address: Option<String>,
    #[arg(long)]
    worker_uds_path: Option<String>,
    #[arg(long)]
    meta_endpoint: Option<String>,
    #[arg(long)]
    arena_capacity_bytes: Option<u64>,
    #[arg(long)]
    staging_ttl_millis: Option<u64>,
    /// Client Current 缓存租约上限（1..=30000 ms），重启生效。
    #[arg(long)]
    client_cache_lease_ttl_millis: Option<u64>,
    #[command(flatten)]
    log: LogArgs,
    #[command(flatten)]
    tracing: TracingArgs,
}

#[derive(Clone, Debug, Default, clap::Args)]
struct LogArgs {
    #[arg(long)]
    log_level: Option<String>,
    #[arg(long)]
    log_format: Option<String>,
    #[arg(long)]
    log_path: Option<String>,
    #[arg(long)]
    log_async_queue_capacity: Option<usize>,
    #[arg(long)]
    log_overflow: Option<String>,
    #[arg(long)]
    log_max_file_size_bytes: Option<u64>,
    #[arg(long)]
    log_max_backups: Option<usize>,
    #[arg(long)]
    log_max_age_seconds: Option<u64>,
}

impl From<LogArgs> for LoggingCliOverrides {
    fn from(args: LogArgs) -> Self {
        Self {
            level: args.log_level,
            format: args.log_format,
            path: args.log_path,
            async_queue_capacity: args.log_async_queue_capacity,
            overflow: args.log_overflow,
            max_file_size_bytes: args.log_max_file_size_bytes,
            max_backups: args.log_max_backups,
            max_age_seconds: args.log_max_age_seconds,
        }
    }
}

#[derive(Clone, Debug, Default, clap::Args)]
struct TracingArgs {
    #[arg(long)]
    tracing_enabled: Option<bool>,
    /// Include successful periodic heartbeat/keepalive operations in traces.
    #[arg(long)]
    tracing_periodic_operations: Option<bool>,
    #[arg(long)]
    tracing_otlp_endpoint: Option<String>,
    #[arg(long)]
    tracing_sample_ratio: Option<f64>,
    #[arg(long)]
    tracing_queue_capacity: Option<usize>,
    #[arg(long)]
    tracing_max_export_batch_size: Option<usize>,
    #[arg(long)]
    tracing_batch_interval_millis: Option<u64>,
    #[arg(long)]
    tracing_export_timeout_millis: Option<u64>,
}

impl From<TracingArgs> for TracingCliOverrides {
    fn from(args: TracingArgs) -> Self {
        Self {
            enabled: args.tracing_enabled,
            periodic_operations: args.tracing_periodic_operations,
            otlp_endpoint: args.tracing_otlp_endpoint,
            sample_ratio: args.tracing_sample_ratio,
            queue_capacity: args.tracing_queue_capacity,
            max_export_batch_size: args.tracing_max_export_batch_size,
            batch_interval_millis: args.tracing_batch_interval_millis,
            export_timeout_millis: args.tracing_export_timeout_millis,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("dms-node error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => serve(args),
    }
}

fn serve(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let file = match args.config {
        Some(path) => NodeConfigFile::load(path)?,
        None => NodeConfigFile::default(),
    };
    let resolved = ResolvedNodeConfig::from_sources(
        file,
        NodeCliOverrides {
            node_id: args.node_id,
            health_address: args.health_address,
            worker_tcp_address: args.worker_tcp_address,
            worker_uds_path: args.worker_uds_path,
            meta_endpoint: args.meta_endpoint,
            arena_capacity_bytes: args.arena_capacity_bytes,
            staging_ttl_millis: args.staging_ttl_millis,
            client_cache_lease_ttl_millis: args.client_cache_lease_ttl_millis,
            log: args.log.into(),
            tracing: args.tracing.into(),
        },
    )?;
    let node_id = dms_server::NodeId::new(&resolved.node_id)?;
    let logging_guard = init_process_logging(
        &resolved.logging,
        ProcessIdentity::new("dms-node", resolved.node_id.clone()),
    )?;
    let result = dms_server::node::serve(dms_server::node::NodeConfig {
        node_id,
        health_address: resolved.health_address,
        worker_tcp_address: resolved.worker_tcp_address,
        worker_uds_path: resolved.worker_uds_path.map(PathBuf::from),
        meta_endpoint: resolved.meta_endpoint,
        arena_capacity_bytes: resolved.arena_capacity_bytes,
        staging_ttl: resolved.staging_ttl,
        client_cache_lease_ttl: resolved.client_cache_lease_ttl,
        log_level: logging_guard.level_controller(),
        tracing: resolved.tracing,
    });
    if let Err(error) = &result {
        dms_logging::error!(
            "dms-node stopped with an error";
            "event" => "node.process.failed",
            "error" => error.to_string(),
        );
    }
    drop(logging_guard);
    result
}

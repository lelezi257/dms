//! `dms-meta` process entry point.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use dms_logging::{ProcessIdentity, init_process_logging};
use dms_server::config::{
    LoggingCliOverrides, MetaCliOverrides, MetaConfigFile, ResolvedMetaConfig, TracingCliOverrides,
};

#[derive(Debug, Parser)]
#[command(name = "dms-meta")]
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
    grpc_address: Option<String>,
    #[arg(long)]
    journal_dir: Option<String>,
    /// 全量元数据快照间隔：默认 4096 条 journal 记录，必须大于 0。
    #[arg(long)]
    checkpoint_every_records: Option<u64>,
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
        eprintln!("dms-meta error: {error}");
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
        Some(path) => MetaConfigFile::load(path)?,
        None => MetaConfigFile::default(),
    };
    let resolved = ResolvedMetaConfig::from_sources(
        file,
        MetaCliOverrides {
            node_id: args.node_id,
            health_address: args.health_address,
            grpc_address: args.grpc_address,
            journal_dir: args.journal_dir,
            checkpoint_every_records: args.checkpoint_every_records,
            log: args.log.into(),
            tracing: args.tracing.into(),
        },
    )?;
    let node_id = dms_server::NodeId::new(&resolved.node_id)?;
    let logging_guard = init_process_logging(
        &resolved.logging,
        ProcessIdentity::new("dms-meta", resolved.node_id.clone()),
    )?;
    let result = dms_server::meta::serve(dms_server::meta::MetaConfig {
        node_id,
        health_address: resolved.health_address,
        grpc_address: resolved.grpc_address,
        journal_dir: resolved.journal_dir.map(PathBuf::from),
        checkpoint_every_records: resolved.checkpoint_every_records,
        tracing: resolved.tracing,
    });
    if let Err(error) = &result {
        dms_logging::error!(
            "dms-meta stopped with an error";
            "event" => "meta.process.failed",
            "error" => error.to_string(),
        );
    }
    drop(logging_guard);
    result
}

//! Typed startup and online configuration for `dms-node` and `dms-meta`.
//!
//! The two product processes resolve configuration in the same order:
//! built-in defaults < TOML file < explicit CLI flags. The process binaries own
//! CLI parsing; this module owns typed TOML parsing, merge rules and dynamic
//! capability checks. Runtime transports still receive plain value objects such
//! as `GrpcConfig`; they do not read files, environment variables or CLI flags.

use std::{fs, path::Path, str::FromStr, time::Duration};

use dms_logging::{LogFormat, LogOutput, LoggingConfig, OverflowPolicy, parse_level};
use dms_tracing::TracingConfig;
use serde::Deserialize;

pub const DEFAULT_HEALTH_ADDRESS: &str = "127.0.0.1:0";
pub const DEFAULT_ARENA_CAPACITY_BYTES: u64 = 1024 * 1024 * 1024;
pub const DEFAULT_REGION_SIZE_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_STAGING_TTL_MILLIS: u64 = 30_000;
pub const DEFAULT_CLIENT_CACHE_LEASE_TTL_MILLIS: u64 = 1_000;
pub const DEFAULT_NODE_CURRENT_CACHE_BYTES: u64 = 8 * 1024 * 1024;
pub const DEFAULT_NODE_CURRENT_CACHE_TTL_MILLIS: u64 = 1_000;
pub const DEFAULT_META_CHECKPOINT_EVERY_RECORDS: u64 = 4_096;
pub const DEFAULT_LOG_QUEUE_CAPACITY: usize = 10_240;
pub const DEFAULT_LOG_MAX_FILE_SIZE_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_LOG_MAX_BACKUPS: usize = 14;
pub const DEFAULT_LOG_MAX_AGE_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const DEFAULT_TRACE_QUEUE_CAPACITY: usize = 4_096;
pub const DEFAULT_TRACE_MAX_EXPORT_BATCH_SIZE: usize = 512;
pub const DEFAULT_TRACE_BATCH_INTERVAL_MILLIS: u64 = 5_000;
pub const DEFAULT_TRACE_EXPORT_TIMEOUT_MILLIS: u64 = 3_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigFieldCapability {
    Online,
    RestartRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigChange {
    HealthAddress(Option<String>),
    GrpcAddress(Option<String>),
    JournalDir(Option<String>),
    WorkerTcpAddress(Option<String>),
    WorkerUdsPath(Option<String>),
    MetaEndpoint(Option<String>),
    ArenaCapacityBytes(u64),
    RegionSizeBytes(u64),
    StagingTtl(Duration),
    LogLevel(slog::Level),
}

impl ConfigChange {
    #[must_use]
    pub const fn capability(&self) -> ConfigFieldCapability {
        match self {
            Self::StagingTtl(_) | Self::LogLevel(_) => ConfigFieldCapability::Online,
            Self::HealthAddress(_)
            | Self::GrpcAddress(_)
            | Self::JournalDir(_)
            | Self::WorkerTcpAddress(_)
            | Self::WorkerUdsPath(_)
            | Self::MetaEndpoint(_)
            | Self::ArenaCapacityBytes(_)
            | Self::RegionSizeBytes(_) => ConfigFieldCapability::RestartRequired,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file `{path}`: {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid TOML config: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("missing required config field `{0}`")]
    MissingRequired(&'static str),
    #[error("invalid config value `{field}`: {reason}")]
    InvalidValue {
        field: &'static str,
        reason: &'static str,
    },
    #[error("online config change for `{0}` requires restart")]
    RestartRequired(&'static str),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeConfigFile {
    pub node_id: Option<String>,
    pub health_address: Option<String>,
    pub worker_tcp_address: Option<String>,
    pub worker_uds_path: Option<String>,
    pub meta_endpoint: Option<String>,
    pub arena_capacity_bytes: Option<u64>,
    pub region_size_bytes: Option<u64>,
    pub staging_ttl_millis: Option<u64>,
    /// Client Current 缓存资格上限；启动配置，不在线修改已发出的租约。
    pub client_cache_lease_ttl_millis: Option<u64>,
    /// Node 本地 Current 布局缓存预算；只缓存 key->VersionLayout，不缓存 value bytes。
    pub node_current_cache_bytes: Option<u64>,
    /// Node 本地 Current 布局缓存 TTL；重启生效，且不延长 Meta 授权的可见性窗口。
    pub node_current_cache_ttl_millis: Option<u64>,
    #[serde(default)]
    pub log: LoggingConfigFile,
    #[serde(default)]
    pub tracing: TracingConfigFile,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MetaConfigFile {
    pub node_id: Option<String>,
    pub health_address: Option<String>,
    pub grpc_address: Option<String>,
    pub journal_dir: Option<String>,
    /// Meta WAL 每累计多少条新 record 做一次 snapshot 并截断前缀。
    pub checkpoint_every_records: Option<u64>,
    #[serde(default)]
    pub log: LoggingConfigFile,
    #[serde(default)]
    pub tracing: TracingConfigFile,
}

/// TOML shape for process-owned distributed tracing.
///
/// Tracing is disabled unless explicitly enabled. This keeps the business path
/// independent from collector availability and makes the default deployment a
/// true no-export configuration.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct TracingConfigFile {
    pub enabled: Option<bool>,
    pub periodic_operations: Option<bool>,
    pub otlp_endpoint: Option<String>,
    pub sample_ratio: Option<f64>,
    pub queue_capacity: Option<usize>,
    pub max_export_batch_size: Option<usize>,
    pub batch_interval_millis: Option<u64>,
    pub export_timeout_millis: Option<u64>,
}

/// TOML shape shared by the two long-running server processes.
///
/// Human-readable sizes and durations keep operator config concise. CLI flags
/// remain byte/second based so shell automation does not need another parser.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct LoggingConfigFile {
    pub level: Option<String>,
    pub format: Option<String>,
    pub path: Option<String>,
    pub async_queue_capacity: Option<usize>,
    pub overflow: Option<String>,
    pub max_file_size: Option<String>,
    pub max_backups: Option<usize>,
    pub max_age: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoggingCliOverrides {
    pub level: Option<String>,
    pub format: Option<String>,
    pub path: Option<String>,
    pub async_queue_capacity: Option<usize>,
    pub overflow: Option<String>,
    pub max_file_size_bytes: Option<u64>,
    pub max_backups: Option<usize>,
    pub max_age_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TracingCliOverrides {
    pub enabled: Option<bool>,
    pub periodic_operations: Option<bool>,
    pub otlp_endpoint: Option<String>,
    pub sample_ratio: Option<f64>,
    pub queue_capacity: Option<usize>,
    pub max_export_batch_size: Option<usize>,
    pub batch_interval_millis: Option<u64>,
    pub export_timeout_millis: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeCliOverrides {
    pub node_id: Option<String>,
    pub health_address: Option<String>,
    pub worker_tcp_address: Option<String>,
    pub worker_uds_path: Option<String>,
    pub meta_endpoint: Option<String>,
    pub arena_capacity_bytes: Option<u64>,
    pub region_size_bytes: Option<u64>,
    pub staging_ttl_millis: Option<u64>,
    pub client_cache_lease_ttl_millis: Option<u64>,
    pub node_current_cache_bytes: Option<u64>,
    pub node_current_cache_ttl_millis: Option<u64>,
    pub log: LoggingCliOverrides,
    pub tracing: TracingCliOverrides,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetaCliOverrides {
    pub node_id: Option<String>,
    pub health_address: Option<String>,
    pub grpc_address: Option<String>,
    pub journal_dir: Option<String>,
    pub checkpoint_every_records: Option<u64>,
    pub log: LoggingCliOverrides,
    pub tracing: TracingCliOverrides,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedNodeConfig {
    pub node_id: String,
    pub health_address: String,
    pub worker_tcp_address: Option<String>,
    pub worker_uds_path: Option<String>,
    pub meta_endpoint: String,
    pub arena_capacity_bytes: u64,
    pub region_size_bytes: u64,
    pub staging_ttl: Duration,
    pub client_cache_lease_ttl: Duration,
    pub node_current_cache_bytes: u64,
    pub node_current_cache_ttl: Duration,
    pub logging: LoggingConfig,
    pub tracing: TracingConfig,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMetaConfig {
    pub node_id: String,
    pub health_address: String,
    pub grpc_address: String,
    pub journal_dir: Option<String>,
    pub checkpoint_every_records: u64,
    pub logging: LoggingConfig,
    pub tracing: TracingConfig,
}

impl NodeConfigFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
            path: path.display().to_string(),
            source,
        })?;
        Ok(toml::from_str(&contents)?)
    }
}

impl MetaConfigFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
            path: path.display().to_string(),
            source,
        })?;
        Ok(toml::from_str(&contents)?)
    }
}

impl ResolvedNodeConfig {
    pub fn from_sources(file: NodeConfigFile, cli: NodeCliOverrides) -> Result<Self, ConfigError> {
        let node_id =
            pick(cli.node_id, file.node_id).ok_or(ConfigError::MissingRequired("node_id"))?;
        let health_address =
            pick(cli.health_address, file.health_address).unwrap_or_else(default_health_address);
        let worker_tcp_address = pick(cli.worker_tcp_address, file.worker_tcp_address);
        let worker_uds_path = pick(cli.worker_uds_path, file.worker_uds_path);
        if worker_tcp_address.is_none() && worker_uds_path.is_none() {
            return Err(ConfigError::MissingRequired(
                "worker_tcp_address or worker_uds_path",
            ));
        }
        let meta_endpoint = pick(cli.meta_endpoint, file.meta_endpoint)
            .ok_or(ConfigError::MissingRequired("meta_endpoint"))?;
        let arena_capacity_bytes = pick(cli.arena_capacity_bytes, file.arena_capacity_bytes)
            .unwrap_or(DEFAULT_ARENA_CAPACITY_BYTES);
        validate_positive_u64("arena_capacity_bytes", arena_capacity_bytes)?;
        let region_size_bytes = pick(cli.region_size_bytes, file.region_size_bytes)
            .unwrap_or(DEFAULT_REGION_SIZE_BYTES);
        if region_size_bytes < 64 || region_size_bytes.checked_add(63).is_none() {
            return Err(ConfigError::InvalidValue {
                field: "region_size_bytes",
                reason: "must be at least 64 bytes and allow 64-byte alignment",
            });
        }
        let staging_ttl_millis = pick(cli.staging_ttl_millis, file.staging_ttl_millis)
            .unwrap_or(DEFAULT_STAGING_TTL_MILLIS);
        validate_positive_u64("staging_ttl_millis", staging_ttl_millis)?;
        let cache_ttl = pick(
            cli.client_cache_lease_ttl_millis,
            file.client_cache_lease_ttl_millis,
        )
        .unwrap_or(DEFAULT_CLIENT_CACHE_LEASE_TTL_MILLIS);
        validate_positive_u64("client_cache_lease_ttl_millis", cache_ttl)?;
        if cache_ttl > 30_000 {
            return Err(ConfigError::InvalidValue {
                field: "client_cache_lease_ttl_millis",
                reason: "must be at most 30000; actual grant is also capped by the upstream lease",
            });
        }
        let node_current_cache_bytes =
            pick(cli.node_current_cache_bytes, file.node_current_cache_bytes)
                .unwrap_or(DEFAULT_NODE_CURRENT_CACHE_BYTES);
        let node_current_cache_ttl_millis = pick(
            cli.node_current_cache_ttl_millis,
            file.node_current_cache_ttl_millis,
        )
        .unwrap_or(DEFAULT_NODE_CURRENT_CACHE_TTL_MILLIS);
        validate_positive_u64(
            "node_current_cache_ttl_millis",
            node_current_cache_ttl_millis,
        )?;
        if node_current_cache_ttl_millis > 30_000 {
            return Err(ConfigError::InvalidValue {
                field: "node_current_cache_ttl_millis",
                reason: "must be at most 30000",
            });
        }
        let logging = resolve_logging(file.log, cli.log)?;
        let tracing = resolve_tracing(file.tracing, cli.tracing)?;
        Ok(Self {
            node_id,
            health_address,
            worker_tcp_address,
            worker_uds_path,
            meta_endpoint,
            arena_capacity_bytes,
            region_size_bytes,
            staging_ttl: Duration::from_millis(staging_ttl_millis),
            client_cache_lease_ttl: Duration::from_millis(cache_ttl),
            node_current_cache_bytes,
            node_current_cache_ttl: Duration::from_millis(node_current_cache_ttl_millis),
            logging,
            tracing,
        })
    }
}

impl ResolvedMetaConfig {
    pub fn from_sources(file: MetaConfigFile, cli: MetaCliOverrides) -> Result<Self, ConfigError> {
        let node_id =
            pick(cli.node_id, file.node_id).ok_or(ConfigError::MissingRequired("node_id"))?;
        let health_address =
            pick(cli.health_address, file.health_address).unwrap_or_else(default_health_address);
        let grpc_address = pick(cli.grpc_address, file.grpc_address)
            .ok_or(ConfigError::MissingRequired("grpc_address"))?;
        let checkpoint_every_records =
            pick(cli.checkpoint_every_records, file.checkpoint_every_records)
                .unwrap_or(DEFAULT_META_CHECKPOINT_EVERY_RECORDS);
        validate_positive_u64("checkpoint_every_records", checkpoint_every_records)?;
        let logging = resolve_logging(file.log, cli.log)?;
        let tracing = resolve_tracing(file.tracing, cli.tracing)?;
        Ok(Self {
            node_id,
            health_address,
            grpc_address,
            journal_dir: pick(cli.journal_dir, file.journal_dir),
            checkpoint_every_records,
            logging,
            tracing,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnlineConfigController {
    staging_ttl: Duration,
    log_level: slog::Level,
    version: u64,
}

impl OnlineConfigController {
    pub const fn new(staging_ttl: Duration) -> Self {
        Self {
            staging_ttl,
            log_level: slog::Level::Info,
            version: 1,
        }
    }

    pub fn apply(&mut self, change: ConfigChange) -> Result<u64, ConfigError> {
        if change.capability() == ConfigFieldCapability::RestartRequired {
            return Err(ConfigError::RestartRequired(change_name(&change)));
        }
        match change {
            ConfigChange::StagingTtl(ttl) if !ttl.is_zero() => {
                self.staging_ttl = ttl;
                self.version += 1;
                Ok(self.version)
            }
            ConfigChange::StagingTtl(_) => Err(ConfigError::InvalidValue {
                field: "staging_ttl",
                reason: "must be positive",
            }),
            ConfigChange::LogLevel(level) => {
                self.log_level = level;
                self.version += 1;
                Ok(self.version)
            }
            _ => unreachable!("restart-required changes returned above"),
        }
    }

    pub const fn staging_ttl(&self) -> Duration {
        self.staging_ttl
    }

    pub const fn version(&self) -> u64 {
        self.version
    }

    pub const fn log_level(&self) -> slog::Level {
        self.log_level
    }
}

fn resolve_logging(
    file: LoggingConfigFile,
    cli: LoggingCliOverrides,
) -> Result<LoggingConfig, ConfigError> {
    let level_text = pick(cli.level, file.level).unwrap_or_else(|| "info".to_string());
    let level = parse_level(&level_text).map_err(|_| ConfigError::InvalidValue {
        field: "log.level",
        reason: "must be trace/debug/info/warn/error/critical",
    })?;
    let format_text = pick(cli.format, file.format).unwrap_or_else(|| "json".to_string());
    let format = LogFormat::from_str(&format_text).map_err(|_| ConfigError::InvalidValue {
        field: "log.format",
        reason: "must be text or json",
    })?;
    let overflow_text =
        pick(cli.overflow, file.overflow).unwrap_or_else(|| "drop-and-report".to_string());
    let overflow =
        OverflowPolicy::from_str(&overflow_text).map_err(|_| ConfigError::InvalidValue {
            field: "log.overflow",
            reason: "must be drop-and-report or block",
        })?;
    let async_queue_capacity = pick(cli.async_queue_capacity, file.async_queue_capacity)
        .unwrap_or(DEFAULT_LOG_QUEUE_CAPACITY);
    if async_queue_capacity == 0 {
        return Err(ConfigError::InvalidValue {
            field: "log.async-queue-capacity",
            reason: "must be positive",
        });
    }
    let max_file_size = match cli.max_file_size_bytes {
        Some(bytes) => bytes,
        None => file
            .max_file_size
            .as_deref()
            .map(parse_byte_size)
            .transpose()?
            .unwrap_or(DEFAULT_LOG_MAX_FILE_SIZE_BYTES),
    };
    validate_positive_u64("log.max-file-size", max_file_size)?;
    let max_backups = pick(cli.max_backups, file.max_backups).unwrap_or(DEFAULT_LOG_MAX_BACKUPS);
    let max_age_seconds = match cli.max_age_seconds {
        Some(seconds) => seconds,
        None => file
            .max_age
            .as_deref()
            .map(parse_duration_seconds)
            .transpose()?
            .unwrap_or(DEFAULT_LOG_MAX_AGE_SECONDS),
    };
    validate_positive_u64("log.max-age", max_age_seconds)?;
    Ok(LoggingConfig {
        level,
        format,
        output: pick(cli.path, file.path)
            .map(|path| LogOutput::File(path.into()))
            .unwrap_or(LogOutput::Stderr),
        async_queue_capacity,
        overflow,
        max_file_size,
        max_backups,
        max_age: Duration::from_secs(max_age_seconds),
    })
}

fn resolve_tracing(
    file: TracingConfigFile,
    cli: TracingCliOverrides,
) -> Result<TracingConfig, ConfigError> {
    let mut config = TracingConfig {
        enabled: pick(cli.enabled, file.enabled).unwrap_or(false),
        periodic_operations: pick(cli.periodic_operations, file.periodic_operations)
            .unwrap_or(false),
        otlp_endpoint: pick(cli.otlp_endpoint, file.otlp_endpoint)
            .unwrap_or_else(|| "http://127.0.0.1:4317".to_string()),
        sample_ratio: pick(cli.sample_ratio, file.sample_ratio).unwrap_or(0.01),
        queue_capacity: pick(cli.queue_capacity, file.queue_capacity)
            .unwrap_or(DEFAULT_TRACE_QUEUE_CAPACITY),
        max_export_batch_size: pick(cli.max_export_batch_size, file.max_export_batch_size)
            .unwrap_or(DEFAULT_TRACE_MAX_EXPORT_BATCH_SIZE),
        batch_interval: Duration::from_millis(
            pick(cli.batch_interval_millis, file.batch_interval_millis)
                .unwrap_or(DEFAULT_TRACE_BATCH_INTERVAL_MILLIS),
        ),
        export_timeout: Duration::from_millis(
            pick(cli.export_timeout_millis, file.export_timeout_millis)
                .unwrap_or(DEFAULT_TRACE_EXPORT_TIMEOUT_MILLIS),
        ),
    };
    config.validate().map_err(|_| ConfigError::InvalidValue {
        field: "tracing",
        reason: "contains an invalid ratio, queue, batch, or timeout value",
    })?;
    // Normalize an accidental trailing slash so operator-visible configuration
    // and exporter diagnostics use one stable endpoint spelling.
    while config.otlp_endpoint.ends_with('/') {
        config.otlp_endpoint.pop();
    }
    Ok(config)
}

fn parse_byte_size(value: &str) -> Result<u64, ConfigError> {
    parse_scaled_u64(
        "log.max-file-size",
        value,
        &[
            ("GiB", 1024_u64.pow(3)),
            ("MiB", 1024_u64.pow(2)),
            ("KiB", 1024),
            ("B", 1),
        ],
    )
}

fn parse_duration_seconds(value: &str) -> Result<u64, ConfigError> {
    parse_scaled_u64(
        "log.max-age",
        value,
        &[("d", 24 * 60 * 60), ("h", 60 * 60), ("m", 60), ("s", 1)],
    )
}

fn parse_scaled_u64(
    field: &'static str,
    value: &str,
    units: &[(&str, u64)],
) -> Result<u64, ConfigError> {
    let value = value.trim();
    for (suffix, multiplier) in units {
        if let Some(number) = value.strip_suffix(suffix) {
            let number = number
                .parse::<u64>()
                .map_err(|_| ConfigError::InvalidValue {
                    field,
                    reason: "contains an invalid number",
                })?;
            return number
                .checked_mul(*multiplier)
                .ok_or(ConfigError::InvalidValue {
                    field,
                    reason: "is too large",
                });
        }
    }
    value.parse::<u64>().map_err(|_| ConfigError::InvalidValue {
        field,
        reason: "uses an unsupported unit",
    })
}

fn pick<T>(cli: Option<T>, file: Option<T>) -> Option<T> {
    cli.or(file)
}

fn default_health_address() -> String {
    DEFAULT_HEALTH_ADDRESS.to_string()
}

fn validate_positive_u64(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value == 0 {
        return Err(ConfigError::InvalidValue {
            field,
            reason: "must be positive",
        });
    }
    Ok(())
}

fn change_name(change: &ConfigChange) -> &'static str {
    match change {
        ConfigChange::HealthAddress(_) => "health_address",
        ConfigChange::GrpcAddress(_) => "grpc_address",
        ConfigChange::JournalDir(_) => "journal_dir",
        ConfigChange::WorkerTcpAddress(_) => "worker_tcp_address",
        ConfigChange::WorkerUdsPath(_) => "worker_uds_path",
        ConfigChange::MetaEndpoint(_) => "meta_endpoint",
        ConfigChange::ArenaCapacityBytes(_) => "arena_capacity_bytes",
        ConfigChange::RegionSizeBytes(_) => "region_size_bytes",
        ConfigChange::StagingTtl(_) => "staging_ttl",
        ConfigChange::LogLevel(_) => "log.level",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_config_uses_default_file_cli_precedence() {
        let file = toml::from_str::<NodeConfigFile>(
            r#"
            node_id = "from-file"
            worker_tcp_address = "127.0.0.1:19001"
            meta_endpoint = "http://127.0.0.1:19300"
            arena_capacity_bytes = 4096
            staging_ttl_millis = 3000
            "#,
        )
        .expect("file config");
        let cli = NodeCliOverrides {
            node_id: Some("from-cli".to_string()),
            staging_ttl_millis: Some(7000),
            ..NodeCliOverrides::default()
        };

        let resolved = ResolvedNodeConfig::from_sources(file, cli).expect("resolve");

        assert_eq!(resolved.node_id, "from-cli");
        assert_eq!(resolved.health_address, DEFAULT_HEALTH_ADDRESS);
        assert_eq!(
            resolved.worker_tcp_address.as_deref(),
            Some("127.0.0.1:19001")
        );
        assert_eq!(resolved.arena_capacity_bytes, 4096);
        assert_eq!(resolved.staging_ttl, Duration::from_millis(7000));
        assert_eq!(resolved.region_size_bytes, DEFAULT_REGION_SIZE_BYTES);
    }

    #[test]
    fn region_size_configuration_is_bounded_and_cli_overrides_file() {
        let base = NodeConfigFile {
            node_id: Some("node".into()),
            worker_tcp_address: Some("127.0.0.1:19200".into()),
            meta_endpoint: Some("http://127.0.0.1:19300".into()),
            region_size_bytes: Some(4096),
            ..NodeConfigFile::default()
        };
        assert_eq!(
            ResolvedNodeConfig::from_sources(base.clone(), NodeCliOverrides::default())
                .unwrap()
                .region_size_bytes,
            4096
        );
        assert_eq!(
            ResolvedNodeConfig::from_sources(
                base.clone(),
                NodeCliOverrides {
                    region_size_bytes: Some(8192),
                    ..NodeCliOverrides::default()
                }
            )
            .unwrap()
            .region_size_bytes,
            8192
        );
        for size in [0, 63, u64::MAX] {
            assert!(
                ResolvedNodeConfig::from_sources(
                    base.clone(),
                    NodeCliOverrides {
                        region_size_bytes: Some(size),
                        ..NodeCliOverrides::default()
                    }
                )
                .is_err()
            );
        }
        assert_eq!(
            ConfigChange::RegionSizeBytes(8192).capability(),
            ConfigFieldCapability::RestartRequired
        );
    }

    #[test]
    fn client_cache_lease_uses_default_file_cli_and_rejects_invalid_limits() {
        let base = NodeConfigFile {
            node_id: Some("test-node".into()),
            worker_tcp_address: Some("127.0.0.1:19200".into()),
            meta_endpoint: Some("http://127.0.0.1:19300".into()),
            ..NodeConfigFile::default()
        };
        let resolve = |file, cli| ResolvedNodeConfig::from_sources(file, cli);
        assert_eq!(
            resolve(base.clone(), NodeCliOverrides::default())
                .unwrap()
                .client_cache_lease_ttl,
            Duration::from_millis(1000)
        );
        let file = NodeConfigFile {
            client_cache_lease_ttl_millis: Some(500),
            ..base.clone()
        };
        assert_eq!(
            resolve(file.clone(), NodeCliOverrides::default())
                .unwrap()
                .client_cache_lease_ttl,
            Duration::from_millis(500)
        );
        let cli = NodeCliOverrides {
            client_cache_lease_ttl_millis: Some(750),
            ..NodeCliOverrides::default()
        };
        assert_eq!(
            resolve(file, cli).unwrap().client_cache_lease_ttl,
            Duration::from_millis(750)
        );
        for invalid in [0, 30001] {
            assert!(
                resolve(
                    base.clone(),
                    NodeCliOverrides {
                        client_cache_lease_ttl_millis: Some(invalid),
                        ..NodeCliOverrides::default()
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn node_current_cache_uses_default_file_cli_and_rejects_invalid_limits() {
        let base = NodeConfigFile {
            node_id: Some("test-node".into()),
            worker_tcp_address: Some("127.0.0.1:19200".into()),
            meta_endpoint: Some("http://127.0.0.1:19300".into()),
            ..NodeConfigFile::default()
        };
        let resolve = |file, cli| ResolvedNodeConfig::from_sources(file, cli);
        let defaults = resolve(base.clone(), NodeCliOverrides::default()).unwrap();
        assert_eq!(
            defaults.node_current_cache_bytes,
            DEFAULT_NODE_CURRENT_CACHE_BYTES
        );
        assert_eq!(
            defaults.node_current_cache_ttl,
            Duration::from_millis(DEFAULT_NODE_CURRENT_CACHE_TTL_MILLIS)
        );

        let file = NodeConfigFile {
            node_current_cache_bytes: Some(0),
            node_current_cache_ttl_millis: Some(500),
            ..base.clone()
        };
        let from_file = resolve(file.clone(), NodeCliOverrides::default()).unwrap();
        assert_eq!(from_file.node_current_cache_bytes, 0);
        assert_eq!(from_file.node_current_cache_ttl, Duration::from_millis(500));

        let cli = NodeCliOverrides {
            node_current_cache_bytes: Some(16 * 1024 * 1024),
            node_current_cache_ttl_millis: Some(750),
            ..NodeCliOverrides::default()
        };
        let from_cli = resolve(file, cli).unwrap();
        assert_eq!(from_cli.node_current_cache_bytes, 16 * 1024 * 1024);
        assert_eq!(from_cli.node_current_cache_ttl, Duration::from_millis(750));

        for invalid in [0, 30_001] {
            assert!(
                resolve(
                    base.clone(),
                    NodeCliOverrides {
                        node_current_cache_ttl_millis: Some(invalid),
                        ..NodeCliOverrides::default()
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn meta_config_uses_file_when_cli_is_unspecified() {
        let file = toml::from_str::<MetaConfigFile>(
            r#"
            node_id = "meta-file"
            grpc_address = "127.0.0.1:19300"
            journal_dir = "/tmp/dms-meta"
            checkpoint_every_records = 128
            "#,
        )
        .expect("file config");

        let resolved =
            ResolvedMetaConfig::from_sources(file, MetaCliOverrides::default()).expect("resolve");

        assert_eq!(resolved.node_id, "meta-file");
        assert_eq!(resolved.health_address, DEFAULT_HEALTH_ADDRESS);
        assert_eq!(resolved.grpc_address, "127.0.0.1:19300");
        assert_eq!(resolved.journal_dir.as_deref(), Some("/tmp/dms-meta"));
        assert_eq!(resolved.checkpoint_every_records, 128);
    }

    #[test]
    fn meta_checkpoint_records_use_default_file_cli_and_reject_zero() {
        let base = MetaConfigFile {
            node_id: Some("meta".into()),
            grpc_address: Some("127.0.0.1:19300".into()),
            ..MetaConfigFile::default()
        };
        assert_eq!(
            ResolvedMetaConfig::from_sources(base.clone(), MetaCliOverrides::default())
                .unwrap()
                .checkpoint_every_records,
            DEFAULT_META_CHECKPOINT_EVERY_RECORDS
        );
        let file = MetaConfigFile {
            checkpoint_every_records: Some(256),
            ..base.clone()
        };
        assert_eq!(
            ResolvedMetaConfig::from_sources(file.clone(), MetaCliOverrides::default())
                .unwrap()
                .checkpoint_every_records,
            256
        );
        assert_eq!(
            ResolvedMetaConfig::from_sources(
                file,
                MetaCliOverrides {
                    checkpoint_every_records: Some(512),
                    ..MetaCliOverrides::default()
                }
            )
            .unwrap()
            .checkpoint_every_records,
            512
        );
        assert!(
            ResolvedMetaConfig::from_sources(
                base,
                MetaCliOverrides {
                    checkpoint_every_records: Some(0),
                    ..MetaCliOverrides::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn toml_unknown_fields_are_rejected() {
        let error = toml::from_str::<NodeConfigFile>(
            r#"
            node_id = "node-a"
            worker_tcp_address = "127.0.0.1:19001"
            meta_endpoint = "http://127.0.0.1:19300"
            surprise = true
            "#,
        )
        .expect_err("unknown field");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn invalid_values_are_rejected() {
        let file = NodeConfigFile {
            node_id: Some("node-a".to_string()),
            worker_tcp_address: Some("127.0.0.1:19001".to_string()),
            meta_endpoint: Some("http://127.0.0.1:19300".to_string()),
            arena_capacity_bytes: Some(0),
            ..NodeConfigFile::default()
        };

        let error = ResolvedNodeConfig::from_sources(file, NodeCliOverrides::default())
            .expect_err("invalid capacity");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                field: "arena_capacity_bytes",
                ..
            }
        ));
    }

    #[test]
    fn online_controller_rejects_restart_required_fields() {
        let mut controller = OnlineConfigController::new(Duration::from_secs(30));
        let error = controller
            .apply(ConfigChange::GrpcAddress(Some("127.0.0.1:1".to_string())))
            .expect_err("restart required");

        assert!(matches!(
            error,
            ConfigError::RestartRequired("grpc_address")
        ));
        assert_eq!(controller.version(), 1);
    }

    #[test]
    fn online_controller_applies_dynamic_staging_ttl() {
        let mut controller = OnlineConfigController::new(Duration::from_secs(30));
        let version = controller
            .apply(ConfigChange::StagingTtl(Duration::from_millis(5)))
            .expect("online change");

        assert_eq!(version, 2);
        assert_eq!(controller.staging_ttl(), Duration::from_millis(5));
    }

    #[test]
    fn logging_config_uses_defaults_file_and_cli_precedence() {
        let file = toml::from_str::<MetaConfigFile>(
            r#"
            node_id = "meta-a"
            grpc_address = "127.0.0.1:19300"

            [log]
            level = "debug"
            format = "text"
            path = "/tmp/from-file.log"
            async-queue-capacity = 512
            overflow = "block"
            max-file-size = "2MiB"
            max-backups = 3
            max-age = "2h"
            "#,
        )
        .expect("file config");
        let resolved = ResolvedMetaConfig::from_sources(
            file,
            MetaCliOverrides {
                log: LoggingCliOverrides {
                    level: Some("warn".to_string()),
                    path: Some("/tmp/from-cli.log".to_string()),
                    ..LoggingCliOverrides::default()
                },
                ..MetaCliOverrides::default()
            },
        )
        .expect("resolved");

        assert_eq!(resolved.logging.level, slog::Level::Warning);
        assert_eq!(resolved.logging.format, LogFormat::Text);
        assert_eq!(
            resolved.logging.output,
            LogOutput::File("/tmp/from-cli.log".into())
        );
        assert_eq!(resolved.logging.async_queue_capacity, 512);
        assert_eq!(resolved.logging.overflow, OverflowPolicy::Block);
        assert_eq!(resolved.logging.max_file_size, 2 * 1024 * 1024);
        assert_eq!(resolved.logging.max_backups, 3);
        assert_eq!(resolved.logging.max_age, Duration::from_secs(2 * 60 * 60));
    }

    #[test]
    fn periodic_trace_policy_defaults_off_and_cli_overrides_file() {
        let defaults = ResolvedMetaConfig::from_sources(
            MetaConfigFile {
                node_id: Some("meta-default".to_string()),
                grpc_address: Some("127.0.0.1:19300".to_string()),
                ..MetaConfigFile::default()
            },
            MetaCliOverrides::default(),
        )
        .expect("default tracing config");
        assert!(!defaults.tracing.periodic_operations);

        let file = toml::from_str::<MetaConfigFile>(
            r#"
            node_id = "meta-file"
            grpc_address = "127.0.0.1:19300"

            [tracing]
            periodic-operations = true
            "#,
        )
        .expect("file tracing config");
        let resolved = ResolvedMetaConfig::from_sources(
            file,
            MetaCliOverrides {
                tracing: TracingCliOverrides {
                    periodic_operations: Some(false),
                    ..TracingCliOverrides::default()
                },
                ..MetaCliOverrides::default()
            },
        )
        .expect("CLI tracing override");
        assert!(!resolved.tracing.periodic_operations);
    }

    #[test]
    fn online_controller_applies_log_level() {
        let mut controller = OnlineConfigController::new(Duration::from_secs(30));
        let version = controller
            .apply(ConfigChange::LogLevel(slog::Level::Debug))
            .expect("online log level");

        assert_eq!(version, 2);
        assert_eq!(controller.log_level(), slog::Level::Debug);
    }
}

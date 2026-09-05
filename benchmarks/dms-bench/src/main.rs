//! 通过公开 Rust SDK 测量 Client → Node → Meta 的进程级路径。
//!
//! 保留固定案例和三轮时间窗口；所有案例校验精确数据。写回读排除在
//! 主操作计时外，但仍计入全程进程资源和服务指标。没有插桩的 RPC/copy
//! 不输出虚构数值，未实现的传输能力也不以仿真数字替代。

#![forbid(unsafe_code)]

use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use dms_client::{
    ClientOptions, DmsClient, HashEntry, HashScanOptions, HashWriteOptions, KvEntry, MSetOptions,
    RangeWriteOptions, ScanCursor,
};
use dms_tracing::{ProcessIdentity, TracingConfig, init_process_tracing};

const DEFAULT_OPS: usize = 200;
const DEFAULT_WARMUP_OPS: usize = 20;
const DEFAULT_INLINE_BYTES: usize = 1024;
const DEFAULT_STAGED_BYTES: usize = 80 * 1024;
const DEFAULT_BATCH_SIZE: usize = 8;
const DEFAULT_HASH_FIELDS: usize = 16;

type BenchResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> BenchResult<()> {
    let config = BenchConfig::parse()?;
    // 基准进程仅在显式开启时安装 tracing，以同一产物对照关闭和采样路径。
    let _tracing_guard = init_process_tracing(
        &TracingConfig {
            enabled: env_bool("DMS_TRACING_ENABLED", false),
            otlp_endpoint: env::var("DMS_TRACING_OTLP_ENDPOINT")
                .unwrap_or_else(|_| "http://127.0.0.1:4317".to_string()),
            sample_ratio: env::var("DMS_TRACING_SAMPLE_RATIO")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1.0),
            ..TracingConfig::default()
        },
        ProcessIdentity::new("dms-client", "dms-bench"),
        None,
    )?;
    let started_unix_millis = unix_millis();
    let before = ProcessSample::collect();

    let cases = vec![
        run_case(
            &config,
            "set_inline",
            "native_worker_set_inline",
            config.inline_bytes,
            |context| {
                let key = context.key("inline");
                context.client.set(key, &context.value)
            },
        ),
        run_case(
            &config,
            "set_staged_grpc",
            "native_worker_allocate_upload_set",
            config.staged_bytes,
            |context| {
                let key = context.key("staged");
                context.client.set(key, &context.value)
            },
        ),
        run_get_case(&config),
        run_range_case(&config),
        run_mset_case(&config),
        run_mget_case(&config),
        run_hset_case(&config),
        run_hget_case(&config),
        run_hscan_case(&config),
    ];

    let after = ProcessSample::collect();
    let document = BenchDocument {
        generated_unix_millis: started_unix_millis,
        endpoint: config.endpoint.clone(),
        config: config.clone(),
        process_before: before,
        process_after: after,
        cases,
    };
    let json = document.to_json();
    if let Some(path) = &config.output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json.as_bytes())?;
        println!("{}", path.display());
    } else {
        println!("{json}");
    }
    if document
        .cases
        .iter()
        .flat_map(|case| &case.runs)
        .any(|run| run.errors + run.warmup_errors > 0)
    {
        return Err("benchmark operation or exact-data validation failed; see JSON".into());
    }
    Ok(())
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .and_then(|value| match value.as_str() {
            "1" | "true" | "TRUE" | "yes" | "YES" => Some(true),
            "0" | "false" | "FALSE" | "no" | "NO" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

#[derive(Clone)]
struct BenchConfig {
    endpoint: String,
    output: Option<PathBuf>,
    ops: usize,
    warmup_ops: usize,
    inline_bytes: usize,
    staged_bytes: usize,
    batch_size: usize,
    hash_fields: usize,
}

impl BenchConfig {
    fn parse() -> BenchResult<Self> {
        let mut config = Self {
            endpoint: env::var("DMS_ENDPOINT")
                .unwrap_or_else(|_| "http://127.0.0.1:19200".to_string()),
            output: None,
            ops: DEFAULT_OPS,
            warmup_ops: DEFAULT_WARMUP_OPS,
            inline_bytes: DEFAULT_INLINE_BYTES,
            staged_bytes: DEFAULT_STAGED_BYTES,
            batch_size: DEFAULT_BATCH_SIZE,
            hash_fields: DEFAULT_HASH_FIELDS,
        };
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--endpoint" => config.endpoint = next_value(&mut args, "--endpoint")?,
                "--output" => {
                    config.output = Some(PathBuf::from(next_value(&mut args, "--output")?))
                }
                "--ops" => config.ops = parse_usize(&mut args, "--ops")?,
                "--warmup-ops" => config.warmup_ops = parse_usize(&mut args, "--warmup-ops")?,
                "--inline-bytes" => config.inline_bytes = parse_usize(&mut args, "--inline-bytes")?,
                "--staged-bytes" => config.staged_bytes = parse_usize(&mut args, "--staged-bytes")?,
                "--batch-size" => config.batch_size = parse_usize(&mut args, "--batch-size")?,
                "--hash-fields" => config.hash_fields = parse_usize(&mut args, "--hash-fields")?,
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument `{other}`").into()),
            }
        }
        if config.ops == 0 || config.warmup_ops == 0 {
            return Err("--ops and --warmup-ops must be positive".into());
        }
        if config.inline_bytes == 0 || config.staged_bytes == 0 {
            return Err("--inline-bytes and --staged-bytes must be positive".into());
        }
        if config.batch_size == 0 || config.hash_fields == 0 {
            return Err("--batch-size and --hash-fields must be positive".into());
        }
        Ok(config)
    }
}

fn next_value(args: &mut impl Iterator<Item = String>, name: &'static str) -> BenchResult<String> {
    args.next()
        .ok_or_else(|| format!("{name} requires a value").into())
}

fn parse_usize(args: &mut impl Iterator<Item = String>, name: &'static str) -> BenchResult<usize> {
    let value = next_value(args, name)?;
    value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be a positive integer").into())
}

fn print_help() {
    println!(
        "usage: dms-bench [--endpoint URI] [--output PATH] [--ops N] [--warmup-ops N] [--inline-bytes N] [--staged-bytes N] [--batch-size N] [--hash-fields N]"
    );
}

struct CaseContext {
    client: DmsClient,
    endpoint: String,
    case_name: &'static str,
    run_index: usize,
    op_index: usize,
    value: Vec<u8>,
}

impl CaseContext {
    fn key(&self, suffix: &str) -> String {
        format!(
            "bench/{}/{}/{}/{}",
            self.case_name, self.run_index, self.op_index, suffix
        )
    }
}

fn run_case<T>(
    config: &BenchConfig,
    name: &'static str,
    implementation: &'static str,
    value_len: usize,
    operation: impl Fn(&CaseContext) -> Result<T, dms_client::DmsError>,
) -> CaseReport {
    let samples = measured_runs(config, name, value_len, |context| {
        operation(context).map(|_| ())
    });
    CaseReport::available(
        name,
        implementation,
        value_len,
        estimate_steps(name, config),
        samples,
    )
}

fn run_get_case(config: &BenchConfig) -> CaseReport {
    let samples = measured_runs_with_prepare(
        config,
        "get_current_uncached",
        config.inline_bytes,
        |context| {
            context
                .client
                .set(context.key("get"), &context.value)
                .map(|_| ())
        },
        |context| {
            let key = context.key("get");
            let reader = connect(&context.endpoint).map_err(|error| {
                dms_client::DmsError::client_protocol_violation(error.to_string())
            })?;
            match reader.get(&key)? {
                Some(value) if value == context.value => Ok(()),
                Some(_) => Err(dms_client::DmsError::node_transfer_corrupt_data(
                    "DMS data is corrupt",
                )),
                None => Err(dms_client::DmsError::node_object_not_found(
                    "DMS object not found",
                )),
            }
        },
    );
    CaseReport::available(
        "get_current_uncached",
        "native_worker_get_download_with_fresh_reader",
        config.inline_bytes,
        estimate_steps("get_current_uncached", config),
        samples,
    )
}

fn run_range_case(config: &BenchConfig) -> CaseReport {
    let samples = measured_runs_with_prepare(
        config,
        "set_range",
        config.inline_bytes,
        |context| {
            context
                .client
                .set(context.key("range"), &context.value)
                .map(|_| ())
        },
        |context| {
            let patch = vec![0x52; context.value.len().min(32)];
            let offset = context.value.len().saturating_sub(patch.len()) / 2;
            context
                .client
                .set_range_with_options(
                    context.key("range"),
                    offset as u64,
                    &patch,
                    RangeWriteOptions::default(),
                )
                .map(|_| ())
        },
    );
    CaseReport::available(
        "set_range",
        "node_native_immutable_extent_overlay",
        config.inline_bytes,
        estimate_steps("set_range", config),
        samples,
    )
}

fn run_mset_case(config: &BenchConfig) -> CaseReport {
    let samples = measured_runs(config, "mset", config.inline_bytes, |context| {
        let mut entries = Vec::with_capacity(config.batch_size);
        for item in 0..config.batch_size {
            entries.push(
                KvEntry::new(
                    format!("{}-{item}", context.key("mset")),
                    context.value.clone(),
                )
                .map_err(|error| {
                    dms_client::DmsError::client_invalid_argument(error.to_string())
                })?,
            );
        }
        context
            .client
            .mset(&entries, MSetOptions::default())
            .map(|_| ())
    });
    CaseReport::available(
        "mset",
        "node_staging_plus_single_meta_atomic_batch",
        config.inline_bytes * config.batch_size,
        estimate_steps("mset", config),
        samples,
    )
}

fn run_mget_case(config: &BenchConfig) -> CaseReport {
    let samples = measured_runs_with_prepare(
        config,
        "mget",
        config.inline_bytes,
        |context| {
            for item in 0..config.batch_size {
                context
                    .client
                    .set(format!("{}-{item}", context.key("mget")), &context.value)?;
            }
            Ok(())
        },
        |context| {
            let keys = (0..config.batch_size)
                .map(|item| format!("{}-{item}", context.key("mget")))
                .collect::<Vec<_>>();
            let key_refs = keys.iter().map(String::as_str).collect::<Vec<_>>();
            let values = context.client.mget(&key_refs)?;
            validate_batch(&values, config.batch_size, &context.value)
        },
    );
    CaseReport::available(
        "mget",
        "worker_batch_resolve_and_payload_download",
        config.inline_bytes * config.batch_size,
        estimate_steps("mget", config),
        samples,
    )
}

fn run_hset_case(config: &BenchConfig) -> CaseReport {
    let samples = measured_runs(config, "hset", config.inline_bytes, |context| {
        let entries = hash_entries(config.hash_fields, &context.value)?;
        context
            .client
            .hset(context.key("hash"), &entries, HashWriteOptions::default())
            .map(|_| ())
    });
    CaseReport::available(
        "hset",
        "worker_hash_read_modify_cas",
        config.inline_bytes * config.hash_fields,
        estimate_steps("hset", config),
        samples,
    )
}

fn run_hget_case(config: &BenchConfig) -> CaseReport {
    let samples = measured_runs_with_prepare(
        config,
        "hget",
        config.inline_bytes,
        |context| {
            let entries = hash_entries(config.hash_fields, &context.value)?;
            context
                .client
                .hset(context.key("hget"), &entries, HashWriteOptions::default())
                .map(|_| ())
        },
        |context| {
            let field = format!("field-{:04}", context.op_index % config.hash_fields);
            match context.client.hget(context.key("hget"), field.as_bytes())? {
                Some(value)
                    if value.bytes == context.value
                        && value.field.as_bytes() == field.as_bytes() =>
                {
                    Ok(())
                }
                Some(_) => Err(dms_client::DmsError::node_transfer_corrupt_data(
                    "DMS data is corrupt",
                )),
                None => Err(dms_client::DmsError::node_object_not_found(
                    "DMS object not found",
                )),
            }
        },
    );
    CaseReport::available(
        "hget",
        "worker_hash_materialize_and_field_lookup",
        config.inline_bytes,
        estimate_steps("hget", config),
        samples,
    )
}

fn run_hscan_case(config: &BenchConfig) -> CaseReport {
    let samples = measured_runs_with_prepare(
        config,
        "hscan",
        config.inline_bytes,
        |context| {
            let entries = hash_entries(config.hash_fields, &context.value)?;
            context
                .client
                .hset(context.key("hscan"), &entries, HashWriteOptions::default())
                .map(|_| ())
        },
        |context| {
            let result = context.client.hscan(
                context.key("hscan"),
                ScanCursor(0),
                HashScanOptions {
                    limit: config.hash_fields.min(8),
                    ..HashScanOptions::default()
                },
            )?;
            let count = config.hash_fields.min(8);
            validate_hash_entries(&result.entries, count, &context.value)?;
            if (result.next_cursor.0 == 0) != (count == config.hash_fields) {
                return Err(corrupt("HSCAN cursor does not match remaining fields"));
            }
            Ok(())
        },
    );
    CaseReport::available(
        "hscan",
        "worker_hash_materialize_and_ordered_scan",
        config.inline_bytes * config.hash_fields.min(8),
        estimate_steps("hscan", config),
        samples,
    )
}

fn hash_entries(count: usize, value: &[u8]) -> Result<Vec<HashEntry>, dms_client::DmsError> {
    (0..count)
        .map(|index| {
            HashEntry::new(format!("field-{index:04}"), value.to_vec())
                .map_err(|error| dms_client::DmsError::client_invalid_argument(error.to_string()))
        })
        .collect()
}

fn corrupt(message: &str) -> dms_client::DmsError {
    dms_client::DmsError::node_transfer_corrupt_data(message)
}

fn validate_values(
    values: &[Option<Vec<u8>>],
    count: usize,
    expected: &[u8],
) -> Result<(), dms_client::DmsError> {
    if values.len() != count
        || values
            .iter()
            .any(|value| value.as_deref() != Some(expected))
    {
        return Err(corrupt("value count, presence or exact bytes mismatch"));
    }
    Ok(())
}

fn validate_batch(
    values: &[Option<dms_client::GetResult>],
    count: usize,
    expected: &[u8],
) -> Result<(), dms_client::DmsError> {
    if values.len() != count
        || values
            .iter()
            .any(|value| value.as_ref().map(|value| value.bytes.as_slice()) != Some(expected))
    {
        return Err(corrupt("batch count, presence or exact bytes mismatch"));
    }
    Ok(())
}

fn validate_hash_entries(
    values: &[dms_client::HashValue],
    count: usize,
    expected: &[u8],
) -> Result<(), dms_client::DmsError> {
    if values.len() != count
        || values.iter().enumerate().any(|(index, value)| {
            value.field.as_bytes() != format!("field-{index:04}").as_bytes()
                || value.bytes != expected
        })
    {
        return Err(corrupt("hash field order, count or exact bytes mismatch"));
    }
    Ok(())
}

/// 写后的完整字节回读不计入主操作延迟；进程资源/指标仍包含准备与校验。
fn verify_write(config: &BenchConfig, context: &CaseContext) -> Result<(), dms_client::DmsError> {
    match context.case_name {
        "set_inline" | "set_staged_grpc" | "set_range" => {
            let suffix = match context.case_name {
                "set_inline" => "inline",
                "set_staged_grpc" => "staged",
                _ => "range",
            };
            let mut expected = context.value.clone();
            if context.case_name == "set_range" {
                let len = expected.len().min(32);
                let offset = expected.len().saturating_sub(len) / 2;
                expected[offset..offset + len].fill(0x52);
            }
            validate_values(&[context.client.get(context.key(suffix))?], 1, &expected)
        }
        "mset" => {
            let keys = (0..config.batch_size)
                .map(|item| format!("{}-{item}", context.key("mset")))
                .collect::<Vec<_>>();
            let key_refs = keys.iter().map(String::as_str).collect::<Vec<_>>();
            validate_batch(
                &context.client.mget(&key_refs)?,
                config.batch_size,
                &context.value,
            )
        }
        "hset" => validate_hash_entries(
            &context
                .client
                .hgetall(context.key("hash"), Default::default())?
                .entries,
            config.hash_fields,
            &context.value,
        ),
        _ => Ok(()),
    }
}

fn measured_runs(
    config: &BenchConfig,
    name: &'static str,
    value_len: usize,
    operation: impl Fn(&CaseContext) -> Result<(), dms_client::DmsError>,
) -> Vec<RunReport> {
    measured_runs_with_prepare(config, name, value_len, |_| Ok(()), operation)
}

fn measured_runs_with_prepare(
    config: &BenchConfig,
    name: &'static str,
    value_len: usize,
    prepare: impl Fn(&CaseContext) -> Result<(), dms_client::DmsError>,
    operation: impl Fn(&CaseContext) -> Result<(), dms_client::DmsError>,
) -> Vec<RunReport> {
    let warmup = execute_iterations(
        config,
        name,
        0,
        config.warmup_ops,
        value_len,
        &prepare,
        &operation,
    );
    let mut runs = (0..3)
        .map(|run| {
            execute_iterations(
                config,
                name,
                run + 1,
                config.ops,
                value_len,
                &prepare,
                &operation,
            )
        })
        .collect::<Vec<_>>();
    // 预热错误不伪装为测量样本，但仍使整次基准失败。
    runs[0].warmup_errors = warmup.errors;
    if warmup.errors > 0 && runs[0].first_error.is_none() {
        runs[0].first_error = warmup.first_error.map(|error| format!("warmup: {error}"));
    }
    runs
}

fn execute_iterations(
    config: &BenchConfig,
    name: &'static str,
    run_index: usize,
    iterations: usize,
    value_len: usize,
    prepare: &impl Fn(&CaseContext) -> Result<(), dms_client::DmsError>,
    operation: &impl Fn(&CaseContext) -> Result<(), dms_client::DmsError>,
) -> RunReport {
    let client = match connect(&config.endpoint) {
        Ok(client) => client,
        Err(error) => {
            return RunReport::connection_failed(iterations, error.to_string());
        }
    };
    let mut latencies = Vec::with_capacity(iterations);
    let mut errors = 0_u64;
    let mut first_error = None;
    let mut measured_elapsed = Duration::ZERO;
    for op_index in 0..iterations {
        let value = deterministic_bytes(value_len, run_index, op_index);
        let context = CaseContext {
            client: client.clone(),
            endpoint: config.endpoint.clone(),
            case_name: name,
            run_index,
            op_index,
            value,
        };
        if let Err(error) = prepare(&context) {
            errors += 1;
            if first_error.is_none() {
                first_error = Some(error.to_string());
            }
            continue;
        }
        let op_started = Instant::now();
        let outcome = operation(&context);
        let elapsed = op_started.elapsed();
        measured_elapsed += elapsed;
        if let Err(error) = outcome.and_then(|()| verify_write(config, &context)) {
            errors += 1;
            if first_error.is_none() {
                first_error = Some(error.to_string());
            }
        }
        latencies.push(elapsed);
    }
    let measured_samples = latencies.len() as u64;
    let mut report = RunReport::from_latencies(
        measured_samples,
        measured_elapsed,
        latencies,
        errors,
        first_error,
    );
    report.attempted_operations = iterations as u64;
    report
}

fn connect(endpoint: &str) -> Result<DmsClient, dms_client::ConnectError> {
    DmsClient::connect(endpoint, ClientOptions::default())
}

fn deterministic_bytes(len: usize, run_index: usize, op_index: usize) -> Vec<u8> {
    (0..len)
        .map(|offset| ((offset + run_index * 31 + op_index * 17) & 0xff) as u8)
        .collect()
}

fn estimate_steps(name: &str, config: &BenchConfig) -> StepEstimate {
    StepEstimate {
        items_per_operation: match name {
            "mset" | "mget" => config.batch_size,
            "hset" => config.hash_fields,
            "hscan" => config.hash_fields.min(8),
            _ => 1,
        },
    }
}

struct StepEstimate {
    items_per_operation: usize,
}

impl StepEstimate {
    fn to_json(&self) -> String {
        format!(
            "{{\"rpc_steps_per_operation\":null,\"payload_copy_estimate_per_operation\":null,\"measurement\":\"not_instrumented\",\"logical_requests_per_operation\":1,\"items_per_operation\":{}}}",
            self.items_per_operation
        )
    }
}

struct CaseReport {
    name: &'static str,
    capability: &'static str,
    implementation: &'static str,
    payload_bytes_per_operation: usize,
    estimate: StepEstimate,
    runs: Vec<RunReport>,
}

impl CaseReport {
    fn available(
        name: &'static str,
        implementation: &'static str,
        payload_bytes_per_operation: usize,
        estimate: StepEstimate,
        runs: Vec<RunReport>,
    ) -> Self {
        let capability = if reports_capability_unavailable(&runs) {
            "unavailable"
        } else {
            "available"
        };
        Self {
            name,
            capability,
            implementation,
            payload_bytes_per_operation,
            estimate,
            runs,
        }
    }

    fn to_json(&self) -> String {
        format!(
            "{{\"name\":\"{}\",\"capability\":\"{}\",\"implementation\":\"{}\",\"payload_bytes_per_operation\":{},\"estimate\":{},\"runs\":[{}]}}",
            json_escape(self.name),
            json_escape(self.capability),
            json_escape(self.implementation),
            self.payload_bytes_per_operation,
            self.estimate.to_json(),
            self.runs
                .iter()
                .map(RunReport::to_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

fn reports_capability_unavailable(runs: &[RunReport]) -> bool {
    !runs.is_empty()
        && runs.iter().all(|run| {
            run.errors == run.attempted_operations
                && run
                    .first_error
                    .as_deref()
                    .is_some_and(|error| error.contains("unsupported"))
        })
}

struct RunReport {
    attempted_operations: u64,
    warmup_errors: u64,
    samples: u64,
    elapsed_micros: u128,
    throughput_ops_per_sec: f64,
    p50_micros: u128,
    p99_micros: u128,
    errors: u64,
    first_error: Option<String>,
}

impl RunReport {
    fn connection_failed(samples: usize, error: String) -> Self {
        Self {
            attempted_operations: samples as u64,
            warmup_errors: 0,
            samples: 0,
            elapsed_micros: 0,
            throughput_ops_per_sec: 0.0,
            p50_micros: 0,
            p99_micros: 0,
            errors: samples as u64,
            first_error: Some(error),
        }
    }

    fn from_latencies(
        samples: u64,
        elapsed: Duration,
        latencies: Vec<Duration>,
        errors: u64,
        first_error: Option<String>,
    ) -> Self {
        let mut micros = latencies
            .into_iter()
            .map(|duration| duration.as_micros())
            .collect::<Vec<_>>();
        micros.sort_unstable();
        let elapsed_secs = elapsed.as_secs_f64();
        Self {
            attempted_operations: samples,
            warmup_errors: 0,
            samples,
            elapsed_micros: elapsed.as_micros(),
            throughput_ops_per_sec: if elapsed_secs > 0.0 {
                samples as f64 / elapsed_secs
            } else {
                0.0
            },
            p50_micros: percentile(&micros, 0.50),
            p99_micros: percentile(&micros, 0.99),
            errors,
            first_error,
        }
    }

    fn to_json(&self) -> String {
        format!(
            "{{\"attempted_operations\":{},\"warmup_errors\":{},\"samples\":{},\"elapsed_micros\":{},\"throughput_ops_per_sec\":{:.3},\"p50_micros\":{},\"p99_micros\":{},\"errors\":{},\"first_error\":{}}}",
            self.attempted_operations,
            self.warmup_errors,
            self.samples,
            self.elapsed_micros,
            self.throughput_ops_per_sec,
            self.p50_micros,
            self.p99_micros,
            self.errors,
            self.first_error
                .as_ref()
                .map(|error| format!("\"{}\"", json_escape(error)))
                .unwrap_or_else(|| "null".to_string())
        )
    }
}

fn percentile(sorted: &[u128], ratio: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((sorted.len() as f64 * ratio).ceil() as usize).saturating_sub(1);
    sorted[rank.min(sorted.len() - 1)]
}

#[derive(Clone, Default)]
struct ProcessStats {
    pid: Option<u32>,
    rss_kib: Option<u64>,
    user_ticks: Option<u64>,
    system_ticks: Option<u64>,
}

impl ProcessStats {
    fn to_json(&self) -> String {
        format!(
            "{{\"pid\":{},\"rss_kib\":{},\"user_ticks\":{},\"system_ticks\":{}}}",
            option_u32(self.pid),
            option_u64(self.rss_kib),
            option_u64(self.user_ticks),
            option_u64(self.system_ticks)
        )
    }
}

#[derive(Clone, Default)]
struct ProcessSample {
    client: ProcessStats,
    node: ProcessStats,
    meta: ProcessStats,
}

impl ProcessSample {
    fn collect() -> Self {
        Self {
            client: read_process_stats(std::process::id()),
            node: process_from_env("DMS_BENCH_NODE_PID"),
            meta: process_from_env("DMS_BENCH_META_PID"),
        }
    }

    fn to_json(&self) -> String {
        format!(
            "{{\"client\":{},\"node\":{},\"meta\":{}}}",
            self.client.to_json(),
            self.node.to_json(),
            self.meta.to_json()
        )
    }
}

fn process_from_env(name: &str) -> ProcessStats {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .map(read_process_stats)
        .unwrap_or_default()
}

fn read_process_stats(pid: u32) -> ProcessStats {
    let mut stats = ProcessStats {
        pid: Some(pid),
        ..ProcessStats::default()
    };
    if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                stats.rss_kib = rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok());
            }
        }
    }
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat"))
        && let Some(after_comm) = stat.rsplit_once(") ")
    {
        let fields = after_comm.1.split_whitespace().collect::<Vec<_>>();
        stats.user_ticks = fields.get(11).and_then(|value| value.parse::<u64>().ok());
        stats.system_ticks = fields.get(12).and_then(|value| value.parse::<u64>().ok());
    }
    stats
}

struct BenchDocument {
    generated_unix_millis: u128,
    endpoint: String,
    config: BenchConfig,
    process_before: ProcessSample,
    process_after: ProcessSample,
    cases: Vec<CaseReport>,
}

impl BenchDocument {
    fn to_json(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"schema\":\"dms.benchmark.v2\",",
                "\"measurement_contract\":\"one logical SDK request per timed operation; get_current_uncached includes fresh connect; write readback outside latency; reads validate bytes inside latency; resources include setup, warmup and verification; no RPC or copy measurement\",",
                "\"generated_unix_millis\":{},",
                "\"endpoint\":\"{}\",",
                "\"environment\":{},",
                "\"config\":{},",
                "\"process_before\":{},",
                "\"process_after\":{},",
                "\"cases\":[{}]",
                "}}"
            ),
            self.generated_unix_millis,
            json_escape(&self.endpoint),
            environment_json(),
            config_json(&self.config),
            self.process_before.to_json(),
            self.process_after.to_json(),
            self.cases
                .iter()
                .map(CaseReport::to_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

fn config_json(config: &BenchConfig) -> String {
    format!(
        "{{\"ops\":{},\"warmup_ops\":{},\"inline_bytes\":{},\"staged_bytes\":{},\"batch_size\":{},\"hash_fields\":{},\"runs_per_case\":3}}",
        config.ops,
        config.warmup_ops,
        config.inline_bytes,
        config.staged_bytes,
        config.batch_size,
        config.hash_fields
    )
}

fn environment_json() -> String {
    format!(
        "{{\"os\":\"{}\",\"arch\":\"{}\",\"rust_version\":\"{}\",\"note\":\"runs only inside Linux VM/container; RDMA and UB are not simulated\"}}",
        json_escape(env::consts::OS),
        json_escape(env::consts::ARCH),
        json_escape(option_env!("RUSTC_VERSION").unwrap_or("unknown"))
    )
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn option_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn option_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_validation_rejects_present_but_corrupt_or_extra_bytes() {
        assert!(validate_values(&[Some(vec![1, 2])], 1, &[1, 2]).is_ok());
        assert!(validate_values(&[Some(vec![1, 3])], 1, &[1, 2]).is_err());
        assert!(validate_values(&[Some(vec![1, 2, 3])], 1, &[1, 2]).is_err());
        assert!(validate_values(&[None], 1, &[1, 2]).is_err());
        assert!(validate_values(&[], 1, &[1, 2]).is_err());
    }

    #[test]
    fn batch_rejects_wrong_bytes_even_when_every_slot_is_present() {
        let mut values = vec![Some(dms_client::GetResult {
            version: dms_client::ObjectVersion(1),
            bytes: vec![1, 2],
        })];
        assert!(validate_batch(&values, 1, &[1, 2]).is_ok());
        values[0].as_mut().unwrap().bytes[1] = 3;
        assert!(validate_batch(&values, 1, &[1, 2]).is_err());
    }

    #[test]
    fn hash_page_rejects_wrong_field_or_bytes_despite_nonempty_result() {
        let mut values = vec![dms_client::HashValue {
            field: dms_client::HashField::new(b"field-0000".to_vec()).unwrap(),
            hash_version: dms_client::HashVersion(1),
            value_version: dms_client::ObjectVersion(1),
            bytes: vec![1, 2],
        }];
        assert!(validate_hash_entries(&values, 1, &[1, 2]).is_ok());
        values[0].bytes[0] = 3;
        assert!(validate_hash_entries(&values, 1, &[1, 2]).is_err());
        values[0].bytes[0] = 1;
        values[0].field = dms_client::HashField::new(b"field-0001".to_vec()).unwrap();
        assert!(validate_hash_entries(&values, 1, &[1, 2]).is_err());
        assert!(validate_hash_entries(&values, 2, &[1, 2]).is_err());
    }

    #[test]
    fn failed_connection_does_not_invent_latency_samples() {
        let report = RunReport::connection_failed(40, "offline".into());
        assert_eq!(report.attempted_operations, 40);
        assert_eq!(report.samples, 0);
        assert_eq!(report.errors, 40);
    }

    #[test]
    fn rpc_and_copy_values_are_explicitly_not_measured() {
        let json = StepEstimate {
            items_per_operation: 8,
        }
        .to_json();
        assert!(json.contains("\"rpc_steps_per_operation\":null"));
        assert!(json.contains("\"payload_copy_estimate_per_operation\":null"));
        assert!(json.contains("\"logical_requests_per_operation\":1"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn explicit_live_pid_has_resource_samples() {
        let sample = read_process_stats(std::process::id());
        assert_eq!(sample.pid, Some(std::process::id()));
        assert!(sample.rss_kib.is_some_and(|rss| rss > 0));
        assert!(sample.user_ticks.is_some());
        assert!(sample.system_ticks.is_some());
    }
}

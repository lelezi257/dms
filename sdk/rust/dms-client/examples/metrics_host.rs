//! Minimal embedding application that exposes Client SDK metrics.
//!
//! The DMS SDK only registers collectors in the host-owned Registry. This
//! example supplies the HTTP endpoint so users can see that ownership boundary
//! without introducing an SDK listener or global Registry.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
};

use dms_client::{ClientOptions, DmsClient, MetricsRegistry, encode_metrics_text};
use dms_metrics::{OPENMETRICS_CONTENT_TYPE, TraceRuntimeMetrics};
use dms_tracing::{ProcessIdentity, TracingConfig, init_process_tracing};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::var("DMS_ENDPOINT")
        .unwrap_or_else(|_| "unix:///tmp/dms-local-dev/run/dms-worker.sock".to_string());
    let listen_address =
        std::env::var("DMS_CLIENT_METRICS_ADDRESS").unwrap_or_else(|_| "0.0.0.0:19400".to_string());
    let registry = MetricsRegistry::new();
    // This example is the host process, so it owns the Subscriber/exporter.
    // `dms-client` itself never performs this initialization.
    let trace_metrics = TraceRuntimeMetrics::register(&registry)?;
    let tracing_enabled = env_bool("DMS_TRACING_ENABLED", false);
    let _tracing_guard = init_process_tracing(
        &TracingConfig {
            enabled: tracing_enabled,
            otlp_endpoint: std::env::var("DMS_TRACING_OTLP_ENDPOINT")
                .unwrap_or_else(|_| "http://127.0.0.1:4317".to_string()),
            sample_ratio: std::env::var("DMS_TRACING_SAMPLE_RATIO")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1.0),
            ..TracingConfig::default()
        },
        ProcessIdentity::new(
            "dms-client",
            std::env::var("DMS_CLIENT_INSTANCE").unwrap_or_else(|_| "metrics-host".to_string()),
        ),
        Some(trace_metrics),
    )?;
    let client = DmsClient::connect_with_options(ClientOptions {
        endpoint: Some(endpoint.clone()),
        // 本示例默认连接本地 UDS，显式打开 SHM，确保 /metrics 展示真实
        // mmap/SCM_RIGHTS 数据面，而不是只展示 gRPC fallback。
        shared_memory: Some(true),
        metrics_registry: Some(registry.clone()),
        ..ClientOptions::default()
    })?;

    // 首次抓取前执行真实 SET/GET；GET 始终访问 Node，不在 SDK 缓存 value。
    client.set("metrics/example", b"visible-client-metrics")?;
    let _ = client.get("metrics/example")?;
    // 96 KiB 超过默认 inline 阈值，强制走 AllocateStaging→SHM upload→Set，
    // 随后的 GET 再走 SHM read。这样示例不是只注册空指标，而是真正产生
    // provider、bytes、duration、Region mapping hit/miss 数据。
    let large_value = vec![b'm'; 96 * 1024];
    client.set("metrics/large-shm", &large_value)?;
    let large_read = client
        .get("metrics/large-shm")?
        .ok_or("large metrics value unexpectedly missing")?;
    if large_read != large_value {
        return Err("large SHM metrics value mismatch".into());
    }
    // Exercise one stable SDK-boundary failure so the CounterVec-backed
    // dms_errors_total family is visible without corrupting server state.
    let _expected_invalid_key = client.get(b"");

    let listener = TcpListener::bind(&listen_address)?;
    println!("dms-client metrics host ready address={listen_address} endpoint={endpoint}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => serve_one(stream, &registry)?,
            Err(error) => eprintln!("metrics host accept failed: {error}"),
        }
    }
    Ok(())
}

fn serve_one(mut stream: TcpStream, registry: &MetricsRegistry) -> std::io::Result<()> {
    let mut request = [0_u8; 1024];
    let read = stream.read(&mut request)?;
    let first_line = std::str::from_utf8(&request[..read])
        .ok()
        .and_then(|text| text.lines().next())
        .unwrap_or_default();
    let (status, content_type, body) = if first_line.starts_with("GET /metrics ") {
        match encode_metrics_text(registry) {
            Ok(body) => ("200 OK", OPENMETRICS_CONTENT_TYPE, body),
            Err(error) => (
                "500 Internal Server Error",
                "text/plain",
                format!("metrics encode failed: {error}\n"),
            ),
        }
    } else if first_line.starts_with("GET /healthz ") {
        ("200 OK", "text/plain", "ok\n".to_string())
    } else {
        ("404 Not Found", "text/plain", "not found\n".to_string())
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.as_str() {
            "1" | "true" | "TRUE" | "yes" | "YES" => Some(true),
            "0" | "false" | "FALSE" | "no" | "NO" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

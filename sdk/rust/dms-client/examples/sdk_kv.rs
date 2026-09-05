//! Tiny command-line SDK probe used by process-level recovery tests.
//!
//! It intentionally depends only on the public `dms-client` API, the same way a
//! Rust application would.  The endpoint is resolved from `DMS_ENDPOINT` so the
//! same binary can exercise either local UDS or TCP.

use dms_client::{ClientOptions, DmsClient};
use dms_error::NODE_ARENA_CAPACITY_EXHAUSTED;
use dms_tracing::{ProcessIdentity, TracingConfig, init_process_tracing};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::var("DMS_ENDPOINT")
        .unwrap_or_else(|_| "unix:///tmp/dms-local-dev/run/dms-worker.sock".to_string());
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // `sdk_kv` is a host application, so it may install a process Subscriber.
    // The SDK library itself never performs this initialization. Keeping it
    // environment-driven also lets the exact same binary run with tracing off.
    let _tracing_guard = init_process_tracing(
        &TracingConfig {
            enabled: env_bool("DMS_TRACING_ENABLED", false),
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
            std::env::var("DMS_CLIENT_INSTANCE").unwrap_or_else(|_| "sdk-kv".to_string()),
        ),
        None,
    )?;
    let command = args.first().map(String::as_str).unwrap_or("unknown");
    let invocation_span = invocation_span(command);
    let _invocation_scope = invocation_span.enter();
    let client = DmsClient::connect(&endpoint, ClientOptions::default())?;

    match args.as_slice() {
        [command, key, value] if command == "set" => {
            let result = client.set(key, value.as_bytes())?;
            println!(
                "set ok endpoint={endpoint} key={key} version={}",
                result.version.0
            );
        }
        [command, key, expected] if command == "get" => {
            let actual = client.get(key)?;
            if actual.as_deref() != Some(expected.as_bytes()) {
                return Err(format!(
                    "get mismatch endpoint={endpoint} key={key} expected={expected:?} actual={:?}",
                    actual.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                )
                .into());
            }
            println!("get ok endpoint={endpoint} key={key} value={expected}");
        }
        [command, key] if command == "del" => {
            let result = client.del(key)?;
            println!(
                "del ok endpoint={endpoint} key={key} deleted={}",
                result.deleted
            );
        }
        [command, present_key, present_value, missing_key] if command == "mget-miss" => {
            // This command is intentionally used by process-level scripts: it
            // proves a missing item inside MGET is represented as Ok(None), not
            // as a process/API error.
            client.set(present_key, present_value.as_bytes())?;
            let values = client.mget(&[present_key.as_bytes(), missing_key.as_bytes()])?;
            if values.len() != 2 {
                return Err(format!("mget returned {} items, expected 2", values.len()).into());
            }
            if values[0].as_ref().map(|value| value.bytes.as_slice())
                != Some(present_value.as_bytes())
            {
                return Err("mget present item mismatch".into());
            }
            if values[1].is_some() {
                return Err("mget missing item unexpectedly returned a value".into());
            }
            println!(
                "mget miss ok endpoint={endpoint} present={present_key} missing={missing_key} missing_is_none=true"
            );
        }
        [command, key, length] if command == "expect-capacity-error" => {
            let length = length.parse::<usize>()?;
            let value = vec![b'x'; length];
            let error = client
                .set(key, &value)
                .expect_err("capacity probe must exceed the configured Node arena");
            println!(
                "set expected error endpoint={endpoint} key={key} error_code_raw={} error_code_hex={:#010x} kind={:?} message={}",
                error.code().raw(),
                error.code().raw(),
                error.kind(),
                error.message()
            );
            if error.code() != NODE_ARENA_CAPACITY_EXHAUSTED {
                return Err(format!(
                    "unexpected DMS error code: got {} expected {}",
                    error.code().raw(),
                    NODE_ARENA_CAPACITY_EXHAUSTED.raw()
                )
                .into());
            }
        }
        _ => {
            return Err(
                "usage: sdk_kv set <key> <value> | sdk_kv get <key> <expected> | sdk_kv del <key> | sdk_kv mget-miss <present-key> <present-value> <missing-key> | sdk_kv expect-capacity-error <key> <length>"
                    .into(),
            );
        }
    }

    if let Some(correlation) = dms_tracing::current_correlation() {
        println!("trace_id={}", correlation.trace_id);
    }

    Ok(())
}

/// Gives Tempo a useful root name. The command set is finite, so this does not
/// turn arbitrary user input into an unbounded span-name dimension.
fn invocation_span(command: &str) -> dms_tracing::tracing::Span {
    match command {
        "set" => dms_tracing::tracing::info_span!(
            target: "dms_client",
            "dms.sdk_kv.set",
            otel.kind = "internal"
        ),
        "get" => dms_tracing::tracing::info_span!(
            target: "dms_client",
            "dms.sdk_kv.get",
            otel.kind = "internal"
        ),
        "del" => dms_tracing::tracing::info_span!(
            target: "dms_client",
            "dms.sdk_kv.del",
            otel.kind = "internal"
        ),
        "mget-miss" => dms_tracing::tracing::info_span!(
            target: "dms_client",
            "dms.sdk_kv.mget_miss",
            otel.kind = "internal"
        ),
        "expect-capacity-error" => dms_tracing::tracing::info_span!(
            target: "dms_client",
            "dms.sdk_kv.expect_capacity_error",
            otel.kind = "internal"
        ),
        _ => dms_tracing::tracing::info_span!(
            target: "dms_client",
            "dms.sdk_kv.unknown",
            otel.kind = "internal"
        ),
    }
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

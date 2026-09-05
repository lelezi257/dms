//! Small application that exercises the explicit local shared-memory SDK path.
//!
//! Run it with `DMS_ENDPOINT=unix:///path/to/dms-worker.sock`. TCP endpoints
//! intentionally fail because `allocate_write` must not silently fall back to
//! copied gRPC payloads.

use dms_client::{ClientOptions, DmsClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::var("DMS_ENDPOINT")
        .unwrap_or_else(|_| "unix:///tmp/dms-local-dev/run/dms-worker.sock".to_string());
    let client = DmsClient::connect(
        &endpoint,
        ClientOptions {
            shared_memory: Some(true),
            ..ClientOptions::default()
        },
    )?;
    let key = "shm/checkpoint/latest";
    let payload = b"manifest-from-shared-memory";

    let mut buffer = client.allocate_write(key, payload.len())?;
    buffer.as_mut_slice()?.copy_from_slice(payload);
    let committed = client.commit_shared(buffer)?;

    let view = client.get_view(key)?.ok_or("shared value is missing")?;
    if view.version() != committed.version {
        return Err("shared view returned an unexpected version".into());
    }
    if view.as_slice()? != payload {
        return Err("shared view bytes do not match committed payload".into());
    }

    println!(
        "dms-client explicit SHM write/view passed endpoint={endpoint} version={} len={}",
        committed.version.0,
        view.len()?
    );
    Ok(())
}

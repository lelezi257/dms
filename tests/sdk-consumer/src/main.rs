//! External-consumer compile fixture.
//!
//! Every imported domain type comes through `dms-client`; this crate does not
//! depend on a separately published common or protocol crate.

use dms_client::{ClientOptions, DmsClient, HashEntry, HashWriteMode, HashWriteOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = DmsClient::connect("unix:///run/dms/worker.sock", ClientOptions::default())?;
    client.hset(
        "checkpoint/42",
        &[HashEntry::new("model-0001", b"object-location")?],
        HashWriteOptions {
            mode: HashWriteMode::Replace,
            ..HashWriteOptions::default()
        },
    )?;
    Ok(())
}

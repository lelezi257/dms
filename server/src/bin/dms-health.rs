//! Minimal operator CLI used by the M0 deployment and manual walkthrough.

#![forbid(unsafe_code)]

use std::{net::SocketAddr, time::Duration};

use dms_server::NodeId;
use dms_server::health::HealthClient;

fn main() {
    if let Err(error) = run() {
        eprintln!("dms-health error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().map(String::as_str) != Some("health") {
        return Err(usage().into());
    }

    let address: SocketAddr = required_option(&arguments, "--address")?.parse()?;
    let expected_component = optional_option(&arguments, "--expect-component")
        .map(str::parse)
        .transpose()?;
    let expected_node = optional_option(&arguments, "--expect-node")
        .map(NodeId::new)
        .transpose()?;

    let status = HealthClient::new(Duration::from_secs(2)).probe(address)?;
    verify_expected(status.component, expected_component, "component")?;
    verify_expected(
        status.node_id.as_str(),
        expected_node.as_ref().map(NodeId::as_str),
        "node",
    )?;
    println!(
        "ready component={} node={} protocol=1 address={address}",
        status.component, status.node_id
    );
    Ok(())
}

fn required_option<'a>(arguments: &'a [String], name: &str) -> Result<&'a str, &'static str> {
    optional_option(arguments, name).ok_or_else(usage)
}

fn optional_option<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn verify_expected<T>(actual: T, expected: Option<T>, field: &'static str) -> Result<(), String>
where
    T: Eq + std::fmt::Display,
{
    if let Some(expected) = expected
        && actual != expected
    {
        return Err(format!(
            "unexpected {field}: expected {expected}, received {actual}"
        ));
    }
    Ok(())
}

const fn usage() -> &'static str {
    "usage: dms-health health --address HOST:PORT [--expect-component dms-node|dms-meta] [--expect-node ID]"
}

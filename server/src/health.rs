//! Versioned cross-process protocol contracts.
//!
//! M0 contains only an operational health protocol. Object APIs are added here
//! only when their corresponding IF1/IF2/IF3 implementation iteration begins.

#![forbid(unsafe_code)]

use std::{
    fmt,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use dms_metrics::{OPENMETRICS_CONTENT_TYPE, Registry, encode_text};
use tokio::net::TcpListener as TokioTcpListener;

use crate::{ComponentKind, InvalidNodeId, NodeId, ParseComponentKindError};

/// Version of the M0 operational health wire contract.
pub const HEALTH_PROTOCOL_VERSION: u16 = 1;

/// Exact request sent by an operational health client.
pub const HEALTH_REQUEST: &str = "DMS/1 HEALTH\n";

/// Process lifecycle exposed by the operational health endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Readiness {
    Starting,
    Ready,
    Stopping,
}

impl Readiness {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::Starting => "STARTING",
            Self::Ready => "READY",
            Self::Stopping => "STOPPING",
        }
    }

    fn from_wire(value: &str) -> Result<Self, ProtocolError> {
        match value {
            "STARTING" => Ok(Self::Starting),
            "READY" => Ok(Self::Ready),
            "STOPPING" => Ok(Self::Stopping),
            other => Err(ProtocolError::UnexpectedState(other.to_owned())),
        }
    }
}

/// Cloneable lifecycle cell shared by the process composition root and health task.
#[derive(Clone, Debug)]
pub struct ReadinessState(Arc<AtomicU8>);

impl Default for ReadinessState {
    fn default() -> Self {
        Self(Arc::new(AtomicU8::new(0)))
    }
}

impl ReadinessState {
    #[must_use]
    pub fn get(&self) -> Readiness {
        match self.0.load(Ordering::Acquire) {
            1 => Readiness::Ready,
            2 => Readiness::Stopping,
            _ => Readiness::Starting,
        }
    }

    pub fn set(&self, state: Readiness) {
        let encoded = match state {
            Readiness::Starting => 0,
            Readiness::Ready => 1,
            Readiness::Stopping => 2,
        };
        self.0.store(encoded, Ordering::Release);
    }
}

/// Process identity returned by the operational health endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthStatus {
    /// Kind of process that accepted the connection.
    pub component: ComponentKind,
    /// Stable identity configured for that process instance.
    pub node_id: NodeId,
    /// Current process lifecycle state.
    pub readiness: Readiness,
}

impl HealthStatus {
    /// Encodes one newline-terminated response.
    #[must_use]
    pub fn encode(&self) -> String {
        format!(
            "DMS/{HEALTH_PROTOCOL_VERSION} {} component={} node={}\n",
            self.readiness.as_wire(),
            self.component,
            self.node_id
        )
    }

    /// Parses and validates one response line.
    pub fn decode(line: &str) -> Result<Self, ProtocolError> {
        let line = line.trim_end_matches(['\r', '\n']);
        let mut fields = line.split_whitespace();
        let version = fields.next().ok_or(ProtocolError::MalformedResponse)?;
        let state = fields.next().ok_or(ProtocolError::MalformedResponse)?;
        if version != "DMS/1" {
            return Err(ProtocolError::UnsupportedVersion(version.to_owned()));
        }
        let readiness = Readiness::from_wire(state)?;

        let component = parse_named_field(fields.next(), "component")?.parse()?;
        let node_id = NodeId::from_str(parse_named_field(fields.next(), "node")?)?;
        if fields.next().is_some() {
            return Err(ProtocolError::MalformedResponse);
        }
        Ok(Self {
            component,
            node_id,
            readiness,
        })
    }
}

fn parse_named_field<'a>(
    field: Option<&'a str>,
    expected_name: &'static str,
) -> Result<&'a str, ProtocolError> {
    let field = field.ok_or(ProtocolError::MissingField(expected_name))?;
    let (name, value) = field
        .split_once('=')
        .ok_or(ProtocolError::MalformedResponse)?;
    if name == expected_name && !value.is_empty() {
        Ok(value)
    } else {
        Err(ProtocolError::MissingField(expected_name))
    }
}

/// Error caused by an invalid or incompatible health message.
#[derive(Debug)]
pub enum ProtocolError {
    /// The peer used a protocol version this implementation does not support.
    UnsupportedVersion(String),
    /// A required named field was absent.
    MissingField(&'static str),
    /// The response did not report readiness.
    UnexpectedState(String),
    /// The response was not a valid health message.
    MalformedResponse,
    /// A component name was invalid.
    InvalidComponent(ParseComponentKindError),
    /// A process identity was invalid.
    InvalidNodeId(InvalidNodeId),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported health protocol version: {version}")
            }
            Self::MissingField(field) => write!(formatter, "missing health field: {field}"),
            Self::UnexpectedState(state) => write!(formatter, "unexpected health state: {state}"),
            Self::MalformedResponse => formatter.write_str("malformed health response"),
            Self::InvalidComponent(error) => write!(formatter, "{error}"),
            Self::InvalidNodeId(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<ParseComponentKindError> for ProtocolError {
    fn from(error: ParseComponentKindError) -> Self {
        Self::InvalidComponent(error)
    }
}

impl From<InvalidNodeId> for ProtocolError {
    fn from(error: InvalidNodeId) -> Self {
        Self::InvalidNodeId(error)
    }
}

#[derive(Clone)]
struct StatusState {
    component: ComponentKind,
    node_id: NodeId,
    readiness: ReadinessState,
    registry: Registry,
}

/// Serves process health, readiness and Prometheus exposition over HTTP.
///
/// This is an operational endpoint, separate from business gRPC. Both Node and
/// Meta use the same routes: `/healthz`, `/readyz`, and `/metrics`.
pub async fn serve_status(
    listener: TokioTcpListener,
    component: ComponentKind,
    node_id: NodeId,
    readiness: ReadinessState,
    registry: Registry,
) -> io::Result<()> {
    let state = StatusState {
        component,
        node_id,
        readiness,
        registry,
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(state);
    axum::serve(listener, app).await
}

async fn healthz(State(state): State<StatusState>) -> String {
    HealthStatus {
        component: state.component,
        node_id: state.node_id,
        readiness: state.readiness.get(),
    }
    .encode()
}

async fn readyz(State(state): State<StatusState>) -> Response {
    let status = HealthStatus {
        component: state.component,
        node_id: state.node_id,
        readiness: state.readiness.get(),
    };
    let http_status = if status.readiness == Readiness::Ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (http_status, status.encode()).into_response()
}

async fn metrics(State(state): State<StatusState>) -> Response {
    match encode_text(&state.registry) {
        Ok(body) => (
            StatusCode::OK,
            [("content-type", OPENMETRICS_CONTENT_TYPE)],
            body,
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to encode metrics: {error}\n"),
        )
            .into_response(),
    }
}

/// Blocking operator client for the versioned health endpoint.
#[derive(Clone, Debug)]
pub struct HealthClient {
    timeout: Duration,
}

impl HealthClient {
    /// Creates a health client with the same connect/read/write timeout.
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Connects to one process and returns its validated identity.
    pub fn probe(&self, address: SocketAddr) -> Result<HealthStatus, HealthClientError> {
        let mut stream = TcpStream::connect_timeout(&address, self.timeout)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        stream.write_all(
            format!("GET /readyz HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )?;
        stream.flush()?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        let (_, body) = response
            .split_once("\r\n\r\n")
            .ok_or(ProtocolError::MalformedResponse)?;
        let status = HealthStatus::decode(body)?;
        if status.readiness != Readiness::Ready {
            return Err(HealthClientError::NotReady(status.readiness));
        }
        Ok(status)
    }
}

/// Failure to reach or validate a DMS process health endpoint.
#[derive(Debug)]
pub enum HealthClientError {
    /// Socket or timeout failure.
    Io(io::Error),
    /// The peer response violated the versioned health contract.
    Protocol(ProtocolError),
    /// The endpoint is reachable but the business service is not ready.
    NotReady(Readiness),
}

impl fmt::Display for HealthClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "health I/O failed: {error}"),
            Self::Protocol(error) => write!(formatter, "health protocol failed: {error}"),
            Self::NotReady(state) => write!(formatter, "process is not ready: {state:?}"),
        }
    }
}

impl std::error::Error for HealthClientError {}

impl From<io::Error> for HealthClientError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ProtocolError> for HealthClientError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

#[cfg(test)]
mod tests {
    use crate::{ComponentKind, NodeId};

    use super::HealthStatus;

    #[test]
    fn health_status_round_trips() {
        let status = HealthStatus {
            component: ComponentKind::Node,
            node_id: NodeId::new("n1").expect("valid fixture"),
            readiness: super::Readiness::Ready,
        };
        assert_eq!(HealthStatus::decode(&status.encode()).unwrap(), status);
    }

    #[test]
    fn health_status_rejects_wrong_version() {
        let error = HealthStatus::decode("DMS/2 READY component=dms-node node=n1\n")
            .expect_err("version must be rejected");
        assert!(error.to_string().contains("unsupported"));
    }
}
